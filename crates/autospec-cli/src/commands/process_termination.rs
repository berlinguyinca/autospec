//! `autospec process-kill` — pattern-based process termination that cannot
//! kill its own session (issue #4448).
//!
//! CLI front-end for [`autospec_core::process_termination`]. This is the
//! documented way to terminate processes by command-line pattern: raw
//! `pkill -f <pattern>` matches the invoking shell's own command line (the
//! pattern is in it) and dies with exit 144, taking the work queued after
//! the kill with it. The helper brackets the pattern's first character,
//! excludes its own session (itself and every ancestor), and kills the
//! remaining matches by pid.
//!
//! Exit-code contract:
//! - `0` — at least one process was signalled.
//! - `1` — the pattern matched nothing: a kill that matches nothing is a
//!   false negative, not a clean state (the line says so).
//! - `2` — usage or tool failure (empty pattern, unbracketable first
//!   character, `pgrep`/`ps` unavailable).

use nix::sys::signal::Signal;

use autospec_core::process_termination;

const HELP: &str = "\
autospec process-kill

USAGE:
    autospec process-kill <pattern> [--signal <NAME>]

Terminate the processes whose command line matches <pattern> — the
documented pattern-based kill. Raw `pkill -f <pattern>` matches the
invoking shell's own command line (the pattern is in it) and dies with
exit 144; this helper brackets the pattern's first character, excludes
its own session (itself and every ancestor), and kills the remaining
matches by pid.

OPTIONS:
    --signal <NAME>   Signal to send: HUP INT QUIT TERM KILL USR1 USR2
                      (default: TERM)

EXIT CODES:
    0   at least one process was signalled
    1   the pattern matched nothing (a false negative, not a clean state)
    2   usage or tool failure
";

fn signal_from_name(name: &str) -> Option<Signal> {
    use Signal::*;
    match name.to_ascii_uppercase().as_str() {
        "HUP" => Some(SIGHUP),
        "INT" => Some(SIGINT),
        "QUIT" => Some(SIGQUIT),
        "TERM" => Some(SIGTERM),
        "KILL" => Some(SIGKILL),
        "USR1" => Some(SIGUSR1),
        "USR2" => Some(SIGUSR2),
        _ => None,
    }
}

/// Status-code shape: `Ok(0)` killed something, `Ok(1)` matched nothing,
/// `Err` is a diagnostic (exit 2).
pub fn run(args: &[String]) -> Result<i32, String> {
    let mut pattern: Option<String> = None;
    let mut signal_name = "TERM".to_string();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--help" | "-h" => {
                print!("{HELP}");
                return Ok(0);
            }
            "--signal" => {
                let value = args
                    .get(index + 1)
                    .ok_or("--signal requires a value")?;
                signal_name = value.clone();
                index += 2;
            }
            flag if flag.starts_with("--signal=") => {
                signal_name = flag.trim_start_matches("--signal=").to_string();
                index += 1;
            }
            flag if flag.starts_with('-') => {
                return Err(format!("unknown option: {flag}\n\n{HELP}"));
            }
            other => {
                if pattern.is_some() {
                    return Err(format!(
                        "unexpected extra argument: {other} (one pattern per call)\n\n{HELP}"
                    ));
                }
                pattern = Some(other.to_string());
                index += 1;
            }
        }
    }
    let pattern = pattern.ok_or_else(|| format!("a pattern is required\n\n{HELP}"))?;
    let signal = signal_from_name(&signal_name)
        .ok_or_else(|| format!("unknown signal: {signal_name} (HUP INT QUIT TERM KILL USR1 USR2)"))?;

    match process_termination::kill_matching(&pattern, signal) {
        Ok(report) => {
            let line = report.line();
            if report.matched_any() {
                println!("{line}");
                Ok(0)
            } else {
                // A zero-match kill is reported on stderr: it is the
                // process-kill twin of the silent-false-negative class —
                // the emptiness was manufactured by the call, not observed
                // in the world.
                eprintln!("{line}");
                Ok(1)
            }
        }
        Err(error) => Err(error.to_string()),
    }
}
