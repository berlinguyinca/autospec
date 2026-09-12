//! The conversion pass's hold reason: classification, names, provenance
//! (issue #3747).
//!
//! The incident, measured across a full session of conversion: 49 patches
//! held as `HELD: build error`, of which 38 were unambiguous test failures
//! carrying cargo's own rerun hint:
//!
//! ```text
//! autospec-2755 HELD: build error -- error: test failed, to rerun pass `-p autospec-core --test validation_runner`
//! ```
//!
//! The gate classified by grepping the combined output:
//!
//! ```sh
//! if printf '%s' "$out" | grep -qE '^error(\[|:)'; then
//!   log "HELD: build error -- ..."
//! ```
//!
//! `cargo` prints `error:` for several unrelated conditions: `error[E0308]`
//! and `error: could not compile` for a real compile failure, and
//! `error: test failed, to rerun pass ...` / `error: N targets failed` for
//! code that compiled perfectly. The build check ran before the test check,
//! so it won, and every test failure was labelled "build error" — 78% of
//! the largest hold category. The label is the only artefact a human or
//! agent reads when deciding what to do with a held patch, and "build
//! error" and "test failed" call for opposite responses: broken code is
//! re-dispatch material, a test disagreement may be the patch being right.
//!
//! The rule: **classify by structured outcome, not by scraping
//! human-readable output.** Cargo already answers this precisely:
//! `cargo build`'s exit status for "does it compile", `cargo test`'s exit
//! status for "do the tests pass". Where scraping is unavoidable, anchor on
//! what *distinguishes* the cases rather than on what they share: when two
//! conditions share a prefix, a prefix match picks the one you tested
//! first, silently, forever.
//!
//! Invariants, each encoded as a checkable primitive:
//!
//! 1. **The classification is the exit code**
//!    ([`failure_kind_from_exit_codes`]). A patch that compiles but fails
//!    tests classifies [`HoldKind::Tests`], never [`HoldKind::Compile`],
//!    whatever the output says — a test's stdout may print any line, and
//!    some genuine compile failures (an unclosed delimiter) match no text
//!    discriminator at all.
//! 2. **Any remaining text match is anchored on the discriminator**
//!    ([`is_compile_error_line`], [`is_test_failure_line`],
//!    [`classify_output`]): `^error(\[[A-Z]|: could not compile)` says a
//!    compile failure, `^error: (test failed|[0-9]+ targets? failed)` says
//!    the tests ran and failed. The two patterns are disjoint, so a match
//!    is a match; output matching neither classifies `None` — unclassified
//!    is recorded, not guessed.
//! 3. **A hold reached by any route carries the same evidence**
//!    ([`HoldReason`]): every test hold names the failing tests parsed
//!    from the output ([`failing_test_names`]) — a hold reached by the
//!    aggregate count cannot be unnamed, the way the incident's
//!    `HELD: 1 failing (-p autospec-cli) -- 2720 passed; 1 failed across
//!    43 targets` was — and a test hold with no names to parse says
//!    `unnamed` rather than staying silent about it.
//! 4. **The test gate compares failing-test names against the recorded
//!    baseline, not a count against zero** ([`TestBaseline`],
//!    [`compare_to_baseline`]): one pre-existing failure on `main` holds
//!    every patch in the backlog under a count compared to zero, and the
//!    hold says only "1 failing", indistinguishable from a real
//!    regression. Compared by name, a pre-existing failure is reported as
//!    pre-existing and a new failure is held by name. (This is the hold
//!    record for the per-scope gate; the full-suite flaky-aware
//!    re-verification policy is a different policy in
//!    [`crate::execution::test_diff`], where the diff is a trigger that
//!    never holds on its own.)
//! 5. **The recorded base sha is the sha the branch was actually created
//!    from** ([`BaseSha`]): the branch is cut from `origin/main` *after*
//!    the per-patch fetch, so a sha captured before the fetch is one merge
//!    behind and reads as fact. Capture it after the fetch, or drop it —
//!    a stale recorded sha is a finding ([`BaseSha::stale_finding`]).

use std::collections::BTreeSet;

use crate::execution::patch_pipeline::{classify_gate, GateVerdict};

/// What went wrong with a held patch, as the gate's structured outcome
/// says it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldKind {
    /// A target the patch touched does not build: broken code,
    /// re-dispatch material.
    Compile,
    /// The patch compiles; the tests ran and some failed. The patch may be
    /// coherent and disagreeing with a test — which may be the patch being
    /// right.
    Tests,
}

impl HoldKind {
    /// The label the hold line carries.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Compile => "build error",
            Self::Tests => "tests failed",
        }
    }
}

/// The structured classification: the exit codes decide, not the output.
///
/// `build_rc` is the exit status of the all-targets build gate and
/// `test_rc` the exit status of `cargo test`. A non-zero build status
/// wins over a non-zero test status: with the build red, the test status
/// answers nothing. `None` when both are zero — there is nothing to hold
/// on.
///
/// This delegates to the conversion pipeline's own gate classifier
/// ([`crate::execution::patch_pipeline::classify_gate`]): one
/// classification for one fact, two labelings.
pub fn failure_kind_from_exit_codes(build_rc: i32, test_rc: i32) -> Option<HoldKind> {
    Some(match classify_gate(build_rc, test_rc) {
        GateVerdict::Green => return None,
        GateVerdict::CompileFailure => HoldKind::Compile,
        GateVerdict::TestsFailed => HoldKind::Tests,
    })
}

/// Whether one line of cargo output is a genuine compile error:
/// `error[E0308]: ...` or `error: could not compile ...`.
///
/// Anchored on what distinguishes a compile failure from a test failure,
/// not on the shared `error` prefix: the test-failure lines start
/// `error: test failed` / `error: N targets failed`, which this pattern
/// does not match.
pub fn is_compile_error_line(line: &str) -> bool {
    let line = line.trim_start();
    if let Some(rest) = line.strip_prefix("error[") {
        rest.starts_with(|c: char| c.is_ascii_uppercase())
    } else {
        line.starts_with("error: could not compile")
    }
}

/// Whether one line of cargo output is a test-failure line:
/// `error: test failed, to rerun pass ...` (one failing target, printed in
/// both fail-fast and `--no-fail-fast` modes) or `error: N targets
/// failed` / `error: 1 target failed` (the `--no-fail-fast` summary).
///
/// The compile discriminators (`error[<code>`, `error: could not
/// compile`) do not match this pattern and vice versa: the two classes
/// share the `error` prefix and nothing else, so a match on either is a
/// match.
pub fn is_test_failure_line(line: &str) -> bool {
    let Some(rest) = line.trim_start().strip_prefix("error: ") else {
        return false;
    };
    if rest.starts_with("test failed") {
        return true;
    }
    let Some((count, remainder)) = rest.split_once(' ') else {
        return false;
    };
    !count.is_empty()
        && count.bytes().all(|b| b.is_ascii_digit())
        && (remainder.starts_with("target failed") || remainder.starts_with("targets failed"))
}

/// Classify a gate's output by text, for a gate that has no exit codes to
/// classify by.
///
/// The two anchored discriminators are disjoint, so the answer is
/// whichever matches. When both match (a capture spanning a rebuild and a
/// rerun), the compile evidence wins: a target that does not build is the
/// stronger fact, and it is the one that needs the re-dispatch response.
/// Output matching neither returns `None` — unclassified is recorded, not
/// guessed.
pub fn classify_output(output: &str) -> Option<HoldKind> {
    let mut compile = false;
    let mut tests = false;
    for line in output.lines() {
        if is_compile_error_line(line) {
            compile = true;
        }
        if is_test_failure_line(line) {
            tests = true;
        }
    }
    match (compile, tests) {
        (true, _) => Some(HoldKind::Compile),
        (false, true) => Some(HoldKind::Tests),
        (false, false) => None,
    }
}

/// The first discriminating compile line, verbatim — the evidence a
/// `build error` hold carries.
pub fn first_compile_error(output: &str) -> Option<&str> {
    output.lines().find(|line| is_compile_error_line(line))
}

/// The failing test names cargo reported, in output order, duplicates
/// collapsed.
///
/// Cargo prints the failing names in the `failures:` summary block — the
/// names indented four spaces under a `failures:` line, up to the next
/// blank line — after the per-test stdout section. The first `failures:`
/// line opens the stdout section and ends at the blank line that follows
/// it, so it contributes nothing; the stdout section itself is skipped,
/// because a test's stdout may indent anything. A `failures:` line inside
/// a test's stdout is the one thing this reader cannot tell apart from a
/// real block boundary — the same limitation the shell's `sed -n
/// '/^failures:$/,/^$/p' | grep -E '^\s{4}\S'` had.
pub fn failing_test_names(output: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let mut in_block = false;
    for line in output.lines() {
        if line.trim_end() == "failures:" {
            in_block = true;
            continue;
        }
        if !in_block {
            continue;
        }
        if line.trim().is_empty() {
            in_block = false;
            continue;
        }
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        if indent >= 4 {
            let name = trimmed.to_string();
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }
    names
}

/// The evidence marker a `build error` hold carries when the output
/// carried no discriminating compile line: the label is the exit code's,
/// and the record says the evidence was not found rather than staying
/// silent about it.
pub const NO_DISCRIMINATING_LINE: &str = "no discriminating compile line in output";

/// How many failing tests a hold line names before it switches to a
/// count: three names plus a count is actionable, thirty names is a log.
pub const NAMED_FAILURE_LIMIT: usize = 3;

/// A hold the conversion pass records for one patch: why it is held, and
/// the evidence the record carries.
///
/// The kind comes from the gate's structured outcome (the exit codes);
/// the output only supplies the evidence — the discriminating line for a
/// compile hold, the failing test names for a test hold. A hold reached
/// by the names parse and a hold reached by the aggregate count are the
/// same variant: the evidence recorded does not depend on which route
/// produced the verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HoldReason {
    /// A target the patch touched does not build. `detail` is the first
    /// discriminating compile line, verbatim, or
    /// [`NO_DISCRIMINATING_LINE`] when the output carried none.
    BuildError { detail: String },
    /// The patch compiles; the tests ran and some failed. `named` is the
    /// failing tests parsed from the output (empty only when the output
    /// named none), `aggregate` the run's summary (`2720 passed; 1 failed
    /// across 43 targets`).
    TestsFailed {
        named: Vec<String>,
        aggregate: String,
    },
}

impl HoldReason {
    /// Build the hold for a patch whose gate produced the given outcome.
    ///
    /// `build_rc`/`test_rc` are the exit codes of the all-targets build
    /// gate and of `cargo test`; `output` the combined gate output
    /// (evidence, not classification); `aggregate` the test run's summary
    /// line. `None` when neither exit code is non-zero — there is nothing
    /// to hold on, and a hold with no failing gate is a mislabel waiting
    /// to happen.
    pub fn from_gate(build_rc: i32, test_rc: i32, output: &str, aggregate: &str) -> Option<Self> {
        let kind = failure_kind_from_exit_codes(build_rc, test_rc)?;
        Some(match kind {
            HoldKind::Compile => Self::BuildError {
                detail: first_compile_error(output)
                    .unwrap_or(NO_DISCRIMINATING_LINE)
                    .to_string(),
            },
            HoldKind::Tests => Self::TestsFailed {
                named: failing_test_names(output),
                aggregate: aggregate.to_string(),
            },
        })
    }

    /// The hold line. `scope` is the cargo scope the tests ran under
    /// (`-p autospec-cli`), carried by the test holds.
    pub fn line(&self, scope: &str) -> String {
        match self {
            Self::BuildError { detail } => format!("HELD: build error -- {detail}"),
            Self::TestsFailed { named, aggregate } => {
                format!(
                    "HELD: tests failed ({scope}) -- {} | {}",
                    named_clause(named),
                    aggregate
                )
            }
        }
    }

    /// Whether this hold names at least one failing test. A test hold
    /// that cannot say which test failed has not recorded the evidence
    /// the run already had in hand.
    pub fn names_failures(&self) -> bool {
        matches!(self, Self::TestsFailed { named, .. } if !named.is_empty())
    }
}

/// Classify and build a hold reason from output alone — the legacy
/// scrape-only gate. The classification is the anchored text
/// discriminator ([`classify_output`]); `None` when the output matches
/// neither, which is recorded rather than guessed.
pub fn hold_reason_from_output(output: &str, aggregate: &str) -> Option<HoldReason> {
    Some(match classify_output(output)? {
        HoldKind::Compile => HoldReason::BuildError {
            detail: first_compile_error(output)
                .unwrap_or(NO_DISCRIMINATING_LINE)
                .to_string(),
        },
        HoldKind::Tests => HoldReason::TestsFailed {
            named: failing_test_names(output),
            aggregate: aggregate.to_string(),
        },
    })
}

/// The named clause of a test hold: up to [`NAMED_FAILURE_LIMIT`] names,
/// then a count of the rest — or `unnamed` when the output named none.
fn named_clause(named: &[String]) -> String {
    if named.is_empty() {
        return "unnamed".to_string();
    }
    let shown = &named[..named.len().min(NAMED_FAILURE_LIMIT)];
    let mut clause = shown.join(", ");
    if named.len() > NAMED_FAILURE_LIMIT {
        clause.push_str(&format!(" +{} more", named.len() - NAMED_FAILURE_LIMIT));
    }
    clause
}

/// The recorded baseline: the names of the tests failing on clean
/// `main`, captured by a baseline run of the same scope.
///
/// The test gate's question is not "how many tests failed" — a count
/// compared against zero holds every patch in the backlog on one
/// pre-existing failure, and the hold says only "1 failing",
/// indistinguishable from a real regression and expensive to disprove.
/// The question is "which tests failed, and did they fail before the
/// patch": a name set compared against the recorded baseline, the way the
/// validate gate compares against its baseline.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TestBaseline {
    failing: BTreeSet<String>,
}

impl TestBaseline {
    /// Record the baseline from a baseline run's failing test names.
    /// Input order is ignored and duplicates collapse.
    pub fn new(names: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            failing: names.into_iter().map(Into::into).collect(),
        }
    }

    /// Whether `name` fails on the baseline: a pre-existing failure,
    /// never charged to the patch.
    pub fn contains(&self, name: &str) -> bool {
        self.failing.contains(name)
    }

    /// How many tests fail on the baseline.
    pub fn len(&self) -> usize {
        self.failing.len()
    }

    /// Whether the baseline has no failing tests: a count-against-zero
    /// gate and this gate agree on an all-green baseline, and diverge
    /// everywhere else.
    pub fn is_empty(&self) -> bool {
        self.failing.is_empty()
    }

    /// The baseline's failing names, sorted.
    pub fn names(&self) -> &BTreeSet<String> {
        &self.failing
    }
}

/// The test gate's verdict: the current run's failing test names against
/// the recorded baseline, by name — not a count against zero.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BaselineVerdict {
    /// Every failing test is on the baseline (or nothing failed):
    /// pre-existing, not charged to the patch. Nothing to hold on.
    PreExisting { preexisting: Vec<String> },
    /// At least one failing test the baseline does not know: held, and
    /// the hold names it. `preexisting` is reported, never charged.
    NewFailures {
        new: Vec<String>,
        preexisting: Vec<String>,
    },
}

impl BaselineVerdict {
    /// Whether this verdict holds the patch.
    pub fn is_hold(&self) -> bool {
        matches!(self, Self::NewFailures { .. })
    }

    /// The failing names the verdict charges to the patch, sorted.
    pub fn new_failures(&self) -> &[String] {
        match self {
            Self::PreExisting { .. } => &[],
            Self::NewFailures { new, .. } => new,
        }
    }

    /// The hold line for a holding verdict: the new failures by name, the
    /// run's aggregate, and the pre-existing count — the evidence the run
    /// already had in hand, recorded on every route to the verdict.
    /// `None` for a non-holding verdict: there is nothing to hold.
    pub fn hold_line(&self, scope: &str, aggregate: &str) -> Option<String> {
        match self {
            Self::PreExisting { .. } => None,
            Self::NewFailures { new, preexisting } => {
                let mut line = format!(
                    "HELD: tests failed ({scope}) -- {} | {}",
                    named_clause(new),
                    aggregate
                );
                if !preexisting.is_empty() {
                    line.push_str(&format!(
                        " ({} pre-existing on baseline)",
                        preexisting.len()
                    ));
                }
                Some(line)
            }
        }
    }
}

/// Compare the current run's failing test names against the recorded
/// baseline. Input order is ignored and duplicates collapse: the
/// comparison is over the sets of names.
pub fn compare_to_baseline(
    failing: impl IntoIterator<Item = impl Into<String>>,
    baseline: &TestBaseline,
) -> BaselineVerdict {
    let failing: BTreeSet<String> = failing.into_iter().map(Into::into).collect();
    let mut new = Vec::new();
    let mut preexisting = Vec::new();
    for name in failing {
        if baseline.contains(&name) {
            preexisting.push(name);
        } else {
            new.push(name);
        }
    }
    if new.is_empty() {
        BaselineVerdict::PreExisting { preexisting }
    } else {
        BaselineVerdict::NewFailures { new, preexisting }
    }
}

/// The provenance of the base sha a log line records.
///
/// The branch is cut from `origin/main` *after* the per-patch fetch; the
/// sha a log line may carry is the one the branch was actually created
/// from. The incident captured `mainsha` before the fetch, so every line
/// read `[base=bfc62571]` while `origin/main` had advanced to `1d159336`
/// — harmless to the result, actively misleading to the reader, who spent
/// time treating the stale sha as fact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseSha {
    /// The sha the branch was actually created from (after the fetch).
    pub actual: String,
    /// The sha the log line recorded. `None` when the line recorded the
    /// real base or recorded no base at all (dropping it is the other
    /// permitted fix).
    pub recorded: Option<String>,
}

impl BaseSha {
    /// Record the provenance. The real base must be a nonempty sha: a log
    /// line that records a base records a real one, and "no base" is
    /// expressed by recording none, not by recording empty.
    pub fn new(actual: impl Into<String>, recorded: Option<String>) -> Result<Self, String> {
        let actual = actual.into();
        if actual.trim().is_empty() {
            return Err(
                "the branch's base sha must be nonempty: a log line that records a base \
                 records a real sha; record none rather than empty"
                    .to_string(),
            );
        }
        Ok(Self { actual, recorded })
    }

    /// Whether the recorded sha is stale: present, and not the sha the
    /// branch was created from. A sha captured before the per-patch fetch
    /// is one merge behind exactly this way.
    pub fn is_stale(&self) -> bool {
        self.recorded
            .as_deref()
            .is_some_and(|recorded| recorded != self.actual)
    }

    /// The token the log line carries: `[base=<actual>]`. The line
    /// carries the sha the branch was created from — never the pre-fetch
    /// snapshot.
    pub fn log_token(&self) -> String {
        format!("[base={}]", self.actual)
    }

    /// The finding for a stale recorded sha: the record pointed the
    /// reader at a base one merge behind the branch's real one.
    pub fn stale_finding(&self) -> Option<String> {
        let recorded = self.recorded.as_deref()?;
        (recorded != self.actual).then(|| {
            format!(
                "STALE_BASE_SHA: log line records base={recorded} but the branch was created \
                 from {}; capture the base after the per-patch fetch, or drop it",
                self.actual
            )
        })
    }
}
