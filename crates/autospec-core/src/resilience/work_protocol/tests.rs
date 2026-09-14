    use super::*;

    fn make_work() -> WorkItem {
        WorkItem {
            work_id: WorkId::new(b"w"),
            state: WorkState::Created,
            non_happy: None,
            current_attempt: None,
            created_at: 0,
            updated_at: 0,
            attempts: Vec::new(),
        }
    }

    #[test]
    fn canonical_transitions_are_forward() {
        assert!(can_transition(WorkState::Created, WorkState::Assigned));
        assert!(can_transition(WorkState::Assigned, WorkState::Delivered));
        assert!(can_transition(WorkState::Delivered, WorkState::Claimed));
        assert!(can_transition(WorkState::Claimed, WorkState::Running));
        assert!(can_transition(WorkState::Running, WorkState::Completed));
        assert!(can_transition(WorkState::Completed, WorkState::Validated));
        assert!(can_transition(WorkState::Validated, WorkState::Reviewed));
        assert!(can_transition(WorkState::Reviewed, WorkState::Merged));
        // Illegal: skipping or jumping backward.
        assert!(!can_transition(WorkState::Created, WorkState::Running));
        assert!(!can_transition(WorkState::Merged, WorkState::Created));
    }

    #[test]
    fn transition_rejects_illegal_and_model_override() {
        let mut w = make_work();
        // A model cannot skip straight to Merged (deterministic gate).
        assert!(transition(&mut w, WorkState::Merged, 1).is_err());
        assert!(transition(&mut w, WorkState::Assigned, 1).is_ok());
        // Cannot move a non-happy item forward via naive transition.
        w.non_happy = Some(WorkStateNonHappy::Blocked);
        assert!(transition(&mut w, WorkState::Completed, 2).is_err());
    }

    #[test]
    fn lease_acquire_heartbeat_expiry_reclaim_fencing() {
        let mut store = InMemoryWorkStore::default();
        let work_id = WorkId::new(b"w");
        let mut work = make_work();
        work.work_id = work_id.clone();
        store.insert_work(work);

        let clock = FixedClock(1000);
        let attempt = AttemptId::new(b"a");
        let session = SessionId::new(b"s");
        let acquired = acquire_lease(
            &mut store,
            &work_id,
            &attempt,
            &session,
            1000,
            DEFAULT_LEASE_SECONDS,
            &clock,
        );
        let lease = match acquired {
            AcquireResult::Acquired(l) => l,
            other => panic!("expected acquired, got {:?}", other),
        };

        // Heartbeat renews.
        assert!(heartbeat(&mut store, &work_id, &session, lease.fencing_generation, 1100));
        // Wrong generation is rejected (stale worker).
        assert!(!heartbeat(&mut store, &work_id, &session, lease.fencing_generation + 1, 1200));

        // Finalize works with correct generation, fails with stale.
        assert!(try_finalize(&mut store, &work_id, &attempt, &session, lease.fencing_generation).is_ok());
        assert!(try_finalize(&mut store, &work_id, &attempt, &session, lease.fencing_generation + 1).is_err());

        // Expiry + reclaim bumps generation. The lease was renewed at 1100 so
        // it expires at 1400; use a now past that renewal.
        let now = 1100 + DEFAULT_LEASE_SECONDS + 10;
        assert!(reclaim_expired(&mut store, &work_id, now));
        let new_lease = store.load_lease(&work_id).unwrap();
        assert_eq!(new_lease.fencing_generation, lease.fencing_generation + 1);
        // Old generation can no longer finalize.
        assert!(try_finalize(&mut store, &work_id, &attempt, &session, lease.fencing_generation).is_err());
    }

    #[test]
    fn duplicate_delivery_is_idempotent() {
        let mut store = InMemoryWorkStore::default();
        let receipt = WorkReceipt::new(
            ReceiptId::new(b"r"),
            IdempotencyKey::new(b"k"),
            WorkId::new(b"w"),
            AttemptId::new(b"a"),
            "worker-1",
            ReceiptStage::Delivered,
            1,
        );
        assert!(!record_delivery(&mut store, &receipt)); // first delivery
        assert!(record_delivery(&mut store, &receipt)); // duplicate
    }

    #[test]
    fn recovery_blocks_when_no_lease_and_retries_when_expired() {
        let mut store = InMemoryWorkStore::default();
        let work_id = WorkId::new(b"w");
        let mut work = make_work();
        work.work_id = work_id.clone();
        store.insert_work(work);

        assert_eq!(reconcile(&store, &work_id, 1000), RecoveryAction::Block);

        let clock = FixedClock(1000);
        let attempt = AttemptId::new(b"a");
        let session = SessionId::new(b"s");
        acquire_lease(&mut store, &work_id, &attempt, &session, 1000, DEFAULT_LEASE_SECONDS, &clock);
        assert_eq!(reconcile(&store, &work_id, 1100), RecoveryAction::Resume);
        // After expiry, deterministic retry.
        assert_eq!(
            reconcile(&store, &work_id, 1000 + DEFAULT_LEASE_SECONDS + 1),
            RecoveryAction::Retry
        );
    }

    #[test]
    fn terminal_work_is_not_reacquired() {
        let mut store = InMemoryWorkStore::default();
        let work_id = WorkId::new(b"w");
        let mut work = make_work();
        work.work_id = work_id.clone();
        work.state = WorkState::Merged;
        store.insert_work(work);
        let clock = FixedClock(1000);
        let result = acquire_lease(
            &mut store,
            &work_id,
            &AttemptId::new(b"a"),
            &SessionId::new(b"s"),
            1000,
            DEFAULT_LEASE_SECONDS,
            &clock,
        );
        assert_eq!(result, AcquireResult::Terminal);
    }
