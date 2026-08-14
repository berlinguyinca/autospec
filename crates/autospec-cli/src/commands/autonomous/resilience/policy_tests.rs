#[allow(dead_code)]
fn acquisition_does_not_claim_when_core_policy_parks_or_rejects() {
    let capacity_root = test_root("capacity");
    let capacity = test_store(&capacity_root);
    fs::create_dir_all(capacity.spend_root.join("owner__repo")).expect("create spend root");
    fs::write(
        capacity.spend_root.join("owner__repo/spend.json"),
        "{\"schema\":1,\"tokens\":1,\"issues\":0}",
    )
    .expect("write spend record");
    assert!(matches!(
        capacity.acquire(None, 1, 1),
        Err(StoreError::Policy(admission)) if matches!(admission.capacity, CapacityDecision::UsageCap)
    ));
    assert!(!capacity.canonical_state_path().exists());

    let failure_root = test_root("failure");
    let failure = test_store(&failure_root);
    let failure_path = failure.state_root.join("owner__repo/issues/42.json");
    fs::create_dir_all(failure_path.parent().expect("failure parent"))
        .expect("create failure root");
    fs::write(
        failure_path,
        "{\"issue\":42,\"failures\":3,\"updated_at\":1}",
    )
    .expect("write failure record");
    assert!(matches!(
        failure.acquire(Some(42), 1, 1),
        Err(StoreError::Policy(admission)) if admission.failure_count == 3
    ));
    assert!(!failure.canonical_state_path().exists());

    let _ = fs::remove_dir_all(capacity_root);
    let _ = fs::remove_dir_all(failure_root);
}
