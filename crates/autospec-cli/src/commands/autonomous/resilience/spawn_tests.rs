    #[test]
    fn matching_token_transfers_a_pre_spawn_renewed_lease() {
        let root = test_root("adopt-pre-spawn-renewed");
        let store = test_store(&root);
        let (_, claimed) = match store.acquire(None, 1, 1) {
            Ok(value) => value,
            Err(_) => panic!("acquire claimed lease"),
        };
        if store.renew(&claimed).is_err() {
            panic!("epic reconciliation renews before spawn");
        }

        let before = match store.read_state() {
            Ok(Some((state, _))) => state,
            _ => panic!("read renewed state"),
        };
        assert_eq!(before.status, "running");
        let adopted = match store.adopt(&claimed.token) {
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
