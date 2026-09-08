//! Strong ids for the evaluation subsystem (spec §Data model).

use std::fmt;

use serde::{Deserialize, Serialize};

use super::error::EvaluationError;

/// The nine evaluator slots. Fixed for slice 1; a new slot is a spec change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
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
        match self {
            Self::SpecCompliance => "spec_compliance",
            Self::Architecture => "architecture",
            Self::Maintainability => "maintainability",
            Self::ComplexityDesign => "complexity_design",
            Self::TestQuality => "test_quality",
            Self::SecurityReasoning => "security_reasoning",
            Self::Documentation => "documentation",
            Self::UiUx => "ui_ux",
            Self::Operability => "operability",
        }
    }

    pub fn parse(value: &str) -> Result<Self, EvaluationError> {
        Self::ALL
            .iter()
            .copied()
            .find(|s| s.as_str() == value)
            .ok_or_else(|| EvaluationError::parse(format!("unknown evaluator slot {value:?}")))
    }
}

impl fmt::Display for EvaluatorSlot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A pinned evaluator version, serialized `"<slot>@<n>"` with `n >= 1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct EvaluatorVersionRef {
    pub slot: EvaluatorSlot,
    pub version: u32,
}

impl EvaluatorVersionRef {
    pub fn parse(value: &str) -> Result<Self, EvaluationError> {
        let (slot, version) = value.split_once('@').ok_or_else(|| {
            EvaluationError::parse(format!(
                "evaluator version must be <slot>@<n>, got {value:?}"
            ))
        })?;
        let version: u32 = version.parse().map_err(|_| {
            EvaluationError::parse(format!("bad evaluator version number in {value:?}"))
        })?;
        if version == 0 {
            return Err(EvaluationError::parse("evaluator versions start at 1"));
        }
        Ok(Self {
            slot: EvaluatorSlot::parse(slot)?,
            version,
        })
    }
}

impl fmt::Display for EvaluatorVersionRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.slot, self.version)
    }
}

impl TryFrom<String> for EvaluatorVersionRef {
    type Error = EvaluationError;
    fn try_from(v: String) -> Result<Self, Self::Error> {
        Self::parse(&v)
    }
}

impl From<EvaluatorVersionRef> for String {
    fn from(v: EvaluatorVersionRef) -> String {
        v.to_string()
    }
}

/// Epoch number, serialized `epoch-NNNNNN`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct EpochId(pub u64);

impl EpochId {
    pub fn parse(value: &str) -> Result<Self, EvaluationError> {
        let digits = value
            .strip_prefix("epoch-")
            .filter(|d| d.len() == 6 && d.bytes().all(|b| b.is_ascii_digit()))
            .ok_or_else(|| {
                EvaluationError::parse(format!("epoch id must be epoch-NNNNNN, got {value:?}"))
            })?;
        Ok(Self(digits.parse().expect("six ascii digits parse")))
    }

    /// The strictly following epoch.
    pub fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

impl fmt::Display for EpochId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "epoch-{:06}", self.0)
    }
}

impl TryFrom<String> for EpochId {
    type Error = EvaluationError;
    fn try_from(v: String) -> Result<Self, Self::Error> {
        Self::parse(&v)
    }
}

impl From<EpochId> for String {
    fn from(v: EpochId) -> String {
        v.to_string()
    }
}

fn valid_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 64
        && (bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit())
        && bytes.iter().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-')
        })
        && !value.contains("..")
}

macro_rules! string_id {
    ($name:ident, $label:literal) => {
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            pub fn parse(value: &str) -> Result<Self, EvaluationError> {
                if valid_id(value) {
                    Ok(Self(value.to_string()))
                } else {
                    Err(EvaluationError::parse(format!(
                        "invalid {}: {value:?} (expected [a-z0-9][a-z0-9._-]{{0,63}})",
                        $label
                    )))
                }
            }
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl TryFrom<String> for $name {
            type Error = EvaluationError;
            fn try_from(v: String) -> Result<Self, Self::Error> {
                Self::parse(&v)
            }
        }

        impl From<$name> for String {
            fn from(v: $name) -> String {
                v.0
            }
        }
    };
}

string_id!(AnchorSuiteId, "anchor suite id");
string_id!(AnchorCaseId, "anchor case id");
string_id!(ChallengerTrialId, "challenger trial id");
string_id!(PromotionId, "promotion id");
string_id!(EvaluationId, "evaluation id");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_ref_round_trips_through_display_and_parse() {
        let v = EvaluatorVersionRef {
            slot: EvaluatorSlot::Architecture,
            version: 12,
        };
        assert_eq!(v.to_string(), "architecture@12");
        assert_eq!(EvaluatorVersionRef::parse("architecture@12").unwrap(), v);
        assert!(EvaluatorVersionRef::parse("architecture@0").is_err());
        assert!(EvaluatorVersionRef::parse("judge@1").is_err());
    }

    #[test]
    fn epoch_id_formats_six_digits() {
        assert_eq!(EpochId(1).to_string(), "epoch-000001");
        assert_eq!(EpochId::parse("epoch-000042").unwrap(), EpochId(42));
        assert!(EpochId::parse("epoch-42").is_err());
    }

    #[test]
    fn string_ids_reject_unsafe_characters() {
        assert!(AnchorCaseId::parse("case-01").is_ok());
        assert!(AnchorCaseId::parse("../case").is_err());
        assert!(AnchorCaseId::parse("Case").is_err());
        assert!(AnchorCaseId::parse("").is_err());
    }

    #[test]
    fn serde_uses_string_form() {
        let json = serde_json::to_string(&EvaluatorVersionRef {
            slot: EvaluatorSlot::Documentation,
            version: 3,
        })
        .unwrap();
        assert_eq!(json, "\"documentation@3\"");
        let json = serde_json::to_string(&EpochId(7)).unwrap();
        assert_eq!(json, "\"epoch-000007\"");
    }
}
