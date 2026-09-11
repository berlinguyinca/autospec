//! Per-worker outcome state and the circuit breaker (issue #4378).
//!
//! The regression tests run in the configuration the incident required:
//! a worker whose `/health` kept returning 200 while generation stopped —
//! the gateway logged the probe timeout 237 times and routed to the
//! deadlocked worker for hours, because the signals had nowhere to go.

use std::time::Duration;

use autospec_core::circuit_breaker::{
    BreakerConfig, CircuitState, Exclusion, Fleet, OpenReason, Outcome, RoutingDecision,
    Transition, WorkerCapability, WorkerOutcome,
};

fn s(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn cap(context_window: u64) -> WorkerCapability {
    WorkerCapability {
        slots: 8,
        context_window,
        quantisation: "q8".to_string(),
    }
}

fn worker(id: &str, model: &str) -> WorkerOutcome {
    WorkerOutcome::new(
        id,
        model,
        cap(131_072),
        BreakerConfig::default(),
        Duration::ZERO,
    )
}

/// A request that started and then failed.
fn fail(w: &mut WorkerOutcome, at: Duration) {
    w.request_started(at);
    w.request_finished(at + s(1), Outcome::Failed);
}

#[test]
fn the_incident_237_probe_timeouts_open_the_circuit_and_the_exclusion_is_reported() {
    // The incident shape: the worker deadlocked, /health kept answering
    // 200, and the gateway's probe timed out 237 times against it — all
    // logged, none counted.
    let mut w = worker("gw-b", "qwen3.8");
    for i in 0..237 {
        w.probe_timeout(s(i as u64 + 1));
    }
    assert_eq!(w.consecutive_failures(), 237);
    // Invariant 2: the detector writes to state a decision can read.
    assert_eq!(
        w.routing_decision(),
        RoutingDecision::Route, // nothing decided yet — the tick decides
    );

    let transitions = w.tick(s(240), false);
    assert_eq!(transitions.len(), 1);
    assert_eq!(
        transitions[0],
        Transition {
            worker: "gw-b".to_string(),
            from: CircuitState::Closed,
            to: CircuitState::Open,
            at: s(240),
            reason: Some(OpenReason::ConsecutiveFailures { count: 237 }),
        }
    );
    assert_eq!(w.state(), CircuitState::Open);

    // Invariant 4: the routing decision excludes the peer and says that
    // it did — with the age and the reopen time, so "no capacity" and
    // "capacity that does not work" stay visibly distinct.
    assert_eq!(
        w.routing_decision(),
        RoutingDecision::Excluded(Exclusion::CircuitOpen {
            since: s(240),
            reopens_at: s(270),
        })
    );

    // The worker stays registered: the fleet reports the exclusion rather
    // than silently dropping the worker.
    let mut fleet = Fleet::new();
    fleet.register(w);
    assert_eq!(fleet.len(), 1);
    let report = fleet.routing_report("qwen3.8", None, s(250));
    assert!(
        report.contains("gw-b excluded: circuit open for 10s (reopens in 20s)"),
        "report was: {report}"
    );
    let lines = fleet.summary_lines(s(250));
    assert!(
        lines
            .iter()
            .any(|l| l.contains("gw-b") && l.contains("open")),
        "summary was: {lines:?}"
    );
}

#[test]
fn a_health_ok_worker_that_never_completed_is_not_live() {
    // Invariant 1: liveness is the last successful unit of work, never a
    // health endpoint. There is no method here that records a health
    // response — a /health 200 cannot make this state.
    let mut w = worker("gw-b", "qwen3.8");
    for i in 0..50 {
        w.probe_timeout(s(i + 1));
    }
    assert_eq!(w.last_success(), None);
    assert!(!w.is_live(s(200), s(60)));

    // A real completion is the only thing that sets liveness, and it
    // expires.
    let mut live = worker("gw-a", "qwen3.8");
    live.request_started(s(100));
    live.request_finished(s(101), Outcome::Completed);
    assert_eq!(live.last_success(), Some(s(101)));
    assert!(live.is_live(s(120), s(60)));
    assert!(!live.is_live(s(200), s(60)));
}

#[test]
fn busy_with_no_progress_opens_the_circuit_but_progress_keeps_it_closed() {
    // "Busy is not dead" needs a bound: three in-flight requests and no
    // token in 120 seconds is not busy. The bound belongs next to the
    // rule (BreakerConfig::stuck_timeout).
    let mut stuck = worker("gw-b", "qwen3.8");
    for _ in 0..3 {
        stuck.request_started(s(0));
    }
    assert_eq!(stuck.in_flight(), 3);
    // 120 seconds with no token: exactly the bound, not stuck yet
    // (>= is the test at 120).
    assert!(stuck.stuck(s(119)).is_none());
    assert_eq!(stuck.stuck(s(120)), Some(s(120)));

    let transitions = stuck.tick(s(120), false);
    assert_eq!(
        transitions,
        vec![Transition {
            worker: "gw-b".to_string(),
            from: CircuitState::Closed,
            to: CircuitState::Open,
            at: s(120),
            reason: Some(OpenReason::Stuck {
                no_progress_for: s(120),
                in_flight: 3,
            }),
        }]
    );

    // Invariant 3: the discriminator is progress, not wall-clock. A
    // request two hours old that keeps emitting tokens is slow, not
    // stopped — it never opens.
    let mut slow = worker("gw-a", "qwen3.8");
    slow.request_started(s(0));
    for t in (0..=7200).step_by(10) {
        slow.token(s(t));
    }
    assert_eq!(slow.in_flight(), 1);
    assert!(slow.stuck(s(7200)).is_none());
    let transitions = slow.tick(s(7200), false);
    assert!(transitions.is_empty());
    assert_eq!(slow.state(), CircuitState::Closed);

    // An idle worker with no tokens is not stuck either: there is no
    // in-flight work to be stuck on.
    let idle = worker("gw-c", "qwen3.8");
    assert!(idle.stuck(s(10_000)).is_none());
}

#[test]
fn half_open_admits_one_probe_and_success_closes_failure_reopens_longer() {
    let mut w = worker("gw-b", "qwen3.8");
    for i in 0..5 {
        w.probe_timeout(s(i as u64 + 1));
    }
    assert_eq!(w.tick(s(10), false).len(), 1);
    assert_eq!(w.state(), CircuitState::Open);
    assert_eq!(
        w.routing_decision(),
        RoutingDecision::Excluded(Exclusion::CircuitOpen {
            since: s(10),
            reopens_at: s(40),
        })
    );

    // Back-off elapsed: half-open, exactly one probe.
    assert_eq!(w.tick(s(40), false).len(), 1);
    assert_eq!(w.state(), CircuitState::HalfOpen);
    assert_eq!(w.routing_decision(), RoutingDecision::Probe);
    assert!(w.send_probe(s(40)));
    // No second request while the probe is in flight.
    assert!(!w.send_probe(s(41)));
    assert_eq!(
        w.routing_decision(),
        RoutingDecision::Excluded(Exclusion::ProbeInFlight)
    );

    // Probe succeeds: closed, back-off reset.
    w.request_finished(s(50), Outcome::Completed);
    assert_eq!(w.state(), CircuitState::Closed);
    assert_eq!(w.consecutive_failures(), 0);

    // Re-open, then a failed probe: re-opened with a doubled back-off.
    for i in 0..5 {
        w.probe_timeout(s(100 + i as u64));
    }
    w.tick(s(110), false);
    assert_eq!(w.state(), CircuitState::Open);
    w.tick(s(140), false); // back-off 30s elapsed
    assert_eq!(w.state(), CircuitState::HalfOpen);
    assert!(w.send_probe(s(140)));
    w.request_finished(s(141), Outcome::Failed);
    assert_eq!(w.state(), CircuitState::Open);
    assert_eq!(
        w.routing_decision(),
        RoutingDecision::Excluded(Exclusion::CircuitOpen {
            since: s(141),
            reopens_at: s(141 + 60), // 30s doubled
        })
    );
}

#[test]
fn a_failure_against_healthy_peers_opens_sooner_than_the_absolute_threshold() {
    // Compare peers: a worker failing while its siblings on the same model
    // succeeded recently is a far stronger signal, so the threshold drops.
    let mut fleet = Fleet::new();
    let mut a = worker("gw-a", "qwen3.8");
    a.request_started(s(90));
    a.request_finished(s(91), Outcome::Completed);
    fleet.register(a);
    let mut b = worker("gw-b", "qwen3.8");
    fail(&mut b, s(95));
    fail(&mut b, s(96)); // 2 consecutive — under the absolute threshold of 5
    fleet.register(b);

    let transitions = fleet.tick(s(100));
    assert_eq!(
        transitions,
        vec![Transition {
            worker: "gw-b".to_string(),
            from: CircuitState::Closed,
            to: CircuitState::Open,
            at: s(100),
            reason: Some(OpenReason::ConsecutiveFailures { count: 2 }),
        }]
    );

    // Control: the same 4 failures with no live peer do not open — the
    // absolute threshold still applies.
    let mut lonely = worker("gw-c", "qwen3.8");
    for t in 0..4 {
        fail(&mut lonely, s(t as u64 + 1));
    }
    let transitions = lonely.tick(s(10), false);
    assert!(transitions.is_empty());
    assert_eq!(lonely.state(), CircuitState::Closed);
    // ...until the fifth.
    fail(&mut lonely, s(11));
    assert_eq!(lonely.tick(s(12), false).len(), 1);
    assert_eq!(lonely.state(), CircuitState::Open);
}

#[test]
fn a_restart_restores_the_picture_instead_of_starting_optimistic() {
    let mut w = worker("gw-b", "qwen3.8");
    for i in 0..5 {
        w.probe_timeout(s(i as u64 + 1));
    }
    w.tick(s(100), false);
    assert_eq!(w.state(), CircuitState::Open);

    let snap = w.snapshot();
    // The gateway restarts an hour later.
    let restored = WorkerOutcome::restore(
        "gw-b",
        "qwen3.8",
        cap(131_072),
        BreakerConfig::default(),
        snap,
    );
    // "How long has this been bad" survives the restart.
    assert_eq!(restored.state(), CircuitState::Open);
    assert_eq!(restored.state_age(s(3700)), s(3600));
    assert_eq!(
        restored.routing_decision(),
        RoutingDecision::Excluded(Exclusion::CircuitOpen {
            since: s(100),
            reopens_at: s(130),
        })
    );
    // The transition history came along, so the picture is the old one,
    // not a fresh optimistic one.
    assert_eq!(restored.snapshot().transitions.len(), 1);

    // And it is not stuck forever: after the back-off the restored
    // worker goes half-open like any open one.
    let mut r = restored;
    assert_eq!(r.tick(s(3700), false).len(), 1);
    assert_eq!(r.state(), CircuitState::HalfOpen);
}

#[test]
fn the_report_is_per_worker_so_one_dead_model_is_not_hidden_by_the_aggregate() {
    // Invariant 5: per-peer outcome history, not just aggregate counters.
    // The fleet looked healthy in aggregate the entire time one model was
    // at 0% success.
    let mut fleet = Fleet::new();
    let mut a = worker("gw-a", "qwen3.8");
    a.request_started(s(0));
    a.request_finished(s(1), Outcome::Completed);
    fleet.register(a);

    let mut b = worker("gw-b", "qwen3.8");
    for t in 0..10 {
        fail(&mut b, s(t as u64 + 1));
    }
    fleet.register(b);
    fleet.tick(s(30));

    let lines = fleet.summary_lines(s(30));
    assert_eq!(lines.len(), 2, "one line per worker, got: {lines:?}");
    let a_line = lines.iter().find(|l| l.starts_with("gw-a")).unwrap();
    let b_line = lines.iter().find(|l| l.starts_with("gw-b")).unwrap();
    assert!(a_line.contains("1/1 completed (100%)"), "was: {a_line}");
    assert!(
        b_line.contains("0/10 completed (0%)") && b_line.contains("open"),
        "was: {b_line}"
    );
}

#[test]
fn the_declared_capability_rejects_a_request_that_cannot_fit() {
    let mut fleet = Fleet::new();
    let big = worker("gw-big", "qwen3.8");
    let small = WorkerOutcome::new(
        "gw-small",
        "qwen3.8",
        cap(32_768),
        BreakerConfig::default(),
        Duration::ZERO,
    );
    fleet.register(big);
    fleet.register(small);

    // 64k tokens: the 32k-window worker provably cannot fit them.
    let report = fleet.routing_report("qwen3.8", Some(65_536), s(0));
    assert!(report.contains("gw-big route"), "was: {report}");
    assert!(
        report.contains("gw-small excluded: cannot fit: context 32768 < requested 65536"),
        "was: {report}"
    );

    // And the same worker routes a request that fits.
    let report = fleet.routing_report("qwen3.8", Some(32_768), s(0));
    assert!(report.contains("gw-small route"), "was: {report}");
}

#[test]
fn recent_outcomes_are_kept_per_worker_and_bounded() {
    let mut w = worker("gw-b", "qwen3.8");
    for i in 0..20 {
        w.probe_timeout(s(i + 1));
    }
    assert_eq!(w.recent_outcomes().len(), 16); // the configured history_len
    assert!(w.recent_outcomes().iter().all(|o| *o == Outcome::Failed));

    // The history is per worker: a sibling's outcomes do not mix in.
    let mut other = worker("gw-a", "qwen3.8");
    other.request_started(s(1));
    other.request_finished(s(2), Outcome::Completed);
    assert_eq!(other.recent_outcomes(), &[Outcome::Completed]);
}
