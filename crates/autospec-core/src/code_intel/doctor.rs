use serde::Serialize;

use super::config::CodeIntelConfig;
use super::error::CodeIntelError;
use super::fallback::FallbackChain;
use super::language::{DetectedLanguage, Language};
use super::schema::{Operation, ResultSource};
use super::workspace::WorkspaceRegistry;

/// One line of the `autospec doctor code-intel` report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DoctorCheck {
    pub name: String,
    pub status: String,
    pub detail: String,
}

impl DoctorCheck {
    pub fn ok(name: impl Into<String>, detail: impl Into<String>) -> Self {
        Self::new(name, "ok", detail)
    }

    pub fn warn(name: impl Into<String>, detail: impl Into<String>) -> Self {
        Self::new(name, "warn", detail)
    }

    pub fn fail(name: impl Into<String>, detail: impl Into<String>) -> Self {
        Self::new(name, "fail", detail)
    }

    fn new(name: impl Into<String>, status: &str, detail: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status: status.to_string(),
            detail: detail.into(),
        }
    }

    pub fn is_failure(&self) -> bool {
        self.status == "fail"
    }
}

/// The machine-readable body of `autospec doctor code-intel`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DoctorReport {
    pub command: String,
    pub status: String,
    pub backend: String,
    pub backend_version: String,
    pub mode: String,
    pub enabled: bool,
    pub languages: Vec<DetectedLanguage>,
    pub workspaces: usize,
    pub checks: Vec<DoctorCheck>,
}

impl DoctorReport {
    pub fn to_json_string(&self) -> Result<String, CodeIntelError> {
        serde_json::to_string(self).map_err(|error| CodeIntelError::config(error.to_string()))
    }

    pub fn is_healthy(&self) -> bool {
        self.status == "ok"
    }

    /// Human-readable rendering for a terminal.
    pub fn to_text(&self) -> String {
        let mut lines = vec![format!(
            "AutoSpec code intelligence: {} ({} {}, mode {})",
            self.status, self.backend, self.backend_version, self.mode
        )];
        for check in &self.checks {
            lines.push(format!(
                "  [{}] {} — {}",
                check.status, check.name, check.detail
            ));
        }
        lines.join("\n")
    }
}

/// Which binaries the host actually has. The caller probes the filesystem or
/// `PATH`; assembly stays pure so it is testable without those binaries.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HostProbe {
    /// Server binaries found on the host, by binary name.
    pub available_servers: Vec<String>,
    /// Fallback tools found on the host (`ast-grep`, `rg`).
    pub available_fallback_tools: Vec<String>,
    /// Whether the configured backend binary was found.
    pub backend_present: bool,
    /// The backend version string reported by the binary, when it ran.
    pub backend_version: Option<String>,
}

impl HostProbe {
    fn has_server(&self, server: &str) -> bool {
        self.available_servers.iter().any(|found| found == server)
    }

    fn has_tool(&self, tool: &str) -> bool {
        self.available_fallback_tools
            .iter()
            .any(|found| found == tool)
    }
}

/// Assemble the doctor report from configuration, detection and a host probe.
pub fn report(
    config: &CodeIntelConfig,
    languages: &[DetectedLanguage],
    registry: &WorkspaceRegistry,
    probe: &HostProbe,
) -> DoctorReport {
    let mut checks = vec![enabled_check(config), backend_check(config, probe)];
    checks.extend(language_checks(languages, probe));
    checks.push(fallback_check(config, probe));
    checks.push(security_check(config));
    checks.push(smoke_check(config, probe));
    let status = if checks.iter().any(DoctorCheck::is_failure) {
        "blocked"
    } else {
        "ok"
    };
    DoctorReport {
        command: "doctor code-intel".to_string(),
        status: status.to_string(),
        backend: config.backend.kind.clone(),
        backend_version: probe
            .backend_version
            .clone()
            .unwrap_or_else(|| "unknown".to_string()),
        mode: config.backend.mode.as_str().to_string(),
        enabled: config.enabled,
        languages: languages.to_vec(),
        workspaces: registry.active().len(),
        checks,
    }
}

fn enabled_check(config: &CodeIntelConfig) -> DoctorCheck {
    if config.enabled {
        DoctorCheck::ok("enabled", "code intelligence is enabled")
    } else {
        DoctorCheck::warn(
            "enabled",
            "code intelligence is disabled; every query falls back to search",
        )
    }
}

fn backend_check(config: &CodeIntelConfig, probe: &HostProbe) -> DoctorCheck {
    if probe.backend_present {
        return DoctorCheck::ok(
            "backend",
            format!("{} found on PATH", config.backend.binary),
        );
    }
    DoctorCheck::fail(
        "backend",
        format!(
            "{} not found; install it or set backend.binary",
            config.backend.binary
        ),
    )
}

fn language_checks(languages: &[DetectedLanguage], probe: &HostProbe) -> Vec<DoctorCheck> {
    if languages.is_empty() {
        return vec![DoctorCheck::warn(
            "languages",
            "no supported language detected in this worktree",
        )];
    }
    languages
        .iter()
        .map(|detected| language_check(detected, probe))
        .collect()
}

fn language_check(detected: &DetectedLanguage, probe: &HostProbe) -> DoctorCheck {
    let name = format!("language:{}", detected.language);
    if probe.has_server(&detected.server) {
        return DoctorCheck::ok(
            name,
            format!("{} available for {}", detected.server, detected.language),
        );
    }
    DoctorCheck::warn(
        name,
        format!(
            "{} not found; {} queries fall back to structural search",
            detected.server, detected.language
        ),
    )
}

fn fallback_check(config: &CodeIntelConfig, probe: &HostProbe) -> DoctorCheck {
    let chain = FallbackChain::resolve(Operation::References, &config.fallback);
    let missing: Vec<String> = [ResultSource::AstGrep, ResultSource::Ripgrep]
        .iter()
        .filter_map(|source| chain.tool_for(*source, &config.fallback))
        .filter(|tool| !probe.has_tool(tool))
        .collect();
    if missing.is_empty() {
        return DoctorCheck::ok("fallback", "structural and textual fallbacks available");
    }
    DoctorCheck::warn(
        "fallback",
        format!("missing fallback tool(s): {}", missing.join(", ")),
    )
}

fn security_check(config: &CodeIntelConfig) -> DoctorCheck {
    if config.backend.mode.requires_authentication() && config.security.allow_public_bind {
        return DoctorCheck::fail(
            "security",
            "http mode with allow_public_bind exposes the backend publicly",
        );
    }
    let build_script_languages: Vec<&str> = Language::all()
        .into_iter()
        .filter(|language| language.server_runs_build_scripts())
        .map(Language::config_key)
        .collect();
    if config.security.trust_project_build_scripts {
        return DoctorCheck::warn(
            "security",
            format!(
                "project build scripts are trusted; server startup may execute project code ({})",
                build_script_languages.join(", ")
            ),
        );
    }
    DoctorCheck::ok(
        "security",
        "local binding only; project build scripts are untrusted",
    )
}

/// Readiness of the definition/reference smoke probe.
///
/// The report states whether the probe *can* run, not that it returned results:
/// claiming a passing semantic query without a bound transport would be exactly
/// the unverified claim the rest of this gateway exists to prevent.
fn smoke_check(config: &CodeIntelConfig, probe: &HostProbe) -> DoctorCheck {
    if !config.enabled {
        return DoctorCheck::warn("smoke", "skipped: code intelligence is disabled");
    }
    let probes = format!(
        "{} and {}",
        Operation::Definition.as_api_name(),
        Operation::References.as_api_name()
    );
    if !probe.backend_present {
        return DoctorCheck::warn(
            "smoke",
            format!("skipped: {probes} need a reachable backend"),
        );
    }
    DoctorCheck::ok("smoke", format!("{probes} probes are ready to run"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe() -> HostProbe {
        HostProbe {
            available_servers: vec!["rust-analyzer".to_string()],
            available_fallback_tools: vec!["ast-grep".to_string(), "rg".to_string()],
            backend_present: true,
            backend_version: Some("0.1.0".to_string()),
        }
    }

    fn rust_language() -> DetectedLanguage {
        DetectedLanguage {
            language: "rust".to_string(),
            server: "rust-analyzer".to_string(),
            evidence: vec!["Cargo.toml".to_string()],
            source_files: 12,
        }
    }

    fn report_with(config: &CodeIntelConfig, probe: &HostProbe) -> DoctorReport {
        report(config, &[rust_language()], &WorkspaceRegistry::new(), probe)
    }

    #[test]
    fn a_fully_provisioned_host_reports_ok() {
        let report = report_with(&CodeIntelConfig::default(), &probe());

        assert!(report.is_healthy());
        assert_eq!(report.backend, "agent-lsp");
        assert_eq!(report.backend_version, "0.1.0");
        assert_eq!(report.languages.len(), 1);
    }

    #[test]
    fn a_missing_backend_blocks_the_report() {
        let probe = HostProbe {
            backend_present: false,
            ..probe()
        };

        let report = report_with(&CodeIntelConfig::default(), &probe);

        assert!(!report.is_healthy());
        assert_eq!(report.status, "blocked");
    }

    #[test]
    fn a_missing_language_server_warns_rather_than_blocks() {
        let probe = HostProbe {
            available_servers: Vec::new(),
            ..probe()
        };

        let report = report_with(&CodeIntelConfig::default(), &probe);

        assert!(report.is_healthy());
        let check = report
            .checks
            .iter()
            .find(|check| check.name == "language:rust")
            .unwrap();
        assert_eq!(check.status, "warn");
        assert!(check.detail.contains("fall back"));
    }

    #[test]
    fn missing_fallback_tools_are_reported() {
        let probe = HostProbe {
            available_fallback_tools: vec!["rg".to_string()],
            ..probe()
        };

        let report = report_with(&CodeIntelConfig::default(), &probe);

        let check = report
            .checks
            .iter()
            .find(|check| check.name == "fallback")
            .unwrap();
        assert_eq!(check.status, "warn");
        assert!(check.detail.contains("ast-grep"));
    }

    #[test]
    fn no_detected_language_warns() {
        let report = report(
            &CodeIntelConfig::default(),
            &[],
            &WorkspaceRegistry::new(),
            &probe(),
        );

        let check = report
            .checks
            .iter()
            .find(|check| check.name == "languages")
            .unwrap();
        assert_eq!(check.status, "warn");
    }

    #[test]
    fn public_binding_over_http_blocks_the_report() {
        let mut config = CodeIntelConfig::default();
        config.backend.mode = super::super::config::BackendMode::Http;
        config.security.allow_public_bind = true;

        let report = report_with(&config, &probe());

        assert_eq!(report.status, "blocked");
    }

    #[test]
    fn trusting_build_scripts_warns_and_names_the_languages() {
        let config = CodeIntelConfig {
            security: super::super::config::SecurityConfig {
                trust_project_build_scripts: true,
                ..Default::default()
            },
            ..Default::default()
        };

        let report = report_with(&config, &probe());

        let check = report
            .checks
            .iter()
            .find(|check| check.name == "security")
            .unwrap();
        assert_eq!(check.status, "warn");
        assert!(check.detail.contains("java"));
        assert!(check.detail.contains("scala"));
    }

    #[test]
    fn disabling_code_intelligence_skips_the_smoke_probe() {
        let config = CodeIntelConfig {
            enabled: false,
            ..Default::default()
        };

        let report = report_with(&config, &probe());

        let check = report
            .checks
            .iter()
            .find(|check| check.name == "smoke")
            .unwrap();
        assert_eq!(check.status, "warn");
        assert!(check.detail.contains("disabled"));
        assert!(report.is_healthy());
    }

    #[test]
    fn the_smoke_probe_is_skipped_when_no_backend_is_reachable() {
        let probe = HostProbe {
            backend_present: false,
            ..probe()
        };

        let report = report_with(&CodeIntelConfig::default(), &probe);
        let check = report
            .checks
            .iter()
            .find(|check| check.name == "smoke")
            .unwrap();

        assert_eq!(check.status, "warn");
        assert!(check.detail.contains("need a reachable backend"));
    }

    #[test]
    fn the_report_renders_as_text_and_json() {
        let report = report_with(&CodeIntelConfig::default(), &probe());

        let text = report.to_text();
        let json = report.to_json_string().unwrap();

        assert!(text.starts_with("AutoSpec code intelligence: ok (agent-lsp 0.1.0, mode local)"));
        assert!(text.contains("[ok] backend"));
        assert!(json.contains("\"command\":\"doctor code-intel\""));
    }

    #[test]
    fn an_unrun_backend_reports_an_unknown_version() {
        let probe = HostProbe {
            backend_version: None,
            ..probe()
        };

        let report = report_with(&CodeIntelConfig::default(), &probe);

        assert_eq!(report.backend_version, "unknown");
    }
}
