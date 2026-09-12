//! A count check is unsafe the moment main carries a tolerated-failure floor
//! (issue #4301).
//!
//! The regression tests run in the configuration the incident required:
//! `main` carries a permanent floor of 2 tolerated failures (#4291),
//! `convpass` attributes by name, `iwconv` attributes by count, and a patch
//! breaks two tests with names the floor does not carry. Before the floor
//! existed the two converters agreed in every real input, which is why the
//! divergence sat invisible in both files at once.

use autospec_core::failure_floor::{
    attribute, conclusion_for, count_reconciles, count_vs_name_divergence, floor_gate_conflict,
    sibling_drift, Attribution, Conclusion, ConverterRecord, FailureFloor, GateStyle,
};

/// The floor `main` carries (#4291): the two known-failing tests recorded
/// there.
fn incident_floor() -> FailureFloor {
    FailureFloor::new(["tests::known_floor_alpha", "tests::known_floor_beta"])
}

/// The run #4065 converted: it failed exactly the two tests the floor
/// carries, and the pass recorded
/// `all 2 failing test(s) also fail on main; not attributed to this patch`.
fn tolerated_run() -> Vec<String> {
    vec![
        "tests::known_floor_alpha".to_string(),
        "tests::known_floor_beta".to_string(),
    ]
}

/// The incident input: a patch that breaks two tests, with names the floor
/// does not carry. The totals reconcile (2 == 2); the names do not.
fn incident_run() -> Vec<String> {
    vec![
        "tests::broke_by_patch_gamma".to_string(),
        "tests::broke_by_patch_delta".to_string(),
    ]
}

// --- Invariant 1: attribution is by test identity, never by count ----------

#[test]
fn tolerated_run_is_clean_under_name_attribution() {
    // The #4065 case, still correct after the floor existed.
    let attribution = attribute(&incident_floor(), &tolerated_run());
    assert_eq!(attribution, Attribution::Tolerated { count: 2 });
    assert_eq!(
        attribution.line("4065"),
        "4065 note: all 2 failing test(s) also fail on main; not attributed to this patch"
    );
    assert!(count_vs_name_divergence(&incident_floor(), &tolerated_run()).is_empty());
}

#[test]
fn incident_run_breaks_two_new_tests_and_counts_reconcile() {
    // The incident: 2 failures in the run, 2 on the floor — the totals
    // reconcile, and they reconcile for the wrong reason.
    let floor = incident_floor();
    let run = incident_run();
    assert!(count_reconciles(&floor, &run));
    let attribution = attribute(&floor, &run);
    assert_eq!(
        attribution,
        Attribution::NewFailures {
            names: vec![
                "tests::broke_by_patch_delta".to_string(),
                "tests::broke_by_patch_gamma".to_string(),
            ]
        }
    );
    let finding = count_vs_name_divergence(&floor, &run);
    assert_eq!(finding.len(), 1);
    assert!(finding[0].starts_with("COUNT_ATTRIBUTION_UNSAFE:"));
    assert!(finding[0].contains("2 == 2"), "{}", finding[0]);
    assert!(
        finding[0].contains("tests::broke_by_patch_gamma")
            && finding[0].contains("tests::broke_by_patch_delta"),
        "{}",
        finding[0]
    );
}

#[test]
fn partial_overlap_with_an_extra_failure_does_not_reconcile() {
    // Two known failures plus one new one: 3 != 2, so even the count gate
    // fires — but only the name attribution says which test the patch is
    // responsible for, and the divergence check is silent because the
    // counts did not falsely reconcile.
    let floor = incident_floor();
    let run = vec![
        "tests::known_floor_alpha".to_string(),
        "tests::known_floor_beta".to_string(),
        "tests::broke_by_patch_gamma".to_string(),
    ];
    assert_eq!(
        attribute(&floor, &run),
        Attribution::NewFailures {
            names: vec!["tests::broke_by_patch_gamma".to_string()]
        }
    );
    assert!(!count_reconciles(&floor, &run));
    assert!(count_vs_name_divergence(&floor, &run).is_empty());
}

#[test]
fn one_known_and_one_new_is_a_second_divergence_shape() {
    // One known failure plus one new one: 2 == 2, the totals reconcile —
    // again for the wrong reason. The same unsafe shape, one failure fewer.
    let floor = incident_floor();
    let run = vec![
        "tests::known_floor_alpha".to_string(),
        "tests::broke_by_patch_gamma".to_string(),
    ];
    assert_eq!(
        attribute(&floor, &run),
        Attribution::NewFailures {
            names: vec!["tests::broke_by_patch_gamma".to_string()]
        }
    );
    let finding = count_vs_name_divergence(&floor, &run);
    assert_eq!(finding.len(), 1);
    assert!(finding[0].starts_with("COUNT_ATTRIBUTION_UNSAFE:"));
}

#[test]
fn clean_run_is_clean_under_both_definitions() {
    let floor = incident_floor();
    assert_eq!(attribute(&floor, &[]), Attribution::Tolerated { count: 0 });
    assert!(count_vs_name_divergence(&floor, &[]).is_empty());
}

// --- Invariant 2: a floor and a count-based gate are mutually exclusive ----

#[test]
fn empty_floor_and_count_gate_are_still_safe() {
    // Before #4291: `failures != 0` was itself the signal, because any
    // failure in the run was new by definition.
    let floor = FailureFloor::default();
    assert!(floor_gate_conflict(&floor, GateStyle::Counts).is_empty());
}

#[test]
fn admitting_the_first_known_failure_fails_the_count_gate() {
    // The act invariant 2 is about: the floor goes 0 -> 1 and the gate that
    // compares totals is invalid from that moment.
    let mut floor = FailureFloor::default();
    assert!(floor_gate_conflict(&floor, GateStyle::Counts).is_empty());
    floor.admit("tests::known_floor_alpha");
    let finding = floor_gate_conflict(&floor, GateStyle::Counts);
    assert_eq!(finding.len(), 1);
    assert!(finding[0].starts_with("FLOOR_AND_COUNT_GATE:"));
    assert!(
        finding[0].contains("1 known-failing test(s)"),
        "{}",
        finding[0]
    );
}

#[test]
fn incident_floor_fails_the_count_gate_and_not_the_name_gate() {
    let floor = incident_floor();
    let by_counts = floor_gate_conflict(&floor, GateStyle::Counts);
    assert_eq!(by_counts.len(), 1);
    assert!(by_counts[0].contains("2 known-failing test(s)"));
    assert!(floor_gate_conflict(&floor, GateStyle::Names).is_empty());
}

// --- Invariant 3: the shared attribution is the one implementation ---------

#[test]
fn shared_conclusion_agrees_with_name_attribution_for_both_inputs() {
    let floor = incident_floor();
    assert_eq!(
        conclusion_for(&floor, &tolerated_run()),
        Conclusion::NotAttributed
    );
    assert_eq!(
        conclusion_for(&floor, &incident_run()),
        Conclusion::Attributed
    );
}

#[test]
fn count_based_rederivation_disagrees_on_the_incident_input() {
    // iwconv's count gate, re-derived: totals reconcile => not attributed.
    // The shared attribution says attributed. That difference is the gap.
    let floor = incident_floor();
    let run = incident_run();
    let count_based_conclusion = if count_reconciles(&floor, &run) {
        Conclusion::NotAttributed
    } else {
        Conclusion::Attributed
    };
    assert_eq!(
        count_based_conclusion,
        Conclusion::NotAttributed,
        "precondition: the count gate merges the incident patch"
    );
    assert_eq!(
        conclusion_for(&floor, &run),
        Conclusion::Attributed,
        "the shared attribution holds it"
    );
}

// --- Invariant 4: the siblings are compared in the same change -------------

#[test]
fn siblings_agree_on_the_tolerated_input() {
    let floor = incident_floor();
    let run = tolerated_run();
    let shared = conclusion_for(&floor, &run);
    let convpass = ConverterRecord::new("convpass", run.clone(), shared);
    let iwconv = ConverterRecord::new("iwconv", run, shared);
    assert!(sibling_drift(&convpass, &iwconv).is_empty());
}

#[test]
fn siblings_drift_on_the_incident_input_and_the_finding_names_both() {
    // convpass (by name) holds the patch; iwconv (by count) merges it — the
    // same input, different conclusions. Invisible from either record alone.
    let floor = incident_floor();
    let run = incident_run();
    let shared = conclusion_for(&floor, &run);
    let convpass = ConverterRecord::new("convpass", run.clone(), shared);
    let iwconv = ConverterRecord::new("iwconv", run, Conclusion::NotAttributed); // by count
    let finding = sibling_drift(&convpass, &iwconv);
    assert_eq!(finding.len(), 1);
    assert!(finding[0].starts_with("SIBLING_ATTRIBUTION_DRIFT:"));
    assert!(finding[0].contains("convpass") && finding[0].contains("iwconv"));
    assert!(
        finding[0].contains("not-attributed") && finding[0].contains("attributed"),
        "{}",
        finding[0]
    );
}

#[test]
fn same_conclusion_for_the_same_set_is_not_drift() {
    let run = incident_run();
    let a = ConverterRecord::new("convpass", run.clone(), Conclusion::Attributed);
    let b = ConverterRecord::new("iwconv", run, Conclusion::Attributed);
    assert!(sibling_drift(&a, &b).is_empty());
}

#[test]
fn different_inputs_are_not_compared() {
    let a = ConverterRecord::new("convpass", tolerated_run(), Conclusion::NotAttributed);
    let b = ConverterRecord::new("iwconv", incident_run(), Conclusion::Attributed);
    // Different failure sets: no drift claim is possible between them.
    assert!(sibling_drift(&a, &b).is_empty());
}

#[test]
fn same_converter_is_not_a_sibling_pair() {
    let a = ConverterRecord::new("convpass", incident_run(), Conclusion::NotAttributed);
    let b = ConverterRecord::new("convpass", incident_run(), Conclusion::Attributed);
    assert!(sibling_drift(&a, &b).is_empty());
}
