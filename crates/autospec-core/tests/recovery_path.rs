//! Recovery paths and readiness waits (issue #4399).
//!
//! The incident: a worker probed its model server's `/props` endpoint
//! **once**, 10 seconds after start, and treated the failure as permanent.
//! `/props` answers `503 {"error":"Loading model"}` until the weights are
//! resident, and `qwen3.8-flash-next` is a 105 GiB mmap from a network
//! filesystem that had not finished loading after **1h44m**. The probe
//! always failed, the endpoint file was never written, and four workers
//! sat holding **8 GPUs** — fully loaded and completely invisible to the
//! gateway.
//!
//! The comment in the code said "REGISTER FAILED http=$code -- regsweep
//! will retry within 5 minutes". That reassurance is false on this path:
//! `regsweep` reconciles *from* the endpoint files — and the endpoint file
//! is only written once the probe succeeds. The recovery mechanism
//! consumes the artifact that the failing step was supposed to produce.
//!
//! The regression tests run in the configuration the bug required: a
//! one-shot 10-second probe against a load measured at 1h44m, and a
//! reconciler whose only input is the artifact the failing step writes.
//! The controls: the same reconciler reading an input that survives the
//! failure covers it, and the poll whose deadline is scaled to the
//! measured worst case is a wait.

use std::time::Duration;

use autospec_core::recovery_path::{
    claim_verdict, readiness_verdict, recovery_verdict, ClaimVerdict, MeasuredWorstCase,
    ReadinessProbe, ReadinessVerdict, Recovery, RecoveryVerdict, RegistrationSpec, RetryClaim,
    RetryOwner, Step,
};

/// The measured worst case of the startup phase the worker depends on:
/// 1h44m, the point at which the 105 GiB mmap from the network filesystem
/// still had not finished.
fn worst_case() -> Duration {
    Duration::from_secs(60 * 104)
}

/// The worker: it writes the endpoint file only after its readiness check
/// against the model server succeeds.
fn worker_step() -> Step {
    Step {
        id: "worker".into(),
        produces: vec!["endpoint-file".into()],
        readiness_worst_case: worst_case(),
    }
}

/// A step whose artifacts exist independently of the worker: the scheduler
/// writes its job record when the job is submitted, before any probe.
fn scheduler_step() -> Step {
    Step {
        id: "scheduler".into(),
        produces: vec!["slurm-job-record".into()],
        readiness_worst_case: Duration::ZERO,
    }
}

fn incident_steps() -> Vec<Step> {
    vec![worker_step(), scheduler_step()]
}

/// The probe as written: one check, 10 seconds, 10 seconds after start.
fn incident_probe() -> ReadinessProbe {
    ReadinessProbe {
        resource: "/props".into(),
        checks: 1,
        deadline: Duration::from_secs(10),
    }
}

/// The recovery as written: regsweep reconciles from the endpoint files.
fn incident_recovery() -> Recovery {
    Recovery {
        name: "regsweep".into(),
        recovers: "worker".into(),
        reads: vec!["endpoint-file".into()],
    }
}

/// The comment in the code: the reassurance that was false on this path.
fn incident_claim() -> RetryClaim {
    RetryClaim {
        text: "REGISTER FAILED http=$code -- regsweep will retry within 5 minutes".into(),
        names: "regsweep".into(),
    }
}

#[test]
fn one_check_ten_seconds_after_start_is_a_probe_not_a_wait() {
    let verdict = readiness_verdict(&incident_probe(), worst_case());

    assert!(
        matches!(verdict, ReadinessVerdict::OneShot { .. }),
        "{verdict:?}"
    );
    let line = verdict.line();
    assert!(line.starts_with("FAIL:"), "{line}");
    assert!(line.contains("a wait, not a probe"), "{line}");
    assert!(line.contains("6240s"), "{line}");
}

#[test]
fn a_deadline_inside_the_measured_worst_case_is_a_guaranteed_failure() {
    // Even polling does not rescue a 10-second budget: the deadline
    // expires while the model is still loading, every time.
    let probe = ReadinessProbe {
        resource: "/props".into(),
        checks: 624,
        deadline: Duration::from_secs(10),
    };
    let verdict = readiness_verdict(&probe, worst_case());

    assert!(
        matches!(verdict, ReadinessVerdict::GuaranteedFailure { .. }),
        "{verdict:?}"
    );
    let line = verdict.line();
    assert!(line.starts_with("FAIL:"), "{line}");
    assert!(line.contains("guaranteed failure"), "{line}");
    assert!(line.contains("falls inside"), "{line}");
}

#[test]
fn a_poll_to_a_deadline_scaled_to_the_resource_is_a_wait() {
    // Two hours against a 1h44m measured worst case: the deadline is
    // scaled to the resource, so the check is a wait.
    let probe = ReadinessProbe {
        resource: "/props".into(),
        checks: 720,
        deadline: Duration::from_secs(2 * 60 * 60),
    };
    let verdict = readiness_verdict(&probe, worst_case());

    assert!(
        matches!(verdict, ReadinessVerdict::Waits { .. }),
        "{verdict:?}"
    );
    let line = verdict.line();
    assert!(line.starts_with("OK:"), "{line}");
    assert!(line.contains("the check is a wait"), "{line}");
}

#[test]
fn a_single_check_is_fine_when_the_resource_has_no_startup_phase() {
    // Invariant 1 is about resources with a startup phase: a resource
    // that is ready at t=0 can be checked once.
    let probe = ReadinessProbe {
        resource: "/healthz".into(),
        checks: 1,
        deadline: Duration::from_secs(10),
    };
    let verdict = readiness_verdict(&probe, Duration::ZERO);
    assert!(
        matches!(verdict, ReadinessVerdict::Waits { .. }),
        "{verdict:?}"
    );
    assert!(verdict.line().starts_with("OK:"), "{}", verdict.line());
}

#[test]
fn the_incident_reconciler_reads_the_failing_step_s_artifact() {
    let verdict = recovery_verdict(&incident_recovery(), &incident_steps());

    assert!(
        matches!(
            verdict,
            RecoveryVerdict::Decorative {
                ref artifact, ..
            } if artifact == "endpoint-file"
        ),
        "{verdict:?}"
    );
    let line = verdict.line();
    assert!(line.starts_with("FAIL:"), "{line}");
    assert!(line.contains("decorative"), "{line}");
    assert!(
        line.contains("can never fire for the case that actually needs it"),
        "{line}"
    );
}

#[test]
fn a_recovery_that_reads_an_input_that_survives_the_failure_covers_it() {
    // The same reconciler, pointed at the scheduler's job record — which
    // exists when the job is submitted, before any probe — covers the hard
    // failure: the worker's failure cannot prevent the input from existing.
    let recovery = Recovery {
        name: "regsweep".into(),
        recovers: "worker".into(),
        reads: vec!["slurm-job-record".into()],
    };
    let verdict = recovery_verdict(&recovery, &incident_steps());

    assert!(
        matches!(
            verdict,
            RecoveryVerdict::Covers {
                independent_inputs: 1,
                ..
            }
        ),
        "{verdict:?}"
    );
    let line = verdict.line();
    assert!(line.starts_with("OK:"), "{line}");
    assert!(line.contains("exist even when 'worker' fails"), "{line}");
}

#[test]
fn a_recovery_over_a_step_not_in_the_model_is_unverifiable_not_covers() {
    // Fail-closed: a reconciler whose step nobody has modelled cannot be
    // read as covering the failure.
    let recovery = Recovery {
        name: "regsweep".into(),
        recovers: "ghost".into(),
        reads: vec!["endpoint-file".into()],
    };
    let verdict = recovery_verdict(&recovery, &incident_steps());
    assert!(
        matches!(verdict, RecoveryVerdict::Unverifiable { .. }),
        "{verdict:?}"
    );
    let line = verdict.line();
    assert!(line.starts_with("FAIL:"), "{line}");
    assert!(line.contains("not in the step model"), "{line}");
}

#[test]
fn an_input_produced_by_another_step_is_not_the_failing_step_s_artifact() {
    // The decoration is specific: the failing step producing the input.
    // The scheduler writing an input regsweep reads does not make regsweep
    // decorative for the worker.
    let recovery = Recovery {
        name: "regsweep".into(),
        recovers: "worker".into(),
        reads: vec!["slurm-job-record".into(), "endpoint-file".into()],
    };
    let verdict = recovery_verdict(&recovery, &incident_steps());
    // One poisoned input is enough: the retry still cannot fire when the
    // endpoint file is the input that matters.
    assert!(
        matches!(verdict, RecoveryVerdict::Decorative { .. }),
        "{verdict:?}"
    );
}

#[test]
fn the_comment_that_was_wrong_is_a_false_claim() {
    let verdict = claim_verdict(&incident_claim(), &[incident_recovery()], &incident_steps());

    assert!(
        matches!(
            verdict,
            ClaimVerdict::False {
                ref recovery,
                ref step,
                ref artifact,
            } if recovery == "regsweep" && step == "worker" && artifact == "endpoint-file"
        ),
        "{verdict:?}"
    );
    let line = verdict.line();
    assert!(line.starts_with("FAIL:"), "{line}");
    assert!(
        line.contains("can never fire for the case that needs it"),
        "{line}"
    );
}

#[test]
fn a_claim_over_a_recovery_that_covers_is_verified() {
    let recovery = Recovery {
        name: "regsweep".into(),
        recovers: "worker".into(),
        reads: vec!["slurm-job-record".into()],
    };
    let verdict = claim_verdict(&incident_claim(), &[recovery], &incident_steps());
    assert!(
        matches!(verdict, ClaimVerdict::Verified { .. }),
        "{verdict:?}"
    );
    assert!(verdict.line().starts_with("OK:"), "{}", verdict.line());
}

#[test]
fn a_claim_over_a_recovery_that_does_not_exist_names_nothing() {
    let claim = RetryClaim {
        text: "the sweep will retry within 5 minutes".into(),
        names: "retriesweep".into(),
    };
    let verdict = claim_verdict(&claim, &[incident_recovery()], &incident_steps());
    assert!(
        matches!(verdict, ClaimVerdict::NamesNothing { .. }),
        "{verdict:?}"
    );
    assert!(verdict.line().starts_with("FAIL:"), "{}", verdict.line());
}

#[test]
fn a_claim_over_an_unverifiable_recovery_is_not_verified() {
    // The claim points at a real recovery, but one whose inputs cannot be
    // checked: unverified is not verified.
    let recovery = Recovery {
        name: "regsweep".into(),
        recovers: "ghost".into(),
        reads: vec!["slurm-job-record".into()],
    };
    let verdict = claim_verdict(&incident_claim(), &[recovery], &incident_steps());
    assert!(
        matches!(verdict, ClaimVerdict::Unverifiable { .. }),
        "{verdict:?}"
    );
    assert!(
        verdict.line().contains("unverified, not verified"),
        "{}",
        verdict.line()
    );
}

#[test]
fn the_spec_must_state_the_readiness_policy() {
    let spec = RegistrationSpec::default();
    let findings = spec.findings();

    assert_eq!(findings.len(), 3, "{findings:?}");
    assert!(
        findings.iter().any(|f| f.contains("not ready yet")),
        "{findings:?}"
    );
    assert!(
        findings
            .iter()
            .any(|f| f.contains("how long readiness may take")),
        "{findings:?}"
    );
    assert!(
        findings.iter().any(|f| f.contains("who retries")),
        "{findings:?}"
    );
    assert!(!spec.is_complete());
}

#[test]
fn the_readiness_bound_is_the_measured_worst_case_not_a_guess() {
    // A number without a source is a guess: that is the choice that became
    // 10 seconds.
    assert!(MeasuredWorstCase::new(Duration::from_secs(10), "").is_none());
    assert!(MeasuredWorstCase::new(
        worst_case(),
        "qwen3.8-flash-next, 105 GiB mmap, NFS, 2026-09-11"
    )
    .is_some());

    let spec = RegistrationSpec {
        not_ready: Some("poll /props to the deadline, then fail the registration".into()),
        readiness: Some(MeasuredWorstCase {
            value: worst_case(),
            source: String::new(),
        }),
        retry: Some(RetryOwner {
            owner: "regsweep".into(),
            input: "slurm-job-record".into(),
        }),
    };
    let findings = spec.findings();
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert!(
        findings[0].contains("not a measured worst case"),
        "{findings:?}"
    );
}

#[test]
fn the_retry_owner_must_name_the_input_its_retry_depends_on() {
    let spec = RegistrationSpec {
        not_ready: Some("poll /props to the deadline, then fail the registration".into()),
        readiness: Some(
            MeasuredWorstCase::new(worst_case(), "qwen3.8-flash-next, NFS, 2026-09-11").unwrap(),
        ),
        retry: Some(RetryOwner {
            owner: "regsweep".into(),
            input: String::new(),
        }),
    };
    let findings = spec.findings();
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert!(
        findings[0].contains("does not name the input that retry depends on"),
        "{findings:?}"
    );
}

#[test]
fn a_complete_spec_has_no_findings() {
    let spec = RegistrationSpec {
        not_ready: Some("poll /props to the deadline, then fail the registration".into()),
        readiness: Some(
            MeasuredWorstCase::new(worst_case(), "qwen3.8-flash-next, NFS, 2026-09-11").unwrap(),
        ),
        retry: Some(RetryOwner {
            owner: "regsweep".into(),
            input: "slurm-job-record".into(),
        }),
    };
    assert!(spec.is_complete());
    assert!(spec.findings().is_empty());
}

#[test]
fn the_incident_end_to_end_eight_gpus_invisible_to_the_gateway() {
    let steps = incident_steps();

    // The probe: one check, 10 seconds, against a load measured at 1h44m.
    let probe = readiness_verdict(&incident_probe(), worker_step().readiness_worst_case);
    assert!(
        matches!(probe, ReadinessVerdict::OneShot { .. }),
        "{probe:?}"
    );
    assert!(probe.line().starts_with("FAIL:"), "{}", probe.line());

    // The recovery: regsweep reads the endpoint file the worker writes
    // only if the probe succeeded.
    let recovery = recovery_verdict(&incident_recovery(), &steps);
    assert!(
        matches!(recovery, RecoveryVerdict::Decorative { .. }),
        "{recovery:?}"
    );
    assert!(
        recovery.line().contains("decorative"),
        "{}",
        recovery.line()
    );

    // The comment: the reassurance that shaped the behaviour of everyone
    // reading the code, and was false on the path that needed it.
    let claim = claim_verdict(&incident_claim(), &[incident_recovery()], &steps);
    assert!(matches!(claim, ClaimVerdict::False { .. }), "{claim:?}");
    assert!(claim.line().starts_with("FAIL:"), "{}", claim.line());

    // The spec that produced it states none of the three things.
    let spec = RegistrationSpec::default();
    assert_eq!(spec.findings().len(), 3, "{spec:?}");

    // The remedy: the same reconciler reading an input that exists even
    // when the worker fails — the launch record written before any probe.
    let independent = Recovery {
        name: "regsweep".into(),
        recovers: "worker".into(),
        reads: vec!["slurm-job-record".into()],
    };
    assert!(matches!(
        recovery_verdict(&independent, &steps),
        RecoveryVerdict::Covers { .. }
    ));
    assert!(matches!(
        claim_verdict(&incident_claim(), &[independent], &steps),
        ClaimVerdict::Verified { .. }
    ));
}
