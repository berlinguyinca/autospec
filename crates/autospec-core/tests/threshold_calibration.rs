//! Measured thresholds and capped destructive automation (issue #4276).
//!
//! The regression tests instantiate the configuration the incident
//! produced: a threshold derived from the callee's default (2700 s) plus
//! grace, sitting in the bulk of the measured distribution, applied to a
//! confoundable signal, by an uncapped-until-too-late destructive
//! automation. On a threshold in the tail, built from the operating value,
//! on an unconfoundable signal, every gate passes — which is the invariant
//! the incident violated on all five counts at once.

use autospec_core::threshold_calibration::{
    build_gate, cap_verdict, cap_verdict_line, confoundable, CallSite, CapVerdict, DetectorSignal,
    Distribution, KillLedger, KillVerdict, LimitProvenance, ThresholdAudit, ThresholdPlacement,
};

// --- The incident's numbers ------------------------------------------------

/// The measured distribution from the incident: 418 completed runs,
/// median 4077 s, p90 13939 s, max 25201 s (the 7 h limit), with 190 of
/// 418 runs longer than the 4500 s (75 min) threshold — 45% of healthy
/// work.
fn incident_samples() -> Vec<u64> {
    let mut v: Vec<u64> = Vec::with_capacity(418);
    // 208 values below the median (max 500 + 207*17 = 4019 < 4077).
    for i in 0..208u64 {
        v.push(500 + i * 17);
    }
    // index 208 (nearest-rank p50 of 418): the median.
    v.push(4077);
    // 19 more values at or below the 4500 s threshold: 228 total <= 4500.
    for i in 0..19u64 {
        v.push(4078 + i);
    }
    // 148 values above the threshold but below p90.
    for i in 0..148u64 {
        v.push(4501 + i * 64);
    }
    // index 376 (nearest-rank p90 of 418): p90.
    v.push(13939);
    // 40 values above p90 but below the max.
    for i in 0..40u64 {
        v.push(13940 + i * 280);
    }
    // The maximum: the 7 h operating limit.
    v.push(25201);
    debug_assert_eq!(v.len(), 418);
    v
}

fn incident_distribution() -> Distribution {
    Distribution::from_samples(&incident_samples()).expect("non-empty sample")
}

/// The limit variable from the incident: the callee defaults to
/// `LIMIT:-2700` (45 min); the dispatcher's call site submits
/// `LIMIT=25200` (7 h).
fn incident_limit() -> LimitProvenance {
    LimitProvenance {
        variable: "LIMIT".to_string(),
        default: Some(2700),
        callers: vec![CallSite {
            site: "topup.sh submit".to_string(),
            value: 25200,
        }],
    }
}

// --- Rule 1: measure the distribution before building a detector ----------

#[test]
fn the_incident_threshold_sits_in_the_bulk_not_the_tail() {
    let dist = incident_distribution();
    assert_eq!(dist.count, 418);
    assert_eq!(dist.median, 4077);
    assert_eq!(dist.p90, 13939);
    assert_eq!(dist.max, 25201);

    // The 75 min (4500 s) threshold sits just above the median (4077 s)
    // and far below p90 (13939 s): it is in the bulk of normal work, not
    // the tail.
    let audit = ThresholdAudit::new(4500, dist);
    assert_eq!(audit.placement(), ThresholdPlacement::Bulk);
    assert!(!audit.is_outlier_threshold());
}

#[test]
fn a_threshold_at_or_below_the_median_is_a_coin_flip_on_healthy_work() {
    let dist = incident_distribution();
    // 4000 s: at or below the median (4077 s) — kills half or more of
    // normal work.
    let audit = ThresholdAudit::new(4000, dist);
    assert_eq!(audit.placement(), ThresholdPlacement::Median);
    assert!(!audit.is_outlier_threshold());
}

#[test]
fn the_incident_threshold_would_have_killed_45_percent_of_healthy_work() {
    let samples = incident_samples();
    let audit = ThresholdAudit::new(4500, incident_distribution());
    let killed = audit.would_kill(&samples);
    assert_eq!(killed, 190);
    assert!((killed as f64 / samples.len() as f64 - 0.454).abs() < 0.01);
}

#[test]
fn a_threshold_beyond_p90_is_a_tail_threshold() {
    let dist = incident_distribution();
    let audit = ThresholdAudit::new(25_500, dist);
    assert_eq!(audit.placement(), ThresholdPlacement::Tail);
    assert!(audit.is_outlier_threshold());
    // The 7 h limit (25201 s) is the maximum observed; a threshold beyond
    // it kills nothing — which is where a destructive threshold belongs.
    assert_eq!(audit.would_kill(&incident_samples()), 0);
}

#[test]
fn a_threshold_between_median_and_p90_is_bulk_not_tail() {
    let dist = incident_distribution();
    // 10_000 s: above the median (4077), at or below p90 (13939).
    let audit = ThresholdAudit::new(10_000, dist);
    assert_eq!(audit.placement(), ThresholdPlacement::Bulk);
    assert!(!audit.is_outlier_threshold());
}

#[test]
fn a_distribution_from_an_empty_sample_is_refused() {
    let err = Distribution::from_samples(&[]).unwrap_err();
    assert!(err.contains("empty sample"), "got: {err}");
}

#[test]
fn the_audit_line_names_threshold_and_placement() {
    let audit = ThresholdAudit::new(4500, incident_distribution());
    let line = audit.line();
    assert!(line.contains("4500s"), "got: {line}");
    assert!(line.contains("418 runs"), "got: {line}");
    assert!(line.contains("median 4077s"), "got: {line}");
    assert!(line.contains("p90 13939s"), "got: {line}");
    assert!(line.contains("max 25201s"), "got: {line}");
    assert!(line.contains("Bulk"), "got: {line}");
}

#[test]
fn the_bulk_refusal_line_names_p90() {
    let audit = ThresholdAudit::new(4500, incident_distribution());
    let line = audit.refusal_line();
    assert!(line.contains("p90"), "got: {line}");
    assert!(line.contains("13939"), "got: {line}");
    assert!(line.contains("healthy work"), "got: {line}");
}

#[test]
fn the_median_refusal_line_names_the_median() {
    let audit = ThresholdAudit::new(4000, incident_distribution());
    let line = audit.refusal_line();
    assert!(line.contains("median"), "got: {line}");
    assert!(line.contains("4077"), "got: {line}");
    assert!(line.contains("not an outlier detector"), "got: {line}");
}

// --- Rule 2: read the caller before trusting a default ---------------------

#[test]
fn the_default_is_not_the_operating_value_when_a_caller_overrides_it() {
    let limit = incident_limit();
    assert_eq!(limit.operating_value(), Some(25200));
    assert!(!limit.default_is_operating());

    let line = limit.mismatch_line().expect("default and caller disagree");
    assert!(line.contains("LIMIT"), "got: {line}");
    assert!(line.contains("2700"), "got: {line}");
    assert!(line.contains("topup.sh submit"), "got: {line}");
    assert!(line.contains("25200"), "got: {line}");
}

#[test]
fn the_default_is_operating_only_when_no_caller_overrides_it() {
    let limit = LimitProvenance {
        variable: "LIMIT".to_string(),
        default: Some(2700),
        callers: vec![],
    };
    assert!(limit.default_is_operating());
    assert_eq!(limit.operating_value(), Some(2700));
    assert!(limit.mismatch_line().is_none());
}

#[test]
fn agreeing_callers_yield_a_single_operating_value() {
    let limit = LimitProvenance {
        variable: "LIMIT".to_string(),
        default: Some(2700),
        callers: vec![
            CallSite {
                site: "a".into(),
                value: 25200,
            },
            CallSite {
                site: "b".into(),
                value: 25200,
            },
        ],
    };
    assert_eq!(limit.operating_value(), Some(25200));
    assert!(!limit.default_is_operating());
}

#[test]
fn disagreeing_callers_have_no_operating_value() {
    let limit = LimitProvenance {
        variable: "LIMIT".to_string(),
        default: Some(2700),
        callers: vec![
            CallSite {
                site: "a".into(),
                value: 25200,
            },
            CallSite {
                site: "b".into(),
                value: 5400,
            },
        ],
    };
    assert_eq!(limit.operating_value(), None);
    let line = limit.no_operating_value_line();
    assert!(line.contains("LIMIT"), "got: {line}");
    assert!(line.contains("disagree"), "got: {line}");
    assert!(line.contains("5400"), "got: {line}");
    assert!(line.contains("25200"), "got: {line}");
}

#[test]
fn no_default_and_no_caller_fails_closed() {
    let limit = LimitProvenance {
        variable: "LIMIT".to_string(),
        default: None,
        callers: vec![],
    };
    assert_eq!(limit.operating_value(), None);
    let line = limit.no_operating_value_line();
    assert!(line.contains("read the caller"), "got: {line}");
}

// --- Rule 3: prefer a detector normal operation cannot confound ------------

#[test]
fn output_silence_is_always_confoundable() {
    assert!(confoundable(
        DetectorSignal::OutputSilence,
        4500,
        Some(25200)
    ));
    assert!(confoundable(DetectorSignal::OutputSilence, 999_999, None));
}

#[test]
fn past_own_limit_below_the_operating_limit_is_confoundable() {
    // The incident: threshold 4500 s, operating limit 25200 s. The
    // detector fires 6 hours before the dispatcher's own timeout.
    assert!(confoundable(
        DetectorSignal::PastOwnLimit,
        4500,
        Some(25200)
    ));
}

#[test]
fn past_own_limit_beyond_the_operating_limit_is_not_confoundable() {
    assert!(!confoundable(
        DetectorSignal::PastOwnLimit,
        25_500,
        Some(25200)
    ));
}

#[test]
fn past_own_limit_with_unknown_limit_fails_closed() {
    assert!(confoundable(DetectorSignal::PastOwnLimit, 25_500, None));
}

// --- Rules 4 + 5: the build gate -------------------------------------------

#[test]
fn the_incident_detector_is_refused_on_every_count() {
    // The detector as built: threshold 4500 s (default 2700 + grace),
    // signal "no output for 15 min", limit read from the callee's default.
    let audit = ThresholdAudit::new(4500, incident_distribution());
    let limit = incident_limit();
    let refusals = build_gate(&audit, &limit, DetectorSignal::OutputSilence);

    // Bulk placement (kills 45% of normal work)...
    assert!(
        refusals
            .iter()
            .any(|r| r.contains("p90") && r.contains("13939")),
        "refusals: {refusals:?}"
    );
    // ...racing the operating limit...
    assert!(
        refusals
            .iter()
            .any(|r| r.contains("25200") && r.contains("race")),
        "refusals: {refusals:?}"
    );
    // ...on a confoundable signal.
    assert!(
        refusals.iter().any(|r| r.contains("OutputSilence")),
        "refusals: {refusals:?}"
    );
}

#[test]
fn a_tail_threshold_on_past_own_limit_passes_the_gate() {
    // The detector the incident should have been: threshold beyond the
    // operating limit (25200 + grace), unconfoundable signal.
    let audit = ThresholdAudit::new(25_500, incident_distribution());
    let limit = incident_limit();
    let refusals = build_gate(&audit, &limit, DetectorSignal::PastOwnLimit);
    assert!(
        refusals.is_empty(),
        "expected the gate to pass, got: {refusals:?}"
    );
}

#[test]
fn a_tail_threshold_still_refused_on_a_confoundable_signal() {
    // Even a well-placed threshold cannot fix a confoundable signal.
    let audit = ThresholdAudit::new(25_500, incident_distribution());
    let limit = incident_limit();
    let refusals = build_gate(&audit, &limit, DetectorSignal::OutputSilence);
    assert_eq!(refusals.len(), 1, "refusals: {refusals:?}");
    assert!(refusals[0].contains("OutputSilence"));
}

#[test]
fn a_default_only_limit_below_the_threshold_passes_when_tail_calibrated() {
    // No caller overrides the default: the default is the operating
    // value, and a tail threshold beyond it is a valid outlier detector.
    let dist = incident_distribution();
    let limit = LimitProvenance {
        variable: "LIMIT".to_string(),
        default: Some(4000),
        callers: vec![],
    };
    let audit = ThresholdAudit::new(25_500, dist);
    let refusals = build_gate(&audit, &limit, DetectorSignal::PastOwnLimit);
    assert!(refusals.is_empty(), "refusals: {refusals:?}");
}

#[test]
fn a_threshold_at_the_operating_limit_is_refused_not_accepted() {
    // "At or above" is the wrong comparison: at the limit, the detector
    // fires the same moment the dispatcher's own timeout would, and
    // between the two they race.
    let audit = ThresholdAudit::new(25200, incident_distribution());
    let limit = incident_limit();
    let refusals = build_gate(&audit, &limit, DetectorSignal::PastOwnLimit);
    assert!(
        refusals.iter().any(|r| r.contains("race")),
        "refusals: {refusals:?}"
    );
}

// --- Rule 4: cap destructive automation and log it --------------------------

#[test]
fn a_zero_cap_is_refused() {
    let err = KillLedger::new(0).unwrap_err();
    assert!(err.contains("at least 1"), "got: {err}");
}

#[test]
fn the_ledger_records_up_to_the_cap_then_refuses() {
    let mut ledger = KillLedger::new(5).unwrap();
    for i in 0..5 {
        assert_eq!(
            ledger.record(format!("agent-{i}"), 4500 + i),
            KillVerdict::Recorded
        );
    }
    assert_eq!(ledger.len(), 5);
    assert!(ledger.cap_hit());
    assert_eq!(ledger.record("agent-5", 4600), KillVerdict::CapReached);
    assert_eq!(ledger.len(), 5, "the cap must not be exceeded");
}

#[test]
fn every_kill_is_logged_with_its_position_in_the_cap() {
    let mut ledger = KillLedger::new(5).unwrap();
    ledger.record("agent-a", 4500);
    ledger.record("agent-b", 4600);
    assert_eq!(
        ledger.last_line().as_deref(),
        Some("kill 2/5: agent-b (elapsed 4600s)")
    );
}

#[test]
fn an_empty_ledger_has_no_last_line() {
    let ledger = KillLedger::new(5).unwrap();
    assert!(ledger.is_empty());
    assert!(ledger.last_line().is_none());
}

#[test]
fn the_cap_line_names_the_stop_and_the_remeasure() {
    let mut ledger = KillLedger::new(5).unwrap();
    for i in 0..5 {
        ledger.record(format!("agent-{i}"), 4500 + i);
    }
    let line = ledger.cap_line();
    assert!(line.contains("5/5"), "got: {line}");
    assert!(line.contains("stop"), "got: {line}");
    assert!(line.contains("re-measure"), "got: {line}");
}

// --- The cap as a measurement window ----------------------------------------

#[test]
fn a_cap_hit_killing_agents_at_the_median_is_a_detector_defect() {
    // The incident's shape: the ledger fills with kills at ~75 min
    // (4500-4504 s), while the fleet's measured p90 is 13939 s. The
    // detector, not the fleet, is the defect.
    let dist = incident_distribution();
    let mut ledger = KillLedger::new(5).unwrap();
    for i in 0..5 {
        ledger.record(format!("agent-{i}"), 4500 + i);
    }
    assert_eq!(cap_verdict(&ledger, &dist), CapVerdict::DetectorDefect);

    let line = cap_verdict_line(&ledger, &dist);
    assert!(line.contains("13939"), "got: {line}");
    assert!(line.contains("normal work"), "got: {line}");
}

#[test]
fn a_cap_hit_killing_agents_beyond_p90_is_the_cap_doing_its_job() {
    let dist = incident_distribution();
    let mut ledger = KillLedger::new(5).unwrap();
    // Kills at 26000 s: beyond p90 (13939), near the 7 h limit.
    for i in 0..5 {
        ledger.record(format!("agent-{i}"), 26_000 + i * 10);
    }
    assert_eq!(cap_verdict(&ledger, &dist), CapVerdict::Tail);
    let line = cap_verdict_line(&ledger, &dist);
    assert!(line.contains("did"), "got: {line}");
}

#[test]
fn a_ledger_below_the_cap_has_no_conclusion() {
    let dist = incident_distribution();
    let mut ledger = KillLedger::new(5).unwrap();
    ledger.record("agent-0", 4500);
    ledger.record("agent-1", 4600);
    assert_eq!(cap_verdict(&ledger, &dist), CapVerdict::NotFull);
}
