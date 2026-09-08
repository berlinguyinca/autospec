//! Evaluator store types: slots, definitions, promotion policy, epochs,
//! promotion events, and the typed error. Self-contained for this CLI slice;
//! the layout and digests mirror the evaluator-coevolution design spec.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use autospec_core::autonomous::waterfall::sha256_hex;

/// Schema version accepted by the evaluator store reader.
pub const EVALUATOR_STORE_SCHEMA: u64 = 1;

/// Typed error for evaluator store operations. Renders as `<kind>: <message>`.
#[derive(Debug)]
pub struct EvaluationError {
    pub kind: EvaluationErrorKind,
    pub message: String,
}

/// Error classes, rendered with the design's `kind:` prefix (exit code 2).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EvaluationErrorKind {
    Io,
    Invariant,
    Integrity,
    Parse,
    Immutable,
    FailClosed,
}

impl EvaluationErrorKind {
    fn label(self) -> &'static str {
        match self {
            Self::Io => "io",
            Self::Invariant => "invariant",
            Self::Integrity => "integrity",
            Self::Parse => "parse",
            Self::Immutable => "immutable",
            Self::FailClosed => "fail-closed",
        }
    }
}

impl EvaluationError {
    pub fn new(kind: EvaluationErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl fmt::Display for EvaluationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.kind.label(), self.message)
    }
}

impl std::error::Error for EvaluationError {}

/// The nine evaluator slots. Serialized as their snake_case wire names.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvaluatorSlot {
    SpecCompliance,
    Architecture,
    Maintainability,
    ComplexityDesign,
    TestQuality,
    SecurityReasoning,
    Documentation,
    UiUx,
    Operability,
}

impl EvaluatorSlot {
    /// Wire names, indexed by variant (mirrors the serde snake_case names).
    const WIRE: [&'static str; 9] = [
        "spec_compliance",
        "architecture",
        "maintainability",
        "complexity_design",
        "test_quality",
        "security_reasoning",
        "documentation",
        "ui_ux",
        "operability",
    ];
    pub const ALL: [EvaluatorSlot; 9] = [
        Self::SpecCompliance,
        Self::Architecture,
        Self::Maintainability,
        Self::ComplexityDesign,
        Self::TestQuality,
        Self::SecurityReasoning,
        Self::Documentation,
        Self::UiUx,
        Self::Operability,
    ];

    pub fn as_str(self) -> &'static str {
        Self::WIRE[self as usize]
    }

    pub fn parse(text: &str) -> Result<Self, EvaluationError> {
        Self::ALL
            .iter()
            .copied()
            .find(|slot| slot.as_str() == text)
            .ok_or_else(|| {
                EvaluationError::new(
                    EvaluationErrorKind::Parse,
                    format!("unknown evaluator slot: {text}"),
                )
            })
    }
}

impl fmt::Display for EvaluatorSlot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Evaluator implementation kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EvaluatorKind {
    Learned,
    Deterministic,
}

impl EvaluatorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Learned => "learned",
            Self::Deterministic => "deterministic",
        }
    }
}

impl fmt::Display for EvaluatorKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// One registered evaluator: `slot@version`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct EvaluatorVersionRef {
    pub slot: EvaluatorSlot,
    pub version: u32,
}

impl EvaluatorVersionRef {
    pub fn new(slot: EvaluatorSlot, version: u32) -> Self {
        Self { slot, version }
    }
}

impl FromStr for EvaluatorVersionRef {
    type Err = EvaluationError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let (slot_text, version_text) = text.split_once('@').ok_or_else(|| {
            EvaluationError::new(
                EvaluationErrorKind::Parse,
                format!("expected slot@version, got: {text}"),
            )
        })?;
        let slot = EvaluatorSlot::parse(slot_text)?;
        let version: u32 = version_text.parse().map_err(|_| {
            EvaluationError::new(
                EvaluationErrorKind::Parse,
                format!("expected a version number after @, got: {text}"),
            )
        })?;
        if version < 1 {
            return Err(EvaluationError::new(
                EvaluationErrorKind::Parse,
                format!("evaluator version must be >= 1, got: {text}"),
            ));
        }
        Ok(Self { slot, version })
    }
}

impl fmt::Display for EvaluatorVersionRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}@{}", self.slot, self.version)
    }
}

/// A registered evaluator definition (immutable once written).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EvaluatorDefinition {
    pub schema: u64,
    pub slot: EvaluatorSlot,
    pub version: u32,
    pub kind: EvaluatorKind,
    pub rubric_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_pack_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knowledge_base_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_settings_digest: Option<String>,
    pub routing_policy_digest: String,
    pub tool_policy_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_family: Option<String>,
    pub created_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_version: Option<u32>,
    /// Opaque runtime provenance blob; excluded from the definition digest.
    #[serde(default)]
    pub provenance: serde_json::Value,
}

impl EvaluatorDefinition {
    /// Structural validation. A digest is 64 lowercase hex characters.
    pub fn validate(&self) -> Result<(), EvaluationError> {
        if self.schema != EVALUATOR_STORE_SCHEMA {
            return Err(EvaluationError::new(
                EvaluationErrorKind::Invariant,
                format!("unsupported evaluator schema: {}", self.schema),
            ));
        }
        if self.version < 1 {
            return Err(EvaluationError::new(
                EvaluationErrorKind::Invariant,
                format!("evaluator version must be >= 1, got: {}", self.version),
            ));
        }
        if self.kind == EvaluatorKind::Learned && self.prompt_digest.is_none() {
            return Err(EvaluationError::new(
                EvaluationErrorKind::Invariant,
                "a learned evaluator requires a prompt_digest",
            ));
        }
        if let Some(parent) = self.parent_version {
            if parent >= self.version {
                return Err(EvaluationError::new(
                    EvaluationErrorKind::Invariant,
                    format!("parent_version {parent} must be < version {}", self.version),
                ));
            }
        }
        Self::validate_digest("routing_policy_digest", &self.routing_policy_digest)?;
        Self::validate_digest("tool_policy_digest", &self.tool_policy_digest)?;
        for (name, digest) in [
            ("prompt_digest", self.prompt_digest.as_deref()),
            ("skill_pack_digest", self.skill_pack_digest.as_deref()),
            (
                "knowledge_base_digest",
                self.knowledge_base_digest.as_deref(),
            ),
            (
                "runtime_settings_digest",
                self.runtime_settings_digest.as_deref(),
            ),
        ] {
            if let Some(digest) = digest {
                Self::validate_digest(name, digest)?;
            }
        }
        Ok(())
    }

    fn validate_digest(name: &str, digest: &str) -> Result<(), EvaluationError> {
        let is_hex = digest
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'));
        if digest.len() != 64 || !is_hex {
            return Err(EvaluationError::new(
                EvaluationErrorKind::Parse,
                format!("{name} must be 64 lowercase hex characters"),
            ));
        }
        Ok(())
    }

    /// Behavior-affecting content digest: NUL-joined fields, SHA-256.
    /// `created_at` and `provenance` are deliberately excluded.
    pub fn definition_digest(&self) -> String {
        let parent = self
            .parent_version
            .map(|version| version.to_string())
            .unwrap_or_default();
        let parts = [
            self.slot.as_str(),
            &self.version.to_string(),
            self.kind.as_str(),
            &self.rubric_ref,
            self.prompt_digest.as_deref().unwrap_or("-"),
            self.skill_pack_digest.as_deref().unwrap_or("-"),
            self.knowledge_base_digest.as_deref().unwrap_or("-"),
            self.runtime_settings_digest.as_deref().unwrap_or("-"),
            &self.routing_policy_digest,
            &self.tool_policy_digest,
            self.model_family.as_deref().unwrap_or("-"),
            parent.as_str(),
        ];
        sha256_hex(parts.join("\0").as_bytes())
    }
}

/// Promotion policy. Integer ppm values only; no floats anywhere.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PromotionPolicy {
    pub schema: u64,
    /// Minimum win epsilon, in parts per million (1_000_000 == 1.0).
    pub epsilon: u32,
    /// Minimum margin, in parts per million.
    pub minimum_margin: u32,
    /// Minimum cases required before a promotion decision.
    pub minimum_cases: usize,
    pub require_human_approval_slots: Vec<EvaluatorSlot>,
}

const PPM_SCALE: u32 = 1_000_000;

impl Default for PromotionPolicy {
    fn default() -> Self {
        Self {
            schema: EVALUATOR_STORE_SCHEMA,
            epsilon: 50_000,
            minimum_margin: 10_000,
            minimum_cases: 40,
            require_human_approval_slots: vec![
                EvaluatorSlot::Architecture,
                EvaluatorSlot::SecurityReasoning,
            ],
        }
    }
}

impl PromotionPolicy {
    pub fn validate(&self) -> Result<(), EvaluationError> {
        if self.schema != EVALUATOR_STORE_SCHEMA {
            return Err(EvaluationError::new(
                EvaluationErrorKind::Invariant,
                format!("unsupported policy schema: {}", self.schema),
            ));
        }
        if self.epsilon > PPM_SCALE || self.minimum_margin > PPM_SCALE {
            return Err(EvaluationError::new(
                EvaluationErrorKind::Invariant,
                "epsilon and minimum_margin are ppm values and must be <= 1000000",
            ));
        }
        Ok(())
    }

    /// Canonical policy digest: NUL-joined fields with sorted slots.
    pub fn policy_digest(&self) -> String {
        let mut slots = self.require_human_approval_slots.clone();
        slots.sort_unstable();
        let parts = [
            "policy",
            &self.schema.to_string(),
            &self.epsilon.to_string(),
            &self.minimum_margin.to_string(),
            &self.minimum_cases.to_string(),
            &slots
                .iter()
                .map(|slot| slot.as_str())
                .collect::<Vec<_>>()
                .join(","),
        ];
        sha256_hex(parts.join("\0").as_bytes())
    }
}

/// Epoch identifier, serialized as the canonical `epoch-NNNNNN` string.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct EpochId(pub u64);

impl EpochId {
    pub fn genesis() -> Self {
        Self(0)
    }
    pub fn next(self) -> Self {
        Self(self.0 + 1)
    }

    pub fn parse(text: &str) -> Result<Self, EvaluationError> {
        let digits = text.strip_prefix("epoch-").ok_or_else(|| {
            EvaluationError::new(
                EvaluationErrorKind::Parse,
                format!("expected epoch-NNNNNN, got: {text}"),
            )
        })?;
        let value: u64 = digits.parse().map_err(|_| {
            EvaluationError::new(
                EvaluationErrorKind::Parse,
                format!("expected epoch-NNNNNN, got: {text}"),
            )
        })?;
        if format!("epoch-{value:06}") != text {
            return Err(EvaluationError::new(
                EvaluationErrorKind::Parse,
                format!("expected canonical epoch-NNNNNN, got: {text}"),
            ));
        }
        Ok(Self(value))
    }
}

impl fmt::Display for EpochId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "epoch-{:06}", self.0)
    }
}

impl Serialize for EpochId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for EpochId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).map_err(D::Error::custom)
    }
}

/// One active epoch: the exact slot->version set evaluators ran under.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EvaluatorEpoch {
    pub schema: u64,
    pub epoch_id: EpochId,
    pub slot_versions: BTreeMap<EvaluatorSlot, u32>,
    pub started_at: u64,
    pub predecessor: Option<EpochId>,
    pub promotion: Option<String>,
    pub policy_digest: String,
    pub anchor_suite_digests: BTreeMap<String, String>,
}

impl EvaluatorEpoch {
    pub fn check(&self) -> Result<(), EvaluationError> {
        if self.schema != EVALUATOR_STORE_SCHEMA {
            return Err(EvaluationError::new(
                EvaluationErrorKind::Invariant,
                format!("unsupported epoch schema: {}", self.schema),
            ));
        }
        if self.epoch_id.0 > 0 {
            match (self.predecessor, self.promotion.as_ref()) {
                (Some(predecessor), Some(_promotion)) if predecessor.next() == self.epoch_id => {}
                _ => {
                    return Err(EvaluationError::new(
                        EvaluationErrorKind::Integrity,
                        format!(
                            "epoch {} must carry its predecessor and promotion id",
                            self.epoch_id
                        ),
                    ));
                }
            }
        }
        Ok(())
    }
}

/// `current.json`: the single atomic pointer to the active epoch.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CurrentPointer {
    pub schema: u64,
    pub epoch_id: EpochId,
}

/// Approval record for a promotion or pin event.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Approval {
    pub kind: ApprovalKind,
    pub actor: String,
    pub at: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalKind {
    Human,
    Policy,
}

/// A committed promotion event. `pin` writes `from: None` (seed) and no
/// challenger trial; `promote` (a later slice) writes `from: Some(version)`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PromotionEvent {
    pub schema: u64,
    pub promotion_id: String,
    pub state: String,
    pub slot: EvaluatorSlot,
    pub from: Option<u32>,
    pub to: u32,
    pub challenger_trial: Option<String>,
    pub prior_epoch: EpochId,
    pub new_epoch: EpochId,
    pub invalidated_evaluations: Vec<String>,
    pub approval: Approval,
    pub created_at: u64,
    pub committed_at: Option<u64>,
}
