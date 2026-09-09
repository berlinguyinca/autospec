use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_TEMP_ROOT: AtomicU64 = AtomicU64::new(0);

fn autospec() -> Command {
    Command::new(env!("CARGO_BIN_EXE_autospec"))
}

fn temp_dir(prefix: &str) -> std::path::PathBuf {
    let nonce = NEXT_TEMP_ROOT.fetch_add(1, Ordering::Relaxed);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time is after Unix epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "{prefix}-{}-{timestamp}-{nonce}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

#[test]
fn status_reports_parent_issue_terminal_state_counts() {
    let root = temp_dir("autospec-status-parent-issues");
    let state_dir = root.join(".autospec").join("state");
    std::fs::create_dir_all(&state_dir).expect("state dir");
    std::fs::write(
        state_dir.join("specs.json"),
        r#"{"schema":1,"specs":[],"parent_issues":[{"parent_issue":1899,"child_issues":[{"issue":1900,"terminal":true},{"issue":1901,"terminal":true}],"quarantined_parent":true,"decomposition_comment_posted":true,"parent_closed":false},{"parent_issue":1902,"child_issues":[{"issue":1903,"terminal":false}],"quarantined_parent":false,"decomposition_comment_posted":true,"parent_closed":false}]}"#,
    )
    .expect("state file");

    let text = autospec()
        .arg("status")
        .current_dir(&root)
        .output()
        .expect("status text");
    let text_stdout = String::from_utf8_lossy(&text.stdout);
    assert!(text.status.success());
    assert_eq!(
        text_stdout,
        "AutoSpec status: planned=0 ready=0 running=0 passed=0 failed=0 blocked=0 deferred=0 superseded=0\nparent issues: pending_children=1 quarantined_parent_decomposed=0 complete_but_stale=1 closed=0\nparent issue #1899: complete but stale (#1900, #1901)\nparent issue #1902: children pending (#1903)\n"
    );

    let json = autospec()
        .args(["status", "--json"])
        .current_dir(&root)
        .output()
        .expect("status json");
    assert!(json.status.success());
    let body: serde_json::Value = serde_json::from_slice(&json.stdout).expect("status json");
    assert_eq!(body["parent_issues"]["pending_children"], 1);
    assert_eq!(body["parent_issues"]["complete_but_stale"], 1);
}

/// Five parent issues, one non-terminal child each: the text rendering
/// prints exactly one list line per parent issue, in parent-number order.
const FIVE_PARENTS_JSON: &str = r#"{"schema":1,"specs":[],"parent_issues":[{"parent_issue":2001,"child_issues":[{"issue":2101,"terminal":false}],"quarantined_parent":false,"decomposition_comment_posted":true,"parent_closed":false},{"parent_issue":2002,"child_issues":[{"issue":2102,"terminal":false}],"quarantined_parent":false,"decomposition_comment_posted":true,"parent_closed":false},{"parent_issue":2003,"child_issues":[{"issue":2103,"terminal":false}],"quarantined_parent":false,"decomposition_comment_posted":true,"parent_closed":false},{"parent_issue":2004,"child_issues":[{"issue":2104,"terminal":false}],"quarantined_parent":false,"decomposition_comment_posted":true,"parent_closed":false},{"parent_issue":2005,"child_issues":[{"issue":2105,"terminal":false}],"quarantined_parent":false,"decomposition_comment_posted":true,"parent_closed":false}]}"#;

fn write_state(root: &std::path::Path, json: &str) {
    let state_dir = root.join(".autospec").join("state");
    std::fs::create_dir_all(&state_dir).expect("state dir");
    std::fs::write(state_dir.join("specs.json"), json).expect("state file");
}

#[test]
fn status_limit_truncates_the_list_and_announces_the_withheld_count_on_stdout() {
    let root = temp_dir("autospec-status-limit-truncated");
    write_state(&root, FIVE_PARENTS_JSON);

    let output = autospec()
        .args(["status", "--limit", "2"])
        .current_dir(&root)
        .output()
        .expect("status text");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    // The notice must ride the same stream as the list: stdout.
    let stdout_text = String::from_utf8_lossy(&output.stdout).to_string();
    let lines: Vec<&str> = stdout_text.lines().collect();
    assert_eq!(
        lines.len(),
        5,
        "two header lines + two list lines + one notice, got:\n{stdout_text}"
    );
    assert_eq!(lines[2], "parent issue #2001: children pending (#2101)");
    assert_eq!(lines[3], "parent issue #2002: children pending (#2102)");
    // Item-based counts: 3 lines withheld out of 5 total.
    assert_eq!(lines[4], "… 3 more lines (5 total)");
}

#[test]
fn status_limit_that_fits_emits_every_line_and_no_truncation_notice() {
    let root = temp_dir("autospec-status-limit-fits");
    write_state(&root, FIVE_PARENTS_JSON);

    for args in [
        vec!["status"],
        vec!["status", "--limit", "5"],
        vec!["status", "--limit", "10"],
    ] {
        let output = autospec()
            .args(&args)
            .current_dir(&root)
            .output()
            .expect("status text");
        assert!(
            output.status.success(),
            "stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout_text = String::from_utf8_lossy(&output.stdout).to_string();
        for parent in 2001..=2005 {
            assert!(
                stdout_text.contains(&format!("parent issue #{parent}")),
                "missing parent #{parent} for args {args:?} in:\n{stdout_text}"
            );
        }
        assert!(
            !stdout_text.contains("more lines"),
            "no notice when the list fits, got for args {args:?}:\n{stdout_text}"
        );
    }
}

#[test]
fn status_rejects_invalid_limits_with_a_diagnostic() {
    let root = temp_dir("autospec-status-limit-invalid");
    write_state(&root, FIVE_PARENTS_JSON);

    for limit in ["0", "abc", ""] {
        let output = autospec()
            .args(["status", "--limit", limit])
            .current_dir(&root)
            .output()
            .expect("status text");
        assert!(!output.status.success(), "--limit {limit:?} must fail");
        let stderr_text = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr_text.contains("--limit expects a positive integer"),
            "stderr for --limit {limit:?}: {stderr_text}"
        );
    }
    // Missing value after the flag.
    let output = autospec()
        .args(["status", "--limit"])
        .current_dir(&root)
        .output()
        .expect("status text");
    assert!(!output.status.success(), "bare --limit must fail");
}

#[test]
fn status_reads_shared_parent_state_without_replacing_local_spec_state() {
    let root = temp_dir("autospec-status-local-specs");
    let shared = temp_dir("autospec-status-shared-parents");
    std::fs::create_dir_all(root.join(".autospec/state")).expect("local state dir");
    std::fs::create_dir_all(shared.join(".autospec/state")).expect("shared state dir");
    std::fs::write(
        root.join(".autospec/state/specs.json"),
        r#"{"schema":1,"specs":[{"spec_id":"v1-local","state":"planned","deferred_reason":null,"superseded_by":null}],"parent_issues":[]}"#,
    )
    .expect("local state");
    std::fs::write(
        shared.join(".autospec/state/specs.json"),
        r#"{"schema":1,"specs":[],"parent_issues":[{"parent_issue":10,"child_issues":[{"issue":11,"terminal":false}],"quarantined_parent":true,"decomposition_comment_posted":true,"parent_closed":false}]}"#,
    )
    .expect("shared state");

    let output = autospec()
        .args(["status", "--json"])
        .current_dir(&root)
        .env("AUTOSPEC_PARENT_STATE_ROOT", &shared)
        .output()
        .expect("status json");

    assert!(output.status.success());
    let body: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("status JSON document");
    assert_eq!(body["specs"]["planned"], 1);
    assert_eq!(body["parent_issues"]["quarantined_parent_decomposed"], 1);
}
