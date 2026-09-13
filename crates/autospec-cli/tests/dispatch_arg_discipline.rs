//! The dispatch argument discipline, end to end (#4568).
//!
//! Asking a command what it does must not make it do the thing. `dispatch
//! stamp --help` used to stamp the queue: `stamp` takes no required
//! arguments, so an unrecognized flag fell through to execution. The
//! discipline is now at the dispatcher, where every subcommand goes
//! through it and none can opt out: `-h`/`--help` is answered before any
//! argument interpretation and writes nothing, and a flag no dispatch
//! subcommand accepts is an error naming the flag.

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
}

fn run(argv: &[String]) -> Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_autospec"))
        .args(argv)
        .output()
        .expect("autospec runs")
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

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

#[test]
fn stamp_help_prints_usage_and_writes_nothing() {
    let harness = Harness::new("autospec-dispatch-stamp-help");

    let output = harness.dispatch(&["stamp", "--help"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let help = stdout(&output);
    assert!(help.contains("USAGE: autospec dispatch"), "{help}");
    assert!(help.contains("stamp"), "{help}");
    assert!(!harness.queue.exists(), "--help must not write the queue");
    assert!(!harness.state.exists(), "--help must not write the ledger");
}

#[test]
fn stamp_unknown_flag_is_an_error_that_names_it_and_writes_nothing() {
    let harness = Harness::new("autospec-dispatch-stamp-bogus");

    let output = harness.dispatch(&["stamp", "--bogus"]);
    assert_eq!(
        output.status.code(),
        Some(2),
        "{} {}",
        stdout(&output),
        stderr(&output)
    );
    let message = stderr(&output);
    assert!(message.contains("--bogus"), "{message}");
    assert!(message.contains("unknown flag"), "{message}");
    assert!(
        !harness.queue.exists(),
        "an unknown flag must not write the queue"
    );
    assert!(
        !harness.state.exists(),
        "an unknown flag must not write the ledger"
    );
}

#[test]
fn a_help_flag_anywhere_in_the_arguments_answers_help() {
    let harness = Harness::new("autospec-dispatch-stamp-help-late");
    harness.write_queue("44\n");

    let output = harness.dispatch(&["stamp", "--at", &NOW.to_string(), "--help"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(stdout(&output).contains("USAGE: autospec dispatch"));
    // Help was answered before interpretation: the queue is untouched.
    assert_eq!(
        harness.read_queue(),
        "44\n",
        "--help must not stamp the queue"
    );
}

#[test]
fn every_dispatch_subcommand_answers_help_and_rejects_unknown_flags() {
    let harness = Harness::new("autospec-dispatch-arg-discipline");
    for subcommand in [
        "check",
        "reconcile",
        "queue-gap",
        "guard",
        "stage",
        "freshness",
        "preflight",
        "stamp",
        "beat",
        "status",
        "runs",
        "tick",
        "mark",
        "schedule",
    ] {
        let help = harness.dispatch(&[subcommand, "--help"]);
        assert_eq!(
            help.status.code(),
            Some(0),
            "{subcommand} --help: {} {}",
            stdout(&help),
            stderr(&help)
        );
        assert!(
            stdout(&help).contains("USAGE: autospec dispatch"),
            "{subcommand} --help: {}",
            stdout(&help)
        );

        let bogus = harness.dispatch(&[subcommand, "--bogus-flag-xyz"]);
        assert_eq!(
            bogus.status.code(),
            Some(2),
            "{subcommand} --bogus-flag-xyz: {} {}",
            stdout(&bogus),
            stderr(&bogus)
        );
        assert!(
            stderr(&bogus).contains("--bogus-flag-xyz"),
            "{subcommand} --bogus-flag-xyz: {}",
            stderr(&bogus)
        );
    }
    assert!(
        !harness.queue.exists(),
        "none of the invocations above may write the queue"
    );
    assert!(
        !harness.state.exists(),
        "none of the invocations above may write the ledger"
    );
}

#[test]
fn stamp_without_a_named_queue_refuses_instead_of_defaulting() {
    let harness = Harness::new("autospec-dispatch-stamp-no-queue");
    let home = harness.temp.join("home");
    std::fs::create_dir_all(&home).expect("home created");

    let output = run_home(&home, &["dispatch", "stamp", "--at", &NOW.to_string()]);
    assert_eq!(
        output.status.code(),
        Some(2),
        "{} {}",
        stdout(&output),
        stderr(&output)
    );
    let message = stderr(&output);
    assert!(message.contains("--queue"), "{message}");
    assert!(
        !home.join(".autospec").exists(),
        "a refused stamp must not create the default queue"
    );

    // Named at a path that does not exist, it still refuses: stamping a
    // queue into existence certifies a producer that never ran.
    let missing = run_home(
        &home,
        &[
            "dispatch",
            "stamp",
            "--queue",
            &home.join(".autospec").join("queue.txt").to_string_lossy(),
            "--at",
            &NOW.to_string(),
        ],
    );
    assert_eq!(missing.status.code(), Some(2), "{}", stderr(&missing));
    assert!(
        stderr(&missing).contains("does not exist"),
        "{}",
        stderr(&missing)
    );
    assert!(
        !home.join(".autospec").join("queue.txt").exists(),
        "a refused stamp must not create the queue"
    );
}
