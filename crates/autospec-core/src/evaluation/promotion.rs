//! Promotion planning: the only path by which a new evaluator version becomes
//! visible to ranking (spec §Promotion, plan Task 8).
//!
//! A promotion is *planned*, not applied: [`plan_promotion`] and [`plan_pin`]
//! produce an immutable [`PromotionEvent`] carrying the successor epoch
//! ([`EvaluatorEpoch::successor`]) and the evidence that justified it. The
//! plan fails closed on every unmet precondition — a non-qualified trial
//! verdict, a stale incumbent pin, policy-digest drift between trial time and
//! now, a challenger that is not strictly newer, and a missing human approval
//! for a policy-gated slot. There is no partial success and no default
//! approval: absence of evidence is a refusal, never an acceptance.

use serde::{Deserialize, Serialize};

use super::digest::Digest;
use super::epoch::EvaluatorEpoch;
use super::error::EvaluationError;
use super::ids::{
    ChallengerTrialId, EpochId, EvaluationId, EvaluatorSlot, EvaluatorVersionRef, PromotionId,
};
use super::policy::PromotionPolicy;

/// Who (or what) signed off on a promotion.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalKind {
    /// Granted by policy (thresholds met); recorded for the audit trail.
    Policy,
    /// Granted by a person; mandatory for policy-gated slots.
    Human,
}

/// One explicit sign-off for one evaluator version.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Approval {
    pub kind: ApprovalKind,
    pub slot: EvaluatorSlot,
    pub version: u32,
    /// Who granted it (person or policy name).
    pub by: String,
    /// Unix seconds.
    pub at: u64,
    /// The trial this approval refers to; `None` for policy grants and pins.
    pub trial: Option<ChallengerTrialId>,
}

/// What a challenger trial concluded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChallengerVerdict {
    /// The challenger beat the incumbent over the protected anchor suite.
    Qualified,
    /// The evidence did not settle the question; nothing may change.
    Inconclusive,
    /// The challenger lost.
    Rejected,
}

impl ChallengerVerdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Qualified => "qualified",
            Self::Inconclusive => "inconclusive",
            Self::Rejected => "rejected",
        }
    }
}

/// One paired trial: incumbent vs challenger over a protected anchor suite,
/// run under the policy whose digest is recorded here so later drift is
/// detectable.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChallengerTrial {
    pub id: ChallengerTrialId,
    pub incumbent: EvaluatorVersionRef,
    pub challenger: EvaluatorVersionRef,
    /// Digest of the policy under which the trial ran.
    pub policy_digest: Digest,
    pub verdict: ChallengerVerdict,
    /// Digest of the trial's full report; the event carries the digest, not
    /// the report, so the event stays small and content-addressed.
    pub report: Digest,
}

/// Whether a planned promotion has been committed (persisted + epoch
/// activated) yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromotionState {
    /// Planned; nothing has changed yet.
    Pending,
    /// Persisted and the successor epoch is active.
    Committed,
}

/// The immutable description of one promotion: what changes, why it was
/// allowed, and the successor epoch that will result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromotionEvent {
    pub id: PromotionId,
    /// The successor epoch this event will activate; `from`/`to` describe the
    /// slot pin it changes.
    pub epoch: EvaluatorEpoch,
    /// The version being displaced; `None` when pinning an empty slot.
    pub from: Option<EvaluatorVersionRef>,
    pub to: EvaluatorVersionRef,
    /// The qualifying trial; `None` for a pin (a first introduction has no
    /// incumbent to beat).
    pub trial: Option<ChallengerTrialId>,
    pub approvals: Vec<Approval>,
    pub state: PromotionState,
    /// Records whose evaluator version is displaced by this event. Supplied
    /// by the caller (the displaced `slot@version`'s `Active` records) and
    /// stored verbatim on the event; a pin carries an empty list.
    pub invalidated_evaluations: Vec<EvaluationId>,
}

/// Derive the deterministic [`PromotionId`] for a planned event:
/// `promo-` plus the first 16 hex characters of the digest of
/// `(basis, previous epoch, successor epoch)`. The same plan always yields
/// the same id, which is what makes planning idempotent.
fn derive_promotion_id(basis: &str, previous: &EpochId, successor: &EpochId) -> PromotionId {
    let previous = previous.to_string();
    let successor = successor.to_string();
    let digest = Digest::of_parts(&[basis.as_bytes(), previous.as_bytes(), successor.as_bytes()]);
    PromotionId::parse(&format!("promo-{}", &digest.as_str()[..16]))
        .expect("promo-<16 lowercase hex> is a valid promotion id")
}

/// Plan a promotion from a qualified challenger trial.
///
/// Fails closed (no event, no mutation) when:
///
/// - the trial verdict is not [`ChallengerVerdict::Qualified`];
/// - the trial's incumbent is not the version the current epoch pins for the
///   slot (the pin moved under the trial — a stale trial);
/// - the trial and the policy disagree on the slot, or on the policy digest
///   (policy changed since the trial ran);
/// - the challenger version is not strictly greater than the pinned version;
/// - the slot requires human approval and no human approval exists for the
///   exact challenger version from this trial.
///
/// `invalidated` is the caller's list of records this promotion displaces;
/// it is stored verbatim on the event (the caller computes it from the
/// displaced `slot@version`'s `Active` records).
///
/// On success the event's `from` is the epoch's pinned version, its `to` the
/// challenger, and its `epoch` the successor built via
/// [`EvaluatorEpoch::successor`] (which independently re-checks the version
/// ordering).
pub fn plan_promotion(
    epoch: &EvaluatorEpoch,
    trial: &ChallengerTrial,
    policy: &PromotionPolicy,
    invalidated: &[EvaluationId],
    approvals: &[Approval],
    now: u64,
) -> Result<PromotionEvent, EvaluationError> {
    if trial.verdict != ChallengerVerdict::Qualified {
        return Err(EvaluationError::fail_closed(format!(
            "trial {} verdict is {}, not qualified",
            trial.id,
            trial.verdict.as_str()
        )));
    }
    if trial.incumbent.slot != trial.challenger.slot {
        return Err(EvaluationError::invariant(format!(
            "trial {} spans two slots ({} vs {}); a trial is within one slot",
            trial.id, trial.incumbent.slot, trial.challenger.slot
        )));
    }
    let slot = trial.challenger.slot;
    let pinned = match epoch.version_of(slot) {
        Some(v) => v,
        None => {
            return Err(EvaluationError::fail_closed(format!(
                "epoch {} pins no version for {slot}; a promotion needs an incumbent, use plan_pin",
                epoch.epoch_id
            )))
        }
    };
    if trial.incumbent.version != pinned {
        return Err(EvaluationError::fail_closed(format!(
            "trial {} judged incumbent {slot}@{}, but epoch {} pins {slot}@{pinned}; the trial is stale",
            trial.id, trial.incumbent.version, epoch.epoch_id
        )));
    }
    if trial.policy_digest != policy.policy_digest() {
        return Err(EvaluationError::fail_closed(format!(
            "trial {} ran under policy {}, current policy is {}; rerun the trial",
            trial.id,
            trial.policy_digest,
            policy.policy_digest()
        )));
    }
    if trial.challenger.version <= pinned {
        return Err(EvaluationError::fail_closed(format!(
            "challenger {slot}@{} is not newer than the pinned {slot}@{pinned}",
            trial.challenger.version,
        )));
    }
    if policy.require_human_approval_slots.contains(&slot) {
        let approved = approvals.iter().any(|a| {
            a.kind == ApprovalKind::Human
                && a.slot == slot
                && a.version == trial.challenger.version
                && a.trial == Some(trial.id.clone())
        });
        if !approved {
            return Err(EvaluationError::fail_closed(format!(
                "slot {slot} requires human approval for {slot}@{} from trial {}, none given",
                trial.challenger.version, trial.id
            )));
        }
    }

    // The successor's id is fixed by the epoch chain, so the promotion id can
    // be derived before the successor is built and passed to it directly.
    let id = derive_promotion_id(
        &trial.id.to_string(),
        &epoch.epoch_id,
        &epoch.epoch_id.next(),
    );
    let successor = epoch.successor(
        slot,
        trial.challenger.version,
        id.clone(),
        policy.policy_digest(),
        None,
        now,
    )?;

    Ok(PromotionEvent {
        id,
        epoch: successor,
        from: Some(trial.incumbent),
        to: trial.challenger,
        trial: Some(trial.id.clone()),
        approvals: approvals.to_vec(),
        state: PromotionState::Pending,
        invalidated_evaluations: invalidated.to_vec(),
    })
}

/// Plan a pin: the first introduction of an evaluator version into an empty
/// slot. A pin has no incumbent (`from: None`) and no trial; it is still a
/// promotion event, with its own id and its own successor epoch.
///
/// Fails closed when the slot is already pinned (that is a promotion, not a
/// pin) or when `evaluator` names a different slot than the epoch allows.
pub fn plan_pin(
    epoch: &EvaluatorEpoch,
    evaluator: &EvaluatorVersionRef,
    policy: &PromotionPolicy,
    now: u64,
) -> Result<PromotionEvent, EvaluationError> {
    let slot = evaluator.slot;
    if epoch.version_of(slot).is_some() {
        return Err(EvaluationError::invariant(format!(
            "epoch {} already pins {slot}; plan_promotion, not plan_pin, changes a pinned slot",
            epoch.epoch_id
        )));
    }
    let id = derive_promotion_id(
        &format!("pin:{evaluator}"),
        &epoch.epoch_id,
        &epoch.epoch_id.next(),
    );
    let successor = epoch.successor(
        slot,
        evaluator.version,
        id.clone(),
        policy.policy_digest(),
        None,
        now,
    )?;

    Ok(PromotionEvent {
        id,
        epoch: successor,
        from: None,
        to: *evaluator,
        trial: None,
        approvals: Vec::new(),
        state: PromotionState::Pending,
        invalidated_evaluations: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::evaluation::error::EvaluationErrorKind;
    use crate::evaluation::ids::EvaluatorSlot::Architecture;

    fn policy() -> PromotionPolicy {
        PromotionPolicy {
            require_human_approval_slots: BTreeSet::from([Architecture]),
            ..PromotionPolicy::default()
        }
    }

    fn trial_id(tag: &str) -> ChallengerTrialId {
        ChallengerTrialId::parse(tag).unwrap()
    }

    fn digest(tag: &str) -> Digest {
        Digest::of_bytes(tag.as_bytes())
    }

    /// An epoch that pins architecture@1 under `policy`'s digest.
    fn epoch_with_pin(policy: &PromotionPolicy) -> EvaluatorEpoch {
        let genesis = EvaluatorEpoch::genesis(policy.policy_digest(), 1);
        genesis
            .successor(
                Architecture,
                1,
                PromotionId::parse("promo-0000000000000001").unwrap(),
                policy.policy_digest(),
                None,
                2,
            )
            .unwrap()
    }

    fn qualified_trial(epoch: &EvaluatorEpoch, policy: &PromotionPolicy) -> ChallengerTrial {
        let slot_version = epoch.version_of(Architecture).unwrap();
        ChallengerTrial {
            id: trial_id("trial-1"),
            incumbent: EvaluatorVersionRef {
                slot: Architecture,
                version: slot_version,
            },
            challenger: EvaluatorVersionRef {
                slot: Architecture,
                version: slot_version + 1,
            },
            policy_digest: policy.policy_digest(),
            verdict: ChallengerVerdict::Qualified,
            report: digest("report"),
        }
    }

    fn human_approval(trial: &ChallengerTrial) -> Approval {
        Approval {
            kind: ApprovalKind::Human,
            slot: Architecture,
            version: trial.challenger.version,
            by: "ada@example.com".into(),
            at: 10,
            trial: Some(trial.id.clone()),
        }
    }

    #[test]
    fn promotion_fails_closed_when_trial_is_not_qualified() {
        let policy = policy();
        let epoch = epoch_with_pin(&policy);
        for verdict in [ChallengerVerdict::Inconclusive, ChallengerVerdict::Rejected] {
            let mut trial = qualified_trial(&epoch, &policy);
            trial.verdict = verdict;
            let err = plan_promotion(&epoch, &trial, &policy, &[], &[human_approval(&trial)], 3)
                .unwrap_err();
            assert_eq!(err.kind, EvaluationErrorKind::FailClosed);
            assert!(err.to_string().contains(verdict.as_str()), "{}", err);
        }
    }

    #[test]
    fn promotion_fails_closed_when_incumbent_version_is_stale() {
        let policy = policy();
        let epoch = epoch_with_pin(&policy);
        let mut trial = qualified_trial(&epoch, &policy);
        trial.incumbent.version = 99; // epoch pins 1
        let err =
            plan_promotion(&epoch, &trial, &policy, &[], &[human_approval(&trial)], 3).unwrap_err();
        assert_eq!(err.kind, EvaluationErrorKind::FailClosed);
        assert!(err.to_string().contains("stale"), "{}", err);

        // And when the slot is unpinned at all: that is a pin, not a promotion.
        let genesis = EvaluatorEpoch::genesis(policy.policy_digest(), 1);
        let err = plan_promotion(&genesis, &trial, &policy, &[], &[], 3).unwrap_err();
        assert_eq!(err.kind, EvaluationErrorKind::FailClosed);
    }

    #[test]
    fn promotion_fails_closed_on_policy_digest_drift() {
        let policy = policy();
        let epoch = epoch_with_pin(&policy);
        let trial = qualified_trial(&epoch, &policy);
        let drifted = PromotionPolicy {
            require_human_approval_slots: BTreeSet::new(),
            ..PromotionPolicy::default()
        };
        let err = plan_promotion(&epoch, &trial, &drifted, &[], &[], 3).unwrap_err();
        assert_eq!(err.kind, EvaluationErrorKind::FailClosed);
        assert!(err.to_string().contains("policy"), "{}", err);
    }

    #[test]
    fn promotion_fails_closed_without_human_approval_on_gated_slot() {
        let policy = policy();
        let epoch = epoch_with_pin(&policy);
        let trial = qualified_trial(&epoch, &policy);
        // No approvals at all.
        assert!(plan_promotion(&epoch, &trial, &policy, &[], &[], 3).is_err());
        // A policy-kind approval does not count.
        let policy_approval = Approval {
            kind: ApprovalKind::Policy,
            ..human_approval(&trial)
        };
        assert!(plan_promotion(&epoch, &trial, &policy, &[], &[policy_approval], 3).is_err());
        // A human approval for the wrong version does not count.
        let wrong_version = Approval {
            version: trial.challenger.version + 1,
            ..human_approval(&trial)
        };
        assert!(plan_promotion(&epoch, &trial, &policy, &[], &[wrong_version], 3).is_err());
        // A human approval tied to a different trial does not count.
        let other_trial = Approval {
            trial: Some(trial_id("trial-other")),
            ..human_approval(&trial)
        };
        assert!(plan_promotion(&epoch, &trial, &policy, &[], &[other_trial], 3).is_err());
    }

    #[test]
    fn promotion_fails_closed_when_challenger_is_not_newer() {
        let policy = policy();
        let epoch = epoch_with_pin(&policy);
        let mut trial = qualified_trial(&epoch, &policy);
        trial.challenger.version = trial.incumbent.version; // same version
        let err =
            plan_promotion(&epoch, &trial, &policy, &[], &[human_approval(&trial)], 3).unwrap_err();
        assert_eq!(err.kind, EvaluationErrorKind::FailClosed);
        assert!(err.to_string().contains("not newer"), "{}", err);
    }

    #[test]
    fn pin_seeds_an_empty_slot_with_from_none() {
        let policy = policy();
        let genesis = EvaluatorEpoch::genesis(policy.policy_digest(), 1);
        let evaluator = EvaluatorVersionRef {
            slot: Architecture,
            version: 1,
        };
        let event = plan_pin(&genesis, &evaluator, &policy, 5).unwrap();
        assert!(event.from.is_none(), "a pin has no incumbent");
        assert_eq!(event.to, evaluator);
        assert!(event.trial.is_none(), "a pin has no trial");
        assert!(event.approvals.is_empty());
        assert!(event.invalidated_evaluations.is_empty());
        assert_eq!(event.state, PromotionState::Pending);
        assert_eq!(event.epoch.version_of(Architecture), Some(1));
        assert_eq!(event.epoch.epoch_id, EpochId(1));
        assert_eq!(event.epoch.predecessor, Some(EpochId(0)));
        assert_eq!(event.epoch.promotion, Some(event.id.clone()));
        assert!(event.id.as_str().starts_with("promo-"), "{}", event.id);
        assert_eq!(event.id.as_str().len(), "promo-".len() + 16);

        // Pinning an already-pinned slot is a promotion, not a pin.
        let err = plan_pin(
            &event.epoch,
            &EvaluatorVersionRef {
                slot: Architecture,
                version: 2,
            },
            &policy,
            6,
        )
        .unwrap_err();
        assert_eq!(err.kind, EvaluationErrorKind::Invariant);
    }

    #[test]
    fn plan_promotion_is_idempotent_and_uses_epoch_versions() {
        let policy = policy();
        let epoch = epoch_with_pin(&policy);
        let trial = qualified_trial(&epoch, &policy);
        let approvals = vec![human_approval(&trial)];

        let first = plan_promotion(&epoch, &trial, &policy, &[], &approvals, 3).unwrap();
        let second = plan_promotion(&epoch, &trial, &policy, &[], &approvals, 42).unwrap();
        assert_eq!(
            first.id, second.id,
            "the same plan must yield the same promotion id"
        );

        // `from` is what the epoch pins, not what the trial happened to
        // record as a label: the plan speaks the epoch's version.
        assert_eq!(
            first.from,
            Some(EvaluatorVersionRef {
                slot: Architecture,
                version: 1
            })
        );
        assert_eq!(first.to, trial.challenger);
        assert_eq!(first.trial, Some(trial.id.clone()));
        assert_eq!(
            first.epoch.version_of(Architecture),
            Some(trial.challenger.version)
        );
        assert_eq!(first.epoch.policy_digest, policy.policy_digest());
        assert_eq!(first.epoch.promotion, Some(first.id.clone()));
        first.epoch.validate().unwrap();

        // Caller-supplied `invalidated` is stored verbatim on the event; the
        // promotion id does not depend on the invalidation set.
        let invalidated = vec![
            EvaluationId::parse("evaluation-000001").unwrap(),
            EvaluationId::parse("evaluation-000002").unwrap(),
        ];
        let stamped = plan_promotion(&epoch, &trial, &policy, &invalidated, &approvals, 3).unwrap();
        assert_eq!(stamped.invalidated_evaluations, invalidated);
        assert_eq!(stamped.id, first.id);

        // A different trial basis derives a different id.
        let other = ChallengerTrial {
            id: trial_id("trial-2"),
            ..trial
        };
        let other_event =
            plan_promotion(&epoch, &other, &policy, &[], &[human_approval(&other)], 3).unwrap();
        assert_ne!(first.id, other_event.id);
    }

    #[test]
    fn promotion_event_round_trips_through_json() {
        let policy = policy();
        let epoch = epoch_with_pin(&policy);
        let trial = qualified_trial(&epoch, &policy);
        let event =
            plan_promotion(&epoch, &trial, &policy, &[], &[human_approval(&trial)], 3).unwrap();
        let json = serde_json::to_string(&event).unwrap();
        assert_eq!(
            serde_json::from_str::<PromotionEvent>(&json).unwrap(),
            event
        );
    }
}
