//! Deterministic coverage for issue #3321: bounded tool output, file-read
//! protection, and compaction at the 60/72/88 percent occupancy thresholds.

use autospec_core::context::context_control::{
    compact_session, normalize_tool_output, plan_file_read, CompactionTracker, FileReadPlan,
    SessionSnapshot, ToolOutput, COMPACTION_THRESHOLDS, MAX_FILE_LINES, MAX_NORMALIZED_LINES,
};

/// AC 1: a 5000-line passing log contributes at most 40 normalized lines.
#[test]
fn context_control_passing_5000_line_log_is_normalized_to_at_most_40_lines() {
    let log = (0..5000)
        .map(|i| format!("line {i} of successful build output"))
        .collect::<Vec<_>>();
    let log = log.join("\n");
    let result = normalize_tool_output(&ToolOutput {
        command: "cargo build --workspace".to_string(),
        exit_code: 0,
        log,
        artifact_dir: None,
    });

    assert!(result.ok);
    assert_eq!(result.exit_code, 0);
    assert!(
        result.injected_lines.len() <= MAX_NORMALIZED_LINES,
        "expected at most {MAX_NORMALIZED_LINES} normalized lines, got {}",
        result.injected_lines.len()
    );
    // First and last log lines are preserved so a reviewer can spot-check the ends.
    assert_eq!(
        result.injected_lines[1],
        "line 0 of successful build output"
    );
    assert_eq!(
        result.injected_lines.last().unwrap(),
        "line 4999 of successful build output"
    );
    // A passing log is summarized in place; no full-log artifact is required.
    assert!(result.full_log_artifact.is_none());
}

/// A short passing log (well under the bound) is passed through verbatim.
#[test]
fn context_control_short_passing_log_is_preserved_verbatim() {
    let result = normalize_tool_output(&ToolOutput {
        command: "cargo fmt --check".to_string(),
        exit_code: 0,
        log: "ok line one\nok line two".to_string(),
        artifact_dir: None,
    });

    assert!(result.ok);
    assert!(result.injected_lines.len() <= MAX_NORMALIZED_LINES);
    assert!(result.injected_lines.contains(&"ok line one".to_string()));
    assert!(result.injected_lines.contains(&"ok line two".to_string()));
    // No omission marker for a log that fits.
    assert!(!result.injected_lines.iter().any(|l| l.contains("omitted")));
}

/// AC 2: a failed command carries `exit_code` and `full_log_artifact`,
/// and only a concise failure is injected.
#[test]
fn context_control_failed_command_carries_exit_code_and_full_log_artifact() {
    let log = (0..200)
        .map(|i| format!("stderr line {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let result = normalize_tool_output(&ToolOutput {
        command: "cargo test -p autospec-core".to_string(),
        exit_code: 101,
        log,
        artifact_dir: None,
    });

    assert!(!result.ok);
    assert_eq!(result.exit_code, 101);
    let artifact = result
        .full_log_artifact
        .as_deref()
        .expect("failed command must persist a full_log_artifact");
    assert!(artifact.contains("cargo-test"), "artifact was {artifact}");
    // Concise failure only: far fewer lines than the 200-line log.
    assert!(result.injected_lines.len() <= MAX_NORMALIZED_LINES);
    assert!(result.injected_lines.len() < 20);
    // The failure summary names the exit code.
    assert!(result.injected_lines.first().unwrap().contains("exit=101"));
}

/// The artifact path honors a caller-supplied artifact directory.
#[test]
fn context_control_failed_command_artifact_uses_configured_directory() {
    let result = normalize_tool_output(&ToolOutput {
        command: "cargo clippy --workspace".to_string(),
        exit_code: 1,
        log: "error: could not compile".to_string(),
        artifact_dir: Some("/tmp/reports/tool-logs".to_string()),
    });

    assert_eq!(
        result.full_log_artifact.as_deref(),
        Some("/tmp/reports/tool-logs/cargo-clippy-workspace.log")
    );
}

/// AC 3: a 1501-line file requires an explicit range.
#[test]
fn context_control_file_with_1501_lines_requires_an_explicit_range() {
    let plan = plan_file_read(1501, None, MAX_FILE_LINES);
    assert!(
        matches!(plan, FileReadPlan::RangeRequired),
        "expected RangeRequired, got {plan:?}"
    );

    // The same file with an explicit valid range is allowed.
    let plan = plan_file_read(1501, Some((100, 400)), MAX_FILE_LINES);
    assert_eq!(
        plan,
        FileReadPlan::Range {
            start: 100,
            end: 400
        }
    );

    // An invalid range is not an explicit range.
    assert_eq!(
        plan_file_read(1501, Some((400, 100)), MAX_FILE_LINES),
        FileReadPlan::RangeRequired
    );
    assert_eq!(
        plan_file_read(1501, Some((0, 10)), MAX_FILE_LINES),
        FileReadPlan::RangeRequired
    );
    assert_eq!(
        plan_file_read(1501, Some((1, 1502)), MAX_FILE_LINES),
        FileReadPlan::RangeRequired
    );
}

/// A file exactly at the limit may be read whole; one over may not.
#[test]
fn context_control_file_at_line_limit_is_readable_whole() {
    assert_eq!(
        plan_file_read(1500, None, MAX_FILE_LINES),
        FileReadPlan::Whole
    );
    assert_eq!(plan_file_read(1, None, MAX_FILE_LINES), FileReadPlan::Whole);
    // A range on a small file is honored, not rejected.
    assert_eq!(
        plan_file_read(100, Some((10, 20)), MAX_FILE_LINES),
        FileReadPlan::Range { start: 10, end: 20 }
    );
}

/// AC 4: occupancy at 72 percent emits exactly 1 compaction event.
#[test]
fn context_control_occupancy_at_72_percent_emits_exactly_one_compaction_event() {
    let mut tracker = CompactionTracker::new();

    assert_eq!(tracker.record(59).len(), 0);
    let events = tracker.record(72);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].threshold_percent, 72);
    assert_eq!(events[0].occupancy_percent, 72);

    // Staying in the same band emits no further events.
    assert_eq!(tracker.record(72).len(), 0);
    assert_eq!(tracker.record(87).len(), 0);

    // Crossing the next threshold emits exactly one event for it.
    let events = tracker.record(88);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].threshold_percent, 88);
}

/// A compaction that drops occupancy below the lowest threshold re-arms
/// the tracker so the next crossing fires again.
#[test]
fn context_control_dropping_below_lowest_threshold_rearms_compaction() {
    let mut tracker = CompactionTracker::new();
    tracker.record(72);
    assert_eq!(tracker.record(65).len(), 0);

    let events = tracker.record(55);
    assert!(events.is_empty());
    // Re-armed: the next crossing of 60 fires exactly once.
    let events = tracker.record(63);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].threshold_percent, 60);
}

/// The default thresholds are the ones the issue pins.
#[test]
fn context_control_default_compaction_thresholds_are_60_72_88() {
    assert_eq!(COMPACTION_THRESHOLDS, [60, 72, 88]);
    assert_eq!(CompactionTracker::new().thresholds(), [60, 72, 88]);
}

/// Compacting a session must not lose goal, diff, tests, or failures.
#[test]
fn context_control_compact_session_preserves_goal_diff_tests_and_failures() {
    let snapshot = SessionSnapshot {
        goal: "Bound tool output and compaction".to_string(),
        diff: "crates/autospec-core/src/context/context_control.rs".to_string(),
        tests: "cargo test -p autospec-core context_control".to_string(),
        failures: vec!["FAILED context_monitor::parity".to_string()],
    };

    let compacted = compact_session(&snapshot);
    assert!(
        compacted.contains("Bound tool output and compaction"),
        "goal lost: {compacted}"
    );
    assert!(
        compacted.contains("crates/autospec-core/src/context/context_control.rs"),
        "diff lost: {compacted}"
    );
    assert!(
        compacted.contains("cargo test -p autospec-core context_control"),
        "tests lost: {compacted}"
    );
    assert!(
        compacted.contains("FAILED context_monitor::parity"),
        "failures lost: {compacted}"
    );
}

/// A session with no failures still names the failures section.
#[test]
fn context_control_compact_session_names_failures_section_when_empty() {
    let snapshot = SessionSnapshot {
        goal: "g".to_string(),
        diff: "d".to_string(),
        tests: "t".to_string(),
        failures: Vec::new(),
    };

    let compacted = compact_session(&snapshot);
    assert!(compacted.contains("failures: none"), "got: {compacted}");
}
