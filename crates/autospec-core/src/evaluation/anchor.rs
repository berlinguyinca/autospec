//! Labeled anchor suites with protected holdout access.
//!
//! Holdout labels are the asset under attack: a mutation-role consumer must
//! never see `protected_holdout` labels (reward hacking, handoff §11.3/§12.1),
//! and every artifact is re-digested before a suite is trusted. Design:
//! `docs/specs/2026-09-05-evaluator-coevolution-design.md` §"Security and trust".
use std::collections::BTreeSet;
use std::fmt;
use std::path::{Component, Path};

use serde::{Deserialize, Serialize};

use super::digest::Digest;
use super::error::EvaluationError;
use super::ids::{AnchorCaseId, AnchorSuiteId, EvaluatorSlot};
use super::statistics::Ppm;
use super::EVALUATION_SCHEMA_VERSION;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProtectedLabel {
    Accept,
    Reject,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnchorVisibility {
    Development,
    PublicRegression,
    ProtectedHoldout,
    Quarantine,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessRole {
    Operator,
    Qualification,
    Mutation,
}

impl AccessRole {
    pub const ALL: [AccessRole; 3] = [Self::Operator, Self::Qualification, Self::Mutation];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Operator => "operator",
            Self::Qualification => "qualification",
            Self::Mutation => "mutation",
        }
    }

    pub fn parse(value: &str) -> Result<Self, EvaluationError> {
        Self::ALL
            .iter()
            .copied()
            .find(|r| r.as_str() == value)
            .ok_or_else(|| EvaluationError::parse(format!("unknown access role {value:?}")))
    }
}

impl fmt::Display for AccessRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Who made this evaluation object, and from what source. Shared with
/// `evaluator.rs` (plan Task 4); kept here until that module lands so anchor
/// suites are self-contained. Excluded from `suite_digest()`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    pub created_by: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProtectedSubset {
    pub name: String,
    pub tag: String,
    #[serde(default)]
    pub max_false_accept: Option<Ppm>,
    #[serde(default)]
    pub max_false_reject: Option<Ppm>,
    #[serde(default)]
    pub regression_tolerance_cases: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnchorCase {
    pub case_id: AnchorCaseId,
    pub artifact_ref: String,
    /// `None` only in redacted views; on-disk suites must always carry a label.
    #[serde(default)]
    pub expected_label: Option<ProtectedLabel>,
    pub severity: Severity,
    #[serde(default)]
    pub tags: BTreeSet<String>,
    pub visibility: AnchorVisibility,
    pub source: String,
    #[serde(default)]
    pub adjudication: Option<String>,
    pub content_digest: Digest,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnchorSuite {
    pub schema: u64,
    pub suite_id: AnchorSuiteId,
    pub version: u32,
    pub slot: EvaluatorSlot,
    pub cases: Vec<AnchorCase>,
    pub minimum_case_count: usize,
    #[serde(default)]
    pub required_subsets: Vec<ProtectedSubset>,
    pub provenance: Provenance,
}

impl AnchorSuite {
    pub fn validate(&self) -> Result<(), EvaluationError> {
        if self.schema != EVALUATION_SCHEMA_VERSION {
            return Err(EvaluationError::invariant(
                "unsupported anchor suite schema",
            ));
        }
        if self.version == 0 {
            return Err(EvaluationError::invariant(
                "anchor suite versions start at 1",
            ));
        }
        let mut seen = BTreeSet::new();
        for case in &self.cases {
            if !seen.insert(&case.case_id) {
                return Err(EvaluationError::invariant(format!(
                    "duplicate anchor case {}",
                    case.case_id
                )));
            }
            if case.expected_label.is_none() {
                return Err(EvaluationError::invariant(format!(
                    "anchor case {} has no expected_label",
                    case.case_id
                )));
            }
            let path = Path::new(&case.artifact_ref);
            if path.is_absolute()
                || path
                    .components()
                    .any(|c| matches!(c, Component::ParentDir | Component::Prefix(_)))
            {
                return Err(EvaluationError::invariant(format!(
                    "anchor case {} artifact_ref must be repo-relative without '..'",
                    case.case_id
                )));
            }
        }
        if self.minimum_case_count > self.cases.len() {
            return Err(EvaluationError::invariant(format!(
                "minimum_case_count {} exceeds {} cases",
                self.minimum_case_count,
                self.cases.len()
            )));
        }
        for subset in &self.required_subsets {
            if !self.cases.iter().any(|c| c.tags.contains(&subset.tag)) {
                return Err(EvaluationError::invariant(format!(
                    "required subset {} references tag {:?} that no case carries",
                    subset.name, subset.tag
                )));
            }
        }
        Ok(())
    }

    /// Covers ids, labels, visibility, severity, tags, artifact digests, subsets, version.
    /// Not provenance.
    pub fn suite_digest(&self) -> Digest {
        let mut parts: Vec<Vec<u8>> = vec![
            self.suite_id.as_str().into(),
            self.version.to_string().into(),
            self.slot.as_str().into(),
        ];
        for case in &self.cases {
            let label = match case.expected_label {
                Some(ProtectedLabel::Accept) => "accept",
                Some(ProtectedLabel::Reject) => "reject",
                None => "",
            };
            parts.push(
                format!(
                    "{}|{}|{:?}|{:?}|{}|{}",
                    case.case_id,
                    label,
                    case.visibility,
                    case.severity,
                    case.tags.iter().cloned().collect::<Vec<_>>().join(","),
                    case.content_digest
                )
                .into_bytes(),
            );
        }
        for subset in &self.required_subsets {
            parts.push(
                format!(
                    "{}|{}|{:?}|{:?}|{}",
                    subset.name,
                    subset.tag,
                    subset.max_false_accept,
                    subset.max_false_reject,
                    subset.regression_tolerance_cases
                )
                .into_bytes(),
            );
        }
        let refs: Vec<&[u8]> = parts.iter().map(Vec::as_slice).collect();
        Digest::of_parts(&refs)
    }

    /// Redacts protected labels for roles that must never see them (handoff
    /// §11.3, §12.1). `Mutation` drops quarantine cases and nulls
    /// protected-holdout labels; every other role sees the full suite.
    pub fn view(&self, role: AccessRole) -> AnchorSuite {
        let mut view = self.clone();
        if role == AccessRole::Mutation {
            view.cases
                .retain(|c| c.visibility != AnchorVisibility::Quarantine);
            for case in &mut view.cases {
                if case.visibility == AnchorVisibility::ProtectedHoldout {
                    case.expected_label = None;
                }
            }
        }
        view
    }

    /// Qualification input: every case must carry a label. A redacted
    /// (mutation) view errors instead of silently qualifying on visible cases.
    pub fn labeled_cases(&self) -> Result<Vec<(&AnchorCase, ProtectedLabel)>, EvaluationError> {
        self.cases
            .iter()
            .map(|c| {
                c.expected_label
                    .map(|l| (c, l))
                    .ok_or_else(|| {
                        EvaluationError::access_denied(format!(
                            "case {} has a redacted label; qualification needs AccessRole::Qualification",
                            c.case_id
                        ))
                    })
            })
            .collect()
    }

    /// Re-digests every artifact against the pinned `content_digest`.
    /// Mismatches name the offending cases and fail closed.
    pub fn verify_artifacts(&self, repo_root: &Path) -> Result<(), EvaluationError> {
        let mut mismatches = Vec::new();
        for case in &self.cases {
            let bytes = std::fs::read(repo_root.join(&case.artifact_ref)).map_err(|e| {
                EvaluationError::integrity(format!(
                    "anchor case {} artifact {} unreadable: {e}",
                    case.case_id, case.artifact_ref
                ))
            })?;
            if Digest::of_bytes(&bytes) != case.content_digest {
                mismatches.push(case.case_id.to_string());
            }
        }
        if mismatches.is_empty() {
            Ok(())
        } else {
            Err(EvaluationError::integrity(format!(
                "anchor artifact digest mismatch for cases: {}",
                mismatches.join(", ")
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn case(
        id: &str,
        label: ProtectedLabel,
        vis: AnchorVisibility,
        tag: Option<&str>,
    ) -> AnchorCase {
        AnchorCase {
            case_id: AnchorCaseId::parse(id).unwrap(),
            artifact_ref: format!("fixtures/{id}/patch.diff"),
            expected_label: Some(label),
            severity: Severity::Medium,
            tags: tag.into_iter().map(String::from).collect(),
            visibility: vis,
            source: "synthetic".into(),
            adjudication: None,
            content_digest: Digest::of_bytes(id.as_bytes()),
        }
    }

    fn suite() -> AnchorSuite {
        AnchorSuite {
            schema: 1,
            suite_id: AnchorSuiteId::parse("architecture-fixture").unwrap(),
            version: 1,
            slot: EvaluatorSlot::Architecture,
            cases: vec![
                case(
                    "c1",
                    ProtectedLabel::Accept,
                    AnchorVisibility::Development,
                    None,
                ),
                case(
                    "c2",
                    ProtectedLabel::Reject,
                    AnchorVisibility::ProtectedHoldout,
                    Some("critical-security"),
                ),
                case(
                    "c3",
                    ProtectedLabel::Reject,
                    AnchorVisibility::Quarantine,
                    None,
                ),
            ],
            minimum_case_count: 2,
            required_subsets: vec![ProtectedSubset {
                name: "critical".into(),
                tag: "critical-security".into(),
                max_false_accept: Some(Ppm(0)),
                max_false_reject: None,
                regression_tolerance_cases: 0,
            }],
            provenance: Provenance {
                created_by: "operator".into(),
                source: "fixture".into(),
                notes: None,
            },
        }
    }

    #[test]
    fn mutation_view_strips_holdout_labels_and_drops_quarantine() {
        let view = suite().view(AccessRole::Mutation);
        assert_eq!(view.cases.len(), 2);
        assert_eq!(view.cases[0].expected_label, Some(ProtectedLabel::Accept));
        assert_eq!(view.cases[1].expected_label, None);
        assert!(
            view.labeled_cases().is_err(),
            "a redacted view cannot qualify anything"
        );
        assert_eq!(
            suite()
                .view(AccessRole::Qualification)
                .labeled_cases()
                .unwrap()
                .len(),
            3
        );
    }

    #[test]
    fn suite_digest_changes_when_a_label_flips() {
        let a = suite();
        let mut b = suite();
        b.cases[0].expected_label = Some(ProtectedLabel::Reject);
        assert_ne!(a.suite_digest(), b.suite_digest());
        let mut c = suite();
        c.provenance.notes = Some("irrelevant".into());
        assert_eq!(a.suite_digest(), c.suite_digest());
    }

    #[test]
    fn validate_rejects_duplicates_missing_labels_and_unknown_subset_tags() {
        let mut s = suite();
        s.cases.push(case(
            "c1",
            ProtectedLabel::Accept,
            AnchorVisibility::Development,
            None,
        ));
        assert!(s.validate().is_err());
        let mut s = suite();
        s.cases[0].expected_label = None;
        assert!(s.validate().is_err());
        let mut s = suite();
        s.required_subsets[0].tag = "nope".into();
        assert!(s.validate().is_err());
        let mut s = suite();
        s.minimum_case_count = 99;
        assert!(s.validate().is_err());
        let mut s = suite();
        s.cases[0].artifact_ref = "/abs/path".into();
        assert!(s.validate().is_err());
        let mut s = suite();
        s.cases[0].artifact_ref = "../escape".into();
        assert!(s.validate().is_err());
        assert!(suite().validate().is_ok());
    }

    #[test]
    fn verify_artifacts_names_the_tampered_case() {
        let root = std::env::temp_dir().join(format!("autospec-anchor-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let s = suite();
        for c in &s.cases {
            let path = root.join(&c.artifact_ref);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, c.case_id.as_str()).unwrap();
        }
        assert!(s.verify_artifacts(&root).is_ok());
        std::fs::write(root.join(&s.cases[1].artifact_ref), b"poisoned").unwrap();
        let err = s.verify_artifacts(&root).unwrap_err();
        assert_eq!(err.kind, crate::evaluation::EvaluationErrorKind::Integrity);
        assert!(err.message.contains("c2"), "{err}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn access_role_round_trips() {
        for role in AccessRole::ALL {
            assert_eq!(AccessRole::parse(role.as_str()).unwrap(), role);
        }
        assert!(AccessRole::parse("root").is_err());
        assert_eq!(AccessRole::Mutation.to_string(), "mutation");
    }

    #[test]
    fn suite_json_round_trip_preserves_fields() {
        let s = suite();
        let text = serde_json::to_string_pretty(&s).unwrap();
        assert_eq!(serde_json::from_str::<AnchorSuite>(&text).unwrap(), s);
    }
}
