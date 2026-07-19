use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

fn autospec() -> Command {
    Command::new(env!("CARGO_BIN_EXE_autospec"))
}

#[test]
fn issue_promote_outputs_auto_implement_only_after_safety_pass() {
    let body = "## Goal\nAdd `autospec issue promote`.\n\n## Safety review\n\n<!-- autospec-safety:begin -->\n- **decision:** `SAFETY_PASS`\n<!-- autospec-safety:end -->\n";
    let path = write_issue_body("pass", body);

    let output = autospec()
        .args([
            "issue",
            "promote",
            "--number",
            "1890",
            "--title",
            "Add a typed issue promotion command",
            "--body-file",
            path.to_str().unwrap(),
            "--author",
            "berlinguyinca",
            "--label",
            "safety:reviewed",
            "--json",
        ])
        .output()
        .expect("autospec issue promote runs");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"decision\":\"pass\""));
    assert!(stdout.contains("\"auto-implement\":true"));
    assert!(stdout.contains("\"drainable\":true"));
}

#[test]
fn issue_promote_returns_structured_block_without_auto_implement() {
    let body = "## Goal\nImplement a harmless local queue change.\n";
    let path = write_issue_body("blocked", body);

    let output = autospec()
        .args([
            "issue",
            "promote",
            "--number",
            "1892",
            "--title",
            "Implement safe queue work",
            "--body-file",
            path.to_str().unwrap(),
            "--author",
            "contributor",
            "--json",
        ])
        .output()
        .expect("autospec issue promote runs");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"decision\":\"blocked\""));
    assert!(stdout.contains("\"auto-implement\":false"));
    assert!(stdout.contains("\"missing_safety_reviewed\":1"));
}

fn write_issue_body(name: &str, body: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "autospec-issue-promote-{name}-{}-{}.md",
        std::process::id(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, body).expect("issue fixture");
    path
}
