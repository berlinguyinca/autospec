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

const SENSITIVE_HOME_SUFFIXES: [&str; 26] = [
    ".aws",
    ".azure",
    ".cargo/credentials",
    ".cargo/credentials.toml",
    ".codex/archived_sessions",
    ".codex/auth.json",
    ".codex/config.toml",
    ".codex/history.jsonl",
    ".codex/sessions",
    ".codex/shell_snapshots",
    ".config/containers",
    ".config/gcloud",
    ".config/gh",
    ".config/pip",
    ".docker",
    ".git-credentials",
    ".gnupg",
    ".gradle",
    ".kube",
    ".m2",
    ".netrc",
    ".npmrc",
    ".pypirc",
    ".ssh",
    ".terraform.d",
    ".vault-token",
];

fn toml_basic_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\u{0008}' => escaped.push_str("\\b"),
            '\t' => escaped.push_str("\\t"),
            '\n' => escaped.push_str("\\n"),
            '\u{000C}' => escaped.push_str("\\f"),
            '\r' => escaped.push_str("\\r"),
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            character if character <= '\u{001F}' || character == '\u{007F}' => {
                escaped.push_str(&format!("\\u{:04X}", character as u32));
            }
            character => escaped.push(character),
        }
    }
    escaped
}

fn validated_permission_roots(variable: &str, root: &Path) -> Result<Vec<PathBuf>, String> {
    if !root.is_absolute() {
        return Err(format!(
            "{variable} must be absolute for executor credential isolation"
        ));
    }
    if root
        .components()
        .any(|component| component == Component::ParentDir)
    {
        return Err(format!(
            "{variable} must not contain parent-directory components for executor credential isolation"
        ));
    }
    root.to_str()
        .ok_or_else(|| format!("{variable} must be UTF-8 for executor credential isolation"))?;

    let mut roots = vec![root.to_path_buf()];
    match fs::canonicalize(root) {
        Ok(canonical) => {
            canonical.to_str().ok_or_else(|| {
                format!("canonical {variable} must be UTF-8 for executor credential isolation")
            })?;
            if canonical != root {
                roots.push(canonical);
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "canonicalize {variable} for executor credential isolation: {error}"
            ));
        }
    }
    Ok(roots)
}

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
                let home = std::env::var_os("HOME").ok_or_else(|| {
                    "HOME must be set for executor credential isolation".to_string()
                })?;
                let codex_home = std::env::var_os("CODEX_HOME").map(PathBuf::from);
                self.permission_profile_args_for_roots(
                    codex,
                    Path::new(&home),
                    codex_home.as_deref(),
                )
            }
        }
    }

    pub(super) fn permission_profile_args_for_roots(
        self,
        codex: &Path,
        home: &Path,
        codex_home: Option<&Path>,
    ) -> Result<Vec<String>, String> {
        match self {
            Self::Default => Ok(Vec::new()),
            Self::NetworkPermissionProfile => {
                let codex = codex
                    .to_str()
                    .ok_or_else(|| "validated Codex executable path must be UTF-8".to_string())?;
                let codex = toml_basic_string(codex);
                let mut filesystem = format!(
                    "permissions.{CODEX_NETWORK_PERMISSION_PROFILE}.filesystem={{\
                     \":minimal\"=\"read\",\
                     \":workspace_roots\"={{\".\"=\"write\"}},\
                     \"{codex}\"=\"read\""
                );

                let mut denied = BTreeSet::new();
                for suffix in SENSITIVE_HOME_SUFFIXES {
                    denied.insert(format!("~/{suffix}"));
                }
                for home in validated_permission_roots("HOME", home)? {
                    for suffix in SENSITIVE_HOME_SUFFIXES {
                        let path = home.join(suffix);
                        let path = path.to_str().ok_or_else(|| {
                            "HOME credential path must be UTF-8 for executor credential isolation"
                                .to_string()
                        })?;
                        denied.insert(path.to_string());
                    }
                }
                if let Some(codex_home) = codex_home {
                    for codex_home in validated_permission_roots("CODEX_HOME", codex_home)? {
                        for suffix in SENSITIVE_HOME_SUFFIXES {
                            if let Some(suffix) = suffix.strip_prefix(".codex/") {
                                let path = codex_home.join(suffix);
                                let path = path.to_str().ok_or_else(|| {
                                    "CODEX_HOME credential path must be UTF-8 for executor credential isolation"
                                        .to_string()
                                })?;
                                denied.insert(path.to_string());
                            }
                        }
                    }
                }
                for path in denied {
                    filesystem.push_str(&format!(",\"{}\"=\"deny\"", toml_basic_string(&path)));
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
        .args(["--", "/usr/bin/true"])
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
