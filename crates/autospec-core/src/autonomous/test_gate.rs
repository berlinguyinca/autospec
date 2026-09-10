//! Gate attribution for full-suite test failures.
//!
//! A gate that runs the full test suite on a patch must attribute a failure
//! *before* it blocks on one. A single failure out of hundreds, in code the
//! patch does not touch, is evidence about the suite, not about the patch.
//! The three signals, in order of cost:
//!
//! 1. **Blast radius** — the set of crates a patch could affect is computable
//!    from the files it touches and the reverse-dependency closure. Failures
//!    outside that set are reported as *unattributable*, not as failures of
//!    the change.
//! 2. **One retry** — a failing test is re-run in isolation before the
//!    verdict is recorded. A test that passes on re-run is reported as
//!    *flaky*, and both outcomes are kept. Flaky outcomes accumulate in a
//!    durable ledger so that a test that flakes repeatedly becomes visible
//!    as a defect.
//! 3. **The baseline** — was this test already failing on the untouched
//!    base? If so, the failure is *pre-existing*: the suite is broken, the
//!    patch did not break it.
//!
//! A hold names which of caused / pre-existing / flaky / unattributable it
//! is. A verdict that cannot say which is not a verdict: [`evaluate`] fails
//! closed when a failing test has no isolated re-run recorded, and every
//! reported failure carries exactly one [`FailureClass`].

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::Path;

use serde::Serialize;

use crate::autonomous::verdict_validity::{self, RecordedVerdict};

/// A test that flakes at least this many times is reported as a defect, not as noise.
pub const REPEATED_FLAKY_THRESHOLD: u64 = 3;

/// The class a gate assigns to one failing test.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FailureClass {
    /// The failing test lives in the patch's blast radius: the change plausibly caused it.
    Caused,
    /// The test was already failing on the untouched base (baseline).
    PreExisting,
    /// The test failed once and passed on the isolated re-run.
    Flaky,
    /// The failing test lives outside the patch's blast radius: a failure of the suite, not of the change.
    Unattributable,
}

impl FailureClass {
    /// The token the verdict prints: `caused`, `pre-existing`, `flaky`, `unattributable`.
    pub fn token(self) -> &'static str {
        match self {
            FailureClass::Caused => "caused",
            FailureClass::PreExisting => "pre-existing",
            FailureClass::Flaky => "flaky",
            FailureClass::Unattributable => "unattributable",
        }
    }
}

/// One failing test as the suite reported it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuiteFailure {
    /// Full test name as the runner reports it.
    pub name: String,
    /// The crate that owns the failing test, when it can be told. `None` means
    /// "cannot be told", which is *not* the same as "outside the blast radius":
    /// unknown components are classified fail-closed.
    pub component: Option<String>,
}

/// The outcome of one full-suite run on the patch.
#[derive(Debug, Clone, Default)]
pub struct SuiteOutcome {
    /// Tests that passed.
    pub passed: usize,
    /// Tests that failed, in suite order.
    pub failures: Vec<SuiteFailure>,
}

/// One failing test, classified. Both outcomes are kept: the suite run (always
/// failed — it is on the failure list) and the isolated re-run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureReport {
    pub name: String,
    pub component: Option<String>,
    pub class: FailureClass,
    /// The test failed in the suite run. Always `true` for a report.
    pub suite_failed: bool,
    /// The test's result on the isolated re-run.
    pub rerun_passed: bool,
}

/// The gate's decision. A hold names its attribution; a hold that cannot say
/// which of caused / pre-existing / unattributable it is cannot be constructed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateDecision {
    /// No failure of the change: nothing held.
    Pass,
    /// A failure that survived the isolated re-run held the patch. Flaky failures never hold.
    Hold {
        /// The strongest attributable class among the persistent failures: caused > pre-existing > unattributable.
        attribution: FailureClass,
    },
}

/// The recorded verdict: the decision plus every failure, classified, plus
/// the two validity conditions the decision was graded under (#4031).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateVerdict {
    pub decision: GateDecision,
    /// Tests that passed in the suite run.
    pub passed: usize,
    /// Every failing test from the suite, classified, in suite order.
    pub failures: Vec<FailureReport>,
    /// Tests that failed once and passed on re-run. Reported, not held.
    pub flaky: Vec<String>,
    /// Persistent failures outside the patch's blast radius: suite failures, not failures of the change.
    pub unattributable: Vec<String>,
    /// The commit of the tree the verdict was graded against. `None` means the
    /// runner could not read it, and a verdict persisted without it is
    /// unverifiable, never trusted ([`verdict_validity`]).
    pub tree_commit: Option<String>,
    /// [`verdict_validity::baseline_hash`] of the failing baseline used at
    /// grading time: the condition the decision was made under.
    pub baseline_hash: String,
}

impl GateVerdict {
    /// The one-line verdict. A hold always names its attribution:
    /// `HELD: tests failed -- caused | pre-existing | unattributable`.
    pub fn message(&self) -> String {
        match self.decision {
            GateDecision::Pass => format!("PASS: {} passed; {}", self.passed, self.flaky_summary()),
            GateDecision::Hold { attribution } => {
                format!("HELD: tests failed -- {}", attribution.token())
            }
        }
    }

    /// The decision token the runner records per patch: `pass`, or
    /// `new-test-failures` for any hold — the token the converter
    /// short-circuits on.
    pub fn verdict_token(&self) -> &'static str {
        match self.decision {
            GateDecision::Pass => "pass",
            GateDecision::Hold { .. } => "new-test-failures",
        }
    }

    /// The recorded form this verdict must be persisted in (#4031): the
    /// decision token plus the failing tests it named (the persistent
    /// failures — a flaky failure that passed on re-run is not a failure the
    /// verdict stands on) and the two validity conditions. `identity` is the
    /// patch the verdict is recorded for; `at` is Unix epoch seconds, audit
    /// only.
    pub fn recorded(&self, identity: &str, at: i64) -> RecordedVerdict {
        let failing_tests: BTreeSet<String> = self
            .failures
            .iter()
            .filter(|report| !report.rerun_passed)
            .map(|report| report.name.clone())
            .collect();
        RecordedVerdict {
            patch_identity: identity.to_string(),
            verdict: self.verdict_token().to_string(),
            failing_tests,
            tree_commit: self.tree_commit.clone(),
            baseline_hash: Some(self.baseline_hash.clone()),
            recorded_at: at,
        }
    }

    fn flaky_summary(&self) -> String {
        if self.flaky.is_empty() {
            "none flaky".to_string()
        } else {
            format!("{} flaky ({})", self.flaky.len(), self.flaky.join(", "))
        }
    }
}

/// Errors the gate can return. The gate fails closed: it would rather reject a
/// verdict it cannot fully attribute than record one that cannot say which.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateError {
    /// The test failed in the suite but no isolated re-run was recorded. A
    /// failing test is re-run in isolation *before* the verdict is recorded;
    /// without the re-run there is no verdict.
    MissingRerun {
        /// The failing test with no re-run.
        test: String,
    },
}

impl fmt::Display for GateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GateError::MissingRerun { test } => write!(
                f,
                "no isolated re-run recorded for failing test `{test}`; the gate re-runs every \
                 failing test in isolation before the verdict is recorded"
            ),
        }
    }
}

impl std::error::Error for GateError {}

/// The set of crates a patch could affect, computed from the files it touches
/// and the reverse-dependency closure.
///
/// - A file under `crates/<name>/` marks crate `<name>` (when `<name>` is a workspace crate).
/// - A root `Cargo.toml` or `Cargo.lock` marks every crate: the workspace manifest is a compiled input to all.
/// - Non-compiled files (`scripts/`, `docs/`, `skills/`, …) mark nothing: a patch that does not
///   change compiled code cannot break a compiled test by the dependency graph.
/// - Every marked crate pulls in every crate that depends on it, transitively.
pub fn affected_crates(
    touched: &[impl AsRef<str>],
    crates: &[String],
    depends_on: &BTreeMap<String, Vec<String>>,
) -> BTreeSet<String> {
    let mut seed = BTreeSet::new();
    for file in touched {
        let path = normalize_path(file.as_ref());
        if path == "Cargo.toml" || path == "Cargo.lock" {
            return crates.iter().cloned().collect();
        }
        if let Some(crate_name) = crate_for_path(&path, crates) {
            seed.insert(crate_name);
        }
    }
    reverse_closure(&seed, depends_on)
}

/// Classify every failing test and produce the verdict.
///
/// `reruns` maps each failing test name to its isolated re-run result
/// (true = passed). It must cover every failure: a failing test is
/// re-run in isolation before the verdict is recorded, so a missing entry
/// is [`GateError::MissingRerun`], not a guess.
///
/// `baseline` is the set of test names already failing on the untouched
/// base; an empty set means "no baseline data", and no failure is ever
/// claimed pre-existing on nothing.
///
/// `tree_commit` is the commit of the tree the suite ran on. The verdict
/// records it (and the hash of `baseline`) so a later reader can tell
/// whether the verdict was graded against the tree it is being applied to
/// (#4031); `None` means the commit could not be read, and the persisted
/// verdict is then unverifiable, never trusted.
///
/// Classification order per failure: flaky (passed on re-run) beats
/// everything — a test that passes on re-run is not a failure at all.
/// Then unattributable (known component outside the blast radius — the
/// patch cannot have caused it, whatever the baseline says). Then
/// pre-existing (in the baseline). Then caused — the fail-closed default,
/// including for failures whose component cannot be told.
pub fn evaluate(
    suite: &SuiteOutcome,
    reruns: &BTreeMap<String, bool>,
    baseline: &BTreeSet<String>,
    affected: &BTreeSet<String>,
    tree_commit: Option<&str>,
) -> Result<GateVerdict, GateError> {
    let mut reports: Vec<FailureReport> = Vec::with_capacity(suite.failures.len());
    for failure in &suite.failures {
        let rerun_passed = *reruns
            .get(&failure.name)
            .ok_or_else(|| GateError::MissingRerun {
                test: failure.name.clone(),
            })?;
        let class = classify(
            &failure.name,
            failure.component.as_deref(),
            rerun_passed,
            baseline,
            affected,
        );
        reports.push(FailureReport {
            name: failure.name.clone(),
            component: failure.component.clone(),
            class,
            suite_failed: true,
            rerun_passed,
        });
    }

    let decision = if reports.iter().any(|r| r.class == FailureClass::Caused) {
        GateDecision::Hold {
            attribution: FailureClass::Caused,
        }
    } else if reports.iter().any(|r| r.class == FailureClass::PreExisting) {
        GateDecision::Hold {
            attribution: FailureClass::PreExisting,
        }
    } else if reports
        .iter()
        .any(|r| r.class == FailureClass::Unattributable)
    {
        GateDecision::Hold {
            attribution: FailureClass::Unattributable,
        }
    } else {
        GateDecision::Pass
    };

    let flaky = reports
        .iter()
        .filter(|r| r.class == FailureClass::Flaky)
        .map(|r| r.name.clone())
        .collect();
    let unattributable = reports
        .iter()
        .filter(|r| r.class == FailureClass::Unattributable)
        .map(|r| r.name.clone())
        .collect();

    Ok(GateVerdict {
        decision,
        passed: suite.passed,
        failures: reports,
        flaky,
        unattributable,
        tree_commit: tree_commit.map(|commit| commit.to_string()),
        baseline_hash: verdict_validity::baseline_hash(baseline),
    })
}

fn classify(
    name: &str,
    component: Option<&str>,
    rerun_passed: bool,
    baseline: &BTreeSet<String>,
    affected: &BTreeSet<String>,
) -> FailureClass {
    if rerun_passed {
        return FailureClass::Flaky;
    }
    if let Some(component) = component {
        if !affected.contains(component) {
            return FailureClass::Unattributable;
        }
    }
    if baseline.contains(name) {
        return FailureClass::PreExisting;
    }
    FailureClass::Caused
}

fn normalize_path(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    let trimmed = normalized.strip_prefix("./").unwrap_or(&normalized);
    trimmed.trim_start_matches('/').to_string()
}

fn crate_for_path(path: &str, crates: &[String]) -> Option<String> {
    let mut segments = path.split('/');
    if segments.next()? != "crates" {
        return None;
    }
    let name = segments.next()?;
    crates.iter().find(|c| c.as_str() == name).cloned()
}

fn reverse_closure(
    seed: &BTreeSet<String>,
    depends_on: &BTreeMap<String, Vec<String>>,
) -> BTreeSet<String> {
    let mut out: BTreeSet<String> = seed.clone();
    let mut changed = true;
    while changed {
        changed = false;
        for (crate_name, deps) in depends_on {
            if !out.contains(crate_name) && deps.iter().any(|d| out.contains(d)) {
                out.insert(crate_name.clone());
                changed = true;
            }
        }
    }
    out
}

/// One flaky outcome written to the durable ledger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct FlakyLedgerEntry {
    /// The test that failed once and passed on re-run.
    pub test: String,
    /// The crate that owns the test, when it can be told.
    pub component: Option<String>,
    /// Unix seconds when the outcome was recorded.
    pub at: u64,
}

/// Append flaky outcomes to the durable ledger, one JSON object per line.
/// The ledger lives outside the patch's working tree so the record
/// survives the branch being discarded.
pub fn record_flaky_outcomes(path: &Path, entries: &[FlakyLedgerEntry]) -> io::Result<()> {
    if entries.is_empty() {
        return Ok(());
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    for entry in entries {
        let line = serde_json::to_string(entry).map_err(io::Error::other)?;
        writeln!(file, "{line}")?;
    }
    Ok(())
}

/// Read the durable ledger back as per-test flaky counts. A missing
/// ledger is an empty one; a malformed line is an error — silently
/// dropping a count would hide a defect.
pub fn read_flaky_ledger(path: &Path) -> io::Result<BTreeMap<String, u64>> {
    let contents = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(e) => return Err(e),
    };
    let mut counts: BTreeMap<String, u64> = BTreeMap::new();
    for (index, line) in contents.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let entry: FlakyLedgerEntry = serde_json::from_str(line).map_err(|e| {
            io::Error::other(format!("malformed flaky ledger line {}: {e}", index + 1))
        })?;
        *counts.entry(entry.test).or_insert(0) += 1;
    }
    Ok(counts)
}

/// Tests that flaked at least [`REPEATED_FLAKY_THRESHOLD`] times. A test
/// that flakes repeatedly is a defect, and this is where it becomes
/// visible.
pub fn repeated_flakes(counts: &BTreeMap<String, u64>) -> Vec<String> {
    counts
        .iter()
        .filter(|(_, count)| **count >= REPEATED_FLAKY_THRESHOLD)
        .map(|(test, _)| test.clone())
        .collect()
}
