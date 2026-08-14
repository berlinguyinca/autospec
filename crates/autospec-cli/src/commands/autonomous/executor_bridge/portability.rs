use super::*;

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;

    static TEST_ENVIRONMENT: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct DirectFixture {
        root: PathBuf,
        worktree: PathBuf,
        artifacts: PathBuf,
    }

    impl DirectFixture {
        fn new() -> Self {
            let root = fs::canonicalize(std::env::temp_dir())
                .expect("canonicalize fixture temp directory")
                .join(format!(
                    "autospec-windows-direct-{}-{}",
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
                assert!(Command::new("git")
                    .args(args)
                    .current_dir(&worktree)
                    .status()
                    .expect("run fixture git command")
                    .success());
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

    struct RestoreEnvironment {
        path: Option<OsString>,
        pathext: Option<OsString>,
    }

    impl Drop for RestoreEnvironment {
        fn drop(&mut self) {
            match self.path.take() {
                Some(value) => std::env::set_var("PATH", value),
                None => std::env::remove_var("PATH"),
            }
            match self.pathext.take() {
                Some(value) => std::env::set_var("PATHEXT", value),
                None => std::env::remove_var("PATHEXT"),
            }
        }
    }

    #[test]
    fn direct_plan_resolves_pathext_wrapper_through_canonical_cmd() {
        let _environment = TEST_ENVIRONMENT
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let fixture = DirectFixture::new();
        let wrapper = fixture.worktree.join("autospec-fixture-tool.CMD");
        fs::write(&wrapper, "@echo wrapper-output\r\n").expect("write command wrapper");
        let restore = RestoreEnvironment {
            path: std::env::var_os("PATH"),
            pathext: std::env::var_os("PATHEXT"),
        };
        let mut search = vec![fixture.worktree.clone()];
        search.extend(std::env::split_paths(
            &restore.path.clone().unwrap_or_default(),
        ));
        std::env::set_var(
            "PATH",
            std::env::join_paths(search).expect("join fixture PATH"),
        );
        std::env::set_var("PATHEXT", ".CMD;.EXE");

        let plan = DirectCommandPlan {
            commands: vec![DirectCommand::success(vec![
                "autospec-fixture-tool".to_string()
            ])],
        };
        let observed = execute_direct_plan(
            &fixture.worktree,
            &plan,
            &fixture.artifacts,
            None,
            Duration::from_secs(5),
        )
        .expect("execute Windows direct wrapper plan");
        drop(restore);

        assert_eq!(observed[0].terminal, AttemptTerminal::Exited(0));
        assert!(fs::read_to_string(&observed[0].stdout_path)
            .expect("read wrapper stdout")
            .contains("wrapper-output"));
        assert_eq!(
            observed[0]
                .process_executable
                .file_name()
                .and_then(OsStr::to_str),
            Some("cmd.exe")
        );
        assert_eq!(&observed[0].process_argv[1..4], ["/d", "/s", "/c"]);
        assert!(observed[0].process_argv[4].contains(
            fs::canonicalize(wrapper)
                .expect("canonicalize wrapper")
                .to_str()
                .expect("UTF-8 wrapper path")
        ));
    }
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
    let reviewer_capture = match direct
        .review_capture
        .as_ref()
        .map(ActiveReviewerCapture::open)
        .transpose()
    {
        Ok(capture) => capture,
        Err(error) => return AttemptTerminal::InfrastructureFailed(error),
    };
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
    #[cfg(windows)]
    let null_input = "NUL";
    #[cfg(unix)]
    let null_input = "/dev/null";
    let spawn = File::open(null_input)
        .map_err(|error| format!("open null input: {error}"))
        .and_then(|stdin| {
            process_owner::OwnedChildTree::spawn_prepared(
                process_owner::PreparedLaunchSpec::inherited(
                    program.to_path_buf(),
                    direct.argv.iter().map(OsString::from).collect(),
                    Some(worktree.to_path_buf()),
                    environment_overrides,
                    Some(stdin),
                    Some(stdout),
                    Some(stderr),
                ),
                attempt_id.to_string(),
            )
        });
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
    state_path: &Path,
    state: &mut PersistedInvocation,
    body: &str,
    issue_title: &str,
    base: &str,
    adapter: &DraftPrAdapter,
    refresh: &mut Refresh,
) -> Result<(), BridgeRunFailure>
where
    Refresh: FnMut() -> Result<BridgeClaimOwnership, BridgeRunFailure>,
{
    if refresh()? == BridgeClaimOwnership::Lost {
        return Err(BridgeRunFailure::ownership_lost(
            "executor draft creation lost exact claim ownership",
        ));
    }
    let body_path = state_path.with_file_name(format!(
        "draft-body-{}-{}.md",
        state.identity.invocation_id,
        INVOCATION_WRITE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    write_private_create_once(&body_path, body.as_bytes(), "executor draft body")?;
    let executable = resolve_draft_executable(adapter)?;
    let args = vec![
        "pr".into(),
        "create".into(),
        "--repo".into(),
        state.identity.repository.clone(),
        "--draft".into(),
        "--head".into(),
        state.identity.branch.clone(),
        "--base".into(),
        base.into(),
        "--title".into(),
        issue_title.into(),
        "--body-file".into(),
        body_path.to_string_lossy().into_owned(),
    ];
    let null = if cfg!(windows) { "NUL" } else { "/dev/null" };
    let stderr_path = state_path.with_extension("draft.stderr");
    let spawn = process_owner::OwnedChildTree::spawn_prepared(
        process_owner::PreparedLaunchSpec::inherited(
            executable.clone(),
            std::iter::once(executable.into_os_string())
                .chain(args.iter().map(OsString::from))
                .collect(),
            Some(state.identity.worktree.clone()),
            adapter.environment.clone().into_iter().collect(),
            Some(File::open(null).map_err(|error| format!("open draft null input: {error}"))?),
            Some(
                File::create(state_path.with_extension("draft.stdout"))
                    .map_err(|error| format!("create draft stdout: {error}"))?,
            ),
            Some(
                File::create(&stderr_path)
                    .map_err(|error| format!("create draft stderr: {error}"))?,
            ),
        ),
        format!("draft-{}", state.identity.invocation_id),
    );
    let mut owned = match spawn {
        Ok(owned) => owned,
        Err(error) => {
            let _ = fs::remove_file(&body_path);
            return Err(BridgeRunFailure::transient(format!(
                "launch executor draft pull request: {error}"
            )));
        }
    };
    let status = owned.wait();
    let _ = fs::remove_file(&body_path);
    let status = status.map_err(|error| {
        BridgeRunFailure::transient(format!("wait for executor draft pull request: {error}"))
    })?;
    if status.success() {
        Ok(())
    } else {
        let stderr = fs::read_to_string(&stderr_path).unwrap_or_default();
        Err(BridgeRunFailure::transient(format!(
            "create executor draft pull request failed: {}",
            stderr.trim()
        )))
    }
}

#[cfg(not(target_os = "linux"))]
pub(super) fn supervise_validated_harness_with_claim_renewal(
    state_path: &Path,
    event_log: &Path,
    state: &mut PersistedInvocation,
    harness: Option<&ValidatedInvocation>,
    snapshot: &MutationSnapshot,
    config: SupervisionConfig,
    mut renewal: ClaimRenewalSchedule,
) -> Result<SupervisionOutcome, String> {
    if config.stall_timeout.is_zero() || config.poll_interval.is_zero() {
        return Err("executor supervision intervals must be non-zero".to_string());
    }
    let sinks = output_sink_paths_for_state(state_path, state)?;
    if state.supervisor.is_some() || state.process.is_some() || sinks.supervisor_identity.exists() {
        state.phase = BridgePhase::Interrupted;
        state.progress_at = unix_now()?;
        write_invocation_atomic(state_path, state)?;
        append_executor_event(
            event_log,
            state,
            "child_recovery_required",
            Some(serde_json::json!({
                "reason": "durable process identity has no live in-process owner; ownership quarantined without signalling"
            })),
        )?;
        return Err(
            "executor ownership is quarantined; portable recovery cannot adopt a PID".to_string(),
        );
    }
    if renewal.is_enabled() {
        match refresh_bridge_claim(state, ClaimRenewalPhase::Implementation) {
            Ok(BridgeClaimOwnership::Refreshed { ttl_seconds }) => {
                renewal.mark_refreshed(ttl_seconds)
            }
            Ok(BridgeClaimOwnership::Lost) => {
                record_claim_ownership_loss(state_path, event_log, state)?;
                return Ok(SupervisionOutcome::OwnershipLost);
            }
            Err(error) => return Ok(SupervisionOutcome::TransientFailure(error)),
        }
    }
    let harness = harness.ok_or_else(|| {
        "executor recovery exhausted durable identities before fresh harness resolution".to_string()
    })?;
    state.phase = BridgePhase::Pending;
    state.supervisor = None;
    state.process = None;
    state.progress_at = unix_now()?;
    write_invocation_atomic(state_path, state)?;
    let stdout =
        File::create(&sinks.stdout).map_err(|error| format!("create executor stdout: {error}"))?;
    let stderr =
        File::create(&sinks.stderr).map_err(|error| format!("create executor stderr: {error}"))?;
    let null = if cfg!(windows) { "NUL" } else { "/dev/null" };
    let mut argv = Vec::with_capacity(harness.args.len() + 1);
    argv.push(
        harness
            .argv_zero
            .clone()
            .unwrap_or_else(|| harness.program.clone().into_os_string()),
    );
    argv.extend(harness.args.iter().map(OsString::from));
    let mut owned = process_owner::OwnedChildTree::spawn_prepared(
        process_owner::PreparedLaunchSpec::inherited(
            harness.program.clone(),
            argv,
            Some(harness.current_dir.clone()),
            harness.environment_overrides.clone(),
            Some(File::open(null).map_err(|error| format!("open executor null input: {error}"))?),
            Some(stdout),
            Some(stderr),
        ),
        state.identity.invocation_id.clone(),
    )?;
    let owner = owned.identity();
    write_private_create_once(
        &sinks.supervisor_identity,
        owner
            .document(&state.identity.invocation_id, &argv_digest(&harness.args))
            .as_bytes(),
        "portable executor owner",
    )?;
    state.phase = BridgePhase::Implementing;
    state.progress_at = unix_now()?;
    write_invocation_atomic(state_path, state)?;
    append_executor_event(event_log, state, "child_started", None)?;
    let mut observed = (0, 0);
    let mut last_progress = Instant::now();
    loop {
        thread::sleep(config.poll_interval);
        if let Some(status) = owned.try_wait()? {
            let _ = fs::remove_file(&sinks.supervisor_identity);
            snapshot.verify(&state.identity.repository_path, &state.identity.branch)?;
            let exit_code = status.code().unwrap_or(1);
            state.phase = if exit_code == 0 {
                BridgePhase::ImplementationComplete
            } else {
                BridgePhase::Interrupted
            };
            state.progress_at = unix_now()?;
            write_invocation_atomic(state_path, state)?;
            append_executor_event(
                event_log,
                state,
                "child_exited",
                Some(serde_json::json!({"exit_code": exit_code, "adopted": false})),
            )?;
            return Ok(SupervisionOutcome::Exited { exit_code });
        }
        let sizes = (
            fs::metadata(&sinks.stdout).map(|m| m.len()).unwrap_or(0),
            fs::metadata(&sinks.stderr).map(|m| m.len()).unwrap_or(0),
        );
        if sizes != observed {
            observed = sizes;
            last_progress = Instant::now();
            state.progress_at = unix_now()?;
            write_invocation_atomic(state_path, state)?;
        }
        if renewal.is_due() {
            match refresh_bridge_claim(state, ClaimRenewalPhase::Implementation) {
                Ok(BridgeClaimOwnership::Refreshed { ttl_seconds }) => {
                    renewal.mark_refreshed(ttl_seconds)
                }
                Ok(BridgeClaimOwnership::Lost) => {
                    owned.terminate()?;
                    let _ = fs::remove_file(&sinks.supervisor_identity);
                    record_claim_ownership_loss(state_path, event_log, state)?;
                    return Ok(SupervisionOutcome::OwnershipLost);
                }
                Err(error) => {
                    owned.terminate()?;
                    return Ok(SupervisionOutcome::TransientFailure(error));
                }
            }
        }
        if last_progress.elapsed() >= config.stall_timeout {
            owned.terminate()?;
            let _ = fs::remove_file(&sinks.supervisor_identity);
            state.phase = BridgePhase::Interrupted;
            state.progress_at = unix_now()?;
            write_invocation_atomic(state_path, state)?;
            append_executor_event(event_log, state, "child_stalled", None)?;
            return Ok(SupervisionOutcome::Stalled);
        }
    }
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
