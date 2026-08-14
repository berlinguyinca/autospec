use super::*;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
const LINUX_EXECUTOR_REQUIRED: &str = "executor supervision requires Linux pidfd ownership";

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn require_linux_executor_supervision() -> Result<(), String> {
    Err(LINUX_EXECUTOR_REQUIRED.to_string())
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(crate) fn run_executor_bridge(
    _request: &ExecutorBridgeRequest,
) -> Result<BridgeRunReceipt, BridgeRunFailure> {
    require_linux_executor_supervision().map_err(BridgeRunFailure::from)?;
    unreachable!("non-Linux executor admission always fails")
}

#[cfg(target_os = "macos")]
pub(crate) fn run_executor_bridge(
    request: &ExecutorBridgeRequest,
) -> Result<BridgeRunReceipt, BridgeRunFailure> {
    let mut observe = |_| Ok(());
    super::run_executor_bridge_with_codex_probe_observed(
        request,
        preflight_codex_sandbox,
        &mut observe,
    )
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(super) fn reconcile_direct_launch(
    _paths: &DirectAttemptPaths,
    _expected_intent_body: Option<&str>,
) -> Result<bool, String> {
    require_linux_executor_supervision()?;
    unreachable!("non-Linux executor admission always fails")
}

#[cfg(target_os = "macos")]
pub(super) fn reconcile_direct_launch(
    paths: &DirectAttemptPaths,
    expected_intent_body: Option<&str>,
) -> Result<bool, String> {
    let intent = match fs::read_to_string(&paths.intent) {
        Ok(intent) => intent,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if paths.launch.exists()
                || direct_retirement_artifacts(paths)
                    .iter()
                    .any(|artifact| artifact.exists())
            {
                return Err(
                    "Darwin direct ownership artifacts exist without their private intent"
                        .to_string(),
                );
            }
            return Ok(false);
        }
        Err(error) => return Err(format!("read direct command intent: {error}")),
    };
    if expected_intent_body.is_some_and(|expected| expected != intent) {
        return Err("direct command intent differs from the requested argv".to_string());
    }
    let attempt_id = direct_intent_attempt_id(&paths.intent)?
        .ok_or_else(|| "direct command intent disappeared during reconciliation".to_string())?;
    let intent_digest = sha256_hex(intent.as_bytes());
    let Some((leader, process)) = read_direct_launch(&paths.launch, &attempt_id, &intent_digest)?
    else {
        return Ok(false);
    };
    if process.is_some() {
        return Err(
            "Darwin direct launch contains an unexpected second process identity".to_string(),
        );
    }
    match crate::commands::autonomous::platform_process::observe_expected(
        leader.pid,
        &leader.boot_id,
        &leader.start_identity,
    ) {
        crate::commands::autonomous::platform_process::ProcessObservation::Exact(birth)
            if birth.process_group == leader.process_group =>
        {
            super::darwin_supervisor::DarwinOwnedGroup::adopt(&leader, &paths.sinks)?
                .terminate()?;
        }
        crate::commands::autonomous::platform_process::ProcessObservation::Dead => {
            let durable_exit = match fs::metadata(&paths.sinks.exit_status) {
                Ok(_) => read_executor_exit_status(&paths.sinks.exit_status)?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(format!("inspect executor exit record: {error}")),
            };
            if durable_exit.is_none()
                || !super::darwin_supervisor::group_is_empty(leader.process_group)?
            {
                return Err(
                    "direct launch leader exited without durable whole-group completion"
                        .to_string(),
                );
            }
        }
        crate::commands::autonomous::platform_process::ProcessObservation::Exact(_)
        | crate::commands::autonomous::platform_process::ProcessObservation::Mismatch
        | crate::commands::autonomous::platform_process::ProcessObservation::Unknown(_) => {
            return Err("direct launch ownership is unverified".to_string());
        }
    }
    retire_direct_launch(paths, &attempt_id)?;
    Ok(true)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
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

#[cfg(target_os = "macos")]
pub(super) fn supervise_validated_harness_with_claim_renewal(
    state_path: &Path,
    event_log: &Path,
    state: &mut PersistedInvocation,
    harness: Option<&ValidatedInvocation>,
    snapshot: &MutationSnapshot,
    config: SupervisionConfig,
    renewal: ClaimRenewalSchedule,
) -> Result<SupervisionOutcome, String> {
    super::supervise_validated_harness_with_claim_renewal(
        state_path, event_log, state, harness, snapshot, config, renewal,
    )
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
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
    unreachable!("unsupported executor admission always fails")
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

#[cfg(all(test, target_os = "macos"))]
mod darwin_reconciliation_tests {
    use super::*;

    fn root(name: &str) -> PathBuf {
        let path = std::env::current_dir()
            .expect("current directory")
            .join("target/executor-bridge-tests")
            .join(format!("darwin-portability-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        ensure_private_directory(&path).expect("private test root");
        path
    }

    #[test]
    fn darwin_reconciliation_rejects_launch_without_private_intent() {
        let root = root("missing-intent");
        let paths = direct_attempt_paths(&root, 0);
        write_private_create_once(&paths.launch, b"{}", "orphan Darwin launch")
            .expect("orphan launch");
        let error = reconcile_direct_launch(&paths, None).expect_err("orphan launch rejected");
        assert!(error.contains("without their private intent"), "{error}");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn darwin_reconciliation_rejects_dead_group_without_fenced_exit() {
        let root = root("dead-no-exit");
        let paths = direct_attempt_paths(&root, 0);
        let attempt = reserve_direct_attempt_id(&paths).expect("attempt id");
        let intent = direct_intent_document(
            &attempt,
            &"a".repeat(40),
            None,
            Path::new("/bin/true"),
            &["true".to_string()],
        );
        write_private_create_once(&paths.intent, intent.as_bytes(), "Darwin direct intent")
            .expect("intent");
        let dead_pid = i32::MAX as u32;
        let boot_id =
            crate::commands::autonomous::platform_process::observe_birth(std::process::id())
                .expect("observe current boot")
                .expect("current process")
                .boot_id;
        let dead = ProcessIdentity {
            pid: dead_pid,
            process_group: dead_pid,
            executable: PathBuf::from("/bin/true"),
            argv_digest: argv_digest(&[]),
            boot_id,
            start_identity: "missing-start".to_string(),
        };
        let launch = direct_launch_document(&attempt, &sha256_hex(intent.as_bytes()), &dead, None);
        write_private_create_once(&paths.launch, launch.as_bytes(), "Darwin direct launch")
            .expect("launch");
        let error = reconcile_direct_launch(&paths, Some(&intent))
            .expect_err("unfenced dead group rejected");
        assert!(
            error.contains("without durable whole-group completion"),
            "{error}"
        );
        assert!(
            paths.launch.is_file(),
            "unproven launch must remain quarantined"
        );
        let _ = fs::remove_dir_all(root);
    }
}

#[cfg(all(test, not(any(target_os = "linux", target_os = "macos"))))]
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

#[cfg(all(test, target_os = "macos"))]
mod darwin_tests {
    #[test]
    fn executor_bridge_portability_admits_native_darwin_supervision() {
        crate::commands::autonomous::platform_process::ensure_autonomous_runtime_supported()
            .expect("Darwin native process identity is supported");
        assert!(!std::env::consts::OS.eq("linux"));
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
