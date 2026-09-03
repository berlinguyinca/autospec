//! Freshness and staleness policy (spec section 31).
//!
//! "Fresh" means something different per source: a runtime metric is stale
//! after a minute, source code is stale the moment the revision moves, and an
//! ADR is stale only when superseded. One TTL for all of them would either
//! serve minute-old telemetry as current or re-fetch an ADR that has not
//! changed in a year.

use crate::rag::score::Score;
use crate::rag::source::SourceKind;

/// When evidence from a source stops counting as current.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StalenessRule {
    /// Stale as soon as the indexed revision changes.
    RevisionChange,
    /// Stale after a fixed number of seconds.
    After { seconds: u64 },
    /// Stale only when a successor record supersedes it.
    Superseded,
    /// Never treated as stale by age alone.
    Never,
}

impl StalenessRule {
    /// Stable wire identifier.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RevisionChange => "revision_change",
            Self::After { .. } => "after",
            Self::Superseded => "superseded",
            Self::Never => "never",
        }
    }
}

/// Everything needed to judge one item's freshness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FreshnessInput {
    /// Revision the evidence was captured at.
    pub captured_revision: String,
    /// Revision considered current now.
    pub current_revision: String,
    /// When the evidence was retrieved, seconds since the Unix epoch.
    pub retrieved_at: u64,
    /// The evaluation time, seconds since the Unix epoch.
    pub now: u64,
    /// Whether a successor record has superseded this one.
    pub superseded: bool,
}

/// The verdict for one item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    /// Current under its source's rule.
    Current,
    /// Aged but still inside its window.
    Aging,
    /// Past its rule; must not be served as current evidence.
    Stale,
}

impl Freshness {
    /// Stable wire identifier.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Aging => "aging",
            Self::Stale => "stale",
        }
    }

    /// Return `true` when the item may still be served as current.
    pub const fn is_usable(self) -> bool {
        !matches!(self, Self::Stale)
    }

    /// The freshness score carried on an evidence item.
    pub const fn score(self) -> Score {
        match self {
            Self::Current => Score::ONE,
            Self::Aging => Score::from_permille(600),
            Self::Stale => Score::ZERO,
        }
    }
}

/// Per-source staleness rules (spec section 31's `source_policies`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FreshnessPolicy {
    rules: Vec<(SourceKind, StalenessRule)>,
}

impl Default for FreshnessPolicy {
    /// The `source_policies` defaults from specification section 31.
    fn default() -> Self {
        Self {
            rules: vec![
                (SourceKind::Repository, StalenessRule::RevisionChange),
                (SourceKind::Test, StalenessRule::RevisionChange),
                (SourceKind::Specification, StalenessRule::Superseded),
                (SourceKind::Adr, StalenessRule::Superseded),
                (SourceKind::Runtime, StalenessRule::After { seconds: 60 }),
                (SourceKind::GitHub, StalenessRule::After { seconds: 3_600 }),
                (
                    SourceKind::Documentation,
                    StalenessRule::After {
                        seconds: 7 * 86_400,
                    },
                ),
                (
                    SourceKind::Web,
                    StalenessRule::After {
                        seconds: 7 * 86_400,
                    },
                ),
                (SourceKind::Memory, StalenessRule::Superseded),
            ],
        }
    }
}

impl FreshnessPolicy {
    /// Override the rule for one source kind.
    pub fn with_rule(mut self, kind: SourceKind, rule: StalenessRule) -> Self {
        if let Some(slot) = self
            .rules
            .iter_mut()
            .find(|(candidate, _)| *candidate == kind)
        {
            slot.1 = rule;
        } else {
            self.rules.push((kind, rule));
        }
        self
    }

    /// The rule for a source kind; unlisted kinds never go stale by age.
    pub fn rule(&self, kind: SourceKind) -> StalenessRule {
        self.rules
            .iter()
            .find(|(candidate, _)| *candidate == kind)
            .map(|(_, rule)| *rule)
            .unwrap_or(StalenessRule::Never)
    }

    /// Judge one item.
    ///
    /// A time-based rule reports `Aging` from three quarters of its window, so
    /// the loop can prefer a fresher authority (section 6's `stale` branch)
    /// before the item actually expires.
    pub fn assess(&self, kind: SourceKind, input: &FreshnessInput) -> Freshness {
        match self.rule(kind) {
            StalenessRule::Never => Freshness::Current,
            StalenessRule::Superseded => {
                if input.superseded {
                    Freshness::Stale
                } else {
                    Freshness::Current
                }
            }
            StalenessRule::RevisionChange => {
                if input.captured_revision == input.current_revision {
                    Freshness::Current
                } else {
                    Freshness::Stale
                }
            }
            StalenessRule::After { seconds } => {
                let age = input.now.saturating_sub(input.retrieved_at);
                if age >= seconds {
                    Freshness::Stale
                } else if seconds > 0 && age * 4 >= seconds * 3 {
                    Freshness::Aging
                } else {
                    Freshness::Current
                }
            }
        }
    }
}
