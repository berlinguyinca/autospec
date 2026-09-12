//! Progress signals and update granularity: a metric that only updates on
//! completion cannot detect progress during a long operation (issue #4407).
//!
//! The incident: a stuck-worker detector sampled
//! `llamacpp:prompt_tokens_total` and concluded "no progress" when it did not
//! move. But that counter is incremented when a request *completes*, not as
//! tokens are processed. A worker in the middle of a 40-minute prefill
//! therefore shows a frozen counter while doing exactly the work it is
//! supposed to do. The detector classified two healthy workers as STUCK; they
//! were logging progress lines the whole time
//! (`prompt processing, n_tokens = 4096, progress = 0.10`).
//!
//! This is the third distinct false-positive mode found in the same detector:
//!
//! 1. response-time probes time out on **busy** workers (they queue behind
//!    real work);
//! 2. `requests_processing` reads 0 on a **wedged** worker once its client
//!    gives up;
//! 3. **completion-keyed** counters freeze during a long in-flight operation.
//!
//! The invariant: **a progress signal must be incremented by the work, not by
//! the completion of the work.** Before using a counter as a liveness signal,
//! establish its update granularity — per unit of work, or per completed unit?
//! Only the former can distinguish "slow" from "stopped", and that distinction
//! is the entire purpose of the check. Where no such counter exists, the
//! correct signal is the one the process emits as it works — a progress log
//! line, a partial-result callback, a per-chunk metric — not a total.
//!
//! The primitives make the invariant checkable:
//!
//! 1. [`UpdateGranularity`] — [`UpdateGranularity::PerUnit`] (incremented by
//!    the work) or [`UpdateGranularity::PerCompletion`] (incremented on
//!    completion). [`UpdateGranularity::mid_operation`] states what the metric
//!    does while its operation is in flight: a per-unit signal advances, a
//!    total is frozen.
//! 2. [`liveness_capable`] — only a per-unit signal can distinguish slow from
//!    stopped; a total cannot, because frozen is its normal in-flight state.
//! 3. [`classify_stuck`] — turns a metric, its movement between two samples,
//!    and the process's own in-flight report into a [`StuckVerdict`]. A total
//!    can only ever yield [`StuckVerdict::InFlight`] or
//!    [`StuckVerdict::Inconclusive`] — never [`StuckVerdict::Stopped`]. That
//!    asymmetry *is* the guard: a detector built on totals must additionally
//!    require that the worker is **not** reporting in-flight progress before
//!    concluding anything.
//! 4. [`LivenessSpec`] — the "for specs" rule: a spec that says "detect a
//!    stalled X by watching metric M" must state M's update granularity and
//!    what M does while X is mid-operation; a spec that does not is specified
//!    against a metric whose semantics nobody checked.

/// How a counter is incremented, and therefore what it can be read as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateGranularity {
    /// Incremented by the work itself — one tick per unit of work (a token, a
    /// chunk, a batch). It advances while the operation is in flight, so a
    /// frozen counter means the work stopped.
    PerUnit,
    /// Incremented once, when the work completes. It is frozen while the
    /// operation is in flight, however much work is being done, so a frozen
    /// counter is the *normal* mid-operation state and says nothing about
    /// progress.
    PerCompletion,
}

/// What a metric does while its operation is mid-flight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MidOperation {
    /// Advances as the work proceeds.
    Advances,
    /// Frozen; it will not move again until the operation completes.
    Frozen,
}

impl UpdateGranularity {
    /// What this metric does while its operation is in flight.
    ///
    /// A per-unit signal advances as the work proceeds; a total is frozen,
    /// because it only moves on completion. This is exactly why the two are
    /// not interchangeable as liveness signals: one's frozen state is
    /// evidence of a stop, the other's is the normal in-flight state.
    pub fn mid_operation(self) -> MidOperation {
        match self {
            Self::PerUnit => MidOperation::Advances,
            Self::PerCompletion => MidOperation::Frozen,
        }
    }

    /// The one-word label used in report lines.
    pub fn label(self) -> &'static str {
        match self {
            Self::PerUnit => "per-unit",
            Self::PerCompletion => "per-completion",
        }
    }
}

/// A counter that a detector might read, with the one fact that decides
/// whether it may be read as a liveness signal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Metric {
    /// The counter's name (`"llamacpp:prompt_tokens_total"`).
    pub name: String,
    /// How the counter is incremented.
    pub granularity: UpdateGranularity,
}

/// Whether a metric can distinguish "slow" from "stopped".
///
/// Only a per-unit signal can: it advances while the work proceeds, so a
/// frozen reading is evidence the work stopped. A total is frozen by
/// definition while the operation is in flight, so it cannot tell slow from
/// stopped — and "distinguish slow from stopped" is the entire purpose of a
/// stuck detector.
pub fn liveness_capable(metric: &Metric) -> bool {
    matches!(metric.granularity, UpdateGranularity::PerUnit)
}

/// The process's own in-flight report, observed while it works: the progress
/// log lines, the partial-result callbacks, the per-chunk metrics — the
/// signal a total cannot provide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InFlight {
    /// Whether the process is currently reporting in-flight progress.
    pub reporting: bool,
}

/// What a stuck-worker detector may conclude from one signal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StuckVerdict {
    /// The signal is per-unit and frozen between samples: the work stopped.
    /// This is the only state that may be read as STUCK.
    Stopped,
    /// The signal is per-unit and advancing between samples: the worker is
    /// slow, not stopped.
    Slow,
    /// The signal is a total, frozen, and the worker **is** reporting
    /// in-flight progress: the worker is mid-operation. Concluding STUCK
    /// here is the incident — the two healthy workers logged
    /// `progress = 0.10` the whole time while the total sat still.
    InFlight,
    /// The signal is a total, frozen, and the worker is **not** reporting
    /// in-flight progress. A total cannot call this either: frozen is its
    /// normal in-flight state, so the signal alone is inconclusive — it needs
    /// a per-unit signal, or the process's own report, to decide.
    Inconclusive,
}

impl StuckVerdict {
    /// Whether this verdict may be read as STUCK.
    ///
    /// Only [`StuckVerdict::Stopped`]. A total never reaches it: a detector
    /// built on totals must additionally require that the worker is not
    /// reporting in-flight progress before concluding anything, and even then
    /// it lands on [`StuckVerdict::Inconclusive`], not stuck.
    pub fn is_stuck(&self) -> bool {
        matches!(self, Self::Stopped)
    }

    /// One-line rendering for a detector log or a closeout.
    pub fn line(&self, metric: &Metric) -> String {
        match self {
            Self::Stopped => format!(
                "STUCK: {} is per-unit and frozen between samples — the work stopped",
                metric.name
            ),
            Self::Slow => format!(
                "OK: {} is per-unit and advancing — slow, not stopped",
                metric.name
            ),
            Self::InFlight => format!(
                "OK: {} is a total and frozen, but the worker is reporting in-flight progress — mid-operation, not stuck",
                metric.name
            ),
            Self::Inconclusive => format!(
                "INCONCLUSIVE: {} is a total and frozen and the worker is not reporting in-flight progress — a total cannot call this; a per-unit signal or the process's own report is required",
                metric.name
            ),
        }
    }
}

/// Turn a metric, its movement between two samples, and the process's own
/// in-flight report into a verdict.
///
/// A per-unit signal is read directly: advancing means slow, frozen means
/// stopped. A total is **never** read as stopped: it is frozen while the
/// operation is in flight by definition. For a total the detector falls back
/// to the process's own report — if the worker is reporting in-flight
/// progress it is mid-operation ([`StuckVerdict::InFlight`]); if it is not,
/// the total still cannot call it ([`StuckVerdict::Inconclusive`]). The
/// asymmetry is the guard: a detector built on totals must additionally
/// require that the worker is not reporting in-flight progress before
/// concluding anything, and it can never conclude STUCK from a total alone.
pub fn classify_stuck(metric: &Metric, signal_moving: bool, in_flight: &InFlight) -> StuckVerdict {
    match metric.granularity {
        UpdateGranularity::PerUnit => {
            if signal_moving {
                StuckVerdict::Slow
            } else {
                StuckVerdict::Stopped
            }
        }
        UpdateGranularity::PerCompletion => {
            if in_flight.reporting {
                StuckVerdict::InFlight
            } else {
                StuckVerdict::Inconclusive
            }
        }
    }
}

/// A spec's decision to use a metric as the signal that detects a stall.
///
/// The "for specs" invariant: a spec that says "detect a stalled X by
/// watching metric M" must state M's update granularity and what M does while
/// X is mid-operation. Otherwise the detector is specified against a metric
/// whose semantics nobody checked — which has now happened three times on one
/// component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LivenessSpec {
    /// The metric the spec watches.
    pub metric: Metric,
    /// What the spec states the metric does while its operation is mid-flight.
    /// `None` when the spec did not state it.
    pub stated_mid_operation: Option<MidOperation>,
}

impl LivenessSpec {
    /// Whether the spec states the metric's mid-operation behavior, and
    /// states it correctly.
    ///
    /// The granularity is fixed by [`Metric`]; the stateable fact is what
    /// that granularity implies mid-flight. A spec that omits it is specified
    /// against a metric whose semantics nobody checked, and a spec that states
    /// it *wrong* (e.g. "advances" for a total) has checked the semantics and
    /// found them to belong to a different metric than the one it is actually
    /// watching — the same defect, one step later.
    pub fn states_granularity(&self) -> bool {
        self.stated_mid_operation
            .is_some_and(|stated| stated == self.metric.granularity.mid_operation())
    }

    /// A finding when the spec does not state — or misstates — the metric's
    /// mid-operation behavior. `None` when the spec is complete.
    pub fn finding(&self) -> Option<String> {
        if self.states_granularity() {
            return None;
        }
        let stated = match self.stated_mid_operation {
            Some(s) => format!("states it {}", mid_operation_label(s)),
            None => "states no mid-operation behavior".to_string(),
        };
        Some(format!(
            "SPEC: '{}' is {} but the spec {} — it must state the metric's update granularity and what it does while the operation is mid-operation ({}); a detector specified against a metric whose semantics nobody checked has now failed three times on one component",
            self.metric.name,
            self.metric.granularity.label(),
            stated,
            mid_operation_label(self.metric.granularity.mid_operation()),
        ))
    }
}

fn mid_operation_label(m: MidOperation) -> &'static str {
    match m {
        MidOperation::Advances => "advancing",
        MidOperation::Frozen => "frozen",
    }
}
