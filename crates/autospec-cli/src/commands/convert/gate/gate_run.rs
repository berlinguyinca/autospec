//! Bounded execution of the gate's cargo stages (issue #4567).
//!
//! A gate that cannot time out cannot be scheduled unattended: one
//! pathological patch would stall the whole pass indefinitely, and the
//! stall is indistinguishable from a hang — the fleet diagnosed one as a
//! wedged process tree before finding a transient child that had already
//! exited. The bound kills the stage, and the kill is reported as a
//! distinct, reserved outcome (exit 124, the code `timeout` itself uses)
//! so the pass can say "unmeasured, not defective" instead of attributing
//! the kill to the patch.
//!
//! The runner drains stdout and stderr on a helper thread while the
//! caller polls the child: a child that fills a pipe it nobody reads
//! blocks forever, and a timeout that cannot see the output is useless
//! either way.

use std::io::Read;
use std::process::{Command, ExitStatus, Output, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;

/// The exit code a timed-out stage is reported with.
///
/// 124 follows the convention `timeout` uses for "the command timed out",
/// sitting next to the 125 this module's wrapper already reserves for
/// "the work was never placed" (#4598). A reserved code keeps the timeout
/// distinguishable from a test that failed for its own reasons: a gate
/// failure is a verdict about the patch, and a kill is not one.
pub(super) const TIMED_OUT_EXIT: i32 = 124;

/// The marker a timed-out stage's output carries, so the attribution step
/// can recognise the kill without an extra parameter.
pub(super) const TIMEOUT_MARKER: &str = "[autospec-gate] stage timed out";

/// The default per-stage bound. Measured gates run in the low minutes;
/// 30 minutes leaves the slowest observed suite well inside the bound
/// while still catching a stall.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(1800);

/// Poll interval while waiting for the child. 50 ms keeps a timed-out
/// stage's overshoot under a second while costing nothing on a fast one.
const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Whether the gate's work is bounded at all, and by how long.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GateTimeout {
    Bounded(Duration),
    Unbounded,
}

impl GateTimeout {
    /// The bound as a duration, `None` when unbounded.
    pub(super) fn as_duration(self) -> Option<Duration> {
        match self {
            GateTimeout::Bounded(d) => Some(d),
            GateTimeout::Unbounded => None,
        }
    }

    /// The bound in whole seconds, for the report line.
    pub(super) fn as_secs(self) -> Option<u64> {
        self.as_duration().map(|d| d.as_secs())
    }
}

/// The bound for a raw environment value: `None` (unset) is the default
/// bound, `0` disables it explicitly, and a value that does not parse as
/// seconds keeps the default — garbage must not silently remove the
/// only thing standing between one bad patch and an unattended pass.
pub(super) fn parse_gate_timeout(raw: Option<&str>) -> GateTimeout {
    match raw.map(str::trim).filter(|raw| !raw.is_empty()) {
        None => GateTimeout::Bounded(DEFAULT_TIMEOUT),
        Some(raw) => match raw.parse::<u64>() {
            Ok(0) => GateTimeout::Unbounded,
            Ok(secs) => GateTimeout::Bounded(Duration::from_secs(secs)),
            Err(_) => GateTimeout::Bounded(DEFAULT_TIMEOUT),
        },
    }
}

/// The live bound, read from `AUTOSPEC_GATE_TIMEOUT_SECS`.
pub(super) fn gate_timeout() -> GateTimeout {
    parse_gate_timeout(std::env::var("AUTOSPEC_GATE_TIMEOUT_SECS").ok().as_deref())
}

/// The outcome of a bounded run: the command finished, or the bound
/// fired and the command was killed.
pub(super) enum BoundedRun {
    /// The command finished on its own; the real exit status.
    Finished(Output),
    /// The bound fired; the child was killed and the output is whatever
    /// it produced before the kill, with the reserved timeout status.
    TimedOut(Output),
}

/// Drain one pipe to the end, on the caller's behalf.
fn drain(mut reader: impl Read) -> Vec<u8> {
    let mut buf = Vec::new();
    let _ = reader.read_to_end(&mut buf);
    buf
}

/// Run a command with piped output and an optional bound.
///
/// The output is drained on a helper thread for the lifetime of the
/// child; the caller polls `try_wait` until the child exits or the bound
/// fires. When the bound fires the child is killed, reaped, and the
/// partial output is returned with the reserved timeout status — the
/// caller decides what that verdict is.
pub(super) fn run_bounded(
    mut command: Command,
    bound: Option<Duration>,
) -> std::io::Result<BoundedRun> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let stdout = child.stdout.take().expect("stdout is piped");
    let stderr = child.stderr.take().expect("stderr is piped");
    let (tx, rx) = mpsc::channel::<(Vec<u8>, Vec<u8>)>();
    std::thread::spawn(move || {
        let out = drain(stdout);
        let err = drain(stderr);
        let _ = tx.send((out, err));
    });
    let deadline = bound.map(|d| Instant::now() + d);
    loop {
        match child.try_wait()? {
            Some(status) => {
                let (out, err) = rx.recv().unwrap_or((Vec::new(), Vec::new()));
                return Ok(BoundedRun::Finished(Output {
                    status,
                    stdout: out,
                    stderr: err,
                }));
            }
            None => {
                let timed_out = deadline.map_or(false, |deadline| Instant::now() >= deadline);
                if timed_out {
                    let _ = child.kill();
                    let _ = child.wait();
                    let (out, err) = rx.recv().unwrap_or((Vec::new(), Vec::new()));
                    return Ok(BoundedRun::TimedOut(Output {
                        status: timed_out_status(),
                        stdout: out,
                        stderr: err,
                    }));
                }
                std::thread::sleep(POLL_INTERVAL);
            }
        }
    }
}

/// A real `ExitStatus` carrying the reserved timeout code.
///
/// Built from the raw platform encoding on unix (a normal exit of 124), so
/// the reported status is indistinguishable in shape from one the shell
/// would have produced — `timeout` itself does exactly this. The gate runs
/// only on unix hosts (the CI matrix and the wrapper model are unix-only),
/// so the other platforms refuse loudly rather than fabricate a status.
#[cfg(unix)]
fn timed_out_status() -> ExitStatus {
    let status: ExitStatus = ExitStatusExt::from_raw(TIMED_OUT_EXIT << 8);
    status
}

#[cfg(not(unix))]
fn timed_out_status() -> ExitStatus {
    panic!("gate timeout synthesis is unix-only; the gate never runs on this platform")
}

/// Report how long one stage took. The pass's gate is the fleet's rate
/// limiter (#4558), so per-patch gate cost is a first-class number, not a
/// fact discoverable only by timing the log.
pub(super) fn report_stage_elapsed(stage: &str, secs: u64) {
    eprintln!("gate: {stage} finished in {}", format_gate_secs(secs));
}

/// Seconds as a short, log-readable duration.
pub(super) fn format_gate_secs(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else {
        format!("{}m{}s", secs / 60, secs % 60)
    }
}

/// The bound a marker line names, for the attribution step.
pub(super) fn timeout_secs_from(text: &str) -> Option<u64> {
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix(TIMEOUT_MARKER) {
            let rest = rest.trim();
            let rest = rest.strip_prefix("after")?.trim();
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            return digits.parse().ok();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_bound_applies_when_the_variable_is_unset() {
        let timeout = parse_gate_timeout(None);
        assert_eq!(timeout, GateTimeout::Bounded(DEFAULT_TIMEOUT));
        assert_eq!(timeout.as_secs(), Some(1800));
    }

    #[test]
    fn an_explicit_zero_disables_the_bound() {
        assert_eq!(parse_gate_timeout(Some("0")), GateTimeout::Unbounded);
        assert_eq!(parse_gate_timeout(Some("0")).as_duration(), None);
    }

    #[test]
    fn an_explicit_bound_is_honoured() {
        assert_eq!(
            parse_gate_timeout(Some("90")),
            GateTimeout::Bounded(Duration::from_secs(90))
        );
    }

    #[test]
    fn a_value_that_does_not_parse_keeps_the_default_bound() {
        // Garbage must not silently remove the bound.
        assert_eq!(
            parse_gate_timeout(Some("soon")),
            GateTimeout::Bounded(DEFAULT_TIMEOUT)
        );
        assert_eq!(
            parse_gate_timeout(Some("")),
            GateTimeout::Bounded(DEFAULT_TIMEOUT)
        );
    }

    #[test]
    fn a_bounded_run_reports_finished_when_the_command_exits_first() {
        let run =
            run_bounded(Command::new("true"), Some(Duration::from_secs(10))).expect("spawn true");
        match run {
            BoundedRun::Finished(output) => assert_eq!(output.status.code(), Some(0)),
            BoundedRun::TimedOut(_) => panic!("`true` finishes in milliseconds"),
        }
    }

    #[test]
    fn a_bounded_run_kills_and_reports_when_the_bound_fires_first() {
        // Five seconds against a one-second bound: the child is killed,
        // the reserved status is reported, and no real exit code leaks
        // through as if the command had failed on its own.
        let mut sleep = Command::new("sleep");
        sleep.arg("5");
        let run = run_bounded(sleep, Some(Duration::from_secs(1))).expect("spawn sleep");
        match run {
            BoundedRun::TimedOut(output) => {
                assert_eq!(output.status.code(), Some(TIMED_OUT_EXIT));
            }
            BoundedRun::Finished(_) => panic!("the bound must fire before sleep 5"),
        }
    }

    #[test]
    fn an_unbounded_run_never_times_out() {
        let mut sleep = Command::new("sleep");
        sleep.arg("1");
        let run = run_bounded(sleep, None).expect("spawn sleep");
        match run {
            BoundedRun::Finished(output) => assert_eq!(output.status.code(), Some(0)),
            BoundedRun::TimedOut(_) => panic!("an unbounded run cannot time out"),
        }
    }

    #[test]
    fn the_timeout_marker_names_the_bound_it_fired_at() {
        let text =
            format!("some output before the kill\n{TIMEOUT_MARKER} after 1s and was killed\n");
        assert_eq!(timeout_secs_from(&text), Some(1));
    }

    #[test]
    fn a_plain_failure_is_not_read_as_a_timeout() {
        assert_eq!(
            timeout_secs_from("test result: FAILED. 0 passed; 1 failed"),
            None
        );
        assert_eq!(timeout_secs_from(""), None);
    }

    #[test]
    fn the_elapsed_report_stays_readable_in_a_log() {
        assert_eq!(format_gate_secs(7), "7s");
        assert_eq!(format_gate_secs(122), "2m2s");
    }
}
