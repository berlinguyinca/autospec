    fn retry_lease<T>(operation: impl FnMut() -> Result<T, StoreError>) -> Result<T, StoreError> {
        retry_transient_lock(operation, |result| matches!(result, Err(StoreError::Held)))
    }

    #[test]
    fn matching_token_transfers_a_pre_spawn_renewed_lease() {
        let root = test_root("adopt-pre-spawn-renewed");
        let store = test_store(&root);
        let (_, claimed) = match store.acquire(None, 1, 1) {
            Ok(value) => value,
            Err(_) => panic!("acquire claimed lease"),
        };
        retry_lease(|| store.renew(&claimed))
            .unwrap_or_else(|_| panic!("epic reconciliation renews before spawn"));

        let before = match store.read_state() {
            Ok(Some((state, _))) => state,
            _ => panic!("read renewed state"),
        };
        assert_eq!(before.status, "claimed");
        let adopted = match retry_lease(|| store.adopt(&claimed.token)) {
            Ok(lease) => lease,
            Err(_) => panic!("spawned child must adopt the exact renewed generation"),
        };

        assert_eq!(adopted.token, claimed.token);
        assert_eq!(adopted.generation, claimed.generation);
        let after = match store.read_state() {
            Ok(Some((state, _))) => state,
            _ => panic!("read transferred state"),
        };
        assert_eq!(after.status, "running");
        assert_eq!(after.lease_generation, Some(claimed.generation));
        assert_eq!(after.lock_pid, Some(std::process::id()));
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn launch_heartbeat_preserves_the_claimed_transfer_window() {
        let root = test_root("claimed-heartbeat");
        let owner = test_store(&root);
        let (_, claimed) = owner
            .acquire(None, 1, 1)
            .unwrap_or_else(|_| panic!("claim lease"));

        retry_lease(|| owner.renew(&claimed)).unwrap_or_else(|_| panic!("renew claimed lease"));

        let renewed = owner
            .read_state()
            .unwrap_or_else(|_| panic!("read renewed claim"))
            .expect("renewed claim")
            .0;
        assert_eq!(renewed.status, "claimed");
        assert_eq!(renewed.lock_pid, Some(std::process::id()));
        let _ = fs::remove_dir_all(root);
    }
