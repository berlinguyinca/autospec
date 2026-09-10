//! Service-address resolution from the authoritative record at use time
//! (issue #3776).
//!
//! The five acceptance scenarios, in the order the incident produced them:
//!
//! * the address is read from the record the gateway writes, not from the
//!   value handed to the worker at launch;
//! * a long-lived process that fails to reach the address re-reads the record
//!   and follows the service to its new location;
//! * a worker that serves but cannot register is degraded, not healthy;
//! * "the scheduler says the job is RUNNING" is not a health check;
//! * the reconciler reports pool size and flags a sustained decline.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use autospec_core::service_address::{
    component_health, parse_address, read_record, service_health, AddressOrigin, AddressResolver,
    ComponentHealth, HealthEvidence, PoolMonitor, PoolTrend, RecordError, RegistrationOutcome,
    ServiceHealth,
};

fn temp_dir(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after the epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "autospec-service-address-{label}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&path).expect("temp dir is created");
    path
}

fn write_record(dir: &Path, url: &str) -> PathBuf {
    let record = dir.join("gateway-url");
    fs::write(&record, format!("{url}\n")).expect("record is written");
    record
}

/// A stand-in gateway: it answers registration only at the address it is
/// currently living at, which the test moves by rewriting the record.
#[derive(Clone)]
struct Gateway {
    live: PathBuf,
}

impl Gateway {
    /// The address the gateway answers at right now, straight from its record.
    fn current(&self) -> String {
        read_record(&self.live).expect("gateway record is readable")
    }

    /// `POST /register` against `address`: 201 when it is where the gateway
    /// actually lives, 000 (connection refused) otherwise.
    fn register(&self, address: &str) -> RegistrationOutcome {
        if address == self.current() {
            RegistrationOutcome::from_status(201)
        } else {
            RegistrationOutcome::from_status(0)
        }
    }
}

/// AC1: the address is read from the authoritative record at use time. A
/// launch argument holding the address that was correct *before* the move is
/// never preferred to a readable record.
#[test]
fn use_time_resolution_reads_the_record_not_the_launch_argument() {
    let dir = temp_dir("record-wins");
    let record = write_record(&dir, "http://node-17:8080");

    let mut resolver = AddressResolver::new(&record, Some("http://node-04:8080"));
    let resolution = resolver.resolve().expect("record is readable");

    assert_eq!(resolution.address, "http://node-17:8080");
    assert_eq!(resolution.origin, AddressOrigin::AuthoritativeRecord);
    assert_eq!(resolver.rereads(), 1);

    // The gateway moves; the next resolution sees it without a restart.
    write_record(&dir, "http://node-22:9001");
    let after = resolver.resolve().expect("record is still readable");
    assert_eq!(
        after.address, "http://node-17:8080",
        "a cached address is reused until something fails"
    );
    resolver.record_failure();
    let reread = resolver.resolve().expect("record is still readable");
    assert_eq!(reread.address, "http://node-22:9001");
    assert_eq!(reread.generation, 2, "the re-read is a new generation");
    assert_eq!(resolver.rereads(), 2);

    let _ = fs::remove_dir_all(&dir);
}

/// AC1: an empty launch default plus an unreadable record is an error naming
/// the record, never an empty address handed to a request.
#[test]
fn missing_record_and_empty_launch_arg_fail_closed() {
    let dir = temp_dir("no-address");
    let record = dir.join("gateway-url");

    let mut resolver = AddressResolver::new(&record, Some(""));
    let err = resolver.resolve().expect_err("nothing supplies an address");
    assert!(err
        .to_string()
        .contains("authoritative address record is missing"));
    assert_eq!(resolver.cached(), None);

    let _ = fs::remove_dir_all(&dir);
}

/// AC1: the record's shape is checked — a blank or non-URL record is a record
/// failure, not an address.
#[test]
fn record_shape_is_validated() {
    assert_eq!(
        parse_address("  http://node-17:8080/  "),
        Some("http://node-17:8080".to_string())
    );
    assert_eq!(parse_address(""), None);
    assert_eq!(parse_address("node-17:8080"), None);

    let dir = temp_dir("record-shape");
    let blank = dir.join("blank");
    fs::write(&blank, "\n\n").expect("write");
    assert_eq!(read_record(&blank), Err(RecordError::Empty));

    let junk = dir.join("junk");
    fs::write(&junk, "gateway moved, no url here\n").expect("write");
    assert!(matches!(read_record(&junk), Err(RecordError::Malformed(_))));

    let _ = fs::remove_dir_all(&dir);
}

/// AC5 (regression): relocate the gateway, start a worker, and it registers —
/// with nothing else restarted.
#[test]
fn worker_started_after_gateway_relocation_registers() {
    let dir = temp_dir("relocation");
    let record = dir.join("gateway-url");
    write_record(&dir, "http://node-04:8080");
    let gateway = Gateway {
        live: record.clone(),
    };
    let old_address = gateway.current();

    // The gateway moves to a new node and port. It rewrites its own record;
    // nothing else is restarted.
    write_record(&dir, "http://node-19:9100");
    assert_ne!(gateway.current(), old_address);

    // A worker launched after the move, carrying the address that was correct
    // before it (the launcher's stale value), registers successfully because
    // the address is resolved from the record at use time.
    let mut resolver = AddressResolver::new(&record, Some(&old_address));
    let outcome = resolver.register(|address| gateway.register(address));
    assert_eq!(outcome, RegistrationOutcome::Registered);
    assert_eq!(
        resolver.cached(),
        Some("http://node-19:9100"),
        "the address the worker registered against is the record's value"
    );

    let health = component_health(true, outcome, 1);
    assert_eq!(health, ComponentHealth::Healthy);

    let _ = fs::remove_dir_all(&dir);
}

/// AC1 + AC2: a long-lived worker whose cached address stops working re-reads
/// the record on the next attempt and registers at the new address — the same
/// process, no restart.
#[test]
fn long_lived_worker_follows_a_relocated_gateway_without_restart() {
    let dir = temp_dir("long-lived");
    let record = dir.join("gateway-url");
    write_record(&dir, "http://node-04:8080");
    let gateway = Gateway {
        live: record.clone(),
    };

    let mut resolver = AddressResolver::new(&record, Some("http://node-04:8080"));
    assert_eq!(
        resolver.register(|address| gateway.register(address)),
        RegistrationOutcome::Registered
    );
    assert_eq!(resolver.rereads(), 1);

    // The gateway moves while the worker is running. Its cache still holds the
    // old address, so the next attempt fails 000 and invalidates the cache.
    write_record(&dir, "http://node-27:9300");
    let stale = resolver.register(|address| gateway.register(address));
    assert_eq!(stale, RegistrationOutcome::Unreachable);
    assert!(stale.address_may_be_stale());
    assert_eq!(resolver.cached(), None, "failure invalidates the cache");

    // The next use re-reads the record and registers. No restart happened.
    let retry = resolver.register(|address| gateway.register(address));
    assert_eq!(retry, RegistrationOutcome::Registered);
    assert_eq!(
        resolver.rereads(),
        2,
        "one read before the move, one after the failure"
    );

    let _ = fs::remove_dir_all(&dir);
}

/// AC2: registration failure is a distinct unhealthy state. A worker that
/// serves but never joined the pool is degraded, whatever else it can do.
#[test]
fn serving_without_pool_membership_is_degraded() {
    // The arc from the worker logs: 401 reachable/auth, 201 working, 000 moved.
    assert_eq!(
        RegistrationOutcome::from_status(401),
        RegistrationOutcome::Rejected { status: 401 }
    );
    assert_eq!(
        RegistrationOutcome::from_status(201),
        RegistrationOutcome::Registered
    );
    assert_eq!(
        RegistrationOutcome::from_status(0),
        RegistrationOutcome::Unreachable
    );
    assert!(!RegistrationOutcome::Rejected { status: 401 }.address_may_be_stale());

    let degraded = component_health(true, RegistrationOutcome::Unreachable, 2);
    assert_eq!(
        degraded,
        ComponentHealth::DegradedNotInPool {
            attempts: 2,
            last: RegistrationOutcome::Unreachable,
        }
    );
    assert!(!degraded.is_healthy());
    assert!(degraded.to_string().contains("not in pool"));

    // A serving worker is not healthy on the strength of serving alone, and a
    // non-serving one is down regardless of registration.
    assert!(!component_health(true, RegistrationOutcome::Rejected { status: 401 }, 1).is_healthy());
    assert_eq!(
        component_health(false, RegistrationOutcome::Registered, 1),
        ComponentHealth::Down
    );
}

/// AC3: a health check asserts the service responds, never that the scheduler
/// believes its job is running.
#[test]
fn scheduler_belief_is_not_health() {
    assert_eq!(
        service_health(&HealthEvidence::Responded { status: 200 }),
        ServiceHealth::Up
    );
    assert!(matches!(
        service_health(&HealthEvidence::NoResponse),
        ServiceHealth::Down { .. }
    ));

    // `squeue` says RUNNING, which was true throughout the incident. It is not
    // a verdict about the service in either direction.
    let verdict = service_health(&HealthEvidence::SchedulerJobState {
        state: "RUNNING".to_string(),
    });
    assert!(matches!(verdict, ServiceHealth::AssertedWrongThing { .. }));
    assert_ne!(verdict, ServiceHealth::Up);
    assert!(format!("{verdict:?}").contains("no service response observed"));
}

/// AC4: the reconciler reports pool size, and a sustained decline is flagged
/// before the pool reaches zero.
#[test]
fn reconciler_reports_pool_size_and_flags_sustained_decline() {
    // Ten-minute passes, one preemption at a time.
    let mut monitor = PoolMonitor::new(3);
    for (i, size) in [7usize, 6, 5, 4].iter().enumerate() {
        monitor.record(100 + i as u64, *size);
    }

    assert_eq!(monitor.size(), Some(4));
    assert_eq!(monitor.peak(), Some(7));
    assert!(monitor.decline_flagged());
    assert_eq!(monitor.trend(), PoolTrend::Draining { drop: 3, window: 3 });

    let line = monitor.reconcile_line(1, "nothing to do");
    assert!(
        line.contains("pool: 4 registered"),
        "the line must name the pool size: {line}"
    );
    assert!(
        line.contains("gateways: 1 up"),
        "gateways still reported: {line}"
    );
    assert!(line.contains("DRAINING"), "decline is flagged: {line}");

    // One preemption alone is not a sustained decline.
    let mut single = PoolMonitor::new(3);
    single.record(1, 7);
    single.record(2, 6);
    assert!(!single.decline_flagged());
    assert_eq!(single.trend(), PoolTrend::Stable);

    // A drained pool is its own verdict, naming the size it lost.
    let mut drained = PoolMonitor::new(3);
    for (i, size) in [7usize, 5, 2, 0].iter().enumerate() {
        drained.record(10 + i as u64, *size);
    }
    assert_eq!(drained.trend(), PoolTrend::Drained { was: 7 });
    assert!(drained
        .reconcile_line(1, "nothing to do")
        .contains("DRAINED"));

    // Growth is reported as growth, not as a decline.
    let mut growing = PoolMonitor::new(2);
    for (i, size) in [2usize, 4, 6].iter().enumerate() {
        growing.record(i as u64, *size);
    }
    assert_eq!(growing.trend(), PoolTrend::Growing { delta: 4 });
    assert!(!growing.decline_flagged());

    // No history is not "stable": it is the absence of an observation.
    let empty = PoolMonitor::new(3);
    assert_eq!(empty.trend(), PoolTrend::NoHistory);
    assert!(empty
        .reconcile_line(0, "nothing to do")
        .contains("pool: unknown"));
}
