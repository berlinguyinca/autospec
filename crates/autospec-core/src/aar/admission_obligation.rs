//! A check must run on the path the condition it detects actually takes
//! (issue #4411).
//!
//! The pitfall this module pins: a throughput floor was added to admission
//! so a CPU-bound worker could not join the pool. It was evaluated after the
//! admission completion returned 200. But a CPU-bound worker is so slow that
//! its admission probe **times out** — and the timeout takes a different
//! branch ("the probe timed out but the endpoint reports this model;
//! admitting, busy is not dead") — which bypasses the floor entirely. The
//! floor was live, correct, tested, and it caught nothing.
//!
//! This is the third instance of one structure in the same component:
//!
//! 1. progress sampling ran only inside the probe-failure branch, so it
//!    needed consecutive failures to conclude anything (#4401);
//! 2. a guard excused workers reporting nothing in flight — on a path only
//!    reached after a probe had already failed;
//! 3. now: a floor evaluated only when the probe succeeds, against workers
//!    whose defining symptom is that it does not.
//!
//! Each fix was locally correct. Each was attached to the wrong branch.
//!
//! **The invariant.** Before placing a validity check, write down what the
//! failing case does — not what the healthy case does — and confirm the
//! check sits on that path. For a check added to a success branch, ask:
//! "what does the thing I am trying to catch do instead of succeeding?" If
//! the answer is "it times out", "it errors", or "it returns nothing", the
//! check is in the wrong place. [`check_covers`] is that question as a
//! predicate, and [`MisplacedCheck::find`] is its finding form for spec
//! review: a spec that adds a guard must state the control flow of the
//! condition it guards against, not only the rule.
//!
//! **The fix shape.** Where a bypass exists for a legitimate reason (a busy
//! worker must not be evicted for being busy), the bypass carries an
//! obligation rather than being an exit: admit provisionally, mark the
//! worker **never measured**, and require the measurement to succeed within
//! a bounded window before the worker counts as healthy. A worker that can
//! never complete a measurement is not the same as one that was merely busy
//! when asked. [`provisional_status`] decides that from cheap evidence.
//!
//! Every function here is pure. Callers perform I/O (probe the worker,
//! record the measurement, evict a worker) with the verdicts and findings
//! these functions return.

use super::inferweave::PoolAction;

/// What the condition a validity check guards against does *instead of
/// succeeding*.
///
/// This is the question placement answers: not "what does the healthy case
/// do" but "what does the thing I am trying to catch do when probed?"
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConditionBehavior {
    /// Completes with a success status: the healthy path.
    Succeeds,
    /// Never completes within its deadline: it takes the timeout branch.
    TimesOut,
    /// Completes with an error status.
    Errors,
    /// Returns nothing — no status, no identity, no list. Observed as an
    /// absence, which lands on the timeout branch, not the success branch.
    ReturnsNothing,
}

impl ConditionBehavior {
    pub fn as_str(&self) -> &'static str {
        match self {
            ConditionBehavior::Succeeds => "succeeds",
            ConditionBehavior::TimesOut => "times out",
            ConditionBehavior::Errors => "errors",
            ConditionBehavior::ReturnsNothing => "returns nothing",
        }
    }
}

/// Where in the probe's control flow a validity check is placed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckPlacement {
    /// The check runs only when the probe completes with a success status
    /// (the 200 branch).
    SuccessBranch,
    /// The check runs only when the probe timed out or returned nothing.
    TimeoutBranch,
    /// The check runs only when the probe completed with an error status.
    ErrorBranch,
    /// The check runs regardless of how the probe attempt ended.
    EveryBranch,
    /// No check is present.
    Nowhere,
}

impl CheckPlacement {
    pub fn as_str(&self) -> &'static str {
        match self {
            CheckPlacement::SuccessBranch => "success",
            CheckPlacement::TimeoutBranch => "timeout",
            CheckPlacement::ErrorBranch => "error",
            CheckPlacement::EveryBranch => "every-branch",
            CheckPlacement::Nowhere => "nowhere",
        }
    }
}

/// Does a check placed at `placement` ever run against a condition that
/// behaves as `condition`?
///
/// The invariant: a check must run on the path the condition it detects
/// actually takes. The incident's throughput floor was a
/// [`CheckPlacement::SuccessBranch`] check against a condition that
/// [`ConditionBehavior::TimesOut`] — it caught nothing, because the check
/// was never on the path that condition took.
pub fn check_covers(placement: CheckPlacement, condition: ConditionBehavior) -> bool {
    match placement {
        CheckPlacement::EveryBranch => true,
        CheckPlacement::Nowhere => false,
        CheckPlacement::SuccessBranch => condition == ConditionBehavior::Succeeds,
        CheckPlacement::TimeoutBranch => matches!(
            condition,
            ConditionBehavior::TimesOut | ConditionBehavior::ReturnsNothing
        ),
        CheckPlacement::ErrorBranch => condition == ConditionBehavior::Errors,
    }
}

/// The finding for a check that sits on a branch the condition it guards
/// never takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MisplacedCheck {
    /// Where the check is placed.
    pub placement: CheckPlacement,
    /// What the condition it guards against does instead of succeeding.
    pub condition: ConditionBehavior,
}

impl MisplacedCheck {
    /// The finding, when the stated control flow does not put the check on
    /// the condition's path.
    ///
    /// A spec that adds a guard must state this control flow — "and here is
    /// what a worker below X does when probed" — not only the rule; that is
    /// the whole difficulty, and a rule without it is not implementable
    /// correctly.
    pub fn find(placement: CheckPlacement, condition: ConditionBehavior) -> Option<MisplacedCheck> {
        if check_covers(placement, condition) {
            None
        } else {
            Some(MisplacedCheck {
                placement,
                condition,
            })
        }
    }

    /// The operator line.
    pub fn line(&self) -> String {
        format!(
            "a check on the {} branch never runs against a condition that {}: \
             a check must run on the path the condition it detects actually takes",
            self.placement.as_str(),
            self.condition.as_str()
        )
    }
}

/// The default window, in seconds, within which a provisionally admitted
/// worker must complete its first measurement.
///
/// Two probe intervals at the default 300s interval: a merely busy worker
/// still gets an unqueued attempt (a queue that drains between requests
/// still completes the probe), while a worker that can never complete a
/// measurement is decided against within a bounded time instead of being
/// kept forever as "busy is not dead".
pub const DEFAULT_MEASUREMENT_WINDOW_SECS: u64 = 600;

/// The status of a worker admitted through the "busy is not dead" bypass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProvisionalStatus {
    /// The measurement succeeded: the worker counts as healthy.
    Healthy,
    /// Never measured and the window is not yet closed: the obligation is
    /// outstanding.
    Owed,
    /// Never measured and the window is closed: a worker that can never
    /// complete a measurement is not the same as one that was merely busy
    /// when asked.
    Unmeasurable,
}

impl ProvisionalStatus {
    /// Only a completed measurement counts the worker as healthy.
    pub fn counts_as_healthy(&self) -> bool {
        matches!(self, ProvisionalStatus::Healthy)
    }

    /// Only a closed window without a measurement evicts. A worker that is
    /// merely owed its measurement is not evicted for being busy.
    pub fn pool_action(&self) -> PoolAction {
        match self {
            ProvisionalStatus::Healthy | ProvisionalStatus::Owed => PoolAction::Keep,
            ProvisionalStatus::Unmeasurable => PoolAction::Evict,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            ProvisionalStatus::Healthy => "healthy",
            ProvisionalStatus::Owed => "owed",
            ProvisionalStatus::Unmeasurable => "unmeasurable",
        }
    }
}

/// Decide the status of a provisionally admitted worker.
///
/// `admitted_at` is when the bypass admission happened;
/// `first_measurement_at` is when the measurement (the liveness completion)
/// first completed, if ever; `window_secs` bounds how long "never measured"
/// may persist before the worker stops counting as healthy.
///
/// A completed measurement makes the worker healthy at any age. Without
/// one, a closed window decides the worker unmeasurable — the bounded
/// escape from "busy is not dead" as a perpetual keep. A clock that rewinds
/// is zero age, never an underflow.
pub fn provisional_status(
    admitted_at: u64,
    first_measurement_at: Option<u64>,
    now: u64,
    window_secs: u64,
) -> ProvisionalStatus {
    if first_measurement_at.is_some() {
        return ProvisionalStatus::Healthy;
    }
    if now.saturating_sub(admitted_at) >= window_secs {
        ProvisionalStatus::Unmeasurable
    } else {
        ProvisionalStatus::Owed
    }
}
