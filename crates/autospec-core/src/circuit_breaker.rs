//! Per-worker outcome state and a circuit breaker for the gateway
//! (issue #4378).
//!
//! A worker deadlocked: `/health` kept returning 200, generation stopped,
//! GPUs sat at 0%. The gateway routed to it for hours. It was not short of
//! information — it logged the fault 237 times — it was short of somewhere
//! to put it.
//!
//! Every one of those signals is a durable fact about a worker, and each
//! invariants from the issue maps to a primitive here:
//!
//! 1. **Liveness is the last successful unit of work, never a health
//!    endpoint** ([`WorkerOutcome::last_success`], [`WorkerOutcome::is_live`]).
//!    There is deliberately no method that records a health response: a
//!    check that does not exercise the real path cannot make this state.
//! 2. **A detector must write to state a decision can read**
//!    ([`WorkerOutcome::probe_timeout`]). A probe timeout is recorded as a
//!    failure against the same counters the routing decision reads — a
//!    signal that is worth emitting is worth counting.
//! 3. **Distinguish "slow" from "stopped" by progress, not elapsed time**
//!    ([`WorkerOutcome::stuck`]). A token emitted since the last check
//!    resets the stuck timer, so a legitimately long request that keeps
//!    producing is never punished by wall-clock alone. The bound belongs
//!    next to the "busy is not dead" rule: [`BreakerConfig::stuck_timeout`].
//!    This is the one progress judgement in the module: the operational
//!    liveness scan ([`WorkerOutcome::liveness`], [`Fleet::liveness_report`])
//!    consumes it rather than reimplementing a probe-timeout test
//!    (issue #4459).
//! 4. **A routing decision must be able to exclude a peer, and say that it
//!    did** ([`WorkerOutcome::routing_decision`],
//!    [`Fleet::routing_report`]). The open circuit is reported with its age
//!    and its reopen time, never a silently dropped worker.
//! 5. **Keep per-peer outcome history, not just aggregate counters**
//!    ([`WorkerOutcome::recent_outcomes`], [`Fleet::summary_lines`]). The
//!    fleet looked healthy in aggregate the entire time one model was at 0%
//!    success, so the report is per worker, one line each.
//!
//! The breaker itself: **closed** routes normally; it **opens** after
//! [`BreakerConfig::failure_threshold`] consecutive failures (or
//! [`BreakerConfig::peer_failure_threshold`] when a same-model peer
//! succeeded recently — a worker failing while its siblings answer is a
//! stronger signal than any absolute threshold) or after
//! [`BreakerConfig::stuck_timeout`] with in-flight requests and no
//! progress. While **open** the worker is excluded from routing but stays
//! registered; after the back-off it goes **half-open** and admits exactly
//! one probe request: success closes it, failure re-opens it with a longer
//! back-off. Every state change is a [`Transition`] with a timestamp, so
//! "how long has this been bad" is answerable, and [`WorkerOutcome::snapshot`]
//! / [`WorkerOutcome::restore`] keep the picture across a restart instead
//! of resetting it to optimistic.
//!
//! Everything here is pure: no I/O, no clock, no subprocess. The caller
//! observes the worker (completions, failures, tokens) and passes the
//! monotonic now; the module keeps the state and decides.

use std::collections::{BTreeMap, VecDeque};
use std::time::Duration;

/// The outcome of the scan's own probe of a worker (the `/props` request a
/// health scan sends).
///
/// The probe is characterisation, not judgement: the liveness verdict
/// below is carried by the progress sample — decode advancing across the
/// stuck window — whenever one exists, and the probe only speaks for a
/// worker the progress measure has already suspected, i.e. one with no
/// in-flight work to sample (issue #4459).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeOutcome {
    /// The endpoint answered the probe.
    Answered,
    /// The probe timed out. A timeout alone reports *unknown*, never *bad*:
    /// under load a probe queues behind real work, so the busiest workers
    /// fail it first.
    TimedOut,
}

/// The three-valued liveness verdict for one worker (issue #4459).
///
/// A liveness check distinguishes "slow" from "dead" by construction, not
/// by timeout: three states, because acting on them differs — a busy worker
/// (queue depth high, decode advancing) is left alone, a wedged one (slots
/// saturated, decode flat) is restarted, and an unknown one is re-sampled
/// before any action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LivenessVerdict {
    /// Decode is advancing (a busy worker under load) or the worker is
    /// idle and answering. Route to it; a probe timeout here is ignored —
    /// progress is the only exoneration, and this worker has progress.
    Healthy,
    /// Slots are saturated and decode is flat for the stuck window: the
    /// progress measure — [`WorkerOutcome::stuck`], the gateway's own stuck
    /// detector — says this worker is wedged.
    Wedged,
    /// No progress sample exists to exonerate or condemn (no in-flight
    /// work to sample) and the probe timed out — or was never sent. A
    /// timeout alone is unknown, never a verdict of "not answering"; it is
    /// resolved by a progress sample before any action.
    Unknown,
}

impl LivenessVerdict {
    /// The verdict as reported: `healthy`, `wedged`, `unknown`.
    pub fn label(&self) -> &'static str {
        match self {
            LivenessVerdict::Healthy => "healthy",
            LivenessVerdict::Wedged => "wedged",
            LivenessVerdict::Unknown => "unknown",
        }
    }
}

/// The outcome of a real request to a worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The request completed and produced its result.
    Completed,
    /// The request failed or timed out.
    Failed,
}

/// The breaker state for one worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    /// Routing normally.
    Closed,
    /// Excluded from routing; the back-off runs, then half-open.
    Open,
    /// Admits exactly one probe request: success closes, failure re-opens.
    HalfOpen,
}

/// Why the breaker moved to open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenReason {
    /// The absolute (or peer-adjusted) consecutive-failure threshold hit.
    ConsecutiveFailures { count: u64 },
    /// In-flight requests with no progress for the stuck timeout.
    Stuck {
        no_progress_for: Duration,
        in_flight: u64,
    },
    /// The half-open probe failed.
    ProbeFailed,
}

/// A state transition, with a timestamp, so the history of a worker's
/// breaker is durable state, not a log line.
#[derive(Debug, Clone, PartialEq)]
pub struct Transition {
    pub worker: String,
    pub from: CircuitState,
    pub to: CircuitState,
    pub at: Duration,
    pub reason: Option<OpenReason>,
}

impl Transition {
    /// The one-line report of this transition.
    pub fn line(&self) -> String {
        match self.reason {
            Some(reason) => format!(
                "{}: {} -> {} at {} ({})",
                self.worker,
                self.from.label(),
                self.to.label(),
                fmt_dur(self.at),
                reason.label()
            ),
            None => format!(
                "{}: {} -> {} at {}",
                self.worker,
                self.from.label(),
                self.to.label(),
                fmt_dur(self.at)
            ),
        }
    }
}

impl CircuitState {
    fn label(&self) -> &'static str {
        match self {
            CircuitState::Closed => "closed",
            CircuitState::Open => "open",
            CircuitState::HalfOpen => "half-open",
        }
    }
}

impl OpenReason {
    fn label(&self) -> String {
        match self {
            OpenReason::ConsecutiveFailures { count } => {
                format!("{count} consecutive failures")
            }
            OpenReason::Stuck {
                no_progress_for,
                in_flight,
            } => format!(
                "stuck: {in_flight} in-flight, no progress for {}",
                fmt_dur(*no_progress_for)
            ),
            OpenReason::ProbeFailed => "half-open probe failed".to_string(),
        }
    }
}

/// The breaker's thresholds and back-off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BreakerConfig {
    /// Consecutive failures that open the circuit on its own.
    pub failure_threshold: u64,
    /// Consecutive failures that open the circuit when a same-model peer
    /// succeeded recently — a far stronger signal, so a smaller threshold.
    pub peer_failure_threshold: u64,
    /// With in-flight requests and no progress this long, the worker is
    /// stuck, not busy. The bound on "busy is not dead".
    pub stuck_timeout: Duration,
    /// Back-off of the first opening; doubles on each probe failure.
    pub backoff_initial: Duration,
    /// Cap on the doubled back-off.
    pub backoff_max: Duration,
    /// How many recent per-worker outcomes to keep.
    pub history_len: usize,
    /// A peer's successful completion within this window counts as
    /// "the siblings are answering".
    pub peer_window: Duration,
}

impl Default for BreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            peer_failure_threshold: 2,
            stuck_timeout: Duration::from_secs(120),
            backoff_initial: Duration::from_secs(30),
            backoff_max: Duration::from_secs(600),
            history_len: 16,
            peer_window: Duration::from_secs(60),
        }
    }
}

/// Capability declared by the worker at registration time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerCapability {
    pub slots: u32,
    pub context_window: u64,
    pub quantisation: String,
}

impl WorkerCapability {
    /// Can this worker serve a request of this many tokens at all?
    pub fn fits(&self, requested_tokens: u64) -> bool {
        self.context_window >= requested_tokens
    }
}

/// The gateway's per-worker outcome state and breaker.
#[derive(Debug, Clone)]
pub struct WorkerOutcome {
    pub worker: String,
    pub model: String,
    pub capability: WorkerCapability,
    cfg: BreakerConfig,
    last_success: Option<Duration>,
    consecutive_failures: u64,
    total_completed: u64,
    total_failed: u64,
    recent: VecDeque<Outcome>,
    in_flight: u64,
    in_flight_since: Option<Duration>,
    last_token: Option<Duration>,
    state: CircuitState,
    state_since: Duration,
    backoff: Duration,
    probe_active: bool,
    transitions: Vec<Transition>,
}

impl WorkerOutcome {
    /// A freshly registered worker starts closed — that is the only place
    /// the picture may be optimistic.
    pub fn new(
        worker: impl Into<String>,
        model: impl Into<String>,
        capability: WorkerCapability,
        cfg: BreakerConfig,
        now: Duration,
    ) -> Self {
        let backoff = cfg.backoff_initial;
        Self {
            worker: worker.into(),
            model: model.into(),
            capability,
            cfg,
            last_success: None,
            consecutive_failures: 0,
            total_completed: 0,
            total_failed: 0,
            recent: VecDeque::new(),
            in_flight: 0,
            in_flight_since: None,
            last_token: None,
            state: CircuitState::Closed,
            state_since: now,
            backoff,
            probe_active: false,
            transitions: Vec::new(),
        }
    }

    /// A worker restored after a restart: the picture is taken from
    /// [`WorkerOutcome::snapshot`] of the previous incarnation, not reset
    /// to closed. "How long has this been bad" survives the restart.
    pub fn restore(
        worker: impl Into<String>,
        model: impl Into<String>,
        capability: WorkerCapability,
        cfg: BreakerConfig,
        snap: RestoreState,
    ) -> Self {
        Self {
            worker: worker.into(),
            model: model.into(),
            capability,
            cfg,
            last_success: snap.last_success,
            consecutive_failures: snap.consecutive_failures,
            total_completed: snap.total_completed,
            total_failed: snap.total_failed,
            recent: snap.recent,
            in_flight: snap.in_flight,
            in_flight_since: snap.in_flight_since,
            last_token: snap.last_token,
            state: snap.state,
            state_since: snap.state_since,
            backoff: snap.backoff,
            probe_active: false,
            transitions: snap.transitions,
        }
    }

    /// The state to persist so a restart does not reset the picture.
    pub fn snapshot(&self) -> RestoreState {
        RestoreState {
            state: self.state,
            state_since: self.state_since,
            backoff: self.backoff,
            consecutive_failures: self.consecutive_failures,
            last_success: self.last_success,
            in_flight: self.in_flight,
            in_flight_since: self.in_flight_since,
            last_token: self.last_token,
            recent: self.recent.clone(),
            total_completed: self.total_completed,
            total_failed: self.total_failed,
            transitions: self.transitions.clone(),
        }
    }

    // --- events: the detector writes to state a decision can read ---

    /// A request started against this worker (the gateway proxies the
    /// stream, so it is the component that can see in-flight work).
    pub fn request_started(&mut self, now: Duration) {
        if self.in_flight == 0 {
            self.in_flight_since = Some(now);
        }
        self.in_flight += 1;
    }

    /// A token was emitted for an in-flight request. Progress: it resets
    /// the stuck timer, whatever the request's total age.
    pub fn token(&mut self, now: Duration) {
        self.last_token = Some(now);
    }

    /// A real request finished. A completion is the only liveness signal
    /// this state accepts.
    pub fn request_finished(&mut self, now: Duration, outcome: Outcome) {
        self.in_flight = self.in_flight.saturating_sub(1);
        if self.in_flight == 0 {
            self.in_flight_since = None;
        }
        self.recent.push_back(outcome);
        while self.recent.len() > self.cfg.history_len {
            self.recent.pop_front();
        }
        match outcome {
            Outcome::Completed => {
                self.last_success = Some(now);
                self.consecutive_failures = 0;
                self.total_completed += 1;
                if self.state == CircuitState::HalfOpen && self.probe_active {
                    self.probe_active = false;
                    self.backoff = self.cfg.backoff_initial;
                    self.transition(now, CircuitState::Closed, None);
                }
            }
            Outcome::Failed => {
                self.consecutive_failures += 1;
                self.total_failed += 1;
                if self.state == CircuitState::HalfOpen && self.probe_active {
                    // The probe failed: re-open with a longer back-off.
                    self.probe_active = false;
                    self.backoff = (self.backoff * 2).min(self.cfg.backoff_max);
                    self.transition(now, CircuitState::Open, Some(OpenReason::ProbeFailed));
                }
            }
        }
    }

    /// A probe (the gateway's own timed-out poll of the worker) failed.
    ///
    /// Invariant 2: the detector writes to state a decision can read. A
    /// probe timeout is a failure on the same counters the routing
    /// decision reads — 237 of them, in the incident.
    pub fn probe_timeout(&mut self, now: Duration) {
        self.consecutive_failures += 1;
        self.total_failed += 1;
        self.recent.push_back(Outcome::Failed);
        while self.recent.len() > self.cfg.history_len {
            self.recent.pop_front();
        }
        let _ = now; // the counter, not the timestamp, is what the decision reads
    }

    /// The half-open probe goes out: exactly one request, no more while it
    /// is in flight.
    pub fn send_probe(&mut self, now: Duration) -> bool {
        if self.state != CircuitState::HalfOpen || self.probe_active {
            return false;
        }
        self.probe_active = true;
        self.request_started(now);
        true
    }

    // --- decisions ---

    /// The routing decision for this worker right now.
    pub fn routing_decision(&self) -> RoutingDecision {
        match self.state {
            CircuitState::Closed => RoutingDecision::Route,
            CircuitState::Open => RoutingDecision::Excluded(Exclusion::CircuitOpen {
                since: self.state_since,
                reopens_at: self.state_since.saturating_add(self.backoff),
            }),
            CircuitState::HalfOpen => {
                if self.probe_active {
                    RoutingDecision::Excluded(Exclusion::ProbeInFlight)
                } else {
                    RoutingDecision::Probe
                }
            }
        }
    }

    /// The decision step: opens a closed circuit on its conditions and
    /// moves an expired open circuit to half-open. Returns the transitions
    /// emitted — the caller reports them; they are also kept in
    /// [`WorkerOutcome::transitions`].
    pub fn tick(&mut self, now: Duration, peers_healthy: bool) -> Vec<Transition> {
        let mut emitted = Vec::new();
        match self.state {
            CircuitState::Closed => {
                let threshold = if peers_healthy {
                    self.cfg.peer_failure_threshold
                } else {
                    self.cfg.failure_threshold
                };
                let reason = if self.consecutive_failures >= threshold {
                    Some(OpenReason::ConsecutiveFailures {
                        count: self.consecutive_failures,
                    })
                } else if let Some(no_progress_for) = self.stuck(now) {
                    Some(OpenReason::Stuck {
                        no_progress_for,
                        in_flight: self.in_flight,
                    })
                } else {
                    None
                };
                if let Some(reason) = reason {
                    emitted.push(self.transition(now, CircuitState::Open, Some(reason)));
                }
            }
            CircuitState::Open => {
                if now >= self.state_since.saturating_add(self.backoff) {
                    emitted.push(self.transition(now, CircuitState::HalfOpen, None));
                }
            }
            CircuitState::HalfOpen => {}
        }
        emitted
    }

    // --- the durable facts ---

    /// The only true liveness signal: the last successful completion of
    /// real work. A `/health` 200 never sets this.
    pub fn last_success(&self) -> Option<Duration> {
        self.last_success
    }

    /// Live means a real completion within `window`.
    pub fn is_live(&self, now: Duration, window: Duration) -> bool {
        self.last_success
            .is_some_and(|at| now.saturating_sub(at) <= window)
    }

    pub fn state(&self) -> CircuitState {
        self.state
    }

    pub fn consecutive_failures(&self) -> u64 {
        self.consecutive_failures
    }

    pub fn in_flight(&self) -> u64 {
        self.in_flight
    }

    /// Seconds-since the last token, while in-flight work exists.
    pub fn no_token_for(&self, now: Duration) -> Option<Duration> {
        if self.in_flight == 0 {
            return None;
        }
        let reference = self
            .last_token
            .unwrap_or(self.in_flight_since.unwrap_or(now));
        Some(now.saturating_sub(reference))
    }

    /// "Busy is not dead" needs a bound: in-flight work with no progress
    /// (no token) for at least `stuck_timeout` is stuck. Returns how long
    /// the worker has had no progress. Progress — not wall-clock — is the
    /// discriminator: a long request emitting tokens is not stuck.
    pub fn stuck(&self, now: Duration) -> Option<Duration> {
        let no_progress_for = self.no_token_for(now)?;
        (no_progress_for >= self.cfg.stuck_timeout).then_some(no_progress_for)
    }

    /// How long the current state has held: "how long has this been bad".
    pub fn state_age(&self, now: Duration) -> Duration {
        now.saturating_sub(self.state_since)
    }

    /// The liveness verdict for this worker (issue #4459).
    ///
    /// One implementation, shared with the operational scan: with in-flight
    /// work the progress sample *is* the judgement — decode advancing means
    /// [`LivenessVerdict::Healthy`], decode flat for
    /// [`BreakerConfig::stuck_timeout`] means [`LivenessVerdict::Wedged`] —
    /// exactly [`WorkerOutcome::stuck`], the gateway's own stuck detector,
    /// so a fix to the invariant lands in one place. The probe is ignored
    /// here: a probe that times out behind real traffic is a property of
    /// the probe, not of the worker. With no in-flight work there is no
    /// progress sample: an answering probe is healthy, a timed-out (or
    /// missing) probe is [`LivenessVerdict::Unknown`], never a verdict of
    /// "not answering".
    pub fn liveness(&self, probe: Option<ProbeOutcome>, now: Duration) -> LivenessVerdict {
        if self.in_flight > 0 {
            return match self.stuck(now) {
                Some(_) => LivenessVerdict::Wedged,
                None => LivenessVerdict::Healthy,
            };
        }
        match probe {
            Some(ProbeOutcome::Answered) => LivenessVerdict::Healthy,
            Some(ProbeOutcome::TimedOut) | None => LivenessVerdict::Unknown,
        }
    }

    /// The per-worker liveness report line: the verdict first, then the
    /// evidence that carried it — busy (in-flight, decode advancing),
    /// wedged (saturated, decode flat), or unknown (no progress sample;
    /// the probe result, if any, stated as characterisation). The three
    /// states stay visibly distinct in the line, because acting on them
    /// differs.
    pub fn liveness_line(&self, probe: Option<ProbeOutcome>, now: Duration) -> String {
        let verdict = self.liveness(probe, now);
        let mut line = format!("{} [{}]: {}", self.worker, self.model, verdict.label());
        match verdict {
            LivenessVerdict::Healthy if self.in_flight > 0 => {
                let no_token = self
                    .no_token_for(now)
                    .map(fmt_dur)
                    .unwrap_or_else(|| "0s".to_string());
                line.push_str(&format!(
                    " (busy: {} in-flight, no token for {no_token})",
                    self.in_flight
                ));
            }
            LivenessVerdict::Healthy => line.push_str(" (idle, probe answered)"),
            LivenessVerdict::Wedged => {
                let no_token = self.no_token_for(now).map(fmt_dur).unwrap_or_default();
                line.push_str(&format!(
                    " ({} in-flight, no token for {no_token})",
                    self.in_flight
                ));
            }
            LivenessVerdict::Unknown => {
                let note = match probe {
                    Some(ProbeOutcome::TimedOut) => "probe timed out, no in-flight work to sample",
                    Some(ProbeOutcome::Answered) => unreachable!("answered probe is healthy"),
                    None => "no probe on record, no in-flight work to sample",
                };
                line.push_str(&format!(
                    " ({note} — resolve with a progress sample before acting)"
                ));
            }
        }
        line
    }

    /// The last N per-worker outcomes, oldest first. History, not an
    /// aggregate.
    pub fn recent_outcomes(&self) -> &[Outcome] {
        self.recent.as_slices().0
    }

    pub fn totals(&self) -> (u64, u64) {
        (self.total_completed, self.total_failed)
    }

    /// The per-worker report line. An open circuit is stated in the line,
    /// with its age and its reopen time.
    pub fn line(&self, now: Duration) -> String {
        let (completed, failed) = self.totals();
        let total = completed + failed;
        let success_pct = if total == 0 {
            "no requests yet".to_string()
        } else {
            format!(
                "{completed}/{total} completed ({:.0}%)",
                100.0 * completed as f64 / total as f64
            )
        };
        let mut line = format!(
            "{} [{}] {} — {} consecutive failures, {} in-flight, {}",
            self.worker,
            self.model,
            self.state.label(),
            self.consecutive_failures,
            self.in_flight,
            success_pct
        );
        if let Some(no_token_for) = self.no_token_for(now) {
            line.push_str(&format!(", no token for {}", fmt_dur(no_token_for)));
        }
        match self.last_success {
            Some(at) => line.push_str(&format!(
                ", last success {}",
                fmt_dur(now.saturating_sub(at))
            )),
            None => line.push_str(", never completed"),
        }
        if self.state != CircuitState::Closed {
            line.push_str(&format!(
                " [state held {}{}]",
                fmt_dur(self.state_age(now)),
                if self.state == CircuitState::Open {
                    format!(
                        "; reopens in {}",
                        fmt_dur(
                            self.backoff
                                .saturating_sub(now.saturating_sub(self.state_since))
                        )
                    )
                } else {
                    String::new()
                }
            ));
        }
        line
    }

    fn transition(
        &mut self,
        now: Duration,
        to: CircuitState,
        reason: Option<OpenReason>,
    ) -> Transition {
        let from = self.state;
        self.state = to;
        self.state_since = now;
        if to == CircuitState::Open {
            self.backoff = if reason == Some(OpenReason::ProbeFailed) {
                self.backoff // already doubled by the probe failure
            } else {
                self.cfg.backoff_initial
            };
        }
        if to == CircuitState::Closed {
            self.probe_active = false;
        }
        let t = Transition {
            worker: self.worker.clone(),
            from,
            to,
            at: now,
            reason,
        };
        self.transitions.push(t.clone());
        t
    }
}

/// The durable slice of [`WorkerOutcome`] kept across a restart.
#[derive(Debug, Clone, PartialEq)]
pub struct RestoreState {
    pub state: CircuitState,
    pub state_since: Duration,
    pub backoff: Duration,
    pub consecutive_failures: u64,
    pub last_success: Option<Duration>,
    pub in_flight: u64,
    pub in_flight_since: Option<Duration>,
    pub last_token: Option<Duration>,
    pub recent: VecDeque<Outcome>,
    pub total_completed: u64,
    pub total_failed: u64,
    pub transitions: Vec<Transition>,
}

/// A routing decision for one worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutingDecision {
    /// Route normally.
    Route,
    /// Half-open: exactly one probe request may go.
    Probe,
    /// Excluded — and the exclusion says why.
    Excluded(Exclusion),
}

/// Why a worker is excluded from routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exclusion {
    /// The circuit is open.
    CircuitOpen {
        since: Duration,
        reopens_at: Duration,
    },
    /// A half-open probe is already in flight; no second request.
    ProbeInFlight,
    /// The declared capability cannot fit the request.
    DoesNotFit { context_window: u64, requested: u64 },
}

impl Exclusion {
    pub fn label(&self, now: Duration) -> String {
        match self {
            Exclusion::CircuitOpen { since, reopens_at } => format!(
                "circuit open for {} (reopens in {})",
                fmt_dur(now.saturating_sub(*since)),
                fmt_dur(reopens_at.saturating_sub(now))
            ),
            Exclusion::ProbeInFlight => "half-open probe in flight".to_string(),
            Exclusion::DoesNotFit {
                context_window,
                requested,
            } => format!("cannot fit: context {context_window} < requested {requested}"),
        }
    }
}

/// All registered workers, the unit that makes peer comparison free: the
/// gateway already routes to all of them.
#[derive(Debug, Clone, Default)]
pub struct Fleet {
    workers: BTreeMap<String, WorkerOutcome>,
}

impl Fleet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register (or re-register) a worker. Registration survives the
    /// breaker opening: an open worker stays in the fleet, just excluded
    /// from routing.
    pub fn register(&mut self, worker: WorkerOutcome) {
        self.workers.insert(worker.worker.clone(), worker);
    }

    pub fn get(&self, worker: &str) -> Option<&WorkerOutcome> {
        self.workers.get(worker)
    }

    pub fn len(&self) -> usize {
        self.workers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.workers.is_empty()
    }

    /// Whether any other worker on the same model completed work recently
    /// — "the siblings on the same model are answering".
    fn peers_healthy(&self, model: &str, except: &str, now: Duration) -> bool {
        self.workers
            .values()
            .any(|w| w.model == model && w.worker != except && w.is_live(now, w.cfg.peer_window))
    }

    /// The decision step for the whole fleet: every worker is evaluated
    /// against its own model's peer health. Returns all transitions
    /// emitted, so the caller reports each one.
    pub fn tick(&mut self, now: Duration) -> Vec<Transition> {
        let peer_flags: Vec<(String, bool)> = self
            .workers
            .values()
            .map(|w| {
                (
                    w.worker.clone(),
                    self.peers_healthy(&w.model, &w.worker, now),
                )
            })
            .collect();
        let mut emitted = Vec::new();
        for (id, w) in &mut self.workers {
            let peers = peer_flags
                .iter()
                .find(|(flag_id, _)| flag_id == id)
                .map(|(_, healthy)| *healthy)
                .unwrap_or(false);
            emitted.extend(w.tick(now, peers));
        }
        emitted
    }

    /// The routing report for one model: every registered worker for that
    /// model with its decision. Exclusions are stated in the line — "no
    /// capacity" and "capacity that does not work" stay visibly distinct.
    ///
    /// `requested_tokens` is `Some` when the report is for a specific
    /// request: a worker whose declared capability cannot fit it is
    /// excluded for that request and says so.
    pub fn routing_report(
        &self,
        model: &str,
        requested_tokens: Option<u64>,
        now: Duration,
    ) -> String {
        let workers: Vec<&WorkerOutcome> =
            self.workers.values().filter(|w| w.model == model).collect();
        if workers.is_empty() {
            return format!("{model}: no workers registered");
        }
        let mut parts = Vec::new();
        for w in workers {
            let decision = w.routing_decision();
            let label = match decision {
                RoutingDecision::Route => match requested_tokens {
                    Some(tokens) if !w.capability.fits(tokens) => format!(
                        "excluded: {}",
                        Exclusion::DoesNotFit {
                            context_window: w.capability.context_window,
                            requested: tokens,
                        }
                        .label(now)
                    ),
                    _ => "route".to_string(),
                },
                RoutingDecision::Probe => "probe (half-open)".to_string(),
                RoutingDecision::Excluded(ex) => format!("excluded: {}", ex.label(now)),
            };
            parts.push(format!("{} {}", w.worker, label));
        }
        format!("{model}: {}", parts.join("; "))
    }

    /// The per-worker report lines, one per worker, with the worker's own
    /// counters. The fleet is never reported as an aggregate: the fleet
    /// looked healthy in aggregate the entire time one model was at 0%
    /// success.
    pub fn summary_lines(&self, now: Duration) -> Vec<String> {
        self.workers.values().map(|w| w.line(now)).collect()
    }

    /// The fleet liveness scan (issue #4459): one line per worker, each
    /// verdict carried by the progress sample when one exists and the
    /// probe as characterisation otherwise, plus a summary that never
    /// collapses busy into bad. `probes` maps worker id to the scan's
    /// probe outcome; a worker with no entry is unprobed, which is
    /// [`LivenessVerdict::Unknown`] when it has no in-flight work.
    pub fn liveness_report(
        &self,
        probes: &BTreeMap<String, ProbeOutcome>,
        now: Duration,
    ) -> String {
        let mut lines: Vec<String> = self
            .workers
            .values()
            .map(|w| w.liveness_line(probes.get(&w.worker).copied(), now))
            .collect();
        let (mut healthy, mut wedged, mut unknown) = (0usize, 0usize, 0usize);
        for w in self.workers.values() {
            match w.liveness(probes.get(&w.worker).copied(), now) {
                LivenessVerdict::Healthy => healthy += 1,
                LivenessVerdict::Wedged => wedged += 1,
                LivenessVerdict::Unknown => unknown += 1,
            }
        }
        lines.push(format!(
            "FLEET healthy={healthy} wedged={wedged} unknown={unknown}"
        ));
        lines.join("\n")
    }
}

/// Format a duration as `1h05m`, `2m03s`, `45s`, or `250ms`.
fn fmt_dur(d: Duration) -> String {
    let secs = d.as_secs();
    let ms = d.subsec_millis();
    if secs >= 3600 {
        format!("{}h{:02}m", secs / 3600, (secs % 3600) / 60)
    } else if secs >= 60 {
        format!("{}m{:02}s", secs / 60, secs % 60)
    } else if secs > 0 {
        format!("{secs}s")
    } else {
        format!("{ms}ms")
    }
}
