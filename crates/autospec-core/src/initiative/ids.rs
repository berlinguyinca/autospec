//! Globally unique AutoSpec identifiers.
//!
//! Cross-repository work is addressed by these identifiers and never by
//! repository-local issue numbers (architectural invariant 5). Every
//! identifier is parsed once at the boundary so the rest of the
//! orchestrator can rely on the shape.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Reject an identifier body that is not a zero-padded decimal sequence.
fn parse_sequence(label: &str, raw: &str, body: &str) -> Result<u32, String> {
    if body.is_empty() || !body.chars().all(|character| character.is_ascii_digit()) {
        return Err(format!("{label} must end in a decimal sequence: {raw}"));
    }
    body.parse::<u32>()
        .map_err(|_| format!("{label} sequence is out of range: {raw}"))
}

macro_rules! sequence_id {
    ($name:ident, $prefix:literal, $label:literal) => {
        #[doc = concat!("A `", $prefix, "` identifier.")]
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name {
            text: String,
            sequence: u32,
        }

        impl $name {
            /// Parse the canonical text form, rejecting anything else.
            pub fn parse(value: impl AsRef<str>) -> Result<Self, String> {
                let raw = value.as_ref();
                let body = raw
                    .strip_prefix($prefix)
                    .ok_or_else(|| format!("{} must start with `{}`: {raw}", $label, $prefix))?;
                let sequence = parse_sequence($label, raw, body)?;
                Ok(Self {
                    text: raw.to_string(),
                    sequence,
                })
            }

            /// Build the identifier from a sequence number using `width` digits.
            pub fn from_sequence(sequence: u32, width: usize) -> Self {
                Self {
                    text: format!("{}{:0width$}", $prefix, sequence, width = width),
                    sequence,
                }
            }

            /// The canonical text form.
            pub fn as_str(&self) -> &str {
                &self.text
            }

            /// The numeric suffix, for ordering and allocation.
            pub fn sequence(&self) -> u32 {
                self.sequence
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.text)
            }
        }

        impl TryFrom<String> for $name {
            type Error = String;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::parse(value)
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.text
            }
        }
    };
}

sequence_id!(TaskId, "TASK-", "a task id");
sequence_id!(RequirementId, "REQ-", "a requirement id");
sequence_id!(CriterionId, "AC-", "an acceptance criterion id");
sequence_id!(PlanId, "PLAN-ARCH-", "an architecture plan id");
sequence_id!(GraphId, "DAG-", "a task graph id");
sequence_id!(AttemptId, "ATTEMPT-", "an attempt id");
sequence_id!(EvidenceId, "EV-", "an evidence id");

/// An Initiative identifier, `INIT-<year>-<sequence>`.
///
/// An Initiative is the top-level coordination unit and deliberately carries
/// no repository or organization in its identity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct InitiativeId {
    text: String,
    year: u32,
    sequence: u32,
}

impl InitiativeId {
    /// Parse the canonical `INIT-<year>-<sequence>` text form.
    pub fn parse(value: impl AsRef<str>) -> Result<Self, String> {
        let raw = value.as_ref();
        let body = raw
            .strip_prefix("INIT-")
            .ok_or_else(|| format!("an initiative id must start with `INIT-`: {raw}"))?;
        let (year, sequence) = body
            .split_once('-')
            .ok_or_else(|| format!("an initiative id must be INIT-<year>-<sequence>: {raw}"))?;
        if year.len() != 4 {
            return Err(format!("an initiative id needs a four digit year: {raw}"));
        }
        Ok(Self {
            year: parse_sequence("an initiative id", raw, year)?,
            sequence: parse_sequence("an initiative id", raw, sequence)?,
            text: raw.to_string(),
        })
    }

    /// The canonical text form.
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// The calendar year the Initiative was opened in.
    pub fn year(&self) -> u32 {
        self.year
    }

    /// The sequence number within that year.
    pub fn sequence(&self) -> u32 {
        self.sequence
    }

    /// The short form used inside Pi session names, e.g. `INIT-0042`.
    pub fn short(&self) -> String {
        format!("INIT-{:04}", self.sequence)
    }
}

impl fmt::Display for InitiativeId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.text)
    }
}

impl TryFrom<String> for InitiativeId {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<InitiativeId> for String {
    fn from(value: InitiativeId) -> Self {
        value.text
    }
}

/// A task-local plan identifier, `TASKPLAN-<task sequence>-v<version>`.
///
/// Task plans are regenerated against the current worktree, so the version is
/// part of the identity rather than a mutable field.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct TaskPlanId {
    text: String,
    version: u32,
}

impl TaskPlanId {
    /// Build the identifier for a task and plan version.
    pub fn new(task: &TaskId, version: u32) -> Self {
        Self {
            text: format!("TASKPLAN-{:04}-v{version}", task.sequence()),
            version,
        }
    }

    /// Parse the canonical text form.
    pub fn parse(value: impl AsRef<str>) -> Result<Self, String> {
        let raw = value.as_ref();
        let body = raw
            .strip_prefix("TASKPLAN-")
            .ok_or_else(|| format!("a task plan id must start with `TASKPLAN-`: {raw}"))?;
        let (task, version) = body
            .split_once("-v")
            .ok_or_else(|| format!("a task plan id must be TASKPLAN-<task>-v<version>: {raw}"))?;
        parse_sequence("a task plan id", raw, task)?;
        let version = parse_sequence("a task plan id", raw, version)?;
        if version == 0 {
            return Err(format!("a task plan version starts at 1: {raw}"));
        }
        Ok(Self {
            text: raw.to_string(),
            version,
        })
    }

    /// The canonical text form.
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// The plan version.
    pub fn version(&self) -> u32 {
        self.version
    }
}

impl fmt::Display for TaskPlanId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.text)
    }
}

impl TryFrom<String> for TaskPlanId {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<TaskPlanId> for String {
    fn from(value: TaskPlanId) -> Self {
        value.text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_ids_round_trip_through_their_text_form() {
        let task = TaskId::parse("TASK-0017").expect("valid task id");

        assert_eq!(task.as_str(), "TASK-0017");
        assert_eq!(task.sequence(), 17);
        assert_eq!(TaskId::from_sequence(17, 4), task);
    }

    #[test]
    fn task_ids_reject_repository_local_issue_numbers() {
        let error = TaskId::parse("#412").expect_err("issue numbers are not task ids");

        assert!(error.contains("must start with `TASK-`"), "{error}");
    }

    #[test]
    fn initiative_ids_carry_year_and_sequence_but_no_repository() {
        let initiative = InitiativeId::parse("INIT-2026-0042").expect("valid initiative id");

        assert_eq!(initiative.year(), 2026);
        assert_eq!(initiative.sequence(), 42);
        assert_eq!(initiative.short(), "INIT-0042");
    }

    #[test]
    fn initiative_ids_require_a_four_digit_year() {
        let error = InitiativeId::parse("INIT-26-0042").expect_err("short year is rejected");

        assert!(error.contains("four digit year"), "{error}");
    }

    #[test]
    fn task_plan_ids_encode_the_task_and_version() {
        let task = TaskId::parse("TASK-0017").expect("valid task id");
        let plan = TaskPlanId::new(&task, 2);

        assert_eq!(plan.as_str(), "TASKPLAN-0017-v2");
        assert_eq!(TaskPlanId::parse("TASKPLAN-0017-v2"), Ok(plan));
    }

    #[test]
    fn task_plan_versions_start_at_one() {
        let error = TaskPlanId::parse("TASKPLAN-0017-v0").expect_err("v0 is rejected");

        assert!(error.contains("starts at 1"), "{error}");
    }

    #[test]
    fn identifiers_serialize_as_their_text_form() {
        let rendered = serde_json::to_string(&RequirementId::parse("REQ-012").expect("valid"))
            .expect("serializable");

        assert_eq!(rendered, "\"REQ-012\"");
        assert_eq!(
            serde_json::from_str::<RequirementId>("\"REQ-012\"").expect("deserializable"),
            RequirementId::parse("REQ-012").expect("valid")
        );
    }

    #[test]
    fn identifier_deserialization_rejects_malformed_text() {
        let error = serde_json::from_str::<RequirementId>("\"REQ-twelve\"")
            .expect_err("non numeric sequence is rejected");

        assert!(error.to_string().contains("decimal sequence"), "{error}");
    }
}
