//! `autospec dispatch` CLI (#3800): the exit codes a cron wrapper branches on.
//!
//! The bug these tests pin is a silent one — the queue stopped being
//! repopulated when the session that ran the refresher died, and every later
//! consumer read the untouched file and reported "no work". Each test here
//! therefore asserts on the *exit code* and on the named hop, because those are
//! the two things a wrapper can act on.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Output;

#[path = "support/temp_directory.rs"]
mod temp_directory;
use temp_directory::unique as temp_dir;

const NOW: u64 = 1_800_000_000;

/// A temp checkout of `$HOME/.autospec` with the artifact and ledger paths
/// pinned, so no test ever writes through to the operator's real state.
struct Harness {
    temp: PathBuf,
    queue: PathBuf,
    state: PathBuf,
}

impl Harness {
    fn new(tag: &str) -> Self {
        let temp = temp_dir(tag);
        Self {
            queue: temp.join("queue.txt"),
            state: temp.join("dispatch-liveness.json"),
            temp,
        }
    }

    /// `args[0]` is the subcommand; global options are appended after it.
    fn dispatch(&self, args: &[&str]) -> Output {
        let mut argv: Vec<String> = vec!["dispatch".to_string()];
        argv.extend(args.iter().map(|arg| arg.to_string()));
        argv.extend(
            [
                "--queue",
                &self.queue.display().to_string(),
                "--state-file",
                &self.state.display().to_string(),
            ]
            .iter()
            .map(|arg| arg.to_string()),
        );
        run(&argv)
    }

    fn write_queue(&self, text: &str) {
        std::fs::write(&self.queue, text).expect("queue written");
    }

    fn read_queue(&self) -> String {
        std::fs::read_to_string(&self.queue).expect("queue readable")
    }

    fn write_topology(&self, text: &str) -> PathBuf {
        let path = self.temp.join("topology.json");
        std::fs::write(&path, text).expect("topology written");
        path
    }
}

fn run(argv: &[String]) -> Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_autospec"))
        .args(argv)
        .output()
        .expect("autospec runs")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

fn stamped(age_secs: u64, entries: &[&str]) -> String {
    let mut text = format!(
        "# refreshed-at: {}\n# refreshed-by: refresh-queue\n",
        NOW - age_secs
    );
    for entry in entries {
        text.push_str(entry);
        text.push('\n');
    }
    text
}

/// Beat every hop the reference topology expects a heartbeat from.
fn beat_all_scheduled_hops(harness: &Harness) {
    for hop in ["refresh-queue", "topup", "dispatch-agent"] {
        let output = harness.dispatch(&["beat", "--step", hop, "--at", &NOW.to_string()]);
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    }
}

#[test]
fn dispatch_help_lists_the_four_subcommands() {
    let output = run(&[
        "dispatch".to_string(),
        "--help".to_string(),
        "unused".to_string(),
    ]);

    assert_eq!(output.status.code(), Some(0));
    let help = stdout(&output);
    for subcommand in ["check", "stamp", "beat", "status"] {
        assert!(help.contains(subcommand), "{help}");
    }
    assert!(help.contains("EXIT CODES"), "{help}");
}

#[test]
fn unknown_subcommand_is_a_diagnostic_not_a_verdict() {
    let harness = Harness::new("autospec-dispatch-unknown");

    let output = harness.dispatch(&["nope"]);
    assert_eq!(output.status.code(), Some(2));
    let message = stderr(&output);
    assert!(message.contains("unknown dispatch subcommand"), "{message}");
    assert!(message.contains("check, stamp, beat, status"), "{message}");
}

// ── check: the consumer's gate ──────────────────────────────────────────────

#[test]
fn check_holds_when_the_queue_artifact_is_absent() {
    let harness = Harness::new("autospec-dispatch-absent");

    let output = harness.dispatch(&["check", "--now", &NOW.to_string()]);
    assert_eq!(output.status.code(), Some(1), "{}", stdout(&output));
    let line = stdout(&output);
    assert!(line.contains("LIVENESS FAILURE [QUEUE_MISSING]"), "{line}");
}

#[test]
fn check_holds_an_unstamped_queue_instead_of_calling_it_no_work() {
    let harness = Harness::new("autospec-dispatch-unstamped");
    // Content the consumer cannot date: exactly the frozen-queue shape.
    harness.write_queue("44\n45\n");

    let output = harness.dispatch(&["check", "--now", &NOW.to_string()]);
    assert_eq!(output.status.code(), Some(1));
    let line = stdout(&output);
    assert!(line.contains("QUEUE_UNSTAMPED"), "{line}");
    assert!(line.contains("refresh-queue"), "{line}");
    assert!(!line.contains("no new issues"), "{line}");
}

#[test]
fn check_holds_a_stale_stamp_and_names_the_intervals_and_step() {
    let harness = Harness::new("autospec-dispatch-stale");
    harness.write_queue(&stamped(2_400, &["12"]));

    let output = harness.dispatch(&["check", "--now", &NOW.to_string()]);
    assert_eq!(output.status.code(), Some(1));
    let line = stdout(&output);
    assert!(line.contains("STAMP_NOT_REFRESHED"), "{line}");
    assert!(line.contains("refresh-queue"), "{line}");
    assert!(line.contains("4 intervals"), "{line}");
    assert!(!line.contains("no new issues"), "{line}");
}

#[test]
fn check_proceeds_on_a_fresh_stamp_and_reports_idle_when_genuinely_empty() {
    let harness = Harness::new("autospec-dispatch-fresh");

    harness.write_queue(&stamped(30, &["12", "13"]));
    let work = harness.dispatch(&["check", "--now", &NOW.to_string()]);
    assert_eq!(work.status.code(), Some(0), "{}", stdout(&work));
    let work = stdout(&work);
    assert!(work.contains("queue ready: 2 entries"), "{work}");

    // Same gate, now truly nothing filed: still exit 0, and it says so.
    harness.write_queue(&stamped(60, &[]));
    let idle = harness.dispatch(&["check", "--now", &NOW.to_string()]);
    assert_eq!(idle.status.code(), Some(0));
    let idle = stdout(&idle);
    assert!(idle.contains("queue idle"), "{idle}");
    assert!(idle.contains("no new issues were filed"), "{idle}");
}

#[test]
fn check_json_reports_the_hold_code_for_a_wrapper() {
    let harness = Harness::new("autospec-dispatch-check-json");

    let output = harness.dispatch(&["check", "--now", &NOW.to_string(), "--json"]);
    assert_eq!(output.status.code(), Some(1));
    let json = stdout(&output);
    assert!(json.contains("\"QUEUE_MISSING\""), "{json}");
    assert!(json.contains("refresh-queue"), "{json}");
}

#[test]
fn check_holds_when_the_stamp_is_in_the_future() {
    let harness = Harness::new("autospec-dispatch-future");
    harness.write_queue(&format!("# refreshed-at: {}\n12\n", NOW + 300));

    let output = harness.dispatch(&["check", "--now", &NOW.to_string()]);
    assert_eq!(output.status.code(), Some(1));
    let line = stdout(&output);
    assert!(line.contains("CLOCK_REWIND"), "{line}");
    assert!(line.contains("300s in the future"), "{line}");
}

// ── stamp / beat: the liveness writers ──────────────────────────────────────

#[test]
fn stamp_writes_the_freshness_headers_and_beats_for_the_producer() {
    let harness = Harness::new("autospec-dispatch-stamp");
    harness.write_queue("44\n45\n");

    let output = harness.dispatch(&["stamp", "--at", &NOW.to_string()]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let line = stdout(&output);
    assert!(line.contains("refreshed-by refresh-queue"), "{line}");
    assert!(line.contains("2 entries"), "{line}");

    let artifact = harness.read_queue();
    assert!(
        artifact.contains(&format!("# refreshed-at: {NOW}")),
        "{artifact}"
    );
    assert!(
        artifact.contains("# refreshed-by: refresh-queue"),
        "{artifact}"
    );
    assert!(artifact.contains("44\n"), "{artifact}");

    // The stamp is what turns the same artifact from a hold into work.
    let check = harness.dispatch(&["check", "--now", &NOW.to_string()]);
    assert_eq!(check.status.code(), Some(0), "{}", stdout(&check));

    // And the producer hop is live in the ledger.
    let ledger = std::fs::read_to_string(&harness.state).expect("ledger written");
    assert!(ledger.contains("refresh-queue"), "{ledger}");
}

#[test]
fn stamp_creates_the_artifact_when_the_producer_had_nothing_to_list() {
    let harness = Harness::new("autospec-dispatch-stamp-empty");

    let output = harness.dispatch(&["stamp", "--at", &NOW.to_string(), "--by", "refresh-queue"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));

    let check = harness.dispatch(&["check", "--now", &NOW.to_string()]);
    assert_eq!(check.status.code(), Some(0));
    assert!(stdout(&check).contains("queue idle"), "{}", stdout(&check));
}

#[test]
fn beat_records_a_hop_and_status_then_reads_healthy() {
    let harness = Harness::new("autospec-dispatch-beat");

    let output = harness.dispatch(&["beat", "--step", "topup", "--at", &NOW.to_string()]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(stdout(&output).contains("hop topup"), "{}", stdout(&output));

    // A re-run at an older instant must not erase the newer beat.
    harness.dispatch(&["beat", "--step", "topup", "--at", &(NOW - 500).to_string()]);
    let ledger = std::fs::read_to_string(&harness.state).expect("ledger written");
    assert!(ledger.contains(&NOW.to_string()), "{ledger}");
    assert!(!ledger.contains(&(NOW - 500).to_string()), "{ledger}");
}

#[test]
fn beat_without_a_step_is_a_diagnostic() {
    let harness = Harness::new("autospec-dispatch-beat-nostep");

    let output = harness.dispatch(&["beat"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("--step"), "{}", stderr(&output));
}

#[test]
fn beat_rejects_a_hop_name_that_is_not_a_plain_token() {
    let harness = Harness::new("autospec-dispatch-badname");

    let output = harness.dispatch(&["beat", "--step", "../escape"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("invalid hop name"),
        "{}",
        stderr(&output)
    );
    assert!(!harness.state.exists(), "no ledger may be written");
}

// ── status: the whole chain ─────────────────────────────────────────────────

#[test]
fn status_reports_every_hop_and_exits_zero_when_the_chain_is_alive() {
    let harness = Harness::new("autospec-dispatch-status-healthy");
    harness.write_queue(&stamped(30, &["12"]));
    beat_all_scheduled_hops(&harness);

    let output = harness.dispatch(&["status", "--now", &NOW.to_string()]);
    assert_eq!(output.status.code(), Some(0), "{}", stdout(&output));
    let report = stdout(&output);
    assert!(
        report.contains("credential-holding steps: file-issue@authenticated, refresh-queue@authenticated, topup@authenticated"),
        "{report}"
    );
    for hop in ["file-issue", "refresh-queue", "topup", "dispatch-agent"] {
        assert!(report.contains(&format!("hop {hop}:")), "{report}");
    }
    // The hop that only ever runs from a session is not called out for silence.
    assert!(report.contains("file-issue: EVENT-DRIVEN"), "{report}");
    assert!(!report.contains("DEFECT"), "{report}");
}

#[test]
fn status_names_the_silent_hop_and_exits_one() {
    let harness = Harness::new("autospec-dispatch-status-silent");
    harness.write_queue(&stamped(30, &["12"]));
    // dispatch-agent stopped answering; the others are fine.
    for hop in ["refresh-queue", "topup"] {
        harness.dispatch(&["beat", "--step", hop, "--at", &NOW.to_string()]);
    }

    let output = harness.dispatch(&["status", "--now", &NOW.to_string()]);
    assert_eq!(output.status.code(), Some(1));
    let report = stdout(&output);
    assert!(
        report.contains("hop dispatch-agent: NEVER BEAT"),
        "{report}"
    );
    assert!(report.contains("hop topup: LIVE"), "{report}");
}

#[test]
fn status_holds_when_only_the_artifact_is_stale() {
    let harness = Harness::new("autospec-dispatch-status-stale");
    harness.write_queue(&stamped(10_000, &["12"]));
    beat_all_scheduled_hops(&harness);

    let output = harness.dispatch(&["status", "--now", &NOW.to_string()]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stdout(&output).contains("STAMP_NOT_REFRESHED"),
        "{}",
        stdout(&output)
    );
}

#[test]
fn status_reads_a_declared_topology_and_reports_its_defects() {
    let harness = Harness::new("autospec-dispatch-status-topology");
    harness.write_queue(&stamped(30, &["12"]));
    // A hand-written deployment topology: the refresher holds a gh token on
    // cluster-shared storage, and nothing produces the queue's consumer input.
    let topology = harness.write_topology(
        r#"{"steps":[{"name":"refresh-queue","host":"shared-cluster","credential":"gh-token","schedule":{"scheduled":{"interval_secs":600}},"produces":"queue.txt","consumes":[],"log":"~/.autospec/logs/refresh-queue.log"}]}"#,
    );

    let output = harness.dispatch(&[
        "status",
        "--now",
        &NOW.to_string(),
        "--topology",
        topology.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(output.status.code(), Some(1), "{}", stdout(&output));
    let json = stdout(&output);
    assert!(
        json.contains("credential-on-shared-storage"),
        "declared credential topology must be in the report: {json}"
    );
    assert!(json.contains("\"hops\""), "{json}");

    // The same defect, spelled for a human reading the cron mail.
    let human = harness.dispatch(&[
        "status",
        "--now",
        &NOW.to_string(),
        "--topology",
        topology.to_str().unwrap(),
    ]);
    assert_eq!(human.status.code(), Some(1));
    assert!(
        stdout(&human).contains("TOPOLOGY DEFECT [CREDENTIAL_ON_SHARED_STORAGE]"),
        "{}",
        stdout(&human)
    );
}

#[test]
fn status_rejects_a_topology_file_that_does_not_parse() {
    let harness = Harness::new("autospec-dispatch-status-badtopology");
    let topology = harness.write_topology("{ not json");

    let output = harness.dispatch(&["status", "--topology", topology.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("does not parse"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn status_refuses_to_overwrite_a_corrupt_ledger() {
    let harness = Harness::new("autospec-dispatch-status-badledger");
    std::fs::write(&harness.state, "truncated").expect("ledger written");

    let output = harness.dispatch(&["status", "--now", &NOW.to_string()]);
    assert_eq!(output.status.code(), Some(2));
    let message = stderr(&output);
    assert!(message.contains("does not parse"), "{message}");
    assert!(
        message.contains("refusing to start a fresh one"),
        "{message}"
    );
}

// ── Default paths under $HOME ───────────────────────────────────────────────

#[test]
fn default_paths_resolve_under_home_autospec() {
    let harness = Harness::new("autospec-dispatch-home");
    let home = harness.temp.join("home");
    std::fs::create_dir_all(&home).expect("home created");
    let managed: PathBuf = home.join(".autospec");

    let missing = run_home(&home, &["dispatch", "check", "--now", &NOW.to_string()]);
    assert_eq!(missing.status.code(), Some(1), "{}", stdout(&missing));
    assert!(
        stdout(&missing).contains("QUEUE_MISSING"),
        "{}",
        stdout(&missing)
    );
    assert!(!managed.exists(), "check must not create state");

    let stamp = run_home(&home, &["dispatch", "stamp", "--at", &NOW.to_string()]);
    assert_eq!(stamp.status.code(), Some(0), "{}", stderr(&stamp));
    assert!(queue_exists(&managed), "stamp must create {managed:?}");
    assert!(managed.join("dispatch-liveness.json").exists());
    assert_0600_or_owner_readable(&managed.join("queue.txt"));
}

fn run_home(home: &Path, argv: &[&str]) -> Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_autospec"))
        .args(
            argv.iter()
                .map(|arg| arg.to_string())
                .collect::<Vec<String>>(),
        )
        .env("HOME", home)
        .output()
        .expect("autospec runs")
}

fn queue_exists(managed: &Path) -> bool {
    managed.join("queue.txt").exists()
}

/// The artifact is only read back by the same user; assert it is readable
/// rather than asserting an exact mode, which the umask may tighten.
fn assert_0600_or_owner_readable(path: &Path) {
    let metadata = std::fs::metadata(path).expect("metadata");
    let mode = metadata.permissions().mode();
    assert!(mode & 0o400 != 0, "owner must be able to read {path:?}");
}
