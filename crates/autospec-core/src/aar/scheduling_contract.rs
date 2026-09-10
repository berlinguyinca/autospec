//! Provider-neutral InferWeave scheduling contract (issue #3325).
//!
//! Extends the capability contract in `inferweave.rs` with what the spec adds
//! on top of raw routing: session affinity pinned to a model instance, a
//! seven-level queue priority, the four streaming lifecycle events, capacity
//! admission, and cancellation propagation. AutoSpec owns this contract;
//! InferWeave implements it in an isolated companion-repository issue.
//!
//! Nothing here is node-specific: no hard-coded hardware winners, no server
//! internals, and no production node mutation. Every function is pure and
//! deterministic — all timestamps are monotonic milliseconds supplied by the
//! caller.

use std::collections::BTreeMap;

/// The default bound, in seconds, between a cancellation and its
/// `resource_released` event.
pub const DEFAULT_CANCELLATION_RELEASE_SECS: u64 = 5;

/// The default cancellation-release bound in milliseconds.
pub const DEFAULT_CANCELLATION_RELEASE_MS: u64 = DEFAULT_CANCELLATION_RELEASE_SECS * 1000;

/// Queue priorities, most urgent first.
///
/// The numeric values (100, 80, 70, 60, 30, 20, 10) are contract values: the
/// scheduler orders work by them, so implementations and the contract agree
/// on the numbers, not on the names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DispatchPriority {
    /// 100 — an interactive, human-in-the-loop session. Serialized per
    /// session: at most one in-flight dispatch at a time.
    Interactive,
    /// 80 — a primary coding-agent session.
    AgentPrimary,
    /// 70 — a subagent dispatch spawned by an agent session.
    Subagent,
    /// 60 — queued auto-implementation work.
    Batch,
    /// 30 — background jobs (sweeps, watchdog work).
    Background,
    /// 20 — housekeeping (GC, compaction).
    Maintenance,
    /// 10 — probe and verification traffic.
    Probe,
}

impl DispatchPriority {
    /// All seven levels, most urgent first.
    pub const ALL: [Self; 7] = [
        Self::Interactive,
        Self::AgentPrimary,
        Self::Subagent,
        Self::Batch,
        Self::Background,
        Self::Maintenance,
        Self::Probe,
    ];

    /// The contract value: 100, 80, 70, 60, 30, 20, or 10.
    pub fn as_u32(self) -> u32 {
        match self {
            Self::Interactive => 100,
            Self::AgentPrimary => 80,
            Self::Subagent => 70,
            Self::Batch => 60,
            Self::Background => 30,
            Self::Maintenance => 20,
            Self::Probe => 10,
        }
    }

    /// Parse by contract value (`"100"`) or by name (`"interactive"`).
    /// Unknown text is an error, not a silent fallback.
    pub fn parse(value: &str) -> Option<Self> {
        let text = value.trim();
        let by_name = match text.to_ascii_lowercase().as_str() {
            "interactive" => Some(Self::Interactive),
            "agent" | "agent_primary" => Some(Self::AgentPrimary),
            "subagent" => Some(Self::Subagent),
            "batch" => Some(Self::Batch),
            "background" => Some(Self::Background),
            "maintenance" => Some(Self::Maintenance),
            "probe" => Some(Self::Probe),
            _ => None,
        };
        if text.chars().all(|c| c.is_ascii_digit()) {
            let value: u32 = text.parse().ok()?;
            return Self::ALL.into_iter().find(|p| p.as_u32() == value);
        }
        by_name
    }
}

impl PartialOrd for DispatchPriority {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for DispatchPriority {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.as_u32().cmp(&other.as_u32())
    }
}

/// A session's binding to the model instance serving it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AffinityBinding {
    pub session_id: String,
    pub model_instance_id: String,
    /// Monotonic ms at which the binding was established.
    pub bound_at_ms: u64,
}

/// Session-to-model-instance affinity.
///
/// The contract (acceptance criterion 2): a repeated session preserves the
/// same `model_instance_id` while the bound instance is healthy. When the
/// bound instance becomes unhealthy, the binding is dropped so the scheduler
/// routes the session to a healthy instance.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AffinityTable {
    bindings: BTreeMap<String, AffinityBinding>,
}

impl AffinityTable {
    /// Bind (or re-bind) a session to a model instance.
    pub fn bind(
        &mut self,
        session_id: &str,
        model_instance_id: &str,
        now_ms: u64,
    ) -> AffinityBinding {
        let binding = AffinityBinding {
            session_id: session_id.to_string(),
            model_instance_id: model_instance_id.to_string(),
            bound_at_ms: now_ms,
        };
        self.bindings
            .insert(binding.session_id.clone(), binding.clone());
        binding
    }

    /// Re-resolve a session's instance.
    ///
    /// Returns the existing binding while the bound instance is healthy.
    /// When it is unhealthy, drops the binding and returns `None` so the
    /// session is routed to a healthy instance instead.
    pub fn resolve(
        &mut self,
        session_id: &str,
        instance_healthy: bool,
    ) -> Option<&AffinityBinding> {
        if instance_healthy && self.bindings.contains_key(session_id) {
            return self.bindings.get(session_id);
        }
        if !instance_healthy {
            self.bindings.remove(session_id);
        }
        None
    }

    /// Release a session's binding (session end or cancellation).
    pub fn release(&mut self, session_id: &str) -> Option<AffinityBinding> {
        self.bindings.remove(session_id)
    }

    /// The current binding for a session, if any.
    pub fn get(&self, session_id: &str) -> Option<&AffinityBinding> {
        self.bindings.get(session_id)
    }

    pub fn len(&self) -> usize {
        self.bindings.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }
}

/// What a node advertises about its capacity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeCapacity {
    pub node_id: String,
    /// Advertised health, from the node's own health endpoint.
    pub healthy: bool,
    pub free_context_tokens: u64,
    pub total_context_tokens: u64,
}

impl NodeCapacity {
    /// Advertised context headroom as a percentage of the total, in exact
    /// integer arithmetic. 0 when the total is 0.
    pub fn headroom_percent(&self) -> u32 {
        if self.total_context_tokens == 0 {
            return 0;
        }
        let percent = self.free_context_tokens.saturating_mul(100) / self.total_context_tokens;
        percent.min(100) as u32
    }
}

/// The configured limits a node must meet to accept new work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapacityLimits {
    /// Minimum free context tokens the node must advertise.
    pub minimum_free_context_tokens: u64,
    /// Minimum context headroom, as a percentage of the total (0-100).
    pub minimum_headroom_percent: u32,
}

impl Default for CapacityLimits {
    fn default() -> Self {
        Self {
            minimum_free_context_tokens: 0,
            minimum_headroom_percent: 0,
        }
    }
}

/// Why a node was refused new work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefusalReason {
    /// Advertised health is down: the node receives zero new dispatches.
    Unhealthy,
    /// Advertised free context is below the configured floor.
    InsufficientFreeContext,
    /// Advertised context headroom is below the configured floor.
    InsufficientHeadroom,
}

impl RefusalReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Unhealthy => "unhealthy",
            Self::InsufficientFreeContext => "insufficient_free_context",
            Self::InsufficientHeadroom => "insufficient_headroom",
        }
    }
}

/// The admission outcome for one node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    Admitted,
    Refused(RefusalReason),
}

impl Admission {
    pub fn is_admitted(&self) -> bool {
        matches!(self, Self::Admitted)
    }
}

/// Refuse new work when advertised health or headroom violates the
/// configured limits.
///
/// Health is checked first and is absolute: an unhealthy node receives zero
/// new dispatches no matter how much headroom it advertises (acceptance
/// criterion 4).
pub fn admit(capacity: &NodeCapacity, limits: &CapacityLimits) -> Admission {
    if !capacity.healthy {
        return Admission::Refused(RefusalReason::Unhealthy);
    }
    if capacity.free_context_tokens < limits.minimum_free_context_tokens {
        return Admission::Refused(RefusalReason::InsufficientFreeContext);
    }
    if capacity.headroom_percent() < limits.minimum_headroom_percent {
        return Admission::Refused(RefusalReason::InsufficientHeadroom);
    }
    Admission::Admitted
}

/// The four streaming lifecycle events, in the order the contract requires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventKind {
    /// The dispatch was accepted and submitted to a model instance.
    Submitted,
    /// The first streamed token was delivered.
    FirstToken,
    /// The final streamed token was delivered.
    FinalToken,
    /// A cancelled dispatch released its resources.
    ResourceReleased,
}

impl EventKind {
    /// The four kinds in contract order.
    pub const ALL: [Self; 4] = [
        Self::Submitted,
        Self::FirstToken,
        Self::FinalToken,
        Self::ResourceReleased,
    ];

    /// The wire name used in the event log.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Submitted => "submitted",
            Self::FirstToken => "first_token",
            Self::FinalToken => "final_token",
            Self::ResourceReleased => "resource_released",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.as_str() == value.trim())
    }
}

/// One recorded lifecycle event. Timestamps are monotonic milliseconds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchedulingEvent {
    pub kind: EventKind,
    pub session_id: String,
    pub model_instance_id: String,
    pub monotonic_ms: u64,
}

impl SchedulingEvent {
    pub fn new(
        kind: EventKind,
        session_id: &str,
        model_instance_id: &str,
        monotonic_ms: u64,
    ) -> Self {
        Self {
            kind,
            session_id: session_id.to_string(),
            model_instance_id: model_instance_id.to_string(),
            monotonic_ms,
        }
    }
}

/// An append-only event log with ordering validation.
///
/// The log is the deterministic evidence a streaming run actually happened:
/// `submitted`, `first_token`, `final_token`, and — for a cancelled run —
/// `resource_released`, each with the monotonic ms at which it was observed.
#[derive(Debug, Clone, Default)]
pub struct EventLog {
    events: Vec<SchedulingEvent>,
}

impl EventLog {
    /// Append an event, rejecting out-of-order timestamps and impossible
    /// sequences (a token before submission, a final token before a first
    /// one, a release of something never submitted).
    pub fn record(&mut self, event: SchedulingEvent) -> Result<(), String> {
        if let Some(last) = self.events.last() {
            if event.monotonic_ms < last.monotonic_ms {
                return Err(format!(
                    "non-monotonic timestamp: {} at {}ms follows {} at {}ms",
                    event.kind.as_str(),
                    event.monotonic_ms,
                    last.kind.as_str(),
                    last.monotonic_ms
                ));
            }
        }
        let have = |kind: EventKind| {
            self.events
                .iter()
                .any(|e| e.kind == kind && e.session_id == event.session_id)
        };
        let missing = match event.kind {
            EventKind::Submitted => None,
            EventKind::FirstToken if !have(EventKind::Submitted) => Some(EventKind::Submitted),
            EventKind::FinalToken if !have(EventKind::Submitted) => Some(EventKind::Submitted),
            EventKind::FinalToken if !have(EventKind::FirstToken) => Some(EventKind::FirstToken),
            EventKind::ResourceReleased if !have(EventKind::Submitted) => {
                Some(EventKind::Submitted)
            }
            _ => None,
        };
        if let Some(prerequisite) = missing {
            return Err(format!(
                "{} for session {} recorded before {}",
                event.kind.as_str(),
                event.session_id,
                prerequisite.as_str()
            ));
        }
        self.events.push(event);
        Ok(())
    }

    pub fn events(&self) -> &[SchedulingEvent] {
        &self.events
    }

    /// The most recent event of a kind for a session, if any.
    pub fn latest(&self, session_id: &str, kind: EventKind) -> Option<&SchedulingEvent> {
        self.events
            .iter()
            .rev()
            .find(|e| e.session_id == session_id && e.kind == kind)
    }

    /// Milliseconds from `submitted` to `first_token`, when both are recorded.
    pub fn time_to_first_token_ms(&self, session_id: &str) -> Option<u64> {
        let submitted = self.latest(session_id, EventKind::Submitted)?;
        let first = self.latest(session_id, EventKind::FirstToken)?;
        Some(first.monotonic_ms.saturating_sub(submitted.monotonic_ms))
    }

    /// Milliseconds from `submitted` to `final_token`, when both are recorded.
    pub fn streaming_duration_ms(&self, session_id: &str) -> Option<u64> {
        let submitted = self.latest(session_id, EventKind::Submitted)?;
        let final_token = self.latest(session_id, EventKind::FinalToken)?;
        Some(
            final_token
                .monotonic_ms
                .saturating_sub(submitted.monotonic_ms),
        )
    }
}

/// The state of a cancellation's resource release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancellationReleaseStatus {
    /// The `resource_released` event has not been recorded yet.
    Pending,
    /// Released within the configured bound.
    WithinDeadline,
    /// Released after the bound (or before the cancellation was recorded).
    Lapsed,
}

/// Check a cancellation's release against its bound: `resource_released`
/// must follow `cancelled_at_ms` by at most `limit_ms` — the contract
/// default is the configured 5 seconds (acceptance criterion 3).
pub fn cancellation_release_status(
    cancelled_at_ms: u64,
    resource_released_ms: Option<u64>,
    limit_ms: u64,
) -> CancellationReleaseStatus {
    let Some(released) = resource_released_ms else {
        return CancellationReleaseStatus::Pending;
    };
    if released < cancelled_at_ms {
        return CancellationReleaseStatus::Lapsed;
    }
    if released - cancelled_at_ms <= limit_ms {
        CancellationReleaseStatus::WithinDeadline
    } else {
        CancellationReleaseStatus::Lapsed
    }
}

/// A dispatch currently in flight on a model instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveDispatch {
    pub session_id: String,
    pub model_instance_id: String,
    pub priority: DispatchPriority,
    pub submitted_at_ms: u64,
}

/// Why an interactive dispatch was not admitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractiveRefusal {
    /// A dispatch is already in flight for the session: interactive sessions
    /// serialize, so a second one is queued rather than admitted.
    AlreadyInFlight,
}

/// The interactive rule (acceptance criterion 1): an interactive session is
/// admitted with priority 100, and only while no dispatch for that session is
/// already in flight — interactive sessions serialize.
pub fn admit_interactive(
    active: &[ActiveDispatch],
    session_id: &str,
) -> Result<DispatchPriority, InteractiveRefusal> {
    if active.iter().any(|d| d.session_id == session_id) {
        return Err(InteractiveRefusal::AlreadyInFlight);
    }
    Ok(DispatchPriority::Interactive)
}

/// A dispatch waiting in the queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingDispatch {
    pub session_id: String,
    pub priority: DispatchPriority,
    pub enqueued_at_ms: u64,
}

/// Order pending dispatches: highest priority first, FIFO within a priority,
/// session id as the final deterministic tiebreak.
pub fn order_queue(pending: &[PendingDispatch]) -> Vec<PendingDispatch> {
    let mut ordered = pending.to_vec();
    ordered.sort_by(|a, b| {
        b.priority
            .cmp(&a.priority)
            .then_with(|| a.enqueued_at_ms.cmp(&b.enqueued_at_ms))
            .then_with(|| a.session_id.cmp(&b.session_id))
    });
    ordered
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_priority_levels_carry_the_contract_values() {
        let values: Vec<u32> = DispatchPriority::ALL.iter().map(|p| p.as_u32()).collect();
        assert_eq!(values, vec![100, 80, 70, 60, 30, 20, 10]);
        // Most urgent sorts last under the ascending Ord, so the contract
        // ordering is strict and total.
        let mut ascending = DispatchPriority::ALL.to_vec();
        ascending.sort();
        assert_eq!(
            ascending,
            vec![
                DispatchPriority::Probe,
                DispatchPriority::Maintenance,
                DispatchPriority::Background,
                DispatchPriority::Batch,
                DispatchPriority::Subagent,
                DispatchPriority::AgentPrimary,
                DispatchPriority::Interactive,
            ]
        );
    }

    #[test]
    fn priorities_parse_by_value_and_by_name() {
        assert_eq!(
            DispatchPriority::parse("100"),
            Some(DispatchPriority::Interactive)
        );
        assert_eq!(
            DispatchPriority::parse("interactive"),
            Some(DispatchPriority::Interactive)
        );
        assert_eq!(DispatchPriority::parse("10"), Some(DispatchPriority::Probe));
        assert_eq!(
            DispatchPriority::parse("probe"),
            Some(DispatchPriority::Probe)
        );
        assert_eq!(DispatchPriority::parse("55"), None);
        assert_eq!(DispatchPriority::parse("urgent"), None);
    }
}
