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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
    /// Whether this component's tests need a fixture.
    pub fn requires_fixture(self) -> bool {
        matches!(self, Self::Migration | Self::Schema)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Migration => "migration",
            Self::Schema => "schema",
            Self::Code => "code",
        }
    }
}

/// What a patch touches, as the run knows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatchProfile {
    components: BTreeSet<Component>,
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
        Ok(Self { components })
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
        Ok(Self { checks })
    }

    /// Every check the run recorded, in run order.
    pub fn checks(&self) -> &[RecordedCheck] {
        &self.checks
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

    /// Every reason this status may not read as verified for `patch`, in
    /// run order (fixture-coverage deficiencies last).
    ///
    /// Empty means the status is verified for the patch: every check ran,
    /// every check passed, and — when the patch touches a component whose
    /// tests need a fixture — a fixture check ran rather than skipped or
    /// stayed absent.
    pub fn deficiencies(&self, patch: &PatchProfile) -> Vec<VerificationDeficiency> {
        let mut out: Vec<VerificationDeficiency> = self
            .checks
            .iter()
            .flat_map(Self::check_deficiencies)
            .collect();
        out.extend(self.fixture_coverage_deficiencies(patch));
        out
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

    /// The recorded status line for the patch.
    ///
    /// The line is never bare: it names each check and its outcome, and it
    /// carries the `VERIFIED` prefix only when [`verified_for`] holds. A
    /// consumer reading the line hours later sees exactly what ran, what
    /// did not, and why — and can never mistake a skip for a pass.
    pub fn line(&self, patch: &PatchProfile) -> String {
        let enumerated = self
            .checks
            .iter()
            .map(RecordedCheck::line)
            .collect::<Vec<_>>()
            .join(", ");
        let deficiencies = self.deficiencies(patch);
        if deficiencies.is_empty() {
            format!("VERIFIED: {enumerated}")
        } else {
            let why = deficiencies
                .iter()
                .map(VerificationDeficiency::line)
                .collect::<Vec<_>>()
                .join("; ");
            format!("NOT_VERIFIED: {why}; checks: {enumerated}")
        }
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
        assert_eq!(status.line(&patch), "VERIFIED: build, clippy, tests");
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
