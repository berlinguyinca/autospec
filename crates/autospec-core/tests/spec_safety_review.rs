use autospec_core::spec::{lint_spec_safety, SAFETY_WITHOUT_MECHANISM_RULE_ID};

fn flagged_lines(findings: &[autospec_core::spec::SpecSafetyFinding]) -> Vec<(usize, &str)> {
    findings
        .iter()
        .map(|finding| (finding.line, finding.phrase.as_str()))
        .collect()
}

/// The incident that motivated the review: a spec that states "never touch
/// the live file" by convention only is flagged, while the same property
/// backed by a gate is not.
#[test]
fn convention_only_safety_property_is_flagged() {
    let spec =
        "# Journaling design\n\n## Invariants\n\nThe pipeline never touches the live file.\n";
    let findings = lint_spec_safety(spec);

    assert_eq!(
        flagged_lines(&findings),
        vec![(5, "never")],
        "only the convention-only line should be flagged"
    );
}

#[test]
fn structurally_enforced_safety_property_passes() {
    let spec = "# Journaling design\n\n## Invariants\n\nA snapshot lock is taken before any write, so the writer never sees a torn page.\n";
    assert!(
        lint_spec_safety(spec).is_empty(),
        "a line naming its mechanism is structurally enforced"
    );
}

#[test]
fn mixed_spec_reports_only_convention_only_lines_in_order() {
    let spec = "\
# Mixed spec

## Invariants

- The loader always fails closed on an ambiguous digest.
- Make sure to delete the staging tree by hand.
- The gate rejects a second writer, so the lock is never held twice.
- Be careful when replaying archived sessions.
";
    let findings = lint_spec_safety(spec);

    assert_eq!(
        flagged_lines(&findings),
        vec![(6, "make sure to"), (8, "be careful")],
        "mechanized lines 5 and 7 must pass; convention lines 6 and 8 must not"
    );
    assert!(findings
        .iter()
        .all(|finding| finding.rule_id() == SAFETY_WITHOUT_MECHANISM_RULE_ID));
}

#[test]
fn code_fences_are_not_spec_prose() {
    let spec = "\
# Validation commands

```bash
# never run this against the live state
rm -rf staging
```

The command always fails closed on a dirty tree.
";
    assert!(
        lint_spec_safety(spec).is_empty(),
        "imperative words inside a code fence are not safety properties"
    );
}

#[test]
fn finding_text_preserves_the_offending_line() {
    let findings = lint_spec_safety("Never push to main.\n");
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].text, "Never push to main.");
    assert!(findings[0].message().contains("convention-only"));
}
