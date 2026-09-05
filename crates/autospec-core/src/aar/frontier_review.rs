//! When a high-reasoning frontier model may stand in for a human reviewer.
//!
//! AutoSpec gates some work on human review. That gate exists for two different
//! reasons which are usually conflated, and separating them is the whole point of
//! this module:
//!
//! * **Review judgement** — "is this code correct, well-shaped, adequately
//!   tested?" A capable reviewer answers this from the diff and the spec. Nothing
//!   about it requires a person, and in practice a frontier model reviewing in a
//!   separate session catches a class of defect that a same-tier peer does not.
//!
//! * **Authorization** — "may we spend this money, accept this risk, publish
//!   under this licence, deploy to production?" This is an accountability
//!   decision. A model has no standing to make it, and a model that appears to
//!   make it launders responsibility away from whoever is answerable.
//!
//! So a frontier reviewer may satisfy the first and never the second.
//!
//! The alternative in practice is not "human review" but *no review at all*:
//! `OPEN_GATES.md` in a downstream repo records that no independent reviewer is
//! enrolled, so its "obtain review in a separate session" step is a procedural
//! separation rather than an independent party. A frontier model in a distinct
//! session is strictly better than that, provided it is labelled honestly.

use std::fmt;

/// Who performed a review, in terms of what their sign-off is worth.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReviewerAuthority {
    /// A person. The only authority that can accept risk on behalf of the project.
    Human,
    /// A designated high-reasoning frontier model, reviewing in its own session.
    Frontier,
    /// Any other model, including the implementer's own tier.
    Peer,
}

impl ReviewerAuthority {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::Frontier => "frontier",
            Self::Peer => "peer",
        }
    }
}

impl fmt::Display for ReviewerAuthority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What a gate is actually asking for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GateKind {
    /// "Is this change correct and well-made?"
    ReviewJudgement,
    /// "Do we accept this risk / cost / obligation?"
    Authorization,
}

/// A review that has actually happened, described well enough to be judged.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewAttestation {
    pub authority: ReviewerAuthority,
    /// Model or person identifier, recorded verbatim in the evidence trail.
    pub reviewer_id: String,
    /// The session that produced the review.
    pub session_id: String,
    /// The session that produced the implementation.
    pub implementer_session_id: String,
    /// Whether the reviewer saw the implementer's internal reasoning. A reviewer
    /// that read the author's own defence is not independent of it.
    pub saw_implementer_reasoning: bool,
}

/// Why a review was or was not accepted. Carrying the reason is the point: a
/// bare bool would let "accepted" and "accepted for the wrong reason" look alike
/// in the evidence trail.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReviewAcceptance {
    Accepted { reason: &'static str },
    Rejected { reason: &'static str },
}

impl ReviewAcceptance {
    pub fn is_accepted(&self) -> bool {
        matches!(self, Self::Accepted { .. })
    }
    pub fn reason(&self) -> &'static str {
        match self {
            Self::Accepted { reason } | Self::Rejected { reason } => reason,
        }
    }
}

/// Decide whether an attested review satisfies a gate.
pub fn accepts(gate: GateKind, review: &ReviewAttestation) -> ReviewAcceptance {
    // Independence first: it applies to every authority, including a human who
    // reviewed their own work.
    if review.session_id.trim().is_empty() {
        return ReviewAcceptance::Rejected {
            reason: "review has no session id, so it cannot be shown to be separate",
        };
    }
    if review.session_id == review.implementer_session_id {
        return ReviewAcceptance::Rejected {
            reason: "reviewer and implementer share a session; the review is not independent",
        };
    }
    if review.saw_implementer_reasoning {
        return ReviewAcceptance::Rejected {
            reason: "reviewer saw the implementer's reasoning; it reviewed a defence, not a diff",
        };
    }

    match (gate, review.authority) {
        // Only a person can accept risk on the project's behalf.
        (GateKind::Authorization, ReviewerAuthority::Human) => ReviewAcceptance::Accepted {
            reason: "human authorization",
        },
        (GateKind::Authorization, _) => ReviewAcceptance::Rejected {
            reason: "authorization requires a human; a model has no standing to accept risk",
        },

        (GateKind::ReviewJudgement, ReviewerAuthority::Human) => ReviewAcceptance::Accepted {
            reason: "human review",
        },
        (GateKind::ReviewJudgement, ReviewerAuthority::Frontier) => ReviewAcceptance::Accepted {
            reason: "frontier-model review in a separate session",
        },
        (GateKind::ReviewJudgement, ReviewerAuthority::Peer) => ReviewAcceptance::Rejected {
            reason: "peer-tier review does not satisfy a gate that asked for independence",
        },
    }
}
