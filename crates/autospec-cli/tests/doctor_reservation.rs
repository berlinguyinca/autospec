//! End-to-end tests for `autospec doctor reservation` (issue #3690).
//!
//! The command derives the agent budget from a Slurm reservation's walltime
//! and refuses to start when the budget does not fit. The refusal travels in
//! the exit code: 0 when the budget fits, 1 when it does not, 2 on bad
//! arguments.

use std::process::{Command, Output};

fn autospec_binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_autospec"))
}

fn reservation(args: &[&str]) -> Output {
    autospec_binary()
        .arg("doctor")
        .arg("reservation")
        .args(args)
        .output()
        .expect("autospec doctor reservation runs")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

#[test]
fn an_eight_hour_walltime_derives_a_budget_past_the_old_forty_five_minutes() {
    let output = reservation(&["--walltime", "8:00:00"]);
    assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("28200"), "derived budget, got: {text}");
    assert!(text.contains("28800"), "walltime in seconds, got: {text}");
}

#[test]
fn walltime_is_accepted_as_plain_seconds() {
    let output = reservation(&["--walltime", "28800"]);
    assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));
    assert!(stdout(&output).contains("28200"));
}

#[test]
fn a_requesting_budget_that_fits_is_accepted() {
    let output = reservation(&["--walltime", "28800", "--budget", "2700"]);
    assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));
    assert!(stdout(&output).contains("fits"));
}

#[test]
fn a_budget_that_does_not_fit_the_reservation_is_refused_with_exit_1() {
    // The old LIMIT (45 minutes) inside a 45-minute reservation.
    let output = reservation(&["--walltime", "2700", "--budget", "2700"]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "refusal must travel in the exit code; stderr: {}",
        stderr(&output)
    );
    assert!(stdout(&output).to_ascii_uppercase().contains("REFUSED"));
}

#[test]
fn a_reservation_too_small_to_host_an_agent_is_refused() {
    let output = reservation(&["--walltime", "15:00"]);
    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert!(stdout(&output).to_ascii_uppercase().contains("REFUSED"));
}

#[test]
fn a_malformed_walltime_is_a_bad_argument() {
    let output = reservation(&["--walltime", "1:2:3:4"]);
    assert_eq!(output.status.code(), Some(2), "stderr: {}", stderr(&output));
}

#[test]
fn a_missing_walltime_is_a_bad_argument() {
    let output = reservation(&[]);
    assert_eq!(output.status.code(), Some(2), "stderr: {}", stderr(&output));
}

#[test]
fn json_output_carries_the_derived_budget_and_the_status() {
    let output = reservation(&["--walltime", "8:00:00", "--json"]);
    assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));
    let json: serde_json::Value =
        serde_json::from_str(&stdout(&output)).expect("valid JSON on stdout");
    assert_eq!(json["command"], "doctor");
    assert_eq!(json["subcommand"], "reservation");
    assert_eq!(json["walltime_secs"], 28800);
    assert_eq!(json["derived_budget_secs"], 28200);
    assert_eq!(json["status"], "ok");

    let refused = reservation(&["--walltime", "2700", "--budget", "2700", "--json"]);
    let refused_json: serde_json::Value =
        serde_json::from_str(&stdout(&refused)).expect("valid JSON on stdout");
    assert_eq!(refused_json["status"], "refused");
    assert!(refused_json["reason"]
        .as_str()
        .unwrap_or("")
        .contains("does not fit"));
}

#[test]
fn help_shows_the_options() {
    let output = reservation(&["--help"]);
    assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("--walltime"));
    assert!(text.contains("--budget"));
    assert!(text.contains("EXIT CODES"));
}
