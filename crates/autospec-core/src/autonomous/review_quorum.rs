//! Head-bound post-PR review quorum (Herdr federated workbench, T06).
//!
//! An independent review verdict is a statement about one exact commit
//! (`review_commit` in the executor review receipt, see
//! `executor_bridge/review_receipt.rs`). A quorum is therefore only ever
//! evaluated against the CURRENT head of the PR, never against "the feature":
//!
//! 1. A vote whose commit is not exactly the current head is **stale**. It is
//!    reported, never counted, and never reused after a force-push or repair.
//! 2. Only distinct, normalised reviewer identities count; the same identity
//!    voting twice is one voter.
//! 3. The PR author and any excluded identity (implementer workers) cannot
//!    supply an `lgtm`; with provider diversity required, a reviewer on the
//!    implementer's provider (or on an unknown provider) cannot either.
//! 4. A receipt digest shared by two identities is a replay: neither `lgtm`
//!    counts.
//! 5. A vote for another review target (repository + PR) is ignored: the same
//!    SHA on a stacked or duplicate PR is a different diff against a different base.
//! 6. Filters only ever remove `lgtm` votes. A current-head `blocked` verdict
//!    always blocks, whoever filed it (fail closed).
//!
//! The function is pure and deterministic: no clock, no I/O.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoteVerdict {
    Lgtm,
    Blocked,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuorumVote {
    pub reviewer: String,
    /// The review target the verdict was issued for (repository + PR, e.g.
    /// `owner/repo#12`). A receipt for the same SHA on another PR is not this PR's.
    pub target: String,
    #[serde(default)]
    pub provider: Option<String>,
    pub verdict: VoteVerdict,
    /// The commit the verdict was issued for (the receipt's `review_commit`).
    pub commit: String,
    /// The receipt's `verdict_digest`; required for an `lgtm` to count.
    #[serde(default)]
    pub receipt_digest: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuorumPolicy {
    pub required: usize,
    /// REQUIRED: a missing author would let the author vote as a reviewer.
    pub pr_author: String,
    #[serde(default)]
    pub excluded_identities: Vec<String>,
    #[serde(default)]
    pub implementer_provider: Option<String>,
    #[serde(default)]
    pub require_provider_diversity: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectReason {
    StaleHead,
    WrongTarget,
    InvalidCommit,
    InvalidReviewer,
    Author,
    ExcludedIdentity,
    ProviderNotDiverse,
    MissingReceiptDigest,
    ReplayedReceipt,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Rejection {
    pub reviewer: String,
    pub reason: RejectReason,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuorumStatus {
    Satisfied,
    Insufficient,
    Blocked,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuorumOutcome {
    pub head: String,
    pub status: QuorumStatus,
    pub required: usize,
    pub counted: Vec<String>,
    pub blocking: Vec<String>,
    pub rejected: Vec<Rejection>,
    /// Reviewers whose verdict was for an older head and so was not reused.
    pub stale_reviewers: Vec<String>,
    /// True when the current head still needs fresh independent review.
    pub rereview_required: bool,
    pub rereview_needed: usize,
}

fn canonical_commit(value: &str) -> Option<String> {
    let lower = value.trim().to_ascii_lowercase();
    let ok =
        (lower.len() == 40 || lower.len() == 64) && lower.bytes().all(|b| b.is_ascii_hexdigit());
    ok.then_some(lower)
}

fn canonical_identity(value: &str) -> Option<String> {
    let id = value.trim().to_ascii_lowercase();
    if id.is_empty() || id.len() > 128 || id.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return None;
    }
    Some(id)
}

pub fn evaluate_quorum(
    target: &str,
    head: &str,
    votes: &[QuorumVote],
    policy: &QuorumPolicy,
) -> Result<QuorumOutcome, String> {
    let head = canonical_commit(head)
        .ok_or_else(|| "review quorum requires a full 40 or 64 hex character head".to_string())?;
    if policy.required == 0 {
        return Err("review quorum requires at least one reviewer".to_string());
    }
    let author = canonical_identity(&policy.pr_author).ok_or_else(|| {
        "review quorum pr_author is required and must be a valid identity".to_string()
    })?;
    let target = target.trim();
    if target.is_empty() {
        return Err("review quorum requires a review target".to_string());
    }
    let mut excluded: BTreeSet<String> = BTreeSet::new();
    for raw in &policy.excluded_identities {
        excluded.insert(
            canonical_identity(raw)
                .ok_or_else(|| "review quorum excluded identity is invalid".to_string())?,
        );
    }
    let implementer_provider = policy
        .implementer_provider
        .as_deref()
        .map(|p| p.trim().to_ascii_lowercase());

    let mut rejected: BTreeSet<Rejection> = BTreeSet::new();
    let mut stale: BTreeSet<String> = BTreeSet::new();
    let mut blocking: BTreeSet<String> = BTreeSet::new();
    // reviewer -> receipt digest, for current-head lgtm votes that passed every filter.
    let mut lgtm: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

    for vote in votes {
        let Some(reviewer) = canonical_identity(&vote.reviewer) else {
            rejected.insert(Rejection {
                reviewer: "<invalid>".into(),
                reason: RejectReason::InvalidReviewer,
            });
            continue;
        };
        let reject = |rejected: &mut BTreeSet<Rejection>, reason| {
            rejected.insert(Rejection {
                reviewer: reviewer.clone(),
                reason,
            });
        };
        if vote.target.trim() != target {
            reject(&mut rejected, RejectReason::WrongTarget);
            continue;
        }
        match canonical_commit(&vote.commit) {
            None => {
                reject(&mut rejected, RejectReason::InvalidCommit);
                continue;
            }
            Some(commit) if commit != head => {
                stale.insert(reviewer.clone());
                reject(&mut rejected, RejectReason::StaleHead);
                continue;
            }
            Some(_) => {}
        }
        if vote.verdict == VoteVerdict::Blocked {
            blocking.insert(reviewer);
            continue;
        }
        if author == reviewer {
            reject(&mut rejected, RejectReason::Author);
            continue;
        }
        if excluded.contains(&reviewer) {
            reject(&mut rejected, RejectReason::ExcludedIdentity);
            continue;
        }
        if policy.require_provider_diversity {
            let provider = vote
                .provider
                .as_deref()
                .map(|p| p.trim().to_ascii_lowercase());
            let diverse = matches!((&provider, &implementer_provider),
                (Some(p), Some(i)) if !p.is_empty() && p != i);
            if !diverse {
                reject(&mut rejected, RejectReason::ProviderNotDiverse);
                continue;
            }
        }
        let Some(digest) = vote
            .receipt_digest
            .as_deref()
            .map(str::trim)
            .filter(|d| !d.is_empty())
        else {
            reject(&mut rejected, RejectReason::MissingReceiptDigest);
            continue;
        };
        lgtm.entry(reviewer).or_default().insert(digest.to_string());
    }

    // Replay: one digest backing lgtm votes of several identities voids all of them.
    let mut by_digest: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for (reviewer, digests) in &lgtm {
        for digest in digests {
            by_digest.entry(digest).or_default().insert(reviewer);
        }
    }
    let replayed: BTreeSet<String> = by_digest
        .values()
        .filter(|who| who.len() > 1)
        .flat_map(|who| who.iter().map(|w| w.to_string()))
        .collect();
    let mut counted = Vec::new();
    for reviewer in lgtm.keys() {
        if replayed.contains(reviewer) {
            rejected.insert(Rejection {
                reviewer: reviewer.clone(),
                reason: RejectReason::ReplayedReceipt,
            });
        } else if !blocking.contains(reviewer) {
            counted.push(reviewer.clone());
        }
    }

    let status = if !blocking.is_empty() {
        QuorumStatus::Blocked
    } else if counted.len() >= policy.required {
        QuorumStatus::Satisfied
    } else {
        QuorumStatus::Insufficient
    };
    let rereview_needed = if status == QuorumStatus::Insufficient {
        policy.required - counted.len()
    } else {
        0
    };
    Ok(QuorumOutcome {
        head,
        status,
        required: policy.required,
        counted,
        blocking: blocking.into_iter().collect(),
        rejected: rejected.into_iter().collect(),
        stale_reviewers: stale.into_iter().collect(),
        rereview_required: status == QuorumStatus::Insufficient,
        rereview_needed,
    })
}
