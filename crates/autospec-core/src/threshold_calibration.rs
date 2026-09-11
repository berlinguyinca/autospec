//! Measured thresholds and capped destructive automation (issue #4276).
//!
//! A watchdog built on a default constant and a "no output" signal killed 45%
//! of a fleet's healthy agents: the threshold (75 min) sat at the median of
//! measured run durations (4077 s; 190 of 418 runs longer), not the tail,
//! because it was derived from the runner's `LIMIT:-2700` default — the
//! dispatcher's call site passes `LIMIT=25200` (7 h) — and the "no output
//! for 15 min" signal is confoundable by buffered output (issue #4259).
//!
//! The primitives here make each rule of the incident a checkable invariant:
//!
//! 1. **Measure the distribution before building a detector.**
//!    [`Distribution`] and [`ThresholdAudit`]: a threshold at or below the
//!    median of the measured metric is a coin flip applied to healthy work,
//!    not an outlier detector.
//! 2. **Read the caller before trusting a default.**
//!    [`LimitProvenance`]: a value found in the callee's signature is a
//!    default until a call site sets it; the operating value is what the
//!    caller passes, and the default is operating only when no caller
//!    overrides it.
//! 3. **Prefer a detector that normal operation cannot confound.**
//!    [`DetectorSignal`] and [`confoundable`]: "no output for N minutes"
//!    is confoundable (buffered output, quiet phases); "past the limit its
//!    own dispatcher set and still in the call" is not — given the limit
//!    is the operating value, not a default.
//! 4. **Cap destructive automation, log it, and treat the cap as a
//!    measurement window.** [`KillLedger`]: every kill is logged, the
//!    ledger refuses past its cap, and a cap hit whose kills sit at or
//!    below the p90 of normal work is a detector defect, not a fleet
//!    defect.
//! 5. **Asymmetric cost, asymmetric thresholds.** A killed healthy agent
//!    costs a lost run; a wedged agent costs a slot for a bounded time. The
//!    threshold belongs far out in the tail — at the limit the system
//!    itself enforces, not a guess — and [`build_gate`] refuses a
//!    destructive detector that sits anywhere else.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

// --- Rule 1: measure the distribution before building a detector ----------

/// The distribution of the metric a threshold will be applied to, measured
/// before the detector is written. A threshold decided without this
/// structure is a guess, and a guess at the median is a coin flip applied
/// to healthy work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Distribution {
    /// Number of observed runs the distribution was measured over.
    pub count: u32,
    /// p50 of the metric.
    pub median: u64,
    /// p90 of the metric.
    pub p90: u64,
    /// The maximum observed value.
    pub max: u64,
}

impl Distribution {
    /// Compute median (p50) and p90 by nearest-rank.
    ///
    /// Rejects an empty sample: an audit over zero observations is no
    /// audit at all, and a detector built on an empty "distribution" is
    /// the purest form of guessing.
    pub fn from_samples(samples: &[u64]) -> Result<Self, String> {
        if samples.is_empty() {
            return Err(
                "distribution from an empty sample: measure the metric over real runs \
                 before calibrating a threshold against it"
                    .to_string(),
            );
        }
        let mut sorted = samples.to_vec();
        sorted.sort_unstable();
        let n = sorted.len() as u64;
        Ok(Self {
            count: n as u32,
            median: sorted[nearest_rank_index(n, 50)],
            p90: sorted[nearest_rank_index(n, 90)],
            max: *sorted.last().expect("non-empty sample"),
        })
    }
}

/// Nearest-rank percentile, 0-based index: ceil(p/100 * n) - 1.
fn nearest_rank_index(n: u64, percentile: u64) -> usize {
    // ceil(p*n/100) = (p*n + 99)/100, then convert 1-based rank to 0-based.
    let rank = (percentile * n + 99) / 100;
    (rank - 1) as usize
}

/// Where a threshold sits relative to the measured distribution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThresholdPlacement {
    /// Above p90: the threshold would kill fewer than 10% of observed
    /// normal runs. This is the tail; only here is a destructive threshold
    /// an outlier detector.
    Tail,
    /// Above the median but at or below p90: the threshold would kill
    /// 10-50% of normal runs. Not a tail.
    Bulk,
    /// At or below the median: the threshold would kill half or more of
    /// normal runs. A coin flip applied to healthy work.
    Median,
}

/// The audit a run must perform — and log — before a destructive detector
/// is built on a threshold.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThresholdAudit {
    /// The proposed threshold.
    pub threshold: u64,
    /// The measured distribution of the metric the threshold applies to.
    pub distribution: Distribution,
}

impl ThresholdAudit {
    pub fn new(threshold: u64, distribution: Distribution) -> Self {
        Self {
            threshold,
            distribution,
        }
    }

    /// Where the threshold sits in the measured distribution.
    pub fn placement(&self) -> ThresholdPlacement {
        let d = &self.distribution;
        if self.threshold <= d.median {
            ThresholdPlacement::Median
        } else if self.threshold <= d.p90 {
            ThresholdPlacement::Bulk
        } else {
            ThresholdPlacement::Tail
        }
    }

    /// A threshold may stand as an outlier detector only in the tail. A
    /// threshold at or below the median is not an outlier detector,
    /// whatever its units; a threshold at or below p90 kills a meaningful
    /// share of normal work and must be justified as a policy, not
    /// presented as detection.
    pub fn is_outlier_threshold(&self) -> bool {
        matches!(self.placement(), ThresholdPlacement::Tail)
    }

    /// How many of the observed runs this threshold would have killed
    /// (runs strictly longer than the threshold).
    pub fn would_kill(&self, samples: &[u64]) -> u32 {
        samples.iter().filter(|v| **v > self.threshold).count() as u32
    }

    /// The audit line to log before building the detector: the threshold
    /// against the measured distribution and its placement.
    pub fn line(&self) -> String {
        let d = &self.distribution;
        format!(
            "threshold {}s vs measured distribution ({} runs: median {}s p90 {}s max {}s): placement {:?}",
            self.threshold, d.count, d.median, d.p90, d.max, self.placement()
        )
    }

    /// The refusal line when the threshold is not in the tail — the line a
    /// build gate emits instead of building the detector.
    pub fn refusal_line(&self) -> String {
        let d = &self.distribution;
        match self.placement() {
            ThresholdPlacement::Median => format!(
                "refusing destructive threshold {}s: it is at or below the median ({}s) of the \
                 measured distribution — it would kill half or more of healthy work; it is not \
                 an outlier detector",
                self.threshold, d.median
            ),
            ThresholdPlacement::Bulk => format!(
                "refusing destructive threshold {}s: it is at or below p90 ({}s) of the measured \
                 distribution — it would kill a meaningful share of healthy work; move it far \
                 out in the tail, at the limit the system itself enforces",
                self.threshold, d.p90
            ),
            ThresholdPlacement::Tail => unreachable!("refusal_line only renders for non-tail"),
        }
    }
}

// --- Rule 2: read the caller before trusting a default ---------------------

/// A call site that sets a variable a threshold was derived from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallSite {
    /// The name of the site, e.g. `topup.sh submit`.
    pub site: String,
    /// The value the site passes.
    pub value: u64,
}

/// The provenance of a value treated as a constant: what the callee's
/// default says, and what the callers actually pass.
///
/// A value found in the callee's signature (`LIMIT:-2700`, an argument
/// default, a config fallback) is a default until a call site sets it.
/// The default is the operating value only when no caller overrides it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LimitProvenance {
    /// The variable's name, e.g. `LIMIT`.
    pub variable: String,
    /// The callee's default. `None` when the value is required (no
    /// fallback exists).
    pub default: Option<u64>,
    /// Every call site that sets the variable.
    pub callers: Vec<CallSite>,
}

impl LimitProvenance {
    /// The distinct values the callers pass, sorted ascending.
    pub fn distinct_caller_values(&self) -> Vec<u64> {
        let mut vec: Vec<u64> = self
            .callers
            .iter()
            .map(|c| c.value)
            .collect::<BTreeSet<u64>>()
            .into_iter()
            .collect();
        vec.sort_unstable();
        vec
    }

    /// The operating value: what the callers pass.
    ///
    /// `Some` when there is exactly one value in operation — either no
    /// caller sets the variable (the default operates) or every caller
    /// agrees on one value. `None` when the callers disagree (no single
    /// operating value exists; a detector cannot be calibrated on a guess
    /// among them) or when neither a default nor any caller exists.
    pub fn operating_value(&self) -> Option<u64> {
        if self.callers.is_empty() {
            return self.default;
        }
        let distinct = self.distinct_caller_values();
        if distinct.len() == 1 {
            Some(distinct[0])
        } else {
            None
        }
    }

    /// True only in the one case where trusting the default is safe: no
    /// caller overrides it.
    pub fn default_is_operating(&self) -> bool {
        self.callers.is_empty()
    }

    /// The line that names the trap, when the default exists and some
    /// caller passes a different value: the default and the operating
    /// value are different numbers, and a detector built on the default
    /// measures against the wrong fleet.
    pub fn mismatch_line(&self) -> Option<String> {
        let default = self.default?;
        let mismatched: Vec<&CallSite> =
            self.callers.iter().filter(|c| c.value != default).collect();
        if mismatched.is_empty() {
            return None;
        }
        let sites = mismatched
            .iter()
            .map(|c| format!("{}={}", c.site, c.value))
            .collect::<Vec<_>>()
            .join(", ");
        Some(format!(
            "{} default {} is not the operating value: call site(s) {} override it",
            self.variable, default, sites
        ))
    }

    /// The refusal line when no single operating value exists: disagreeing
    /// callers, or no default and no caller. Fail-closed — the detector is
    /// not built until the call sites are read and reconciled.
    pub fn no_operating_value_line(&self) -> String {
        if self.callers.is_empty() {
            format!(
                "no operating value for {}: no default and no call site sets it; \
                 read the caller before trusting a default",
                self.variable
            )
        } else {
            format!(
                "no single operating value for {}: call sites disagree ({})",
                self.variable,
                self.distinct_caller_values()
                    .iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join(" vs ")
            )
        }
    }
}

// --- Rule 3: prefer a detector normal operation cannot confound ------------

/// The signal a detector fires on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DetectorSignal {
    /// "No output for N minutes." Confoundable: buffered output, slow
    /// writes, and quiet phases of healthy work all produce silence
    /// (issue #4259).
    OutputSilence,
    /// "Past the limit its own dispatcher set and still in the call."
    /// Only a wedged agent produces this — given the limit is the
    /// operating value the dispatcher actually set.
    PastOwnLimit,
}

/// Can healthy work produce this signal?
///
/// `OutputSilence` is always confoundable: normal operation buffers,
/// pauses, and writes in bursts. `PastOwnLimit` is confoundable when the
/// threshold sits at or below the operating limit — the detector then
/// fires before the dispatcher's own timeout has had a chance, killing
/// healthy work that is merely long — and when the operating limit is
/// unknown (fail-closed: the detector cannot know which limit its
/// dispatcher set).
pub fn confoundable(signal: DetectorSignal, threshold: u64, operating_limit: Option<u64>) -> bool {
    match signal {
        DetectorSignal::OutputSilence => true,
        DetectorSignal::PastOwnLimit => operating_limit.map_or(true, |limit| threshold <= limit),
    }
}

// --- Rules 4 + 5: the build gate -------------------------------------------

/// The build gate for a destructive detector: the refusal lines, empty
/// when the detector may be built.
///
/// A destructive detector may be built only when every one of these holds:
///
/// - the threshold is in the tail of the measured distribution
///   ([`ThresholdAudit::is_outlier_threshold`]);
/// - a single operating value for the limit exists and the threshold
///   exceeds it — a watchdog racing the timeout its own dispatcher set
///   kills healthy work by design ([`LimitProvenance::operating_value`]);
/// - the signal is not confoundable by normal operation
///   ([`confoundable`]).
pub fn build_gate(
    audit: &ThresholdAudit,
    limit: &LimitProvenance,
    signal: DetectorSignal,
) -> Vec<String> {
    let mut refusals = Vec::new();

    if !audit.is_outlier_threshold() {
        refusals.push(audit.refusal_line());
    }

    let operating = limit.operating_value();
    match operating {
        None => refusals.push(limit.no_operating_value_line()),
        Some(operating) if audit.threshold <= operating => {
            refusals.push(format!(
                "refusing destructive threshold {}s: it does not exceed the operating limit \
                 {}s for {} — the detector would race the timeout its own dispatcher set and \
                 kill healthy work that is merely long",
                audit.threshold, operating, limit.variable
            ));
        }
        Some(_) => {}
    }

    if confoundable(signal, audit.threshold, operating) {
        refusals.push(match signal {
            DetectorSignal::OutputSilence => format!(
                "refusing signal {:?}: normal operation produces it (buffered output, quiet \
                 phases); only \"past the limit its own dispatcher set\" is unconfoundable",
                signal
            ),
            DetectorSignal::PastOwnLimit => format!(
                "refusing signal {:?} at threshold {}s: the operating limit is unknown or not \
                 exceeded, so the detector cannot distinguish a wedged agent from a healthy \
                 long one",
                signal, audit.threshold
            ),
        });
    }

    refusals
}

// --- Rule 4: cap destructive automation and log it --------------------------

/// One recorded destructive action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KillRecord {
    /// The target that was killed, e.g. an agent id.
    pub target: String,
    /// The target's elapsed time at the moment it was killed.
    pub elapsed: u64,
}

/// The outcome of attempting to record a destructive action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KillVerdict {
    /// The action was recorded and logged.
    Recorded,
    /// The cap was reached: the ledger refuses the action. A detector
    /// calibrated on the tail should not exhaust its cap; a cap hit is a
    /// signal to stop and re-measure, not a reason to raise the cap.
    CapReached,
}

/// Capped, logged destructive automation.
///
/// A destructive automation that is uncapped or unlogged cannot be
/// audited into a correction: the 45%-of-healthy-work incident ran
/// exactly like that for a day. The ledger records every action, renders
/// a log line for each, and refuses past its cap. The cap is a
/// measurement window: the kills it recorded are a sample of what the
/// detector calls abnormal, and [`cap_verdict`] reads that sample against
/// the measured distribution of normal work.
#[derive(Debug)]
pub struct KillLedger {
    cap: u32,
    records: Vec<KillRecord>,
}

impl KillLedger {
    /// A ledger with the given cap.
    ///
    /// Rejects a zero cap: an uncapped destructive automation is a ledger
    /// that cannot protect itself.
    pub fn new(cap: u32) -> Result<Self, String> {
        if cap == 0 {
            return Err(
                "kill cap must be at least 1: an uncapped destructive automation cannot \
                 protect itself"
                    .to_string(),
            );
        }
        Ok(Self {
            cap,
            records: Vec::new(),
        })
    }

    /// The cap.
    pub fn cap(&self) -> u32 {
        self.cap
    }

    /// Number of actions recorded.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Whether no actions have been recorded.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Whether the cap has been reached.
    pub fn cap_hit(&self) -> bool {
        self.records.len() as u32 >= self.cap
    }

    /// Record a destructive action, or refuse at the cap.
    pub fn record(&mut self, target: impl Into<String>, elapsed: u64) -> KillVerdict {
        if self.cap_hit() {
            return KillVerdict::CapReached;
        }
        self.records.push(KillRecord {
            target: target.into(),
            elapsed,
        });
        KillVerdict::Recorded
    }

    /// The log line for the most recently recorded action — the line the
    /// automation must emit so the run can be audited after the fact.
    pub fn last_line(&self) -> Option<String> {
        self.records.last().map(|r| {
            format!(
                "kill {}/{}: {} (elapsed {}s)",
                self.records.len(),
                self.cap,
                r.target,
                r.elapsed
            )
        })
    }

    /// The line emitted when the cap is reached. The cap is a measurement
    /// window: the ledger must stop and name the re-measure, not continue.
    pub fn cap_line(&self) -> String {
        format!(
            "kill cap {}/{} reached: stop and re-measure the distribution before any \
             further destructive actions",
            self.records.len(),
            self.cap
        )
    }

    /// The median elapsed time of the recorded kills — the sample of what
    /// the detector calls abnormal.
    pub fn killed_median(&self) -> Option<u64> {
        if self.records.is_empty() {
            return None;
        }
        let mut elapsed: Vec<u64> = self.records.iter().map(|r| r.elapsed).collect();
        elapsed.sort_unstable();
        Some(elapsed[elapsed.len() / 2])
    }
}

/// What the cap window measures: the kills the ledger recorded, read
/// against the measured distribution of normal work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CapVerdict {
    /// The ledger has not hit its cap: nothing to conclude yet.
    NotFull,
    /// The ledger hit its cap and the median of what it killed sits at or
    /// below the p90 of normal work: the detector is killing normal work.
    /// The detector is the defect, not the fleet.
    DetectorDefect,
    /// The ledger hit its cap and the median of what it killed sits beyond
    /// the p90 of normal work: the cap did its job.
    Tail,
}

/// Read the cap window: the kills the ledger recorded, against the
/// measured distribution of normal work.
///
/// A tail-calibrated detector's kills should sit beyond the p90 of normal
/// work. A cap hit whose kills sit at or below the p90 means the detector
/// is killing normal work — the incident's shape: ~15 kills at ~75 min,
/// while 45% of the fleet's healthy runs exceeded 75 min.
pub fn cap_verdict(ledger: &KillLedger, distribution: &Distribution) -> CapVerdict {
    if !ledger.cap_hit() {
        return CapVerdict::NotFull;
    }
    match ledger.killed_median() {
        Some(median) if median <= distribution.p90 => CapVerdict::DetectorDefect,
        Some(_) => CapVerdict::Tail,
        None => CapVerdict::NotFull,
    }
}

/// The line a run emits when the cap is hit: the verdict and the numbers
/// behind it, so the re-measure is named on the same line as the stop.
pub fn cap_verdict_line(ledger: &KillLedger, distribution: &Distribution) -> String {
    let base = ledger.cap_line();
    match cap_verdict(ledger, distribution) {
        CapVerdict::NotFull => base,
        CapVerdict::DetectorDefect => {
            let killed = ledger.killed_median().unwrap_or(0);
            format!(
                "{base}; killed agents' median elapsed {}s is at or below p90 ({}s) of the \
                 measured distribution — the detector is killing normal work",
                killed, distribution.p90
            )
        }
        CapVerdict::Tail => {
            let killed = ledger.killed_median().unwrap_or(0);
            format!(
                "{base}; killed agents' median elapsed {}s is beyond p90 ({}s) — the cap did \
                 its job",
                killed, distribution.p90
            )
        }
    }
}
