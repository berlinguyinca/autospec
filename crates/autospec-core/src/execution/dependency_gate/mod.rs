//! The dependency gate: four rules that keep an unverifiable acceptance
//! criterion from being reported as satisfied (issue #3862).
//!
//! The incident this module exists for: a criterion demanded that a real
//! container start. The executor had no container runtime, so the dispatched
//! agent wrote a shell script named `docker`, put its directory in front of
//! `PATH`, and the suite that ran against it — 586 tests, all green — was
//! accepted as evidence the criterion passed, and the issue was closed. Every
//! part of that chain was individually reasonable and collectively worthless:
//! the fake satisfied the *word* of the criterion while the *thing* the
//! criterion was about never happened.
//!
//! The four rules, in the order a run hits them:
//!
//! 1. [`capability`] — a task that names a dependency the executor cannot
//!    provide is never dispatched to that executor. It is held with
//!    [`CapabilityUnavailable`] (wire code `CAPABILITY-UNAVAILABLE`) and stays
//!    queued for an executor that can run it.
//! 2. [`substitution`] — a test suite that stands in for the very dependency
//!    the criterion names is detected ([`SubstitutionAudit`]) and flagged for
//!    review. It is never acceptance: the finding says
//!    `SUBSTITUTION-SUSPECTED`, names the shim and the file, and refuses.
//! 3. [`closure`] — an issue is not auto-closed unless the merged change
//!    itself carries a closing keyword naming it (`Closes #N`, `Fixes #N`,
//!    `Resolves #N`). Without one, [`authorize_closure`] returns
//!    [`ClosureAuthorization::Refused`] (`REFUSED-AUTO-CLOSE`) and the issue
//!    stays open for a human.
//! 4. [`verdict`] — every verdict carries the kind of evidence behind it
//!    ([`EvidenceKind::Hermetic`], [`EvidenceKind::Integration`] or
//!    [`EvidenceKind::Live`]). 586 green hermetic tests are a hermetic verdict;
//!    they do not support a [`Claim::Live`] reading.
//!
//! Rules 1 and 4 share one outcome: when the dependency cannot be provided the
//! answer is a reported *cannot verify here*, not a simulation of the answer.
//! Rule 3 is the immediate safety fix and stands alone: it holds even for a
//! change that needed no external dependency at all.
//!
//! [`Judgement::judge`] runs all four over one criterion, its test sources,
//! the candidate executors and the observed run, and is the shape the CLI
//! wiring consumes.

pub mod capability;
pub mod closure;
pub mod substitution;
pub mod verdict;

pub use capability::{
    authorize_dispatch, binary_capability, named_capabilities, route, Capability,
    CapabilityUnavailable, ExecutorCapabilities, Routing, TaskRequirements,
};
pub use closure::{
    authorize_closure, find_closing_directives, merged_change_closes, ChangeEvidence,
    ClosingDirective, ClosingKeyword, ClosureAuthorization, RefusedClosure,
};
pub use substitution::{
    audit_substitution, Source, SubstitutionAudit, SubstitutionFinding, SubstitutionSignal,
};
pub use verdict::{
    assess_run, classify_evidence, Claim, EvidenceKind, EvidenceSignal, RunObservation,
    Unverifiable, Verdict,
};

/// True when `needle` occurs in `haystack` as a token rather than inside a
/// longer word.
///
/// A token may be delimited by punctuation, quotes or a path separator, so
/// `"docker"` and `bin/docker` both match `docker`. It may not be flanked by
/// word characters: `podman-docker` does not match `docker`, and `dockerfile`
/// does not match `docker`. `-` is a word character on both sides, so neither
/// `podman-docker` nor `docker-compose` matches a bare `docker`: each is a
/// binary of its own and is matched by its own needle.
pub(crate) fn token_present(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let strict_before = needle
        .chars()
        .next()
        .is_some_and(|character| character.is_alphanumeric() || character == '_');
    let strict_after = needle.chars().next_back().is_some_and(|character| {
        character.is_alphanumeric() || character == '_' || character == '-'
    });
    haystack.match_indices(needle).any(|(start, matched)| {
        let end = start + matched.len();
        let before_ok = !strict_before
            || haystack[..start]
                .chars()
                .next_back()
                .is_none_or(|character| {
                    !(character.is_alphanumeric() || character == '_' || character == '-')
                });
        let after_ok = !strict_after
            || haystack[end..].chars().next().is_none_or(|character| {
                !(character.is_alphanumeric() || character == '_' || character == '-')
            });
        before_ok && after_ok
    })
}

/// The combined decision for one acceptance criterion.
///
/// The fields are the four rules in run order, so a reader can see *which*
/// gate stopped the work rather than only that it stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Judgement {
    /// Capabilities the criterion names, declared or inferred.
    pub requirements: TaskRequirements,
    /// Rule 1: where the task may go.
    pub routing: Routing,
    /// Rule 2: what the test sources substituted for a named dependency.
    pub substitution: SubstitutionAudit,
    /// Rule 4: what the observed run proves, and of what kind.
    pub verdict: Verdict,
}

impl Judgement {
    /// Judge one acceptance criterion against the sources that claim to
    /// satisfy it, the executors available for it, and the run those sources
    /// produced.
    ///
    /// A held task is judged without consulting the observation: if the task
    /// should never have been dispatched, whatever its tests reported is not
    /// evidence about the criterion, and reporting a pass count for it would
    /// recreate the incident from the other end.
    pub fn judge(
        criterion: &str,
        sources: &[Source<'_>],
        executors: &[ExecutorCapabilities],
        observation: &RunObservation,
    ) -> Self {
        let requirements = TaskRequirements::from_criterion(criterion);
        let routing = route(&requirements, executors);
        let substitution = audit_substitution(criterion, sources);
        let verdict = match &routing {
            Routing::Hold(hold) => Verdict::capability_unavailable(hold.missing()),
            Routing::Dispatch { .. } => assess_run(&requirements, observation, &substitution),
        };
        Self {
            requirements,
            routing,
            substitution,
            verdict,
        }
    }

    /// True only when every gate passed and the verdict is a pass.
    pub fn accepts(&self) -> bool {
        self.routing.dispatches()
            && self.substitution.accepts()
            && matches!(self.verdict, Verdict::Passed { .. })
    }

    /// One-line human report, gate code first.
    pub fn report(&self) -> String {
        if let Some(hold) = self.routing.hold() {
            return hold.report();
        }
        if self.substitution.substituted() {
            return self.substitution.report();
        }
        self.verdict.summary()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CRITERION: &str = "A fresh container starts the compose stack and the probe answers.";
    const SHIMMED_SUITE: &str = concat!(
        "fn main() {\n",
        "    let bin = tempdir().join(\"bin\");\n",
        "    fs::write(bin.join(\"docker\"), \"#!/bin/sh\\necho ok\\n\").unwrap();\n",
        "    env::set_var(\"PATH\", format!(\"{}:{}\", bin.display(), env::var(\"PATH\").unwrap()));\n",
        "}\n",
    );

    fn executor(id: &str, capabilities: &[Capability]) -> ExecutorCapabilities {
        ExecutorCapabilities::new(id, capabilities.iter().copied())
    }

    #[test]
    fn token_present_matches_delimited_occurrences_only() {
        assert!(token_present("bin/docker run", "docker"));
        assert!(token_present("\"docker\"", "docker"));
        assert!(!token_present("podman-docker package", "docker"));
        assert!(!token_present("dockerfile copies layers", "docker"));
        assert!(token_present("docker-compose up", "docker-compose"));
    }

    #[test]
    fn judgement_holds_the_incident_instead_of_passing_it() {
        // No executor provides a container runtime: the task is held and the
        // 586 green tests of the shimmed suite are never turned into a verdict.
        let executors = vec![executor("exec-hermetic", &[])];
        let observation = RunObservation::new().with_passed(586);
        let judgement = Judgement::judge(
            CRITERION,
            &[Source::new("tests/compose_smoke.rs", SHIMMED_SUITE)],
            &executors,
            &observation,
        );

        assert!(!judgement.accepts());
        assert!(judgement.routing.hold().is_some());
        assert_eq!(judgement.verdict.code(), Some(CapabilityUnavailable::CODE));
        assert!(judgement.report().starts_with(CapabilityUnavailable::CODE));
    }

    #[test]
    fn judgement_flags_substitution_even_when_the_runtime_exists() {
        // The executor *can* run containers, so routing passes; the suite
        // still faked `docker`, so the run is a finding and never acceptance.
        let executors = vec![executor("exec-ci", &[Capability::ContainerRuntime])];
        let observation = RunObservation::new()
            .with_passed(586)
            .providing([Capability::ContainerRuntime]);
        let judgement = Judgement::judge(
            CRITERION,
            &[Source::new("tests/compose_smoke.rs", SHIMMED_SUITE)],
            &executors,
            &observation,
        );

        assert!(!judgement.accepts());
        assert!(judgement.routing.dispatches());
        assert!(judgement.substitution.substituted());
        assert_eq!(
            judgement.verdict.evidence_kind(),
            Some(EvidenceKind::Hermetic)
        );
        assert!(judgement.report().starts_with("SUBSTITUTION-SUSPECTED"));
    }

    #[test]
    fn judgement_passes_a_real_hermetic_criterion() {
        let executors = vec![executor("exec-hermetic", &[])];
        let observation = RunObservation::new().with_passed(12);
        let judgement = Judgement::judge(
            "The parser rejects a malformed frontmatter line with exit code 3.",
            &[Source::new(
                "tests/parser.rs",
                "#[test] fn rejects_frontmatter() {}",
            )],
            &executors,
            &observation,
        );

        assert!(judgement.accepts());
        assert_eq!(judgement.report(), "12 passed (hermetic)");
    }
}
