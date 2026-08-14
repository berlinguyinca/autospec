#[test]
fn accountability_epic_never_enters_the_implementation_review_queue() {
    let page = parse_remote_issue_page_json(
        r#"{"raw_count":1,"items":[{"number":3135,"title":"Run epic","body":"managed","labels":["auto-implement","autospec:run-accountability"],"author":{"login":"autospec"}}]}"#,
    )
    .expect("parse issue page");
    assert!(!reviewable_issue(&page.issues[0]));
}
