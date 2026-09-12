//! Baseline coverage and correction scope (issue #4428).
//!
//! The incident: a baseline-diff gate — compare the patch's failing-test
//! set against main's — was built because `autospec-cli` was red on main and
//! a plain pass/fail run could not judge a patch there. The correction was
//! applied **only to `autospec-cli`**. For `autospec-core` the gate kept
//! pass/fail, having never asked whether that crate was also red. It was:
//! `runner_executes_the_newly_registered_bats_suites` fails on main. Three
//! patches — 4070, 4015 and 4094 — were recorded HELD with that test named
//! as their failure. All three were innocent; re-gated against the real
//! baseline, all three passed and are merged. The held reasons were written
//! confidently and were wrong, and a HELD entry is not re-examined, so the
//! patches would have sat there indefinitely with a plausible-looking
//! explanation attached.
//!
//! The regression tests run in the configuration the bug required: a gate
//! over two crates whose mains are both red, with the correction applied to
//! one of them only. The controls: both baselines measured, the correction
//! applied to both crates, and the held entries re-examined.

use std::collections::BTreeSet;

use autospec_core::baseline_coverage::{
    judge, reexamine_held, CorrectionAudit, GatePass, HeldEntry, Judgment, JudgmentMethod,
    MainStatus, ReexamineVerdict,
};

/// The crate whose main fails on the test the three patches were held by.
const CORE_RED_TEST: &str = "runner_executes_the_newly_registered_bats_suites";

/// The pass as it ran in the incident: two crates, and only `autospec-cli`
/// baselined — the correction reached the crate where the red main was
/// found and stopped there.
fn incident_pass() -> GatePass {
    let mut pass = GatePass::new(["autospec-cli", "autospec-core"]);
    pass.establish_baseline(
        "autospec-cli",
        MainStatus::from_failures([
            "autonomous_conductor_commands::first_known_failure",
            "autonomous_conductor_commands::second_known_failure",
        ]),
    )
    .unwrap();
    pass
}

/// The pass as the invariant requires it: the baseline for every crate the
/// gate runs against, measured at the start of the pass.
fn corrected_pass() -> GatePass {
    let mut pass = incident_pass();
    pass.establish_baseline("autospec-core", MainStatus::from_failures([CORE_RED_TEST]))
        .unwrap();
    pass
}

/// The three innocent patches, each HELD with the core main failure named
/// as its reason.
fn incident_held_entries() -> Vec<HeldEntry> {
    ["4070", "4015", "4094"]
        .map(|patch| HeldEntry {
            patch: patch.into(),
            crate_name: "autospec-core".into(),
            cited_failures: [CORE_RED_TEST].map(str::to_string).into_iter().collect(),
        })
        .to_vec()
}

fn failures(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|s| s.to_string()).collect()
}

#[test]
fn the_incident_pass_fail_on_a_red_main_is_an_invalid_judgment() {
    // The correction was applied to autospec-cli only. autospec-core's
    // measured main is red, and the gate judged it by pass/fail.
    let mut pass = GatePass::new(["autospec-cli", "autospec-core"]);
    pass.establish_baseline("autospec-core", MainStatus::from_failures([CORE_RED_TEST]))
        .unwrap();

    // Even a patch whose run printed zero failures cannot be judged by
    // pass/fail on a red main: the method itself is the defect, whatever
    // the run printed.
    let judgment = judge(
        &pass,
        "autospec-core",
        JudgmentMethod::PassFail,
        &BTreeSet::new(),
    );
    match &judgment {
        Judgment::InvalidJudgment {
            crate_name,
            main_failures,
        } => {
            assert_eq!(crate_name, "autospec-core");
            assert_eq!(main_failures, &failures(&[CORE_RED_TEST]));
        }
        other => panic!("expected InvalidJudgment, got {other:?}"),
    }
    let line = judgment.line();
    assert!(line.starts_with("FAIL:"), "{line}");
    assert!(
        line.contains("pass/fail cannot judge a patch on a red main"),
        "{line}"
    );
}

#[test]
fn the_baseline_diff_admits_the_three_innocent_patches() {
    let pass = corrected_pass();
    // Each held patch failed its run on the core main failure. Baseline
    // diff: patch − main is empty, so the patch is innocent.
    for patch in ["4070", "4015", "4094"] {
        let judgment = judge(
            &pass,
            "autospec-core",
            JudgmentMethod::BaselineDiff,
            &failures(&[CORE_RED_TEST]),
        );
        assert_eq!(
            judgment,
            Judgment::Admitted {
                crate_name: "autospec-core".into()
            },
            "patch {patch}"
        );
    }
}

#[test]
fn a_genuinely_new_failure_is_still_held_under_the_diff() {
    let pass = corrected_pass();
    let judgment = judge(
        &pass,
        "autospec-core",
        JudgmentMethod::BaselineDiff,
        &failures(&[CORE_RED_TEST, "some_new_test_added_by_the_patch"]),
    );
    match judgment {
        Judgment::Held {
            crate_name,
            new_failures,
        } => {
            assert_eq!(crate_name, "autospec-core");
            // The main failure is tolerated; only the new one holds.
            assert_eq!(
                new_failures,
                failures(&["some_new_test_added_by_the_patch"])
            );
        }
        other => panic!("expected Held, got {other:?}"),
    }
}

#[test]
fn an_unbaselined_crate_cannot_be_judged_under_either_method() {
    // The incident's pass: core never baselined.
    let pass = incident_pass();

    for method in [JudgmentMethod::PassFail, JudgmentMethod::BaselineDiff] {
        let judgment = judge(&pass, "autospec-core", method, &BTreeSet::new());
        assert_eq!(
            judgment,
            Judgment::NoBaseline {
                crate_name: "autospec-core".into()
            },
            "method {method:?}: green must never be assumed"
        );
    }

    let line = pass.coverage_line();
    assert!(line.starts_with("FAIL:"), "{line}");
    assert!(line.contains("1 of 2"), "{line}");
    assert!(line.contains("unbaselined: autospec-core"), "{line}");
    assert!(
        line.contains("\"main is green here\" is an assumption, not a measurement"),
        "{line}"
    );
}

#[test]
fn a_fully_baselined_pass_reports_complete_coverage() {
    let pass = corrected_pass();
    assert!(pass.unbaselined().is_empty());
    let line = pass.coverage_line();
    assert!(line.starts_with("OK:"), "{line}");
    assert!(line.contains("2 of 2"), "{line}");
}

#[test]
fn the_correction_audit_names_the_unfixed_instance() {
    // The correction reached the crate where the flaw was found.
    let audit = CorrectionAudit::new(["autospec-cli", "autospec-core"], ["autospec-cli"]);
    assert!(!audit.complete());
    assert_eq!(audit.unfixed(), vec!["autospec-core".to_string()]);
    let line = audit.line();
    assert!(line.starts_with("FAIL:"), "{line}");
    assert!(line.contains("unfixed: autospec-core"), "{line}");
    assert!(
        line.contains(
            "a correction applies to every instance of the method, not just where the flaw was found"
        ),
        "{line}"
    );

    // The same correction, applied to every instance: done.
    let complete = CorrectionAudit::new(
        ["autospec-cli", "autospec-core"],
        ["autospec-cli", "autospec-core"],
    );
    assert!(complete.complete());
    assert!(complete.line().starts_with("OK:"));
}

#[test]
fn reexamine_held_exonerates_the_three_innocent_patches() {
    let pass = corrected_pass();
    let verdicts = reexamine_held(&incident_held_entries(), &pass);
    assert_eq!(verdicts.len(), 3);
    for (entry, verdict) in incident_held_entries().iter().zip(&verdicts) {
        match verdict {
            ReexamineVerdict::Innocent {
                patch,
                crate_name,
                main_failures,
            } => {
                assert_eq!(patch, &entry.patch);
                assert_eq!(crate_name, "autospec-core");
                assert_eq!(main_failures, &failures(&[CORE_RED_TEST]));
            }
            other => panic!("patch {} expected Innocent, got {other:?}", entry.patch),
        }
        let line = verdict.line();
        assert!(line.starts_with("OK:"), "{}: {line}", entry.patch);
        assert!(
            line.contains("the hold was main's, not the patch's"),
            "{}: {line}",
            entry.patch
        );
    }
}

#[test]
fn reexamine_held_keeps_a_hold_with_a_genuine_new_failure() {
    let pass = corrected_pass();
    let entries = vec![HeldEntry {
        patch: "4101".into(),
        crate_name: "autospec-core".into(),
        cited_failures: failures(&[CORE_RED_TEST, "a_failure_the_patch_introduced"]),
    }];
    let verdicts = reexamine_held(&entries, &pass);
    assert_eq!(
        verdicts,
        vec![ReexamineVerdict::StillHeld {
            patch: "4101".into(),
            crate_name: "autospec-core".into(),
            new_failures: failures(&["a_failure_the_patch_introduced"]),
        }]
    );
    assert!(verdicts[0].line().starts_with("HOLD:"));
}

#[test]
fn reexamine_held_fails_closed_on_an_unbaselined_crate() {
    // No measured baseline for core: the entry cannot be examined either
    // way. It is not guessed innocent, and it is not confirmed held — it
    // keeps its (possibly wrong) explanation until the baseline exists.
    let pass = incident_pass();
    let verdicts = reexamine_held(&incident_held_entries(), &pass);
    for (entry, verdict) in incident_held_entries().iter().zip(&verdicts) {
        assert_eq!(
            verdict,
            &ReexamineVerdict::NoBaseline {
                patch: entry.patch.clone(),
                crate_name: "autospec-core".into(),
            }
        );
        let line = verdict.line();
        assert!(line.starts_with("FAIL:"), "{line}");
        assert!(
            line.contains("cannot be re-examined without a measured baseline"),
            "{line}"
        );
    }
}

#[test]
fn a_hold_that_cites_no_failure_cannot_be_exonerated() {
    let pass = corrected_pass();
    let entries = vec![HeldEntry {
        patch: "4102".into(),
        crate_name: "autospec-core".into(),
        cited_failures: BTreeSet::new(),
    }];
    let verdicts = reexamine_held(&entries, &pass);
    assert_eq!(
        verdicts,
        vec![ReexamineVerdict::StillHeld {
            patch: "4102".into(),
            crate_name: "autospec-core".into(),
            new_failures: BTreeSet::new(),
        }]
    );
}

#[test]
fn establish_baseline_refuses_a_crate_outside_the_pass() {
    let mut pass = GatePass::new(["autospec-core"]);
    let err = pass
        .establish_baseline("autospec-cli", MainStatus::Green)
        .unwrap_err();
    assert!(
        err.contains("'autospec-cli' is not in the pass's scope"),
        "{err}"
    );
    // The refusal left no baseline behind.
    assert!(pass.baseline("autospec-cli").is_none());
}

#[test]
fn main_status_classification_of_a_measured_run() {
    assert_eq!(
        MainStatus::from_failures(std::iter::empty::<&str>()),
        MainStatus::Green
    );
    assert!(!MainStatus::Green.is_red());
    assert_eq!(MainStatus::Green.label(), "green");
    assert!(MainStatus::Green.failing_set().is_empty());

    let red = MainStatus::from_failures(["t1", "t2", "t3"]);
    assert!(red.is_red());
    assert_eq!(red.label(), "red (3 failing)");
    assert_eq!(red.failing_set(), &failures(&["t1", "t2", "t3"]));
}

#[test]
fn on_a_measured_green_main_both_methods_coincide() {
    let mut pass = GatePass::new(["green-crate"]);
    pass.establish_baseline("green-crate", MainStatus::Green)
        .unwrap();

    for method in [JudgmentMethod::PassFail, JudgmentMethod::BaselineDiff] {
        assert_eq!(
            judge(&pass, "green-crate", method, &BTreeSet::new()),
            Judgment::Admitted {
                crate_name: "green-crate".into()
            }
        );
        assert_eq!(
            judge(&pass, "green-crate", method, &failures(&["f1"])),
            Judgment::Held {
                crate_name: "green-crate".into(),
                new_failures: failures(&["f1"]),
            }
        );
    }
}
