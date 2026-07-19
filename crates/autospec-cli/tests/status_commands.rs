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
