// Heartbeat, lease and contention tests for the resilience store.
//
// include!()d into resilience.rs's `mod tests`, the same way policy_tests.rs and its
// siblings are: the parent file is past the size ratchet, and these ten tests share the
// helpers declared there.

    #[test]
    fn pid_liveness_requires_observed_process_absence() {
        assert!(!pid_is_dead(std::process::id()));
        assert!(!pid_is_dead(0));
        assert!(
            !pid_is_dead(i32::MAX as u32 + 1),
            "an unrepresentable PID is unknown, not proven dead"
        );

        let mut child = ChildGuard(
            Command::new("sleep")
                .arg("60")
                .spawn()
                .expect("start liveness child"),
        );
        let pid = child.0.id();
        assert!(!pid_is_dead(pid));
        child.stop_and_reap();
        assert!(pid_is_dead(pid));
    }

    #[test]
    fn competing_conductors_hold_one_owner_before_operator_write() {
        let root = test_root("contention");
        let lock_root = root.join("lock-holder");
        let ready = lock_root.join("ready");
        let release = lock_root.join("release");
        let mut child = Command::new(std::env::current_exe().expect("test executable"))
            .args([
                "--exact",
                "commands::autonomous::resilience::tests::lease_lock_holder_child",
                "--nocapture",
            ])
            .env(LEASE_LOCK_TEST_ROOT, &lock_root)
            .spawn()
            .expect("start lease lock holder");

        wait_for(&ready, "lease lock holder readiness");
        let contended = test_store(&lock_root).acquire(None, 1, 1);
        assert!(matches!(contended, Err(StoreError::Held)));
        assert!(!test_store(&lock_root).canonical_state_path().exists());
        fs::write(&release, "release\n").expect("release lock holder");
        assert!(child.wait().expect("wait for lock holder").success());

        let first = test_store(&root);
        let second = test_store(&root);
        let (admission, lease) = match first.acquire(None, 1, 1) {
            Ok(value) => value,
            Err(_) => panic!("first conductor must acquire the lease"),
        };
        assert!(admission.lease.is_none());
        assert!(matches!(second.acquire(None, 1, 1), Err(StoreError::Held)));

        let state = match first.read_state() {
            Ok(Some((state, false))) => state,
            _ => panic!("claimed state must be canonical"),
        };
        assert_eq!(state.status, "claimed");
        assert_eq!(state.lease_token.as_deref(), Some(lease.token.as_str()));
        assert_eq!(state.lease_generation, Some(lease.generation));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn stale_token_cannot_adopt_or_release_reclaimed_lease() {
        let root = test_root("fencing");
        let first = test_store(&root);
        let second = test_store(&root);
        let (_, stale_lease) = match first.acquire(None, 1, 1) {
            Ok(value) => value,
            Err(_) => panic!("first conductor must acquire the lease"),
        };
        let mut stale_state = match first.read_state() {
            Ok(Some((state, false))) => state,
            _ => panic!("first acquisition must write canonical state"),
        };
        stale_state.heartbeat_at = Some(now_secs().saturating_sub(300));
        match first.write_state(&stale_state) {
            Ok(()) => {}
            Err(_) => panic!("make first lease reclaimable"),
        }

        let (_, replacement) = match second.acquire(None, 1, 1) {
            Ok(value) => value,
            Err(_) => panic!("second conductor must reclaim the stale lease"),
        };
        assert_ne!(replacement.token, stale_lease.token);
        assert_eq!(replacement.generation, stale_lease.generation + 1);
        let replacement_state =
            fs::read_to_string(second.canonical_state_path()).expect("read replacement state");

        assert_token_mismatch(|| first.adopt(&stale_lease.token));
        assert_eq!(
            fs::read_to_string(second.canonical_state_path())
                .expect("read state after stale adopt"),
            replacement_state
        );
        assert_token_mismatch(|| first.release(&stale_lease));
        assert_eq!(
            fs::read_to_string(second.canonical_state_path())
                .expect("read state after stale release"),
            replacement_state
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn terminated_owner_release_requires_the_exact_dead_pid() {
        let root = test_root("terminated-owner");
        let store = test_store(&root);
        let (_, _) = store
            .acquire(None, 1, 1)
            .unwrap_or_else(|_| panic!("acquire owner lease"));
        let before =
            fs::read_to_string(store.canonical_state_path()).expect("read owner lease state");

        assert_eq!(
            store
                .release_terminated_owner(dead_child_pid())
                .unwrap_or_else(|_| panic!("reject a mismatched dead pid")),
            TerminatedOwnerRelease::OwnerMismatch
        );
        assert_eq!(
            fs::read_to_string(store.canonical_state_path())
                .expect("read state after mismatched release"),
            before
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn terminated_owner_release_handles_a_dead_claim_owner_before_adoption() {
        let root = test_root("terminated-claimed-owner");
        let store = test_store(&root);
        let (_, _) = store
            .acquire(None, 1, 1)
            .unwrap_or_else(|_| panic!("acquire claimed lease"));
        let mut child = ChildGuard(
            Command::new("sleep")
                .arg("60")
                .spawn()
                .expect("start claimed owner"),
        );
        let claimed_pid = child.0.id();
        let mut state = store
            .read_state()
            .unwrap_or_else(|_| panic!("read claimed state"))
            .expect("claimed state")
            .0;
        state.lock_pid = Some(claimed_pid);
        store
            .write_state(&state)
            .unwrap_or_else(|_| panic!("record claimed owner"));
        child.stop_and_reap();

        assert_eq!(
            store
                .release_terminated_owner(dead_child_pid())
                .unwrap_or_else(|_| panic!("release abandoned claim")),
            TerminatedOwnerRelease::Released
        );
        let released = store
            .read_state()
            .unwrap_or_else(|_| panic!("read released state"))
            .expect("released state")
            .0;
        assert_eq!(released.status, "released");
        assert!(released.lock_pid.is_none());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn owned_lease_renewal_prevents_abandonment_reclaim() {
        let root = test_root("renewal");
        let owner = test_store(&root);
        let contender = test_store(&root);
        let (_, claimed) = owner
            .acquire(None, 1, 1)
            .unwrap_or_else(|_| panic!("claim lease"));
        let lease =
            retry_lease(|| owner.adopt(&claimed.token)).unwrap_or_else(|_| panic!("adopt lease"));
        let mut stale_state = owner
            .read_state()
            .unwrap_or_else(|_| panic!("read state"))
            .expect("running state")
            .0;
        stale_state.heartbeat_at = Some(now_secs().saturating_sub(10_801));
        owner
            .write_state(&stale_state)
            .unwrap_or_else(|_| panic!("age running lease"));

        retry_lease(|| owner.renew(&lease)).unwrap_or_else(|_| panic!("renew owned lease"));

        assert!(matches!(
            contender.acquire(None, 1, 1),
            Err(StoreError::Held)
        ));
        let renewed = owner
            .read_state()
            .unwrap_or_else(|_| panic!("read renewed state"))
            .expect("renewed state")
            .0;
        assert!(renewed.heartbeat_at > stale_state.heartbeat_at);
        retry_transient_lock(
            || owner.release(&lease),
            |result| matches!(result, Err(StoreError::Held)),
        )
        .unwrap_or_else(|_| panic!("release renewed lease"));
        assert_eq!(
            owner
                .read_state()
                .unwrap_or_else(|_| panic!("read released state"))
                .expect("released state")
                .0
                .status,
            "released"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn heartbeat_survives_owned_transaction_contention_and_renews_later() {
        let root = test_root("heartbeat-contention");
        let owner = test_store(&root);
        let (_, claimed) = owner
            .acquire(None, 1, 1)
            .unwrap_or_else(|_| panic!("claim lease"));
        let lease =
            retry_lease(|| owner.adopt(&claimed.token)).unwrap_or_else(|_| panic!("adopt lease"));
        let mut stale_state = owner
            .read_state()
            .unwrap_or_else(|_| panic!("read state"))
            .expect("running state")
            .0;
        stale_state.heartbeat_at = Some(now_secs().saturating_sub(10_801));
        owner
            .write_state(&stale_state)
            .unwrap_or_else(|_| panic!("age running lease"));

        let transaction = LeaseTransaction::try_open(&owner.lock_path())
            .unwrap_or_else(|_| panic!("hold owned lease transaction"));
        let heartbeat = start_lifecycle_heartbeat_with_store(
            test_store(&root),
            lease.clone(),
            Duration::from_millis(10),
        )
        .unwrap_or_else(|_| panic!("start heartbeat despite contention"));
        thread::sleep(Duration::from_millis(90));
        assert!(
            !heartbeat
                .handle
                .as_ref()
                .expect("heartbeat handle")
                .is_finished(),
            "transient contention must not terminate the heartbeat"
        );

        drop(transaction);
        // Wait for the renewal instead of sleeping a fixed slice. The heartbeat publishes from
        // its own thread on a 10 ms interval, so a loaded machine can miss a 100 ms window
        // entirely -- a scheduling fact about the host, not a defect in the code under test.
        // Seen as a 1-of-811 failure of the assertion below while this box was busy.
        let renewal_deadline = Instant::now() + Duration::from_secs(10);
        let mut renewed_after_contention = false;
        while Instant::now() < renewal_deadline {
            if let Ok(Some((state, _))) = owner.read_state() {
                if state.heartbeat_at > stale_state.heartbeat_at {
                    renewed_after_contention = true;
                    break;
                }
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(
            renewed_after_contention,
            "heartbeat never renewed once contention cleared"
        );
        heartbeat
            .finish()
            .unwrap_or_else(|_| panic!("finish surviving heartbeat"));
        let renewed = owner
            .read_state()
            .unwrap_or_else(|_| panic!("read renewed state"))
            .expect("renewed state")
            .0;
        assert!(renewed.heartbeat_at > stale_state.heartbeat_at);
        retry_transient_lock(
            || owner.release(&lease),
            |result| matches!(result, Err(StoreError::Held)),
        )
        .unwrap_or_else(|_| panic!("release lease"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn replaced_lease_cannot_persist_tier_one_waterfall_artifacts() {
        let root = test_root("waterfall-fencing");
        let first = test_store(&root);
        let second = test_store(&root);
        let (_, stale_lease) = match first.acquire(None, 1, 1) {
            Ok(value) => value,
            Err(_) => panic!("first lease"),
        };
        let mut stale_state = match first.read_state() {
            Ok(Some((state, _))) => state,
            _ => panic!("first lease state"),
        };
        stale_state.heartbeat_at = Some(now_secs().saturating_sub(300));
        if first.write_state(&stale_state).is_err() {
            panic!("expire first lease");
        }
        let (_, replacement) = match second.acquire(None, 1, 1) {
            Ok(value) => value,
            Err(_) => panic!("replacement lease"),
        };
        assert_ne!(replacement.token, stale_lease.token);

        let plan = ReadyQueuePlan {
            ready: Vec::new(),
            blocked: Vec::new(),
            claimed: Vec::new(),
            conflicts: Vec::new(),
            worker_cap: WorkerCap {
                max_repo_workers: 1,
                active_count: 0,
                remaining: 1,
                reached: false,
            },
            batch: Vec::new(),
            gate_counts: QueueGateCounts::default(),
        };
        let waterfall_root = root.join("operator");
        let policy = super::super::waterfall_policy::WaterfallPolicy::from_config(
            &autospec_core::autonomous::config::AutonomousConfig::default(),
        )
        .expect("default waterfall policy");
        let result = super::super::waterfall_coordinator::record_tier_one(
            &waterfall_root,
            "owner/repo",
            &stale_lease,
            &policy,
            super::super::waterfall_coordinator::Tier1QueueEvidence::EmptyPage(&plan),
        );

        assert!(result.is_err(), "a replaced lease must fail closed");
        assert!(
            !waterfall_root.join("waterfall").exists(),
            "lease validation must precede waterfall lock and artifact creation"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn matching_token_adopts_and_releases_its_lease() {
        let root = test_root("adopt-release");
        let store = test_store(&root);
        let (_, claimed) = match store.acquire(None, 1, 1) {
            Ok(value) => value,
            Err(_) => panic!("acquire claimed lease"),
        };

        let adopted = match retry_transient_lock(
            || store.adopt(&claimed.token),
            |result| matches!(result, Err(StoreError::Held)),
        ) {
            Ok(lease) => lease,
            Err(_) => panic!("adopt matching lease"),
        };
        assert_eq!(adopted.token, claimed.token);
        assert_eq!(adopted.generation, claimed.generation);
        let running = match store.read_state() {
            Ok(Some((state, _))) => state,
            _ => panic!("read adopted state"),
        };
        assert_eq!(running.status, "running");
        assert_eq!(running.lease_token.as_deref(), Some(adopted.token.as_str()));
        assert_eq!(running.lease_generation, Some(adopted.generation));
        assert_eq!(running.lock_pid, Some(std::process::id()));
        assert_eq!(running.lock_host.as_deref(), Some("autospec-test-host"));
        assert!(running.lock_acquired_at.is_some());

        match retry_transient_lock(
            || store.release(&adopted),
            |result| matches!(result, Err(StoreError::Held)),
        ) {
            Ok(()) => {}
            Err(_) => panic!("release matching lease"),
        }
        let released = match store.read_state() {
            Ok(Some((state, _))) => state,
            _ => panic!("read released state"),
        };
        assert_eq!(released.status, "released");
        assert!(released.lease_token.is_none());
        assert_eq!(released.lease_generation, Some(adopted.generation));
        assert!(released.lock_pid.is_none());
        assert!(released.lock_host.is_none());
        assert!(released.lock_session.is_none());
        assert!(released.lock_acquired_at.is_none());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn lease_lock_holder_child() {
        let Ok(root) = std::env::var(LEASE_LOCK_TEST_ROOT) else {
            return;
        };
        let root = PathBuf::from(root);
        let store = test_store(&root);
        let _transaction = match LeaseTransaction::try_open(&store.lock_path()) {
            Ok(transaction) => transaction,
            Err(_) => panic!("child must acquire lease transaction lock"),
        };
        fs::write(root.join("ready"), "ready\n").expect("mark lock holder ready");
        wait_for(&root.join("release"), "lease lock holder release");
    }

    fn test_store(root: &Path) -> ResilienceStore {
        ResilienceStore {
            scope: RepositoryScope::try_from("owner/repo").expect("repository scope"),
            state_root: root.join("state").join("autonomous"),
            spend_root: root.join("spend"),
            host: "autospec-test-host".to_string(),
        }
    }

    fn test_root(name: &str) -> PathBuf {
        let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "autospec-resilience-lease-{name}-{}-{}-{sequence}",
            std::process::id(),
            now_nanos()
        ));
        fs::create_dir_all(&root).expect("create test root");
        root
    }
