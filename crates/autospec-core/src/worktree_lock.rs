//! A git checkout is not a shareable resource (issue #3608).
//!
//! Two conversion runs started while the earlier one was still going, both in
//! the same git worktree, each doing `git checkout -B fix/issue-N origin/main`
//! and then `cargo test` in it:
//!
//! | patch | verdict during the race | verdict re-run alone |
//! |---|---|---|
//! | 2605 | `build error -- test failed` | **passes** — now PR #3599 |
//! | 2607 | `FAILED. 791 passed; 81 failed` | **passes** — now PR #3600 |
//!
//! The failure is not a crash. It is fabricated evidence: a third run's tests
//! compiled against a tree a different process was checking out from under
//! it, and the result is plausible — `81 failed` looks like a real number. A
//! good patch marked broken is more expensive than a broken patch marked
//! good, because nobody re-checks the rejected pile.
//!
//! Six invariants, each a primitive here (all pure: no I/O, no clock, no
//! subprocess — the shell caller owns the `flock` on the lock file and
//! supplies the timestamps and process records):
//!
//! 1. **Any tool that mutates a git checkout takes an exclusive lock on it**
//!    — `flock` on a file beside the worktree — and **refuses to run rather
//!    than queueing indefinitely**, so a second invocation says why it
//!    stopped ([`lock_path`], [`parse_lock_file`], [`acquire`]).
//! 2. **The lock is held for the whole checkout-apply-test cycle, not per
//!    command**: a lease walks `checkout → apply → test → done`, and a
//!    release before `done` is an error naming the phase it abandoned
//!    ([`Lease`], [`Phase`], [`LeaseError::PrematureRelease`]).
//! 3. **Concurrent work uses separate checkouts, one per worker**: two
//!    workers assigned the same checkout path is a finding, not a race
//!    ([`shared_checkout_findings`]).
//! 4. **A verdict records the checkout it was produced in**, so a
//!    contaminated run can be identified afterwards rather than trusted:
//!    a verdict with no recorded checkout is refused, never defaulted
//!    ([`CheckoutVerdict::new`]), and a verdict whose checkout was held by
//!    another holder at its run time is contaminated
//!    ([`contaminated_verdicts`]).
//! 5. **When a background job is superseded, the replacement stops the old
//!    one first and says so**: "start the new one and hope" is what
//!    produced this ([`supersede`], [`SupersedeOutcome`]).
//! 6. **Pre-flight**: a check for "is anything else already running against
//!    this path" (two processes with the same working directory) refuses
//!    the second run outright, before it can contaminate anything
//!    ([`preflight`]).

use serde::{Deserialize, Serialize};

/// The exclusive lock file for a worktree: a file **beside** the worktree,
/// never inside it — a file inside would be caught by the very checkouts the
/// lock guards (`git status`, `git clean`, the patch itself).
///
/// `a/b/c/worktree` → `a/b/c/worktree.lock`.
pub fn lock_path(worktree: &str) -> String {
    format!("{worktree}.lock")
}

/// One recorded holder of a checkout lock: the file's content, written by
/// the process that holds the `flock` before it mutates anything.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockState {
    /// Who holds the lock (`convpass pid=1234`): the refusal names it.
    pub holder: String,
    /// The holder's process id.
    pub pid: u32,
    /// When the lock was taken, epoch seconds.
    pub acquired_at: u64,
}

/// Parse the lock file's content.
///
/// `Ok(None)` for an absent or empty file: with `flock`, the file is a
/// passive name — an empty file with no live holder is not a lock (the same
/// rule as a crashed claim in `issue_lock`). Malformed non-empty content is
/// an error, never silently read as "no lock": a file that could not be
/// parsed is fail-closed, so a second run refuses rather than guessing.
pub fn parse_lock_file(content: &str) -> Result<Option<LockState>, String> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    serde_json::from_str(trimmed).map(Some).map_err(|error| {
        format!("lock file content is malformed (refusing to treat it as free): {error}")
    })
}

/// The outcome of trying to take a checkout's exclusive lock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcquireOutcome {
    /// The lock is free (or already ours): the cycle may proceed under
    /// [`Lease`].
    Held { lease: Lease },
    /// Another process holds the lock. The refusal says why it stopped and
    /// names the remedy — a separate checkout — rather than queueing
    /// indefinitely behind a run whose length is unbounded.
    Refused {
        /// The recorded holder, for the refusal line.
        holder: String,
        /// When the holder took the lock.
        acquired_at: u64,
        /// The line the second invocation prints instead of running.
        reason: String,
    },
}

/// Take the lock on `worktree` on behalf of `holder`.
///
/// `state` is the parsed lock file (`None` when absent or empty). Refusal —
/// not queuing — is the only contention outcome: a conversion run is
/// unbounded in length, and a queued second run that eventually starts on
/// the same tree has waited for nothing.
pub fn acquire(
    worktree: &str,
    state: Option<&LockState>,
    holder: &str,
    now: u64,
) -> AcquireOutcome {
    if let Some(state) = state {
        if state.holder == holder {
            // Idempotent re-acquire by the holder; never a second lock.
            return AcquireOutcome::Held {
                lease: Lease::new(worktree, holder, state.pid, state.acquired_at),
            };
        }
        return AcquireOutcome::Refused {
            holder: state.holder.clone(),
            acquired_at: state.acquired_at,
            reason: refusal_line(worktree, state),
        };
    }
    AcquireOutcome::Held {
        lease: Lease::new(worktree, holder, 0, now),
    }
}

/// The line a refused second invocation prints instead of running: it names
/// the worktree, the holder, and the remedy (a separate checkout).
pub fn refusal_line(worktree: &str, state: &LockState) -> String {
    format!(
        "refusing: checkout {worktree} is locked by {} (pid {}, since {}Z); not queueing — run in a separate checkout",
        state.holder, state.pid, state.acquired_at
    )
}

/// The phase of the checkout-apply-test cycle a lease is in. The lock is
/// held for the whole cycle, not per command (invariant 2): each phase
/// transition is the next step of one cycle, and the lock leaves only when
/// the cycle is complete.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Checkout,
    Apply,
    Test,
    Done,
}

impl Phase {
    pub fn label(self) -> &'static str {
        match self {
            Self::Checkout => "checkout",
            Self::Apply => "apply",
            Self::Test => "test",
            Self::Done => "done",
        }
    }
}

/// Errors a lease can produce.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeaseError {
    /// The lease was released before the cycle finished. Releasing between
    /// commands is exactly the hole the lock exists to close: a second
    /// process that slips in during the gap checks out under the first one.
    PrematureRelease { phase: Phase },
    /// The cycle is already complete; there is nothing left to advance.
    PastDone,
}

impl std::fmt::Display for LeaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PrematureRelease { phase } => write!(
                f,
                "the checkout lock may only be released after the whole checkout-apply-test cycle; it was released at the {phase:?} phase"
            ),
            Self::PastDone => write!(f, "the checkout cycle is already complete"),
        }
    }
}

impl std::error::Error for LeaseError {}

/// One held lock: the exclusive claim over a checkout for the duration of
/// the whole checkout-apply-test cycle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lease {
    /// The checkout the lease covers.
    pub worktree: String,
    /// Who holds it.
    pub holder: String,
    /// The holder's process id, when known (`0` when the caller did not
    /// supply one).
    pub pid: u32,
    /// When the cycle started, epoch seconds.
    pub acquired_at: u64,
    /// Where in the cycle the holder is.
    pub phase: Phase,
}

impl Lease {
    /// Start a lease at the first phase of the cycle.
    pub fn new(worktree: &str, holder: &str, pid: u32, acquired_at: u64) -> Self {
        Self {
            worktree: worktree.to_string(),
            holder: holder.to_string(),
            pid,
            acquired_at,
            phase: Phase::Checkout,
        }
    }

    /// Advance one step of the cycle, in order. Skipping a phase is an
    /// error: a cycle that goes `checkout → test` without `apply` is
    /// reporting a test that was never applied.
    pub fn advance(&mut self) -> Result<(), LeaseError> {
        let next = match self.phase {
            Phase::Checkout => Phase::Apply,
            Phase::Apply => Phase::Test,
            Phase::Test => Phase::Done,
            Phase::Done => return Err(LeaseError::PastDone),
        };
        self.phase = next;
        Ok(())
    }

    /// Release the lock.
    ///
    /// Allowed only at `done`: the lock is held for the whole
    /// checkout-apply-test cycle, not per command. Releasing mid-cycle is
    /// refused with the phase named, because that is the gap a second
    /// process fills.
    pub fn release(&self) -> Result<(), LeaseError> {
        if self.phase != Phase::Done {
            return Err(LeaseError::PrematureRelease { phase: self.phase });
        }
        Ok(())
    }

    /// The line the cycle reports when it completes: it names the checkout
    /// and the holder, so the completion is attributable to the lock that
    /// covered it.
    pub fn complete_line(&self) -> String {
        format!(
            "checkout cycle complete: {worktree} held by {holder} since {at}Z",
            worktree = self.worktree,
            holder = self.holder,
            at = self.acquired_at
        )
    }
}

/// Invariant 3: findings for workers assigned the same checkout path.
///
/// N workers need N checkouts: a shared checkout is the resource that cannot
/// be shared, and the finding names every worker on every shared path. The
/// cost is deliberate — a checkout is cheap next to the GPU hours a wrong
/// verdict wastes.
pub fn shared_checkout_findings(assignments: &[(&str, &str)]) -> Vec<String> {
    let mut findings: Vec<String> = Vec::new();
    let mut reported: Vec<&str> = Vec::new();
    for &(_, checkout) in assignments {
        if reported.contains(&checkout) {
            continue;
        }
        let mut workers: Vec<&str> = Vec::new();
        for &(worker_b, checkout_b) in assignments {
            if checkout_b == checkout {
                workers.push(worker_b);
            }
        }
        if workers.len() > 1 {
            reported.push(checkout);
            findings.push(format!(
                "SHARED_CHECKOUT: workers {} share checkout {checkout} — concurrent work needs separate checkouts, one per worker",
                workers.join(" and ")
            ));
        }
    }
    findings
}

/// Invariant 4: one verdict, bound to the checkout it was produced in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckoutVerdict {
    /// The issue or patch the verdict was about.
    pub issue: u64,
    /// The verdict itself (`passes`, `FAILED. 791 passed; 81 failed`).
    pub result: String,
    /// The checkout the verdict was produced in. Non-empty — enforced at
    /// construction: a verdict that cannot say which tree it ran in is the
    /// defect this module exists to make visible.
    pub checkout: String,
    /// Who produced it.
    pub holder: String,
    /// When it was produced, epoch seconds.
    pub produced_at: u64,
}

impl CheckoutVerdict {
    /// Construct a verdict.
    ///
    /// `None` when the checkout is empty or whitespace: a verdict without a
    /// recorded origin is refused, never defaulted — a contaminated run
    /// must be identifiable afterwards, and it cannot be if the verdict
    /// says where it ran nowhere.
    pub fn new(
        issue: u64,
        result: impl Into<String>,
        checkout: impl Into<String>,
        holder: impl Into<String>,
        produced_at: u64,
    ) -> Option<Self> {
        let checkout = checkout.into();
        if checkout.trim().is_empty() {
            return None;
        }
        Some(Self {
            issue,
            result: result.into(),
            checkout,
            holder: holder.into(),
            produced_at,
        })
    }
}

/// Invariant 4, as a check: the verdicts in `verdicts` that a lease held by
/// another process over the same checkout contaminates.
///
/// A verdict is contaminated when the checkout it ran in was held by a
/// different holder at the same time — that is the state in which patch
/// 2607's `81 failed` was produced. The contaminated verdicts are returned
/// in input order so the caller can flag or discard them as a set.
pub fn contaminated_verdicts(
    verdicts: &[CheckoutVerdict],
    holds: &[Lease],
) -> Vec<CheckoutVerdict> {
    verdicts
        .iter()
        .filter(|verdict| {
            holds
                .iter()
                .any(|hold| hold.worktree == verdict.checkout && hold.holder != verdict.holder)
        })
        .cloned()
        .collect()
}

/// The line a contaminated run reports: it names the verdict, the other
/// holder, and the checkout they shared.
pub fn contaminated_line(verdict: &CheckoutVerdict, hold: &Lease) -> String {
    format!(
        "WARN: verdict for #{issue} ({result}) in {checkout} is contaminated: {holder} also held it (since {at}Z) — re-run in a clean checkout",
        issue = verdict.issue,
        result = verdict.result,
        checkout = verdict.checkout,
        holder = hold.holder,
        at = hold.acquired_at
    )
}

/// Invariant 5: one process from the caller's point of view — what the
/// superseding run knows about the job it replaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessRef {
    pub pid: u32,
    /// What it is (`convpass run 2605`).
    pub command: String,
}

/// The outcome of superseding one background job with another.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SupersedeOutcome {
    /// The replacement stopped the old job first, and the record says so.
    Clean {
        old: u32,
        new: u32,
        /// The line the record carries.
        line: String,
    },
    /// The replacement started without stopping the old job: the incident.
    /// Both processes are now mutating the same checkout.
    StartedNewWithoutStoppingOld { old: u32, new: u32, line: String },
}

/// Supersede `old` with `new`.
///
/// `old_stopped_first` is the caller's record that the replacement killed
/// (or otherwise stopped) the old job before starting. Without it the
/// outcome is `StartedNewWithoutStoppingOld` — "start the new one and hope"
/// is what produced the fabricated `81 failed`.
pub fn supersede(old: ProcessRef, new: ProcessRef, old_stopped_first: bool) -> SupersedeOutcome {
    if old_stopped_first {
        SupersedeOutcome::Clean {
            old: old.pid,
            new: new.pid,
            line: format!(
                "superseded {cmd} (pid {old}) — stopped before starting {new_cmd} (pid {new})",
                cmd = old.command,
                old = old.pid,
                new_cmd = new.command,
                new = new.pid
            ),
        }
    } else {
        SupersedeOutcome::StartedNewWithoutStoppingOld {
            old: old.pid,
            new: new.pid,
            line: format!(
                "refusing: {new_cmd} (pid {new}) started while {cmd} (pid {old}) is still running against the same checkout — stop the old job first",
                new_cmd = new.command,
                new = new.pid,
                cmd = old.command,
                old = old.pid
            ),
        }
    }
}

/// Invariant 6: one line from the caller's process listing (`ps`): the
/// pid, the working directory, and the command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessRecord {
    pub pid: u32,
    pub cwd: String,
    pub command: String,
}

/// The pre-flight check: is anything else already running against this
/// path?
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Preflight {
    /// No other process is in the target checkout: the run may proceed to
    /// the lock.
    Clean,
    /// Another process is already running in the target checkout. The
    /// refusal names the pids and the path, before the run can contaminate
    /// anything.
    Busy { pids: Vec<u32> },
}

/// Run the pre-flight check over `records` for `target`, excluding the
/// caller's own process (`self_pid`).
///
/// Two processes with the same working directory is the whole defect: this
/// is the three-line check that would have refused the second run outright
/// in seconds, rather than letting it compile against a tree being checked
/// out from under it.
pub fn preflight(records: &[ProcessRecord], self_pid: u32, target: &str) -> Preflight {
    let pids: Vec<u32> = records
        .iter()
        .filter(|record| record.pid != self_pid && record.cwd == target)
        .map(|record| record.pid)
        .collect();
    if pids.is_empty() {
        Preflight::Clean
    } else {
        Preflight::Busy { pids }
    }
}

/// The line a busy pre-flight prints instead of running.
pub fn preflight_refusal_line(target: &str, pids: &[u32]) -> String {
    let named = pids
        .iter()
        .map(|pid| pid.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "refusing: {} process(es) (pid {named}) already running in {target} — run in a separate checkout",
        pids.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_file_lives_beside_the_worktree() {
        assert_eq!(
            lock_path("/scratch/wt/worker-0"),
            "/scratch/wt/worker-0.lock"
        );
        assert_eq!(lock_path("worker-1"), "worker-1.lock");
    }

    #[test]
    fn a_free_checkout_admits_and_a_held_one_refuses() {
        let worktree = "/scratch/wt/worker-0";
        let held = acquire(worktree, None, "convpass pid=111", 1000);
        assert!(matches!(
            held,
            AcquireOutcome::Held {
                ref lease
            } if lease.phase == Phase::Checkout
        ));

        let state = LockState {
            holder: "convpass pid=111".to_string(),
            pid: 111,
            acquired_at: 1000,
        };
        let refused = acquire(worktree, Some(&state), "convpass pid=222", 1001);
        match refused {
            AcquireOutcome::Refused {
                ref holder,
                ref reason,
                ..
            } => {
                assert_eq!(holder, "convpass pid=111");
                assert!(reason.contains("locked by convpass pid=111"), "{reason}");
                assert!(reason.contains("not queueing"), "{reason}");
                assert!(reason.contains(worktree), "{reason}");
            }
            other => panic!("expected a refusal, got: {other:?}"),
        }

        // The holder re-acquiring is idempotent, not a second lock.
        let again = acquire(worktree, Some(&state), "convpass pid=111", 1002);
        assert!(matches!(again, AcquireOutcome::Held { .. }));
    }

    #[test]
    fn an_empty_lock_file_is_free_and_a_malformed_one_refuses() {
        assert_eq!(parse_lock_file("").unwrap(), None);
        assert_eq!(parse_lock_file("   \n").unwrap(), None);
        assert!(parse_lock_file("{not json").is_err());
        let state = parse_lock_file(r#"{"holder":"x","pid":7,"acquired_at":1}"#).unwrap();
        assert!(state.is_some());
    }

    #[test]
    fn the_lease_covers_the_whole_cycle() {
        let mut lease = Lease::new("/scratch/wt/worker-0", "convpass pid=111", 111, 1000);
        assert_eq!(lease.phase, Phase::Checkout);
        // Releasing before the cycle finishes is refused, naming the phase.
        match lease.release() {
            Err(LeaseError::PrematureRelease {
                phase: Phase::Checkout,
            }) => {}
            other => panic!("expected a premature-release refusal at checkout, got: {other:?}"),
        }
        lease.advance().unwrap();
        assert_eq!(lease.phase, Phase::Apply);
        assert!(matches!(
            lease.release(),
            Err(LeaseError::PrematureRelease {
                phase: Phase::Apply
            })
        ));
        lease.advance().unwrap();
        lease.advance().unwrap();
        assert_eq!(lease.phase, Phase::Done);
        lease.release().unwrap();
        // The cycle is over: there is nothing left to advance.
        assert!(matches!(lease.advance(), Err(LeaseError::PastDone)));
    }

    #[test]
    fn phase_transitions_do_not_skip() {
        let mut lease = Lease::new("/scratch/wt/worker-0", "h", 1, 0);
        // One advance per step; the phase sequence is fixed.
        let mut seen = vec![lease.phase];
        for _ in 0..3 {
            lease.advance().unwrap();
            seen.push(lease.phase);
        }
        assert_eq!(
            seen,
            vec![Phase::Checkout, Phase::Apply, Phase::Test, Phase::Done]
        );
    }

    #[test]
    fn shared_checkouts_are_findings() {
        let ok = shared_checkout_findings(&[
            ("w0", "/scratch/wt/worker-0"),
            ("w1", "/scratch/wt/worker-1"),
        ]);
        assert!(ok.is_empty());

        let findings = shared_checkout_findings(&[
            ("w0", "/scratch/wt/shared"),
            ("w1", "/scratch/wt/shared"),
            ("w2", "/scratch/wt/worker-2"),
        ]);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].contains("SHARED_CHECKOUT"), "{findings:?}");
        assert!(
            findings[0].contains("w0") && findings[0].contains("w1"),
            "{findings:?}"
        );
        assert!(findings[0].contains("/scratch/wt/shared"), "{findings:?}");
    }

    #[test]
    fn a_verdict_records_its_checkout_or_is_refused() {
        assert!(CheckoutVerdict::new(2607, "passes", "", "convpass", 1000).is_none());
        assert!(CheckoutVerdict::new(2607, "passes", "   ", "convpass", 1000).is_none());
        let verdict = CheckoutVerdict::new(
            2607,
            "passes",
            "/scratch/wt/worker-1",
            "convpass pid=222",
            1000,
        )
        .expect("a verdict with a recorded checkout is valid");
        assert_eq!(verdict.checkout, "/scratch/wt/worker-1");
    }

    #[test]
    fn a_verdict_contaminated_by_a_co_holder_is_flagged() {
        let verdict = CheckoutVerdict::new(
            2607,
            "FAILED. 791 passed; 81 failed",
            "/scratch/wt/shared",
            "convpass pid=222",
            1000,
        )
        .unwrap();
        let clean = Lease::new("/scratch/wt/worker-1", "convpass pid=333", 333, 900);
        assert!(contaminated_verdicts(std::slice::from_ref(&verdict), &[clean]).is_empty());

        let co_holder = Lease::new("/scratch/wt/shared", "convpass pid=111", 111, 990);
        let flagged = contaminated_verdicts(
            std::slice::from_ref(&verdict),
            std::slice::from_ref(&co_holder),
        );
        assert_eq!(flagged, vec![verdict.clone()]);
        let line = contaminated_line(&verdict, &co_holder);
        assert!(line.contains("contaminated"), "{line}");
        assert!(line.contains("convpass pid=111"), "{line}");
    }

    #[test]
    fn superseding_requires_stopping_the_old_job_first() {
        let old = ProcessRef {
            pid: 111,
            command: "convpass run 2605".to_string(),
        };
        let new = ProcessRef {
            pid: 222,
            command: "convpass run 2607".to_string(),
        };
        match supersede(old.clone(), new.clone(), true) {
            SupersedeOutcome::Clean { old, new, ref line } => {
                assert_eq!((old, new), (111, 222));
                assert!(line.contains("stopped before starting"), "{line}");
            }
            other => panic!("expected a clean supersede, got: {other:?}"),
        }
        match supersede(old.clone(), new.clone(), false) {
            SupersedeOutcome::StartedNewWithoutStoppingOld { old, new, ref line } => {
                assert_eq!((old, new), (111, 222));
                assert!(line.contains("still running"), "{line}");
                assert!(line.contains("stop the old job first"), "{line}");
            }
            other => panic!("expected a started-without-stopping outcome, got: {other:?}"),
        }
    }

    #[test]
    fn the_pre_flight_check_finds_other_processes_in_the_path() {
        let records = vec![
            ProcessRecord {
                pid: 111,
                cwd: "/scratch/wt/worker-0".to_string(),
                command: "cargo test".to_string(),
            },
            ProcessRecord {
                pid: 222,
                cwd: "/scratch/wt/worker-1".to_string(),
                command: "cargo test".to_string(),
            },
            // The caller itself: never a reason to refuse.
            ProcessRecord {
                pid: 333,
                cwd: "/scratch/wt/worker-0".to_string(),
                command: "convpass".to_string(),
            },
        ];
        assert_eq!(
            preflight(&records, 333, "/scratch/wt/worker-0"),
            Preflight::Busy { pids: vec![111] }
        );
        assert_eq!(
            preflight(&records, 333, "/scratch/wt/worker-1"),
            Preflight::Busy { pids: vec![222] }
        );
        assert_eq!(
            preflight(&records, 333, "/scratch/wt/worker-2"),
            Preflight::Clean
        );
        let line = preflight_refusal_line("/scratch/wt/worker-0", &[111]);
        assert!(line.contains("already running"), "{line}");
        assert!(line.contains("pid 111"), "{line}");
    }

    #[test]
    fn the_incident_end_to_end() {
        // Two runs, one checkout: the pre-flight check catches it in
        // seconds, the lock refuses it outright, and a verdict produced
        // during the race is identifiable afterwards.
        let worktree = "/scratch/wt/shared";
        let records = vec![
            ProcessRecord {
                pid: 111,
                cwd: worktree.to_string(),
                command: "convpass 2605".to_string(),
            },
            ProcessRecord {
                pid: 222,
                cwd: worktree.to_string(),
                command: "convpass 2607".to_string(),
            },
        ];
        assert_eq!(
            preflight(&records, 222, worktree),
            Preflight::Busy { pids: vec![111] }
        );

        let first = match acquire(worktree, None, "convpass pid=111", 1000) {
            AcquireOutcome::Held { lease } => lease,
            other => panic!("first run must hold: {other:?}"),
        };
        let state = LockState {
            holder: first.holder.clone(),
            pid: first.pid,
            acquired_at: first.acquired_at,
        };
        assert!(matches!(
            acquire(worktree, Some(&state), "convpass pid=222", 1001),
            AcquireOutcome::Refused { .. }
        ));

        // The second run ran anyway (the incident): its verdict is
        // contaminated and is identified by the checkout it names.
        let verdict = CheckoutVerdict::new(
            2607,
            "FAILED. 791 passed; 81 failed",
            worktree,
            "convpass pid=222",
            1200,
        )
        .unwrap();
        assert_eq!(
            contaminated_verdicts(std::slice::from_ref(&verdict), std::slice::from_ref(&first)),
            vec![verdict]
        );
    }
}
