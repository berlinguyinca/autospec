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
    assert!(reviewable_issue_with_recheck(
        issue,
        RecheckScope {
            recheck: true,
            targeted: false,
        }
    ));
}

/// A bulk recheck relaxes the quarantine gate and nothing else. An issue
/// withheld for a human, awaiting classification, or an accountability epic
/// stays out of the queue when the sweep is not aimed at it.
#[test]
fn an_untargeted_recheck_relaxes_no_gate_other_than_quarantine() {
    let sweep = RecheckScope {
        recheck: true,
        targeted: false,
    };
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
            !reviewable_issue_with_recheck(&page.issues[0], sweep),
            "an untargeted recheck wrongly admitted an issue held back by another gate: {labels}"
        );
    }
}

/// A recheck aimed at one issue may re-derive an AMBIGUOUS verdict, and only
/// that. `autospec:needs-human` is overloaded — the safety reviewer writes it
/// for AMBIGUOUS, the orchestrator writes it when an implementer failed
/// repeatedly — so naming the issue is what separates "re-judge this verdict"
/// from a sweep that would clear failures it cannot re-derive.
///
/// Without this, a needs-human label applied by a classifier defect outlived the
/// fix to that defect: InferWeave/inferweave#6 was the sole dependency-ready
/// task in a 133-task graph, so one stale label idled the other 124 issues with
/// no sanctioned way back.
#[test]
fn a_targeted_recheck_admits_needs_human_but_still_no_other_gate() {
    let aimed = RecheckScope {
        recheck: true,
        targeted: true,
    };
    let page = parse_remote_issue_page_json(
        r#"{"raw_count":1,"items":[{"number":6,"title":"P0-T05","body":"storage","labels":["auto-implement","autospec:needs-human"],"author":{"login":"autospec"}}]}"#,
    )
    .expect("parse issue page");
    assert!(reviewable_issue_with_recheck(&page.issues[0], aimed));

    for labels in [
        r#""auto-implement","needs-classify""#,
        r#""auto-implement","autospec:run-accountability""#,
    ] {
        let json = format!(
            r#"{{"raw_count":1,"items":[{{"number":7,"title":"t","body":"b","labels":[{labels}],"author":{{"login":"autospec"}}}}]}}"#
        );
        let page = parse_remote_issue_page_json(&json).expect("parse issue page");
        assert!(
            !reviewable_issue_with_recheck(&page.issues[0], aimed),
            "a targeted recheck wrongly admitted an issue held back by another gate: {labels}"
        );
    }
}
