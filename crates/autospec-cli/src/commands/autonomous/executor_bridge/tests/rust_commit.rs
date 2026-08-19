// executor_bridge tests: rust / commit — 11 cases.
//
// Split out of tests.rs; see the note in that file.

use super::super::{
    supervise_harness, BridgePhase, MutationSnapshot, PersistedInvocation, SupervisionOutcome,
};
use super::support_base::{
    git, git_stdout, test_environment, DirectCrashFixtureCleanup, GitFixture,
};
use super::support_invocation::{
    implementation_proof_fixture, shell_invocation, supervision_config, supervision_state,
};
use crate::commands::autonomous::executor_bridge as bridge;
use std::fs::{self, OpenOptions};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_direct_crash_guard_cleans_before_launch_journal_exists() {
    // Break caught: a panic before the launch journal is durable orphaning the helper's
    // already-spawned descendant because cleanup knows only the std::process::Child.
    let fixture = GitFixture::new("direct-crash-guard-early-drop");
    let descendant_marker = fixture.root.join("descendant.pid");
    let parent = Command::new("/bin/sh")
        .args([
            "-c",
            &format!(
                "sleep 30 & printf '%s' \"$!\" > '{}'; wait",
                descendant_marker.display()
            ),
        ])
        .spawn()
        .expect("spawn crash conductor fixture");
    let parent_pid = parent.id();
    let guard = DirectCrashFixtureCleanup::new(parent, fixture.root.join("missing-launch.json"));
    let deadline = Instant::now() + Duration::from_secs(2);
    while !descendant_marker.is_file() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
    let descendant_pid = fs::read_to_string(&descendant_marker)
        .expect("descendant marker")
        .parse::<u32>()
        .expect("descendant PID");
    let parent_birth = bridge::observe_process_birth(parent_pid)
        .expect("observe crash conductor")
        .expect("live crash conductor");
    let descendant_birth = bridge::observe_process_birth(descendant_pid)
        .expect("observe crash descendant")
        .expect("live crash descendant");

    drop(guard);

    for birth in [parent_birth, descendant_birth] {
        match bridge::OwnedProcess::capture(&birth) {
            Ok(process) => assert!(
                !process.is_live().expect("post-drop pidfd liveness"),
                "early-drop guard left an owned process pidfd-live"
            ),
            Err(_) => assert!(
                bridge::observe_process_birth(birth.pid)
                    .expect("post-drop birth observation")
                    .as_ref()
                    != Some(&birth),
                "early-drop guard left an exact owned birth observable"
            ),
        }
    }
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_ring_sync_failure_never_advances_durable_cursor() {
    let environment = test_environment();
    let fixture = GitFixture::new("ring-sync-order");
    let mut state = supervision_state(&fixture);
    let state_path = fixture.root.join("state/invocation.json");
    let snapshot =
        MutationSnapshot::capture(&fixture.repo, &state.identity.branch).expect("snapshot");
    environment.launch(bridge::LaunchFailpoint::RingBeforeSync);
    let error = supervise_harness(
        &state_path,
        &fixture.root.join("log/executor.jsonl"),
        &mut state,
        &shell_invocation(&fixture.repo, "printf 'not-durable\\n'; sleep 30"),
        &snapshot,
        supervision_config(500),
    )
    .expect_err("ring sync boundary failure");
    environment.launch(bridge::LaunchFailpoint::None);
    assert!(error.contains("supervisor exited"), "{error}");
    let sinks =
        bridge::output_sink_paths(&state_path, &state.identity.invocation_id).expect("sinks");
    let cursor = OpenOptions::new()
        .read(true)
        .open(sinks.stdout_writer_cursor)
        .expect("writer cursor");
    assert_eq!(
        bridge::read_output_cursor(&cursor)
            .expect("durable writer cursor")
            .total,
        0,
        "cursor committed bytes after injected ring sync failure"
    );
}

#[test]
fn autonomous_executor_bridge_launches_once_and_streams_bounded_progress() {
    let _environment = test_environment();
    let fixture = GitFixture::new("supervise-progress");
    let mut state = supervision_state(&fixture);
    let snapshot =
        MutationSnapshot::capture(&fixture.repo, &state.identity.branch).expect("snapshot");
    let state_path = fixture.root.join("state/invocation.json");
    let event_log = fixture.root.join("log/executor.jsonl");
    let launches = fixture.root.join("launches");
    let script = format!(
        "sleep 0.1; printf 'launch\\n' >> '{}'; printf 'first progress\\n'; printf 'second progress\\n' >&2",
        launches.display()
    );

    let outcome = supervise_harness(
        &state_path,
        &event_log,
        &mut state,
        &shell_invocation(&fixture.repo, &script),
        &snapshot,
        supervision_config(2_000),
    )
    .expect("supervise child");

    assert_eq!(outcome, SupervisionOutcome::Exited { exit_code: 0 });
    assert_eq!(state.phase, BridgePhase::ImplementationComplete);
    assert!(state.process.is_none());
    assert_eq!(
        fs::read_to_string(launches).expect("launch count"),
        "launch\n"
    );
    let events = fs::read_to_string(event_log).expect("executor events");
    assert!(events.contains("\"event\":\"child_started\""));
    assert!(events.contains("\"event\":\"child_output\""));
    assert!(events.contains("first progress"));
    assert!(events.contains("second progress"));
    assert!(events.contains("\"event\":\"child_exited\""));
    let recovered = PersistedInvocation::from_json(
        &fs::read_to_string(state_path).expect("persisted invocation"),
    )
    .expect("strict invocation");
    assert_eq!(recovered.phase, BridgePhase::ImplementationComplete);
}

#[test]
fn autonomous_executor_bridge_waits_for_delayed_descendant_stderr_before_success() {
    let _environment = test_environment();
    // Break caught: publishing terminal success after the leader exits but before an inherited
    // stderr writer emits and closes its durable tail.
    let fixture = GitFixture::new("supervise-delayed-tail");
    let mut state = supervision_state(&fixture);
    let snapshot =
        MutationSnapshot::capture(&fixture.repo, &state.identity.branch).expect("snapshot");
    let state_path = fixture.root.join("state/invocation.json");
    let event_log = fixture.root.join("log/executor.jsonl");
    let script = "(sleep 0.2; printf 'delayed-descendant-stderr-tail\\n' >&2) & exit 0".to_string();

    let outcome = supervise_harness(
        &state_path,
        &event_log,
        &mut state,
        &shell_invocation(&fixture.repo, &script),
        &snapshot,
        supervision_config(2_000),
    )
    .expect("supervise delayed stderr tail");
    let events = fs::read_to_string(event_log).expect("delayed tail events");

    assert_eq!(outcome, SupervisionOutcome::Exited { exit_code: 0 });
    assert!(
        events.contains("delayed-descendant-stderr-tail"),
        "{events}"
    );
    assert!(events.contains("\"event\":\"child_exited\""), "{events}");
}

#[test]
fn autonomous_executor_bridge_retries_interrupted_hup_read_before_closing_tail() {
    let environment = test_environment();
    let fixture = GitFixture::new("supervise-eintr-hup-tail");
    let mut state = supervision_state(&fixture);
    let snapshot =
        MutationSnapshot::capture(&fixture.repo, &state.identity.branch).expect("snapshot");
    let state_path = fixture.root.join("state/invocation.json");
    let event_log = fixture.root.join("log/executor.jsonl");

    environment.launch(bridge::LaunchFailpoint::RingReadInterrupted);
    let outcome = supervise_harness(
        &state_path,
        &event_log,
        &mut state,
        &shell_invocation(
            &fixture.repo,
            "printf 'eintr-hup-buffered-tail\\n' >&2; exit 0",
        ),
        &snapshot,
        supervision_config(2_000),
    );
    environment.launch(bridge::LaunchFailpoint::None);
    let outcome = outcome.expect("EINTR must retry the buffered HUP tail");
    let events = fs::read_to_string(event_log).expect("EINTR tail events");

    assert_eq!(outcome, SupervisionOutcome::Exited { exit_code: 0 });
    assert!(events.contains("eintr-hup-buffered-tail"), "{events}");
}

#[test]
fn autonomous_executor_bridge_rejects_unchanged_head_before_remote_mutation() {
    // Break caught: a zero-change harness exit being pushed and opened as a draft PR.
    let fixture = GitFixture::new("proof-unchanged-head");
    git(
        &fixture.repo,
        &["checkout", "-b", "feat/autonomous-issue-42"],
    );
    let mut state = supervision_state(&fixture);
    state.phase = BridgePhase::ImplementationComplete;
    let snapshot =
        MutationSnapshot::capture(&fixture.repo, &state.identity.branch).expect("snapshot");
    let closeout = fixture.root.join("closeout.md");
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
    let remote_before = git_stdout(
        &fixture.root,
        &[
            "--git-dir",
            fixture
                .root
                .join("remote.git")
                .to_str()
                .expect("remote path"),
            "show-ref",
        ],
    );

    let error = bridge::prove_implementation(
        &fixture.root.join("state/invocation.json"),
        &mut state,
        &snapshot,
        &closeout,
    )
    .expect_err("unchanged HEAD must fail closed");

    assert!(error.contains("HEAD"), "{error}");
    assert_eq!(state.phase, BridgePhase::ImplementationComplete);
    assert_eq!(
        git_stdout(
            &fixture.root,
            &[
                "--git-dir",
                fixture
                    .root
                    .join("remote.git")
                    .to_str()
                    .expect("remote path"),
                "show-ref",
            ],
        ),
        remote_before,
        "proof failure mutated the bare remote"
    );
}

#[test]
fn rust_commits_sandboxed_executor_diff_before_proof() {
    let (fixture, state, _snapshot, closeout) =
        implementation_proof_fixture("rust-commit-sandboxed-diff");
    let remote_before = git_stdout(
        &fixture.root,
        &[
            "--git-dir",
            fixture.root.join("remote.git").to_str().unwrap(),
            "show-ref",
        ],
    );
    let base = git_stdout(&state.identity.worktree, &["rev-parse", "HEAD"]);
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
        .expect("ignore private closeout directory");
    fs::write(
        state.identity.worktree.join("implementation.txt"),
        "implemented by sandboxed harness\n",
    )
    .expect("write sandboxed diff");

    assert!(
        bridge::commit_sandboxed_executor_diff(&state, "test: add behavior coverage", "")
            .expect("Rust-owned executor commit")
    );

    let head = git_stdout(&state.identity.worktree, &["rev-parse", "HEAD"]);
    assert_ne!(head, base);
    bridge::verify_proven_local_state(
        &state,
        &bridge::ImplementationProof {
            head_oid: head.clone(),
            closeout_body: fs::read_to_string(closeout).expect("closeout body"),
        },
    )
    .expect("internal closeout must not block proven implementation");
    assert_eq!(
        git_stdout(&state.identity.worktree, &["log", "-1", "--format=%s"]),
        "test: add behavior coverage"
    );
    assert_eq!(
        git_stdout(
            &state.identity.worktree,
            &["diff-tree", "--no-commit-id", "--name-only", "-r", "HEAD"]
        ),
        "implementation.txt"
    );
    assert_eq!(
        String::from_utf8(
            bridge::sandboxed_executor_diff(&state).expect("implementation-only status")
        )
        .expect("utf8 status"),
        ""
    );
    assert!(
        !bridge::commit_sandboxed_executor_diff(&state, "test: duplicate", "")
            .expect("clean no-op")
    );
    assert_eq!(
        git_stdout(
            &fixture.root,
            &[
                "--git-dir",
                fixture.root.join("remote.git").to_str().unwrap(),
                "show-ref",
            ]
        ),
        remote_before
    );
}

#[test]
fn rust_commit_treats_model_inventory_as_literal_paths() {
    let (_fixture, state, _snapshot, closeout) =
        implementation_proof_fixture("rust-commit-literal-paths");
    git(
        &state.identity.worktree,
        &["add", "-f", ".autospec/executor-closeout.md"],
    );
    git(
        &state.identity.worktree,
        &["commit", "-m", "test: track private closeout fixture"],
    );
    fs::write(&closeout, "model-controlled internal rewrite\n")
        .expect("modify tracked internal closeout");
    let magic = ":(exclude)normal.txt";
    fs::write(
        state.identity.worktree.join(magic),
        "literal implementation path\n",
    )
    .expect("write magic-looking implementation path");

    assert!(
        bridge::commit_sandboxed_executor_diff(&state, "test: preserve literal path", "")
            .expect("Rust-owned literal-path commit")
    );

    assert_eq!(
        git_stdout(
            &state.identity.worktree,
            &["diff-tree", "--no-commit-id", "--name-only", "-r", "HEAD"]
        ),
        magic,
        "model-controlled pathspec syntax must not stage excluded internal artifacts"
    );
    assert_eq!(
        git_stdout(
            &state.identity.worktree,
            &[
                "diff",
                "--name-only",
                "HEAD",
                "--",
                ".autospec/executor-closeout.md",
            ]
        ),
        ".autospec/executor-closeout.md",
        "the internal closeout rewrite must remain outside the Rust-owned commit"
    );
}

#[test]
fn rust_commit_rejects_model_writable_hooks_without_executing_them() {
    let (fixture, state, _snapshot, _closeout) =
        implementation_proof_fixture("rust-commit-model-hooks");
    fs::write(
        state.identity.worktree.join("implementation.txt"),
        "recoverable diff\n",
    )
    .expect("write sandboxed diff");
    let hooks = state.identity.worktree.join(".githooks");
    fs::create_dir_all(&hooks).expect("hooks directory");
    let marker = fixture.root.join("hook-escaped-sandbox");
    let hook = hooks.join("pre-commit");
    fs::write(
        &hook,
        format!("#!/bin/sh\nprintf compromised > {}\n", marker.display()),
    )
    .expect("write model-controlled hook");
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755))
        .expect("make model-controlled hook executable");
    git(&fixture.repo, &["config", "core.hooksPath", ".githooks"]);

    let error = bridge::commit_sandboxed_executor_diff(&state, "test: blocked hook escape", "")
        .expect_err("model-writable hooks must fail closed");

    assert!(error.contains("hook"), "{error}");
    assert!(
        !marker.exists(),
        "model-controlled hook escaped its sandbox"
    );
    assert!(state.identity.worktree.join("implementation.txt").exists());
}

#[cfg(unix)]
#[test]
fn rust_commit_rejects_registered_hook_symlink_into_worktree() {
    let (fixture, state, _snapshot, _closeout) =
        implementation_proof_fixture("rust-commit-hook-symlink");
    fs::write(
        state.identity.worktree.join("implementation.txt"),
        "recoverable diff\n",
    )
    .expect("write sandboxed diff");
    let model_hook = state.identity.worktree.join(".githooks/post-index-change");
    fs::create_dir_all(model_hook.parent().expect("model hook parent"))
        .expect("model hook directory");
    let marker = fixture.root.join("hook-symlink-escaped-sandbox");
    fs::write(
        &model_hook,
        format!("#!/bin/sh\nprintf compromised > {}\n", marker.display()),
    )
    .expect("write model-controlled hook");
    fs::set_permissions(&model_hook, fs::Permissions::from_mode(0o755))
        .expect("make model-controlled hook executable");
    let common_dir = PathBuf::from(git_stdout(
        &state.identity.worktree,
        &["rev-parse", "--git-common-dir"],
    ));
    let common_dir = if common_dir.is_absolute() {
        common_dir
    } else {
        state.identity.worktree.join(common_dir)
    };
    std::os::unix::fs::symlink(&model_hook, common_dir.join("hooks/post-index-change"))
        .expect("hook symlink");

    let error = bridge::commit_sandboxed_executor_diff(&state, "test: blocked hook symlink", "")
        .expect_err("hook symlinks into the model worktree must fail closed");

    assert!(error.contains("hook"), "{error}");
    assert!(
        !marker.exists(),
        "symlinked model-controlled hook escaped its sandbox"
    );
}

#[cfg(unix)]
#[test]
fn rust_commit_rejects_model_selected_clean_filter_without_executing_it() {
    let (fixture, state, _snapshot, _closeout) =
        implementation_proof_fixture("rust-commit-clean-filter");
    fs::write(
        state.identity.worktree.join("implementation.txt"),
        "recoverable diff\n",
    )
    .expect("write sandboxed diff");
    fs::write(
        state.identity.worktree.join(".gitattributes"),
        "implementation.txt filter=escape\n",
    )
    .expect("write model-controlled attributes");
    let marker = fixture.root.join("filter-escaped-sandbox");
    let filter = state.identity.worktree.join("filter.sh");
    fs::write(
        &filter,
        format!(
            "#!/bin/sh\nprintf compromised > {}\ncat\n",
            marker.display()
        ),
    )
    .expect("write model-controlled filter");
    fs::set_permissions(&filter, fs::Permissions::from_mode(0o755))
        .expect("make model-controlled filter executable");
    git(
        &fixture.repo,
        &[
            "config",
            "filter.escape.clean",
            filter.to_str().expect("filter path"),
        ],
    );

    let error = bridge::commit_sandboxed_executor_diff(&state, "test: blocked filter escape", "")
        .expect_err("external clean filters must fail closed");

    assert!(error.contains("filter"), "{error}");
    assert!(!marker.exists(), "clean filter escaped its sandbox");
}
