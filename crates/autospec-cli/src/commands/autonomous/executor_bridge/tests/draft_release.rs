// executor_bridge tests: draft / release — 10 cases.
//
// Split out of tests.rs; see the note in that file.

use super::super::{BridgePhase, PersistedInvocation};
use super::support_base::{git, git_stdout, test_environment};
use super::support_invocation::{commit_implementation, implementation_proof_fixture};
use super::support_launch::{
    adapter_path, draft_pr_adapter_fixture, prepared_draft_transaction, DRAFT_ISSUE_BODY,
};
use crate::commands::autonomous::executor_bridge as bridge;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::time::{Duration, Instant};

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_retry_adopts_preserved_remote_wip_branch() {
    let (fixture, mut state, _snapshot, closeout) =
        implementation_proof_fixture("retry-preserved-branch");
    let state_path = fixture.root.join("state/invocation.json");
    commit_implementation(&state);
    let preserved_head = git_stdout(&state.identity.worktree, &["rev-parse", "HEAD"]);
    git(
        &state.identity.worktree,
        &[
            "push",
            "origin",
            &format!("{preserved_head}:refs/heads/{}", state.identity.branch),
        ],
    );
    let snapshot = bridge::MutationSnapshot::capture(&fixture.repo, &state.identity.branch)
        .expect("retry local snapshot");
    let empty_adapter = draft_pr_adapter_fixture(&fixture, &state_path, "[]");
    state.phase = BridgePhase::Pending;
    bridge::write_invocation_atomic(&state_path, &state).expect("retry pending");
    bridge::RemoteMutationSnapshot::capture_and_persist(&state_path, &mut state, &empty_adapter)
        .expect("retry prelaunch remote");
    fs::write(
        state.identity.worktree.join("implementation.txt"),
        "implemented\ncontinued\n",
    )
    .expect("continued WIP");
    git(&state.identity.worktree, &["add", "implementation.txt"]);
    git(
        &state.identity.worktree,
        &["commit", "-m", "feat: continue preserved WIP"],
    );
    state.phase = BridgePhase::ImplementationComplete;
    let proof = bridge::prove_implementation(&state_path, &mut state, &snapshot, &closeout)
        .expect("prove continued WIP");
    let body = format!("Closes #42\n\n{}", proof.closeout_body);
    let created = format!(
        "[{{\"number\":17,\"body\":{},\"headRefName\":\"{}\",\"headRefOid\":\"{}\",\"isDraft\":true,\"baseRefName\":\"main\"}}]",
        serde_json::to_string(&body).unwrap(),
        state.identity.branch,
        proof.head_oid,
    );
    let adapter = draft_pr_adapter_fixture(&fixture, &state_path, &created);

    assert_eq!(
        bridge::push_and_create_draft(
            &state_path,
            &mut state,
            &proof,
            "Retry issue",
            DRAFT_ISSUE_BODY,
            &adapter,
        )
        .expect("fast-forward preserved branch and create draft"),
        17
    );
    assert_eq!(state.phase, BridgePhase::DraftCreated);
    assert_ne!(proof.head_oid, preserved_head);
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_retry_closes_pr_but_preserves_remote_wip() {
    let mut prepared = prepared_draft_transaction("retry-close-preserve");
    prepared.publish().expect("published draft");
    let branch = prepared.state.identity.branch.clone();
    let head = prepared.proof.head_oid.clone();

    bridge::close_retryable_pull_request(&prepared.state_path, &prepared.state, &prepared.adapter)
        .expect("close retryable PR");
    bridge::close_retryable_pull_request(&prepared.state_path, &prepared.state, &prepared.adapter)
        .expect("restart adopts durable close");

    assert_eq!(
        fs::read_to_string(adapter_path(&prepared.adapter, "GH_PR_STATE"))
            .expect("closed PR inventory")
            .trim(),
        "[]"
    );
    assert_eq!(
        git_stdout(
            &prepared.fixture.root,
            &[
                "--git-dir",
                prepared.fixture.root.join("remote.git").to_str().unwrap(),
                "rev-parse",
                &format!("refs/heads/{branch}"),
            ],
        ),
        head,
        "retry cleanup must retain committed WIP"
    );
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_retry_close_requires_live_claim_and_exact_closed_state() {
    let mut takeover = prepared_draft_transaction("retry-close-takeover");
    takeover.publish().expect("published draft");
    let error = bridge::close_retryable_pull_request_with_refresh(
        &takeover.state_path,
        &takeover.state,
        &takeover.adapter,
        || Ok(bridge::BridgeClaimOwnership::Lost),
    )
    .expect_err("takeover must block PR close");
    assert!(error.contains("ownership"), "{error}");
    assert!(
        !fs::read_to_string(takeover.fixture.root.join("gh-calls"))
            .expect("gh calls")
            .lines()
            .any(|call| call.starts_with("pr close")),
        "stale worker issued a PR close"
    );

    let mut merged = prepared_draft_transaction("retry-close-merged");
    merged.publish().expect("published draft");
    let binding = autospec_core::autonomous::waterfall::sha256_hex(
        format!(
            "{}\0{}\0{}",
            bridge::cleanup_binding(&merged.state),
            merged.state.pr.expect("PR"),
            merged.state.head_oid.as_deref().expect("head")
        )
        .as_bytes(),
    );
    bridge::ensure_cleanup_record(
        &bridge::cleanup_record_path(&merged.state_path, "retry-pr-close-intent"),
        &binding,
        "test retry close intent",
    )
    .expect("close intent");
    fs::write(adapter_path(&merged.adapter, "GH_PR_STATE"), "[]")
        .expect("stale empty open PR inventory");
    fs::write(
        adapter_path(&merged.adapter, "GH_CLOSED_PR"),
        serde_json::json!({
            "number": merged.state.pr,
            "state": "CLOSED",
            "mergedAt": null,
            "headRefName": merged.state.identity.branch,
            "headRefOid": merged.state.head_oid,
            "baseRefName": "main",
            "body": "Closes #999\n\n## Closeout report\n",
        })
        .to_string(),
    )
    .expect("merged observation");
    let error =
        bridge::close_retryable_pull_request(&merged.state_path, &merged.state, &merged.adapter)
            .expect_err("merged PR must not be treated as safely closed");
    assert!(error.contains("CLOSED non-merged"), "{error}");
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_recovers_push_and_draft_creation_boundaries() {
    // Break caught: restart duplicating a push or draft after Rust mutated remote state.
    let mut pushed = prepared_draft_transaction("draft-recover-push");
    pushed.push_exact_at_intent();
    pushed.publish().expect("recover exact pushed OID");
    assert_eq!(pushed.state.phase, BridgePhase::DraftCreated);

    let mut created = prepared_draft_transaction("draft-recover-create");
    created.push_exact_at_intent();
    created.state.phase = BridgePhase::DraftCreating;
    bridge::write_invocation_atomic(&created.state_path, &created.state).expect("create intent");
    fs::copy(
        adapter_path(&created.adapter, "GH_CREATED_PR"),
        adapter_path(&created.adapter, "GH_PR_STATE"),
    )
    .expect("simulate completed create");
    fs::write(created.fixture.root.join("gh-calls"), "").expect("clear calls");

    let pr = created.publish().expect("adopt exact authoritative draft");

    assert_eq!(pr, 17);
    assert_eq!(created.state.phase, BridgePhase::DraftCreated);
    let calls = fs::read_to_string(created.fixture.root.join("gh-calls")).expect("gh calls");
    assert_eq!(calls.matches("pr create").count(), 0);

    let mut before_create = prepared_draft_transaction("draft-recover-before-create");
    before_create.push_exact_at_intent();
    before_create.state.phase = BridgePhase::DraftCreating;
    bridge::write_invocation_atomic(&before_create.state_path, &before_create.state)
        .expect("create intent");
    fs::write(before_create.fixture.root.join("gh-calls"), "").expect("clear calls");

    let pr = before_create
        .publish()
        .expect("unreleased create intent is safe to retry");
    assert_eq!(pr, 17);
    let calls = fs::read_to_string(before_create.fixture.root.join("gh-calls")).expect("gh calls");
    assert_eq!(calls.matches("pr create").count(), 1);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn autonomous_executor_bridge_restart_never_duplicates_inflight_draft_create() {
    let _environment = test_environment();
    // Break caught: spawn returning before exact gh child identity is durable.
    let mut prepared = prepared_draft_transaction("draft-create-inflight");
    let delay = prepared.fixture.root.join("create.delay");
    let started = prepared.fixture.root.join("create.started");
    let inflight = prepared.fixture.root.join("create.inflight");
    let state_path = prepared.state_path.clone();
    let calls_path = prepared.fixture.root.join("gh-calls");
    fs::write(&delay, "").expect("delay sentinel");
    prepared
        .adapter
        .environment
        .insert("GH_CREATE_DELAY".into(), delay.clone().into_os_string());
    prepared
        .adapter
        .environment
        .insert("GH_CREATE_STARTED".into(), started.clone().into_os_string());
    prepared
        .adapter
        .environment
        .insert("GH_INFLIGHT".into(), inflight.into_os_string());
    let publisher = std::thread::spawn(move || {
        let result = prepared.publish();
        (prepared, result)
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while !started.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(started.exists(), "delayed gh never entered create");
    let durable = fs::read_to_string(&state_path).expect("durable create identity");
    assert!(durable.contains("\"draft_process\":{"), "{durable}");
    let calls = fs::read_to_string(&calls_path).expect("gh calls");
    assert_eq!(calls.matches("pr create").count(), 1);
    fs::remove_file(delay).expect("release delayed gh");
    let (prepared, result) = publisher.join().expect("join publisher");
    assert_eq!(result.expect("finish single create"), 17);
    assert!(prepared.state.draft_process.is_some());
}

#[cfg(target_os = "macos")]
#[test]
fn autonomous_executor_bridge_draft_release_child_closes_unrelated_inherited_lock() {
    let _environment = test_environment();
    let mut prepared = prepared_draft_transaction("draft-release-inherited-lock");
    let hold = prepared.fixture.root.join("draft-release.hold");
    let started = prepared.fixture.root.join("draft-release.started");
    let lock_path = prepared.fixture.root.join("unrelated.lock");
    fs::write(&hold, b"hold\n").expect("create draft release hold");
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&lock_path)
        .expect("open unrelated lock");
    lock.try_lock().expect("acquire unrelated lock");
    prepared.adapter.environment.insert(
        "AUTOSPEC_TEST_DRAFT_RELEASE_HOLD".into(),
        hold.clone().into_os_string(),
    );
    prepared.adapter.environment.insert(
        "AUTOSPEC_TEST_DRAFT_RELEASE_HOLD_STARTED".into(),
        started.clone().into_os_string(),
    );
    let publisher = std::thread::spawn(move || prepared.publish());
    let deadline = Instant::now() + Duration::from_secs(5);
    while !started.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(started.exists(), "draft release child did not reach hold");
    drop(lock);
    let probe = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(lock_path)
        .expect("reopen unrelated lock");
    let probe_result = probe.try_lock();
    fs::remove_file(hold).expect("release draft child");
    assert_eq!(publisher.join().expect("join publisher").expect("publish"), 17);
    probe_result.expect("draft release child must not retain unrelated lock");
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn autonomous_executor_bridge_durable_release_gate_precedes_gh_start() {
    // Break caught: gh starting before its exact prepared identity/release is durable.
    let mut prepared = prepared_draft_transaction("draft-create-release-gate");
    prepared
        .adapter
        .environment
        .insert("GH_REQUIRE_DURABLE_RELEASE".into(), "1".into());

    let pull_request = prepared
        .publish()
        .expect("durable release gate must precede gh start");

    assert_eq!(pull_request, 17);
    assert!(prepared.state.draft_process.is_some());
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn autonomous_executor_bridge_retries_only_a_proven_never_released_draft_child() {
    // Break caught: a parent crash before release permanently stranding a safe create intent.
    let mut prepared = prepared_draft_transaction("draft-create-never-released");
    prepared.push_exact_at_intent();
    prepared.state.phase = BridgePhase::DraftCreating;
    prepared.state.draft_process = Some(bridge::ProcessIdentity {
        pid: 4_000_000,
        process_group: 4_000_000,
        executable: prepared.adapter.gh.clone(),
        argv_digest: "prepared-never-released".to_string(),
        boot_id: "missing-boot".to_string(),
        start_identity: "missing-start".to_string(),
    });
    bridge::write_invocation_atomic(&prepared.state_path, &prepared.state)
        .expect("prepared create identity");

    let pull_request = prepared
        .publish()
        .expect("dead child without release receipt is safe to retry");

    assert_eq!(pull_request, 17);
    let calls = fs::read_to_string(prepared.fixture.root.join("gh-calls")).expect("gh calls");
    assert_eq!(calls.matches("pr create").count(), 1);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn autonomous_executor_bridge_parent_loss_before_release_never_starts_gh() {
    // Break caught: the suspended child sending a request after its parent disappears pre-release.
    let mut prepared = prepared_draft_transaction("draft-create-parent-loss");
    prepared.adapter.environment.insert(
        "AUTOSPEC_TEST_DRAFT_ABORT_BEFORE_RELEASE".into(),
        "1".into(),
    );

    let error = prepared
        .publish()
        .expect_err("injected parent loss must stop before release");

    assert!(error.contains("before release"), "{error}");
    assert_eq!(prepared.state.phase, BridgePhase::DraftCreating);
    assert!(prepared.state.draft_process.is_some());
    assert!(!bridge::draft_release_receipt_path(&prepared.state_path).exists());
    let calls = fs::read_to_string(prepared.fixture.root.join("gh-calls")).expect("gh calls");
    assert_eq!(calls.matches("pr create").count(), 0);

    prepared.adapter.environment.remove(std::ffi::OsStr::new(
        "AUTOSPEC_TEST_DRAFT_ABORT_BEFORE_RELEASE",
    ));
    let pull_request = prepared
        .publish()
        .expect("restart safely retries the never-released child");
    assert_eq!(pull_request, 17);
    let calls = fs::read_to_string(prepared.fixture.root.join("gh-calls")).expect("gh calls");
    assert_eq!(calls.matches("pr create").count(), 1);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn autonomous_executor_bridge_launch_failure_after_release_is_safely_retryable() {
    // Break caught: failed child launch leaving a release receipt that permanently blocks retry.
    let mut prepared = prepared_draft_transaction("draft-create-launch-failure");
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

    let error = prepared
        .publish()
        .expect_err("removed executable must fail after durable release");

    assert!(
        error.contains("list executor open pull requests")
            || error.contains("draft pull request failed"),
        "{error}"
    );
    assert!(!bridge::draft_release_receipt_path(&prepared.state_path).exists());
    let calls = fs::read_to_string(prepared.fixture.root.join("gh-calls")).expect("gh calls");
    assert_eq!(calls.matches("pr create").count(), 0);

    fs::write(&executable, executable_body).expect("restore fixture executable");
    fs::set_permissions(&executable, fs::Permissions::from_mode(executable_mode))
        .expect("restore fixture executable mode");
    prepared.adapter.environment.remove(std::ffi::OsStr::new(
        "AUTOSPEC_TEST_DRAFT_REMOVE_EXECUTABLE_BEFORE_RELEASE",
    ));

    let pull_request = prepared
        .publish()
        .expect("known-safe launch failure must retry once");

    assert_eq!(pull_request, 17);
    let calls = fs::read_to_string(prepared.fixture.root.join("gh-calls")).expect("gh calls");
    assert_eq!(calls.matches("pr create").count(), 1);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn autonomous_executor_bridge_recovers_after_durable_intent_clear_crash() {
    // Break caught: a crash after durable intent removal permanently stranding a safe retry.
    let mut prepared = prepared_draft_transaction("draft-create-intent-clear-crash");
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
        "AUTOSPEC_TEST_DRAFT_ABORT_AFTER_INTENT_CLEAR".into(),
        "1".into(),
    );

    let error = prepared
        .publish()
        .expect_err("post-intent-clear crash must interrupt the durable reset");

    assert!(error.contains("after intent clear"), "{error}");
    let durable = PersistedInvocation::from_json(
        &fs::read_to_string(&prepared.state_path).expect("cleanup-pending invocation"),
    )
    .expect("strict cleanup-pending invocation");
    assert_eq!(durable.phase, BridgePhase::DraftCleanupPending);
    assert!(durable.draft_process.is_some());
    assert!(!bridge::draft_release_receipt_path(&prepared.state_path).exists());
    assert!(!bridge::draft_release_intent_path(&prepared.state_path).exists());
    let calls = fs::read_to_string(prepared.fixture.root.join("gh-calls")).expect("gh calls");
    assert_eq!(calls.matches("pr create").count(), 0);

    fs::write(&executable, executable_body).expect("restore fixture executable");
    fs::set_permissions(&executable, fs::Permissions::from_mode(executable_mode))
        .expect("restore fixture executable mode");
    prepared.adapter.environment.remove(std::ffi::OsStr::new(
        "AUTOSPEC_TEST_DRAFT_REMOVE_EXECUTABLE_BEFORE_RELEASE",
    ));
    prepared.adapter.environment.remove(std::ffi::OsStr::new(
        "AUTOSPEC_TEST_DRAFT_ABORT_AFTER_INTENT_CLEAR",
    ));
    prepared.state = durable;

    let pull_request = prepared
        .publish()
        .expect("durably cleared cleanup guards must authorize one retry");

    assert_eq!(pull_request, 17);
    let calls = fs::read_to_string(prepared.fixture.root.join("gh-calls")).expect("gh calls");
    assert_eq!(calls.matches("pr create").count(), 1);
}
