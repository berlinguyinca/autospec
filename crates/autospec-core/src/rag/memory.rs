//! Tiered memory and the memory write policy (spec sections 16 and 17).
//!
//! Section 17's rule is that agents do not write arbitrary model output into
//! durable memory. The gate below is that rule as code: seven checks, all of
//! which must pass, evaluated in a fixed order so a rejection always names the
//! first thing wrong rather than a random one.

use crate::rag::evidence::{Evidence, Privacy, inherited_privacy};
use crate::rag::score::Score;

/// How long a memory entry is meant to live (spec section 16).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MemoryTier {
    /// Discoveries within one task or workflow.
    ShortTermTask,
    /// Durable project-specific knowledge.
    Project,
    /// Reusable patterns not tied to sensitive project content.
    GlobalAutospec,
}

impl MemoryTier {
    /// Stable wire identifier.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ShortTermTask => "short_term_task",
            Self::Project => "project",
            Self::GlobalAutospec => "global_autospec",
        }
    }
}

/// Lifecycle state of a memory entry (spec section 17).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryState {
    /// Proposed, not yet checked.
    Candidate,
    /// Passed the write policy.
    Validated,
    /// In use.
    Active,
    /// Replaced by a newer entry.
    Superseded,
    /// Failed the write policy.
    Rejected,
    /// Aged out.
    Expired,
}

impl MemoryState {
    /// Stable wire identifier.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Candidate => "candidate",
            Self::Validated => "validated",
            Self::Active => "active",
            Self::Superseded => "superseded",
            Self::Rejected => "rejected",
            Self::Expired => "expired",
        }
    }
}

/// A proposed memory write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryCandidate {
    /// Entry identifier.
    pub id: String,
    /// Tier the entry is proposed for.
    pub tier: MemoryTier,
    /// The claim to remember.
    pub content: String,
    /// Evidence ids backing it.
    pub provenance: Vec<String>,
    /// How confident the proposer is.
    pub confidence: Score,
    /// Privacy class of the strictest supporting evidence.
    pub privacy: Privacy,
    /// Whether this was a durable conclusion or a transient observation.
    pub durable: bool,
}

impl MemoryCandidate {
    /// Build a candidate from the evidence that supports it.
    ///
    /// Provenance and privacy are taken from the evidence rather than supplied,
    /// so a caller cannot assert a claim is public or well-sourced when its
    /// evidence says otherwise.
    pub fn from_evidence(
        id: impl Into<String>,
        tier: MemoryTier,
        content: impl Into<String>,
        supporting: &[Evidence],
        durable: bool,
    ) -> Self {
        let privacy = inherited_privacy(supporting);
        let confidence = Score::mean(&supporting.iter().map(Evidence::confidence).collect::<Vec<_>>());
        Self {
            id: id.into(),
            tier,
            content: content.into(),
            provenance: supporting.iter().map(|item| item.id().to_string()).collect(),
            confidence,
            privacy,
            durable,
        }
    }
}

/// Which of section 17's checks a candidate failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryRejection {
    /// Nothing a future task could act on.
    NotUseful,
    /// True only for the current task.
    NotDurable,
    /// No evidence backing.
    NoProvenance,
    /// Already remembered.
    Duplicate { existing_id: String },
    /// Contradicts an active entry.
    Contradicts { existing_id: String },
    /// Too sensitive for the proposed tier.
    PrivacyViolation { detail: String },
    /// Below the confidence floor.
    LowConfidence { threshold: Score },
}

impl MemoryRejection {
    /// A sentence for the audit log.
    pub fn describe(&self) -> String {
        match self {
            Self::NotUseful => "candidate carries nothing a future task could act on".to_string(),
            Self::NotDurable => "candidate is true only for the current task".to_string(),
            Self::NoProvenance => "candidate cites no supporting evidence".to_string(),
            Self::Duplicate { existing_id } => {
                format!("candidate duplicates active memory {existing_id}")
            }
            Self::Contradicts { existing_id } => {
                format!("candidate contradicts active memory {existing_id}")
            }
            Self::PrivacyViolation { detail } => format!("privacy policy: {detail}"),
            Self::LowConfidence { threshold } => {
                format!("confidence below the {threshold} floor")
            }
        }
    }
}

/// An entry that passed the write policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryEntry {
    /// Entry identifier.
    pub id: String,
    /// Tier it lives in.
    pub tier: MemoryTier,
    /// The remembered claim.
    pub content: String,
    /// Evidence ids backing it.
    pub provenance: Vec<String>,
    /// Confidence at write time.
    pub confidence: Score,
    /// Privacy class.
    pub privacy: Privacy,
    /// Lifecycle state.
    pub state: MemoryState,
}

/// The section 17 write gate.
#[derive(Debug, Clone)]
pub struct MemoryWritePolicy {
    confidence_floor: Score,
    active: Vec<MemoryEntry>,
}

impl Default for MemoryWritePolicy {
    fn default() -> Self {
        Self {
            confidence_floor: Score::from_permille(700),
            active: Vec::new(),
        }
    }
}

impl MemoryWritePolicy {
    /// A policy with the default confidence floor and no existing memory.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the confidence floor.
    pub fn with_confidence_floor(mut self, floor: Score) -> Self {
        self.confidence_floor = floor;
        self
    }

    /// Seed the policy with the memory already held, so the duplicate and
    /// contradiction checks have something to compare against.
    pub fn with_active(mut self, entries: Vec<MemoryEntry>) -> Self {
        self.active = entries;
        self
    }

    /// Active entries.
    pub fn active(&self) -> &[MemoryEntry] {
        &self.active
    }

    /// Run the seven checks in order.
    ///
    /// Order matters for the message the operator sees: an unsourced candidate
    /// is reported as unsourced even when it is also a duplicate, because
    /// fixing provenance is the actionable step.
    pub fn evaluate(&self, candidate: &MemoryCandidate) -> Result<MemoryEntry, MemoryRejection> {
        if candidate.content.trim().len() < 8 {
            return Err(MemoryRejection::NotUseful);
        }
        if !candidate.durable && candidate.tier != MemoryTier::ShortTermTask {
            return Err(MemoryRejection::NotDurable);
        }
        if candidate.provenance.is_empty() {
            return Err(MemoryRejection::NoProvenance);
        }
        if let Some(existing) = self
            .active
            .iter()
            .find(|entry| normalized(&entry.content) == normalized(&candidate.content))
        {
            return Err(MemoryRejection::Duplicate {
                existing_id: existing.id.clone(),
            });
        }
        if let Some(existing) = self.contradicting(candidate) {
            return Err(MemoryRejection::Contradicts {
                existing_id: existing.id.clone(),
            });
        }
        // Global memory is shared across projects; private evidence must not
        // reach it, and neither must project-internal evidence (section 16.3).
        if candidate.tier == MemoryTier::GlobalAutospec && candidate.privacy != Privacy::Public {
            return Err(MemoryRejection::PrivacyViolation {
                detail: format!(
                    "{} evidence cannot enter global AutoSpec memory",
                    candidate.privacy.as_str()
                ),
            });
        }
        if !candidate.confidence.at_least(self.confidence_floor) {
            return Err(MemoryRejection::LowConfidence {
                threshold: self.confidence_floor,
            });
        }
        Ok(MemoryEntry {
            id: candidate.id.clone(),
            tier: candidate.tier,
            content: candidate.content.clone(),
            provenance: candidate.provenance.clone(),
            confidence: candidate.confidence,
            privacy: candidate.privacy,
            state: MemoryState::Validated,
        })
    }

    /// Admit a validated entry as active.
    pub fn admit(&mut self, mut entry: MemoryEntry) -> &MemoryEntry {
        entry.state = MemoryState::Active;
        self.active.push(entry);
        self.active
            .last()
            .expect("an entry was just pushed")
    }

    /// Find an active entry the candidate contradicts.
    ///
    /// The heuristic is deliberately narrow: an entry about the same subject
    /// where one side negates the other. Broad contradiction detection belongs
    /// to the evaluator, which has the evidence; here a false positive would
    /// silently discard a correct memory.
    fn contradicting(&self, candidate: &MemoryCandidate) -> Option<&MemoryEntry> {
        let candidate_negated = is_negated(&candidate.content);
        let candidate_subject = subject(&candidate.content);
        self.active.iter().find(|entry| {
            entry.state == MemoryState::Active
                && subject(&entry.content) == candidate_subject
                && !candidate_subject.is_empty()
                && is_negated(&entry.content) != candidate_negated
        })
    }
}

fn normalized(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn is_negated(text: &str) -> bool {
    let lowered = text.to_lowercase();
    [" never ", " not ", " no longer ", " must not ", " does not "]
        .iter()
        .any(|marker| lowered.contains(marker))
}

fn subject(text: &str) -> String {
    normalized(text)
        .split_whitespace()
        .filter(|word| {
            !matches!(
                *word,
                "never" | "not" | "no" | "longer" | "must" | "does" | "is" | "are" | "the" | "a"
            )
        })
        .take(4)
        .collect::<Vec<_>>()
        .join(" ")
}
