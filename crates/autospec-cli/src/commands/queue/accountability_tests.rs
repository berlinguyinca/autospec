#[test]
fn accountability_epic_never_enters_the_implementation_review_queue() {
    let page = parse_remote_issue_page_json(
        r#"{"raw_count":1,"items":[{"number":3135,"title":"Run epic","body":"managed","labels":["auto-implement","autospec:run-accountability"],"author":{"login":"autospec"}}]}"#,
    )
    .expect("parse issue page");
    assert!(!reviewable_issue(&page.issues[0]));
}

/// A quarantine applied by a classifier defect must be recoverable by the same
/// typed reviewer that applied it.
///
/// `reviewable_issue` excludes `security:quarantined` unconditionally, so once
/// an issue is quarantined the Rust reviewer never looks at it again — not even
/// after the rule that quarantined it has been fixed. The only escape is a human
/// editing the security label by hand, which is precisely the action a guarded
/// environment refuses, so the sanctioned path has no way back at all.
///
/// This was not hypothetical: a false positive in `ci-or-review-bypass`
/// quarantined InferWeave/inferweave #1, #2, #5, #10, #50 and #123, and because
/// #1 is the sole transitive root, the whole 123-issue queue became
/// unrecoverable through any sanctioned path.
#[test]
fn a_quarantined_issue_is_reviewable_only_under_recheck() {
    let page = parse_remote_issue_page_json(
        r#"{"raw_count":1,"items":[{"number":1,"title":"P0-T01","body":"bootstrap","labels":["auto-implement","security:quarantined"],"author":{"login":"autospec"}}]}"#,
    )
    .expect("parse issue page");
    let issue = &page.issues[0];

    // Default stays sticky, exactly as today.
    assert!(!reviewable_issue(issue));
    // Under recheck the reviewer may re-derive a verdict for it.
    assert!(reviewable_issue_with_recheck(issue, true));
}

/// Recheck relaxes the quarantine gate and nothing else. An issue withheld for a
/// human, awaiting classification, or an accountability epic stays out of the
/// queue however it is asked for.
#[test]
fn recheck_does_not_relax_any_gate_other_than_quarantine() {
    for labels in [
        r#""auto-implement","autospec:needs-human""#,
        r#""auto-implement","needs-classify""#,
        r#""auto-implement","autospec:run-accountability""#,
    ] {
        let json = format!(
            r#"{{"raw_count":1,"items":[{{"number":7,"title":"t","body":"b","labels":[{labels}],"author":{{"login":"autospec"}}}}]}}"#
        );
        let page = parse_remote_issue_page_json(&json).expect("parse issue page");
        assert!(
            !reviewable_issue_with_recheck(&page.issues[0], true),
            "recheck wrongly admitted an issue held back by another gate: {labels}"
        );
    }
}
