//! Unconditional progress sampling for stuck-worker detection (issue #4401).
//!
//! The invariants this module exists to enforce:
//!
//! 1. Do not gate a cheap, reliable signal behind an expensive, unreliable
//!    one. The trigger becomes the detector's true sensitivity and the
//!    good signal's properties are wasted. If a check is cheap enough to
//!    run unconditionally, run it unconditionally and let the decision
//!    logic hold the nuance.
//! 2. Progress is a difference: the first sample for a worker is a baseline
//!    and can never conclude anything about that worker.
//! 3. The decision: counters advancing means healthy, whatever the probe
//!    said; counters frozen AND the worker failing to serve is a stalled
//!    sample; `stuckThreshold` consecutive stalled samples evict.
//! 4. Telemetry is per worker per cycle: a `worker_health` row exists for
//!    every live worker on every sweep, so a dashboard can show progress
//!    rate per worker, not only rows for workers that happened to trip a
//!    probe timeout.
//!
//! The incident (metabolomics-us/inferweave-gateway #133/#136): the
//! stuck-worker detector sampled token progress **only from inside the
//! probe-timeout branch**, making detection conditional on a second,
//! unrelated event. Worker `23015220` was genuinely stuck — token counters
//! frozen across a 35s window while it failed a real generation request —
//! and the gateway log for the same period read:
//!
//! ```text
//! probe timed out; leaving worker in the pool (busy is not dead)   3
//! worker claims a request in flight but moved no tokens            0
//! worker is STUCK ...                                              0
//! mentions of 23015220                                             1
//! ```
//!
//! Three timeouts spread across three workers produced three *first*
//! samples and zero conclusions: a stuck worker does not reliably fail the
//! gateway's probe (the probe is a one-token completion, which can succeed
//! on a worker that cannot serve real traffic), and even when failures do
//! land, they must land consecutively on one worker before the TTL or the
//! reconciler removes it for other reasons.
//!
//! The fix: sample progress for **every live worker on every probe sweep**,
//! independent of whether that worker's probe failed. The token counter is
//! answered off the work queue (`/metrics`, one small GET per worker per
//! cycle) and is cheap, reliable, and independent. The decision logic stays
//! exactly as it was and gains a precondition it can actually rely on.
//!
//! Every function here is pure. Callers perform I/O (probe the worker, GET
//! `/metrics`, evict a worker) with the verdicts and rows these functions
//! return.

use std::collections::BTreeMap;

// ── Invariant 1: do not gate a cheap, reliable signal ─────────────────────

/// How expensive a single sample of a signal is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalCost {
    /// One small GET off the work queue (the token counter from `/metrics`):
    /// cheap enough to run for every live worker on every sweep.
    Cheap,
    /// Runs the model and queues behind production traffic (a completion
    /// probe): must be gated by an interval or a failure condition.
    Expensive,
}

/// A detector's sampling design for one signal: how expensive the signal
/// is, whether it tells the truth, and what triggers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SamplingDesign {
    /// The cost of one sample of the signal.
    pub cost: SignalCost,
    /// The signal tells the truth when the condition it measures holds: a
    /// frozen token counter means no tokens were generated.
    pub reliable: bool,
    /// The signal is sampled for every live worker on every sweep,
    /// independent of whether that worker's probe failed. `false` means the
    /// signal is gated behind a trigger condition (e.g. the probe-timeout
    /// branch).
    pub unconditional: bool,
}

/// Why a sampling design is unsound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SamplingFinding {
    /// A cheap, reliable signal is gated behind a trigger condition. The
    /// trigger becomes the detector's true sensitivity and the signal's
    /// properties are wasted; if the check is cheap enough to run
    /// unconditionally, run it unconditionally and let the decision logic
    /// hold the nuance.
    GatedSignal,
}

impl SamplingFinding {
    pub fn line(&self) -> String {
        match self {
            SamplingFinding::GatedSignal => {
                "a cheap, reliable signal is gated behind a trigger: the trigger \
                 becomes the detector's true sensitivity — run it unconditionally \
                 and let the decision logic hold the nuance"
                    .to_string()
            }
        }
    }
}

impl SamplingDesign {
    /// The incident's detector design: the token counter sampled only from
    /// inside the probe-timeout branch.
    pub fn gated_by_probe_failure() -> Self {
        Self {
            cost: SignalCost::Cheap,
            reliable: true,
            unconditional: false,
        }
    }

    /// The fix: the same signal sampled for every live worker on every
    /// sweep.
    pub fn unconditional_sampling() -> Self {
        Self {
            cost: SignalCost::Cheap,
            reliable: true,
            unconditional: true,
        }
    }

    /// The invariant: do not gate a cheap, reliable signal behind an
    /// expensive, unreliable trigger. A cheap, reliable signal must be
    /// sampled unconditionally; an expensive signal may be gated (that is
    /// how a completion probe is supposed to be sampled), and a cheap but
    /// unreliable signal is not worth sampling unconditionally either.
    pub fn findings(&self) -> Vec<SamplingFinding> {
        if self.cost == SignalCost::Cheap && self.reliable && !self.unconditional {
            vec![SamplingFinding::GatedSignal]
        } else {
            Vec::new()
        }
    }

    /// The operator line: the design and its finding, when there is one.
    pub fn line(&self) -> String {
        let cost = match self.cost {
            SignalCost::Cheap => "cheap",
            SignalCost::Expensive => "expensive",
        };
        let reliability = if self.reliable {
            "reliable"
        } else {
            "unreliable"
        };
        let trigger = if self.unconditional {
            "unconditional"
        } else {
            "gated behind a trigger"
        };
        match self.findings().first() {
            Some(finding) => {
                format!(
                    "signal is {cost}, {reliability}, {trigger} — {}",
                    finding.line()
                )
            }
            None => format!("signal is {cost}, {reliability}, {trigger}"),
        }
    }
}

// ── Invariants 2 + 3: the decision, with a precondition it can rely on ────

/// What one probe sweep observed for one worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProgressSample {
    /// The worker's cumulative token counter, read from `/metrics`.
    pub tokens: u64,
    /// The worker is failing to serve real traffic: its probe timed out, or
    /// it claims a request in flight but moved no tokens. This is the
    /// serve-failure condition, not the probe's verdict alone — the probe
    /// is a one-token completion and can succeed on a worker that cannot
    /// serve real traffic.
    pub failing_to_serve: bool,
}

/// What one sweep's sample concludes for its worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SweepOutcome {
    /// The first sample ever taken for this worker (or a counter that moved
    /// backwards after a restart): progress is a difference and the baseline
    /// can never conclude anything.
    Baseline,
    /// The counter advanced since the previous sweep: healthy, whatever the
    /// probe said.
    Advancing,
    /// The counter is frozen while the worker is failing to serve: a
    /// stalled sample.
    Stalled,
    /// The counter is frozen while the worker is serving: not stalled — a
    /// stalled sample requires both.
    FrozenButServing,
}

/// The verdict of one sweep's sample for one worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Observation {
    /// What this sweep's sample concluded.
    pub outcome: SweepOutcome,
    /// How many consecutive stalled samples this worker has accumulated
    /// after this sweep.
    pub stalled_streak: u32,
    /// True only when the streak has reached `stuckThreshold`: evict.
    pub evict: bool,
}

/// One `worker_health` row: one per live worker per sweep (invariant 4),
/// independent of whether the worker's probe failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerHealthRow {
    /// The worker the row is for.
    pub worker: String,
    /// The worker's cumulative token counter at this sweep.
    pub tokens: u64,
    /// Tokens moved since the previous sweep; `None` on the baseline sample.
    pub progress: Option<u64>,
    /// Whether the worker was failing to serve at this sweep.
    pub failing_to_serve: bool,
    /// Consecutive stalled samples accumulated by this worker.
    pub stalled_streak: u32,
}

/// The default number of consecutive stalled samples that evict a worker.
pub const DEFAULT_STUCK_THRESHOLD: u32 = 3;

#[derive(Debug, Clone)]
struct WorkerState {
    last_tokens: Option<u64>,
    stalled_streak: u32,
    row: WorkerHealthRow,
}

/// Per-worker stuck detection over unconditional progress samples.
///
/// Callers sample every live worker on every probe sweep (the token counter
/// off `/metrics`) and hand each sample to [`StuckTracker::observe`]. The
/// tracker keeps the previous counter and the stalled streak per worker;
/// the decision logic (invariant 3) holds the nuance the old detector
/// delegated to the probe-timeout branch.
#[derive(Debug, Clone)]
pub struct StuckTracker {
    threshold: u32,
    workers: BTreeMap<String, WorkerState>,
}

impl StuckTracker {
    /// A tracker that evicts after `threshold` consecutive stalled samples.
    /// A threshold of 0 is treated as 1: a worker is evicted on its first
    /// stalled sample, never on a baseline.
    pub fn new(threshold: u32) -> Self {
        Self {
            threshold: threshold.max(1),
            workers: BTreeMap::new(),
        }
    }

    /// A tracker with [`DEFAULT_STUCK_THRESHOLD`].
    pub fn with_default_threshold() -> Self {
        Self::new(DEFAULT_STUCK_THRESHOLD)
    }

    /// The configured threshold.
    pub fn threshold(&self) -> u32 {
        self.threshold
    }

    /// Fold one sweep's sample for one worker into the tracker.
    ///
    /// The sample is taken unconditionally — the caller sampled this worker
    /// because it is live, not because its probe failed. The verdict:
    ///
    /// * no previous counter (or the counter moved backwards after a
    ///   restart): the sample is a baseline and concludes nothing;
    /// * the counter advanced: healthy, whatever the probe said — the
    ///   stalled streak resets;
    /// * the counter is frozen and the worker is failing to serve: a
    ///   stalled sample — the streak grows, and reaching the threshold
    ///   evicts;
    /// * the counter is frozen and the worker is serving: not stalled — the
    ///   streak resets.
    pub fn observe(&mut self, worker: &str, sample: ProgressSample) -> Observation {
        let prev = self.workers.get(worker).and_then(|w| w.last_tokens);

        let (outcome, progress) = match prev {
            None => (SweepOutcome::Baseline, None),
            Some(last) if sample.tokens > last => {
                (SweepOutcome::Advancing, Some(sample.tokens - last))
            }
            Some(last) if sample.tokens < last => {
                // The counter moved backwards: the worker's process
                // restarted. Re-baseline; a backwards counter is not a
                // stalled sample.
                (SweepOutcome::Baseline, None)
            }
            Some(_) if sample.failing_to_serve => (SweepOutcome::Stalled, Some(0)),
            Some(_) => (SweepOutcome::FrozenButServing, Some(0)),
        };

        let entry = self
            .workers
            .entry(worker.to_string())
            .or_insert_with(|| WorkerState {
                last_tokens: None,
                stalled_streak: 0,
                row: WorkerHealthRow {
                    worker: worker.to_string(),
                    tokens: sample.tokens,
                    progress: None,
                    failing_to_serve: sample.failing_to_serve,
                    stalled_streak: 0,
                },
            });

        entry.last_tokens = Some(sample.tokens);
        match outcome {
            SweepOutcome::Stalled => {
                entry.stalled_streak = entry.stalled_streak.saturating_add(1);
            }
            // Advancing and FrozenButServing break the streak; a Baseline
            // (first sample, or re-baseline after a restart) does not
            // inherit an old streak either.
            SweepOutcome::Advancing | SweepOutcome::FrozenButServing | SweepOutcome::Baseline => {
                entry.stalled_streak = 0
            }
        }

        entry.row.tokens = sample.tokens;
        entry.row.progress = progress;
        entry.row.failing_to_serve = sample.failing_to_serve;
        entry.row.stalled_streak = entry.stalled_streak;

        Observation {
            outcome,
            stalled_streak: entry.stalled_streak,
            evict: entry.stalled_streak >= self.threshold,
        }
    }

    /// The latest `worker_health` row for one worker.
    pub fn row(&self, worker: &str) -> Option<&WorkerHealthRow> {
        self.workers.get(worker).map(|w| &w.row)
    }

    /// One row per live worker, in worker order: the dashboard can show
    /// progress rate per worker, not only for workers that tripped a probe
    /// timeout.
    pub fn rows(&self) -> Vec<&WorkerHealthRow> {
        self.workers.values().map(|w| &w.row).collect()
    }

    /// The worker's current consecutive stalled-sample streak.
    pub fn stalled_streak(&self, worker: &str) -> u32 {
        self.workers
            .get(worker)
            .map(|w| w.stalled_streak)
            .unwrap_or(0)
    }
}
