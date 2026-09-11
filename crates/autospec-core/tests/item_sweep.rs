//! A retry job looping over independent items must collect every result and
//! report state, not attempt (issue #4242).
//!
//! The incident: a 60-second systemd unit registered the fleet's models via
//! `curl` under `set -euo pipefail`. One timed-out item aborted every
//! remaining registration in the round, and a unit that failed most of the
//! time had taught its operator to treat red as the resting state.
//!
//! Invariants under test:
//! - Invariant 1: one item's failure does not end the loop; the fold names
//!   every failure, and an unattempted item is a failed item, not an absence
//!   (`fold_item_results`).
//! - Invariant 2: a retry job's exit status describes state, not attempt;
//!   3/4, 0/4, and "already registered and still alive" are three distinct
//!   states (`SweepState::exit_code`, `SweepState::summary_line`).
//! - Invariant 3: the per-item timeout belongs to the item, not the run; the
//!   overall budget must strictly exceed the worst case
//!   (`validate_sweep_budget`).
//! - Invariant 4: alert on the transition, not the state; a persisted
//!   failure set re-alerts only when the streak crosses the threshold
//!   (`FailureLedger::record_run`).

use autospec_core::item_sweep::{
    diff_failure_sets, fold_item_results, validate_sweep_budget, AlertDecision, FailureLedger,
    FailureTransition, ItemOutcome, ItemResult, SweepBudget, NOT_ATTEMPTED,
};
use std::time::Duration;

fn item(name: &str) -> String {
    name.to_string()
}

fn registered(name: &str) -> ItemResult {
    ItemResult {
        item: item(name),
        outcome: ItemOutcome::Registered,
    }
}

fn already_alive(name: &str) -> ItemResult {
    ItemResult {
        item: item(name),
        outcome: ItemOutcome::AlreadyAlive,
    }
}

fn failed(name: &str, reason: &str) -> ItemResult {
    ItemResult {
        item: item(name),
        outcome: ItemOutcome::Failed {
            reason: reason.to_string(),
        },
    }
}

fn names_of(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| item(s)).collect()
}

// ---------------------------------------------------------------------------
// Invariant 1: one item's failure does not end the loop
// ---------------------------------------------------------------------------

#[test]
fn the_incident_shape_is_a_value() {
    // Eight models declared; item 4 times out under `set -e`; items 5–8 are
    // never attempted, so they have no result at all.
    let declared = names_of(&[
        "deepseek-v4-flash",
        "deepseek-v4-pro",
        "glm-5.2",
        "kimi-k3",
        "kimi-k3-thinking",
        "qwen3.5-397b",
        "qwen3.6-27b",
        "qwen3.8-27b",
    ]);
    let results = vec![
        registered("deepseek-v4-flash"),
        registered("deepseek-v4-pro"),
        registered("glm-5.2"),
        failed("kimi-k3", "timeout after 30s (000)"),
    ];

    let state = fold_item_results(&declared, &results).expect("fold must succeed");

    // The loop's abort is a *named* part of the state, not a shrunk fleet:
    // five items are failing — one timed out, four never attempted.
    let failed_items = state.failed_items();
    assert_eq!(
        failed_items.len(),
        5,
        "all five failures must be named: {state:?}"
    );
    assert_eq!(failed_items[0].item, "kimi-k3");
    assert_eq!(failed_items[0].reason, "timeout after 30s (000)");
    for (idx, name) in [
        "kimi-k3-thinking",
        "qwen3.5-397b",
        "qwen3.6-27b",
        "qwen3.8-27b",
    ]
    .iter()
    .enumerate()
    {
        assert_eq!(failed_items[idx + 1].item.as_str(), *name);
        assert_eq!(
            failed_items[idx + 1].reason,
            NOT_ATTEMPTED,
            "{name} was never attempted"
        );
    }
    assert_eq!(state.total(), 8);
    assert_eq!(state.healthy(), 3);
    assert_eq!(
        state,
        autospec_core::item_sweep::SweepState::Partial {
            total: 8,
            failed: failed_items.to_vec(),
        }
    );
}

#[test]
fn a_failed_item_does_not_drop_the_remaining_items() {
    // The pre-fix shape: `set -e` stopped the loop at item 2, and the report
    // covered only what ran. Now every declared item is in the state.
    let declared = names_of(&["a", "b", "c"]);
    let results = vec![registered("a"), failed("b", "connection refused")];

    let state = fold_item_results(&declared, &results).expect("fold must succeed");
    assert_eq!(
        state.total(),
        3,
        "the state covers all declared items, not just the attempted ones"
    );
    assert_eq!(state.healthy(), 1);
    let failed_items = state.failed_items();
    assert_eq!(failed_items.len(), 2);
    assert_eq!(failed_items[0].item, "b");
    assert_eq!(failed_items[0].reason, "connection refused");
    assert_eq!(failed_items[1].item, "c");
    assert_eq!(failed_items[1].reason, NOT_ATTEMPTED);
}

#[test]
fn the_failure_list_is_in_declared_order_regardless_of_result_order() {
    let declared = names_of(&["a", "b", "c", "d"]);
    let results = vec![
        failed("d", "timeout after 30s (000)"),
        registered("a"),
        failed("b", "http 503"),
        registered("c"),
    ];

    let state = fold_item_results(&declared, &results).expect("fold must succeed");
    let failed_items = state.failed_items();
    assert_eq!(
        failed_items
            .iter()
            .map(|f| f.item.as_str())
            .collect::<Vec<_>>(),
        vec!["b", "d"],
        "failures must appear in declared order"
    );
}

#[test]
fn a_result_for_undeclared_item_is_a_report_defect() {
    let declared = names_of(&["a", "b"]);
    let results = vec![registered("a"), registered("c")];

    let err = fold_item_results(&declared, &results)
        .expect_err("the report must cover exactly the declared items");
    assert!(err.contains("undeclared item 'c'"), "{err}");
}

#[test]
fn a_duplicate_result_is_a_report_defect() {
    let declared = names_of(&["a", "b"]);
    let results = vec![registered("a"), failed("a", "http 503"), already_alive("b")];

    let err = fold_item_results(&declared, &results).expect_err("one result per item per run");
    assert!(err.contains("duplicate results for item 'a'"), "{err}");
}

#[test]
fn a_sweep_over_zero_items_is_a_misconfiguration() {
    let err = fold_item_results(&[], &[]).expect_err("zero declared items is not a no-op");
    assert!(err.contains("zero declared items"), "{err}");
}

// ---------------------------------------------------------------------------
// Invariant 2: exit status describes state, not attempt
// ---------------------------------------------------------------------------

#[test]
fn three_over_four_zero_over_four_and_noop_are_three_states() {
    // 3/4: one item failed.
    let declared = names_of(&["a", "b", "c", "d"]);
    let partial = fold_item_results(
        &declared,
        &[
            registered("a"),
            registered("b"),
            registered("c"),
            failed("d", "timeout after 30s (000)"),
        ],
    )
    .expect("fold must succeed");

    // 0/4: every item failed.
    let all_failed = fold_item_results(
        &declared,
        &[
            failed("a", "timeout after 30s (000)"),
            failed("b", "http 503"),
            failed("c", "not attempted (loop aborted before this item)"),
            failed("d", "not attempted (loop aborted before this item)"),
        ],
    )
    .expect("fold must succeed");

    // No-op: everything was already registered and still alive.
    let no_op = fold_item_results(
        &declared,
        &[
            already_alive("a"),
            already_alive("b"),
            already_alive("c"),
            already_alive("d"),
        ],
    )
    .expect("fold must succeed");

    // Three different states.
    assert_ne!(partial, all_failed);
    assert_ne!(partial, no_op);
    assert_ne!(all_failed, no_op);

    // The success states exit 0; the failure states exit 1 — and partial is
    // NOT a success, even though three of four items are healthy.
    assert_eq!(no_op.exit_code(), 0);
    assert_eq!(
        partial.exit_code(),
        1,
        "3/4 is a degraded fleet, not a success"
    );
    assert_eq!(all_failed.exit_code(), 1);

    // And the three states render three different lines.
    let lines = [
        partial.summary_line(),
        all_failed.summary_line(),
        no_op.summary_line(),
    ];
    assert_ne!(lines[0], lines[1]);
    assert_ne!(lines[0], lines[2]);
    assert_ne!(lines[1], lines[2]);
}

#[test]
fn the_report_line_names_the_state_the_counts_and_every_failure() {
    let declared = names_of(&["a", "b", "c"]);
    let state = fold_item_results(
        &declared,
        &[registered("a"), failed("b", "http 503"), already_alive("c")],
    )
    .expect("fold must succeed");

    let line = state.summary_line();
    assert_eq!(
        line,
        "sweep: 2/3 registered and alive; failed: b (http 503)"
    );
}

#[test]
fn all_healthy_reports_how_much_work_the_run_did() {
    let declared = names_of(&["a", "b", "c"]);
    let state = fold_item_results(
        &declared,
        &[registered("a"), registered("b"), already_alive("c")],
    )
    .expect("fold must succeed");

    assert_eq!(
        state,
        autospec_core::item_sweep::SweepState::AllHealthy {
            total: 3,
            registered_now: 2,
        }
    );
    assert_eq!(state.exit_code(), 0);
    assert_eq!(
        state.summary_line(),
        "sweep: 3/3 registered and alive (2 registered this run)"
    );
}

#[test]
fn no_op_reports_no_work_done() {
    let declared = names_of(&["a", "b"]);
    let state = fold_item_results(&declared, &[already_alive("a"), already_alive("b")])
        .expect("fold must succeed");

    assert_eq!(
        state,
        autospec_core::item_sweep::SweepState::AlreadyUpToDate { total: 2 }
    );
    assert_eq!(
        state.summary_line(),
        "sweep: 2/2 already registered and alive (no work done)"
    );
}

#[test]
fn total_failure_reports_zero_over_total() {
    let declared = names_of(&["a", "b"]);
    let state = fold_item_results(
        &declared,
        &[
            failed("a", "timeout after 30s (000)"),
            failed("b", "timeout after 30s (000)"),
        ],
    )
    .expect("fold must succeed");

    assert_eq!(state.healthy(), 0);
    assert!(
        state
            .summary_line()
            .starts_with("sweep: 0/2 registered and alive; failed: "),
        "{}",
        state.summary_line()
    );
}

// ---------------------------------------------------------------------------
// Invariant 3: the per-item timeout belongs to the item, not the run
// ---------------------------------------------------------------------------

#[test]
fn a_budget_that_covers_the_worst_case_passes() {
    // Eight items at 30s each: 240s worst case; 300s overall is strictly
    // above it.
    let budget = SweepBudget {
        per_item: Duration::from_secs(30),
        overall: Duration::from_secs(300),
    };
    assert!(
        validate_sweep_budget(&budget, 8).is_ok(),
        "300s overall covers 8 x 30s"
    );
}

#[test]
fn an_overall_bound_equal_to_the_worst_case_is_a_violation() {
    // 4 x 30s = 120s exactly: a run that reaches its budget exactly is a run
    // that hit its own deadline — the #3637 race, one level up.
    let budget = SweepBudget {
        per_item: Duration::from_secs(30),
        overall: Duration::from_secs(120),
    };
    let violation = validate_sweep_budget(&budget, 4).expect_err("equality is not enough");
    assert!(
        violation.reason.contains("strictly exceed"),
        "{}",
        violation.line()
    );
    assert_eq!(violation.items, 4);
    assert_eq!(violation.per_item, Duration::from_secs(30));
    assert_eq!(violation.overall, Duration::from_secs(120));
}

#[test]
fn an_overall_bound_below_the_worst_case_is_a_violation() {
    let budget = SweepBudget {
        per_item: Duration::from_secs(30),
        overall: Duration::from_secs(90),
    };
    assert!(
        validate_sweep_budget(&budget, 4).is_err(),
        "90s cannot cover 4 items that may each take 30s"
    );
}

#[test]
fn an_item_without_a_per_item_bound_is_a_violation() {
    // The incident itself: a curl with no --max-time. The per-item bound is
    // zero, so nothing bounds that call but the whole run.
    let budget = SweepBudget {
        per_item: Duration::from_secs(0),
        overall: Duration::from_secs(300),
    };
    let violation = validate_sweep_budget(&budget, 8).expect_err("per-item bound is mandatory");
    assert!(
        violation.reason.contains("per-item bound"),
        "{}",
        violation.line()
    );
}

#[test]
fn a_run_without_an_overall_bound_is_a_violation() {
    let budget = SweepBudget {
        per_item: Duration::from_secs(30),
        overall: Duration::from_secs(0),
    };
    let violation = validate_sweep_budget(&budget, 4).expect_err("overall bound is mandatory");
    assert!(
        violation.reason.contains("overall bound"),
        "{}",
        violation.line()
    );
}

#[test]
fn a_budget_over_zero_items_is_a_violation() {
    let budget = SweepBudget {
        per_item: Duration::from_secs(30),
        overall: Duration::from_secs(300),
    };
    assert!(validate_sweep_budget(&budget, 0).is_err());
}

#[test]
fn the_violation_line_names_every_number() {
    let budget = SweepBudget {
        per_item: Duration::from_secs(30),
        overall: Duration::from_secs(90),
    };
    let violation = validate_sweep_budget(&budget, 4).expect_err("90s < 120s worst case");
    let line = violation.line();
    assert!(line.contains("per-item 30s"), "{line}");
    assert!(line.contains("4 items"), "{line}");
    assert!(line.contains("overall 90s"), "{line}");
}

// ---------------------------------------------------------------------------
// Invariant 4: alert on the transition, not the state
// ---------------------------------------------------------------------------

#[test]
fn steady_runs_do_not_alert() {
    let mut ledger = FailureLedger::with_threshold(6).expect("threshold 6 is valid");
    for _ in 0..3 {
        let decision = ledger.record_run(&[]);
        assert!(
            !decision.fires(),
            "steady runs must not alert: {decision:?}"
        );
    }
}

#[test]
fn the_first_healthy_observation_does_not_alert() {
    let mut ledger = FailureLedger::with_threshold(6).expect("threshold 6 is valid");
    let decision = ledger.record_run(&[]);
    assert!(!decision.fires(), "{decision:?}");
}

#[test]
fn the_first_failing_observation_does_alert() {
    // The first observed run has no previous state; a failure is a
    // transition from unknown, and unknown is not a reason to stay quiet.
    let mut ledger = FailureLedger::with_threshold(6).expect("threshold 6 is valid");
    let decision = ledger.record_run(&[item("kimi-k3")]);
    match &decision {
        AlertDecision::Fire { line } => {
            assert!(line.contains("first observed run"), "{line}");
            assert!(line.contains("kimi-k3"), "{line}");
        }
        other => panic!("first failing run must fire: {other:?}"),
    }
    assert_eq!(ledger.streak(), 1);
}

#[test]
fn an_onset_alerts_and_names_every_new_failure() {
    let mut ledger = FailureLedger::with_threshold(6).expect("threshold 6 is valid");
    ledger.record_run(&[]);

    let decision = ledger.record_run(&[item("a"), item("b")]);
    match &decision {
        AlertDecision::Fire { line } => {
            assert!(line.contains("newly failing"), "{line}");
            assert!(line.contains("a") && line.contains("b"), "{line}");
        }
        other => panic!("onset must fire: {other:?}"),
    }
    assert_eq!(ledger.streak(), 1);
}

#[test]
fn a_persisted_failure_set_is_quiet_until_the_threshold() {
    // The usually-red unit: the same failure set, run after run. With a
    // threshold of 6, runs 1–5 are quiet (the state is unchanged and was
    // already reported); only the crossing of the threshold alerts.
    let mut ledger = FailureLedger::with_threshold(6).expect("threshold 6 is valid");
    let failing = vec![item("kimi-k3"), item("qwen3.8-27b")];

    let first = ledger.record_run(&failing);
    assert!(
        first.fires(),
        "the first failing observation fires: {first:?}"
    );

    for run in 2..=5 {
        let decision = ledger.record_run(&failing);
        assert!(
            !decision.fires(),
            "run {run}: an unchanged red state must not re-alert: {decision:?}"
        );
        assert_eq!(ledger.streak(), run);
    }

    // Run 6 crosses the threshold: exactly one alert, naming the streak.
    let decision = ledger.record_run(&failing);
    match &decision {
        AlertDecision::Fire { line } => {
            assert!(line.contains("6 consecutive runs"), "{line}");
            assert!(
                line.contains("kimi-k3") && line.contains("qwen3.8-27b"),
                "{line}"
            );
        }
        other => panic!("the threshold crossing must fire: {other:?}"),
    }

    // Run 7: the crossing already happened; the state is still just red.
    let decision = ledger.record_run(&failing);
    assert!(!decision.fires(), "no second threshold alert: {decision:?}");
}

#[test]
fn a_recovery_alerts_and_resets_the_streak() {
    let mut ledger = FailureLedger::with_threshold(2).expect("threshold 2 is valid");
    let failing = vec![item("a"), item("b")];

    ledger.record_run(&failing); // onset: fires, streak 1
    let decision = ledger.record_run(&failing);
    assert!(decision.fires(), "streak 2 == threshold 2 fires");
    assert_eq!(ledger.streak(), 2);

    let decision = ledger.record_run(&[]);
    match &decision {
        AlertDecision::Fire { line } => {
            assert!(line.contains("recovered"), "{line}");
            assert!(line.contains("a") && line.contains("b"), "{line}");
        }
        other => panic!("recovery must fire: {other:?}"),
    }
    assert_eq!(ledger.streak(), 0, "recovery resets the streak");

    // A later failure is a fresh onset, not a continuation.
    let decision = ledger.record_run(&[item("c")]);
    assert!(decision.fires(), "a new failure after recovery fires");
    assert_eq!(ledger.streak(), 1);
}

#[test]
fn a_changed_failure_set_alerts_and_names_both_sides() {
    let mut ledger = FailureLedger::with_threshold(6).expect("threshold 6 is valid");
    ledger.record_run(&[item("a"), item("b")]); // onset

    // a recovered; c and d newly failed.
    let decision = ledger.record_run(&[item("c"), item("d")]);
    match &decision {
        AlertDecision::Fire { line } => {
            assert!(line.contains("failure set changed"), "{line}");
            assert!(
                line.contains("c") && line.contains("d"),
                "new failures named: {line}"
            );
            assert!(
                line.contains("a") && line.contains("b"),
                "recoveries named: {line}"
            );
        }
        other => panic!("a changed set must fire: {other:?}"),
    }
}

#[test]
fn diff_failure_sets_classifies_every_transition() {
    let a = vec![item("a")];
    let b = vec![item("b")];

    assert_eq!(diff_failure_sets(&[], &[]), FailureTransition::Steady);
    assert_eq!(
        diff_failure_sets(&[], &a),
        FailureTransition::Onset {
            failed: vec![item("a")]
        }
    );
    assert_eq!(
        diff_failure_sets(&a, &[]),
        FailureTransition::Recovered {
            recovered: vec![item("a")]
        }
    );
    assert_eq!(
        diff_failure_sets(&a, &a),
        FailureTransition::Persisted {
            failed: vec![item("a")]
        }
    );
    assert_eq!(
        diff_failure_sets(&a, &b),
        FailureTransition::Changed {
            new: vec![item("b")],
            recovered: vec![item("a")],
        }
    );
}

#[test]
fn duplicate_item_names_do_not_double_count() {
    // The job's own bookkeeping may repeat an item name; the set semantics
    // must not.
    let mut ledger = FailureLedger::with_threshold(6).expect("threshold 6 is valid");
    ledger.record_run(&[item("a"), item("a")]); // onset, streak 1

    let decision = ledger.record_run(&[item("a"), item("a")]);
    assert!(
        !decision.fires(),
        "the same single item failing twice over is persistence, not a change: {decision:?}"
    );
    assert_eq!(ledger.last_failed(), vec![item("a")]);
}

#[test]
fn a_zero_threshold_is_rejected() {
    // A threshold of 0 would alert on every red run — alerting on state, the
    // defect this module removes.
    let err = FailureLedger::with_threshold(0).expect_err("threshold 0 must be rejected");
    assert!(err.contains(">= 1"), "{err}");
}

#[test]
fn the_ledger_round_trips_through_json() {
    // The unit persists this between runs; a round trip must preserve the
    // streak, the previous failure set, and the threshold.
    let mut ledger = FailureLedger::with_threshold(6).expect("threshold 6 is valid");
    ledger.record_run(&[item("a")]);
    ledger.record_run(&[item("a"), item("b")]);

    let json = serde_json::to_string(&ledger).expect("ledger must serialize");
    let restored: FailureLedger = serde_json::from_str(&json).expect("ledger must deserialize");
    assert_eq!(restored, ledger);
    assert_eq!(restored.streak(), 2);
    assert_eq!(restored.threshold(), 6);
}
