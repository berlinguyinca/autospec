use super::*;

#[cfg(not(target_os = "linux"))]
const LINUX_EXECUTOR_REQUIRED: &str = "executor supervision requires Linux pidfd ownership";

#[cfg(not(target_os = "linux"))]
fn require_linux_executor_supervision() -> Result<(), String> {
    Err(LINUX_EXECUTOR_REQUIRED.to_string())
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn run_executor_bridge(
    _request: &ExecutorBridgeRequest,
) -> Result<BridgeRunReceipt, BridgeRunFailure> {
    require_linux_executor_supervision().map_err(BridgeRunFailure::from)?;
    unreachable!("non-Linux executor admission always fails")
}

#[cfg(not(target_os = "linux"))]
pub(super) fn reconcile_direct_launch(
    _paths: &DirectAttemptPaths,
    _expected_intent_body: Option<&str>,
) -> Result<bool, String> {
    require_linux_executor_supervision()?;
    unreachable!("non-Linux executor admission always fails")
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn execute_direct_plan(
    _worktree: &Path,
    _plan: &DirectCommandPlan,
    _artifact_root: &Path,
    _runtime: Option<&DirectRuntimeAdapter>,
    _stall_timeout: Duration,
) -> Result<Vec<ObservedDirectCommand>, String> {
    require_linux_executor_supervision()?;
    unreachable!("non-Linux executor admission always fails")
}

#[cfg(not(target_os = "linux"))]
pub(super) fn create_draft_pull_request<Refresh>(
    _state_path: &Path,
    _state: &mut PersistedInvocation,
    _body: &str,
    _issue_title: &str,
    _base: &str,
    _adapter: &DraftPrAdapter,
    _refresh: &mut Refresh,
) -> Result<(), BridgeRunFailure>
where
    Refresh: FnMut() -> Result<BridgeClaimOwnership, BridgeRunFailure>,
{
    require_linux_executor_supervision().map_err(BridgeRunFailure::from)?;
    unreachable!("non-Linux executor admission always fails")
}

#[cfg(not(target_os = "linux"))]
pub(super) fn supervise_validated_harness_with_claim_renewal(
    _state_path: &Path,
    _event_log: &Path,
    _state: &mut PersistedInvocation,
    _harness: Option<&ValidatedInvocation>,
    _snapshot: &MutationSnapshot,
    _config: SupervisionConfig,
    _renewal: ClaimRenewalSchedule,
) -> Result<SupervisionOutcome, String> {
    require_linux_executor_supervision()?;
    unreachable!("non-Linux executor admission always fails")
}

pub(super) fn resolve_executor_supervisor_executable(
    current_executable: Result<PathBuf, String>,
    argv_zero: Option<&OsStr>,
) -> Result<PathBuf, String> {
    let primary_error = match current_executable {
        Ok(path) => match fs::canonicalize(&path) {
            Ok(canonical) => return Ok(canonical),
            Err(error) => format!("canonicalize executor supervisor executable: {error}"),
        },
        Err(error) => error,
    };
    let fallback = argv_zero
        .map(Path::new)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| {
            format!(
                "{primary_error}; executor supervisor argv-zero fallback is not an absolute path"
            )
        })?;
    let _canonical = fs::canonicalize(fallback).map_err(|error| {
        format!(
            "{primary_error}; canonicalize executor supervisor argv-zero fallback {}: {error}",
            fallback.display()
        )
    })?;
    #[cfg(target_os = "linux")]
    {
        let running = fs::metadata("/proc/self/exe").map_err(|error| {
            format!("{primary_error}; inspect running executor supervisor image: {error}")
        })?;
        let candidate = fs::metadata(&_canonical).map_err(|error| {
            format!(
                "{primary_error}; inspect executor supervisor argv-zero fallback {}: {error}",
                _canonical.display()
            )
        })?;
        if running.dev() != candidate.dev() || running.ino() != candidate.ino() {
            return Err(format!(
                "{primary_error}; executor supervisor argv-zero fallback does not identify the running image"
            ));
        }
        Ok(_canonical)
    }
    #[cfg(not(target_os = "linux"))]
    Err(format!(
        "{primary_error}; executor supervisor argv-zero fallback cannot prove running-image identity on this platform"
    ))
}

#[cfg(all(test, not(target_os = "linux")))]
mod tests {
    use super::*;

    #[test]
    fn executor_bridge_fails_closed_before_state_mutation_without_linux_pidfds() {
        assert_eq!(
            require_linux_executor_supervision().unwrap_err(),
            "executor supervision requires Linux pidfd ownership"
        );
    }

    #[test]
    fn executor_bridge_keeps_harness_alias_parsing_portable() {
        let aliases = HarnessConfig::parse_alias_table("codex\tcodex\t--yolo\tCodex CLI\n")
            .expect("parse portable harness alias");
        assert_eq!(aliases.len(), 1);
        assert_eq!(aliases[0].kind, HarnessKind::Codex);
    }

    #[test]
    fn executor_bridge_keeps_primary_supervisor_resolution_portable() {
        let executable = std::env::current_exe().expect("current test executable");
        let resolved = resolve_executor_supervisor_executable(Ok(executable.clone()), None)
            .expect("resolve primary executable");
        assert_eq!(
            resolved,
            fs::canonicalize(executable).expect("canonical test executable")
        );
    }
}

#[cfg(all(test, not(any(target_os = "linux", target_os = "macos"))))]
mod autonomous_runtime_support_tests {
    use crate::commands::autonomous;
    use std::ffi::OsString;
    use std::path::Path;
    use std::sync::Mutex;

    static ENVIRONMENT: Mutex<()> = Mutex::new(());

    struct Environment {
        previous: Vec<(&'static str, Option<OsString>)>,
    }

    impl Environment {
        fn set(values: &[(&'static str, &Path)]) -> Self {
            let previous = values
                .iter()
                .map(|(key, value)| {
                    let previous = std::env::var_os(key);
                    // SAFETY: the unsupported-platform tests serialize all environment mutation.
                    unsafe { std::env::set_var(key, value) };
                    (*key, previous)
                })
                .collect();
            Self { previous }
        }
    }

    impl Drop for Environment {
        fn drop(&mut self) {
            for (key, value) in self.previous.drain(..).rev() {
                // SAFETY: the unsupported-platform tests serialize all environment mutation.
                unsafe {
                    match value {
                        Some(value) => std::env::set_var(key, value),
                        None => std::env::remove_var(key),
                    }
                }
            }
        }
    }

    fn launch_arguments(command: &str, repo_dir: &Path, dry_run: bool) -> Vec<String> {
        let mut arguments = vec![
            command.to_string(),
            "--repo".to_string(),
            "owner/repo".to_string(),
            "--repo-dir".to_string(),
            repo_dir.display().to_string(),
        ];
        if dry_run {
            arguments.push("--dry-run".to_string());
        }
        arguments
    }

    #[test]
    fn unsupported_platform_rejects_mutating_launches_before_artifact_creation() {
        let _serial = ENVIRONMENT.lock().expect("lock environment");
        let root = std::env::temp_dir().join(format!(
            "autospec-unsupported-runtime-{}",
            std::process::id()
        ));
        let repo_dir = root.join("repo");
        std::fs::create_dir_all(&repo_dir).expect("create valid repository directory");
        let operator = root.join("operator");
        let logs = root.join("logs");
        let claims = root.join("claims");
        let heartbeats = root.join("heartbeats");
        let _environment = Environment::set(&[
            ("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator),
            ("AUTOSPEC_AUTONOMOUS_LOG_DIR", &logs),
            ("AUTOSPEC_CLAIM_GIT_STATE_DIR", &claims),
            ("AUTOSPEC_HEARTBEAT_DIR", &heartbeats),
        ]);
        let fixtures = [
            operator.clone(),
            operator.join("owner_repo"),
            operator.join("owner_repo/lifecycle.json"),
            operator.join("owner_repo/accountability.json"),
            operator.join("owner_repo/conductor.pid"),
            logs.clone(),
            logs.join("owner_repo"),
            claims.clone(),
            heartbeats.clone(),
            repo_dir.join(".autospec"),
        ];
        for command in ["start", "restart", "resume"] {
            let error = autonomous::run(&launch_arguments(command, &repo_dir, false))
                .expect_err("unsupported mutating launch must fail");
            assert!(error.message.contains("requires Linux or macOS"));
            assert!(fixtures.iter().all(|fixture| !fixture.exists()));
        }

        for command in ["start", "restart", "resume"] {
            autonomous::run(&launch_arguments(command, &repo_dir, true))
                .expect("unsupported dry-run preview remains available");
            assert!(fixtures.iter().all(|fixture| !fixture.exists()));
        }
        std::fs::remove_dir_all(root).expect("remove unsupported-platform fixture");
    }
}
