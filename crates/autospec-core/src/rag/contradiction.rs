//! Contradiction records (spec sections 9 and 30).
//!
//! Section 9 is explicit: conflicts are surfaced, not silently resolved.
//! Authority tells the caller which side to *prefer*; it never deletes the
//! other side. The worked example in section 9 — spec says 30s, code says 60s,
//! telemetry shows 45s — is exactly the case where picking a winner would hide
//! the fact that the specification is wrong.

use crate::rag::authority::{AuthorityLadder, SourceAuthority};
use crate::rag::evidence::Evidence;

/// How much a contradiction matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ContradictionSeverity {
    /// Worth reporting; does not affect required behavior.
    Low,
    /// Affects a decision the caller is about to make.
    Medium,
    /// Affects behavior the specification requires.
    High,
}

impl ContradictionSeverity {
    /// Stable wire identifier.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

/// A recorded disagreement between two pieces of evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contradiction {
    id: String,
    topic: String,
    left_evidence: String,
    right_evidence: String,
    left_authority: SourceAuthority,
    right_authority: SourceAuthority,
    description: String,
    severity: ContradictionSeverity,
}

impl Contradiction {
    /// Record a contradiction between two evidence items.
    pub fn new(
        id: impl Into<String>,
        topic: impl Into<String>,
        left: &Evidence,
        right: &Evidence,
        description: impl Into<String>,
        severity: ContradictionSeverity,
    ) -> Self {
        Self {
            id: id.into(),
            topic: topic.into(),
            left_evidence: left.id().to_string(),
            right_evidence: right.id().to_string(),
            left_authority: left.authority(),
            right_authority: right.authority(),
            description: description.into(),
            severity,
        }
    }

    /// Contradiction identifier.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// What the two sides disagree about.
    pub fn topic(&self) -> &str {
        &self.topic
    }

    /// The first evidence id.
    pub fn left_evidence(&self) -> &str {
        &self.left_evidence
    }

    /// The second evidence id.
    pub fn right_evidence(&self) -> &str {
        &self.right_evidence
    }

    /// Human-readable description of the disagreement.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Assessed severity.
    pub fn severity(&self) -> ContradictionSeverity {
        self.severity
    }

    /// Return `true` when the contradiction must be resolved before autonomous
    /// implementation proceeds (spec section 30).
    pub fn requires_resolution(&self) -> bool {
        self.severity == ContradictionSeverity::High
    }

    /// The evidence id the ladder prefers, or `None` when both sides carry the
    /// same authority and nothing but a human can choose.
    pub fn preferred_evidence(&self, ladder: &AuthorityLadder) -> Option<&str> {
        if ladder.outranks(self.left_authority, self.right_authority) {
            Some(&self.left_evidence)
        } else if ladder.outranks(self.right_authority, self.left_authority) {
            Some(&self.right_evidence)
        } else {
            None
        }
    }

    /// A one-line summary for the context package and the dashboard.
    pub fn summary(&self) -> String {
        format!(
            "[{}] {}: {} ({} vs {})",
            self.severity.as_str(),
            self.topic,
            self.description,
            self.left_evidence,
            self.right_evidence
        )
    }
}

/// Accumulates contradictions found across a retrieval.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContradictionSet {
    records: Vec<Contradiction>,
}

impl ContradictionSet {
    /// An empty set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a contradiction, ignoring a duplicate of one already held.
    pub fn record(&mut self, contradiction: Contradiction) {
        if self
            .records
            .iter()
            .any(|held| held.id == contradiction.id || same_pair(held, &contradiction))
        {
            return;
        }
        self.records.push(contradiction);
    }

    /// Every recorded contradiction.
    pub fn records(&self) -> &[Contradiction] {
        &self.records
    }

    /// Return `true` when nothing was recorded.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// The first contradiction that blocks autonomous implementation.
    pub fn blocking(&self) -> Option<&Contradiction> {
        self.records
            .iter()
            .find(|contradiction| contradiction.requires_resolution())
    }

    /// Contradictions at or above a severity, most severe first.
    pub fn at_least(&self, severity: ContradictionSeverity) -> Vec<&Contradiction> {
        let mut matching = self
            .records
            .iter()
            .filter(|contradiction| contradiction.severity >= severity)
            .collect::<Vec<_>>();
        matching.sort_by(|left, right| right.severity.cmp(&left.severity));
        matching
    }
}

fn same_pair(left: &Contradiction, right: &Contradiction) -> bool {
    left.topic == right.topic
        && ((left.left_evidence == right.left_evidence
            && left.right_evidence == right.right_evidence)
            || (left.left_evidence == right.right_evidence
                && left.right_evidence == right.left_evidence))
}
