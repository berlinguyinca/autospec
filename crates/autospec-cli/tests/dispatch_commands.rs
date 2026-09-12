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
fn dispatch_help_lists_the_subcommands() {
    let output = run(&[
        "dispatch".to_string(),
        "--help".to_string(),
        "unused".to_string(),
    ]);

    assert_eq!(output.status.code(), Some(0));
    let help = stdout(&output);
    for subcommand in ["check", "reconcile", "guard", "stamp", "beat", "status"] {
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
    assert!(
        message.contains("check, reconcile, guard, stamp, beat, status, stage, freshness"),
        "{message}"
    );
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

// ── guard: the pre-dispatch gate against unconverted output (#3764) ────────

fn guard_out_dir(harness: &Harness) -> PathBuf {
    let out = harness.temp.join("out");
    std::fs::create_dir_all(&out).expect("out dir created");
    out
}

#[test]
fn guard_holds_when_an_unconverted_patch_exists_and_touches_nothing() {
    let harness = Harness::new("autospec-dispatch-guard-hold");
    let out = guard_out_dir(&harness);
    let issue_dir = out.join("issue-1234");
    std::fs::create_dir_all(&issue_dir).expect("issue dir");
    let patch = issue_dir.join("changes.patch");
    std::fs::write(&patch, "diff --git a/x b/x\n+fix\n").expect("patch written");

    let output = harness.dispatch(&[
        "guard",
        "--issue",
        "1234",
        "--out-dir",
        &out.display().to_string(),
    ]);
    assert_eq!(output.status.code(), Some(1), "{}", stdout(&output));
    let line = stdout(&output);
    assert!(line.contains("DISPATCH issue 1234 HELD"), "{line}");
    assert!(line.contains("unconverted patch exists"), "{line}");
    assert!(line.contains("changes.patch"), "{line}");
    // A held guard touches nothing: the patch must survive the dispatch.
    assert!(patch.exists(), "a held guard must not destroy the patch");
    assert!(issue_dir.exists());
}

#[test]
fn guard_dry_run_reports_every_check_and_the_verdict_without_mutation() {
    let harness = Harness::new("autospec-dispatch-guard-dryrun");
    let out = guard_out_dir(&harness);
    let issue_dir = out.join("issue-42");
    std::fs::create_dir_all(&issue_dir).expect("issue dir");
    let patch = issue_dir.join("changes.patch");
    std::fs::write(&patch, "diff\n").expect("patch written");

    // Patch present: the dry run names the check, the evidence and the hold.
    let held = harness.dispatch(&[
        "guard",
        "--issue",
        "42",
        "--out-dir",
        &out.display().to_string(),
        "--dry-run",
    ]);
    assert_eq!(held.status.code(), Some(1), "{}", stdout(&held));
    let report = stdout(&held);
    assert!(
        report.contains("check unconverted_patch: DANGEROUS"),
        "{report}"
    );
    assert!(report.contains("DISPATCH issue 42 HELD"), "{report}");
    assert!(patch.exists(), "--dry-run must not mutate the directory");

    // Patch gone: the same command reports clear and authorized, exit 0 —
    // and still does not remove the directory, because it was asked to look,
    // not to act.
    std::fs::remove_file(&patch).expect("patch removed");
    let clear = harness.dispatch(&[
        "guard",
        "--issue",
        "42",
        "--out-dir",
        &out.display().to_string(),
        "--dry-run",
    ]);
    assert_eq!(clear.status.code(), Some(0), "{}", stdout(&clear));
    let report = stdout(&clear);
    assert!(
        report.contains("check unconverted_patch: clear"),
        "{report}"
    );
    assert!(report.contains("DISPATCH issue 42 authorized"), "{report}");
    assert!(
        issue_dir.exists(),
        "--dry-run must not remove the directory"
    );
}

#[test]
fn guard_removes_stale_output_only_when_the_patch_is_gone() {
    let harness = Harness::new("autospec-dispatch-guard-clean");
    let out = guard_out_dir(&harness);
    let issue_dir = out.join("issue-7");
    let debris = issue_dir.join("logs");
    std::fs::create_dir_all(&debris).expect("debris dir");
    std::fs::write(debris.join("run.log"), "old run").expect("debris written");

    let output = harness.dispatch(&[
        "guard",
        "--issue",
        "7",
        "--out-dir",
        &out.display().to_string(),
    ]);
    assert_eq!(output.status.code(), Some(0), "{}", stdout(&output));
    let report = stdout(&output);
    assert!(report.contains("DISPATCH issue 7 authorized"), "{report}");
    assert!(report.contains("removed stale output"), "{report}");
    assert!(!issue_dir.exists(), "stale output must be gone");

    // Absent directory: still authorized, and it says so rather than failing.
    let again = harness.dispatch(&[
        "guard",
        "--issue",
        "7",
        "--out-dir",
        &out.display().to_string(),
    ]);
    assert_eq!(again.status.code(), Some(0), "{}", stdout(&again));
    assert!(
        stdout(&again).contains("nothing to remove"),
        "{}",
        stdout(&again)
    );
}

#[test]
fn guard_fails_closed_when_the_check_cannot_answer() {
    let harness = Harness::new("autospec-dispatch-guard-failclosed");
    let out = guard_out_dir(&harness);
    // `issue-7` is a regular file, so `issue-7/changes.patch` is a stat error,
    // not a clean "not found": the check cannot answer. The dispatcher that
    // read that error as "no patch" is exactly the #3764 failure, so the
    // guard must hold, not authorize.
    let issue_file = out.join("issue-7");
    std::fs::write(&issue_file, "not a directory").expect("file written");

    let output = harness.dispatch(&[
        "guard",
        "--issue",
        "7",
        "--out-dir",
        &out.display().to_string(),
    ]);
    assert_eq!(output.status.code(), Some(1), "{}", stdout(&output));
    let line = stdout(&output);
    assert!(line.contains("DISPATCH issue 7 HELD"), "{line}");
    assert!(line.contains("check unconverted_patch failed"), "{line}");
    assert!(
        line.contains("a check that cannot answer is unsafe"),
        "{line}"
    );
    // And it must not have destroyed the thing it could not inspect.
    assert!(
        issue_file.exists(),
        "a failed check must not authorize removal"
    );
}

#[test]
fn guard_json_reports_the_verdict_and_the_evidence() {
    let harness = Harness::new("autospec-dispatch-guard-json");
    let out = guard_out_dir(&harness);
    let issue_dir = out.join("issue-9");
    std::fs::create_dir_all(&issue_dir).expect("issue dir");
    let patch = issue_dir.join("changes.patch");
    std::fs::write(&patch, "diff\n").expect("patch written");

    let output = harness.dispatch(&[
        "guard",
        "--issue",
        "9",
        "--out-dir",
        &out.display().to_string(),
        "--json",
    ]);
    assert_eq!(output.status.code(), Some(1), "{}", stdout(&output));
    let json = stdout(&output);
    assert!(json.contains("\"unconverted_patch\""), "{json}");
    assert!(json.contains("\"dangerous\""), "{json}");
    assert!(json.contains("\"hold\""), "{json}");
    assert!(json.contains("changes.patch"), "{json}");
}

#[test]
fn guard_rejects_non_numeric_issues_and_traversal_patch_names() {
    let harness = Harness::new("autospec-dispatch-guard-validation");

    let bad_issue = harness.dispatch(&[
        "guard",
        "--issue",
        "../x",
        "--out-dir",
        &harness.temp.display().to_string(),
    ]);
    assert_eq!(bad_issue.status.code(), Some(2));
    assert!(
        stderr(&bad_issue).contains("positive integer"),
        "{}",
        stderr(&bad_issue)
    );

    let bad_name = harness.dispatch(&[
        "guard",
        "--issue",
        "3",
        "--out-dir",
        &harness.temp.display().to_string(),
        "--patch-name",
        "../changes.patch",
    ]);
    assert_eq!(bad_name.status.code(), Some(2));
    assert!(
        stderr(&bad_name).contains("plain file name"),
        "{}",
        stderr(&bad_name)
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

// ── #3927: the admission reconciliation ────────────────────────────────
//
// The label set is authoritative; the queue is a derived copy. These pin the
// CLI surface that keeps the two honest: `reconcile` reports the count of
// admitted-but-unschedulable issues (zero is the expected answer, nonzero a
// failure), and `check --admitted-file` turns an idle queue over filed work
// into a named fault instead of a silent idle.

fn write_admitted(harness: &Harness, text: &str) -> String {
    let path = harness.temp.join("admitted.txt");
    std::fs::write(&path, text).expect("admitted written");
    path.display().to_string()
}

#[test]
fn reconcile_clean_exits_zero() {
    let harness = Harness::new("autospec-dispatch-reconcile-clean");
    harness.write_queue(&stamped(0, &["10", "11", "12"]));
    let admitted = write_admitted(&harness, "10\n11\n12\n");

    let output = harness.dispatch(&["reconcile", "--admitted-file", &admitted]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{} {}",
        stderr(&output),
        stdout(&output)
    );
    assert!(
        stdout(&output).contains("RECONCILE clean"),
        "{}",
        stdout(&output)
    );
}

#[test]
fn reconcile_defect_exits_one_and_names_the_missing_issues() {
    let harness = Harness::new("autospec-dispatch-reconcile-defect");
    harness.write_queue(&stamped(0, &["10", "11", "12"]));
    let admitted = write_admitted(&harness, "10\n11\n12\n99\n100\n");

    let output = harness.dispatch(&["reconcile", "--admitted-file", &admitted]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "{} {}",
        stderr(&output),
        stdout(&output)
    );
    let out = stdout(&output);
    assert!(out.contains("RECONCILE DEFECT"), "{out}");
    assert!(out.contains("99"), "{out}");
    assert!(out.contains("100"), "{out}");
}

#[test]
fn reconcile_missing_admitted_file_is_a_diagnostic_not_a_verdict() {
    let harness = Harness::new("autospec-dispatch-reconcile-missing");
    harness.write_queue(&stamped(0, &["10"]));
    let absent = harness.temp.join("absent.txt");

    let output = harness.dispatch(&[
        "reconcile",
        "--admitted-file",
        &absent.display().to_string(),
    ]);
    assert_eq!(
        output.status.code(),
        Some(2),
        "{} {}",
        stderr(&output),
        stdout(&output)
    );
    assert!(
        stderr(&output).contains("cannot read admitted file"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn reconcile_reports_stale_queue_entries_without_failing() {
    let harness = Harness::new("autospec-dispatch-reconcile-stale");
    harness.write_queue(&stamped(0, &["10", "11", "42"]));
    // 42 is in the queue but no longer admitted: stale, reported, not the defect.
    let admitted = write_admitted(&harness, "10\n11\n");

    let output = harness.dispatch(&["reconcile", "--admitted-file", &admitted]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{} {}",
        stderr(&output),
        stdout(&output)
    );
    let out = stdout(&output);
    assert!(out.contains("RECONCILE clean with 1 stale"), "{out}");
    assert!(out.contains("42"), "{out}");
}

#[test]
fn check_with_admitted_file_holds_an_idle_queue_over_filed_work() {
    let harness = Harness::new("autospec-dispatch-check-admitted-idle");
    harness.write_queue(&stamped(0, &[])); // fresh, empty, stamped
    let admitted = write_admitted(&harness, "99\n100\n");

    let output = harness.dispatch(&[
        "check",
        "--admitted-file",
        &admitted,
        "--now",
        &NOW.to_string(),
    ]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "{} {}",
        stderr(&output),
        stdout(&output)
    );
    let out = stdout(&output);
    assert!(out.contains("ADMITTED_NOT_SCHEDULABLE"), "{out}");
    assert!(out.contains("99"), "{out}");
}

#[test]
fn check_with_admitted_file_proceeds_when_the_queue_is_populated() {
    let harness = Harness::new("autospec-dispatch-check-admitted-populated");
    harness.write_queue(&stamped(0, &["10", "11"]));
    let admitted = write_admitted(&harness, "10\n11\n");

    let output = harness.dispatch(&[
        "check",
        "--admitted-file",
        &admitted,
        "--now",
        &NOW.to_string(),
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{} {}",
        stderr(&output),
        stdout(&output)
    );
    assert!(
        stdout(&output).contains("DISPATCH queue ready"),
        "{}",
        stdout(&output)
    );
}

#[test]
fn check_without_admitted_file_still_reads_an_empty_queue_as_idle() {
    let harness = Harness::new("autospec-dispatch-check-no-admitted");
    harness.write_queue(&stamped(0, &[]));

    let output = harness.dispatch(&["check", "--now", &NOW.to_string()]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{} {}",
        stderr(&output),
        stdout(&output)
    );
    assert!(
        stdout(&output).contains("DISPATCH queue idle"),
        "{}",
        stdout(&output)
    );
}

// ── #3784: failed-run artifact archiving and classified holds ─────────────

#[test]
fn guard_archives_failed_run_and_frees_the_dispatch_slot() {
    let harness = Harness::new("autospec-dispatch-guard-archive-build-fail");
    let out = guard_out_dir(&harness);
    let issue_dir = out.join("issue-5678");
    std::fs::create_dir_all(&issue_dir).expect("issue dir");
    let patch = issue_dir.join("changes.patch");
    std::fs::write(&patch, "diff --git a/x b/x\n+fix\n").expect("patch written");
    // Gate-shape status.txt recording a failed build.
    std::fs::write(issue_dir.join("status.txt"), "status=BUILD-FAIL\n").expect("status.txt");

    let output = harness.dispatch(&[
        "guard",
        "--issue",
        "5678",
        "--out-dir",
        &out.display().to_string(),
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{} {}",
        stdout(&output),
        stderr(&output)
    );
    let report = stdout(&output);
    assert!(
        report.contains("archiving failed run (status: BUILD-FAIL)"),
        "{report}"
    );
    assert!(report.contains("  archived to "), "{report}");
    // The issue directory is moved (renamed) to the archive: it must be gone.
    assert!(
        !issue_dir.exists(),
        "archived issue dir must no longer exist"
    );
    assert!(!patch.exists(), "patch inside archived dir must be gone");
    // The archive root must exist with something in it.
    let archive = out.join("archive");
    assert!(archive.exists(), "archive dir created");
    let entries: Vec<_> = std::fs::read_dir(&archive)
        .expect("archive readable")
        .filter_map(|e| e.ok())
        .collect();
    assert_eq!(entries.len(), 1, "one archive entry");
}

#[test]
fn guard_holds_for_fmt_dirty_with_awaiting_conversion_reason() {
    let harness = Harness::new("autospec-dispatch-guard-fmt-dirty");
    let out = guard_out_dir(&harness);
    let issue_dir = out.join("issue-5679");
    std::fs::create_dir_all(&issue_dir).expect("issue dir");
    let patch = issue_dir.join("changes.patch");
    std::fs::write(&patch, "diff\n").expect("patch written");
    // FMT-DIRTY is convertible (#3775): it must NOT be archived.
    std::fs::write(issue_dir.join("status.txt"), "status=FMT-DIRTY\n").expect("status.txt");

    let output = harness.dispatch(&[
        "guard",
        "--issue",
        "5679",
        "--out-dir",
        &out.display().to_string(),
    ]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "{} {}",
        stdout(&output),
        stderr(&output)
    );
    let report = stdout(&output);
    assert!(report.contains("DISPATCH issue 5679 HELD"), "{report}");
    assert!(report.contains("FMT-DIRTY"), "{report}");
    assert!(report.contains("awaiting conversion"), "{report}");
    // The patch must survive: it is convertible, not a failed run.
    assert!(patch.exists(), "FMT-DIRTY patch must not be archived");
    assert!(issue_dir.exists());
}

#[test]
fn guard_holds_when_status_file_is_absent() {
    let harness = Harness::new("autospec-dispatch-guard-no-status");
    let out = guard_out_dir(&harness);
    let issue_dir = out.join("issue-5680");
    std::fs::create_dir_all(&issue_dir).expect("issue dir");
    let patch = issue_dir.join("changes.patch");
    std::fs::write(&patch, "diff\n").expect("patch written");
    // No status.txt: unrecorded outcome, fail closed.

    let output = harness.dispatch(&[
        "guard",
        "--issue",
        "5680",
        "--out-dir",
        &out.display().to_string(),
    ]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "{} {}",
        stdout(&output),
        stderr(&output)
    );
    let report = stdout(&output);
    assert!(report.contains("DISPATCH issue 5680 HELD"), "{report}");
    assert!(report.contains("unconverted patch exists"), "{report}");
    assert!(report.contains("cannot classify outcome"), "{report}");
    // The patch must survive: unknown provenance means never archive.
    assert!(patch.exists(), "patch without status must not be archived");
    assert!(issue_dir.exists());
}

#[test]
fn guard_dry_run_reports_would_archive_for_failed_run() {
    let harness = Harness::new("autospec-dispatch-guard-dryrun-archive");
    let out = guard_out_dir(&harness);
    let issue_dir = out.join("issue-5681");
    std::fs::create_dir_all(&issue_dir).expect("issue dir");
    let patch = issue_dir.join("changes.patch");
    std::fs::write(&patch, "diff\n").expect("patch written");
    std::fs::write(issue_dir.join("status.txt"), "status=BUILD-FAIL\n").expect("status.txt");

    let output = harness.dispatch(&[
        "guard",
        "--issue",
        "5681",
        "--out-dir",
        &out.display().to_string(),
        "--dry-run",
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{} {}",
        stdout(&output),
        stderr(&output)
    );
    let report = stdout(&output);
    assert!(
        report.contains("would archive failed run (status: BUILD-FAIL)"),
        "{report}"
    );
    // Dry-run must not mutate: the patch and its directory survive.
    assert!(patch.exists(), "--dry-run must not archive the patch");
    assert!(issue_dir.exists());
}

#[test]
fn guard_archives_fleet_shaped_status_file() {
    let harness = Harness::new("autospec-dispatch-guard-fleet-shape");
    let out = guard_out_dir(&harness);
    let issue_dir = out.join("issue-5682");
    std::fs::create_dir_all(&issue_dir).expect("issue dir");
    let patch = issue_dir.join("changes.patch");
    std::fs::write(&patch, "diff\n").expect("patch written");
    // Fleet shape: one `key: value` per line.
    std::fs::write(
        issue_dir.join("status.txt"),
        "status: TIMEOUT\nagent_secs: 3600\n",
    )
    .expect("fleet status.txt");

    let output = harness.dispatch(&[
        "guard",
        "--issue",
        "5682",
        "--out-dir",
        &out.display().to_string(),
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{} {}",
        stdout(&output),
        stderr(&output)
    );
    let report = stdout(&output);
    assert!(
        report.contains("archiving failed run (status: TIMEOUT)"),
        "{report}"
    );
    assert!(report.contains("  archived to "), "{report}");
    assert!(!issue_dir.exists(), "archived issue dir must be gone");
}

// ── #4450: the eligible-vs-queued gap ──────────────────────────────────
//
// 178 open labelled issues, 90 queued, 75 already covered by a branch or a PR,
// and 37 in none of the three sets — filed, labelled, and invisible to dispatch.
// The queue looked healthy because a queue that stopped accepting work looks
// like a queue that is keeping up. These pin that `queue-gap` prints all four
// counts every run (zero included), exits 1 on a gap without correcting it, and
// treats a required component with no implementation as an error that names it.

fn write_issue_list(harness: &Harness, name: &str, text: &str) -> String {
    let path = harness.temp.join(name);
    std::fs::write(&path, text).expect("issue list written");
    path.display().to_string()
}

#[test]
fn queue_gap_prints_all_four_counts_when_the_gap_is_zero() {
    let harness = Harness::new("autospec-dispatch-queue-gap-zero");
    harness.write_queue(&stamped(0, &["10", "11", "12"]));
    let eligible = write_issue_list(&harness, "eligible.txt", "10\n11\n12\n");
    let covered = write_issue_list(&harness, "covered.txt", "12\n");

    let output = harness.dispatch(&[
        "queue-gap",
        "--admitted-file",
        &eligible,
        "--covered-file",
        &covered,
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{} {}",
        stdout(&output),
        stderr(&output)
    );
    let out = stdout(&output);
    assert!(
        out.contains("queue gap: eligible 3, queued 3, has_branch_or_pr 1, missing 0"),
        "{out}"
    );
}

#[test]
fn queue_gap_defect_exits_one_names_the_issues_and_writes_nothing() {
    let harness = Harness::new("autospec-dispatch-queue-gap-defect");
    // The incident in miniature: 4384 and 4385 are filed and labelled but in
    // neither the queue nor a branch/PR, and nothing said so.
    let queue_before = stamped(0, &["4382", "4383"]);
    harness.write_queue(&queue_before);
    let eligible = write_issue_list(&harness, "eligible.txt", "4382\n4383\n4384\n4385\n");
    let covered = write_issue_list(&harness, "covered.txt", "4383\n");

    let output = harness.dispatch(&[
        "queue-gap",
        "--admitted-file",
        &eligible,
        "--covered-file",
        &covered,
    ]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "{} {}",
        stdout(&output),
        stderr(&output)
    );
    let out = stdout(&output);
    assert!(out.contains("QUEUE GAP DEFECT"), "{out}");
    assert!(
        out.contains("eligible 4, queued 2, has_branch_or_pr 1, missing 2"),
        "{out}"
    );
    assert!(out.contains("4384"), "{out}");
    assert!(out.contains("4385"), "{out}");
    assert!(out.contains("reported, not corrected"), "{out}");
    assert_eq!(
        harness.read_queue(),
        queue_before,
        "a reported gap must never be patched by appending to the queue"
    );
}

#[test]
fn queue_gap_requires_the_covered_file_rather_than_over_reporting() {
    let harness = Harness::new("autospec-dispatch-queue-gap-no-covered");
    harness.write_queue(&stamped(0, &["10"]));
    let eligible = write_issue_list(&harness, "eligible.txt", "10\n11\n");

    let output = harness.dispatch(&["queue-gap", "--admitted-file", &eligible]);
    assert_eq!(
        output.status.code(),
        Some(2),
        "{} {}",
        stdout(&output),
        stderr(&output)
    );
    assert!(
        stderr(&output).contains("--covered-file"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn queue_gap_missing_component_is_an_error_naming_step_and_command() {
    let harness = Harness::new("autospec-dispatch-queue-gap-component");
    harness.write_queue(&stamped(0, &["10"]));
    let eligible = write_issue_list(&harness, "eligible.txt", "10\n");
    let covered = write_issue_list(&harness, "covered.txt", "");
    // The loop step's refresher does not exist under the deployment root.
    let absent = harness.temp.join("bin/refresh-queue.sh");

    let output = harness.dispatch(&[
        "queue-gap",
        "--admitted-file",
        &eligible,
        "--covered-file",
        &covered,
        "--require-step",
        &format!("refresh-queue={}", absent.display()),
    ]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "{} {}",
        stdout(&output),
        stderr(&output)
    );
    let out = stdout(&output);
    assert!(out.contains("MISSING COMPONENT"), "{out}");
    assert!(out.contains("refresh-queue"), "{out}");
    assert!(out.contains("never a no-op"), "{out}");
    assert!(out.contains("components: 1 required, 1 missing"), "{out}");
    // The four counts are still printed on the failing run.
    assert!(out.contains("missing 0"), "{out}");
}

#[test]
fn queue_gap_resolved_component_and_zero_gap_exit_clean() {
    let harness = Harness::new("autospec-dispatch-queue-gap-component-present");
    harness.write_queue(&stamped(0, &["10"]));
    let eligible = write_issue_list(&harness, "eligible.txt", "10\n");
    let covered = write_issue_list(&harness, "covered.txt", "");
    let present = harness.temp.join("refresh-queue.sh");
    std::fs::write(&present, "#!/usr/bin/env bash\n").expect("component written");

    let output = harness.dispatch(&[
        "queue-gap",
        "--admitted-file",
        &eligible,
        "--covered-file",
        &covered,
        "--require-step",
        &format!("refresh-queue={}", present.display()),
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{} {}",
        stdout(&output),
        stderr(&output)
    );
    let out = stdout(&output);
    assert!(out.contains("components: 1 required, 0 missing"), "{out}");
    assert!(!out.contains("MISSING COMPONENT"), "{out}");
}

#[test]
fn queue_gap_says_loudly_when_no_component_was_declared() {
    let harness = Harness::new("autospec-dispatch-queue-gap-no-component");
    harness.write_queue(&stamped(0, &["10"]));
    let eligible = write_issue_list(&harness, "eligible.txt", "10\n");
    let covered = write_issue_list(&harness, "covered.txt", "");

    let output = harness.dispatch(&[
        "queue-gap",
        "--admitted-file",
        &eligible,
        "--covered-file",
        &covered,
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{} {}",
        stdout(&output),
        stderr(&output)
    );
    assert!(
        stdout(&output).contains("no required components declared"),
        "{}",
        stdout(&output)
    );
}
