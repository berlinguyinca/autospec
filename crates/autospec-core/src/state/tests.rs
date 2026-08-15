use super::{ParentIssueStatus, SpecStateStore};

#[test]
fn extend_parent_decomposition_preserves_terminal_children_and_is_idempotent() {
    let mut store = SpecStateStore::new();
    store
        .record_parent_decomposition(10, vec![11, 12], false)
        .expect("initial decomposition");
    store.record_child_terminal(11).expect("merged child");
    let extended = store
        .extend_parent_decomposition(10, vec![11, 12, 13])
        .expect("ordered extension");
    assert!(extended.changed);
    assert_eq!(extended.added_children, vec![13]);
    assert!(extended
        .comment_body
        .contains("append-only-parent-extension"));

    let repeated = store
        .extend_parent_decomposition(10, vec![11, 12, 13])
        .expect("same full list");
    assert!(!repeated.changed);
    store
        .record_child_terminal(12)
        .expect("second merged child");
    store
        .record_child_terminal(13)
        .expect("extension merged child");
    assert_eq!(
        store.parent_issue_status(10),
        Some(ParentIssueStatus::CompleteButStale)
    );
}

#[test]
fn extend_parent_decomposition_rejects_non_append_and_owned_children() {
    let mut store = SpecStateStore::new();
    store
        .record_parent_decomposition(10, vec![11, 12], false)
        .expect("initial decomposition");
    store
        .record_parent_decomposition(20, vec![21], false)
        .expect("other decomposition");

    for children in [
        vec![11],
        vec![12, 11, 13],
        vec![11, 12, 10],
        vec![11, 12, 13, 13],
    ] {
        assert!(
            store.extend_parent_decomposition(10, children).is_err(),
            "invalid full list must fail closed"
        );
    }
    assert!(store
        .extend_parent_decomposition(10, vec![11, 12, 21])
        .expect_err("child owned by another parent")
        .to_string()
        .contains("parent #20"));
    assert_eq!(store.parent_issue_children(10), Some(vec![11, 12]));
}
