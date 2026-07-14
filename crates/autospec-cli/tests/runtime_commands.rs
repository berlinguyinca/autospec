use std::path::PathBuf;
use std::process::Command;

fn autospec() -> Command {
    Command::new(env!("CARGO_BIN_EXE_autospec"))
}

fn audit_fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/runtime-audit")
}

#[test]
fn runtime_commands_json_reports_r1_for_stateful_shell_helpers() {
    let output = autospec()
        .args(["runtime", "classify", "scripts/lint-issue.sh", "--json"])
        .output()
        .expect("autospec runtime classify runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("\"path\":\"scripts/lint-issue.sh\""));
    assert!(stdout.contains("\"runtime\":\"shell\""));
    assert!(stdout.contains("\"class\":\"R1\""));
    assert!(stdout.contains("\"stateful platform behavior belongs in Rust core\""));
}

#[test]
fn runtime_commands_text_reports_one_line_with_class_and_path() {
    let output = autospec()
        .args(["runtime", "classify", "scripts/lint-issue.sh"])
        .output()
        .expect("autospec runtime classify runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert_eq!(stdout.lines().count(), 1);
    assert!(stdout.starts_with("R1 scripts/lint-issue.sh "));
}

#[test]
fn runtime_commands_unknown_paths_return_r2_json() {
    let output = autospec()
        .args(["runtime", "classify", "docs/specs/example.md", "--json"])
        .output()
        .expect("autospec runtime classify runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("\"path\":\"docs/specs/example.md\""));
    assert!(stdout.contains("\"runtime\":\"unknown\""));
    assert!(stdout.contains("\"class\":\"R2\""));
}

#[test]
fn runtime_audit_json_groups_platform_files_and_skips_build_output() {
    let output = autospec()
        .args([
            "runtime",
            "audit",
            "--root",
            audit_fixture().to_str().expect("fixture path is UTF-8"),
            "--json",
        ])
        .output()
        .expect("autospec runtime audit runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("\"command\":\"runtime audit\""));
    assert!(stdout.contains(
        "\"R1\":[\"scripts/lint-issue.sh\",\"skills/autospec-run/tests/watchdog_claim_timeout.bats\"]"
    ));
    assert!(stdout.contains("\"R2\":[\"packages/example/go.mod\"]"));
    assert!(stdout.contains("\"R4\":[\"skills/autospec-fab/scripts/mesh.py\"]"));
    assert!(!stdout.contains("target/ignored.rs"));
}

#[test]
fn runtime_audit_rejects_missing_root() {
    let output = autospec()
        .args(["runtime", "audit", "--root", "/missing/runtime-audit-root"])
        .output()
        .expect("autospec runtime audit starts");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("does not exist"));
}

#[cfg(unix)]
#[test]
fn runtime_audit_does_not_follow_a_symlinked_platform_root() {
    let temporary = std::env::temp_dir().join(format!(
        "autospec-runtime-audit-{}-{}",
        std::process::id(),
        "symlink-root"
    ));
    let root = temporary.join("root");
    let outside = temporary.join("outside");
    std::fs::create_dir_all(&root).expect("create fixture root");
    std::fs::create_dir_all(&outside).expect("create outside directory");
    std::fs::write(outside.join("secret.py"), "print('secret')\n").expect("write outside file");
    std::os::unix::fs::symlink(&outside, root.join("scripts")).expect("link scripts root");

    let output = autospec()
        .args([
            "runtime",
            "audit",
            "--root",
            root.to_str().expect("temporary path is UTF-8"),
            "--json",
        ])
        .output()
        .expect("autospec runtime audit runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    std::fs::remove_dir_all(&temporary).expect("remove temporary fixture");
    assert!(output.status.success());
    assert!(!stdout.contains("secret.py"));
}
