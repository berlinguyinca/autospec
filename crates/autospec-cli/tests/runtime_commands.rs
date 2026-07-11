use std::process::Command;

fn autospec() -> Command {
    Command::new(env!("CARGO_BIN_EXE_autospec"))
}

#[test]
fn runtime_commands_json_reports_r1_for_validation_script() {
    let output = autospec()
        .args(["runtime", "classify", "scripts/validate.sh", "--json"])
        .output()
        .expect("autospec runtime classify runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("\"path\":\"scripts/validate.sh\""));
    assert!(stdout.contains("\"runtime\":\"shell\""));
    assert!(stdout.contains("\"class\":\"R1\""));
    assert!(stdout.contains("\"stateful platform behavior belongs in Rust core\""));
}

#[test]
fn runtime_commands_text_reports_one_line_with_class_and_path() {
    let output = autospec()
        .args(["runtime", "classify", "scripts/validate.sh"])
        .output()
        .expect("autospec runtime classify runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert_eq!(stdout.lines().count(), 1);
    assert!(stdout.starts_with("R1 scripts/validate.sh "));
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
