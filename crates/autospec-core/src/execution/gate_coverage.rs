//! Gate coverage (#3804): grade a patch by what its gates can prove, not by
//! how many are green.
//!
//! A cluster job reported `test_passed=506 test_failed=0 fmt_rc=0 clippy_rc=0`
//! for a patch whose feature could not complete a single health check against
//! any real PostgreSQL — the startup packet encoded the protocol version wrong,
//! so the server always answered `ErrorResponse`. The one test that would have
//! caught it is `#[ignore]`d and needs a database, and the worker had none.
//!
//! 506 green tests over that patch is not weak evidence. It is *irrelevant*
//! evidence: no count of assertions against constructed byte sequences can say
//! anything about behaviour only an external system can demonstrate. Counting
//! green gates measures effort, not coverage, and the count grows most
//! reassuringly exactly where the mocks are most elaborate.
//!
//! Three rules, each a pure, testable primitive here:
//!
//! 1. **A patch declares the capabilities its verification requires.**
//!    [`parse_ignored_tests`] reads that declaration straight out of the source:
//!    `#[ignore]` is already machine-readable text saying "this test needs
//!    something this environment does not have". A patch that adds ignored
//!    tests and runs none of them is unverified in the dimension it just
//!    declared to matter.
//! 2. **Unrun is a distinct outcome, and it dominates.** [`GateOutcome::NotRun`]
//!    is neither passed nor failed and carries no evidence either way, so
//!    [`grade_patch`] never derives the verdict from the gates that happened to
//!    be runnable. A declared gate that did not run, or a required capability
//!    that was absent, yields [`Verdict::UnverifiedCapability`] or
//!    [`Verdict::UnverifiedGateSubset`] whatever the other counters say.
//! 3. **The status field says what was not measured.** [`PatchGrade`] is the
//!    per-patch artifact record: required vs available capabilities, declared
//!    vs executed gates, ignored tests declared vs executed. Its
//!    [`PatchGrade::status_field`] never reads as a pass when coverage is
//!    partial — `VERIFIED` is reserved for a full declared gate set, all green,
//!    with every required capability present.
//!
//! Verdict dominance is deliberate: a gate that *ran and failed* says the patch
//! is broken, which is more actionable than "unknown"; anything outranked is
//! still recorded in [`PatchGrade::verdict_detail`] so no reason is lost to the
//! higher-priority label.

use serde::Serialize;
use std::collections::BTreeSet;
use std::fmt;

/// A gate set or grade input that cannot form a valid coverage record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoverageError {
    /// A gate, capability or patch identity was blank.
    EmptyName,
    /// The same gate name appears twice in one declared gate set: coverage
    /// arithmetic (`run / declared`) would count it twice.
    DuplicateGate(String),
}

impl fmt::Display for CoverageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyName => write!(f, "gate, capability and patch names must not be empty"),
            Self::DuplicateGate(name) => write!(
                f,
                "gate {name} declared more than once in the same gate set"
            ),
        }
    }
}

impl std::error::Error for CoverageError {}

/// An external dependency a test needs to say anything at all: a database, a
/// network, a container runtime.
///
/// Lowercase slug form (`[a-z0-9._-]+`) so the same capability read from an
/// `#[ignore]` reason and from an environment probe compare equal.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct Capability(String);

impl Capability {
    /// Capability names recognised inside an `#[ignore]` reason, matched as
    /// whole tokens.
    pub const VOCABULARY: [&'static str; 7] = [
        "postgres", "mysql", "sqlite", "redis", "network", "docker", "gpu",
    ];

    /// The name used when a declaration gives no capability: still an absent
    /// capability, just an unattributed one.
    pub const UNSPECIFIED: &'static str = "unspecified";

    /// A validated capability name.
    pub fn new(name: &str) -> Result<Self, CoverageError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(CoverageError::EmptyName);
        }
        Ok(Self(
            name.to_ascii_lowercase()
                .chars()
                .map(|c| if c.is_whitespace() { '-' } else { c })
                .collect(),
        ))
    }

    /// The name as written on the artifact.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// A capability we know is required but cannot attribute, e.g. a bare
    /// `#[ignore]` with no reason.
    pub fn unspecified() -> Self {
        Self(Capability::UNSPECIFIED.to_string())
    }

    /// Best-effort capability named by an `#[ignore]` reason. Tokens are split
    /// on non-alphanumerics and compared for equality against
    /// [`Capability::VOCABULARY`], so `"needs a live postgres"` names `postgres`
    /// and `"flaky"` names nothing. An unattributed declaration is still an
    /// absent capability ([`Capability::unspecified`]), which keeps the grading
    /// rule independent of this inference.
    pub fn from_ignore_reason(reason: &str) -> Self {
        reason
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|token| !token.is_empty())
            .map(|token| token.to_ascii_lowercase())
            .find(|token| Self::VOCABULARY.contains(&token.as_str()))
            .map(Self)
            .unwrap_or_else(Self::unspecified)
    }
}

/// What one gate of the declared gate set actually did.
///
/// [`GateOutcome::NotRun`] is a first-class outcome: the gate produced no
/// evidence about anything, so it must not be counted toward a pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum GateOutcome {
    /// The gate ran and reported success.
    Passed,
    /// The gate ran and reported failure — evidence that the patch is broken.
    Failed,
    /// The gate did not run. No evidence in either direction.
    NotRun,
}

impl GateOutcome {
    /// The artifact spelling: `passed`, `failed`, `not-run`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::NotRun => "not-run",
        }
    }

    /// True when the gate produced evidence a verdict may rest on.
    pub fn is_evidence(self) -> bool {
        !matches!(self, Self::NotRun)
    }
}

/// One gate of the declared gate set with the outcome observed for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Gate {
    /// Gate name, e.g. `build`, `test`, `fmt`, `clippy`, `test-ignored`.
    pub name: String,
    /// What happened to it in this pass.
    pub outcome: GateOutcome,
}

impl Gate {
    /// A gate that did not run — the shape a capability gap leaves behind.
    pub fn not_run(name: &str) -> Result<Self, CoverageError> {
        if name.trim().is_empty() {
            return Err(CoverageError::EmptyName);
        }
        Ok(Self {
            name: name.trim().to_string(),
            outcome: GateOutcome::NotRun,
        })
    }
}

/// The declared gate set for a patch, with the outcome each gate produced.
///
/// The *declared* set is the repository's, not whatever the worker managed to
/// start: coverage is measured against it, so a pass that skipped a gate shows
/// up as a strict subset instead of a smaller denominator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GateExecution {
    gates: Vec<Gate>,
}

impl GateExecution {
    /// A non-empty gate set with unique names. Duplicates are rejected: they
    /// would inflate `declared` and let a skipped gate hide in the ratio.
    pub fn new(gates: Vec<Gate>) -> Result<Self, CoverageError> {
        if gates.is_empty() {
            return Err(CoverageError::EmptyName);
        }
        let mut seen = BTreeSet::new();
        for gate in &gates {
            if gate.name.trim().is_empty() {
                return Err(CoverageError::EmptyName);
            }
            if !seen.insert(gate.name.as_str()) {
                return Err(CoverageError::DuplicateGate(gate.name.clone()));
            }
        }
        Ok(Self { gates })
    }

    /// Gates the repository declares for this patch.
    pub fn declared(&self) -> usize {
        self.gates.len()
    }

    /// Gates that actually ran (passed or failed).
    pub fn run(&self) -> usize {
        self.gates
            .iter()
            .filter(|gate| gate.outcome.is_evidence())
            .count()
    }

    /// Gates that ran and failed.
    pub fn failed(&self) -> usize {
        self.gates
            .iter()
            .filter(|gate| gate.outcome == GateOutcome::Failed)
            .count()
    }

    /// Names of the gates that produced no evidence.
    pub fn unrun(&self) -> Vec<String> {
        self.gates
            .iter()
            .filter(|gate| !gate.outcome.is_evidence())
            .map(|gate| gate.name.clone())
            .collect()
    }

    /// True when the pass executed a strict subset of the declared gate set.
    pub fn is_strict_subset(&self) -> bool {
        self.run() < self.declared()
    }

    /// True when every declared gate ran and none failed.
    pub fn all_green(&self) -> bool {
        self.gates
            .iter()
            .all(|gate| gate.outcome == GateOutcome::Passed)
    }
}

/// A test the patch declares with `#[ignore]`, and whether this pass ran it.
///
/// The declaration is the point: an ignored test is a machine-readable
/// statement that a capability is required and was absent where the test was
/// written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IgnoredTest {
    /// Test function name.
    pub name: String,
    /// Capability its `#[ignore]` reason names, else
    /// [`Capability::unspecified`].
    pub capability: Capability,
    /// True when the pass executed it (`cargo test -- --ignored`).
    pub executed: bool,
}

/// Read the `#[ignore]` declarations out of Rust source — typically the added
/// lines of a patch. Handles `#[ignore]` and `#[ignore = "reason"]` over the
/// usual `#[test]` / `#[tokio::test]` / `async fn` shapes; `executed` starts
/// false because a normal pass does not run ignored tests.
pub fn parse_ignored_tests(source: &str) -> Vec<IgnoredTest> {
    let mut found = Vec::new();
    let mut pending: Option<Capability> = None;
    for line in source.lines() {
        let text = line.trim();
        if let Some(reason) = ignore_attribute(text) {
            pending = Some(Capability::from_ignore_reason(reason));
            continue;
        }
        let Some(capability) = pending.clone() else {
            continue;
        };
        if let Some(name) = function_name(text) {
            found.push(IgnoredTest {
                name,
                capability,
                executed: false,
            });
            pending = None;
        } else if text.is_empty() || text.starts_with('#') || text.starts_with(['{', '}']) {
            // Still between the attribute and its function; keep it pending.
            continue;
        } else {
            // Some other item sits here, so the attribute does not decorate a
            // nameable test. Drop it rather than attribute the next function.
            pending = None;
        }
    }
    found
}

/// The reason carried by an `#[ignore]` attribute line, or `None` when the
/// line is not one. A bare `#[ignore]` carries the empty reason.
fn ignore_attribute(text: &str) -> Option<&str> {
    let rest = text.strip_prefix("#[ignore")?;
    if let Some(open) = rest.find('"') {
        return rest[open + 1..]
            .find('"')
            .map(|close| &rest[open + 1..open + 1 + close]);
    }
    rest.trim_start().starts_with(']').then_some("")
}

/// The function name declared by a line such as `pub async fn foo() {`.
fn function_name(text: &str) -> Option<String> {
    const QUALIFIERS: [&str; 6] = ["pub", "async", "const", "unsafe", "default", "extern"];
    let mut tokens = text.split_whitespace().peekable();
    while let Some(token) = tokens.peek() {
        if !QUALIFIERS.contains(token) {
            break;
        }
        tokens.next();
    }
    if tokens.next() != Some("fn") {
        return None;
    }
    let raw = tokens.next()?;
    let name = raw
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .next()
        .unwrap_or_default();
    (!name.is_empty()).then(|| name.to_string())
}

/// Everything one pass observed about one patch, as inputs to
/// [`grade_patch`].
#[derive(Debug, Clone)]
pub struct GradeInput<'a> {
    /// Identity of the graded patch (must be non-empty).
    pub patch: &'a str,
    /// The declared gate set with the outcome each gate produced.
    pub gates: &'a GateExecution,
    /// Tests the patch declares with `#[ignore]`.
    pub ignored: &'a [IgnoredTest],
    /// Capabilities the patch's tests require beyond its ignored tests, e.g.
    /// a live suite listed in the repository's gate contract.
    pub required_capabilities: &'a [Capability],
    /// Capabilities the verification environment actually provided.
    pub available_capabilities: &'a [Capability],
    /// Passing tests counted by the runnable gates.
    pub tests_passed: usize,
    /// Failing tests counted by the runnable gates.
    pub tests_failed: usize,
}

/// How a patch may be described to the next reader of the artifact.
///
/// Only [`Verdict::Verified`] reads as a pass, and it requires full coverage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Verdict {
    /// Every declared gate ran green and every required capability was there.
    Verified,
    /// Something ran and reported breakage.
    Failed,
    /// A capability the patch's own tests require was absent, or ignored tests
    /// for its core behaviour did not run. The dimension that matters was not
    /// measured.
    UnverifiedCapability,
    /// The verdict rests on a strict subset of the declared gate set with no
    /// single named capability to blame (the gate itself never started).
    UnverifiedGateSubset,
}

impl Verdict {
    /// The artifact spelling, e.g. `UNVERIFIED-CAPABILITY`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Verified => "VERIFIED",
            Self::Failed => "FAILED",
            Self::UnverifiedCapability => "UNVERIFIED-CAPABILITY",
            Self::UnverifiedGateSubset => "UNVERIFIED-GATE-SUBSET",
        }
    }

    /// True only for [`Verdict::Verified`]. Anything else must never be
    /// summarised as a pass (#3804).
    pub fn reads_as_pass(self) -> bool {
        self == Self::Verified
    }
}

/// The per-patch grading record: what the gates were, what ran, what the tests
/// required and what the environment had.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PatchGrade {
    /// Identity of the graded patch.
    pub patch: String,
    /// The dominant verdict.
    pub verdict: Verdict,
    /// Every coverage finding behind the verdict, including those outranked by
    /// a higher-priority one.
    pub verdict_detail: Vec<String>,
    /// Gates the repository declares.
    pub gates_declared: usize,
    /// Gates that actually ran.
    pub gates_run: usize,
    /// Declared gates that produced no evidence.
    pub gates_unrun: Vec<String>,
    /// Capabilities the patch's tests require.
    pub capabilities_required: Vec<String>,
    /// Capabilities the verification environment provided.
    pub capabilities_available: Vec<String>,
    /// Required capabilities the environment did not provide.
    pub capabilities_missing: Vec<String>,
    /// Tests declared with `#[ignore]`.
    pub ignored_declared: usize,
    /// Of those, how many the pass executed.
    pub ignored_executed: usize,
    /// Passing tests counted by the runnable gates (effort, not coverage).
    pub tests_passed: usize,
    /// Failing tests counted by the runnable gates.
    pub tests_failed: usize,
}

impl PatchGrade {
    /// One line for the artifact status field: the verdict plus the coverage
    /// it rests on, so a partial pass cannot be read as a pass even by a
    /// reader who never opens the detail.
    pub fn status_field(&self) -> String {
        let mut line = format!(
            "{} gates={}/{}",
            self.verdict.as_str(),
            self.gates_run,
            self.gates_declared
        );
        if !self.capabilities_missing.is_empty() {
            line.push_str(&format!(
                " capabilities-missing={}",
                self.capabilities_missing.join(",")
            ));
        }
        line
    }

    /// The `key=value` block a `status.txt` artifact carries for this patch.
    /// Lowercase keys, one per line, verdict first, so a reader grepping the
    /// artifact sees coverage and not only counters.
    pub fn status_lines(&self) -> String {
        let gates = format!("{}/{}", self.gates_run, self.gates_declared);
        let detail = self.verdict_detail.join("; ");
        let required = self.capabilities_required.join(",");
        let available = self.capabilities_available.join(",");
        let missing = self.capabilities_missing.join(",");
        let unrun = self.gates_unrun.join(",");
        [
            format!("verdict={}", self.verdict.as_str()),
            format!("verdict_detail={detail}"),
            format!("gates={gates}"),
            format!("gates_unrun={unrun}"),
            format!("capabilities_required={required}"),
            format!("capabilities_available={available}"),
            format!("capabilities_missing={missing}"),
            format!("ignored_declared={}", self.ignored_declared),
            format!("ignored_executed={}", self.ignored_executed),
            format!("tests_passed={}", self.tests_passed),
            format!("tests_failed={}", self.tests_failed),
        ]
        .join("\n")
    }

    /// The artifact record as JSON.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_string())
    }
}

/// Grade one patch against its declared gate set.
///
/// Dominance order (first match wins, all findings recorded):
/// a gate or test that ran and **failed**; then a **required capability
/// missing** or **ignored tests that did not run** (the patch's own
/// declaration that its core dimension was not measured); then a strict
/// **subset** of the declared gates; only then `VERIFIED`. Test counters are
/// reported, never consulted for the pass/fail line beyond "did anything
/// fail": 506 passing tests cannot buy a verdict about a dimension they did
/// not exercise (#3804).
pub fn grade_patch(input: GradeInput<'_>) -> Result<PatchGrade, CoverageError> {
    if input.patch.trim().is_empty() {
        return Err(CoverageError::EmptyName);
    }
    let required = required_capabilities(&input);
    let available: BTreeSet<String> = input
        .available_capabilities
        .iter()
        .map(|capability| capability.as_str().to_string())
        .collect();
    let missing: Vec<String> = required
        .iter()
        .filter(|capability| !available.contains(capability.as_str()))
        .map(|capability| capability.as_str().to_string())
        .collect();
    let unexecuted: Vec<&IgnoredTest> =
        input.ignored.iter().filter(|test| !test.executed).collect();

    let mut detail = Vec::new();
    let mut verdict = Verdict::Verified;
    if input.gates.failed() > 0 || input.tests_failed > 0 {
        verdict = Verdict::Failed;
        detail.push(format!(
            "gates_failed={} tests_failed={}",
            input.gates.failed(),
            input.tests_failed
        ));
    }
    if !missing.is_empty() {
        if verdict == Verdict::Verified {
            verdict = Verdict::UnverifiedCapability;
        }
        detail.push(format!("capabilities_missing={}", missing.join(",")));
    }
    if !unexecuted.is_empty() {
        if verdict == Verdict::Verified {
            verdict = Verdict::UnverifiedCapability;
        }
        detail.push(format!(
            "ignored_tests_not_run={}/{}",
            unexecuted.len(),
            input.ignored.len()
        ));
    }
    if input.gates.is_strict_subset() {
        if verdict == Verdict::Verified {
            verdict = Verdict::UnverifiedGateSubset;
        }
        // A strict subset always leaves at least one gate unrun, so the
        // finding names the gates that produced no evidence.
        detail.push(format!("gates_unrun={}", input.gates.unrun().join(",")));
    }

    Ok(PatchGrade {
        patch: input.patch.trim().to_string(),
        verdict,
        verdict_detail: detail,
        gates_declared: input.gates.declared(),
        gates_run: input.gates.run(),
        gates_unrun: input.gates.unrun(),
        capabilities_required: required
            .iter()
            .map(|capability| capability.as_str().to_string())
            .collect(),
        capabilities_available: available.into_iter().collect(),
        capabilities_missing: missing,
        ignored_declared: input.ignored.len(),
        ignored_executed: input.ignored.len() - unexecuted.len(),
        tests_passed: input.tests_passed,
        tests_failed: input.tests_failed,
    })
}

/// Capabilities the patch requires: those declared beyond its ignored tests,
/// plus those named by every ignored test.
fn required_capabilities(input: &GradeInput<'_>) -> BTreeSet<Capability> {
    let mut required: BTreeSet<Capability> = input.required_capabilities.iter().cloned().collect();
    for test in input.ignored {
        required.insert(test.capability.clone());
    }
    required
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gate(name: &str, outcome: GateOutcome) -> Gate {
        Gate {
            name: name.to_string(),
            outcome,
        }
    }

    /// The declared gate set of the repository that produced #3804.
    fn declared_gates() -> Vec<Gate> {
        ["build", "test", "fmt", "clippy", "test-ignored"]
            .into_iter()
            .map(|name| gate(name, GateOutcome::Passed))
            .collect()
    }

    /// The same gate set as executed by the worker: `test-ignored` never ran.
    fn worker_gate_set() -> GateExecution {
        let mut gates = declared_gates();
        gates[4].outcome = GateOutcome::NotRun;
        GateExecution::new(gates).expect("distinct gate names")
    }

    /// The test the patch added to declare the behaviour it cared about.
    fn health_check() -> IgnoredTest {
        IgnoredTest {
            name: "health_check_sees_live_postgres".to_string(),
            capability: Capability::new("postgres").expect("valid slug"),
            executed: false,
        }
    }

    fn grade_with(gates: &GateExecution, ignored: &[IgnoredTest]) -> PatchGrade {
        grade_patch(GradeInput {
            patch: "patch-42",
            gates,
            ignored,
            required_capabilities: &[],
            available_capabilities: &[],
            tests_passed: 506,
            tests_failed: 0,
        })
        .expect("patch identity present")
    }

    #[test]
    fn not_run_is_a_distinct_outcome_from_passed_and_failed() {
        assert_ne!(GateOutcome::NotRun, GateOutcome::Passed);
        assert_ne!(GateOutcome::NotRun, GateOutcome::Failed);
        assert_eq!(GateOutcome::NotRun.as_str(), "not-run");
    }

    #[test]
    fn not_run_carries_no_evidence() {
        assert!(!GateOutcome::NotRun.is_evidence());
        assert!(GateOutcome::Passed.is_evidence());
        assert!(GateOutcome::Failed.is_evidence());
    }

    #[test]
    fn ac1_unexecuted_ignored_tests_dominate_green_counters() {
        // The #3804 case: 506 passed, 0 failed, fmt and clippy clean, and the
        // one test for the shipped behaviour did not run.
        let grade = grade_with(&worker_gate_set(), &[health_check()]);
        assert_eq!(grade.verdict, Verdict::UnverifiedCapability);
        assert!(!grade.verdict.reads_as_pass());
        assert_eq!(grade.tests_passed, 506);
        assert_eq!(grade.ignored_declared, 1);
        assert_eq!(grade.ignored_executed, 0);
    }

    #[test]
    fn ac1_unverified_when_the_ignored_suite_was_skipped_by_choice() {
        // The capability was there; the pass still did not exercise it.
        let gates = GateExecution::new(declared_gates()).expect("distinct gate names");
        let grade = grade_patch(GradeInput {
            patch: "patch-42",
            gates: &gates,
            ignored: &[health_check()],
            required_capabilities: &[],
            available_capabilities: &[Capability::new("postgres").expect("valid slug")],
            tests_passed: 506,
            tests_failed: 0,
        })
        .expect("patch identity present");
        assert_eq!(grade.verdict, Verdict::UnverifiedCapability);
        assert!(grade.capabilities_missing.is_empty());
    }

    #[test]
    fn ac3_status_field_names_the_gap_and_never_reads_as_a_pass() {
        let grade = grade_with(&worker_gate_set(), &[health_check()]);
        let status = grade.status_field();
        assert!(status.starts_with("UNVERIFIED-CAPABILITY"), "{status}");
        assert!(status.contains("gates=4/5"), "{status}");
        assert!(status.contains("capabilities-missing=postgres"), "{status}");
    }

    #[test]
    fn ac3_a_strict_subset_of_the_gate_set_says_so_in_the_status() {
        let mut gates = declared_gates();
        gates[3].outcome = GateOutcome::NotRun; // clippy never started
        let execution = GateExecution::new(gates).expect("distinct gate names");
        let grade = grade_with(&execution, &[]);
        assert_eq!(grade.verdict, Verdict::UnverifiedGateSubset);
        assert_eq!(grade.gates_unrun, vec!["clippy".to_string()]);
        assert_eq!(grade.status_field(), "UNVERIFIED-GATE-SUBSET gates=4/5");
    }

    #[test]
    fn full_coverage_and_green_gates_verify() {
        let gates = GateExecution::new(declared_gates()).expect("distinct gate names");
        let grade = grade_with(&gates, &[]);
        assert_eq!(grade.verdict, Verdict::Verified);
        assert!(grade.verdict.reads_as_pass());
        assert_eq!(grade.status_field(), "VERIFIED gates=5/5");
    }

    #[test]
    fn declared_capability_absent_from_the_environment_is_unverified() {
        let gates = GateExecution::new(declared_gates()).expect("distinct gate names");
        let grade = grade_patch(GradeInput {
            patch: "patch-7",
            gates: &gates,
            ignored: &[],
            required_capabilities: &[Capability::new("postgres").expect("valid slug")],
            available_capabilities: &[Capability::new("network").expect("valid slug")],
            tests_passed: 12,
            tests_failed: 0,
        })
        .expect("patch identity present");
        assert_eq!(grade.verdict, Verdict::UnverifiedCapability);
        assert_eq!(grade.capabilities_missing, vec!["postgres".to_string()]);
    }

    #[test]
    fn ac2_record_lists_required_available_and_missing_capabilities() {
        let gates = GateExecution::new(declared_gates()).expect("distinct gate names");
        let grade = grade_patch(GradeInput {
            patch: "patch-7",
            gates: &gates,
            ignored: &[health_check()],
            required_capabilities: &[Capability::new("network").expect("valid slug")],
            available_capabilities: &[Capability::new("network").expect("valid slug")],
            tests_passed: 12,
            tests_failed: 0,
        })
        .expect("patch identity present");
        assert_eq!(
            grade.capabilities_required,
            vec!["network".to_string(), "postgres".to_string()]
        );
        assert_eq!(grade.capabilities_available, vec!["network".to_string()]);
        assert_eq!(grade.capabilities_missing, vec!["postgres".to_string()]);
    }

    #[test]
    fn a_gate_that_ran_and_failed_outranks_the_unverified_findings() {
        let mut gates = declared_gates();
        gates[1].outcome = GateOutcome::Failed;
        let execution = GateExecution::new(gates).expect("distinct gate names");
        let grade = grade_patch(GradeInput {
            patch: "patch-8",
            gates: &execution,
            ignored: &[health_check()],
            required_capabilities: &[],
            available_capabilities: &[],
            tests_passed: 500,
            tests_failed: 6,
        })
        .expect("patch identity present");
        assert_eq!(grade.verdict, Verdict::Failed);
        // The outranked coverage finding is still recorded, not swallowed.
        assert!(
            grade
                .verdict_detail
                .iter()
                .any(|line| line.contains("capabilities_missing=postgres")),
            "{:?}",
            grade.verdict_detail
        );
    }

    #[test]
    fn running_the_ignored_tests_restores_the_verdict() {
        let gates = GateExecution::new(declared_gates()).expect("distinct gate names");
        let mut health = health_check();
        health.executed = true;
        let grade = grade_patch(GradeInput {
            patch: "patch-9",
            gates: &gates,
            ignored: &[health],
            required_capabilities: &[],
            available_capabilities: &[Capability::new("postgres").expect("valid slug")],
            tests_passed: 507,
            tests_failed: 0,
        })
        .expect("patch identity present");
        assert_eq!(grade.verdict, Verdict::Verified);
        assert_eq!(grade.ignored_executed, 1);
    }

    #[test]
    fn ac1_parser_counts_ignored_tests_and_their_capabilities() {
        let source = "\
#[test]
fn builds_startup_packet_without_a_server() {}

#[ignore]
#[test]
fn needs_something_unnamed() {}

#[ignore = \"needs a live postgres\"]
#[tokio::test]
async fn health_check_sees_live_postgres() {}
";
        let ignored = parse_ignored_tests(source);
        assert_eq!(ignored.len(), 2, "the plain #[test] is not a declaration");
        assert_eq!(ignored[0].name, "needs_something_unnamed");
        assert_eq!(ignored[0].capability.as_str(), Capability::UNSPECIFIED);
        assert_eq!(ignored[1].name, "health_check_sees_live_postgres");
        assert_eq!(ignored[1].capability.as_str(), "postgres");
        assert!(
            ignored.iter().all(|test| !test.executed),
            "a normal pass does not run ignored tests"
        );
    }

    #[test]
    fn parser_does_not_attribute_an_ignore_to_a_non_function() {
        let ignored = parse_ignored_tests("#[ignore]\npub struct NotATest {\n    field: u8,\n}\n");
        assert!(ignored.is_empty(), "{ignored:?}");
    }

    #[test]
    fn capability_slugs_are_validated_and_normalised() {
        assert_eq!(Capability::new(""), Err(CoverageError::EmptyName));
        assert_eq!(Capability::new("   "), Err(CoverageError::EmptyName));
        assert_eq!(
            Capability::new(" Postgres ").expect("non-empty").as_str(),
            "postgres",
            "a reason and a probe must agree on one slug form"
        );
    }

    #[test]
    fn capability_inference_matches_whole_tokens_only() {
        assert_eq!(
            Capability::from_ignore_reason("needs a live Postgres").as_str(),
            "postgres"
        );
        assert_eq!(
            Capability::from_ignore_reason("postgress is not a capability").as_str(),
            Capability::UNSPECIFIED
        );
        assert_eq!(
            Capability::from_ignore_reason("").as_str(),
            Capability::UNSPECIFIED
        );
    }

    #[test]
    fn gate_sets_reject_empty_and_duplicate_names() {
        assert_eq!(GateExecution::new(vec![]), Err(CoverageError::EmptyName));
        assert_eq!(
            Gate::not_run(" "),
            Err(CoverageError::EmptyName),
            "an unnamed gate cannot be graded"
        );
        assert_eq!(
            GateExecution::new(vec![
                gate("test", GateOutcome::Passed),
                gate("test", GateOutcome::NotRun),
            ]),
            Err(CoverageError::DuplicateGate("test".to_string()))
        );
    }

    #[test]
    fn coverage_arithmetic_counts_only_the_gates_that_ran() {
        let execution = worker_gate_set();
        assert_eq!(execution.declared(), 5);
        assert_eq!(execution.run(), 4);
        assert_eq!(execution.failed(), 0);
        assert_eq!(execution.unrun(), vec!["test-ignored".to_string()]);
        assert!(execution.is_strict_subset());
        assert!(!execution.all_green());
    }

    #[test]
    fn grading_rejects_an_unidentified_patch() {
        let gates = GateExecution::new(declared_gates()).expect("distinct gate names");
        assert_eq!(
            grade_patch(GradeInput {
                patch: " ",
                gates: &gates,
                ignored: &[],
                required_capabilities: &[],
                available_capabilities: &[],
                tests_passed: 0,
                tests_failed: 0,
            }),
            Err(CoverageError::EmptyName)
        );
    }

    #[test]
    fn ac2_artifact_lines_and_json_carry_coverage_not_only_counters() {
        let grade = grade_with(&worker_gate_set(), &[health_check()]);
        let lines = grade.status_lines();
        for key in [
            "verdict=UNVERIFIED-CAPABILITY",
            "gates=4/5",
            "gates_unrun=test-ignored",
            "capabilities_required=postgres",
            "capabilities_missing=postgres",
            "ignored_declared=1",
            "ignored_executed=0",
        ] {
            assert!(lines.contains(key), "missing {key} in\n{lines}");
        }
        let json: serde_json::Value =
            serde_json::from_str(&grade.to_json()).expect("grade serialises");
        assert_eq!(json["verdict"], "unverified-capability");
        assert_eq!(json["gates_run"], 4);
        assert_eq!(json["capabilities_missing"][0], "postgres");
    }
}
