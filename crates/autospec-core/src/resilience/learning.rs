//! Verified Engineering Learning — turning validated engineering outcomes into
//! reusable memory without letting unverified agent guesses poison shared
//! memory.
//!
//! The loop: implementation -> deterministic validation -> independent review
//! -> outcome evidence -> lesson candidate -> critic/validator -> scope +
//! confidence -> durable promotion to shared memory -> later retrieval through
//! the memory map.
//!
//! Critical invariant: an unvalidated candidate can never silently become
//! authoritative shared memory, and role/safety/merge policy is versioned
//! policy that the learning system must never be able to weaken by writing a
//! "lesson".

use serde::{Deserialize, Serialize};

use super::ids::{
    AttemptId, CandidateId, WorkId,
};

/// Versioned lesson-candidate schema identity.
pub const LESSON_CANDIDATE_SCHEMA: &str = "autospec.lesson-candidate.v1";

/// The kind of lesson.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LessonKind {
    Procedure,
    Warning,
    Architecture,
    Debugging,
    Test,
    Tooling,
    FailurePattern,
}

impl LessonKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            LessonKind::Procedure => "procedure",
            LessonKind::Warning => "warning",
            LessonKind::Architecture => "architecture",
            LessonKind::Debugging => "debugging",
            LessonKind::Test => "test",
            LessonKind::Tooling => "tooling",
            LessonKind::FailurePattern => "failure-pattern",
        }
    }
}

/// Lesson lifecycle status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum LessonStatus {
    Candidate,
    Validated,
    Rejected,
    Promoted,
    Superseded,
}

impl LessonStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            LessonStatus::Candidate => "candidate",
            LessonStatus::Validated => "validated",
            LessonStatus::Rejected => "rejected",
            LessonStatus::Promoted => "promoted",
            LessonStatus::Superseded => "superseded",
        }
    }
}

/// Evidence attached to a candidate.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Evidence {
    pub validation_results: Vec<String>,
    pub review_results: Vec<String>,
    pub counterevidence: Vec<String>,
}

/// A lesson candidate with provenance and confidence.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LessonCandidate {
    pub schema: String,
    pub candidate_id: CandidateId,
    pub created_at: String,
    pub source_work_id: WorkId,
    pub source_attempt_id: AttemptId,
    pub repository: Option<String>,
    pub commit: Option<String>,
    pub issue: Option<String>,
    pub pull_request: Option<String>,
    pub kind: LessonKind,
    pub statement: String,
    pub scope: String,
    pub preconditions: Vec<String>,
    pub evidence: Evidence,
    pub confidence: f32,
    pub status: LessonStatus,
    pub supersedes: Option<CandidateId>,
    pub expires_at: Option<String>,
    pub memory_target: Option<String>,
}

impl LessonCandidate {
    pub fn new(
        candidate_id: CandidateId,
        source_work_id: WorkId,
        source_attempt_id: AttemptId,
        kind: LessonKind,
        statement: impl Into<String>,
        scope: impl Into<String>,
    ) -> Self {
        Self {
            schema: LESSON_CANDIDATE_SCHEMA.to_string(),
            candidate_id,
            created_at: String::new(),
            source_work_id,
            source_attempt_id,
            repository: None,
            commit: None,
            issue: None,
            pull_request: None,
            kind,
            statement: statement.into(),
            scope: scope.into(),
            preconditions: Vec::new(),
            evidence: Evidence {
                validation_results: Vec::new(),
                review_results: Vec::new(),
                counterevidence: Vec::new(),
            },
            confidence: 0.0,
            status: LessonStatus::Candidate,
            supersedes: None,
            expires_at: None,
            memory_target: None,
        }
    }
}

/// The promotion decision for a candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromotionVerdict {
    /// Has validation + review evidence, scoped, no unresolved contradiction.
    Promote,
    /// Has some evidence but not enough to be authoritative; stays a candidate.
    KeepCandidate,
    /// Has disqualifying evidence or is unsafe/secret-bearing.
    Reject,
}

/// Deterministic promotion gate.
///
/// A lesson may only be promoted when it has traceable source work, validation
/// evidence appropriate to the claim, independent review evidence, scoped
/// applicability, no unresolved counterevidence that invalidates the claim, and
/// explicit confidence. A candidate may stay retrievable when explicitly
/// requested, but never becomes authoritative by default.
pub fn promote_verdict(candidate: &LessonCandidate) -> PromotionVerdict {
    if candidate.status == LessonStatus::Rejected {
        return PromotionVerdict::Reject;
    }
    // Safety: never promote a secret-bearing lesson.
    if contains_secret(candidate) {
        return PromotionVerdict::Reject;
    }
    if candidate.statement.trim().is_empty() || candidate.scope.trim().is_empty() {
        return PromotionVerdict::KeepCandidate;
    }
    let has_validation = !candidate.evidence.validation_results.is_empty();
    let has_review = !candidate.evidence.review_results.is_empty();
    let has_unresolved_contradiction = !candidate.evidence.counterevidence.is_empty();
    if has_validation && has_review && !has_unresolved_contradiction && candidate.confidence > 0.0 {
        PromotionVerdict::Promote
    } else {
        PromotionVerdict::KeepCandidate
    }
}

/// Secret detection for lessons — reuses the conservative denylist.
fn contains_secret(candidate: &LessonCandidate) -> bool {
    use super::context_guardian::contains_secret_like;
    let text = [
        candidate.statement.as_str(),
        &candidate.scope,
        &candidate.evidence.validation_results.join("\n"),
        &candidate.evidence.review_results.join("\n"),
        &candidate.evidence.counterevidence.join("\n"),
    ]
    .join("\n");
    contains_secret_like(&text).is_some()
}

/// Mark a candidate validated, promoted, rejected, or superseded.
pub fn set_status(candidate: &mut LessonCandidate, status: LessonStatus) {
    candidate.status = status;
}

/// Whether the candidate may be treated as authoritative shared memory.
pub fn is_authoritative(candidate: &LessonCandidate) -> bool {
    candidate.status == LessonStatus::Promoted
}

/// A candidate supersedes another: the old one becomes superseded and the new
/// one carries the `supersedes` reference. The old lesson is no longer
/// authoritative.
pub fn supersede(
    old: &mut LessonCandidate,
    new: &mut LessonCandidate,
) {
    old.status = LessonStatus::Superseded;
    new.supersedes = Some(old.candidate_id.clone());
}

/// Guard: role/safety/merge policy can never be weakened through a "lesson".
/// These topics are versioned policy, not learned memory.
pub fn touches_immutable_policy(statement: &str) -> bool {
    let markers = [
        "merge gate",
        "security gate",
        "review requirement",
        "model separation",
        "role permission",
        "resource cap",
        "secret handling",
        "merge policy",
        "gate authority",
    ];
    markers.iter().any(|m| statement.to_lowercase().contains(m))
}

/// Policy topics that must never be written as a lesson.
pub fn is_unsafe_lesson(candidate: &LessonCandidate) -> bool {
    touches_immutable_policy(&candidate.statement)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base(kind: LessonKind, statement: &str) -> LessonCandidate {
        LessonCandidate::new(
            CandidateId::new(b"c"),
            WorkId::new(b"w"),
            AttemptId::new(b"a"),
            kind,
            statement,
            "repo/autospec-orchestrator",
        )
    }

    fn with_evidence(mut c: LessonCandidate) -> LessonCandidate {
        c.evidence.validation_results.push("validation passed".to_string());
        c.evidence.review_results.push("independent review OK".to_string());
        c.confidence = 0.9;
        c
    }

    #[test]
    fn candidate_from_success_requires_evidence_to_promote() {
        let bare = base(LessonKind::Procedure, "use X");
        assert_eq!(promote_verdict(&bare), PromotionVerdict::KeepCandidate);
        let evidenced = with_evidence(base(LessonKind::Procedure, "use X"));
        assert_eq!(promote_verdict(&evidenced), PromotionVerdict::Promote);
    }

    #[test]
    fn contradiction_blocks_promotion() {
        let mut c = with_evidence(base(LessonKind::Procedure, "use X"));
        c.evidence.counterevidence.push("X fails on Y".to_string());
        assert_eq!(promote_verdict(&c), PromotionVerdict::KeepCandidate);
    }

    #[test]
    fn rejected_candidate_is_not_promoted() {
        let mut c = with_evidence(base(LessonKind::Procedure, "use X"));
        c.status = LessonStatus::Rejected;
        assert_eq!(promote_verdict(&c), PromotionVerdict::Reject);
        assert!(!is_authoritative(&c));
    }

    #[test]
    fn secret_containing_lesson_is_rejected() {
        let mut c = with_evidence(base(LessonKind::Warning, "use token ghp_abcdefghijklmnopqrstuvwxyz"));
        assert_eq!(promote_verdict(&c), PromotionVerdict::Reject);
    }

    #[test]
    fn supersession_marks_old_lesson_non_authoritative() {
        let mut old = with_evidence(base(LessonKind::Procedure, "old way"));
        old.status = LessonStatus::Promoted;
        let mut new = with_evidence(base(LessonKind::Procedure, "new way"));
        supersede(&mut old, &mut new);
        assert_eq!(old.status, LessonStatus::Superseded);
        assert_eq!(new.supersedes, Some(old.candidate_id.clone()));
        assert!(!is_authoritative(&old));
    }

    #[test]
    fn policy_cannot_be_weakened_by_lesson() {
        assert!(touches_immutable_policy("relax the merge gate"));
        assert!(touches_immutable_policy("weaken security review requirements"));
        assert!(!touches_immutable_policy("use cargo test for validation"));
        let c = base(LessonKind::Procedure, "weaken the security gate");
        assert!(is_unsafe_lesson(&c));
    }

    #[test]
    fn failure_patterns_are_valid_lessons() {
        let mut c = base(LessonKind::FailurePattern, "CI file-size-ratchet fails on main");
        c.evidence.validation_results.push("baseline confirmed".to_string());
        c.evidence.review_results.push("review confirmed".to_string());
        c.confidence = 0.7;
        assert_eq!(promote_verdict(&c), PromotionVerdict::Promote);
    }
}
