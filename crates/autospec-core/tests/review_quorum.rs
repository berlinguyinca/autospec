//! Head-bound review quorum. The shared vector file is loaded by this suite AND by
//! herdr-engineering's tests (byte-identical copy): one policy, two implementations.

use autospec_core::autonomous::review_quorum::{
    evaluate_quorum, receipt_binding, QuorumPolicy, QuorumStatus, QuorumVote, RejectReason,
};
use serde::Deserialize;

#[derive(Deserialize)]
struct Vectors {
    cases: Vec<Case>,
}
#[derive(Deserialize)]
struct Case {
    name: String,
    applies: Vec<String>,
    target: String,
    head: String,
    policy: QuorumPolicy,
    votes: Vec<QuorumVote>,
    expect: Expect,
}
#[derive(Deserialize)]
struct Expect {
    status: String,
    counted: Vec<String>,
    blocking: Vec<String>,
}

#[test]
fn shared_conformance_vectors() {
    let raw = include_str!("fixtures/review-quorum-vectors.json");
    let vectors: Vectors = serde_json::from_str(raw).expect("vectors parse");
    assert!(vectors.cases.len() >= 20);
    let mut ran = 0;
    for case in vectors
        .cases
        .iter()
        .filter(|c| c.applies.iter().any(|a| a == "rust"))
    {
        let got = evaluate_quorum(&case.target, &case.head, &case.votes, &case.policy);
        let (status, counted, blocking) = match got {
            Ok(o) => (
                match o.status {
                    QuorumStatus::Satisfied => "satisfied",
                    QuorumStatus::Insufficient => "insufficient",
                    QuorumStatus::Blocked => "blocked",
                }
                .to_string(),
                o.counted,
                o.blocking,
            ),
            Err(e) if e.contains("pr_author") => ("unknown_author".into(), vec![], vec![]),
            Err(e) if e.contains("ASCII") => ("invalid_target".into(), vec![], vec![]),
            Err(e) => panic!("{}: unexpected error {e}", case.name),
        };
        assert_eq!(status, case.expect.status, "{}", case.name);
        assert_eq!(counted, case.expect.counted, "{}", case.name);
        assert_eq!(blocking, case.expect.blocking, "{}", case.name);
        ran += 1;
    }
    assert!(ran >= 20, "only {ran} vectors applied to rust");
}

fn sha(n: u8) -> String {
    format!("{n:02x}").repeat(20)
}

fn policy() -> QuorumPolicy {
    serde_json::from_str(r#"{"required":1,"pr_author":"author"}"#).unwrap()
}

fn lgtm(reviewer: &str, commit: &str, target: &str) -> QuorumVote {
    let digest = "c".repeat(64);
    QuorumVote {
        reviewer: reviewer.into(),
        target: target.into(),
        provider: None,
        verdict: "lgtm".into(),
        commit: commit.into(),
        receipt_binding: Some(receipt_binding(target, commit, reviewer, &digest)),
        receipt_digest: Some(digest),
    }
}

#[test]
fn rereview_is_requested_only_when_the_current_head_lacks_approvals() {
    let (old, new) = (sha(1), sha(2));
    let votes = [lgtm("r1", &old, "o/r#1")];
    let out = evaluate_quorum("o/r#1", &new, &votes, &policy()).unwrap();
    assert_eq!(out.stale_reviewers, ["r1"]);
    assert!(out.rereview_required && out.rereview_needed == 1);
    let out = evaluate_quorum("o/r#1", &old, &votes, &policy()).unwrap();
    assert!(!out.rereview_required);
}

#[test]
fn a_blocked_vote_with_a_hostile_reviewer_id_or_commit_is_never_dropped() {
    let head = sha(1);
    for (reviewer, commit) in [
        ("a b", head.as_str()),
        ("", head.as_str()),
        ("x", "ZZZ"),
        ("x", &head[..9]),
    ] {
        let mut block = lgtm("r2", &head, "o/r#1");
        block.verdict = "Blocked".into();
        block.reviewer = reviewer.into();
        block.commit = commit.into();
        let votes = [lgtm("r1", &head, "o/r#1"), block];
        let out = evaluate_quorum("O/R#1", &head, &votes, &policy()).unwrap();
        assert_eq!(out.status, QuorumStatus::Blocked, "{reviewer:?} {commit:?}");
    }
}

#[test]
fn a_binding_cannot_be_copied_to_another_commit() {
    let (a, b) = (sha(1), sha(2));
    let mut v = lgtm("r1", &a, "o/r#1");
    v.commit = b.clone();
    let out = evaluate_quorum("o/r#1", &b, &[v], &policy()).unwrap();
    assert!(out
        .rejected
        .iter()
        .any(|r| r.reason == RejectReason::BadReceiptBinding));
}

#[test]
fn invalid_header_is_an_error_not_a_pass() {
    assert!(evaluate_quorum("t", "abc", &[], &policy()).is_err());
    assert!(evaluate_quorum(" ", &sha(1), &[], &policy()).is_err());
    let mut p = policy();
    p.required = 0;
    assert!(evaluate_quorum("t", &sha(1), &[], &p).is_err());
    p = policy();
    p.pr_author = "  ".into();
    assert!(evaluate_quorum("t", &sha(1), &[], &p).is_err());
    p = policy();
    p.reviewer_groups = vec![vec!["a".into(), "b".into()], vec!["b".into()]];
    assert!(evaluate_quorum("t", &sha(1), &[], &p).is_err());
}

#[test]
fn evaluation_is_order_independent() {
    let head = sha(1);
    let mut votes = vec![
        lgtm("r1", &head, "t"),
        lgtm("r2", &sha(2), "t"),
        lgtm("r3", &head, "t"),
    ];
    let a = evaluate_quorum("t", &head, &votes, &policy()).unwrap();
    votes.reverse();
    assert_eq!(a, evaluate_quorum("t", &head, &votes, &policy()).unwrap());
}

#[test]
fn hmac_matches_the_rfc_4231_vectors() {
    use autospec_core::autonomous::review_quorum::hmac_sha256_hex;
    assert_eq!(
        hmac_sha256_hex(&[0x0b; 20], b"Hi There"),
        "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
    );
    assert_eq!(
        hmac_sha256_hex(b"Jefe", b"what do ya want for nothing?"),
        "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
    );
    // a key longer than the block size is hashed first (RFC 4231 case 6)
    assert_eq!(
        hmac_sha256_hex(
            &[0xaa; 131],
            b"Test Using Larger Than Block-Size Key - Hash Key First"
        ),
        "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
    );
}
