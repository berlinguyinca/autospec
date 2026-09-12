//! Spot measurement versus continuous instrumentation (issue #4444).
//!
//! The incident: `qwen3.8-flash-next` prefill was hand-measured at
//! **4–6 tok/s** on several workers, and one spot check during such a
//! prefill read **0% GPU utilisation with idle power draw** — exactly
//! what CPU execution looks like. The conclusion, filed and argued:
//! llama.cpp has no GPU path for the architecture and runs the model on
//! CPU. An architectural claim about an upstream project, and every
//! individual observation behind it was true.
//!
//! The remedy cost one small change: instrument the admission probe to
//! log its throughput decision on every path. The first eight
//! measurements, on uniquely-generated uncacheable prompts:
//!
//! ```text
//! qwen3.8-27b         799 … 1797 tok/s
//! qwen3.8-27b-vision  798 tok/s
//! qwen3.8-flash-next  222 tok/s      <- 37x above the hand-taken floor
//! ```
//!
//! 222 tok/s is not CPU speed for a 107 GB model. The conclusion was
//! wrong, or at best described specific degraded workers rather than
//! the model. The error was the generalisation from "these workers were
//! slow on these prompts" to "the runtime cannot use the GPU for this
//! architecture" — a local symptom upgraded into a claim about someone
//! else's software. A spot sample of a utilisation gauge during a long
//! operation can land in a gap between compute phases; one sample, on
//! one worker, at one moment, felt decisive and stopped the
//! investigation.
//!
//! The invariants this module makes checkable, in the order the
//! incident produced them:
//!
//! 1. **Prefer a measurement the system takes continuously over one
//!    you take by hand.** Where they disagree, the instrument wins: it
//!    samples the population, you sampled an occasion.
//!    [`adjudicate`]
//! 2. **A conclusion drawn from spot checks is provisional until
//!    something measures it repeatedly.** Write it down with that
//!    status, rather than as a finding — especially before escalating
//!    it to a claim about someone else's software.
//!    [`permitted_status`], [`judge_claim`]
//! 3. **Instrument before concluding, not only before fixing.**
//!    [`audit_sequence`]
//! 4. **Record how a claim was measured, and how many times.** That is
//!    what makes it possible to notice later that it was one sample —
//!    unusually expensive for autonomous work, where the wrong
//!    conclusion becomes an input to later reasoning. [`Provenance`]
//!
//! Everything here is pure: no I/O, no clock, no subprocesses. The
//! caller takes the measurements and reports them; this code only
//! decides what they are evidence of.

/// How a measurement was taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The system itself records it on every occurrence (a probe log
    /// line, a gauge the runtime emits). Samples the population.
    Continuous,
    /// Taken by hand, once, against whatever the system happened to be
    /// doing. Samples an occasion.
    Manual,
}

impl Source {
    /// The label for records.
    pub fn label(self) -> &'static str {
        match self {
            Source::Continuous => "continuous instrumentation",
            Source::Manual => "spot check",
        }
    }
}

/// One measurement of one subject.
#[derive(Debug, Clone, PartialEq)]
pub struct Observation {
    pub source: Source,
    /// What was measured, in the terms the claim uses it (e.g.
    /// `qwen3.8-flash-next prefill`).
    pub subject: String,
    /// The measured value (e.g. tok/s, percent).
    pub value: f64,
}

impl Observation {
    pub fn new(source: Source, subject: impl Into<String>, value: f64) -> Self {
        Observation {
            source,
            subject: subject.into(),
            value,
        }
    }
}

/// A relative difference above this fraction of the larger reading
/// counts as a disagreement. A gauge read in a gap between compute
/// phases is not the same reading as the population; agreement must be
/// claimed, not assumed.
pub const DISAGREEMENT_TOLERANCE: f64 = 0.25;

fn disagrees(a: f64, b: f64) -> bool {
    let hi = a.abs().max(b.abs());
    hi > 0.0 && (a - b).abs() > DISAGREEMENT_TOLERANCE * hi
}

/// The first reading pair where the spot check disagrees with the
/// system's own record for the same subject.
#[derive(Debug, Clone, PartialEq)]
pub struct Disagreement {
    pub subject: String,
    pub instrumented: f64,
    pub manual: f64,
    /// The larger reading divided by the smaller — the incident's 37.
    /// Infinite when one side read zero.
    pub ratio: f64,
}

fn ratio(a: f64, b: f64) -> f64 {
    let (hi, lo) = (a.abs().max(b.abs()), a.abs().min(b.abs()));
    if lo == 0.0 {
        f64::INFINITY
    } else {
        hi / lo
    }
}

/// A spot check and the system's own record for the same subject
/// (invariant 1).
#[derive(Debug, Clone, PartialEq)]
pub enum Adjudication {
    /// The system's record agrees with the spot check (within
    /// [`DISAGREEMENT_TOLERANCE`]): the reading holds.
    Agreement { subject: String, value: f64 },
    /// The system's record disagrees with the spot check: the
    /// instrument wins — it samples the population, the spot check
    /// sampled an occasion.
    InstrumentedWins { disagreement: Disagreement },
    /// The system has no record for the subject: the spot check
    /// stands, but only as provisional (invariant 2).
    NoInstrumentation { subject: String, value: f64 },
}

/// Adjudicate one hand-taken reading against the system's continuous
/// record for the same subject (invariant 1). `instrumented` holds the
/// observations the system itself logged; only those whose subject
/// matches are considered.
pub fn adjudicate(manual: &Observation, instrumented: &[Observation]) -> Adjudication {
    let matching: Vec<&Observation> = instrumented
        .iter()
        .filter(|o| o.source == Source::Continuous && o.subject == manual.subject)
        .collect();
    match matching.first() {
        None => Adjudication::NoInstrumentation {
            subject: manual.subject.clone(),
            value: manual.value,
        },
        Some(record) => {
            if disagrees(record.value, manual.value) {
                Adjudication::InstrumentedWins {
                    disagreement: Disagreement {
                        subject: manual.subject.clone(),
                        instrumented: record.value,
                        manual: manual.value,
                        ratio: ratio(record.value, manual.value),
                    },
                }
            } else {
                Adjudication::Agreement {
                    subject: manual.subject.clone(),
                    value: record.value,
                }
            }
        }
    }
}

/// How a claim was measured, recorded so a later reader can tell one
/// sample from a measurement (corollary, invariant 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Provenance {
    pub source: Source,
    /// How many times the measurement behind the claim was taken.
    /// At least one: a claim with no measurement is not a claim, it is
    /// a guess, and [`Provenance::new`] refuses to record it.
    pub count: u32,
}

impl Provenance {
    /// Build a provenance. Rejects a count of zero — the record must
    /// say how many times, and "zero times" means there is nothing to
    /// record.
    pub fn new(source: Source, count: u32) -> Option<Self> {
        (count > 0).then_some(Provenance { source, count })
    }
}

/// What a claim is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimScope {
    /// "These workers were slow on these prompts" — about the observed
    /// occasion.
    LocalSymptom,
    /// "The runtime cannot use the GPU for this architecture" — about
    /// the model, or an upstream project.
    AboutTheSystem,
}

impl ClaimScope {
    /// The label for records.
    pub fn label(self) -> &'static str {
        match self {
            ClaimScope::LocalSymptom => "local symptom",
            ClaimScope::AboutTheSystem => "the system itself",
        }
    }
}

/// The status a claim was written down with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimStatus {
    /// Written down as a finding.
    Finding,
    /// Written down with its provisional status, as the invariant
    /// requires of spot-check conclusions.
    Provisional,
}

/// The strongest status a claim with this provenance may be written
/// down with (invariant 2).
///
/// A continuous instrument measures repeatedly — on every occurrence —
/// so a conclusion on its record may stand as a finding. A spot check
/// measures once, no matter how many times it is repeated by hand, so
/// its conclusion is provisional until something measures it
/// repeatedly.
pub fn permitted_status(provenance: &Provenance) -> ClaimStatus {
    match provenance.source {
        Source::Continuous => ClaimStatus::Finding,
        Source::Manual => ClaimStatus::Provisional,
    }
}

/// A claim about the system, with its scope, its recorded status, and
/// how it was measured.
#[derive(Debug, Clone, PartialEq)]
pub struct Claim {
    pub statement: String,
    pub scope: ClaimScope,
    pub recorded_as: ClaimStatus,
    pub provenance: Option<Provenance>,
}

/// What [`judge_claim`] found.
#[derive(Debug, Clone, PartialEq)]
pub enum ClaimVerdict {
    /// Properly recorded: the status matches the provenance, and the
    /// provenance is there to be read.
    Sound,
    /// No provenance recorded: how the claim was measured, and how many
    /// times, is unrecorded. A one-sample claim reads exactly like a
    /// measured one — and an agent that cannot easily revisit
    /// yesterday's measurement has every incentive to build on it.
    Unmeasured,
    /// A claim about the model or an upstream project, on spot-check
    /// provenance, written down as a finding: a local symptom upgraded
    /// into a claim about someone else's software. The incident.
    UnwarrantedGeneralisation,
    /// A spot-check conclusion written down as a finding instead of
    /// with its provisional status (invariant 2).
    SpotCheckRecordedAsFinding,
    /// The provenance shows the measurement was taken exactly once:
    /// flagged so it is noticed later that it was one sample.
    SingleSample,
}

impl ClaimVerdict {
    /// The line the verdict renders in a report.
    pub fn line(&self) -> String {
        match self {
            ClaimVerdict::Sound => {
                "OK: claim recorded with status permitted by its provenance".into()
            }
            ClaimVerdict::Unmeasured => {
                "FAIL: claim has no provenance — how it was measured, and how many \
                 times, is unrecorded"
                    .into()
            }
            ClaimVerdict::UnwarrantedGeneralisation => {
                "FAIL: claim about the system on spot-check provenance, recorded as a \
                 finding — a local symptom upgraded into a claim about someone else's \
                 software"
                    .into()
            }
            ClaimVerdict::SpotCheckRecordedAsFinding => {
                "FAIL: spot-check conclusion recorded as a finding — provisional until \
                 something measures it repeatedly"
                    .into()
            }
            ClaimVerdict::SingleSample => "WARN: measured once — one sample, revisitable".into(),
        }
    }
}

/// Judge a claim against invariants 2 and 4 and the corollary.
pub fn judge_claim(claim: &Claim) -> ClaimVerdict {
    let Some(provenance) = claim.provenance else {
        return ClaimVerdict::Unmeasured;
    };
    let is_finding = claim.recorded_as == ClaimStatus::Finding;
    let spot_check = provenance.source == Source::Manual;
    if spot_check && is_finding && claim.scope == ClaimScope::AboutTheSystem {
        return ClaimVerdict::UnwarrantedGeneralisation;
    }
    if spot_check && is_finding {
        return ClaimVerdict::SpotCheckRecordedAsFinding;
    }
    if provenance.count == 1 {
        return ClaimVerdict::SingleSample;
    }
    ClaimVerdict::Sound
}

/// One step in the order the diagnosis proceeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// A spot check was taken by hand.
    SpotCheck,
    /// A conclusion was drawn, filed, or argued.
    Concluded,
    /// Instrumentation was added and started recording.
    Instrumented,
}

/// Whether the diagnosis instrumented before it concluded
/// (invariant 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SequenceVerdict {
    /// No conclusion was drawn; nothing to gate.
    NoConclusion,
    /// The conclusion came after instrumentation existed: the record
    /// was in place before the claim.
    InstrumentedFirst,
    /// A conclusion was drawn before instrumentation existed:
    /// premature — the claim stands provisionally until the instrument
    /// says. The incident's hours, and a filed architectural claim.
    ConcludedBeforeInstrumenting,
}

/// Audit the order the diagnosis proceeded (invariant 3). The first
/// `Concluded` step is judged against the first `Instrumented` step.
pub fn audit_sequence(steps: &[Step]) -> SequenceVerdict {
    let Some(concluded_at) = steps.iter().position(|s| *s == Step::Concluded) else {
        return SequenceVerdict::NoConclusion;
    };
    match steps.iter().position(|s| *s == Step::Instrumented) {
        Some(instrumented_at) if instrumented_at < concluded_at => {
            SequenceVerdict::InstrumentedFirst
        }
        _ => SequenceVerdict::ConcludedBeforeInstrumenting,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manual(subject: &str, value: f64) -> Observation {
        Observation::new(Source::Manual, subject, value)
    }

    fn instrumented(subject: &str, value: f64) -> Observation {
        Observation::new(Source::Continuous, subject, value)
    }

    #[test]
    fn the_incident_37x_above_the_floor() {
        let manual = manual("qwen3.8-flash-next prefill", 6.0);
        let record = [instrumented("qwen3.8-flash-next prefill", 222.0)];
        match adjudicate(&manual, &record) {
            Adjudication::InstrumentedWins { disagreement } => {
                assert_eq!(disagreement.instrumented, 222.0);
                assert_eq!(disagreement.manual, 6.0);
                assert_eq!(disagreement.ratio, 37.0);
            }
            other => panic!("expected InstrumentedWins, got {other:?}"),
        }
    }

    #[test]
    fn agreement_within_tolerance_holds() {
        let manual = manual("qwen3.8-flash-next prefill", 5.0);
        let record = [instrumented("qwen3.8-flash-next prefill", 6.0)];
        assert!(matches!(
            adjudicate(&manual, &record),
            Adjudication::Agreement { value: 6.0, .. }
        ));
    }

    #[test]
    fn a_zero_reading_disagrees_with_a_nonzero_record_and_has_infinite_ratio() {
        let manual = manual("gpu utilisation during prefill", 0.0);
        let record = [instrumented("gpu utilisation during prefill", 91.0)];
        match adjudicate(&manual, &record) {
            Adjudication::InstrumentedWins { disagreement } => {
                assert_eq!(disagreement.ratio, f64::INFINITY);
            }
            other => panic!("expected InstrumentedWins, got {other:?}"),
        }
    }

    #[test]
    fn no_record_for_the_subject_is_provisional() {
        let manual = manual("qwen3.8-flash-next prefill", 5.0);
        let record = [instrumented("qwen3.8-27b prefill", 799.0)];
        assert!(matches!(
            adjudicate(&manual, &record),
            Adjudication::NoInstrumentation { value: 5.0, .. }
        ));
    }

    #[test]
    fn a_manual_observation_in_the_record_is_not_a_record() {
        let spot = manual("qwen3.8-flash-next prefill", 5.0);
        let not_a_record = [Observation::new(
            Source::Manual,
            "qwen3.8-flash-next prefill",
            222.0,
        )];
        assert!(matches!(
            adjudicate(&spot, &not_a_record),
            Adjudication::NoInstrumentation { .. }
        ));
    }

    #[test]
    fn spot_check_conclusions_are_provisional() {
        assert_eq!(
            permitted_status(&Provenance::new(Source::Manual, 8).unwrap()),
            ClaimStatus::Provisional
        );
        assert_eq!(
            permitted_status(&Provenance::new(Source::Continuous, 8).unwrap()),
            ClaimStatus::Finding
        );
    }

    #[test]
    fn the_incident_claim_was_unwarranted_generalisation() {
        let claim = Claim {
            statement: "llama.cpp has no GPU path for this architecture".into(),
            scope: ClaimScope::AboutTheSystem,
            recorded_as: ClaimStatus::Finding,
            provenance: Some(Provenance::new(Source::Manual, 3).unwrap()),
        };
        assert_eq!(judge_claim(&claim), ClaimVerdict::UnwarrantedGeneralisation);
    }

    #[test]
    fn the_same_claim_on_instrumented_provenance_is_sound() {
        let claim = Claim {
            statement: "this model prefills at 222 tok/s on the cluster".into(),
            scope: ClaimScope::AboutTheSystem,
            recorded_as: ClaimStatus::Finding,
            provenance: Some(Provenance::new(Source::Continuous, 8).unwrap()),
        };
        assert_eq!(judge_claim(&claim), ClaimVerdict::Sound);
    }

    #[test]
    fn a_spot_check_local_symptom_recorded_as_finding_is_a_finding_violation() {
        let claim = Claim {
            statement: "these workers are slow on these prompts".into(),
            scope: ClaimScope::LocalSymptom,
            recorded_as: ClaimStatus::Finding,
            provenance: Some(Provenance::new(Source::Manual, 2).unwrap()),
        };
        assert_eq!(
            judge_claim(&claim),
            ClaimVerdict::SpotCheckRecordedAsFinding
        );
    }

    #[test]
    fn recorded_provisionally_the_local_symptom_holds() {
        let claim = Claim {
            statement: "these workers are slow on these prompts".into(),
            scope: ClaimScope::LocalSymptom,
            recorded_as: ClaimStatus::Provisional,
            provenance: Some(Provenance::new(Source::Manual, 2).unwrap()),
        };
        assert_eq!(judge_claim(&claim), ClaimVerdict::Sound);
    }

    #[test]
    fn unrecorded_provenance_is_unmeasured() {
        let claim = Claim {
            statement: "the runtime cannot use the GPU for this architecture".into(),
            scope: ClaimScope::AboutTheSystem,
            recorded_as: ClaimStatus::Provisional,
            provenance: None,
        };
        assert_eq!(judge_claim(&claim), ClaimVerdict::Unmeasured);
    }

    #[test]
    fn a_single_sample_is_flagged_revisitable() {
        let claim = Claim {
            statement: "these workers are slow on these prompts".into(),
            scope: ClaimScope::LocalSymptom,
            recorded_as: ClaimStatus::Provisional,
            provenance: Some(Provenance::new(Source::Manual, 1).unwrap()),
        };
        assert_eq!(judge_claim(&claim), ClaimVerdict::SingleSample);
    }

    #[test]
    fn provenance_refuses_zero_measurements() {
        assert!(Provenance::new(Source::Manual, 0).is_none());
        assert!(Provenance::new(Source::Continuous, 0).is_none());
        assert_eq!(
            Provenance::new(Source::Manual, 1).unwrap(),
            Provenance {
                source: Source::Manual,
                count: 1
            }
        );
    }

    #[test]
    fn the_incident_sequence_concluded_before_instrumenting() {
        let steps = [Step::SpotCheck, Step::Concluded, Step::Instrumented];
        assert_eq!(
            audit_sequence(&steps),
            SequenceVerdict::ConcludedBeforeInstrumenting
        );
    }

    #[test]
    fn instrumenting_first_clears_the_gate() {
        let steps = [Step::Instrumented, Step::Concluded];
        assert_eq!(audit_sequence(&steps), SequenceVerdict::InstrumentedFirst);
        // Instrumentation added between spot checks but before the
        // conclusion still counts as before.
        let steps = [Step::SpotCheck, Step::Instrumented, Step::Concluded];
        assert_eq!(audit_sequence(&steps), SequenceVerdict::InstrumentedFirst);
    }

    #[test]
    fn no_conclusion_nothing_to_gate() {
        let steps = [Step::SpotCheck, Step::Instrumented];
        assert_eq!(audit_sequence(&steps), SequenceVerdict::NoConclusion);
        assert_eq!(audit_sequence(&[]), SequenceVerdict::NoConclusion);
    }

    #[test]
    fn labels_for_records() {
        assert_eq!(Source::Continuous.label(), "continuous instrumentation");
        assert_eq!(Source::Manual.label(), "spot check");
        assert_eq!(ClaimScope::LocalSymptom.label(), "local symptom");
        assert_eq!(ClaimScope::AboutTheSystem.label(), "the system itself");
    }

    #[test]
    fn the_incident_end_to_end() {
        // The diagnosis as it happened: spot checks, a filed
        // architectural claim, then the instrument that refuted it.
        assert_eq!(
            audit_sequence(&[Step::SpotCheck, Step::Concluded, Step::Instrumented]),
            SequenceVerdict::ConcludedBeforeInstrumenting
        );
        let claim = Claim {
            statement: "llama.cpp runs this model on CPU".into(),
            scope: ClaimScope::AboutTheSystem,
            recorded_as: ClaimStatus::Finding,
            provenance: Some(Provenance::new(Source::Manual, 1).unwrap()),
        };
        assert_eq!(judge_claim(&claim), ClaimVerdict::UnwarrantedGeneralisation);
        // The instrument, on uniquely-generated uncacheable prompts.
        let manual = manual("qwen3.8-flash-next prefill", 5.0);
        let record = vec![
            instrumented("qwen3.8-27b prefill", 799.0),
            instrumented("qwen3.8-27b-vision prefill", 798.0),
            instrumented("qwen3.8-flash-next prefill", 222.0),
        ];
        match adjudicate(&manual, &record) {
            Adjudication::InstrumentedWins { disagreement } => {
                assert!(disagreement.ratio > 30.0);
            }
            other => panic!("expected InstrumentedWins, got {other:?}"),
        }
        // Re-recorded on the instrument's provenance, the same
        // statement stands as a finding.
        let re_recorded = Claim {
            statement: "this model prefills at 222 tok/s on the cluster".into(),
            scope: ClaimScope::AboutTheSystem,
            recorded_as: ClaimStatus::Finding,
            provenance: Some(Provenance::new(Source::Continuous, 8).unwrap()),
        };
        assert_eq!(judge_claim(&re_recorded), ClaimVerdict::Sound);
    }
}
