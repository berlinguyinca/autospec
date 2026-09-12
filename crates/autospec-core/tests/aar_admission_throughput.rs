//! The admission throughput probe (issue #4412).
//!
//! The regression this suite pins: the probe sent the same ~512-token
//! prompt every time, so `llama-server`'s prompt cache could serve the
//! second and later probes and report a cache-lookup rate as a prefill
//! rate; and the admission decision logged nothing on any path, so after
//! deployment "zero unverified admissions" could not be told apart from
//! "the probe never ran" — while a direct 512-token request to the same
//! worker timed out at 140s. Each test below maps to one of the issue's
//! invariants; the final test reconstructs the incident end to end.

use autospec_core::aar::admission_throughput::{
    decide_admission, nonce_prefix, probe_prompt, prompt_shape, AdmissionBranch, AdmissionDecision,
    ProbeMeasurement, PromptShape,
};

/// The probe body: the same ~512-token workload on every probe. Only the
/// nonce may vary.
const BODY: &str = "summarise the following text in one sentence: <text>";

// ── Invariant 1: the probe prompt must be unique per probe ─────────────────

#[test]
fn the_probe_prompt_carries_the_nonce_prefix_over_the_fixed_body() {
    let prompt = probe_prompt("nonce-1", BODY).unwrap();
    assert!(prompt.starts_with(nonce_prefix("nonce-1").as_str()));
    assert!(prompt.contains("nonce-1"));
    assert!(prompt.ends_with(BODY));
    // The nonce is at the front: the prefix diverges on the first token, so
    // the prompt cache cannot serve one probe's prompt to the next.
    assert_eq!(prompt, format!("{} {BODY}", nonce_prefix("nonce-1")));
}

#[test]
fn a_blank_nonce_or_body_is_rejected() {
    // A blank nonce leaves the prompt constant across probes — exactly the
    // defect this exists to remove.
    assert!(probe_prompt("", BODY).is_err());
    assert!(probe_prompt("   ", BODY).is_err());
    assert!(probe_prompt("nonce-1", "").is_err());
    assert!(probe_prompt("nonce-1", "   ").is_err());
}

#[test]
fn distinct_nonces_yield_distinct_prompts() {
    let nonces = [
        "2026-09-11T18:16:00Z-w1",
        "2026-09-11T18:16:05Z-w1",
        "other-worker",
    ];
    let prompts: Vec<String> = nonces
        .iter()
        .map(|nonce| probe_prompt(nonce, BODY).unwrap())
        .collect();
    for (i, a) in prompts.iter().enumerate() {
        for b in &prompts[i + 1..] {
            assert_ne!(a, b, "two probes must never share a prompt");
        }
    }
}

#[test]
fn a_constant_prompt_is_classified_reused() {
    let constant = BODY;
    assert_eq!(prompt_shape(&[constant, constant]), PromptShape::Reused);
    assert_eq!(
        prompt_shape(&[constant, "other", constant]),
        PromptShape::Reused
    );
}

#[test]
fn unique_prompts_are_classified_unique_per_probe() {
    let a = probe_prompt("nonce-1", BODY).unwrap();
    let b = probe_prompt("nonce-2", BODY).unwrap();
    assert_eq!(prompt_shape(&[&a, &b]), PromptShape::UniquePerProbe);
    // The reuse defect needs two identical prompts: an empty or single-probe
    // set has nothing to compare.
    assert_eq!(prompt_shape(&[]), PromptShape::UniquePerProbe);
    assert_eq!(prompt_shape(&[&a]), PromptShape::UniquePerProbe);
}

// ── Invariant 2: the decision renders a line on every path ─────────────────

#[test]
fn a_measured_rate_above_the_floor_is_verified() {
    let measurement = ProbeMeasurement::measured(1420.0).unwrap();
    let decision = decide_admission("worker-1", &measurement, 100.0).unwrap();
    assert_eq!(decision.branch, AdmissionBranch::Verified);
    assert!(decision
        .line()
        .contains("admitted, verified against the floor"));
}

#[test]
fn the_floor_is_inclusive() {
    let measurement = ProbeMeasurement::measured(100.0).unwrap();
    let decision = decide_admission("worker-1", &measurement, 100.0).unwrap();
    assert_eq!(decision.branch, AdmissionBranch::Verified);
}

#[test]
fn a_measured_rate_below_the_floor_is_refused() {
    let measurement = ProbeMeasurement::measured(6.2).unwrap();
    let decision = decide_admission("worker-1", &measurement, 100.0).unwrap();
    assert_eq!(decision.branch, AdmissionBranch::Refused);
    assert!(decision.line().contains("refused, below the floor"));
}

#[test]
fn a_not_measured_probe_takes_the_bypass() {
    let measurement = ProbeMeasurement::not_measured("probe timed out after 30s").unwrap();
    let decision = decide_admission("worker-1", &measurement, 100.0).unwrap();
    assert_eq!(decision.branch, AdmissionBranch::Bypassed);
    assert!(decision.line().contains("admitted via bypass, unverified"));
}

#[test]
fn the_floor_must_be_positive() {
    let measurement = ProbeMeasurement::measured(100.0).unwrap();
    assert!(decide_admission("worker-1", &measurement, 0.0).is_err());
    assert!(decide_admission("worker-1", &measurement, -1.0).is_err());
    assert!(decide_admission("worker-1", &measurement, f64::NAN).is_err());
}

#[test]
fn an_unusable_rate_is_not_a_measurement() {
    // Zero and negative would fail the floor and refuse a healthy worker;
    // NaN compares false against every floor and would refuse the fastest
    // worker of all.
    assert!(ProbeMeasurement::measured(0.0).is_err());
    assert!(ProbeMeasurement::measured(-6.2).is_err());
    assert!(ProbeMeasurement::measured(f64::NAN).is_err());
    assert!(ProbeMeasurement::measured(6.2).is_ok());
}

#[test]
fn a_not_measured_probe_must_name_its_reason() {
    assert!(ProbeMeasurement::not_measured("").is_err());
    assert!(ProbeMeasurement::not_measured("   ").is_err());
    assert!(ProbeMeasurement::not_measured("probe timed out after 30s").is_ok());
}

#[test]
fn every_branch_renders_a_line_naming_its_inputs() {
    let verified = decide_admission(
        "worker-1",
        &ProbeMeasurement::measured(1420.5).unwrap(),
        100.0,
    )
    .unwrap();
    let refused =
        decide_admission("worker-2", &ProbeMeasurement::measured(6.2).unwrap(), 100.0).unwrap();
    let bypassed = decide_admission(
        "worker-3",
        &ProbeMeasurement::not_measured("probe timed out after 30s").unwrap(),
        100.0,
    )
    .unwrap();

    for decision in [&verified, &refused, &bypassed] {
        let line = decision.line();
        assert!(!line.trim().is_empty(), "no branch may be silent");
        assert!(line.contains(&decision.worker));
        assert!(line.contains(&decision.floor.to_string()));
        let phrase = match decision.branch {
            AdmissionBranch::Verified => "admitted, verified against the floor",
            AdmissionBranch::Refused => "refused, below the floor",
            AdmissionBranch::Bypassed => "admitted via bypass, unverified",
        };
        assert!(line.contains(phrase));
    }
    assert!(verified.line().contains("measured 1420.5 tok/s"));
    assert!(refused.line().contains("measured 6.2 tok/s"));
    assert!(bypassed.line().contains("not measured"));
    assert!(bypassed.line().contains("probe timed out after 30s"));
    // A verified admission and a bypass are different log lines: "it
    // worked" and "it never ran" no longer look identical from outside.
    assert_ne!(verified.line(), bypassed.line());
}

#[test]
fn the_line_naming_helper_agrees_with_the_branches() {
    assert_eq!(AdmissionBranch::Verified.as_str(), "verified");
    assert_eq!(AdmissionBranch::Refused.as_str(), "refused");
    assert_eq!(AdmissionBranch::Bypassed.as_str(), "bypassed");
}

// ── The incident, reconstructed ────────────────────────────────────────────

#[test]
fn a_cpu_bound_worker_cannot_clear_the_floor_on_a_cache_served_probe() {
    // The measured prefill on the CPU-bound worker was ~6 tok/s. A 512-token
    // probe prompt needs 512/6 ≈ 85.3s of prefill — far past any probe bound
    // well below the 140s a direct request already timed out at.
    let prompt_tokens: f64 = 512.0;
    let prefill_tok_s: f64 = 6.0;
    let probe_bound_secs: f64 = 30.0;
    assert!(prompt_tokens / prefill_tok_s > probe_bound_secs);

    // So the probe cannot complete: it is not-measured, and the admission
    // takes the bypass — now as a named branch with a rendered line instead
    // of a silence around a 201.
    let timed_out = ProbeMeasurement::not_measured("probe timed out after 30s").unwrap();
    let bypassed = decide_admission("worker-qwen3.8-flash-next", &timed_out, 100.0).unwrap();
    assert_eq!(bypassed.branch, AdmissionBranch::Bypassed);
    let line = bypassed.line();
    assert!(line.contains("not measured"));
    assert!(line.contains("admitted via bypass, unverified"));

    // And if the rate IS measured — say the prompt cache served it and the
    // "rate" comes back as 6 tok/s of real work — it is refused, not
    // verified: the CPU-bound worker fails the floor it should fail.
    let slow = ProbeMeasurement::measured(6.0).unwrap();
    let refused = decide_admission("worker-qwen3.8-flash-next", &slow, 100.0).unwrap();
    assert_eq!(refused.branch, AdmissionBranch::Refused);

    // The defect itself: with the old constant prompt, the second probe's
    // prompt was identical to the first, so the cache could serve it.
    let constant = BODY;
    assert_eq!(prompt_shape(&[constant, constant]), PromptShape::Reused);
    // With the nonce prefix, the same two probes are unique per probe.
    let first = probe_prompt("probe-1", BODY).unwrap();
    let second = probe_prompt("probe-2", BODY).unwrap();
    assert_eq!(
        prompt_shape(&[&first, &second]),
        PromptShape::UniquePerProbe
    );
    // The decision carries the measurement and the floor it decided on.
    let decision: &AdmissionDecision = &bypassed;
    assert!(matches!(
        decision.measurement,
        ProbeMeasurement::NotMeasured { .. }
    ));
    assert_eq!(decision.floor, 100.0);
}
