//! `autospec observe` — the state of a long-running step, from its markers
//! (#3973).
//!
//! Six diagnoses in one session were each an observation read as a
//! conclusion: a conversion log caught mid-write was "stalled", a log tail
//! read eight minutes before the dispatch lines landed was "not dispatching",
//! three `pgrep` hits that were the matcher's own wrapper shells were "still
//! running", and a loop that had opened 86 pull requests was stopped as "low
//! yield" on the evidence of two log lines. The last one cost a 21-hour job.
//!
//! Subcommands:
//! - `step <LOG>` — `running` / `complete` / `stalled` / `unknown` from the
//!   step's own heartbeat and terminal markers. Log recency is reported as an
//!   observation and never used as a status, which is why a mid-write pass
//!   reads `running` instead of being stopped. Exit 0 anything but stalled,
//!   1 stalled.
//! - `growth <FILE>` — the two-sample rule: the file is sampled twice across
//!   a gap and only then described. One sample, or two inside the gap, is
//!   `unknown`. Exit 0; motion is report content.
//! - `count <FILE>` — a count over the whole source, with `--characterise`
//!   refusing to render a phrase without a count and refusing a count taken
//!   over a `--tail` of the file. Exit 0, 2 on a refused characterisation.
//! - `gate` — authorize a destructive action (`stop`, `re-dispatch`,
//!   `archive`) on a terminal marker or two independent sources. A single
//!   glance holds. Exit 0 authorized, 1 held.
//!
//! Every subcommand prints `OBSERVED` lines before any `INFERRED` line, so
//! the gap between what was seen and what was concluded stays visible to the
//! reader.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use autospec_core::observation::{
    characterise, classify, Observations, Report, Sample, Series, StepSignals, StepStatus, Tail,
    MIN_SAMPLE_GAP_SECS,
};

use super::CommandFailure;

/// A stalled step (`step`) or a held destructive action (`gate`).
const HOLD_EXIT: i32 = 1;

/// The pipeline's terminal marker (`######## complete ########`).
const DEFAULT_TERMINAL_MARKER: &str = "######## complete ########";
/// The pipeline's heartbeat marker (`## heartbeat`).
const DEFAULT_HEARTBEAT_MARKER: &str = "## heartbeat";
/// The declared beat period of a step that does not say otherwise.
const DEFAULT_INTERVAL_SECS: u64 = 60;

const HELP: &str = "\
autospec observe — the state of a running step, from its markers, not from one look

Usage: autospec observe <SUBCOMMAND> [args]

Subcommands:
  step <LOG> [--terminal-marker STR] [--heartbeat-marker STR] [--interval SECS]
       [--now EPOCH] [--json]
       running / complete / stalled / unknown from the heartbeat and terminal
       markers. A heartbeat line may carry a trailing epoch seconds token; an
       undated beat cannot be aged, so the status is unknown, never stalled.
       Exit 0 unless stalled (1).
  growth <FILE> [--gap-secs SECS] [--samples N] [--json]
       sample the source N times across a gap, then describe it. One sample,
       or two inside the gap, is 'unknown'. Exit 0.
  count <FILE> --pattern STR [--label LABEL] [--tail N] [--characterise PHRASE] [--json]
       count matching lines over the whole source. --characterise renders a
       phrase with its counts and refuses a phrase with no count behind it, or
       a count taken over a tail. Exit 0, 2 on refusal.
  gate --action STR [--source NAME]... [--terminal] [--json]
       authorize a destructive action on the terminal marker or on two
       independent sources. Repeat looks from one source are one observation.
       Exit 0 authorized, 1 held.

Options:
  -h, --help   show this help";

pub fn run(args: &[String]) -> Result<(), CommandFailure> {
    let (subcommand, rest) = args
        .split_first()
        .ok_or_else(|| CommandFailure::diagnostic("usage: autospec observe <subcommand> ..."))?;
    match subcommand.as_str() {
        "-h" | "--help" => {
            println!("{HELP}");
            Ok(())
        }
        "step" => step(rest),
        "growth" => growth(rest),
        "count" => count(rest),
        "gate" => gate(rest),
        other => Err(CommandFailure::diagnostic(format!(
            "unknown observe subcommand: {other} (expected one of: step, growth, count, gate)"
        ))),
    }
}

/// `step` — status from markers.
fn step(args: &[String]) -> Result<(), CommandFailure> {
    let path = positional(args, "autospec observe step <LOG>")?;
    // A step may name its own markers; the defaults are the pipeline's.
    let terminal_marker = opt_string(args, "--terminal-marker")?
        .unwrap_or_else(|| DEFAULT_TERMINAL_MARKER.to_string());
    let heartbeat_marker = opt_string(args, "--heartbeat-marker")?
        .unwrap_or_else(|| DEFAULT_HEARTBEAT_MARKER.to_string());
    let interval = opt_u64(args, "--interval")?.unwrap_or(DEFAULT_INTERVAL_SECS);
    let now = opt_u64(args, "--now")?.unwrap_or(now_epoch()?);

    let text = read_text(&path)?;
    let terminal_seen = text.lines().any(|line| line.contains(&terminal_marker));
    // The most recent *dated* beat. Recency of other output is collected as
    // an observation below and never used as a status.
    let beat_at = text
        .lines()
        .rev()
        .filter(|line| line.contains(&heartbeat_marker))
        .find_map(trailing_epoch);
    let output_age = now.saturating_sub(file_mtime(&path)?);

    let signals = StepSignals {
        terminal_seen,
        heartbeat_age: beat_at.map(|at| now.saturating_sub(at)),
        output_age: Some(output_age),
    };
    let verdict = classify(&signals, interval);

    let mut report = Report::new();
    report.observe(format!(
        "{}: {} lines, terminal marker {}, {}",
        path.display(),
        text.lines().count(),
        if terminal_seen { "present" } else { "absent" },
        match beat_at {
            Some(at) => format!("last dated heartbeat at {at}"),
            None => "no dated heartbeat".to_string(),
        }
    ));
    report.observe(format!(
        "last byte written {output_age}s ago (recency, not status)"
    ));
    report
        .infer(format!("{} — {}", verdict.status.as_str(), verdict.basis))
        .expect("observations recorded above");

    if super::is_json(args) {
        println!(
            "{}",
            serde_json::json!({
                "path": path.display().to_string(),
                "status": verdict.status.as_str(),
                "basis": verdict.basis,
                "observed": texts(&report, "observed"),
                "inferred": texts(&report, "inferred"),
            })
        );
    } else {
        for line in report.lines() {
            println!("{line}");
        }
        // The hook a runbook polls: the status token on its own line, so a
        // caller greps the verdict without parsing prose.
        println!("status={}", verdict.status.as_str());
    }

    if verdict.status == StepStatus::Stalled {
        return Err(CommandFailure::status(String::new(), HOLD_EXIT));
    }
    Ok(())
}

/// `growth` — the two-sample rule, applied by the tool instead of by intent.
fn growth(args: &[String]) -> Result<(), CommandFailure> {
    let path = positional(args, "autospec observe growth <FILE>")?;
    let gap = opt_u64(args, "--gap-secs")?.unwrap_or(MIN_SAMPLE_GAP_SECS);
    let samples = opt_u64(args, "--samples")?.unwrap_or(2).max(1);

    let mut series = Series::default();
    let mut observed: Vec<String> = Vec::new();
    for index in 0..samples {
        if index > 0 {
            thread_sleep(gap);
        }
        let lines = read_text(&path)?.lines().count() as u64;
        let at = now_epoch()?;
        observed.push(format!("sample {} at {at}: {lines} lines", index + 1));
        series.push(Sample { at, signal: lines });
    }
    let motion = series.motion();

    let mut report = Report::new();
    for line in observed {
        report.observe(format!("{}: {line}", path.display()));
    }
    report
        .infer(format!("motion {}", motion.as_str()))
        .expect("observations recorded above");

    if super::is_json(args) {
        println!(
            "{}",
            serde_json::json!({
                "path": path.display().to_string(),
                "samples": series.len(),
                "gap_secs": gap,
                "motion": motion.as_str(),
                "conclusive": motion.is_conclusive(),
            })
        );
    } else {
        for line in report.lines() {
            println!("{line}");
        }
        if !motion.is_conclusive() {
            println!("INFERRED motion unknown — sample again after a gap before concluding");
        }
    }
    Ok(())
}

/// `count` — the number behind a characterisation, over the whole source.
fn count(args: &[String]) -> Result<(), CommandFailure> {
    let path = positional(args, "autospec observe count <FILE>")?;
    let pattern = opt_string(args, "--pattern")?
        .ok_or_else(|| CommandFailure::diagnostic("count needs --pattern <STR>"))?;
    let label = opt_string(args, "--label")?.unwrap_or_else(|| pattern.clone());
    let tail = opt_u64(args, "--tail")?;
    let phrase = opt_string(args, "--characterise")?;

    let text = read_text(&path)?;
    let total = text.lines().count() as u64;
    let visible = tail.unwrap_or(total).min(total);
    let skipped = total - visible;
    let value = text
        .lines()
        .skip(skipped as usize)
        .filter(|line| line.contains(pattern.as_str()))
        .count() as u64;
    let view = Tail { visible, total };

    if let Some(phrase) = phrase {
        // The refusal is the feature: the same grep over the tail of a
        // dispatch log said "low yield"; over the whole log it said 86 PRs.
        let measurement = view
            .count(&label, value, "lines")
            .map_err(CommandFailure::diagnostic)?;
        let line = characterise(&phrase, std::slice::from_ref(&measurement))
            .map_err(CommandFailure::diagnostic)?;
        println!("OBSERVED {}", measurement.line());
        println!("INFERRED {line}");
        return Ok(());
    }

    if super::is_json(args) {
        println!(
            "{}",
            serde_json::json!({
                "path": path.display().to_string(),
                "label": label,
                "count": value,
                "visible_lines": visible,
                "total_lines": total,
                "complete": view.complete(),
            })
        );
    } else {
        println!("OBSERVED {label}={value} matching lines over {visible} of {total} lines");
        if !view.complete() {
            println!("OBSERVED view is a tail of the source: not evidence for a characterisation");
        }
    }
    Ok(())
}

/// `gate` — the destructive-action check.
fn gate(args: &[String]) -> Result<(), CommandFailure> {
    let action = opt_string(args, "--action")?
        .ok_or_else(|| CommandFailure::diagnostic("gate needs --action <STR>"))?;
    let mut observations = Observations::new();
    for source in opt_all(args, "--source") {
        observations = observations.observed_from(source);
    }
    if args.iter().any(|arg| arg == "--terminal") {
        observations = observations.with_terminal_marker();
    }
    let decision = observations.authorize(&action);

    if super::is_json(args) {
        println!(
            "{}",
            serde_json::json!({
                "action": action,
                "allowed": decision.is_allowed(),
                "independent_observations": observations.independent_observations(),
                "decision": decision.line(),
            })
        );
    } else {
        println!("{}", decision.line());
    }

    if decision.is_allowed() {
        Ok(())
    } else {
        Err(CommandFailure::status(String::new(), HOLD_EXIT))
    }
}

/// The last whitespace-separated token of a line, when it reads as an epoch
/// in seconds (10 digits or more, so a line number or a count is never
/// mistaken for a timestamp).
// texts — the report's statement texts of one kind ("observed" | "inferred").
fn texts<'a>(report: &'a Report, kind: &str) -> Vec<&'a str> {
    report
        .statements()
        .iter()
        .filter(|s| s.kind() == kind)
        .map(|s| s.text())
        .collect()
}

fn trailing_epoch(line: &str) -> Option<u64> {
    let token = line.split_whitespace().rev().next()?;
    if token.len() < 10 || !token.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    token.parse().ok()
}

fn positional(args: &[String], usage: &str) -> Result<PathBuf, CommandFailure> {
    args.iter()
        .find(|arg| !arg.starts_with('-'))
        .cloned()
        .map(PathBuf::from)
        .ok_or_else(|| CommandFailure::diagnostic(format!("usage: {usage} [options]")))
}

fn opt_string(args: &[String], flag: &str) -> Result<Option<String>, CommandFailure> {
    let Some(index) = args.iter().position(|arg| arg == flag) else {
        return Ok(None);
    };
    args.get(index + 1)
        .cloned()
        .map(Some)
        .ok_or_else(|| CommandFailure::diagnostic(format!("{flag} takes a value")))
}

fn opt_u64(args: &[String], flag: &str) -> Result<Option<u64>, CommandFailure> {
    match opt_string(args, flag)? {
        Some(raw) => raw
            .parse()
            .map(Some)
            .map_err(|_| CommandFailure::diagnostic(format!("{flag} expects a number, got {raw}"))),
        None => Ok(None),
    }
}

/// Every value given for a repeatable flag.
fn opt_all(args: &[String], flag: &str) -> Vec<String> {
    args.iter()
        .enumerate()
        .filter(|(_, arg)| arg.as_str() == flag)
        .filter_map(|(index, _)| args.get(index + 1))
        .filter(|value| !value.starts_with('-'))
        .cloned()
        .collect()
}

fn read_text(path: &Path) -> Result<String, CommandFailure> {
    fs::read_to_string(path).map_err(|error| {
        CommandFailure::diagnostic(format!("cannot read {}: {error}", path.display()))
    })
}

fn file_mtime(path: &Path) -> Result<u64, CommandFailure> {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .map(|time| {
            time.duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        })
        .map_err(|error| {
            CommandFailure::diagnostic(format!("cannot stat {}: {error}", path.display()))
        })
}

fn now_epoch() -> Result<u64, CommandFailure> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .map_err(|error| CommandFailure::diagnostic(format!("clock before unix epoch: {error}")))
}

/// Between-sample wait, factored out so `--samples 1` never sleeps.
fn thread_sleep(secs: u64) {
    std::thread::sleep(Duration::from_secs(secs));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_string()).collect()
    }

    #[test]
    fn a_dated_heartbeat_line_ages_and_a_fresh_one_says_running() {
        let now = 1_700_000_600;
        let line = "## heartbeat 1700000597";
        assert_eq!(trailing_epoch(line), Some(1_700_000_597));
        let verdict = classify(
            &StepSignals {
                terminal_seen: false,
                heartbeat_age: Some(now - 1_700_000_597),
                output_age: Some(0),
            },
            60,
        );
        assert_eq!(verdict.status, StepStatus::Running);
    }

    #[test]
    fn a_line_number_is_never_mistaken_for_a_beat_timestamp() {
        assert_eq!(trailing_epoch("## heartbeat 42"), None);
        assert_eq!(trailing_epoch("## heartbeat"), None);
    }

    #[test]
    fn repeated_source_flags_are_collected_for_independence() {
        let observed = opt_all(
            &args(&["--source", "pgrep", "--source", "log:topup"]),
            "--source",
        );
        assert_eq!(observed, vec!["pgrep".to_string(), "log:topup".to_string()]);
    }
}
