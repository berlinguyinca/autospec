mod parse;
mod yaml;

use std::collections::BTreeMap;

use serde::Serialize;

use super::error::CodeIntelError;

/// Default path of the operator-owned gateway configuration.
pub const CONFIG_PATH: &str = ".autospec/code-intelligence.yaml";

/// The only schema version this build understands.
pub const SUPPORTED_VERSION: &str = "1";

/// How the backend process is reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum BackendMode {
    /// Supervised child processes on the same host as the worktree (v1 default).
    Local,
    /// Supervised inside the AutoSpec execution container.
    Container,
    /// A process-separated backend reached over HTTP.
    Http,
}

impl BackendMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Container => "container",
            Self::Http => "http",
        }
    }

    pub fn parse(value: &str) -> Result<Self, CodeIntelError> {
        match value {
            "local" => Ok(Self::Local),
            "container" => Ok(Self::Container),
            "http" => Ok(Self::Http),
            other => Err(CodeIntelError::config(format!(
                "unknown backend mode: {other}"
            ))),
        }
    }

    /// HTTP is the only mode that opens a socket, so it is the only one that
    /// must be authenticated before use.
    pub fn requires_authentication(self) -> bool {
        matches!(self, Self::Http)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BackendConfig {
    pub kind: String,
    pub mode: BackendMode,
    pub binary: String,
}

impl Default for BackendConfig {
    fn default() -> Self {
        Self {
            kind: "agent-lsp".to_string(),
            mode: BackendMode::Local,
            binary: "agent-lsp".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkspaceConfig {
    pub isolation: String,
    pub idle_ttl_minutes: u64,
    pub warm_cache: bool,
}

impl Default for WorkspaceConfig {
    fn default() -> Self {
        Self {
            isolation: "worktree".to_string(),
            idle_ttl_minutes: 30,
            warm_cache: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LanguagesConfig {
    pub auto_detect: bool,
    /// Language config key -> server binary name.
    pub overrides: BTreeMap<String, String>,
}

impl Default for LanguagesConfig {
    fn default() -> Self {
        Self {
            auto_detect: true,
            overrides: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FallbackConfig {
    pub structural: Option<String>,
    pub textual: Option<String>,
}

impl Default for FallbackConfig {
    fn default() -> Self {
        Self {
            structural: Some("ast-grep".to_string()),
            textual: Some("rg".to_string()),
        }
    }
}

/// The mandatory semantic gates. Each defaults to on: a gate that defaults off
/// would let a task silently skip the analysis it claims to have run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkflowConfig {
    pub require_pre_change_impact: bool,
    pub require_post_change_diagnostics: bool,
    pub reviewer_independent_analysis: bool,
    pub block_new_errors: bool,
}

impl Default for WorkflowConfig {
    fn default() -> Self {
        Self {
            require_pre_change_impact: true,
            require_post_change_diagnostics: true,
            reviewer_independent_analysis: true,
            block_new_errors: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContextConfig {
    pub prefer_semantic: bool,
    pub include_related_tests: bool,
    pub include_rag: bool,
    pub include_git_history: String,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            prefer_semantic: true,
            include_related_tests: true,
            include_rag: true,
            include_git_history: "targeted".to_string(),
        }
    }
}

/// Fail-closed security posture: nothing binds publicly and no build script runs
/// until an operator opts in.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SecurityConfig {
    pub allow_public_bind: bool,
    pub trust_project_build_scripts: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CodeIntelConfig {
    pub version: String,
    pub enabled: bool,
    pub backend: BackendConfig,
    pub workspace: WorkspaceConfig,
    pub languages: LanguagesConfig,
    pub fallback: FallbackConfig,
    pub workflow: WorkflowConfig,
    pub context: ContextConfig,
    pub security: SecurityConfig,
}

impl Default for CodeIntelConfig {
    fn default() -> Self {
        Self {
            version: SUPPORTED_VERSION.to_string(),
            enabled: true,
            backend: BackendConfig::default(),
            workspace: WorkspaceConfig::default(),
            languages: LanguagesConfig::default(),
            fallback: FallbackConfig::default(),
            workflow: WorkflowConfig::default(),
            context: ContextConfig::default(),
            security: SecurityConfig::default(),
        }
    }
}

impl CodeIntelConfig {
    /// Parse an operator configuration. Unknown keys are rejected rather than
    /// ignored so a typo in a security or workflow gate cannot silently
    /// disable it.
    pub fn parse(source: &str) -> Result<Self, CodeIntelError> {
        parse::parse(source)
    }

    pub fn to_json_string(&self) -> Result<String, CodeIntelError> {
        serde_json::to_string(self).map_err(|error| CodeIntelError::config(error.to_string()))
    }

    pub fn idle_ttl_minutes(&self) -> u64 {
        self.workspace.idle_ttl_minutes
    }

    /// Whether this configuration would expose the backend on a public
    /// interface. Refused unless the operator explicitly allowed it.
    pub fn rejects_public_bind(&self) -> bool {
        !self.security.allow_public_bind
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_documented_posture() {
        let config = CodeIntelConfig::default();

        assert!(config.enabled);
        assert_eq!(config.backend.kind, "agent-lsp");
        assert_eq!(config.backend.mode, BackendMode::Local);
        assert_eq!(config.workspace.isolation, "worktree");
        assert_eq!(config.idle_ttl_minutes(), 30);
        assert!(config.rejects_public_bind());
        assert!(!config.security.trust_project_build_scripts);
    }

    #[test]
    fn every_workflow_gate_defaults_on() {
        let workflow = WorkflowConfig::default();

        assert!(workflow.require_pre_change_impact);
        assert!(workflow.require_post_change_diagnostics);
        assert!(workflow.reviewer_independent_analysis);
        assert!(workflow.block_new_errors);
    }

    #[test]
    fn only_http_mode_requires_authentication() {
        assert!(BackendMode::Http.requires_authentication());
        assert!(!BackendMode::Local.requires_authentication());
        assert!(!BackendMode::Container.requires_authentication());
    }

    #[test]
    fn backend_modes_round_trip() {
        for mode in [
            BackendMode::Local,
            BackendMode::Container,
            BackendMode::Http,
        ] {
            assert_eq!(BackendMode::parse(mode.as_str()).unwrap(), mode);
        }
    }

    #[test]
    fn unknown_backend_mode_is_rejected() {
        assert!(BackendMode::parse("remote").is_err());
    }

    #[test]
    fn config_serializes_for_the_doctor_report() {
        let json = CodeIntelConfig::default().to_json_string().unwrap();

        assert!(json.contains("\"mode\":\"local\""));
        assert!(json.contains("\"block_new_errors\":true"));
    }
}
