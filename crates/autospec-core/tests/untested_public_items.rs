//! Regression tests for the untested-public-item gate (issue #4329).
//!
//! Incident configuration: the patch for #4238 added eight public functions
//! to `shared_write_target.rs` with zero tests. Every existing test still
//! passed, the merge record said `CONVERTED+MERGED: 9065 passed, 0 failing`,
//! and `hold_line` shipped with a format string carrying two `{}`
//! placeholders against one argument — a compile error the gate never saw,
//! because no test called the function.
//!
//! A patch whose additions are all exercised cannot see the defect: the
//! incident tests below are the ones that must fail if any of the four
//! invariants regresses, and they are mutation-verified (invariant 4) — the
//! audit with `hold_line` missing from the exercised set is the red state.

use autospec_core::untested_public_items::{
    audit, diff_public_items, gate, parse_exemptions, record_carries_coverage, ChangeCoverage,
    Exemption, GateVerdict,
};

/// The eight public functions the #4238 patch added, in source order.
fn eight() -> Vec<String> {
    [
        "entry_path",
        "parse_table_entries",
        "render_entry",
        "render_index",
        "is_self_contained",
        "decide_append",
        "contended_targets",
        "hold_line",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

fn seven_without_hold_line() -> Vec<String> {
    eight()
        .into_iter()
        .filter(|name| name != "hold_line")
        .collect()
}

#[test]
fn incident_eight_untested_functions_refuse_the_gate() {
    // Invariant 1: all eight added, none exercised, no exemptions.
    let verdict = gate(&eight(), &[], &[]);
    let untested = match &verdict {
        GateVerdict::Refused { untested, .. } => untested,
        other => panic!("gate must refuse, got: {}", other.line()),
    };
    assert_eq!(
        untested,
        &eight(),
        "the refusal names every untested item, in patch order"
    );
    assert!(!verdict.passed());
    assert_eq!(
        verdict.line(),
        "change coverage: 8 new public items introduced, 0 exercised — gate refused: 8 \
         untested: entry_path, parse_table_entries, render_entry, render_index, \
         is_self_contained, decide_append, contended_targets, hold_line"
    );
}

#[test]
fn incident_record_with_a_bare_suite_total_carries_no_signal() {
    // Invariant 2: `9065 passed, 0 failing` is true and says nothing about
    // the eight functions just added.
    let record = "CONVERTED+MERGED: 9065 passed, 0 failing";
    let coverage = ChangeCoverage::new(8, 0);
    assert!(
        !record_carries_coverage(record, &coverage),
        "a suite total is not change coverage: {record}"
    );
    let good = format!("{record}; {}", coverage.line());
    assert!(record_carries_coverage(&good, &coverage));
}

#[test]
fn mutation_verification_holding_out_hold_line_turns_the_audit_red() {
    // Invariant 4, made mechanical. The mutation: the fix's test file is
    // absent, so `hold_line` is missing from the exercised set. The audit
    // must fail against the mutation and pass once it is restored.
    let mutated = audit(&eight(), &seven_without_hold_line(), &[]);
    assert_eq!(
        mutated.untested,
        vec!["hold_line".to_string()],
        "with hold_line unexercised the audit flags exactly that item"
    );
    assert!(!mutated.complete());

    let restored = audit(&eight(), &eight(), &[]);
    assert!(restored.complete(), "restored, the audit passes");
    assert_eq!(restored.exercised, eight());
    assert!(gate(&eight(), &eight(), &[]).passed());
}

#[test]
fn exemption_with_a_reason_clears_one_item_and_names_the_reason() {
    // Invariant 1: "or state why not" is explicit and annotated, never
    // silent.
    let body = "\
        # docs-only renderer, exercised end-to-end by the index generator test
        # linter:allow-UNTESTED_PUBLIC render_index: docs-only, exercised by the generator e2e
        # linter:allow-UNTESTED_PUBLIC hold_line
        linter:allow-UNTESTED_PUBLIC_X hold_line: a different rule, not this one
        linter:allow-UNTESTED_PUBLIC render_entry:
    ";
    let exemptions = parse_exemptions(body);
    assert_eq!(
        exemptions,
        vec![Exemption {
            item: "render_index".to_string(),
            reason: "docs-only, exercised by the generator e2e".to_string(),
        }],
        "the bare marker, the empty reason, the different rule, and the \
         non-marker line are all rejected"
    );

    let audit = audit(&eight(), &[], &exemptions);
    assert_eq!(audit.exempted, vec!["render_index".to_string()]);
    assert_eq!(audit.untested.len(), 7);
    let verdict = gate(&eight(), &[], &exemptions);
    assert!(!verdict.passed());
    assert!(
        !verdict.line().contains("untested: render_index"),
        "the exempted item is not on the refusal list: {}",
        verdict.line()
    );
}

#[test]
fn control_all_exercised_passes_and_carries_its_coverage_line() {
    let verdict = gate(&eight(), &eight(), &[]);
    assert!(verdict.passed());
    assert_eq!(
        verdict.line(),
        "change coverage: 8 new public items introduced, 8 exercised — gate passed"
    );
    assert!(record_carries_coverage(
        &verdict.line(),
        &ChangeCoverage::new(8, 8)
    ));
}

#[test]
fn control_empty_patch_passes_with_zero_coverage() {
    let verdict = gate(&[], &[], &[]);
    assert!(verdict.passed());
    assert_eq!(
        verdict.line(),
        "change coverage: 0 new public items introduced, 0 exercised — gate passed"
    );
}

#[test]
fn diff_public_items_reports_additions_and_removals_in_order() {
    let before = vec![
        "entry_path".to_string(),
        "parse_table_entries".to_string(),
        "render_entry".to_string(),
    ];
    let after = vec![
        "render_entry".to_string(),
        "decide_append".to_string(),
        "hold_line".to_string(),
        "hold_line".to_string(),
    ];
    let diff = diff_public_items(&before, &after);
    assert_eq!(
        diff.added,
        vec!["decide_append".to_string(), "hold_line".to_string()],
        "additions keep `after` order and deduplicate"
    );
    assert_eq!(
        diff.removed,
        vec!["entry_path".to_string(), "parse_table_entries".to_string()],
        "removals keep `before` order"
    );
}

#[test]
fn exercised_means_exercised_by_the_patchs_own_tests() {
    // The existing suite staying green is not evidence about new code:
    // `exercised` is the patch's own test code, passed in as a fact the
    // caller searched for, and an item absent from it is untested — full
    // stop.
    let additions = vec!["hold_line".to_string()];
    let audit = audit(&additions, &["entry_path".to_string()], &[]);
    assert_eq!(
        audit.untested, additions,
        "naming a different item does not exercise hold_line"
    );
}
