// executor_bridge tests: closeout / repairs — 4 cases.
//
// Split out of tests.rs; see the note in that file.

use crate::commands::autonomous::executor_bridge as bridge;
use super::super::{
    BridgePhase, MutationSnapshot, PersistedInvocation, ProcessIdentity, ResolvedBase,
};
use super::support_base::{git, git_stdout, zero_effect_classifier_fixture};
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::atomic::Ordering;

#[test]
fn autonomous_executor_bridge_repairs_invalid_closeout() {
    let (_fixture, state, state_path, _) =
        zero_effect_classifier_fixture("invalid-closeout-recovery", false, false);
    fs::write(
        state.identity.worktree.join("implementation.txt"),
        "implemented\n",
    )
    .expect("write implementation");
    git(&state.identity.worktree, &["add", "implementation.txt"]);
    git(
        &state.identity.worktree,
        &["commit", "-m", "test: preserve completed implementation"],
    );
    let head_before = git_stdout(&state.identity.worktree, &["rev-parse", "HEAD"]);
    let closeout = state
        .identity
        .worktree
        .join(".autospec/executor-closeout.md");
    fs::create_dir_all(closeout.parent().expect("closeout parent"))
        .expect("create closeout parent");
    let invalid = "Implemented the issue.\n\nCloseout: executor-closeout.md\n";
    fs::write(&closeout, invalid).expect("write invalid closeout");
    fs::set_permissions(&closeout, fs::Permissions::from_mode(0o600))
        .expect("secure invalid closeout");

    assert!(
        bridge::recoverable_implementation_completion_for_state(&state_path, &state)
            .expect("repair invalid closeout")
    );

    let normalized = fs::read_to_string(&closeout).expect("read normalized closeout");
    assert!(
        bridge::validate_closeout_report(&state.identity.worktree, &closeout).is_ok(),
        "{normalized}"
    );
    assert!(normalized.contains("Claims: [assumed] static"));
    assert_eq!(
        git_stdout(&state.identity.worktree, &["rev-parse", "HEAD"]),
        head_before,
        "closeout repair must preserve the committed implementation"
    );
    let archive_count = fs::read_dir(closeout.parent().expect("closeout parent"))
        .expect("read closeout directory")
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("executor-closeout.invalid-")
        })
        .count();
    assert_eq!(
        archive_count, 1,
        "original malformed output must be archived"
    );
    let prompt = bridge::build_implementer_prompt(
        &state.identity,
        "Implement issue",
        "## Goal\n\nImplement the issue.",
        &closeout,
    )
    .expect("build exact closeout prompt");
    for field in [
        "Result:",
        "Claims:",
        "Proof type:",
        "Before/after:",
        "Artifacts:",
        "Scoped git status:",
        "One likely hidden failure:",
    ] {
        assert!(prompt.contains(field), "prompt omitted {field}");
    }
    assert!(prompt.contains("final response byte-for-byte"));
}

#[test]
fn autonomous_executor_bridge_recovers_exact_nonzero_implementation_completion() {
    let (_fixture, state, state_path, _) =
        zero_effect_classifier_fixture("implementation-complete-nonzero", false, false);
    fs::write(
        state.identity.worktree.join("implementation.txt"),
        "implemented\n",
    )
    .expect("write implementation diff");
    let closeout = state
        .identity
        .worktree
        .join(".autospec/executor-closeout.md");
    fs::create_dir_all(closeout.parent().expect("closeout parent"))
        .expect("create closeout parent");
    fs::write(
        &closeout,
        "## Closeout report\n\n\
Result: Added the requested implementation.\n\
Claims: [verified] runtime the focused test exits with status 0.\n\
Proof type: runtime\n\
Before/after: Before 0 implementation files; after 1 implementation file.\n\
Artifacts: `implementation.txt`; rerun with `test -f implementation.txt`.\n\
Scoped git status: Added `implementation.txt`; closeout excluded from the commit.\n\
One likely hidden failure: The focused fixture does not exercise a remote push.\n",
    )
    .expect("write closeout");
    fs::set_permissions(&closeout, fs::Permissions::from_mode(0o600)).expect("private closeout");

    assert!(
        bridge::recoverable_implementation_completion_for_state(&state_path, &state)
            .expect("classify exact nonzero completion")
    );
    bridge::record_worktree_creation_identity(
        &state.identity.repository_path,
        &state.identity.branch,
        &ResolvedBase {
            base_ref: state.identity.base_ref.clone(),
            base_oid: state.identity.base_oid.clone(),
            explore_mode: false,
        },
    )
    .expect("record worktree creation identity");
    let common_dir = PathBuf::from(git_stdout(
        &state.identity.worktree,
        &["rev-parse", "--git-common-dir"],
    ));
    let common_dir = if common_dir.is_absolute() {
        common_dir
    } else {
        state.identity.worktree.join(common_dir)
    };
    fs::write(common_dir.join("info/exclude"), ".autospec/\n")
        .expect("ignore private executor artifacts");
    assert!(
        bridge::recover_invocation(&state_path, &state.identity)
            .expect("recover nonzero completion")
            .is_some(),
        "bridge invocation loading must preserve its completed implementation diff"
    );

    fs::remove_file(&closeout).expect("remove closeout");
    assert!(
        !bridge::recoverable_implementation_completion_for_state(&state_path, &state)
            .expect("missing closeout remains fail-closed")
    );

    fs::write(&closeout, "internal-only\n").expect("write ignored internal metadata");
    fs::set_permissions(&closeout, fs::Permissions::from_mode(0o600))
        .expect("private internal metadata");
    fs::remove_file(state.identity.worktree.join("implementation.txt"))
        .expect("remove implementation diff");
    assert!(
        !bridge::recoverable_implementation_completion_for_state(&state_path, &state)
            .expect("internal metadata alone remains fail-closed")
    );

    fs::write(
        state.identity.worktree.join("implementation.txt"),
        "implemented\n",
    )
    .expect("restore implementation diff");
    let mut active = state.clone();
    active.process = Some(ProcessIdentity {
        pid: 42,
        process_group: 42,
        executable: PathBuf::from("/usr/bin/codex"),
        argv_digest: "a".repeat(64),
        boot_id: "boot".to_string(),
        start_identity: "start".to_string(),
    });
    assert!(
        !bridge::recoverable_implementation_completion_for_state(&state_path, &active)
            .expect("live child state remains fail-closed")
    );

    fs::write(
        &closeout,
        "## Closeout report\n\n\
Result: Added the requested implementation.\n\
Claims: [verified] runtime the focused test exits with status 0.\n\
Proof type: runtime\n\
Before/after: Before 0 implementation files; after 1 implementation file.\n\
Artifacts: `implementation.txt`; rerun with `test -f implementation.txt`.\n\
Scoped git status: Added `implementation.txt`; closeout excluded from the commit.\n\
One likely hidden failure: The focused fixture does not exercise a remote push.\n",
    )
    .expect("restore valid closeout");
    git(&state.identity.worktree, &["add", "implementation.txt"]);
    git(
        &state.identity.worktree,
        &["commit", "-m", "test: preserve completed implementation"],
    );
    let mut proven = state.clone();
    proven.phase = BridgePhase::ImplementationProven;
    proven.head_oid = Some(git_stdout(
        &proven.identity.worktree,
        &["rev-parse", "HEAD"],
    ));
    proven.closeout_path = Some(fs::canonicalize(&closeout).expect("canonical closeout"));
    proven.closeout_digest = Some(bridge::sha256_hex(
        fs::read_to_string(&closeout)
            .expect("read valid closeout")
            .as_bytes(),
    ));
    bridge::write_invocation_atomic(&state_path, &proven).expect("persist proven implementation");
    assert!(
        bridge::recoverable_implementation_completion_for_state(&state_path, &proven)
            .expect("classify proven completion"),
        "post-proof bridge state must remain resumable after a correctable gate failure"
    );

    let mut draft_created = proven.clone();
    draft_created.phase = BridgePhase::DraftCreated;
    draft_created.pr = Some(17);
    draft_created.draft_process = Some(ProcessIdentity {
        pid: u32::MAX,
        process_group: u32::MAX,
        executable: PathBuf::from("/usr/bin/gh"),
        argv_digest: "b".repeat(64),
        boot_id: "exited-boot".to_string(),
        start_identity: "exited-start".to_string(),
    });
    for (phase, terminal_result) in [
        (BridgePhase::DraftCreated, None),
        (BridgePhase::Ready, None),
        (BridgePhase::CiPassed, None),
        (BridgePhase::ReviewPassed, None),
        (
            BridgePhase::ResultAccepted,
            Some(format!("accepted:{}", "d".repeat(64))),
        ),
        (
            BridgePhase::MergeRequested,
            Some(format!("accepted:{}", "d".repeat(64))),
        ),
        (BridgePhase::Merged, Some("e".repeat(40))),
        (BridgePhase::CleanupPending, Some("e".repeat(40))),
    ] {
        let mut completed_phase = draft_created.clone();
        completed_phase.phase = phase;
        completed_phase.terminal_result = terminal_result;
        bridge::write_invocation_atomic(&state_path, &completed_phase)
            .expect("persist exact post-draft state");
        assert!(
            bridge::recoverable_implementation_completion_for_state(&state_path, &completed_phase)
                .expect("classify post-draft state with exited gh process"),
            "{phase:?} must remain resumable after its gh process exits"
        );
    }

    for phase in [
        BridgePhase::ImplementationProven,
        BridgePhase::BranchPushing,
        BridgePhase::BranchPushed,
        BridgePhase::DraftCreating,
        BridgePhase::DraftCleanupPending,
    ] {
        let mut pre_draft = draft_created.clone();
        pre_draft.phase = phase;
        assert!(
            !bridge::recoverable_implementation_completion_for_state(&state_path, &pre_draft)
                .expect("pre-draft process remains fail-closed"),
            "{phase:?} must not treat its draft process as completed"
        );
    }

    let birth = bridge::observe_process_birth(std::process::id())
        .expect("observe test process")
        .expect("test process remains live");
    let mut active_draft = draft_created;
    active_draft.draft_process = Some(ProcessIdentity {
        pid: birth.pid,
        process_group: birth.process_group,
        executable: PathBuf::from("/usr/bin/gh"),
        argv_digest: "c".repeat(64),
        boot_id: birth.boot_id,
        start_identity: birth.start_identity,
    });
    assert!(
        !bridge::recoverable_implementation_completion_for_state(&state_path, &active_draft)
            .expect("live draft child remains fail-closed")
    );
}

#[cfg(target_os = "linux")]
#[test]
fn process_group_observation_treats_esrch_as_absent() {
    // Break caught: a process exiting between /proc stat and getpgid being reported as an
    // infrastructure failure instead of a completed observation.
    assert_eq!(
        bridge::observe_process_group(i32::MAX as u32).expect("observe absent process group"),
        None
    );
}

#[test]
fn autonomous_executor_bridge_recovers_commit_before_proof_persistence() {
    let (_fixture, state, state_path, _) =
        zero_effect_classifier_fixture("implementation-commit-before-proof", false, false);
    let protected =
        MutationSnapshot::capture(&state.identity.repository_path, &state.identity.branch)
            .expect("capture protected repository state");
    fs::write(
        state.identity.worktree.join("implementation.txt"),
        "committed before proof persistence\n",
    )
    .expect("write implementation diff");
    let closeout = state
        .identity
        .worktree
        .join(".autospec/executor-closeout.md");
    fs::create_dir_all(closeout.parent().expect("closeout parent"))
        .expect("create closeout parent");
    fs::write(
        &closeout,
        "## Closeout report\n\n\
Result: Added the requested implementation.\n\
Claims: [verified] runtime the focused test exits with status 0.\n\
Proof type: runtime\n\
Before/after: Before 0 implementation files; after 1 implementation file.\n\
Artifacts: `implementation.txt`; rerun with `test -f implementation.txt`.\n\
Scoped git status: Added `implementation.txt`; closeout excluded from the commit.\n\
One likely hidden failure: The focused fixture does not exercise a remote push.\n",
    )
    .expect("write closeout");
    fs::set_permissions(&closeout, fs::Permissions::from_mode(0o600)).expect("private closeout");
    bridge::record_worktree_creation_identity(
        &state.identity.repository_path,
        &state.identity.branch,
        &ResolvedBase {
            base_ref: state.identity.base_ref.clone(),
            base_oid: state.identity.base_oid.clone(),
            explore_mode: false,
        },
    )
    .expect("record worktree creation identity");
    let common_dir = PathBuf::from(git_stdout(
        &state.identity.worktree,
        &["rev-parse", "--git-common-dir"],
    ));
    let common_dir = if common_dir.is_absolute() {
        common_dir
    } else {
        state.identity.worktree.join(common_dir)
    };
    fs::write(common_dir.join("info/exclude"), ".autospec/\n")
        .expect("ignore private executor artifacts");

    bridge::IMPLEMENTATION_COMMIT_FAILPOINT.store(1, Ordering::SeqCst);
    let error =
        bridge::commit_sandboxed_executor_diff(&state, "test: persist implementation proof", "")
            .expect_err("interrupt after Rust commit");
    assert!(error.contains("after implementation commit"), "{error}");
    let durable = PersistedInvocation::from_json(
        &fs::read_to_string(&state_path).expect("read interrupted invocation"),
    )
    .expect("parse interrupted invocation");
    assert_eq!(durable.phase, BridgePhase::ImplementationComplete);
    assert_eq!(
        git_stdout(&durable.identity.worktree, &["status", "--porcelain=v1"]),
        "",
        "the Rust commit must leave an exact clean worktree"
    );
    assert_ne!(
        git_stdout(&durable.identity.worktree, &["rev-parse", "HEAD"]),
        durable.identity.base_oid
    );
    assert!(
        bridge::recoverable_implementation_completion_for_state(&state_path, &durable)
            .expect("classify commit-before-proof recovery"),
        "an exact clean committed HEAD must resume without another implementer"
    );

    let mut recovered = bridge::recover_invocation(&state_path, &durable.identity)
        .expect("recover committed invocation")
        .expect("committed invocation remains present");
    let proof = bridge::prove_implementation(&state_path, &mut recovered, &protected, &closeout)
        .expect("persist recovered implementation proof");
    assert_eq!(recovered.phase, BridgePhase::ImplementationProven);
    assert_eq!(recovered.head_oid.as_deref(), Some(proof.head_oid.as_str()));
}
