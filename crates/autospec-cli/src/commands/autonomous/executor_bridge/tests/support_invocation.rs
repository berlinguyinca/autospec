// executor_bridge tests: shared fixtures (2 of 3).
//
// Split out of tests.rs; see the note in that file. These are the helpers more than
// one test module builds on, so they are `pub(super)` rather than private.

use super::super::{
    resolve_base, BridgeIdentity, BridgePhase, ExecutorBridgeRequest, HarnessInvocation,
    HarnessKind, MutationSnapshot, PersistedInvocation, ProcessIdentity, ResolvedBase,
    SupervisionConfig,
};
#[cfg(target_os = "linux")]
use super::support_base::DetachedForkedCleanup;
use super::support_base::{git, git_stdout, GitFixture, TEST_SEQUENCE};
use crate::commands::autonomous::executor_bridge as bridge;
use std::collections::BTreeMap;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(target_os = "linux")]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
#[cfg(target_os = "linux")]
use std::process::Command;
use std::sync::atomic::Ordering;
use std::time::Duration;
#[cfg(target_os = "linux")]
use std::time::Instant;

pub(super) fn prunable_zero_effect_branch_fixture(
    label: &str,
    preserve_transfer: bool,
) -> (GitFixture, String, bridge::IssueWorktree, ResolvedBase) {
    let fixture = GitFixture::new(label);
    let original = resolve_base(&fixture.repo, &BTreeMap::new()).expect("resolve original base");
    let scope = format!(
        "{label}_{}_{}",
        std::process::id(),
        TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let worktree = bridge::provision_issue_worktree_for_claim(
        &fixture.repo,
        &scope,
        42,
        &original,
        Some(("claim-old", "invocation-old")),
    )
    .expect("provision original zero-effect worktree");
    let transfer_path =
        bridge::ownership_transfer_path(worktree.path.parent().expect("scope root"), 42);
    let mut transfer: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&transfer_path).expect("read ownership transfer"))
            .expect("parse ownership transfer");
    transfer["state"] = serde_json::json!("available");
    transfer
        .as_object_mut()
        .expect("ownership transfer object")
        .remove("to_claim_id");
    transfer
        .as_object_mut()
        .expect("ownership transfer object")
        .remove("to_invocation_id");
    fs::write(
        &transfer_path,
        format!(
            "{}\n",
            serde_json::to_string(&transfer).expect("serialize ownership transfer")
        ),
    )
    .expect("release zero-effect worktree ownership");
    if !preserve_transfer {
        fs::remove_file(&transfer_path).expect("remove optional ownership transfer");
    }
    fs::remove_dir_all(&worktree.path).expect("make worktree registration prunable");

    fs::write(fixture.repo.join("base-advance.txt"), "advanced\n").expect("write base advance");
    git(&fixture.repo, &["add", "base-advance.txt"]);
    git(&fixture.repo, &["commit", "-m", "feat: advance base"]);
    git(&fixture.repo, &["push", "origin", "main"]);
    let advanced = resolve_base(&fixture.repo, &BTreeMap::new()).expect("resolve advanced base");
    (fixture, scope, worktree, advanced)
}

pub(super) fn session_record_ids(state_root: &Path) -> Vec<String> {
    let mut ids = Vec::new();
    for environment in fs::read_dir(state_root).expect("read runtime state root") {
        let sessions = environment
            .expect("environment entry")
            .path()
            .join("sessions");
        let Ok(entries) = fs::read_dir(sessions) else {
            continue;
        };
        for entry in entries {
            let path = entry.expect("session entry").path();
            if path.extension().and_then(|value| value.to_str()) == Some("json") {
                let value: serde_json::Value =
                    serde_json::from_str(&fs::read_to_string(path).expect("read session record"))
                        .expect("parse session record");
                ids.push(
                    value["session_id"]
                        .as_str()
                        .expect("session ID")
                        .to_string(),
                );
            }
        }
    }
    ids
}

pub(super) fn reviewer_request(
    state: &PersistedInvocation,
    state_path: PathBuf,
) -> ExecutorBridgeRequest {
    ExecutorBridgeRequest {
        repository: state.identity.repository.clone(),
        repository_path: state.identity.repository_path.clone(),
        issue: state.identity.issue,
        issue_title: "Review configured harness".to_string(),
        issue_body: "## Goal\n\nReview the configured harness.".to_string(),
        serialization_reasons: Vec::new(),
        worker_id: state.identity.worker_id.clone(),
        claim_id: state.identity.claim_id.clone(),
        invocation_id: state.identity.invocation_id.clone(),
        event_log: state_path.with_extension("events.jsonl"),
        state_path,
    }
}

pub(crate) fn persisted_invocation() -> PersistedInvocation {
    PersistedInvocation {
        schema: 1,
        identity: BridgeIdentity {
            repository: "owner/repo".to_string(),
            repository_path: PathBuf::from("/safe/repo"),
            issue: 42,
            worker_id: "worker-1".to_string(),
            branch: "feat/autonomous-issue-42".to_string(),
            claim_id: "claim-42".to_string(),
            invocation_id: "invocation-42".to_string(),
            base_ref: "refs/remotes/origin/main".to_string(),
            base_oid: "a".repeat(40),
            worktree: PathBuf::from("/safe/worktree"),
            runtime_environment_dir: Some(PathBuf::from("/safe/runtime")),
            runtime_session_id: Some("runtime-42".to_string()),
        },
        harness: HarnessKind::Codex,
        phase: BridgePhase::Implementing,
        supervisor: Some(ProcessIdentity {
            pid: 122,
            process_group: 122,
            executable: PathBuf::from("/usr/bin/autospec"),
            argv_digest: "c".repeat(64),
            boot_id: "boot-1".to_string(),
            start_identity: "455".to_string(),
        }),
        process: Some(ProcessIdentity {
            pid: 123,
            process_group: 123,
            executable: PathBuf::from("/usr/bin/codex"),
            argv_digest: "b".repeat(64),
            boot_id: "boot-1".to_string(),
            start_identity: "456".to_string(),
        }),
        progress_at: 1_722_000_000,
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

pub(super) fn supervision_state(fixture: &GitFixture) -> PersistedInvocation {
    let mut state = persisted_invocation();
    state.identity.repository_path = fixture.repo.canonicalize().expect("canonical repo");
    state.identity.worktree = state.identity.repository_path.clone();
    state.identity.branch = "feat/autonomous-issue-42".to_string();
    state.identity.base_ref = "origin/main".to_string();
    state.identity.base_oid = git_stdout(&fixture.repo, &["rev-parse", "origin/main"]);
    state.supervisor = None;
    state.process = None;
    state
}

pub(crate) fn shell_invocation(directory: &Path, script: &str) -> HarnessInvocation {
    HarnessInvocation {
        program: PathBuf::from("/bin/sh"),
        supervised_executable: PathBuf::from("/bin/sh"),
        args: vec!["-c".to_string(), script.to_string()],
        current_dir: directory.to_path_buf(),
        requires_mutation_snapshots: false,
    }
}

pub(crate) fn supervision_config(stall_millis: u64) -> SupervisionConfig {
    SupervisionConfig {
        stall_timeout: Duration::from_millis(stall_millis),
        poll_interval: Duration::from_millis(10),
    }
}

#[cfg(target_os = "linux")]
pub(super) fn detach_harness_for_adoption(
    _fixture: &GitFixture,
    state_path: &Path,
    state: &mut PersistedInvocation,
    script: &str,
) -> bridge::ValidatedInvocation {
    let invocation = shell_invocation(&state.identity.worktree, script);
    let validated = bridge::validate_invocation(
        &HarnessInvocation {
            program: invocation.program.canonicalize().expect("canonical shell"),
            supervised_executable: invocation.program.canonicalize().expect("canonical shell"),
            args: invocation.args,
            current_dir: invocation
                .current_dir
                .canonicalize()
                .expect("canonical fixture repo"),
            requires_mutation_snapshots: false,
        },
        &state.identity.worktree,
    )
    .expect("validate adoptable harness");
    let sinks = bridge::output_sink_paths(state_path, &state.identity.invocation_id)
        .expect("adoption output paths");
    let mut child =
        bridge::spawn_blocked_harness(&validated, &sinks, None).expect("spawn adoptable harness");
    let supervisor_birth = child.supervisor_birth().clone();
    let harness_birth = child.birth().clone();
    state.phase = BridgePhase::Implementing;
    state.supervisor = Some(ProcessIdentity {
        pid: supervisor_birth.pid,
        process_group: supervisor_birth.process_group,
        executable: std::env::current_exe().expect("test executable"),
        argv_digest: bridge::argv_digest(&std::env::args().skip(1).collect::<Vec<_>>()),
        boot_id: supervisor_birth.boot_id,
        start_identity: supervisor_birth.start_identity,
    });
    state.process = Some(ProcessIdentity {
        pid: harness_birth.pid,
        process_group: harness_birth.process_group,
        executable: validated.program.clone(),
        argv_digest: bridge::argv_digest(&validated.args),
        boot_id: harness_birth.boot_id,
        start_identity: harness_birth.start_identity,
    });
    state.progress_at = bridge::unix_now().expect("progress time");
    bridge::write_invocation_atomic(state_path, state).expect("persist adoptable identities");
    child
        .release_launch_barrier()
        .expect("release adoptable harness");
    drop(child);
    validated
}

#[cfg(target_os = "linux")]
pub(super) struct NonDescendantDirectFixture {
    pub(super) _supervisor_cleanup: DetachedForkedCleanup,
    pub(super) _supervisor_child: std::process::Child,
    pub(super) _harness_cleanup: DetachedForkedCleanup,
    pub(super) _harness_child: std::process::Child,
    pub(super) _fixture: GitFixture,
    pub(super) paths: bridge::DirectAttemptPaths,
    pub(super) intent: String,
    pub(super) attempt_id: String,
    pub(super) supervisor: ProcessIdentity,
    pub(super) harness: ProcessIdentity,
}

#[cfg(target_os = "linux")]
impl NonDescendantDirectFixture {
    pub(super) fn new(label: &str) -> Self {
        let fixture = GitFixture::new(label);
        let artifact_root = fixture.root.join("evidence");
        fs::create_dir_all(&artifact_root).expect("artifact root");
        fs::set_permissions(&artifact_root, fs::Permissions::from_mode(0o700))
            .expect("private artifact root");
        let paths = bridge::direct_attempt_paths(&artifact_root, 0);
        let args = vec!["30".to_string()];
        let expected_executable = fs::canonicalize("/bin/sleep").expect("canonical sleep fixture");
        let spawn_identity = || {
            let child = Command::new(&expected_executable)
                .arg("30")
                .process_group(0)
                .spawn()
                .expect("spawn isolated sleep fixture");
            let mut cleanup =
                DetachedForkedCleanup::new(child.id()).expect("arm sleep fixture cleanup");
            let deadline = Instant::now() + Duration::from_secs(2);
            let identity = loop {
                if let Some(identity) =
                    bridge::observe_process_identity(child.id(), &bridge::argv_digest(&args))
                        .expect("observe sleep fixture")
                {
                    if identity.executable == expected_executable
                        && identity.argv_digest == bridge::argv_digest(&args)
                        && identity.process_group == identity.pid
                    {
                        break identity;
                    }
                }
                assert!(
                    Instant::now() < deadline,
                    "sleep fixture never reached exact isolated identity"
                );
                std::thread::sleep(Duration::from_millis(1));
            };
            cleanup.confirm_identity(identity.clone());
            (child, cleanup, identity)
        };
        let (supervisor_child, supervisor_cleanup, supervisor) = spawn_identity();
        let (harness_child, harness_cleanup, harness) = spawn_identity();
        let attempt_id = bridge::reserve_direct_attempt_id(&paths).expect("attempt id");
        let commit = bridge::git_stdout(&fixture.repo, &["rev-parse", "--verify", "HEAD^{commit}"])
            .expect("fixture commit");
        let argv = vec![expected_executable.display().to_string(), "30".to_string()];
        let intent =
            bridge::direct_intent_document(&attempt_id, &commit, None, &expected_executable, &argv);
        bridge::write_private_create_once(
            &paths.intent,
            intent.as_bytes(),
            "non-descendant intent",
        )
        .expect("intent");
        let launch = bridge::direct_launch_document(
            &attempt_id,
            &autospec_core::autonomous::waterfall::sha256_hex(intent.as_bytes()),
            &supervisor,
            Some(&harness),
        );
        bridge::write_private_create_once(
            &paths.launch,
            launch.as_bytes(),
            "non-descendant launch",
        )
        .expect("launch");
        Self {
            _supervisor_cleanup: supervisor_cleanup,
            _supervisor_child: supervisor_child,
            _harness_cleanup: harness_cleanup,
            _harness_child: harness_child,
            _fixture: fixture,
            paths,
            intent,
            attempt_id,
            supervisor,
            harness,
        }
    }

    pub(super) fn marker_path(&self) -> PathBuf {
        bridge::direct_ownership_disproven_marker(&self.paths, &self.attempt_id)
            .expect("canonical quarantine marker path")
    }

    pub(super) fn assert_anchor_liveness(&self, supervisor_live: bool, harness_live: bool) {
        assert_eq!(
            bridge::cleanup_instance_is_live(&self.supervisor)
                .expect("supervisor fixture liveness"),
            supervisor_live,
            "unexpected supervisor liveness"
        );
        assert_eq!(
            bridge::cleanup_instance_is_live(&self.harness).expect("harness fixture liveness"),
            harness_live,
            "unexpected harness liveness"
        );
    }

    pub(super) fn exact_marker(&self) -> String {
        bridge::direct_ownership_disproven_document(
            &self.attempt_id,
            &autospec_core::autonomous::waterfall::sha256_hex(self.intent.as_bytes()),
            &autospec_core::autonomous::waterfall::sha256_hex(
                &fs::read(&self.paths.launch).expect("launch bytes"),
            ),
            &self.supervisor,
            &self.harness,
        )
    }

    pub(super) fn replace_marker(&self, body: &str) {
        let path = self.marker_path();
        if path.exists() {
            fs::remove_file(&path).expect("replace quarantine marker");
        }
        bridge::write_private_create_once(
            &path,
            body.as_bytes(),
            "corrupted ownership-disproven quarantine",
        )
        .expect("write quarantine marker");
    }
}

pub(crate) fn implementation_proof_fixture(
    label: &str,
) -> (GitFixture, PersistedInvocation, MutationSnapshot, PathBuf) {
    let fixture = GitFixture::new(label);
    #[cfg(target_os = "macos")]
    {
        let hooks = fixture.root.join("trusted-hooks");
        fs::create_dir(&hooks).expect("create deterministic Darwin hook directory");
        git(
            &fixture.repo,
            &[
                "config",
                "core.hooksPath",
                hooks.to_str().expect("hook path"),
            ],
        );
    }
    let worktree = fixture.root.join("issue-worktree");
    git(
        &fixture.repo,
        &[
            "worktree",
            "add",
            "-b",
            "feat/autonomous-issue-42",
            worktree.to_str().expect("worktree path"),
            "origin/main",
        ],
    );
    let mut state = supervision_state(&fixture);
    state.identity.worktree = worktree.canonicalize().expect("canonical worktree");
    let common_dir = PathBuf::from(git_stdout(
        &state.identity.worktree,
        &["rev-parse", "--git-common-dir"],
    ));
    let common_dir = if common_dir.is_absolute() {
        common_dir
    } else {
        state.identity.worktree.join(common_dir)
    };
    fs::create_dir_all(common_dir.join("hooks")).expect("create fixture Git hook directory");
    state.phase = BridgePhase::ImplementationComplete;
    let snapshot =
        MutationSnapshot::capture(&fixture.repo, &state.identity.branch).expect("snapshot");
    let closeout = state
        .identity
        .worktree
        .join(".autospec/executor-closeout.md");
    fs::create_dir_all(closeout.parent().expect("closeout parent")).expect("closeout directory");
    fs::write(
        &closeout,
        "## Closeout report\n\n\
         Result: shipped\n\
         Claims: [verified] static behavior is covered\n\
         Proof type: static\n\
         Before/after: 0 to 1\n\
         Artifacts: README.md; `git diff origin/main...HEAD`\n\
         Scoped git status: README.md\n\
         One likely hidden failure: none observed\n",
    )
    .expect("write closeout");
    fs::set_permissions(&closeout, fs::Permissions::from_mode(0o600)).expect("private closeout");
    (fixture, state, snapshot, closeout)
}

pub(super) fn commit_implementation(state: &PersistedInvocation) {
    fs::write(
        state.identity.worktree.join("implementation.txt"),
        "implemented\n",
    )
    .expect("write implementation");
    git(&state.identity.worktree, &["add", "."]);
    git(
        &state.identity.worktree,
        &["commit", "-m", "feat: implement fixture"],
    );
}
