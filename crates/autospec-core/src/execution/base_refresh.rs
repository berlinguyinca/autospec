//! The base refresh chain names the step that failed and why (#4535).
//!
//! The incident: the topup refresh chain
//! (`git fetch` → count commits behind `origin/main` → hard reset to
//! `origin/main`) sent every step's stderr to
//! `/dev/null` and collapsed the four steps into one generic
//! `WARN base refresh failed`. The operator timed the fetch by hand, saw
//! 132 s against a `timeout 120`, raised the timeout, saw one
//! `refreshed base +3 commits` line, and recorded it as verified. The
//! chain then failed eight more times, because the real cause was three
//! steps further down: a stale `HEAD.lock` left by a `git reset` that had
//! been killed mid-operation, which blocks *every* git command in the
//! repository until it is removed — and the one "successful" run had
//! simply happened before the lock existed.
//!
//! Two invariants, each a primitive here (all pure: the shell/Rust caller
//! owns the git I/O — it runs each step with stderr **piped, not
//! redirected to `/dev/null`**, gathers the lock file and the process
//! listing, and hands this module the exit statuses, captured output, and
//! records):
//!
//! 1. **A failure log states which step failed and why.** Each step
//!    reports its own failure with its own captured stderr —
//!    `fetch failed: <err>`, `reset failed: <err>` — never a single
//!    generic warning for the whole chain ([`StepReport`],
//!    [`refresh_lines`]). A step that exited non-zero without emitting
//!    anything (killed by `timeout`) says so in its own line, so
//!    "the step printed nothing" is never confused with "we threw it
//!    away".
//! 2. **A stale lock is reported as a stale lock.** Before the chain runs,
//!    a git lock file with no owning process is classified and reported
//!    explicitly — a stale `HEAD.lock` fails every subsequent git command
//!    in the repository, so letting the next step fail opaquely is how a
//!    lock got fixed as a timeout ([`LockEvidence`], [`lock_status`],
//!    [`lock_line`]).
//!
//! The second half of the incident is a verification rule, not a code
//! rule: a fix for an intermittent failure is verified by the absence of
//! recurrence over a window, never by one success — one passing run
//! distinguishes nothing.

/// The fetch step of the refresh chain.
pub const STEP_FETCH: &str = "fetch";
/// The behind-count step of the refresh chain.
pub const STEP_BEHIND: &str = "behind";
/// The reset step of the refresh chain.
pub const STEP_RESET: &str = "reset";

/// One step of the refresh chain, as observed by the caller.
///
/// The caller owns the git I/O: it runs the step with stderr piped (never
/// to `/dev/null`) and hands over the exit status and the captured
/// output. Suppressing stderr on a step whose failure is reported is the
/// defect this module exists to make impossible: the report must be
/// actionable, and `something failed` is not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepReport {
    /// Which step of the chain this is ([`STEP_FETCH`], [`STEP_BEHIND`],
    /// [`STEP_RESET`]).
    pub step: &'static str,
    /// The step's exit status (0 = success).
    pub exit_status: i32,
    /// The step's captured stderr, trimmed. Empty when the step printed
    /// nothing (e.g. killed by `timeout`) — the failure line then says so
    /// explicitly instead of quoting an empty string.
    pub stderr: String,
}

impl StepReport {
    /// Record one step from the caller's observation.
    ///
    /// `stderr` is the step's captured output. It must come from a pipe:
    /// a failed step whose stderr was redirected to `/dev/null` produces a
    /// line that says `no stderr captured`, which is the log telling the
    /// operator the capture was discarded — never a silent, generic
    /// warning.
    pub fn new(step: &'static str, exit_status: i32, stderr: impl Into<String>) -> Self {
        Self {
            step,
            exit_status,
            stderr: stderr.into().trim().to_string(),
        }
    }

    /// Whether the step succeeded.
    pub fn ok(&self) -> bool {
        self.exit_status == 0
    }

    /// The log line for this step, or `None` when it succeeded.
    ///
    /// Invariant 1: a failed step's line names the step and carries its
    /// captured stderr — `fetch failed: <err>`, `reset failed: <err>`.
    /// A failure with no captured output names itself anyway: the exit
    /// status is stated and the empty capture is visible, so a
    /// `timeout`-killed step reads as one.
    pub fn line(&self) -> Option<String> {
        if self.ok() {
            return None;
        }
        if self.stderr.is_empty() {
            Some(format!(
                "{} failed (exit {}, no stderr captured)",
                self.step, self.exit_status
            ))
        } else {
            Some(format!("{} failed: {}", self.step, self.stderr))
        }
    }
}

/// Evidence about a git lock file in the base repository, gathered by the
/// caller **before** the chain runs. A git command killed mid-operation
/// (a hard reset interrupted by the operator or a timeout) leaves
/// `.git/HEAD.lock` behind; while it exists, every git command in that
/// repository fails with `cannot lock ref: File exists`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockEvidence {
    /// The lock path that exists, e.g. `<base>/.git/HEAD.lock`.
    pub path: String,
    /// The owning process recorded for the lock, if any. Git's own lock
    /// files record no owner at all, so this is `None` for a plain
    /// `HEAD.lock` — and with no recorded owner there is no live process
    /// to wait for.
    pub owner_pid: Option<u32>,
}

/// What the lock file means for the chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LockStatus {
    /// No lock file: the chain may proceed.
    Clear,
    /// A live process still owns the lock: the chain must not touch it.
    /// The report names the owner.
    Held {
        /// The lock path.
        path: String,
        /// The live owning process.
        owner_pid: u32,
    },
    /// The lock exists and no process owns it: it blocks every git command
    /// in the repository until it is removed.
    Stale {
        /// The lock path.
        path: String,
    },
}

/// Classify the lock file from the caller's records.
///
/// `evidence` is `None` when no lock file exists. `live_pids` is the
/// caller's process listing (`ps`). A lock whose recorded owner is not
/// alive is stale, and a lock with no recorded owner is stale: in both
/// cases there is no process left to release it, and it will fail the
/// next step — every subsequent run — until it is removed.
pub fn lock_status(evidence: Option<&LockEvidence>, live_pids: &[u32]) -> LockStatus {
    let Some(evidence) = evidence else {
        return LockStatus::Clear;
    };
    match evidence.owner_pid {
        Some(owner_pid) if live_pids.contains(&owner_pid) => LockStatus::Held {
            path: evidence.path.clone(),
            owner_pid,
        },
        _ => LockStatus::Stale {
            path: evidence.path.clone(),
        },
    }
}

/// The explicit line for a non-clear lock, or `None` when the lock is
/// `Clear`. Invariant 2: a stale lock is reported **as a stale lock** —
/// naming the path and the fact that no process owns it — rather than
/// surfacing later as an opaque `cannot lock ref` failure of whatever
/// step happens to run next.
pub fn lock_line(status: &LockStatus) -> Option<String> {
    match status {
        LockStatus::Clear => None,
        LockStatus::Held { path, owner_pid } => Some(format!(
            "lock {path} held by live pid {owner_pid} — leaving it in place, not refreshing"
        )),
        LockStatus::Stale { path } => Some(format!(
            "stale lock {path}: no owning process — every git command in this repository fails until it is removed"
        )),
    }
}

/// The lines the log receives for a completed refresh chain.
///
/// One line for the observed lock (when it was not `Clear`) plus one line
/// per failed step, in chain order. A clean run produces no lines — the
/// caller logs its own success line (e.g. `refreshed base +3`) — and a
/// failing run never produces a single generic "base refresh failed"
/// line: each step's failure is its own line, with its own captured
/// stderr, so the operator reads which of the four steps stopped and why
/// without re-running the chain with stderr visible.
pub fn refresh_lines(lock: Option<&LockStatus>, steps: &[StepReport]) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(line) = lock.and_then(lock_line) {
        lines.push(line);
    }
    for step in steps {
        if let Some(line) = step.line() {
            lines.push(line);
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_step_names_itself_and_quotes_its_stderr() {
        let fetch = StepReport::new(
            STEP_FETCH,
            128,
            "fatal: unable to access 'https://origin/': connection timed out\n",
        );
        let line = fetch.line().expect("a failed step has a line");
        assert_eq!(
            line,
            "fetch failed: fatal: unable to access 'https://origin/': connection timed out"
        );
        assert!(!fetch.ok());

        let ok = StepReport::new(STEP_BEHIND, 0, "warning: progress noise on stderr\n");
        assert!(ok.ok());
        assert_eq!(ok.line(), None, "a successful step logs no line");
    }

    #[test]
    fn the_incident_two_failed_steps_get_two_lines_not_one_generic_warning() {
        // The incident chain: the fetch succeeded in 4 s (the timeout
        // "fix" had bought nothing) and the reset died on the stale lock.
        // The log must say which step failed and carry git's error —
        // never `WARN base refresh failed`.
        let fetch = StepReport::new(STEP_FETCH, 0, "");
        let behind = StepReport::new(STEP_BEHIND, 0, "");
        let reset = StepReport::new(
            STEP_RESET,
            128,
            "error: update_ref failed for ref 'HEAD': cannot lock ref 'HEAD': \
             Unable to create '.../base/.git/HEAD.lock': File exists.\n",
        );

        let lines = refresh_lines(None, &[fetch.clone(), behind.clone(), reset.clone()]);
        assert_eq!(lines.len(), 1, "one line per failed step, {lines:?}");
        assert!(
            lines[0].starts_with("reset failed: "),
            "the line names the step that failed: {lines:?}"
        );
        assert!(
            lines[0].contains("Unable to create '.../base/.git/HEAD.lock': File exists."),
            "the line carries the captured stderr: {lines:?}"
        );
        assert!(!lines.iter().any(|l| l == "WARN base refresh failed"));
        assert!(fetch.line().is_none() && behind.line().is_none());
    }

    #[test]
    fn two_failed_steps_never_collapse_into_one_line() {
        let fetch = StepReport::new(STEP_FETCH, 124, "fatal: early EOF\n");
        let reset = StepReport::new(STEP_RESET, 128, "error: cannot lock ref 'HEAD'\n");
        let lines = refresh_lines(None, &[fetch, reset]);
        assert_eq!(
            lines,
            vec![
                "fetch failed: fatal: early EOF",
                "reset failed: error: cannot lock ref 'HEAD'",
            ]
        );
    }

    #[test]
    fn a_timeout_killed_step_reports_itself_without_inventing_stderr() {
        // `timeout 120 git fetch` kills the fetch: exit 124, no output.
        // The line states the exit status and the empty capture, so the
        // operator sees a timeout-shaped failure instead of "something
        // failed".
        let fetch = StepReport::new(STEP_FETCH, 124, "");
        assert_eq!(
            fetch.line().as_deref(),
            Some("fetch failed (exit 124, no stderr captured)")
        );
    }

    #[test]
    fn no_lock_is_clear_and_clear_logs_nothing() {
        assert_eq!(lock_status(None, &[111]), LockStatus::Clear);
        assert_eq!(lock_line(&LockStatus::Clear), None);
        let fetch = StepReport::new(STEP_FETCH, 0, "");
        let behind = StepReport::new(STEP_BEHIND, 0, "");
        let reset = StepReport::new(STEP_RESET, 0, "");
        assert!(
            refresh_lines(Some(&LockStatus::Clear), &[fetch, behind, reset]).is_empty(),
            "a clean run over a clear lock produces no lines"
        );
    }

    #[test]
    fn a_lock_with_no_owner_is_stale_and_reported_as_one() {
        // Git's own lock files record no owner: a `HEAD.lock` on disk with
        // no process owning it is the killed-reset state from the
        // incident.
        let evidence = LockEvidence {
            path: "/scratch/base/.git/HEAD.lock".to_string(),
            owner_pid: None,
        };
        let status = lock_status(Some(&evidence), &[4321, 4322]);
        assert_eq!(
            status,
            LockStatus::Stale {
                path: "/scratch/base/.git/HEAD.lock".to_string()
            }
        );
        let line = lock_line(&status).expect("a stale lock is reported explicitly");
        assert!(
            line.contains("/scratch/base/.git/HEAD.lock"),
            "the line names the path: {line}"
        );
        assert!(
            line.contains("no owning process"),
            "the line states why it is stale: {line}"
        );
        assert!(
            line.contains("until it is removed"),
            "the line names the remedy: {line}"
        );
    }

    #[test]
    fn a_dead_owner_is_stale_and_a_live_owner_is_held() {
        let evidence = LockEvidence {
            path: "/scratch/base/.git/HEAD.lock".to_string(),
            owner_pid: Some(4321),
        };
        // The owner is gone from the process listing: stale.
        assert_eq!(
            lock_status(Some(&evidence), &[4322]),
            LockStatus::Stale {
                path: "/scratch/base/.git/HEAD.lock".to_string()
            }
        );
        // The owner is alive: the chain must not touch the lock, and the
        // report names the owner.
        assert_eq!(
            lock_status(Some(&evidence), &[4321, 4322]),
            LockStatus::Held {
                path: "/scratch/base/.git/HEAD.lock".to_string(),
                owner_pid: 4321
            }
        );
        let line = lock_line(&LockStatus::Held {
            path: "/scratch/base/.git/HEAD.lock".to_string(),
            owner_pid: 4321,
        })
        .unwrap();
        assert!(line.contains("pid 4321"), "{line}");
        assert!(line.contains("leaving it in place"), "{line}");
    }

    #[test]
    fn the_full_incident_stale_lock_then_reset_failure_is_two_nameable_lines() {
        // What the operator finally saw by re-running with stderr visible,
        // now reported on the first failed run: the stale lock up front,
        // and the reset's own error — the fetch's success produces no line
        // and cannot be mistaken for the failure.
        let evidence = LockEvidence {
            path: "/scratch/base/.git/HEAD.lock".to_string(),
            owner_pid: None,
        };
        let status = lock_status(Some(&evidence), &[]);
        let fetch = StepReport::new(STEP_FETCH, 0, "");
        let behind = StepReport::new(STEP_BEHIND, 0, "");
        let reset = StepReport::new(
            STEP_RESET,
            128,
            "error: update_ref failed for ref 'HEAD': cannot lock ref 'HEAD': \
             Unable to create '.../base/.git/HEAD.lock': File exists.\n",
        );
        let lines = refresh_lines(Some(&status), &[fetch, behind, reset]);
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(
            lines[0].starts_with("stale lock /scratch/base/.git/HEAD.lock:"),
            "{lines:?}"
        );
        assert!(
            lines[1].starts_with("reset failed: error: update_ref failed"),
            "{lines:?}"
        );
    }
}
