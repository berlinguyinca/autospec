//! Regression tests for issue #3779: a gate must attribute a failure before
//! it blocks on one. The motivating case is a patch that touched only
//! `scripts/autospec-explore.sh` (no compiled code) held by one flaky test
//! out of 920, with a recorded verdict that said only
//! `HELD: tests failed (--workspace)`.

use std::collections::{BTreeMap, BTreeSet};

use autospec_core::autonomous::test_gate::{
    affected_crates, evaluate, read_flaky_ledger, record_flaky_outcomes, repeated_flakes,
    FailureClass, FlakyLedgerEntry, GateDecision, GateError, SuiteFailure, SuiteOutcome,
    REPEATED_FLAKY_THRESHOLD,
};

fn workspace() -> (Vec<String>, BTreeMap<String, Vec<String>>) {
    (
        vec!["autospec-core".to_string(), "autospec-cli".to_string()],
        BTreeMap::from([(
            "autospec-cli".to_string(),
            vec!["autospec-core".to_string()],
        )]),
    )
}

/// Regression: a patch that touches only non-compiled files, against a
/// suite with one flaky test, is NOT held.
#[test]
fn non_compiled_patch_against_one_flaky_test_is_not_held() {
    let (crates, deps) = workspace();
    let touched = [
        "scripts/autospec-explore.sh",
        "docs/runbooks/needs-classify-sweep.md",
    ];

    let affected = affected_crates(&touched, &crates, &deps);
    assert!(
        affected.is_empty(),
        "a patch that touches no compiled code cannot affect any crate by the dependency graph"
    );

    let suite = SuiteOutcome {
        passed: 919,
        failures: vec![SuiteFailure {
            name: "executor_bridge::tests::terminal_label".to_string(),
            component: Some("autospec-cli".to_string()),
        }],
    };
    // The gate re-runs the failing test in isolation; it passes.
    let reruns = BTreeMap::from([("executor_bridge::tests::terminal_label".to_string(), true)]);

    let verdict = evaluate(&suite, &reruns, &BTreeSet::new(), &affected, None).expect("verdict");

    assert_eq!(verdict.decision, GateDecision::Pass);
    assert_eq!(
        verdict.failures[0].class,
        FailureClass::Flaky,
        "the failure is reported as flaky, and both outcomes are kept"
    );
    assert!(verdict.failures[0].suite_failed);
    assert!(verdict.failures[0].rerun_passed);
    assert_eq!(
        verdict.message(),
        "PASS: 919 passed; 1 flaky (executor_bridge::tests::terminal_label)"
    );
}

/// The blast radius closes over reverse dependencies: a change to a
/// dependency marks every crate that depends on it.
#[test]
fn blast_radius_closes_over_reverse_dependencies() {
    let (crates, deps) = workspace();
    // autospec-cli depends on autospec-core: touching the core marks both.
    let affected = affected_crates(&["crates/autospec-core/src/lib.rs"], &crates, &deps);
    assert_eq!(
        affected,
        BTreeSet::from(["autospec-core".to_string(), "autospec-cli".to_string()])
    );

    // Touching the leaf marks only the leaf.
    let affected = affected_crates(&["crates/autospec-cli/src/main.rs"], &crates, &deps);
    assert_eq!(affected, BTreeSet::from(["autospec-cli".to_string()]));
}

/// The root workspace manifest is a compiled input to every crate.
#[test]
fn root_workspace_manifest_affects_every_crate() {
    let (crates, deps) = workspace();
    let affected = affected_crates(&["Cargo.toml"], &crates, &deps);
    assert_eq!(
        affected,
        BTreeSet::from(["autospec-core".to_string(), "autospec-cli".to_string()])
    );
}

/// A failure whose component cannot be told is not "outside the blast
/// radius": the gate fails closed and classifies it caused.
#[test]
fn an_unknown_component_is_classified_fail_closed() {
    let suite = SuiteOutcome {
        passed: 917,
        failures: vec![SuiteFailure {
            name: "mystery".to_string(),
            component: None,
        }],
    };
    let reruns = BTreeMap::from([("mystery".to_string(), false)]);
    let affected = BTreeSet::from(["autospec-core".to_string()]);
    let verdict = evaluate(&suite, &reruns, &BTreeSet::new(), &affected, None).expect("verdict");

    assert_eq!(
        verdict.decision,
        GateDecision::Hold {
            attribution: FailureClass::Caused
        }
    );
    assert_eq!(verdict.failures[0].class, FailureClass::Caused);
    assert_eq!(
        verdict.message(),
        "HELD: tests failed -- caused -- 1 failing: mystery",
        "a hold names the test it stands on, not only its attribution (#3747)"
    );
}

/// AC1: a persistent failure outside the blast radius is reported as
/// unattributable, not as a failure of the change.
#[test]
fn persistent_failure_outside_blast_radius_is_unattributable() {
    let (crates, deps) = workspace();
    let touched = ["scripts/autospec-explore.sh"];
    let affected = affected_crates(&touched, &crates, &deps);

    let suite = SuiteOutcome {
        passed: 919,
        failures: vec![SuiteFailure {
            name: "some_deterministic_failure".to_string(),
            component: Some("autospec-cli".to_string()),
        }],
    };
    let reruns = BTreeMap::from([("some_deterministic_failure".to_string(), false)]);

    let verdict = evaluate(&suite, &reruns, &BTreeSet::new(), &affected, None).expect("verdict");

    assert_eq!(
        verdict.decision,
        GateDecision::Hold {
            attribution: FailureClass::Unattributable
        },
        "the hold names its attribution; a verdict that cannot say which is not a verdict"
    );
    assert_eq!(verdict.unattributable, ["some_deterministic_failure"]);
    assert_eq!(
        verdict.message(),
        "HELD: tests failed -- unattributable -- 1 failing: some_deterministic_failure"
    );
}

/// AC3: a hold names caused / pre-existing / unattributable; the three
/// signals each resolve the attribution.
#[test]
fn hold_attribution_follows_the_signals() {
    let affected = BTreeSet::from(["autospec-core".to_string()]);
    let suite = SuiteOutcome {
        passed: 917,
        failures: vec![
            SuiteFailure {
                name: "caused_one".to_string(),
                component: Some("autospec-core".to_string()),
            },
            SuiteFailure {
                name: "pre_existing_one".to_string(),
                component: Some("autospec-core".to_string()),
            },
            SuiteFailure {
                name: "unattributable_one".to_string(),
                component: Some("autospec-cli".to_string()),
            },
        ],
    };
    let reruns = BTreeMap::from([
        ("caused_one".to_string(), false),
        ("pre_existing_one".to_string(), false),
        ("unattributable_one".to_string(), false),
    ]);
    let baseline = BTreeSet::from(["pre_existing_one".to_string()]);

    let verdict = evaluate(&suite, &reruns, &baseline, &affected, None).expect("verdict");

    assert_eq!(verdict.failures[0].class, FailureClass::Caused);
    assert_eq!(verdict.failures[1].class, FailureClass::PreExisting);
    assert_eq!(verdict.failures[2].class, FailureClass::Unattributable);
    assert_eq!(
        verdict.decision,
        GateDecision::Hold {
            attribution: FailureClass::Caused
        },
        "the strongest attributable class names the hold"
    );
    assert_eq!(
        verdict.message(),
        "HELD: tests failed -- caused -- 3 failing: caused_one, pre_existing_one, unattributable_one"
    );
}

/// AC (#3747): a hold names the failing tests, and names only the persistent
/// ones. A test that passed on its isolated re-run is flaky and stays out of
/// the hold line.
#[test]
fn a_hold_names_its_persistent_failing_tests_and_no_flaky_ones() {
    let affected = BTreeSet::from(["autospec-core".to_string()]);
    let suite = SuiteOutcome {
        passed: 900,
        failures: vec![
            SuiteFailure {
                name: "flaky_one".to_string(),
                component: Some("autospec-core".to_string()),
            },
            SuiteFailure {
                name: "real_one".to_string(),
                component: Some("autospec-core".to_string()),
            },
        ],
    };
    let reruns = BTreeMap::from([
        ("flaky_one".to_string(), true),
        ("real_one".to_string(), false),
    ]);

    let verdict = evaluate(&suite, &reruns, &BTreeSet::new(), &affected, None).expect("verdict");

    let message = verdict.message();
    assert_eq!(
        message, "HELD: tests failed -- caused -- 1 failing: real_one",
        "the hold names the persistent failure and not the flaky one"
    );
}

/// A failing test with no isolated re-run is not a verdict.
#[test]
fn missing_rerun_fails_closed() {
    let suite = SuiteOutcome {
        passed: 919,
        failures: vec![SuiteFailure {
            name: "no_rerun".to_string(),
            component: Some("autospec-core".to_string()),
        }],
    };
    let error = evaluate(
        &suite,
        &BTreeMap::new(),
        &BTreeSet::new(),
        &BTreeSet::new(),
        None,
    )
    .expect_err("a failing test is re-run before the verdict is recorded");
    assert_eq!(
        error,
        GateError::MissingRerun {
            test: "no_rerun".to_string()
        }
    );
    assert!(
        error.to_string().contains("no_rerun"),
        "the error names the test: {}",
        error
    );
}

/// AC4: flaky outcomes accumulate in a durable location, and a test that
/// flakes repeatedly becomes visible as a defect.
#[test]
fn flaky_ledger_accumulates_durably_and_flags_repeated_flakes() {
    let dir = std::env::temp_dir().join(format!(
        "autospec-test-gate-{}-{}",
        std::process::id(),
        "ledger"
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let ledger = dir.join("flaky.jsonl");
    std::fs::remove_file(&ledger).ok();

    let test = "executor_bridge::tests::terminal_label";
    // Three gate runs, one flaky outcome each, recorded over time.
    for at in [1_752_000_000u64, 1_752_003_600, 1_752_007_200] {
        record_flaky_outcomes(
            &ledger,
            &[FlakyLedgerEntry {
                test: test.to_string(),
                component: Some("autospec-cli".to_string()),
                at,
            }],
        )
        .expect("append to ledger");
    }
    // A different test flaking twice stays below the threshold.
    record_flaky_outcomes(
        &ledger,
        &[
            FlakyLedgerEntry {
                test: "other_test".to_string(),
                component: None,
                at: 1_752_000_001,
            },
            FlakyLedgerEntry {
                test: "other_test".to_string(),
                component: None,
                at: 1_752_003_601,
            },
        ],
    )
    .expect("append to ledger");

    // Read back from disk: the record survives the process.
    let counts = read_flaky_ledger(&ledger).expect("read ledger");
    assert_eq!(counts[test], REPEATED_FLAKY_THRESHOLD);
    assert_eq!(counts["other_test"], 2);
    assert_eq!(repeated_flakes(&counts), [test]);

    let _ = std::fs::remove_dir_all(&dir);
}

/// A malformed ledger line is an error, not a silently dropped count.
#[test]
fn malformed_ledger_line_fails_closed() {
    let dir = std::env::temp_dir().join(format!(
        "autospec-test-gate-{}-{}",
        std::process::id(),
        "ledger-bad"
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let ledger = dir.join("flaky.jsonl");
    std::fs::write(
        &ledger,
        "{\"test\":\"ok_test\",\"component\":null,\"at\":1}\nnot json\n",
    )
    .expect("write");

    let error = read_flaky_ledger(&ledger).expect_err("malformed line");
    assert!(error.to_string().contains("line 2"), "error: {error}");

    let _ = std::fs::remove_dir_all(&dir);
}
