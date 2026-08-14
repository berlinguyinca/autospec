#[test]
fn released_generation_is_a_fenced_predecessor_proof_for_successor_acquisition() {
    let root = test_root("released-generation-proof");
    let store = test_store(&root);
    let (_, first) = store
        .acquire(None, 1, 1)
        .unwrap_or_else(|_| panic!("acquire first lifecycle"));
    store
        .release(&first)
        .unwrap_or_else(|_| panic!("release first lifecycle"));

    let (_, successor) = store
        .acquire(None, 1, 1)
        .unwrap_or_else(|_| panic!("acquire successor lifecycle"));

    assert_eq!(successor.generation(), first.generation() + 1);
    assert_eq!(
        successor.accountability_predecessor_generation(),
        Some(first.generation())
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn reclaimed_stale_running_generation_is_not_an_accountability_predecessor_proof() {
    let root = test_root("stale-running-generation-proof");
    let store = test_store(&root);
    let (_, first) = store
        .acquire(None, 1, 1)
        .unwrap_or_else(|_| panic!("acquire stale lifecycle"));
    let mut stale = store
        .read_state()
        .unwrap_or_else(|_| panic!("read stale lifecycle"))
        .expect("stale lifecycle state")
        .0;
    stale.status = "running".to_owned();
    stale.heartbeat_at = Some(1);
    stale.lock_pid = Some(u32::MAX);
    store
        .write_state(&stale)
        .unwrap_or_else(|_| panic!("write stale lifecycle"));

    let (_, successor) = store
        .acquire(None, 1, 1)
        .unwrap_or_else(|_| panic!("reclaim stale lifecycle"));

    assert_eq!(successor.generation(), first.generation() + 1);
    assert_eq!(successor.accountability_predecessor_generation(), None);
    let _ = fs::remove_dir_all(root);
}
