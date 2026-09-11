//! Failure attribution: names come from the `failures:` block, never the
//! progress stream (issue #4331).
//!
//! The conversion pass extracted failing test ids with a pattern over the
//! libtest *progress* stream — `test NAME ... FAILED` — and compared that set
//! against the baseline. The progress stream is a rendering, not a report: it
//! is absent under `cargo test -q`, it is suppressed when a target's output is
//! captured and replayed, and a harness that prints progress to a tty-only
//! channel yields nothing. The `failures:` name list and the `test result:`
//! summary are the authoritative record the harness emits for its own verdict.
//! The extractor read the former's shadow and compared against neither: a run
//! whose failures appear only in the summary block produced an empty id set,
//! and an empty id set matches no baseline entry, so the gate had nothing to
//! call new and the run passed as "not attributed".
//!
//! Four invariants, each a primitive here:
//!
//! 1. **Names come from the `failures:` name list.** [`authoritative_names`]
//!    parses the indented name list under `failures:` (and the `---- NAME
//!    stdout ----` detail markers, which name the same tests). The progress
//!    stream is never consulted; [`progress_stream_names`] exists only to
//!    demonstrate the gap it leaves.
//! 2. **Declared and named must agree.** The `test result:` summary states
//!    `N failed` per target; [`attribute`] compares that count with the number
//!    of distinct names attributed for the same target. A shortfall — the run
//!    declares failures the extractor cannot name — is a hard finding
//!    ([`TargetVerdict::Shortfall`]), never a `WARN` about a test "listed
//!    twice": a duplicate inflates the *named* side, so a shortfall cannot be
//!    explained by one.
//! 3. **A surplus is a finding too.** More names than the summary declares
//!    means the parser captured something that is not a failing test (or read
//!    another target's block); silence about the mismatch is not an option.
//! 4. **A run attributed only from the progress stream is not verified.**
//!    Progress names are never authoritative, so a target whose only names
//!    come from `test NAME ... FAILED` lines has an *empty* authoritative set
//!    and fails invariant 2 as a [`TargetVerdict::Shortfall`];
//!    [`AttributionReport::source_warnings`] additionally says why, since the
//!    pre-fix extractor would have reported those names as a match.
//!
//! This module parses *names*; [`crate::verification::judge_test_run`] judges
//! the run. Both read the summary block, and neither accepts the absence of a
//! name as evidence of a pass.

use serde::{Deserialize, Serialize};

use crate::verification::parse_test_run;

/// Where a failing test name was found, in descending order of authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum FailureSource {
    /// The indented name list under the final `failures:` heading — the
    /// harness's own list of what failed.
    NameList,
    /// A `---- NAME stdout ----` detail marker: the harness captured output
    /// for this test because it failed.
    StdoutBlock,
    /// A `test NAME ... FAILED` progress line. A rendering, not a record.
    ProgressStream,
}

impl FailureSource {
    pub fn as_str(self) -> &'static str {
        match self {
            FailureSource::NameList => "failures-name-list",
            FailureSource::StdoutBlock => "stdout-detail-marker",
            FailureSource::ProgressStream => "progress-stream",
        }
    }

    /// True for the two channels the harness emits as its own record.
    pub fn is_authoritative(self) -> bool {
        !matches!(self, FailureSource::ProgressStream)
    }
}

/// One test target's slice of a run log, and what was attributed for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetRun {
    /// The target name as cargo prints it on the `Running` line (basename).
    pub target: String,
    /// Failures declared by this target's `test result:` summary line.
    pub declared_failed: u64,
    /// Whether this target reported a `test result:` line at all. Without one
    /// the target's verdict is unknown, never zero.
    pub has_result_line: bool,
    /// The names attributed to this target, from the most authoritative
    /// channel that supplied any.
    pub names: Vec<String>,
    /// Which channel supplied [`TargetRun::names`].
    pub source: Option<FailureSource>,
    /// Names found in each channel, for the report line.
    pub name_list: usize,
    pub stdout_block: usize,
    pub progress: usize,
}

impl TargetRun {
    /// The names that count as evidence: the harness's own, never the progress
    /// stream (invariant 1). A target sourced from the progress stream has an
    /// empty authoritative set, so it cannot satisfy invariant 2.
    pub fn authoritative(&self) -> &[String] {
        match self.source {
            Some(source) if source.is_authoritative() => &self.names,
            _ => &[],
        }
    }

    /// Invariants 2, 3 and the trivial case: does declared agree with named?
    pub fn verdict(&self) -> TargetVerdict {
        let named = self.authoritative().len() as u64;
        if !self.has_result_line {
            return TargetVerdict::NoResultLine { named };
        }
        if named > self.declared_failed {
            TargetVerdict::Surplus {
                declared: self.declared_failed,
                named,
            }
        } else if named < self.declared_failed {
            TargetVerdict::Shortfall {
                declared: self.declared_failed,
                named,
            }
        } else {
            TargetVerdict::Complete {
                declared: self.declared_failed,
                named,
            }
        }
    }
}

/// The verdict for one target (invariants 2 and 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TargetVerdict {
    /// Declared and named agree — including the `0 == 0` case, which is
    /// "nothing to attribute", not "attribution succeeded".
    Complete { declared: u64, named: u64 },
    /// The summary declares more failures than were named. The unnamed ones
    /// are invisible to any comparison built on names: hard finding.
    Shortfall { declared: u64, named: u64 },
    /// More names than declared: the parser captured a non-failure, or read
    /// across a target boundary.
    Surplus { declared: u64, named: u64 },
    /// No `test result:` line for this target, so there is nothing to check
    /// the names against. Never read as a pass (see [`crate::verification`]).
    NoResultLine { named: u64 },
}

impl TargetVerdict {
    pub fn is_finding(&self) -> bool {
        !matches!(self, TargetVerdict::Complete { .. })
    }

    pub fn line(&self, target: &str) -> String {
        match self {
            TargetVerdict::Complete { declared, named } => {
                format!("failures attributed: {named} of {declared} declared for `{target}`")
            }
            TargetVerdict::Shortfall { declared, named } => format!(
                "FAILURE_ATTRIBUTION: `{target}` declares {declared} failed test(s) but its \
                 `failures:` block names {named}; the {} unnamed cannot be compared against a \
                 baseline and must not be read as tolerated",
                declared - named
            ),
            TargetVerdict::Surplus { declared, named } => format!(
                "FAILURE_ATTRIBUTION: `{target}` names {named} failing test(s) but its summary \
                 declares {declared}; the extractor captured something the harness did not \
                 call a failure"
            ),
            TargetVerdict::NoResultLine { named } => format!(
                "FAILURE_ATTRIBUTION: `{target}` names {named} failing test(s) but reported no \
                 `test result:` line; the declared count is unknown, so nothing is verified"
            ),
        }
    }
}

/// Every failing test name the harness recorded in its own `failures:`
/// block, in first-seen order (invariant 1). Names found only in the progress
/// stream are not included.
pub fn authoritative_names(output: &str) -> Vec<String> {
    let mut names = Vec::new();
    for run in target_runs(output) {
        if run.source.is_some_and(|source| source.is_authoritative()) {
            names.extend(run.names);
        }
    }
    names
}

/// The pre-fix extractor: names from `test NAME ... FAILED` progress lines
/// only. Kept so the regression test can show the gap concretely — on a quiet
/// run this returns an empty set while the summary declares failures.
pub fn progress_stream_names(output: &str) -> Vec<String> {
    let mut names = Vec::new();
    for run in target_runs(output) {
        if run.source == Some(FailureSource::ProgressStream) {
            names.extend(run.names);
        }
    }
    names
}

/// Total failures declared by every `test result:` line in the run, reusing
/// the authoritative summary parser rather than a second implementation.
pub fn declared_failures(output: &str) -> u64 {
    parse_test_run(output).failed
}

/// Split a run log into per-target segments and attribute failures in each.
pub fn target_runs(output: &str) -> Vec<TargetRun> {
    let mut runs: Vec<TargetRun> = Vec::new();
    // Head segment: cargo's own preamble before the first `Running` line. It
    // carries no harness names, so it is only kept if it reports a summary.
    let mut current = TargetRun {
        target: "(preamble)".to_string(),
        declared_failed: 0,
        has_result_line: false,
        names: Vec::new(),
        source: None,
        name_list: 0,
        stdout_block: 0,
        progress: 0,
    };
    let mut name_list: Vec<String> = Vec::new();
    let mut stdout_block: Vec<String> = Vec::new();
    let mut progress: Vec<String> = Vec::new();
    let mut in_name_list = false;

    fn push_unique(hay: &mut Vec<String>, needle: &str) {
        if !hay.iter().any(|existing| existing == needle) {
            hay.push(needle.to_string());
        }
    }

    fn flush(
        run: &mut TargetRun,
        name_list: &mut Vec<String>,
        stdout_block: &mut Vec<String>,
        progress: &mut Vec<String>,
    ) {
        run.name_list = name_list.len();
        run.stdout_block = stdout_block.len();
        run.progress = progress.len();
        // Authority order: the name list, then detail markers, then progress.
        let (source, names) = if !name_list.is_empty() {
            (Some(FailureSource::NameList), name_list.clone())
        } else if !stdout_block.is_empty() {
            (Some(FailureSource::StdoutBlock), stdout_block.clone())
        } else if !progress.is_empty() {
            (Some(FailureSource::ProgressStream), progress.clone())
        } else {
            (None, Vec::new())
        };
        run.source = source;
        run.names = names;
        name_list.clear();
        stdout_block.clear();
        progress.clear();
    }

    for raw in output.lines() {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        let trimmed = line.trim();

        // Target boundary.
        if let Some(name) = segment_name(trimmed) {
            flush(
                &mut current,
                &mut name_list,
                &mut stdout_block,
                &mut progress,
            );
            let keep = current.has_result_line
                || current.name_list > 0
                || current.stdout_block > 0
                || current.progress > 0;
            if keep {
                runs.push(current.clone());
            }
            current = TargetRun {
                target: name,
                declared_failed: 0,
                has_result_line: false,
                names: Vec::new(),
                source: None,
                name_list: 0,
                stdout_block: 0,
                progress: 0,
            };
            in_name_list = false;
            continue;
        }

        // The authoritative name-list heading. The first `failures:` in a
        // verbose run is the detail header (followed by a blank line and the
        // `----` blocks); the last is the name list. Both open the state; the
        // blank line closes the first with nothing collected.
        if trimmed == "failures:" {
            in_name_list = true;
            continue;
        }

        // Detail markers name a failing test regardless of state.
        if let Some(name) = detail_marker_name(trimmed) {
            in_name_list = false;
            push_unique(&mut stdout_block, &name);
            continue;
        }

        if trimmed.starts_with("test result:") {
            in_name_list = false;
            current.has_result_line = true;
            current.declared_failed += parse_test_run(line).failed;
            continue;
        }

        if in_name_list {
            if trimmed.is_empty() {
                in_name_list = false;
                continue;
            }
            // A name-list entry is indented and is not itself a heading, a
            // cargo status line or an error.
            if line.starts_with(' ') || line.starts_with('\t') {
                push_unique(&mut name_list, trimmed);
                continue;
            }
            in_name_list = false;
        }

        if let Some(name) = progress_failed_name(trimmed) {
            push_unique(&mut progress, &name);
        }
    }
    flush(
        &mut current,
        &mut name_list,
        &mut stdout_block,
        &mut progress,
    );
    if current.has_result_line
        || current.name_list > 0
        || current.stdout_block > 0
        || current.progress > 0
    {
        runs.push(current);
    }
    runs
}

/// The cargo target-boundary line: `Running unittests src/lib.rs (deps/x-hex)`
/// or `Doc-tests my_crate`. Returns the target label.
fn segment_name(trimmed: &str) -> Option<String> {
    if let Some(rest) = trimmed.strip_prefix("Running ") {
        let rest = rest.strip_prefix("unittests ").unwrap_or(rest);
        // `src/lib.rs (target/debug/deps/name-hash)` -> `name`.
        if let Some(paren) = rest.find('(') {
            let inside = rest[paren + 1..].trim_end_matches(')');
            let basename = inside.rsplit('/').next().unwrap_or(inside);
            let stem = basename.split('-').next().unwrap_or(basename);
            if !stem.is_empty() {
                return Some(stem.to_string());
            }
        }
        return Some(basename_of(rest));
    }
    if let Some(rest) = trimmed.strip_prefix("Doc-tests ") {
        let name = rest.split_whitespace().next().unwrap_or(rest);
        if !name.is_empty() {
            return Some(format!("Doc-tests {name}"));
        }
    }
    None
}

fn basename_of(path: &str) -> String {
    let first = path.split_whitespace().next().unwrap_or(path);
    let last = first.rsplit('/').next().unwrap_or(first);
    last.trim_end_matches(".rs").to_string()
}

/// `---- NAME stdout ----` / `---- NAME stderr ----`. The name may contain
/// spaces (doc-tests carry paths and line numbers), so the marker is parsed
/// from both ends.
fn detail_marker_name(trimmed: &str) -> Option<String> {
    let rest = trimmed.strip_prefix("---- ")?;
    let rest = rest.strip_suffix(" ----")?;
    let name = rest
        .strip_suffix(" stdout")
        .or_else(|| rest.strip_suffix(" stderr"))?
        .trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// `test NAME ... FAILED` — the progress line. Retained for invariant 4's
/// warning and for [`progress_stream_names`], never for attribution.
fn progress_failed_name(trimmed: &str) -> Option<String> {
    let rest = trimmed.strip_prefix("test ")?;
    let name = rest.strip_suffix(" ... FAILED")?.trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// The attribution of a whole run log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttributionReport {
    pub targets: Vec<TargetRun>,
    /// Failures declared across every target's summary line.
    pub declared: u64,
    /// Distinct names attributed from authoritative channels.
    pub named: usize,
    /// Whether any `test result:` line was seen at all.
    pub harness_ran: bool,
}

impl AttributionReport {
    /// The name list every consumer should compare against a baseline.
    pub fn names(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for run in &self.targets {
            if run.source.is_some_and(|s| s.is_authoritative()) {
                for name in &run.names {
                    if !out.contains(name) {
                        out.push(name.clone());
                    }
                }
            }
        }
        out
    }

    /// Every target whose declared and named counts disagree (invariants 2
    /// and 3), plus the targets with no summary line to check against.
    pub fn findings(&self) -> Vec<String> {
        let mut out = Vec::new();
        for run in &self.targets {
            match run.verdict() {
                TargetVerdict::Complete { .. } => {}
                verdict => out.push(verdict.line(&run.target)),
            }
        }
        out
    }

    /// Invariant 4: names sourced from the progress stream while the summary
    /// declares failures means the harness block was never read. The progress
    /// names do not count, so the target is also a
    /// [`TargetVerdict::Shortfall`] in [`Self::findings`]; this line explains
    /// why names exist in the log and still amount to nothing.
    pub fn source_warnings(&self) -> Vec<String> {
        self.targets
            .iter()
            .filter(|run| {
                run.declared_failed > 0 && run.source == Some(FailureSource::ProgressStream)
            })
            .map(|run| {
                format!(
                    "WARN: `{}` named {} failure(s) from the progress stream only; the \
                     `failures:` name list is absent, so the harness record was not read and \
                     the run is not verified",
                    run.target,
                    run.names.len()
                )
            })
            .collect()
    }

    /// The gap the run leaves: declared failures that no name accounts for.
    pub fn shortfall(&self) -> u64 {
        self.targets
            .iter()
            .map(|run| match run.verdict() {
                TargetVerdict::Shortfall { declared, named } => declared - named,
                _ => 0,
            })
            .sum()
    }

    /// Attribution is complete when the harness ran, every declared failure is
    /// named by the harness's own block, and no target rested on the progress
    /// stream. Absence of a finding is not enough: a run whose names all came
    /// from progress lines has no findings to make.
    pub fn complete(&self) -> bool {
        self.harness_ran && self.findings().is_empty() && self.source_warnings().is_empty()
    }

    /// One line for the conversion pass log.
    pub fn line(&self) -> String {
        if !self.harness_ran {
            return "FAILURE_ATTRIBUTION: no `test result:` line in the run; nothing is \
                    attributed and nothing is tolerated"
                .to_string();
        }
        let source = self
            .targets
            .iter()
            .find_map(|run| run.source)
            .map(|s| s.as_str())
            .unwrap_or("none");
        format!(
            "failures attributed: {} of {} declared from {source} ({} {})",
            self.named,
            self.declared,
            self.targets.len(),
            if self.targets.len() == 1 {
                "target"
            } else {
                "targets"
            }
        )
    }
}

/// Attribute a captured cargo test log.
pub fn attribute(output: &str) -> AttributionReport {
    let targets = target_runs(output);
    let declared = targets.iter().map(|run| run.declared_failed).sum();
    let harness_ran = targets.iter().any(|run| run.has_result_line);
    let named = targets
        .iter()
        .filter(|run| run.source.is_some_and(|s| s.is_authoritative()))
        .map(|run| run.names.len())
        .sum();
    AttributionReport {
        targets,
        declared,
        named,
        harness_ran,
    }
}
