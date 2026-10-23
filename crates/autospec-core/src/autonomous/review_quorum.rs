//! Head-bound post-PR review quorum (Herdr federated workbench, T06).
//!
//! ONE policy, implemented identically here (merge gating) and in Herdr's
//! `control_plane/review_quorum.py` (workbench read model). Both suites load
//! `tests/fixtures/review-quorum-vectors.json` (a byte-identical copy lives in
//! herdr-engineering); change a rule there first.
//!
//! A verdict is a statement about one exact commit of one review target:
//!
//! 1. **Blocks are processed first and are never silently dropped.** A `blocked`
//!    vote at the current head blocks, whoever filed it, unless an
//!    `eligible_reviewers` list is configured and the voter is not on it. A block
//!    whose reviewer id is malformed or whose commit is unparseable or a prefix of
//!    the head FAILS CLOSED (blocks); a block for another head is stale; a block
//!    for another target is irrelevant.
//! 2. An `lgtm` counts only if it is for this target and EXACTLY this head, from a
//!    valid eligible identity that is not the PR author or an excluded worker (nor
//!    on the implementer's provider when diversity is required), with a 64-hex
//!    `receipt_digest` whose `receipt_binding` recomputes over
//!    `(target, commit, reviewer, receipt_digest)`.
//! 3. Identities are trimmed, lower-cased, and `x[bot]` equals `x`; verdicts and
//!    digests are case-normalised. Identities in one `reviewer_groups` entry count
//!    once. One digest backing `lgtm` votes of two groups voids both (replay).
//! 4. A missing author is an error, never a pass.
//!
//! Residual: the binding proves a stored vote was not copied across target, commit
//! or reviewer; it is not a signature. Two keys held by one person that are not
//! listed in a group still count twice.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuorumVote {
    pub reviewer: String,
    pub target: String,
    #[serde(default)]
    pub provider: Option<String>,
    /// `lgtm` or `blocked`, case-insensitive.
    pub verdict: String,
    pub commit: String,
    #[serde(default)]
    pub receipt_digest: Option<String>,
    #[serde(default)]
    pub receipt_binding: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuorumPolicy {
    pub required: usize,
    /// REQUIRED: a missing author would let the author vote as a reviewer.
    pub pr_author: String,
    #[serde(default)]
    pub excluded_identities: Vec<String>,
    /// When non-empty only these identities can approve OR block.
    #[serde(default)]
    pub eligible_reviewers: Vec<String>,
    /// Identities of one person/organisation count once.
    #[serde(default)]
    pub reviewer_groups: Vec<Vec<String>>,
    /// Optional GitHub login of a reviewer identity, to recognise the PR author behind a key.
    #[serde(default)]
    pub reviewer_logins: BTreeMap<String, String>,
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
    InvalidVerdict,
    NotEligible,
    Author,
    ExcludedIdentity,
    ProviderNotDiverse,
    BadReceiptDigest,
    BadReceiptBinding,
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
    pub stale_reviewers: Vec<String>,
    pub rereview_required: bool,
    pub rereview_needed: usize,
}

/// ASCII-only normalisation, identical in Herdr: whitespace is the six ASCII blanks and case
/// folding touches only A-Z. Unicode folding (Kelvin sign, dotted I, ...) is deliberately NOT
/// applied, so the two implementations cannot disagree on who a name is.
fn ascii_trim(value: &str) -> &str {
    value.trim_matches(|c: char| matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0b' | '\x0c'))
}

fn printable_ascii(value: &str) -> bool {
    value.bytes().all(|b| (0x21..=0x7e).contains(&b))
}

/// A review target (`pr:host/owner/repo#n`): printable ASCII only, lower-cased.
pub fn canonical_target(value: &str) -> Option<String> {
    let lower = ascii_trim(value).to_ascii_lowercase();
    (!lower.is_empty() && printable_ascii(&lower)).then_some(lower)
}

pub fn canonical_commit(value: &str) -> Option<String> {
    let lower = ascii_trim(value).to_ascii_lowercase();
    let ok =
        (lower.len() == 40 || lower.len() == 64) && lower.bytes().all(|b| b.is_ascii_hexdigit());
    ok.then_some(lower)
}

pub fn canonical_identity(value: &str) -> Option<String> {
    let mut id = ascii_trim(value).to_ascii_lowercase();
    if let Some(stripped) = id.strip_suffix("[bot]") {
        id = stripped.to_string();
    }
    if id.is_empty() || id.len() > 128 || !printable_ascii(&id) {
        return None;
    }
    Some(id)
}

fn canonical_digest(value: &str) -> Option<String> {
    let lower = ascii_trim(value).to_ascii_lowercase();
    (lower.len() == 64 && lower.bytes().all(|b| b.is_ascii_hexdigit())).then_some(lower)
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The binding a producer attaches to a receipt digest and an evaluator recomputes.
pub fn receipt_binding(target: &str, commit: &str, reviewer: &str, receipt_digest: &str) -> String {
    let tuple = format!(
        "pr-review-receipt-v1\0{}\0{}\0{}\0{}",
        canonical_target(target).unwrap_or_default(),
        ascii_trim(commit).to_ascii_lowercase(),
        canonical_identity(reviewer).unwrap_or_default(),
        ascii_trim(receipt_digest).to_ascii_lowercase()
    );
    Sha256::digest(tuple.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[derive(PartialEq)]
enum CommitClass {
    Current,
    Stale,
    Invalid,
}

fn classify_commit(raw: &str, head: &str) -> CommitClass {
    match canonical_commit(raw) {
        Some(c) if c == head => CommitClass::Current,
        Some(_) => CommitClass::Stale,
        None => {
            // A prefix of the head (>= 7 hex) is "this head, badly written"; anything
            // else unparseable is unknowable. Both must fail closed for a block.
            let lower = ascii_trim(raw).to_ascii_lowercase();
            let hex = !lower.is_empty() && lower.bytes().all(|b| b.is_ascii_hexdigit());
            if hex && lower.len() >= 7 && head.starts_with(&lower) {
                CommitClass::Current
            } else {
                CommitClass::Invalid
            }
        }
    }
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
    let target = canonical_target(target)
        .ok_or_else(|| "review quorum target must be non-empty printable ASCII".to_string())?;
    let author = canonical_identity(&policy.pr_author).ok_or_else(|| {
        "review quorum pr_author is required and must be a valid identity".to_string()
    })?;
    let ident = |raw: &String, what: &str| {
        canonical_identity(raw).ok_or_else(|| format!("review quorum {what} is invalid"))
    };
    let excluded: BTreeSet<String> = policy
        .excluded_identities
        .iter()
        .map(|r| ident(r, "excluded identity"))
        .collect::<Result<_, _>>()?;
    let eligible: BTreeSet<String> = policy
        .eligible_reviewers
        .iter()
        .map(|r| ident(r, "eligible reviewer"))
        .collect::<Result<_, _>>()?;
    let mut group_of: BTreeMap<String, String> = BTreeMap::new();
    for group in &policy.reviewer_groups {
        let members: Vec<String> = group
            .iter()
            .map(|r| ident(r, "reviewer group member"))
            .collect::<Result<_, _>>()?;
        let key = members
            .iter()
            .min()
            .cloned()
            .ok_or("review quorum reviewer group is empty")?;
        for member in members {
            if group_of.insert(member, key.clone()).is_some() {
                return Err("review quorum reviewer groups overlap".to_string());
            }
        }
    }
    let logins: BTreeMap<String, String> = policy
        .reviewer_logins
        .iter()
        .filter_map(|(id, login)| Some((canonical_identity(id)?, canonical_identity(login)?)))
        .collect();
    let group = |id: &str| group_of.get(id).cloned().unwrap_or_else(|| id.to_string());
    let implementer_provider = policy
        .implementer_provider
        .as_deref()
        .map(|p| ascii_trim(p).to_ascii_lowercase());

    let mut rejected: BTreeSet<Rejection> = BTreeSet::new();
    let mut stale: BTreeSet<String> = BTreeSet::new();
    let mut blocking: BTreeSet<String> = BTreeSet::new();
    let reject = |set: &mut BTreeSet<Rejection>, who: &str, reason| {
        set.insert(Rejection {
            reviewer: who.to_string(),
            reason,
        });
    };

    // Phase 1: blocks. Normalise, or fail closed; never drop.
    let mut approvals: Vec<(&QuorumVote, String)> = Vec::new();
    for vote in votes {
        let verdict = ascii_trim(&vote.verdict).to_ascii_lowercase();
        let identity = canonical_identity(&vote.reviewer);
        let label = identity.clone().unwrap_or_else(|| "<invalid>".to_string());
        if canonical_target(&vote.target).as_deref() != Some(target.as_str()) {
            reject(&mut rejected, &label, RejectReason::WrongTarget);
            continue;
        }
        match verdict.as_str() {
            "blocked" => {
                if !eligible.is_empty() && !identity.as_ref().is_some_and(|i| eligible.contains(i))
                {
                    reject(&mut rejected, &label, RejectReason::NotEligible);
                    continue;
                }
                match classify_commit(&vote.commit, &head) {
                    CommitClass::Stale => {
                        stale.insert(label.clone());
                        reject(&mut rejected, &label, RejectReason::StaleHead);
                    }
                    CommitClass::Current => {
                        blocking.insert(group(&label));
                    }
                    CommitClass::Invalid => {
                        reject(&mut rejected, &label, RejectReason::InvalidCommit);
                        blocking.insert(group(&label));
                    }
                }
            }
            "lgtm" => approvals.push((vote, verdict)),
            _ => reject(&mut rejected, &label, RejectReason::InvalidVerdict),
        }
    }

    // Phase 2: approvals.
    let mut lgtm: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (vote, _) in approvals {
        let Some(reviewer) = canonical_identity(&vote.reviewer) else {
            reject(&mut rejected, "<invalid>", RejectReason::InvalidReviewer);
            continue;
        };
        match canonical_commit(&vote.commit) {
            None => {
                reject(&mut rejected, &reviewer, RejectReason::InvalidCommit);
                continue;
            }
            Some(c) if c != head => {
                stale.insert(reviewer.clone());
                reject(&mut rejected, &reviewer, RejectReason::StaleHead);
                continue;
            }
            Some(_) => {}
        }
        if !eligible.is_empty() && !eligible.contains(&reviewer) {
            reject(&mut rejected, &reviewer, RejectReason::NotEligible);
            continue;
        }
        let login = logins.get(&reviewer);
        if author == reviewer || login == Some(&author) {
            reject(&mut rejected, &reviewer, RejectReason::Author);
            continue;
        }
        if excluded.contains(&reviewer) {
            reject(&mut rejected, &reviewer, RejectReason::ExcludedIdentity);
            continue;
        }
        if policy.require_provider_diversity {
            let provider = vote
                .provider
                .as_deref()
                .map(|p| ascii_trim(p).to_ascii_lowercase());
            let diverse = matches!((&provider, &implementer_provider),
                (Some(p), Some(i)) if !p.is_empty() && p != i);
            if !diverse {
                reject(&mut rejected, &reviewer, RejectReason::ProviderNotDiverse);
                continue;
            }
        }
        let Some(digest) = vote.receipt_digest.as_deref().and_then(canonical_digest) else {
            reject(&mut rejected, &reviewer, RejectReason::BadReceiptDigest);
            continue;
        };
        let expected = receipt_binding(&target, &head, &reviewer, &digest);
        if vote
            .receipt_binding
            .as_deref()
            .and_then(canonical_digest)
            .as_deref()
            != Some(expected.as_str())
        {
            reject(&mut rejected, &reviewer, RejectReason::BadReceiptBinding);
            continue;
        }
        lgtm.entry(group(&reviewer)).or_default().insert(digest);
    }

    let mut holders: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for (who, digests) in &lgtm {
        for digest in digests {
            holders.entry(digest).or_default().insert(who);
        }
    }
    let replayed: BTreeSet<String> = holders
        .values()
        .filter(|who| who.len() > 1)
        .flat_map(|who| who.iter().map(|w| w.to_string()))
        .collect();
    let mut counted = Vec::new();
    for who in lgtm.keys() {
        if replayed.contains(who) {
            reject(&mut rejected, who, RejectReason::ReplayedReceipt);
        } else if !blocking.contains(who) {
            counted.push(who.clone());
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
