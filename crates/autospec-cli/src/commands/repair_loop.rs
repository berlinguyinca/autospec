//! `autospec repair-loop` — record repair sweeps and inspect the ledger.
//!
//! Subcommands:
//! - `record` — feed one sweep into the per-loop ledger and print the verdict
//!   line. Exit code: 0 idle, 1 repaired, 2 persistent (alert).
//! - `status` — print the ledger summary (repair rate, active streaks, defect
//!   tickets) without recording anything.

use std::fs;
use std::path::{Path, PathBuf};

use autospec_core::repair_loop::{RepairSweep, RepairTracker, RepairVerdict};

use super::CommandFailure;

/// Idle sweep: nothing missing, nothing repaired.
const IDLE_EXIT: i32 = 0;
/// Informational: identities were re-established this sweep.
const REPAIRED_EXIT: i32 = 1;
/// Alert: an identity has been repaired on enough consecutive sweeps.
const PERSISTENT_EXIT: i32 = 2;

const SUBCOMMANDS: &[(&str, &str)] = &[
    (
        "record",
        "Record one repair sweep (exit 0 idle / 1 repaired / 2 persistent)",
    ),
    (
        "status",
        "Show the ledger: repair rate, active streaks, defect tickets",
    ),
];

pub fn run(args: &[String]) -> Result<(), CommandFailure> {
    let (subcommand, rest) = args.split_first().ok_or_else(|| {
        CommandFailure::diagnostic("usage: autospec repair-loop <subcommand> ...")
    })?;
    match subcommand.as_str() {
        "-h" | "--help" => {
            print_help();
            Ok(())
        }
        "record" => record(rest),
        "status" => status(rest),
        other => Err(CommandFailure::diagnostic(format!(
            "unknown repair-loop subcommand: {other} (expected one of: {})",
            SUBCOMMANDS
                .iter()
                .map(|(name, _)| *name)
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

fn print_help() {
    println!("USAGE: autospec repair-loop <subcommand> [options]");
    println!();
    println!("SUBCOMMANDS:");
    for (name, description) in SUBCOMMANDS {
        println!("    {name:<8} {description}");
    }
    println!(
        "\nOPTIONS:\n    --loop <NAME>         Name of the repair loop (required)\n    --expected <ID>       Identity the loop expected to be present (repeatable)\n    --repaired <ID>       Identity found missing and re-established (repeatable)\n    --ticket <ID=TICKET>  Defect ticket the repair of ID stands in for (repeatable)\n    --state-file <PATH>   Ledger file (default $HOME/.autospec/repair-loops/NAME.json)\n    --json                status: emit JSON\n    -h, --help            Print help\n\n\
         RECORD exit codes:\n    0  idle       0 missing; healthy line\n    1  repaired   identities re-established this sweep; status line\n    2  persistent same identity repaired N consecutive sweeps; ALERT line"
    );
}

/// `record` — feed one sweep into the ledger and print its verdict line.
fn record(args: &[String]) -> Result<(), CommandFailure> {
    let name = required_flag(args, "--loop")?;
    let state_file = state_file_for(&name, flag_value(args, "--state-file")?)?;
    let mut tracker = load_or_new(&state_file, &name)?;

    for ticket in flag_values(args, "--ticket")? {
        let (identity, ticket) = ticket.split_once('=').ok_or_else(|| {
            CommandFailure::diagnostic(format!("--ticket expects ID=TICKET, got: {ticket}"))
        })?;
        tracker.attach_defect_ticket(identity, ticket);
    }

    let sweep = RepairSweep {
        expected: flag_values(args, "--expected")?
            .into_iter()
            .map(ToOwned::to_owned)
            .collect(),
        repaired: flag_values(args, "--repaired")?
            .into_iter()
            .map(ToOwned::to_owned)
            .collect(),
    };
    let report = tracker.record(&sweep);
    save_tracker(&tracker, &state_file)?;
    println!("{}", report.line);

    let exit_code = match report.verdict {
        RepairVerdict::Idle => IDLE_EXIT,
        RepairVerdict::Repaired => REPAIRED_EXIT,
        RepairVerdict::Persistent => PERSISTENT_EXIT,
    };
    if exit_code == IDLE_EXIT {
        Ok(())
    } else {
        Err(CommandFailure::status(String::new(), exit_code))
    }
}

/// `status` — show the ledger without recording a sweep.
fn status(args: &[String]) -> Result<(), CommandFailure> {
    let name = required_flag(args, "--loop")?;
    let state_file = state_file_for(&name, flag_value(args, "--state-file")?)?;
    let tracker = match load_tracker(&state_file) {
        Ok(tracker) => tracker,
        Err(_) => {
            if super::is_json(args) {
                println!("{{\"command\":\"repair-loop\",\"subcommand\":\"status\",\"loop\":\"{name}\",\"state\":\"absent\"}}");
            } else {
                println!("repair {name}: no ledger recorded yet");
            }
            return Ok(());
        }
    };

    if super::is_json(args) {
        println!("{}", tracker.status_json());
        return Ok(());
    }

    let mut lines = vec![format!(
        "repair {}: {} sweeps recorded; repair rate {:.1} over last {} sweeps",
        tracker.loop_name(),
        tracker.sweep_count(),
        tracker.repair_rate(),
        tracker.window_size()
    )];
    let streaks = tracker.active_streaks();
    if streaks.is_empty() {
        lines.push("  no active repair streaks".to_string());
    } else {
        for (identity, streak) in streaks {
            lines.push(format!(
                "  streak {identity}={streak} (total {})",
                tracker.total_repairs(&identity)
            ));
        }
    }
    for (identity, ticket) in tracker.defect_tickets() {
        lines.push(format!("  defect ticket for {identity}: {ticket}"));
    }
    println!("{}", lines.join("\n"));
    Ok(())
}

fn load_or_new(state_file: &Path, name: &str) -> Result<RepairTracker, CommandFailure> {
    Ok(load_tracker(state_file).unwrap_or_else(|_| RepairTracker::new(name)))
}

fn load_tracker(state_file: &Path) -> Result<RepairTracker, CommandFailure> {
    let text = fs::read_to_string(state_file).map_err(|error| {
        CommandFailure::diagnostic(format!("cannot read ledger {state_file:?}: {error}"))
    })?;
    match RepairTracker::from_json(&text) {
        Ok(tracker) => Ok(tracker),
        Err(error) => Err(CommandFailure::diagnostic(format!(
            "ledger {state_file:?} does not parse ({error}); refusing to start a fresh one over it"
        ))),
    }
}

fn save_tracker(tracker: &RepairTracker, state_file: &Path) -> Result<(), CommandFailure> {
    if let Some(parent) = state_file.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            CommandFailure::diagnostic(format!("cannot create {}: {error}", parent.display()))
        })?;
    }
    fs::write(state_file, tracker.to_json()).map_err(|error| {
        CommandFailure::diagnostic(format!("cannot write ledger {state_file:?}: {error}"))
    })
}

/// `--loop` names the state file path component, so it must not be able to
/// escape the ledger directory.
fn validate_loop_name(name: &str) -> Result<(), CommandFailure> {
    let valid = !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'));
    if valid {
        Ok(())
    } else {
        Err(CommandFailure::diagnostic(format!(
            "invalid repair-loop name {name:?}: use [a-z0-9._-] only"
        )))
    }
}

fn state_file_for(name: &str, override_path: Option<&str>) -> Result<PathBuf, CommandFailure> {
    validate_loop_name(name)?;
    let path = match override_path {
        Some(path) => PathBuf::from(path),
        None => {
            let home = std::env::var("HOME")
                .or_else(|_| std::env::var("USERPROFILE"))
                .map_err(|_| {
                    CommandFailure::diagnostic("no HOME set for the ledger path".to_string())
                })?;
            PathBuf::from(home)
                .join(".autospec")
                .join("repair-loops")
                .join(format!("{name}.json"))
        }
    };
    Ok(path)
}

fn required_flag(args: &[String], flag: &str) -> Result<String, CommandFailure> {
    flag_value(args, flag)?
        .ok_or_else(|| CommandFailure::diagnostic(format!("missing required flag: {flag}")))
        .map(String::from)
}

fn flag_value<'a>(args: &'a [String], flag: &str) -> Result<Option<&'a str>, CommandFailure> {
    let mut value: Option<&str> = None;
    let mut index = 0;
    while index < args.len() {
        if args[index] == flag {
            value = Some(
                args.get(index + 1)
                    .ok_or_else(|| {
                        CommandFailure::diagnostic(format!("flag needs a value: {flag}"))
                    })?
                    .as_str(),
            );
            index += 2;
        } else {
            index += 1;
        }
    }
    Ok(value)
}

fn flag_values<'a>(args: &'a [String], flag: &str) -> Result<Vec<&'a str>, CommandFailure> {
    let mut values: Vec<&'a str> = Vec::new();
    let mut index = 0;
    while index < args.len() {
        if args[index] == flag {
            let value = args
                .get(index + 1)
                .ok_or_else(|| CommandFailure::diagnostic(format!("flag needs a value: {flag}")))?
                .as_str();
            values.push(value);
            index += 2;
        } else {
            index += 1;
        }
    }
    Ok(values)
}
