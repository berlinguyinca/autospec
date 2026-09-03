use std::collections::BTreeMap;
use std::str::FromStr;

use yaml_edit::{Document, Mapping};

use super::super::error::CodeIntelError;
use super::yaml::{
    entry_key, flag, optional_scalar, required_scalar, section, unsigned, validate_keys,
};
use super::{
    BackendConfig, BackendMode, CodeIntelConfig, ContextConfig, FallbackConfig, LanguagesConfig,
    SecurityConfig, WorkflowConfig, WorkspaceConfig, SUPPORTED_VERSION,
};

const ROOT_KEYS: &[&str] = &[
    "version",
    "enabled",
    "backend",
    "workspace",
    "languages",
    "fallback",
    "workflow",
    "context",
    "security",
];

pub(super) fn parse(source: &str) -> Result<CodeIntelConfig, CodeIntelError> {
    let document = Document::from_str(source).map_err(|error| {
        CodeIntelError::config(format!("could not parse code intelligence YAML: {error}"))
    })?;
    let root = document
        .as_mapping()
        .ok_or_else(|| CodeIntelError::config("code intelligence config root must be a mapping"))?;
    validate_keys(&root, ROOT_KEYS, "code intelligence config")?;
    let version = required_scalar(&root, "version", "code intelligence version")?;
    if version != SUPPORTED_VERSION {
        return Err(CodeIntelError::config(format!(
            "unsupported code intelligence config version: {version}"
        )));
    }
    let defaults = CodeIntelConfig::default();
    Ok(CodeIntelConfig {
        version,
        enabled: flag(&root, "enabled", defaults.enabled)?,
        backend: parse_backend(&root)?,
        workspace: parse_workspace(&root)?,
        languages: parse_languages(&root)?,
        fallback: parse_fallback(&root)?,
        workflow: parse_workflow(&root)?,
        context: parse_context(&root)?,
        security: parse_security(&root)?,
    })
}

fn parse_backend(root: &Mapping) -> Result<BackendConfig, CodeIntelError> {
    let defaults = BackendConfig::default();
    let Some(section) = section(root, "backend")? else {
        return Ok(defaults);
    };
    validate_keys(&section, &["type", "mode", "binary"], "backend")?;
    let mode = match optional_scalar(&section, "mode", "backend mode")? {
        Some(value) => BackendMode::parse(&value)?,
        None => defaults.mode,
    };
    Ok(BackendConfig {
        kind: optional_scalar(&section, "type", "backend type")?.unwrap_or(defaults.kind),
        mode,
        binary: optional_scalar(&section, "binary", "backend binary")?.unwrap_or(defaults.binary),
    })
}

fn parse_workspace(root: &Mapping) -> Result<WorkspaceConfig, CodeIntelError> {
    let defaults = WorkspaceConfig::default();
    let Some(section) = section(root, "workspace")? else {
        return Ok(defaults);
    };
    validate_keys(
        &section,
        &["isolation", "idle_ttl_minutes", "warm_cache"],
        "workspace",
    )?;
    let isolation = optional_scalar(&section, "isolation", "workspace isolation")?
        .unwrap_or(defaults.isolation);
    if isolation != "worktree" && isolation != "repository" {
        return Err(CodeIntelError::config(format!(
            "unknown workspace isolation: {isolation}"
        )));
    }
    Ok(WorkspaceConfig {
        isolation,
        idle_ttl_minutes: unsigned(
            &section,
            "idle_ttl_minutes",
            "workspace idle TTL",
            defaults.idle_ttl_minutes,
        )?,
        warm_cache: flag(&section, "warm_cache", defaults.warm_cache)?,
    })
}

fn parse_languages(root: &Mapping) -> Result<LanguagesConfig, CodeIntelError> {
    let defaults = LanguagesConfig::default();
    let Some(section) = section(root, "languages")? else {
        return Ok(defaults);
    };
    validate_keys(&section, &["auto_detect", "overrides"], "languages")?;
    Ok(LanguagesConfig {
        auto_detect: flag(&section, "auto_detect", defaults.auto_detect)?,
        overrides: parse_overrides(&section)?,
    })
}

fn parse_overrides(languages: &Mapping) -> Result<BTreeMap<String, String>, CodeIntelError> {
    let Some(mapping) = section(languages, "overrides")? else {
        return Ok(BTreeMap::new());
    };
    let mut overrides = BTreeMap::new();
    for entry in mapping.entries() {
        let key = entry_key(&entry, "language override")?;
        let value = entry
            .value_node()
            .and_then(|node| node.as_mapping().cloned())
            .ok_or_else(|| {
                CodeIntelError::config(format!("language override {key} must be a mapping"))
            })?;
        validate_keys(&value, &["server"], "language override")?;
        let server = required_scalar(&value, "server", "language override server")?;
        if overrides.insert(key.clone(), server).is_some() {
            return Err(CodeIntelError::config(format!(
                "duplicate language override: {key}"
            )));
        }
    }
    Ok(overrides)
}

fn parse_fallback(root: &Mapping) -> Result<FallbackConfig, CodeIntelError> {
    let defaults = FallbackConfig::default();
    let Some(section) = section(root, "fallback")? else {
        return Ok(defaults);
    };
    validate_keys(&section, &["structural", "textual"], "fallback")?;
    Ok(FallbackConfig {
        structural: optional_scalar(&section, "structural", "structural fallback")?
            .or(defaults.structural)
            .filter(|value| !value.is_empty()),
        textual: optional_scalar(&section, "textual", "textual fallback")?
            .or(defaults.textual)
            .filter(|value| !value.is_empty()),
    })
}

fn parse_workflow(root: &Mapping) -> Result<WorkflowConfig, CodeIntelError> {
    let defaults = WorkflowConfig::default();
    let Some(section) = section(root, "workflow")? else {
        return Ok(defaults);
    };
    validate_keys(
        &section,
        &[
            "require_pre_change_impact",
            "require_post_change_diagnostics",
            "reviewer_independent_analysis",
            "block_new_errors",
        ],
        "workflow",
    )?;
    Ok(WorkflowConfig {
        require_pre_change_impact: flag(
            &section,
            "require_pre_change_impact",
            defaults.require_pre_change_impact,
        )?,
        require_post_change_diagnostics: flag(
            &section,
            "require_post_change_diagnostics",
            defaults.require_post_change_diagnostics,
        )?,
        reviewer_independent_analysis: flag(
            &section,
            "reviewer_independent_analysis",
            defaults.reviewer_independent_analysis,
        )?,
        block_new_errors: flag(&section, "block_new_errors", defaults.block_new_errors)?,
    })
}

fn parse_context(root: &Mapping) -> Result<ContextConfig, CodeIntelError> {
    let defaults = ContextConfig::default();
    let Some(section) = section(root, "context")? else {
        return Ok(defaults);
    };
    validate_keys(
        &section,
        &[
            "prefer_semantic",
            "include_related_tests",
            "include_rag",
            "include_git_history",
        ],
        "context",
    )?;
    let history = optional_scalar(&section, "include_git_history", "git history mode")?
        .unwrap_or(defaults.include_git_history);
    if !["targeted", "none", "full"].contains(&history.as_str()) {
        return Err(CodeIntelError::config(format!(
            "unknown git history mode: {history}"
        )));
    }
    Ok(ContextConfig {
        prefer_semantic: flag(&section, "prefer_semantic", defaults.prefer_semantic)?,
        include_related_tests: flag(
            &section,
            "include_related_tests",
            defaults.include_related_tests,
        )?,
        include_rag: flag(&section, "include_rag", defaults.include_rag)?,
        include_git_history: history,
    })
}

fn parse_security(root: &Mapping) -> Result<SecurityConfig, CodeIntelError> {
    let defaults = SecurityConfig::default();
    let Some(section) = section(root, "security")? else {
        return Ok(defaults);
    };
    validate_keys(
        &section,
        &["allow_public_bind", "trust_project_build_scripts"],
        "security",
    )?;
    Ok(SecurityConfig {
        allow_public_bind: flag(&section, "allow_public_bind", defaults.allow_public_bind)?,
        trust_project_build_scripts: flag(
            &section,
            "trust_project_build_scripts",
            defaults.trust_project_build_scripts,
        )?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = r#"
version: 1
enabled: true

backend:
  type: agent-lsp
  mode: local
  binary: agent-lsp

workspace:
  isolation: worktree
  idle_ttl_minutes: 45
  warm_cache: true

languages:
  auto_detect: true
  overrides:
    python: { server: basedpyright }
    rust: { server: rust-analyzer }

fallback:
  structural: ast-grep
  textual: rg

workflow:
  require_pre_change_impact: true
  require_post_change_diagnostics: true
  reviewer_independent_analysis: true
  block_new_errors: true

context:
  prefer_semantic: true
  include_related_tests: true
  include_rag: true
  include_git_history: targeted

security:
  allow_public_bind: false
  trust_project_build_scripts: false
"#;

    #[test]
    fn the_documented_configuration_parses() {
        let config = CodeIntelConfig::parse(FULL).unwrap();

        assert!(config.enabled);
        assert_eq!(config.backend.binary, "agent-lsp");
        assert_eq!(config.workspace.idle_ttl_minutes, 45);
        assert_eq!(
            config.languages.overrides.get("python").map(String::as_str),
            Some("basedpyright")
        );
        assert_eq!(config.fallback.structural.as_deref(), Some("ast-grep"));
        assert!(config.workflow.block_new_errors);
        assert_eq!(config.context.include_git_history, "targeted");
        assert!(!config.security.allow_public_bind);
    }

    #[test]
    fn a_minimal_configuration_takes_every_default() {
        let config = CodeIntelConfig::parse("version: 1\n").unwrap();

        assert_eq!(config, CodeIntelConfig::default());
    }

    #[test]
    fn a_missing_version_is_rejected() {
        let error = CodeIntelConfig::parse("enabled: true\n").unwrap_err();

        assert!(error.message().contains("version is required"));
    }

    #[test]
    fn an_unsupported_version_is_rejected() {
        let error = CodeIntelConfig::parse("version: 2\n").unwrap_err();

        assert!(error
            .message()
            .contains("unsupported code intelligence config version: 2"));
    }

    #[test]
    fn an_unknown_top_level_key_is_rejected_rather_than_ignored() {
        let error = CodeIntelConfig::parse("version: 1\nbackends: {}\n").unwrap_err();

        assert!(error
            .message()
            .contains("unknown key in code intelligence config: backends"));
    }

    #[test]
    fn a_typo_in_a_workflow_gate_is_rejected_rather_than_silently_disabling_it() {
        let source = "version: 1\nworkflow:\n  block_new_error: false\n";

        let error = CodeIntelConfig::parse(source).unwrap_err();

        assert!(error.message().contains("unknown key in workflow"));
    }

    #[test]
    fn a_typo_in_a_security_key_is_rejected() {
        let source = "version: 1\nsecurity:\n  allow_public_binding: true\n";

        assert!(CodeIntelConfig::parse(source).is_err());
    }

    #[test]
    fn gates_can_be_turned_off_explicitly() {
        let source = "version: 1\nworkflow:\n  block_new_errors: false\n";

        let config = CodeIntelConfig::parse(source).unwrap();

        assert!(!config.workflow.block_new_errors);
        assert!(config.workflow.require_pre_change_impact);
    }

    #[test]
    fn a_non_boolean_flag_is_rejected() {
        let error = CodeIntelConfig::parse("version: 1\nenabled: yes\n").unwrap_err();

        assert!(error.message().contains("must be true or false"));
    }

    #[test]
    fn a_non_numeric_ttl_is_rejected() {
        let source = "version: 1\nworkspace:\n  idle_ttl_minutes: half-an-hour\n";

        let error = CodeIntelConfig::parse(source).unwrap_err();

        assert!(error.message().contains("must be a non-negative integer"));
    }

    #[test]
    fn an_unknown_isolation_mode_is_rejected() {
        let source = "version: 1\nworkspace:\n  isolation: process\n";

        let error = CodeIntelConfig::parse(source).unwrap_err();

        assert!(error
            .message()
            .contains("unknown workspace isolation: process"));
    }

    #[test]
    fn an_unknown_git_history_mode_is_rejected() {
        let source = "version: 1\ncontext:\n  include_git_history: everything\n";

        assert!(CodeIntelConfig::parse(source).is_err());
    }

    #[test]
    fn a_language_override_without_a_server_is_rejected() {
        let source = "version: 1\nlanguages:\n  overrides:\n    python: {}\n";

        let error = CodeIntelConfig::parse(source).unwrap_err();

        assert!(error
            .message()
            .contains("language override server is required"));
    }

    #[test]
    fn an_unknown_language_override_key_is_rejected() {
        let source = "version: 1\nlanguages:\n  overrides:\n    python: { binary: pyright }\n";

        assert!(CodeIntelConfig::parse(source).is_err());
    }

    #[test]
    fn a_section_of_the_wrong_shape_is_rejected() {
        let error = CodeIntelConfig::parse("version: 1\nbackend: agent-lsp\n").unwrap_err();

        assert!(error.message().contains("backend must be a mapping"));
    }

    #[test]
    fn malformed_yaml_is_rejected() {
        assert!(CodeIntelConfig::parse("version: 1\n  bad indent: [\n").is_err());
    }

    #[test]
    fn a_scalar_document_is_rejected() {
        let error = CodeIntelConfig::parse("just-a-string\n").unwrap_err();

        assert!(error.message().contains("root must be a mapping"));
    }

    #[test]
    fn an_http_backend_parses_with_its_mode_preserved() {
        let source = "version: 1\nbackend:\n  mode: http\n";

        let config = CodeIntelConfig::parse(source).unwrap();

        assert!(config.backend.mode.requires_authentication());
    }
}
