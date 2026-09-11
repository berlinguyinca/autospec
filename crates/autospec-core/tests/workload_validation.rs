//! Workload validation before trusting a configuration change (issue #4282).
//!
//! The regression tests instantiate the configuration the bug required: a
//! worker whose configuration change was raised (`--ubatch-size` 512 →
//! 2048), passed every monitor signal, and could not answer a request. A
//! test that only ever sees a worker with a completed external request
//! cannot see the bug, because on such a worker every verdict agrees.

use autospec_core::workload_validation::{
    baseline_attribution, evaluate_bottleneck_claim, probe_soundness, registration_standing,
    serving_report, traffic_gate, trust_config_change, Baseline, BottleneckClaim, ConfigTrust,
    ExternalOutcome, ProbeSoundness, RegistrationStanding, ServingEvidence, TrafficGate,
};

const INTENDED: &str = "12k-token prompt";

/// The incident worker: every monitor signal says healthy, including the
/// admission probe — and no external request has completed.
fn incident_evidence() -> Vec<ServingEvidence> {
    vec![
        ServingEvidence::JobState { running: true },
        ServingEvidence::LogScan { error_lines: 0 },
        ServingEvidence::HealthEndpoint { status: 200 },
        ServingEvidence::SlotsReported { free_slots: 4 },
        ServingEvidence::Registration { status: 201 },
        ServingEvidence::AdmissionProbe { completed: true },
    ]
}

// --- Invariant 1: the check's code path decides what it is evidence of ---

#[test]
fn every_monitor_signal_passing_is_still_not_evidence() {
    let trust = trust_config_change(INTENDED, &incident_evidence());
    match trust {
        ConfigTrust::Untrusted { insufficient } => {
            // The report keeps the whole "everything said healthy" table so
            // the refusal is visible against the signals it refused.
            assert_eq!(insufficient.len(), 6);
            assert!(insufficient.iter().any(|l| l.contains("GET /health: 200")));
            assert!(insufficient
                .iter()
                .any(|l| l.contains("gateway registration: 201")));
            assert!(insufficient
                .iter()
                .any(|l| l.contains("admission probe completion: passed")));
        }
        ConfigTrust::Trusted => panic!("the incident worker must not be trusted"),
    }
}

#[test]
fn a_completed_external_request_of_the_intended_shape_is_trusted() {
    let mut ev = incident_evidence();
    ev.push(ServingEvidence::WorkloadRequest {
        shape: INTENDED.to_string(),
        completed: true,
    });
    assert_eq!(trust_config_change(INTENDED, &ev), ConfigTrust::Trusted);
}

#[test]
fn a_failed_external_request_is_not_evidence() {
    // The one-token completion from outside: http 000 after 90 s.
    let mut ev = incident_evidence();
    ev.push(ServingEvidence::WorkloadRequest {
        shape: "one-token completion".to_string(),
        completed: false,
    });
    assert!(matches!(
        trust_config_change(INTENDED, &ev),
        ConfigTrust::Untrusted { .. }
    ));
}

#[test]
fn a_completed_request_of_the_wrong_shape_is_not_evidence() {
    // The change was made for 12k-token prompts; a completed one-token
    // request from outside is resemblance-failure, not evidence.
    let mut ev = incident_evidence();
    ev.push(ServingEvidence::WorkloadRequest {
        shape: "one-token completion".to_string(),
        completed: true,
    });
    assert!(matches!(
        trust_config_change(INTENDED, &ev),
        ConfigTrust::Untrusted { .. }
    ));
}

#[test]
fn no_evidence_at_all_is_untrusted_with_an_empty_table() {
    let trust = trust_config_change(INTENDED, &[]);
    match trust {
        ConfigTrust::Untrusted { insufficient } => assert!(insufficient.is_empty()),
        ConfigTrust::Trusted => panic!("no evidence must not be trusted"),
    }
}

#[test]
fn control_plane_checks_answer_from_the_control_plane() {
    for e in &incident_evidence()[..5] {
        assert_eq!(
            e.class(),
            autospec_core::workload_validation::EvidenceClass::ControlPlane
        );
    }
    assert_eq!(
        incident_evidence()[5].class(),
        autospec_core::workload_validation::EvidenceClass::PrivilegedWorkload
    );
    assert_eq!(
        ServingEvidence::WorkloadRequest {
            shape: INTENDED.to_string(),
            completed: true,
        }
        .class(),
        autospec_core::workload_validation::EvidenceClass::ExternalWorkload
    );
}

#[test]
fn the_report_shows_every_signal_next_to_the_verdict_that_refused_it() {
    let report = serving_report(INTENDED, &incident_evidence());
    assert!(report.contains("GET /slots: 4 slots"));
    assert!(report.contains("worker log: 0 error line(s)"));
    assert!(report.contains("verdict: untrusted"));
    assert!(report.contains(INTENDED));
}

#[test]
fn the_report_of_a_verified_worker_says_trusted() {
    let mut ev = incident_evidence();
    ev.push(ServingEvidence::WorkloadRequest {
        shape: INTENDED.to_string(),
        completed: true,
    });
    assert!(serving_report(INTENDED, &ev).contains("verdict: trusted"));
}

// --- Invariant 2: a probe is only as good as its resemblance to traffic --

#[test]
fn a_probe_passing_while_the_identical_external_request_failed_is_unsound() {
    assert_eq!(
        probe_soundness(true, Some(ExternalOutcome::Failed)),
        ProbeSoundness::Unsound
    );
}

#[test]
fn a_probe_passing_with_a_completed_external_request_is_sound() {
    assert_eq!(
        probe_soundness(true, Some(ExternalOutcome::Completed)),
        ProbeSoundness::Sound
    );
}

#[test]
fn an_unadjudicated_probe_is_fail_closed() {
    assert_eq!(probe_soundness(true, None), ProbeSoundness::Unverified);
}

#[test]
fn a_failed_probe_rejects_regardless_of_the_outside() {
    assert_eq!(
        probe_soundness(false, Some(ExternalOutcome::Completed)),
        ProbeSoundness::Rejected
    );
    assert_eq!(
        probe_soundness(false, Some(ExternalOutcome::Failed)),
        ProbeSoundness::Rejected
    );
}

#[test]
fn an_unsound_probe_leaves_the_worker_live_and_eligible() {
    let soundness = probe_soundness(true, Some(ExternalOutcome::Failed));
    assert_eq!(
        registration_standing(soundness),
        RegistrationStanding::UnsoundLive
    );
}

#[test]
fn soundness_decides_the_registration_standing() {
    assert_eq!(
        registration_standing(ProbeSoundness::Sound),
        RegistrationStanding::Valid
    );
    assert_eq!(
        registration_standing(ProbeSoundness::Rejected),
        RegistrationStanding::Void
    );
    assert_eq!(
        registration_standing(ProbeSoundness::Unverified),
        RegistrationStanding::Held
    );
}

// --- Invariant 3: no window between "up" and "eligible for traffic" ------

#[test]
fn immediate_registration_with_unverified_config_has_no_window() {
    assert_eq!(traffic_gate(true, false), TrafficGate::RegisteredUnverified);
}

#[test]
fn verification_restores_the_window_even_with_immediate_registration() {
    assert_eq!(traffic_gate(true, true), TrafficGate::VerifiedBeforeTraffic);
}

#[test]
fn delayed_registration_leaves_a_staging_window_even_unverified() {
    assert_eq!(
        traffic_gate(false, false),
        TrafficGate::VerifiedBeforeTraffic
    );
}

// --- Invariant 4: a number taken under load describes the load -----------

/// The number that motivated the change: 794 tok/s with three of four slots
/// busy.
fn contended_baseline() -> Baseline {
    Baseline {
        rate: 794.0,
        busy_slots: 3,
        total_slots: 4,
    }
}

/// The idle worker at the unchanged setting: 2434 tok/s.
fn idle_baseline() -> Baseline {
    Baseline {
        rate: 2434.0,
        busy_slots: 0,
        total_slots: 4,
    }
}

#[test]
fn a_number_taken_under_contention_describes_the_load() {
    match baseline_attribution(&contended_baseline()) {
        autospec_core::workload_validation::BaselineAttribution::Load {
            busy_slots,
            total_slots,
        } => {
            assert_eq!(busy_slots, 3);
            assert_eq!(total_slots, 4);
        }
        other => panic!("contended baseline must not read as the parameter: {other:?}"),
    }
}

#[test]
fn an_idle_measurement_describes_the_parameter() {
    assert_eq!(
        baseline_attribution(&idle_baseline()),
        autospec_core::workload_validation::BaselineAttribution::Parameter
    );
}

#[test]
fn a_parameter_claim_built_on_a_contended_number_is_refused() {
    match evaluate_bottleneck_claim("--ubatch-size", &contended_baseline()) {
        BottleneckClaim::Unsupported { reason } => {
            assert!(reason.contains("--ubatch-size"));
            assert!(reason.contains("794"));
            assert!(reason.contains("3/4"));
            assert!(reason.contains("describes the load, not the parameter"));
        }
        BottleneckClaim::Supported => panic!("the incident claim must not be supported"),
    }
}

#[test]
fn a_parameter_claim_built_on_an_idle_measurement_is_supported() {
    assert_eq!(
        evaluate_bottleneck_claim("--ubatch-size", &idle_baseline()),
        BottleneckClaim::Supported
    );
}

#[test]
fn the_idle_number_at_the_unchanged_setting_exceeds_the_contended_one() {
    // The measurement that motivated the change was wrong: contention was
    // the dominant term all along.
    assert!(idle_baseline().rate > contended_baseline().rate);
}
