//! Head-bound review quorum: stale verdicts, authors, duplicate identities,
//! replayed receipts, force-push and verdict-after-head-change races (T06).

use autospec_core::autonomous::review_quorum::{
    evaluate_quorum, QuorumPolicy, QuorumStatus, QuorumVote, RejectReason, VoteVerdict,
};

const TARGET: &str = "owner/repo#12";

fn sha(n: u8) -> String {
    format!("{n:02x}").repeat(20)
}

fn vote(reviewer: &str, verdict: VoteVerdict, commit: &str, digest: &str) -> QuorumVote {
    QuorumVote {
        reviewer: reviewer.into(),
        target: TARGET.into(),
        provider: Some("provider-b".into()),
        verdict,
        commit: commit.into(),
        receipt_digest: Some(digest.into()),
    }
}

fn policy(required: usize) -> QuorumPolicy {
    QuorumPolicy {
        required,
        pr_author: "Author".into(),
        excluded_identities: vec!["worker-1".into()],
        implementer_provider: Some("provider-a".into()),
        require_provider_diversity: false,
    }
}

fn reasons(
    o: &autospec_core::autonomous::review_quorum::QuorumOutcome,
) -> Vec<(String, RejectReason)> {
    o.rejected
        .iter()
        .map(|r| (r.reviewer.clone(), r.reason))
        .collect()
}

#[test]
fn two_distinct_reviewers_at_current_head_satisfy() {
    let head = sha(1);
    let votes = [
        vote("r1", VoteVerdict::Lgtm, &head, "d1"),
        vote("r2", VoteVerdict::Lgtm, &head, "d2"),
    ];
    let out = evaluate_quorum(TARGET, &head, &votes, &policy(2)).unwrap();
    assert_eq!(out.status, QuorumStatus::Satisfied);
    assert_eq!(out.counted, ["r1", "r2"]);
    assert!(!out.rereview_required);
}

#[test]
fn verdict_for_an_older_head_is_never_reused() {
    let (old, new) = (sha(1), sha(2));
    let votes = [
        vote("r1", VoteVerdict::Lgtm, &old, "d1"),
        vote("r2", VoteVerdict::Lgtm, &old, "d2"),
    ];
    let out = evaluate_quorum(TARGET, &new, &votes, &policy(2)).unwrap();
    assert_eq!(out.status, QuorumStatus::Insufficient);
    assert!(out.counted.is_empty());
    assert_eq!(out.stale_reviewers, ["r1", "r2"]);
    assert!(out.rereview_required);
    assert_eq!(out.rereview_needed, 2);
}

#[test]
fn a_prefix_or_case_variant_of_the_head_is_not_the_head() {
    let head = sha(1);
    let votes = [vote("r1", VoteVerdict::Lgtm, &head[..12], "d1")];
    let out = evaluate_quorum(TARGET, &head, &votes, &policy(1)).unwrap();
    assert_eq!(
        reasons(&out),
        [("r1".to_string(), RejectReason::InvalidCommit)]
    );
    let upper = [vote("r1", VoteVerdict::Lgtm, &head.to_uppercase(), "d1")];
    assert_eq!(
        evaluate_quorum(TARGET, &head, &upper, &policy(1))
            .unwrap()
            .status,
        QuorumStatus::Satisfied
    );
}

#[test]
fn the_author_and_excluded_workers_cannot_make_quorum() {
    let head = sha(1);
    let votes = [
        vote("AUTHOR ", VoteVerdict::Lgtm, &head, "d1"),
        vote("worker-1", VoteVerdict::Lgtm, &head, "d2"),
        vote("r3", VoteVerdict::Lgtm, &head, "d3"),
    ];
    let out = evaluate_quorum(TARGET, &head, &votes, &policy(2)).unwrap();
    assert_eq!(out.status, QuorumStatus::Insufficient);
    assert_eq!(out.counted, ["r3"]);
    assert!(reasons(&out).contains(&("author".to_string(), RejectReason::Author)));
    assert!(reasons(&out).contains(&("worker-1".to_string(), RejectReason::ExcludedIdentity)));
}

#[test]
fn one_identity_counted_once_even_with_case_and_whitespace_variants() {
    let head = sha(1);
    let votes = [
        vote("r1", VoteVerdict::Lgtm, &head, "d1"),
        vote(" R1", VoteVerdict::Lgtm, &head, "d2"),
        vote("r1", VoteVerdict::Lgtm, &head, "d3"),
    ];
    let out = evaluate_quorum(TARGET, &head, &votes, &policy(2)).unwrap();
    assert_eq!(out.counted, ["r1"]);
    assert_eq!(out.status, QuorumStatus::Insufficient);
}

#[test]
fn a_replayed_receipt_digest_voids_both_identities() {
    let head = sha(1);
    let votes = [
        vote("r1", VoteVerdict::Lgtm, &head, "same"),
        vote("r2", VoteVerdict::Lgtm, &head, "same"),
    ];
    let out = evaluate_quorum(TARGET, &head, &votes, &policy(1)).unwrap();
    assert_eq!(out.status, QuorumStatus::Insufficient);
    assert!(out
        .rejected
        .iter()
        .all(|r| r.reason == RejectReason::ReplayedReceipt));
}

#[test]
fn lgtm_without_a_receipt_digest_does_not_count() {
    let head = sha(1);
    let mut v = vote("r1", VoteVerdict::Lgtm, &head, "x");
    v.receipt_digest = None;
    let out = evaluate_quorum(TARGET, &head, &[v], &policy(1)).unwrap();
    assert_eq!(
        reasons(&out),
        [("r1".to_string(), RejectReason::MissingReceiptDigest)]
    );
}

#[test]
fn a_current_head_block_always_blocks_even_from_the_author() {
    let head = sha(1);
    let votes = [
        vote("r1", VoteVerdict::Lgtm, &head, "d1"),
        vote("r2", VoteVerdict::Lgtm, &head, "d2"),
        vote("author", VoteVerdict::Blocked, &head, "d3"),
    ];
    let out = evaluate_quorum(TARGET, &head, &votes, &policy(2)).unwrap();
    assert_eq!(out.status, QuorumStatus::Blocked);
    assert_eq!(out.blocking, ["author"]);
    assert!(!out.rereview_required);
}

#[test]
fn a_reviewer_who_both_approves_and_blocks_is_blocking_not_counted() {
    let head = sha(1);
    let votes = [
        vote("r1", VoteVerdict::Lgtm, &head, "d1"),
        vote("r1", VoteVerdict::Blocked, &head, "d2"),
    ];
    let out = evaluate_quorum(TARGET, &head, &votes, &policy(1)).unwrap();
    assert_eq!(out.status, QuorumStatus::Blocked);
    assert!(out.counted.is_empty());
}

#[test]
fn a_block_for_an_old_head_does_not_block_the_repaired_head() {
    let (old, new) = (sha(1), sha(2));
    let votes = [
        vote("r1", VoteVerdict::Blocked, &old, "d1"),
        vote("r2", VoteVerdict::Lgtm, &new, "d2"),
    ];
    let out = evaluate_quorum(TARGET, &new, &votes, &policy(1)).unwrap();
    assert_eq!(out.status, QuorumStatus::Satisfied);
    assert_eq!(out.stale_reviewers, ["r1"]);
}

#[test]
fn verdict_arriving_after_a_head_change_is_judged_against_the_new_head() {
    // Race: the verdict was issued for sha(1), the PR moved to sha(2) before it landed.
    let (issued, current) = (sha(1), sha(2));
    let out = evaluate_quorum(
        TARGET,
        &current,
        &[vote("r1", VoteVerdict::Lgtm, &issued, "d1")],
        &policy(1),
    )
    .unwrap();
    assert_eq!(out.status, QuorumStatus::Insufficient);
    assert!(out.rereview_required);
}

#[test]
fn force_push_back_to_a_reviewed_sha_restores_only_that_exact_commit() {
    let (a, b) = (sha(1), sha(2));
    let votes = [vote("r1", VoteVerdict::Lgtm, &a, "d1")];
    assert_eq!(
        evaluate_quorum(TARGET, &b, &votes, &policy(1))
            .unwrap()
            .status,
        QuorumStatus::Insufficient
    );
    assert_eq!(
        evaluate_quorum(TARGET, &a, &votes, &policy(1))
            .unwrap()
            .status,
        QuorumStatus::Satisfied
    );
}

#[test]
fn provider_diversity_rejects_same_or_unknown_provider() {
    let head = sha(1);
    let mut p = policy(1);
    p.require_provider_diversity = true;
    let mut same = vote("r1", VoteVerdict::Lgtm, &head, "d1");
    same.provider = Some(" Provider-A".into());
    let mut unknown = vote("r2", VoteVerdict::Lgtm, &head, "d2");
    unknown.provider = None;
    let out = evaluate_quorum(TARGET, &head, &[same, unknown], &p).unwrap();
    assert_eq!(out.status, QuorumStatus::Insufficient);
    assert!(out
        .rejected
        .iter()
        .all(|r| r.reason == RejectReason::ProviderNotDiverse));
    let ok = vote("r3", VoteVerdict::Lgtm, &head, "d3");
    assert_eq!(
        evaluate_quorum(TARGET, &head, &[ok], &p).unwrap().status,
        QuorumStatus::Satisfied
    );
}

#[test]
fn invalid_head_or_zero_quorum_is_an_error_not_a_pass() {
    assert!(evaluate_quorum(TARGET, "abc", &[], &policy(1)).is_err());
    assert!(evaluate_quorum(TARGET, &sha(1), &[], &policy(0)).is_err());
    assert!(evaluate_quorum(
        TARGET,
        &sha(1),
        &[],
        &QuorumPolicy {
            pr_author: "  ".into(),
            ..policy(1)
        }
    )
    .is_err());
}

#[test]
fn evaluation_is_order_independent() {
    let head = sha(1);
    let mut votes = vec![
        vote("r1", VoteVerdict::Lgtm, &head, "d1"),
        vote("r2", VoteVerdict::Lgtm, &sha(2), "d2"),
        vote("r3", VoteVerdict::Lgtm, &head, "d3"),
    ];
    let a = evaluate_quorum(TARGET, &head, &votes, &policy(2)).unwrap();
    votes.reverse();
    assert_eq!(
        a,
        evaluate_quorum(TARGET, &head, &votes, &policy(2)).unwrap()
    );
}

#[test]
fn a_receipt_for_the_same_sha_on_another_pr_is_not_this_prs_review() {
    let head = sha(1);
    let mut other = vote("r1", VoteVerdict::Lgtm, &head, "d1");
    other.target = "owner/repo#13".into();
    let mut other_block = vote("r2", VoteVerdict::Blocked, &head, "d2");
    other_block.target = "other/repo#12".into();
    let out = evaluate_quorum(TARGET, &head, &[other, other_block], &policy(1)).unwrap();
    assert_eq!(out.status, QuorumStatus::Insufficient);
    assert!(out
        .rejected
        .iter()
        .all(|r| r.reason == RejectReason::WrongTarget));
    assert!(evaluate_quorum(" ", &head, &[], &policy(1)).is_err());
}
