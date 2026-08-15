// executor_bridge tests: cleanup / restart — 12 cases.
//
// Split out of tests.rs; see the note in that file.

use super::super::BridgePhase;
use super::support_base::{git, git_stdout, test_environment};
use super::support_invocation::{commit_implementation, implementation_proof_fixture};
use super::support_launch::{
    adapter_path, draft_pr_adapter_fixture, prepared_draft_transaction, PreparedDraftTransaction,
};
use crate::commands::autonomous::executor_bridge as bridge;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;

#[cfg(target_os = "linux")]
fn cleanup_pending_transaction(label: &str) -> PreparedDraftTransaction {
    let mut prepared = prepared_draft_transaction(label);
    prepared.push_exact_at_intent();
    prepared.state.phase = BridgePhase::DraftCleanupPending;
    prepared.state.draft_process = Some(bridge::ProcessIdentity {
        pid: 4_000_002,
        process_group: 4_000_002,
        executable: prepared.adapter.gh.clone(),
        argv_digest: format!("{label}-dead-draft"),
        boot_id: "missing-boot".to_string(),
        start_identity: "missing-start".to_string(),
    });
    bridge::write_invocation_atomic(&prepared.state_path, &prepared.state)
        .expect("cleanup-pending invocation");
    fs::write(prepared.fixture.root.join("gh-calls"), "").expect("clear calls");
    prepared
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_cleanup_restart_requires_both_guards_absent() {
    let _environment = test_environment();
    // Break caught: cleanup recovery retrying while a release guard still exists.
    for guard in ["receipt", "intent"] {
        let mut prepared = cleanup_pending_transaction(&format!("draft-cleanup-guard-{guard}"));
        let path = if guard == "receipt" {
            bridge::draft_release_receipt_path(&prepared.state_path)
        } else {
            bridge::draft_release_intent_path(&prepared.state_path)
        };
        let process = prepared
            .state
            .draft_process
            .as_ref()
            .expect("draft process");
        fs::write(
            &path,
            bridge::draft_release_digest(&prepared.state, process),
        )
        .expect("release guard");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .expect("private release guard");

        let error = prepared
            .publish()
            .expect_err("present release guard must prohibit cleanup recovery");

        assert!(
            error.contains("cleanup") || error.contains("release"),
            "{error}"
        );
        let calls = fs::read_to_string(prepared.fixture.root.join("gh-calls")).expect("gh calls");
        assert_eq!(calls.matches("pr create").count(), 0);
    }
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_cleanup_restart_requires_both_directory_syncs() {
    let _environment = test_environment();
    // Break caught: cleanup recovery authorizing retry after either parent sync fails.
    for failpoint in [
        "AUTOSPEC_TEST_DRAFT_FAIL_CLEANUP_RECOVERY_RECEIPT_FSYNC",
        "AUTOSPEC_TEST_DRAFT_FAIL_CLEANUP_RECOVERY_INTENT_FSYNC",
    ] {
        let mut prepared =
            cleanup_pending_transaction(&format!("draft-cleanup-recovery-fsync-{failpoint}"));
        prepared
            .adapter
            .environment
            .insert(failpoint.into(), "1".into());

        let error = prepared
            .publish()
            .expect_err("failed cleanup recovery sync must prohibit retry");

        assert!(
            error.contains("cleanup") || error.contains("sync"),
            "{error}"
        );
        let calls = fs::read_to_string(prepared.fixture.root.join("gh-calls")).expect("gh calls");
        assert_eq!(calls.matches("pr create").count(), 0);
    }
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_cleanup_restart_rejects_public_guard_parent() {
    let _environment = test_environment();
    // Break caught: cleanup recovery silently repairing an untrusted guard directory.
    let mut prepared = cleanup_pending_transaction("draft-cleanup-public-guard-parent");
    let parent = prepared
        .state_path
        .parent()
        .expect("state parent")
        .to_path_buf();
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o755)).expect("public guard parent");

    let error = prepared
        .publish()
        .expect_err("public guard parent must prohibit cleanup recovery");

    let parent_mode = fs::metadata(&parent)
        .expect("public guard parent metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(
        parent_mode, 0o755,
        "rejected guard parent permissions must remain unchanged"
    );
    assert!(
        error.contains("private") || error.contains("unsafe"),
        "{error}"
    );
    let calls = fs::read_to_string(prepared.fixture.root.join("gh-calls")).expect("gh calls");
    assert_eq!(calls.matches("pr create").count(), 0);
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_cleanup_restart_requires_durable_state_match() {
    let _environment = test_environment();
    // Break caught: cleanup recovery trusting in-memory identity not bound to durable state.
    let mut prepared = cleanup_pending_transaction("draft-cleanup-durable-state");
    prepared
        .state
        .draft_process
        .as_mut()
        .expect("draft process")
        .argv_digest = "foreign-in-memory-argv".to_string();

    let error = prepared
        .publish()
        .expect_err("in-memory cleanup identity must match durable state");

    assert!(
        error.contains("durable") || error.contains("state"),
        "{error}"
    );
    let calls = fs::read_to_string(prepared.fixture.root.join("gh-calls")).expect("gh calls");
    assert_eq!(calls.matches("pr create").count(), 0);
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_cleanup_restart_rejects_live_and_foreign_child_identity() {
    let _environment = test_environment();
    // Break caught: cleanup recovery retrying while the recorded PID is live or reused.
    for foreign in [false, true] {
        let mut prepared = cleanup_pending_transaction(if foreign {
            "draft-cleanup-foreign-child"
        } else {
            "draft-cleanup-live-child"
        });
        let mut child = Command::new("/bin/sh")
            .args(["-c", "sleep 30"])
            .spawn()
            .expect("live draft child");
        let birth = bridge::observe_process_birth(child.id())
            .expect("observe draft child")
            .expect("live draft child birth");
        prepared.state.draft_process = Some(bridge::ProcessIdentity {
            pid: birth.pid,
            process_group: birth.process_group,
            executable: PathBuf::from("/bin/sh"),
            argv_digest: "cleanup-live-child".to_string(),
            boot_id: birth.boot_id,
            start_identity: if foreign {
                format!("{}-foreign", birth.start_identity)
            } else {
                birth.start_identity
            },
        });
        bridge::write_invocation_atomic(&prepared.state_path, &prepared.state)
            .expect("live cleanup-pending invocation");

        let error = prepared
            .publish()
            .expect_err("live or foreign draft identity must prohibit retry");

        assert!(
            error.contains("live") || error.contains("identity") || error.contains("cleanup"),
            "{error}"
        );
        let calls = fs::read_to_string(prepared.fixture.root.join("gh-calls")).expect("gh calls");
        assert_eq!(calls.matches("pr create").count(), 0);
        child.kill().expect("stop live draft child");
        child.wait().expect("reap live draft child");
    }
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_cleanup_restart_rejects_any_exact_draft() {
    let _environment = test_environment();
    // Break caught: cleanup recovery adopting an exact PR instead of proving zero requests.
    let mut prepared = cleanup_pending_transaction("draft-cleanup-exact-pr");
    fs::copy(
        adapter_path(&prepared.adapter, "GH_CREATED_PR"),
        adapter_path(&prepared.adapter, "GH_PR_STATE"),
    )
    .expect("exact draft fixture");

    let error = prepared
        .publish()
        .expect_err("cleanup recovery requires zero exact drafts");

    assert!(
        error.contains("pull request") || error.contains("remote"),
        "{error}"
    );
    let calls = fs::read_to_string(prepared.fixture.root.join("gh-calls")).expect("gh calls");
    assert_eq!(calls.matches("pr create").count(), 0);
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_unlink_failure_never_authorizes_draft_retry() {
    let _environment = test_environment();
    // Break caught: ignored receipt unlink failure being mistaken for proven safe cleanup.
    let mut prepared = prepared_draft_transaction("draft-create-unlink-failure");
    let executable = prepared.adapter.gh.clone();
    let executable_body = fs::read(&executable).expect("fixture executable body");
    let executable_mode = fs::metadata(&executable)
        .expect("fixture executable metadata")
        .permissions()
        .mode();
    prepared.adapter.environment.insert(
        "AUTOSPEC_TEST_DRAFT_REMOVE_EXECUTABLE_BEFORE_RELEASE".into(),
        "1".into(),
    );
    prepared
        .adapter
        .environment
        .insert("AUTOSPEC_TEST_DRAFT_FAIL_RECEIPT_UNLINK".into(), "1".into());

    let error = prepared
        .publish()
        .expect_err("receipt unlink failure must remain fail-closed");

    assert!(
        error.contains("cleanup") && error.contains("unlink"),
        "{error}"
    );
    assert!(bridge::draft_release_receipt_path(&prepared.state_path).exists());
    assert!(prepared
        .state_path
        .with_extension("draft-release-intent")
        .exists());
    let calls = fs::read_to_string(prepared.fixture.root.join("gh-calls")).expect("gh calls");
    assert_eq!(calls.matches("pr create").count(), 0);

    fs::write(&executable, executable_body).expect("restore fixture executable");
    fs::set_permissions(&executable, fs::Permissions::from_mode(executable_mode))
        .expect("restore fixture executable mode");
    prepared.adapter.environment.remove(std::ffi::OsStr::new(
        "AUTOSPEC_TEST_DRAFT_REMOVE_EXECUTABLE_BEFORE_RELEASE",
    ));
    prepared.adapter.environment.remove(std::ffi::OsStr::new(
        "AUTOSPEC_TEST_DRAFT_FAIL_RECEIPT_UNLINK",
    ));

    let error = prepared
        .publish()
        .expect_err("failed unlink must prohibit a later request");
    assert!(
        error.contains("released") && error.contains("ambiguous"),
        "{error}"
    );
    let calls = fs::read_to_string(prepared.fixture.root.join("gh-calls")).expect("gh calls");
    assert_eq!(calls.matches("pr create").count(), 0);
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_cleanup_fsync_failure_never_authorizes_draft_retry() {
    let _environment = test_environment();
    // Break caught: ignored receipt-directory fsync failure allowing an unproven retry.
    let mut prepared = prepared_draft_transaction("draft-create-cleanup-fsync-failure");
    let executable = prepared.adapter.gh.clone();
    let executable_body = fs::read(&executable).expect("fixture executable body");
    let executable_mode = fs::metadata(&executable)
        .expect("fixture executable metadata")
        .permissions()
        .mode();
    prepared.adapter.environment.insert(
        "AUTOSPEC_TEST_DRAFT_REMOVE_EXECUTABLE_BEFORE_RELEASE".into(),
        "1".into(),
    );
    prepared.adapter.environment.insert(
        "AUTOSPEC_TEST_DRAFT_FAIL_RECEIPT_DIRECTORY_FSYNC".into(),
        "1".into(),
    );

    let error = prepared
        .publish()
        .expect_err("receipt directory fsync failure must remain fail-closed");

    assert!(
        error.contains("cleanup") && error.contains("sync"),
        "{error}"
    );
    assert!(!bridge::draft_release_receipt_path(&prepared.state_path).exists());
    assert!(prepared
        .state_path
        .with_extension("draft-release-intent")
        .exists());
    let calls = fs::read_to_string(prepared.fixture.root.join("gh-calls")).expect("gh calls");
    assert_eq!(calls.matches("pr create").count(), 0);

    fs::write(&executable, executable_body).expect("restore fixture executable");
    fs::set_permissions(&executable, fs::Permissions::from_mode(executable_mode))
        .expect("restore fixture executable mode");
    prepared.adapter.environment.remove(std::ffi::OsStr::new(
        "AUTOSPEC_TEST_DRAFT_REMOVE_EXECUTABLE_BEFORE_RELEASE",
    ));
    prepared.adapter.environment.remove(std::ffi::OsStr::new(
        "AUTOSPEC_TEST_DRAFT_FAIL_RECEIPT_DIRECTORY_FSYNC",
    ));

    let error = prepared
        .publish()
        .expect_err("failed cleanup sync must prohibit a later request");
    assert!(
        error.contains("release intent") && error.contains("refusing retry"),
        "{error}"
    );
    let calls = fs::read_to_string(prepared.fixture.root.join("gh-calls")).expect("gh calls");
    assert_eq!(calls.matches("pr create").count(), 0);
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_never_retries_a_released_draft_without_visible_pr() {
    let _environment = test_environment();
    // Break caught: treating a child-recorded request release as a safe pre-request crash.
    let mut prepared = prepared_draft_transaction("draft-create-released-ambiguous");
    prepared.push_exact_at_intent();
    prepared.state.phase = BridgePhase::DraftCreating;
    prepared.state.draft_process = Some(bridge::ProcessIdentity {
        pid: 4_000_001,
        process_group: 4_000_001,
        executable: prepared.adapter.gh.clone(),
        argv_digest: "released-request".to_string(),
        boot_id: "missing-boot".to_string(),
        start_identity: "missing-start".to_string(),
    });
    bridge::write_invocation_atomic(&prepared.state_path, &prepared.state)
        .expect("released create identity");
    let receipt = bridge::draft_release_receipt_path(&prepared.state_path);
    let digest = bridge::draft_release_digest(
        &prepared.state,
        prepared
            .state
            .draft_process
            .as_ref()
            .expect("draft process"),
    );
    fs::write(&receipt, digest).expect("released receipt");
    fs::set_permissions(&receipt, fs::Permissions::from_mode(0o600))
        .expect("private release receipt");
    fs::write(prepared.fixture.root.join("gh-calls"), "").expect("clear calls");

    let error = prepared
        .publish()
        .expect_err("released request without an authoritative PR is ambiguous");

    assert!(
        error.contains("released") && error.contains("ambiguous"),
        "{error}"
    );
    let calls = fs::read_to_string(prepared.fixture.root.join("gh-calls")).expect("gh calls");
    assert_eq!(calls.matches("pr create").count(), 0);
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_issue_contract_blocks_missing_and_pathless_outlines_before_remote_mutation(
) {
    let _environment = test_environment();
    // Break caught: compatibility lint treating a missing/pathless outline as unrestricted.
    for (case, issue_body) in [
        ("missing", "## Goal\n\nImplement the executor behavior.\n"),
        (
            "pathless",
            "## Goal\n\nImplement the executor behavior.\n\n\
             ## Implementation outline\n\n- Update the executor behavior.\n",
        ),
    ] {
        let (fixture, mut state, snapshot, closeout) =
            implementation_proof_fixture(&format!("issue-contract-{case}"));
        let state_path = fixture.root.join("state/invocation.json");
        let adapter = draft_pr_adapter_fixture(&fixture, &state_path, "[]");
        state.phase = BridgePhase::Pending;
        bridge::write_invocation_atomic(&state_path, &state).expect("pending invocation");
        bridge::RemoteMutationSnapshot::capture_and_persist(&state_path, &mut state, &adapter)
            .expect("prelaunch remote");
        state.phase = BridgePhase::ImplementationComplete;
        commit_implementation(&state);
        let proof = bridge::prove_implementation(&state_path, &mut state, &snapshot, &closeout)
            .expect("prove implementation");

        let error = bridge::push_and_create_draft(
            &state_path,
            &mut state,
            &proof,
            "Implement issue contract",
            issue_body,
            &adapter,
        )
        .expect_err("missing or pathless outline must block before remote mutation");

        assert!(error.contains("OUT_OF_SCOPE"), "{case}: {error}");
        assert!(!git_stdout(
            &fixture.root,
            &[
                "--git-dir",
                fixture.root.join("remote.git").to_str().unwrap(),
                "show-ref",
            ],
        )
        .contains(&state.identity.branch));
        let calls = fs::read_to_string(fixture.root.join("gh-calls")).expect("gh calls");
        assert_eq!(calls.matches("pr create").count(), 0);
    }
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_lint_blocks_before_git_or_gh_mutation() {
    let _environment = test_environment();
    // Break caught: a deterministic unfinished-work finding reaching a remote boundary.
    let (fixture, mut state, snapshot, closeout) = implementation_proof_fixture("draft-lint-block");
    let state_path = fixture.root.join("state/invocation.json");
    let adapter = draft_pr_adapter_fixture(&fixture, &state_path, "[]");
    state.phase = BridgePhase::Pending;
    bridge::write_invocation_atomic(&state_path, &state).expect("pending invocation");
    bridge::RemoteMutationSnapshot::capture_and_persist(&state_path, &mut state, &adapter)
        .expect("prelaunch remote");
    state.phase = BridgePhase::ImplementationComplete;
    fs::write(
        state.identity.worktree.join("unsafe.rs"),
        format!("fn unsafe_change() {{ /* {} */ }}\n", ["TO", "DO"].concat()),
    )
    .expect("lint finding");
    commit_implementation(&state);
    let proof = bridge::prove_implementation(&state_path, &mut state, &snapshot, &closeout)
        .expect("prove implementation");

    let error = bridge::push_and_create_draft(
        &state_path,
        &mut state,
        &proof,
        "Implement issue",
        "## Implementation outline\n\n- implementation.txt\n- unsafe.rs\n- .autospec/executor-closeout.md\n",
        &adapter,
    )
    .expect_err("lint must block");

    assert!(error.contains("TODO_LEFT"), "{error}");
    assert!(git_stdout(
        &fixture.root,
        &[
            "--git-dir",
            fixture.root.join("remote.git").to_str().unwrap(),
            "show-ref",
        ],
    )
    .lines()
    .all(|line| !line.ends_with(&format!("refs/heads/{}", state.identity.branch))));
    let calls = fs::read_to_string(fixture.root.join("gh-calls")).expect("gh calls");
    assert_eq!(calls.matches("pr create").count(), 0);
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_pr_size_blocks_oversized_push_and_draft_without_mutation() {
    let _environment = test_environment();
    // Break caught: an oversized exact-head diff reaching git push or gh pr create.
    for (label, files, lines) in [("lines", 1, 401), ("files", 9, 1)] {
        let (fixture, mut state, snapshot, closeout) =
            implementation_proof_fixture(&format!("pr-size-{label}"));
        let state_path = fixture.root.join("state/invocation.json");
        let adapter = draft_pr_adapter_fixture(&fixture, &state_path, "[]");
        state.phase = BridgePhase::Pending;
        bridge::write_invocation_atomic(&state_path, &state).expect("pending");
        bridge::RemoteMutationSnapshot::capture_and_persist(&state_path, &mut state, &adapter)
            .expect("remote baseline");
        state.phase = BridgePhase::ImplementationComplete;
        for file in 0..files {
            fs::write(
                state.identity.worktree.join(format!("slice-{file}.txt")),
                "changed\n".repeat(lines),
            )
            .expect("oversized slice");
        }
        git(&state.identity.worktree, &["add", "slice-*.txt"]);
        git(
            &state.identity.worktree,
            &["commit", "-m", "test: oversized slice"],
        );
        let proof = bridge::prove_implementation(&state_path, &mut state, &snapshot, &closeout)
            .expect("proof");
        let admission = bridge::evaluate_patch_size_admission(&state, &proof.head_oid, "")
            .expect_err("exact oversized diff must be rejected before any remote transition");
        assert!(admission.contains("PR_SIZE"), "{admission}");
        let diff = bridge::git_stdout(
            &state.identity.worktree,
            &[
                "diff",
                "--unified=3",
                &state.identity.base_oid,
                &proof.head_oid,
            ],
        )
        .and_then(|diff| bridge::parse_unified_diff(&diff).map_err(|error| error.to_string()))
        .expect("exact oversized diff");
        let size = bridge::evaluate_patch_size(&diff, bridge::PatchSizeLimits::default()).size();
        assert_eq!((size.changed_lines, size.raw_files), (files * lines, files));
        let outline = (0..files)
            .map(|file| format!("- slice-{file}.txt"))
            .collect::<Vec<_>>()
            .join("\n");

        let error = bridge::push_and_create_draft(
            &state_path,
            &mut state,
            &proof,
            "Oversized slice",
            &format!("## Implementation outline\n\n{outline}\n"),
            &adapter,
        )
        .expect_err("oversized slice must fail closed");

        assert!(error.contains("PR_SIZE"), "{label}: {error}");
        assert!(!git_stdout(
            &fixture.root,
            &[
                "--git-dir",
                fixture.root.join("remote.git").to_str().unwrap(),
                "show-ref",
            ],
        )
        .contains(&state.identity.branch));
        assert!(!fs::read_to_string(fixture.root.join("gh-calls"))
            .expect("gh ledger")
            .contains("pr create"));
    }
}
