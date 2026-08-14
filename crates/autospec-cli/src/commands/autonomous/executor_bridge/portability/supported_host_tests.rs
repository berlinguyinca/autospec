use super::*;

const HARNESS_HELPER: &str = "commands::autonomous::executor_bridge::portability::supported_host_tests::supported_host_noop_harness_helper";

#[test]
#[ignore = "launched as the production harness child by the supported-host admission test"]
fn supported_host_noop_harness_helper() {
    std::process::exit(17);
}

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
    // Break caught: a supported host compiling the bridge while skipping released
    // predecessor retirement, successor heartbeat ownership, real OS child ownership, or
    // terminal receipt publication.
    let _serial = PORTABLE_LIFECYCLE_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let sequence = DIRECT_TRANSACTION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let repository = format!("owner/portable-admission-{}-{sequence}", std::process::id());
    // Keep the spelling accepted by external tools. On Windows, canonicalizing the
    // temporary directory adds a verbatim (`\\?\`) prefix that Git rejects for `init`.
    let root = std::env::temp_dir().join(format!(
        "autospec-supported-host-admission-{}-{}",
        std::process::id(),
        sequence
    ));
    let repo = root.join("repo");
    let remote = root.join("remote.git");
    let heartbeat = root.join("heartbeats");
    let claim_state = root.join("claim-state");
    let bin = root.join("bin");
    fs::create_dir_all(&repo).expect("create admission repo");
    fs::create_dir_all(&heartbeat).expect("create heartbeat root");
    fs::create_dir_all(&bin).expect("create admission bin");
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
    assert!(Command::new("git")
        .args([
            "--git-dir",
            remote.to_str().expect("UTF-8 remote"),
            "symbolic-ref",
            "HEAD",
            "refs/heads/main",
        ])
        .status()
        .expect("set admission remote HEAD")
        .success());
    git(&repo, &["remote", "set-head", "origin", "main"]);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let gh = bin.join("gh");
        fs::write(
            &gh,
            "#!/bin/sh\nset -eu\ncase \"$1 $2\" in\n  'issue view')\n    mode=initial; [ ! -f \"$AUTOSPEC_TEST_GH_STATE\" ] || mode=$(cat \"$AUTOSPEC_TEST_GH_STATE\")\n    case \" $* \" in\n      *' labels,body,title,author '*)\n        if [ \"$mode\" = claimed ]; then\n          printf '%s\\n' '{\"labels\":[\"in-progress-by-bot\",\"safety:reviewed\"],\"body\":\"## Safety review\\n\\n<!-- autospec-safety:begin -->\\n- **decision:** `SAFETY_PASS`\\n<!-- autospec-safety:end -->\\n\\n## Goal\\nRun the portable admission.\",\"title\":\"Portable admission\",\"author\":\"fixture\"}'\n        else\n          printf '%s\\n' '{\"labels\":[\"auto-implement\",\"safety:reviewed\"],\"body\":\"## Safety review\\n\\n<!-- autospec-safety:begin -->\\n- **decision:** `SAFETY_PASS`\\n<!-- autospec-safety:end -->\\n\\n## Goal\\nRun the portable admission.\",\"title\":\"Portable admission\",\"author\":\"fixture\"}'\n        fi ;;\n      *) printf '%s\\n' '{\"labels\":[{\"name\":\"auto-implement\"},{\"name\":\"safety:reviewed\"}]}' ;;\n    esac ;;\n  'issue edit')\n    case \" $* \" in *' --add-label in-progress-by-bot '*) printf claimed > \"$AUTOSPEC_TEST_GH_STATE\" ;; *' --add-label auto-implement '*) printf released > \"$AUTOSPEC_TEST_GH_STATE\" ;; esac ;;\n  'pr list') printf '%s\\n' '[]' ;;\n  api*) printf '%s\\n' '[]' ;;\n  *) : ;;\nesac\n",
        )
        .expect("write admission gh shim");
        fs::set_permissions(&gh, fs::Permissions::from_mode(0o700))
            .expect("make admission gh shim executable");
    }
    #[cfg(windows)]
    fs::write(
        bin.join("gh.cmd"),
        "@echo off\r\nif \"%1\"==\"api\" echo []\r\nif \"%1 %2\"==\"issue edit\" (\r\n  echo %* | findstr /c:\"--add-label in-progress-by-bot\" >nul && echo claimed>\"%AUTOSPEC_TEST_GH_STATE%\"\r\n  echo %* | findstr /c:\"--add-label auto-implement\" >nul && echo released>\"%AUTOSPEC_TEST_GH_STATE%\"\r\n)\r\nif \"%1 %2\"==\"issue view\" (\r\n  echo %* | findstr /c:\"labels,body,title,author\" >nul\r\n  if errorlevel 1 (\r\n    echo {\"labels\":[{\"name\":\"auto-implement\"},{\"name\":\"safety:reviewed\"}]}\r\n  ) else (\r\n    findstr /x claimed \"%AUTOSPEC_TEST_GH_STATE%\" >nul 2>nul && (echo {\"labels\":[\"in-progress-by-bot\",\"safety:reviewed\"],\"body\":\"## Safety review\\n\\n^<!-- autospec-safety:begin --^>\\n- **decision:** `SAFETY_PASS`\\n^<!-- autospec-safety:end --^>\\n\\n## Goal\\nRun the portable admission.\",\"title\":\"Portable admission\",\"author\":\"fixture\"}) || (echo {\"labels\":[\"auto-implement\",\"safety:reviewed\"],\"body\":\"## Safety review\\n\\n^<!-- autospec-safety:begin --^>\\n- **decision:** `SAFETY_PASS`\\n^<!-- autospec-safety:end --^>\\n\\n## Goal\\nRun the portable admission.\",\"title\":\"Portable admission\",\"author\":\"fixture\"})\r\n  )\r\n)\r\nif \"%1 %2\"==\"pr list\" echo []\r\nexit /b 0\r\n",
    )
    .expect("write admission gh shim");
    let harness_aliases = root.join("harness-runtime-aliases.tsv");
    fs::write(
        &harness_aliases,
        format!(
            "claude\t{}\tclaude\tPortable admission helper\n",
            std::env::current_exe()
                .expect("resolve admission harness executable")
                .display()
        ),
    )
    .expect("write admission harness alias table");

    let keys = [
        "AUTOSPEC_HEARTBEAT_DIR",
        "AUTOSPEC_CLAIM_GIT_REMOTE",
        "AUTOSPEC_CLAIM_GIT_STATE_DIR",
        "AUTOSPEC_CLAIM_CONFIRM_READS",
        "AUTOSPEC_TEST_GH_STATE",
        "AUTOSPEC_HARNESS_RUNTIME_ALIASES",
        "AUTOSPEC_HANDOFF_DISPATCHER_KIND",
        "PATH",
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
        std::env::set_var("AUTOSPEC_CLAIM_CONFIRM_READS", "1");
        std::env::set_var("AUTOSPEC_TEST_GH_STATE", root.join("gh-state"));
        std::env::set_var("AUTOSPEC_HARNESS_RUNTIME_ALIASES", &harness_aliases);
        std::env::set_var("AUTOSPEC_HANDOFF_DISPATCHER_KIND", "claude");
        let mut path = vec![bin.clone()];
        path.extend(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        ));
        std::env::set_var(
            "PATH",
            std::env::join_paths(path).expect("join admission PATH"),
        );
    }

    let predecessor_claimed = autospec_core::claim::RunStateRecord::new(
        &repository,
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
        &repository,
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
        &repository,
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

    let lease = crate::commands::claim::acquire_for_conductor(
        &repository,
        42,
        "worker-successor",
        "feat/autonomous-issue-42",
        "main",
    )
    .unwrap_or_else(|error| match error {
        crate::commands::claim::ConductorClaimError::Diagnostic(error) => {
            panic!("production successor acquisition: {}", error.message)
        }
        crate::commands::claim::ConductorClaimError::Deferred { json, exit_code } => {
            panic!("production successor acquisition deferred ({exit_code}): {json}")
        }
    });
    let issue_heartbeat = heartbeat
        .join(crate::commands::autonomous::drain::repository_progress_key(
            &repository,
        ))
        .join("42.json");
    let heartbeat_body = fs::read_to_string(&issue_heartbeat).expect("read successor heartbeat");
    assert!(!heartbeat_body.contains("claim-predecessor"));
    assert!(heartbeat_body.contains(&lease.claim_id));

    let state_path = root.join("state/invocation.json");
    let invocation_id = format!("42-{}", lease.claim_id);
    let _harness_adapter = set_test_executor_harness_exact(HARNESS_HELPER);
    let request = ExecutorBridgeRequest {
        repository: repository.clone(),
        repository_path: fs::canonicalize(&repo).expect("canonical admission repo"),
        issue: 42,
        issue_title: "Portable admission".into(),
        issue_body: "No-op admission run".into(),
        serialization_reasons: Vec::new(),
        worker_id: lease.worker_id,
        claim_id: lease.claim_id,
        invocation_id,
        state_path: state_path.clone(),
        event_log: root.join("state/events.jsonl"),
    };
    let result =
        run_executor_bridge(&request).expect("production bridge launch and reconciliation");

    assert_eq!(result.claim_id, request.claim_id);
    assert!(matches!(
        result.status,
        BridgeRunStatus::Retryable { ref reason }
            if reason == "executor_harness_exit_17"
    ));
    let terminal_path = state_path.with_extension("terminal.json");
    assert!(
        terminal_path.is_file(),
        "terminal receipt was not published"
    );
    let sinks = output_sink_paths(&state_path, &request.invocation_id)
        .expect("resolve production admission ownership sinks");
    let cleanup_path = sinks.supervisor_identity.with_extension("cleanup.json");
    let cleanup_evidence = fs::read(&cleanup_path)
        .expect("read production admission cleanup evidence before reconciliation");
    let first_run_events =
        fs::read_to_string(&request.event_log).expect("read first-run admission events");

    let reconciled =
        run_executor_bridge(&request).expect("production bridge entry-time reconciliation");
    assert_eq!(reconciled, result);

    let events = fs::read_to_string(&request.event_log).expect("read admission events");
    assert_eq!(
        events, first_run_events,
        "entry-time reconciliation appended lifecycle side effects"
    );
    assert_eq!(
        events.matches("\"event\":\"child_started\"").count(),
        1,
        "entry-time reconciliation replayed the harness child: {events}"
    );
    for event in ["child_started", "child_cleanup_complete", "child_exited"] {
        assert!(
            events.contains(&format!("\"event\":\"{event}\"")),
            "{events}"
        );
    }
    for forbidden in [
        "requires Linux executor supervision",
        "unsupported on non-Linux",
        "require_linux_executor_supervision",
    ] {
        assert!(!events.contains(forbidden), "{events}");
    }
    let persisted = PersistedInvocation::from_json(
        &fs::read_to_string(&state_path).expect("read terminal admission state"),
    )
    .expect("parse terminal admission state");
    assert_eq!(persisted.phase, BridgePhase::Complete);
    assert!(persisted.supervisor.is_none() && persisted.process.is_none());
    assert!(
        cleanup_path.is_file(),
        "production supervision omitted durable cleanup evidence"
    );
    assert_eq!(
        fs::read(&cleanup_path)
            .expect("read production admission cleanup evidence after reconciliation"),
        cleanup_evidence,
        "entry-time reconciliation rewrote child cleanup evidence"
    );
    assert!(
        !sinks.supervisor_identity.exists(),
        "entry-time reconciliation created or retained an owner journal"
    );
    fs::remove_dir_all(root).expect("remove supported-host fixture");
}
