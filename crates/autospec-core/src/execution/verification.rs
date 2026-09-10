//! Agent verification status (#3786).
//!
//! An agent that finishes an implementation run records a status —
//! `status=VERIFIED clippy_rc=0 test_passed=384 test_failed=0 fmt_ok=true`.
//! The defect is that such a status can read as more than it is. The
//! enumeration says *how many* tests passed, not *which* ran: a suite gated
//! on a live database is simply absent from the record, and `VERIFIED`
//! reads as though it had run. Consumers act on the record without
//! re-running anything — an ordering that trusts the status ranks that
//! patch ahead of one that ran the full gate, and an admission decision
//! counts a check that never ran as though it had passed.
//!
//! The invariant this module encodes: **a VERIFIED status is an
//! enumeration, never a bare assertion.** The agent runs its checks and
//! records each one; this module decides what the record is entitled to
//! claim. Everything here is pure and testable: no I/O, no clock, no
//! subprocess.
//!
//! 1. **A bare `VERIFIED` is not a permitted value**
//!    ([`VerificationStatus::new`]). Every status enumerates the checks
//!    that ran and the checks that were skipped, each skip carrying its
//!    reason ([`SkipReason`]). A status with no enumeration is a
//!    construction error, not a default.
//! 2. **Skipped is distinct from passed**
//!    ([`CheckOutcome::counts_as_passed_evidence`],
//!    [`VerificationStatus::passed_evidence`]). Consumers — ordering,
//!    admission, the conversion gate — count only checks that ran and
//!    passed as evidence. A skip is recorded, and it is never counted.
//! 3. **A fixture-dependent patch needs its fixture tests run**
//!    ([`VerificationStatus::deficiencies`], [`PatchProfile`]). A patch
//!    that touches migrations or schemas is not verified unless the tests
//!    that need a live database actually ran — not skipped, not merely
//!    absent from the record.
//! 4. **What can be provisioned must be provisioned**
//!    ([`Fixture::provisionable`]). A database is a container the run can
//!    start, not an intrinsically unavailable resource. A skip for a
//!    missing provisionable fixture is recorded as a deficiency
//!    ([`VerificationDeficiency::FixtureNotProvisioned`]) — the run had
//!    the means and did not use them.
//! 5. **The recorded line is never bare** ([`VerificationStatus::line`]).
//!    Every status line names each check and its outcome; the `VERIFIED`
//!    prefix is earned, and each skip is named with its reason.
//!
//! #3804 sharpens the invariant for capabilities: counting green gates
//! measures effort, not coverage — a feature whose entire job is I/O
//! against an external system, verified only against mocks of it, has
//! approximately zero verification no matter how many assertions pass.
//! 6. **A patch declares the capabilities its verification requires**
//!    ([`PatchProfile::with_ignored_tests`]). An `#[ignore]`d test is a
//!    machine-readable statement that a capability is needed and absent.
//!    A patch that adds `#[ignore]`d tests and runs none of them is
//!    unverified in the dimension it just declared to matter.
//! 7. **Unrun is a distinct outcome from passed and failed, and it
//!    dominates** ([`VerificationStatus::status_for`]). Whatever the other
//!    counters say, such a patch classifies as `UNVERIFIED-CAPABILITY` —
//!    never a status derived from the checks that happened to be runnable.
//! 8. **The artifact records what was required and what was available**
//!    ([`VerificationStatus::line`]): per patch, the capabilities its
//!    tests required and the capabilities the environment had — and a
//!    verdict computed from a strict subset of the declared gate set says
//!    so in the status field, not only in a log line.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// Why a check was skipped.
///
/// A skip always carries its reason: the record must let a consumer tell
/// "the fixture was missing" from "the run ran out of time" from "this
/// check does not apply to this patch", without re-running anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    /// The fixture the check needs was not available to the run.
    ///
    /// Honest only when the fixture could not have been provisioned; see
    /// [`Fixture::provisionable`].
    FixtureUnavailable,
    /// The check was started but the run ran out of time for it.
    TimedOut,
    /// The check does not apply to this patch.
    NotApplicable,
}

impl SkipReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FixtureUnavailable => "fixture_unavailable",
            Self::TimedOut => "timed_out",
            Self::NotApplicable => "not_applicable",
        }
    }
}

/// A fixture a check needs from its environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fixture {
    /// A live database.
    Database,
}

impl Fixture {
    /// Whether the run can provision this fixture itself.
    ///
    /// A database is a container the run can start, not an intrinsically
    /// unavailable resource (#3786): a run that skips a check for lack of
    /// one had the means to start it and did not.
    pub fn provisionable(self) -> bool {
        matches!(self, Self::Database)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Database => "database",
        }
    }
}

/// A component of the system that a patch touches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Component {
    /// A database migration.
    Migration,
    /// A database schema.
    Schema,
    /// Any other component: its tests run without a fixture.
    Code,
}

impl Component {
    /// The fixture this component's tests need, if any.
    pub fn required_fixture(self) -> Option<Fixture> {
        match self {
            Self::Migration | Self::Schema => Some(Fixture::Database),
            Self::Code => None,
        }
    }

    /// Whether this component's tests need a fixture.
    pub fn requires_fixture(self) -> bool {
        self.required_fixture().is_some()
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Migration => "migration",
            Self::Schema => "schema",
            Self::Code => "code",
        }
    }
}

/// A test the patch adds with `#[ignore]`.
///
/// An ignored test is a machine-readable declaration: the patch's
/// verification requires `capability`, and the capability was absent where
/// the test was written (#3804). The run reads the declaration and must
/// either run the capability or report `UNVERIFIED-CAPABILITY`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IgnoredTest {
    /// The test name, e.g. `health_check_sees_live_postgres`.
    pub name: String,
    /// The capability the test requires in order to run.
    pub capability: Fixture,
}

impl IgnoredTest {
    /// Declare one ignored test. Fails on an empty name: an unnamed test
    /// cannot be enumerated in the artifact.
    pub fn new(name: impl Into<String>, capability: Fixture) -> Result<Self, String> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err("an ignored test needs a nonempty name".to_string());
        }
        Ok(Self { name, capability })
    }
}

/// What a patch touches, as the run knows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatchProfile {
    components: BTreeSet<Component>,
    /// The tests the patch adds with `#[ignore]`: the patch's declaration
    /// of the capabilities its verification requires (#3804).
    #[serde(default)]
    ignored_tests: Vec<IgnoredTest>,
}

impl PatchProfile {
    /// Build the profile from the components the patch touches.
    ///
    /// A patch that touches nothing has no verification surface, and a
    /// status against it would be unverifiable vacuous truth.
    pub fn new(components: impl IntoIterator<Item = Component>) -> Result<Self, String> {
        let components: BTreeSet<Component> = components.into_iter().collect();
        if components.is_empty() {
            return Err("a patch profile names at least one component".to_string());
        }
        Ok(Self {
            components,
            ignored_tests: Vec::new(),
        })
    }

    /// The components the patch touches.
    pub fn components(&self) -> &BTreeSet<Component> {
        &self.components
    }

    /// Whether any component the patch touches needs a fixture for its
    /// tests: migrations and schemas do, plain code does not.
    pub fn requires_fixture(&self) -> bool {
        self.components
            .iter()
            .any(|component| component.requires_fixture())
    }

    /// Record the tests the patch adds with `#[ignore]`.
    ///
    /// This is the patch's declaration of the capabilities its
    /// verification requires: an ignored test is a machine-readable
    /// statement that a capability is needed and absent (#3804).
    pub fn with_ignored_tests(mut self, tests: impl IntoIterator<Item = IgnoredTest>) -> Self {
        self.ignored_tests.extend(tests);
        self
    }

    /// The tests the patch adds with `#[ignore]`.
    pub fn ignored_tests(&self) -> &[IgnoredTest] {
        &self.ignored_tests
    }

    /// The capabilities the patch declared through its `#[ignore]`d tests,
    /// in a stable order.
    pub fn declared_capabilities(&self) -> BTreeSet<Fixture> {
        self.ignored_tests
            .iter()
            .map(|test| test.capability)
            .collect()
    }

    /// Every capability the patch's verification requires: what its
    /// `#[ignore]`d tests declare plus what its components need.
    pub fn required_capabilities(&self) -> BTreeSet<Fixture> {
        let mut capabilities = self.declared_capabilities();
        for component in &self.components {
            if let Some(fixture) = component.required_fixture() {
                capabilities.insert(fixture);
            }
        }
        capabilities
    }
}

/// What the run observed for one check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckOutcome {
    /// The check ran and passed.
    Passed,
    /// The check ran and failed.
    Failed {
        /// The check's own detail line, carried verbatim into the record.
        detail: String,
    },
    /// The check did not run, and why. A skip is a record, never an
    /// absence: the reason rides with it.
    Skipped { reason: SkipReason },
}

impl CheckOutcome {
    /// Whether this outcome counts as evidence that the check's subject is
    /// healthy.
    ///
    /// Only a check that ran and passed is evidence. A skip is never
    /// evidence, whatever its reason: it says only that the check did not
    /// run. Ordering and admission decisions count this, not the skip.
    pub fn counts_as_passed_evidence(&self) -> bool {
        matches!(self, Self::Passed)
    }

    pub fn is_skipped(&self) -> bool {
        matches!(self, Self::Skipped { .. })
    }

    pub fn is_failed(&self) -> bool {
        matches!(self, Self::Failed { .. })
    }

    /// The reason, if the check was skipped.
    pub fn skip_reason(&self) -> Option<SkipReason> {
        match self {
            Self::Skipped { reason } => Some(*reason),
            _ => None,
        }
    }
}

/// One check of the run: what it covers and what the run observed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordedCheck {
    name: String,
    requires_fixture: Option<Fixture>,
    outcome: CheckOutcome,
}

impl RecordedCheck {
    /// Record one check.
    ///
    /// Fails closed on incoherent records: a check without a name cannot
    /// be enumerated, and a skip for a missing fixture that names no
    /// fixture cannot be judged against [`Fixture::provisionable`].
    pub fn new(
        name: impl Into<String>,
        requires_fixture: Option<Fixture>,
        outcome: CheckOutcome,
    ) -> Result<Self, String> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err("a recorded check needs a nonempty name".to_string());
        }
        if matches!(
            outcome,
            CheckOutcome::Skipped {
                reason: SkipReason::FixtureUnavailable
            }
        ) && requires_fixture.is_none()
        {
            return Err(format!(
                "check {name} was skipped for a missing fixture but names none; \
                 a fixture skip must say which fixture was missing"
            ));
        }
        Ok(Self {
            name,
            requires_fixture,
            outcome,
        })
    }

    /// The check's stable name, e.g. `build`, `clippy`, `integration`.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The fixture the check needs, if any.
    pub fn requires_fixture(&self) -> Option<Fixture> {
        self.requires_fixture
    }

    /// What the run observed.
    pub fn outcome(&self) -> &CheckOutcome {
        &self.outcome
    }

    /// This check's entry in the status line.
    pub fn line(&self) -> String {
        match &self.outcome {
            CheckOutcome::Passed => self.name.clone(),
            CheckOutcome::Failed { detail } => format!("{}: failed ({})", self.name, detail),
            CheckOutcome::Skipped { reason } => {
                format!("{}: skipped ({})", self.name, reason.as_str())
            }
        }
    }
}

/// Why a recorded status may not read as verified for a patch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerificationDeficiency {
    /// The patch declared a capability its verification requires
    /// (`#[ignore]`d tests) and the run ran no test that needs it. Unrun is
    /// a distinct outcome from passed and failed, and it dominates the
    /// verdict: the status field is `UNVERIFIED-CAPABILITY`, whatever the
    /// other counters say (#3804).
    DeclaredCapabilityUnrun {
        /// The capability the patch declared.
        capability: Fixture,
        /// The patch's `#[ignore]`d tests that declared it.
        tests: Vec<String>,
    },
    /// The verdict was computed from a strict subset of the declared gate
    /// set: the declared gates listed here were never recorded by the run,
    /// and the status field says so rather than only a log line (#3804).
    SubsetOfDeclaredGates {
        /// The declared gates the run did not record.
        gates: Vec<String>,
    },
    /// A check ran and failed.
    FailedCheck {
        /// The check that failed.
        check: String,
        /// The check's own detail line, verbatim.
        detail: String,
    },
    /// A check did not run. The run's reason is recorded, not discarded —
    /// and a skip is never evidence the check's subject is healthy.
    SkippedCheck {
        /// The check that was skipped.
        check: String,
        /// The run's own reason for the skip.
        reason: SkipReason,
    },
    /// A check needing a provisionable fixture was skipped for lack of the
    /// fixture: the run could have started the container and did not
    /// (#3786).
    FixtureNotProvisioned {
        /// The check that was skipped.
        check: String,
        /// The fixture the run could have started.
        fixture: Fixture,
    },
    /// The patch touches a component whose tests need a fixture, but no
    /// check in the status needs one: the tests the patch demands were
    /// never even attempted, and their absence is invisible in a bare
    /// count of passed tests.
    FixtureTestsNotAttempted {
        /// The component the patch touches.
        component: Component,
    },
}

impl VerificationDeficiency {
    /// This deficiency's entry in the status line.
    pub fn line(&self) -> String {
        match self {
            Self::DeclaredCapabilityUnrun { capability, tests } => format!(
                "declared capability unrun: {} ({}: #[ignore]d, none ran)",
                capability.as_str(),
                tests.join(", ")
            ),
            Self::SubsetOfDeclaredGates { gates } => format!(
                "computed from a strict subset of the declared gate set (not recorded: {})",
                gates.join(", ")
            ),
            Self::FailedCheck { check, detail } => format!("failed: {check} ({detail})"),
            Self::SkippedCheck { check, reason } => {
                format!("skipped: {check} ({})", reason.as_str())
            }
            Self::FixtureNotProvisioned { check, fixture } => format!(
                "fixture not provisioned: {check} was skipped for a missing {}; the run could have started it",
                fixture.as_str()
            ),
            Self::FixtureTestsNotAttempted { component } => format!(
                "fixture tests not attempted: the patch touches {}, but no check in the status needs the fixture",
                component.as_str()
            ),
        }
    }
}

/// The verification status an agent records for one run, against one patch.
///
/// The status is the enumeration: every check the run considered, with
/// what the run observed. What the enumeration is entitled to claim is
/// decided by [`deficiencies`](Self::deficiencies) against the patch it
/// was produced for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationStatus {
    checks: Vec<RecordedCheck>,
    /// The capabilities the run's environment provided, e.g. a live
    /// database. Recorded in the artifact with the capabilities the patch
    /// required (#3804).
    #[serde(default)]
    capabilities_available: BTreeSet<Fixture>,
    /// The gate set the repository declared for verification. When the run
    /// recorded a strict subset of it, the status field says so (#3804).
    #[serde(default)]
    declared_gates: BTreeSet<String>,
}

impl VerificationStatus {
    /// Build the status from the run's check records.
    ///
    /// A bare `VERIFIED` is not a permitted value (#3786): a status with
    /// no enumeration says nothing, and it is rejected, not defaulted.
    /// Duplicate names are rejected too: the enumeration must let a
    /// consumer point at a check by name.
    pub fn new(checks: impl IntoIterator<Item = RecordedCheck>) -> Result<Self, String> {
        let checks: Vec<RecordedCheck> = checks.into_iter().collect();
        if checks.is_empty() {
            return Err(
                "a bare VERIFIED is not a permitted value: the status must enumerate the checks \
                 it ran and the checks it skipped, with reasons"
                    .to_string(),
            );
        }
        let mut names = BTreeSet::new();
        for check in &checks {
            if !names.insert(check.name.clone()) {
                return Err(format!("duplicate check in the status: {}", check.name));
            }
        }
        Ok(Self {
            checks,
            capabilities_available: BTreeSet::new(),
            declared_gates: BTreeSet::new(),
        })
    }

    /// Record the capabilities the run's environment provided (#3804).
    pub fn with_capabilities_available(
        mut self,
        capabilities: impl IntoIterator<Item = Fixture>,
    ) -> Self {
        self.capabilities_available.extend(capabilities);
        self
    }

    /// Record the gate set the repository declared for verification.
    ///
    /// A verdict computed from a strict subset of this set must say so in
    /// the status field, not only in a log line (#3804).
    pub fn with_declared_gates(
        mut self,
        gates: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.declared_gates
            .extend(gates.into_iter().map(Into::into));
        self
    }

    /// Every check the run recorded, in run order.
    pub fn checks(&self) -> &[RecordedCheck] {
        &self.checks
    }

    /// The capabilities the run's environment provided.
    pub fn capabilities_available(&self) -> &BTreeSet<Fixture> {
        &self.capabilities_available
    }

    /// The gate set the repository declared for verification.
    pub fn declared_gates(&self) -> &BTreeSet<String> {
        &self.declared_gates
    }

    /// The check named `name`, if the status has one.
    pub fn check(&self, name: &str) -> Option<&RecordedCheck> {
        self.checks.iter().find(|check| check.name == name)
    }

    /// The checks that did not run, in run order.
    pub fn skipped(&self) -> Vec<&RecordedCheck> {
        self.checks
            .iter()
            .filter(|check| check.outcome.is_skipped())
            .collect()
    }

    /// The checks that ran and failed, in run order.
    pub fn failed(&self) -> Vec<&RecordedCheck> {
        self.checks
            .iter()
            .filter(|check| check.outcome.is_failed())
            .collect()
    }

    /// The evidence a consumer may count: the checks that ran and passed,
    /// in run order.
    ///
    /// A skip is never evidence, whatever its reason — ordering and
    /// admission decisions count this list, never the skipped checks.
    pub fn passed_evidence(&self) -> Vec<&RecordedCheck> {
        self.checks
            .iter()
            .filter(|check| check.outcome.counts_as_passed_evidence())
            .collect()
    }

    /// Every reason this status may not read as verified for `patch`:
    /// declared-capability deficiencies first (unrun dominates), then the
    /// declared-gate-set deficiency, then run order (fixture-coverage
    /// deficiencies last).
    ///
    /// Empty means the status is verified for the patch: every declared
    /// capability ran, the full declared gate set was recorded, every check
    /// ran, every check passed, and — when the patch touches a component
    /// whose tests need a fixture — a fixture check ran rather than skipped
    /// or stayed absent.
    pub fn deficiencies(&self, patch: &PatchProfile) -> Vec<VerificationDeficiency> {
        let mut out: Vec<VerificationDeficiency> = self.declared_capability_deficiencies(patch);
        out.extend(self.declared_gates_deficiencies());
        out.extend(self.checks.iter().flat_map(Self::check_deficiencies));
        out.extend(self.fixture_coverage_deficiencies(patch));
        out
    }

    /// Whether the run ran any test that needs `capability`: a check
    /// requiring it whose outcome is not a skip. A failed check still
    /// counts as run: the environment had the capability and the verdict is
    /// then a plain `NOT_VERIFIED`, not an unrun one.
    fn capability_ran(&self, capability: Fixture) -> bool {
        self.checks
            .iter()
            .any(|check| check.requires_fixture == Some(capability) && !check.outcome.is_skipped())
    }

    /// The declared-capability deficiencies (#3804): the patch declared a
    /// capability through `#[ignore]`d tests, and the run ran no test that
    /// needs it. Unrun is a distinct outcome from passed and failed, and it
    /// dominates: the verdict is `UNVERIFIED-CAPABILITY`, whatever the
    /// other counters say.
    fn declared_capability_deficiencies(
        &self,
        patch: &PatchProfile,
    ) -> Vec<VerificationDeficiency> {
        patch
            .declared_capabilities()
            .into_iter()
            .filter_map(|capability| {
                if self.capability_ran(capability) {
                    return None;
                }
                let tests = patch
                    .ignored_tests()
                    .iter()
                    .filter(|test| test.capability == capability)
                    .map(|test| test.name.clone())
                    .collect::<Vec<_>>();
                if tests.is_empty() {
                    return None;
                }
                Some(VerificationDeficiency::DeclaredCapabilityUnrun { capability, tests })
            })
            .collect()
    }

    /// The strict-subset deficiency (#3804): the run recorded a strict
    /// subset of the declared gate set. The status field says so rather
    /// than only a log line — a green artifact over a subset is irrelevant
    /// evidence about the missing gates.
    fn declared_gates_deficiencies(&self) -> Vec<VerificationDeficiency> {
        if self.declared_gates.is_empty() {
            return Vec::new();
        }
        let recorded: BTreeSet<&str> = self
            .checks
            .iter()
            .map(|check| check.name.as_str())
            .collect();
        let missing = self
            .declared_gates
            .iter()
            .filter(|gate| !recorded.contains(gate.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        if missing.is_empty() {
            Vec::new()
        } else {
            vec![VerificationDeficiency::SubsetOfDeclaredGates { gates: missing }]
        }
    }

    /// The deficiencies one check contributes to the record.
    fn check_deficiencies(check: &RecordedCheck) -> Vec<VerificationDeficiency> {
        match &check.outcome {
            CheckOutcome::Passed => Vec::new(),
            CheckOutcome::Failed { detail } => vec![VerificationDeficiency::FailedCheck {
                check: check.name.clone(),
                detail: detail.clone(),
            }],
            // A skip is never "fully verified", whatever its reason: the
            // check did not run, and the record must say so rather than
            // let the status read as broader than it is.
            CheckOutcome::Skipped { reason } => Self::skipped_deficiencies(check, *reason),
        }
    }

    /// The deficiencies one skipped check contributes: the skip itself,
    /// and — when the missing fixture was one the run could have started
    /// (#3786) — the fact that the run had the means and did not use them.
    fn skipped_deficiencies(
        check: &RecordedCheck,
        reason: SkipReason,
    ) -> Vec<VerificationDeficiency> {
        let mut out = vec![VerificationDeficiency::SkippedCheck {
            check: check.name.clone(),
            reason,
        }];
        if reason != SkipReason::FixtureUnavailable {
            return out;
        }
        match check.requires_fixture {
            Some(fixture) if fixture.provisionable() => {
                out.push(VerificationDeficiency::FixtureNotProvisioned {
                    check: check.name.clone(),
                    fixture,
                })
            }
            _ => {}
        }
        out
    }

    /// The fixture-coverage deficiencies: the patch touches a component
    /// whose tests need a fixture, but no check in the status needs one.
    /// A count of passed tests cannot see this; the profile can.
    fn fixture_coverage_deficiencies(&self, patch: &PatchProfile) -> Vec<VerificationDeficiency> {
        let has_fixture_check = self
            .checks
            .iter()
            .any(|check| check.requires_fixture.is_some());
        if !patch.requires_fixture() || has_fixture_check {
            return Vec::new();
        }
        patch
            .components()
            .iter()
            .filter(|component| component.requires_fixture())
            .map(
                |component| VerificationDeficiency::FixtureTestsNotAttempted {
                    component: *component,
                },
            )
            .collect()
    }

    /// Whether the status is verified for this patch: no deficiency at
    /// all.
    pub fn verified_for(&self, patch: &PatchProfile) -> bool {
        self.deficiencies(patch).is_empty()
    }

    /// The status field for the patch (#3804): `UNVERIFIED-CAPABILITY`
    /// when the patch declared a capability the run never ran — unrun
    /// dominates, whatever the other counters say — `VERIFIED` when there
    /// is no deficiency at all, `NOT_VERIFIED` otherwise. A verdict
    /// computed from a strict subset of the declared gate set is
    /// `NOT_VERIFIED`, never `VERIFIED`.
    pub fn status_for(&self, patch: &PatchProfile) -> &'static str {
        let deficiencies = self.deficiencies(patch);
        if deficiencies.iter().any(|deficiency| {
            matches!(
                deficiency,
                VerificationDeficiency::DeclaredCapabilityUnrun { .. }
            )
        }) {
            "UNVERIFIED-CAPABILITY"
        } else if deficiencies.is_empty() {
            "VERIFIED"
        } else {
            "NOT_VERIFIED"
        }
    }

    /// The recorded status line for the patch.
    ///
    /// The line is never bare: it names each check and its outcome, and it
    /// carries the `VERIFIED` prefix only when [`verified_for`] holds; an
    /// unrun declared capability instead reads as `UNVERIFIED-CAPABILITY`
    /// ([`status_for`]). It also records, per patch, the capabilities its
    /// tests required and the capabilities the environment had. A consumer
    /// reading the line hours later sees exactly what ran, what did not,
    /// and why — and can never mistake a skip for a pass.
    pub fn line(&self, patch: &PatchProfile) -> String {
        let enumerated = self
            .checks
            .iter()
            .map(RecordedCheck::line)
            .collect::<Vec<_>>()
            .join(", ");
        let capabilities = format!(
            "capabilities_required={} capabilities_available={}",
            render_capabilities(&patch.required_capabilities()),
            render_capabilities(&self.capabilities_available)
        );
        let deficiencies = self.deficiencies(patch);
        if deficiencies.is_empty() {
            format!("VERIFIED: {enumerated} {capabilities}")
        } else {
            let why = deficiencies
                .iter()
                .map(VerificationDeficiency::line)
                .collect::<Vec<_>>()
                .join("; ");
            format!(
                "{}: {why}; checks: {enumerated} {capabilities}",
                self.status_for(patch)
            )
        }
    }
}

/// The capability list for the status line: a stable, comma-separated
/// enumeration, or `none` when the set is empty.
fn render_capabilities(capabilities: &BTreeSet<Fixture>) -> String {
    if capabilities.is_empty() {
        "none".to_string()
    } else {
        capabilities
            .iter()
            .map(|capability| capability.as_str())
            .collect::<Vec<_>>()
            .join(",")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn passed(name: &str) -> RecordedCheck {
        RecordedCheck::new(name, None, CheckOutcome::Passed).unwrap()
    }

    /// The run the issue describes: every check that could run without a
    /// database passed; the integration suite behind the live database was
    /// skipped because no database was available.
    fn run_without_database() -> VerificationStatus {
        VerificationStatus::new([
            passed("build"),
            passed("clippy"),
            passed("unit"),
            passed("fmt"),
            RecordedCheck::new(
                "integration",
                Some(Fixture::Database),
                CheckOutcome::Skipped {
                    reason: SkipReason::FixtureUnavailable,
                },
            )
            .unwrap(),
        ])
        .unwrap()
    }

    #[test]
    fn a_bare_verified_is_not_a_permitted_value() {
        let error = VerificationStatus::new(Vec::<RecordedCheck>::new()).unwrap_err();
        assert!(error.contains("bare VERIFIED"), "{error}");
    }

    #[test]
    fn check_names_must_be_nonempty_and_unique() {
        let unnamed = RecordedCheck::new("  ", None, CheckOutcome::Passed);
        assert!(unnamed.is_err());
        let duplicate = VerificationStatus::new([passed("build"), passed("build")]);
        assert!(duplicate.unwrap_err().contains("duplicate"));
    }

    #[test]
    fn a_fixture_skip_must_name_the_fixture() {
        let record = RecordedCheck::new(
            "integration",
            None,
            CheckOutcome::Skipped {
                reason: SkipReason::FixtureUnavailable,
            },
        );
        assert!(record.unwrap_err().contains("names none"));
    }

    #[test]
    fn a_migration_patch_run_without_a_database_does_not_read_as_verified() {
        // Regression (#3786): the agent's run had no database. Every
        // check that could run passed; the integration suite behind the
        // live database was skipped. The status must not read as fully
        // verified — and the record must say why, not merely that it
        // does not.
        let patch = PatchProfile::new([Component::Migration, Component::Code]).unwrap();
        let status = run_without_database();

        assert!(!status.verified_for(&patch));
        assert!(!status.verified_for(&PatchProfile::new([Component::Migration]).unwrap()));

        let line = status.line(&patch);
        assert!(
            !line.starts_with("VERIFIED"),
            "the line must not read as fully verified: {line}"
        );
        assert!(
            line.contains("skipped: integration (fixture_unavailable)"),
            "the skip is named with its reason: {line}"
        );
        // The run could have started the container (#3786): a database is
        // a container, not an intrinsically unavailable resource.
        assert!(
            status.deficiencies(&patch).iter().any(|deficiency| {
                matches!(
                    deficiency,
                    VerificationDeficiency::FixtureNotProvisioned {
                        check,
                        fixture: Fixture::Database
                    } if check == "integration"
                )
            }),
            "the missing container is a deficiency, not an excuse: {:?}",
            status.deficiencies(&patch)
        );
        // And a skip is not evidence: the evidence holds the four checks
        // that ran, not the one that did not.
        assert_eq!(
            status
                .passed_evidence()
                .iter()
                .map(|check| check.name.clone())
                .collect::<Vec<_>>(),
            &[
                "build".to_string(),
                "clippy".to_string(),
                "unit".to_string(),
                "fmt".to_string()
            ]
        );
    }

    #[test]
    fn the_same_patch_is_verified_once_the_run_starts_the_database() {
        // The fixture is provisioned: the integration suite runs and
        // passes, and the status is entitled to the full claim.
        let patch = PatchProfile::new([Component::Migration, Component::Code]).unwrap();
        let status = VerificationStatus::new([
            passed("build"),
            passed("clippy"),
            passed("unit"),
            passed("fmt"),
            RecordedCheck::new("integration", Some(Fixture::Database), CheckOutcome::Passed)
                .unwrap(),
        ])
        .unwrap();
        assert!(status.verified_for(&patch));
        let line = status.line(&patch);
        assert!(line.starts_with("VERIFIED: "), "{line}");
        assert!(
            line.contains("integration"),
            "the line enumerates its checks: {line}"
        );
    }

    #[test]
    fn a_schema_patch_with_no_fixture_check_at_all_is_not_verified() {
        // The absence is the trap: every recorded check passed, yet the
        // tests the schema demands never appear in the record at all. A
        // count of passed tests cannot see this; the profile can.
        let patch = PatchProfile::new([Component::Schema]).unwrap();
        let status = VerificationStatus::new([passed("build"), passed("tests")]).unwrap();
        assert!(!status.verified_for(&patch));
        assert_eq!(
            status.deficiencies(&patch),
            vec![VerificationDeficiency::FixtureTestsNotAttempted {
                component: Component::Schema
            }]
        );
        assert!(status.line(&patch).contains("fixture tests not attempted"));
    }

    #[test]
    fn a_failed_check_blocks_verification_and_carries_its_detail() {
        let patch = PatchProfile::new([Component::Code]).unwrap();
        let status = VerificationStatus::new([
            passed("build"),
            RecordedCheck::new(
                "unit",
                None,
                CheckOutcome::Failed {
                    detail: "3 failed".to_string(),
                },
            )
            .unwrap(),
        ])
        .unwrap();
        assert!(!status.verified_for(&patch));
        assert_eq!(
            status.deficiencies(&patch),
            vec![VerificationDeficiency::FailedCheck {
                check: "unit".to_string(),
                detail: "3 failed".to_string()
            }]
        );
    }

    #[test]
    fn a_skip_is_never_counted_as_passed_evidence() {
        // Ordering and admission decisions count evidence, and a skip is
        // not evidence — not for fixture unavailability, not for a
        // timeout, not for "not applicable".
        assert!(CheckOutcome::Passed.counts_as_passed_evidence());
        for reason in [
            SkipReason::FixtureUnavailable,
            SkipReason::TimedOut,
            SkipReason::NotApplicable,
        ] {
            let skipped = CheckOutcome::Skipped { reason };
            assert!(!skipped.counts_as_passed_evidence(), "{reason:?}");
            assert_eq!(skipped.skip_reason(), Some(reason));
        }

        let patch = PatchProfile::new([Component::Code]).unwrap();
        let status = VerificationStatus::new([
            passed("build"),
            passed("tests"),
            RecordedCheck::new(
                "integration",
                Some(Fixture::Database),
                CheckOutcome::Skipped {
                    reason: SkipReason::TimedOut,
                },
            )
            .unwrap(),
        ])
        .unwrap();
        assert_eq!(status.passed_evidence().len(), 2);
        assert_eq!(status.skipped().len(), 1);
        // A timed-out check is recorded with its reason and still keeps
        // the status from reading as verified.
        assert!(!status.verified_for(&patch));
        assert!(status
            .line(&patch)
            .contains("skipped: integration (timed_out)"));
    }

    #[test]
    fn a_code_patch_fully_run_reads_as_verified() {
        let patch = PatchProfile::new([Component::Code]).unwrap();
        let status =
            VerificationStatus::new([passed("build"), passed("clippy"), passed("tests")]).unwrap();
        assert!(status.verified_for(&patch));
        assert_eq!(
            status.line(&patch),
            "VERIFIED: build, clippy, tests capabilities_required=none capabilities_available=none"
        );
    }

    #[test]
    fn the_status_line_always_enumerates_its_checks() {
        // The line is never bare in either direction: a verified line
        // names every check, and a non-verified line names every
        // deficiency and every check.
        let patch = PatchProfile::new([Component::Migration]).unwrap();
        let line = run_without_database().line(&patch);
        for name in ["build", "clippy", "unit", "fmt", "integration"] {
            assert!(line.contains(name), "the line enumerates {name}: {line}");
        }
        assert!(line.starts_with("NOT_VERIFIED: "), "{line}");
    }

    #[test]
    fn the_database_is_provisionable() {
        // The invariant from the issue: a database is a container the run
        // can start, not an intrinsically unavailable resource.
        assert!(Fixture::Database.provisionable());
    }

    // --- #3804: grade by what the gates can prove -------------------------

    /// The incident: a patch whose feature talks to a live PostgreSQL, with
    /// its only live-server test `#[ignore]`d. Every gate that could run
    /// without a database passed; the ignored test never ran. 506 green
    /// assertions are irrelevant evidence about the capability.
    fn incident_patch() -> PatchProfile {
        PatchProfile::new([Component::Code])
            .unwrap()
            .with_ignored_tests([IgnoredTest::new(
                "health_check_sees_live_postgres",
                Fixture::Database,
            )
            .unwrap()])
    }

    #[test]
    fn a_patch_that_adds_ignored_tests_and_runs_none_is_unverified_capability() {
        let patch = incident_patch();
        let status = run_without_database(); // four runnable gates passed, db check skipped

        assert!(!status.verified_for(&patch));
        assert_eq!(status.status_for(&patch), "UNVERIFIED-CAPABILITY");

        let deficiencies = status.deficiencies(&patch);
        assert!(
            matches!(
                &deficiencies.first(),
                Some(VerificationDeficiency::DeclaredCapabilityUnrun {
                    capability: Fixture::Database,
                    tests
                }) if tests == &["health_check_sees_live_postgres".to_string()]
            ),
            "the unrun declared capability is first and names its test: {deficiencies:?}"
        );

        let line = status.line(&patch);
        assert!(
            line.starts_with("UNVERIFIED-CAPABILITY: "),
            "never reads as a pass: {line}"
        );
        assert!(line.contains("health_check_sees_live_postgres"), "{line}");
        // The artifact records what the patch required and what the
        // environment had.
        assert!(line.contains("capabilities_required=database"), "{line}");
        assert!(line.contains("capabilities_available=none"), "{line}");
    }

    #[test]
    fn unrun_dominates_whatever_the_other_counters_say() {
        // Even with a failed check in the record, the unrun declared
        // capability owns the status field: the verdict is not derived from
        // the checks that happened to be runnable.
        let patch = incident_patch();
        let status = VerificationStatus::new([
            passed("build"),
            passed("clippy"),
            RecordedCheck::new(
                "unit",
                None,
                CheckOutcome::Failed {
                    detail: "3 failed".to_string(),
                },
            )
            .unwrap(),
            passed("fmt"),
            RecordedCheck::new(
                "integration",
                Some(Fixture::Database),
                CheckOutcome::Skipped {
                    reason: SkipReason::FixtureUnavailable,
                },
            )
            .unwrap(),
        ])
        .unwrap();

        assert!(status.failed().iter().any(|check| check.name == "unit"));
        assert_eq!(status.status_for(&patch), "UNVERIFIED-CAPABILITY");
        assert!(status.line(&patch).starts_with("UNVERIFIED-CAPABILITY: "));
    }

    #[test]
    fn running_the_declared_capability_earns_verified() {
        // The environment had the database, the fixture gate ran and passed:
        // the dimension the patch declared to matter was measured, and the
        // status is entitled to the full claim.
        let patch = incident_patch();
        let status = VerificationStatus::new([
            passed("build"),
            passed("clippy"),
            passed("unit"),
            passed("fmt"),
            RecordedCheck::new("integration", Some(Fixture::Database), CheckOutcome::Passed)
                .unwrap(),
        ])
        .unwrap()
        .with_capabilities_available([Fixture::Database]);

        assert_eq!(status.status_for(&patch), "VERIFIED");
        assert!(status.verified_for(&patch));
        let line = status.line(&patch);
        assert!(line.starts_with("VERIFIED: "), "{line}");
        assert!(line.contains("capabilities_required=database"), "{line}");
        assert!(line.contains("capabilities_available=database"), "{line}");
    }

    #[test]
    fn a_failed_capability_check_is_run_not_unrun() {
        // The environment had the database and the fixture gate ran — and
        // failed. The capability is not unrun; the verdict is the plain
        // NOT_VERIFIED of a failed check, and it still says what ran.
        let patch = incident_patch();
        let status = VerificationStatus::new([
            passed("build"),
            RecordedCheck::new(
                "integration",
                Some(Fixture::Database),
                CheckOutcome::Failed {
                    detail: "2 failed".to_string(),
                },
            )
            .unwrap(),
        ])
        .unwrap()
        .with_capabilities_available([Fixture::Database]);

        assert_eq!(status.status_for(&patch), "NOT_VERIFIED");
        assert_eq!(
            status.deficiencies(&patch),
            vec![VerificationDeficiency::FailedCheck {
                check: "integration".to_string(),
                detail: "2 failed".to_string()
            }]
        );
    }

    #[test]
    fn a_verdict_over_a_strict_subset_of_the_declared_gates_says_so_in_the_status() {
        // The repository declares five gates; the run recorded four. Every
        // recorded check passed, yet the verdict is a subset verdict, and
        // the status field — not only a log line — says so.
        let patch = PatchProfile::new([Component::Code]).unwrap();
        let status = VerificationStatus::new([
            passed("build"),
            passed("clippy"),
            passed("unit"),
            passed("fmt"),
        ])
        .unwrap()
        .with_declared_gates(["build", "clippy", "unit", "fmt", "--ignored"]);

        assert!(!status.verified_for(&patch));
        assert_eq!(status.status_for(&patch), "NOT_VERIFIED");
        let line = status.line(&patch);
        assert!(
            line.contains("strict subset of the declared gate set"),
            "{line}"
        );
        assert!(line.contains("not recorded: --ignored"), "{line}");

        // Recording the full declared gate set removes the deficiency.
        let full = VerificationStatus::new([
            passed("build"),
            passed("clippy"),
            passed("unit"),
            passed("fmt"),
            passed("--ignored"),
        ])
        .unwrap()
        .with_declared_gates(["build", "clippy", "unit", "fmt", "--ignored"]);
        assert!(full.verified_for(&patch));
        assert_eq!(full.status_for(&patch), "VERIFIED");
    }

    #[test]
    fn an_unnamed_ignored_test_is_rejected() {
        assert!(IgnoredTest::new("  ", Fixture::Database).is_err());
    }

    #[test]
    fn declarations_survive_a_serde_round_trip() {
        let patch = incident_patch();
        let status = run_without_database()
            .with_capabilities_available([Fixture::Database])
            .with_declared_gates(["build", "clippy", "unit", "fmt", "--ignored"]);
        let patch_json = serde_json::to_string(&patch).unwrap();
        let status_json = serde_json::to_string(&status).unwrap();
        let patch: PatchProfile = serde_json::from_str(&patch_json).unwrap();
        let status: VerificationStatus = serde_json::from_str(&status_json).unwrap();

        assert_eq!(
            patch.required_capabilities(),
            BTreeSet::from([Fixture::Database])
        );
        // Availability is not the same as running: the database was
        // available, but the fixture gate never ran.
        assert_eq!(status.status_for(&patch), "UNVERIFIED-CAPABILITY");
    }

    #[test]
    fn a_status_recorded_before_declarations_existed_still_parses() {
        let json = r#"{"checks":[{"name":"build","requires_fixture":null,"outcome":"passed"}]}"#;
        let status: VerificationStatus = serde_json::from_str(json).unwrap();
        assert!(status.capabilities_available().is_empty());
        assert!(status.declared_gates().is_empty());
    }

    #[test]
    fn the_status_survives_a_serde_round_trip() {
        let patch = PatchProfile::new([Component::Migration, Component::Code]).unwrap();
        let status = run_without_database();
        let patch_json = serde_json::to_string(&patch).unwrap();
        let status_json = serde_json::to_string(&status).unwrap();
        let patch: PatchProfile = serde_json::from_str(&patch_json).unwrap();
        let status: VerificationStatus = serde_json::from_str(&status_json).unwrap();
        assert_eq!(patch.requires_fixture(), true);
        assert!(!status.verified_for(&patch));
    }
}
