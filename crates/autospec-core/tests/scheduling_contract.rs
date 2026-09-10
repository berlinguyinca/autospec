//! Issue #3325: the provider-neutral InferWeave scheduling contract —
//! affinity, priorities, streaming events, capacity, and cancellation.
//!
//! Every test is deterministic: timestamps are monotonic milliseconds
//! supplied by the test, and no clock, network, or node is touched.

use autospec_core::aar::scheduling_contract::{
    admit, admit_interactive, cancellation_release_status, order_queue, ActiveDispatch, Admission,
    AffinityTable, CancellationReleaseStatus, CapacityLimits, DispatchPriority, EventKind,
    EventLog, InteractiveRefusal, NodeCapacity, PendingDispatch, RefusalReason, SchedulingEvent,
    DEFAULT_CANCELLATION_RELEASE_MS, DEFAULT_CANCELLATION_RELEASE_SECS,
};

fn node(node_id: &str, healthy: bool, free: u64, total: u64) -> NodeCapacity {
    NodeCapacity {
        node_id: node_id.to_string(),
        healthy,
        free_context_tokens: free,
        total_context_tokens: total,
    }
}

fn limits(free_floor: u64, headroom_percent: u32) -> CapacityLimits {
    CapacityLimits {
        minimum_free_context_tokens: free_floor,
        minimum_headroom_percent: headroom_percent,
    }
}

fn active(session_id: &str, instance: &str, priority: DispatchPriority) -> ActiveDispatch {
    ActiveDispatch {
        session_id: session_id.to_string(),
        model_instance_id: instance.to_string(),
        priority,
        submitted_at_ms: 1_000,
    }
}

fn pending(session_id: &str, priority: DispatchPriority, enqueued_at_ms: u64) -> PendingDispatch {
    PendingDispatch {
        session_id: session_id.to_string(),
        priority,
        enqueued_at_ms,
    }
}

// --- Acceptance criterion 1: interactive sessions serialize at 100 -------

#[test]
fn scheduling_contract_interactive_sessions_run_at_priority_100() {
    assert_eq!(DispatchPriority::Interactive.as_u32(), 100);
    assert_eq!(
        admit_interactive(&[], "session-1").unwrap(),
        DispatchPriority::Interactive
    );
    assert_eq!(
        DispatchPriority::parse("100"),
        Some(DispatchPriority::Interactive)
    );

    // Interactive strictly outranks every other level.
    for other in DispatchPriority::ALL {
        if other != DispatchPriority::Interactive {
            assert!(DispatchPriority::Interactive > other);
        }
    }
}

#[test]
fn scheduling_contract_interactive_sessions_serialize() {
    let in_flight = vec![active("session-1", "inst-a", DispatchPriority::Interactive)];

    // A second interactive dispatch for the same session is refused, whatever
    // its urgency.
    assert_eq!(
        admit_interactive(&in_flight, "session-1"),
        Err(InteractiveRefusal::AlreadyInFlight)
    );
    // A different session is unaffected.
    assert_eq!(
        admit_interactive(&in_flight, "session-2").unwrap(),
        DispatchPriority::Interactive
    );
    // The session frees up once its in-flight dispatch is gone.
    assert_eq!(
        admit_interactive(&[], "session-1").unwrap(),
        DispatchPriority::Interactive
    );
}

#[test]
fn scheduling_contract_the_queue_orders_by_priority_then_fifo() {
    let pending = vec![
        pending("batch-1", DispatchPriority::Batch, 1_000),
        pending("probe-1", DispatchPriority::Probe, 900),
        pending("interactive-2", DispatchPriority::Interactive, 2_000),
        pending("interactive-1", DispatchPriority::Interactive, 1_500),
        pending("agent-1", DispatchPriority::AgentPrimary, 950),
    ];

    let ordered = order_queue(&pending);
    let sessions: Vec<&str> = ordered.iter().map(|p| p.session_id.as_str()).collect();
    assert_eq!(
        sessions,
        vec![
            "interactive-1",
            "interactive-2",
            "agent-1",
            "batch-1",
            "probe-1",
        ]
    );
}

// --- Acceptance criterion 2: affinity while healthy ----------------------

#[test]
fn scheduling_contract_a_repeated_session_preserves_the_same_model_instance_while_healthy() {
    let mut table = AffinityTable::default();
    table.bind("session-7", "inst-a", 1_000);

    // Repeated resolutions keep the same instance and the original binding
    // time while the instance is healthy.
    for _ in 0..3 {
        let binding = table.resolve("session-7", true).expect("healthy binding");
        assert_eq!(binding.model_instance_id, "inst-a");
        assert_eq!(binding.bound_at_ms, 1_000);
    }
    assert_eq!(table.get("session-7").unwrap().model_instance_id, "inst-a");
}

#[test]
fn scheduling_contract_an_unhealthy_instance_breaks_the_affinity_binding() {
    let mut table = AffinityTable::default();
    table.bind("session-7", "inst-a", 1_000);

    assert!(table.resolve("session-7", true).is_some());
    // The instance went unhealthy: the binding is dropped so the session is
    // routed to a healthy instance.
    assert!(table.resolve("session-7", false).is_none());
    assert!(table.get("session-7").is_none());

    table.bind("session-7", "inst-b", 5_000);
    assert_eq!(
        table.resolve("session-7", true).unwrap().model_instance_id,
        "inst-b"
    );

    // Explicit release (session end / cancellation) also drops the binding.
    let released = table.release("session-7");
    assert_eq!(released.unwrap().model_instance_id, "inst-b");
    assert!(table.is_empty());
}

// --- Acceptance criterion 3: cancellation release within 5s ---------------

#[test]
fn scheduling_contract_the_default_cancellation_release_bound_is_five_seconds() {
    assert_eq!(DEFAULT_CANCELLATION_RELEASE_SECS, 5);
    assert_eq!(DEFAULT_CANCELLATION_RELEASE_MS, 5_000);
}

#[test]
fn scheduling_contract_cancellation_emits_resource_released_within_the_configured_window() {
    let cancelled = 10_000;

    // Still open.
    assert_eq!(
        cancellation_release_status(cancelled, None, DEFAULT_CANCELLATION_RELEASE_MS),
        CancellationReleaseStatus::Pending
    );
    // Inside the window, including the exact boundary.
    assert_eq!(
        cancellation_release_status(
            cancelled,
            Some(cancelled + 3_000),
            DEFAULT_CANCELLATION_RELEASE_MS
        ),
        CancellationReleaseStatus::WithinDeadline
    );
    assert_eq!(
        cancellation_release_status(
            cancelled,
            Some(cancelled + DEFAULT_CANCELLATION_RELEASE_MS),
            DEFAULT_CANCELLATION_RELEASE_MS
        ),
        CancellationReleaseStatus::WithinDeadline
    );
    // Past the window.
    assert_eq!(
        cancellation_release_status(
            cancelled,
            Some(cancelled + DEFAULT_CANCELLATION_RELEASE_MS + 1),
            DEFAULT_CANCELLATION_RELEASE_MS
        ),
        CancellationReleaseStatus::Lapsed
    );
    // A custom bound is honoured.
    assert_eq!(
        cancellation_release_status(cancelled, Some(cancelled + 2_000), 1_000),
        CancellationReleaseStatus::Lapsed
    );
}

#[test]
fn scheduling_contract_the_event_log_records_a_full_cancellation_with_resource_released() {
    let mut log = EventLog::default();
    let session = "session-9";
    let instance = "inst-a";

    log.record(SchedulingEvent::new(
        EventKind::Submitted,
        session,
        instance,
        1_000,
    ))
    .unwrap();
    log.record(SchedulingEvent::new(
        EventKind::FirstToken,
        session,
        instance,
        1_400,
    ))
    .unwrap();
    log.record(SchedulingEvent::new(
        EventKind::ResourceReleased,
        session,
        instance,
        3_000,
    ))
    .unwrap();

    let released = log
        .latest(session, EventKind::ResourceReleased)
        .expect("resource_released recorded");
    assert_eq!(released.monotonic_ms, 3_000);
    assert_eq!(log.time_to_first_token_ms(session), Some(400));
    assert_eq!(
        cancellation_release_status(
            1_000,
            log.latest(session, EventKind::ResourceReleased)
                .map(|e| e.monotonic_ms),
            DEFAULT_CANCELLATION_RELEASE_MS
        ),
        CancellationReleaseStatus::WithinDeadline
    );
}

#[test]
fn scheduling_contract_the_event_log_rejects_impossible_sequences() {
    // A final token before any first token.
    let mut log = EventLog::default();
    log.record(SchedulingEvent::new(EventKind::Submitted, "s", "i", 100))
        .unwrap();
    let err = log
        .record(SchedulingEvent::new(EventKind::FinalToken, "s", "i", 200))
        .unwrap_err();
    assert!(err.contains("first_token"), "got: {err}");

    // A token before any submission.
    let mut log = EventLog::default();
    let err = log
        .record(SchedulingEvent::new(EventKind::FirstToken, "s", "i", 100))
        .unwrap_err();
    assert!(err.contains("submitted"), "got: {err}");

    // A release of something never submitted.
    let mut log = EventLog::default();
    let err = log
        .record(SchedulingEvent::new(
            EventKind::ResourceReleased,
            "s",
            "i",
            100,
        ))
        .unwrap_err();
    assert!(err.contains("submitted"), "got: {err}");

    // A non-monotonic timestamp.
    let mut log = EventLog::default();
    log.record(SchedulingEvent::new(EventKind::Submitted, "s", "i", 200))
        .unwrap();
    let err = log
        .record(SchedulingEvent::new(
            EventKind::ResourceReleased,
            "s",
            "i",
            199,
        ))
        .unwrap_err();
    assert!(err.contains("non-monotonic"), "got: {err}");

    // A later session's events may interleave at equal-or-later timestamps.
    let mut log = EventLog::default();
    log.record(SchedulingEvent::new(EventKind::Submitted, "a", "i", 100))
        .unwrap();
    log.record(SchedulingEvent::new(EventKind::Submitted, "b", "i", 100))
        .unwrap();
    log.record(SchedulingEvent::new(EventKind::FirstToken, "b", "i", 150))
        .unwrap();
    log.record(SchedulingEvent::new(EventKind::FirstToken, "a", "i", 150))
        .unwrap();
    assert_eq!(log.events().len(), 4);
}

// --- Acceptance criterion 4: unhealthy nodes receive zero dispatches ------

#[test]
fn scheduling_contract_an_unhealthy_node_receives_zero_new_dispatches() {
    // Even with absurd headroom, an unhealthy node is refused.
    let drowning_in_headroom = node("down", false, 999_999_999, 1_000_000_000);
    assert_eq!(
        admit(&drowning_in_headroom, &limits(0, 0)),
        Admission::Refused(RefusalReason::Unhealthy)
    );
    assert_eq!(
        admit(&drowning_in_headroom, &limits(1_000, 90)),
        Admission::Refused(RefusalReason::Unhealthy)
    );
}

#[test]
fn scheduling_contract_a_healthy_node_is_admitted_only_when_headroom_meets_the_limits() {
    // 50% headroom, 40_000 free tokens: admitted under permissive limits.
    let healthy = node("ok", true, 40_000, 80_000);
    assert_eq!(admit(&healthy, &limits(1_000, 10)), Admission::Admitted);

    // Free context below the floor.
    let tight = node("tight", true, 500, 80_000);
    assert_eq!(
        admit(&tight, &limits(1_000, 0)),
        Admission::Refused(RefusalReason::InsufficientFreeContext)
    );

    // Headroom below the floor, even with plenty of free tokens.
    assert_eq!(
        admit(&healthy, &limits(0, 60)),
        Admission::Refused(RefusalReason::InsufficientHeadroom)
    );

    // The advertised headroom is exact integer arithmetic.
    assert_eq!(healthy.headroom_percent(), 50);
    assert_eq!(node("full", true, 0, 80_000).headroom_percent(), 0);
    assert_eq!(node("zero-total", true, 0, 0).headroom_percent(), 0);
}

// --- Streaming events ------------------------------------------------------

#[test]
fn scheduling_contract_a_complete_streaming_run_records_submitted_first_and_final_token() {
    let mut log = EventLog::default();
    let session = "session-3";
    let instance = "inst-b";

    log.record(SchedulingEvent::new(
        EventKind::Submitted,
        session,
        instance,
        10_000,
    ))
    .unwrap();
    log.record(SchedulingEvent::new(
        EventKind::FirstToken,
        session,
        instance,
        10_400,
    ))
    .unwrap();
    log.record(SchedulingEvent::new(
        EventKind::FinalToken,
        session,
        instance,
        13_250,
    ))
    .unwrap();

    assert_eq!(log.time_to_first_token_ms(session), Some(400));
    assert_eq!(log.streaming_duration_ms(session), Some(3_250));
    assert!(log.latest(session, EventKind::ResourceReleased).is_none());

    // All four event kinds carry the contract wire names.
    assert_eq!(
        EventKind::ALL
            .iter()
            .map(|k| k.as_str())
            .collect::<Vec<_>>(),
        vec![
            "submitted",
            "first_token",
            "final_token",
            "resource_released"
        ]
    );
    for kind in EventKind::ALL {
        assert_eq!(EventKind::parse(kind.as_str()), Some(kind));
    }
    assert_eq!(EventKind::parse("token"), None);
}

#[test]
fn scheduling_contract_the_seven_priority_levels_are_exactly_the_contract_set() {
    let values: Vec<u32> = DispatchPriority::ALL.iter().map(|p| p.as_u32()).collect();
    assert_eq!(values, vec![100, 80, 70, 60, 30, 20, 10]);
}
