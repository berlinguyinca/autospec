//! Rule 4 — evidence kind in verdicts.
//!
//! A verdict used to be a pass count. 586 green tests and 1 green test looked
//! the same, and a suite that never touched the dependency the criterion names
//! was indistinguishable from one that drove it end to end. This module makes
//! the kind of evidence part of the verdict, so the weaker claim can never be
//! read as the stronger one:
//!
//! - [`EvidenceKind::Hermetic`] — self-contained. No external dependency was
//!   exercised, or a stand-in was used in its place. True and useful, but it
//!   certifies nothing about the outside system.
//! - [`EvidenceKind::Integration`] — a real dependency was exercised: a real
//!   container, a real database, a real browser.
//! - [`EvidenceKind::Live`] — the deployed system itself was driven.
//!
//! The three form an ordering ([`Claim`]), and the rule the incident demanded
//! is the one thing that ordering exists for: a hermetic verdict does not
//! [`Claim::supports`] an integration or live claim, no matter how many tests
//! it contains. A count is never a kind.
//!
//! When the criterion names a dependency and the run cannot speak to it, the
//! answer is [`Verdict::Unverifiable`] — reported as *cannot verify here* with
//! the dependencies listed — rather than a pass inherited from unrelated green
//! tests.

use std::collections::BTreeSet;
use std::fmt;

use super::capability::{Capability, TaskRequirements};
use super::substitution::SubstitutionAudit;

/// How close a verdict came to the thing the criterion is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EvidenceKind {
    /// Self-contained: no external dependency exercised, or a stand-in used.
    Hermetic,
    /// A real dependency named by the criterion was exercised.
    Integration,
    /// The deployed system itself was driven.
    Live,
}

impl EvidenceKind {
    /// The wire name, written into verdict records.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hermetic => "hermetic",
            Self::Integration => "integration",
            Self::Live => "live",
        }
    }

    /// Parse a wire name.
    pub fn from_token(token: &str) -> Option<Self> {
        match token.trim().to_ascii_lowercase().as_str() {
            "hermetic" | "unit" | "offline" | "sandboxed" => Some(Self::Hermetic),
            "integration" | "dependency" | "real-dependency" => Some(Self::Integration),
            "live" | "production" | "deployed" => Some(Self::Live),
            _ => None,
        }
    }

    /// Stronger evidence implies weaker: live evidence also supports the
    /// integration and hermetic readings of the same criterion.
    pub fn supports(self, claim: Claim) -> bool {
        self.rank() >= claim.rank()
    }

    fn rank(self) -> u8 {
        match self {
            Self::Hermetic => 0,
            Self::Integration => 1,
            Self::Live => 2,
        }
    }
}

impl fmt::Display for EvidenceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The claim a reader wants to make from a verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Claim {
    /// The change is internally consistent.
    Hermetic,
    /// The change works against the real dependency.
    Integration,
    /// The change works against the deployed system.
    Live,
}

impl Claim {
    /// The claim a criterion asks for: naming an external dependency asks for
    /// at least the integration reading, and nothing in a criterion text asks
    /// for a live reading unless it says the deployed system is driven.
    pub fn for_requirements(requirements: &TaskRequirements) -> Self {
        if requirements.is_hermetic() {
            Self::Hermetic
        } else {
            Self::Integration
        }
    }

    /// The wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hermetic => "hermetic",
            Self::Integration => "integration",
            Self::Live => "live",
        }
    }

    fn rank(self) -> u8 {
        match self {
            Self::Hermetic => 0,
            Self::Integration => 1,
            Self::Live => 2,
        }
    }
}

impl fmt::Display for Claim {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What a run actually touched, as inputs to the classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EvidenceSignal {
    /// A stand-in was used in place of a dependency the criterion names.
    pub substituted: bool,
    /// A real dependency named by the criterion was exercised.
    pub exercised_dependency: bool,
    /// The deployed system itself was driven.
    pub drove_live_system: bool,
}

/// Classify the evidence behind a run.
///
/// The order of the rules is the point:
///
/// 1. A substituted dependency is *hermetic*, whatever else the run did. A
///    suite driving a script named `docker` is not integration evidence, and
///    letting `exercised_dependency: true` outrank the shim is precisely how
///    the incident's run got reported as if it had proved something.
/// 2. Driving the deployed system is live evidence.
/// 3. Exercising a real dependency is integration evidence.
/// 4. Everything else is hermetic — including the case where the criterion
///    named a dependency and the run quietly skipped it.
pub fn classify_evidence(signal: EvidenceSignal) -> EvidenceKind {
    if signal.substituted {
        return EvidenceKind::Hermetic;
    }
    if signal.drove_live_system {
        return EvidenceKind::Live;
    }
    if signal.exercised_dependency {
        return EvidenceKind::Integration;
    }
    EvidenceKind::Hermetic
}

/// What a test run reported, as observed from outside.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunObservation {
    passed: u64,
    failed: u64,
    skipped: u64,
    provided: BTreeSet<Capability>,
    substituted: bool,
    exercised_dependency: bool,
    drove_live_system: bool,
}

impl RunObservation {
    /// An observation with no tests and no dependencies available.
    pub fn new() -> Self {
        Self {
            passed: 0,
            failed: 0,
            skipped: 0,
            provided: BTreeSet::new(),
            substituted: false,
            exercised_dependency: false,
            drove_live_system: false,
        }
    }

    /// Number of passing tests.
    pub fn with_passed(mut self, passed: u64) -> Self {
        self.passed = passed;
        self
    }

    /// Number of failing tests.
    pub fn with_failed(mut self, failed: u64) -> Self {
        self.failed = failed;
        self
    }

    /// Number of skipped tests. Skips are not evidence of anything; they are
    /// recorded so a verdict of `12 passed` out of 600 is readable as such.
    pub fn with_skipped(mut self, skipped: u64) -> Self {
        self.skipped = skipped;
        self
    }

    /// Which capabilities the executor actually had during this run. This is
    /// observed, not declared: a host that reported a runtime but lost it
    /// mid-run provides nothing.
    pub fn providing(mut self, capabilities: impl IntoIterator<Item = Capability>) -> Self {
        self.provided = capabilities.into_iter().collect();
        self
    }

    /// Record that the run used a stand-in for a dependency.
    pub fn substituted(mut self) -> Self {
        self.substituted = true;
        self
    }

    /// Record that a real dependency named by the criterion was exercised.
    pub fn exercised_dependency(mut self) -> Self {
        self.exercised_dependency = true;
        self
    }

    /// Record that the deployed system was driven.
    pub fn drove_live_system(mut self) -> Self {
        self.drove_live_system = true;
        self
    }

    /// Passing tests.
    pub fn passed(&self) -> u64 {
        self.passed
    }

    /// Failing tests.
    pub fn failed(&self) -> u64 {
        self.failed
    }

    /// Skipped tests.
    pub fn skipped(&self) -> u64 {
        self.skipped
    }

    /// Capabilities available during the run.
    pub fn provided(&self) -> &BTreeSet<Capability> {
        &self.provided
    }

    /// True when a stand-in was used, whether recorded here or found by rule 2.
    pub fn uses_substitute(&self) -> bool {
        self.substituted
    }

    /// The classification inputs for this run, with `audit`'s findings folded
    /// in so a caller cannot classify a shimmed suite as integration evidence
    /// by forgetting to pass the audit.
    pub fn signal(&self, audit: &SubstitutionAudit) -> EvidenceSignal {
        EvidenceSignal {
            substituted: self.substituted || audit.substituted(),
            exercised_dependency: self.exercised_dependency,
            drove_live_system: self.drove_live_system,
        }
    }
}

impl Default for RunObservation {
    fn default() -> Self {
        Self::new()
    }
}

/// Which gate made a verdict unverifiable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unverifiable {
    /// The executor did not have the dependency at run time.
    ExecutorLacks,
    /// The dependency was available but the run never exercised it — the
    /// green tests are about something else.
    DependencyUntouched,
}

impl Unverifiable {
    /// The wire code for this reason.
    pub fn code(self) -> &'static str {
        match self {
            Self::ExecutorLacks => super::CapabilityUnavailable::CODE,
            Self::DependencyUntouched => "DEPENDENCY-UNVERIFIABLE",
        }
    }
}

/// The verdict for one criterion, carrying its evidence kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// The tests passed, and this is what they were evidence of.
    Passed {
        /// Passing tests.
        passed: u64,
        /// The kind of evidence those tests are.
        evidence: EvidenceKind,
    },
    /// The tests failed. The kind still matters: a failing *live* check is a
    /// different bug from a failing unit test.
    Failed {
        /// Failing tests.
        failed: u64,
        /// The kind of evidence those tests are.
        evidence: EvidenceKind,
    },
    /// The run cannot speak to this criterion. There is no evidence kind,
    /// because there is no evidence.
    Unverifiable {
        /// Dependencies the verdict cannot speak to.
        missing: Vec<Capability>,
        /// Which gate produced the hold.
        reason: Unverifiable,
    },
}

impl Verdict {
    /// A hold because the executor lacked the dependency at run time.
    pub fn capability_unavailable(missing: &[Capability]) -> Self {
        Self::Unverifiable {
            missing: missing.to_vec(),
            reason: Unverifiable::ExecutorLacks,
        }
    }

    /// A hold because the dependency was available and the run skipped it.
    pub fn dependency_untouched(required: &[Capability]) -> Self {
        Self::Unverifiable {
            missing: required.to_vec(),
            reason: Unverifiable::DependencyUntouched,
        }
    }

    /// The evidence kind, absent for an unverifiable verdict by construction.
    pub fn evidence_kind(&self) -> Option<EvidenceKind> {
        match self {
            Self::Passed { evidence, .. } | Self::Failed { evidence, .. } => Some(*evidence),
            Self::Unverifiable { .. } => None,
        }
    }

    /// The gate code this verdict reports, when it reports one.
    pub fn code(&self) -> Option<&'static str> {
        match self {
            Self::Passed { .. } | Self::Failed { .. } => None,
            Self::Unverifiable { reason, .. } => Some(reason.code()),
        }
    }

    /// True when this verdict supports `claim`. An unverifiable verdict
    /// supports nothing, including the hermetic reading of itself.
    pub fn certifies(&self, claim: Claim) -> bool {
        self.evidence_kind()
            .is_some_and(|kind| kind.supports(claim))
    }

    /// Which dependencies are missing, empty unless unverifiable.
    pub fn missing(&self) -> &[Capability] {
        match self {
            Self::Unverifiable { missing, .. } => missing,
            _ => &[],
        }
    }

    /// The one-line form: counts and kind, or the code and what is missing.
    pub fn summary(&self) -> String {
        match self {
            Self::Passed { passed, evidence } => format!("{passed} passed ({evidence})"),
            Self::Failed { failed, evidence } => format!("{failed} failed ({evidence})"),
            Self::Unverifiable { missing, reason } => {
                let dependencies = missing
                    .iter()
                    .map(|capability| capability.as_str())
                    .collect::<Vec<_>>()
                    .join(",");
                format!(
                    "verdict=unverifiable code={code} missing={dependencies}",
                    code = reason.code()
                )
            }
        }
    }
}

/// Decide what the run proves about `requirements`.
///
/// The gate order is deliberate:
///
/// 1. A dependency the run never had cannot be spoken about, whatever the
///    tests reported.
/// 2. Failures are failures, classified by the kind of evidence they are.
/// 3. A criterion that names a dependency, satisfied only by hermetic
///    evidence and *not* flagged as a substitution, is unverifiable: the
///    dependency was reachable and the run did not touch it.
///
/// Substituted runs are deliberately not downgraded here. Rule 2 already
/// refuses them with `SUBSTITUTION-SUSPECTED`; a second code for the same fact
/// would let the two drift. What the verdict does is tell the truth about the
/// green count — it is a hermetic pass — which is exactly why it cannot carry
/// the criterion.
pub fn assess_run(
    requirements: &TaskRequirements,
    observation: &RunObservation,
    audit: &SubstitutionAudit,
) -> Verdict {
    let missing = requirements.missing_in(observation.provided());
    if !missing.is_empty() {
        return Verdict::capability_unavailable(&missing);
    }

    let signal = observation.signal(audit);
    let evidence = classify_evidence(signal);

    if observation.failed() > 0 {
        return Verdict::Failed {
            failed: observation.failed(),
            evidence,
        };
    }

    let claim = Claim::for_requirements(requirements);
    if !signal.substituted && !evidence.supports(claim) {
        return Verdict::dependency_untouched(&requirements.all());
    }

    Verdict::Passed {
        passed: observation.passed(),
        evidence,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution::dependency_gate::audit_substitution;
    use crate::execution::dependency_gate::Source;

    const CONTAINER_CRITERION: &str = "A fresh container starts the compose stack and answers 200.";

    fn requirements(criterion: &str) -> TaskRequirements {
        TaskRequirements::from_criterion(criterion)
    }

    #[test]
    fn evidence_kinds_are_ordered_and_render() {
        assert!(EvidenceKind::Live > EvidenceKind::Integration);
        assert!(EvidenceKind::Integration > EvidenceKind::Hermetic);
        assert_eq!(EvidenceKind::Hermetic.as_str(), "hermetic");
        assert_eq!(EvidenceKind::Integration.as_str(), "integration");
        assert_eq!(EvidenceKind::Live.as_str(), "live");
        assert_eq!(EvidenceKind::from_token("Live"), Some(EvidenceKind::Live));
        assert_eq!(EvidenceKind::from_token("nonsense"), None);
    }

    #[test]
    fn hermetic_evidence_never_supports_a_stronger_claim() {
        assert!(EvidenceKind::Hermetic.supports(Claim::Hermetic));
        assert!(!EvidenceKind::Hermetic.supports(Claim::Integration));
        assert!(!EvidenceKind::Hermetic.supports(Claim::Live));
        assert!(EvidenceKind::Integration.supports(Claim::Integration));
        assert!(!EvidenceKind::Integration.supports(Claim::Live));
        assert!(EvidenceKind::Live.supports(Claim::Hermetic));
        assert!(EvidenceKind::Live.supports(Claim::Live));
    }

    #[test]
    fn a_substituted_dependency_is_hermetic_however_the_run_describes_it() {
        // The incident's run, self-reported as integration evidence about a
        // container. The shim outranks the claim.
        let signal = EvidenceSignal {
            substituted: true,
            exercised_dependency: true,
            drove_live_system: true,
        };
        assert_eq!(classify_evidence(signal), EvidenceKind::Hermetic);
    }

    #[test]
    fn live_beats_integration_beats_nothing() {
        assert_eq!(
            classify_evidence(EvidenceSignal {
                substituted: false,
                exercised_dependency: true,
                drove_live_system: true,
            }),
            EvidenceKind::Live
        );
        assert_eq!(
            classify_evidence(EvidenceSignal {
                substituted: false,
                exercised_dependency: true,
                drove_live_system: false,
            }),
            EvidenceKind::Integration
        );
        assert_eq!(
            classify_evidence(EvidenceSignal::default()),
            EvidenceKind::Hermetic
        );
    }

    #[test]
    fn a_green_hermetic_suite_cannot_certify_a_container_criterion() {
        // 586 tests, all passing, none of them touching a container.
        let observation = RunObservation::new()
            .with_passed(586)
            .providing([Capability::ContainerRuntime]);
        let audit = audit_substitution(
            CONTAINER_CRITERION,
            &[Source::new("tests/unit.rs", "assert_eq!(2 + 2, 4);")],
        );

        let verdict = assess_run(&requirements(CONTAINER_CRITERION), &observation, &audit);
        assert_eq!(
            verdict,
            Verdict::dependency_untouched(&[Capability::ContainerRuntime])
        );
        assert_eq!(verdict.code(), Some("DEPENDENCY-UNVERIFIABLE"));
        assert_eq!(verdict.evidence_kind(), None);
        assert!(!verdict.certifies(Claim::Integration));
        assert!(verdict.missing().contains(&Capability::ContainerRuntime));
        assert_eq!(
            verdict.summary(),
            "verdict=unverifiable code=DEPENDENCY-UNVERIFIABLE missing=container-runtime"
        );
    }

    #[test]
    fn a_run_without_the_dependency_is_unverifiable_not_failed() {
        let observation = RunObservation::new().with_passed(586).with_failed(3);
        let audit = audit_substitution(CONTAINER_CRITERION, &[]);
        let verdict = assess_run(&requirements(CONTAINER_CRITERION), &observation, &audit);
        // The failures are not the story: without a runtime nothing here is
        // evidence about the criterion, in either direction.
        assert_eq!(verdict.code(), Some("CAPABILITY-UNAVAILABLE"));
        assert_eq!(verdict.evidence_kind(), None);
        assert!(!verdict.certifies(Claim::Hermetic));
        assert!(!verdict.certifies(Claim::Integration));
    }

    #[test]
    fn a_real_container_run_is_an_integration_pass() {
        let observation = RunObservation::new()
            .with_passed(586)
            .providing([Capability::ContainerRuntime])
            .exercised_dependency();
        let audit = audit_substitution(CONTAINER_CRITERION, &[]);
        let verdict = assess_run(&requirements(CONTAINER_CRITERION), &observation, &audit);
        assert_eq!(
            verdict,
            Verdict::Passed {
                passed: 586,
                evidence: EvidenceKind::Integration
            }
        );
        assert_eq!(verdict.code(), None);
        assert!(verdict.certifies(Claim::Integration));
        assert!(!verdict.certifies(Claim::Live));
        assert_eq!(verdict.summary(), "586 passed (integration)");
    }

    #[test]
    fn driving_the_deployed_system_is_a_live_pass() {
        let observation = RunObservation::new()
            .with_passed(3)
            .providing([Capability::ContainerRuntime])
            .exercised_dependency()
            .drove_live_system();
        let audit = audit_substitution(CONTAINER_CRITERION, &[]);
        let verdict = assess_run(&requirements(CONTAINER_CRITERION), &observation, &audit);
        assert_eq!(verdict.evidence_kind(), Some(EvidenceKind::Live));
        assert!(verdict.certifies(Claim::Live));
        assert_eq!(verdict.summary(), "3 passed (live)");
    }

    #[test]
    fn a_substituted_run_reports_a_hermetic_pass_and_rule_two_refuses_it() {
        // The verdict tells the truth about the count; the substitution gate
        // does the refusing. One fact, one code.
        let observation = RunObservation::new()
            .with_passed(586)
            .providing([Capability::ContainerRuntime])
            .exercised_dependency();
        let shim = "fs::write(bin.join(\"docker\"), script).unwrap();\n";
        let audit = audit_substitution(CONTAINER_CRITERION, &[Source::new("tests/smoke.rs", shim)]);
        assert!(audit.substituted());

        let verdict = assess_run(&requirements(CONTAINER_CRITERION), &observation, &audit);
        assert_eq!(verdict.evidence_kind(), Some(EvidenceKind::Hermetic));
        assert_eq!(verdict.summary(), "586 passed (hermetic)");
        assert!(!verdict.certifies(Claim::Integration));
    }

    #[test]
    fn failures_keep_their_evidence_kind() {
        let observation = RunObservation::new()
            .with_passed(585)
            .with_failed(1)
            .providing([Capability::ContainerRuntime])
            .exercised_dependency();
        let audit = audit_substitution(CONTAINER_CRITERION, &[]);
        let verdict = assess_run(&requirements(CONTAINER_CRITERION), &observation, &audit);
        assert_eq!(
            verdict,
            Verdict::Failed {
                failed: 1,
                evidence: EvidenceKind::Integration
            }
        );
        assert_eq!(verdict.summary(), "1 failed (integration)");
        // A failing verdict is still evidence of the kind it is.
        assert!(verdict.certifies(Claim::Integration));
    }

    #[test]
    fn a_hermetic_criterion_is_satisfied_by_hermetic_evidence() {
        let criterion = "The parser rejects a malformed frontmatter line with exit code 3.";
        let observation = RunObservation::new().with_passed(12).with_skipped(2);
        let audit = audit_substitution(criterion, &[]);
        let verdict = assess_run(&requirements(criterion), &observation, &audit);
        assert_eq!(
            verdict,
            Verdict::Passed {
                passed: 12,
                evidence: EvidenceKind::Hermetic
            }
        );
        assert!(verdict.certifies(Claim::for_requirements(&requirements(criterion))));
    }

    #[test]
    fn the_required_claim_comes_from_the_criterion() {
        assert_eq!(
            Claim::for_requirements(&requirements("The parser rejects exit code 3.")),
            Claim::Hermetic
        );
        assert_eq!(
            Claim::for_requirements(&requirements(CONTAINER_CRITERION)),
            Claim::Integration
        );
    }

    #[test]
    fn the_audit_is_folded_in_so_a_shim_cannot_be_classified_away() {
        let observation = RunObservation::new()
            .with_passed(1)
            .providing([Capability::ContainerRuntime])
            .exercised_dependency()
            .drove_live_system();
        let shim = "fs::write(bin.join(\"docker\"), script).unwrap();\n";
        let audit = audit_substitution(CONTAINER_CRITERION, &[Source::new("tests/smoke.rs", shim)]);
        assert_eq!(
            observation.signal(&audit),
            EvidenceSignal {
                substituted: true,
                exercised_dependency: true,
                drove_live_system: true,
            }
        );
        // The flag came from the audit alone; the observation never said so.
        assert!(!observation.uses_substitute());
        assert!(
            !observation
                .signal(&audit_substitution(CONTAINER_CRITERION, &[]))
                .substituted
        );
    }
}
