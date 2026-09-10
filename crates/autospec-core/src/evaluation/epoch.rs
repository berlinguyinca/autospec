//! Epochs: the pinned set of active evaluator versions (spec §Data model).
//!
//! An epoch is immutable once written. A promotion never edits one; it creates
//! [`EvaluatorEpoch::successor`], which is the only way a new evaluator version
//! becomes visible to ranking. That is what makes a bad evaluator reversible:
//! the previous epoch stays on disk with its own id.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::digest::Digest;
use super::error::EvaluationError;
use super::ids::{AnchorSuiteId, EpochId, EvaluatorSlot, PromotionId};
use super::EVALUATION_SCHEMA_VERSION;

/// One immutable generation of evaluator versions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvaluatorEpoch {
    pub schema: u64,
    pub epoch_id: EpochId,
    /// The version active per slot. A `BTreeMap` makes "at most one version per
    /// slot" structural rather than something `validate()` has to check.
    pub slot_versions: BTreeMap<EvaluatorSlot, u32>,
    pub started_at: u64,
    pub predecessor: Option<EpochId>,
    /// The promotion that rotated this epoch; `None` only for genesis.
    pub promotion: Option<PromotionId>,
    pub policy_digest: Digest,
    /// `suite_id -> suite_digest` for every anchor suite pinned here, so a
    /// later run can prove which frozen suite a verdict was reached against.
    pub anchor_suite_digests: BTreeMap<AnchorSuiteId, Digest>,
}

impl EvaluatorEpoch {
    /// The empty epoch a chain starts from: nothing pinned, no predecessor.
    pub fn genesis(policy_digest: Digest, at: u64) -> Self {
        Self {
            schema: EVALUATION_SCHEMA_VERSION,
            epoch_id: EpochId(0),
            slot_versions: BTreeMap::new(),
            started_at: at,
            predecessor: None,
            promotion: None,
            policy_digest,
            anchor_suite_digests: BTreeMap::new(),
        }
    }

    /// The version pinned for `slot` in this epoch, if any.
    pub fn version_of(&self, slot: EvaluatorSlot) -> Option<u32> {
        self.slot_versions.get(&slot).copied()
    }

    /// The epoch following this one, with `slot` pinned to `version`.
    ///
    /// Every other slot is carried forward, and `suite` is pinned alongside it
    /// when given. The new id is always `self.epoch_id.next()`, so ids only
    /// ever increase by one.
    ///
    /// Fails when `version` is not strictly greater than the version already
    /// pinned for the slot (and `0` for an unpinned slot): an epoch may not
    /// silently downgrade or re-pin to the version it already has, because a
    /// replacement has to be a *new* evaluator version for the stale marking in
    /// [`crate::evaluation::record`] to mean anything.
    pub fn successor(
        &self,
        slot: EvaluatorSlot,
        version: u32,
        promotion: PromotionId,
        policy_digest: Digest,
        suite: Option<(AnchorSuiteId, Digest)>,
        at: u64,
    ) -> Result<Self, EvaluationError> {
        // An unpinned slot starts at 0, so any `version >= 1` is an increase.
        let existing = self.slot_versions.get(&slot).copied().unwrap_or(0);
        if version <= existing {
            return Err(EvaluationError::invariant(format!(
                "epoch {} pins {slot}@{existing}; a successor must pin a higher version, got {version}",
                self.epoch_id
            )));
        }

        let mut slot_versions = self.slot_versions.clone();
        slot_versions.insert(slot, version);
        let mut anchor_suite_digests = self.anchor_suite_digests.clone();
        if let Some((suite_id, suite_digest)) = suite {
            anchor_suite_digests.insert(suite_id, suite_digest);
        }

        Ok(Self {
            schema: EVALUATION_SCHEMA_VERSION,
            epoch_id: self.epoch_id.next(),
            slot_versions,
            started_at: at,
            predecessor: Some(self.epoch_id),
            promotion: Some(promotion),
            policy_digest,
            anchor_suite_digests,
        })
    }

    /// The invariants of a persisted epoch: known schema, strictly increasing
    /// ids, and a genesis that was not produced by a promotion.
    pub fn validate(&self) -> Result<(), EvaluationError> {
        if self.schema != EVALUATION_SCHEMA_VERSION {
            return Err(EvaluationError::parse(format!(
                "epoch {} has schema {}, expected {}",
                self.epoch_id, self.schema, EVALUATION_SCHEMA_VERSION
            )));
        }
        match self.predecessor {
            Some(predecessor) if predecessor >= self.epoch_id => {
                return Err(EvaluationError::invariant(format!(
                    "epoch {} claims predecessor {}, which is not strictly older",
                    self.epoch_id, predecessor
                )));
            }
            None if self.epoch_id != EpochId(0) => {
                return Err(EvaluationError::invariant(format!(
                    "epoch {} has no predecessor; only epoch-000000 may",
                    self.epoch_id
                )));
            }
            _ => {}
        }
        if self.epoch_id == EpochId(0) && self.promotion.is_some() {
            return Err(EvaluationError::invariant(
                "epoch-000000 is genesis and carries no promotion",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn promo(tag: &str) -> PromotionId {
        PromotionId::parse(tag).unwrap()
    }

    fn genesis() -> EvaluatorEpoch {
        EvaluatorEpoch::genesis(Digest::of_bytes(b"policy"), 1)
    }

    #[test]
    fn successor_increments_id_and_requires_a_newer_version() {
        let g = genesis();
        assert_eq!(g.epoch_id, EpochId(0));
        let e1 = g
            .successor(
                EvaluatorSlot::Architecture,
                1,
                promo("promo-1"),
                Digest::of_bytes(b"policy"),
                None,
                2,
            )
            .unwrap();
        assert_eq!(e1.epoch_id, EpochId(1));
        assert_eq!(e1.predecessor, Some(EpochId(0)));
        assert_eq!(e1.version_of(EvaluatorSlot::Architecture), Some(1));
        assert!(e1
            .successor(
                EvaluatorSlot::Architecture,
                1,
                promo("promo-2"),
                Digest::of_bytes(b"policy"),
                None,
                3
            )
            .is_err());
        let e2 = e1
            .successor(
                EvaluatorSlot::Documentation,
                1,
                promo("promo-3"),
                Digest::of_bytes(b"policy"),
                None,
                3,
            )
            .unwrap();
        assert_eq!(e2.slot_versions.len(), 2, "other slots are carried forward");
    }

    #[test]
    fn successor_rejects_a_downgrade_and_accepts_a_higher_version() {
        let e1 = genesis().successor(
            EvaluatorSlot::Architecture,
            3,
            promo("promo-1"),
            Digest::of_bytes(b"policy"),
            None,
            2,
        );
        let e1 = e1.unwrap();
        assert!(e1
            .successor(
                EvaluatorSlot::Architecture,
                2,
                promo("promo-2"),
                Digest::of_bytes(b"policy"),
                None,
                3
            )
            .is_err());
        let e2 = e1
            .successor(
                EvaluatorSlot::Architecture,
                4,
                promo("promo-2"),
                Digest::of_bytes(b"policy"),
                None,
                3,
            )
            .unwrap();
        assert_eq!(e2.version_of(EvaluatorSlot::Architecture), Some(4));
        assert_eq!(e2.epoch_id, EpochId(2));
    }

    #[test]
    fn genesis_pins_nothing_and_validates() {
        let g = genesis();
        assert!(g.slot_versions.is_empty());
        assert_eq!(g.version_of(EvaluatorSlot::Architecture), None);
        assert_eq!(g.predecessor, None);
        assert_eq!(g.promotion, None);
        assert_eq!(g.started_at, 1);
        assert!(g.validate().is_ok());
    }

    #[test]
    fn successor_pins_the_suite_it_was_given() {
        let suite = AnchorSuiteId::parse("architecture-fixture").unwrap();
        let digest = Digest::of_bytes(b"suite");
        let e = genesis()
            .successor(
                EvaluatorSlot::Architecture,
                1,
                promo("promo-1"),
                Digest::of_bytes(b"policy"),
                Some((suite.clone(), digest.clone())),
                7,
            )
            .unwrap();
        assert_eq!(e.anchor_suite_digests.get(&suite), Some(&digest));
        assert_eq!(e.started_at, 7);
        assert_eq!(e.promotion, Some(promo("promo-1")));
        assert!(e.validate().is_ok(), "{:?}", e.validate());
    }

    #[test]
    fn validate_rejects_a_backwards_or_missing_predecessor() {
        let mut e = genesis()
            .successor(
                EvaluatorSlot::Architecture,
                1,
                promo("promo-1"),
                Digest::of_bytes(b"policy"),
                None,
                2,
            )
            .unwrap();
        e.predecessor = Some(EpochId(3));
        assert!(e.validate().is_err(), "predecessor must be older");
        e.predecessor = None;
        assert!(e.validate().is_err(), "only genesis may lack a predecessor");
    }

    #[test]
    fn epoch_round_trips_through_json() {
        let e = genesis()
            .successor(
                EvaluatorSlot::Architecture,
                1,
                promo("promo-1"),
                Digest::of_bytes(b"policy"),
                None,
                2,
            )
            .unwrap();
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("\"epoch-000001\""), "{json}");
        assert!(json.contains("\"architecture\""), "{json}");
        assert_eq!(serde_json::from_str::<EvaluatorEpoch>(&json).unwrap(), e);
    }
}
