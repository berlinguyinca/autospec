// executor_bridge/harness/codex_sandbox.rs — how a Codex invocation is sandboxed.
//
// Split from harness.rs, which had two concerns in it: which harness runs, and — for one of
// the three — the bwrap preflight, permission profile and per-phase network policy that
// decides how it is confined. Harness selection says nothing about sandboxing and sandboxing
// says nothing about selection.

use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CodexSandboxPolicy {
    Default,
    NetworkPermissionProfile,
}

pub(super) const CODEX_NETWORK_PERMISSION_PROFILE: &str = "autospec-network-executor";

impl CodexSandboxPolicy {
    pub(super) fn invocation_args(self, codex: &Path) -> Result<Vec<String>, String> {
        match self {
            Self::Default => Ok(Vec::new()),
            Self::NetworkPermissionProfile => {
                let mut args = vec!["--ignore-user-config".into()];
                args.extend(self.permission_profile_args(codex)?);
                Ok(args)
            }
        }
    }

    pub(super) fn permission_profile_args(self, codex: &Path) -> Result<Vec<String>, String> {
        match self {
            Self::Default => Ok(Vec::new()),
            Self::NetworkPermissionProfile => {
                let codex = codex
                    .to_str()
                    .ok_or_else(|| "validated Codex executable path must be UTF-8".to_string())?;
                let codex = codex.replace('\\', "\\\\").replace('"', "\\\"");
                let mut filesystem = format!(
                    "permissions.{CODEX_NETWORK_PERMISSION_PROFILE}.filesystem={{\
                     \":minimal\"=\"read\",\
                     \":workspace_roots\"={{\".\"=\"write\"}},\
                     \"{codex}\"=\"read\",\
                     \"~/.aws\"=\"deny\",\
                     \"~/.azure\"=\"deny\",\
                     \"~/.cargo/credentials\"=\"deny\",\
                     \"~/.cargo/credentials.toml\"=\"deny\",\
                     \"~/.codex/archived_sessions\"=\"deny\",\
                     \"~/.codex/auth.json\"=\"deny\",\
                     \"~/.codex/config.toml\"=\"deny\",\
                     \"~/.codex/history.jsonl\"=\"deny\",\
                     \"~/.codex/sessions\"=\"deny\",\
                     \"~/.codex/shell_snapshots\"=\"deny\",\
                     \"~/.config/containers\"=\"deny\",\
                     \"~/.config/gcloud\"=\"deny\",\
                     \"~/.config/gh\"=\"deny\",\
                     \"~/.config/pip\"=\"deny\",\
                     \"~/.docker\"=\"deny\",\
                     \"~/.git-credentials\"=\"deny\",\
                     \"~/.gnupg\"=\"deny\",\
                     \"~/.gradle\"=\"deny\",\
                     \"~/.kube\"=\"deny\",\
                     \"~/.m2\"=\"deny\",\
                     \"~/.netrc\"=\"deny\",\
                     \"~/.npmrc\"=\"deny\",\
                     \"~/.pypirc\"=\"deny\",\
                     \"~/.ssh\"=\"deny\",\
                     \"~/.terraform.d\"=\"deny\",\
                     \"~/.vault-token\"=\"deny\""
                );
                if let Some(codex_home) = std::env::var_os("CODEX_HOME") {
                    let codex_home = PathBuf::from(codex_home);
                    if !codex_home.is_absolute() {
                        return Err(
                            "CODEX_HOME must be absolute for executor credential isolation"
                                .to_string(),
                        );
                    }
                    let default_codex_home =
                        std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".codex"));
                    if default_codex_home.as_ref() != Some(&codex_home) {
                        for sensitive_path in [
                            "archived_sessions",
                            "auth.json",
                            "config.toml",
                            "history.jsonl",
                            "sessions",
                            "shell_snapshots",
                        ] {
                            let sensitive_path = codex_home.join(sensitive_path);
                            let sensitive_path = sensitive_path.to_str().ok_or_else(|| {
                                "CODEX_HOME must be UTF-8 for executor credential isolation"
                                    .to_string()
                            })?;
                            let sensitive_path =
                                sensitive_path.replace('\\', "\\\\").replace('"', "\\\"");
                            filesystem.push_str(&format!(",\"{sensitive_path}\"=\"deny\""));
                        }
                    }
                }
                filesystem.push('}');
                Ok(vec![
                    "-c".into(),
                    format!(
                        "default_permissions=\"{CODEX_NETWORK_PERMISSION_PROFILE}\""
                    ),
                    "-c".into(),
                    filesystem,
                    "-c".into(),
                    format!(
                        "permissions.{CODEX_NETWORK_PERMISSION_PROFILE}.network.enabled=true"
                    ),
                    "-c".into(),
                    "shell_environment_policy.inherit=\"all\"".into(),
                    "-c".into(),
                    "shell_environment_policy.ignore_default_excludes=false".into(),
                    "-c".into(),
                    "shell_environment_policy.exclude=[\"AWS_*\",\"AZURE_*\",\"CODEX_API_KEY\",\"DOCKER_*\",\"GH_*\",\"GITHUB_*\",\"GOOGLE_*\",\"KUBE*\",\"NPM_*\",\"OPENAI_API_KEY\",\"SSH_*\",\"VAULT_*\",\"*TOKEN*\",\"*SECRET*\",\"*PASSWORD*\",\"*API_KEY*\",\"*CREDENTIAL*\"]".into(),
                ])
            }
        }
    }
}

const CODEX_BWRAP_LOOPBACK_FAILURE: &str =
    "bwrap: loopback: Failed RTM_NEWADDR: Operation not permitted";

#[cfg(target_os = "linux")]
const CODEX_BWRAP_UID_MAP_FAILURES: [&str; 2] = [
    "bwrap: setting up uid map: Permission denied",
    "bwrap: setting up uid map: Operation not permitted",
];

#[cfg(target_os = "linux")]
pub(super) fn preflight_codex_sandbox(codex: &Path) -> Result<CodexSandboxPolicy, String> {
    let policy = CodexSandboxPolicy::NetworkPermissionProfile;
    let worktree = std::env::current_dir()
        .map_err(|error| format!("resolve Codex sandbox probe worktree: {error}"))?;
    let profile_args = policy.permission_profile_args(codex)?;
    let output = Command::new(codex)
        .arg("sandbox")
        .arg("-C")
        .arg(&worktree)
        .arg("-P")
        .arg(CODEX_NETWORK_PERMISSION_PROFILE)
        .args(profile_args)
        .args(["--", "/bin/true"])
        .output()
        .map_err(|error| {
            format!("executor_codex_sandbox_unavailable: cannot run Codex sandbox probe: {error}")
        })?;
    if output.status.success() {
        return Ok(policy);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stderr = stderr.trim();
    let host_setup_failure = stderr.lines().map(str::trim).find(|line| {
        *line == CODEX_BWRAP_LOOPBACK_FAILURE || CODEX_BWRAP_UID_MAP_FAILURES.contains(line)
    });
    if let Some(host_setup_failure) = host_setup_failure {
        return Err(format!(
            "executor_codex_sandbox_host_setup_required: {host_setup_failure}; install the supported bwrap AppArmor profile tracked by installer issue #2619 (https://github.com/berlinguyinca/autospec/issues/2619)"
        ));
    }
    Err(format!(
        "executor_codex_sandbox_unavailable: Codex permission-profile probe failed: {stderr}",
    ))
}

#[cfg(not(target_os = "linux"))]
pub(super) fn preflight_codex_sandbox(_codex: &Path) -> Result<CodexSandboxPolicy, String> {
    Ok(CodexSandboxPolicy::Default)
}

pub(super) fn configure_codex_sandbox_for_phase(
    harness: &mut ResolvedHarness,
    phase: BridgePhase,
    probe: impl FnOnce(&Path) -> Result<CodexSandboxPolicy, String>,
) -> Result<(), String> {
    if harness.kind == HarnessKind::Codex
        && matches!(
            phase,
            BridgePhase::Pending | BridgePhase::Implementing | BridgePhase::Interrupted
        )
    {
        harness.codex_sandbox = probe(&harness.executable)?;
    }
    Ok(())
}
