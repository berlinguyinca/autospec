//! Immutable evaluator definitions. Definitions pin behaviour digests, never
//! prompt contents; `definition_digest()` covers behaviour-affecting fields only.
use serde::{Deserialize, Serialize};

use super::digest::Digest;
use super::error::EvaluationError;
use super::ids::{EvaluatorSlot, EvaluatorVersionRef};
use super::EVALUATION_SCHEMA_VERSION;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvaluatorKind {
    Learned,
    Deterministic,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    pub created_by: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluatorDefinition {
    pub schema: u64,
    pub slot: EvaluatorSlot,
    pub version: u32,
    pub kind: EvaluatorKind,
    pub rubric_ref: String,
    #[serde(default)]
    pub prompt_digest: Option<Digest>,
    #[serde(default)]
    pub skill_pack_digest: Option<Digest>,
    #[serde(default)]
    pub knowledge_base_digest: Option<Digest>,
    #[serde(default)]
    pub runtime_settings_digest: Option<Digest>,
    pub routing_policy_digest: Digest,
    pub tool_policy_digest: Digest,
    #[serde(default)]
    pub model_family: Option<String>,
    pub created_at: u64,
    #[serde(default)]
    pub parent_version: Option<u32>,
    pub provenance: Provenance,
}

fn opt(d: &Option<Digest>) -> &[u8] {
    d.as_ref().map(|d| d.as_str().as_bytes()).unwrap_or(b"")
}

impl EvaluatorDefinition {
    pub fn version_ref(&self) -> EvaluatorVersionRef {
        EvaluatorVersionRef {
            slot: self.slot,
            version: self.version,
        }
    }

    /// Behaviour-affecting fields only (handoff §5.4): never timestamp or provenance.
    pub fn definition_digest(&self) -> Digest {
        let kind = match self.kind {
            EvaluatorKind::Learned => "learned",
            EvaluatorKind::Deterministic => "deterministic",
        };
        let version = self.version.to_string();
        let parent = self
            .parent_version
            .map(|v| v.to_string())
            .unwrap_or_default();
        Digest::of_parts(&[
            self.slot.as_str().as_bytes(),
            version.as_bytes(),
            kind.as_bytes(),
            self.rubric_ref.as_bytes(),
            opt(&self.prompt_digest),
            opt(&self.skill_pack_digest),
            opt(&self.knowledge_base_digest),
            opt(&self.runtime_settings_digest),
            self.routing_policy_digest.as_str().as_bytes(),
            self.tool_policy_digest.as_str().as_bytes(),
            self.model_family.as_deref().unwrap_or("").as_bytes(),
            parent.as_bytes(),
        ])
    }

    pub fn validate(&self) -> Result<(), EvaluationError> {
        if self.schema != EVALUATION_SCHEMA_VERSION {
            return Err(EvaluationError::invariant(format!(
                "unsupported evaluator schema {}",
                self.schema
            )));
        }
        if self.version == 0 {
            return Err(EvaluationError::invariant("evaluator versions start at 1"));
        }
        if matches!(self.kind, EvaluatorKind::Learned) && self.prompt_digest.is_none() {
            return Err(EvaluationError::invariant(
                "learned evaluators must pin a prompt_digest",
            ));
        }
        if self.rubric_ref.trim().is_empty() {
            return Err(EvaluationError::invariant("rubric_ref is required"));
        }
        if let Some(parent) = self.parent_version {
            if parent >= self.version {
                return Err(EvaluationError::invariant(format!(
                    "parent_version {parent} must be older than version {}",
                    self.version
                )));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::EvaluationErrorKind;
    use super::*;

    fn sample() -> EvaluatorDefinition {
        EvaluatorDefinition {
            schema: 1,
            slot: EvaluatorSlot::Architecture,
            version: 2,
            kind: EvaluatorKind::Learned,
            rubric_ref: "docs/rules/evaluator-qualification.rules.yaml#architecture".into(),
            prompt_digest: Some(Digest::of_bytes(b"prompt v2")),
            skill_pack_digest: None,
            knowledge_base_digest: None,
            runtime_settings_digest: None,
            routing_policy_digest: Digest::of_bytes(b"starter"),
            tool_policy_digest: Digest::of_bytes(b"read-only"),
            model_family: Some("anthropic".into()),
            created_at: 1_760_000_000,
            parent_version: Some(1),
            provenance: Provenance {
                created_by: "operator".into(),
                source: "manual".into(),
                notes: None,
            },
        }
    }

    #[test]
    fn definition_digest_ignores_timestamp_and_provenance_but_not_prompt() {
        let a = sample();
        let mut b = sample();
        b.created_at += 1;
        b.provenance.created_by = "someone".into();
        assert_eq!(a.definition_digest(), b.definition_digest());
        let mut c = sample();
        c.prompt_digest = Some(Digest::of_bytes(b"prompt v2 edited"));
        assert_ne!(a.definition_digest(), c.definition_digest());
    }

    #[test]
    fn learned_evaluators_require_a_prompt_digest_and_versions_start_at_one() {
        let mut d = sample();
        d.prompt_digest = None;
        assert_eq!(
            d.validate().unwrap_err().kind,
            EvaluationErrorKind::Invariant
        );
        let mut d = sample();
        d.version = 0;
        assert!(d.validate().is_err());
        let mut d = sample();
        d.parent_version = Some(2);
        assert!(
            d.validate().is_err(),
            "parent must be older than this version"
        );
        let mut d = sample();
        d.schema = 2;
        assert!(d.validate().is_err(), "future schema is rejected");
        assert!(sample().validate().is_ok());
    }

    #[test]
    fn json_round_trip_preserves_every_field() {
        let d = sample();
        let text = serde_json::to_string_pretty(&d).unwrap();
        assert_eq!(
            serde_json::from_str::<EvaluatorDefinition>(&text).unwrap(),
            d
        );
    }

    #[test]
    fn json_rejects_unknown_keys() {
        let mut d = sample();
        d.provenance.notes = Some("hand-tuned".into());
        let text = serde_json::to_string(&d).unwrap();
        let text = text.replace("{\"schema\"", "{\"bogus\":1,\"schema\"");
        let err = serde_json::from_str::<EvaluatorDefinition>(&text).unwrap_err();
        assert!(err.to_string().contains("unknown field"), "{err}");
    }
}
