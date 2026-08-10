use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
#[cfg(unix)]
use std::net::{TcpListener, TcpStream};
#[cfg(unix)]
use std::os::unix::fs::{symlink, PermissionsExt};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use autospec_core::runtime_env::{EnvironmentLifecycle, EnvironmentOwner};
#[cfg(target_os = "linux")]
use nix::sys::signal::Signal;

use super::{
    build_implementer_prompt, provision_issue_worktree, recover_invocation, resolve_base,
    runtime_session_adapter, supervise_harness, validate_trusted_ownership,
    write_invocation_atomic, BridgeIdentity, BridgePhase, ExecutorBridgeRequest, HarnessConfig,
    HarnessInvocation, HarnessKind, MutationSnapshot, PersistedInvocation, ProcessIdentity,
    ResolvedBase, SupervisionConfig, SupervisionOutcome, CLAUDE_BUILTIN_TOOLS,
    CLAUDE_FORBIDDEN_TOOLS, CLAUDE_LOCAL_TOOLS,
};

mod support_base;
use support_base::*;
mod support_invocation;
use support_invocation::*;
mod support_launch;
use support_launch::*;
mod attempt_generation;
mod worktree_post;
mod license_checker;
mod descendant_spawn;
mod full_suite;
mod reviewer_runtime;
mod draft_release;
mod json_identity;
mod quarantine_nested;
mod dispatcher_temporary;
mod generation_input;
mod cleanup_reap;
mod closeout_repairs;
mod snapshot_identity;
mod branch_predecessor;
mod runtime_fixture;
mod terminal_label;
mod reviewer_automatic;
mod prunable_zero;
mod codex_permission;
mod identity_reviewer;
mod repair_implementation;
mod closeout_harness;
mod pull_mutation;
mod scope_root;
mod production_entry;
mod commit_rust;
mod codex_sandbox;
mod closeout_remote;
mod sidecar_launch;
mod restart_direct;
mod adoption_cleanup;
mod ready_harness;
mod rust_commit;
mod result_reviewer;
mod remote_base;
mod sync_integration;
mod harness_supervisor;
mod continuation_event;

#[cfg(target_os = "linux")]
fn cleanup_pending_transaction(label: &str) -> PreparedDraftTransaction {
    let mut prepared = prepared_draft_transaction(label);
    prepared.push_exact_at_intent();
    prepared.state.phase = BridgePhase::DraftCleanupPending;
    prepared.state.draft_process = Some(super::ProcessIdentity {
        pid: 4_000_002,
        process_group: 4_000_002,
        executable: prepared.adapter.gh.clone(),
        argv_digest: format!("{label}-dead-draft"),
        boot_id: "missing-boot".to_string(),
        start_identity: "missing-start".to_string(),
    });
    super::write_invocation_atomic(&prepared.state_path, &prepared.state)
        .expect("cleanup-pending invocation");
    fs::write(prepared.fixture.root.join("gh-calls"), "").expect("clear calls");
    prepared
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_cleanup_restart_requires_both_guards_absent() {
    // Break caught: cleanup recovery retrying while a release guard still exists.
    for guard in ["receipt", "intent"] {
        let mut prepared = cleanup_pending_transaction(&format!("draft-cleanup-guard-{guard}"));
        let path = if guard == "receipt" {
            super::draft_release_receipt_path(&prepared.state_path)
        } else {
            super::draft_release_intent_path(&prepared.state_path)
        };
        let process = prepared
            .state
            .draft_process
            .as_ref()
            .expect("draft process");
        fs::write(&path, super::draft_release_digest(&prepared.state, process))
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
        let calls =
            fs::read_to_string(prepared.fixture.root.join("gh-calls")).expect("gh calls");
        assert_eq!(calls.matches("pr create").count(), 0);
    }
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_cleanup_restart_requires_both_directory_syncs() {
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
        let calls =
            fs::read_to_string(prepared.fixture.root.join("gh-calls")).expect("gh calls");
        assert_eq!(calls.matches("pr create").count(), 0);
    }
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_cleanup_restart_rejects_public_guard_parent() {
    // Break caught: cleanup recovery silently repairing an untrusted guard directory.
    let mut prepared = cleanup_pending_transaction("draft-cleanup-public-guard-parent");
    let parent = prepared
        .state_path
        .parent()
        .expect("state parent")
        .to_path_buf();
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o755))
        .expect("public guard parent");

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
        let birth = super::observe_process_birth(child.id())
            .expect("observe draft child")
            .expect("live draft child birth");
        prepared.state.draft_process = Some(super::ProcessIdentity {
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
        super::write_invocation_atomic(&prepared.state_path, &prepared.state)
            .expect("live cleanup-pending invocation");

        let error = prepared
            .publish()
            .expect_err("live or foreign draft identity must prohibit retry");

        assert!(
            error.contains("live") || error.contains("identity") || error.contains("cleanup"),
            "{error}"
        );
        let calls =
            fs::read_to_string(prepared.fixture.root.join("gh-calls")).expect("gh calls");
        assert_eq!(calls.matches("pr create").count(), 0);
        child.kill().expect("stop live draft child");
        child.wait().expect("reap live draft child");
    }
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_cleanup_restart_rejects_any_exact_draft() {
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
    assert!(super::draft_release_receipt_path(&prepared.state_path).exists());
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
    assert!(!super::draft_release_receipt_path(&prepared.state_path).exists());
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
    // Break caught: treating a child-recorded request release as a safe pre-request crash.
    let mut prepared = prepared_draft_transaction("draft-create-released-ambiguous");
    prepared.push_exact_at_intent();
    prepared.state.phase = BridgePhase::DraftCreating;
    prepared.state.draft_process = Some(super::ProcessIdentity {
        pid: 4_000_001,
        process_group: 4_000_001,
        executable: prepared.adapter.gh.clone(),
        argv_digest: "released-request".to_string(),
        boot_id: "missing-boot".to_string(),
        start_identity: "missing-start".to_string(),
    });
    super::write_invocation_atomic(&prepared.state_path, &prepared.state)
        .expect("released create identity");
    let receipt = super::draft_release_receipt_path(&prepared.state_path);
    let digest = super::draft_release_digest(
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
        super::write_invocation_atomic(&state_path, &state).expect("pending invocation");
        super::RemoteMutationSnapshot::capture_and_persist(&state_path, &mut state, &adapter)
            .expect("prelaunch remote");
        state.phase = BridgePhase::ImplementationComplete;
        commit_implementation(&state);
        let proof = super::prove_implementation(&state_path, &mut state, &snapshot, &closeout)
            .expect("prove implementation");

        let error = super::push_and_create_draft(
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
    // Break caught: a deterministic unfinished-work finding reaching a remote boundary.
    let (fixture, mut state, snapshot, closeout) =
        implementation_proof_fixture("draft-lint-block");
    let state_path = fixture.root.join("state/invocation.json");
    let adapter = draft_pr_adapter_fixture(&fixture, &state_path, "[]");
    state.phase = BridgePhase::Pending;
    super::write_invocation_atomic(&state_path, &state).expect("pending invocation");
    super::RemoteMutationSnapshot::capture_and_persist(&state_path, &mut state, &adapter)
        .expect("prelaunch remote");
    state.phase = BridgePhase::ImplementationComplete;
    fs::write(
        state.identity.worktree.join("unsafe.rs"),
        format!("fn unsafe_change() {{ /* {} */ }}\n", ["TO", "DO"].concat()),
    )
    .expect("lint finding");
    commit_implementation(&state);
    let proof = super::prove_implementation(&state_path, &mut state, &snapshot, &closeout)
        .expect("prove implementation");

    let error = super::push_and_create_draft(
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
    // Break caught: an oversized exact-head diff reaching git push or gh pr create.
    for (label, files, lines) in [("lines", 1, 401), ("files", 9, 1)] {
        let (fixture, mut state, snapshot, closeout) =
            implementation_proof_fixture(&format!("pr-size-{label}"));
        let state_path = fixture.root.join("state/invocation.json");
        let adapter = draft_pr_adapter_fixture(&fixture, &state_path, "[]");
        state.phase = BridgePhase::Pending;
        super::write_invocation_atomic(&state_path, &state).expect("pending");
        super::RemoteMutationSnapshot::capture_and_persist(&state_path, &mut state, &adapter)
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
        let proof = super::prove_implementation(&state_path, &mut state, &snapshot, &closeout)
            .expect("proof");
        let admission = super::evaluate_patch_size_admission(&state, &proof.head_oid, "")
            .expect_err("exact oversized diff must be rejected before any remote transition");
        assert!(admission.contains("PR_SIZE"), "{admission}");
        let diff = super::git_stdout(
            &state.identity.worktree,
            &[
                "diff",
                "--unified=3",
                &state.identity.base_oid,
                &proof.head_oid,
            ],
        )
        .and_then(|diff| super::parse_unified_diff(&diff).map_err(|error| error.to_string()))
        .expect("exact oversized diff");
        let size = super::evaluate_patch_size(&diff, super::PatchSizeLimits::default()).size();
        assert_eq!((size.changed_lines, size.raw_files), (files * lines, files));
        let outline = (0..files)
            .map(|file| format!("- slice-{file}.txt"))
            .collect::<Vec<_>>()
            .join("\n");

        let error = super::push_and_create_draft(
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

#[cfg(unix)]
#[test]
fn bound_continuation_publication_is_ordered_and_restart_safe() {
    // Break caught: proactive receipts existed locally but never became ordered GitHub work.
    let _environment = TEST_ENVIRONMENT.lock().expect("test environment lock");
    let (fixture, mut state, _, _) = implementation_proof_fixture("continuation-publication");
    let state_path = fixture.root.join("state/invocation.json");
    let event_log = fixture.root.join("events.jsonl");
    let store = fixture.root.join("continuation-gh");
    let bin = fixture.root.join("continuation-bin");
    fs::create_dir_all(store.join("issues")).expect("issue store");
    fs::create_dir_all(store.join("comments")).expect("comment store");
    fs::create_dir_all(&bin).expect("fake gh bin");
    fs::write(store.join("next"), "100").expect("issue sequence");
    write_executable(
        &bin.join("gh"),
        r#"#!/bin/sh
set -eu; printf '%s\n' "$*" >> "$GH_CALLS"
case "$1 $2" in
"issue view") issue=$3; case "$*" in
*"--json comments"*) test ! -f "$GH_STORE/comments/$issue" || cat "$GH_STORE/comments/$issue";;
*"--json body"*) cat "$GH_STORE/issues/$issue.body";;
*"--json state"*) printf 'OPEN\n';;
*) exit 64;;
  esac;;
"issue list") marker=
  while [ "$#" -gt 0 ]; do [ "$1" != "--search" ] || marker=$2; shift; done; marker=${marker% in:body}
  for body in "$GH_STORE"/issues/*.body; do if [ -f "$body" ] && grep -Fq "$marker" "$body"; then basename "$body" .body; fi; done;;
"issue create") body= title=; while [ "$#" -gt 0 ]; do case "$1" in --body) body=$2; shift;; --title) title=$2; shift;; esac; shift; done
  number=$(cat "$GH_STORE/next"); number=$((number + 1)); printf '%s' "$number" > "$GH_STORE/next"
  printf '%s\n' "$body" > "$GH_STORE/issues/$number.body"; printf '%s\n' "$title" > "$GH_STORE/issues/$number.title"; printf 'https://example.invalid/issues/%s\n' "$number";;
"issue comment") issue=$3; body=
  while [ "$#" -gt 0 ]; do [ "$1" != "--body" ] || { body=$2; shift; }; shift; done; printf '%s\n' "$body" > "$GH_STORE/comments/$issue";;
"api user") printf 'berlinguyinca\n';;
*) exit 64;;
esac
"#,
    );
    let previous_path = std::env::var_os("PATH");
    let previous_store = std::env::var_os("GH_STORE");
    let previous_calls = std::env::var_os("GH_CALLS");
    std::env::set_var("PATH", format!("{}:/usr/bin:/bin", bin.display()));
    std::env::set_var("GH_STORE", &store);
    std::env::set_var("GH_CALLS", store.join("calls"));
    let quoted = store.join("comments/42");
    fs::write(&quoted, "Ordinary issue quoting Part of #9").unwrap();
    assert_eq!(super::continuation_parent("owner/repo", 42).unwrap(), None);
    fs::remove_file(quoted).unwrap();
    for name in [
        "skills/a/SKILL.md",
        "skills/a/codex/prompt.md",
        "skills/a/opencode/agent.md",
        "skills/b/SKILL.md",
        "skills/b/codex/prompt.md",
        "skills/b/opencode/agent.md",
        "slice.txt",
    ] {
        fs::create_dir_all(state.identity.worktree.join(name).parent().unwrap())
            .expect("proactive directory");
        fs::write(state.identity.worktree.join(name), "changed\n").expect("proactive file");
        git(&state.identity.worktree, &["add", name]);
    }
    git(
        &state.identity.worktree,
        &["commit", "-m", "test: proactive continuation"],
    );
    let head = git_stdout(&state.identity.worktree, &["rev-parse", "HEAD"]);
    state.head_oid = Some(head.clone());
    let proof = super::ImplementationProof {
        head_oid: head,
        closeout_body: "## Closeout report\nResult: slice\nClaims: [verified] static slice\nProof type: static\nBefore/after: 0 to 1\nArtifacts: slice-0.txt; `git diff`\nScoped git status: slice files\nOne likely hidden failure: boundary\nCompleted criteria: [\"first current slice criterion with a reproducible command and exact artifact path\",\"second current slice criterion with a reproducible command and exact artifact path\"]\nUnmet criteria: [\"Run `scripts/continuation-second.sh` and verify café café café café café café café café café café café\",\"Run `scripts/continuation-third.sh` once\"]\n".into(),
    };
    super::require_continuation_checkpoint(&state_path, &event_log, &state, &proof, "", true)
        .expect("proactive continuation");
    let bound =
        super::PersistedInvocation::from_json(&fs::read_to_string(&state_path).unwrap())
            .unwrap();
    assert_eq!((bound.umbrella, bound.current_child), (Some(42), Some(101)));
    let parent = fs::read_to_string(store.join("comments/42")).unwrap();
    assert!(parent.contains("append-only-parent-extension"));
    let mut sibling_misbinding = bound.clone();
    sibling_misbinding.current_child = Some(102);
    assert!(super::recover_bound_continuation(&sibling_misbinding).is_err());
    let receipt =
        super::continuation_receipt_path(&state_path, &proof.head_oid).expect("receipt path");
    assert!(receipt.exists(), "seven files must plan a continuation");
    let bodies = [101, 102, 103]
        .map(|number| fs::read_to_string(store.join(format!("issues/{number}.body"))).unwrap());
    assert!(bodies[0].contains("ordinal=1"));
    assert!(bodies[1].contains("Depends on issue #101"));
    assert!(bodies[2].contains("Depends on issue #102"));
    fs::write(store.join("calls"), "").expect("clear calls");
    super::require_continuation_checkpoint(&state_path, &event_log, &state, &proof, "", true)
        .expect("publication restart");
    assert!(!fs::read_to_string(store.join("calls"))
        .expect("restart calls")
        .contains("issue create"));

    git(
        &state.identity.worktree,
        &[
            "push",
            "origin",
            &format!("{}:refs/heads/{}", proof.head_oid, state.identity.branch),
        ],
    );
    let mut rebound = bound.clone();
    rebound.phase = super::BridgePhase::DraftCreated;
    rebound.pr = Some(17);
    super::write_invocation_atomic(&state_path, &rebound).expect("bound draft state");
    fs::write(
        fixture.root.join("seed/continuation-base.txt"),
        "base drift\n",
    )
    .expect("advance continuation base");
    git(
        &fixture.root.join("seed"),
        &["add", "continuation-base.txt"],
    );
    git(
        &fixture.root.join("seed"),
        &["commit", "-m", "test: advance continuation base"],
    );
    git(&fixture.root.join("seed"), &["push", "origin", "main"]);
    let old_base = rebound.identity.base_oid.clone();
    assert!(
        super::reconcile_base_drift_with_refresh(&state_path, &mut rebound, || Ok(
            super::BridgeClaimOwnership::Refreshed { ttl_seconds: 60 }
        ))
        .expect("reconcile bound continuation base")
    );
    assert_ne!(rebound.identity.base_oid, old_base);
    let next_head = rebound.head_oid.clone().expect("reconciled head");
    let next_proof = super::ImplementationProof {
        head_oid: next_head,
        closeout_body: proof.closeout_body.clone(),
    };
    fs::write(store.join("calls"), "").expect("clear calls");
    super::require_continuation_checkpoint(
        &state_path,
        &event_log,
        &rebound,
        &next_proof,
        "",
        true,
    )
    .expect("bound exact-head recovery");
    let recovery_calls = fs::read_to_string(store.join("calls")).expect("recovery calls");
    assert!(!recovery_calls.contains("issue create"));
    assert_eq!(fs::read_to_string(store.join("next")).unwrap(), "103");

    let mut adverse = super::load_continuation_receipt(&state_path, &state).expect("receipt");
    adverse.unmet = vec!["Improve quality should feel nice".into()];
    adverse.content_digest = adverse.digest();
    fs::write(store.join("calls"), "").expect("clear calls");
    assert!(super::publish_continuation_children(&state_path, &adverse).is_err());
    assert!(!fs::read_to_string(store.join("calls"))
        .unwrap()
        .contains("issue create"));
    fs::write(
        store.join("comments/43"),
        "<!-- autospec-parent:10 -->\nChild issue #43 belongs to parent issue #10.\n",
    )
    .expect("existing parent marker");
    fs::write(
        store.join("comments/10"),
        "<!-- autospec-parent-decomposition:begin -->\nParent issue #10 was decomposed\n- #41\n- #43\n- #45\n<!-- autospec-parent-decomposition:end -->\n",
    )
    .expect("existing order");
    let mut existing = super::load_continuation_receipt(&state_path, &state).expect("receipt");
    existing.issue = 43;
    existing.content_digest = existing.digest();
    super::publish_continuation_children(&state_path, &existing).expect("parent extension");
    let first = fs::read_to_string(store.join("issues/104.body")).unwrap();
    assert!(first.contains("Depends on issue #43"));
    assert!(fs::read_to_string(store.join("issues/105.body"))
        .unwrap()
        .contains("Depends on issue #104"));
    assert!(fs::read_to_string(store.join("comments/43"))
        .unwrap()
        .contains("autospec-parent:10"));
    let title = fs::read_to_string(store.join("issues/102.title")).unwrap();
    assert!(title.trim().len() <= 120);

    let remote_before_hard = super::remote_head_refs(&fixture.repo).expect("remote refs");
    state.identity.issue = 44;
    let worktree = &state.identity.worktree;
    fs::write(worktree.join("hard.txt"), "changed\n".repeat(401)).expect("hard slice");
    git(worktree, &["add", "hard.txt"]);
    git(worktree, &["commit", "-m", "test: hard continuation"]);
    let hard_head = git_stdout(worktree, &["rev-parse", "HEAD"]);
    state.head_oid = Some(hard_head.clone());
    let hard_proof = super::ImplementationProof {
        head_oid: hard_head,
        closeout_body: proof.closeout_body.clone(),
    };
    let publish = || {
        super::require_continuation_checkpoint(
            &state_path,
            &event_log,
            &state,
            &hard_proof,
            "",
            true,
        )
    };
    fs::write(store.join("calls"), "").expect("clear calls");
    let error = publish().expect_err("hard continuation invariant");
    assert!(error
        .to_string()
        .contains("oversized continuation checkpoint"));
    let children = super::continuation_child_list(&state.identity.repository, 44)
        .expect("authoritative hard child order");
    assert_eq!(children, [106, 107, 108]);
    for (ordinal, child) in children.iter().enumerate() {
        let body =
            fs::read_to_string(store.join(format!("issues/{child}.body"))).expect("hard child");
        assert!(body.contains(&format!("ordinal={}", ordinal + 1)));
    }
    let calls = fs::read_to_string(store.join("calls")).expect("hard calls");
    assert_eq!(calls.matches("issue create").count(), 3);
    assert!(!calls.contains("pr create"));
    let refs = super::remote_head_refs(&fixture.repo).expect("remote refs");
    assert_eq!(refs, remote_before_hard);
    fs::write(store.join("calls"), "").expect("clear calls");
    assert!(publish().is_err());
    assert!(!fs::read_to_string(store.join("calls"))
        .expect("hard restart calls")
        .contains("issue create"));
    for (key, previous) in [
        ("PATH", previous_path),
        ("GH_STORE", previous_store),
        ("GH_CALLS", previous_calls),
    ] {
        match previous {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }
}

#[test]
fn autonomous_executor_bridge_reuse_lens_follows_exact_environment_contract() {
    // Break caught: draft publication enabling reuse-only detectors when the shell gate is off.
    let original = std::env::var_os("AUTOSPEC_REUSE_LENS");
    std::env::remove_var("AUTOSPEC_REUSE_LENS");
    assert!(!super::implementation_lint_options().enable_reuse_lens);
    std::env::set_var("AUTOSPEC_REUSE_LENS", "off");
    assert!(!super::implementation_lint_options().enable_reuse_lens);
    std::env::set_var("AUTOSPEC_REUSE_LENS", "1");
    assert!(super::implementation_lint_options().enable_reuse_lens);
    match original {
        Some(value) => std::env::set_var("AUTOSPEC_REUSE_LENS", value),
        None => std::env::remove_var("AUTOSPEC_REUSE_LENS"),
    }
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_lints_the_proven_tree_not_mutable_worktree() {
    // Break caught: an uncommitted allow marker suppressing an unsafe committed diff.
    let (fixture, mut state, snapshot, closeout) =
        implementation_proof_fixture("draft-lint-exact-tree");
    let state_path = fixture.root.join("state/invocation.json");
    let adapter = draft_pr_adapter_fixture(&fixture, &state_path, "[]");
    state.phase = BridgePhase::Pending;
    super::write_invocation_atomic(&state_path, &state).expect("pending invocation");
    super::RemoteMutationSnapshot::capture_and_persist(&state_path, &mut state, &adapter)
        .expect("prelaunch remote");
    state.phase = BridgePhase::ImplementationComplete;
    let unsafe_path = state.identity.worktree.join("unsafe.rs");
    let unsafe_source = ["fn unsafe_change() { ev", "al(input); }\n"].concat();
    fs::write(&unsafe_path, unsafe_source).expect("unsafe change");
    git(&state.identity.worktree, &["add", "."]);
    git(
        &state.identity.worktree,
        &["commit", "-m", "feat: unsafe fixture"],
    );
    let proof = super::prove_implementation(&state_path, &mut state, &snapshot, &closeout)
        .expect("prove unsafe committed implementation");
    let mutable_allow_marker = [
        "// linter:allow-SECURITY fixture marker is not committed\nev",
        "al(input);\n",
    ]
    .concat();
    fs::write(&unsafe_path, mutable_allow_marker).expect("mutable allow marker");

    let error = super::push_and_create_draft(
        &state_path,
        &mut state,
        &proof,
        "Implement issue",
        "## Implementation outline\n\n- unsafe.rs\n- .autospec/closeout.md\n",
        &adapter,
    )
    .expect_err("mutable worktree cannot suppress exact-tree lint");

    assert!(error.contains("worktree must be clean"), "{error}");
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
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_rejects_nonexact_authoritative_drafts() {
    // Break caught: treating partial PR identity or additional PRs as the one owned draft.
    for variant in [
        "missing",
        "multiple",
        "wrong-head",
        "wrong-base",
        "ready",
        "wrong-body",
        "extra",
    ] {
        let mut prepared = prepared_draft_transaction(&format!("draft-invalid-{variant}"));
        let created_path = adapter_path(&prepared.adapter, "GH_CREATED_PR");
        let exact: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&created_path).unwrap()).unwrap();
        let mut pull_requests = exact.as_array().unwrap().clone();
        match variant {
            "missing" => pull_requests.clear(),
            "multiple" => {
                let mut duplicate = pull_requests[0].clone();
                duplicate["number"] = 18.into();
                pull_requests.push(duplicate);
            }
            "wrong-head" => {
                pull_requests[0]["headRefOid"] =
                    "ffffffffffffffffffffffffffffffffffffffff".into()
            }
            "wrong-base" => pull_requests[0]["baseRefName"] = "other".into(),
            "ready" => pull_requests[0]["isDraft"] = false.into(),
            "wrong-body" => pull_requests[0]["body"] = "## Closeout report\n".into(),
            "extra" => {
                pull_requests.push(serde_json::json!({
                    "number": 18,
                    "body": "Closes #99",
                    "headRefName": "foreign",
                    "headRefOid": "ffffffffffffffffffffffffffffffffffffffff",
                    "isDraft": true,
                    "baseRefName": "main"
                }));
            }
            _ => unreachable!(),
        }
        fs::write(
            &created_path,
            serde_json::Value::Array(pull_requests).to_string(),
        )
        .expect("invalid authoritative PR fixture");

        let error = super::push_and_create_draft(
            &prepared.state_path,
            &mut prepared.state,
            &prepared.proof,
            "Implement issue",
            "## Implementation outline\n\n- implementation.txt\n- .autospec/executor-closeout.md\n",
            &prepared.adapter,
        )
        .expect_err("nonexact draft must fail closed");

        assert!(
            error.contains("exact") || error.contains("extra"),
            "{variant}: {error}"
        );
        assert_eq!(prepared.state.phase, BridgePhase::DraftCreating);
    }
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_rejects_prelaunch_remote_snapshot_drift() {
    // Break caught: harness-created extra refs or PRs being mistaken for Rust-owned mutations.
    let mut branch = prepared_draft_transaction("draft-extra-branch");
    git(
        &branch.fixture.root.join("seed"),
        &["push", "origin", "HEAD:refs/heads/foreign"],
    );
    let error = super::push_and_create_draft(
        &branch.state_path,
        &mut branch.state,
        &branch.proof,
        "Implement issue",
        "## Implementation outline\n\n- implementation.txt\n- .autospec/closeout.md\n",
        &branch.adapter,
    )
    .expect_err("extra branch must fail closed");
    assert!(error.contains("mutated remote"), "{error}");

    let mut tag = prepared_draft_transaction("draft-extra-tag");
    git(&tag.fixture.root.join("seed"), &["tag", "foreign-tag"]);
    git(
        &tag.fixture.root.join("seed"),
        &["push", "origin", "refs/tags/foreign-tag"],
    );
    let error = tag
        .publish()
        .expect_err("extra safe-namespace tag must fail closed");
    assert!(error.contains("mutated remote"), "{error}");

    let mut pull_request = prepared_draft_transaction("draft-extra-pr");
    fs::write(
        adapter_path(&pull_request.adapter, "GH_PR_STATE"),
        r#"[{"number":99,"body":"Closes #99","headRefName":"foreign","headRefOid":"ffffffffffffffffffffffffffffffffffffffff","isDraft":true,"baseRefName":"main"}]"#,
    )
    .expect("extra PR");
    let error = super::push_and_create_draft(
        &pull_request.state_path,
        &mut pull_request.state,
        &pull_request.proof,
        "Implement issue",
        "## Implementation outline\n\n- implementation.txt\n- .autospec/closeout.md\n",
        &pull_request.adapter,
    )
    .expect_err("extra PR must fail closed");
    assert!(error.contains("mutated remote"), "{error}");

    let mut stale = prepared_draft_transaction("draft-stale-snapshot");
    let snapshot_path = stale.state_path.with_extension("prelaunch-remote.json");
    let mut snapshot: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&snapshot_path).unwrap()).unwrap();
    snapshot["identity"]["invocation_id"] = "stale-invocation".into();
    fs::write(&snapshot_path, snapshot.to_string()).expect("tamper snapshot identity");
    let error = super::push_and_create_draft(
        &stale.state_path,
        &mut stale.state,
        &stale.proof,
        "Implement issue",
        "## Implementation outline\n\n- implementation.txt\n- .autospec/closeout.md\n",
        &stale.adapter,
    )
    .expect_err("stale remote snapshot must fail closed");
    assert!(error.contains("identity"), "{error}");

    let mut during_create = prepared_draft_transaction("draft-ref-during-create");
    during_create.adapter.environment.insert(
        "GH_MUTATE_REF".into(),
        during_create.proof.head_oid.clone().into(),
    );
    let error = during_create
        .publish()
        .expect_err("remote ref mutation during create must fail closed");
    assert!(error.contains("remote refs"), "{error}");
    assert_ne!(during_create.state.phase, BridgePhase::DraftCreated);
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_plan_shrink_cleans_removed_trailing_index() {
    // Break caught: cleanup preflight enumerating only the new shorter plan and abandoning a
    // live interrupted tree owned by a removed trailing command index.
    let fixture = GitFixture::new("direct-plan-shrink-cleanup");
    let artifact_root = fixture.root.join("evidence");
    let marker = fixture.root.join("trailing.pid");
    let old_command = format!(
        "/usr/bin/true && /usr/bin/python3 -c 'from pathlib import Path; import os,time; Path(\"{}\").write_text(str(os.getpid())); time.sleep(30)'",
        marker.display()
    );
    let parent = Command::new(std::env::current_exe().expect("test executable"))
        .args([
            "--exact",
            "commands::autonomous::executor_bridge::tests::autonomous_executor_bridge_parent_crash_helper",
            "--nocapture",
        ])
        .env("AUTOSPEC_TEST_CRASH_REPO", &fixture.repo)
        .env("AUTOSPEC_TEST_CRASH_ARTIFACT", &artifact_root)
        .env("AUTOSPEC_TEST_CRASH_COMMAND", &old_command)
        .spawn()
        .expect("crash-parent process");
    let launch = artifact_root.join("command-001.launch.json");
    let mut cleanup = DirectCrashFixtureCleanup::new(parent, launch.clone());
    let deadline = Instant::now() + Duration::from_secs(5);
    while (!marker.is_file() || !launch.is_file()) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    let value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&launch).expect("trailing launch"))
            .expect("trailing launch JSON");
    let supervisor =
        super::parse_process_identity(value["supervisor"].clone(), "supervisor fixture")
            .expect("supervisor identity");
    let harness = super::parse_process_identity(value["process"].clone(), "harness fixture")
        .expect("harness identity");
    cleanup.arm(supervisor.clone(), harness.clone());
    cleanup.crash_parent();
    let new_plan =
        super::parse_direct_command_plan("/usr/bin/true").expect("shortened recovery plan");

    let observed = super::execute_direct_plan(
        &fixture.repo,
        &new_plan,
        &artifact_root,
        None,
        Duration::from_secs(5),
    )
    .expect("shortened plan proceeds after all-index cleanup");

    assert_eq!(observed.len(), 1);
    assert!(
        super::observe_process_birth(supervisor.pid)
            .expect("observe removed supervisor")
            .is_none(),
        "removed trailing supervisor survived plan-shrink cleanup"
    );
    assert!(
        super::observe_process_birth(harness.pid)
            .expect("observe removed harness")
            .is_none(),
        "removed trailing harness survived plan-shrink cleanup"
    );
    assert!(!launch.exists());
    assert!(
        artifact_root.join("command-001.intent.json").is_file(),
        "removed command intent must remain immutable diagnostic context"
    );
    cleanup.disarm();
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_restart_adopts_live_harness_after_supervisor_death() {
    let fixture = GitFixture::new("direct-dead-supervisor");
    let artifact_root = fixture.root.join("evidence");
    let marker = fixture.root.join("command.pid");
    let command = format!(
        "/usr/bin/python3 -c 'from pathlib import Path; import os,time; p=Path(\"{}\"); first=not p.exists(); p.write_text(str(os.getpid())); time.sleep(30) if first else None'",
        marker.display()
    );
    let mut parent = Command::new(std::env::current_exe().expect("test executable"))
        .args([
            "--exact",
            "commands::autonomous::executor_bridge::tests::autonomous_executor_bridge_parent_crash_helper",
            "--nocapture",
        ])
        .env("AUTOSPEC_TEST_CRASH_REPO", &fixture.repo)
        .env("AUTOSPEC_TEST_CRASH_ARTIFACT", &artifact_root)
        .env("AUTOSPEC_TEST_CRASH_COMMAND", &command)
        .spawn()
        .expect("crash-parent process");
    let launch = artifact_root.join("command-000.launch.json");
    let deadline = Instant::now() + Duration::from_secs(5);
    while (!marker.is_file() || !launch.is_file()) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    let launch_value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&launch).expect("durable launch record"))
            .expect("launch JSON");
    let supervisor = launch_value["supervisor"]["pid"]
        .as_i64()
        .and_then(|pid| i32::try_from(pid).ok())
        .expect("supervisor PID");
    let harness = launch_value["process"]["pid"]
        .as_i64()
        .and_then(|pid| i32::try_from(pid).ok())
        .expect("harness PID");
    nix::sys::signal::kill(nix::unistd::Pid::from_raw(supervisor), Signal::SIGKILL)
        .expect("kill stable supervisor only");
    let deadline = Instant::now() + Duration::from_secs(2);
    while Path::new(&format!("/proc/{supervisor}")).exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        Path::new(&format!("/proc/{harness}")).exists(),
        "harness must remain live after supervisor death"
    );
    parent.kill().expect("crash command parent");
    parent.wait().expect("reap crashed command parent");

    let plan = super::parse_direct_command_plan(&command).expect("recovery plan");
    let recovered = super::execute_direct_plan(
        &fixture.repo,
        &plan,
        &artifact_root,
        None,
        Duration::from_secs(5),
    )
    .expect("restart must adopt the exact live harness and retry");

    assert!(!Path::new(&format!("/proc/{harness}")).exists());
    assert!(!launch.exists());
    assert_eq!(recovered[0].terminal, super::AttemptTerminal::Exited(0));
}

#[cfg(target_os = "linux")]
fn assert_exec_replaced_direct_harness_recovers(
    fixture: &GitFixture,
    command: &str,
    marker: &Path,
) {
    let artifact_root = fixture.root.join("evidence");
    let parent = Command::new(std::env::current_exe().expect("test executable"))
        .args([
            "--exact",
            "commands::autonomous::executor_bridge::tests::autonomous_executor_bridge_parent_crash_helper",
            "--nocapture",
        ])
        .env("AUTOSPEC_TEST_CRASH_REPO", &fixture.repo)
        .env("AUTOSPEC_TEST_CRASH_ARTIFACT", &artifact_root)
        .env("AUTOSPEC_TEST_CRASH_COMMAND", command)
        .spawn()
        .expect("crash-parent process");
    let launch = artifact_root.join("command-000.launch.json");
    let mut cleanup = DirectCrashFixtureCleanup::new(parent, launch.clone());
    let deadline = Instant::now() + Duration::from_secs(5);
    while (!marker.is_file() || !launch.is_file()) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    let launch_value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&launch).expect("durable launch record"))
            .expect("launch JSON");
    let supervisor =
        super::parse_process_identity(launch_value["supervisor"].clone(), "supervisor fixture")
            .expect("supervisor identity");
    let harness =
        super::parse_process_identity(launch_value["process"].clone(), "harness fixture")
            .expect("harness identity");
    cleanup.arm(supervisor.clone(), harness.clone());
    let deadline = Instant::now() + Duration::from_secs(5);
    let observed = loop {
        let Some(observed) = super::observe_process_identity(harness.pid, &harness.argv_digest)
            .expect("observe exec-replaced harness")
        else {
            assert!(
                Instant::now() < deadline,
                "exec-replaced harness disappeared during identity transition"
            );
            std::thread::sleep(Duration::from_millis(10));
            continue;
        };
        if observed.executable != harness.executable
            || observed.argv_digest != harness.argv_digest
        {
            break observed;
        }
        assert!(
            Instant::now() < deadline,
            "fixture harness never replaced its declared exec identity"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(harness.same_birth(&observed));

    let owned =
        super::OwnedProcess::capture(&supervisor.birth()).expect("capture exact supervisor");
    owned
        .signal(Signal::SIGKILL)
        .expect("kill stable supervisor");
    let deadline = Instant::now() + Duration::from_secs(2);
    while super::observe_process_birth(supervisor.pid)
        .expect("observe supervisor exit")
        .is_some()
        && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        super::observe_process_birth(harness.pid)
            .expect("observe surviving harness")
            .is_some(),
        "exec-replaced harness must survive supervisor death"
    );
    cleanup.crash_parent();

    let plan = super::parse_direct_command_plan(command).expect("recovery plan");
    let recovered = super::execute_direct_plan(
        &fixture.repo,
        &plan,
        &artifact_root,
        None,
        Duration::from_secs(5),
    )
    .expect("restart must clean exact-birth harness and retry");

    assert!(
        super::observe_process_birth(harness.pid)
            .expect("post-recovery harness")
            .is_none(),
        "exec-replaced harness survived restart cleanup"
    );
    assert!(!launch.exists());
    assert_eq!(recovered[0].terminal, super::AttemptTerminal::Exited(0));
    cleanup.disarm();
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_restart_cleans_shebang_harness_after_supervisor_death() {
    // Break caught: cleanup recovery requiring the declared script path after Linux replaces
    // the live harness identity with its shebang interpreter.
    let fixture = GitFixture::new("direct-dead-supervisor-shebang");
    let marker = fixture.root.join("command.pid");
    let script = fixture.root.join("fixture-command");
    write_executable(
        &script,
        &format!(
            "#!/bin/sh\nif [ ! -e '{}' ]; then printf '%s' \"$$\" > '{}'; while :; do sleep 1; done; fi\n",
            marker.display(),
            marker.display()
        ),
    );

    assert_exec_replaced_direct_harness_recovers(
        &fixture,
        script.to_str().expect("script path"),
        &marker,
    );
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_restart_cleans_immediate_exec_harness_after_supervisor_death() {
    // Break caught: cleanup recovery rejecting a same-birth harness after the declared shell
    // immediately replaces itself with another executable.
    let fixture = GitFixture::new("direct-dead-supervisor-immediate-exec");
    let marker = fixture.root.join("command.pid");
    let command = format!(
        "/bin/sh -c 'if [ ! -e \"{}\" ]; then printf %s \"$$\" > \"{}\"; exec /usr/bin/sleep 30; fi'",
        marker.display(),
        marker.display()
    );

    assert_exec_replaced_direct_harness_recovers(&fixture, &command, &marker);
}

#[test]
fn autonomous_executor_bridge_runtime_infrastructure_error_is_terminal_evidence() {
    // Break caught: a runtime adapter error returning before command-000.json is persisted.
    let fixture = GitFixture::new("runtime-terminal-error");
    let artifact_root = fixture.root.join("evidence");
    let runtime = super::DirectRuntimeAdapter {
        repo: fixture.repo.clone(),
        session_id: "closed-runtime-session".into(),
        environment_dir: fixture.root.join("runtime"),
        session: std::cell::RefCell::new(None),
    };
    let plan = super::parse_direct_command_plan("/usr/bin/true").expect("direct plan");

    let error = super::execute_direct_plan(
        &fixture.repo,
        &plan,
        &artifact_root,
        Some(&runtime),
        Duration::from_secs(5),
    )
    .expect_err("closed runtime must be typed infrastructure evidence");
    let record: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(artifact_root.join("command-000.json"))
            .expect("terminal infrastructure record"),
    )
    .expect("typed command record");

    assert!(error.contains("infrastructure"), "{error}");
    assert_eq!(record["terminal"]["kind"], "infrastructure_failed");
    assert!(artifact_root.join("command-000.intent.json").is_file());
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_retries_repaired_supervisor_resolution_failure() {
    let fixture = GitFixture::new("direct-repaired-supervisor-resolution");
    let artifact_root = fixture.root.join("evidence");
    let plan = super::parse_direct_command_plan("/usr/bin/true").expect("direct plan");

    super::set_launch_failpoint(super::LaunchFailpoint::ParentReadiness);
    super::execute_direct_plan(
        &fixture.repo,
        &plan,
        &artifact_root,
        None,
        Duration::from_secs(5),
    )
    .expect_err("first attempt records cleanup-proven infrastructure failure");
    super::set_launch_failpoint(super::LaunchFailpoint::None);
    let record_path = artifact_root.join("command-000.json");
    let observed = super::read_observed_command_record(&fixture.repo, &record_path)
        .expect("read infrastructure record");
    let repaired = super::observed_command_document(
        &observed.attempt_id,
        &observed.commit_oid,
        observed.runtime_session_id.as_deref(),
        &observed.executable,
        &observed.argv,
        &observed.process_executable,
        &observed.process_argv,
        &super::AttemptTerminal::InfrastructureFailed(
            "canonicalize executor supervisor executable: No such file or directory (os error 2)"
                .to_string(),
        ),
        &observed.stdout_path,
        &observed.stdout_digest,
        &observed.stderr_path,
        &observed.stderr_digest,
    );
    fs::write(&record_path, repaired).expect("persist repaired failure fixture");

    let recovered = super::execute_direct_plan(
        &fixture.repo,
        &plan,
        &artifact_root,
        None,
        Duration::from_secs(5),
    )
    .expect("healthy supervisor resolution starts a fresh attempt");

    assert_eq!(recovered[0].terminal, super::AttemptTerminal::Exited(0));
    assert!(fs::read_dir(&artifact_root)
        .expect("failure archive")
        .flatten()
        .any(|entry| entry.file_name().to_string_lossy().contains(".archive-")));
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_cleanup_failure_retains_identity_until_restart_reconciles() {
    let fixture = GitFixture::new("direct-cleanup-quarantine");
    let artifact_root = fixture.root.join("evidence");
    let plan = super::parse_direct_command_plan("/usr/bin/true").expect("direct plan");

    super::set_cleanup_failpoint(super::LaunchFailpoint::CleanupSignal);
    let first = super::execute_direct_plan(
        &fixture.repo,
        &plan,
        &artifact_root,
        None,
        Duration::from_secs(5),
    );
    super::set_cleanup_failpoint(super::LaunchFailpoint::None);
    let first = first.expect_err("failed cleanup must be typed and quarantined");
    let launch = artifact_root.join("command-000.launch.json");
    let supervisor = direct_launch_supervisor_pid(&launch);
    let harness = serde_json::from_str::<serde_json::Value>(
        &fs::read_to_string(&launch).expect("durable direct launch identity"),
    )
    .expect("direct launch JSON")["process"]["pid"]
        .as_u64()
        .and_then(|pid| u32::try_from(pid).ok())
        .expect("direct launch harness PID");
    assert!(first.contains("cleanup"), "{first}");
    assert!(Path::new(&format!("/proc/{supervisor}")).exists());

    let retry = super::execute_direct_plan(
        &fixture.repo,
        &plan,
        &artifact_root,
        None,
        Duration::from_secs(5),
    )
    .expect("reconciled cleanup starts a fresh attempt");

    assert_eq!(retry[0].terminal, super::AttemptTerminal::Exited(0));
    assert!(!launch.exists());
    assert!(!Path::new(&format!("/proc/{supervisor}")).exists());
    assert!(!Path::new(&format!("/proc/{harness}")).exists());
    assert!(fs::read_dir(&artifact_root)
        .expect("cleanup archives")
        .flatten()
        .any(|entry| entry.file_name().to_string_lossy().contains(".archive-")));
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_resumes_failure_archive_before_one_fresh_attempt() {
    for boundary in [
        super::LaunchFailpoint::ArchiveAfterManifest,
        super::LaunchFailpoint::ArchiveMidMove,
        super::LaunchFailpoint::ArchiveBeforeComplete,
    ] {
        let fixture = GitFixture::new(&format!("direct-archive-{boundary:?}"));
        let artifact_root = fixture.root.join("evidence");
        let plan = super::parse_direct_command_plan("/usr/bin/true").expect("direct plan");
        super::set_cleanup_failpoint(super::LaunchFailpoint::CleanupSignal);
        let first = super::execute_direct_plan(
            &fixture.repo,
            &plan,
            &artifact_root,
            None,
            Duration::from_secs(5),
        );
        super::set_cleanup_failpoint(super::LaunchFailpoint::None);
        first.expect_err("first attempt leaves cleanup quarantine");

        super::set_launch_failpoint(boundary);
        let interrupted = super::execute_direct_plan(
            &fixture.repo,
            &plan,
            &artifact_root,
            None,
            Duration::from_secs(5),
        );
        super::set_launch_failpoint(super::LaunchFailpoint::None);
        interrupted.expect_err("archive transaction failpoint interrupts rollover");

        let recovered = super::execute_direct_plan(
            &fixture.repo,
            &plan,
            &artifact_root,
            None,
            Duration::from_secs(5),
        )
        .expect("restart completes archive and runs one fresh attempt");
        assert_eq!(recovered[0].terminal, super::AttemptTerminal::Exited(0));
        let archives = fs::read_dir(&artifact_root)
            .expect("archive directory")
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().contains(".archive-"))
            .collect::<Vec<_>>();
        assert_eq!(
            archives.len(),
            1,
            "one immutable archive per failed attempt"
        );
        assert!(archives[0].path().join("complete").is_file());
        assert!(archives[0].path().join("command-000.json").is_file());
        assert!(!artifact_root.join("command-000.archive.pending").exists());
    }
}

#[test]
fn autonomous_executor_bridge_retirement_resumes_every_delete_boundary() {
    // Break caught: a crash after cleanup proof deleting launch ownership without leaving a
    // durable transaction that restart can finish.
    for boundary in [
        super::LaunchFailpoint::RetireAfterProof,
        super::LaunchFailpoint::RetireMidDelete,
        super::LaunchFailpoint::RetireAfterLaunchDelete,
    ] {
        let fixture = GitFixture::new(&format!("direct-retire-{boundary:?}"));
        let artifact_root = fixture.root.join("evidence");
        fs::create_dir_all(&artifact_root).expect("artifact root");
        fs::set_permissions(&artifact_root, fs::Permissions::from_mode(0o700))
            .expect("private artifact root");
        let paths = super::direct_attempt_paths(&artifact_root, 0);
        for path in [
            &paths.launch,
            &paths.sinks.supervisor_identity,
            &paths.sinks.stdout,
            &paths.sinks.stderr,
            &paths.sinks.stdout_writer_cursor,
            &paths.sinks.stderr_writer_cursor,
            &paths.sinks.stdout_reader_cursor,
            &paths.sinks.stderr_reader_cursor,
            &paths.sinks.exit_status,
        ] {
            fs::write(path, b"owned\n").expect("retirement artifact");
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))
                .expect("private retirement artifact");
        }

        super::set_launch_failpoint(boundary);
        let attempt_id = super::new_direct_attempt_id_candidate().expect("attempt id");
        let interrupted = super::retire_direct_launch(&paths, &attempt_id);
        super::set_launch_failpoint(super::LaunchFailpoint::None);
        interrupted.expect_err("retirement failpoint must interrupt transaction");
        super::retire_direct_launch(&paths, &attempt_id).expect("restart resumes retirement");

        assert!(!paths.launch.exists());
        assert!(!paths.sinks.supervisor_identity.exists());
        assert!(
            !paths.record.with_extension("retire.pending").exists(),
            "retirement pending pointer survived commit"
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_complete_retirement_recovers_without_pending_pointer() {
    // Break caught: pending removal followed by parent-sync failure losing the only
    // cleanup-proven locator and preventing the typed failure archive/fresh retry.
    let fixture = GitFixture::new("direct-retire-pointer-cleanup");
    let artifact_root = fixture.root.join("evidence");
    let plan = super::parse_direct_command_plan("/usr/bin/true").expect("direct plan");
    super::set_cleanup_failpoint(super::LaunchFailpoint::CleanupSignal);
    let first = super::execute_direct_plan(
        &fixture.repo,
        &plan,
        &artifact_root,
        None,
        Duration::from_secs(5),
    );
    super::set_cleanup_failpoint(super::LaunchFailpoint::None);
    first.expect_err("first attempt leaves cleanup quarantine");

    super::set_launch_failpoint(super::LaunchFailpoint::RetireAfterPendingRemoval);
    let interrupted = super::execute_direct_plan(
        &fixture.repo,
        &plan,
        &artifact_root,
        None,
        Duration::from_secs(5),
    );
    super::set_launch_failpoint(super::LaunchFailpoint::None);
    interrupted.expect_err("retirement loses pending before final parent sync");
    let failed_record: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(artifact_root.join("command-000.json"))
            .expect("cleanup-failed command record"),
    )
    .expect("cleanup-failed record JSON");
    let failed_attempt_id = failed_record["attempt_id"]
        .as_str()
        .expect("cleanup-failed attempt id")
        .to_string();
    assert!(!artifact_root.join("command-000.retire.pending").exists());
    let completed_retirement = fs::read_dir(&artifact_root)
        .expect("retirement transactions")
        .flatten()
        .find(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("command-000.retire-")
                && entry.path().join("complete").is_file()
        })
        .expect("completed retirement");
    let retirement_commit: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(completed_retirement.path().join("complete"))
            .expect("retirement commit"),
    )
    .expect("retirement commit JSON");
    assert_eq!(
        retirement_commit["attempt_id"].as_str(),
        Some(failed_attempt_id.as_str()),
        "retirement and the later cleanup-failed terminal record must bind one attempt"
    );

    let recovered = super::execute_direct_plan(
        &fixture.repo,
        &plan,
        &artifact_root,
        None,
        Duration::from_secs(5),
    )
    .expect("complete retirement locator archives failure and retries");

    assert_eq!(recovered[0].terminal, super::AttemptTerminal::Exited(0));
    assert_ne!(
        recovered[0].attempt_id, failed_attempt_id,
        "fresh execution after archived cleanup failure must use a new attempt identity"
    );
    assert!(fs::read_dir(&artifact_root)
        .expect("failure archive")
        .flatten()
        .any(|entry| entry.file_name().to_string_lossy().contains(".archive-")));
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_live_sidecar_beats_stale_completed_retirement() {
    // Break caught: a completed retirement from an older attempt suppressing cleanup of a
    // newer live supervisor sidecar for the same command index.
    let fixture = GitFixture::new("direct-stale-retirement-live-sidecar");
    let artifact_root = fixture.root.join("evidence");
    fs::create_dir_all(&artifact_root).expect("artifact root");
    fs::set_permissions(&artifact_root, fs::Permissions::from_mode(0o700))
        .expect("private artifact root");
    let paths = super::direct_attempt_paths(&artifact_root, 0);
    let old_attempt_id = super::reserve_direct_attempt_id(&paths).expect("old attempt id");
    fs::write(&paths.launch, b"old ownership\n").expect("old launch");
    fs::set_permissions(&paths.launch, fs::Permissions::from_mode(0o600))
        .expect("private old launch");
    super::retire_direct_launch(&paths, &old_attempt_id).expect("retire old attempt");

    let invocation = shell_invocation(&fixture.repo, "exec /usr/bin/sleep 30");
    let validated = super::validate_invocation(
        &HarnessInvocation {
            program: invocation.program.canonicalize().expect("canonical shell"),
            args: invocation.args,
            current_dir: invocation
                .current_dir
                .canonicalize()
                .expect("canonical repo"),
            requires_mutation_snapshots: false,
        },
        &fixture.repo.canonicalize().expect("canonical fixture repo"),
    )
    .expect("validate live sidecar harness");
    let new_attempt_id = super::reserve_direct_attempt_id(&paths).expect("new attempt id");
    assert_ne!(old_attempt_id, new_attempt_id);
    let mut argv = vec![validated.program.display().to_string()];
    argv.extend(validated.args.clone());
    let intent = super::direct_intent_document(
        &new_attempt_id,
        &super::git_stdout(&fixture.repo, &["rev-parse", "--verify", "HEAD^{commit}"])
            .expect("fixture commit"),
        None,
        &validated.program,
        &argv,
    );
    super::write_private_create_once(
        &paths.intent,
        intent.as_bytes(),
        "current direct intent fixture",
    )
    .expect("write current intent");
    let mut child =
        super::spawn_blocked_harness(&validated, &paths.sinks, Some(&new_attempt_id))
            .expect("spawn current sidecar harness");
    let supervisor_pid = child.supervisor_birth().pid;
    child
        .release_launch_barrier()
        .expect("release current harness");
    drop(child);

    assert!(super::reconcile_direct_launch(&paths, Some(&intent))
        .expect("reconcile current sidecar"));
    assert!(
        super::observe_process_birth(supervisor_pid)
            .expect("observe cleaned supervisor")
            .is_none(),
        "new live sidecar was suppressed by stale retirement proof"
    );
    assert!(!paths.sinks.supervisor_identity.exists());
}

#[test]
fn autonomous_executor_bridge_terminal_record_attempt_must_match_intent() {
    // Break caught: a syntactically valid terminal record for another attempt being accepted
    // and archived solely because its argv and output digests were self-consistent.
    let fixture = GitFixture::new("direct-terminal-attempt-binding");
    let artifact_root = fixture.root.join("evidence");
    let plan = super::parse_direct_command_plan("/usr/bin/true").expect("direct plan");
    super::execute_direct_plan(
        &fixture.repo,
        &plan,
        &artifact_root,
        None,
        Duration::from_secs(5),
    )
    .expect("initial successful attempt");
    let record_path = artifact_root.join("command-000.json");
    let mut record: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&record_path).expect("terminal record"))
            .expect("terminal record JSON");
    record["attempt_id"] = serde_json::Value::String(
        super::new_direct_attempt_id_candidate().expect("foreign attempt id"),
    );
    fs::write(&record_path, record.to_string()).expect("replace terminal attempt id");

    let error = super::execute_direct_plan(
        &fixture.repo,
        &plan,
        &artifact_root,
        None,
        Duration::from_secs(5),
    )
    .expect_err("foreign terminal attempt must not authorize recovery");

    assert!(error.contains("resolved invocation intent"), "{error}");
}

#[test]
fn autonomous_executor_bridge_attempt_id_collision_uses_durable_reservation() {
    // Break caught: a restored clock/PID/process sequence reproducing a historical attempt ID
    // and making an old completed retirement look authoritative for a later execution.
    let fixture = GitFixture::new("direct-attempt-id-reservation");
    let artifact_root = fixture.root.join("evidence");
    fs::create_dir_all(&artifact_root).expect("artifact root");
    fs::set_permissions(&artifact_root, fs::Permissions::from_mode(0o700))
        .expect("private artifact root");
    let paths = super::direct_attempt_paths(&artifact_root, 0);
    let collision = autospec_core::autonomous::waterfall::sha256_hex(b"restored-generator");
    let fallback = autospec_core::autonomous::waterfall::sha256_hex(b"fresh-generator");
    let historical =
        super::reserve_direct_attempt_id_candidates(&paths, std::iter::once(collision.clone()))
            .expect("reserve historical attempt");
    fs::write(&paths.launch, b"historical ownership\n").expect("historical launch");
    fs::set_permissions(&paths.launch, fs::Permissions::from_mode(0o600))
        .expect("private historical launch");
    super::retire_direct_launch(&paths, &historical).expect("complete historical retirement");

    let current = super::reserve_direct_attempt_id_candidates(
        &paths,
        [collision.clone(), fallback.clone()],
    )
    .expect("retry deterministic collision");

    assert_eq!(historical, collision);
    assert_eq!(current, fallback);
    assert_ne!(current, historical);
    let reservations = super::direct_attempt_reservation_directory(&paths);
    assert!(reservations.join(&historical).is_file());
    assert!(reservations.join(&current).is_file());
}

#[test]
fn autonomous_executor_bridge_runtime_adapter_binds_attempt_to_exact_session() {
    let _environment = TEST_ENVIRONMENT.lock().expect("test environment lock");
    let fixture = GitFixture::new("runtime-qa-prefix");
    fs::create_dir_all(fixture.repo.join(".autospec")).expect("runtime manifest directory");
    fs::write(
        fixture.repo.join("runtime-up.py"),
        "import http.server, os\npid=os.fork()\nif not pid:\n http.server.HTTPServer(('127.0.0.1', int(os.environ['AGENT_FRONTEND_PORT'])), http.server.SimpleHTTPRequestHandler).serve_forever()\nelse:\n open('runtime.pid','w').write(str(pid))\n",
    )
    .expect("runtime up executable");
    fs::write(
        fixture.repo.join("runtime-down.py"),
        "import os, signal\npid=int(open('runtime.pid').read())\nos.kill(pid, signal.SIGTERM)\nos.remove('runtime.pid')\n",
    )
    .expect("runtime down executable");
    fs::write(
        fixture.repo.join(".autospec/runtime.yml"),
        "version: 1\ndefault_mode: local\nmodes:\n  local:\n    command: python3 runtime-up.py\n    down: python3 runtime-down.py\n",
    )
    .expect("runtime manifest");
    let state_root = fixture.root.join("runtime-state");
    let previous = std::env::var_os("AGENT_ENV_STATE_ROOT");
    std::env::set_var("AGENT_ENV_STATE_ROOT", &state_root);
    let runtime = super::DirectRuntimeAdapter::prepare(&fixture.repo).expect("runtime adapter");
    let session_id = runtime.session_id().to_string();
    let plan = super::parse_direct_command_plan("/usr/bin/printf ok").expect("direct plan");

    let observed = super::execute_direct_plan(
        &fixture.repo,
        &plan,
        &fixture.root.join("evidence"),
        Some(&runtime),
        Duration::from_secs(5),
    )
    .expect("runtime execution");

    assert_eq!(
        observed[0].runtime_session_id.as_deref(),
        Some(session_id.as_str())
    );
    assert_eq!(
        observed[0].process_executable,
        PathBuf::from("/usr/bin/printf").canonicalize().unwrap()
    );
    assert_eq!(
        observed[0].process_argv,
        vec!["/usr/bin/printf".to_string(), "ok".to_string()]
    );
    assert_eq!(session_record_ids(&state_root), vec![session_id]);
    runtime.close_verified().expect("verified close");
    assert!(session_record_ids(&state_root).is_empty());
    match previous {
        Some(value) => std::env::set_var("AGENT_ENV_STATE_ROOT", value),
        None => std::env::remove_var("AGENT_ENV_STATE_ROOT"),
    }
}

#[test]
fn autonomous_executor_bridge_direct_proxy_argv_zero_helper() {
    if std::env::var_os("AUTOSPEC_TEST_DIRECT_PROXY_ARGV_ZERO").is_none() {
        return;
    }
    assert_eq!(
        std::env::args_os()
            .next()
            .as_deref()
            .and_then(|arg0| Path::new(arg0).file_name()),
        Some(std::ffi::OsStr::new("cargo-proxy"))
    );
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_direct_proxy_preserves_argv_zero() {
    let _environment = TEST_ENVIRONMENT.lock().expect("test environment lock");
    let fixture = GitFixture::new("direct-proxy-argv-zero");
    let proxy = fixture.root.join("cargo-proxy");
    let executable = std::env::current_exe()
        .expect("current test executable")
        .canonicalize()
        .expect("canonical test executable");
    std::os::unix::fs::symlink(&executable, &proxy).expect("test executable proxy");
    let plan = super::parse_direct_command_plan(&format!(
        "{} commands::autonomous::executor_bridge::tests::autonomous_executor_bridge_direct_proxy_argv_zero_helper --exact --nocapture",
        proxy.display()
    ))
    .expect("proxy command plan");
    let previous = std::env::var_os("AUTOSPEC_TEST_DIRECT_PROXY_ARGV_ZERO");
    std::env::set_var("AUTOSPEC_TEST_DIRECT_PROXY_ARGV_ZERO", "1");

    let result = super::execute_direct_plan(
        &fixture.repo,
        &plan,
        &fixture.root.join("proxy-evidence"),
        None,
        Duration::from_secs(5),
    );

    match previous {
        Some(value) => std::env::set_var("AUTOSPEC_TEST_DIRECT_PROXY_ARGV_ZERO", value),
        None => std::env::remove_var("AUTOSPEC_TEST_DIRECT_PROXY_ARGV_ZERO"),
    }
    let observed = result.expect("validated proxy must preserve argv zero");
    assert_eq!(observed[0].executable, executable);
    assert_eq!(observed[0].process_executable, executable);
    assert_eq!(observed[0].argv[0], proxy.display().to_string());
    assert_eq!(observed[0].process_argv[0], proxy.display().to_string());
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_direct_proxy_change_retries_terminal_failure() {
    let _environment = TEST_ENVIRONMENT.lock().expect("test environment lock");
    let fixture = GitFixture::new("direct-proxy-retry");
    let artifact_root = fixture.root.join("proxy-evidence");
    let executable = std::env::current_exe()
        .expect("current test executable")
        .canonicalize()
        .expect("canonical test executable");
    let proxy = fixture.root.join("cargo-proxy");
    std::os::unix::fs::symlink(&executable, &proxy).expect("test executable proxy");
    let arguments = "commands::autonomous::executor_bridge::tests::autonomous_executor_bridge_direct_proxy_argv_zero_helper --exact --nocapture";
    let canonical_plan =
        super::parse_direct_command_plan(&format!("{} {arguments}", executable.display()))
            .expect("canonical command plan");
    let proxy_plan =
        super::parse_direct_command_plan(&format!("{} {arguments}", proxy.display()))
            .expect("proxy command plan");
    let previous = std::env::var_os("AUTOSPEC_TEST_DIRECT_PROXY_ARGV_ZERO");
    std::env::set_var("AUTOSPEC_TEST_DIRECT_PROXY_ARGV_ZERO", "1");

    let first = super::execute_direct_plan(
        &fixture.repo,
        &canonical_plan,
        &artifact_root,
        None,
        Duration::from_secs(5),
    );
    assert!(
        first.is_err(),
        "canonical argv zero must reproduce the proxy failure"
    );
    let result = super::execute_direct_plan(
        &fixture.repo,
        &proxy_plan,
        &artifact_root,
        None,
        Duration::from_secs(5),
    );

    match previous {
        Some(value) => std::env::set_var("AUTOSPEC_TEST_DIRECT_PROXY_ARGV_ZERO", value),
        None => std::env::remove_var("AUTOSPEC_TEST_DIRECT_PROXY_ARGV_ZERO"),
    }
    let observed = result.expect("proxy correction must archive and retry prior failure");
    assert_eq!(observed[0].terminal, super::AttemptTerminal::Exited(0));
    assert_eq!(observed[0].argv[0], proxy.display().to_string());
    assert!(fs::read_dir(&artifact_root)
        .expect("proxy failure archive")
        .flatten()
        .any(|entry| entry.file_name().to_string_lossy().contains(".archive-")));
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_direct_proxy_change_retries_terminal_success() {
    let fixture = GitFixture::new("direct-proxy-success-retry");
    let artifact_root = fixture.root.join("proxy-evidence");
    let executable = PathBuf::from("/usr/bin/true")
        .canonicalize()
        .expect("canonical true executable");
    let proxy = fixture.root.join("true-proxy");
    std::os::unix::fs::symlink(&executable, &proxy).expect("true executable proxy");
    let canonical_plan = super::parse_direct_command_plan(&executable.display().to_string())
        .expect("canonical command plan");
    let proxy_plan = super::parse_direct_command_plan(&proxy.display().to_string())
        .expect("proxy command plan");

    let first = super::execute_direct_plan(
        &fixture.repo,
        &canonical_plan,
        &artifact_root,
        None,
        Duration::from_secs(5),
    )
    .expect("canonical command success");
    assert_eq!(first[0].terminal, super::AttemptTerminal::Exited(0));
    let observed = super::execute_direct_plan(
        &fixture.repo,
        &proxy_plan,
        &artifact_root,
        None,
        Duration::from_secs(5),
    )
    .expect("proxy correction must archive and rerun prior success");

    assert_eq!(observed[0].terminal, super::AttemptTerminal::Exited(0));
    assert_eq!(observed[0].argv[0], proxy.display().to_string());
    assert!(fs::read_dir(&artifact_root)
        .expect("proxy success archive")
        .flatten()
        .any(|entry| entry.file_name().to_string_lossy().contains(".archive-")));
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_cargo_proxy_dispatches_rustup() {
    let fixture = GitFixture::new("cargo-rustup-proxy");
    let Ok(rustup) = super::resolve_direct_executable(&fixture.repo, "rustup") else {
        return;
    };
    let proxy = fixture.root.join("cargo");
    std::os::unix::fs::symlink(&rustup.program, &proxy).expect("Cargo rustup proxy");
    let plan = super::parse_direct_command_plan(&format!("{} --version", proxy.display()))
        .expect("Cargo proxy command plan");

    let observed = super::execute_direct_plan(
        &fixture.repo,
        &plan,
        &fixture.root.join("cargo-evidence"),
        None,
        Duration::from_secs(5),
    )
    .expect("Cargo proxy dispatch through rustup");

    assert_eq!(observed[0].terminal, super::AttemptTerminal::Exited(0));
    assert_eq!(observed[0].executable, rustup.program);
    assert_eq!(observed[0].argv[0], proxy.display().to_string());
    assert!(fs::read_to_string(&observed[0].stdout_path)
        .expect("Cargo version output")
        .starts_with("cargo "));
}

#[test]
fn autonomous_executor_bridge_codex_sandbox_fallback_children_have_no_sensitive_credentials() {
    let _environment = TEST_ENVIRONMENT.lock().expect("test environment lock");
    let fixture = GitFixture::new("credentialless-direct-child");
    let keys = [
        "GH_TOKEN",
        "GH_ENTERPRISE_TOKEN",
        "GITHUB_TOKEN",
        "GITHUB_ENTERPRISE_TOKEN",
        "SSH_AUTH_SOCK",
        "GIT_ASKPASS",
        "AWS_ACCESS_KEY_ID",
        "AWS_SECRET_ACCESS_KEY",
        "NPM_TOKEN",
        "DATABASE_URL",
        "INTERNAL_API_KEY",
        "DOCKER_AUTH_CONFIG",
        "KUBECONFIG",
        "VAULT_TOKEN",
        "OPENAI_API_KEY",
    ];
    let previous = keys
        .iter()
        .map(|key| ((*key).to_string(), std::env::var_os(key)))
        .collect::<Vec<_>>();
    for key in keys {
        std::env::set_var(key, format!("forbidden-{key}"));
    }
    let plan = super::parse_direct_command_plan("/usr/bin/env").expect("environment command");
    let observed = super::execute_direct_plan(
        &fixture.repo,
        &plan,
        &fixture.root.join("credential-evidence"),
        None,
        Duration::from_secs(5),
    )
    .expect("credentialless child");
    let environment =
        fs::read_to_string(&observed[0].stdout_path).expect("captured environment");

    for key in keys {
        assert!(
            !environment
                .lines()
                .any(|line| line.starts_with(&format!("{key}="))),
            "{key} leaked to direct child"
        );
    }
    for expected in [
        "GIT_CONFIG_NOSYSTEM=1",
        "GIT_CONFIG_GLOBAL=/dev/null",
        "GIT_TERMINAL_PROMPT=0",
    ] {
        assert!(
            environment.lines().any(|line| line == expected),
            "{expected}"
        );
    }
    assert!(environment
        .lines()
        .any(|line| line.starts_with("GH_CONFIG_DIR=")
            && line.ends_with("/credentialless-config")));
    assert!(environment
        .lines()
        .any(|line| line.starts_with("GIT_SSH_COMMAND=/usr/bin/ssh -F /dev/null")));
    for (key, value) in previous {
        match value {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_codex_sandbox_preserves_host_auth_for_codex_only() {
    let _environment = TEST_ENVIRONMENT.lock().expect("test environment lock");
    let fixture = GitFixture::new("codex-host-auth");
    let fake_codex = fixture.root.join("codex");
    write_executable(
        &fake_codex,
        "#!/bin/sh\nprintf 'codex=%s openai=%s\\n' \"${CODEX_API_KEY:-missing}\" \"${OPENAI_API_KEY:-missing}\"\n",
    );
    let mut state = supervision_state(&fixture);
    state.identity.branch = git_stdout(
        &fixture.repo,
        &["symbolic-ref", "--quiet", "--short", "HEAD"],
    );
    let snapshot =
        MutationSnapshot::capture(&fixture.repo, &state.identity.branch).expect("snapshot");
    let state_path = fixture.root.join("state/codex-host-auth.json");
    let event_log = fixture.root.join("log/codex-host-auth.jsonl");
    let artifact = fixture.repo.join(".autospec/executor-closeout.md");
    let harness = super::ResolvedHarness {
        kind: HarnessKind::Codex,
        executable: fake_codex.canonicalize().expect("canonical fake Codex"),
        opencode_adapter: None,
        codex_sandbox: super::CodexSandboxPolicy::NetworkPermissionProfile,
    };
    let previous_codex = std::env::var_os("CODEX_API_KEY");
    let previous_openai = std::env::var_os("OPENAI_API_KEY");
    let previous_claim = std::env::var_os("AUTOSPEC_TEST_EXACT_EVIDENCE_CLAIM");
    std::env::set_var("CODEX_API_KEY", "codex-host-auth-value");
    std::env::set_var("OPENAI_API_KEY", "openai-host-auth-value");
    std::env::set_var("AUTOSPEC_TEST_EXACT_EVIDENCE_CLAIM", "1");

    let outcome = super::supervise_resolved_harness(
        &state_path,
        &event_log,
        &mut state,
        super::HarnessLaunch {
            resolved: &harness,
            artifact: &artifact,
            prompt: "prove host auth",
        },
        &snapshot,
        supervision_config(5_000),
    )
    .expect("supervise fake Codex");

    match previous_codex {
        Some(value) => std::env::set_var("CODEX_API_KEY", value),
        None => std::env::remove_var("CODEX_API_KEY"),
    }
    match previous_openai {
        Some(value) => std::env::set_var("OPENAI_API_KEY", value),
        None => std::env::remove_var("OPENAI_API_KEY"),
    }
    match previous_claim {
        Some(value) => std::env::set_var("AUTOSPEC_TEST_EXACT_EVIDENCE_CLAIM", value),
        None => std::env::remove_var("AUTOSPEC_TEST_EXACT_EVIDENCE_CLAIM"),
    }
    assert_eq!(outcome, SupervisionOutcome::Exited { exit_code: 0 });
    let events = fs::read_to_string(&event_log).expect("Codex host output");
    assert!(
        events.contains("codex=codex-host-auth-value openai=openai-host-auth-value"),
        "Codex host auth was stripped: {events}"
    );
}

#[test]
fn autonomous_executor_bridge_resolves_full_suite_in_authoritative_order() {
    let fixture = GitFixture::new("full-suite-order");
    fs::write(
        fixture.repo.join("Cargo.toml"),
        "[package]\nname='fixture'\nversion='0.1.0'\n",
    )
    .expect("write cargo manifest");
    let issue = "### Operator/full verification\n\n```bash\n/usr/bin/printf issue\n```\n";
    let spec = "### Operator full\n\n```\n/usr/bin/printf spec\n```\n";
    let override_env = BTreeMap::from([(
        "AUTOSPEC_FULL_TEST_COMMAND".to_string(),
        OsString::from("/usr/bin/printf override"),
    )]);

    let overridden = super::resolve_full_suite(&fixture.repo, issue, &[spec], &override_env)
        .expect("environment override");
    assert_eq!(overridden.source, super::FullSuiteSource::Environment);
    assert_eq!(
        overridden.plan.commands[0].argv,
        vec!["/usr/bin/printf", "override"]
    );

    let declared = super::resolve_full_suite(&fixture.repo, issue, &[spec], &BTreeMap::new())
        .expect("declared full verification");
    assert_eq!(declared.source, super::FullSuiteSource::Declared);
    assert_eq!(declared.plan.commands.len(), 2);

    let fallback = super::resolve_full_suite(&fixture.repo, "", &[], &BTreeMap::new())
        .expect("ecosystem fallback");
    assert_eq!(fallback.source, super::FullSuiteSource::Ecosystem);
    assert_eq!(
        fallback
            .plan
            .commands
            .iter()
            .map(|command| command.argv.join(" "))
            .collect::<Vec<_>>(),
        vec![
            "cargo fmt --check",
            "cargo clippy --all-targets -- -D warnings",
            "cargo test --all-targets",
            "cargo build --all-targets",
        ]
    );
}

#[test]
fn autonomous_executor_bridge_full_suite_rejects_unknown_or_incomplete_repositories() {
    let fixture = GitFixture::new("full-suite-unknown");
    let error = super::resolve_full_suite(&fixture.repo, "", &[], &BTreeMap::new())
        .expect_err("unknown repository must not have a fabricated suite");
    assert!(error.contains("complete full suite"), "{error}");

    fs::write(
        fixture.repo.join("package.json"),
        r#"{"scripts":{"test":"vitest"}}"#,
    )
    .expect("write incomplete package manifest");
    let error = super::resolve_full_suite(&fixture.repo, "", &[], &BTreeMap::new())
        .expect_err("package suite missing lint/typecheck/build must fail closed");
    assert!(error.contains("incomplete"), "{error}");
}

#[test]
fn autonomous_executor_bridge_every_required_scanner_fails_closed_when_missing() {
    for missing in ["gitleaks", "semgrep", "trivy", "license-checker"] {
        let mut paths = BTreeMap::new();
        for scanner in ["gitleaks", "semgrep", "trivy", "license-checker"] {
            if scanner != missing {
                paths.insert(scanner.to_string(), PathBuf::from("/usr/bin/true"));
            }
        }
        let error = super::ScannerExecutables::from_paths(paths)
            .expect_err("missing required scanner must fail closed");
        assert!(error.contains(missing), "{missing}: {error}");
    }
}

#[test]
fn autonomous_executor_bridge_every_degraded_or_failing_scanner_blocks() {
    let fixture = GitFixture::new("scanner-status");
    for scanner in ["gitleaks", "semgrep", "trivy", "license-checker"] {
        let degraded =
            super::validate_scanner_result(scanner, 0, b"", b"scanner degraded fallback")
                .expect_err("degraded scanner output must fail closed");
        assert!(degraded.contains(scanner), "{degraded}");

        let failed = super::validate_scanner_result(scanner, 1, b"{}", b"")
            .expect_err("failing scanner must fail closed");
        assert!(failed.contains(scanner), "{failed}");

        let generic = super::validate_scanner_result(scanner, 0, b"{}", b"")
            .expect_err("generic JSON must not impersonate scanner-native evidence");
        assert!(generic.contains(scanner), "{scanner}: {generic}");
    }
    drop(fixture);
}

#[test]
fn autonomous_executor_bridge_scanner_results_require_native_clean_schemas() {
    let clean = [
        ("gitleaks", br#"[]"#.as_slice()),
        (
            "semgrep",
            br#"{"results":[],"errors":[],"paths":{"scanned":["feature.js"],"skipped":[]},"version":"1.0"}"#
                .as_slice(),
        ),
        (
            "trivy",
            br#"{"SchemaVersion":2,"Results":[{"Target":".","Vulnerabilities":[],"Misconfigurations":[],"Secrets":[]}]}"#
                .as_slice(),
        ),
        (
            "license-checker",
            br#"{"fixture@1.0.0":{"licenses":"MIT","repository":"https://example.invalid"}}"#
                .as_slice(),
        ),
    ];
    for (scanner, output) in clean {
        super::validate_scanner_result(scanner, 0, output, b"")
            .unwrap_or_else(|error| panic!("{scanner} native clean output rejected: {error}"));
    }

    let findings = [
        ("gitleaks", br#"[{"RuleID":"secret"}]"#.as_slice()),
        (
            "semgrep",
            br#"{"results":[{"check_id":"rule"}],"errors":[],"paths":{"scanned":["feature.js"],"skipped":[]}}"#
                .as_slice(),
        ),
        (
            "trivy",
            br#"{"Results":[{"Target":".","Vulnerabilities":[{"VulnerabilityID":"CVE-1"}]}]}"#
                .as_slice(),
        ),
        (
            "license-checker",
            br#"{"fixture@1.0.0":{"licenses":"GPL-3.0"}}"#.as_slice(),
        ),
    ];
    for (scanner, output) in findings {
        let error = super::validate_scanner_result(scanner, 0, output, b"")
            .expect_err("native finding must block");
        assert!(error.contains(scanner), "{scanner}: {error}");
        assert!(error.contains("reported"), "{scanner}: {error}");
    }
    let semgrep_finding = br#"{"results":[{"check_id":"rule"}],"errors":[],"paths":{"scanned":["feature.js"],"skipped":[]}}"#;
    let error = super::validate_scanner_result("semgrep", 1, semgrep_finding, b"")
        .expect_err("Semgrep finding exit must reach native JSON validation");
    assert!(error.contains("reported findings"), "{error}");
    let error = super::validate_scanner_result(
        "semgrep",
        1,
        br#"{"results":[],"errors":[],"paths":{"scanned":["feature.js"],"skipped":[]}}"#,
        b"",
    )
    .expect_err("empty Semgrep JSON must not legitimize a non-zero exit");
    assert!(error.contains("exit status 1"), "{error}");
    let error = super::validate_scanner_result(
        "semgrep",
        0,
        br#"{"results":[],"errors":[],"paths":{"scanned":[],"skipped":[{"path":"large.rs","reason":"exceeded_size_limit"}]}}"#,
        b"",
    )
    .expect_err("a successful Semgrep process must not hide skipped changed files");
    assert!(
        error.contains("scanned no files") || error.contains("skipped files"),
        "{error}"
    );

    let wrong_shape = [
        ("gitleaks", br#"{"results":[],"errors":[]}"#.as_slice()),
        ("semgrep", br#"[]"#.as_slice()),
        ("trivy", br#"{"results":[],"errors":[]}"#.as_slice()),
        (
            "license-checker",
            br#"{"results":[],"errors":[]}"#.as_slice(),
        ),
    ];
    for (scanner, output) in wrong_shape {
        let error = super::validate_scanner_result(scanner, 0, output, b"")
            .expect_err("another tool's JSON shape must not be accepted");
        assert!(error.contains(scanner), "{scanner}: {error}");
    }
}

#[test]
fn autonomous_executor_bridge_changed_paths_preserve_git_status_classes() {
    let changed = super::parse_changed_paths(
        b"A\0added\0D\0deleted\0M\0modified\0R100\0old\0new\0C100\0source\0copy\0T\0typed\0",
    )
    .expect("parse Git name-status output");

    for path in [
        "added", "deleted", "modified", "old", "new", "copy", "typed",
    ] {
        assert!(changed.all.contains(path), "missing changed path {path}");
    }
    for path in ["added", "new", "copy"] {
        assert!(changed.added.contains(path), "missing added path {path}");
    }
    for path in ["deleted", "old"] {
        assert!(
            changed.deleted.contains(path),
            "missing deleted path {path}"
        );
    }
    assert!(changed.type_changed.contains("typed"));
    assert!(!changed.all.contains("source"));
}

#[test]
fn autonomous_executor_bridge_changed_paths_reject_malformed_name_status() {
    for (output, expected) in [
        (b"X\0path\0".as_slice(), "status is unsupported"),
        (b"R100\0old\0".as_slice(), "output is truncated"),
        (b"\0path\0".as_slice(), "empty status"),
        (b"\xff\0path\0".as_slice(), "status is not valid UTF-8"),
        (b"M\0\xff\0".as_slice(), "path is not valid UTF-8"),
    ] {
        let error = super::parse_changed_paths(output)
            .expect_err("malformed Git name-status output must fail");
        assert!(error.contains(expected), "{error}");
    }
}

#[test]
fn autonomous_executor_bridge_npm_dependency_inputs_follow_manifest_policy() {
    // Break caught: treating scripts as graph input, or treating any other top-level
    // manifest field as irrelevant, weakens the conservative npm classification policy.
    let fixture = GitFixture::new("npm-dependency-inputs-manifest-policy");
    fs::write(
        fixture.repo.join("package.json"),
        r#"{"scripts":{"test":"old"}}"#,
    )
    .expect("baseline manifest");
    git(&fixture.repo, &["add", "package.json"]);
    git(&fixture.repo, &["commit", "-m", "baseline manifest"]);
    let base_oid = git_stdout(&fixture.repo, &["rev-parse", "HEAD"]);

    for (field, current, expected) in [
        ("scripts", r#"{"scripts":{"test":"new"}}"#, false),
        (
            "dependencies",
            r#"{"scripts":{"test":"old"},"dependencies":{"fixture":"1"}}"#,
            true,
        ),
        (
            "devDependencies",
            r#"{"scripts":{"test":"old"},"devDependencies":{"fixture":"1"}}"#,
            true,
        ),
        (
            "optionalDependencies",
            r#"{"scripts":{"test":"old"},"optionalDependencies":{"fixture":"1"}}"#,
            true,
        ),
        (
            "peerDependencies",
            r#"{"scripts":{"test":"old"},"peerDependencies":{"fixture":"1"}}"#,
            true,
        ),
        (
            "peerDependenciesMeta",
            r#"{"scripts":{"test":"old"},"peerDependenciesMeta":{"fixture":{"optional":true}}}"#,
            true,
        ),
        (
            "overrides",
            r#"{"scripts":{"test":"old"},"overrides":{"fixture":"2"}}"#,
            true,
        ),
        (
            "version",
            r#"{"scripts":{"test":"old"},"version":"2.0.0"}"#,
            true,
        ),
        (
            "unknown",
            r#"{"scripts":{"test":"old"},"autospecUnknown":true}"#,
            true,
        ),
    ] {
        fs::write(fixture.repo.join("package.json"), current).expect("current manifest");
        git(&fixture.repo, &["add", "package.json"]);
        git(&fixture.repo, &["commit", "-m", field]);
        let changed = super::changed_paths_since_base(&fixture.repo, &base_oid)
            .expect("changed manifest paths");

        assert_eq!(
            super::npm_dependency_inputs_changed(&fixture.repo, &base_oid, &changed)
                .expect("manifest classification"),
            expected,
            "{field}"
        );
    }
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_npm_dependency_inputs_preserve_git_status_classes() {
    // Break caught: dropping a lockfile identity on modify, delete, rename, copy, or
    // type change can misclassify a runtime dependency graph change as irrelevant.
    for lockfile in [
        "package-lock.json",
        "npm-shrinkwrap.json",
        "pnpm-lock.yaml",
        "yarn.lock",
    ] {
        let fixture = GitFixture::new(&format!("npm-dependency-inputs-{lockfile}"));
        fs::write(fixture.repo.join(lockfile), "baseline\n").expect("baseline lockfile");
        git(&fixture.repo, &["add", lockfile]);
        git(&fixture.repo, &["commit", "-m", "baseline lockfile"]);
        let base_oid = git_stdout(&fixture.repo, &["rev-parse", "HEAD"]);
        fs::write(fixture.repo.join(lockfile), "changed\n").expect("changed lockfile");
        git(&fixture.repo, &["add", lockfile]);
        git(&fixture.repo, &["commit", "-m", "change lockfile"]);
        let changed = super::changed_paths_since_base(&fixture.repo, &base_oid)
            .expect("changed lockfile paths");

        assert!(
            super::npm_dependency_inputs_changed(&fixture.repo, &base_oid, &changed)
                .expect("lockfile classification"),
            "{lockfile}"
        );
    }

    let deleted = GitFixture::new("npm-dependency-inputs-deleted");
    fs::write(deleted.repo.join("package.json"), r#"{"name":"fixture"}"#)
        .expect("baseline manifest");
    git(&deleted.repo, &["add", "package.json"]);
    git(&deleted.repo, &["commit", "-m", "baseline manifest"]);
    let base_oid = git_stdout(&deleted.repo, &["rev-parse", "HEAD"]);
    git(&deleted.repo, &["rm", "package.json"]);
    git(&deleted.repo, &["commit", "-m", "delete manifest"]);
    let changed = super::changed_paths_since_base(&deleted.repo, &base_oid)
        .expect("deleted manifest paths");
    assert!(
        super::npm_dependency_inputs_changed(&deleted.repo, &base_oid, &changed)
            .expect("deleted manifest classification")
    );

    let renamed = GitFixture::new("npm-dependency-inputs-renamed");
    fs::write(renamed.repo.join("package-lock.json"), "{}\n").expect("baseline lockfile");
    git(&renamed.repo, &["add", "package-lock.json"]);
    git(&renamed.repo, &["commit", "-m", "baseline lockfile"]);
    let base_oid = git_stdout(&renamed.repo, &["rev-parse", "HEAD"]);
    git(
        &renamed.repo,
        &["mv", "package-lock.json", "archived-lock.json"],
    );
    git(&renamed.repo, &["commit", "-m", "rename lockfile"]);
    let changed = super::changed_paths_since_base(&renamed.repo, &base_oid)
        .expect("renamed lockfile paths");
    assert!(
        super::npm_dependency_inputs_changed(&renamed.repo, &base_oid, &changed)
            .expect("renamed lockfile classification")
    );

    let copied = GitFixture::new("npm-dependency-inputs-copied");
    fs::write(copied.repo.join("source-lock.json"), "{}\n").expect("baseline source");
    git(&copied.repo, &["add", "source-lock.json"]);
    git(&copied.repo, &["commit", "-m", "baseline source"]);
    let base_oid = git_stdout(&copied.repo, &["rev-parse", "HEAD"]);
    fs::copy(
        copied.repo.join("source-lock.json"),
        copied.repo.join("package-lock.json"),
    )
    .expect("copy lockfile");
    git(&copied.repo, &["add", "package-lock.json"]);
    git(&copied.repo, &["commit", "-m", "copy lockfile"]);
    let changed = super::changed_paths_since_base(&copied.repo, &base_oid)
        .expect("copied lockfile paths");
    assert!(
        super::npm_dependency_inputs_changed(&copied.repo, &base_oid, &changed)
            .expect("copied lockfile classification")
    );

    let typed = GitFixture::new("npm-dependency-inputs-type-changed");
    fs::write(typed.repo.join("package-lock.json"), "{}\n").expect("baseline lockfile");
    git(&typed.repo, &["add", "package-lock.json"]);
    git(&typed.repo, &["commit", "-m", "baseline lockfile"]);
    let base_oid = git_stdout(&typed.repo, &["rev-parse", "HEAD"]);
    fs::remove_file(typed.repo.join("package-lock.json")).expect("remove lockfile");
    symlink("README.md", typed.repo.join("package-lock.json")).expect("symlink lockfile");
    git(&typed.repo, &["add", "package-lock.json"]);
    git(&typed.repo, &["commit", "-m", "type change lockfile"]);
    let changed = super::changed_paths_since_base(&typed.repo, &base_oid)
        .expect("type-changed lockfile paths");
    assert!(changed.type_changed.contains("package-lock.json"));
    assert!(
        super::npm_dependency_inputs_changed(&typed.repo, &base_oid, &changed)
            .expect("type-changed lockfile classification")
    );
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_npm_dependency_inputs_fail_closed_on_bad_evidence() {
    // Break caught: malformed, unsafe, unreadable, or unattributable package.json
    // evidence silently becoming an absent manifest and weakening classification.
    let fixture = GitFixture::new("npm-dependency-inputs-bad-evidence");
    fs::write(fixture.repo.join("package.json"), r#"{"name":"fixture"}"#)
        .expect("baseline manifest");
    git(&fixture.repo, &["add", "package.json"]);
    git(&fixture.repo, &["commit", "-m", "baseline manifest"]);
    let base_oid = git_stdout(&fixture.repo, &["rev-parse", "HEAD"]);

    fs::write(fixture.repo.join("package.json"), b"{").expect("malformed manifest");
    git(&fixture.repo, &["add", "package.json"]);
    git(&fixture.repo, &["commit", "-m", "malformed manifest"]);
    let changed = super::changed_paths_since_base(&fixture.repo, &base_oid)
        .expect("malformed manifest paths");
    let error = super::npm_dependency_inputs_changed(&fixture.repo, &base_oid, &changed)
        .expect_err("malformed JSON must fail closed");
    assert!(error.contains("parse current package.json:"), "{error}");

    fs::write(fixture.repo.join("package.json"), "[]\n").expect("non-object manifest");
    git(&fixture.repo, &["add", "package.json"]);
    git(&fixture.repo, &["commit", "-m", "non-object manifest"]);
    let changed = super::changed_paths_since_base(&fixture.repo, &base_oid)
        .expect("non-object manifest paths");
    assert_eq!(
        super::npm_dependency_inputs_changed(&fixture.repo, &base_oid, &changed)
            .expect_err("non-object JSON must fail closed"),
        "current package.json is not a JSON object"
    );

    fs::write(fixture.repo.join("package.json"), r#"{"name":"changed"}"#)
        .expect("changed manifest");
    git(&fixture.repo, &["add", "package.json"]);
    git(&fixture.repo, &["commit", "-m", "changed manifest"]);
    let changed = super::changed_paths_since_base(&fixture.repo, &base_oid)
        .expect("changed manifest paths");
    let manifest = fixture.repo.join("package.json");
    let mut permissions = fs::metadata(&manifest)
        .expect("manifest metadata")
        .permissions();
    permissions.set_mode(0o000);
    fs::set_permissions(&manifest, permissions).expect("make manifest unreadable");
    let result = super::npm_dependency_inputs_changed(&fixture.repo, &base_oid, &changed);
    let mut permissions = fs::metadata(&manifest)
        .expect("manifest metadata")
        .permissions();
    permissions.set_mode(0o600);
    fs::set_permissions(&manifest, permissions).expect("restore manifest permissions");
    let error = result.expect_err("unreadable current manifest must fail closed");
    assert!(error.contains("read current package.json:"), "{error}");

    let error = super::npm_dependency_inputs_changed(&fixture.repo, "missing-base", &changed)
        .expect_err("missing base evidence must fail closed");
    assert!(error.contains("read base package.json:"), "{error}");

    fs::remove_file(&manifest).expect("remove regular manifest");
    symlink("README.md", &manifest).expect("unsafe manifest symlink");
    git(&fixture.repo, &["add", "package.json"]);
    git(&fixture.repo, &["commit", "-m", "symlink manifest"]);
    let changed = super::changed_paths_since_base(&fixture.repo, &base_oid)
        .expect("symlink manifest paths");
    let error = super::npm_dependency_inputs_changed(&fixture.repo, &base_oid, &changed)
        .expect_err("unsafe manifest symlink must fail closed");
    assert!(error.contains("current package.json is unsafe:"), "{error}");
    assert!(error.contains("path contains a symlink"), "{error}");
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_npm_dependency_inputs_reject_manifest_swap_before_open() {
    // Break caught: validating package.json by path and reopening it lets a symlink swap
    // redirect the classifier to attacker-controlled dependency data.
    let fixture = GitFixture::new("npm-dependency-inputs-open-swap");
    let manifest = fixture.repo.join("package.json");
    fs::write(
        &manifest,
        r#"{"dependencies":{"fixture":"1"},"scripts":{"test":"old"}}"#,
    )
    .expect("baseline manifest");
    git(&fixture.repo, &["add", "package.json"]);
    git(&fixture.repo, &["commit", "-m", "baseline manifest"]);
    let base_oid = git_stdout(&fixture.repo, &["rev-parse", "HEAD"]);
    fs::write(
        &manifest,
        r#"{"dependencies":{"fixture":"1"},"scripts":{"test":"new"}}"#,
    )
    .expect("scripts-only manifest");
    git(&fixture.repo, &["add", "package.json"]);
    git(&fixture.repo, &["commit", "-m", "scripts-only change"]);
    let changed = super::changed_paths_since_base(&fixture.repo, &base_oid)
        .expect("changed manifest paths");
    super::NPM_MANIFEST_OPEN_FAILPOINT.store(1, Ordering::SeqCst);
    let repo = fixture.repo.clone();
    let classifier =
        thread::spawn(move || super::npm_dependency_inputs_changed(&repo, &base_oid, &changed));
    let deadline = Instant::now() + Duration::from_secs(5);
    while super::NPM_MANIFEST_OPEN_FAILPOINT.load(Ordering::SeqCst) != 2 {
        assert!(
            Instant::now() < deadline,
            "classifier did not reach open boundary"
        );
        thread::yield_now();
    }
    fs::rename(&manifest, fixture.repo.join("package.original.json"))
        .expect("move validated manifest");
    let attacker = fixture.root.join("attacker-package.json");
    fs::write(&attacker, r#"{"dependencies":{"fixture":"2"}}"#).expect("attacker manifest");
    symlink(&attacker, &manifest).expect("replace manifest with symlink");
    super::NPM_MANIFEST_OPEN_FAILPOINT.store(3, Ordering::SeqCst);

    let error = classifier
        .join()
        .expect("classifier thread")
        .expect_err("manifest swap must fail closed");
    assert!(error.contains("current package.json"), "{error}");
    super::NPM_MANIFEST_OPEN_FAILPOINT.store(0, Ordering::SeqCst);
}

#[test]
fn autonomous_executor_bridge_restart_reruns_all_scanner_results() {
    let _environment = TEST_ENVIRONMENT.lock().expect("test environment lock");
    // Break caught: successful scanner records recovered from disk becoming authoritative
    // security evidence without re-executing each scanner in the current process.
    let fixture = GitFixture::new("scanner-recovery");
    let bin = fixture.root.join("scanner-bin");
    fs::create_dir_all(&bin).expect("scanner bin");
    let mut paths = BTreeMap::new();
    for (scanner, output) in [
        ("gitleaks", "[]"),
        (
            "semgrep",
            r#"{"results":[],"errors":[],"paths":{"scanned":["feature.js"],"skipped":[]}}"#,
        ),
        ("trivy", r#"{"Results":[{"Target":"."}]}"#),
        ("license-checker", r#"{"fixture@1.0.0":{"licenses":"MIT"}}"#),
    ] {
        let executable = bin.join(scanner);
        let count = bin.join(format!("{scanner}.count"));
        let result_logic = if scanner == "gitleaks" {
            format!(
                "report=''\nwhile [ \"$#\" -gt 0 ]; do if [ \"$1\" = --report-path ]; then report=\"$2\"; shift 2; else shift; fi; done\nprintf '%s' '{}' > \"$report\"\n",
                output
            )
        } else {
            format!("printf '%s' '{}'\n", output)
        };
        write_executable(
            &executable,
            &format!(
                "#!/bin/sh\nset -eu\nn=0\n[ ! -f '{}' ] || n=$(cat '{}')\nn=$((n+1))\nprintf '%s' \"$n\" > '{}'\n{}",
                count.display(),
                count.display(),
                count.display(),
                result_logic
            ),
        );
        paths.insert(scanner.to_string(), executable);
    }
    let scanners = super::ScannerExecutables::from_paths(paths).expect("scanner paths");
    let artifact_root = fixture.root.join("scanner-evidence");

    let first = super::run_required_scanners(
        &fixture.repo,
        &git_stdout(&fixture.repo, &["rev-parse", "HEAD"]),
        &artifact_root,
        &scanners,
        None,
        Duration::from_secs(5),
    )
    .expect("first scanner pass");
    let second = super::run_required_scanners(
        &fixture.repo,
        &git_stdout(&fixture.repo, &["rev-parse", "HEAD"]),
        &artifact_root,
        &scanners,
        None,
        Duration::from_secs(5),
    )
    .expect("adopt scanner pass");

    assert_eq!(first.len(), second.len());
    for scanner in ["gitleaks", "semgrep", "trivy", "license-checker"] {
        assert_eq!(
            fs::read_to_string(bin.join(format!("{scanner}.count"))).expect("scanner count"),
            "2",
            "{scanner} did not rerun after its durable terminal record"
        );
    }
}

#[test]
fn autonomous_executor_bridge_policy_digest_prevents_command_replay() {
    // Break caught: an identical scanner argv recovering a terminal record created under
    // different generated-policy content.
    let fixture = GitFixture::new("scanner-policy-command-identity");
    let artifact_root = fixture.root.join("command-evidence");
    let command = |digest: &str| {
        let mut command = super::DirectCommand::success(vec!["/usr/bin/true".to_string()]);
        command.identity_digest = Some(digest.to_string());
        super::DirectCommandPlan {
            commands: vec![command],
        }
    };

    super::execute_direct_plan(
        &fixture.repo,
        &command(&"a".repeat(64)),
        &artifact_root,
        None,
        Duration::from_secs(5),
    )
    .expect("first policy-bound command");
    let error = super::execute_direct_plan(
        &fixture.repo,
        &command(&"b".repeat(64)),
        &artifact_root,
        None,
        Duration::from_secs(5),
    )
    .expect_err("different policy digest must not replay the old terminal record");

    assert!(error.contains("invocation intent"), "{error}");
}

#[test]
fn autonomous_executor_bridge_scanner_argv_is_direct_and_fail_closed() {
    let worktree = Path::new("/safe/worktree");
    let config = Path::new("/safe/evidence/gitleaks-policy.toml");
    let report = Path::new("/safe/evidence/gitleaks.json");
    let expected = [
        (
            "gitleaks",
            vec![
                "/scanner/gitleaks",
                "detect",
                "--no-git",
                "--no-banner",
                "--redact",
                "--source",
                "/safe/worktree",
                "--config",
                "/safe/evidence/gitleaks-policy.toml",
                "--report-format",
                "json",
                "--report-path",
                "/safe/evidence/gitleaks.json",
            ],
        ),
        (
            "semgrep",
            vec![
                "/scanner/semgrep",
                "scan",
                "--config",
                "p/default",
                "--metrics",
                "off",
                "--error",
                "--json",
                "--verbose",
                "--max-target-bytes",
                "0",
                "--timeout",
                "0",
                "--timeout-threshold",
                "0",
                "--baseline-commit",
                "base-oid",
                "/safe/worktree",
            ],
        ),
        (
            "trivy",
            vec![
                "/scanner/trivy",
                "fs",
                "--quiet",
                "--format",
                "json",
                "--exit-code",
                "1",
                "/safe/worktree",
            ],
        ),
        (
            "license-checker",
            vec![
                "/scanner/license-checker",
                "--json",
                "--production",
                "--start",
                "/safe/worktree",
            ],
        ),
    ];
    for (scanner, argv) in expected {
        assert_eq!(
            super::scanner_command(
                scanner,
                Path::new(argv[0]),
                worktree,
                "base-oid",
                config,
                report,
            )
            .expect("scanner command")
            .argv,
            argv
        );
    }
}

#[test]
fn autonomous_executor_bridge_scanner_command_semgrep_is_private_and_baseline_scoped() {
    // Break caught: `--config auto --metrics off` is rejected by Semgrep before scanning,
    // while an unscoped repository scan also blocks feature work on pre-existing findings.
    let command = super::scanner_command(
        "semgrep",
        Path::new("/scanner/semgrep"),
        Path::new("/safe/worktree"),
        "base-oid",
        Path::new("/safe/evidence/gitleaks-policy.toml"),
        Path::new("/safe/evidence/gitleaks.json"),
    )
    .expect("Semgrep command");

    assert!(
        command
            .argv
            .windows(2)
            .any(|pair| pair == ["--config", "p/default"]),
        "{:?}",
        command.argv
    );
    assert!(
        command
            .argv
            .iter()
            .any(|argument| argument == "--baseline-commit"),
        "{:?}",
        command.argv
    );
    assert!(
        command
            .argv
            .windows(2)
            .any(|pair| pair == ["--metrics", "off"]),
        "{:?}",
        command.argv
    );
    assert!(
        command
            .argv
            .windows(2)
            .any(|pair| pair == ["--max-target-bytes", "0"]),
        "{:?}",
        command.argv
    );
    assert_eq!(command.accepted_exit_codes, vec![0, 1]);
}

#[test]
fn autonomous_executor_bridge_scanner_command_semgrep_baseline_is_diff_scoped() {
    let _environment = TEST_ENVIRONMENT.lock().expect("test environment lock");
    // Break caught: a repository-wide scan blocking a feature on findings already present
    // in its claimed base commit, instead of evaluating only feature-introduced findings.
    let fixture = GitFixture::new("semgrep-baseline");
    let rule = fixture.root.join("semgrep-rule.yml");
    fs::write(
        &rule,
        r#"rules:
  - id: autospec-test-dangerous-call
languages:
  - generic
message: deterministic test finding
severity: ERROR
pattern-regex: dangerous_call
"#,
    )
    .expect("deterministic Semgrep rule");
    fs::write(fixture.repo.join("old.js"), "dangerous_call('old');\n")
        .expect("pre-existing finding");
    git(&fixture.repo, &["add", "old.js"]);
    git(&fixture.repo, &["commit", "-m", "baseline finding"]);
    let base_oid = git_stdout(&fixture.repo, &["rev-parse", "HEAD"]);
    fs::write(
        fixture.repo.join("feature.js"),
        "safe_call('feature');\n".repeat(60_000),
    )
    .expect("clean feature larger than Semgrep's default 1 MB limit");
    git(&fixture.repo, &["add", "feature.js"]);
    git(&fixture.repo, &["commit", "-m", "clean feature"]);
    let semgrep = super::resolve_direct_executable(&fixture.repo, "semgrep")
        .expect("real Semgrep")
        .program;
    let scan = |artifact: &str| {
        let mut command = super::DirectCommand::success(vec![
            semgrep.display().to_string(),
            "scan".to_string(),
            "--config".to_string(),
            rule.display().to_string(),
            "--metrics".to_string(),
            "off".to_string(),
            "--error".to_string(),
            "--json".to_string(),
            "--verbose".to_string(),
            "--max-target-bytes".to_string(),
            "0".to_string(),
            "--timeout".to_string(),
            "0".to_string(),
            "--timeout-threshold".to_string(),
            "0".to_string(),
            "--baseline-commit".to_string(),
            base_oid.clone(),
            ".".to_string(),
        ]);
        command.accepted_exit_codes = vec![0, 1];
        let observed = super::execute_direct_plan(
            &fixture.repo,
            &super::DirectCommandPlan {
                commands: vec![command],
            },
            &fixture.root.join(artifact),
            None,
            Duration::from_secs(30),
        )
        .expect("Semgrep process observation");
        let command = &observed[0];
        let stdout = fs::read(&command.stdout_path).expect("Semgrep JSON");
        let stderr = fs::read(&command.stderr_path).expect("Semgrep diagnostics");
        (
            command.exit_code().expect("Semgrep exit status"),
            stdout,
            stderr,
        )
    };

    let (exit_status, stdout, stderr) = scan("clean-scan");
    super::validate_scanner_result("semgrep", exit_status, &stdout, &stderr).unwrap_or_else(
        |error| {
            panic!(
                "pre-existing finding is outside the feature diff: {error}; {}",
                String::from_utf8_lossy(&stderr)
            )
        },
    );

    fs::write(
        fixture.repo.join("feature.js"),
        "dangerous_call('feature');\n",
    )
    .expect("new finding");
    git(&fixture.repo, &["add", "feature.js"]);
    git(
        &fixture.repo,
        &["commit", "-m", "introduce feature finding"],
    );
    let (exit_status, stdout, stderr) = scan("finding-scan");
    let error = super::validate_scanner_result("semgrep", exit_status, &stdout, &stderr)
        .expect_err("feature-introduced finding must block");
    assert!(error.contains("reported findings"), "{error}");
}

#[test]
fn autonomous_executor_bridge_command_artifact_digest_and_commit_tamper_block() {
    let fixture = GitFixture::new("command-evidence-tamper");
    let artifacts = fixture.root.join("evidence");
    let plan =
        super::parse_direct_command_plan("/usr/bin/printf exact").expect("direct command plan");
    let records = super::execute_direct_plan(
        &fixture.repo,
        &plan,
        &artifacts,
        None,
        Duration::from_secs(5),
    )
    .expect("observed command");
    super::validate_observed_command(&fixture.repo, &records[0])
        .expect("untampered observation");

    fs::write(&records[0].stdout_path, "tampered").expect("tamper command output");
    let error = super::validate_observed_command(&fixture.repo, &records[0])
        .expect_err("artifact digest tamper must fail");
    assert!(error.contains("digest"), "{error}");

    git(&fixture.repo, &["commit", "--allow-empty", "-m", "drift"]);
    let error = super::validate_observed_command(&fixture.repo, &records[0])
        .expect_err("commit drift must fail");
    assert!(error.contains("commit"), "{error}");
}

#[test]
fn autonomous_executor_bridge_observed_results_are_the_only_typed_pass_authority() {
    let fixture = GitFixture::new("typed-evidence");
    let commit = git_stdout(&fixture.repo, &["rev-parse", "HEAD"]);
    let lane = super::PremergeLaneIdentity::new(
        "test/repo",
        42,
        "worker-42",
        "claim-42",
        "main",
        commit.clone(),
    )
    .expect("typed lane");
    let mut qa = Vec::new();
    let mut scanners = Vec::new();
    for (index, scanner) in ["gitleaks", "semgrep", "trivy", "license-checker"]
        .into_iter()
        .enumerate()
    {
        let output = match scanner {
            "gitleaks" => "[]",
            "semgrep" => {
                r#"{"results":[],"errors":[],"paths":{"scanned":["feature.js"],"skipped":[]}}"#
            }
            "trivy" => r#"{"Results":[{"Target":"."}]}"#,
            "license-checker" => r#"{"fixture@1.0.0":{"licenses":"MIT"}}"#,
            _ => unreachable!(),
        };
        let plan = super::parse_direct_command_plan(&format!("/usr/bin/printf '{output}'"))
            .expect("native scanner JSON observation command");
        let records = super::execute_direct_plan(
            &fixture.repo,
            &plan,
            &fixture.root.join(format!("observed-{index}")),
            None,
            Duration::from_secs(5),
        )
        .expect("real observed process");
        if index == 0 {
            qa.push(records[0].clone());
        }
        scanners.push(super::ObservedScanner {
            name: scanner.to_string(),
            base_oid: git_stdout(&fixture.repo, &["rev-parse", "HEAD"]),
            command: records[0].clone(),
            result_path: records[0].stdout_path.clone(),
            result_digest: records[0].stdout_digest.clone(),
        });
    }

    let complete = super::typed_evidence_from_observed(
        &fixture.repo,
        &commit,
        &lane,
        Ok(&qa),
        Ok(&scanners),
        Some("PASS"),
        1_800_000_000,
    );
    assert!(matches!(complete.0.verdict, super::EvidenceVerdict::Pass));
    assert!(matches!(complete.1.verdict, super::EvidenceVerdict::Pass));

    let missing = super::typed_evidence_from_observed(
        &fixture.repo,
        &commit,
        &lane,
        Ok(&qa),
        Ok(&scanners[..3]),
        Some("PASS"),
        1_800_000_001,
    );
    assert!(
        !matches!(missing.1.verdict, super::EvidenceVerdict::Pass),
        "fabricated model Pass must not upgrade missing scanner evidence"
    );

    let failed = super::typed_evidence_from_observed(
        &fixture.repo,
        &commit,
        &lane,
        Err("full suite failed"),
        Ok(&scanners),
        Some("PASS"),
        1_800_000_002,
    );
    assert!(
        !matches!(failed.0.verdict, super::EvidenceVerdict::Pass),
        "fabricated model Pass must not upgrade failed QA evidence"
    );
    let lint_failed = super::typed_evidence_from_observed(
        &fixture.repo,
        &commit,
        &lane,
        Ok(&qa),
        Err("implementation lint failed"),
        Some("PASS"),
        1_800_000_003,
    );
    assert!(
        !matches!(lint_failed.1.verdict, super::EvidenceVerdict::Pass),
        "implementation-lint failure must block security Pass"
    );
}

#[test]
fn autonomous_executor_bridge_remote_snapshot_ignores_claim_ledger_refs() {
    let document = format!(
        "{}\trefs/heads/main\n{}\trefs/autospec/claims/issue-42\n",
        "a".repeat(40),
        "b".repeat(40)
    );

    assert_eq!(
        super::parse_bridge_remote_refs(&document).expect("claim ledger is separate authority"),
        BTreeMap::from([("refs/heads/main".to_string(), "a".repeat(40))])
    );
    assert!(
        super::parse_bridge_remote_refs(&format!(
            "{}\trefs/autospec/unowned/issue-42\n",
            "c".repeat(40)
        ))
        .is_err(),
        "only the exact claim-ledger namespace may be excluded"
    );
}

#[test]
fn autonomous_executor_bridge_requires_observed_exact_merged_state() {
    let document = r#"{
        "number":17,
        "state":"MERGED",
        "isDraft":false,
        "headRefOid":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "baseRefName":"main",
        "mergeCommit":{"oid":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}
    }"#;
    assert_eq!(
        super::parse_observed_merge(document, 17, &"a".repeat(40), "main",).expect("merged"),
        "b".repeat(40)
    );
    assert!(super::parse_observed_merge(
        &document.replace("\"MERGED\"", "\"OPEN\""),
        17,
        &"a".repeat(40),
        "main",
    )
    .is_err());
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_reconciles_merged_existing_worktree_before_stale_proof() {
    let _environment = TEST_ENVIRONMENT.lock().expect("test environment lock");
    let (fixture, mut state, _snapshot, closeout) =
        implementation_proof_fixture("merged-existing-worktree-entrypoint");
    commit_implementation(&state);
    let persisted_head = git_stdout(&state.identity.worktree, &["rev-parse", "HEAD"]);
    state.phase = super::BridgePhase::DraftCreated;
    state.pr = Some(17);
    state.head_oid = Some(persisted_head.clone());
    state.closeout_path = Some(fs::canonicalize(&closeout).expect("canonical closeout"));
    let closeout_body = fs::read_to_string(&closeout).expect("read closeout");
    state.closeout_digest = Some(super::sha256_hex(closeout_body.as_bytes()));
    state.identity.runtime_environment_dir = None;
    state.identity.runtime_session_id = None;
    state.supervisor = None;
    state.process = None;
    state.draft_process = None;
    super::record_worktree_creation_identity(
        &state.identity.repository_path,
        &state.identity.branch,
        &ResolvedBase {
            base_ref: state.identity.base_ref.clone(),
            base_oid: state.identity.base_oid.clone(),
            explore_mode: false,
        },
    )
    .expect("record worktree creation identity");
    let claimed = autospec_core::claim::RunStateRecord::new(
        state.identity.repository.clone(),
        state.identity.issue,
        state.identity.worker_id.clone(),
        "claimed",
        state.identity.branch.clone(),
        "",
        "claimed",
        Vec::new(),
        "2026-08-04T00:00:00Z",
        "2026-08-04T00:00:00Z",
        999_999,
    )
    .with_claim_id(state.identity.claim_id.clone());
    assert!(crate::commands::claim::advance_claim_ref_for_test(
        &state.identity.repository_path,
        &claimed,
    )
    .expect("seed claimed generation"));

    fs::write(
        state.identity.worktree.join("reviewer-follow-up.txt"),
        "reviewer follow-up\n",
    )
    .expect("reviewer follow-up");
    git(&state.identity.worktree, &["add", "reviewer-follow-up.txt"]);
    git(
        &state.identity.worktree,
        &["commit", "-m", "test: reviewer follow-up"],
    );
    let merged_head = git_stdout(&state.identity.worktree, &["rev-parse", "HEAD"]);

    let state_path = fixture.root.join("state/invocation.json");
    super::write_invocation_atomic(&state_path, &state).expect("persist stale draft state");
    let observation = fixture.root.join("merged-observation.json");
    fs::write(
        &observation,
        serde_json::json!({
            "number": 17,
            "state": "MERGED",
            "isDraft": false,
            "headRefName": state.identity.branch,
            "headRefOid": merged_head,
            "baseRefName": "main",
            "mergeCommit": {"oid": "b".repeat(40)},
            "body": super::canonical_pull_request_body(&state, &closeout_body).unwrap(),
        })
        .to_string(),
    )
    .expect("merged observation");
    let gh = fixture.root.join("gh");
    fs::write(
        &gh,
        format!(
            "#!/bin/sh\nset -eu\n\
             if [ \"$1 $2\" = 'pr view' ]; then cat '{}'; exit 0; fi\n\
             if [ \"$1 $2\" = 'issue view' ]; then printf '%s\\n' '{{\"labels\":[]}}'; exit 0; fi\n\
             if [ \"$1 $2\" = 'issue edit' ] || [ \"$1 $2\" = 'issue comment' ]; then exit 0; fi\n\
             if [ \"$1\" = 'api' ]; then printf '%s\\n' '[]'; exit 0; fi\n\
             exit 64\n",
            observation.display()
        ),
    )
    .expect("gh fixture");
    fs::set_permissions(&gh, fs::Permissions::from_mode(0o755)).expect("gh mode");
    let failpoint = fixture.root.join("merged-reconciliation-failpoint");
    let previous_path = std::env::var_os("PATH");
    let previous_claim = std::env::var_os("AUTOSPEC_TEST_EXACT_EVIDENCE_CLAIM");
    let previous_failpoint = std::env::var_os("AUTOSPEC_TEST_MERGED_RECONCILIATION_FAIL_ONCE");
    let previous_claim_remote = std::env::var_os("AUTOSPEC_CLAIM_GIT_REMOTE");
    let previous_claim_state = std::env::var_os("AUTOSPEC_CLAIM_GIT_STATE_DIR");
    let previous_retry_sleep = std::env::var_os("AUTOSPEC_CLAIM_RETRY_SLEEP_MS");
    std::env::set_var(
        "PATH",
        format!(
            "{}:{}",
            fixture.root.display(),
            previous_path
                .as_deref()
                .unwrap_or_default()
                .to_string_lossy()
        ),
    );
    std::env::set_var("AUTOSPEC_TEST_EXACT_EVIDENCE_CLAIM", "1");
    std::env::set_var("AUTOSPEC_TEST_MERGED_RECONCILIATION_FAIL_ONCE", &failpoint);
    std::env::set_var("AUTOSPEC_CLAIM_GIT_REMOTE", fixture.root.join("remote.git"));
    std::env::set_var(
        "AUTOSPEC_CLAIM_GIT_STATE_DIR",
        fixture.root.join("claim-state"),
    );
    std::env::set_var("AUTOSPEC_CLAIM_RETRY_SLEEP_MS", "0");
    let request = super::ExecutorBridgeRequest {
        repository: state.identity.repository.clone(),
        repository_path: state.identity.repository_path.clone(),
        issue: state.identity.issue,
        issue_title: "Retire merged executor".to_string(),
        issue_body: DRAFT_ISSUE_BODY.to_string(),
        worker_id: state.identity.worker_id.clone(),
        claim_id: state.identity.claim_id.clone(),
        invocation_id: state.identity.invocation_id.clone(),
        state_path: state_path.clone(),
        event_log: fixture.root.join("events.jsonl"),
    };

    let outcome = super::run_executor_bridge_with_codex_probe(&request, |_| {
        panic!("merged recovery must precede Codex probing")
    });
    let error = outcome.expect_err("failpoint stops after merged reconciliation");
    assert!(
        error.to_string().contains("injected executor crash"),
        "{error}"
    );
    let durable = super::PersistedInvocation::from_json(
        &fs::read_to_string(&state_path).expect("read reconciled invocation"),
    )
    .expect("parse reconciled invocation");
    assert_eq!(durable.phase, super::BridgePhase::Merged);
    assert_eq!(durable.head_oid.as_deref(), Some(merged_head.as_str()));
    assert_eq!(
        durable.terminal_result.as_deref(),
        Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
    );
    assert!(super::cleanup_record_path(&state_path, "merged-reconciliation").exists());
    assert!(
        state.identity.worktree.exists(),
        "failpoint must precede cleanup"
    );
    let receipt = super::run_executor_bridge_with_codex_probe(&request, |_| {
        panic!("merged restart must finalize before Codex probing")
    })
    .expect("restart finalizes the reconciled merge");
    let complete = super::PersistedInvocation::from_json(
        &fs::read_to_string(&state_path).expect("read completed invocation"),
    )
    .expect("parse completed invocation");

    for (key, previous) in [
        ("PATH", previous_path),
        ("AUTOSPEC_TEST_EXACT_EVIDENCE_CLAIM", previous_claim),
        (
            "AUTOSPEC_TEST_MERGED_RECONCILIATION_FAIL_ONCE",
            previous_failpoint,
        ),
        ("AUTOSPEC_CLAIM_GIT_REMOTE", previous_claim_remote),
        ("AUTOSPEC_CLAIM_GIT_STATE_DIR", previous_claim_state),
        ("AUTOSPEC_CLAIM_RETRY_SLEEP_MS", previous_retry_sleep),
    ] {
        match previous {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }

    assert!(matches!(
        receipt.status,
        super::BridgeRunStatus::Merged {
            pull_request: 17,
            ref head_oid,
            ref merge_oid,
        } if head_oid == &merged_head && merge_oid == &"b".repeat(40)
    ));
    assert_eq!(complete.phase, super::BridgePhase::Complete);
    assert!(!state.identity.worktree.exists());
    assert!(state_path.with_extension("terminal.json").is_file());
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_merged_reconciliation_is_exact_and_fail_closed() {
    let (fixture, mut state, _snapshot, _) =
        implementation_proof_fixture("merged-reconciliation-exact");
    commit_implementation(&state);
    let persisted_head = git_stdout(&state.identity.worktree, &["rev-parse", "HEAD"]);
    fs::write(
        state.identity.worktree.join("reviewer-follow-up.txt"),
        "reviewer follow-up\n",
    )
    .expect("reviewer follow-up");
    git(&state.identity.worktree, &["add", "reviewer-follow-up.txt"]);
    git(
        &state.identity.worktree,
        &["commit", "-m", "reviewer follow-up"],
    );
    let merged_head = git_stdout(&state.identity.worktree, &["rev-parse", "HEAD"]);
    state.phase = super::BridgePhase::DraftCreated;
    state.pr = Some(17);
    state.head_oid = Some(persisted_head.clone());
    state.supervisor = None;
    state.process = None;
    state.draft_process = None;
    state.umbrella = Some(42);
    state.current_child = Some(101);
    let closeout = "## Closeout report\n";
    state.closeout_digest = Some(autospec_core::autonomous::waterfall::sha256_hex(
        closeout.as_bytes(),
    ));
    let observation = fixture.root.join("merged-observation.json");
    let gh = fixture.root.join("gh-merged-reconciliation");
    fs::write(&gh, "#!/bin/sh\nset -eu\ncat \"$MERGED_OBSERVATION\"\n").expect("gh");
    fs::set_permissions(&gh, fs::Permissions::from_mode(0o755)).expect("gh mode");
    let adapter = super::DraftPrAdapter {
        gh,
        environment: BTreeMap::from([(
            "MERGED_OBSERVATION".into(),
            observation.clone().into_os_string(),
        )]),
    };
    let exact = serde_json::json!({
        "number": 17,
        "state": "MERGED",
        "isDraft": false,
        "headRefName": state.identity.branch,
        "headRefOid": merged_head,
        "baseRefName": "main",
        "mergeCommit": {"oid": "b".repeat(40)},
        "body": super::canonical_pull_request_body(&state, closeout).unwrap(),
    });
    for phase in [
        super::BridgePhase::Merged,
        super::BridgePhase::CleanupPending,
        super::BridgePhase::Complete,
    ] {
        state.phase = phase;
        for (body, accepted) in [("Closes #101", true), ("Closes #42", false)] {
            fs::write(&observation, exact.to_string().replace("Closes #101", body)).unwrap();
            assert_eq!(
                super::revalidate_live_canonical_pull_request(&state, &adapter).is_ok(),
                accepted
            );
        }
    }
    state.phase = super::BridgePhase::DraftCreated;
    fs::write(&observation, exact.to_string()).expect("exact observation");
    let state_path = fixture.root.join("state/exact.json");
    super::write_invocation_atomic(&state_path, &state).expect("persist pre-reconciliation");
    let mut reconciled = state.clone();
    assert!(super::reconcile_exact_merged_invocation_with_refresh(
        &state_path,
        &mut reconciled,
        &adapter,
        || Ok(super::BridgeClaimOwnership::Refreshed { ttl_seconds: 60 }),
    )
    .expect("exact merged reconciliation"));
    assert_eq!(reconciled.phase, super::BridgePhase::Merged);
    assert_eq!(reconciled.head_oid.as_deref(), Some(merged_head.as_str()));
    assert_eq!(
        reconciled.terminal_result.as_deref(),
        Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
    );
    super::validate_merged_reconciliation_record(&state_path, &reconciled)
        .expect("bound reconciliation record");
    let mut rebound = reconciled.clone();
    rebound.current_child = Some(102);
    assert!(super::validate_merged_reconciliation_record(&state_path, &rebound).is_err());

    for (name, mutated) in [
        ("open", serde_json::json!({"state": "OPEN"})),
        ("draft", serde_json::json!({"isDraft": true})),
        ("number", serde_json::json!({"number": 18})),
        (
            "branch",
            serde_json::json!({"headRefName": "feat/autonomous-issue-99"}),
        ),
        ("base", serde_json::json!({"baseRefName": "release"})),
        (
            "body",
            serde_json::json!({"body": format!("Closes #42\n\n{closeout}")}),
        ),
        ("head-oid", serde_json::json!({"headRefOid": "not-an-oid"})),
        (
            "merge-oid",
            serde_json::json!({"mergeCommit": {"oid": "not-an-oid"}}),
        ),
        (
            "local-head",
            serde_json::json!({"headRefOid": persisted_head}),
        ),
    ] {
        let mut document = exact.clone();
        let object = document.as_object_mut().expect("observation object");
        for (key, value) in mutated.as_object().expect("mutation object") {
            object.insert(key.clone(), value.clone());
        }
        fs::write(&observation, document.to_string()).expect("mutated observation");
        let mut candidate = state.clone();
        let candidate_path = fixture.root.join(format!("state/{name}.json"));
        let outcome = super::reconcile_exact_merged_invocation_with_refresh(
            &candidate_path,
            &mut candidate,
            &adapter,
            || Ok(super::BridgeClaimOwnership::Refreshed { ttl_seconds: 60 }),
        );
        if name == "open" {
            assert!(!outcome.expect("open PR is not terminal"));
        } else {
            assert!(outcome.is_err(), "{name} must fail closed");
        }
        assert_eq!(candidate, state, "{name} must not mutate invocation state");
        assert!(
            !candidate_path.exists(),
            "{name} must not publish terminal state"
        );
    }

    let reconciliation_path = super::cleanup_record_path(&state_path, "merged-reconciliation");
    let mut changed_record: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(&reconciliation_path).expect("read reconciliation"),
    )
    .expect("parse reconciliation");
    changed_record["persisted_head"] = serde_json::json!(state.identity.base_oid);
    fs::write(&reconciliation_path, changed_record.to_string())
        .expect("change reconciliation record");
    assert!(
        super::validate_merged_reconciliation_record(&state_path, &reconciled).is_err(),
        "changed persisted evidence must fail closed"
    );

    let base = state.identity.base_oid.clone();
    let tree = git_stdout(
        &state.identity.repository_path,
        &["rev-parse", &format!("{base}^{{tree}}")],
    );
    let divergent = git_stdout(
        &state.identity.repository_path,
        &["commit-tree", &tree, "-p", &base, "-m", "divergent head"],
    );
    git(
        &state.identity.repository_path,
        &[
            "update-ref",
            &format!("refs/heads/{}", state.identity.branch),
            &divergent,
            &merged_head,
        ],
    );
    let mut divergent_observation = exact.clone();
    divergent_observation["headRefOid"] = serde_json::json!(divergent);
    fs::write(&observation, divergent_observation.to_string()).expect("divergent observation");
    let mut nonancestor = state.clone();
    let nonancestor_path = fixture.root.join("state/nonancestor.json");
    let error = super::reconcile_exact_merged_invocation_with_refresh(
        &nonancestor_path,
        &mut nonancestor,
        &adapter,
        || Ok(super::BridgeClaimOwnership::Refreshed { ttl_seconds: 60 }),
    )
    .expect_err("non-ancestor merged head must fail closed");
    assert!(error.to_string().contains("not contained"), "{error}");
    assert_eq!(nonancestor, state);
    assert!(!nonancestor_path.exists());

    fs::write(&observation, exact.to_string()).expect("restore exact observation");
    let mut lost = state.clone();
    let lost_path = fixture.root.join("state/ownership-lost.json");
    let error = super::reconcile_exact_merged_invocation_with_refresh(
        &lost_path,
        &mut lost,
        &adapter,
        || Ok(super::BridgeClaimOwnership::Lost),
    )
    .expect_err("claim takeover must block reconciliation");
    assert!(error.to_string().contains("ownership"), "{error}");
    assert_eq!(lost, state);
    assert!(!lost_path.exists());
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_merged_reconciliation_waits_for_exact_live_process() {
    let (_fixture, mut state, _snapshot, _) =
        implementation_proof_fixture("merged-reconciliation-live-process");
    let args = vec!["30".to_string()];
    let executable = fs::canonicalize("/usr/bin/sleep").expect("sleep executable");
    let mut child = Command::new(&executable)
        .arg("30")
        .process_group(0)
        .spawn()
        .expect("spawn live executor fixture");
    let mut cleanup =
        DetachedForkedCleanup::new(child.id()).expect("arm live executor cleanup");
    let deadline = Instant::now() + Duration::from_secs(2);
    let identity = loop {
        if let Some(identity) =
            super::observe_process_identity(child.id(), &super::argv_digest(&args))
                .expect("observe live executor")
        {
            if identity.executable == executable
                && identity.argv_digest == super::argv_digest(&args)
                && identity.process_group == identity.pid
            {
                break identity;
            }
        }
        assert!(Instant::now() < deadline, "live executor was not observed");
        std::thread::sleep(Duration::from_millis(1));
    };
    cleanup.confirm_identity(identity.clone());
    state.supervisor = Some(identity);

    assert!(
        !super::executor_terminal_processes_are_quiescent(&state)
            .expect("inspect exact live process"),
        "remote terminal truth must not retire a generation while its exact process is live"
    );
    assert!(
        child
            .try_wait()
            .expect("inspect live executor fixture")
            .is_none(),
        "the quiescence gate must not mutate the live process"
    );
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_claim_takeover_blocks_admin_merge() {
    let (fixture, mut state, _snapshot, _) = implementation_proof_fixture("merge-takeover");
    commit_implementation(&state);
    let head = git_stdout(&state.identity.worktree, &["rev-parse", "HEAD"]);
    git(
        &state.identity.worktree,
        &[
            "push",
            "origin",
            &format!("{head}:refs/heads/{}", state.identity.branch),
        ],
    );
    state.phase = super::BridgePhase::ResultAccepted;
    state.pr = Some(17);
    state.head_oid = Some(head.clone());
    let closeout = "## Closeout report\n";
    state.closeout_digest = Some(autospec_core::autonomous::waterfall::sha256_hex(
        closeout.as_bytes(),
    ));
    let body =
        serde_json::to_string(&super::canonical_pull_request_body(&state, closeout).unwrap())
            .unwrap();
    let state_path = fixture.root.join("state/invocation.json");
    super::write_invocation_atomic(&state_path, &state).expect("accepted state");
    let gh = fixture.root.join("gh-merge");
    let calls = fixture.root.join("merge-calls");
    fs::write(
        &gh,
        format!("#!/bin/sh\nset -eu\nprintf '%s\\n' \"$*\" >> \"$GH_CALLS\"\n\
         if [ \"$1 $2\" = 'pr view' ]; then\n\
         printf '%s\\n' '{{\"number\":17,\"state\":\"OPEN\",\"isDraft\":false,\"headRefOid\":\"{head}\",\"baseRefName\":\"main\",\"mergeCommit\":null,\"body\":{body}}}'\n\
         exit 0\nfi\nexit 64\n"),
    )
    .expect("gh");
    fs::set_permissions(&gh, fs::Permissions::from_mode(0o755)).expect("gh mode");
    let adapter = super::DraftPrAdapter {
        gh,
        environment: BTreeMap::from([("GH_CALLS".into(), calls.clone().into_os_string())]),
    };

    let error = super::admin_squash_merge_exact_with_refresh_and_admission(
        &state_path,
        &mut state,
        &adapter,
        || Ok(super::BridgeClaimOwnership::Lost),
        || Ok(()),
    )
    .expect_err("takeover blocks merge");
    assert!(error.contains("ownership"), "{error}");
    assert!(!fs::read_to_string(calls)
        .expect("calls")
        .contains("pr merge"));
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_merge_failure_resumes_from_accepted_evidence() {
    let (fixture, mut state, _snapshot, _) = implementation_proof_fixture("merge-retry");
    commit_implementation(&state);
    let head = git_stdout(&state.identity.worktree, &["rev-parse", "HEAD"]);
    git(
        &state.identity.worktree,
        &[
            "push",
            "origin",
            &format!("{head}:refs/heads/{}", state.identity.branch),
        ],
    );
    state.phase = super::BridgePhase::ResultAccepted;
    state.pr = Some(17);
    state.head_oid = Some(head.clone());
    let closeout = "## Closeout report\n";
    state.closeout_digest = Some(autospec_core::autonomous::waterfall::sha256_hex(
        closeout.as_bytes(),
    ));
    let body =
        serde_json::to_string(&super::canonical_pull_request_body(&state, closeout).unwrap())
            .unwrap();
    let state_path = fixture.root.join("state/invocation.json");
    super::write_invocation_atomic(&state_path, &state).expect("accepted state");
    let gh = fixture.root.join("gh-merge-retry");
    let fail_once = fixture.root.join("fail-once");
    let merged = fixture.root.join("merged");
    fs::write(&fail_once, "").expect("fail marker");
    fs::write(
        &gh,
        format!(
            "#!/bin/sh\nset -eu\n\
             if [ \"$1 $2\" = 'pr view' ]; then\n\
               if [ -e \"$MERGED\" ]; then printf '%s\\n' '{{\"number\":17,\"state\":\"MERGED\",\"isDraft\":false,\"headRefOid\":\"{head}\",\"baseRefName\":\"main\",\"mergeCommit\":{{\"oid\":\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\"}},\"body\":{body}}}';\n\
               else printf '%s\\n' '{{\"number\":17,\"state\":\"OPEN\",\"isDraft\":false,\"headRefOid\":\"{head}\",\"baseRefName\":\"main\",\"mergeCommit\":null,\"body\":{body}}}'; fi\n\
               exit 0\n\
             fi\n\
             if [ \"$1 $2\" = 'pr merge' ]; then\n\
               case \" $* \" in *\" --match-head-commit {head} \"*) ;; *) exit 74 ;; esac\n\
               if [ -e \"$FAIL_ONCE\" ]; then rm \"$FAIL_ONCE\"; exit 73; fi\n\
               touch \"$MERGED\"; exit 0\n\
             fi\n\
             exit 64\n"
        ),
    )
    .expect("gh");
    fs::set_permissions(&gh, fs::Permissions::from_mode(0o755)).expect("gh mode");
    let adapter = super::DraftPrAdapter {
        gh,
        environment: BTreeMap::from([
            ("FAIL_ONCE".into(), fail_once.into_os_string()),
            ("MERGED".into(), merged.into_os_string()),
        ]),
    };

    super::admin_squash_merge_exact_with_refresh_and_admission(
        &state_path,
        &mut state,
        &adapter,
        || Ok(super::BridgeClaimOwnership::Refreshed { ttl_seconds: 60 }),
        || Ok(()),
    )
    .expect_err("first merge fails");
    assert_eq!(state.phase, super::BridgePhase::MergeRequested);
    let merge_oid = super::admin_squash_merge_exact_with_refresh_and_admission(
        &state_path,
        &mut state,
        &adapter,
        || Ok(super::BridgeClaimOwnership::Refreshed { ttl_seconds: 60 }),
        || Ok(()),
    )
    .expect("retry observes merge");
    assert_eq!(merge_oid, "b".repeat(40));
    assert_eq!(state.phase, super::BridgePhase::Merged);
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_recovers_merged_pr_after_branch_deletion_and_base_advance() {
    let (fixture, mut state, _snapshot, _) =
        implementation_proof_fixture("merge-observed-after-delete");
    commit_implementation(&state);
    let head = git_stdout(&state.identity.worktree, &["rev-parse", "HEAD"]);
    git(
        &state.identity.worktree,
        &[
            "push",
            "origin",
            &format!("{head}:refs/heads/{}", state.identity.branch),
        ],
    );
    state.phase = super::BridgePhase::MergeRequested;
    state.pr = Some(17);
    state.head_oid = Some(head.clone());
    let closeout = "## Closeout report\n";
    state.closeout_digest = Some(autospec_core::autonomous::waterfall::sha256_hex(
        closeout.as_bytes(),
    ));
    let body =
        serde_json::to_string(&super::canonical_pull_request_body(&state, closeout).unwrap())
            .unwrap();
    let state_path = fixture.root.join("state/invocation.json");
    super::write_invocation_atomic(&state_path, &state).expect("requested state");
    git(
        &fixture.root.join("seed"),
        &["push", "origin", &format!(":{}", state.identity.branch)],
    );
    fs::write(fixture.root.join("seed/merged.txt"), "merged\n").expect("base advance");
    git(&fixture.root.join("seed"), &["add", "merged.txt"]);
    git(
        &fixture.root.join("seed"),
        &["commit", "-m", "merged result"],
    );
    git(&fixture.root.join("seed"), &["push", "origin", "main"]);
    let gh = fixture.root.join("gh-merged-observation");
    fs::write(
        &gh,
        format!(
            "#!/bin/sh\nset -eu\n\
             if [ \"$1 $2\" = 'pr view' ]; then\n\
               printf '%s\\n' '{{\"number\":17,\"state\":\"MERGED\",\"isDraft\":false,\"headRefOid\":\"{head}\",\"baseRefName\":\"main\",\"mergeCommit\":{{\"oid\":\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\"}},\"body\":{body}}}'\n\
               exit 0\n\
             fi\n\
             exit 64\n"
        ),
    )
    .expect("gh");
    fs::set_permissions(&gh, fs::Permissions::from_mode(0o755)).expect("gh mode");
    let adapter = super::DraftPrAdapter {
        gh,
        environment: BTreeMap::new(),
    };

    assert_eq!(
        super::admin_squash_merge_exact_with_refresh(&state_path, &mut state, &adapter, || Ok(
            super::BridgeClaimOwnership::Refreshed { ttl_seconds: 60 }
        ))
        .expect("merged observation must win over deleted refs"),
        "b".repeat(40)
    );
    assert_eq!(state.phase, super::BridgePhase::Merged);
}

#[test]
fn autonomous_executor_bridge_pr_size_base_drift_recomputes_exact_admission() {
    let (fixture, mut state, _snapshot, _) = implementation_proof_fixture("merge-base-drift");
    commit_implementation(&state);
    let original_head = git_stdout(&state.identity.worktree, &["rev-parse", "HEAD"]);
    git(
        &state.identity.worktree,
        &[
            "push",
            "origin",
            &format!("{original_head}:refs/heads/{}", state.identity.branch),
        ],
    );
    state.phase = super::BridgePhase::ReviewPassed;
    state.pr = Some(17);
    state.head_oid = Some(original_head.clone());
    let state_path = fixture.root.join("state/invocation.json");
    super::write_invocation_atomic(&state_path, &state).expect("reviewed state");
    let original =
        super::evaluate_patch_size_admission(&state, &original_head, DRAFT_ISSUE_BODY)
            .expect("original admission");
    super::persist_patch_size_admission(&state_path, &original).expect("original receipt");

    fs::write(fixture.root.join("seed/base-drift.txt"), "drift\n").expect("base drift");
    git(&fixture.root.join("seed"), &["add", "base-drift.txt"]);
    git(&fixture.root.join("seed"), &["commit", "-m", "base drift"]);
    git(&fixture.root.join("seed"), &["push", "origin", "main"]);

    assert!(
        super::reconcile_base_drift_with_refresh(&state_path, &mut state, || {
            Ok(super::BridgeClaimOwnership::Refreshed { ttl_seconds: 60 })
        })
        .expect("reconcile"),
        "drift must regenerate the lane"
    );
    assert_eq!(state.phase, super::BridgePhase::DraftCreated);
    assert_ne!(state.identity.base_oid, original_head);
    assert_ne!(state.head_oid.as_deref(), Some(original_head.as_str()));
    let admission = super::validate_patch_size_admission(&state_path, &state)
        .expect("drifted base/head have fresh patch-size evidence");
    assert_eq!(admission.base_oid, state.identity.base_oid);
    assert_eq!(Some(admission.head_oid.as_str()), state.head_oid.as_deref());
    assert_ne!(admission.base_oid, original.base_oid);
    assert_ne!(admission.head_oid, original.head_oid);
    assert_ne!(admission.evaluation_digest, original.evaluation_digest);
    let ancestor = Command::new("git")
        .args([
            "merge-base",
            "--is-ancestor",
            &state.identity.base_oid,
            state.head_oid.as_deref().expect("new head"),
        ])
        .current_dir(&state.identity.worktree)
        .status()
        .expect("ancestry");
    assert!(ancestor.success());
}

#[test]
fn autonomous_executor_bridge_reconciles_draft_base_before_premerge_evidence() {
    // Break caught: stale branches running scanners and the full suite before they receive
    // process fixes already merged to their integration base.
    let _environment = TEST_ENVIRONMENT.lock().expect("test environment lock");
    let previous_claim = std::env::var_os("AUTOSPEC_TEST_EXACT_EVIDENCE_CLAIM");
    std::env::set_var("AUTOSPEC_TEST_EXACT_EVIDENCE_CLAIM", "1");
    let (fixture, mut state, _snapshot, _) =
        implementation_proof_fixture("draft-premerge-base-drift");
    commit_implementation(&state);
    let original_head = git_stdout(&state.identity.worktree, &["rev-parse", "HEAD"]);
    git(
        &state.identity.worktree,
        &[
            "push",
            "origin",
            &format!("{original_head}:refs/heads/{}", state.identity.branch),
        ],
    );
    state.phase = super::BridgePhase::DraftCreated;
    state.pr = Some(17);
    state.head_oid = Some(original_head.clone());
    let state_path = fixture.root.join("state/invocation.json");
    super::write_invocation_atomic(&state_path, &state).expect("draft state");
    let admission =
        super::evaluate_patch_size_admission(&state, &original_head, DRAFT_ISSUE_BODY)
            .expect("original admission");
    super::persist_patch_size_admission(&state_path, &admission)
        .expect("original admission receipt");

    fs::write(fixture.root.join("seed/premerge-fix.txt"), "base fix\n").expect("base drift");
    git(&fixture.root.join("seed"), &["add", "premerge-fix.txt"]);
    git(
        &fixture.root.join("seed"),
        &["commit", "-m", "premerge fix"],
    );
    git(&fixture.root.join("seed"), &["push", "origin", "main"]);
    let updated_base = git_stdout(&fixture.root.join("seed"), &["rev-parse", "HEAD"]);
    let request = super::ExecutorBridgeRequest {
        repository: state.identity.repository.clone(),
        repository_path: state.identity.repository_path.clone(),
        issue: state.identity.issue,
        issue_title: "Reconcile stale implementation".to_string(),
        issue_body: DRAFT_ISSUE_BODY.to_string(),
        worker_id: state.identity.worker_id.clone(),
        claim_id: state.identity.claim_id.clone(),
        invocation_id: state.identity.invocation_id.clone(),
        state_path: state_path.clone(),
        event_log: fixture.root.join("events.jsonl"),
    };
    let proof = super::ImplementationProof {
        head_oid: original_head.clone(),
        closeout_body: String::new(),
    };

    let result = super::ensure_premerge_and_review(
        &request,
        &BTreeMap::new(),
        &super::DraftPrAdapter::github_cli(),
        &mut state,
        &proof,
        None,
    );
    match previous_claim {
        Some(value) => std::env::set_var("AUTOSPEC_TEST_EXACT_EVIDENCE_CLAIM", value),
        None => std::env::remove_var("AUTOSPEC_TEST_EXACT_EVIDENCE_CLAIM"),
    }

    assert_eq!(
        result.expect("base drift must precede scanner resolution"),
        None
    );
    assert_eq!(state.phase, super::BridgePhase::DraftCreated);
    assert_eq!(state.identity.base_oid, updated_base);
    assert_ne!(state.head_oid.as_deref(), Some(original_head.as_str()));
    assert!(
        !state.identity.worktree.join(".autospec/evidence").exists(),
        "premerge evidence started before the stale branch was updated"
    );
}

#[test]
fn autonomous_executor_bridge_base_drift_invalidates_accepted_and_requested_results() {
    for phase in [
        super::BridgePhase::ResultAccepted,
        super::BridgePhase::MergeRequested,
    ] {
        let (fixture, mut state, _snapshot, _) =
            implementation_proof_fixture(&format!("late-drift-{phase:?}"));
        commit_implementation(&state);
        let original_head = git_stdout(&state.identity.worktree, &["rev-parse", "HEAD"]);
        git(
            &state.identity.worktree,
            &[
                "push",
                "origin",
                &format!("{original_head}:refs/heads/{}", state.identity.branch),
            ],
        );
        state.phase = phase;
        state.pr = Some(17);
        state.head_oid = Some(original_head);
        state.terminal_result = Some("accepted-result".into());
        let state_path = fixture.root.join("state/invocation.json");
        super::write_invocation_atomic(&state_path, &state).expect("accepted state");
        fs::write(
            fixture.root.join("seed/late-drift.txt"),
            format!("{phase:?}\n"),
        )
        .expect("base drift");
        git(&fixture.root.join("seed"), &["add", "late-drift.txt"]);
        git(&fixture.root.join("seed"), &["commit", "-m", "late drift"]);
        git(&fixture.root.join("seed"), &["push", "origin", "main"]);

        assert!(
            super::reconcile_base_drift_with_refresh(&state_path, &mut state, || Ok(
                super::BridgeClaimOwnership::Refreshed { ttl_seconds: 60 }
            ))
            .expect("late drift must regenerate")
        );
        assert_eq!(state.phase, super::BridgePhase::DraftCreated);
        assert_eq!(state.terminal_result, None);
    }
}

#[test]
fn autonomous_executor_bridge_claim_takeover_blocks_base_drift_push() {
    let (fixture, mut state, _snapshot, _) = implementation_proof_fixture("drift-takeover");
    commit_implementation(&state);
    let original_head = git_stdout(&state.identity.worktree, &["rev-parse", "HEAD"]);
    git(
        &state.identity.worktree,
        &[
            "push",
            "origin",
            &format!("{original_head}:refs/heads/{}", state.identity.branch),
        ],
    );
    state.phase = super::BridgePhase::ReviewPassed;
    state.pr = Some(17);
    state.head_oid = Some(original_head.clone());
    let state_path = fixture.root.join("state/invocation.json");
    super::write_invocation_atomic(&state_path, &state).expect("reviewed state");
    let durable_before = fs::read(&state_path).expect("durable state");
    let local_head_before = git_stdout(
        &state.identity.worktree,
        &["rev-parse", "--verify", "HEAD^{commit}"],
    );
    fs::write(fixture.root.join("seed/takeover.txt"), "drift\n").expect("base drift");
    git(&fixture.root.join("seed"), &["add", "takeover.txt"]);
    git(&fixture.root.join("seed"), &["commit", "-m", "base drift"]);
    git(&fixture.root.join("seed"), &["push", "origin", "main"]);

    let error = super::reconcile_base_drift_with_refresh(&state_path, &mut state, || {
        Ok(super::BridgeClaimOwnership::Lost)
    })
    .expect_err("takeover blocks push");
    assert!(error.contains("ownership"), "{error}");
    let issue_ref = format!("refs/heads/{}", state.identity.branch);
    assert_eq!(
        super::remote_head_refs(&state.identity.repository_path)
            .expect("remote refs")
            .get(&issue_ref),
        Some(&original_head)
    );
    assert_eq!(
        git_stdout(
            &state.identity.worktree,
            &["rev-parse", "--verify", "HEAD^{commit}"]
        ),
        local_head_before
    );
    assert_eq!(fs::read(state_path).expect("durable state"), durable_before);
}

#[test]
fn autonomous_executor_bridge_recovers_crash_after_owned_base_merge() {
    let (fixture, mut state, _snapshot, _) = implementation_proof_fixture("drift-crash");
    commit_implementation(&state);
    let original_head = git_stdout(&state.identity.worktree, &["rev-parse", "HEAD"]);
    git(
        &state.identity.worktree,
        &[
            "push",
            "origin",
            &format!("{original_head}:refs/heads/{}", state.identity.branch),
        ],
    );
    state.phase = super::BridgePhase::ReviewPassed;
    state.pr = Some(17);
    state.head_oid = Some(original_head);
    let state_path = fixture.root.join("state/invocation.json");
    super::write_invocation_atomic(&state_path, &state).expect("reviewed state");
    fs::write(fixture.root.join("seed/crash.txt"), "drift\n").expect("base drift");
    git(&fixture.root.join("seed"), &["add", "crash.txt"]);
    git(&fixture.root.join("seed"), &["commit", "-m", "base drift"]);
    git(&fixture.root.join("seed"), &["push", "origin", "main"]);

    super::BASE_DRIFT_FAILPOINT.store(1, Ordering::SeqCst);
    let error = super::reconcile_base_drift_with_refresh(&state_path, &mut state, || {
        Ok(super::BridgeClaimOwnership::Refreshed { ttl_seconds: 60 })
    })
    .expect_err("injected crash");
    assert!(error.contains("injected crash"), "{error}");
    let durable = super::PersistedInvocation::from_json(
        &fs::read_to_string(&state_path).expect("durable invocation"),
    )
    .expect("durable pre-merge binding remains");
    assert_eq!(durable.phase, super::BridgePhase::ReviewPassed);

    assert!(
        super::reconcile_base_drift_with_refresh(&state_path, &mut state, || {
            Ok(super::BridgeClaimOwnership::Refreshed { ttl_seconds: 60 })
        })
        .expect("adopt exact merge")
    );
    assert_eq!(state.phase, super::BridgePhase::DraftCreated);
}

#[test]
fn autonomous_executor_bridge_cleanup_resumes_after_owned_worktree_removal() {
    let (fixture, mut state, _snapshot, _) = implementation_proof_fixture("cleanup-resume");
    commit_implementation(&state);
    state.phase = super::BridgePhase::CleanupPending;
    state.identity.runtime_environment_dir = None;
    state.identity.runtime_session_id = None;
    let state_path = fixture.root.join("state/invocation.json");
    super::write_invocation_atomic(&state_path, &state).expect("cleanup state");
    let intent = super::cleanup_record_path(&state_path, "worktree-intent");
    super::ensure_cleanup_record(
        &intent,
        &super::cleanup_binding(&state),
        "test removal intent",
    )
    .expect("durable intent");
    git(
        &state.identity.repository_path,
        &[
            "worktree",
            "remove",
            state.identity.worktree.to_str().expect("worktree"),
        ],
    );

    super::finalize_merged_executor(&state_path, &mut state, None).expect("resume cleanup");
    assert_eq!(state.phase, super::BridgePhase::Complete);
    assert!(super::cleanup_record_path(&state_path, "worktree-complete").exists());
}

#[test]
fn autonomous_executor_bridge_runtime_close_recovers_after_receipt_gap() {
    let _environment = TEST_ENVIRONMENT.lock().expect("test environment lock");
    let (fixture, mut state, _snapshot, _) =
        implementation_proof_fixture("runtime-close-recovery");
    let autospec = state.identity.worktree.join(".autospec");
    fs::create_dir_all(&autospec).expect("runtime manifest directory");
    fs::write(
        state.identity.worktree.join("runtime-up.py"),
        "import http.server, os\npid=os.fork()\nif not pid:\n http.server.HTTPServer(('127.0.0.1', int(os.environ['AGENT_FRONTEND_PORT'])), http.server.SimpleHTTPRequestHandler).serve_forever()\nelse:\n open('runtime.pid','w').write(str(pid))\n",
    )
    .expect("runtime up");
    fs::write(
        state.identity.worktree.join("runtime-down.py"),
        "import os, signal\npid=int(open('runtime.pid').read())\nos.kill(pid, signal.SIGTERM)\nos.remove('runtime.pid')\n",
    )
    .expect("runtime down");
    fs::write(
        autospec.join("runtime.yml"),
        "version: 1\ndefault_mode: local\nmodes:\n  local:\n    command: python3 runtime-up.py\n    down: python3 runtime-down.py\n",
    )
    .expect("runtime manifest");
    let state_root = fixture.root.join("runtime-state");
    let previous = std::env::var_os("AGENT_ENV_STATE_ROOT");
    std::env::set_var("AGENT_ENV_STATE_ROOT", &state_root);
    let runtime = super::runtime_session_adapter(&state.identity.worktree)
        .expect("runtime adapter")
        .expect("runtime manifest");
    state.identity.runtime_session_id = Some(runtime.session_id.clone());
    let state_path = fixture.root.join("state/invocation.json");
    super::write_invocation_atomic(&state_path, &state).expect("runtime state");

    super::RUNTIME_CLOSE_FAILPOINT.store(1, Ordering::SeqCst);
    let error = super::finalize_failed_executor(
        &state_path,
        &mut state,
        Some(runtime),
        None,
        true,
        false,
        "implementation failed",
    )
    .expect_err("injected failure-finalization receipt gap");
    assert!(error.to_string().contains("injected crash"), "{error}");
    assert_eq!(
        super::read_failure_cleanup_intent(&state_path, &state)
            .expect("restart reads exact cleanup intent")
            .reason,
        "implementation failed"
    );
    assert!(
        super::cleanup_record_path(&state_path, "runtime-intent.json").exists(),
        "close intent must precede runtime mutation"
    );
    assert!(!super::cleanup_record_path(&state_path, "runtime-complete").exists());

    super::close_owned_runtime(&state_path, &state, None)
        .expect("restart proves absence and writes receipt");
    assert!(super::cleanup_record_path(&state_path, "runtime-complete").exists());
    assert!(session_record_ids(&state_root).is_empty());
    match previous {
        Some(value) => std::env::set_var("AGENT_ENV_STATE_ROOT", value),
        None => std::env::remove_var("AGENT_ENV_STATE_ROOT"),
    }
}

#[test]
fn autonomous_executor_bridge_reattaches_after_error_and_derives_cleanup_intent() {
    let _environment = TEST_ENVIRONMENT.lock().expect("test environment lock");
    let (fixture, mut state, _snapshot, _) =
        implementation_proof_fixture("runtime-reattach-cleanup-intent");
    let autospec = state.identity.worktree.join(".autospec");
    fs::create_dir_all(&autospec).expect("runtime manifest directory");
    fs::write(
        state.identity.worktree.join("runtime-up.py"),
        "import http.server, os\npid=os.fork()\nif not pid:\n http.server.HTTPServer(('127.0.0.1', int(os.environ['AGENT_FRONTEND_PORT'])), http.server.SimpleHTTPRequestHandler).serve_forever()\nelse:\n open('runtime.pid','w').write(str(pid))\n",
    )
    .expect("runtime up");
    fs::write(
        state.identity.worktree.join("runtime-down.py"),
        "import os, signal\npid=int(open('runtime.pid').read())\nos.kill(pid, signal.SIGTERM)\nos.remove('runtime.pid')\n",
    )
    .expect("runtime down");
    fs::write(
        autospec.join("runtime.yml"),
        "version: 1\ndefault_mode: local\nmodes:\n  local:\n    command: python3 runtime-up.py\n    down: python3 runtime-down.py\n",
    )
    .expect("runtime manifest");
    let state_root = fixture.root.join("runtime-state");
    let previous = std::env::var_os("AGENT_ENV_STATE_ROOT");
    std::env::set_var("AGENT_ENV_STATE_ROOT", &state_root);
    let runtime = super::runtime_session_adapter(&state.identity.worktree)
        .expect("runtime adapter")
        .expect("runtime manifest");
    let environment_dir = runtime.environment_dir().to_path_buf();
    let session_id = runtime.session_id.clone();
    state.phase = super::BridgePhase::CleanupPending;
    state.identity.runtime_environment_dir = Some(environment_dir.clone());
    state.identity.runtime_session_id = Some(session_id.clone());
    let state_path = fixture.root.join("state/invocation.json");
    super::write_invocation_atomic(&state_path, &state).expect("runtime state");

    drop(runtime);
    assert!(
        !super::cleanup_record_path(&state_path, "runtime-intent.json").exists(),
        "the crash window precedes cleanup intent"
    );
    fs::remove_file(autospec.join("runtime.yml"))
        .expect("remove mutable manifest before bridge reattach");
    let reattached = super::reattach_runtime_session_adapter(
        &state.identity.worktree,
        &environment_dir,
        &session_id,
    )
    .expect("reattach exact durable runtime from its private snapshot")
    .expect("persisted runtime binding");
    drop(reattached);
    fs::write(
        autospec.join("runtime.yml"),
        "version: 1\ndefault_mode: local\nmodes:\n  local:\n    command: /bin/false\n    down: /bin/false\n",
    )
    .expect("replace live manifest after persisted runtime binding");

    super::close_owned_runtime(&state_path, &state, None)
        .expect("cleanup from persisted original manifest and binding");
    assert!(super::cleanup_record_path(&state_path, "runtime-intent.json").exists());
    assert!(super::cleanup_record_path(&state_path, "runtime-complete").exists());
    match previous {
        Some(value) => std::env::set_var("AGENT_ENV_STATE_ROOT", value),
        None => std::env::remove_var("AGENT_ENV_STATE_ROOT"),
    }
}

#[test]
fn autonomous_executor_bridge_runtime_close_retries_partial_teardown_failure() {
    let _environment = TEST_ENVIRONMENT.lock().expect("test environment lock");
    let (fixture, mut state, _snapshot, _) =
        implementation_proof_fixture("runtime-close-partial-retry");
    let autospec = state.identity.worktree.join(".autospec");
    fs::create_dir_all(&autospec).expect("runtime manifest directory");
    fs::write(
        state.identity.worktree.join("runtime-up.py"),
        "import http.server, os\npid=os.fork()\nif not pid:\n http.server.HTTPServer(('127.0.0.1', int(os.environ['AGENT_FRONTEND_PORT'])), http.server.SimpleHTTPRequestHandler).serve_forever()\nelse:\n open('runtime.pid','w').write(str(pid))\n",
    )
    .expect("runtime up");
    fs::write(
        state.identity.worktree.join("runtime-down.py"),
        "import os, signal\ntry:\n open('down-attempted','x').write('1')\n raise RuntimeError('42')\nexcept FileExistsError:\n pass\npid=int(open('runtime.pid').read())\nos.kill(pid, signal.SIGTERM)\nos.remove('runtime.pid')\n",
    )
    .expect("runtime down");
    fs::write(
        autospec.join("runtime.yml"),
        "version: 1\ndefault_mode: local\nmodes:\n  local:\n    command: python3 runtime-up.py\n    down: python3 runtime-down.py\n",
    )
    .expect("runtime manifest");
    let state_root = fixture.root.join("runtime-state");
    let previous = std::env::var_os("AGENT_ENV_STATE_ROOT");
    std::env::set_var("AGENT_ENV_STATE_ROOT", &state_root);
    let runtime = super::runtime_session_adapter(&state.identity.worktree)
        .expect("runtime adapter")
        .expect("runtime manifest");
    state.identity.runtime_session_id = Some(runtime.session_id.clone());
    let state_path = fixture.root.join("state/invocation.json");
    super::write_invocation_atomic(&state_path, &state).expect("runtime state");

    let error = super::close_owned_runtime(&state_path, &state, Some(runtime))
        .expect_err("first teardown fails");
    assert!(error.contains("42") || error.contains("runtime"), "{error}");
    assert!(!super::cleanup_record_path(&state_path, "runtime-complete").exists());

    super::close_owned_runtime(&state_path, &state, None)
        .expect("restart retries authoritative teardown");
    assert!(super::cleanup_record_path(&state_path, "runtime-complete").exists());
    assert!(session_record_ids(&state_root).is_empty());
    match previous {
        Some(value) => std::env::set_var("AGENT_ENV_STATE_ROOT", value),
        None => std::env::remove_var("AGENT_ENV_STATE_ROOT"),
    }
}

#[test]
fn autonomous_executor_bridge_rejects_missing_worktree_without_prior_intent() {
    let (fixture, mut state, _snapshot, _) = implementation_proof_fixture("cleanup-no-intent");
    commit_implementation(&state);
    state.phase = super::BridgePhase::CleanupPending;
    state.identity.runtime_environment_dir = None;
    state.identity.runtime_session_id = None;
    let state_path = fixture.root.join("state/invocation.json");
    super::write_invocation_atomic(&state_path, &state).expect("cleanup state");
    git(
        &state.identity.repository_path,
        &[
            "worktree",
            "remove",
            state.identity.worktree.to_str().expect("worktree"),
        ],
    );

    let error = super::finalize_merged_executor(&state_path, &mut state, None)
        .expect_err("missing intent must fail closed");
    assert!(error.contains("before a durable removal intent"), "{error}");
}

#[test]
fn autonomous_executor_bridge_requires_owned_runtime_cleanup_proof() {
    let (fixture, mut state, _snapshot, _) =
        implementation_proof_fixture("cleanup-runtime-proof");
    state.phase = super::BridgePhase::CleanupPending;
    state.identity.runtime_session_id = Some("runtime-owned".into());
    let state_path = fixture.root.join("state/invocation.json");
    super::write_invocation_atomic(&state_path, &state).expect("cleanup state");

    let error = super::finalize_merged_executor(&state_path, &mut state, None)
        .expect_err("runtime receipt is mandatory");
    assert!(error.contains("runtime session"), "{error}");
    assert!(state.identity.worktree.exists());
}

#[test]
fn autonomous_executor_bridge_failure_budget_selects_queue_or_needs_human() {
    assert_eq!(
        super::failure_disposition(true, false),
        crate::commands::claim::BridgeClaimDisposition::Retryable
    );
    assert_eq!(
        super::failure_disposition(true, true),
        crate::commands::claim::BridgeClaimDisposition::NeedsHuman
    );
    assert_eq!(
        super::failure_disposition(false, false),
        crate::commands::claim::BridgeClaimDisposition::NeedsHuman
    );
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_closes_integration_issue() {
    let (fixture, mut state, _snapshot, _) =
        implementation_proof_fixture("close-integration-issue");
    state.phase = super::BridgePhase::Merged;
    state.terminal_result = Some(git_stdout(&fixture.repo, &["rev-parse", "origin/main"]));
    state.identity.base_ref = "origin/integration".into();
    state.current_child = Some(101);
    git(
        &fixture.repo,
        &[
            "--git-dir",
            fixture
                .root
                .join("remote.git")
                .to_str()
                .expect("remote path"),
            "update-ref",
            "refs/heads/integration",
            state.terminal_result.as_deref().expect("merge OID"),
        ],
    );

    let calls = fixture.root.join("issue-calls");
    let issue_state = fixture.root.join("issue-state");
    fs::write(&issue_state, "OPEN\n").expect("issue state");
    let gh = fixture.root.join("gh-close-issue");
    write_executable(
        &gh,
        r#"#!/bin/sh
set -eu
printf '%s\n' "$*" >> "$ISSUE_CALLS"
number=${RETURN_NUMBER:-$3}
case "$1 $2" in
  "issue view") printf '{"number":%s,"state":"%s"}\n' "$number" "$(cat "$ISSUE_STATE")" ;;
  "issue close")
[ "${CLOSE_FAIL:-0}" = 0 ] || exit 65
printf '%s\n' CLOSED > "$ISSUE_STATE"
;;
  *) exit 64 ;;
esac
"#,
    );
    let adapter = super::DraftPrAdapter {
        gh,
        environment: BTreeMap::from([
            ("ISSUE_CALLS".into(), calls.clone().into_os_string()),
            ("ISSUE_STATE".into(), issue_state.clone().into_os_string()),
        ]),
    };

    super::close_merged_integration_issue(&state, &adapter).expect("close continuation");
    let call_log = fs::read_to_string(&calls).expect("calls");
    assert_eq!(call_log.matches("issue close 101").count(), 1, "{call_log}");
    assert!(!call_log.contains("issue close 42"), "{call_log}");
    assert_eq!(state.phase, super::BridgePhase::Merged);

    fs::write(fixture.repo.join("integration-advance"), "next\n")
        .expect("integration advance fixture");
    git(&fixture.repo, &["add", "integration-advance"]);
    git(&fixture.repo, &["commit", "-m", "advance integration"]);
    git(
        &fixture.repo,
        &["push", "origin", "HEAD:refs/heads/integration"],
    );
    fs::write(&calls, "").expect("clear advanced-tip calls");
    fs::write(&issue_state, "OPEN\n").expect("reopen advanced-tip issue");
    super::close_merged_integration_issue(&state, &adapter)
        .expect("descendant integration tip preserves closure proof");
    let advanced_calls = fs::read_to_string(&calls).expect("advanced-tip calls");
    assert_eq!(
        advanced_calls.matches("issue close 101").count(),
        1,
        "{advanced_calls}"
    );

    fs::write(&calls, "").expect("clear calls");
    fs::write(&issue_state, "OPEN\n").expect("reopen legacy issue");
    state.current_child = None;
    super::close_merged_integration_issue(&state, &adapter).expect("close legacy issue");
    let legacy_calls = fs::read_to_string(&calls).expect("legacy calls");
    assert_eq!(
        legacy_calls.matches("issue close 42").count(),
        1,
        "{legacy_calls}"
    );
    assert!(!legacy_calls.contains("issue close 101"), "{legacy_calls}");

    fs::write(&calls, "").expect("clear default-branch calls");
    state.identity.base_ref = "origin/main".into();
    super::close_merged_integration_issue(&state, &adapter).expect("default branch merge");
    assert_eq!(fs::read_to_string(&calls).expect("default calls"), "");

    state.identity.base_ref = "origin/integration".into();
    let mut mismatched = adapter.clone();
    mismatched
        .environment
        .insert("RETURN_NUMBER".into(), "99".into());
    assert!(super::close_merged_integration_issue(&state, &mismatched).is_err());
    assert_eq!(state.phase, super::BridgePhase::Merged);

    let mut failing = adapter.clone();
    failing.environment.insert("CLOSE_FAIL".into(), "1".into());
    fs::write(&issue_state, "OPEN\n").expect("reopen failed-close issue");
    assert!(super::close_merged_integration_issue(&state, &failing).is_err());
    assert_eq!(state.phase, super::BridgePhase::Merged);

    let rewrite_tree = git_stdout(&fixture.repo, &["rev-parse", "HEAD^{tree}"]);
    let rewritten_integration = git_stdout(
        &fixture.repo,
        &["commit-tree", &rewrite_tree, "-m", "rewrite integration"],
    );
    git(
        &fixture.repo,
        &[
            "push",
            "--force",
            "origin",
            &format!("{rewritten_integration}:refs/heads/integration"),
        ],
    );
    fs::write(&calls, "").expect("clear rewritten-tip calls");
    let error = super::close_merged_integration_issue(&state, &adapter)
        .expect_err("rewritten integration tip must fail closed");
    assert!(format!("{error:?}").contains("not an ancestor"));
    assert_eq!(fs::read_to_string(&calls).expect("rewritten-tip calls"), "");
}
