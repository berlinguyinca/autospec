//! Promotion seeding: [`EvaluationStore::pin`] (Task 11 of the evaluator
//! coevolution slice).
//!
//! A pin seeds a *registered* evaluator version into an *empty* slot as a
//! committed, human-approved promotion: it writes the promotion event, the
//! successor epoch, and the new `current.json` pointer, then journals
//! `evaluator.pinned`. Changing an *already-pinned* slot is a promotion
//! ([`crate::evaluation::promotion::plan_promotion`]), not a pin.

use std::collections::BTreeMap;

use crate::evaluation::ids::EvaluatorVersionRef;
use crate::evaluation::promotion::{
    plan_pin, Approval, ApprovalKind, PromotionEvent, PromotionState,
};
use crate::evaluation::EVALUATION_SCHEMA_VERSION;

use super::io;
use super::{
    atomic_write_json, CurrentPointer, EvaluationError, EvaluationErrorKind, EvaluationStore,
    Result,
};

impl EvaluationStore {
    /// Seed a registered evaluator version into an empty slot as a committed,
    /// human-approved pin.
    ///
    /// Writes the promotion event, the successor epoch, and the new
    /// `current.json` pointer, then journals `evaluator.pinned`. Fails when the
    /// evaluator is not registered, `actor` is empty, or the slot is already
    /// pinned (that is a promotion, not a pin).
    pub fn pin(
        &mut self,
        reference: EvaluatorVersionRef,
        actor: &str,
        at: u64,
    ) -> Result<PromotionEvent> {
        self.evaluator(reference)?;
        if actor.trim().is_empty() {
            return Err(EvaluationError::new(
                EvaluationErrorKind::Invariant,
                "pin requires a non-empty actor",
            ));
        }
        let current = self.current_epoch()?;
        let mut event = plan_pin(&current, &reference, &self.policy, at)?;
        event.approvals.push(Approval {
            kind: ApprovalKind::Human,
            slot: reference.slot,
            version: reference.version,
            by: actor.to_string(),
            at,
            trial: None,
        });
        event.state = PromotionState::Committed;

        io::write_immutable_json(&self.layout.promotion_file(event.id.as_str()), &event)?;
        io::write_immutable_json(
            &self.layout.epoch_file(event.epoch.epoch_id.0),
            &event.epoch,
        )?;
        let pointer = CurrentPointer {
            schema: EVALUATION_SCHEMA_VERSION,
            epoch_id: event.epoch.epoch_id,
        };
        atomic_write_json(&self.layout.current_file(), &pointer)?;

        let mut fields = BTreeMap::new();
        fields.insert("evaluator".into(), reference.to_string());
        fields.insert("epoch".into(), event.epoch.epoch_id.to_string());
        fields.insert("actor".into(), actor.to_string());
        self.journal.append(
            at,
            "evaluator.pinned",
            &format!("promotion:{}", event.id),
            fields,
        )?;
        Ok(event)
    }
}
