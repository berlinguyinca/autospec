// executor_bridge tests: production / entry — 10 cases.
//
// Split out of tests.rs; see the note in that file.

use super::super::{
    supervise_harness, HarnessKind, MutationSnapshot, SupervisionConfig, SupervisionOutcome,
};
use super::support_base::GitFixture;
use super::support_invocation::{shell_invocation, supervision_config, supervision_state};
use crate::commands::autonomous::executor_bridge as bridge;
use std::fs::{self, File, OpenOptions};
#[cfg(unix)]
use std::os::unix::fs::symlink;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[test]
fn autonomous_executor_bridge_frames_split_utf8_and_bounds_sustained_output() {
    let fixture = GitFixture::new("supervise-split-utf8");
    let mut state = supervision_state(&fixture);
    let snapshot =
        MutationSnapshot::capture(&fixture.repo, &state.identity.branch).expect("snapshot");
    let state_path = fixture.root.join("state/invocation.json");

    let outcome = supervise_harness(
        &state_path,
        &fixture.root.join("log/executor.jsonl"),
        &mut state,
        &shell_invocation(
            &fixture.repo,
            "printf '\\342'; sleep 0.03; printf '\\202'; sleep 0.03; printf '\\254\\n'; i=0; while [ \"$i\" -lt 400 ]; do printf 'line-%s\\n' \"$i\"; i=$((i+1)); done",
        ),
        &snapshot,
        SupervisionConfig {
            stall_timeout: Duration::from_millis(2_000),
            poll_interval: Duration::from_millis(250),
        },
    )
    .expect("split UTF-8 and sustained output");

    assert_eq!(outcome, SupervisionOutcome::Exited { exit_code: 0 });
    let event_log = fixture.root.join("log/executor.jsonl");
    let events = fs::read_to_string(&event_log).expect("events");
    let backup = event_log.with_extension("jsonl.1");
    let retained = if backup.exists() {
        format!(
            "{}{events}",
            fs::read_to_string(&backup).expect("backup events")
        )
    } else {
        events.clone()
    };
    assert!(retained.contains('€'), "{retained}");
    assert!(
        retained.contains("\"event\":\"child_output_dropped\""),
        "{retained}"
    );
    let sinks =
        bridge::output_sink_paths(&state_path, &state.identity.invocation_id).expect("sinks");
    let writer = bridge::read_output_cursor(
        &OpenOptions::new()
            .read(true)
            .open(&sinks.stdout_writer_cursor)
            .expect("writer cursor"),
    )
    .expect("writer position");
    let reader = bridge::read_output_cursor(
        &OpenOptions::new()
            .read(true)
            .open(&sinks.stdout_reader_cursor)
            .expect("reader cursor"),
    )
    .expect("reader position");
    assert_eq!(
        reader.total, writer.total,
        "coalesced output tail was not durably acknowledged"
    );
    assert!(
        fs::metadata(&event_log)
            .expect("current event segment")
            .len()
            <= bridge::EVENT_LOG_SEGMENT_LIMIT
    );
    if backup.exists() {
        assert!(
            fs::metadata(backup).expect("backup event segment").len()
                <= bridge::EVENT_LOG_SEGMENT_LIMIT
        );
    }
}

#[test]
fn autonomous_executor_bridge_closed_streams_live_child_transitions_to_stall() {
    let fixture = GitFixture::new("supervise-closed-streams");
    let mut state = supervision_state(&fixture);
    let snapshot =
        MutationSnapshot::capture(&fixture.repo, &state.identity.branch).expect("snapshot");

    let outcome = supervise_harness(
        &fixture.root.join("state/invocation.json"),
        &fixture.root.join("log/executor.jsonl"),
        &mut state,
        &shell_invocation(&fixture.repo, "exec >/dev/null 2>&1; sleep 30"),
        &snapshot,
        supervision_config(100),
    )
    .expect("closed streams must remain timed supervision");

    assert_eq!(outcome, SupervisionOutcome::Stalled);
    let events = fs::read_to_string(fixture.root.join("log/executor.jsonl")).expect("events");
    assert!(events.contains("\"stall_timeout_ms\":100"), "{events}");
    assert!(events.contains("\"last_progress_at\":"), "{events}");
}

#[test]
fn autonomous_executor_bridge_stall_reports_actual_durable_progress_timestamp() {
    // Break caught: subsecond stalls synthesizing "last progress" from the timeout after the
    // durable timestamp had already been overwritten.
    while SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .subsec_millis()
        < 900
    {
        std::thread::sleep(Duration::from_millis(5));
    }
    let fixture = GitFixture::new("supervise-stall-timestamp");
    let mut state = supervision_state(&fixture);
    let snapshot =
        MutationSnapshot::capture(&fixture.repo, &state.identity.branch).expect("snapshot");
    let event_log = fixture.root.join("log/executor.jsonl");

    let outcome = supervise_harness(
        &fixture.root.join("state/invocation.json"),
        &event_log,
        &mut state,
        &shell_invocation(&fixture.repo, "sleep 30"),
        &snapshot,
        supervision_config(200),
    )
    .expect("stalled child");
    assert_eq!(outcome, SupervisionOutcome::Stalled);

    let events = fs::read_to_string(event_log).expect("events");
    let parsed = events
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("event JSON"))
        .collect::<Vec<_>>();
    let started_at = parsed
        .iter()
        .find(|event| event["event"] == "child_started")
        .and_then(|event| event["progress_at"].as_u64())
        .expect("child-started progress");
    let stalled_at = parsed
        .iter()
        .find(|event| event["event"] == "child_stalled")
        .and_then(|event| event["last_progress_at"].as_u64())
        .expect("stall last progress");
    assert_eq!(stalled_at, started_at, "{events}");
}

#[test]
fn autonomous_executor_bridge_reader_io_error_is_structured() {
    let fixture = GitFixture::new("supervise-reader-error");
    let directory = File::open(&fixture.root).expect("open real directory descriptor");
    let writer_cursor = bridge::open_private_file(&fixture.root.join("writer.cursor"), true)
        .expect("writer cursor");
    let reader_cursor = bridge::open_private_file(&fixture.root.join("reader.cursor"), true)
        .expect("reader cursor");
    for cursor in [&writer_cursor, &reader_cursor] {
        cursor
            .set_len(bridge::OUTPUT_CURSOR_FILE_BYTES)
            .expect("size cursor");
        bridge::write_output_cursor(cursor, bridge::OutputCursor::default())
            .expect("initialize cursor");
    }
    bridge::write_output_cursor(
        &writer_cursor,
        bridge::OutputCursor {
            generation: 1,
            total: 1,
            dropped: 0,
        },
    )
    .expect("publish one byte");
    let mut readers = bridge::DurableOutputReaders {
        streams: vec![bridge::DurableOutputStream {
            name: "stdout",
            path: fixture.root.clone(),
            file: directory,
            offset: 0,
            dropped: 0,
            writer_cursor,
            reader_cursor,
            partial: Vec::new(),
            discarding_oversized: false,
        }],
        pending: Vec::new(),
        last_flush: Instant::now() - Duration::from_secs(1),
        io_failed: false,
        reported_events: 0,
        coalesced_reported: false,
    };

    assert_eq!(readers.poll().expect("I/O error becomes an event"), 0);
    assert!(readers.io_failed());
    assert!(readers.pending.iter().any(|event| event.io_error));
}

#[test]
fn autonomous_executor_bridge_completion_drain_flushes_a_full_pending_batch_before_eof() {
    let fixture = GitFixture::new("completion-drain-pending-cap");
    let mut state = supervision_state(&fixture);
    let state_path = fixture.root.join("invocation.json");
    let event_log = fixture.root.join("events.jsonl");
    let ring_path = fixture.root.join("stdout.ring");
    let ring_contents = b"tail-marker\n";
    fs::write(&ring_path, ring_contents).expect("write unread ring contents");
    let writer_cursor = bridge::open_private_file(&fixture.root.join("writer.cursor"), true)
        .expect("writer cursor");
    let reader_cursor = bridge::open_private_file(&fixture.root.join("reader.cursor"), true)
        .expect("reader cursor");
    for cursor in [&writer_cursor, &reader_cursor] {
        cursor
            .set_len(bridge::OUTPUT_CURSOR_FILE_BYTES)
            .expect("size cursor");
        bridge::write_output_cursor(cursor, bridge::OutputCursor::default())
            .expect("initialize cursor");
    }
    bridge::write_output_cursor(
        &writer_cursor,
        bridge::OutputCursor {
            generation: 1,
            total: ring_contents.len() as u64,
            dropped: 0,
        },
    )
    .expect("publish unread ring contents");
    let pending = (0..bridge::OUTPUT_EVENTS_PER_HEARTBEAT)
        .map(|sequence| bridge::OutputEvent {
            stream: "stdout",
            line: format!("pending-{sequence}"),
            truncated: false,
            io_error: false,
            dropped: 0,
        })
        .collect();
    let mut readers = bridge::DurableOutputReaders {
        streams: vec![bridge::DurableOutputStream {
            name: "stdout",
            path: ring_path.clone(),
            file: File::open(&ring_path).expect("open ring"),
            offset: 0,
            dropped: 0,
            writer_cursor,
            reader_cursor,
            partial: Vec::new(),
            discarding_oversized: false,
        }],
        pending,
        last_flush: Instant::now() - Duration::from_secs(1),
        io_failed: false,
        reported_events: 0,
        coalesced_reported: false,
    };
    let mut renewal = bridge::ClaimRenewalSchedule::Disabled;

    let outcome = readers
        .drain_after_completion(&state_path, &event_log, &mut state, &mut renewal)
        .expect("drain unread ring after flushing pending batch");

    assert_eq!(outcome, bridge::CompletionDrainOutcome::Drained);
    assert_eq!(readers.streams[0].offset, ring_contents.len() as u64);
    assert!(
        fs::read_to_string(event_log)
            .expect("drain events")
            .contains("tail-marker"),
        "unread ring contents were not drained"
    );
}

#[test]
fn autonomous_executor_bridge_production_entry_rejects_foreign_worktree() {
    let fixture = GitFixture::new("supervise-production-worktree");
    let mut state = supervision_state(&fixture);
    state.identity.worktree = fixture.root.join("foreign");
    fs::create_dir(&state.identity.worktree).expect("foreign worktree");
    let snapshot =
        MutationSnapshot::capture(&fixture.repo, &state.identity.branch).expect("snapshot");
    let harness = bridge::ResolvedHarness {
        kind: HarnessKind::Codex,
        executable: PathBuf::from("/bin/sh")
            .canonicalize()
            .expect("canonical shell"),
        opencode_adapter: None,
        codex_sandbox: bridge::CodexSandboxPolicy::Default,
        opencode_model: None,
        opencode_variant: None,
    };

    let error = bridge::supervise_resolved_harness(
        &fixture.root.join("state/invocation.json"),
        &fixture.root.join("log/executor.jsonl"),
        &mut state,
        bridge::HarnessLaunch {
            resolved: &harness,
            artifact: &fixture.root.join("artifact"),
            prompt: "prompt",
        },
        &snapshot,
        supervision_config(2_000),
    )
    .expect_err("foreign worktree must be rejected");

    assert!(error.contains("validated executor worktree"));
}

#[test]
fn autonomous_executor_bridge_production_entry_rejects_foreign_artifact() {
    // Break caught: Codex writing its result artifact outside the exact registered worktree.
    let fixture = GitFixture::new("supervise-production-foreign-artifact");
    let mut state = supervision_state(&fixture);
    state.identity.branch = "main".to_string();
    let snapshot =
        MutationSnapshot::capture(&fixture.repo, &state.identity.branch).expect("snapshot");
    let harness = bridge::ResolvedHarness {
        kind: HarnessKind::Codex,
        executable: PathBuf::from("/bin/sh")
            .canonicalize()
            .expect("canonical shell"),
        opencode_adapter: None,
        codex_sandbox: bridge::CodexSandboxPolicy::Default,
        opencode_model: None,
        opencode_variant: None,
    };

    let error = bridge::supervise_resolved_harness(
        &fixture.root.join("state/invocation.json"),
        &fixture.root.join("log/executor.jsonl"),
        &mut state,
        bridge::HarnessLaunch {
            resolved: &harness,
            artifact: &fixture.root.join("foreign-artifact"),
            prompt: "prompt",
        },
        &snapshot,
        supervision_config(2_000),
    )
    .expect_err("foreign artifact must be rejected before argv construction");

    assert!(error.contains("artifact"), "unexpected error: {error}");
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_production_entry_rejects_symlinked_artifact() {
    // Break caught: an in-worktree artifact pathname escaping through a symlink component.
    let fixture = GitFixture::new("supervise-production-symlink-artifact");
    let mut state = supervision_state(&fixture);
    state.identity.branch = "main".to_string();
    let target = fixture.repo.join("real-artifact");
    fs::write(&target, "").expect("artifact target");
    let artifact = fixture.repo.join("artifact-link");
    symlink(&target, &artifact).expect("artifact symlink");
    let snapshot =
        MutationSnapshot::capture(&fixture.repo, &state.identity.branch).expect("snapshot");
    let harness = bridge::ResolvedHarness {
        kind: HarnessKind::Codex,
        executable: PathBuf::from("/bin/sh")
            .canonicalize()
            .expect("canonical shell"),
        opencode_adapter: None,
        codex_sandbox: bridge::CodexSandboxPolicy::Default,
        opencode_model: None,
        opencode_variant: None,
    };

    let error = bridge::supervise_resolved_harness(
        &fixture.root.join("state/invocation.json"),
        &fixture.root.join("log/executor.jsonl"),
        &mut state,
        bridge::HarnessLaunch {
            resolved: &harness,
            artifact: &artifact,
            prompt: "prompt",
        },
        &snapshot,
        supervision_config(2_000),
    )
    .expect_err("symlinked artifact must be rejected before argv construction");

    assert!(error.contains("symlink"), "unexpected error: {error}");
}

#[test]
fn autonomous_executor_bridge_parses_primary_smoke_as_direct_segments() {
    let body = "## Verification\n\n### Primary smoke test (inner loop)\n\n```bash\n/usr/bin/printf 'first value' '' \"\" && /usr/bin/printf second\n```\n";

    let plan = bridge::parse_primary_smoke(body).expect("bounded direct smoke plan");

    assert_eq!(plan.commands.len(), 2);
    assert_eq!(
        plan.commands[0].argv,
        vec!["/usr/bin/printf", "first value", "", ""]
    );
    assert_eq!(plan.commands[1].argv, vec!["/usr/bin/printf", "second"]);
}

#[test]
fn autonomous_executor_bridge_rejects_shell_operator_families_and_unbounded_smoke() {
    for line in [
        "printf ok | cat",
        "printf ok > out",
        "printf $(id)",
        "printf `id`",
        "printf ok &",
        "printf ok ; printf bad",
        "if true; then printf ok; fi",
        "cd /tmp",
        "printf ok\nprintf bad",
        "printf ok && && printf bad",
        "if true",
        "then true",
        "else true",
        "elif true",
        "fi",
        "for value",
        "while true",
        "until true",
        "do true",
        "done",
        "case value",
        "esac",
        "function run",
        "select value",
        "time true",
        "coproc true",
        "command true",
        "{ true",
        "} true",
        "[ true",
        "] true",
        "[[ true",
        "]] true",
    ] {
        let body = format!("### Primary smoke test (inner loop)\n\n```bash\n{line}\n```\n");
        assert!(
            bridge::parse_primary_smoke(&body).is_err(),
            "unsafe smoke was accepted: {line:?}"
        );
    }
    let oversized = format!(
        "### Primary smoke test (inner loop)\n\n```bash\n/usr/bin/printf {}\n```\n",
        "x".repeat(bridge::MAX_DIRECT_COMMAND_LINE + 1)
    );
    assert!(bridge::parse_primary_smoke(&oversized).is_err());
}
