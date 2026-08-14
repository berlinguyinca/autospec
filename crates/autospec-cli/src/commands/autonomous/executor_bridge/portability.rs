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

    #[test]
    fn windows_executor_worktree_root_is_canonical_and_provisionable() {
        // Break caught: `/tmp/autospec-executor` is root-relative on Windows and cannot match
        // the canonical drive-qualified path persisted in bridge identity.
        let _environment = TEST_ENVIRONMENT
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let fixture = DirectFixture::new();
        let remote = fixture.root.join("remote.git");
        assert!(Command::new("git")
            .args(["init", "--bare", remote.to_str().expect("UTF-8 remote")])
            .status()
            .expect("initialize Windows fixture remote")
            .success());
        for args in [
            vec!["branch", "-M", "main"],
            vec![
                "remote",
                "add",
                "origin",
                remote.to_str().expect("UTF-8 remote"),
            ],
            vec!["push", "--quiet", "-u", "origin", "main"],
        ] {
            assert!(Command::new("git")
                .args(&args)
                .current_dir(&fixture.worktree)
                .status()
                .expect("prepare Windows worktree fixture")
                .success());
        }
        let root = executor_worktree_root();
        assert!(root.is_absolute(), "executor root must be drive-qualified");
        assert!(root.starts_with(
            fs::canonicalize(std::env::temp_dir()).expect("canonical Windows temp directory")
        ));
        harden_executor_worktree_root(&fixture.worktree, &root)
            .expect("provision canonical Windows executor root");
        assert_eq!(
            fs::canonicalize(&root).expect("canonical provisioned executor root"),
            root
        );
        let base = resolve_base(&fixture.worktree, &BTreeMap::new())
            .expect("resolve Windows fixture base");
        let scope = format!(
            "windows-native-{}-{}",
            std::process::id(),
            DIRECT_TRANSACTION_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        );
        let issue = provision_issue_worktree_for_claim(
            &fixture.worktree,
            &scope,
            42,
            &base,
            Some(("claim-windows", "invocation-windows")),
        )
        .expect("provision issue worktree through the Windows bridge path");
        assert!(issue.path.is_absolute());
        assert!(issue.path.starts_with(&root));
        assert_eq!(
            fs::canonicalize(&issue.path).expect("canonical Windows issue worktree"),
            issue.path
        );
    }
}

#[cfg(all(test, not(target_os = "linux")))]
mod supported_host_tests {
    use super::*;

    static ADMISSION_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());
    const CHILD_ENV: &str = "AUTOSPEC_TEST_SUPPORTED_HOST_NOOP_CHILD";

    struct RestoreEnvironment(Vec<(&'static str, Option<OsString>)>);

    impl Drop for RestoreEnvironment {
        fn drop(&mut self) {
            for (key, value) in self.0.drain(..) {
                match value {
                    Some(value) => unsafe { std::env::set_var(key, value) },
                    None => unsafe { std::env::remove_var(key) },
                }
            }
        }
    }

    fn git(directory: &Path, args: &[&str]) {
        assert!(
            Command::new("git")
                .args(args)
                .current_dir(directory)
                .status()
                .expect("run admission fixture git")
                .success(),
            "git command failed: {args:?}"
        );
    }

    #[test]
    fn supported_host_retires_predecessor_runs_noop_and_publishes_terminal_receipt() {
        if std::env::var_os(CHILD_ENV).is_some() {
            thread::sleep(Duration::from_millis(100));
            return;
        }
        // Break caught: a supported non-Linux host compiling the bridge while skipping released
        // predecessor retirement, successor heartbeat ownership, real OS child ownership, or
        // terminal receipt publication.
        let _serial = ADMISSION_TEST
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let root = fs::canonicalize(std::env::temp_dir())
            .expect("canonical temp directory")
            .join(format!(
                "autospec-supported-host-admission-{}-{}",
                std::process::id(),
                DIRECT_TRANSACTION_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
        let repo = root.join("repo");
        let remote = root.join("remote.git");
        let heartbeat = root.join("heartbeats");
        let claim_state = root.join("claim-state");
        fs::create_dir_all(&repo).expect("create admission repo");
        fs::create_dir_all(&heartbeat).expect("create heartbeat root");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&heartbeat, fs::Permissions::from_mode(0o700))
                .expect("secure heartbeat root");
        }
        assert!(Command::new("git")
            .args(["init", "--bare", remote.to_str().expect("UTF-8 remote")])
            .status()
            .expect("initialize bare claim remote")
            .success());
        git(&repo, &["init", "--quiet"]);
        git(&repo, &["config", "user.email", "autospec@example.invalid"]);
        git(&repo, &["config", "user.name", "Autospec Test"]);
        git(
            &repo,
            &["commit", "--quiet", "--allow-empty", "-m", "fixture"],
        );
        git(&repo, &["branch", "-M", "main"]);
        git(
            &repo,
            &[
                "remote",
                "add",
                "origin",
                remote.to_str().expect("UTF-8 remote"),
            ],
        );
        git(&repo, &["push", "--quiet", "-u", "origin", "main"]);

        let keys = [
            "AUTOSPEC_HEARTBEAT_DIR",
            "AUTOSPEC_CLAIM_GIT_REMOTE",
            "AUTOSPEC_CLAIM_GIT_STATE_DIR",
        ];
        let _restore = RestoreEnvironment(
            keys.into_iter()
                .map(|key| (key, std::env::var_os(key)))
                .collect(),
        );
        unsafe {
            std::env::set_var("AUTOSPEC_HEARTBEAT_DIR", &heartbeat);
            std::env::set_var("AUTOSPEC_CLAIM_GIT_REMOTE", &remote);
            std::env::set_var("AUTOSPEC_CLAIM_GIT_STATE_DIR", &claim_state);
        }

        let predecessor_claimed = autospec_core::claim::RunStateRecord::new(
            "owner/repo",
            42,
            "worker-predecessor",
            "claimed",
            "feat/autonomous-issue-42",
            "",
            "claimed",
            Vec::new(),
            "2026-08-14T00:00:00Z",
            "2026-08-14T00:00:00Z",
            300,
        )
        .with_claim_id("claim-predecessor");
        let predecessor = autospec_core::claim::RunStateRecord::new(
            "owner/repo",
            42,
            "worker-predecessor",
            "released",
            "feat/autonomous-issue-42",
            "",
            "released",
            Vec::new(),
            "2026-08-14T00:00:00Z",
            "2026-08-14T00:00:01Z",
            300,
        )
        .with_claim_id("claim-predecessor");
        crate::commands::claim::write_startup_heartbeat_for_test(
            "owner/repo",
            42,
            "worker-predecessor",
            "feat/autonomous-issue-42",
            "claim-predecessor",
            Some("session-predecessor"),
        )
        .expect("publish predecessor heartbeat");
        assert!(
            crate::commands::claim::advance_claim_ref_for_test(&repo, &predecessor_claimed)
                .expect("publish claimed predecessor")
        );
        assert!(
            crate::commands::claim::advance_claim_ref_for_test(&repo, &predecessor)
                .expect("publish released predecessor claim")
        );

        let identity = crate::commands::claim::ClaimMutationIdentity {
            repo: "owner/repo",
            issue: 42,
            worker_id: "worker-predecessor",
            branch: "feat/autonomous-issue-42",
            claim_id: "claim-predecessor",
        };
        let terminal_path = root.join("terminal.json");
        let result = crate::commands::claim::with_released_bridge_predecessor_authority(
            identity,
            || {
                let issue_heartbeat = heartbeat
                    .join(crate::commands::autonomous::drain::repository_progress_key(
                        "owner/repo",
                    ))
                    .join("42.json");
                assert!(
                    !issue_heartbeat.exists(),
                    "released predecessor heartbeat survived retirement"
                );
                let successor = autospec_core::claim::RunStateRecord::new(
                    "owner/repo",
                    42,
                    "worker-successor",
                    "claimed",
                    "feat/autonomous-issue-42",
                    "",
                    "claimed",
                    Vec::new(),
                    "2026-08-14T00:00:02Z",
                    "2026-08-14T00:00:02Z",
                    300,
                )
                .with_claim_id("claim-successor");
                assert!(crate::commands::claim::advance_claim_ref_for_test(&repo, &successor)?);
                crate::commands::claim::write_startup_heartbeat_for_test(
                    "owner/repo",
                    42,
                    "worker-successor",
                    "feat/autonomous-issue-42",
                    "claim-successor",
                    Some("session-successor"),
                )?;

                let executable = std::env::current_exe().map_err(|error| {
                    crate::commands::CommandFailure::diagnostic(format!(
                        "resolve admission no-op executable: {error}"
                    ))
                })?;
                let test_name = "commands::autonomous::executor_bridge::portability::supported_host_tests::supported_host_retires_predecessor_runs_noop_and_publishes_terminal_receipt";
                let mut child = process_owner::OwnedChildTree::spawn_prepared(
                    process_owner::PreparedLaunchSpec::inherited(
                        executable.clone(),
                        vec![
                            executable.into_os_string(),
                            "--exact".into(),
                            test_name.into(),
                            "--nocapture".into(),
                        ],
                        Some(repo.clone()),
                        vec![(CHILD_ENV.into(), "1".into())],
                        None,
                        None,
                        None,
                    ),
                    "supported-host-noop".into(),
                )
                .map_err(crate::commands::CommandFailure::diagnostic)?;
                let status = child
                    .wait()
                    .and_then(|status| child.terminate().map(|_| status))
                    .map_err(crate::commands::CommandFailure::diagnostic)?;
                if !status.success() {
                    return Err(crate::commands::CommandFailure::diagnostic(
                        "supported-host no-op child failed",
                    ));
                }
                let receipt = BridgeRunReceipt {
                    repository: "owner/repo".into(),
                    issue: 42,
                    worker_id: "worker-successor".into(),
                    branch: "feat/autonomous-issue-42".into(),
                    claim_id: "claim-successor".into(),
                    invocation_id: "invocation-successor".into(),
                    status: BridgeRunStatus::Retryable {
                        reason: "supported-host-noop".into(),
                    },
                };
                write_private_create_once(
                    &terminal_path,
                    format!("{}\n", receipt.to_json()).as_bytes(),
                    "supported-host terminal receipt",
                )
                .map_err(crate::commands::CommandFailure::diagnostic)?;
                Ok(receipt)
            },
        )
        .expect("retire predecessor and admit successor")
        .expect("released predecessor authority");

        assert_eq!(result.claim_id, "claim-successor");
        assert!(
            terminal_path.is_file(),
            "terminal receipt was not published"
        );
        let heartbeat_body = fs::read_to_string(
            heartbeat
                .join(crate::commands::autonomous::drain::repository_progress_key(
                    "owner/repo",
                ))
                .join("42.json"),
        )
        .expect("read successor heartbeat");
        assert!(heartbeat_body.contains("claim-successor"));
        fs::remove_dir_all(root).expect("remove supported-host fixture");
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
    let failpoint = |stage: &str| {
        adapter
            .environment
            .get(std::ffi::OsStr::new("AUTOSPEC_TEST_PORTABLE_DRAFT_FAIL"))
            .is_some_and(|value| value == std::ffi::OsStr::new(stage))
    };
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

    // Portable process creation cannot use Linux's fork/pipe release barrier. Persist a
    // transaction identity and release guard before spawning instead: a crash after release may
    // strand a request, but can never authorize replay. Exact visible PR observation reconciles
    // it in `push_and_create_draft`; absent/delayed visibility remains quarantined.
    let process = ProcessIdentity {
        pid: u32::MAX - 1,
        process_group: u32::MAX - 1,
        executable: executable.clone(),
        argv_digest: argv_digest(&args),
        boot_id: "portable-draft-release-v1".to_string(),
        start_identity: state.identity.invocation_id.clone(),
    };
    state.draft_process = Some(process.clone());
    write_invocation_atomic(state_path, state)?;
    if failpoint("prepare") {
        let _ = fs::remove_file(&body_path);
        return Err("injected portable draft prepare failure".to_string().into());
    }
    write_draft_release_intent(state_path, state, &process)?;
    if failpoint("release") {
        let _ = fs::remove_file(&body_path);
        return Err("injected portable draft release failure".to_string().into());
    }
    write_private_create_once(
        &draft_release_receipt_path(state_path),
        draft_release_digest(state, &process).as_bytes(),
        "portable executor draft release receipt",
    )?;

    let null = if cfg!(windows) { "NUL" } else { "/dev/null" };
    let stderr_path = state_path.with_extension("draft.stderr");
    let stdout_path = state_path.with_extension("draft.stdout");
    let spawn = process_owner::OwnedChildTree::spawn_prepared(
        process_owner::PreparedLaunchSpec::inherited(
            executable.clone(),
            std::iter::once(executable.into_os_string())
                .chain(args.iter().map(OsString::from))
                .collect(),
            Some(state.identity.worktree.clone()),
            adapter.environment.clone().into_iter().collect(),
            Some(File::open(null).map_err(|error| format!("open draft null input: {error}"))?),
            Some(open_private_file(&stdout_path, true)?),
            Some(open_private_file(&stderr_path, true)?),
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
    let owner = owned.identity();
    let owner_path = state_path.with_extension("draft-owner.json");
    if let Err(error) = write_private_create_once(
        &owner_path,
        owner
            .document(
                &format!("draft-{}", state.identity.invocation_id),
                &draft_release_digest(state, &process),
            )
            .as_bytes(),
        "portable executor draft owner",
    ) {
        let cleanup = owned.terminate();
        let _ = fs::remove_file(&body_path);
        return Err(match cleanup {
            Ok(_) => error.into(),
            Err(cleanup) => format!("{error}; draft cleanup ambiguous: {cleanup}").into(),
        });
    }
    let status = owned.wait();
    let cleanup = owned.terminate();
    let _ = fs::remove_file(&body_path);
    let cleanup_status = cleanup.map_err(|error| {
        BridgeRunFailure::transient(format!(
            "portable executor draft cleanup is ambiguous: {error}"
        ))
    })?;
    write_private_create_once(
        &owner_path.with_extension("cleanup.json"),
        serde_json::json!({
            "schema": 1,
            "invocation_id": state.identity.invocation_id,
            "tree_cleanup": "proven",
            "exit_code": cleanup_status.code(),
        })
        .to_string()
        .as_bytes(),
        "portable executor draft cleanup evidence",
    )?;
    fs::remove_file(&owner_path)
        .map_err(|error| format!("retire portable executor draft owner: {error}"))?;
    let status = status.map_err(|error| {
        BridgeRunFailure::transient(format!("wait for executor draft pull request: {error}"))
    })?;
    if failpoint("post-request") {
        return Err("injected portable draft post-request failure"
            .to_string()
            .into());
    }
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
    enum PortableTerminal {
        Exited(i32),
        OwnershipLost,
        Transient(BridgeRunFailure),
        Stalled,
    }

    let operation = (|| -> Result<PortableTerminal, String> {
        fail_launch_at("persist")?;
        state.phase = BridgePhase::Implementing;
        state.progress_at = unix_now()?;
        write_invocation_atomic(state_path, state)?;
        fail_launch_at("log")?;
        append_executor_event(event_log, state, "child_started", None)?;
        let mut observed = (0, 0);
        let mut last_progress = Instant::now();
        loop {
            thread::sleep(config.poll_interval);
            fail_launch_at("direct-poll")?;
            if let Some(status) = owned.try_wait()? {
                return Ok(PortableTerminal::Exited(status.code().unwrap_or(1)));
            }
            let sizes = (
                fs::metadata(&sinks.stdout).map(|m| m.len()).unwrap_or(0),
                fs::metadata(&sinks.stderr).map(|m| m.len()).unwrap_or(0),
            );
            if sizes != observed {
                observed = sizes;
                last_progress = Instant::now();
                state.progress_at = unix_now()?;
                fail_launch_at("adopt-flush")?;
                write_invocation_atomic(state_path, state)?;
            }
            if renewal.is_due() {
                match refresh_bridge_claim(state, ClaimRenewalPhase::Implementation) {
                    Ok(BridgeClaimOwnership::Refreshed { ttl_seconds }) => {
                        renewal.mark_refreshed(ttl_seconds)
                    }
                    Ok(BridgeClaimOwnership::Lost) => return Ok(PortableTerminal::OwnershipLost),
                    Err(error) => return Ok(PortableTerminal::Transient(error)),
                }
            }
            if last_progress.elapsed() >= config.stall_timeout {
                return Ok(PortableTerminal::Stalled);
            }
        }
    })();

    // This is the single post-spawn finalizer. It consumes the live OS ownership authority on
    // every branch, including a leader that already exited while descendants remain. The durable
    // journal is retained unless tree cleanup, cleanup evidence, and its event are all proven.
    let cleanup_status = match owned.terminate() {
        Ok(status) => status,
        Err(cleanup) => {
            state.phase = BridgePhase::Interrupted;
            state.progress_at = unix_now().unwrap_or(state.progress_at);
            let _ = write_invocation_atomic(state_path, state);
            let _ = append_executor_event(
                event_log,
                state,
                "child_cleanup_ambiguous",
                Some(serde_json::json!({"reason": &cleanup})),
            );
            let operation = operation
                .err()
                .map(|error| format!("; operation failed: {error}"))
                .unwrap_or_default();
            return Err(format!(
                "executor portable child cleanup is ambiguous: {cleanup}{operation}"
            ));
        }
    };
    let cleanup_path = sinks.supervisor_identity.with_extension("cleanup.json");
    let cleanup_document = serde_json::json!({
        "schema": 1,
        "invocation_id": state.identity.invocation_id,
        "owner": owner.document(&state.identity.invocation_id, &argv_digest(&harness.args)),
        "exit_code": cleanup_status.code(),
        "tree_cleanup": "proven",
    })
    .to_string();
    write_private_create_once(
        &cleanup_path,
        cleanup_document.as_bytes(),
        "portable executor cleanup evidence",
    )?;
    append_executor_event(event_log, state, "child_cleanup_complete", None)?;
    fs::remove_file(&sinks.supervisor_identity)
        .map_err(|error| format!("retire portable executor owner journal: {error}"))?;

    let terminal = operation?;
    match terminal {
        PortableTerminal::Exited(exit_code) => {
            fail_launch_at("pre-verify")?;
            snapshot.verify(&state.identity.repository_path, &state.identity.branch)?;
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
            Ok(SupervisionOutcome::Exited { exit_code })
        }
        PortableTerminal::OwnershipLost => {
            record_claim_ownership_loss(state_path, event_log, state)?;
            Ok(SupervisionOutcome::OwnershipLost)
        }
        PortableTerminal::Transient(error) => Ok(SupervisionOutcome::TransientFailure(error)),
        PortableTerminal::Stalled => {
            state.phase = BridgePhase::Interrupted;
            state.progress_at = unix_now()?;
            write_invocation_atomic(state_path, state)?;
            append_executor_event(event_log, state, "child_stalled", None)?;
            Ok(SupervisionOutcome::Stalled)
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

    static PORTABLE_SUPERVISION_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());

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

    fn portable_supervision_state(fixture: &DirectFixture) -> PersistedInvocation {
        let head = git_stdout(&fixture.worktree, &["rev-parse", "HEAD"])
            .expect("read supervision fixture head");
        PersistedInvocation {
            schema: 1,
            identity: BridgeIdentity {
                repository: "owner/repo".into(),
                repository_path: fixture.worktree.clone(),
                issue: 42,
                worker_id: "worker-portable".into(),
                branch: "feat/autonomous-issue-42".into(),
                claim_id: "claim-portable".into(),
                invocation_id: format!(
                    "invocation-portable-{}",
                    DIRECT_TRANSACTION_SEQUENCE.fetch_add(1, Ordering::Relaxed)
                ),
                base_ref: "main".into(),
                base_oid: head,
                worktree: fixture.worktree.clone(),
                runtime_environment_dir: None,
                runtime_session_id: None,
            },
            harness: HarnessKind::Codex,
            phase: BridgePhase::Pending,
            supervisor: None,
            process: None,
            progress_at: unix_now().expect("fixture timestamp"),
            pr: None,
            head_oid: None,
            closeout_path: None,
            closeout_digest: None,
            remote_snapshot_digest: None,
            draft_process: None,
            terminal_result: None,
            umbrella: None,
            current_child: None,
            implementation_repair_attempt: 0,
        }
    }

    fn portable_harness(fixture: &DirectFixture, script: &str) -> ValidatedInvocation {
        ValidatedInvocation {
            program: fs::canonicalize("/bin/sh").expect("canonical shell"),
            argv_zero: None,
            args: vec!["-c".into(), script.into()],
            current_dir: fixture.worktree.clone(),
            environment_overrides: Vec::new(),
        }
    }

    #[test]
    fn portable_supervision_failures_cleanup_before_retiring_owner_journal() {
        // Break caught: any post-spawn `?` returning while the harness or its descendants remain
        // live, or removing ownership without durable cleanup evidence.
        let _serial = PORTABLE_SUPERVISION_TEST
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for (name, failpoint, script) in [
            ("persist", LaunchFailpoint::PersistAfterSpawn, "sleep 30"),
            ("event", LaunchFailpoint::LogAfterSpawn, "sleep 30"),
            ("poll", LaunchFailpoint::DirectPoll, "sleep 30"),
            (
                "progress",
                LaunchFailpoint::AdoptedFlush,
                "printf progress; sleep 30",
            ),
            (
                "snapshot",
                LaunchFailpoint::BeforeSnapshotVerification,
                "exit 0",
            ),
        ] {
            let fixture = DirectFixture::new(&format!("supervision-{name}"));
            let mut state = portable_supervision_state(&fixture);
            let state_path = fixture.root.join("state/invocation.json");
            let event_log = fixture.root.join("state/events.jsonl");
            let snapshot =
                MutationSnapshot::capture(&state.identity.repository_path, &state.identity.branch)
                    .expect("capture supervision snapshot");
            set_launch_failpoint(failpoint);
            let error = supervise_validated_harness_with_claim_renewal(
                &state_path,
                &event_log,
                &mut state,
                Some(&portable_harness(&fixture, script)),
                &snapshot,
                SupervisionConfig {
                    stall_timeout: Duration::from_secs(5),
                    poll_interval: Duration::from_millis(10),
                },
                ClaimRenewalSchedule::Disabled,
            )
            .expect_err("injected portable lifecycle failure");
            set_launch_failpoint(LaunchFailpoint::None);
            assert!(
                error.contains("injected"),
                "unexpected {name} error: {error}"
            );
            let sinks = output_sink_paths_for_state(&state_path, &state)
                .expect("resolve portable supervision sinks");
            assert!(
                !sinks.supervisor_identity.exists(),
                "{name} retained a journal after proven cleanup"
            );
            assert!(
                sinks
                    .supervisor_identity
                    .with_extension("cleanup.json")
                    .is_file(),
                "{name} omitted durable cleanup evidence"
            );
        }
    }

    #[test]
    fn portable_supervision_retains_owner_when_cleanup_evidence_is_ambiguous() {
        // Break caught: deleting the only ownership journal when cleanup evidence cannot be
        // published, making restart treat an unproven tree as safe.
        let _serial = PORTABLE_SUPERVISION_TEST
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let fixture = DirectFixture::new("supervision-cleanup-evidence");
        let mut state = portable_supervision_state(&fixture);
        let state_path = fixture.root.join("state/invocation.json");
        let event_log = fixture.root.join("state/events.jsonl");
        let snapshot =
            MutationSnapshot::capture(&state.identity.repository_path, &state.identity.branch)
                .expect("capture supervision snapshot");
        let sinks = output_sink_paths_for_state(&state_path, &state)
            .expect("resolve portable supervision sinks");
        write_private_create_once(
            &sinks.supervisor_identity.with_extension("cleanup.json"),
            b"foreign cleanup evidence",
            "foreign portable cleanup evidence",
        )
        .expect("seed conflicting cleanup evidence");

        let error = supervise_validated_harness_with_claim_renewal(
            &state_path,
            &event_log,
            &mut state,
            Some(&portable_harness(&fixture, "exit 0")),
            &snapshot,
            SupervisionConfig {
                stall_timeout: Duration::from_secs(5),
                poll_interval: Duration::from_millis(10),
            },
            ClaimRenewalSchedule::Disabled,
        )
        .expect_err("conflicting cleanup evidence must fail closed");
        assert!(
            error.contains("differs"),
            "unexpected cleanup error: {error}"
        );
        assert!(
            sinks.supervisor_identity.is_file(),
            "ambiguous cleanup evidence discarded the ownership journal"
        );
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
