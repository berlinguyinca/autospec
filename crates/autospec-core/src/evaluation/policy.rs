//! The promotion policy: the conservative, digest-pinned contract that gates
//! whether a challenger evaluator may replace an incumbent. Protected-kernel
//! state: it is validated on every load and its digest travels with every
//! challenger trial and epoch.
use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::digest::Digest;
use super::error::EvaluationError;
use super::ids::EvaluatorSlot;
use super::statistics::Ppm;
use super::EVALUATION_SCHEMA_VERSION;

/// Conservative promotion defaults: ε = 50 000 ppm, margin = 10 000 ppm,
/// minimum_cases = 40 (at n=40 a difference under ~7 items is noise; the
/// margin check is on the lower bound, so the policy states its resolution
/// limit rather than hiding it), human approval for the protected slots
/// `architecture` and `security_reasoning`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromotionPolicy {
    pub schema: u64,
    pub epsilon: Ppm,
    pub minimum_margin: Ppm,
    pub minimum_cases: usize,
    #[serde(default)]
    pub require_human_approval_slots: BTreeSet<EvaluatorSlot>,
}

impl Default for PromotionPolicy {
    fn default() -> Self {
        Self {
            schema: EVALUATION_SCHEMA_VERSION,
            epsilon: Ppm(50_000),
            minimum_margin: Ppm(10_000),
            minimum_cases: 40,
            require_human_approval_slots: [
                EvaluatorSlot::Architecture,
                EvaluatorSlot::SecurityReasoning,
            ]
            .into_iter()
            .collect(),
        }
    }
}

impl PromotionPolicy {
    /// epsilon in (0, 0.5] ppm, margin <= 0.5 ppm, at least one case, and a
    /// supported schema.
    pub fn validate(&self) -> Result<(), EvaluationError> {
        if self.schema != EVALUATION_SCHEMA_VERSION {
            return Err(EvaluationError::invariant("unsupported policy schema"));
        }
        if self.epsilon.0 == 0 || self.epsilon.0 > 500_000 {
            return Err(EvaluationError::invariant("epsilon must be in (0, 0.5]"));
        }
        if self.minimum_margin.0 > 500_000 {
            return Err(EvaluationError::invariant("minimum_margin must be <= 0.5"));
        }
        if self.minimum_cases == 0 {
            return Err(EvaluationError::invariant("minimum_cases must be >= 1"));
        }
        Ok(())
    }

    /// Digest over schema, epsilon, margin, cases, and the sorted
    /// human-approval slot list. BTreeSet iteration is sorted, so the digest
    /// is canonical.
    pub fn policy_digest(&self) -> Digest {
        let slots = self
            .require_human_approval_slots
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(",");
        Digest::of_parts(&[
            self.schema.to_string().as_bytes(),
            self.epsilon.0.to_string().as_bytes(),
            self.minimum_margin.0.to_string().as_bytes(),
            self.minimum_cases.to_string().as_bytes(),
            slots.as_bytes(),
        ])
    }

    /// Parse and validate; unknown fields are rejected.
    pub fn from_json(text: &str) -> Result<Self, EvaluationError> {
        let policy: Self = serde_json::from_str(text)?;
        policy.validate()?;
        Ok(policy)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("policy serializes")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_policy_is_conservative_and_digest_stable() {
        let p = PromotionPolicy::default();
        assert_eq!(p.epsilon, Ppm(50_000));
        assert_eq!(p.minimum_margin, Ppm(10_000));
        assert_eq!(p.minimum_cases, 40);
        assert!(p
            .require_human_approval_slots
            .contains(&EvaluatorSlot::Architecture));
        assert!(p
            .require_human_approval_slots
            .contains(&EvaluatorSlot::SecurityReasoning));
        assert_eq!(
            p.policy_digest(),
            PromotionPolicy::from_json(&p.to_json())
                .unwrap()
                .policy_digest()
        );
        let mut q = p.clone();
        q.minimum_margin = Ppm(20_000);
        assert_ne!(p.policy_digest(), q.policy_digest());
    }

    #[test]
    fn validate_bounds_epsilon_and_margin() {
        let mut p = PromotionPolicy::default();
        p.epsilon = Ppm(0);
        assert!(p.validate().is_err());
        let mut p = PromotionPolicy::default();
        p.epsilon = Ppm(500_001);
        assert!(
            p.validate().is_err(),
            "epsilon above one half is not a lower bound"
        );
        let mut p = PromotionPolicy::default();
        p.minimum_cases = 0;
        assert!(p.validate().is_err());
        assert!(
            PromotionPolicy::from_json(
                r#"{"schema":1,"epsilon":50000,"minimum_margin":10000,"minimum_cases":40,"require_human_approval_slots":["architecture"],"extra":1}"#
            )
            .is_err(),
            "unknown fields are rejected"
        );
    }
}
