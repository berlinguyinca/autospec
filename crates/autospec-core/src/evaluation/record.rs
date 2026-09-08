//! Evaluation records: one judgment by one evaluator version, and when ranking
//! stops trusting it (spec §Data model).
//!
//! A record is an observation, so it is never edited. What does change is its
//! [`ActiveRankingStatus`] — the single field [`EvaluationRecord::mark_stale`]
//! touches — and every such change appends to `ranking_history`.

use serde::{Deserialize, Serialize};

use super::ids::{EpochId, EvaluationId, EvaluatorSlot, EvaluatorVersionRef, PromotionId};
use super::qualification::Verdict;
use super::statistics::Ppm;

/// How independent the judging runtime is from the thing it judged.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Independence {
    High,
    Reduced,
    None,
}

/// Which runtime produced a verdict.
///
/// `independence` is derived, not asserted: a judge drawn from the same
/// `provider_family` as the subject it judges is not independent of it, and an
/// unknown provider family resolves to [`Independence::None`] — the same
/// resolution an absent value gets, because in ranking an unknown is exactly as
/// untrustworthy as a known conflict.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeProvenance {
    pub model_id: String,
    pub provider_family: String,
    /// What actually ran: sandbox, commit, or runner identity.
    pub execution_identity: String,
    pub independence: Independence,
}

/// Whether active ranking may still use this record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActiveRankingStatus {
    /// Usable as it stands.
    Active,
    /// Its evaluator version was displaced by `superseded_by` at `epoch`.
    StaleForActiveRanking {
        superseded_by: EvaluatorVersionRef,
        epoch: EpochId,
    },
    /// Superseded by a replay of `replay_of`; the replay is the live record.
    Replayed { replay_of: EvaluationId },
    /// Kept for audit only; never ranked.
    HistoricalOnly,
}

/// One status change, appended to [`EvaluationRecord::ranking_history`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RankingTransition {
    pub at: u64,
    pub from: ActiveRankingStatus,
    pub to: ActiveRankingStatus,
    /// The promotion that caused it, when one did.
    pub promotion: Option<PromotionId>,
}

/// One judgment: which evaluator version, at which epoch, with what outcome.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvaluationRecord {
    pub schema: u64,
    pub evaluation_id: EvaluationId,
    /// What was judged — an issue, PR, diff, or artifact reference.
    pub subject: String,
    pub epoch_id: EpochId,
    pub evaluator: EvaluatorVersionRef,
    pub outcome: Verdict,
    /// Confidence in `outcome` when the evaluator reports one.
    pub score: Option<Ppm>,
    pub evidence_refs: Vec<String>,
    pub runtime: RuntimeProvenance,
    pub created_at: u64,
    pub active_ranking_status: ActiveRankingStatus,
    pub ranking_history: Vec<RankingTransition>,
}

impl EvaluationRecord {
    /// Does active ranking still need this record for `slot`@`version`?
    ///
    /// True only while the record is `Active` *and* pinned to exactly that
    /// version: a stale, replayed, or historical record has already left the
    /// ranking set, and a record from another version of the same slot was
    /// never in it.
    pub fn depends_on(&self, slot: EvaluatorSlot, version: u32) -> bool {
        matches!(self.active_ranking_status, ActiveRankingStatus::Active)
            && self.evaluator.slot == slot
            && self.evaluator.version == version
    }

    /// Mark this record stale because `superseded_by` displaced its evaluator
    /// at `epoch`. Returns whether anything changed.
    ///
    /// Idempotent by construction: only an `Active` record transitions, so a
    /// replayed promotion appends no second transition and cannot restale a
    /// record that is already stale, replayed, or historical.
    ///
    /// The observation itself is never rewritten. `outcome`, `score` and
    /// `evidence_refs` stay exactly as they were, because staleness is a claim
    /// about *ranking*, not about what the evaluator saw at the time.
    pub fn mark_stale(
        &mut self,
        superseded_by: EvaluatorVersionRef,
        epoch: EpochId,
        promotion: PromotionId,
        at: u64,
    ) -> bool {
        if !matches!(self.active_ranking_status, ActiveRankingStatus::Active) {
            return false;
        }
        let from = self.active_ranking_status.clone();
        let to = ActiveRankingStatus::StaleForActiveRanking {
            superseded_by,
            epoch,
        };
        self.active_ranking_status = to.clone();
        self.ranking_history.push(RankingTransition {
            at,
            from,
            to,
            promotion: Some(promotion),
        });
        true
    }
}

/// The records that depend on `slot`@`version`, sorted by evaluation id.
///
/// A promotion feeds this straight into `PromotionEvent::invalidated_evaluations`,
/// so the output is sorted: the same set of records must produce the same list
/// on every run, otherwise two runs of one promotion write different events and
/// the journal chain stops being reproducible.
pub fn stale_candidates<'a>(
    records: impl Iterator<Item = &'a EvaluationRecord>,
    slot: EvaluatorSlot,
    version: u32,
) -> Vec<EvaluationId> {
    let mut ids: Vec<EvaluationId> = records
        .filter(|record| record.depends_on(slot, version))
        .map(|record| record.evaluation_id.clone())
        .collect();
    ids.sort();
    ids
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::evaluation::EVALUATION_SCHEMA_VERSION;

    fn record(id: &str, evaluator: &str) -> EvaluationRecord {
        EvaluationRecord {
            schema: EVALUATION_SCHEMA_VERSION,
            evaluation_id: EvaluationId::parse(id).unwrap(),
            subject: format!("issue-{id}"),
            epoch_id: EpochId(1),
            evaluator: EvaluatorVersionRef::parse(evaluator).unwrap(),
            outcome: Verdict::Accept,
            score: Some(Ppm(900_000)),
            evidence_refs: vec!["evidence/review.md".into()],
            runtime: RuntimeProvenance {
                model_id: "judge-model".into(),
                provider_family: "family-a".into(),
                execution_identity: "runner-1".into(),
                independence: Independence::High,
            },
            created_at: 1,
            active_ranking_status: ActiveRankingStatus::Active,
            ranking_history: Vec::new(),
        }
    }

    #[test]
    fn mark_stale_is_idempotent_and_keeps_history() {
        let mut r = record("ev-a1", "architecture@1");
        let to = EvaluatorVersionRef::parse("architecture@2").unwrap();
        let promotion = PromotionId::parse("promo-x").unwrap();
        assert!(r.depends_on(EvaluatorSlot::Architecture, 1));

        assert!(r.mark_stale(to, EpochId(2), promotion.clone(), 10));
        assert!(!r.mark_stale(to, EpochId(2), promotion, 11));

        assert_eq!(r.ranking_history.len(), 1, "no second transition");
        assert!(!r.depends_on(EvaluatorSlot::Architecture, 1));
        assert_eq!(r.outcome, Verdict::Accept, "judgment is untouched");
        assert_eq!(r.score, Some(Ppm(900_000)), "judgment is untouched");
        assert_eq!(r.evidence_refs, vec!["evidence/review.md".to_string()]);

        let transition = &r.ranking_history[0];
        assert_eq!(transition.at, 10);
        assert_eq!(transition.from, ActiveRankingStatus::Active);
        assert_eq!(
            transition.to,
            ActiveRankingStatus::StaleForActiveRanking {
                superseded_by: to,
                epoch: EpochId(2)
            }
        );
        assert_eq!(r.active_ranking_status, transition.to);
    }

    #[test]
    fn mark_stale_leaves_replayed_and_historical_records_alone() {
        let to = EvaluatorVersionRef::parse("architecture@2").unwrap();
        let promotion = PromotionId::parse("promo-x").unwrap();
        for status in [
            ActiveRankingStatus::Replayed {
                replay_of: EvaluationId::parse("ev-old").unwrap(),
            },
            ActiveRankingStatus::HistoricalOnly,
            ActiveRankingStatus::StaleForActiveRanking {
                superseded_by: to,
                epoch: EpochId(9),
            },
        ] {
            let mut r = record("ev-a1", "architecture@1");
            r.active_ranking_status = status.clone();
            assert!(!r.mark_stale(to, EpochId(2), promotion.clone(), 12));
            assert_eq!(r.active_ranking_status, status);
            assert!(r.ranking_history.is_empty());
            assert!(!r.depends_on(EvaluatorSlot::Architecture, 1));
        }
    }

    #[test]
    fn depends_on_requires_the_exact_slot_and_version() {
        let r = record("ev-a1", "architecture@1");
        assert!(r.depends_on(EvaluatorSlot::Architecture, 1));
        assert!(!r.depends_on(EvaluatorSlot::Architecture, 2));
        assert!(!r.depends_on(EvaluatorSlot::Documentation, 1));
    }

    #[test]
    fn stale_candidates_selects_only_the_displaced_version() {
        let records = [
            record("ev-a1", "architecture@1"),
            record("ev-a2", "architecture@2"),
            record("ev-d1", "documentation@1"),
        ];
        assert_eq!(
            stale_candidates(records.iter(), EvaluatorSlot::Architecture, 1),
            vec![EvaluationId::parse("ev-a1").unwrap()]
        );
    }

    #[test]
    fn stale_candidates_skips_stale_records_and_sorts_its_output() {
        let mut displaced = record("ev-zzz", "architecture@1");
        let fresh = record("ev-aaa", "architecture@1");
        displaced.mark_stale(
            EvaluatorVersionRef::parse("architecture@2").unwrap(),
            EpochId(2),
            PromotionId::parse("promo-x").unwrap(),
            10,
        );

        let ids = stale_candidates(
            [displaced, fresh, record("ev-a2", "architecture@2")].iter(),
            EvaluatorSlot::Architecture,
            1,
        );
        assert_eq!(ids, vec![EvaluationId::parse("ev-aaa").unwrap()]);

        let many: Vec<EvaluationRecord> = ["ev-c", "ev-a", "ev-b"]
            .iter()
            .map(|id| record(id, "architecture@1"))
            .collect();
        let sorted = stale_candidates(many.iter(), EvaluatorSlot::Architecture, 1);
        let deduped: BTreeSet<_> = sorted.iter().collect();
        assert_eq!(sorted.len(), deduped.len());
        assert!(sorted.is_sorted(), "{sorted:?} must be sorted");
    }

    #[test]
    fn record_round_trips_through_json() {
        let r = record("ev-a1", "architecture@1");
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains("\"architecture@1\""), "{json}");
        assert!(json.contains("\"epoch-000001\""), "{json}");
        assert!(
            json.contains("\"active_ranking_status\":\"active\""),
            "{json}"
        );
        assert_eq!(serde_json::from_str::<EvaluationRecord>(&json).unwrap(), r);

        let mut stale = record("ev-a1", "architecture@1");
        stale.mark_stale(
            EvaluatorVersionRef::parse("architecture@2").unwrap(),
            EpochId(2),
            PromotionId::parse("promo-x").unwrap(),
            10,
        );
        let json = serde_json::to_string(&stale).unwrap();
        assert!(
            json.contains("stale_for_active_ranking"),
            "status tag must be snake_case: {json}"
        );
        assert_eq!(
            serde_json::from_str::<EvaluationRecord>(&json).unwrap(),
            stale
        );
    }
}
