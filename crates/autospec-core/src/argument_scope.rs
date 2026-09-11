//! Argument parsers and environment-scoped runs (issue #4292).
//!
//! `convselect.sh` was scoped to a project through environment variables
//! (`R=` / `OUT=` / `SEEN=`) and its argument loop had no `*)` branch:
//! `convselect.sh iw` was accepted, ignored, and scoped to autospec anyway.
//! Both invocations printed byte-identical counts —
//! `considered=100 finished_patches=3 closed_issue=0 candidates=0
//! retry-held=2` — and that identity is what made InferWeave's real
//! candidate (issue #288) invisible: the number was right and the input
//! was wrong.
//!
//! Invariants, each encoded as a checkable primitive:
//!
//! 1. Every argument parser has a `*)` catch-all that rejects unknown
//!    arguments as an error the caller turns into a non-zero exit —
//!    `StrictParser::parse` returns `Err(Rejection)`, never swallows.
//! 2. The rejection message names the correct mechanism, not just the
//!    error ("scope with R=/OUT=/SEEN= env vars, not positionally") —
//!    `validate_rejection_message` checks a message names both the
//!    offending argument and the mechanism.
//! 3. Two runs that should differ but produced identical output is a
//!    defect — `identical_output_pairs` / `per_project_findings` are
//!    worth an explicit check anywhere a tool is run per-project in a
//!    loop.
//! 4. A tool whose scope comes from the environment prints that scope
//!    on every run — `EnvScope::line` renders the line and
//!    `EnvScope::reported_in` checks it appears in the output.

use std::collections::{BTreeMap, BTreeSet};

/// Invariant 1: an argument parser with a `*)` catch-all.
///
/// `known` is the closed set of accepted arguments; anything else is a
/// [`Rejection`], never a silently ignored value. `mechanism` is the
/// correct way to give the tool the value the caller tried to pass
/// positionally (for `convselect.sh`: "scope with R=/OUT=/SEEN= env
/// vars, not positionally") and is embedded in the rejection message —
/// a rejection that only says "unknown argument" tells the caller there
/// is a problem but not how to fix it.
///
/// The caller maps `Err` to a non-zero exit (exit 2 in the shell fix).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StrictParser {
    known: BTreeSet<String>,
    mechanism: String,
}

impl StrictParser {
    pub fn new(
        known: impl IntoIterator<Item = impl Into<String>>,
        mechanism: impl Into<String>,
    ) -> Self {
        Self {
            known: known.into_iter().map(Into::into).collect(),
            mechanism: mechanism.into(),
        }
    }

    /// Parse `args` against the closed `known` set.
    ///
    /// Ok: the accepted arguments, in order. Err: the first argument not
    /// in the set, as a [`Rejection`] — the parser stops there, the way a
    /// shell `case` with a `*)` that `exit 2`s would.
    pub fn parse(&self, args: &[&str]) -> Result<Vec<String>, Rejection> {
        let mut out = Vec::new();
        for &arg in args {
            if self.known.contains(arg) {
                out.push(arg.to_string());
            } else {
                return Err(Rejection {
                    argument: arg.to_string(),
                    mechanism: self.mechanism.clone(),
                });
            }
        }
        Ok(out)
    }
}

/// Invariant 1 + 2: an unknown argument the parser refused.
///
/// The caller prints [`Rejection::line`] to stderr and exits non-zero.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejection {
    pub argument: String,
    pub mechanism: String,
}

impl Rejection {
    /// The stderr line for this rejection: it names the offending
    /// argument and the mechanism that would have been correct.
    pub fn line(&self) -> String {
        format!(
            "error: unknown argument '{}': {}",
            self.argument, self.mechanism
        )
    }
}

/// Invariant 2, as a check on a hand-written rejection message:
/// findings for a message that does not name the offending argument and
/// the correct mechanism. Empty means the message is adequate — the
/// caller knows both that something went wrong and how to fix it.
pub fn validate_rejection_message(message: &str, argument: &str, mechanism: &str) -> Vec<String> {
    let mut findings = Vec::new();
    if !argument.is_empty() && !message.contains(argument) {
        findings.push(format!(
            "REJECTION_MISSING_ARGUMENT: message does not name the offending argument '{argument}'"
        ));
    }
    if !mechanism.is_empty() && !message.contains(mechanism) {
        findings.push(format!(
            "REJECTION_MISSING_MECHANISM: message does not name the correct mechanism ({mechanism})"
        ));
    }
    findings
}

/// Invariant 4: the scope an environment-scoped tool actually ran under.
///
/// `vars` maps the variable names (`R`, `OUT`, `SEEN`) to the values the
/// run used. [`EnvScope::line`] is the line the tool prints on every run
/// — the scope is a fact about the output, not an assumption the reader
/// makes about it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EnvScope {
    vars: BTreeMap<String, String>,
}

impl EnvScope {
    pub fn new(vars: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>) -> Self {
        Self {
            vars: vars
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.vars.is_empty()
    }

    /// The scope line: `scope: K1=V1 K2=V2 …`, keys sorted so the line is
    /// deterministic regardless of insertion order.
    pub fn line(&self) -> String {
        let mut parts = Vec::new();
        for (k, v) in &self.vars {
            parts.push(format!("{k}={v}"));
        }
        format!("scope: {}", parts.join(" "))
    }

    /// Whether `output` reports this scope on a line of its own.
    pub fn reported_in(&self, output: &str) -> bool {
        if self.is_empty() {
            return true;
        }
        let line = self.line();
        output.lines().any(|l| l.trim() == line)
    }
}

/// One per-project run: which project, under what scope, and what the
/// tool printed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunOutput {
    pub name: String,
    pub scope: EnvScope,
    pub output: String,
}

/// Invariant 3: pairs of runs under *distinct* scopes whose output is
/// byte-identical, sorted.
///
/// Identical output under the *same* scope is an idempotent re-run, not a
/// defect; identical output under different scopes means two distinct
/// inputs gave the same numbers, which proves one of them did not reach
/// the computation.
pub fn identical_output_pairs(runs: &[RunOutput]) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    for i in 0..runs.len() {
        for j in (i + 1)..runs.len() {
            if runs[i].output == runs[j].output && runs[i].scope.line() != runs[j].scope.line() {
                let (a, b) = if runs[i].name <= runs[j].name {
                    (runs[i].name.as_str(), runs[j].name.as_str())
                } else {
                    (runs[j].name.as_str(), runs[i].name.as_str())
                };
                let pair = (a.to_string(), b.to_string());
                if !pairs.contains(&pair) {
                    pairs.push(pair);
                }
            }
        }
    }
    pairs.sort();
    pairs
}

/// The defect line for an identical-output pair.
pub fn identical_output_line(a: &str, b: &str) -> String {
    format!(
        "WARN: {a} and {b} produced byte-identical output: two distinct inputs giving the same numbers proves one input did not reach the computation — treat as a defect until proven otherwise"
    )
}

/// Invariant 4, as a check: the runs whose output does not print the
/// scope they ran under.
pub fn unreported_scope_runs(runs: &[RunOutput]) -> Vec<String> {
    runs.iter()
        .filter(|r| !r.scope.reported_in(&r.output))
        .map(|r| r.name.clone())
        .collect()
}

/// The per-project loop check combining invariants 3 and 4: the defect
/// lines for this set of runs. Empty means the loop is sound — distinct
/// runs printed distinct, scope-stamped output.
pub fn per_project_findings(runs: &[RunOutput]) -> Vec<String> {
    let mut findings = Vec::new();
    for (a, b) in identical_output_pairs(runs) {
        findings.push(identical_output_line(&a, &b));
    }
    for run in runs {
        if !run.scope.reported_in(&run.output) {
            findings.push(format!(
                "WARN: run '{}' did not report its scope ({}): a scoped tool prints its scope on every run",
                run.name,
                run.scope.line()
            ));
        }
    }
    findings
}
