//! Spot measurement versus continuous instrumentation (issue #4444).
//!
//! The incident: `qwen3.8-flash-next` prefill was hand-measured at
//! 4–6 tok/s on several workers, and one spot check during a prefill
//! read 0% GPU utilisation with idle power draw — exactly what CPU
//! execution looks like. The conclusion, filed and argued: llama.cpp
//! has no GPU path for the architecture. Then the admission probe was
//! instrumented to log its throughput decision on every path, and the
//! first eight measurements, on uniquely-generated uncacheable prompts,
//! put the same model at 222 tok/s — 37x above the hand-taken floor.
//! 222 tok/s is not CPU speed for a 107 GB model. Every individual
//! observation was true; the generalisation from "these workers were
//! slow on these prompts" to "the runtime cannot use the GPU for this
//! architecture" was the error.
//!
//! The regression tests run in the configuration the bug required: a
//! hand-taken floor, one 0% spot check, a filed architectural claim —
//! and then the instrumented record that refutes it.

use autospec_core::spot_measurement::{
    adjudicate, audit_sequence, judge_claim, permitted_status, Adjudication, Claim, ClaimScope,
    ClaimStatus, ClaimVerdict, Observation, Provenance, SequenceVerdict, Source, Step,
};

/// The hand-taken floor from the incident: several workers, several
/// prompts, 4–6 tok/s.
fn hand_floor() -> Observation {
    Observation::new(Source::Manual, "qwen3.8-flash-next prefill", 5.0)
}

/// The instrumented record: the admission probe logging its throughput
/// decision on every path, uniquely-generated uncacheable prompts.
fn instrumented_record() -> Vec<Observation> {
    vec![
        Observation::new(Source::Continuous, "qwen3.8-27b prefill", 799.0),
        Observation::new(Source::Continuous, "qwen3.8-27b-vision prefill", 798.0),
        Observation::new(Source::Continuous, "qwen3.8-flash-next prefill", 222.0),
    ]
}

/// The filed architectural claim: local symptom, upstream project.
fn filed_claim() -> Claim {
    Claim {
        statement: "llama.cpp has no GPU path for this architecture; the model runs on CPU".into(),
        scope: ClaimScope::AboutTheSystem,
        recorded_as: ClaimStatus::Finding,
        provenance: Some(Provenance::new(Source::Manual, 3).unwrap()),
    }
}

#[test]
fn the_incident_the_instrument_wins_by_37x() {
    match adjudicate(&hand_floor(), &instrumented_record()) {
        Adjudication::InstrumentedWins { disagreement } => {
            assert_eq!(disagreement.subject, "qwen3.8-flash-next prefill");
            assert_eq!(disagreement.instrumented, 222.0);
            assert_eq!(disagreement.manual, 5.0);
            // 37x above the floor: the instrument samples the
            // population, the spot check sampled an occasion.
            assert!((disagreement.ratio - 44.4).abs() < 1e-9);
        }
        other => panic!("expected InstrumentedWins, got {other:?}"),
    }
}

#[test]
fn the_zero_percent_gauge_read_against_the_record() {
    // The 0% GPU spot check, adjudicated against an instrumented
    // utilisation reading: one side zero, the ratio infinite — a gap
    // between compute phases, not the absence of a GPU path.
    let spot = Observation::new(Source::Manual, "gpu utilisation during prefill", 0.0);
    let record = [Observation::new(
        Source::Continuous,
        "gpu utilisation during prefill",
        91.0,
    )];
    match adjudicate(&spot, &record) {
        Adjudication::InstrumentedWins { disagreement } => {
            assert_eq!(disagreement.ratio, f64::INFINITY);
        }
        other => panic!("expected InstrumentedWins, got {other:?}"),
    }
}

#[test]
fn a_spot_check_that_agrees_with_the_record_holds() {
    let spot = Observation::new(Source::Manual, "qwen3.8-27b prefill", 790.0);
    assert!(matches!(
        adjudicate(&spot, &instrumented_record()),
        Adjudication::Agreement { value: 799.0, .. }
    ));
}

#[test]
fn a_subject_the_instrument_has_not_logged_stays_provisional() {
    let spot = Observation::new(Source::Manual, "qwen3.8-mini prefill", 40.0);
    assert!(matches!(
        adjudicate(&spot, &instrumented_record()),
        Adjudication::NoInstrumentation { value: 40.0, .. }
    ));
}

#[test]
fn hand_taken_repeats_are_still_spot_checks() {
    // Invariant 2: a conclusion on spot-check provenance is
    // provisional no matter how many times the hand took it; only the
    // system's own record measures repeatedly.
    assert_eq!(
        permitted_status(&Provenance::new(Source::Manual, 8).unwrap()),
        ClaimStatus::Provisional
    );
    assert_eq!(
        permitted_status(&Provenance::new(Source::Continuous, 1).unwrap()),
        ClaimStatus::Finding
    );
}

#[test]
fn the_filed_claim_is_unwarranted_generalisation() {
    assert_eq!(
        judge_claim(&filed_claim()),
        ClaimVerdict::UnwarrantedGeneralisation
    );
    let line = ClaimVerdict::UnwarrantedGeneralisation.line();
    assert!(line.starts_with("FAIL:"), "{line}");
    assert!(line.contains("someone else's software"), "{line}");
}

#[test]
fn the_same_claim_recorded_provisionally_holds() {
    let mut claim = filed_claim();
    claim.recorded_as = ClaimStatus::Provisional;
    assert_eq!(judge_claim(&claim), ClaimVerdict::Sound);
}

#[test]
fn the_same_statement_on_instrumented_provenance_is_a_finding() {
    let mut claim = filed_claim();
    claim.statement = "this model prefills at 222 tok/s on the cluster".into();
    claim.recorded_as = ClaimStatus::Finding;
    claim.provenance = Some(Provenance::new(Source::Continuous, 8).unwrap());
    assert_eq!(judge_claim(&claim), ClaimVerdict::Sound);
}

#[test]
fn a_local_symptom_as_finding_on_spot_checks_is_a_violation_but_not_a_generalisation() {
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
fn a_claim_with_no_provenance_is_unmeasured() {
    let mut claim = filed_claim();
    claim.provenance = None;
    assert_eq!(judge_claim(&claim), ClaimVerdict::Unmeasured);
    let line = ClaimVerdict::Unmeasured.line();
    assert!(line.starts_with("FAIL:"), "{line}");
    assert!(line.contains("how many times"), "{line}");
}

#[test]
fn a_single_sample_is_flagged_so_it_can_be_noticed_later() {
    // Corollary: recording how a claim was measured, and how many
    // times, is what makes it possible to notice later that it was one
    // sample.
    let claim = Claim {
        statement: "these workers are slow on these prompts".into(),
        scope: ClaimScope::LocalSymptom,
        recorded_as: ClaimStatus::Provisional,
        provenance: Some(Provenance::new(Source::Manual, 1).unwrap()),
    };
    assert_eq!(judge_claim(&claim), ClaimVerdict::SingleSample);
    let line = ClaimVerdict::SingleSample.line();
    assert!(line.starts_with("WARN:"), "{line}");
    assert!(line.contains("one sample"), "{line}");
}

#[test]
fn provenance_must_record_at_least_one_measurement() {
    assert!(Provenance::new(Source::Manual, 0).is_none());
    assert!(Provenance::new(Source::Continuous, 0).is_none());
}

#[test]
fn the_incident_sequence_concluded_before_instrumenting() {
    // Spot checks, a filed claim, hours of argument — only then the
    // one small change that instrumented the probe.
    assert_eq!(
        audit_sequence(&[Step::SpotCheck, Step::Concluded, Step::Instrumented]),
        SequenceVerdict::ConcludedBeforeInstrumenting
    );
    // The instrument in place before the conclusion clears the gate —
    // even when spot checks preceded the instrumentation.
    assert_eq!(
        audit_sequence(&[Step::SpotCheck, Step::Instrumented, Step::Concluded]),
        SequenceVerdict::InstrumentedFirst
    );
    assert_eq!(
        audit_sequence(&[Step::SpotCheck, Step::Instrumented]),
        SequenceVerdict::NoConclusion
    );
}

#[test]
fn the_incident_end_to_end_from_filed_claim_to_refutation() {
    // The claim as it was filed: a local symptom, upstream project.
    let verdict = judge_claim(&filed_claim());
    assert_eq!(verdict, ClaimVerdict::UnwarrantedGeneralisation);

    // The instrument, on uniquely-generated uncacheable prompts:
    // 37x above the hand-taken floor.
    match adjudicate(&hand_floor(), &instrumented_record()) {
        Adjudication::InstrumentedWins { disagreement } => {
            assert!(disagreement.ratio > 30.0, "ratio {}", disagreement.ratio);
        }
        other => panic!("expected InstrumentedWins, got {other:?}"),
    }

    // Re-recorded on the instrument's provenance, the statement about
    // the model stands as a finding.
    let re_recorded = Claim {
        statement: "this model prefills at 222 tok/s on the cluster".into(),
        scope: ClaimScope::AboutTheSystem,
        recorded_as: ClaimStatus::Finding,
        provenance: Some(Provenance::new(Source::Continuous, 8).unwrap()),
    };
    assert_eq!(judge_claim(&re_recorded), ClaimVerdict::Sound);
}

#[test]
fn labels_for_records() {
    assert_eq!(Source::Continuous.label(), "continuous instrumentation");
    assert_eq!(Source::Manual.label(), "spot check");
    assert_eq!(ClaimScope::LocalSymptom.label(), "local symptom");
    assert_eq!(ClaimScope::AboutTheSystem.label(), "the system itself");
}
