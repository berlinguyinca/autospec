//! `autospec dispatch` — liveness between filing an issue and dispatching an
//! agent (#3800).
//!
//! Subcommands:
//! - `check` — the consumer's gate. Reads the queue artifact and refuses to
//!   call a stale or unstamped queue "no work". Exit 0 proceed/idle, 1 hold.
//! - `stamp` — the producer's call, just before it renames the artifact into
//!   place: writes `refreshed-at`/`refreshed-by` atomically and beats for the
//!   producing hop.
//! - `beat` — record liveness for any other hop (file, top-up, dispatch).
//! - `status` — topology, credential-hosted steps, per-hop liveness, verdict.
//!   Exit 0 healthy, 1 any hop failed.
//!
//! A cron line for the refresh step is the intended deployment:
//!
//! ```text
//! */10 * * * * refresh-queue.sh && autospec dispatch stamp || \
//!   echo "refresh-queue failed" >> ~/.autospec/logs/refresh-queue.log
//! ```

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use autospec_core::dispatch_pipeline::{
    DispatchPipeline, FreshnessPolicy, LivenessLedger, PipelineReport, PipelineTopology, QueueFile,
    DEFAULT_INTERVAL_SECS, DEFAULT_MAX_STALE_INTERVALS, QUEUE_ARTIFACT,
};

use super::CommandFailure;

/// The artifact is authoritative and every hop is live.
const OK_EXIT: i32 = 0;
/// A liveness failure: the artifact is not authoritative or a hop is silent.
const HOLD_EXIT: i32 = 1;

const SUBCOMMANDS: &[(&str, &str)] = &[
    (
        "check",
        "Gate on the queue artifact (exit 0 proceed/idle / 1 hold)",
    ),
    ("stamp", "Stamp the queue artifact after repopulating it"),
    ("beat", "Record a liveness beat for one hop"),
    (
        "status",
        "Print topology, credential hosts, hop liveness, verdict",
    ),
];

pub fn run(args: &[String]) -> Result<(), CommandFailure> {
    let (subcommand, rest) = args
        .split_first()
        .ok_or_else(|| CommandFailure::diagnostic("usage: autospec dispatch <subcommand> ..."))?;
    match subcommand.as_str() {
        "-h" | "--help" => {
            print_help();
            Ok(())
        }
        "check" => check(rest),
        "stamp" => stamp(rest),
        "beat" => beat(rest),
        "status" => status(rest),
        other => Err(CommandFailure::diagnostic(format!(
            "unknown dispatch subcommand: {other} (expected one of: {})",
            SUBCOMMANDS
                .iter()
                .map(|(name, _)| *name)
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

fn print_help() {
    println!("USAGE: autospec dispatch <subcommand> [options]");
    println!();
    println!("SUBCOMMANDS:");
    for (name, description) in SUBCOMMANDS {
        println!("    {name:<7} {description}");
    }
    println!("OPTIONS:");
    println!("    --queue <PATH>        Queue artifact (default $HOME/.autospec/{QUEUE_ARTIFACT})");
    println!("    --state-file <PATH>   Liveness ledger (default $HOME/.autospec/dispatch-liveness.json)");
    println!(
        "    --topology <PATH>     Topology JSON (default: built-in filing-to-dispatch chain)"
    );
    println!("    --step <NAME>         Hop the beat is for (required for beat)");
    println!(
        "    --by <NAME>           Producer named in the stamp (default: declared queue producer)"
    );
    println!("    --at <EPOCH>          Beat timestamp in epoch seconds (default: current time)");
    println!("    --now <EPOCH>         Evaluate against this instant instead of the clock");
    println!("    --interval <SECONDS>  Interval for hops that declare none (default {DEFAULT_INTERVAL_SECS})");
    println!("    --max-intervals <N>   Missed intervals tolerated before a hold (default {DEFAULT_MAX_STALE_INTERVALS})");
    println!("    --json                Emit JSON");
    println!("    -h, --help            Print help");
    println!();
    println!("EXIT CODES:");
    println!("    {OK_EXIT}  ok        queue fresh (proceed or genuinely idle) and every hop live");
    println!("    {HOLD_EXIT}  hold      queue missing/unstamped/stale, clock rewind, silent hop, or topology defect");
    println!("    2  diagnostic usage error, unreadable artifact, or unparseable ledger");
}

/// `check` — refuse to read a stale queue as "no work".
fn check(args: &[String]) -> Result<(), CommandFailure> {
    let pipeline = build_pipeline(args)?;
    let queue = read_queue(&queue_path(args)?)?;
    let outcome = pipeline.authorize_queue(queue.as_ref(), eval_now(args)?);

    if super::is_json(args) {
        println!(
            "{}",
            serde_json::to_string_pretty(&outcome)
                .map_err(|error| CommandFailure::diagnostic(error.to_string()))?
        );
    } else {
        println!("{}", outcome.line());
    }

    verdict_exit(outcome.held())
}

/// `stamp` — write the freshness headers into the artifact and beat for the hop.
fn stamp(args: &[String]) -> Result<(), CommandFailure> {
    let pipeline = build_pipeline(args)?;
    let path = queue_path(args)?;
    let mut queue = read_queue(&path)?.unwrap_or_default();
    let writer = match opt_string(args, "--by")? {
        Some(name) => name,
        None => pipeline
            .queue_producer()
            .map(|step| step.name.clone())
            .ok_or_else(|| {
                CommandFailure::diagnostic(format!(
                    "no step in the topology produces {QUEUE_ARTIFACT}; pass --by <NAME>"
                ))
            })?,
    };
    validate_step_name(&writer)?;
    let at = opt_u64(args, "--at")?.unwrap_or(now_epoch()?);

    queue.stamp(at, &writer);
    write_atomic(&path, &queue.render())?;

    let ledger_path = state_file(args)?;
    let mut ledger = load_ledger(&ledger_path)?;
    ledger.record(&writer, at);
    save_ledger(&ledger_path, &ledger)?;

    println!(
        "stamped {QUEUE_ARTIFACT}: refreshed-by {writer} at {at} ({} entries)",
        queue.entry_count()
    );
    Ok(())
}

/// `beat` — record that one hop ran, without touching the artifact.
fn beat(args: &[String]) -> Result<(), CommandFailure> {
    let step = opt_string(args, "--step")?.ok_or_else(|| {
        CommandFailure::diagnostic("beat needs --step <NAME> (the hop that is alive)")
    })?;
    validate_step_name(&step)?;
    let at = opt_u64(args, "--at")?.unwrap_or(now_epoch()?);

    let path = state_file(args)?;
    let mut ledger = load_ledger(&path)?;
    ledger.record(&step, at);
    save_ledger(&path, &ledger)?;

    println!("beat recorded: hop {step} at {at}");
    Ok(())
}

/// `status` — the whole filing-to-dispatch chain in one report.
fn status(args: &[String]) -> Result<(), CommandFailure> {
    let pipeline = build_pipeline(args)?;
    let queue = read_queue(&queue_path(args)?)?;
    let report = pipeline.report(queue.as_ref(), eval_now(args)?);

    if super::is_json(args) {
        println!("{}", report.to_json());
    } else {
        print_human(&report, &pipeline);
    }

    verdict_exit(!report.healthy())
}

/// Exit 0 when the answer may be acted on, 1 when a liveness failure names the
/// hop that stopped moving.
fn verdict_exit(failed: bool) -> Result<(), CommandFailure> {
    if failed {
        return Err(CommandFailure::status(String::new(), HOLD_EXIT));
    }
    Ok(())
}

fn print_human(report: &PipelineReport, pipeline: &DispatchPipeline) {
    let credential_hosts: Vec<String> = pipeline
        .topology
        .credential_steps()
        .into_iter()
        .map(|step| format!("{}@{}", step.name, step.host.as_str()))
        .collect();
    println!(
        "credential-holding steps: {}",
        if credential_hosts.is_empty() {
            "none".to_string()
        } else {
            credential_hosts.join(", ")
        }
    );
    for line in report.lines() {
        println!("{line}");
    }
}

/// Assemble topology + ledger + policy from flags and on-disk state.
fn build_pipeline(args: &[String]) -> Result<DispatchPipeline, CommandFailure> {
    let topology = load_topology(&topology_path(args)?)?;
    let liveness = load_ledger(&state_file(args)?)?;
    let policy = FreshnessPolicy::new(
        opt_u64(args, "--interval")?.unwrap_or(DEFAULT_INTERVAL_SECS),
        opt_u64(args, "--max-intervals")?.unwrap_or(DEFAULT_MAX_STALE_INTERVALS),
    )
    .ok_or_else(|| {
        CommandFailure::diagnostic("--interval and --max-intervals must both be greater than zero")
    })?;
    Ok(DispatchPipeline::new(topology, liveness, policy))
}

/// Read the artifact; `None` means it is not on disk, which is a verdict, not
/// an I/O error.
fn read_queue(path: &Path) -> Result<Option<QueueFile>, CommandFailure> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(QueueFile::parse(&text))),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(CommandFailure::diagnostic(format!(
            "cannot read queue {path:?}: {error}"
        ))),
    }
}

/// Write via a sibling temp file and rename, so a consumer never observes a
/// half-written artifact.
fn write_atomic(path: &Path, text: &str) -> Result<(), CommandFailure> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            CommandFailure::diagnostic(format!("cannot create {}: {error}", parent.display()))
        })?;
    }
    let temp = path.with_extension(format!("tmp-{}", std::process::id()));
    fs::write(&temp, text).map_err(|error| {
        CommandFailure::diagnostic(format!("cannot write {}: {error}", temp.display()))
    })?;
    fs::rename(&temp, path).map_err(|error| {
        let _ = fs::remove_file(&temp);
        CommandFailure::diagnostic(format!(
            "cannot move {} onto {path:?}: {error}",
            temp.display()
        ))
    })
}

/// A missing ledger is not a failure: it means no hop has beaten yet.
fn load_ledger(path: &Path) -> Result<LivenessLedger, CommandFailure> {
    match fs::read_to_string(path) {
        Ok(text) => LivenessLedger::from_json(&text).map_err(|error| {
            CommandFailure::diagnostic(format!(
                "liveness ledger {path:?} does not parse ({error}); refusing to start a fresh one over it"
            ))
        }),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(LivenessLedger::new()),
        Err(error) => Err(CommandFailure::diagnostic(format!(
            "cannot read liveness ledger {path:?}: {error}"
        ))),
    }
}

fn save_ledger(path: &Path, ledger: &LivenessLedger) -> Result<(), CommandFailure> {
    write_atomic(path, &ledger.to_json())
}

/// No topology file means the built-in filing-to-dispatch chain.
fn load_topology(path: &Option<PathBuf>) -> Result<PipelineTopology, CommandFailure> {
    let Some(path) = path else {
        return Ok(PipelineTopology::reference());
    };
    let text = fs::read_to_string(path).map_err(|error| {
        CommandFailure::diagnostic(format!("cannot read topology {path:?}: {error}"))
    })?;
    serde_json::from_str(&text).map_err(|error| {
        CommandFailure::diagnostic(format!("topology {path:?} does not parse: {error}"))
    })
}

fn queue_path(args: &[String]) -> Result<PathBuf, CommandFailure> {
    match opt_string(args, "--queue")? {
        Some(path) => Ok(PathBuf::from(path)),
        None => Ok(autospec_home()?.join(QUEUE_ARTIFACT)),
    }
}

fn state_file(args: &[String]) -> Result<PathBuf, CommandFailure> {
    match opt_string(args, "--state-file")? {
        Some(path) => Ok(PathBuf::from(path)),
        None => Ok(autospec_home()?.join("dispatch-liveness.json")),
    }
}

fn topology_path(args: &[String]) -> Result<Option<PathBuf>, CommandFailure> {
    Ok(opt_string(args, "--topology")?.map(PathBuf::from))
}

fn autospec_home() -> Result<PathBuf, CommandFailure> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map_err(|_| {
            CommandFailure::diagnostic("no HOME set for the dispatch state path".to_string())
        })?;
    Ok(PathBuf::from(home).join(".autospec"))
}

fn now_epoch() -> Result<u64, CommandFailure> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .map_err(|error| {
            CommandFailure::diagnostic(format!("system clock is before the unix epoch: {error}"))
        })
}

/// `--now` lets a wrapper (or a test) evaluate staleness against a fixed instant.
fn eval_now(args: &[String]) -> Result<u64, CommandFailure> {
    match opt_u64(args, "--now")? {
        Some(now) => Ok(now),
        None => now_epoch(),
    }
}

/// Hop names land in a JSON ledger key and in every report line, so they are
/// restricted to a safe charset.
fn validate_step_name(name: &str) -> Result<(), CommandFailure> {
    let valid = !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'));
    if valid {
        Ok(())
    } else {
        Err(CommandFailure::diagnostic(format!(
            "invalid hop name {name:?}: use [a-z0-9._-] only"
        )))
    }
}

fn opt_string(args: &[String], flag: &str) -> Result<Option<String>, CommandFailure> {
    opt_raw(args, flag).map(|value| value.map(str::to_string))
}

fn opt_u64(args: &[String], flag: &str) -> Result<Option<u64>, CommandFailure> {
    opt_raw(args, flag).and_then(|value| match value {
        Some(text) => text.parse::<u64>().map(Some).map_err(|error| {
            CommandFailure::diagnostic(format!(
                "flag {flag} needs an integer, got {text:?} ({error})"
            ))
        }),
        None => Ok(None),
    })
}

fn opt_raw<'a>(args: &'a [String], flag: &str) -> Result<Option<&'a str>, CommandFailure> {
    let Some(index) = args.iter().position(|arg| arg == flag) else {
        return Ok(None);
    };
    let value = args
        .get(index + 1)
        .ok_or_else(|| CommandFailure::diagnostic(format!("flag needs a value: {flag}")))?;
    Ok(Some(value.as_str()))
}
