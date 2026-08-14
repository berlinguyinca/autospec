use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

static FIXTURE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
}

/// Reports whether Rust source `text` contains a literal call-site for
/// `pattern` outside of `//` line comments. Raw `str::contains` over the
/// whole file body would also match the pattern inside a comment (e.g. a
/// doc note explaining why the legacy invocation was removed), which is
/// not an actual restoration of the forbidden authority. Stripping each
/// line's trailing `//...` comment before scanning keeps the check
/// anchored to real code content.
fn source_invokes(text: &str, pattern: &str) -> bool {
    text.lines()
        .map(|line| line.split("//").next().unwrap_or(""))
        .any(|code| code.contains(pattern))
}

#[test]
fn rust_drain_source_does_not_restore_shell_or_legacy_drain_authority() {
    let source = fs::read_to_string(
        workspace_root().join("crates/autospec-cli/src/commands/autonomous/drain.rs"),
    )
    .expect("read drain command source");

    assert!(
        source.contains("DrainExecutorInput::omx_autospec_run"),
        "drain command must receive typed executor input from autospec-core"
    );
    assert!(
        !source.contains("DRAIN_RUN_PROMPT"),
        "drain command must not retain a raw shell-shaped prompt string"
    );
    for forbidden in [
        "Command::new(\"sh\")",
        "Command::new(\"bash\")",
        "autospec-autonomous-run-drain.sh",
    ] {
        assert!(
            !source_invokes(&source, forbidden),
            "drain command retains legacy authority: {forbidden}"
        );
    }
}

#[test]
fn drain_dispatches_literal_autospec_run_token_without_legacy_script_authority() {
    let fixture = DrainFixture::new();
    let bin = fixture.root.join("bin");
    let args_file = fixture.root.join("omx-args");
    let legacy_marker = fixture.root.join("legacy-drain-invoked");
    fs::create_dir_all(&bin).expect("create fake bin");
    write_executable(
        &bin.join("omx"),
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$AUTOSPEC_TEST_OMX_ARGS\"\n",
    );
    write_executable(
        &bin.join("autospec-autonomous-run-drain.sh"),
        "#!/bin/sh\nprintf legacy > \"$AUTOSPEC_TEST_LEGACY_MARKER\"\nexit 9\n",
    );
    write_executable(&bin.join("gh"), "#!/bin/sh\nprintf '[]\\n'\n");

    let output = fixture
        .command(&bin)
        .env("AUTOSPEC_TEST_OMX_ARGS", &args_file)
        .env("AUTOSPEC_TEST_LEGACY_MARKER", &legacy_marker)
        .output()
        .expect("run drain with typed executor input");

    assert!(output.status.success(), "stderr={}", stderr(&output));
    let args = fs::read_to_string(&args_file).expect("read captured omx args");
    assert_eq!(
        args.lines().collect::<Vec<_>>(),
        [
            "exec",
            "--cd",
            fixture.repo_dir.to_str().expect("repo dir"),
            "--dangerously-bypass-approvals-and-sandbox",
            "$autospec-run"
        ]
    );
    assert!(
        !legacy_marker.exists(),
        "native drain must not invoke the historical drain script"
    );
}

#[test]
fn malformed_repo_is_rejected_before_state_or_child_creation() {
    let fixture = DrainFixture::new();
    let bin = fixture.root.join("bin");
    let launched = fixture.root.join("launched");
    fs::create_dir_all(&bin).expect("create fake bin");
    write_executable(
        &bin.join("omx"),
        "#!/bin/sh\nprintf launched > \"$AUTOSPEC_TEST_DRAIN_LAUNCHED\"\n",
    );

    let output = fixture
        .command(&bin)
        .args(["--repo", "malformed"])
        .env("AUTOSPEC_TEST_DRAIN_LAUNCHED", &launched)
        .output()
        .expect("run drain with malformed repo");

    assert_eq!(output.status.code(), Some(2));
    assert!(!launched.exists());
    assert!(!fixture.operator_root.exists());
}

#[test]
fn missing_origin_is_rejected_before_state_or_child_creation() {
    let fixture = DrainFixture::new();
    let bin = fixture.root.join("bin");
    let launched = fixture.root.join("launched");
    fs::create_dir_all(&bin).expect("create fake bin");
    write_executable(
        &bin.join("omx"),
        "#!/bin/sh\nprintf launched > \"$AUTOSPEC_TEST_DRAIN_LAUNCHED\"\n",
    );
    let removed = Command::new("git")
        .args([
            "-C",
            fixture.repo_dir.to_str().expect("repo dir"),
            "remote",
            "remove",
            "origin",
        ])
        .output()
        .expect("remove origin");
    assert!(removed.status.success());

    let output = fixture
        .command(&bin)
        .env("AUTOSPEC_TEST_DRAIN_LAUNCHED", &launched)
        .output()
        .expect("reject missing origin");

    assert_eq!(output.status.code(), Some(2), "stderr={}", stderr(&output));
    assert!(!launched.exists());
    assert!(!fixture.operator_root.exists());
}

#[test]
fn unsupported_origin_is_rejected_before_state_or_child_creation() {
    let fixture = DrainFixture::new();
    let bin = fixture.root.join("bin");
    let launched = fixture.root.join("launched");
    fs::create_dir_all(&bin).expect("create fake bin");
    write_executable(
        &bin.join("omx"),
        "#!/bin/sh\nprintf launched > \"$AUTOSPEC_TEST_DRAIN_LAUNCHED\"\n",
    );
    let changed = Command::new("git")
        .args([
            "-C",
            fixture.repo_dir.to_str().expect("repo dir"),
            "remote",
            "set-url",
            "origin",
            "https://example.com/owner/repo.git",
        ])
        .output()
        .expect("replace origin");
    assert!(changed.status.success());

    let output = fixture
        .command(&bin)
        .env("AUTOSPEC_TEST_DRAIN_LAUNCHED", &launched)
        .output()
        .expect("reject unsupported origin");

    assert_eq!(output.status.code(), Some(2), "stderr={}", stderr(&output));
    assert!(!launched.exists());
    assert!(!fixture.operator_root.exists());
}

#[test]
fn outside_env_artifact_path_is_rejected_before_child_creation() {
    let fixture = DrainFixture::new();
    let bin = fixture.root.join("bin");
    let launched = fixture.root.join("launched");
    let outside = fixture.root.join("outside").join("drain.log");
    fs::create_dir_all(&bin).expect("create fake bin");
    write_executable(
        &bin.join("omx"),
        "#!/bin/sh\nprintf launched > \"$AUTOSPEC_TEST_DRAIN_LAUNCHED\"\n",
    );

    let output = fixture
        .command(&bin)
        .env("AUTOSPEC_TEST_DRAIN_LAUNCHED", &launched)
        .env("AUTOSPEC_AUTONOMOUS_DRAIN_LOG", &outside)
        .output()
        .expect("reject outside drain artifact");

    assert_eq!(output.status.code(), Some(2), "stderr={}", stderr(&output));
    assert!(!launched.exists(), "validation must precede child creation");
}

#[test]
fn quiet_child_with_heartbeat_progress_completes_without_termination() {
    let fixture = DrainFixture::new();
    let bin = fixture.root.join("bin");
    fs::create_dir_all(&bin).expect("create fake bin");
    write_executable(
        &bin.join("omx"),
        r#"#!/bin/sh
set -eu
heartbeat_dir="$AUTOSPEC_PROCESS_HEARTBEAT_DIR/o5_owner_r4_repo"
mkdir -p "$heartbeat_dir"
for step in claimed validation merge finalize; do
  printf '{"repo":"owner/repo","issue":42,"branch":"feat/autonomous-issue-42","step":"%s","ts":"2026-07-28T00:00:00Z","worker_id":"test-worker","claim_id":"test-claim"}\n' "$step" > "$heartbeat_dir/42.json"
  sleep 1
  if [ -f "$AUTOSPEC_TEST_DRAIN_OBSERVATION" ] && grep -q '"progress":"heartbeat"' "$AUTOSPEC_TEST_DRAIN_OBSERVATION"; then
    exit 0
  fi
done
exit 1
"#,
    );
    write_executable(&bin.join("gh"), "#!/bin/sh\nprintf '[]\\n'\n");

    let output = fixture
        .command(&bin)
        .args(["--stall-secs", "3", "--poll-secs", "1", "--json"])
        .env(
            "AUTOSPEC_TEST_DRAIN_OBSERVATION",
            fixture.drain_observation_path(),
        )
        .output()
        .expect("run drain with heartbeat progress");

    assert!(output.status.success(), "stderr={}", stderr(&output));
    assert!(stdout(&output).contains("quiet_stdout_external_progress"));
    assert!(fixture.drain_observation_path().exists());
    assert!(fs::read_to_string(fixture.drain_observation_path())
        .expect("read drain observation")
        .contains("heartbeat"));
}

#[test]
fn quiet_child_with_claim_heartbeat_progress_completes_without_termination() {
    let fixture = DrainFixture::new();
    let bin = fixture.root.join("bin");
    fs::create_dir_all(&bin).expect("create fake bin");
    write_executable(
        &bin.join("omx"),
        r#"#!/bin/sh
set -eu
heartbeat_dir="$AUTOSPEC_HEARTBEAT_DIR/o5_owner_r4_repo"
mkdir -p "$heartbeat_dir"
sleep 1
printf '%s\n' '{"repo":"owner/repo","issue":42,"branch":"feat/autonomous-issue-42","step":"claimed","ts":"2026-07-28T00:00:00Z","worker_id":"test-worker","claim_id":"test-claim"}' > "$heartbeat_dir/42.json"
sleep 1
printf '%s\n' '{"repo":"owner/repo","issue":42,"branch":"feat/autonomous-issue-42","step":"validated","ts":"2026-07-28T00:00:00Z","worker_id":"test-worker","claim_id":"test-claim"}' > "$heartbeat_dir/42.json"
sleep 3
"#,
    );
    write_executable(&bin.join("gh"), "#!/bin/sh\nprintf '[]\\n'\n");

    let output = fixture
        .command(&bin)
        .args(["--stall-secs", "5", "--poll-secs", "1", "--json"])
        .env_remove("AUTOSPEC_PROCESS_HEARTBEAT_DIR")
        .env("AUTOSPEC_HEARTBEAT_DIR", &fixture.heartbeat_root)
        .output()
        .expect("run drain with claim heartbeat progress");

    assert!(output.status.success(), "stderr={}", stderr(&output));
    assert!(stdout(&output).contains("quiet_stdout_external_progress"));
}

#[test]
fn claim_heartbeat_wins_when_process_root_conflicts() {
    let fixture = DrainFixture::new();
    let bin = fixture.root.join("bin");
    let claim_root = fixture.root.join("claim-heartbeats");
    fs::create_dir_all(&bin).expect("create fake bin");
    write_executable(
        &bin.join("omx"),
        r#"#!/bin/sh
set -eu
heartbeat_dir="$AUTOSPEC_HEARTBEAT_DIR/o5_owner_r4_repo"
mkdir -p "$heartbeat_dir"
sleep 1
printf '%s\n' '{"repo":"owner/repo","issue":42,"branch":"feat/autonomous-issue-42","step":"claimed","ts":"2026-07-28T00:00:00Z","worker_id":"test-worker","claim_id":"test-claim"}' > "$heartbeat_dir/42.json"
sleep 1
printf '%s\n' '{"repo":"owner/repo","issue":42,"branch":"feat/autonomous-issue-42","step":"validated","ts":"2026-07-28T00:00:00Z","worker_id":"test-worker","claim_id":"test-claim"}' > "$heartbeat_dir/42.json"
sleep 3
"#,
    );
    write_executable(&bin.join("gh"), "#!/bin/sh\nprintf '[]\\n'\n");

    let output = fixture
        .command(&bin)
        .args(["--stall-secs", "3", "--poll-secs", "1", "--json"])
        .env("AUTOSPEC_HEARTBEAT_DIR", &claim_root)
        .output()
        .expect("run drain with conflicting heartbeat roots");

    assert!(output.status.success(), "stderr={}", stderr(&output));
    assert!(stdout(&output).contains("quiet_stdout_external_progress"));
}

#[test]
fn quiet_child_with_watchdog_heartbeat_progress_completes_without_termination() {
    let fixture = DrainFixture::new();
    let bin = fixture.root.join("bin");
    let watchdog_root = fixture.root.join("watchdog");
    fs::create_dir_all(&bin).expect("create fake bin");
    write_executable(
        &bin.join("omx"),
        r#"#!/bin/sh
set -eu
heartbeat_dir="$AUTOSPEC_WATCHDOG_DIR/process-heartbeats/o5_owner_r4_repo"
mkdir -p "$heartbeat_dir"
for step in claimed validated finalized complete; do
  printf '{"repo":"owner/repo","issue":42,"branch":"feat/autonomous-issue-42","step":"%s","ts":"2026-07-28T00:00:00Z","worker_id":"test-worker","claim_id":"test-claim"}\n' "$step" > "$heartbeat_dir/42.json"
  sleep 1
  if [ -f "$AUTOSPEC_TEST_DRAIN_OBSERVATION" ] && grep -q '"progress":"heartbeat"' "$AUTOSPEC_TEST_DRAIN_OBSERVATION"; then
    exit 0
  fi
done
exit 1
"#,
    );
    write_executable(&bin.join("gh"), "#!/bin/sh\nprintf '[]\\n'\n");

    let output = fixture
        .command(&bin)
        .args(["--stall-secs", "3", "--poll-secs", "1", "--json"])
        .env_remove("AUTOSPEC_PROCESS_HEARTBEAT_DIR")
        .env("AUTOSPEC_WATCHDOG_DIR", &watchdog_root)
        .env(
            "AUTOSPEC_TEST_DRAIN_OBSERVATION",
            fixture.drain_observation_path(),
        )
        .output()
        .expect("run drain with watchdog heartbeat progress");

    assert!(output.status.success(), "stderr={}", stderr(&output));
    assert!(stdout(&output).contains("quiet_stdout_external_progress"));
}

#[test]
fn json_drain_keeps_child_output_out_of_structured_stdout() {
    let fixture = DrainFixture::new();
    let bin = fixture.root.join("bin");
    fs::create_dir_all(&bin).expect("create fake bin");
    write_executable(&bin.join("omx"), "#!/bin/sh\nprintf 'agent output\\n'\n");
    write_executable(&bin.join("gh"), "#!/bin/sh\nprintf '[]\\n'\n");

    let output = fixture.run(&bin, &["--json"]);

    assert!(output.status.success(), "stderr={}", stderr(&output));
    assert!(!stdout(&output).contains("agent output"));
    assert!(stderr(&output).contains("agent output"));
    assert!(stdout(&output).contains("\"decision\":\"complete\""));
}

#[test]
fn matching_session_reconciliation_cancels_the_startup_deadline() {
    let fixture = DrainFixture::new();
    let bin = fixture.root.join("bin");
    fs::create_dir_all(&bin).expect("create fake bin");
    write_executable(
        &bin.join("omx"),
        r#"#!/bin/sh
set -eu
printf 'session_start child=child-1850 session_turns=0 issue_claim=none\n'
printf 'session_start_reconciled child=child-1850 session_turns=1 issue_claim=1850\n'
for step in 1 2 3 4; do
  printf 'working step=%s\n' "$step"
  sleep 0.4
done
"#,
    );
    write_executable(&bin.join("gh"), "#!/bin/sh\nprintf '[]\\n'\n");

    let output = fixture.run(&bin, &["--stall-secs", "1", "--poll-secs", "1", "--json"]);

    assert!(output.status.success(), "stderr={}", stderr(&output));
    assert!(stdout(&output).contains("\"event\":\"session_start_reconciled\""));
    assert!(!stdout(&output).contains("session_start_timeout"));
}

#[test]
fn matching_reconciliation_from_the_other_pipe_cancels_an_inverted_delivery() {
    let fixture = DrainFixture::new();
    let bin = fixture.root.join("bin");
    fs::create_dir_all(&bin).expect("create fake bin");
    write_executable(
        &bin.join("omx"),
        r#"#!/bin/sh
set -eu
printf 'session_start_reconciled child=child-cross-pipe-1850 session_turns=1 issue_claim=1850\n' >&2
sleep 0.1
printf 'session_start child=child-cross-pipe-1850 session_turns=0 issue_claim=none\n'
for step in 1 2 3 4; do
  printf 'working step=%s\n' "$step"
  sleep 0.4
done
"#,
    );
    write_executable(&bin.join("gh"), "#!/bin/sh\nprintf '[]\\n'\n");

    let output = fixture.run(&bin, &["--stall-secs", "1", "--poll-secs", "1", "--json"]);

    assert!(output.status.success(), "stderr={}", stderr(&output));
    assert!(stdout(&output).contains("\"event\":\"session_start_reconciled\""));
    assert!(!stdout(&output).contains("session_start_timeout"));
}

#[test]
fn unreconciled_startup_exhausts_bounded_retries_and_kills_every_process_group() {
    let fixture = DrainFixture::new();
    let bin = fixture.root.join("bin");
    let attempts = fixture.root.join("attempts");
    let descendant_pids = fixture.root.join("descendant-pids");
    fs::create_dir_all(&bin).expect("create fake bin");
    write_executable(
        &bin.join("omx"),
        r#"#!/bin/sh
set -eu
attempt=0
if [ -f "$AUTOSPEC_TEST_ATTEMPTS" ]; then
  attempt=$(cat "$AUTOSPEC_TEST_ATTEMPTS")
fi
attempt=$((attempt + 1))
printf '%s\n' "$attempt" > "$AUTOSPEC_TEST_ATTEMPTS"
sh -c 'trap "" TERM; printf "%s\n" "$$" >> "$AUTOSPEC_TEST_DESCENDANT_PIDS"; while :; do sleep 1; done' &
printf 'session_start child=child-1850 session_turns=0 issue_claim=none\n'
trap '' TERM
while :; do
  printf 'startup still active attempt=%s\n' "$attempt"
  sleep 0.1
done
"#,
    );
    write_executable(&bin.join("gh"), "#!/bin/sh\nprintf '[]\\n'\n");

    let output = fixture
        .command(&bin)
        .args(["--stall-secs", "1", "--poll-secs", "1", "--json"])
        .env("AUTOSPEC_TEST_ATTEMPTS", &attempts)
        .env("AUTOSPEC_TEST_DESCENDANT_PIDS", &descendant_pids)
        .output()
        .expect("run drain through startup retry exhaustion");

    assert_eq!(
        output.status.code(),
        Some(124),
        "stderr={}",
        stderr(&output)
    );
    assert_eq!(
        fs::read_to_string(&attempts)
            .expect("read attempt count")
            .trim(),
        "4"
    );
    assert!(stdout(&output).contains("\"event\":\"session_start_timeout\""));
    assert!(stdout(&output).contains("\"retry\":\"scheduled\""));
    assert!(stdout(&output).contains("\"retry\":\"exhausted\""));
    for pid in fs::read_to_string(&descendant_pids)
        .expect("read descendant pids")
        .lines()
    {
        assert_pid_is_gone(pid);
    }
}

#[test]
fn startup_retry_relaunches_the_actual_omx_child_and_then_reconciles() {
    let fixture = DrainFixture::new();
    let bin = fixture.root.join("bin");
    let attempts = fixture.root.join("attempts");
    fs::create_dir_all(&bin).expect("create fake bin");
    write_executable(
        &bin.join("omx"),
        r#"#!/bin/sh
set -eu
attempt=0
if [ -f "$AUTOSPEC_TEST_ATTEMPTS" ]; then
  attempt=$(cat "$AUTOSPEC_TEST_ATTEMPTS")
fi
attempt=$((attempt + 1))
printf '%s\n' "$attempt" > "$AUTOSPEC_TEST_ATTEMPTS"
printf 'session_start child=child-1850 session_turns=0 issue_claim=none\n'
if [ "$attempt" -eq 1 ]; then
  while :; do
    printf 'startup still active\n'
    sleep 0.1
  done
fi
printf 'session_start_reconciled child=child-1850 session_turns=1 issue_claim=1850\n'
"#,
    );
    write_executable(&bin.join("gh"), "#!/bin/sh\nprintf '[]\\n'\n");

    let started = Instant::now();
    let output = fixture
        .command(&bin)
        .args(["--stall-secs", "1", "--poll-secs", "10", "--json"])
        .env("AUTOSPEC_TEST_ATTEMPTS", &attempts)
        .output()
        .expect("run drain through one startup retry");

    assert!(output.status.success(), "stderr={}", stderr(&output));
    assert!(
        started.elapsed().as_secs() < 6,
        "startup deadline waited for the ordinary progress poll"
    );
    assert_eq!(
        fs::read_to_string(attempts)
            .expect("read attempt count")
            .trim(),
        "2"
    );
    assert!(stdout(&output).contains("\"retry\":\"scheduled\""));
    assert!(stdout(&output).contains("\"event\":\"session_start_reconciled\""));
}

#[test]
fn startup_retry_exhaustion_survives_separate_drain_invocations() {
    let fixture = DrainFixture::new();
    let bin = fixture.root.join("bin");
    let attempts = fixture.root.join("attempts");
    let phase = fixture.root.join("phase");
    fs::create_dir_all(&bin).expect("create fake bin");
    fs::write(&phase, "first\n").expect("write first invocation phase");
    write_executable(
        &bin.join("omx"),
        r#"#!/bin/sh
set -eu
attempt=0
if [ -f "$AUTOSPEC_TEST_ATTEMPTS" ]; then
  attempt=$(cat "$AUTOSPEC_TEST_ATTEMPTS")
fi
attempt=$((attempt + 1))
printf '%s\n' "$attempt" > "$AUTOSPEC_TEST_ATTEMPTS"
if [ "$(cat "$AUTOSPEC_TEST_PHASE")" = first ] && [ "$attempt" -eq 2 ]; then
  exit 7
fi
printf 'session_start child=child-persisted-1850 session_turns=0 issue_claim=none\n'
trap '' TERM
while :; do
  printf 'startup still active attempt=%s\n' "$attempt"
  sleep 0.1
done
"#,
    );
    write_executable(&bin.join("gh"), "#!/bin/sh\nprintf '[]\\n'\n");

    let first = fixture
        .command(&bin)
        .args(["--stall-secs", "1", "--poll-secs", "1", "--json"])
        .env("AUTOSPEC_TEST_ATTEMPTS", &attempts)
        .env("AUTOSPEC_TEST_PHASE", &phase)
        .output()
        .expect("run first drain invocation");
    assert_eq!(first.status.code(), Some(7), "stderr={}", stderr(&first));
    assert!(stdout(&first).contains("\"attempt\":1"));

    fs::write(&phase, "second\n").expect("write second invocation phase");
    let second = fixture
        .command(&bin)
        .args(["--stall-secs", "1", "--poll-secs", "1", "--json"])
        .env("AUTOSPEC_TEST_ATTEMPTS", &attempts)
        .env("AUTOSPEC_TEST_PHASE", &phase)
        .output()
        .expect("run second drain invocation");

    assert_eq!(
        second.status.code(),
        Some(124),
        "stderr={}",
        stderr(&second)
    );
    assert!(stdout(&second).contains("\"retry\":\"exhausted\",\"attempt\":4"));
    assert_eq!(
        fs::read_to_string(&attempts)
            .expect("read total process launches")
            .trim(),
        "5",
        "the second command must resume at persisted startup failure 1"
    );

    let third = fixture
        .command(&bin)
        .args(["--stall-secs", "1", "--poll-secs", "1", "--json"])
        .env("AUTOSPEC_TEST_ATTEMPTS", &attempts)
        .env("AUTOSPEC_TEST_PHASE", &phase)
        .output()
        .expect("run after persisted retry exhaustion");
    assert_eq!(third.status.code(), Some(124), "stderr={}", stderr(&third));
    assert_eq!(
        fs::read_to_string(&attempts)
            .expect("read launches after persisted exhaustion")
            .trim(),
        "5",
        "an exhausted restart must not launch another child"
    );
}

#[test]
fn successful_command_clears_recovered_startup_failures_without_session_output() {
    let fixture = DrainFixture::new();
    let bin = fixture.root.join("bin");
    let attempts = fixture.root.join("attempts");
    fs::create_dir_all(&bin).expect("create fake bin");
    write_executable(
        &bin.join("omx"),
        r#"#!/bin/sh
set -eu
attempt=0
if [ -f "$AUTOSPEC_TEST_ATTEMPTS" ]; then
  attempt=$(cat "$AUTOSPEC_TEST_ATTEMPTS")
fi
attempt=$((attempt + 1))
printf '%s\n' "$attempt" > "$AUTOSPEC_TEST_ATTEMPTS"
if [ "$attempt" -gt 1 ]; then
  exit 0
fi
printf 'session_start child=child-reset-1850 session_turns=0 issue_claim=none\n'
while :; do
  printf 'startup still active\n'
  sleep 0.1
done
"#,
    );
    write_executable(&bin.join("gh"), "#!/bin/sh\nprintf '[]\\n'\n");

    let output = fixture
        .command(&bin)
        .args(["--stall-secs", "1", "--poll-secs", "1", "--json"])
        .env("AUTOSPEC_TEST_ATTEMPTS", &attempts)
        .output()
        .expect("run timeout followed by successful child");

    assert!(output.status.success(), "stderr={}", stderr(&output));
    assert!(
        !fixture.startup_retry_path().exists(),
        "a successful command must clear prior startup failures"
    );
}

#[test]
fn session_event_persistence_failure_terminates_the_live_process_group() {
    let fixture = DrainFixture::new();
    let bin = fixture.root.join("bin");
    let leader_pid = fixture.root.join("leader-pid");
    let descendant_pid = fixture.root.join("descendant-pid");
    fs::create_dir_all(&bin).expect("create fake bin");
    fs::create_dir_all(fixture.session_events_path())
        .expect("make the event journal path unwritable as a file");
    write_executable(
        &bin.join("omx"),
        r#"#!/bin/sh
set -eu
printf '%s\n' "$$" > "$AUTOSPEC_TEST_LEADER_PID"
(
  trap '' TERM
  exec sleep 30
) &
printf '%s\n' "$!" > "$AUTOSPEC_TEST_DESCENDANT_PID"
trap '' TERM
printf 'session_start child=child-persist-error-1850 session_turns=0 issue_claim=none\n'
while :; do
  printf 'startup still active\n'
  sleep 0.1
done
"#,
    );
    write_executable(&bin.join("gh"), "#!/bin/sh\nprintf '[]\\n'\n");

    let output = fixture
        .command(&bin)
        .args(["--stall-secs", "30", "--poll-secs", "1", "--json"])
        .env("AUTOSPEC_TEST_LEADER_PID", &leader_pid)
        .env("AUTOSPEC_TEST_DESCENDANT_PID", &descendant_pid)
        .output()
        .expect("run drain with an unwritable event journal");

    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("cannot open"),
        "stderr={}",
        stderr(&output)
    );
    let leader_was_alive = pid_path_is_alive(&leader_pid);
    let descendant_was_alive = pid_path_is_alive(&descendant_pid);
    kill_recorded_process_group(&leader_pid);
    assert!(
        !leader_was_alive && !descendant_was_alive,
        "persistence error leaked process group: leader_alive={leader_was_alive} descendant_alive={descendant_was_alive}"
    );
}

#[test]
fn startup_event_parsing_drains_output_larger_than_pipe_capacity() {
    let fixture = DrainFixture::new();
    let bin = fixture.root.join("bin");
    fs::create_dir_all(&bin).expect("create fake bin");
    write_executable(
        &bin.join("omx"),
        r#"#!/bin/sh
set -eu
printf 'session_start child=child-1850 session_turns=0 issue_claim=none\n'
dd if=/dev/zero bs=65536 count=4 2>/dev/null | tr '\000' x
printf '\nsession_start_reconciled child=child-1850 session_turns=1 issue_claim=1850\n'
"#,
    );
    write_executable(&bin.join("gh"), "#!/bin/sh\nprintf '[]\\n'\n");

    let started = Instant::now();
    let output = fixture.run(&bin, &["--stall-secs", "2", "--poll-secs", "1", "--json"]);

    assert!(output.status.success(), "stderr={}", stderr(&output));
    assert!(
        started.elapsed().as_secs() < 10,
        "drain deadlocked on child pipes"
    );
    assert!(stdout(&output).contains("\"event\":\"session_start_reconciled\""));
}

#[test]
fn quiet_child_with_github_progress_warns_and_completes() {
    let fixture = DrainFixture::new();
    let bin = fixture.root.join("bin");
    let gh_counter = fixture.root.join("gh-counter");
    fs::create_dir_all(&bin).expect("create fake bin");
    write_executable(
        &bin.join("omx"),
        r#"#!/bin/sh
set -eu
for attempt in 1 2 3 4 5 6 7 8; do
  if [ -f "$AUTOSPEC_TEST_DRAIN_OBSERVATION" ] && grep -q '"progress":"github"' "$AUTOSPEC_TEST_DRAIN_OBSERVATION"; then
    exit 0
  fi
  sleep 1
done
exit 1
"#,
    );
    write_executable(
        &bin.join("gh"),
        r#"#!/bin/sh
set -eu
if [ -f "$AUTOSPEC_TEST_GH_COUNTER" ]; then
  printf '[{"number":42,"updatedAt":"2026-07-16T00:00:01Z"}]\n'
else
  : > "$AUTOSPEC_TEST_GH_COUNTER"
  printf '[{"number":42,"updatedAt":"2026-07-16T00:00:00Z"}]\n'
fi
"#,
    );

    let output = fixture
        .command(&bin)
        .args(["--stall-secs", "2", "--poll-secs", "1", "--json"])
        .env("AUTOSPEC_TEST_GH_COUNTER", gh_counter)
        .env(
            "AUTOSPEC_TEST_DRAIN_OBSERVATION",
            fixture.drain_observation_path(),
        )
        .output()
        .expect("run drain with github progress");

    assert!(output.status.success(), "stderr={}", stderr(&output));
    assert!(stdout(&output).contains("quiet_stdout_external_progress"));
    assert!(fs::read_to_string(fixture.drain_observation_path())
        .expect("read drain observation")
        .contains("github"));
}

#[test]
fn recovered_github_baseline_is_not_false_progress() {
    let fixture = DrainFixture::new();
    let bin = fixture.root.join("bin");
    let gh_counter = fixture.root.join("gh-counter");
    fs::create_dir_all(&bin).expect("create fake bin");
    write_executable(&bin.join("omx"), "#!/bin/sh\nexec sleep 30\n");
    write_executable(
        &bin.join("gh"),
        r#"#!/bin/sh
set -eu
counter="$AUTOSPEC_TEST_GH_COUNTER"
count=0
if [ -f "$counter" ]; then
  count=$(cat "$counter")
fi
count=$((count + 1))
printf '%s\n' "$count" > "$counter"
if [ "$count" -eq 1 ]; then
  exit 1
fi
printf '[]\n'
"#,
    );

    let output = fixture
        .command(&bin)
        .args(["--stall-secs", "1", "--poll-secs", "1", "--json"])
        .env("AUTOSPEC_TEST_GH_COUNTER", gh_counter)
        .output()
        .expect("run drain after a failed github baseline");

    assert_eq!(
        output.status.code(),
        Some(124),
        "stderr={}",
        stderr(&output)
    );
    assert!(!stdout(&output).contains("quiet_stdout_external_progress"));
    assert!(stdout(&output).contains("terminate_stalled"));
}

#[test]
fn silent_live_child_is_reaped_before_process_group_liveness_check() {
    let fixture = DrainFixture::new();
    let bin = fixture.root.join("bin");
    let child_pid = fixture.root.join("child-pid");
    fs::create_dir_all(&bin).expect("create fake bin");
    write_executable(
        &bin.join("omx"),
        r#"#!/bin/sh
printf '%s\n' "$$" > "$AUTOSPEC_TEST_CHILD_PID"
exec sleep 30
"#,
    );
    write_executable(&bin.join("gh"), "#!/bin/sh\nprintf '[]\\n'\n");

    let output = fixture
        .command(&bin)
        .args(["--stall-secs", "1", "--poll-secs", "1", "--json"])
        .env("AUTOSPEC_TEST_CHILD_PID", &child_pid)
        .output()
        .expect("run stalled drain");

    assert_eq!(
        output.status.code(),
        Some(124),
        "stderr={}",
        stderr(&output)
    );
    let pid = fs::read_to_string(&child_pid).expect("read stalled child pid");
    assert!(
        !Command::new("kill")
            .args(["-0", pid.trim()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("inspect stalled child")
            .success(),
        "the stalled child must no longer be alive"
    );
    assert!(stdout(&output).contains("terminate_stalled"));
}

#[test]
fn child_exit_while_termination_is_attempted_is_reported_as_complete() {
    let fixture = DrainFixture::new();
    let bin = fixture.root.join("bin");
    fs::create_dir_all(&bin).expect("create fake bin");
    write_executable(&bin.join("omx"), "#!/bin/sh\nexec sleep 2\n");
    write_executable(&bin.join("gh"), "#!/bin/sh\nprintf '[]\\n'\n");
    write_executable(&bin.join("kill"), "#!/bin/sh\nsleep 2\nexit 1\n");

    let output = fixture.run(&bin, &["--stall-secs", "1", "--poll-secs", "1", "--json"]);

    assert!(output.status.success(), "stderr={}", stderr(&output));
    assert!(!stdout(&output).contains("terminate_stalled"));
    assert!(stdout(&output).contains("\"decision\":\"complete\""));
}

#[test]
fn stalled_drain_terminates_the_wrapper_and_its_descendant() {
    let fixture = DrainFixture::new();
    let bin = fixture.root.join("bin");
    let wrapper_pid = fixture.root.join("wrapper-pid");
    let descendant_pid = fixture.root.join("descendant-pid");
    fs::create_dir_all(&bin).expect("create fake bin");
    write_executable(
        &bin.join("omx"),
        r#"#!/bin/sh
printf '%s\n' "$$" > "$AUTOSPEC_TEST_WRAPPER_PID"
sleep 30 &
printf '%s\n' "$!" > "$AUTOSPEC_TEST_DESCENDANT_PID"
wait
"#,
    );
    write_executable(&bin.join("gh"), "#!/bin/sh\nprintf '[]\\n'\n");

    let output = fixture
        .command(&bin)
        .args(["--stall-secs", "1", "--poll-secs", "1", "--json"])
        .env("AUTOSPEC_TEST_WRAPPER_PID", &wrapper_pid)
        .env("AUTOSPEC_TEST_DESCENDANT_PID", &descendant_pid)
        .output()
        .expect("run stalled drain with descendant");

    assert_eq!(
        output.status.code(),
        Some(124),
        "stderr={}",
        stderr(&output)
    );
    assert_process_is_gone(&wrapper_pid);
    assert_process_is_gone(&descendant_pid);
}

#[test]
fn stalled_drain_kills_a_term_ignoring_descendant_before_joining_readers() {
    let fixture = DrainFixture::new();
    let bin = fixture.root.join("bin");
    let descendant_pid = fixture.root.join("descendant-pid");
    fs::create_dir_all(&bin).expect("create fake bin");
    write_executable(
        &bin.join("omx"),
        r#"#!/bin/sh
trap 'exit 0' TERM
(
  trap '' TERM
  exec sleep 20
) &
printf '%s\n' "$!" > "$AUTOSPEC_TEST_DESCENDANT_PID"
wait
"#,
    );
    write_executable(&bin.join("gh"), "#!/bin/sh\nprintf '[]\\n'\n");

    let started = Instant::now();
    let output = fixture
        .command(&bin)
        .args(["--stall-secs", "1", "--poll-secs", "1", "--json"])
        .env("AUTOSPEC_TEST_DESCENDANT_PID", &descendant_pid)
        .output()
        .expect("run stalled drain with a term-ignoring descendant");

    assert_eq!(
        output.status.code(),
        Some(124),
        "stderr={}",
        stderr(&output)
    );
    assert!(
        started.elapsed().as_secs() < 12,
        "drain waited for a surviving descendant for {:?}",
        started.elapsed()
    );
    assert_process_is_gone(&descendant_pid);
}

#[test]
fn completed_leader_terminates_a_descendant_that_keeps_both_pipes_open() {
    let fixture = DrainFixture::new();
    let bin = fixture.root.join("bin");
    let descendant_pid = fixture.root.join("descendant-pid");
    fs::create_dir_all(&bin).expect("create fake bin");
    write_executable(
        &bin.join("omx"),
        r#"#!/bin/sh
(
  trap '' TERM
  exec sleep 8
) &
printf '%s\n' "$!" > "$AUTOSPEC_TEST_DESCENDANT_PID"
exit 0
"#,
    );
    write_executable(&bin.join("gh"), "#!/bin/sh\nprintf '[]\\n'\n");

    let started = Instant::now();
    let output = fixture
        .command(&bin)
        .args(["--stall-secs", "30", "--poll-secs", "1", "--json"])
        .env("AUTOSPEC_TEST_DESCENDANT_PID", &descendant_pid)
        .output()
        .expect("run drain after leader exits with inherited pipes");

    assert!(output.status.success(), "stderr={}", stderr(&output));
    assert!(
        started.elapsed().as_secs() < 5,
        "drain joined inherited pipes before terminating the process group: {:?}",
        started.elapsed()
    );
    assert_process_is_gone(&descendant_pid);
}

#[test]
fn child_exit_is_not_blocked_by_a_hung_github_snapshot() {
    let fixture = DrainFixture::new();
    let bin = fixture.root.join("bin");
    fs::create_dir_all(&bin).expect("create fake bin");
    write_executable(&bin.join("omx"), "#!/bin/sh\nexec sleep 1\n");
    write_executable(&bin.join("gh"), "#!/bin/sh\nsleep 15\nprintf '[]\\n'\n");

    let started = Instant::now();
    let output = fixture.run(&bin, &["--json"]);

    assert!(output.status.success(), "stderr={}", stderr(&output));
    assert!(
        started.elapsed().as_secs() < 10,
        "hung GitHub observation delayed completed child for {:?}",
        started.elapsed()
    );
}

#[test]
fn child_exit_during_final_reconciliation_wins_over_termination() {
    let fixture = DrainFixture::new();
    let bin = fixture.root.join("bin");
    let gh_counter = fixture.root.join("gh-counter");
    fs::create_dir_all(&bin).expect("create fake bin");
    write_executable(&bin.join("omx"), "#!/bin/sh\nsleep 2\n");
    write_executable(
        &bin.join("gh"),
        r#"#!/bin/sh
set -eu
counter="$AUTOSPEC_TEST_GH_COUNTER"
count=0
if [ -f "$counter" ]; then
  count=$(cat "$counter")
fi
count=$((count + 1))
printf '%s\n' "$count" > "$counter"
if [ "$count" -gt 2 ]; then
  sleep 1
fi
printf '[]\n'
"#,
    );

    let output = fixture
        .command(&bin)
        .args(["--stall-secs", "1", "--poll-secs", "1", "--json"])
        .env("AUTOSPEC_TEST_GH_COUNTER", gh_counter)
        .output()
        .expect("run drain with child exit during reconciliation");

    assert!(output.status.success(), "stderr={}", stderr(&output));
    assert!(stdout(&output).contains("\"decision\":\"complete\""));
}

struct DrainFixture {
    root: PathBuf,
    repo_dir: PathBuf,
    operator_root: PathBuf,
    heartbeat_root: PathBuf,
}

impl DrainFixture {
    fn new() -> Self {
        let sequence = FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "autospec-drain-test-{}-{}-{}",
            std::process::id(),
            now_millis(),
            sequence
        ));
        let repo_dir = root.join("repo");
        fs::create_dir_all(&repo_dir).expect("create fixture repository");
        let fixture = Self {
            operator_root: root.join("operator"),
            heartbeat_root: root.join("heartbeats"),
            root,
            repo_dir,
        };
        fixture.initialize_git_remote();
        fixture
    }

    fn command(&self, bin: &Path) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_autospec"));
        command
            .args(["autonomous", "drain", "--repo", "owner/repo", "--repo-dir"])
            .arg(&self.repo_dir)
            .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &self.operator_root)
            .env("AUTOSPEC_PROCESS_HEARTBEAT_DIR", &self.heartbeat_root)
            .env("HOME", &self.root)
            .env("PATH", path_with(bin));
        command
    }

    fn run(&self, bin: &Path, args: &[&str]) -> Output {
        self.command(bin)
            .args(args)
            .output()
            .expect("run autonomous drain")
    }

    fn drain_observation_path(&self) -> PathBuf {
        self.operator_root
            .join("o5_owner_r4_repo")
            .join("drain-observation.json")
    }

    fn startup_retry_path(&self) -> PathBuf {
        self.operator_root
            .join("o5_owner_r4_repo")
            .join("drain-startup-retry.json")
    }

    fn session_events_path(&self) -> PathBuf {
        self.operator_root
            .join("o5_owner_r4_repo")
            .join("drain-session-events.jsonl")
    }

    fn initialize_git_remote(&self) {
        let init = Command::new("git")
            .args([
                "init",
                "-q",
                self.repo_dir.to_str().expect("repo directory"),
            ])
            .output()
            .expect("initialize fixture repository");
        assert!(init.status.success());
        let remote = Command::new("git")
            .args([
                "-C",
                self.repo_dir.to_str().expect("repo directory"),
                "remote",
                "add",
                "origin",
                "https://github.com/owner/repo.git",
            ])
            .output()
            .expect("set fixture remote");
        assert!(remote.status.success());
    }
}

impl Drop for DrainFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("current time")
        .as_millis()
}

fn write_executable(path: &Path, contents: &str) {
    fs::write(path, contents).expect("write fake executable");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("make fake executable");
}

fn path_with(bin: &Path) -> String {
    format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").expect("PATH is set")
    )
}

fn assert_process_is_gone(pid_path: &Path) {
    let pid = fs::read_to_string(pid_path).expect("read child pid");
    assert_pid_is_gone(pid.trim());
}

fn assert_pid_is_gone(pid: &str) {
    assert!(
        !Command::new("kill")
            .args(["-0", pid])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("inspect child")
            .success(),
        "process {} must no longer be alive",
        pid
    );
}

fn pid_path_is_alive(pid_path: &Path) -> bool {
    let pid = fs::read_to_string(pid_path).expect("read recorded pid");
    Command::new("kill")
        .args(["-0", pid.trim()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("inspect recorded process")
        .success()
}

fn kill_recorded_process_group(pid_path: &Path) {
    let pid = fs::read_to_string(pid_path).expect("read process-group leader pid");
    let group = format!("-{}", pid.trim());
    let _ = Command::new("kill")
        .args(["-KILL", "--", &group])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}
