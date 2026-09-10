//! Endpoint grants and re-resolution for preemptible worker pools (issue #3746).
//!
//! Dispatch used to hand the agent a single concrete worker endpoint by value
//! at launch. When that worker was preempted (Slurm's `low` partition), the
//! agent died with "Connection error" and the run was lost.
//!
//! This module fixes the dispatch contract with two complementary rules, and
//! the dispatcher uses whichever the deployment supports:
//!
//! - **Gateway fronting** (preferred): a pool installs a stable gateway URL
//!   ([`EndpointPool::set_gateway`]), after which [`EndpointPool::resolve`]
//!   hands out [`Grant::Gateway`] — the agent never holds a direct worker
//!   reference, so a preemption behind the gateway is invisible to it.
//! - **Bounded re-resolution**: for a [`Grant::Direct`] grant, a connection
//!   loss records the dead worker as unreachable and [`ReResolution`]
//!   re-dispatches against *another healthy worker of the same model*,
//!   bounded by attempt count (anti-loop guardrail: bounded by behavior,
//!   not wall clock).
//!
//! Re-resolution is not provider fallback. The spec §15 fail-closed rule
//! still binds at the executor: an executor never re-routes between
//! providers (a GPU failure must not silently become a cloud dispatch).
//! Re-resolution stays inside one model: same model, different worker. A
//! settled failure from a live worker is never re-resolved.
//!
//! **A grant must not outlive the resource guarantee.** A direct grant is
//! minted with `valid_until` equal to the worker's scheduler guarantee
//! horizon, never later, and [`EndpointPool::resolve`] refuses to mint a
//! grant for a worker whose guarantee has already expired. A gateway grant
//! has no worker horizon: the gateway is stable infrastructure, not a
//! scheduler-managed resource.
//!
//! **The agent's budget is derived from the allocation.** The agent's
//! wall-clock budget and the scheduler's allocation are one decision, not
//! two independent limits where the smaller one wins invisibly
//! (issue #3613): [`plan_agent_budget`] derives the budget from the grant's
//! `valid_until` minus a flush margin, and fails closed when an explicit
//! budget would outlive the allocation. [`startup_limits_line`] names both
//! limits at dispatch start. There is no default budget constant.
//!
//! **Statuses are recorded distinctly.** [`classify_status`] maps a finished
//! dispatch to a [`StatusReason`] for `status.txt`, where `connection-error`,
//! `no-output`, and `timeout-partial` are separate stable tokens: a run that
//! lost its endpoint is not a run that produced no output, and a timeout
//! that produced partial work is not a provider error.
//!
//! **A run is never dispatched to a saturated worker.** Each worker
//! declares `total_slots`, and the pool tracks how many of those slots are
//! held by dispatched runs: [`EndpointPool::resolve`] refuses to mint a
//! grant — direct *or* gateway — while every eligible worker's slots are in
//! use ([`PoolError::Saturated`], distinct from [`PoolError::NoCapacity`]:
//! a full fleet is not a dead one), and [`ReResolution`] reserves the slot
//! for each attempt and releases it when the run settles (issue #3758).
//!
//! **Watchdog terminations record the queue state.** A stall watchdog kill
//! is recorded as a [`WatchdogTermination`]: whether the run held a slot at
//! the moment of firing, plus the endpoint's per-worker [`QueueState`
//! snapshot taken at that moment]. `status.txt` distinguishes
//! `no-output-queued` (the run was still waiting for a slot — the fleet's
//! saturation, not the agent's doing) from `no-output-idle` (the run held a
//! slot and produced nothing) (issue #3758).
//!
//! Everything here is pure: the pool and the driver hold in-memory state and
//! callers perform I/O with the grants and steps they return.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use super::executor::{ExecutionStatus, ExecutorResult, FailureClass};

/// Liveness of a worker in the pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkerState {
    /// Serves inference; the scheduler's guarantee holds.
    Healthy,
    /// The connection was lost (e.g. the worker was preempted). Excluded
    /// from resolution until the operator re-registers it.
    Unreachable,
}

/// One inference worker in the pool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Worker {
    pub worker_id: String,
    /// Concrete endpoint URL of this worker.
    pub endpoint: String,
    /// Model id this worker serves.
    pub model: String,
    pub state: WorkerState,
    /// Epoch second until which the scheduler guarantees this allocation.
    /// Direct grants must not outlive this horizon.
    pub guaranteed_until: u64,
    /// Total concurrent run slots this worker serves. Admission control:
    /// no run is dispatched to a worker whose slots are all in use (issue
    /// #3758). Records persisted before that field existed deserialize to
    /// 0, which [`Worker::validate`] rejects — fail closed, never fabricate
    /// capacity.
    #[serde(default)]
    pub total_slots: u32,
}

impl Worker {
    pub fn validate(&self) -> Result<(), PoolError> {
        if self.worker_id.trim().is_empty() {
            return Err(PoolError::Invalid("worker_id must not be empty".into()));
        }
        if self.endpoint.trim().is_empty() {
            return Err(PoolError::Invalid("endpoint must not be empty".into()));
        }
        if self.model.trim().is_empty() {
            return Err(PoolError::Invalid("model must not be empty".into()));
        }
        if self.total_slots == 0 {
            return Err(PoolError::Invalid("total_slots must be at least 1".into()));
        }
        Ok(())
    }
}

/// The value dispatch hands the agent at launch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Grant {
    /// Stable front onto the pool. The holder never references a direct
    /// worker; preemption behind the gateway is invisible.
    Gateway { url: String },
    /// Concrete worker reference, valid only until `valid_until` — minted
    /// equal to the worker's `guaranteed_until`, never later.
    Direct {
        worker_id: String,
        endpoint: String,
        valid_until: u64,
    },
}

impl Grant {
    /// The worker behind this grant, if any (direct grants only).
    pub fn worker_id(&self) -> Option<&str> {
        match self {
            Self::Gateway { .. } => None,
            Self::Direct { worker_id, .. } => Some(worker_id),
        }
    }

    /// Whether the grant has passed its validity horizon at `now`.
    /// Gateway grants carry no horizon and never expire on their own.
    pub fn expired(&self, now: u64) -> bool {
        match self {
            Self::Gateway { .. } => false,
            Self::Direct { valid_until, .. } => now >= *valid_until,
        }
    }

    pub fn validate(&self) -> Result<(), PoolError> {
        match self {
            Self::Gateway { url } if url.trim().is_empty() => Err(PoolError::Invalid(
                "gateway grant url must not be empty".into(),
            )),
            Self::Direct {
                worker_id,
                endpoint,
                ..
            } if worker_id.trim().is_empty() || endpoint.trim().is_empty() => Err(
                PoolError::Invalid("direct grant worker_id and endpoint must not be empty".into()),
            ),
            _ => Ok(()),
        }
    }
}

/// Why a pool operation failed. Fail closed: the pool never returns a stale
/// grant, never resolves to a worker it does not know, and never admits a
/// run to a worker with no free slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PoolError {
    /// No healthy worker serving `model` with a live guarantee at resolve
    /// time; dispatch must not proceed on a grant that would expire
    /// immediately.
    NoCapacity { model: String },
    /// Healthy workers serving `model` exist, but every one of their slots
    /// is already held by a dispatched run. Distinct from
    /// [`PoolError::NoCapacity`]: the fleet is healthy but full, so the
    /// answer is "wait for a slot or grow the fleet", not "fix a dead
    /// worker" (issue #3758).
    Saturated { model: String },
    /// A worker id that was never registered in this pool.
    UnknownWorker(String),
    /// A malformed pool entry or driver state (empty id, duplicate worker,
    /// zero attempt budget).
    Invalid(String),
}

impl fmt::Display for PoolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoCapacity { model } => write!(f, "no healthy capacity for model {model}"),
            Self::Saturated { model } => {
                write!(f, "model {model} is saturated: every slot is in use")
            }
            Self::UnknownWorker(id) => write!(f, "unknown worker: {id}"),
            Self::Invalid(msg) => write!(f, "invalid: {msg}"),
        }
    }
}

impl std::error::Error for PoolError {}

/// The inference worker pool behind one model deployment.
#[derive(Debug, Default)]
pub struct EndpointPool {
    workers: BTreeMap<String, Worker>,
    /// Slots currently held by dispatched runs, per worker id. A worker
    /// absent from this map holds zero slots (issue #3758).
    in_use: BTreeMap<String, u32>,
    gateway: Option<String>,
}

impl EndpointPool {
    pub fn new() -> Self {
        Self::default()
    }

    /// The installed gateway URL, if any.
    pub fn gateway_url(&self) -> Option<&str> {
        self.gateway.as_deref()
    }

    /// Install the pool front. After this, resolution hands out
    /// [`Grant::Gateway`] — agents no longer hold direct worker references.
    pub fn set_gateway(&mut self, url: &str) -> Result<(), PoolError> {
        if url.trim().is_empty() {
            return Err(PoolError::Invalid("gateway url must not be empty".into()));
        }
        self.gateway = Some(url.to_string());
        Ok(())
    }

    /// Register a worker. Rejects malformed entries and duplicate ids.
    pub fn register(&mut self, worker: Worker) -> Result<(), PoolError> {
        worker.validate()?;
        if self.workers.contains_key(&worker.worker_id) {
            return Err(PoolError::Invalid(format!(
                "worker already registered: {}",
                worker.worker_id
            )));
        }
        self.workers.insert(worker.worker_id.clone(), worker);
        Ok(())
    }

    pub fn worker(&self, worker_id: &str) -> Option<&Worker> {
        self.workers.get(worker_id)
    }

    /// Healthy workers serving `model` whose guarantee is still live at
    /// `now`.
    pub fn healthy_capacity(&self, model: &str, now: u64) -> usize {
        self.workers
            .values()
            .filter(|w| {
                w.state == WorkerState::Healthy && w.model == model && w.guaranteed_until > now
            })
            .count()
    }

    /// Slots still free on `worker_id`, or `None` if the worker was never
    /// registered.
    pub fn free_slots(&self, worker_id: &str) -> Option<u32> {
        let worker = self.workers.get(worker_id)?;
        let held = self.in_use.get(worker_id).copied().unwrap_or(0);
        Some(worker.total_slots - held)
    }

    /// Reserve one slot on `worker_id` for a dispatched run. Fails closed
    /// with [`PoolError::Saturated`] when the worker has no free slot —
    /// that is the admission rule: a run is never dispatched to a worker
    /// with zero free slots (issue #3758).
    pub fn acquire(&mut self, worker_id: &str) -> Result<(), PoolError> {
        let worker = self
            .workers
            .get(worker_id)
            .ok_or_else(|| PoolError::UnknownWorker(worker_id.to_string()))?;
        let held = self.in_use.get(worker_id).copied().unwrap_or(0);
        if held >= worker.total_slots {
            return Err(PoolError::Saturated {
                model: worker.model.clone(),
            });
        }
        *self.in_use.entry(worker_id.to_string()).or_insert(0) += 1;
        Ok(())
    }

    /// Release one slot previously reserved on `worker_id`. Releasing below
    /// zero is driver-state corruption and fails closed.
    pub fn release(&mut self, worker_id: &str) -> Result<(), PoolError> {
        self.workers
            .get(worker_id)
            .ok_or_else(|| PoolError::UnknownWorker(worker_id.to_string()))?;
        let Some(held) = self.in_use.get_mut(worker_id) else {
            return Err(PoolError::Invalid(format!(
                "worker {worker_id} holds no slot"
            )));
        };
        if *held == 0 {
            return Err(PoolError::Invalid(format!(
                "worker {worker_id} holds no slot"
            )));
        }
        *held -= 1;
        if *held == 0 {
            self.in_use.remove(worker_id);
        }
        Ok(())
    }

    /// Free slots across every healthy, guarantee-live worker serving
    /// `model` at `now`: how many more runs the fleet can admit.
    pub fn free_slot_capacity(&self, model: &str, now: u64) -> u32 {
        self.workers
            .values()
            .filter(|w| {
                w.state == WorkerState::Healthy && w.model == model && w.guaranteed_until > now
            })
            .map(|w| {
                let held = self.in_use.get(&w.worker_id).copied().unwrap_or(0);
                w.total_slots - held
            })
            .sum()
    }

    /// The endpoint's queue state at `now` for `model` (issue #3758):
    /// per-worker occupancy over every healthy, guarantee-live worker —
    /// including fully occupied ones, which is exactly the evidence a
    /// watchdog post-mortem needs — plus the aggregate free slots. A pure
    /// read: taking a snapshot never mutates the pool.
    pub fn queue_state(&self, model: &str, now: u64) -> QueueState {
        let workers: Vec<WorkerOccupancy> = self
            .workers
            .values()
            .filter(|w| {
                w.state == WorkerState::Healthy && w.model == model && w.guaranteed_until > now
            })
            .map(|w| WorkerOccupancy {
                worker_id: w.worker_id.clone(),
                in_use: self.in_use.get(&w.worker_id).copied().unwrap_or(0),
                total_slots: w.total_slots,
            })
            .collect();
        let free_slots = workers.iter().map(|w| w.total_slots - w.in_use).sum();
        QueueState {
            model: model.to_string(),
            workers,
            free_slots,
        }
    }

    /// The grant for dispatching `model` at `now`.
    ///
    /// When a gateway is installed the grant is the gateway — the agent
    /// never holds a direct worker reference. Otherwise the grant is a
    /// deterministic (lowest worker id) direct reference, minted with
    /// `valid_until` equal to the worker's guarantee horizon so the value
    /// never outlives the resource guarantee. Fails closed with
    /// [`PoolError::NoCapacity`] when the pool is empty or every candidate
    /// guarantee has expired.
    pub fn resolve(&self, model: &str, now: u64) -> Result<Grant, PoolError> {
        if model.trim().is_empty() {
            return Err(PoolError::Invalid("model must not be empty".into()));
        }
        if self.healthy_capacity(model, now) == 0 {
            return Err(PoolError::NoCapacity {
                model: model.to_string(),
            });
        }
        // Admission control (issue #3758): a run is never dispatched to a
        // worker with zero free slots — and not via the gateway either, a
        // dispatch the fleet cannot serve would only wait. Saturated is
        // distinct from NoCapacity: the fleet is healthy but full.
        if self.free_slot_capacity(model, now) == 0 {
            return Err(PoolError::Saturated {
                model: model.to_string(),
            });
        }
        if let Some(url) = &self.gateway {
            return Ok(Grant::Gateway { url: url.clone() });
        }
        let worker = self
            .workers
            .values()
            .filter(|w| {
                w.state == WorkerState::Healthy
                    && w.model == model
                    && w.guaranteed_until > now
                    && self.free_slots(&w.worker_id).unwrap_or(0) > 0
            })
            .min_by(|a, b| a.worker_id.cmp(&b.worker_id))
            .expect("a worker with a free slot was checked above");
        Ok(Grant::Direct {
            worker_id: worker.worker_id.clone(),
            endpoint: worker.endpoint.clone(),
            valid_until: worker.guaranteed_until,
        })
    }

    /// Record that `worker_id` lost its connection (e.g. preemption).
    /// Idempotent: re-recording an already-unreachable worker is a no-op.
    pub fn mark_unreachable(&mut self, worker_id: &str) -> Result<(), PoolError> {
        let worker = self
            .workers
            .get_mut(worker_id)
            .ok_or_else(|| PoolError::UnknownWorker(worker_id.to_string()))?;
        worker.state = WorkerState::Unreachable;
        // The allocation is gone: every slot the worker held dies with it,
        // so the reservations are voided, not leaked.
        self.in_use.remove(worker_id);
        Ok(())
    }

    /// Re-resolution after a connection loss: record the failed worker, if
    /// any, then resolve again — always against the same model. The failed
    /// worker is never re-granted: it is unreachable by definition.
    pub fn re_resolve(
        &mut self,
        model: &str,
        failed_worker_id: Option<&str>,
        now: u64,
    ) -> Result<Grant, PoolError> {
        if let Some(id) = failed_worker_id {
            self.mark_unreachable(id)?;
        }
        self.resolve(model, now)
    }
}

/// Occupancy of one worker in a [`QueueState`] snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerOccupancy {
    pub worker_id: String,
    /// Slots currently held by dispatched runs.
    pub in_use: u32,
    /// Total slots the worker serves.
    pub total_slots: u32,
}

impl WorkerOccupancy {
    /// Slots still free on this worker.
    pub fn free_slots(&self) -> u32 {
        self.total_slots - self.in_use
    }
}

/// The endpoint's queue state at one instant for one model (issue #3758).
///
/// This is the evidence a watchdog termination records so a no-output kill
/// can be attributed after the fact: was the run still waiting for a slot,
/// or did it hold a slot and produce nothing? Fully occupied workers appear
/// in the snapshot — saturation is the interesting case, not the absence of
/// workers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueState {
    /// Model the snapshot was taken for.
    pub model: String,
    /// Every healthy, guarantee-live worker serving `model`, in worker-id
    /// order.
    pub workers: Vec<WorkerOccupancy>,
    /// Sum of free slots over `workers`; `0` means the fleet is saturated
    /// and a new run can only wait.
    pub free_slots: u32,
}

/// What `status.txt` records for a run. The tokens are stable: monitors and
/// post-mortems branch on them, and `connection-error` and `no-output` must
/// stay distinct — a run that lost its endpoint is not a run that produced
/// no output (issue #3746).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StatusReason {
    /// The run produced output or a patch.
    Completed,
    /// The connection to the granted endpoint died (e.g. the worker was
    /// preempted). Distinct from [`StatusReason::NoOutput`].
    ConnectionError,
    /// The endpoint was reachable but the run produced neither output nor a
    /// patch — including a timeout that spent its budget without producing
    /// anything. Recorded by [`classify_status`], which has no queue
    /// evidence; watchdog terminations use the more specific
    /// [`StatusReason::NoOutputQueued`] / [`StatusReason::NoOutputIdle`]
    /// instead.
    NoOutput,
    /// A stall watchdog killed a run that had never been admitted to a
    /// worker slot: it was still queued, waiting for capacity. The stall
    /// is the fleet's saturation, not the agent's (issue #3758). Recorded
    /// by [`classify_watchdog_termination`].
    NoOutputQueued,
    /// A stall watchdog killed a run that held a worker slot: the endpoint
    /// was alive, the run had capacity, and it produced nothing (issue
    /// #3758). Recorded by [`classify_watchdog_termination`].
    NoOutputIdle,
    /// The dispatch hit its wall-clock budget after producing output or a
    /// patch. The partial work must be preserved — and the run is not a
    /// `provider-error`: the endpoint was alive, the budget simply ran out
    /// (issue #3613).
    TimeoutPartial,
    /// The dispatch failed at or beyond a live endpoint: provider error or
    /// unclassified failure.
    ProviderError,
}

impl StatusReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::ConnectionError => "connection-error",
            Self::NoOutput => "no-output",
            Self::NoOutputQueued => "no-output-queued",
            Self::NoOutputIdle => "no-output-idle",
            Self::TimeoutPartial => "timeout-partial",
            Self::ProviderError => "provider-error",
        }
    }

    /// Fail closed: unknown tokens are an error, never a guess.
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "completed" => Ok(Self::Completed),
            "connection-error" => Ok(Self::ConnectionError),
            "no-output" => Ok(Self::NoOutput),
            "no-output-queued" => Ok(Self::NoOutputQueued),
            "no-output-idle" => Ok(Self::NoOutputIdle),
            "timeout-partial" => Ok(Self::TimeoutPartial),
            "provider-error" => Ok(Self::ProviderError),
            other => Err(format!("unknown status reason: {other}")),
        }
    }
}

/// Classify a finished dispatch for `status.txt`.
///
/// A connection-level failure takes precedence over everything else: once
/// the endpoint is lost, the (possibly empty) output says nothing about the
/// run. A timeout is not a provider error: it is classified by what the
/// agent managed to produce before its budget ran out — partial output is
/// recorded as `timeout-partial` so the work is preserved, and a budget
/// spent without any output is `no-output` (issue #3613). An unclassified
/// dispatch that produced neither output nor a patch is `no-output`, never
/// `completed`.
pub fn classify_status(result: &ExecutorResult) -> StatusReason {
    let produced = !result.output.trim().is_empty()
        || result
            .patch
            .as_deref()
            .is_some_and(|patch| !patch.trim().is_empty());
    match result.failure_class {
        Some(FailureClass::ProviderUnavailable) => StatusReason::ConnectionError,
        Some(FailureClass::Timeout) if produced => StatusReason::TimeoutPartial,
        Some(FailureClass::Timeout) => StatusReason::NoOutput,
        Some(_) => StatusReason::ProviderError,
        None if !produced => StatusReason::NoOutput,
        None => StatusReason::Completed,
    }
}

/// A stall watchdog termination of a run, recorded with the endpoint's
/// queue state at the moment of firing (issue #3758).
///
/// The watchdog itself lives in the deployment; this is what it records.
/// `held_slot` is the dispatch side's fact — whether the run had been
/// admitted to a worker slot — and it is the classification key.
/// `queue_state` is the supporting evidence: the per-worker occupancy the
/// operator needs to see *why* a run stalled without re-running the fleet
/// at post-mortem time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchdogTermination {
    /// Whether the run held a worker slot at the moment the watchdog
    /// fired. `false`: the run was still in the queue — it never had a
    /// slot to produce output on.
    pub held_slot: bool,
    /// The endpoint's queue state at the moment the watchdog fired.
    pub queue_state: QueueState,
}

impl WatchdogTermination {
    /// The [`StatusReason`] `status.txt` records for this termination.
    pub fn status(&self) -> StatusReason {
        classify_watchdog_termination(self)
    }

    /// The `status.txt` line: the classified token plus the queue state at
    /// the moment of firing, e.g.
    /// `status=no-output-queued held_slot=false free_slots=0 workers=w1:16/16,w2:0/4`.
    /// Workers are rendered `worker_id:in_use/total` in snapshot order.
    pub fn status_line(&self) -> String {
        let workers = if self.queue_state.workers.is_empty() {
            "-".to_string()
        } else {
            self.queue_state
                .workers
                .iter()
                .map(|w| format!("{}:{}/{}", w.worker_id, w.in_use, w.total_slots))
                .collect::<Vec<_>>()
                .join(",")
        };
        format!(
            "status={} held_slot={} free_slots={} workers={}",
            self.status().as_str(),
            self.held_slot,
            self.queue_state.free_slots,
            workers
        )
    }
}

/// Classify a watchdog termination (issue #3758).
///
/// The discriminator is `held_slot`, not `free_slots`: a run that holds
/// the last slot of a saturated fleet sees `free_slots == 0` yet is idle —
/// it had a slot and produced nothing. `free_slots` is evidence recorded
/// for the post-mortem; the dispatch side's admission fact is the
/// classification.
pub fn classify_watchdog_termination(t: &WatchdogTermination) -> StatusReason {
    if t.held_slot {
        StatusReason::NoOutputIdle
    } else {
        StatusReason::NoOutputQueued
    }
}

/// The agent's wall-clock budget for a dispatch against `grant` at `now`
/// (issue #3613).
///
/// `explicit_secs` is the operator's budget (`ExecutorRequest::timeout_secs`);
/// `margin_secs` is the headroom the dispatcher needs between the agent
/// stopping and the scheduler killing the allocation, so partial output can
/// be flushed before the job dies. The budget and the allocation are related
/// by construction:
///
/// - `None` derives the budget from the allocation: the seconds between
///   `now` and the grant's `valid_until`, minus the margin. There is no
///   default budget constant — a budget that is not derived from the
///   allocation does not exist.
/// - `Some(budget)` is used as-is when it fits inside the derived budget, and
///   is a fail-closed [`PoolError::Invalid`] naming both numbers when it
///   outlives the allocation or eats the flush margin.
///
/// Gateway grants carry no allocation horizon, so neither derivation nor
/// checking is possible; a budget against a gateway grant is the operator's
/// to own, and this function fails closed rather than guessing (spec §15).
pub fn plan_agent_budget(
    grant: &Grant,
    now: u64,
    explicit_secs: Option<u64>,
    margin_secs: u64,
) -> Result<u64, PoolError> {
    let Grant::Direct { valid_until, .. } = grant else {
        return Err(PoolError::Invalid(
            "gateway grants carry no allocation horizon; the agent budget must be owned by the operator, not derived"
                .into(),
        ));
    };
    if *valid_until <= now {
        return Err(PoolError::Invalid(format!(
            "allocation horizon {valid_until} is at or before now {now}; no budget can be planned"
        )));
    }
    let remaining = valid_until - now;
    let derived = remaining.saturating_sub(margin_secs);
    if derived == 0 {
        return Err(PoolError::Invalid(format!(
            "allocation has {remaining}s remaining and the {margin_secs}s flush margin leaves no agent budget"
        )));
    }
    match explicit_secs {
        Some(budget) if budget <= derived => Ok(budget),
        Some(budget) => Err(PoolError::Invalid(format!(
            "explicit agent budget {budget}s outlives the allocation: {remaining}s remaining minus a {margin_secs}s flush margin leaves {derived}s"
        ))),
        None => Ok(derived),
    }
}

/// The startup log line naming both limits, e.g.
/// `walltime=4h agent_limit=45m` (issue #3613): the allocation the
/// scheduler owns and the budget the agent is actually given.
pub fn startup_limits_line(remaining_secs: u64, agent_budget_secs: u64) -> String {
    format!(
        "walltime={} agent_limit={}",
        format_limits_duration(remaining_secs),
        format_limits_duration(agent_budget_secs)
    )
}

fn format_limits_duration(secs: u64) -> String {
    if secs % 3600 == 0 {
        format!("{}h", secs / 3600)
    } else if secs % 60 == 0 {
        format!("{}m", secs / 60)
    } else {
        format!("{}s", secs)
    }
}

/// What happened to a dispatched attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttemptOutcome {
    /// The attempt reached a terminal result (any status); re-resolution
    /// does not apply — the worker was reachable, so the outcome is the
    /// provider's or the agent's.
    Settled(StatusReason),
    /// The connection to the granted endpoint died before the run
    /// finished, e.g. a preempted worker.
    ConnectionLost,
}

/// The next dispatch step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Dispatch against this grant; `attempt` counts from 1.
    Dispatch { grant: Grant, attempt: u32 },
    /// Stop; `status` is what `status.txt` records.
    Halt { status: StatusReason },
}

/// Bounded re-resolution across attempts for one run — the mechanism by
/// which a preempted worker does not kill the agent.
///
/// The bound is on attempt count, not wall clock (anti-loop guardrail): at
/// most `max_attempts` dispatches in total, and re-resolution happens only
/// on connection loss and only against another healthy worker of the same
/// model. A settled outcome from a live worker is never re-resolved (spec
/// §15: no provider fallback).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReResolution {
    model: String,
    max_attempts: u32,
    attempts: u32,
    last_worker: Option<String>,
}

impl ReResolution {
    /// Default attempt budget: the first dispatch plus two re-resolutions.
    pub const DEFAULT_MAX_ATTEMPTS: u32 = 3;

    /// `max_attempts` is the total dispatch count, including the first; it
    /// must be at least 1.
    pub fn new(model: impl Into<String>, max_attempts: u32) -> Result<Self, PoolError> {
        let model = model.into();
        if model.trim().is_empty() {
            return Err(PoolError::Invalid("model must not be empty".into()));
        }
        if max_attempts == 0 {
            return Err(PoolError::Invalid("max_attempts must be at least 1".into()));
        }
        Ok(Self {
            model,
            max_attempts,
            attempts: 0,
            last_worker: None,
        })
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn max_attempts(&self) -> u32 {
        self.max_attempts
    }

    pub fn attempts(&self) -> u32 {
        self.attempts
    }

    /// The first dispatch step for the run's model at `now`.
    ///
    /// For a direct grant the step also reserves one slot on the granted
    /// worker (issue #3758): resolution checks that a free slot exists, and
    /// the reservation makes that check binding — a saturated worker can
    /// never accumulate more runs than slots.
    pub fn start(&mut self, pool: &mut EndpointPool, now: u64) -> Result<Step, PoolError> {
        let grant = pool.resolve(&self.model, now)?;
        if let Some(worker_id) = grant.worker_id() {
            pool.acquire(worker_id)?;
        }
        self.attempts += 1;
        self.last_worker = grant.worker_id().map(str::to_string);
        Ok(Step::Dispatch {
            grant,
            attempt: self.attempts,
        })
    }

    /// Feed the outcome of the last dispatched attempt and get the next
    /// step.
    ///
    /// - [`AttemptOutcome::Settled`] halts with that status: a live
    ///   worker's failure is never re-resolved.
    /// - [`AttemptOutcome::ConnectionLost`] on a direct grant records the
    ///   dead worker and re-dispatches against another healthy worker of
    ///   the same model, until the attempt budget is exhausted or capacity
    ///   runs out — either way halting with [`StatusReason::ConnectionError`].
    /// - [`AttemptOutcome::ConnectionLost`] on a gateway grant halts at
    ///   once: a dead gateway is an infrastructure failure, not worker
    ///   preemption, and re-dispatching the same gateway would spin.
    ///
    /// Slot lifecycle (issue #3758): a settled run releases the slot it
    /// held; a lost connection voids the failed worker's slots via
    /// [`EndpointPool::re_resolve`] (the allocation is preempted) and the
    /// replacement grant acquires a fresh slot.
    pub fn on_outcome(
        &mut self,
        pool: &mut EndpointPool,
        outcome: AttemptOutcome,
        now: u64,
    ) -> Result<Step, PoolError> {
        if let AttemptOutcome::Settled(status) = outcome {
            // The run is over: give the slot back. `last_worker` is set
            // only for direct grants, which always acquired.
            if let Some(held) = &self.last_worker {
                pool.release(held)?;
            }
            return Ok(Step::Halt { status });
        }
        let Some(failed) = &self.last_worker else {
            return Ok(Step::Halt {
                status: StatusReason::ConnectionError,
            });
        };
        if self.attempts >= self.max_attempts {
            return Ok(Step::Halt {
                status: StatusReason::ConnectionError,
            });
        }
        match pool.re_resolve(&self.model, Some(failed), now) {
            Ok(grant) => {
                if let Some(worker_id) = grant.worker_id() {
                    pool.acquire(worker_id)?;
                }
                self.attempts += 1;
                self.last_worker = grant.worker_id().map(str::to_string);
                Ok(Step::Dispatch {
                    grant,
                    attempt: self.attempts,
                })
            }
            // The run stopped because the endpoint was lost and no
            // replacement can take it over — dead, or every surviving slot
            // held: record it as such.
            Err(PoolError::NoCapacity { .. }) | Err(PoolError::Saturated { .. }) => {
                Ok(Step::Halt {
                    status: StatusReason::ConnectionError,
                })
            }
            Err(err) => Err(err),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn worker(id: &str, model: &str, until: u64) -> Worker {
        worker_slots(id, model, until, 1)
    }

    fn worker_slots(id: &str, model: &str, until: u64, total_slots: u32) -> Worker {
        Worker {
            worker_id: id.into(),
            endpoint: format!("http://{id}:8080"),
            model: model.into(),
            state: WorkerState::Healthy,
            guaranteed_until: until,
            total_slots,
        }
    }

    fn executor_result(
        status: ExecutionStatus,
        failure_class: Option<FailureClass>,
        output: &str,
        patch: Option<&str>,
    ) -> ExecutorResult {
        ExecutorResult {
            status,
            output: output.into(),
            patch: patch.map(str::to_string),
            input_tokens: None,
            output_tokens: None,
            cached_tokens: None,
            prompt_tok_s: None,
            decode_tok_s: None,
            ttft_ms: None,
            wall_clock_ms: None,
            tool_calls: None,
            failure_class,
        }
    }

    #[test]
    fn worker_rejects_empty_fields() {
        let mut w = worker("w1", "m", 100);
        w.worker_id = "  ".into();
        assert!(matches!(w.validate(), Err(PoolError::Invalid(_))));
        w = worker("w1", "m", 100);
        w.endpoint = "".into();
        assert!(matches!(w.validate(), Err(PoolError::Invalid(_))));
        w = worker("w1", "m", 100);
        w.model = " ".into();
        assert!(matches!(w.validate(), Err(PoolError::Invalid(_))));
        assert_eq!(worker("w1", "m", 0).validate(), Ok(()));
    }

    #[test]
    fn worker_zero_slots_fails_closed() {
        // A worker that serves no slots can admit no run: validation
        // rejects it rather than letting the pool pretend it has
        // capacity.
        let zero = worker_slots("w1", "m", 100, 0);
        assert!(matches!(zero.validate(), Err(PoolError::Invalid(_))));
        let mut pool = EndpointPool::new();
        assert!(matches!(pool.register(zero), Err(PoolError::Invalid(_))));
        // Pool state persisted before total_slots existed deserializes to
        // 0 and fails closed on re-validation: old records are never read
        // as healthy capacity.
        let legacy: Worker = serde_json::from_str(
            r#"{"worker_id":"w1","endpoint":"http://w1:8080","model":"m","state":"healthy","guaranteed_until":100}"#,
        )
        .unwrap();
        assert_eq!(legacy.total_slots, 0);
        assert!(matches!(legacy.validate(), Err(PoolError::Invalid(_))));
    }

    #[test]
    fn grant_worker_id_and_expiry() {
        let direct = Grant::Direct {
            worker_id: "w1".into(),
            endpoint: "http://w1".into(),
            valid_until: 100,
        };
        assert_eq!(direct.worker_id(), Some("w1"));
        assert!(!direct.expired(99));
        assert!(direct.expired(100));
        let gateway = Grant::Gateway {
            url: "http://gw".into(),
        };
        assert_eq!(gateway.worker_id(), None);
        assert!(!gateway.expired(u64::MAX));
    }

    #[test]
    fn grant_serde_round_trip() {
        let direct = Grant::Direct {
            worker_id: "w1".into(),
            endpoint: "http://w1:8080".into(),
            valid_until: 100,
        };
        let json = serde_json::to_string(&direct).unwrap();
        assert_eq!(
            json,
            r#"{"kind":"direct","worker_id":"w1","endpoint":"http://w1:8080","valid_until":100}"#
        );
        assert_eq!(serde_json::from_str::<Grant>(&json).unwrap(), direct);
        let gateway = Grant::Gateway {
            url: "http://gw".into(),
        };
        let json = serde_json::to_string(&gateway).unwrap();
        assert_eq!(json, r#"{"kind":"gateway","url":"http://gw"}"#);
        assert_eq!(serde_json::from_str::<Grant>(&json).unwrap(), gateway);
    }

    #[test]
    fn register_rejects_malformed_and_duplicates() {
        let mut pool = EndpointPool::new();
        pool.register(worker("w1", "m", 100)).unwrap();
        assert!(matches!(
            pool.register(worker("w1", "m", 200)),
            Err(PoolError::Invalid(_))
        ));
        let mut bad = worker("w2", "m", 100);
        bad.endpoint = "".into();
        assert!(matches!(pool.register(bad), Err(PoolError::Invalid(_))));
        assert_eq!(pool.healthy_capacity("m", 50), 1);
    }

    #[test]
    fn set_gateway_rejects_empty_url() {
        let mut pool = EndpointPool::new();
        assert!(matches!(pool.set_gateway("  "), Err(PoolError::Invalid(_))));
        pool.set_gateway("http://gw:8080").unwrap();
        assert_eq!(pool.gateway_url(), Some("http://gw:8080"));
    }

    #[test]
    fn resolve_hands_out_gateway_when_installed() {
        let mut pool = EndpointPool::new();
        pool.register(worker("w1", "m", 100)).unwrap();
        pool.register(worker("w2", "m", 200)).unwrap();
        pool.set_gateway("http://gw:8080").unwrap();
        assert_eq!(
            pool.resolve("m", 50).unwrap(),
            Grant::Gateway {
                url: "http://gw:8080".into()
            }
        );
    }

    #[test]
    fn resolve_direct_never_outlives_the_guarantee() {
        let mut pool = EndpointPool::new();
        pool.register(worker("w-a", "m", 100)).unwrap();
        pool.register(worker("w-b", "m", 500)).unwrap();
        // w-a's guarantee ends at 100: at now == 100 it is already stale.
        let grant = pool.resolve("m", 100).unwrap();
        match grant {
            Grant::Direct {
                worker_id,
                endpoint,
                valid_until,
            } => {
                assert_eq!(worker_id, "w-b");
                assert_eq!(endpoint, "http://w-b:8080");
                assert_eq!(valid_until, 500);
            }
            other => panic!("expected direct grant, got {other:?}"),
        }
        assert_eq!(pool.resolve("m", 50).unwrap().worker_id(), Some("w-a"));
        assert_eq!(pool.resolve("m", 499).unwrap().worker_id(), Some("w-b"));
        assert!(matches!(
            pool.resolve("m", 500),
            Err(PoolError::NoCapacity { .. })
        ));
    }

    #[test]
    fn resolve_fails_closed_without_capacity() {
        let pool = EndpointPool::new();
        assert!(matches!(
            pool.resolve("m", 0),
            Err(PoolError::NoCapacity { .. })
        ));
        let mut pool = EndpointPool::new();
        pool.register(worker("w1", "m", 100)).unwrap();
        pool.mark_unreachable("w1").unwrap();
        assert!(matches!(
            pool.resolve("m", 0),
            Err(PoolError::NoCapacity { .. })
        ));
        let mut pool = EndpointPool::new();
        pool.register(worker("w1", "m", 100)).unwrap();
        assert!(matches!(pool.resolve(" ", 0), Err(PoolError::Invalid(_))));
    }

    #[test]
    fn resolve_is_deterministic_and_model_scoped() {
        let mut pool = EndpointPool::new();
        pool.register(worker("w-c", "model-a", 1000)).unwrap();
        pool.register(worker("w-a", "model-a", 1000)).unwrap();
        pool.register(worker("w-b", "model-a", 1000)).unwrap();
        pool.register(worker("w-d", "model-b", 1000)).unwrap();
        assert_eq!(pool.resolve("model-a", 0).unwrap().worker_id(), Some("w-a"));
        assert_eq!(pool.resolve("model-b", 0).unwrap().worker_id(), Some("w-d"));
        assert!(matches!(
            pool.resolve("model-z", 0),
            Err(PoolError::NoCapacity { .. })
        ));
    }

    #[test]
    fn mark_unreachable_is_idempotent_and_unknown_fails() {
        let mut pool = EndpointPool::new();
        pool.register(worker("w1", "m", 100)).unwrap();
        pool.mark_unreachable("w1").unwrap();
        pool.mark_unreachable("w1").unwrap();
        assert_eq!(pool.worker("w1").unwrap().state, WorkerState::Unreachable);
        assert!(matches!(
            pool.mark_unreachable("ghost"),
            Err(PoolError::UnknownWorker(_))
        ));
    }

    #[test]
    fn re_resolve_excludes_failed_worker_and_stays_on_model() {
        let mut pool = EndpointPool::new();
        pool.register(worker("w1", "m", 1000)).unwrap();
        pool.register(worker("w2", "m", 1000)).unwrap();
        pool.register(worker("w3", "m", 1000)).unwrap();
        pool.register(worker("w9", "other-model", 1000)).unwrap();
        let grant = pool.re_resolve("m", Some("w1"), 0).unwrap();
        assert_eq!(grant.worker_id(), Some("w2"));
        assert_eq!(pool.worker("w1").unwrap().state, WorkerState::Unreachable);
        assert_eq!(
            pool.re_resolve("m", None, 0).unwrap().worker_id(),
            Some("w2")
        );
        // w9 serves a different model and is never granted for "m".
        assert_ne!(grant.worker_id(), Some("w9"));
    }

    #[test]
    fn preempted_worker_does_not_kill_the_run() {
        let mut pool = EndpointPool::new();
        pool.register(worker("w1", "m", 1000)).unwrap();
        pool.register(worker("w2", "m", 1000)).unwrap();
        let mut driver = ReResolution::new("m", ReResolution::DEFAULT_MAX_ATTEMPTS).unwrap();
        let step = driver.start(&mut pool, 0).unwrap();
        assert_eq!(
            step,
            Step::Dispatch {
                grant: Grant::Direct {
                    worker_id: "w1".into(),
                    endpoint: "http://w1:8080".into(),
                    valid_until: 1000
                },
                attempt: 1
            }
        );
        // w1 is preempted mid-run: the connection dies.
        let step = driver
            .on_outcome(&mut pool, AttemptOutcome::ConnectionLost, 10)
            .unwrap();
        assert_eq!(
            step,
            Step::Dispatch {
                grant: Grant::Direct {
                    worker_id: "w2".into(),
                    endpoint: "http://w2:8080".into(),
                    valid_until: 1000
                },
                attempt: 2
            }
        );
        let step = driver
            .on_outcome(
                &mut pool,
                AttemptOutcome::Settled(StatusReason::Completed),
                20,
            )
            .unwrap();
        assert_eq!(
            step,
            Step::Halt {
                status: StatusReason::Completed
            }
        );
        assert_eq!(driver.attempts(), 2);
    }

    #[test]
    fn re_resolution_is_bounded_by_attempt_count() {
        let mut pool = EndpointPool::new();
        for id in ["w1", "w2", "w3"] {
            pool.register(worker(id, "m", 10_000)).unwrap();
        }
        let mut driver = ReResolution::new("m", 3).unwrap();
        let grant = pool.resolve("m", 0).unwrap();
        let step = driver.start(&mut pool, 0).unwrap();
        assert_eq!(step, Step::Dispatch { grant, attempt: 1 });
        let step = driver
            .on_outcome(&mut pool, AttemptOutcome::ConnectionLost, 1)
            .unwrap();
        assert!(matches!(step, Step::Dispatch { attempt: 2, .. }));
        let step = driver
            .on_outcome(&mut pool, AttemptOutcome::ConnectionLost, 2)
            .unwrap();
        assert!(matches!(step, Step::Dispatch { attempt: 3, .. }));
        let step = driver
            .on_outcome(&mut pool, AttemptOutcome::ConnectionLost, 3)
            .unwrap();
        assert_eq!(
            step,
            Step::Halt {
                status: StatusReason::ConnectionError
            }
        );
        assert_eq!(driver.attempts(), 3);
    }

    #[test]
    fn provider_failure_is_never_re_resolved() {
        let mut pool = EndpointPool::new();
        pool.register(worker("w1", "m", 1000)).unwrap();
        let mut driver = ReResolution::new("m", 3).unwrap();
        driver.start(&mut pool, 0).unwrap();
        let step = driver
            .on_outcome(
                &mut pool,
                AttemptOutcome::Settled(StatusReason::ProviderError),
                10,
            )
            .unwrap();
        assert_eq!(
            step,
            Step::Halt {
                status: StatusReason::ProviderError
            }
        );
        // The worker answered; it is not recorded as unreachable.
        assert_eq!(pool.worker("w1").unwrap().state, WorkerState::Healthy);
        assert_eq!(driver.attempts(), 1);
    }

    #[test]
    fn gateway_connection_loss_halts_without_spinning() {
        let mut pool = EndpointPool::new();
        pool.register(worker("w1", "m", 1000)).unwrap();
        pool.set_gateway("http://gw:8080").unwrap();
        let mut driver = ReResolution::new("m", 3).unwrap();
        let step = driver.start(&mut pool, 0).unwrap();
        assert!(matches!(step, Step::Dispatch { attempt: 1, .. }));
        let step = driver
            .on_outcome(&mut pool, AttemptOutcome::ConnectionLost, 10)
            .unwrap();
        assert_eq!(
            step,
            Step::Halt {
                status: StatusReason::ConnectionError
            }
        );
        assert_eq!(driver.attempts(), 1);
    }

    #[test]
    fn exhausted_capacity_halts_with_connection_error() {
        let mut pool = EndpointPool::new();
        pool.register(worker("w1", "m", 1000)).unwrap();
        let mut driver = ReResolution::new("m", 3).unwrap();
        driver.start(&mut pool, 0).unwrap();
        let step = driver
            .on_outcome(&mut pool, AttemptOutcome::ConnectionLost, 10)
            .unwrap();
        assert_eq!(
            step,
            Step::Halt {
                status: StatusReason::ConnectionError
            }
        );
        assert_eq!(pool.worker("w1").unwrap().state, WorkerState::Unreachable);
    }

    #[test]
    fn driver_rejects_zero_attempts_and_empty_model() {
        assert!(matches!(
            ReResolution::new("m", 0),
            Err(PoolError::Invalid(_))
        ));
        assert!(matches!(
            ReResolution::new("  ", 3),
            Err(PoolError::Invalid(_))
        ));
    }

    #[test]
    fn classify_records_connection_error_distinct_from_no_output() {
        // A lost endpoint with no output is a connection error, not
        // no-output.
        let lost = executor_result(
            ExecutionStatus::Failed,
            Some(FailureClass::ProviderUnavailable),
            "",
            None,
        );
        assert_eq!(classify_status(&lost), StatusReason::ConnectionError);
        let provider = executor_result(
            ExecutionStatus::Failed,
            Some(FailureClass::ProviderError),
            "boom",
            None,
        );
        assert_eq!(classify_status(&provider), StatusReason::ProviderError);
        let silent = executor_result(ExecutionStatus::Completed, None, "   ", None);
        assert_eq!(classify_status(&silent), StatusReason::NoOutput);
        let done = executor_result(ExecutionStatus::Completed, None, "done", None);
        assert_eq!(classify_status(&done), StatusReason::Completed);
        let patch_only = executor_result(ExecutionStatus::Completed, None, "", Some("diff"));
        assert_eq!(classify_status(&patch_only), StatusReason::Completed);
    }

    #[test]
    fn classify_timeout_splits_partial_from_no_output() {
        // A timeout that produced work is not a provider error and not
        // completed: the partial work is preserved under its own token.
        let partial = executor_result(
            ExecutionStatus::TimedOut,
            Some(FailureClass::Timeout),
            "half-done",
            None,
        );
        assert_eq!(classify_status(&partial), StatusReason::TimeoutPartial);
        let partial_patch = executor_result(
            ExecutionStatus::TimedOut,
            Some(FailureClass::Timeout),
            "",
            Some("diff"),
        );
        assert_eq!(
            classify_status(&partial_patch),
            StatusReason::TimeoutPartial
        );
        // A timeout that spent its budget without output is no-output.
        let timed_out = executor_result(
            ExecutionStatus::TimedOut,
            Some(FailureClass::Timeout),
            "   ",
            None,
        );
        assert_eq!(classify_status(&timed_out), StatusReason::NoOutput);
    }

    #[test]
    fn status_tokens_are_stable_and_parse_fails_closed() {
        for reason in [
            StatusReason::Completed,
            StatusReason::ConnectionError,
            StatusReason::NoOutput,
            StatusReason::NoOutputQueued,
            StatusReason::NoOutputIdle,
            StatusReason::TimeoutPartial,
            StatusReason::ProviderError,
        ] {
            assert_eq!(StatusReason::parse(reason.as_str()).unwrap(), reason);
        }
        assert!(StatusReason::parse("NO-OUTPUT").is_err());
        assert!(StatusReason::parse("bogus").is_err());
        assert!(StatusReason::parse("").is_err());
    }

    #[test]
    fn budget_is_derived_from_the_allocation() {
        let direct = Grant::Direct {
            worker_id: "w1".into(),
            endpoint: "http://w1".into(),
            valid_until: 10_000,
        };
        // Derived: remaining minus the flush margin.
        assert_eq!(plan_agent_budget(&direct, 7_300, None, 300).unwrap(), 2_400);
        // An explicit budget that fits inside the derived budget is
        // used as-is.
        assert_eq!(
            plan_agent_budget(&direct, 7_300, Some(1_000), 300).unwrap(),
            1_000
        );
    }

    #[test]
    fn explicit_budget_outrunning_the_allocation_fails_closed() {
        let direct = Grant::Direct {
            worker_id: "w1".into(),
            endpoint: "http://w1".into(),
            valid_until: 10_000,
        };
        // 45 m explicit on a job with 25 m left: the error names both
        // numbers.
        let err = plan_agent_budget(&direct, 8_500, Some(2_700), 300).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("2700s"), "{msg}");
        assert!(msg.contains("1500s remaining"), "{msg}");
        // Eating the flush margin also fails: the margin is the window
        // partial output gets flushed before the scheduler kills the job.
        assert!(matches!(
            plan_agent_budget(&direct, 8_500, Some(1_300), 300),
            Err(PoolError::Invalid(_))
        ));
    }

    #[test]
    fn budget_planning_fails_closed_on_dead_allocations_and_gateways() {
        let dead = Grant::Direct {
            worker_id: "w1".into(),
            endpoint: "http://w1".into(),
            valid_until: 1_000,
        };
        assert!(matches!(
            plan_agent_budget(&dead, 1_000, None, 300),
            Err(PoolError::Invalid(_))
        ));
        // A margin consuming the whole remainder leaves no budget.
        let tight = Grant::Direct {
            worker_id: "w1".into(),
            endpoint: "http://w1".into(),
            valid_until: 1_300,
        };
        assert!(matches!(
            plan_agent_budget(&tight, 1_000, None, 300),
            Err(PoolError::Invalid(_))
        ));
        // Gateway grants carry no horizon: derivation is impossible, so
        // the plan fails closed whether or not a budget was given.
        let gateway = Grant::Gateway {
            url: "http://gw".into(),
        };
        assert!(matches!(
            plan_agent_budget(&gateway, 1_000, None, 300),
            Err(PoolError::Invalid(_))
        ));
        assert!(matches!(
            plan_agent_budget(&gateway, 1_000, Some(600), 300),
            Err(PoolError::Invalid(_))
        ));
    }

    #[test]
    fn startup_line_names_both_limits() {
        assert_eq!(
            startup_limits_line(4 * 3600, 45 * 60),
            "walltime=4h agent_limit=45m"
        );
        assert_eq!(startup_limits_line(90, 45), "walltime=90s agent_limit=45s");
        assert_eq!(
            startup_limits_line(3_967, 95),
            "walltime=3967s agent_limit=95s"
        );
    }

    #[test]
    fn pool_error_display_covers_all_variants() {
        assert_eq!(
            PoolError::NoCapacity { model: "m".into() }.to_string(),
            "no healthy capacity for model m"
        );
        assert_eq!(
            PoolError::Saturated { model: "m".into() }.to_string(),
            "model m is saturated: every slot is in use"
        );
        assert_eq!(
            PoolError::UnknownWorker("w1".into()).to_string(),
            "unknown worker: w1"
        );
        assert_eq!(PoolError::Invalid("bad".into()).to_string(), "invalid: bad");
    }

    #[test]
    fn acquire_release_tracks_slots_and_fails_closed() {
        let mut pool = EndpointPool::new();
        pool.register(worker_slots("w1", "m", 100, 2)).unwrap();
        assert_eq!(pool.free_slots("w1"), Some(2));
        pool.acquire("w1").unwrap();
        assert_eq!(pool.free_slots("w1"), Some(1));
        pool.release("w1").unwrap();
        assert_eq!(pool.free_slots("w1"), Some(2));
        assert_eq!(pool.free_slots("ghost"), None);
        assert!(matches!(
            pool.acquire("ghost"),
            Err(PoolError::UnknownWorker(_))
        ));
        // Releasing a slot the worker does not hold is driver-state
        // corruption: it fails closed.
        assert!(matches!(pool.release("w1"), Err(PoolError::Invalid(_))));
    }

    #[test]
    fn admission_rejects_dispatch_when_no_slot_is_free() {
        // Issue #3758: a run is never dispatched to a worker with zero
        // free slots.
        let mut pool = EndpointPool::new();
        pool.register(worker_slots("w1", "m", 100, 1)).unwrap();
        pool.register(worker_slots("w2", "m", 100, 1)).unwrap();
        pool.acquire("w1").unwrap();
        pool.acquire("w2").unwrap();
        // Saturated is distinct from NoCapacity: the fleet is healthy
        // but full.
        assert!(matches!(
            pool.resolve("m", 0),
            Err(PoolError::Saturated { .. })
        ));
        assert_eq!(pool.healthy_capacity("m", 0), 2);
        pool.release("w1").unwrap();
        assert_eq!(pool.resolve("m", 0).unwrap().worker_id(), Some("w1"));
    }

    #[test]
    fn resolve_skips_a_fully_occupied_worker() {
        let mut pool = EndpointPool::new();
        pool.register(worker_slots("w1", "m", 100, 1)).unwrap();
        pool.register(worker_slots("w2", "m", 100, 4)).unwrap();
        pool.acquire("w1").unwrap();
        // w1 has no free slot even though its id sorts first: the grant
        // goes to w2.
        assert_eq!(pool.resolve("m", 0).unwrap().worker_id(), Some("w2"));
        assert_eq!(pool.free_slot_capacity("m", 0), 4);
    }

    #[test]
    fn gateway_resolution_fails_closed_when_the_fleet_is_saturated() {
        let mut pool = EndpointPool::new();
        pool.register(worker_slots("w1", "m", 100, 4)).unwrap();
        pool.register(worker_slots("w2", "m", 100, 2)).unwrap();
        pool.set_gateway("http://gw:8080").unwrap();
        for _ in 0..4 {
            pool.acquire("w1").unwrap();
        }
        for _ in 0..2 {
            pool.acquire("w2").unwrap();
        }
        assert_eq!(pool.free_slot_capacity("m", 0), 0);
        assert!(matches!(
            pool.resolve("m", 0),
            Err(PoolError::Saturated { .. })
        ));
    }

    #[test]
    fn queue_state_records_occupancy_including_full_workers() {
        let mut pool = EndpointPool::new();
        pool.register(worker_slots("w-b", "m", 100, 4)).unwrap();
        pool.register(worker_slots("w-a", "m", 100, 16)).unwrap();
        pool.acquire("w-a").unwrap();
        for _ in 0..4 {
            pool.acquire("w-b").unwrap();
        }
        let state = pool.queue_state("m", 0);
        assert_eq!(state.model, "m");
        // Snapshot order is worker-id order, and the fully occupied
        // worker appears: saturation is the interesting case.
        assert_eq!(
            state.workers,
            vec![
                WorkerOccupancy {
                    worker_id: "w-a".into(),
                    in_use: 1,
                    total_slots: 16,
                },
                WorkerOccupancy {
                    worker_id: "w-b".into(),
                    in_use: 4,
                    total_slots: 4,
                }
            ]
        );
        assert_eq!(state.free_slots, 15);
        // A snapshot is a pure read: taking it twice yields the same
        // state.
        assert_eq!(pool.queue_state("m", 0), state);
        // Model-scoped: a snapshot for another model sees nothing.
        let other = pool.queue_state("other", 0);
        assert_eq!(other.workers, Vec::<WorkerOccupancy>::new());
        assert_eq!(other.free_slots, 0);
    }

    #[test]
    fn mark_unreachable_voids_the_worker_slots() {
        let mut pool = EndpointPool::new();
        pool.register(worker_slots("w1", "m", 100, 4)).unwrap();
        pool.acquire("w1").unwrap();
        pool.acquire("w1").unwrap();
        pool.mark_unreachable("w1").unwrap();
        // The allocations are gone with the worker: the reservations are
        // voided, not leaked.
        assert_eq!(pool.free_slots("w1"), Some(4));
        assert_eq!(pool.free_slot_capacity("m", 0), 0);
    }

    #[test]
    fn watchdog_kills_distinguish_queued_from_idle() {
        // Issue #3758: a stall kill is classified by whether the run
        // held a slot, not by the fleet's free-slot count.
        let mut pool = EndpointPool::new();
        pool.register(worker_slots("w1", "m", 100, 1)).unwrap();
        pool.acquire("w1").unwrap();
        // Saturated: free_slots is 0. The queued run (no slot) is
        // queued; the run holding the last slot is idle.
        let queued = WatchdogTermination {
            held_slot: false,
            queue_state: pool.queue_state("m", 0),
        };
        assert_eq!(queued.status(), StatusReason::NoOutputQueued);
        let idle = WatchdogTermination {
            held_slot: true,
            queue_state: pool.queue_state("m", 0),
        };
        assert_eq!(idle.status(), StatusReason::NoOutputIdle);
        assert_eq!(
            classify_watchdog_termination(&idle),
            StatusReason::NoOutputIdle
        );
        assert_ne!(idle.status(), queued.status());
    }

    #[test]
    fn watchdog_status_line_records_the_queue_state() {
        let mut pool = EndpointPool::new();
        pool.register(worker_slots("w1", "m", 100, 16)).unwrap();
        pool.register(worker_slots("w2", "m", 100, 4)).unwrap();
        for _ in 0..16 {
            pool.acquire("w1").unwrap();
        }
        for _ in 0..3 {
            pool.acquire("w2").unwrap();
        }
        let term = WatchdogTermination {
            held_slot: false,
            queue_state: pool.queue_state("m", 0),
        };
        assert_eq!(
            term.status_line(),
            "status=no-output-queued held_slot=false free_slots=1 workers=w1:16/16,w2:3/4"
        );
        let idle = WatchdogTermination {
            held_slot: true,
            queue_state: pool.queue_state("m", 0),
        };
        assert_eq!(
            idle.status_line(),
            "status=no-output-idle held_slot=true free_slots=1 workers=w1:16/16,w2:3/4"
        );
        // An empty snapshot renders without inventing workers.
        let empty = WatchdogTermination {
            held_slot: false,
            queue_state: pool.queue_state("other", 0),
        };
        assert_eq!(
            empty.status_line(),
            "status=no-output-queued held_slot=false free_slots=0 workers=-"
        );
    }

    #[test]
    fn driver_acquires_a_slot_on_start_and_releases_it_when_settled() {
        let mut pool = EndpointPool::new();
        pool.register(worker_slots("w1", "m", 1000, 2)).unwrap();
        let mut driver = ReResolution::new("m", 3).unwrap();
        driver.start(&mut pool, 0).unwrap();
        // The dispatched run holds a slot: a second run can still be
        // admitted but not a third.
        assert_eq!(pool.free_slots("w1"), Some(1));
        pool.acquire("w1").unwrap();
        assert!(matches!(
            pool.resolve("m", 0),
            Err(PoolError::Saturated { .. })
        ));
        driver
            .on_outcome(
                &mut pool,
                AttemptOutcome::Settled(StatusReason::Completed),
                10,
            )
            .unwrap();
        // The settled run gave its slot back.
        assert_eq!(pool.free_slots("w1"), Some(1));
        assert_eq!(pool.free_slot_capacity("m", 0), 1);
    }

    #[test]
    fn redrive_into_a_saturated_fleet_halts_without_spinning() {
        let mut pool = EndpointPool::new();
        pool.register(worker_slots("w1", "m", 1000, 1)).unwrap();
        pool.register(worker_slots("w2", "m", 1000, 1)).unwrap();
        let mut driver = ReResolution::new("m", 3).unwrap();
        driver.start(&mut pool, 0).unwrap(); // w1 holds its only slot.
        pool.acquire("w2").unwrap(); // w2 is occupied by another run.
                                     // w1 dies; the only healthy worker is full.
        let step = driver
            .on_outcome(&mut pool, AttemptOutcome::ConnectionLost, 10)
            .unwrap();
        assert_eq!(
            step,
            Step::Halt {
                status: StatusReason::ConnectionError
            }
        );
        // w1 is recorded unreachable; w2 keeps its run.
        assert_eq!(pool.worker("w1").unwrap().state, WorkerState::Unreachable);
        assert_eq!(pool.worker("w2").unwrap().state, WorkerState::Healthy);
        assert_eq!(pool.free_slots("w2"), Some(0));
        assert_eq!(driver.attempts(), 1);
    }
}
