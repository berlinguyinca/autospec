use std::process::Command;

fn autospec() -> Command {
    Command::new(env!("CARGO_BIN_EXE_autospec"))
}

#[test]
fn cli_commands_help_lists_required_commands() {
    let output = autospec().arg("--help").output().expect("autospec runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    for command in [
        "init",
        "doctor",
        "status",
        "plan",
        "validate",
        "run",
        "resume",
        "report",
        "showcase",
        "benchmark",
        "growth-report",
    ] {
        assert!(stdout.contains(command), "help missing {command}");
    }
}

#[test]
fn cli_commands_json_modes_emit_json() {
    for command in [
        "doctor",
        "status",
        "plan",
        "validate",
        "report",
        "showcase",
        "growth-report",
    ] {
        let output = autospec()
            .args([command, "--json"])
            .output()
            .unwrap_or_else(|error| panic!("{command} runs: {error}"));
        let stdout = String::from_utf8_lossy(&output.stdout);

        assert!(output.status.success(), "{command} failed");
        assert!(
            stdout.trim_start().starts_with('{'),
            "{command} did not emit JSON"
        );
        assert!(stdout.contains(&format!("\"command\":\"{command}\"")));
    }
}

#[test]
fn cli_commands_unimplemented_mutating_commands_are_explicit() {
    for command in ["init", "run", "resume", "benchmark"] {
        let output = autospec().arg(command).output().expect("autospec runs");
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert!(
            !output.status.success(),
            "{command} should not silently succeed"
        );
        assert!(stderr.contains("not yet implemented"));
    }
}
