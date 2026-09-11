//! A lock's lifecycle is acquire and release (issue #4220).
//!
//! The regression tests run in the configuration the incident required:
//! one conversion pass (pid 815956) claiming per-issue lock files into a
//! claim directory, with the current claim held in a single variable
//! (`_held_lock`) overwritten each iteration and released only by the
//! exit trap. After launch the claims were verified —
//! `claim 3813 held by pid 815956 (alive: yes)`, `claim 4183 held by pid
//! 815956 (alive: yes)` — and later, with both issues long finished (3813
//! held on conflicts, 4183 converted and merged), the claim directory
//! still held `3813 4131 4183`, only 4131 actually being worked on.

use std::collections::BTreeMap;

use autospec_core::claim_lifecycle::{
    after_crash, blocked_retries, blocked_retries_line, claim_file_name, lifecycle_coverage,
    parse_claim_file, settle_claims, stale_verdict, AfterCrash, CheckPoint, ClaimFileDesign,
    ClaimIteration, ClaimLedger, IssueState, LifecycleCoverage, LoopClaimShape, ReleaseSite,
    StaleVerdict,
};

/// The incident pass's pid.
const PASS_PID: u32 = 815956;
/// The issue held on conflicts (the retry path the stale claim blocked).
const HELD_ISSUE: u64 = 3813;
/// The issue converted and merged.
const MERGED_ISSUE: u64 = 4183;
/// The issue actually being worked on at the time of observation.
const LIVE_ISSUE: u64 = 4131;
/// The number of `continue` statements in the incident loop's body.
const CONTINUES: usize = 6;

/// The incident loop: a single variable holding the current claim,
/// released only by the exit trap, six `continue` statements in the body.
fn incident_shape() -> LoopClaimShape {
    LoopClaimShape {
        release_site: ReleaseSite::FunctionExit,
        continues: CONTINUES,
    }
}

/// The completed iterations of the incident pass at the time of
/// observation: 3813 (held on conflicts, the loop continued on) and 4183
/// (converted, the loop continued on). 4131 is in progress and not in
/// the list — it is the live claim, not a leak.
fn incident_iterations() -> Vec<ClaimIteration> {
    vec![
        ClaimIteration {
            issue: HELD_ISSUE,
            exited_via_continue: true,
        },
        ClaimIteration {
            issue: MERGED_ISSUE,
            exited_via_continue: true,
        },
    ]
}

// --- Invariant 1: a lock's lifecycle is acquire and release -------------

#[test]
fn the_incident_verification_covered_only_the_acquire_side() {
    // After launching, the operator confirmed two claims, correct owner,
    // live process — a check after start. Release was inferred, not
    // checked, and the inference was the defect.
    let coverage = lifecycle_coverage(&[CheckPoint::AfterStart]);
    assert_eq!(coverage, LifecycleCoverage::AcquireInferredRelease);
    assert!(!coverage.is_complete());
    assert_eq!(
        coverage.warn_line(),
        Some(
            "WARN: lock lifecycle: acquire verified after start; release inferred, never checked"
                .to_string()
        )
    );
}

#[test]
fn a_run_end_check_counts_for_neither_side() {
    // The incident's cleanup line ran at the summary: by then the in-run
    // retry path the claim protects was already blocked for the whole
    // run, and an empty directory at run end proves nothing about
    // whether the claim was ever taken.
    assert_eq!(
        lifecycle_coverage(&[CheckPoint::AfterRun]),
        LifecycleCoverage::Unverified
    );
    assert_eq!(
        lifecycle_coverage(&[CheckPoint::AfterStart, CheckPoint::AfterRun]),
        LifecycleCoverage::AcquireInferredRelease
    );
}

#[test]
fn checking_release_after_completion_completes_the_lifecycle() {
    // The assertion that matters is that the claim is gone once the work
    // is done — check the claim directory after an item completes, not
    // only after it starts.
    let coverage = lifecycle_coverage(&[CheckPoint::AfterStart, CheckPoint::AfterCompletion]);
    assert_eq!(coverage, LifecycleCoverage::Complete);
    assert!(coverage.is_complete());
    assert_eq!(coverage.warn_line(), None);
    assert_eq!(
        coverage.line(),
        "lock lifecycle: acquire verified after start, release verified after completion"
    );
}

#[test]
fn release_only_and_nothing_at_all_are_their_own_findings() {
    // The lock goes away but was never confirmed to be there: the race
    // it exists to prevent is unproven.
    assert_eq!(
        lifecycle_coverage(&[CheckPoint::AfterCompletion]),
        LifecycleCoverage::ReleaseWithoutAcquire
    );
    assert_eq!(lifecycle_coverage(&[]), LifecycleCoverage::Unverified);
}

// --- Invariant 2: release in the loop, never only at function exit ------

#[test]
fn the_incident_shape_leaks_every_claim_but_the_last() {
    // The single variable is overwritten each iteration, so the exit
    // trap releases only the last claim; the rest survive until the
    // summary-time cleanup line — too late for the in-run retry path.
    let ledger = settle_claims(&incident_iterations(), &incident_shape());
    assert_eq!(
        ledger,
        ClaimLedger {
            released: vec![MERGED_ISSUE],
            leaked: vec![HELD_ISSUE],
        }
    );
    assert_eq!(ledger.line(), "claims: released=1 leaked=1 (#3813)");
    assert_eq!(
        ledger.warn_line(),
        Some(
            "WARN: claims: released=1 leaked=1 (#3813) — a leaked claim on a held issue blocks \
             its retry for the remainder of the run"
                .to_string()
        )
    );
}

#[test]
fn the_incident_observation_is_the_leak_made_visible() {
    // At the time of observation the claim directory held 3813 4131
    // 4183: one live claim and two claims whose work is done. Of the two
    // dead ones, the settled ledger names the one the release discipline
    // never released (3813) and the one only the exit trap eventually
    // released (4183). The live claim is not a leak: it is the work in
    // progress.
    let ledger = settle_claims(&incident_iterations(), &incident_shape());
    let on_disk = [HELD_ISSUE, LIVE_ISSUE, MERGED_ISSUE];
    let dead_claims = on_disk
        .into_iter()
        .filter(|issue| *issue != LIVE_ISSUE)
        .collect::<Vec<_>>();
    let unexplained = dead_claims
        .into_iter()
        .filter(|issue| !ledger.released.contains(issue) && !ledger.leaked.contains(issue))
        .collect::<Vec<_>>();
    assert!(
        unexplained.is_empty(),
        "claim(s) on disk that the ledger neither released nor leaked: {unexplained:?}"
    );
}

#[test]
fn releasing_at_the_top_of_the_loop_cannot_leak() {
    // Every path through the loop — six `continue` statements included —
    // reaches the top of the next iteration: one edit site, zero leaks.
    let shape = LoopClaimShape {
        release_site: ReleaseSite::TopOfLoop,
        continues: CONTINUES,
    };
    let ledger = settle_claims(&incident_iterations(), &shape);
    assert_eq!(
        ledger,
        ClaimLedger {
            released: vec![HELD_ISSUE, MERGED_ISSUE],
            leaked: vec![],
        }
    );
    assert_eq!(ledger.line(), "claims: released=2 leaked=0");
    assert_eq!(ledger.warn_line(), None);
}

#[test]
fn releasing_at_the_end_of_the_body_leaks_the_continues() {
    // Each `continue` above the release jumps over it: those iterations'
    // claims leak, the fallen-through ones release.
    let shape = LoopClaimShape {
        release_site: ReleaseSite::EndOfBody,
        continues: CONTINUES,
    };
    let iterations = vec![
        ClaimIteration {
            issue: 1,
            exited_via_continue: true,
        },
        ClaimIteration {
            issue: 2,
            exited_via_continue: false,
        },
        ClaimIteration {
            issue: 3,
            exited_via_continue: true,
        },
    ];
    let ledger = settle_claims(&iterations, &shape);
    assert_eq!(
        ledger,
        ClaimLedger {
            released: vec![2, 3],
            leaked: vec![1],
        }
    );
}

#[test]
fn a_single_iteration_releases_its_only_claim_under_every_site() {
    // N locks, one released lock, N−1 leaks: for N = 1 there is nothing
    // to leak, and the exit trap releases the one claim.
    let iterations = vec![ClaimIteration {
        issue: 7,
        exited_via_continue: true,
    }];
    for site in [
        ReleaseSite::TopOfLoop,
        ReleaseSite::EndOfBody,
        ReleaseSite::FunctionExit,
    ] {
        let ledger = settle_claims(
            &iterations,
            &LoopClaimShape {
                release_site: site,
                continues: CONTINUES,
            },
        );
        assert_eq!(
            ledger,
            ClaimLedger {
                released: vec![7],
                leaked: vec![],
            },
            "site {site:?}"
        );
    }
}

#[test]
fn the_edit_site_count_is_one_for_the_top_and_n_plus_one_for_the_rest() {
    // "Releasing at the top of the next iteration is the form with one
    // edit site instead of six": the incident's six `continue`
    // statements owe six extra release points in any other shape.
    assert_eq!(incident_shape().release_edit_sites(), CONTINUES + 1);
    assert_eq!(
        LoopClaimShape {
            release_site: ReleaseSite::TopOfLoop,
            continues: CONTINUES,
        }
        .release_edit_sites(),
        1
    );
    assert_eq!(
        LoopClaimShape {
            release_site: ReleaseSite::EndOfBody,
            continues: CONTINUES,
        }
        .release_edit_sites(),
        CONTINUES + 1
    );
}

#[test]
fn only_the_held_leak_blocks_the_retry_path() {
    // The consequence is narrow but real: a completed issue is harmless
    // — it has a PR, so the existing check catches it anyway. The held
    // issue is exactly the case a retry should pick up, and its leaked
    // claim blocks that for the remainder of the run.
    let ledger = settle_claims(&incident_iterations(), &incident_shape());
    let mut states = BTreeMap::new();
    states.insert(HELD_ISSUE, IssueState::Held);
    states.insert(MERGED_ISSUE, IssueState::AlreadyHasPr);
    assert_eq!(blocked_retries(&ledger.leaked, &states), vec![HELD_ISSUE]);
    assert_eq!(
        blocked_retries_line(&ledger.leaked, &states),
        "leaked claims block the retry path for: #3813"
    );
}

#[test]
fn a_leak_on_a_merged_issue_is_masked_by_the_existing_check() {
    // 4183 alone on disk after its merge: the existing PR check catches
    // it, so the retry path is unblocked — which is why the bug stayed
    // invisible in the common path.
    let ledger = ClaimLedger {
        released: vec![],
        leaked: vec![MERGED_ISSUE],
    };
    let mut states = BTreeMap::new();
    states.insert(MERGED_ISSUE, IssueState::AlreadyHasPr);
    assert_eq!(blocked_retries(&ledger.leaked, &states), Vec::<u64>::new());
    assert_eq!(
        blocked_retries_line(&ledger.leaked, &states),
        "leaked claims: none block the retry path"
    );
}

// --- Invariant 4: prefer the shape that cannot leak ---------------------

#[test]
fn the_incident_claim_cannot_name_its_owner() {
    // The incident claim file's name is the issue number alone: after a
    // crash the leftover claim is indistinguishable from a live one, and
    // nothing on read tells the reader to look past it.
    let design = ClaimFileDesign {
        pid_in_name: false,
        liveness_checked_on_read: false,
    };
    assert_eq!(stale_verdict(&design), StaleVerdict::Ownerless);
    assert_eq!(after_crash(&design, false), AfterCrash::StaleUndetected);
}

#[test]
fn a_pid_the_reader_never_checks_is_never_seen() {
    // The stale state exists — the pid is right there in the name — but
    // a reader that never checks liveness treats the dead owner's claim
    // as live. One check is all the remediation is; the report says so.
    let design = ClaimFileDesign {
        pid_in_name: true,
        liveness_checked_on_read: false,
    };
    assert_eq!(stale_verdict(&design), StaleVerdict::PidNeverChecked);
    assert_eq!(after_crash(&design, false), AfterCrash::StaleUndetected);
}

#[test]
fn the_pid_encoded_and_checked_design_degrades_safely_on_crash() {
    // The shape that cannot leak: the claim file's name encodes the
    // owning PID and the reader checks liveness on read, so a crashed
    // owner's leftover claim is recognized as stale and the retry
    // proceeds — no release path at all is needed.
    let design = ClaimFileDesign {
        pid_in_name: true,
        liveness_checked_on_read: true,
    };
    assert_eq!(stale_verdict(&design), StaleVerdict::Detectable);
    assert_eq!(
        stale_verdict(&design).line(),
        "stale-claim state: detectable on read (owner pid in name, liveness checked)"
    );
    assert_eq!(after_crash(&design, false), AfterCrash::StaleDetected);
}

#[test]
fn a_live_owner_is_live_either_way() {
    // There is no stale state to detect while the owner lives: the
    // liveness check is what makes the claim a lock, not a one-way
    // marker, and a live owner's claim stands under every design.
    for design in [
        ClaimFileDesign {
            pid_in_name: true,
            liveness_checked_on_read: true,
        },
        ClaimFileDesign {
            pid_in_name: false,
            liveness_checked_on_read: false,
        },
    ] {
        assert_eq!(after_crash(&design, true), AfterCrash::Live);
    }
}

#[test]
fn the_claim_file_name_encodes_the_owner_pid() {
    let name = claim_file_name(HELD_ISSUE, PASS_PID);
    assert_eq!(name, "claim-3813-815956");
    assert_eq!(parse_claim_file(&name), Some((HELD_ISSUE, PASS_PID)));
}

#[test]
fn a_name_without_an_encoded_pid_does_not_parse() {
    // A claim that cannot name its owner cannot be checked for one: the
    // parse refusing is the same refusal as the Ownerless verdict.
    assert_eq!(parse_claim_file("claim-3813"), None);
    assert_eq!(parse_claim_file("claim-3813-"), None);
    assert_eq!(parse_claim_file("claim-abc-def"), None);
    assert_eq!(parse_claim_file(""), None);
}
