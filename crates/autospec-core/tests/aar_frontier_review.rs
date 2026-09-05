//! When a frontier model may stand in for a human reviewer, and when it may not.

use autospec_core::aar::frontier_review::{accepts, GateKind, ReviewAttestation, ReviewerAuthority};

fn review(authority: ReviewerAuthority) -> ReviewAttestation {
    ReviewAttestation {
        authority,
        reviewer_id: "claude-opus-4.5".to_string(),
        session_id: "sess-review".to_string(),
        implementer_session_id: "sess-impl".to_string(),
        saw_implementer_reasoning: false,
    }
}

#[test]
fn a_frontier_model_satisfies_review_judgement() {
    let got = accepts(GateKind::ReviewJudgement, &review(ReviewerAuthority::Frontier));
    assert!(got.is_accepted(), "{got:?}");
    assert_eq!(got.reason(), "frontier-model review in a separate session");
}

#[test]
fn a_frontier_model_never_satisfies_authorization() {
    // Review judgement and authorization are different questions. A model can
    // answer "is this correct"; it has no standing to accept risk for the
    // project, and letting it appear to launders accountability.
    let got = accepts(GateKind::Authorization, &review(ReviewerAuthority::Frontier));
    assert!(!got.is_accepted(), "{got:?}");
    assert!(got.reason().contains("human"), "{got:?}");
}

#[test]
fn a_peer_tier_model_does_not_satisfy_a_gate_that_asked_for_independence() {
    let got = accepts(GateKind::ReviewJudgement, &review(ReviewerAuthority::Peer));
    assert!(!got.is_accepted(), "{got:?}");
}

#[test]
fn a_human_satisfies_both_gates() {
    for gate in [GateKind::ReviewJudgement, GateKind::Authorization] {
        assert!(accepts(gate, &review(ReviewerAuthority::Human)).is_accepted());
    }
}

#[test]
fn sharing_a_session_with_the_implementer_is_never_independent() {
    // Applies to every authority, including a human reviewing their own work.
    for authority in [
        ReviewerAuthority::Human,
        ReviewerAuthority::Frontier,
        ReviewerAuthority::Peer,
    ] {
        let mut r = review(authority);
        r.session_id = r.implementer_session_id.clone();
        let got = accepts(GateKind::ReviewJudgement, &r);
        assert!(!got.is_accepted(), "{authority} in a shared session: {got:?}");
        assert!(got.reason().contains("not independent"), "{got:?}");
    }
}

#[test]
fn a_reviewer_that_read_the_authors_reasoning_is_rejected() {
    // This is a real leak, not a hypothetical: a reviewer brief that includes the
    // PR body gets the implementer's own justification along with the diff.
    let mut r = review(ReviewerAuthority::Frontier);
    r.saw_implementer_reasoning = true;
    let got = accepts(GateKind::ReviewJudgement, &r);
    assert!(!got.is_accepted(), "{got:?}");
    assert!(got.reason().contains("defence"), "{got:?}");
}

#[test]
fn a_review_without_a_session_id_cannot_be_shown_to_be_separate() {
    let mut r = review(ReviewerAuthority::Frontier);
    r.session_id = "  ".to_string();
    assert!(!accepts(GateKind::ReviewJudgement, &r).is_accepted());
}
