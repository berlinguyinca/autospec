//! The conductor pre-check must displace an owner whose lease has expired.
//!
//! A dead worker cannot release its own claim, and `claim release` validates the
//! caller's identity, so an unconditional refusal wedged the issue permanently.

use super::super::lease::conductor_claim_owner_holds_lease;
use autospec_core::claim::RunStateRecord;

pub(super) fn owner_record(updated_at: &str, ttl_seconds: u64, step: &str) -> RunStateRecord {
    RunStateRecord::new(
        "owner/repo",
        42,
        "rust-foreground-conductor-dead-1785286182924880200",
        "claimed",
        "feat/worker".to_string(),
        "",
        step,
        Vec::new(),
        "2026-07-29T00:49:45Z",
        updated_at,
        ttl_seconds,
    )
    .with_claim_id("claim-a")
}

#[test]
fn an_owner_whose_lease_has_expired_no_longer_holds_it() {
    let record = owner_record("2026-07-29T00:49:45Z", 10800, "verification");

    assert!(
        !conductor_claim_owner_holds_lease(&record),
        "a claim last touched in July must not block a fresh worker forever"
    );
}

#[test]
fn an_owner_still_inside_its_ttl_keeps_the_lease() {
    let updated_at = super::super::utc_now_iso().expect("current timestamp");
    let record = owner_record(&updated_at, 10800, "verification");

    assert!(
        conductor_claim_owner_holds_lease(&record),
        "a live worker one minute into a three-hour lease must not be displaced"
    );
}

#[test]
fn an_owner_mid_heartbeat_publish_keeps_the_lease_even_when_stale() {
    let record = owner_record(
        "2026-07-29T00:49:45Z",
        10800,
        "heartbeat-publishing:verification",
    );

    assert!(
        conductor_claim_owner_holds_lease(&record),
        "a publish in flight is protected regardless of the recorded timestamp"
    );
}

#[test]
fn an_unparseable_timestamp_keeps_the_lease() {
    let record = owner_record("not-a-timestamp", 10800, "verification");

    assert!(
        conductor_claim_owner_holds_lease(&record),
        "an unreadable record must fail closed and keep the owner"
    );
}

mod requeue {
    use super::super::super::lease::{
        acquisition_blocking_owner, claim_is_abandoned, owner_still_holds,
        quarantine_abandoned_claim_generation_with,
    };
    use super::super::super::{ClaimRefAdvance, ClaimRefHead};
    use super::owner_record;
    #[cfg(target_os = "linux")]
    use crate::commands::claim::tests::support::{
        startup_heartbeat_fixture, STARTUP_HEARTBEAT_ENV,
    };
    use crate::commands::claim::utc_now_iso;
    #[cfg(target_os = "linux")]
    use std::os::unix::fs::PermissionsExt;

    #[cfg(target_os = "linux")]
    fn install_dead_heartbeat(root: &std::path::Path, worker_id: &str, claim_id: &str) {
        let repo = root.join(crate::commands::autonomous::drain::repository_progress_key(
            "owner/repo",
        ));
        std::fs::create_dir_all(&repo).expect("heartbeat repository");
        for directory in [root, repo.as_path()] {
            std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))
                .expect("private heartbeat directory");
        }
        let host = std::fs::read_to_string("/proc/sys/kernel/hostname")
            .expect("hostname")
            .trim()
            .to_string();
        let boot_id = crate::commands::autonomous::current_boot_identity().expect("boot identity");
        let nonce = super::super::super::startup_heartbeat_nonce("owner/repo", 42, claim_id);
        let document = format!(
            r#"{{"repo":"owner/repo","issue":"42","worker_id":"{worker_id}","branch":"feat/worker","pr":"","claim_id":"{claim_id}","step":"claimed","ts":1,"ttl_seconds":1,"pid":2147483647,"nonce":"{nonce}","host":"{host}","boot_id":"{boot_id}","process_start":"1"}}"#
        );
        let heartbeat = repo.join("42.json");
        std::fs::write(&heartbeat, document).expect("dead current heartbeat");
        std::fs::set_permissions(heartbeat, std::fs::Permissions::from_mode(0o600))
            .expect("private heartbeat file");
    }

    fn record(state: &str, updated_at: &str) -> autospec_core::claim::RunStateRecord {
        let mut record = owner_record(updated_at, 10800, "verification");
        record.state = state.to_string();
        record
    }

    fn ready_record(updated_at: &str) -> autospec_core::claim::RunStateRecord {
        owner_record(updated_at, 10800, "heartbeat-ready:none")
    }

    fn lose_generation(
        expected: Option<&ClaimRefHead>,
        successor: &autospec_core::claim::RunStateRecord,
    ) -> Result<ClaimRefAdvance, crate::commands::CommandFailure> {
        assert_eq!(
            expected.map(|head| head.oid.as_str()),
            Some("expired-generation")
        );
        assert_eq!(successor.state, "available");
        Ok(ClaimRefAdvance::Lost)
    }

    fn win_generation(
        _expected: Option<&ClaimRefHead>,
        successor: &autospec_core::claim::RunStateRecord,
    ) -> Result<ClaimRefAdvance, crate::commands::CommandFailure> {
        Ok(ClaimRefAdvance::Won(Box::new(ClaimRefHead {
            oid: "available-generation".to_string(),
            generation: "generation-2".to_string(),
            record: successor.clone(),
        })))
    }

    #[test]
    fn a_missing_claim_record_is_abandoned() {
        assert!(claim_is_abandoned(None, false), "nothing owns the issue");
    }

    #[test]
    fn a_released_claim_is_abandoned() {
        let live = utc_now_iso().expect("timestamp");
        for state in ["available", "released", "retryable", "failed"] {
            assert!(
                claim_is_abandoned(Some(&record(state, &live)), true),
                "{state} leaves the issue unowned even with a fresh timestamp"
            );
        }
    }

    #[test]
    fn an_expired_lease_is_abandoned() {
        assert!(claim_is_abandoned(
            Some(&record("claimed", "2026-07-29T00:49:45Z")),
            false
        ));
    }

    #[test]
    fn a_live_claim_is_left_alone() {
        let live = utc_now_iso().expect("timestamp");

        assert!(
            !claim_is_abandoned(Some(&record("claimed", &live)), true),
            "a worker inside its lease must keep the issue"
        );
    }

    #[test]
    fn a_merged_claim_is_left_alone() {
        assert!(
            !claim_is_abandoned(Some(&record("merged", "2026-07-29T00:49:45Z")), false),
            "merged work is finished, not abandoned"
        );
    }

    #[test]
    fn a_concurrent_lease_renewal_prevents_label_requeue() {
        let selected = ClaimRefHead {
            oid: "expired-generation".to_string(),
            generation: "generation-1".to_string(),
            record: record("claimed", "2026-07-29T00:49:45Z"),
        };
        let quarantined = quarantine_abandoned_claim_generation_with(
            "owner/repo",
            42,
            Some(selected),
            &mut lose_generation,
        )
        .expect("a lost compare-and-swap is not an error");

        assert!(
            quarantined.is_none(),
            "the renewing worker won, so the caller has no authority to mutate labels"
        );
    }

    /// The requeue path and the acquisition path must agree about ownership.
    ///
    /// They diverged once: requeue asked only the TTL clock while acquisition also
    /// asked whether the owner was alive. A dead owner's fresh lease then read as
    /// "owned" to requeue and "takeable" to acquisition, so the conductor was
    /// willing to take the issue but never saw it — the label kept it out of the
    /// candidate pool and it idled for hours beside work it could have done.
    #[test]
    fn an_owner_that_no_longer_holds_the_claim_is_always_abandoned() {
        let live = utc_now_iso().expect("timestamp");
        for (state, owner_holds, expected) in [
            ("claimed", true, false),
            ("claimed", false, true),
            ("merged", false, false),
            ("released", true, true),
        ] {
            assert_eq!(
                claim_is_abandoned(Some(&record(state, &live)), owner_holds),
                expected,
                "state={state} owner_holds={owner_holds} must match what acquisition decides"
            );
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn dead_current_owner_is_requeued_and_no_longer_blocks_acquisition() {
        let _guard = STARTUP_HEARTBEAT_ENV.lock().expect("heartbeat env");
        let (sandbox, _) = startup_heartbeat_fixture("current-owner-takeover");
        let root = sandbox.join("heartbeats");
        install_dead_heartbeat(
            &root,
            "rust-foreground-conductor-dead-1785286182924880200",
            "claim-a",
        );
        let previous = std::env::var_os("AUTOSPEC_HEARTBEAT_DIR");
        std::env::set_var("AUTOSPEC_HEARTBEAT_DIR", &root);
        let selected = ClaimRefHead {
            oid: "dead-generation".to_string(),
            generation: "generation-1".to_string(),
            record: ready_record(&utc_now_iso().expect("timestamp")),
        };
        let requeued = quarantine_abandoned_claim_generation_with(
            "owner/repo",
            42,
            Some(selected),
            &mut win_generation,
        )
        .expect("dead owner classification")
        .expect("dead owner requeued");
        assert_eq!(requeued.record.state, "available");
        assert_eq!(
            acquisition_blocking_owner(&requeued.record),
            None,
            "the requeued generation must pass acquire_record's ownership gate"
        );
        match previous {
            Some(value) => std::env::set_var("AUTOSPEC_HEARTBEAT_DIR", value),
            None => std::env::remove_var("AUTOSPEC_HEARTBEAT_DIR"),
        }
        std::fs::remove_dir_all(sandbox).expect("remove heartbeat fixture");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn missing_current_heartbeat_does_not_preserve_a_fresh_claim() {
        let _guard = STARTUP_HEARTBEAT_ENV.lock().expect("heartbeat env");
        let (sandbox, _) = startup_heartbeat_fixture("missing-current-owner");
        let root = sandbox.join("heartbeats");
        let parent = root.parent().expect("heartbeat parent");
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
            .expect("private heartbeat parent");
        let previous = std::env::var_os("AUTOSPEC_HEARTBEAT_DIR");
        std::env::set_var("AUTOSPEC_HEARTBEAT_DIR", &root);
        let owner_holds = owner_still_holds(
            "owner/repo",
            42,
            &ready_record(&utc_now_iso().expect("timestamp")),
        )
        .expect("missing heartbeat classification");
        assert!(
            !owner_holds,
            "missing owner evidence must not wedge the issue"
        );
        match previous {
            Some(value) => std::env::set_var("AUTOSPEC_HEARTBEAT_DIR", value),
            None => std::env::remove_var("AUTOSPEC_HEARTBEAT_DIR"),
        }
        std::fs::remove_dir_all(sandbox).expect("remove heartbeat fixture");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn live_ready_owner_keeps_its_fresh_claim() {
        let _guard = STARTUP_HEARTBEAT_ENV.lock().expect("heartbeat env");
        let (sandbox, _) = startup_heartbeat_fixture("live-ready-owner");
        let root = sandbox.join("heartbeats");
        let previous = std::env::var_os("AUTOSPEC_HEARTBEAT_DIR");
        std::env::set_var("AUTOSPEC_HEARTBEAT_DIR", &root);
        super::super::super::write_startup_heartbeat(
            "owner/repo",
            42,
            "rust-foreground-conductor-dead-1785286182924880200",
            "feat/worker",
            "claim-a",
            None,
        )
        .expect("publish live current heartbeat");
        assert!(
            owner_still_holds(
                "owner/repo",
                42,
                &ready_record(&utc_now_iso().expect("timestamp")),
            )
            .expect("live current owner classification"),
            "a live heartbeat-ready owner must not be displaced"
        );
        match previous {
            Some(value) => std::env::set_var("AUTOSPEC_HEARTBEAT_DIR", value),
            None => std::env::remove_var("AUTOSPEC_HEARTBEAT_DIR"),
        }
        std::fs::remove_dir_all(sandbox).expect("remove heartbeat fixture");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn retained_dead_predecessor_cannot_authorize_current_owner_takeover() {
        let _guard = STARTUP_HEARTBEAT_ENV.lock().expect("heartbeat env");
        let (sandbox, _) = startup_heartbeat_fixture("retained-prior-owner");
        let root = sandbox.join("heartbeats");
        install_dead_heartbeat(&root, "prior-worker", "prior-claim");
        let previous = std::env::var_os("AUTOSPEC_HEARTBEAT_DIR");
        std::env::set_var("AUTOSPEC_HEARTBEAT_DIR", &root);
        let mut prior = owner_record("2026-07-29T00:49:45Z", 1, "verification");
        prior.worker_id = "prior-worker".to_string();
        prior.claim_id = Some("prior-claim".to_string());
        assert!(
            super::super::super::quarantine_authoritative_stale_heartbeat(
                "owner/repo",
                42,
                &prior,
                None,
                &mut || Ok(()),
            )
            .expect("retain dead predecessor")
        );
        assert!(
            owner_still_holds(
                "owner/repo",
                42,
                &ready_record(&utc_now_iso().expect("timestamp")),
            )
            .expect("retained predecessor classification"),
            "dead predecessor evidence must fail closed for the current owner"
        );
        match previous {
            Some(value) => std::env::set_var("AUTOSPEC_HEARTBEAT_DIR", value),
            None => std::env::remove_var("AUTOSPEC_HEARTBEAT_DIR"),
        }
        std::fs::remove_dir_all(sandbox).expect("remove heartbeat fixture");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn terminal_records_do_not_inspect_heartbeat_storage() {
        let _guard = STARTUP_HEARTBEAT_ENV.lock().expect("heartbeat env");
        let previous = std::env::var_os("AUTOSPEC_HEARTBEAT_DIR");
        std::env::set_var("AUTOSPEC_HEARTBEAT_DIR", "/");
        for state in ["available", "released", "retryable", "failed", "merged"] {
            let selected = ClaimRefHead {
                oid: "terminal-generation".to_string(),
                generation: "generation-1".to_string(),
                record: record(state, "not-a-timestamp"),
            };
            let result = quarantine_abandoned_claim_generation_with(
                "owner/repo",
                42,
                Some(selected),
                &mut |_, _| Ok(ClaimRefAdvance::Lost),
            )
            .expect("terminal state must bypass heartbeat IO");
            assert!(result.is_none(), "state={state}");
        }
        match previous {
            Some(value) => std::env::set_var("AUTOSPEC_HEARTBEAT_DIR", value),
            None => std::env::remove_var("AUTOSPEC_HEARTBEAT_DIR"),
        }
    }
}
