use super::*;

/// Consecutive immediate exits before the supervisor stops restarting and quarantines instead.
pub(super) const RESTART_FAST_EXIT_LIMIT: u32 = 5;

/// Ceiling on the restart backoff, so a quarantine decision is still reached in bounded time.
pub(super) const RESTART_BACKOFF_MAX_SECS: u64 = 300;

/// Restart policy for the supervisor: how many conductors in a row died on the way up, whether
/// to keep trying, and how long to wait before the next attempt.
///
/// Kept as its own type so the policy is testable without spawning a conductor. The behaviour it
/// encodes is the fix for berlinguyinca/autospec#3012 section 1, where a conductor exiting about
/// a second after launch was relaunched every interval indefinitely.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct RestartPolicy {
    pub(super) consecutive_fast_exits: u32,
    pub(super) quarantined: bool,
}

impl RestartPolicy {
    /// Record the outcome of a restart. `alive` is the result of re-probing the replacement
    /// after a grace period -- not merely whether the spawn call returned.
    pub(super) fn record_restart(&mut self, alive: bool) {
        if alive {
            self.consecutive_fast_exits = 0;
            return;
        }
        self.consecutive_fast_exits = self.consecutive_fast_exits.saturating_add(1);
        if self.consecutive_fast_exits >= RESTART_FAST_EXIT_LIMIT {
            self.quarantined = true;
        }
    }

    /// False once the breaker has tripped: the supervisor keeps observing and reporting, but
    /// stops relaunching, because something is wrong that another restart will not fix.
    pub(super) fn may_restart(&self) -> bool {
        !self.quarantined
    }

    /// Seconds to wait before the next cycle. A healthy supervisor keeps its configured cadence;
    /// a failing one backs off so the observation interval does not become the relaunch rate.
    pub(super) fn delay_secs(&self, base: u64) -> u64 {
        if self.consecutive_fast_exits == 0 {
            return base;
        }
        base.saturating_mul(1u64 << self.consecutive_fast_exits.min(6))
            .min(RESTART_BACKOFF_MAX_SECS)
    }
}

pub(super) fn supervise(options: Options) -> Result<(), String> {
    let mut iteration = 0;
    // A conductor that exits immediately used to be restarted every interval forever: the
    // observed storm was 1017 respawns in a few hours, each one logged `conductor=running`
    // because the spawn result was trusted without looking again. Counting consecutive fast
    // exits lets the supervisor back off and eventually stop, so a one-second failure stays a
    // one-second failure instead of becoming an outage. See berlinguyinca/autospec#3012 section 1.
    let mut restart_policy = RestartPolicy::default();
    // The pid launched by the previous cycle, judged on the next tick. Verifying inline would
    // mean sleeping before every report, which throttles legitimate relaunches of a conductor
    // that finishes its cycles quickly; waiting a tick costs nothing and observes for longer.
    let mut pending_restart: Option<String> = None;
    loop {
        iteration += 1;
        let layout = RunLayout::new(&options)?;
        let recorded = read_unit("conductor", &layout);
        let mut watched_pid = if options.pid.is_empty() {
            recorded.pid.clone()
        } else {
            options.pid.clone()
        };
        let mut conductor_running = process_alive(&watched_pid);
        if let Some(previous) = pending_restart.take() {
            // Judge the last relaunch now that it has had a full interval to get going. A spawn
            // returning is not a start: trusting it is what let 1017 doomed conductors each be
            // logged `conductor=running` (berlinguyinca/autospec#3012 section 1).
            let survived = process_alive(&previous);
            restart_policy.record_restart(survived);
            if !survived && !restart_policy.may_restart() {
                eprintln!(
                    "autospec-supervise: {} consecutive conductors exited immediately; no longer restarting. Inspect the conductor log, resolve the cause, then run `autospec-autonomous restart`.",
                    restart_policy.consecutive_fast_exits
                );
            }
        }
        let mut action = if options.pid.is_empty() && recorded.stale_pid {
            "stale-metadata".to_string()
        } else if !watched_pid.is_empty() && !conductor_running {
            "conductor-not-running".to_string()
        } else {
            "none".to_string()
        };
        let repairable = options.pid.is_empty()
            && !conductor_running
            && layout.state_dir.join("launch.json").is_file()
            && restart_policy.may_restart();
        if !restart_policy.may_restart() && !conductor_running {
            action = "restart-loop-quarantined".to_string();
        }
        if repairable {
            if persisted_stop_mode(&layout)?.is_some() {
                action = "stop-requested".to_string();
            } else {
                match repair_stopped_conductor(&layout, &options) {
                    Ok(RepairOutcome::Restarted(replacement)) => {
                        watched_pid = replacement.pid;
                        conductor_running = process_alive(&watched_pid);
                        // Whether this one actually took is decided next tick, not here.
                        pending_restart = Some(watched_pid.clone());
                        action = if restart_policy.consecutive_fast_exits == 0 {
                            "restarted-conductor".to_string()
                        } else {
                            format!(
                                "restarted-conductor-after-{}-immediate-exits",
                                restart_policy.consecutive_fast_exits
                            )
                        };
                    }
                    Ok(RepairOutcome::AlreadyRunning(pid)) => {
                        watched_pid = pid;
                        conductor_running = true;
                        action = "already-repaired".to_string();
                    }
                    Ok(RepairOutcome::StopRequested) => {
                        action = "stop-requested".to_string();
                    }
                    Err(error) => {
                        eprintln!("autospec-supervise: repair deferred: {error}");
                        action = "repair-deferred".to_string();
                    }
                }
            }
        }
        let conductor = if conductor_running {
            "running"
        } else {
            "stopped"
        };
        if options.json {
            println!(
                "{{\"command\":\"autonomous\",\"subcommand\":\"supervise\",\"repo\":\"{}\",\"conductor\":\"{}\",\"pid\":\"{}\",\"action\":\"{}\"}}",
                json_escape(&options.repo),
                conductor,
                json_escape(&watched_pid),
                action
            );
        } else {
            println!(
                "autospec-supervise: ok repo={} conductor={} pid={} action={}",
                options.repo, conductor, watched_pid, action
            );
        }
        if options.once || (options.iterations > 0 && iteration >= options.iterations) {
            break;
        }
        // Back off exponentially while restarts keep failing, so the interval that is right for
        // observing a healthy conductor does not become the rate at which a broken one is
        // relaunched.
        let base = options.repair_interval_sec.unwrap_or(options.interval_sec);
        thread::sleep(Duration::from_secs(restart_policy.delay_secs(base)));
    }
    Ok(())
}
