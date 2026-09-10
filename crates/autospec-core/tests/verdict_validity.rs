//! Integration: a recorded verdict carries its validity conditions, and the
//! converter's decisions about a cached verdict follow the conditions, not a
//! date (#4031).

use std::collections::{BTreeMap, BTreeSet};

use autospec_core::autonomous::regrade::HostConditions;
use autospec_core::autonomous::test_gate::{evaluate, GateDecision, SuiteFailure, SuiteOutcome};
use autospec_core::autonomous::verdict_validity::{
    baseline_hash, decode, encode, fixed_causes, guard_destructive, route, verify, CurrentTree,
    PatchRoute, StaleReason, VerdictValidity,
};

fn suite_with_failing(name: &str) -> SuiteOutcome {
    SuiteOutcome {
        passed: 919,
        failures: vec![SuiteFailure {
            name: name.to_string(),
            component: Some("autospec-core".to_string()),
        }],
    }
}

fn rerun_fails(name: &str) -> BTreeMap<String, bool> {
    BTreeMap::from([(name.to_string(), false)])
}

fn host() -> HostConditions {
    HostConditions {
        load_average: 0.5,
        concurrent_agents: 1,
    }
}

/// AC1: the verdict records the commit and the baseline hash it was graded
/// under, and the recorded form persists both through encode/decode.
#[test]
fn a_verdict_records_the_conditions_it_was_graded_under() {
    let baseline = BTreeSet::from(["pre_existing".to_string()]);
    let verdict = evaluate(
        &suite_with_failing("pre_existing"),
        &rerun_fails("pre_existing"),
        &baseline,
        &BTreeSet::from(["autospec-core".to_string()]),
        Some("abc123"),
    )
    .expect("verdict");

    assert_eq!(verdict.tree_commit.as_deref(), Some("abc123"));
    assert_eq!(verdict.baseline_hash, baseline_hash(&baseline));

    let recorded = verdict.recorded("issue-42/0001-fix.patch", 1_700_000_000, &host());
    assert_eq!(recorded.patch_identity, "issue-42/0001-fix.patch");
    assert_eq!(recorded.verdict, "new-test-failures");
    assert_eq!(recorded.failing_tests, baseline);

    let decoded = decode(&encode(&recorded)).expect("the persisted verdict decodes");
    assert_eq!(decoded, recorded);
    let current = CurrentTree {
        commit: Some("abc123".to_string()),
        failing_baseline: baseline.clone(),
    };
    assert_eq!(verify(&decoded, &current), VerdictValidity::Fresh);
    assert_eq!(route(&decoded, &current), PatchRoute::Trust);
}

/// AC2: a verdict whose commit does not match the current tree is stale, and
/// staleness only marks the patch for re-verification — discard and retire
/// both refuse.
#[test]
fn a_commit_mismatch_is_stale_and_refuses_destructive_actions() {
    let verdict = evaluate(
        &suite_with_failing("t"),
        &rerun_fails("t"),
        &BTreeSet::new(),
        &BTreeSet::from(["autospec-core".to_string()]),
        Some("abc123"),
    )
    .expect("verdict");
    let record = verdict.recorded("issue-42/0001-fix.patch", 1_700_000_000, &host());

    let current = CurrentTree {
        commit: Some("def456".to_string()),
        failing_baseline: BTreeSet::new(),
    };
    assert_eq!(
        verify(&record, &current),
        VerdictValidity::Stale(vec![StaleReason::TreeCommitDrift {
            recorded: "abc123".to_string(),
            current: "def456".to_string(),
        }])
    );
    match route(&record, &current) {
        PatchRoute::Reverify(reason) => {
            assert!(reason.contains("abc123") && reason.contains("def456"));
        }
        other => panic!("a stale verdict must route to re-verification, got {other:?}"),
    }
    for action in ["discard", "retire"] {
        let error =
            guard_destructive(action, &record, &current).expect_err("a stale verdict must refuse");
        assert!(error.contains(&format!("refuse to {action}")), "{error}");
    }
}

/// AC3: a failing test fixed on main moves the baseline; the verdict that
/// named it goes back to the gate with the changed tests named, and the
/// pre-fix verdict is not trusted.
#[test]
fn a_baseline_move_sends_the_patch_back_to_the_gate() {
    let recorded_baseline =
        BTreeSet::from(["fixed_on_main".to_string(), "still_failing".to_string()]);
    let suite = SuiteOutcome {
        passed: 917,
        failures: vec![
            SuiteFailure {
                name: "fixed_on_main".to_string(),
                component: Some("autospec-core".to_string()),
            },
            SuiteFailure {
                name: "still_failing".to_string(),
                component: Some("autospec-core".to_string()),
            },
        ],
    };
    let reruns = BTreeMap::from([
        ("fixed_on_main".to_string(), false),
        ("still_failing".to_string(), false),
    ]);
    let verdict = evaluate(
        &suite,
        &reruns,
        &recorded_baseline,
        &BTreeSet::from(["autospec-core".to_string()]),
        Some("abc123"),
    )
    .expect("verdict");
    let record = verdict.recorded("issue-42/0001-fix.patch", 1_700_000_000, &host());

    // The tree did not move, but the failing baseline did: the cause was
    // fixed on main.
    let current = CurrentTree {
        commit: Some("abc123".to_string()),
        failing_baseline: BTreeSet::from(["still_failing".to_string()]),
    };
    match verify(&record, &current) {
        VerdictValidity::Stale(reasons) => match &reasons[0] {
            StaleReason::BaselineDrift { changed, .. } => {
                assert_eq!(changed, &["fixed_on_main".to_string()]);
            }
            other => panic!("expected baseline drift, got {other:?}"),
        },
        other => panic!("expected stale, got {other:?}"),
    }
    assert_eq!(
        fixed_causes(&record, &current),
        vec!["fixed_on_main".to_string()],
        "the fixed test is the systemic cause that was fixed on main"
    );
    assert!(
        !verify(&record, &current).is_trustworthy(),
        "the pre-fix verdict is not trusted"
    );
    assert!(
        guard_destructive("retire", &record, &current).is_err(),
        "a moved baseline must not authorize retiring the patch"
    );
}

/// AC4: an unverifiable verdict — no recorded commit, or an unreadable
/// current commit — refuses to act and reports why.
#[test]
fn an_unverifiable_verdict_refuses_destructive_actions_with_a_reason() {
    let verdict = evaluate(
        &suite_with_failing("t"),
        &rerun_fails("t"),
        &BTreeSet::new(),
        &BTreeSet::from(["autospec-core".to_string()]),
        Some("abc123"),
    )
    .expect("verdict");
    // The runner could not read the commit: the persisted verdict has none.
    let mut record = verdict.recorded("issue-42/0001-fix.patch", 1_700_000_000, &host());
    record.tree_commit = None;

    let readable = CurrentTree {
        commit: Some("abc123".to_string()),
        failing_baseline: BTreeSet::new(),
    };
    match verify(&record, &readable) {
        VerdictValidity::Unverifiable(reason) => {
            assert!(
                reason.contains("no tree commit"),
                "the refusal names the missing field: {reason}"
            );
        }
        other => panic!("expected unverifiable, got {other:?}"),
    }
    for action in ["discard", "retire"] {
        let error = guard_destructive(action, &record, &readable)
            .expect_err("an unverifiable verdict must refuse");
        assert!(error.contains(&format!("refuse to {action}")), "{error}");
        assert!(
            error.contains("cannot verify"),
            "the refusal says the verdict cannot be verified: {error}"
        );
    }
    assert!(
        matches!(route(&record, &readable), PatchRoute::Reverify(_)),
        "an unverifiable verdict is no verdict: the patch is re-verified"
    );

    // The other unreadable side: the current tree's commit cannot be read.
    let full = verdict.recorded("issue-42/0001-fix.patch", 1_700_000_000, &host());
    let unreadable = CurrentTree {
        commit: None,
        failing_baseline: BTreeSet::new(),
    };
    match verify(&full, &unreadable) {
        VerdictValidity::Unverifiable(reason) => {
            assert!(reason.contains("could not be read"), "{reason}");
        }
        other => panic!("expected unverifiable, got {other:?}"),
    }
    assert!(guard_destructive("discard", &full, &unreadable).is_err());
}

/// AC5: the end-to-end shape of the incident — a verdict recorded while the
/// systemic cause still fails is stale once the cause is fixed, and the
/// patch is re-verified against the new tree instead of being discarded.
#[test]
fn a_pre_fix_verdict_is_not_trusted_after_the_cause_is_fixed() {
    let pre_fix_baseline = BTreeSet::from(["executor_bridge::tests::failpoint".to_string()]);
    let pre_fix = evaluate(
        &suite_with_failing("executor_bridge::tests::failpoint"),
        &rerun_fails("executor_bridge::tests::failpoint"),
        &pre_fix_baseline,
        &BTreeSet::from(["autospec-core".to_string()]),
        Some("pre-fix"),
    )
    .expect("verdict");
    let record = pre_fix.recorded("issue-42/0001-fix.patch", 1_700_000_000, &host());

    let post_fix = CurrentTree {
        commit: Some("post-fix".to_string()),
        failing_baseline: BTreeSet::new(),
    };
    assert!(!verify(&record, &post_fix).is_trustworthy());
    assert_eq!(
        fixed_causes(&record, &post_fix),
        vec!["executor_bridge::tests::failpoint".to_string()]
    );
    assert!(
        matches!(route(&record, &post_fix), PatchRoute::Reverify(_)),
        "the patch is re-verified, not discarded on the pre-fix verdict"
    );
    assert!(
        guard_destructive("discard", &record, &post_fix).is_err(),
        "the pre-fix verdict must not authorize discarding the patch"
    );
}

/// A flaky failure is not a cause the verdict stands on: `recorded` names
/// only the persistent failures, and a pass verdict names none.
#[test]
fn recorded_names_only_persistent_failures() {
    let suite = SuiteOutcome {
        passed: 919,
        failures: vec![
            SuiteFailure {
                name: "persistent".to_string(),
                component: Some("autospec-core".to_string()),
            },
            SuiteFailure {
                name: "flaky".to_string(),
                component: Some("autospec-core".to_string()),
            },
        ],
    };
    let reruns = BTreeMap::from([
        ("persistent".to_string(), false),
        ("flaky".to_string(), true),
    ]);
    let verdict = evaluate(
        &suite,
        &reruns,
        &BTreeSet::new(),
        &BTreeSet::from(["autospec-core".to_string()]),
        Some("abc123"),
    )
    .expect("verdict");
    assert_eq!(
        verdict.decision,
        GateDecision::Hold {
            attribution: autospec_core::autonomous::test_gate::FailureClass::Caused
        }
    );
    let record = verdict.recorded("issue-42/0001-fix.patch", 1_700_000_000, &host());
    assert_eq!(
        record.failing_tests,
        BTreeSet::from(["persistent".to_string()])
    );
    assert_eq!(
        record.flaky_tests,
        BTreeSet::from(["flaky".to_string()]),
        "the flaky set rides along in the recorded verdict"
    );

    let passing = SuiteOutcome {
        passed: 920,
        failures: vec![SuiteFailure {
            name: "flaky_only".to_string(),
            component: Some("autospec-core".to_string()),
        }],
    };
    let pass_reruns = BTreeMap::from([("flaky_only".to_string(), true)]);
    let pass = evaluate(
        &passing,
        &pass_reruns,
        &BTreeSet::new(),
        &BTreeSet::from(["autospec-core".to_string()]),
        Some("abc123"),
    )
    .expect("verdict");
    assert_eq!(pass.verdict_token(), "pass");
    assert!(pass
        .recorded("issue-42/0002-fix.patch", 1_700_000_000, &host())
        .failing_tests
        .is_empty());
}
