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
//! * the reconciler reports pool size and flags a sustained decline;
//! * a merged fix is a deployed fix only when the running revision matches
//!   the expected tip, and a redeploy is refused until restart safety is
//!   tested (issue #4228).

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use autospec_core::service_address::{
    component_health, decide, drift, parse_address, parse_revision, read_record, reconcile_line,
    service_health, AddressOrigin, AddressResolver, ComponentHealth, DeployAction, HealthEvidence,
    PoolMonitor, PoolTrend, Precondition, PreconditionLedger, RecordError, RegistrationOutcome,
    RevisionDrift, RevisionError, ServiceHealth,
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

/// #4228 invariant 2: the service reports the revision it is running
/// (`branch @ sha`), and a report that cannot be read is "no revision",
/// never "current".
#[test]
fn service_reports_the_revision_it_is_running() {
    let main = parse_revision("main @ 1a2b3c4d").expect("a valid report parses");
    assert_eq!(main.branch, "main");
    assert_eq!(main.sha, "1a2b3c4d");
    assert_eq!(main.to_string(), "main @ 1a2b3c4d");

    // A full 40-digit sha, surrounding whitespace, and a 64-hex (sha-256
    // repository) sha all parse.
    assert!(parse_revision("  origin/main @ 1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b  ").is_ok());
    assert!(parse_revision(&format!("main @ {}", "ab".repeat(32))).is_ok());

    // Garbage reports are no answers, never answers: the service is unreported.
    assert_eq!(parse_revision("1a2b3c4d"), Err(RevisionError::NoSeparator));
    assert_eq!(
        parse_revision("no revision"),
        Err(RevisionError::NoSeparator)
    );
    assert_eq!(
        parse_revision("@ 1a2b3c4d"),
        Err(RevisionError::EmptyBranch)
    );
    assert_eq!(
        parse_revision("my branch @ 1a2b3c4d"),
        Err(RevisionError::EmptyBranch)
    );
    assert_eq!(parse_revision("main @ xyz"), Err(RevisionError::BadSha));
    assert_eq!(parse_revision("main @ 123"), Err(RevisionError::BadSha));
    assert_eq!(
        parse_revision("main @ 1a2b-3c4d"),
        Err(RevisionError::BadSha)
    );
    assert_eq!(
        parse_revision(&format!("main @ {}", "ab".repeat(33))),
        Err(RevisionError::BadSha)
    );
}

/// #4228 invariant 3: the reconciler compares the running revision against
/// the expected tip (origin/main) and reports the difference. The same commit
/// under a different branch name is in sync; a service that reports nothing is
/// unreported, not current.
#[test]
fn reconciler_reports_drift_against_the_expected_tip() {
    let expected = parse_revision("origin/main @ 1a2b3c4d").expect("valid");
    let same_commit_other_name = parse_revision("main @ 1a2b3c4d").expect("valid");
    let stale = parse_revision("main @ 0f9e8d7c").expect("valid");

    assert_eq!(
        drift(Some(&same_commit_other_name), &expected),
        RevisionDrift::Current,
        "the same commit is the same code, whatever it is called"
    );
    assert_eq!(
        drift(Some(&stale), &expected),
        RevisionDrift::Drifted {
            running: stale.clone(),
            expected: expected.clone()
        }
    );
    assert_eq!(drift(None, &expected), RevisionDrift::Unreported);
    assert!(drift(None, &expected).to_string().contains("unreported"));
}

/// #4228 invariants 1 and 4: "redeploy" is a decision the reconciler makes
/// when the revision differs — not something that happens to the wait — and
/// it is refused, naming the unverified preconditions, until restart safety is
/// tested: the build works, the preflight refuses to bind without auth, the
/// reconciler starts a replacement.
#[test]
fn redeploy_is_a_decision_not_a_wait() {
    let expected = parse_revision("origin/main @ 1a2b3c4d").expect("valid");
    let stale = parse_revision("main @ 0f9e8d7c").expect("valid");
    let current = parse_revision("main @ 1a2b3c4d").expect("valid");

    let mut ledger = PreconditionLedger::new();
    assert_eq!(
        ledger.missing(),
        vec![
            Precondition::Build,
            Precondition::RefusesBindWithoutAuth,
            Precondition::ReplacementStarts
        ]
    );
    assert!(!ledger.all_verified());

    // In sync: nothing to do, whatever the preconditions.
    assert_eq!(
        decide(Some(&current), &expected, &ledger),
        DeployAction::NothingToDo
    );

    // Unreported: never "nothing to do" and never "redeploy" — the deployment
    // cannot be verified either way.
    assert_eq!(decide(None, &expected, &ledger), DeployAction::Unknown);

    // Stale, nothing tested: refused, naming all three preconditions.
    assert_eq!(
        decide(Some(&stale), &expected, &ledger),
        DeployAction::Refused {
            missing: vec![
                Precondition::Build,
                Precondition::RefusesBindWithoutAuth,
                Precondition::ReplacementStarts
            ]
        }
    );

    // Stale, two of three tested: refused, naming the one that remains.
    ledger.record(Precondition::Build);
    ledger.record(Precondition::RefusesBindWithoutAuth);
    assert_eq!(
        decide(Some(&stale), &expected, &ledger),
        DeployAction::Refused {
            missing: vec![Precondition::ReplacementStarts]
        }
    );

    // Stale, all three tested: redeploy to the expected tip.
    ledger.record(Precondition::ReplacementStarts);
    assert!(ledger.all_verified());
    assert_eq!(
        decide(Some(&stale), &expected, &ledger),
        DeployAction::Redeploy {
            expected: expected.clone()
        }
    );
}

/// #4228 invariant 3: the reconciler line names the running revision next to
/// its verdict, so "nothing to do" can never be stated over stale or
/// unreported code. Inconsistent inputs fall to the unreported line.
#[test]
fn reconciler_line_never_says_nothing_to_do_over_stale_code() {
    let expected = parse_revision("origin/main @ 1a2b3c4d").expect("valid");
    let current = parse_revision("main @ 1a2b3c4d").expect("valid");
    let stale = parse_revision("main @ 0f9e8d7c").expect("valid");

    let mut verified = PreconditionLedger::new();
    verified.record(Precondition::Build);
    verified.record(Precondition::RefusesBindWithoutAuth);
    verified.record(Precondition::ReplacementStarts);

    // In sync: "nothing to do" is true, and the line names the revision that
    // makes it true.
    let in_sync = decide(Some(&current), &expected, &verified);
    let line = reconcile_line(Some(&current), &expected, &in_sync);
    assert!(line.contains("nothing to do"), "{line}");
    assert!(
        line.contains("1a2b3c4d"),
        "the line names the running revision: {line}"
    );

    // Stale: the line says drifted and names both revisions.
    let drifted = decide(Some(&stale), &expected, &verified);
    let line = reconcile_line(Some(&stale), &expected, &drifted);
    assert!(
        !line.contains("nothing to do"),
        "stale code is not nothing to do: {line}"
    );
    assert!(line.contains("DRIFTED"), "{line}");
    assert!(
        line.contains("0f9e8d7c") && line.contains("1a2b3c4d"),
        "the line names both revisions: {line}"
    );

    // Refused: the line names the unverified preconditions.
    let mut partial = PreconditionLedger::new();
    partial.record(Precondition::Build);
    let refused = decide(Some(&stale), &expected, &partial);
    let line = reconcile_line(Some(&stale), &expected, &refused);
    assert!(!line.contains("nothing to do"), "{line}");
    assert!(line.contains("refused"), "{line}");
    assert!(
        line.contains("preflight refuses to bind without auth"),
        "the remaining precondition is named: {line}"
    );

    // Unreported: the line says the deployment cannot be verified.
    let unknown = decide(None, &expected, &verified);
    let line = reconcile_line(None, &expected, &unknown);
    assert!(!line.contains("nothing to do"), "{line}");
    assert!(line.contains("unreported"), "{line}");

    // Inconsistent inputs (a redeploy decision with no reported revision)
    // fall to the unreported line: the line never overstates what was
    // verified.
    let bogus = reconcile_line(
        None,
        &expected,
        &DeployAction::Redeploy {
            expected: expected.clone(),
        },
    );
    assert!(bogus.contains("unreported"), "{bogus}");
}

/// #4228 postscript: the redeploy moved the gateway. The consumer that held
/// the address it captured at launch broke, exactly as it should; the consumer
/// that resolves the published record at use time followed it without a
/// restart.
#[test]
fn redeploy_moves_the_address_and_consumers_follow_the_record() {
    let dir = temp_dir("redeploy");
    let record = dir.join("gateway-url");
    write_record(&dir, "http://node-04:8080");
    let gateway = Gateway {
        live: record.clone(),
    };

    // A consumer that captured the address at launch and never re-reads.
    let captured = gateway.current();

    // The redeploy: the new build lands on a different node and port and the
    // gateway rewrites the record.
    write_record(&dir, "http://node-31:9400");

    assert_eq!(
        gateway.register(&captured),
        RegistrationOutcome::Unreachable,
        "the consumer holding the old address cannot reach the moved gateway"
    );

    // A consumer that resolves at use time follows the record — same process,
    // no restart.
    let mut resolver = AddressResolver::new(&record, Some(&captured));
    assert_eq!(
        resolver.register(|address| gateway.register(address)),
        RegistrationOutcome::Registered
    );
    assert_eq!(resolver.cached(), Some("http://node-31:9400"));

    let _ = fs::remove_dir_all(&dir);
}
