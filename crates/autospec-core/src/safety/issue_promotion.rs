use std::collections::{BTreeMap, BTreeSet};

use crate::claim::{evaluate_claim_safety_with_trusted_actors, ClaimSafetyInput};
use crate::coordination::{
    plan_ready_queue_with_trusted_actors, PullRequestEvidence, QueuePolicy, ReadyQueueInput,
    RemoteIssue,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssuePromotionPayload {
    pub number: u64,
    pub title: String,
    pub body: String,
    pub author: String,
    pub labels: Vec<String>,
}

impl IssuePromotionPayload {
    pub fn new(
        number: u64,
        title: impl Into<String>,
        body: impl Into<String>,
        author: impl Into<String>,
        labels: Vec<String>,
    ) -> Self {
        Self {
            number,
            title: title.into(),
            body: body.into(),
            author: author.into(),
            labels,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssuePromotionSafetyDecision {
    Pass,
    Ambiguous,
    Blocked,
    Indeterminate,
}

impl IssuePromotionSafetyDecision {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Ambiguous => "ambiguous",
            Self::Blocked => "blocked",
            Self::Indeterminate => "indeterminate",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssuePromotionDecision {
    pub number: u64,
    pub title: String,
    pub safety_decision: IssuePromotionSafetyDecision,
    pub safety_reason: String,
    pub auto_implement: bool,
    pub eligible: bool,
    pub final_labels: Vec<String>,
    pub blocked_by_reason: BTreeMap<String, usize>,
}

pub fn evaluate_issue_promotion(payload: IssuePromotionPayload) -> IssuePromotionDecision {
    evaluate_issue_promotion_with_trusted_actors(payload, &["berlinguyinca"])
}

pub fn evaluate_issue_promotion_with_trusted_actors(
    payload: IssuePromotionPayload,
    trusted_actors: &[&str],
) -> IssuePromotionDecision {
    let labels_for_final_verdict = with_label(payload.labels.clone(), "auto-implement");
    let safety = evaluate_claim_safety_with_trusted_actors(
        &ClaimSafetyInput::new(
            labels_for_final_verdict,
            payload.title.clone(),
            payload.body.clone(),
            payload.author.clone(),
        ),
        trusted_actors,
    );
    let safety_decision = classify_issue_promotion_safety(safety.allowed, safety.reason);
    let auto_implement = safety_decision == IssuePromotionSafetyDecision::Pass;
    let final_labels = if auto_implement {
        with_label(payload.labels.clone(), "auto-implement")
    } else {
        without_label(payload.labels.clone(), "auto-implement")
    };
    let eligible = auto_implement
        && promoted_payload_is_eligible(&payload, final_labels.clone(), trusted_actors);
    let mut blocked_by_reason = BTreeMap::new();
    if matches!(
        safety_decision,
        IssuePromotionSafetyDecision::Blocked | IssuePromotionSafetyDecision::Indeterminate
    ) {
        *blocked_by_reason
            .entry(safety.reason.to_string())
            .or_insert(0) += 1;
    }

    IssuePromotionDecision {
        number: payload.number,
        title: payload.title,
        safety_decision,
        safety_reason: safety.reason.to_string(),
        auto_implement,
        eligible,
        final_labels,
        blocked_by_reason,
    }
}

fn classify_issue_promotion_safety(allowed: bool, reason: &str) -> IssuePromotionSafetyDecision {
    if allowed {
        return IssuePromotionSafetyDecision::Pass;
    }
    match reason {
        "current_body_safety_ambiguous" => IssuePromotionSafetyDecision::Ambiguous,
        "invalid_safety_markers"
        | "missing_safety_review_heading"
        | "unexpected_safety_review_preamble"
        | "unexpected_safety_block_content"
        | "missing_safety_pass" => IssuePromotionSafetyDecision::Indeterminate,
        _ => IssuePromotionSafetyDecision::Blocked,
    }
}

fn promoted_payload_is_eligible(
    payload: &IssuePromotionPayload,
    final_labels: Vec<String>,
    trusted_actors: &[&str],
) -> bool {
    let issue = RemoteIssue::open(
        payload.number,
        payload.title.clone(),
        payload.body.clone(),
        final_labels,
        payload.author.clone(),
    );
    let plan = plan_ready_queue_with_trusted_actors(
        &ReadyQueueInput {
            candidates: vec![issue],
            active: Vec::new(),
            dependencies: BTreeMap::new(),
            pull_requests: PullRequestEvidence::Available(Vec::new()),
            policy: QueuePolicy::new(1, 0),
        },
        trusted_actors,
    );
    plan.ready
        .iter()
        .any(|ready| ready.issue.number == payload.number)
}

fn with_label(labels: Vec<String>, label: &str) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut normalized = labels
        .into_iter()
        .filter(|current| seen.insert(current.clone()))
        .collect::<Vec<_>>();
    if !seen.contains(label) {
        normalized.push(label.to_string());
    }
    normalized
}

fn without_label(labels: Vec<String>, label: &str) -> Vec<String> {
    let mut seen = BTreeSet::new();
    labels
        .into_iter()
        .filter(|current| current != label)
        .filter(|current| seen.insert(current.clone()))
        .collect()
}
