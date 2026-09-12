//! §10 pattern detection, §11 finding schema, §12 finding lifecycle.
//!
//! Spec: [`docs/specs/2026-09-08-continuous-improvement-engine.md`](../../../../docs/specs/2026-09-08-continuous-improvement-engine.md)
//! §10 (Pattern Detection Engine), §11 (Finding Schema), §12 (Finding
//! Lifecycle), §45 (Configuration).
//!
//! [`detect`] is the deterministic half of the §10 engine: it groups the
//! stored `user_interventions` (§9 user re-steering) by
//! (repo, model, task, intervention class), qualifies a group against the
//! §45 thresholds (`recurring_pattern_min_sessions` /
//! `recurring_pattern_min_occurrences`) and emits one §11 [`Finding`] per
//! qualifying group with recency-weighted confidence. Semantic clustering
//! (embedding-based pattern identity, §10/§36) is a later stage and
//! intentionally absent here: the pattern identity is the structured
//! intervention class, so two runs over the same rows produce the same
//! findings.
//!
//! Security (§39): a finding and its evidence carry **row ids only** —
//! session ids, intervention sequence numbers and labels. No payload,
//! excerpt or prompt text ever enters a [`Finding`] or a
//! `finding_evidence` row.
//!
//! §35 evidence preservation: every returned [`Finding`] writes one
//! `finding_evidence` row per supporting intervention, and
//! [`Finding::sessions`] lists every session the finding is backed by.
//!
//! §12 lifecycle guard: [`detect`] never overwrites the lifecycle status of
//! an already-persisted pattern row — a re-detect refreshes the aggregates
//! only, so a `resolved` finding cannot be reset to `candidate` by the
//! engine. Only [`transition`] moves a finding through the §12 chain.

pub mod detect;

pub use detect::{detect, DetectConfig};

use std::collections::BTreeMap;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::AutospecError;

/// §11 severity values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Below the medium occurrence floor.
    Low,
    /// At or above [`MEDIUM_OCCURRENCE_FLOOR`].
    Medium,
    /// At or above [`HIGH_OCCURRENCE_FLOOR`].
    High,
}

/// §11 severity floor (occurrences) for [`Severity::High`].
pub const HIGH_OCCURRENCE_FLOOR: u64 = 10;
/// §11 severity floor (occurrences) for [`Severity::Medium`].
pub const MEDIUM_OCCURRENCE_FLOOR: u64 = 5;

impl Severity {
    /// The §11 snake_case wire name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }

    /// Deterministic §11 severity from the raw occurrence count.
    pub const fn from_occurrences(occurrences: u64) -> Self {
        if occurrences >= HIGH_OCCURRENCE_FLOOR {
            Self::High
        } else if occurrences >= MEDIUM_OCCURRENCE_FLOOR {
            Self::Medium
        } else {
            Self::Low
        }
    }
}

impl FromStr for Severity {
    type Err = AutospecError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(match value {
            "low" => Self::Low,
            "medium" => Self::Medium,
            "high" => Self::High,
            _ => {
                return Err(AutospecError::parse(
                    "finding severity",
                    format!("unknown §11 severity {value:?}"),
                ))
            }
        })
    }
}

/// §12 finding lifecycle states, in chain order.
///
/// ```text
/// candidate -> active -> acknowledged -> proposal_created -> fix_in_progress
///  -> monitoring -> resolved | regressed | dismissed
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingStatus {
    /// A fresh pattern that has qualified; nobody has looked at it yet.
    #[default]
    Candidate,
    /// Confirmed as a real recurring problem.
    Active,
    /// A human (or the strong model) has confirmed it is worth acting on.
    Acknowledged,
    /// An improvement proposal (§20) was created for this finding.
    ProposalCreated,
    /// The proposed fix is being implemented.
    FixInProgress,
    /// The fix shipped; the engine watches for recurrence.
    Monitoring,
    /// No recurrence while monitored. Terminal.
    Resolved,
    /// The problem recurred after the fix. Terminal.
    Regressed,
    /// Judged not worth acting on. Terminal.
    Dismissed,
}

impl FindingStatus {
    /// The §12 snake_case wire name (stored in `patterns.status`).
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Candidate => "candidate",
            Self::Active => "active",
            Self::Acknowledged => "acknowledged",
            Self::ProposalCreated => "proposal_created",
            Self::FixInProgress => "fix_in_progress",
            Self::Monitoring => "monitoring",
            Self::Resolved => "resolved",
            Self::Regressed => "regressed",
            Self::Dismissed => "dismissed",
        }
    }

    /// The three terminal branches out of `monitoring`: no outgoing edges.
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Resolved | Self::Regressed | Self::Dismissed)
    }
}

impl FromStr for FindingStatus {
    type Err = AutospecError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(match value {
            "candidate" => Self::Candidate,
            "active" => Self::Active,
            "acknowledged" => Self::Acknowledged,
            "proposal_created" => Self::ProposalCreated,
            "fix_in_progress" => Self::FixInProgress,
            "monitoring" => Self::Monitoring,
            "resolved" => Self::Resolved,
            "regressed" => Self::Regressed,
            "dismissed" => Self::Dismissed,
            _ => {
                return Err(AutospecError::parse(
                    "finding status",
                    format!("unknown §12 finding status {value:?}"),
                ))
            }
        })
    }
}

/// §12 lifecycle transition: only the edges drawn in the §12 chain are
/// legal, and each terminal state has no outgoing edges — a `resolved`
/// finding cannot regress back to `active` (or anywhere else). A
/// same-state "transition" is rejected so a caller cannot re-apply a
/// state by accident.
///
/// Returns the new status on success.
pub fn transition(from: FindingStatus, to: FindingStatus) -> Result<FindingStatus, AutospecError> {
    let legal = if from.is_terminal() {
        false
    } else {
        matches!(
            (from, to),
            (FindingStatus::Candidate, FindingStatus::Active)
                | (FindingStatus::Active, FindingStatus::Acknowledged)
                | (FindingStatus::Acknowledged, FindingStatus::ProposalCreated)
                | (FindingStatus::ProposalCreated, FindingStatus::FixInProgress)
                | (FindingStatus::FixInProgress, FindingStatus::Monitoring)
                | (FindingStatus::Monitoring, FindingStatus::Resolved)
                | (FindingStatus::Monitoring, FindingStatus::Regressed)
                | (FindingStatus::Monitoring, FindingStatus::Dismissed)
        )
    };
    if legal {
        Ok(to)
    } else {
        Err(AutospecError::state(
            "finding lifecycle",
            format!(
                "illegal transition {} -> {} (§12: candidate -> active -> acknowledged \
                 -> proposal_created -> fix_in_progress -> monitoring -> resolved | \
                 regressed | dismissed)",
                from.as_str(),
                to.as_str()
            ),
        ))
    }
}

/// §11 `estimated_cost`: the rework a recurring pattern costs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct EstimatedCost {
    /// Extra tokens burned by the recurring problem.
    pub extra_tokens: u64,
    /// Extra minutes burned by the recurring problem.
    pub extra_minutes: u64,
}

/// §35 evidence reference: a row id, never payload text (§39).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FindingEvidence {
    /// The session the supporting row lives in.
    pub session_id: String,
    /// Row anchor inside the session (e.g. `user_intervention#3`); `None`
    /// when the source table carries no per-row sequence.
    pub event_ref: Option<String>,
}

/// One §11 finding — exactly the fifteen fields of the §11 schema, in
/// schema order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    /// §11 `finding_id` — deterministic, stable across detect re-runs.
    pub finding_id: String,
    /// §11 `type` (the field is named `kind`: `type` is a Rust keyword).
    #[serde(rename = "type")]
    pub kind: String,
    /// §11 `title` — human-readable summary built from labels only (§39).
    pub title: String,
    /// §11 `status` — the §12 lifecycle state.
    pub status: FindingStatus,
    /// §11 `first_seen` — ISO-8601 UTC of the oldest supporting row.
    pub first_seen: String,
    /// §11 `last_seen` — ISO-8601 UTC of the newest supporting row.
    pub last_seen: String,
    /// §11 `occurrences` — total supporting rows.
    pub occurrences: u64,
    /// §11 `sessions` — every session the finding is backed by (§35), sorted.
    pub sessions: Vec<String>,
    /// §11 `repositories` — the repositories the pattern was seen in.
    pub repositories: Vec<String>,
    /// §11 `models` — model label -> occurrence count.
    pub models: BTreeMap<String, u64>,
    /// §11 `confidence` — recency-weighted, in `0..=1`.
    pub confidence: f64,
    /// §11 `severity`.
    pub severity: Severity,
    /// §11 `estimated_cost`.
    pub estimated_cost: EstimatedCost,
    /// §11 `evidence` — one row-id reference per supporting row.
    pub evidence: Vec<FindingEvidence>,
    /// §11 `candidate_remediations` — empty until proposal generation
    /// (§20, a later issue) fills it.
    pub candidate_remediations: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_statuses() -> [FindingStatus; 9] {
        [
            FindingStatus::Candidate,
            FindingStatus::Active,
            FindingStatus::Acknowledged,
            FindingStatus::ProposalCreated,
            FindingStatus::FixInProgress,
            FindingStatus::Monitoring,
            FindingStatus::Resolved,
            FindingStatus::Regressed,
            FindingStatus::Dismissed,
        ]
    }

    fn sample_finding() -> Finding {
        Finding {
            finding_id: "finding|fixture/repo|qwen3|wi-1|replan".to_string(),
            kind: "recurring_user_intervention".to_string(),
            title: "Recurring user intervention: replan".to_string(),
            status: FindingStatus::Candidate,
            first_seen: "2026-07-13T00:00:00Z".to_string(),
            last_seen: "2026-07-13T08:00:00Z".to_string(),
            occurrences: 6,
            sessions: vec![
                "sess-1".to_string(),
                "sess-2".to_string(),
                "sess-3".to_string(),
            ],
            repositories: vec!["fixture/repo".to_string()],
            models: std::collections::BTreeMap::from([("qwen3".to_string(), 6)]),
            confidence: 0.87,
            severity: Severity::Medium,
            estimated_cost: EstimatedCost {
                extra_tokens: 72_000,
                extra_minutes: 30,
            },
            evidence: vec![FindingEvidence {
                session_id: "sess-1".to_string(),
                event_ref: Some("user_intervention#1".to_string()),
            }],
            candidate_remediations: vec![],
        }
    }

    #[test]
    fn finding_serializes_exactly_the_15_schema_11_fields() {
        let value = serde_json::to_value(sample_finding()).unwrap();
        let mut keys: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "candidate_remediations",
                "confidence",
                "estimated_cost",
                "evidence",
                "finding_id",
                "first_seen",
                "last_seen",
                "models",
                "occurrences",
                "repositories",
                "sessions",
                "severity",
                "status",
                "title",
                "type",
            ]
        );
        assert_eq!(value["type"], "recurring_user_intervention");
        assert_eq!(value["status"], "candidate");
        assert_eq!(value["severity"], "medium");
        assert_eq!(value["models"]["qwen3"], 6);
    }

    #[test]
    fn finding_status_wire_names_match_section_12() {
        assert_eq!(FindingStatus::Candidate.as_str(), "candidate");
        assert_eq!(FindingStatus::Active.as_str(), "active");
        assert_eq!(FindingStatus::Acknowledged.as_str(), "acknowledged");
        assert_eq!(FindingStatus::ProposalCreated.as_str(), "proposal_created");
        assert_eq!(FindingStatus::FixInProgress.as_str(), "fix_in_progress");
        assert_eq!(FindingStatus::Monitoring.as_str(), "monitoring");
        assert_eq!(FindingStatus::Resolved.as_str(), "resolved");
        assert_eq!(FindingStatus::Regressed.as_str(), "regressed");
        assert_eq!(FindingStatus::Dismissed.as_str(), "dismissed");
    }

    #[test]
    fn all_status_wire_names_round_trip() {
        for status in all_statuses() {
            let parsed = FindingStatus::from_str(status.as_str()).unwrap();
            assert_eq!(parsed, status);
        }
        assert!(matches!(
            FindingStatus::from_str("not-a-status").unwrap_err(),
            AutospecError::Parse { .. }
        ));
    }

    /// Every one of the 81 (from, to) pairs is classified: exactly the
    /// eight §12 edges succeed and return the target; every other pair —
    /// including all same-state pairs and every edge out of a terminal
    /// state — errors.
    #[test]
    fn every_transition_pair_is_classified() {
        let legal: [(FindingStatus, FindingStatus); 8] = [
            (FindingStatus::Candidate, FindingStatus::Active),
            (FindingStatus::Active, FindingStatus::Acknowledged),
            (FindingStatus::Acknowledged, FindingStatus::ProposalCreated),
            (FindingStatus::ProposalCreated, FindingStatus::FixInProgress),
            (FindingStatus::FixInProgress, FindingStatus::Monitoring),
            (FindingStatus::Monitoring, FindingStatus::Resolved),
            (FindingStatus::Monitoring, FindingStatus::Regressed),
            (FindingStatus::Monitoring, FindingStatus::Dismissed),
        ];
        for &from in all_statuses().iter() {
            for &to in all_statuses().iter() {
                let expected = legal.contains(&(from, to));
                match transition(from, to) {
                    Ok(actual) => {
                        assert!(
                            expected,
                            "transition {} -> {} must be rejected",
                            from.as_str(),
                            to.as_str()
                        );
                        assert_eq!(actual, to);
                    }
                    Err(error) => {
                        assert!(
                            !expected,
                            "legal transition {} -> {} was rejected: {error:?}",
                            from.as_str(),
                            to.as_str()
                        );
                    }
                }
            }
        }
    }

    /// §12 operations review: a resolved finding cannot regress back to
    /// active — `resolved` has no outgoing edges at all.
    #[test]
    fn resolved_cannot_regress_to_candidate_or_active() {
        for to in all_statuses() {
            assert!(
                transition(FindingStatus::Resolved, to).is_err(),
                "resolved -> {} must be rejected",
                to.as_str()
            );
        }
        assert!(matches!(
            transition(FindingStatus::Resolved, FindingStatus::Candidate),
            Err(AutospecError::State { .. })
        ));
    }

    #[test]
    fn terminal_states_have_no_outgoing_edges() {
        for terminal in [
            FindingStatus::Resolved,
            FindingStatus::Regressed,
            FindingStatus::Dismissed,
        ] {
            assert!(terminal.is_terminal());
            for to in all_statuses() {
                assert!(transition(terminal, to).is_err());
            }
        }
        // Exactly the three §12 branches out of monitoring are terminal.
        let terminal_count = all_statuses()
            .iter()
            .filter(|status| status.is_terminal())
            .count();
        assert_eq!(terminal_count, 3);
    }

    #[test]
    fn severity_mapping_is_deterministic() {
        assert_eq!(Severity::from_occurrences(0), Severity::Low);
        assert_eq!(Severity::from_occurrences(4), Severity::Low);
        assert_eq!(
            Severity::from_occurrences(MEDIUM_OCCURRENCE_FLOOR),
            Severity::Medium
        );
        assert_eq!(Severity::from_occurrences(9), Severity::Medium);
        assert_eq!(
            Severity::from_occurrences(HIGH_OCCURRENCE_FLOOR),
            Severity::High
        );
    }

    #[test]
    fn severity_wire_names_round_trip() {
        for (name, severity) in [
            ("low", Severity::Low),
            ("medium", Severity::Medium),
            ("high", Severity::High),
        ] {
            assert_eq!(severity.as_str(), name);
            assert_eq!(Severity::from_str(name).unwrap(), severity);
        }
        assert!(matches!(
            Severity::from_str("extreme").unwrap_err(),
            AutospecError::Parse { .. }
        ));
    }
}
