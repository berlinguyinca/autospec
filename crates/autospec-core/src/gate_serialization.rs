//! A gate run alongside another gate is not a gate result (issue #4166).
//!
//! A full `cargo test --workspace --no-fail-fast` was started while a
//! conversion pass was mid-flight — itself running full workspace gates in a
//! second worktree. The result was `passed=6226 failed=3`: main carries two
//! known failures, and the third,
//! `foreground_repeated_restart_observes_one_live_harness_until_merge`,
//! never existed. The suite carries 143 wall-clock deadlines under 100 ms
//! and that test takes ~13 s per run — long enough to lose badly to
//! contention, and it passed 3/3 in isolation, single-threaded. Nine
//! patches had merged during the window: a ready-made suspect list, and the
//! next step would have been bisecting nine innocent patches and ending in a
//! confident wrong answer about someone else's correct patch.
//!
//! The invariants this module encodes:
//!
//! 1. **A gate run concurrently with another gate does not produce a gate
//!    result** ([`GateRun::is_gate_result`]). The number that comes back is
//!    not a property of the code under test, and the record says so, so a
//!    consumer never acts on it as if it were.
//! 2. **Verification is serialised: one gate at a time per host**
//!    ([`GateLock`]). The lock costs queueing time and buys the ability to
//!    attribute a failure at all; without it, every gate result carries an
//!    invisible dependency on what else happened to be running.
//! 3. **The machine's concurrent load is recorded with the number**
//!    ([`LoadReading`], [`GateRun::line`]). A gate log that says
//!    `passed=6226 failed=3` is not interpretable later; one that says
//!    `passed=6226 failed=3 (load 34, 2 concurrent gate runs)` is. Where a
//!    result will be read after the fact — and every gate result is — the
//!    conditions are part of the result.
//! 4. **A new failure is not attributed to a merge until it reproduces
//!    alone** ([`attribute`], [`ReproTrial`]). Alone, single-threaded,
//!    three times, first — the rule from #4150, which applies to
//!    attributing a failure to a *commit* just as much as to load. The
//!    presence of a plausible suspect list is exactly what makes the trap
//!    expensive; no input to [`attribute`] can short-circuit the
//!    isolated-reproduction requirement.
//!
//! Everything here except [`GateLock`] is pure: the lock is the one piece
//! that touches the filesystem, and it claims the file atomically so two
//! acquirers cannot both believe they hold it.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// How many times a new failure must reproduce alone and single-threaded
/// before it may be attributed to a commit at all (the #4150 rule, applied
/// to commits as well as to load).
pub const MIN_ISOLATED_REPROS: u32 = 3;

/// The machine's condition while a gate run was in flight.
///
/// Invariant 3: a result read after the fact must carry its conditions.
/// `passed=6226 failed=3` is not interpretable later; the same number with
/// the load and the concurrency it was measured under is.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LoadReading {
    /// The host's load average when the run started.
    pub load: f64,
    /// Other gate runs observed in flight on this host while the run was
    /// running. Exactly zero when the run held the host gate lock
    /// ([`GateLock`]).
    pub concurrent_gate_runs: usize,
}

/// The outcome of one gate run, recorded with the conditions it ran under.
///
/// The record never separates the number from the machine: a run that was
/// not serialised is constructed, reported, and consumed as a run that
/// cannot be attributed, never as a quiet green or red.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GateRun {
    passed: u64,
    failed: u64,
    /// Whether the run held the host gate lock ([`GateLock`]) for its whole
    /// duration.
    serialized: bool,
    load: LoadReading,
}

impl GateRun {
    /// Record one run.
    ///
    /// Fails closed on the incoherent record: a run that claims the host
    /// gate lock while observing another gate run in flight contradicts the
    /// lock. Such a record is a construction error, not a default.
    pub fn new(
        passed: u64,
        failed: u64,
        serialized: bool,
        load: LoadReading,
    ) -> Result<Self, String> {
        if serialized && load.concurrent_gate_runs > 0 {
            return Err(format!(
                "the run claims the host gate lock but observed {} concurrent gate runs; \
                 a serialised run observes none",
                load.concurrent_gate_runs
            ));
        }
        Ok(Self {
            passed,
            failed,
            serialized,
            load,
        })
    }

    pub fn passed(&self) -> u64 {
        self.passed
    }

    pub fn failed(&self) -> u64 {
        self.failed
    }

    pub fn load(&self) -> &LoadReading {
        &self.load
    }

    /// Whether this run is a gate result at all.
    ///
    /// Invariant 1: a gate run concurrently with another gate does not
    /// produce a gate result. Two full workspace runs on one host compete
    /// for every core, and the number that comes back is not a property of
    /// the code under test. A consumer may act on a gate result; it may
    /// only *note* a run that was not serialised.
    pub fn is_gate_result(&self) -> bool {
        self.serialized
    }

    /// The recorded line: the number and the machine's condition together.
    ///
    /// Invariant 3: the load rides with the number, and a run that was not
    /// serialised says so on the same line, so the log cannot be read as a
    /// plain pass/fail later.
    pub fn line(&self) -> String {
        let mut line = format!(
            "passed={} failed={} (load {}, {} concurrent gate run{})",
            self.passed,
            self.failed,
            self.load.load,
            self.load.concurrent_gate_runs,
            if self.load.concurrent_gate_runs == 1 {
                ""
            } else {
                "s"
            },
        );
        if !self.serialized {
            line.push_str("; not a gate result: ran concurrently with another gate");
        }
        line
    }
}

/// Errors from [`GateLock::try_acquire`] and [`GateLock::release`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateLockError {
    /// The host gate lock is already held by another run: queue, or run and
    /// record the result as not serialised ([`GateRun::is_gate_result`]).
    Held {
        /// The holder's pid, if the lock file carried one.
        holder_pid: Option<u32>,
    },
    /// A filesystem error.
    Io(String),
}

/// One gate at a time per host (issue #4166): the host gate lock.
///
/// Invariant 2: verification must be serialised against other
/// verification. The lock costs queueing time and buys the ability to
/// attribute a failure at all. Without it, every gate result carries an
/// invisible dependency on what else happened to be running.
#[derive(Debug, Clone, PartialEq)]
pub struct GateLock {
    path: PathBuf,
    holder_pid: u32,
}

impl GateLock {
    /// The canonical lock path: one per host, under the state root.
    pub fn path_for(state_root: &Path) -> PathBuf {
        state_root.join("gates").join("gate.lock")
    }

    /// Try to acquire the host gate lock.
    ///
    /// The claim is atomic (`create_new`): a concurrent acquirer gets
    /// [`GateLockError::Held`], not a race. The file carries the holder's
    /// pid so a later reader — and [`GateLock::release`] — can tell whose
    /// lock it is looking at.
    pub fn try_acquire(state_root: &Path) -> Result<GateLock, GateLockError> {
        let path = Self::path_for(state_root);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| GateLockError::Io(error.to_string()))?;
        }
        let holder_pid = std::process::id();
        let mut file = match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err(GateLockError::Held {
                    holder_pid: read_holder_pid(&path),
                });
            }
            Err(error) => return Err(GateLockError::Io(error.to_string())),
        };
        file.write_all(format!("{holder_pid}\n").as_bytes())
            .map_err(|error| GateLockError::Io(error.to_string()))?;
        Ok(GateLock { path, holder_pid })
    }

    /// Release the lock.
    ///
    /// Refuses to delete a lock that another run now holds: the file is
    /// checked against the holder this handle claims to be, and a mismatch
    /// is [`GateLockError::Held`], never a silent deletion of someone
    /// else's lock. A lock file that is already gone has nothing left to
    /// protect and releases cleanly.
    pub fn release(self) -> Result<(), GateLockError> {
        match fs::metadata(&self.path) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(GateLockError::Io(error.to_string())),
        }
        let holder = read_holder_pid(&self.path).ok_or(GateLockError::Held { holder_pid: None })?;
        if holder != self.holder_pid {
            return Err(GateLockError::Held {
                holder_pid: Some(holder),
            });
        }
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(GateLockError::Io(error.to_string())),
        }
    }
}

fn read_holder_pid(path: &Path) -> Option<u32> {
    let content = fs::read_to_string(path).ok()?;
    content.trim().parse::<u32>().ok()
}

/// One attempt to reproduce a specific failure.
///
/// Invariant 4 works through what a trial is entitled to count as: only a
/// reproduction that ran alone, single-threaded, and actually failed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReproTrial {
    /// The test (or check) the trial ran.
    pub test: String,
    /// The trial ran alone: no other gate was in flight on the host.
    pub alone: bool,
    /// The trial ran single-threaded.
    pub single_threaded: bool,
    /// The trial failed: the failure reproduced.
    pub failed: bool,
}

impl ReproTrial {
    /// Whether this trial counts toward the isolated-reproduction rule.
    ///
    /// A trial that ran alongside another gate does not count (#4166): the
    /// same wall-clock deadline that broke under contention is the reason
    /// "alone" is part of the rule. A trial that ran multi-threaded does
    /// not count (#4150), and a trial in which the failure did not
    /// reproduce does not count toward the failure at all.
    pub fn counts_as_isolated_repro(&self) -> bool {
        self.alone && self.single_threaded && self.failed
    }
}

/// The attribution decision for the failures a gate run reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailureAttribution {
    /// Every observed failure was already known before the run: no
    /// attribution is owed.
    NoNewFailures,
    /// At least one observed failure is new, but has not yet reproduced
    /// alone, single-threaded, [`MIN_ISOLATED_REPROS`] times. No merge may
    /// be blamed — however plausible the suspect list — until it has: the
    /// investigation would look well-founded and produce a confident wrong
    /// answer. The next step is to re-run the listed failures in
    /// isolation.
    RequiresIsolatedRepro {
        /// The new failures still awaiting isolated reproduction.
        failures: Vec<String>,
        /// How many more isolated reproductions the closest failure still
        /// needs.
        missing_repros: u32,
    },
    /// Every new failure reproduced alone, single-threaded, at least
    /// [`MIN_ISOLATED_REPROS`] times: the failures are real, and
    /// attribution — for example bisecting over the merged patches — may
    /// proceed.
    Attributable {
        /// The new failures.
        failures: Vec<String>,
        /// The number of isolated reproductions the least-reproduced
        /// failure has.
        isolated_repros: u32,
    },
}

impl FailureAttribution {
    /// The verdict as a gate log line.
    pub fn line(&self) -> String {
        match self {
            Self::NoNewFailures => "no new failures".to_string(),
            Self::RequiresIsolatedRepro {
                failures,
                missing_repros,
            } => format!(
                "not attributable to any merge until isolated: {} (missing {} more isolated repro{} of {})",
                failures.join(", "),
                missing_repros,
                if *missing_repros == 1 { "" } else { "s" },
                failures.first().map(String::as_str).unwrap_or("")
            ),
            Self::Attributable {
                failures,
                isolated_repros,
            } => format!(
                "attributable: {} ({} isolated repro{} each, alone and single-threaded)",
                failures.join(", "),
                isolated_repros,
                if *isolated_repros == 1 { "" } else { "s" }
            ),
        }
    }
}

/// Attribute the failures a gate run reported, against what was already
/// known before it.
///
/// Invariant 4, encoded as the shape of the API: the only input that can
/// move the verdict from [`FailureAttribution::RequiresIsolatedRepro`] to
/// [`FailureAttribution::Attributable`] is
/// [`ReproTrial::counts_as_isolated_repro`] evidence. There is no suspect
/// list, merge count, or "plausible explanation" parameter to take — the
/// presence of a plausible suspect list is exactly what makes the trap
/// expensive, and nothing here accepts it as evidence.
pub fn attribute(
    known_failures: &[String],
    observed_failures: &[String],
    trials: &[ReproTrial],
) -> FailureAttribution {
    let mut new_failures: Vec<String> = observed_failures
        .iter()
        .filter(|name| !known_failures.contains(name))
        .cloned()
        .collect();
    new_failures.sort();
    new_failures.dedup();
    if new_failures.is_empty() {
        return FailureAttribution::NoNewFailures;
    }

    let mut attributable: Vec<String> = Vec::new();
    let mut pending: Vec<String> = Vec::new();
    let mut best_attributable: Option<u32> = None;
    let mut closest_pending: Option<u32> = None;
    for name in &new_failures {
        let count = trials
            .iter()
            .filter(|trial| trial.test == *name && trial.counts_as_isolated_repro())
            .count() as u32;
        if count >= MIN_ISOLATED_REPROS {
            attributable.push(name.clone());
            best_attributable = Some(match best_attributable {
                Some(current) => current.min(count),
                None => count,
            });
        } else {
            pending.push(name.clone());
            let missing = MIN_ISOLATED_REPROS - count;
            closest_pending = Some(match closest_pending {
                Some(current) => current.min(missing),
                None => missing,
            });
        }
    }

    if pending.is_empty() {
        FailureAttribution::Attributable {
            failures: attributable,
            isolated_repros: best_attributable.unwrap_or(MIN_ISOLATED_REPROS),
        }
    } else {
        FailureAttribution::RequiresIsolatedRepro {
            failures: pending,
            missing_repros: closest_pending.unwrap_or(MIN_ISOLATED_REPROS),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock after epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("autospec-gate-serialization-{name}-{nonce}"));
        fs::create_dir_all(&root).expect("temp root");
        root
    }

    fn trial(test: &str, alone: bool, single_threaded: bool, failed: bool) -> ReproTrial {
        ReproTrial {
            test: test.to_string(),
            alone,
            single_threaded,
            failed,
        }
    }

    // --- Invariant 1: a concurrent run is not a gate result -------------

    #[test]
    fn a_run_observing_another_gate_is_not_a_gate_result() {
        // The incident: the suite ran while a conversion pass ran full
        // workspace gates in a second worktree. The third failure it
        // reported never existed.
        let run = GateRun::new(
            6226,
            3,
            false,
            LoadReading {
                load: 34.0,
                concurrent_gate_runs: 2,
            },
        )
        .expect("unserialised runs are recordable");

        assert!(
            !run.is_gate_result(),
            "a concurrent run is not a gate result"
        );
        assert_eq!(run.passed(), 6226);
        assert_eq!(run.failed(), 3);
    }

    #[test]
    fn a_serialised_run_is_a_gate_result() {
        let run = GateRun::new(
            6226,
            2,
            true,
            LoadReading {
                load: 1.3,
                concurrent_gate_runs: 0,
            },
        )
        .expect("serialised run is coherent");
        assert!(run.is_gate_result());
    }

    #[test]
    fn a_run_cannot_claim_the_lock_while_observing_a_concurrent_gate() {
        let error = GateRun::new(
            6226,
            3,
            true,
            LoadReading {
                load: 34.0,
                concurrent_gate_runs: 2,
            },
        )
        .unwrap_err();
        assert!(error.contains("concurrent gate runs"), "{error}");
    }

    // --- Invariant 3: the load rides with the number ---------------------

    #[test]
    fn the_recorded_line_carries_the_load_with_the_number() {
        let run = GateRun::new(
            6226,
            3,
            false,
            LoadReading {
                load: 34.0,
                concurrent_gate_runs: 2,
            },
        )
        .unwrap();
        assert_eq!(
            run.line(),
            "passed=6226 failed=3 (load 34, 2 concurrent gate runs); \
             not a gate result: ran concurrently with another gate"
        );
    }

    #[test]
    fn a_serialised_line_records_zero_concurrency() {
        let run = GateRun::new(
            6226,
            2,
            true,
            LoadReading {
                load: 1.3,
                concurrent_gate_runs: 0,
            },
        )
        .unwrap();
        assert_eq!(
            run.line(),
            "passed=6226 failed=2 (load 1.3, 0 concurrent gate runs)"
        );
    }

    #[test]
    fn the_gate_run_survives_a_serde_round_trip() {
        let run = GateRun::new(
            6226,
            3,
            false,
            LoadReading {
                load: 34.0,
                concurrent_gate_runs: 2,
            },
        )
        .unwrap();
        let json = serde_json::to_string(&run).unwrap();
        let back: GateRun = serde_json::from_str(&json).unwrap();
        assert_eq!(back, run);
        assert!(!back.is_gate_result());
        assert_eq!(back.line(), run.line());
    }

    // --- Invariant 2: one gate at a time per host ------------------------

    #[test]
    fn the_lock_path_is_one_per_host_under_the_state_root() {
        let root = Path::new("/state");
        assert_eq!(
            GateLock::path_for(root),
            PathBuf::from("/state/gates/gate.lock")
        );
    }

    #[test]
    fn the_second_acquirer_is_refused_and_named() {
        let root = temp_root("acquire");
        let first = GateLock::try_acquire(&root).expect("first acquire");
        let err = GateLock::try_acquire(&root).expect_err("second acquire is refused");
        assert_eq!(
            err,
            GateLockError::Held {
                holder_pid: Some(std::process::id())
            }
        );
        first.release().expect("release");
        let second = GateLock::try_acquire(&root).expect("re-acquire after release");
        second.release().expect("release");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn release_refuses_to_delete_a_lock_it_does_not_hold() {
        let root = temp_root("release");
        let first = GateLock::try_acquire(&root).expect("acquire");
        // The file is overwritten with a holder pid that is not ours: a
        // stale claim, or someone else's lock. Deleting it would be wrong.
        fs::write(GateLock::path_for(&root), "999999\n").expect("overwrite");
        let err = first.release().expect_err("release is refused");
        assert_eq!(
            err,
            GateLockError::Held {
                holder_pid: Some(999999)
            }
        );
        // The foreign lock is untouched.
        assert!(GateLock::path_for(&root).exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_lock_file_is_not_a_held_lock() {
        // The file vanished under a holder (manual cleanup): release is a
        // no-op success, because there is nothing left to protect.
        let root = temp_root("vanished");
        let first = GateLock::try_acquire(&root).expect("acquire");
        fs::remove_file(GateLock::path_for(&root)).expect("remove");
        first
            .release()
            .expect("release of a vanished lock is clean");
        let _ = fs::remove_dir_all(&root);
    }

    // --- Invariant 4: no attribution until it reproduces alone -----------

    #[test]
    fn only_alone_single_threaded_failures_count_as_repros() {
        assert!(trial("t", true, true, true).counts_as_isolated_repro());
        assert!(
            !trial("t", false, true, true).counts_as_isolated_repro(),
            "concurrent"
        );
        assert!(
            !trial("t", true, false, true).counts_as_isolated_repro(),
            "multi-threaded"
        );
        assert!(
            !trial("t", true, true, false).counts_as_isolated_repro(),
            "did not fail"
        );
    }

    #[test]
    fn a_new_failure_with_no_repros_is_not_attributable() {
        // Nine patches merged during the window. None of that is
        // evidence, and nothing here takes a suspect list at all.
        let known = vec!["known_a".to_string(), "known_b".to_string()];
        let observed = vec![
            "known_a".to_string(),
            "known_b".to_string(),
            "foreground_repeated_restart_observes_one_live_harness_until_merge".to_string(),
        ];
        let verdict = attribute(&known, &observed, &[]);
        assert_eq!(
            verdict,
            FailureAttribution::RequiresIsolatedRepro {
                failures: vec![
                    "foreground_repeated_restart_observes_one_live_harness_until_merge".to_string()
                ],
                missing_repros: 3
            }
        );
    }

    #[test]
    fn two_repros_still_obligate_a_third_before_any_merge_is_blamed() {
        let name = "flaky_under_contention";
        let known: Vec<String> = Vec::new();
        let observed = vec![name.to_string()];
        let trials = vec![trial(name, true, true, true), trial(name, true, true, true)];
        assert_eq!(
            attribute(&known, &observed, &trials),
            FailureAttribution::RequiresIsolatedRepro {
                failures: vec![name.to_string()],
                missing_repros: 1
            }
        );
    }

    #[test]
    fn concurrent_repros_never_substitute_for_isolated_ones() {
        // The failure reproduced nine times — but every time while
        // something else was running. None of them count.
        let name = "flaky_under_contention";
        let known: Vec<String> = Vec::new();
        let observed = vec![name.to_string()];
        let trials = (0..9)
            .map(|_| trial(name, false, true, true))
            .collect::<Vec<_>>();
        assert_eq!(
            attribute(&known, &observed, &trials),
            FailureAttribution::RequiresIsolatedRepro {
                failures: vec![name.to_string()],
                missing_repros: 3
            }
        );
    }

    #[test]
    fn three_isolated_repros_make_the_failure_attributable() {
        let name = "real_regression";
        let known: Vec<String> = Vec::new();
        let observed = vec![name.to_string()];
        let trials = vec![
            trial(name, true, true, true),
            // A concurrent failure, a multi-threaded pass, and a trial of a
            // different test sit beside the real evidence and do not count.
            trial(name, false, true, true),
            trial(name, true, false, false),
            trial("another_test", true, true, true),
            trial(name, true, true, true),
            trial(name, true, true, true),
        ];
        assert_eq!(
            attribute(&known, &observed, &trials),
            FailureAttribution::Attributable {
                failures: vec![name.to_string()],
                isolated_repros: 3
            }
        );
    }

    #[test]
    fn a_mixed_run_is_not_attributable_while_any_failure_waits() {
        let settled = "settled_regression";
        let pending = "still_checking";
        let known: Vec<String> = Vec::new();
        let observed = vec![settled.to_string(), pending.to_string()];
        let trials = vec![
            trial(settled, true, true, true),
            trial(settled, true, true, true),
            trial(settled, true, true, true),
            trial(pending, true, true, true),
        ];
        assert_eq!(
            attribute(&known, &observed, &trials),
            FailureAttribution::RequiresIsolatedRepro {
                failures: vec![pending.to_string()],
                missing_repros: 2
            }
        );
    }

    #[test]
    fn a_run_with_only_known_failures_ows_no_attribution() {
        let known = vec!["known_a".to_string()];
        let observed = vec!["known_a".to_string()];
        assert_eq!(
            attribute(&known, &observed, &[]),
            FailureAttribution::NoNewFailures
        );
    }

    #[test]
    fn the_verdict_lines_are_distinguishable() {
        assert_eq!(FailureAttribution::NoNewFailures.line(), "no new failures");
        let waiting = FailureAttribution::RequiresIsolatedRepro {
            failures: vec!["t".to_string()],
            missing_repros: 3,
        };
        assert!(
            waiting.line().starts_with("not attributable to any merge"),
            "{}",
            waiting.line()
        );
        assert!(
            waiting.line().contains("3 more isolated repros"),
            "{}",
            waiting.line()
        );
        let ready = FailureAttribution::Attributable {
            failures: vec!["t".to_string()],
            isolated_repros: 3,
        };
        assert!(
            ready.line().starts_with("attributable:"),
            "{}",
            ready.line()
        );
        assert!(
            ready.line().contains("3 isolated repros"),
            "{}",
            ready.line()
        );
    }
}
