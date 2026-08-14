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

#[cfg(any(target_os = "macos", target_os = "freebsd", windows))]
pub(super) fn interrupted_direct_terminal() -> AttemptTerminal {
    AttemptTerminal::CleanupFailed(
        "interrupted attempt was quarantined before terminal publication".to_string(),
    )
}

#[cfg(any(target_os = "macos", target_os = "freebsd", windows))]
pub(super) fn validate_platform_direct_quarantine(
    paths: &DirectAttemptPaths,
) -> Result<(), String> {
    let parent = paths
        .record
        .parent()
        .ok_or_else(|| "direct command record has no parent".to_string())?;
    let command = paths
        .record
        .file_stem()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "direct command record name is malformed".to_string())?;
    let prefix = format!("{command}.ownership-disproven-");
    for entry in fs::read_dir(parent)
        .map_err(|error| format!("inventory portable ownership quarantines: {error}"))?
    {
        let entry =
            entry.map_err(|error| format!("read portable ownership quarantine: {error}"))?;
        let name = entry.file_name();
        let Some(attempt_id) = name
            .to_str()
            .and_then(|name| name.strip_prefix(&prefix))
            .and_then(|name| name.strip_suffix(".json"))
        else {
            continue;
        };
        if !valid_direct_attempt_id(attempt_id) {
            return Err("portable ownership quarantine filename is malformed".to_string());
        }
        validate_private_state_file(&entry.path())
            .map_err(|error| format!("portable ownership quarantine is unsafe: {error}"))?;
        let body = fs::read_to_string(entry.path())
            .map_err(|error| format!("read portable ownership quarantine: {error}"))?;
        let value: serde_json::Value = serde_json::from_str(&body)
            .map_err(|error| format!("parse portable ownership quarantine: {error}"))?;
        let intent_digest = value
            .get("intent_digest")
            .and_then(serde_json::Value::as_str)
            .filter(|digest| valid_direct_attempt_id(digest))
            .ok_or_else(|| {
                "portable ownership quarantine intent digest is malformed".to_string()
            })?;
        if value.get("attempt_id").and_then(serde_json::Value::as_str) != Some(attempt_id) {
            return Err(
                "portable ownership quarantine attempt identity differs from its filename"
                    .to_string(),
            );
        }
        process_owner::DurableProcessOwner::from_document(&body, attempt_id, intent_digest)?;
    }
    Ok(())
}

#[cfg(any(target_os = "macos", target_os = "freebsd", windows))]
pub(super) fn reconcile_direct_launch(
    paths: &DirectAttemptPaths,
    expected_intent_body: Option<&str>,
) -> Result<bool, String> {
    if resume_direct_retirement(paths)? {
        return Ok(true);
    }
    let intent_body = match fs::read_to_string(&paths.intent) {
        Ok(body) => body,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if paths.launch.exists() {
                return Err("direct launch exists without its durable intent".to_string());
            }
            return Ok(false);
        }
        Err(error) => return Err(format!("read direct command intent: {error}")),
    };
    validate_private_state_file(&paths.intent)
        .map_err(|error| format!("direct command intent is unsafe: {error}"))?;
    if expected_intent_body.is_some_and(|expected| expected != intent_body) {
        return Err("direct command intent differs from the requested argv".to_string());
    }
    let attempt_id = direct_intent_attempt_id(&paths.intent)?
        .ok_or_else(|| "direct command intent disappeared during reconciliation".to_string())?;
    validate_direct_attempt_id_reservation(paths, &attempt_id)?;
    if paths.launch.is_file() {
        validate_private_state_file(&paths.launch)
            .map_err(|error| format!("direct launch record is unsafe: {error}"))?;
        let launch = fs::read_to_string(&paths.launch)
            .map_err(|error| format!("read direct launch record: {error}"))?;
        let owner = process_owner::DurableProcessOwner::from_document(
            &launch,
            &attempt_id,
            &sha256_hex(intent_body.as_bytes()),
        )?;
        if process_owner::recover_owner(&owner) != process_owner::RecoveryDisposition::Quarantine {
            return Err("reconstructed direct owner was not quarantined".to_string());
        }
        let marker = paths
            .record
            .with_extension(format!("ownership-disproven-{attempt_id}.json"));
        if !marker.exists() {
            write_private_create_once(
                &marker,
                launch.as_bytes(),
                "portable direct ownership quarantine",
            )?;
        }
        retire_direct_launch(paths, &attempt_id)?;
    }
    Ok(true)
}

#[cfg(any(target_os = "macos", target_os = "freebsd", windows))]
enum OwnedAttemptResult {
    Exited(std::process::ExitStatus),
    TimedOut(std::process::ExitStatus),
    OutputOverflow(std::process::ExitStatus),
    ReviewerLimit(std::process::ExitStatus),
}

#[cfg(any(target_os = "macos", target_os = "freebsd", windows))]
enum OwnedAttemptFailure {
    BeforeCleanup(String),
    ArtifactAfterCleanup(String),
    Cleanup(String),
}

#[cfg(any(target_os = "macos", target_os = "freebsd", windows))]
trait AttemptLifecycle {
    fn reviewer_at_limit(&self, capture: &ActiveReviewerCapture) -> Result<bool, String>;
    fn finalize_reviewer(&self, capture: &ActiveReviewerCapture) -> Result<bool, String>;
    fn bound_output(&self, paths: &DirectAttemptPaths) -> Result<(), String>;
}

#[cfg(any(target_os = "macos", target_os = "freebsd", windows))]
struct SystemAttemptLifecycle;

#[cfg(any(target_os = "macos", target_os = "freebsd", windows))]
impl AttemptLifecycle for SystemAttemptLifecycle {
    fn reviewer_at_limit(&self, capture: &ActiveReviewerCapture) -> Result<bool, String> {
        capture.at_limit()
    }

    fn finalize_reviewer(&self, capture: &ActiveReviewerCapture) -> Result<bool, String> {
        capture.finalize()
    }

    fn bound_output(&self, paths: &DirectAttemptPaths) -> Result<(), String> {
        bound_direct_output(paths)
    }
}

#[cfg(all(test, any(target_os = "macos", target_os = "freebsd")))]
struct InjectedAttemptLifecycle {
    fail_finalize: bool,
}

#[cfg(all(test, any(target_os = "macos", target_os = "freebsd")))]
impl InjectedAttemptLifecycle {
    fn fail_finalize() -> Self {
        Self {
            fail_finalize: true,
        }
    }
}

#[cfg(all(test, any(target_os = "macos", target_os = "freebsd")))]
impl AttemptLifecycle for InjectedAttemptLifecycle {
    fn reviewer_at_limit(&self, _capture: &ActiveReviewerCapture) -> Result<bool, String> {
        Ok(true)
    }

    fn finalize_reviewer(&self, _capture: &ActiveReviewerCapture) -> Result<bool, String> {
        if self.fail_finalize {
            Err("injected finalize failure".to_string())
        } else {
            Ok(false)
        }
    }

    fn bound_output(&self, paths: &DirectAttemptPaths) -> Result<(), String> {
        bound_direct_output(paths)
    }
}

#[cfg(any(target_os = "macos", target_os = "freebsd", windows))]
fn output_sizes(paths: &DirectAttemptPaths) -> Result<(u64, u64), String> {
    Ok((
        fs::metadata(&paths.stdout)
            .map_err(|error| format!("inspect direct stdout: {error}"))?
            .len(),
        fs::metadata(&paths.stderr)
            .map_err(|error| format!("inspect direct stderr: {error}"))?
            .len(),
    ))
}

#[cfg(any(target_os = "macos", target_os = "freebsd", windows))]
fn bound_direct_output(paths: &DirectAttemptPaths) -> Result<(), String> {
    for path in [&paths.stdout, &paths.stderr] {
        let file = OpenOptions::new()
            .write(true)
            .open(path)
            .map_err(|error| format!("open oversized direct output: {error}"))?;
        if file
            .metadata()
            .map_err(|error| format!("inspect oversized direct output: {error}"))?
            .len()
            > MAX_DIRECT_OUTPUT_BYTES
        {
            file.set_len(MAX_DIRECT_OUTPUT_BYTES)
                .map_err(|error| format!("bound direct output artifact: {error}"))?;
        }
    }
    Ok(())
}

#[cfg(any(target_os = "macos", target_os = "freebsd", windows))]
fn run_owned_attempt(
    owned: &mut process_owner::OwnedChildTree,
    paths: &DirectAttemptPaths,
    stall_timeout: Duration,
    reviewer_capture: Option<&ActiveReviewerCapture>,
) -> Result<OwnedAttemptResult, OwnedAttemptFailure> {
    run_owned_attempt_with_lifecycle(
        owned,
        paths,
        stall_timeout,
        reviewer_capture,
        &SystemAttemptLifecycle,
    )
}

#[cfg(any(target_os = "macos", target_os = "freebsd", windows))]
fn run_owned_attempt_with_lifecycle(
    owned: &mut process_owner::OwnedChildTree,
    paths: &DirectAttemptPaths,
    stall_timeout: Duration,
    reviewer_capture: Option<&ActiveReviewerCapture>,
    lifecycle: &impl AttemptLifecycle,
) -> Result<OwnedAttemptResult, OwnedAttemptFailure> {
    let mut last_progress = Instant::now();
    let mut observed = output_sizes(paths).map_err(OwnedAttemptFailure::BeforeCleanup)?;
    loop {
        if let Some(capture) = reviewer_capture {
            if lifecycle
                .reviewer_at_limit(capture)
                .map_err(OwnedAttemptFailure::BeforeCleanup)?
            {
                let status = owned.terminate().map_err(OwnedAttemptFailure::Cleanup)?;
                lifecycle
                    .finalize_reviewer(capture)
                    .map_err(OwnedAttemptFailure::ArtifactAfterCleanup)?;
                return Ok(OwnedAttemptResult::ReviewerLimit(status));
            }
        }
        if let Some(status) = owned
            .try_wait()
            .map_err(OwnedAttemptFailure::BeforeCleanup)?
        {
            if let Some(capture) = reviewer_capture {
                if lifecycle
                    .finalize_reviewer(capture)
                    .map_err(OwnedAttemptFailure::ArtifactAfterCleanup)?
                {
                    return Ok(OwnedAttemptResult::ReviewerLimit(status));
                }
            }
            return Ok(OwnedAttemptResult::Exited(status));
        }
        let sizes = output_sizes(paths).map_err(OwnedAttemptFailure::BeforeCleanup)?;
        if sizes.0 > MAX_DIRECT_OUTPUT_BYTES || sizes.1 > MAX_DIRECT_OUTPUT_BYTES {
            let status = owned.terminate().map_err(OwnedAttemptFailure::Cleanup)?;
            lifecycle
                .bound_output(paths)
                .map_err(OwnedAttemptFailure::ArtifactAfterCleanup)?;
            return Ok(OwnedAttemptResult::OutputOverflow(status));
        }
        if sizes != observed {
            observed = sizes;
            last_progress = Instant::now();
        } else if last_progress.elapsed() >= stall_timeout {
            return owned
                .terminate()
                .map(OwnedAttemptResult::TimedOut)
                .map_err(OwnedAttemptFailure::Cleanup);
        }
        thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(any(target_os = "macos", target_os = "freebsd", windows))]
fn finish_owned_attempt(
    owned: &mut process_owner::OwnedChildTree,
    result: Result<OwnedAttemptResult, OwnedAttemptFailure>,
) -> AttemptTerminal {
    match result {
        Ok(OwnedAttemptResult::Exited(status)) => {
            AttemptTerminal::Exited(status.code().unwrap_or(1))
        }
        Ok(OwnedAttemptResult::TimedOut(_status)) => AttemptTerminal::TimedOut,
        Ok(OwnedAttemptResult::OutputOverflow(_status)) => AttemptTerminal::OutputOverflow,
        Ok(OwnedAttemptResult::ReviewerLimit(_status)) => AttemptTerminal::Exited(70),
        Err(OwnedAttemptFailure::ArtifactAfterCleanup(error)) => {
            AttemptTerminal::InfrastructureFailed(error)
        }
        Err(OwnedAttemptFailure::Cleanup(error)) => AttemptTerminal::CleanupFailed(error),
        Err(OwnedAttemptFailure::BeforeCleanup(error)) => match owned.terminate() {
            Ok(_) => AttemptTerminal::InfrastructureFailed(error),
            Err(cleanup) => AttemptTerminal::CleanupFailed(format!("{error}; {cleanup}")),
        },
    }
}

#[allow(clippy::too_many_arguments)]
#[cfg(any(target_os = "macos", target_os = "freebsd", windows))]
pub(super) fn execute_supervised_direct_attempt(
    attempt_id: &str,
    worktree: &Path,
    program: &Path,
    direct: &DirectCommand,
    paths: &DirectAttemptPaths,
    stdout: &File,
    stderr: &File,
    environment_overrides: Vec<(OsString, OsString)>,
    intent_digest: &str,
    stall_timeout: Duration,
) -> AttemptTerminal {
    #[cfg(unix)]
    use std::os::unix::process::CommandExt;

    let reviewer_capture = match direct
        .review_capture
        .as_ref()
        .map(ActiveReviewerCapture::open)
        .transpose()
    {
        Ok(capture) => capture,
        Err(error) => return AttemptTerminal::InfrastructureFailed(error),
    };
    let mut command = Command::new(program);
    #[cfg(unix)]
    command.arg0(&direct.argv[0]);
    command
        .args(&direct.argv[1..])
        .current_dir(worktree)
        .envs(environment_overrides)
        .stdin(Stdio::null());
    let stdout = match stdout.try_clone() {
        Ok(stdout) => stdout,
        Err(error) => {
            return AttemptTerminal::InfrastructureFailed(format!("clone direct stdout: {error}"))
        }
    };
    let stderr = match stderr.try_clone() {
        Ok(stderr) => stderr,
        Err(error) => {
            return AttemptTerminal::InfrastructureFailed(format!("clone direct stderr: {error}"))
        }
    };
    #[cfg(unix)]
    command
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    #[cfg(unix)]
    let spawn = process_owner::OwnedChildTree::spawn(&mut command, attempt_id.to_string());
    #[cfg(windows)]
    let spawn = {
        use std::os::windows::io::AsRawHandle;
        let stdin = File::open("NUL").map_err(|error| format!("open Windows null input: {error}"));
        match stdin {
            Ok(stdin) => process_owner::OwnedChildTree::spawn_with_stdio(
                &mut command,
                attempt_id.to_string(),
                Some(stdin.as_raw_handle()),
                Some(stdout.as_raw_handle()),
                Some(stderr.as_raw_handle()),
            ),
            Err(error) => Err(error),
        }
    };
    let mut owned = match spawn {
        Ok(owned) => owned,
        Err(error) => return AttemptTerminal::SpawnFailed(error),
    };
    let launch = owned.identity().document(attempt_id, intent_digest);
    if let Err(error) = write_private_create_once(
        &paths.launch,
        launch.as_bytes(),
        "portable direct command launch identity",
    ) {
        return match owned.terminate() {
            Ok(_) => AttemptTerminal::InfrastructureFailed(error),
            Err(cleanup) => AttemptTerminal::CleanupFailed(format!("{error}; {cleanup}")),
        };
    }
    let result = run_owned_attempt(&mut owned, paths, stall_timeout, reviewer_capture.as_ref());
    finish_owned_attempt(&mut owned, result)
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

#[cfg(all(test, any(target_os = "macos", target_os = "freebsd")))]
mod tests {
    use super::*;

    struct DirectFixture {
        root: PathBuf,
        worktree: PathBuf,
        artifacts: PathBuf,
    }

    impl DirectFixture {
        fn new(name: &str) -> Self {
            let root = fs::canonicalize(std::env::temp_dir())
                .expect("canonicalize fixture temp directory")
                .join(format!(
                    "autospec-portable-direct-{name}-{}-{}",
                    std::process::id(),
                    DIRECT_TRANSACTION_SEQUENCE.fetch_add(1, Ordering::Relaxed)
                ));
            let worktree = root.join("worktree");
            let artifacts = root.join("artifacts");
            fs::create_dir_all(&worktree).expect("create direct fixture");
            for args in [
                &["init", "--quiet"][..],
                &["config", "user.email", "autospec@example.invalid"][..],
                &["config", "user.name", "Autospec Test"][..],
                &["commit", "--quiet", "--allow-empty", "-m", "fixture"][..],
            ] {
                let status = Command::new("git")
                    .args(args)
                    .current_dir(&worktree)
                    .status()
                    .expect("run fixture git command");
                assert!(status.success(), "fixture git command failed: {args:?}");
            }
            Self {
                root,
                worktree,
                artifacts,
            }
        }
    }

    impl Drop for DirectFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn execute_direct_plan_uses_owned_process_group() {
        let fixture = DirectFixture::new("success");
        let plan = DirectCommandPlan {
            commands: vec![DirectCommand::success(vec![
                "/bin/sh".to_string(),
                "-c".to_string(),
                "printf portable-output".to_string(),
            ])],
        };
        let observed = execute_direct_plan(
            &fixture.worktree,
            &plan,
            &fixture.artifacts,
            None,
            Duration::from_secs(2),
        )
        .expect("execute portable direct plan");
        assert_eq!(observed.len(), 1);
        assert_eq!(observed[0].terminal, AttemptTerminal::Exited(0));
        assert_eq!(
            fs::read_to_string(&observed[0].stdout_path).expect("read portable stdout"),
            "portable-output"
        );
    }

    #[test]
    fn execute_direct_plan_terminates_stalled_process_group() {
        let fixture = DirectFixture::new("stall");
        let plan = DirectCommandPlan {
            commands: vec![DirectCommand::success(vec![
                "/bin/sh".to_string(),
                "-c".to_string(),
                "sleep 30 & wait".to_string(),
            ])],
        };
        let error = execute_direct_plan(
            &fixture.worktree,
            &plan,
            &fixture.artifacts,
            None,
            Duration::from_millis(100),
        )
        .expect_err("stalled portable direct plan must fail");
        assert!(error.contains("stalled"), "unexpected error: {error}");
    }

    #[test]
    fn reconcile_direct_launch_quarantines_without_signalling_recorded_pid() {
        let fixture = DirectFixture::new("reconcile");
        ensure_private_directory(&fixture.artifacts).expect("create artifact root");
        let artifacts = fs::canonicalize(&fixture.artifacts).expect("canonicalize artifact root");
        let paths = direct_attempt_paths(&artifacts, 0);
        let attempt_id = reserve_direct_attempt_id(&paths).expect("reserve attempt identity");
        let commit_oid = git_stdout(
            &fixture.worktree,
            &["rev-parse", "--verify", "HEAD^{commit}"],
        )
        .expect("read fixture commit");
        let executable = Path::new("/bin/true");
        let argv = vec!["/bin/true".to_string()];
        let intent = direct_intent_document(&attempt_id, &commit_oid, None, executable, &argv);
        write_private_create_once(
            &paths.intent,
            intent.as_bytes(),
            "portable reconciliation fixture intent",
        )
        .expect("write fixture intent");
        let (boot_id, process_start) = process_birth_identity(std::process::id())
            .expect("observe current process")
            .expect("current process is live");
        let owner = process_owner::DurableProcessOwner {
            pid: std::process::id(),
            container_id: std::process::id().to_string(),
            process_start: format!("{boot_id}:{process_start}"),
            launch_nonce: attempt_id.clone(),
        };
        write_private_create_once(
            &paths.launch,
            owner
                .document(&attempt_id, &sha256_hex(intent.as_bytes()))
                .as_bytes(),
            "portable reconciliation fixture launch",
        )
        .expect("write fixture launch");

        assert!(reconcile_direct_launch(&paths, None).expect("reconcile portable launch"));
        assert!(
            process_birth_identity(std::process::id())
                .expect("re-observe current process")
                .is_some(),
            "reconciliation must not signal the recorded PID"
        );
        assert!(
            !paths.launch.exists(),
            "live-authority launch was not retired"
        );
        assert!(
            paths
                .record
                .with_extension(format!("ownership-disproven-{attempt_id}.json"))
                .is_file(),
            "durable quarantine marker was not published"
        );
    }

    #[test]
    fn finalize_failure_after_termination_does_not_cleanup_twice() {
        let fixture = DirectFixture::new("finalize-failure");
        ensure_private_directory(&fixture.artifacts).expect("create artifact root");
        let paths = direct_attempt_paths(&fixture.artifacts, 0);
        File::create(&paths.stdout).expect("create stdout");
        File::create(&paths.stderr).expect("create stderr");
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "sleep 30 & wait"]);
        let mut owned = process_owner::OwnedChildTree::spawn(&mut command, "finalize".into())
            .expect("spawn finalize fixture");
        let capture = ActiveReviewerCapture {
            artifacts: Vec::new(),
        };
        let result = run_owned_attempt_with_lifecycle(
            &mut owned,
            &paths,
            Duration::from_secs(1),
            Some(&capture),
            &InjectedAttemptLifecycle::fail_finalize(),
        );
        assert_eq!(
            finish_owned_attempt(&mut owned, result),
            AttemptTerminal::InfrastructureFailed("injected finalize failure".to_string())
        );
        assert!(
            owned
                .try_wait()
                .expect("read cached terminal state")
                .is_some(),
            "post-cleanup failure lost the reaped terminal state"
        );
        owned
            .terminate()
            .expect("idempotent cleanup must not signal after terminal state");
    }

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
