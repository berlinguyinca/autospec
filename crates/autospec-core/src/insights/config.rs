//! Operator-facing `insights:` configuration block (spec §45).
//!
//! Parsing is fail-closed: unknown keys are rejected rather than ignored,
//! out-of-range thresholds are errors, and every privacy-sensitive default
//! is safe — remote analysis is off, auto-merge is off, and secret
//! redaction is on (§39).

use std::collections::HashSet;
use std::str::FromStr;

use yaml_edit::{Document, Mapping};

use crate::error::AutospecError;

/// Root-level keys accepted in an insights configuration document.
const ROOT_KEYS: &[&str] = &["insights"];

/// Sub-blocks of the `insights:` mapping, in §45 order.
const INSIGHTS_KEYS: &[&str] = &[
    "enabled",
    "ingestion",
    "semantic_analysis",
    "strong_analysis",
    "retention",
    "thresholds",
    "self_improvement",
    "privacy",
];

/// Per-harness session ingestion toggles.
#[derive(Debug, Clone, PartialEq)]
pub struct IngestionConfig {
    pub pi: bool,
    pub codex: bool,
    pub claude: bool,
}

/// Semantic (LLM) analysis policy. Remote analysis is off by default (§39).
#[derive(Debug, Clone, PartialEq)]
pub struct SemanticAnalysisConfig {
    pub provider: String,
    pub model: String,
    pub remote_allowed: bool,
}

/// Strong-model analysis routing.
#[derive(Debug, Clone, PartialEq)]
pub struct StrongAnalysisConfig {
    pub provider: String,
    pub role: String,
}

/// Retention windows (days) for raw and normalized session rows.
#[derive(Debug, Clone, PartialEq)]
pub struct RetentionConfig {
    pub raw_sessions_days: u64,
    pub normalized_events_days: u64,
}

/// Pattern-detection and proposal thresholds.
#[derive(Debug, Clone, PartialEq)]
pub struct ThresholdsConfig {
    pub recurring_pattern_min_sessions: u64,
    pub recurring_pattern_min_occurrences: u64,
    pub proposal_confidence_min: f64,
}

/// Self-improvement gates. Auto-merge is off by default.
#[derive(Debug, Clone, PartialEq)]
pub struct SelfImprovementConfig {
    pub allow_auto_pr: bool,
    pub allow_auto_merge: bool,
}

/// Privacy controls. Secret redaction is on by default.
#[derive(Debug, Clone, PartialEq)]
pub struct PrivacyConfig {
    pub redact_secrets: bool,
}

/// Typed view of the `insights:` block (spec §45).
#[derive(Debug, Clone, PartialEq)]
pub struct InsightsConfig {
    pub enabled: bool,
    pub ingestion: IngestionConfig,
    pub semantic_analysis: SemanticAnalysisConfig,
    pub strong_analysis: StrongAnalysisConfig,
    pub retention: RetentionConfig,
    pub thresholds: ThresholdsConfig,
    pub self_improvement: SelfImprovementConfig,
    pub privacy: PrivacyConfig,
}

impl Default for InsightsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            ingestion: IngestionConfig {
                pi: true,
                codex: false,
                claude: false,
            },
            semantic_analysis: SemanticAnalysisConfig {
                provider: "inferweave".to_string(),
                model: "local-qwen".to_string(),
                remote_allowed: false,
            },
            strong_analysis: StrongAnalysisConfig {
                provider: "autospec-router".to_string(),
                role: "planning".to_string(),
            },
            retention: RetentionConfig {
                raw_sessions_days: 90,
                normalized_events_days: 365,
            },
            thresholds: ThresholdsConfig {
                recurring_pattern_min_sessions: 3,
                recurring_pattern_min_occurrences: 5,
                proposal_confidence_min: 0.80,
            },
            self_improvement: SelfImprovementConfig {
                allow_auto_pr: true,
                allow_auto_merge: false,
            },
            privacy: PrivacyConfig {
                redact_secrets: true,
            },
        }
    }
}

/// Parse an `insights:` configuration document into a typed struct.
///
/// An empty document yields the fail-closed defaults. Unknown keys at any
/// level and out-of-range thresholds are rejected with a descriptive error.
pub fn load(source: &str) -> Result<InsightsConfig, AutospecError> {
    let document = Document::from_str(source).map_err(|error| {
        AutospecError::parse(
            "insights config",
            format!("could not parse insights YAML: {error}"),
        )
    })?;
    let Some(root) = document.as_mapping() else {
        return Ok(InsightsConfig::default());
    };
    validate_keys(&root, ROOT_KEYS, "insights config")?;
    let Some(section) = mapping(&root, "insights")? else {
        return Ok(InsightsConfig::default());
    };
    validate_keys(&section, INSIGHTS_KEYS, "insights")?;
    let defaults = InsightsConfig::default();

    Ok(InsightsConfig {
        enabled: flag(&section, "enabled", defaults.enabled)?,
        ingestion: parse_ingestion(&section, defaults.ingestion)?,
        semantic_analysis: parse_semantic_analysis(&section, defaults.semantic_analysis)?,
        strong_analysis: parse_strong_analysis(&section, defaults.strong_analysis)?,
        retention: parse_retention(&section, defaults.retention)?,
        thresholds: parse_thresholds(&section, defaults.thresholds)?,
        self_improvement: parse_self_improvement(&section, defaults.self_improvement)?,
        privacy: parse_privacy(&section, defaults.privacy)?,
    })
}

fn parse_ingestion(
    section: &Mapping,
    defaults: IngestionConfig,
) -> Result<IngestionConfig, AutospecError> {
    let Some(section) = mapping(section, "ingestion")? else {
        return Ok(defaults);
    };
    validate_keys(&section, &["pi", "codex", "claude"], "ingestion")?;
    Ok(IngestionConfig {
        pi: flag(&section, "pi", defaults.pi)?,
        codex: flag(&section, "codex", defaults.codex)?,
        claude: flag(&section, "claude", defaults.claude)?,
    })
}

fn parse_semantic_analysis(
    section: &Mapping,
    defaults: SemanticAnalysisConfig,
) -> Result<SemanticAnalysisConfig, AutospecError> {
    let Some(section) = mapping(section, "semantic_analysis")? else {
        return Ok(defaults);
    };
    validate_keys(
        &section,
        &["provider", "model", "remote_allowed"],
        "semantic_analysis",
    )?;
    Ok(SemanticAnalysisConfig {
        provider: text(&section, "provider", &defaults.provider)?,
        model: text(&section, "model", &defaults.model)?,
        remote_allowed: flag(&section, "remote_allowed", defaults.remote_allowed)?,
    })
}

fn parse_strong_analysis(
    section: &Mapping,
    defaults: StrongAnalysisConfig,
) -> Result<StrongAnalysisConfig, AutospecError> {
    let Some(section) = mapping(section, "strong_analysis")? else {
        return Ok(defaults);
    };
    validate_keys(&section, &["provider", "role"], "strong_analysis")?;
    Ok(StrongAnalysisConfig {
        provider: text(&section, "provider", &defaults.provider)?,
        role: text(&section, "role", &defaults.role)?,
    })
}

fn parse_retention(
    section: &Mapping,
    defaults: RetentionConfig,
) -> Result<RetentionConfig, AutospecError> {
    let Some(section) = mapping(section, "retention")? else {
        return Ok(defaults);
    };
    validate_keys(
        &section,
        &["raw_sessions_days", "normalized_events_days"],
        "retention",
    )?;
    Ok(RetentionConfig {
        raw_sessions_days: unsigned(&section, "raw_sessions_days", defaults.raw_sessions_days)?,
        normalized_events_days: unsigned(
            &section,
            "normalized_events_days",
            defaults.normalized_events_days,
        )?,
    })
}

fn parse_thresholds(
    section: &Mapping,
    defaults: ThresholdsConfig,
) -> Result<ThresholdsConfig, AutospecError> {
    let Some(section) = mapping(section, "thresholds")? else {
        return Ok(defaults);
    };
    validate_keys(
        &section,
        &[
            "recurring_pattern_min_sessions",
            "recurring_pattern_min_occurrences",
            "proposal_confidence_min",
        ],
        "thresholds",
    )?;
    Ok(ThresholdsConfig {
        recurring_pattern_min_sessions: unsigned(
            &section,
            "recurring_pattern_min_sessions",
            defaults.recurring_pattern_min_sessions,
        )?,
        recurring_pattern_min_occurrences: unsigned(
            &section,
            "recurring_pattern_min_occurrences",
            defaults.recurring_pattern_min_occurrences,
        )?,
        proposal_confidence_min: confidence(
            &section,
            "proposal_confidence_min",
            defaults.proposal_confidence_min,
        )?,
    })
}

fn parse_self_improvement(
    section: &Mapping,
    defaults: SelfImprovementConfig,
) -> Result<SelfImprovementConfig, AutospecError> {
    let Some(section) = mapping(section, "self_improvement")? else {
        return Ok(defaults);
    };
    validate_keys(
        &section,
        &["allow_auto_pr", "allow_auto_merge"],
        "self_improvement",
    )?;
    Ok(SelfImprovementConfig {
        allow_auto_pr: flag(&section, "allow_auto_pr", defaults.allow_auto_pr)?,
        allow_auto_merge: flag(&section, "allow_auto_merge", defaults.allow_auto_merge)?,
    })
}

fn parse_privacy(
    section: &Mapping,
    defaults: PrivacyConfig,
) -> Result<PrivacyConfig, AutospecError> {
    let Some(section) = mapping(section, "privacy")? else {
        return Ok(defaults);
    };
    validate_keys(&section, &["redact_secrets"], "privacy")?;
    Ok(PrivacyConfig {
        redact_secrets: flag(&section, "redact_secrets", defaults.redact_secrets)?,
    })
}

fn validation_error(message: impl Into<String>) -> AutospecError {
    AutospecError::validation(message.into())
}

fn mapping(mapping: &Mapping, key: &str) -> Result<Option<Mapping>, AutospecError> {
    let Some(node) = mapping.get(key) else {
        return Ok(None);
    };
    node.as_mapping()
        .cloned()
        .map(Some)
        .ok_or_else(|| validation_error(format!("{key} must be a mapping")))
}

fn scalar(mapping: &Mapping, key: &str) -> Result<Option<String>, AutospecError> {
    let Some(node) = mapping.get(key) else {
        return Ok(None);
    };
    node.as_scalar()
        .map(|value| value.as_string())
        .map(Some)
        .ok_or_else(|| validation_error(format!("{key} must be a scalar")))
}

fn flag(mapping: &Mapping, key: &str, default: bool) -> Result<bool, AutospecError> {
    let Some(value) = scalar(mapping, key)? else {
        return Ok(default);
    };
    match value.as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        other => Err(validation_error(format!(
            "{key} must be true or false, got: {other}"
        ))),
    }
}

fn unsigned(mapping: &Mapping, key: &str, default: u64) -> Result<u64, AutospecError> {
    let Some(value) = scalar(mapping, key)? else {
        return Ok(default);
    };
    value
        .parse::<u64>()
        .map_err(|_| validation_error(format!("{key} must be a non-negative integer")))
}

fn text(mapping: &Mapping, key: &str, default: &str) -> Result<String, AutospecError> {
    let Some(value) = scalar(mapping, key)? else {
        return Ok(default.to_string());
    };
    if value.is_empty() {
        return Err(validation_error(format!(
            "{key} must be a non-empty string"
        )));
    }
    Ok(value)
}

fn confidence(mapping: &Mapping, key: &str, default: f64) -> Result<f64, AutospecError> {
    let Some(value) = scalar(mapping, key)? else {
        return Ok(default);
    };
    let parsed: f64 = value
        .parse()
        .map_err(|_| validation_error(format!("{key} must be a number")))?;
    if parsed.is_nan() || !(0.0..=1.0).contains(&parsed) {
        return Err(validation_error(format!(
            "{key} must be between 0 and 1, got: {value}"
        )));
    }
    Ok(parsed)
}

fn validate_keys(mapping: &Mapping, allowed: &[&str], label: &str) -> Result<(), AutospecError> {
    let mut seen = HashSet::new();
    for key in mapping.keys() {
        let key = key
            .as_scalar()
            .map(|value| value.as_string())
            .ok_or_else(|| validation_error(format!("{label} key must be a string")))?;
        if !seen.insert(key.clone()) {
            return Err(validation_error(format!("duplicate {label} key: {key}")));
        }
        if !allowed.contains(&key.as_str()) {
            return Err(validation_error(format!("unknown key in {label}: {key}")));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEC_EXAMPLE: &str = r#"
insights:
  enabled: true

  ingestion:
    pi: true
    codex: false
    claude: false

  semantic_analysis:
    provider: inferweave
    model: local-qwen
    remote_allowed: false

  strong_analysis:
    provider: autospec-router
    role: planning

  retention:
    raw_sessions_days: 90
    normalized_events_days: 365

  thresholds:
    recurring_pattern_min_sessions: 3
    recurring_pattern_min_occurrences: 5
    proposal_confidence_min: 0.80

  self_improvement:
    allow_auto_pr: true
    allow_auto_merge: false

  privacy:
    redact_secrets: true
"#;

    #[test]
    fn the_spec_example_parses_into_the_expected_typed_values() {
        let config = load(SPEC_EXAMPLE).unwrap();

        assert!(config.enabled);
        assert!(config.ingestion.pi);
        assert!(!config.ingestion.codex);
        assert!(!config.ingestion.claude);
        assert_eq!(config.semantic_analysis.provider, "inferweave");
        assert_eq!(config.semantic_analysis.model, "local-qwen");
        assert!(!config.semantic_analysis.remote_allowed);
        assert_eq!(config.strong_analysis.provider, "autospec-router");
        assert_eq!(config.strong_analysis.role, "planning");
        assert_eq!(config.retention.raw_sessions_days, 90);
        assert_eq!(config.retention.normalized_events_days, 365);
        assert_eq!(config.thresholds.recurring_pattern_min_sessions, 3);
        assert_eq!(config.thresholds.recurring_pattern_min_occurrences, 5);
        assert!((config.thresholds.proposal_confidence_min - 0.80).abs() < f64::EPSILON);
        assert!(config.self_improvement.allow_auto_pr);
        assert!(!config.self_improvement.allow_auto_merge);
        assert!(config.privacy.redact_secrets);
    }

    #[test]
    fn an_empty_document_yields_the_fail_closed_defaults() {
        assert_eq!(load("").unwrap(), InsightsConfig::default());
        assert_eq!(load("insights: {}\n").unwrap(), InsightsConfig::default());
    }

    #[test]
    fn defaults_are_fail_closed_for_remote_analysis_auto_merge_and_redaction() {
        let defaults = InsightsConfig::default();

        assert!(!defaults.semantic_analysis.remote_allowed);
        assert!(!defaults.self_improvement.allow_auto_merge);
        assert!(defaults.privacy.redact_secrets);
    }

    #[test]
    fn an_unknown_root_key_is_rejected_rather_than_ignored() {
        let error = load("insight:\n").unwrap_err();

        assert!(error
            .to_string()
            .contains("unknown key in insights config: insight"));
    }

    #[test]
    fn an_unknown_key_inside_a_subsection_is_rejected() {
        let source = "insights:\n  thresholds:\n    recurring_pattern_min_session: 3\n";

        let error = load(source).unwrap_err();

        assert!(error
            .to_string()
            .contains("unknown key in thresholds: recurring_pattern_min_session"));
    }

    #[test]
    fn an_unknown_key_in_semantic_analysis_cannot_silently_enable_remote_enrichment() {
        let source = "insights:\n  semantic_analysis:\n    remote_allow: true\n";

        assert!(load(source).is_err());
    }

    #[test]
    fn a_confidence_threshold_above_one_is_rejected() {
        let source = "insights:\n  thresholds:\n    proposal_confidence_min: 1.5\n";

        let error = load(source).unwrap_err();

        assert!(error
            .to_string()
            .contains("proposal_confidence_min must be between 0 and 1"));
    }

    #[test]
    fn a_negative_confidence_threshold_is_rejected() {
        let source = "insights:\n  thresholds:\n    proposal_confidence_min: -0.1\n";

        assert!(load(source).is_err());
    }

    #[test]
    fn a_non_numeric_confidence_threshold_is_rejected() {
        let source = "insights:\n  thresholds:\n    proposal_confidence_min: high\n";

        assert!(load(source).is_err());
    }

    #[test]
    fn a_negative_retention_window_is_rejected() {
        let source = "insights:\n  retention:\n    raw_sessions_days: -90\n";

        let error = load(source).unwrap_err();

        assert!(error
            .to_string()
            .contains("raw_sessions_days must be a non-negative integer"));
    }

    #[test]
    fn a_non_boolean_flag_is_rejected() {
        let source = "insights:\n  enabled: yes\n";

        let error = load(source).unwrap_err();

        assert!(error.to_string().contains("enabled must be true or false"));
    }

    #[test]
    fn a_section_of_the_wrong_shape_is_rejected() {
        let source = "insights:\n  retention: 90\n";

        let error = load(source).unwrap_err();

        assert!(error.to_string().contains("retention must be a mapping"));
    }

    #[test]
    fn a_duplicate_key_is_rejected() {
        let source = "insights:\n  privacy:\n    redact_secrets: true\n    redact_secrets: false\n";

        assert!(load(source).is_err());
    }

    #[test]
    fn values_can_be_overridden_from_the_defaults() {
        let source = r#"
insights:
  enabled: false
  semantic_analysis:
    remote_allowed: true
  self_improvement:
    allow_auto_merge: true
  privacy:
    redact_secrets: false
  thresholds:
    proposal_confidence_min: 0.5
"#;

        let config = load(source).unwrap();

        assert!(!config.enabled);
        assert!(config.semantic_analysis.remote_allowed);
        assert!(config.self_improvement.allow_auto_merge);
        assert!(!config.privacy.redact_secrets);
        assert!((config.thresholds.proposal_confidence_min - 0.5).abs() < f64::EPSILON);
        // Untouched keys keep their defaults.
        assert!(config.ingestion.pi);
        assert_eq!(config.retention.normalized_events_days, 365);
    }

    #[test]
    fn malformed_yaml_is_rejected() {
        assert!(load("insights:\n  bad indent: [\n").is_err());
    }

    #[test]
    fn a_scalar_document_is_treated_as_empty() {
        assert_eq!(load("just-a-string\n").unwrap(), InsightsConfig::default());
    }
}
