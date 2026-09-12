//! A measurement must assert that it measured (issue #4462).
//!
//! Four measurement bugs in one session, all in throwaway verification code,
//! none in the product, and every one reading as the reassuring answer:
//!
//! 1. **A stale file read as a fresh response.** A model probe timed out and
//!    `curl -o /tmp/g.json` wrote nothing, so the parser read the *previous*
//!    model's response and reported `READY` for a model that had failed with
//!    `http=000`.
//! 2. **A broken counter read as total failure.** Ten concurrent requests;
//!    the per-request `read` of the status files failed, so the tally
//!    printed `ok=0 failed=10` while the same output's body check printed
//!    `non-empty content: 10/10`. The service was fine; the counting was
//!    broken, and the two fields contradicted each other in the same output.
//! 3. **A killed gate read as zero failures.** A background test run was
//!    terminated when its tool call timed out, leaving `suites=0 FAILED=0`.
//!    The absence of a completion marker is "I did not measure", never
//!    "everything passed".
//! 4. **A single flat sample read as a wedge.** A fleet scan reported
//!    `WEDGED` from one window; a second, longer sample showed decode
//!    advancing and a direct generation returning in seconds. The correct
//!    rule -- persistence across consecutive samples -- had been written
//!    into the acceptance criteria of the very issue filed about this, and
//!    then omitted from the implementation.
//!
//! The bias: in all four cases the broken measurement produced the answer
//! that invites no further checking. A measurement bug that produced an
//! alarming reading would have been investigated immediately. A bug in the
//! product produces a wrong result; a bug in the verification produces a
//! wrong *belief*, which survives longer and propagates into issues,
//! reports, and decisions made downstream. A clean result from new
//! verification code is the case that warrants a second look, not the case
//! that ends it.
//!
//! So **a measurement must assert that it measured**, and each of the four
//! operations gets a concrete assert:
//!
//! 1. A parse of an output file asserts the file was written *by this
//!    invocation* -- remove it first, or check its mtime -- never read a
//!    path a previous iteration may have populated ([`capture_verdict`]).
//! 2. A tally asserts its own arity: `ok + failed == attempted`; and two
//!    tallies over the same `attempted` set that disagree are a
//!    contradiction, not a measurement ([`Tally`], [`tally_contradiction`]).
//! 3. A run emits a completion marker, and its absence yields *unmeasured*,
//!    never pass or fail ([`marked_run`]).
//! 4. A verdict from a sampled time series requires persistence across
//!    consecutive samples before it is actionable; one window is never
//!    sufficient ([`PersistenceGate`]).
//!
//! Everything here is pure and testable: no I/O, no clock, no subprocess.
//! The file-state, marker-state, and sample inputs are evidence the caller
//! gathers; this module only decides what that evidence does and does not
//! say.

use crate::gate_verdict::GateVerdict;

// ── 1. Clean-file capture ─────────────────────────────────────────────────

/// Evidence about an output file around the invocation that is supposed to
/// write it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureEvidence {
    /// The file existed before the invocation started (i.e. the caller did
    /// not remove it first).
    pub existed_before: bool,
    /// The file exists at parse time.
    pub present: bool,
    /// The file's mtime is strictly later than the invocation's start time.
    /// `None` when the mtime could not be read or compared.
    pub mtime_after_start: Option<bool>,
}

/// Whether an output file was written by the invocation that is supposed to
/// have written it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureVerdict {
    /// Written by this invocation; parsing may proceed.
    Fresh,
    /// Present, but not proven to be from this invocation: a leftover from a
    /// previous iteration, or a request that timed out and wrote nothing.
    /// Parsing this file parses someone else's answer.
    Stale,
    /// Absent at parse time; the invocation produced no output.
    Missing,
}

/// Decides [`CaptureEvidence`] into a [`CaptureVerdict`].
///
/// A file is fresh only when the caller can prove the invocation wrote it:
/// either the file was removed before the invocation and now exists, or its
/// mtime is strictly later than the invocation's start. Everything else --
/// including an unreadable mtime -- is fail-closed [`CaptureVerdict::Stale`],
/// because a stale file is exactly what the previous iteration's response
/// becomes.
pub fn capture_verdict(evidence: &CaptureEvidence) -> CaptureVerdict {
    if !evidence.present {
        return CaptureVerdict::Missing;
    }
    if !evidence.existed_before {
        // Removed first: whatever is here was written by this invocation.
        return CaptureVerdict::Fresh;
    }
    match evidence.mtime_after_start {
        Some(true) => CaptureVerdict::Fresh,
        // Stale, or the mtime could not be established: not proven.
        Some(false) | None => CaptureVerdict::Stale,
    }
}

// ── 2. Arity-checked tally ────────────────────────────────────────────────

/// A tally of an attempted set of operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tally {
    pub attempted: u64,
    pub ok: u64,
    pub failed: u64,
}

/// Why a tally cannot be constructed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TallyArityError {
    pub attempted: u64,
    pub ok: u64,
    pub failed: u64,
}

impl Tally {
    /// `ok + failed` must account for every attempt. A tally that cannot say
    /// what happened to all `attempted` operations is not a measurement: a
    /// per-request `read` that fails for every request produces
    /// `ok=0 failed=0 attempted=10`, and that must be rejected, not printed.
    pub fn new(attempted: u64, ok: u64, failed: u64) -> Result<Self, TallyArityError> {
        if ok + failed != attempted {
            return Err(TallyArityError {
                attempted,
                ok,
                failed,
            });
        }
        Ok(Self {
            attempted,
            ok,
            failed,
        })
    }

    /// The line a report prints. The arity check in [`Tally::new`] is what
    /// makes this line a measurement rather than a hope.
    pub fn line(&self) -> String {
        format!(
            "ok={} failed={} (attempted={})",
            self.ok, self.failed, self.attempted
        )
    }
}

/// Whether two tallies that both describe the same `attempted` set
/// contradict each other.
///
/// Both tallies can individually pass the arity check and still disagree --
/// `ok=0 failed=10` from the broken status-file reads beside
/// `non-empty content: 10/10` from the bodies, in the same output. A report
/// containing such a pair is not a measurement: at least one of the two
/// counters is broken, and neither field is trustworthy.
pub fn tally_contradiction(a: &Tally, b: &Tally) -> bool {
    a.attempted == b.attempted && a.attempted > 0 && (a.ok != b.ok || a.failed != b.failed)
}

// ── 3. Completion-marked run ──────────────────────────────────────────────

/// The verdict for a run that is only trustworthy when it wrote a
/// completion marker.
///
/// A killed run leaves whatever it printed -- `suites=0 FAILED=0` looks
/// like a clean gate -- so the marker, not the counters, is the primary
/// evidence that the run finished at all. Absence of the marker yields
/// [`GateVerdict::NotMeasured`], never pass and never fail.
pub fn marked_run(marker_present: bool, succeeded: bool) -> GateVerdict {
    if !marker_present {
        return GateVerdict::NotMeasured {
            why: "completion marker missing: the run was killed, timed out, or never \
                  finished -- its counters are not a measurement"
                .to_string(),
        };
    }
    if succeeded {
        GateVerdict::Pass
    } else {
        GateVerdict::Fail {
            reasons: vec!["the run completed and did not succeed".to_string()],
        }
    }
}

// ── 4. Persistence-gated verdict ──────────────────────────────────────────

/// A verdict from a sampled time series, held until it persists.
///
/// One window is never sufficient: a single flat sample can be the gap
/// between requests of a healthy worker, and a single active sample can be
/// a one-off. A sustained verdict becomes actionable only after `required`
/// *consecutive* samples sustain it; any sample that does not sustain it
/// resets the streak.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PersistenceGate {
    required: usize,
    streak: usize,
}

impl PersistenceGate {
    /// A single window is never sufficient, so `required < 2` is rejected.
    pub fn new(required: usize) -> Option<Self> {
        (required >= 2).then_some(Self {
            required,
            streak: 0,
        })
    }

    /// Record one sample and report whether the sustained verdict is now
    /// actionable.
    pub fn record(&mut self, sustained: bool) -> bool {
        self.streak = if sustained {
            self.streak.saturating_add(1)
        } else {
            0
        };
        self.streak >= self.required
    }

    /// How many consecutive samples have sustained the verdict.
    pub fn streak(&self) -> usize {
        self.streak
    }

    /// How many consecutive samples the verdict needs before it is
    /// actionable.
    pub fn required(&self) -> usize {
        self.required
    }

    /// Whether the sustained verdict has persisted long enough to act on.
    pub fn is_actionable(&self) -> bool {
        self.streak >= self.required
    }

    /// The line a scan report prints for this gate's state.
    pub fn line(&self) -> String {
        if self.is_actionable() {
            format!(
                "{} of {} consecutive samples -- actionable",
                self.streak, self.required
            )
        } else {
            format!(
                "{} of {} consecutive samples -- not actionable",
                self.streak, self.required
            )
        }
    }
}
