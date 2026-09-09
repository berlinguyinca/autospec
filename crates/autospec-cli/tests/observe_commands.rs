//! End-to-end tests for `autospec observe` (issue #3973).
//!
//! The command must report a step's status from its own heartbeat and
//! terminal markers rather than from log recency, so a log caught mid-write
//! reads `running` instead of being stopped as `stalled`; refuse to authorize
//! a destructive action on a single glance; and compute a characterisation
//! from counts over the whole source rather than the visible tail.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

#[path = "support/temp_directory.rs"]
mod temp_directory;
use temp_directory::unique as temp_dir;

/// A "now" far from the machine clock, so tests never depend on wall time.
const NOW: u64 = 1_700_000_600;

fn autospec() -> Command {
    Command::new(env!("CARGO_BIN_EXE_autospec"))
}

fn run(args: &[&str]) -> Output {
    autospec().args(args).output().expect("run autospec")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).to_string()
}

fn code(out: &Output) -> i32 {
    out.status.code().unwrap_or(-1)
}

/// A log written a while ago: the file the operator stopped at 02:00.
/// `age` seconds of silence are simulated by `--now`, not by sleeping.
fn aged_log(dir: &Path, name: &str, body: &str, age: u64) -> std::path::PathBuf {
    let path = dir.join(name);
    fs::write(&path, body).expect("write log");
    let mtime = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(NOW - age);
    let file = fs::File::options()
        .write(true)
        .open(&path)
        .expect("open for utimes");
    file.set_modified(mtime).expect("set mtime");
    path
}

/// AC5 + AC2: a conversion log caught mid-write — 4000 lines, a recent
/// heartbeat, no terminal marker — is `running`, and exit 0. Under
/// recency-based status this exact file was stopped as `stalled`.
#[test]
fn a_populated_log_mid_write_is_running_not_stalled() {
    let dir = temp_dir("observe");
    let mut body = String::from("## conversion started\n");
    for index in 0..4_000 {
        body.push_str(&format!("## progress {index} elapsed 1620s\n"));
    }
    body.push_str(&format!("## heartbeat {}\n", NOW - 3));
    let log = aged_log(&dir, "mid-write.log", &body, 1);

    let out = run(&[
        "observe",
        "step",
        log.to_str().expect("utf8 path"),
        "--now",
        &NOW.to_string(),
        "--interval",
        "60",
    ]);
    let text = stdout(&out);
    assert_eq!(code(&out), 0, "stdout {text} stderr {}", stderr(&out));
    assert!(text.contains("status=running"), "got: {text}");
    assert!(!text.contains("stalled"), "got: {text}");
    // Recency is reported as an observation, never as the status.
    assert!(text.contains("recency, not status"), "got: {text}");
}

/// AC2: a stale heartbeat says `stalled`, and it is the heartbeat that says
/// so — the same file with a fresh beat is `running`.
#[test]
fn a_stale_heartbeat_is_stalled_and_exit_is_nonzero() {
    let dir = temp_dir("observe");
    let body = format!(
        "## conversion started\n## heartbeat {}\n## progress 10 elapsed 30s\n",
        NOW - 600
    );
    let log = aged_log(&dir, "stale.log", &body, 600);

    let stalled = run(&[
        "observe",
        "step",
        log.to_str().expect("utf8 path"),
        "--now",
        &NOW.to_string(),
        "--interval",
        "60",
    ]);
    assert_eq!(code(&stalled), 1);
    assert!(stdout(&stalled).contains("status=stalled"));
}

/// A finished step is `complete` even while silent: the terminal marker wins
/// over the absence of a beat.
#[test]
fn a_terminal_marker_makes_silence_complete() {
    let dir = temp_dir("observe");
    let body = format!("## conversion started\n{}\n", "######## complete ########");
    let log = aged_log(&dir, "done.log", &body, 7_200);

    let out = run(&[
        "observe",
        "step",
        log.to_str().expect("utf8 path"),
        "--now",
        &NOW.to_string(),
    ]);
    assert_eq!(code(&out), 0);
    assert!(
        stdout(&out).contains("status=complete"),
        "got {}",
        stdout(&out)
    );
}

/// A step that never annotated its progress has an unknown status, never
/// `stalled`, however old its last byte is.
#[test]
fn a_step_without_heartbeats_is_unknown_never_stalled() {
    let dir = temp_dir("observe");
    let log = aged_log(&dir, "quiet.log", "## started\nworking\n", 43_200);

    let out = run(&[
        "observe",
        "step",
        log.to_str().expect("utf8 path"),
        "--now",
        &NOW.to_string(),
    ]);
    let text = stdout(&out);
    assert_eq!(code(&out), 0, "got: {text}");
    assert!(text.contains("status=unknown"), "got: {text}");
    assert!(!text.contains("stalled"), "got: {text}");
    assert!(
        text.contains("recency, not evidence of state"),
        "got: {text}"
    );
}

/// AC2 (JSON): the machine form carries the status and separates the two
/// kinds of statement.
#[test]
fn step_json_separates_observed_from_inferred() {
    let dir = temp_dir("observe");
    let body = format!(
        "## started\n## heartbeat {}\n## progress 1 elapsed 5s\n",
        NOW - 3
    );
    let log = aged_log(&dir, "json.log", &body, 3);

    let out = run(&[
        "observe",
        "step",
        log.to_str().expect("utf8 path"),
        "--now",
        &NOW.to_string(),
        "--json",
    ]);
    assert_eq!(code(&out), 0, "stderr {}", stderr(&out));
    let value: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("json");
    assert_eq!(value["status"], "running");
    assert!(value["observed"].as_array().expect("observed").len() >= 2);
    assert_eq!(value["inferred"].as_array().expect("inferred").len(), 1);
}

/// AC3: one glance does not authorize a destructive action; two independent
/// sources, or the terminal marker, do.
#[test]
fn a_destructive_action_needs_the_marker_or_two_independent_looks() {
    let one = run(&["observe", "gate", "--action", "stop", "--source", "pgrep"]);
    assert_eq!(code(&one), 1);
    assert!(stdout(&one).starts_with("held:"), "got {}", stdout(&one));

    // Three pgrep samples are one source: the repeat look adds nothing.
    let three_same = run(&[
        "observe", "gate", "--action", "stop", "--source", "pgrep", "--source", "pgrep",
        "--source", "pgrep",
    ]);
    assert_eq!(code(&three_same), 1);

    let two = run(&[
        "observe",
        "gate",
        "--action",
        "stop",
        "--source",
        "pgrep",
        "--source",
        "log:topup",
    ]);
    assert_eq!(code(&two), 0);
    assert!(
        stdout(&two).starts_with("authorized:"),
        "got {}",
        stdout(&two)
    );

    let marker = run(&["observe", "gate", "--action", "archive", "--terminal"]);
    assert_eq!(code(&marker), 0);
    assert!(stdout(&marker).contains("terminal marker"));
}

/// AC4: the characterisation is computed from counts over the whole source.
/// The log here opened 86 pull requests; its last two lines show none.
#[test]
fn yield_is_counted_over_the_source_and_a_tail_is_refused() {
    let dir = temp_dir("observe");
    let mut body = String::new();
    for index in 1..=86 {
        body.push_str(&format!(
            "pr-open 17000000{index:02} opened# https://gh/14{index:02}\n"
        ));
    }
    body.push_str("idle wait for next issue\nidle wait for next issue\n");
    let log = dir.join("dispatch.log");
    fs::write(&log, body).expect("write log");
    let path = log.to_str().expect("utf8 path");

    let whole = run(&[
        "observe",
        "count",
        path,
        "--pattern",
        "opened#",
        "--label",
        "prs",
        "--characterise",
        "low yield",
    ]);
    let text = stdout(&whole);
    assert_eq!(code(&whole), 0, "got: {text} stderr {}", stderr(&whole));
    assert!(text.contains("prs=86"), "got: {text}");
    assert!(text.contains("low yield"), "got: {text}");

    // The same grep over the tail — the reading that stopped a 21-hour run.
    let tail = run(&[
        "observe",
        "count",
        path,
        "--pattern",
        "opened#",
        "--label",
        "prs",
        "--tail",
        "2",
        "--characterise",
        "low yield",
    ]);
    assert_eq!(code(&tail), 2, "a tail must not carry a characterisation");
    assert!(
        stderr(&tail).contains("count the whole source"),
        "stderr {}",
        stderr(&tail)
    );

    // Reporting a tail without characterising it stays legal, and says so.
    let tail_only = run(&[
        "observe",
        "count",
        path,
        "--pattern",
        "opened#",
        "--tail",
        "2",
    ]);
    assert_eq!(code(&tail_only), 0);
    assert!(
        stdout(&tail_only).contains("not evidence"),
        "got {}",
        stdout(&tail_only)
    );
}

/// The two-sample rule, applied by the tool: one sample describes nothing.
#[test]
fn one_sample_is_unknown_and_two_across_a_gap_are_moving() {
    let dir = temp_dir("observe");
    let log = dir.join("append.log");
    fs::write(&log, "line one\n").expect("write log");
    let path = log.to_str().expect("utf8 path");

    let once = run(&["observe", "growth", path, "--samples", "1"]);
    assert_eq!(code(&once), 0);
    assert!(
        stdout(&once).contains("motion unknown"),
        "got {}",
        stdout(&once)
    );

    // Two looks at the same instant are one look.
    let no_gap = run(&[
        "observe",
        "growth",
        path,
        "--samples",
        "2",
        "--gap-secs",
        "0",
    ]);
    assert!(
        stdout(&no_gap).contains("motion unknown"),
        "got {}",
        stdout(&no_gap)
    );

    // A second line lands during the gap, so the source is moving.
    let appending = {
        let path = path.to_string();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(500));
            fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .and_then(|mut file| {
                    use std::io::Write;
                    file.write_all(b"line two\n")
                })
                .expect("append")
        })
    };
    let moving = run(&[
        "observe",
        "growth",
        path,
        "--samples",
        "2",
        "--gap-secs",
        "2",
    ]);
    appending.join().expect("appender");
    assert!(
        stdout(&moving).contains("motion moving"),
        "got {}",
        stdout(&moving)
    );
}

/// AC1 in the tool's own vocabulary: a step that annotates its work is
/// readable while it runs, and its report never states a conclusion the
/// observations above it do not support.
#[test]
fn every_report_line_is_observed_or_inferred() {
    let dir = temp_dir("observe");
    let body = format!(
        "## started\n## heartbeat {}\n## progress 1 elapsed 5s\n",
        NOW - 3
    );
    let log = aged_log(&dir, "annotated.log", &body, 3);

    let out = run(&[
        "observe",
        "step",
        log.to_str().expect("utf8 path"),
        "--now",
        &NOW.to_string(),
    ]);
    let text = stdout(&out);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(code(&out), 0);
    assert_eq!(
        lines.last().copied(),
        Some("status=running"),
        "got: {lines:?}"
    );
    assert!(
        lines[..lines.len() - 1]
            .iter()
            .all(|line| line.starts_with("OBSERVED ") || line.starts_with("INFERRED ")),
        "got: {lines:?}"
    );
    let first_inferred = lines
        .iter()
        .position(|line| line.starts_with("INFERRED "))
        .expect("one inferred line");
    assert!(
        first_inferred > 0,
        "an inference must follow an observation: {lines:?}"
    );
}
