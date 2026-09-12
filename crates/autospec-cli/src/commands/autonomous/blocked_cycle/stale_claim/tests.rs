use super::observe::{derive_facts, parse_pull_requests, ObservedPullRequest};
use super::*;
use std::cell::{Cell, RefCell};

fn facts(overrides: impl FnOnce(&mut StaleClaimFacts)) -> StaleClaimFacts {
    let mut facts = StaleClaimFacts {
        recorded_pr: Some(3001),
        pr_state_observed: true,
        pr_open: true,
        pr_merged: false,
        remote_branch_present: true,
        head_oid_matches: true,
        local_identity_intact: true,
        live_descendants: Some(0),
        progress_at: 1_000,
        observed_at: 1_000,
    };
    overrides(&mut facts);
    facts
}

#[test]
fn floor_defaults_to_a_day_and_is_configurable() {
    // SAFETY: single-threaded test process mutating its own environment.
    unsafe { std::env::remove_var(STALE_CLAIM_FLOOR_ENV) };
    assert_eq!(stale_claim_floor_secs(), DEFAULT_STALE_CLAIM_FLOOR_SECS);
    // SAFETY: single-threaded test process mutating its own environment.
    unsafe { std::env::set_var(STALE_CLAIM_FLOOR_ENV, "90") };
    assert_eq!(stale_claim_floor_secs(), 90);
    // A zero floor would quarantine every claim in flight; keep the default.
    // SAFETY: single-threaded test process mutating its own environment.
    unsafe { std::env::set_var(STALE_CLAIM_FLOOR_ENV, "0") };
    assert_eq!(stale_claim_floor_secs(), DEFAULT_STALE_CLAIM_FLOOR_SECS);
    // SAFETY: single-threaded test process mutating its own environment.
    unsafe { std::env::set_var(STALE_CLAIM_FLOOR_ENV, "soon") };
    assert_eq!(stale_claim_floor_secs(), DEFAULT_STALE_CLAIM_FLOOR_SECS);
    // SAFETY: single-threaded test process mutating its own environment.
    unsafe { std::env::remove_var(STALE_CLAIM_FLOOR_ENV) };
}

#[test]
fn floor_quarantines_before_pull_request_state_is_considered() {
    let floor = DEFAULT_STALE_CLAIM_FLOOR_SECS;
    // Closed pull request, branch still pushed: the floor decides first.
    let aged = facts(|facts| facts.observed_at = 1_000 + floor);
    assert_eq!(
        classify_stale_claim(&aged, floor),
        StaleClaimDisposition::Quarantine
    );
    // Unknown liveness past the floor is containment, not a wait.
    let unknown = facts(|facts| {
        facts.observed_at = 1_000 + floor;
        facts.live_descendants = None;
    });
    assert_eq!(
        classify_stale_claim(&unknown, floor),
        StaleClaimDisposition::Quarantine
    );
    // A live descendant always outranks the floor.
    let live = facts(|facts| {
        facts.observed_at = 1_000 + floor;
        facts.live_descendants = Some(2);
    });
    assert_eq!(
        classify_stale_claim(&live, floor),
        StaleClaimDisposition::Resume
    );
}

#[test]
fn inside_the_floor_the_remote_state_decides() {
    let floor = 3_600;
    let aged = facts(|facts| facts.observed_at = 1_000 + floor - 1);
    assert_eq!(
        classify_stale_claim(&aged, floor),
        StaleClaimDisposition::Resume
    );
    let closed = facts(|facts| {
        facts.observed_at = 1_000 + floor - 1;
        facts.pr_open = false;
    });
    assert_eq!(
        classify_stale_claim(&closed, floor),
        StaleClaimDisposition::Quarantine
    );
    let closed_and_pruned = facts(|facts| {
        facts.observed_at = 1_000 + floor - 1;
        facts.pr_open = false;
        facts.remote_branch_present = false;
    });
    assert_eq!(
        classify_stale_claim(&closed_and_pruned, floor),
        StaleClaimDisposition::TerminalRelease
    );
    let drifted = facts(|facts| {
        facts.observed_at = 1_000 + floor - 1;
        facts.head_oid_matches = false;
    });
    assert_eq!(
        classify_stale_claim(&drifted, floor),
        StaleClaimDisposition::Quarantine
    );
    let missing_worktree = facts(|facts| {
        facts.observed_at = 1_000 + floor - 1;
        facts.local_identity_intact = false;
    });
    assert_eq!(
        classify_stale_claim(&missing_worktree, floor),
        StaleClaimDisposition::Quarantine
    );
}

#[test]
fn merged_or_unreadable_claims_are_left_alone() {
    let floor = 3_600;
    let merged = facts(|facts| {
        facts.observed_at = 1_000 + floor * 4;
        facts.pr_merged = true;
        facts.pr_open = false;
    });
    assert_eq!(
        classify_stale_claim(&merged, floor),
        StaleClaimDisposition::Resume
    );
    let unreadable = facts(|facts| {
        facts.observed_at = 1_000 + floor - 1;
        facts.pr_state_observed = false;
        facts.pr_open = false;
        facts.remote_branch_present = false;
    });
    assert_eq!(
        classify_stale_claim(&unreadable, floor),
        StaleClaimDisposition::Resume
    );
}

#[test]
fn quarantine_maps_to_needs_human_and_never_closes_the_pull_request() {
    assert_eq!(
        StaleClaimDisposition::Quarantine.claim_disposition(),
        Some(BridgeClaimDisposition::NeedsHuman)
    );
    assert_eq!(
        StaleClaimDisposition::TerminalRelease.claim_disposition(),
        Some(BridgeClaimDisposition::Retryable)
    );
    assert_eq!(StaleClaimDisposition::Resume.claim_disposition(), None);
    assert_eq!(
        StaleClaimDisposition::Quarantine.reason(),
        QUARANTINE_REASON
    );
}

#[test]
fn derive_facts_reads_the_recorded_pull_request_and_head() {
    let rows = vec![
        ObservedPullRequest {
            number: 3001,
            state: "OPEN".to_string(),
            head_oid: "AA11BB".to_string(),
        },
        ObservedPullRequest {
            number: 2900,
            state: "CLOSED".to_string(),
            head_oid: "cc22".to_string(),
        },
    ];
    let matched = derive_facts(
        Some(3001),
        Some("aa11bb"),
        Some(0),
        true,
        1_000,
        2_000,
        Ok(rows.clone()),
    );
    assert!(matched.pr_state_observed);
    assert!(matched.pr_open);
    assert!(matched.head_oid_matches);
    assert_eq!(matched.age_secs(), 1_000);
    let moved = derive_facts(
        Some(3001),
        Some("ffff00"),
        Some(0),
        true,
        1_000,
        2_000,
        Ok(rows),
    );
    assert!(!moved.head_oid_matches);
    assert!(moved.pr_state_observed);
}

#[test]
fn derive_facts_marks_unreadable_remote_state() {
    let facts = derive_facts(
        Some(3001),
        Some("aa11bb"),
        None,
        true,
        1_000,
        2_000,
        Err("gh unavailable".to_string()),
    );
    assert!(!facts.pr_state_observed);
    assert!(!facts.pr_open);
    assert!(!facts.remote_branch_present);
    assert!(facts.head_oid_matches);
    assert_eq!(facts.live_descendants, None);
}

#[test]
fn parse_pull_requests_reads_state_and_head() {
    let rows = parse_pull_requests(
        r#"[{"number":7,"state":"open","headRefOid":"DEADBEEF"},{"number":8,"state":"MERGED","headRefOid":"cafe"}]"#,
    )
    .expect("json list");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].state, "OPEN");
    assert_eq!(rows[0].head_oid, "deadbeef");
    assert_eq!(rows[1].state, "MERGED");
    assert!(parse_pull_requests("{}").is_err());
}

#[test]
fn dispose_persists_before_releasing_and_clears_last() {
    let log: RefCell<Vec<&'static str>> = RefCell::new(Vec::new());
    let mut persist = |_: StaleClaimDisposition| {
        log.borrow_mut().push("persist");
        Ok(())
    };
    let mut release = |_: StaleClaimDisposition| {
        log.borrow_mut().push("release");
        Ok(StaleClaimRelease::Transitioned)
    };
    let mut clear = || {
        log.borrow_mut().push("clear");
        Ok(())
    };
    let mut effects = StaleClaimEffects {
        persist_terminal: &mut persist,
        release_claim: &mut release,
        clear_receipt: &mut clear,
    };
    let outcome = dispose_stale_claim(StaleClaimDisposition::Quarantine, &mut effects)
        .expect("disposition succeeds");
    assert_eq!(
        outcome,
        Some(StaleClaimOutcome {
            disposition: StaleClaimDisposition::Quarantine,
            release: StaleClaimRelease::Transitioned,
        })
    );
    assert_eq!(*log.borrow(), vec!["persist", "release", "clear"]);
}

#[test]
fn resume_applies_no_effects_and_lost_ownership_still_clears() {
    let calls = Cell::new(0usize);
    let mut persist = |_: StaleClaimDisposition| {
        calls.set(calls.get() + 1);
        Ok(())
    };
    let mut release = |_: StaleClaimDisposition| Ok(StaleClaimRelease::OwnershipLost);
    let mut clear = || {
        calls.set(calls.get() + 1);
        Ok(())
    };
    let mut effects = StaleClaimEffects {
        persist_terminal: &mut persist,
        release_claim: &mut release,
        clear_receipt: &mut clear,
    };
    assert_eq!(
        dispose_stale_claim(StaleClaimDisposition::Resume, &mut effects).expect("resume"),
        None
    );
    assert_eq!(calls.get(), 0);
    let outcome =
        dispose_stale_claim(StaleClaimDisposition::TerminalRelease, &mut effects).expect("release");
    assert_eq!(
        outcome.map(|found| found.release),
        Some(StaleClaimRelease::OwnershipLost)
    );
    // Ownership loss stops the remote mutation only: the local record is
    // still stamped and our own acquisition receipt is still cleared.
    assert_eq!(calls.get(), 2);
}

#[test]
fn failed_stamp_aborts_before_any_claim_mutation() {
    let released = Cell::new(0usize);
    let mut persist = |_: StaleClaimDisposition| Err("state dir is read-only".to_string());
    let mut release = |_: StaleClaimDisposition| {
        released.set(released.get() + 1);
        Ok(StaleClaimRelease::Transitioned)
    };
    let mut clear = || Ok(());
    let mut effects = StaleClaimEffects {
        persist_terminal: &mut persist,
        release_claim: &mut release,
        clear_receipt: &mut clear,
    };
    let error = dispose_stale_claim(StaleClaimDisposition::Quarantine, &mut effects)
        .expect_err("stamp failure propagates");
    assert_eq!(error, "state dir is read-only");
    assert_eq!(released.get(), 0);
}
