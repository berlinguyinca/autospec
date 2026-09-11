//! Environmental preconditions and load-aware selection (issue #4224).
//!
//! The regression tests instantiate the configuration the bug required: a
//! heterogeneous fleet — one model served by workers with differing context
//! windows. On a homogeneous fleet every selection rule agrees, so a test
//! that only ever sees a homogeneous fleet cannot see the bug; that is
//! invariant 2, and it is why the fixture below is deliberately mixed.

use autospec_core::env_preconditions::{
    admit, evaluate, select_worker, verdict, window_mismatches, Admission, Observation,
    Precondition, PreconditionSet, Selection, Verdict, WindowMismatch, WorkerView,
};

const NO_EXCLUDE: &[String] = &[];

/// The incident fleet: one model, two context windows. gw-a1 is the
/// 256k-window worker with no free slots; gw-a2 is the 32k-window worker
/// with four.
fn heterogeneous_fleet() -> Vec<WorkerView> {
    vec![
        WorkerView {
            id: "gw-a1".to_string(),
            model: "qwen3.8-27b".to_string(),
            context_window: 262_144,
            free_slots: 0,
        },
        WorkerView {
            id: "gw-a2".to_string(),
            model: "qwen3.8-27b".to_string(),
            context_window: 32_768,
            free_slots: 4,
        },
    ]
}

/// A control fleet: the same model and load shape, but one window. On this
/// fleet the capability filter changes nothing, which is exactly why a
/// regression test built on it could not catch the incident.
fn homogeneous_fleet() -> Vec<WorkerView> {
    vec![
        WorkerView {
            id: "gw-a1".to_string(),
            model: "qwen3.8-27b".to_string(),
            context_window: 32_768,
            free_slots: 0,
        },
        WorkerView {
            id: "gw-a2".to_string(),
            model: "qwen3.8-27b".to_string(),
            context_window: 32_768,
            free_slots: 4,
        },
    ]
}

// --- Invariant 1: assert the environmental property in code --------------

#[test]
fn a_homogeneous_fleet_reports_no_window_mismatch() {
    assert_eq!(
        window_mismatches(&homogeneous_fleet()),
        Vec::<WindowMismatch>::new()
    );
}

#[test]
fn the_incident_fleet_reports_the_mismatch_and_the_warn_line_names_every_window() {
    let mismatches = window_mismatches(&heterogeneous_fleet());
    assert_eq!(mismatches.len(), 1);
    let mismatch = &mismatches[0];
    assert_eq!(mismatch.model, "qwen3.8-27b");
    assert_eq!(mismatch.distinct, vec![32_768, 262_144]);
    assert_eq!(
        mismatch.reported,
        vec![
            ("gw-a1".to_string(), 262_144),
            ("gw-a2".to_string(), 32_768)
        ]
    );

    let line = mismatch.warn_line();
    assert!(
        line.starts_with("WARN:"),
        "the guard must warn, not log: {line}"
    );
    for expected in [
        "qwen3.8-27b",
        "2 distinct context windows",
        "gw-a1: 262144",
        "gw-a2: 32768",
    ] {
        assert!(
            line.contains(expected),
            "warn line misses {expected:?}: {line}"
        );
    }
}

#[test]
fn mismatches_are_grouped_per_model_not_fleet_wide() {
    let fleet = vec![
        heterogeneous_fleet()[0].clone(),
        heterogeneous_fleet()[1].clone(),
        // A second model, homogeneous: it must not be reported.
        WorkerView {
            id: "gw-b1".to_string(),
            model: "deepseek-v4-flash".to_string(),
            context_window: 32_768,
            free_slots: 2,
        },
        WorkerView {
            id: "gw-b2".to_string(),
            model: "deepseek-v4-flash".to_string(),
            context_window: 32_768,
            free_slots: 1,
        },
    ];
    let mismatches = window_mismatches(&fleet);
    assert_eq!(mismatches.len(), 1);
    assert_eq!(mismatches[0].model, "qwen3.8-27b");
}

#[test]
fn a_violated_precondition_warns_with_the_observed_value() {
    let precondition = Precondition::new(
        "fleet.qwen3.8-27b:ctx-window-homogeneous",
        "all qwen3.8-27b workers report the same context window",
        "window_mismatches(fleet)",
    )
    .unwrap();
    let mismatches = window_mismatches(&heterogeneous_fleet());
    let observed = mismatches[0].warn_line();
    let warnings = evaluate(&precondition, &Observation::Violated { observed });
    assert_eq!(warnings.len(), 1);
    for expected in [
        "WARN:",
        "fleet.qwen3.8-27b:ctx-window-homogeneous",
        "no longer holds",
        "observed: WARN:",
    ] {
        assert!(
            warnings[0].contains(expected),
            "missing {expected:?}: {}",
            warnings[0]
        );
    }
}

#[test]
fn an_unrunnable_assertion_is_fail_closed_not_holds() {
    let precondition = Precondition::new(
        "fleet.qwen3.8-27b:ctx-window-homogeneous",
        "all qwen3.8-27b workers report the same context window",
        "window_mismatches(fleet)",
    )
    .unwrap();
    let warnings = evaluate(
        &precondition,
        &Observation::Unrunnable {
            detail: "fleet state file missing".to_string(),
        },
    );
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].starts_with("WARN:"));
    assert!(warnings[0].contains("could not be re-asserted via window_mismatches(fleet)"));
    assert!(warnings[0].contains("at risk, not as holding"));
}

#[test]
fn a_holding_precondition_warns_about_nothing() {
    let precondition = Precondition::new(
        "fleet.qwen3.8-27b:ctx-window-homogeneous",
        "all qwen3.8-27b workers report the same context window",
        "window_mismatches(fleet)",
    )
    .unwrap();
    assert!(evaluate(&precondition, &Observation::Holds).is_empty());
}

// --- Invariant 3: eligibility filter before ranking filter ---------------

#[test]
fn a_small_request_goes_to_the_eligible_least_loaded_worker_not_the_max_window_worker() {
    // The regression: the load-blind max-window heuristic names gw-a1 here
    // (and holds it — it has no free slots). The eligibility-then-ranking
    // rule names gw-a2, which can serve a 16k request and is free.
    let fleet = heterogeneous_fleet();
    assert_eq!(
        select_worker(&fleet, 16_384, NO_EXCLUDE),
        Selection::Selected {
            worker: "gw-a2".to_string(),
            eligible: true
        }
    );
}

#[test]
fn on_a_homogeneous_fleet_the_capability_filter_changes_nothing_which_is_why_the_regression_must_be_heterogeneous(
) {
    // The control: same request, same load shape, one window. The picker
    // still ranks on free slots — the test passes, and it proves nothing
    // about the incident, which is the point of running the regression
    // above on the mixed fleet instead.
    let fleet = homogeneous_fleet();
    assert_eq!(window_mismatches(&fleet), Vec::<WindowMismatch>::new());
    assert_eq!(
        select_worker(&fleet, 16_384, NO_EXCLUDE),
        Selection::Selected {
            worker: "gw-a2".to_string(),
            eligible: true
        }
    );
}

#[test]
fn a_request_only_the_big_window_can_serve_names_that_worker_and_admission_holds_it() {
    let fleet = heterogeneous_fleet();
    // Selection is total and the named worker is eligible…
    assert_eq!(
        select_worker(&fleet, 200_000, NO_EXCLUDE),
        Selection::Selected {
            worker: "gw-a1".to_string(),
            eligible: true
        }
    );
    // …but the admission decision is separate and refuses a saturated one.
    assert_eq!(admit(&fleet[0]), Admission::Hold);
    assert_eq!(
        verdict(&fleet, 200_000, NO_EXCLUDE),
        Verdict::HeldSaturated {
            worker: "gw-a1".to_string()
        }
    );
}

#[test]
fn a_request_no_worker_can_serve_still_names_a_worker_flagged_incapable() {
    let fleet = heterogeneous_fleet();
    assert_eq!(
        select_worker(&fleet, 999_999_999, NO_EXCLUDE),
        Selection::Selected {
            worker: "gw-a2".to_string(), // least-loaded of the whole fleet
            eligible: false
        }
    );
    // Free slots are not a license to dispatch a worker that cannot serve
    // the request: the verdict names the hold as a capability gap, not as
    // saturation.
    assert_eq!(admit(&fleet[1]), Admission::Dispatch);
    assert_eq!(
        verdict(&fleet, 999_999_999, NO_EXCLUDE),
        Verdict::HeldIncapable {
            worker: "gw-a2".to_string()
        }
    );
}

#[test]
fn the_verdict_dispatches_only_when_eligible_and_admitted() {
    let fleet = heterogeneous_fleet();
    assert_eq!(
        verdict(&fleet, 16_384, NO_EXCLUDE),
        Verdict::Dispatch {
            worker: "gw-a2".to_string()
        }
    );
}

#[test]
fn an_empty_fleet_is_named_not_guessed() {
    assert_eq!(select_worker(&[], 16_384, NO_EXCLUDE), Selection::NoWorkers);
    assert_eq!(verdict(&[], 16_384, NO_EXCLUDE), Verdict::NoWorkers);
}

#[test]
fn a_saturated_fleet_still_names_the_least_loaded_worker() {
    let fleet = vec![
        WorkerView {
            id: "gw-a1".to_string(),
            model: "qwen3.8-27b".to_string(),
            context_window: 32_768,
            free_slots: 0,
        },
        WorkerView {
            id: "gw-a2".to_string(),
            model: "qwen3.8-27b".to_string(),
            context_window: 32_768,
            free_slots: 0,
        },
    ];
    // Every worker is saturated and (in this tie) equally loaded: the
    // pick is deterministic by id, and the hold names the worker.
    assert_eq!(
        select_worker(&fleet, 16_384, NO_EXCLUDE),
        Selection::Selected {
            worker: "gw-a1".to_string(),
            eligible: true
        }
    );
    assert_eq!(
        verdict(&fleet, 16_384, NO_EXCLUDE),
        Verdict::HeldSaturated {
            worker: "gw-a1".to_string()
        }
    );
}

#[test]
fn ties_in_free_slots_break_by_worker_id() {
    let fleet = vec![
        WorkerView {
            id: "gw-b".to_string(),
            model: "qwen3.8-27b".to_string(),
            context_window: 32_768,
            free_slots: 3,
        },
        WorkerView {
            id: "gw-a".to_string(),
            model: "qwen3.8-27b".to_string(),
            context_window: 32_768,
            free_slots: 3,
        },
    ];
    assert_eq!(
        select_worker(&fleet, 16_384, NO_EXCLUDE),
        Selection::Selected {
            worker: "gw-a".to_string(),
            eligible: true
        }
    );
}

#[test]
fn prior_picks_are_excluded_from_the_same_pass() {
    let fleet = heterogeneous_fleet();
    let exclude = vec!["gw-a2".to_string()];
    // gw-a2 already took this pass's slot; the next pick goes to gw-a1 —
    // eligible, and held for lack of slots, rather than re-picked gw-a2.
    assert_eq!(
        verdict(&fleet, 16_384, &exclude),
        Verdict::HeldSaturated {
            worker: "gw-a1".to_string()
        }
    );
    let exclude_all = vec!["gw-a1".to_string(), "gw-a2".to_string()];
    assert_eq!(
        select_worker(&fleet, 16_384, &exclude_all),
        Selection::NoWorkers
    );
}

// --- Invariant 4: record the conditions under which the fix holds --------

#[test]
fn a_precondition_renders_the_valid_while_line() {
    let precondition = Precondition::new(
        "fleet.qwen3.8-27b:ctx-window-homogeneous",
        "all qwen3.8-27b workers report the same context window",
        "window_mismatches(fleet)",
    )
    .unwrap();
    assert_eq!(
        precondition.line(),
        "Valid while: all qwen3.8-27b workers report the same context window (asserted by window_mismatches(fleet))"
    );
}

#[test]
fn a_precondition_without_a_named_assertion_is_rejected() {
    let err = Precondition::new("id", "some property", "   ").unwrap_err();
    assert!(err.contains("no assertion"), "{err}");
    let err = Precondition::new("id", "some property", "").unwrap_err();
    assert!(err.contains("no assertion"), "{err}");
}

#[test]
fn a_precondition_with_a_blank_property_is_rejected() {
    let err = Precondition::new("id", "  ", "some check").unwrap_err();
    assert!(err.contains("must state the property"), "{err}");
    let err = Precondition::new("  ", "some property", "some check").unwrap_err();
    assert!(err.contains("id must not be blank"), "{err}");
}

#[test]
fn a_set_renders_one_valid_while_line_per_precondition_and_an_empty_set_renders_nothing() {
    let mut set = PreconditionSet::new();
    assert!(set.is_empty());
    assert!(set.valid_while_lines().is_empty());

    set.add(
        Precondition::new(
            "fleet.qwen3.8-27b:ctx-window-homogeneous",
            "all qwen3.8-27b workers report the same context window",
            "window_mismatches(fleet)",
        )
        .unwrap(),
    );
    set.add(
        Precondition::new(
            "gateway:single-node",
            "the gateway runs on a single node",
            "gateway inventory count",
        )
        .unwrap(),
    );
    assert_eq!(set.len(), 2);
    assert_eq!(
        set.valid_while_lines(),
        vec![
            "Valid while: all qwen3.8-27b workers report the same context window (asserted by window_mismatches(fleet))",
            "Valid while: the gateway runs on a single node (asserted by gateway inventory count)",
        ]
    );
    assert_eq!(
        set.get("gateway:single-node").unwrap().asserted_by,
        "gateway inventory count"
    );
}

#[test]
fn re_adding_the_same_id_supersedes_not_duplicates() {
    let mut set = PreconditionSet::new();
    set.add(Precondition::new("id", "old property", "old check").unwrap());
    set.add(Precondition::new("id", "new property", "new check").unwrap());
    assert_eq!(set.len(), 1);
    assert_eq!(
        set.get("id").unwrap().line(),
        "Valid while: new property (asserted by new check)"
    );
}

#[test]
fn preconditions_round_trip_through_json() {
    let mut set = PreconditionSet::new();
    set.add(
        Precondition::new(
            "fleet.qwen3.8-27b:ctx-window-homogeneous",
            "all qwen3.8-27b workers report the same context window",
            "window_mismatches(fleet)",
        )
        .unwrap(),
    );
    let raw = serde_json::to_string(&set).unwrap();
    let back: PreconditionSet = serde_json::from_str(&raw).unwrap();
    assert_eq!(back, set);
    // The empty set also round-trips: the absence of preconditions is a
    // recordable state, not a missing file.
    let empty_raw = serde_json::to_string(&PreconditionSet::new()).unwrap();
    let empty_back: PreconditionSet = serde_json::from_str(&empty_raw).unwrap();
    assert!(empty_back.is_empty());
}

// --- The incident, end to end --------------------------------------------

#[test]
fn the_incident_reproduces_and_the_guard_names_it_on_the_same_pass() {
    let fleet = heterogeneous_fleet();

    // The property the fix rested on no longer holds, and the check says
    // so with the windows spelled out.
    let mismatches = window_mismatches(&fleet);
    assert_eq!(mismatches.len(), 1);

    // A small request is no longer stuck on the big window's worker: it
    // dispatches to the free 32k worker that can serve it.
    assert_eq!(
        verdict(&fleet, 16_384, NO_EXCLUDE),
        Verdict::Dispatch {
            worker: "gw-a2".to_string()
        }
    );

    // And the closeout of the fix records what it assumes, asserted by the
    // check above — so the next fleet change that breaks the assumption is
    // a warning, not a silent re-version.
    let precondition = Precondition::new(
        "fleet.qwen3.8-27b:ctx-window-homogeneous",
        "all qwen3.8-27b workers report the same context window",
        "window_mismatches(fleet)",
    )
    .unwrap();
    let warnings = evaluate(
        &precondition,
        &Observation::Violated {
            observed: mismatches[0].warn_line(),
        },
    );
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("not valid for this fleet"));
}
