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
//! **Statuses are recorded distinctly.** [`classify_status`] maps a finished
//! dispatch to a [`StatusReason`] for `status.txt`, where `connection-error`
//! and `no-output` are separate stable tokens: a run that lost its endpoint
//! is not a run that produced no output.
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
/// grant and never resolves to a worker it does not know.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PoolError {
    /// No healthy worker serving `model` with a live guarantee at resolve
    /// time; dispatch must not proceed on a grant that would expire
    /// immediately.
    NoCapacity { model: String },
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
        if let Some(url) = &self.gateway {
            return Ok(Grant::Gateway { url: url.clone() });
        }
        let worker = self
            .workers
            .values()
            .filter(|w| {
                w.state == WorkerState::Healthy && w.model == model && w.guaranteed_until > now
            })
            .min_by(|a, b| a.worker_id.cmp(&b.worker_id))
            .expect("capacity was checked above");
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
    /// patch.
    NoOutput,
    /// The dispatch failed at or beyond a live endpoint: provider error,
    /// timeout, or unclassified failure.
    ProviderError,
}

impl StatusReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::ConnectionError => "connection-error",
            Self::NoOutput => "no-output",
            Self::ProviderError => "provider-error",
        }
    }

    /// Fail closed: unknown tokens are an error, never a guess.
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "completed" => Ok(Self::Completed),
            "connection-error" => Ok(Self::ConnectionError),
            "no-output" => Ok(Self::NoOutput),
            "provider-error" => Ok(Self::ProviderError),
            other => Err(format!("unknown status reason: {other}")),
        }
    }
}

/// Classify a finished dispatch for `status.txt`.
///
/// A connection-level failure takes precedence over everything else: once
/// the endpoint is lost, the (possibly empty) output says nothing about the
/// run. An unclassified dispatch that produced neither output nor a patch is
/// `no-output`, never `completed`.
pub fn classify_status(result: &ExecutorResult) -> StatusReason {
    match result.failure_class {
        Some(FailureClass::ProviderUnavailable) => StatusReason::ConnectionError,
        Some(_) => StatusReason::ProviderError,
        None if result.output.trim().is_empty()
            && result
                .patch
                .as_deref()
                .map_or(true, |patch| patch.trim().is_empty()) =>
        {
            StatusReason::NoOutput
        }
        None => StatusReason::Completed,
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
    pub fn start(&mut self, pool: &EndpointPool, now: u64) -> Result<Step, PoolError> {
        let grant = pool.resolve(&self.model, now)?;
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
    pub fn on_outcome(
        &mut self,
        pool: &mut EndpointPool,
        outcome: AttemptOutcome,
        now: u64,
    ) -> Result<Step, PoolError> {
        if let AttemptOutcome::Settled(status) = outcome {
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
                self.attempts += 1;
                self.last_worker = grant.worker_id().map(str::to_string);
                Ok(Step::Dispatch {
                    grant,
                    attempt: self.attempts,
                })
            }
            // The run stopped because the endpoint was lost and no
            // replacement exists; record it as such.
            Err(PoolError::NoCapacity { .. }) => Ok(Step::Halt {
                status: StatusReason::ConnectionError,
            }),
            Err(err) => Err(err),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn worker(id: &str, model: &str, until: u64) -> Worker {
        Worker {
            worker_id: id.into(),
            endpoint: format!("http://{id}:8080"),
            model: model.into(),
            state: WorkerState::Healthy,
            guaranteed_until: until,
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
        let step = driver.start(&pool, 0).unwrap();
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
        let step = driver.start(&pool, 0).unwrap();
        assert_eq!(
            step,
            Step::Dispatch {
                grant: pool.resolve("m", 0).unwrap(),
                attempt: 1
            }
        );
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
        driver.start(&pool, 0).unwrap();
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
        let step = driver.start(&pool, 0).unwrap();
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
        driver.start(&pool, 0).unwrap();
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
    fn classify_timeout_is_not_a_connection_error() {
        let timed_out = executor_result(
            ExecutionStatus::TimedOut,
            Some(FailureClass::Timeout),
            "",
            None,
        );
        assert_eq!(classify_status(&timed_out), StatusReason::ProviderError);
    }

    #[test]
    fn status_tokens_are_stable_and_parse_fails_closed() {
        for reason in [
            StatusReason::Completed,
            StatusReason::ConnectionError,
            StatusReason::NoOutput,
            StatusReason::ProviderError,
        ] {
            assert_eq!(StatusReason::parse(reason.as_str()).unwrap(), reason);
        }
        assert!(StatusReason::parse("NO-OUTPUT").is_err());
        assert!(StatusReason::parse("bogus").is_err());
        assert!(StatusReason::parse("").is_err());
    }

    #[test]
    fn pool_error_display_covers_all_variants() {
        assert_eq!(
            PoolError::NoCapacity { model: "m".into() }.to_string(),
            "no healthy capacity for model m"
        );
        assert_eq!(
            PoolError::UnknownWorker("w1".into()).to_string(),
            "unknown worker: w1"
        );
        assert_eq!(PoolError::Invalid("bad".into()).to_string(), "invalid: bad");
    }
}
