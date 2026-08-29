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
        supervised_executable: fs::canonicalize("/bin/sh").expect("canonical shell"),
        argv_zero: None,
        args: vec!["-c".into(), script.into()],
        current_dir: fixture.worktree.clone(),
        environment_overrides: Vec::new(),
    }
}

#[test]
fn portable_supervision_failures_cleanup_before_retiring_owner_journal() {
    let _environment = crate::commands::PROCESS_ENVIRONMENT
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    // Break caught: any post-spawn `?` returning while the harness or its descendants remain
    // live, or removing ownership without durable cleanup evidence.
    let _serial = PORTABLE_LIFECYCLE_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for (name, failpoint, script) in [
        ("journal", LaunchFailpoint::JournalWrite, "sleep 30"),
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
        LAST_SPAWN_HARNESS.store(0, Ordering::SeqCst);
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
        let spawned_pid = LAST_SPAWN_HARNESS.load(Ordering::SeqCst);
        assert_ne!(spawned_pid, 0, "{name} did not launch the real child");
        assert!(
            process_birth_identity(spawned_pid)
                .expect("inspect cleaned portable child")
                .is_none(),
            "{name} left the real child alive after cleanup"
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
    let _serial = PORTABLE_LIFECYCLE_TEST
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
fn execute_direct_plan_cleans_descendant_after_successful_leader_exit() {
    let fixture = DirectFixture::new("success-descendant-cleanup");
    let descendant = fixture.worktree.join("descendant.pid");
    let plan = DirectCommandPlan {
        commands: vec![DirectCommand::success(vec![
            "/bin/sh".to_string(),
            "-c".to_string(),
            format!(
                "sleep 30 & printf %s $! > '{}'; exit 0",
                descendant.display()
            ),
        ])],
    };

    let observed = execute_direct_plan(
        &fixture.worktree,
        &plan,
        &fixture.artifacts,
        None,
        Duration::from_secs(2),
    )
    .expect("successful leader exit must drain its owned group");

    assert_eq!(observed[0].terminal, AttemptTerminal::Exited(0));
    let pid = fs::read_to_string(descendant)
        .expect("descendant pid")
        .parse::<i32>()
        .expect("numeric descendant pid");
    assert_eq!(
        unsafe { nix::libc::kill(pid, 0) },
        -1,
        "successful direct command left descendant {pid} alive"
    );
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(nix::libc::ESRCH)
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
fn quarantined_direct_recovery_preserves_launch_and_never_replays_command() {
    // Break caught: treating portable quarantine as cleanup proof, retiring the exact launch
    // evidence, and replaying the mutation command on every recovery attempt.
    let fixture = DirectFixture::new("reconcile-quarantine");
    ensure_private_directory(&fixture.artifacts).expect("create artifact root");
    let artifacts = fs::canonicalize(&fixture.artifacts).expect("canonicalize artifact root");
    let paths = direct_attempt_paths(&artifacts, 0);
    let attempt_id = reserve_direct_attempt_id(&paths).expect("reserve attempt identity");
    let commit_oid = git_stdout(
        &fixture.worktree,
        &["rev-parse", "--verify", "HEAD^{commit}"],
    )
    .expect("read fixture commit");
    let side_effect = fixture.worktree.join("mutation-ran");
    let executable = fs::canonicalize("/bin/sh").expect("canonical shell");
    let script = format!("printf mutation > '{}'", side_effect.display());
    let argv = vec![
        executable.display().to_string(),
        "-c".to_string(),
        script.clone(),
    ];
    let intent = direct_intent_document(&attempt_id, &commit_oid, None, &executable, &argv);
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
    let launch = owner.document(&attempt_id, &sha256_hex(intent.as_bytes()));
    write_private_create_once(
        &paths.launch,
        launch.as_bytes(),
        "portable reconciliation fixture launch",
    )
    .expect("write fixture launch");
    let plan = DirectCommandPlan {
        commands: vec![DirectCommand::success(argv)],
    };

    for _ in 0..2 {
        let error = execute_direct_plan(
            &fixture.worktree,
            &plan,
            &artifacts,
            None,
            Duration::from_secs(2),
        )
        .expect_err("quarantined portable owner must block command replay");
        assert!(error.contains("quarantin"), "unexpected error: {error}");
        assert_eq!(
            fs::read(&paths.launch).expect("retained exact launch evidence"),
            launch.as_bytes(),
            "quarantine mutated the exact launch evidence"
        );
        assert!(
            !side_effect.exists(),
            "quarantined recovery replayed a side-effecting command"
        );
        assert!(
            process_birth_identity(std::process::id())
                .expect("re-observe current process")
                .is_some(),
            "reconciliation signalled the recorded PID"
        );
    }
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
    let capture = ActiveReviewerCapture::empty();
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
