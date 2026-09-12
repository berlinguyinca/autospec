//! Over-committed unified KV deadlocks the server (issue #4377).
//!
//! The regression tests run in the configuration the incident required:
//! slots=4 with a per-slot ceiling of the whole pool and six concurrent
//! large requests — reproduced 6x, and every check that ran was serial,
//! which is exactly why none of them saw the contention.

use autospec_core::kv_overcommit::{
    admit, fleet_audit, fundable_slots, latent, same_change, serial_check, AdmitResult, KvClass,
    KvConfig, WorkerKv,
};

/// The 50 GB KV pool, in MiB.
const POOL: u64 = 51_200;

fn cfg(slots: u32, per_slot_mib: u64, unified: bool) -> KvConfig {
    KvConfig::new(slots, per_slot_mib, POOL, unified).expect("valid configuration")
}

/// A large request: 31.7k tokens of KV, well under the whole pool.
const REQUEST: u64 = 20_000;

// ── The incident, row by row ────────────────────────────────────────────────

#[test]
fn four_slots_whole_pool_unified_deadlocks_six_concurrent_requests() {
    // Row 1: slots=4, per-slot ceiling = whole pool, unified: 000 x 6,
    // wedged, GPU 0%, 50 GB held.
    let c = cfg(4, POOL, true);
    assert_eq!(c.classify(), KvClass::OverCommitted);
    assert_eq!(
        admit(&c, REQUEST, 6),
        AdmitResult::Deadlocked {
            stuck: 4,
            queued: 2,
            held_mib: POOL, // the 50 GB held at GPU 0%
        }
    );
    // /health kept answering 200 throughout: the serial check passes on the
    // wedged configuration, which is why the fleet never flagged it.
    assert!(serial_check(&c));
    assert!(latent(&c));
}

#[test]
fn eight_slots_bounded_ceiling_not_unified_rejects_cleanly_and_stays_healthy() {
    // Row 2: slots=8, per-slot 32k ctx, not unified: 400 x 6 — a clean
    // rejection — healthy afterwards. A request that needs the full 262k
    // window exceeds the 32k per-slot ceiling and is refused, not wedged.
    let c = KvConfig::new(8, 4_096, POOL, false).expect("valid configuration");
    assert_eq!(c.classify(), KvClass::FairShare);
    assert_eq!(admit(&c, 32_768, 6), AdmitResult::Rejected { rejected: 6 });
    assert!(!latent(&c));
    // A request that fits the ceiling is served, and the server stays
    // healthy.
    assert_eq!(admit(&c, 4_096, 6), AdmitResult::Served { queued: 0 });
}

#[test]
fn one_slot_whole_pool_unified_serves_all_six_and_stays_healthy() {
    // Row 3 — the fix: slots=1, per-slot ceiling = whole pool, unified:
    // 200 x 6. The single slot serves all six serially, and still answers a
    // tiny generation afterwards.
    let c = cfg(1, POOL, true);
    assert_eq!(c.classify(), KvClass::SingleSlot);
    assert_eq!(admit(&c, REQUEST, 6), AdmitResult::Served { queued: 5 });
    // "The single-slot worker served all six and still answered a tiny
    // generation afterwards."
    assert_eq!(admit(&c, 1, 1), AdmitResult::Served { queued: 0 });
}

// ── Invariant 1: no safe middle setting ─────────────────────────────────────

#[test]
fn a_ceiling_above_fair_share_is_an_over_commitment_with_no_safe_middle() {
    // The fair share: every slot's ceiling is pool/slots.
    let fair = cfg(4, POOL / 4, true);
    assert_eq!(fair.classify(), KvClass::FairShare);
    assert_eq!(admit(&fair, POOL / 4, 4), AdmitResult::Served { queued: 0 });

    // One slot above the fair share, with more than one slot: over-commit.
    // There is no safe middle setting between a fair share and one slot.
    let middle = cfg(4, POOL / 4 + 1, true);
    assert_eq!(middle.classify(), KvClass::OverCommitted);
    assert_eq!(
        admit(&middle, POOL / 4 + 1, 4),
        AdmitResult::Deadlocked {
            stuck: 4,
            queued: 0,
            held_mib: POOL,
        }
    );

    // The same ceiling with one slot: safe — a wait, not a hang.
    let one = cfg(1, POOL / 4 + 1, true);
    assert_eq!(one.classify(), KvClass::SingleSlot);
    assert!(serial_check(&one));
}

#[test]
fn fundable_slots_is_parallel_one_when_the_ceiling_is_the_whole_pool() {
    // The fix: parallel = 1 whenever the per-slot ceiling is the whole pool.
    assert_eq!(fundable_slots(POOL, POOL), 1);
    // A fair share funds every slot.
    assert_eq!(fundable_slots(POOL, POOL / 4), 4);
    // A ceiling above the pool still funds one slot: a single request
    // physically draws at most the pool.
    assert_eq!(fundable_slots(POOL, POOL * 2), 1);
    // No ceiling bounds nothing.
    assert_eq!(fundable_slots(POOL, 0), 0);
}

#[test]
fn the_remediation_is_parallel_one_and_leaves_safe_configs_untouched() {
    let wedged = cfg(4, POOL, true);
    let fixed = wedged.remediated();
    // The remediated configuration IS row 3 of the reproduction table.
    assert_eq!(fixed, cfg(1, POOL, true));
    assert_eq!(fixed.classify(), KvClass::SingleSlot);
    assert_eq!(admit(&fixed, REQUEST, 6), AdmitResult::Served { queued: 5 });

    // Safe configurations are untouched.
    let fair = cfg(4, POOL / 4, true);
    assert_eq!(fair.remediated(), fair);
    let single = cfg(1, POOL, true);
    assert_eq!(single.remediated(), single);
}

// ── Invariant 2: a single request cannot reveal contention ─────────────────

#[test]
fn a_single_request_cannot_reveal_contention_but_concurrency_can() {
    let c = cfg(4, POOL, true);
    // Serial: one request at a time, green forever. This is the check that
    // ran on every worker in the incident and saw nothing.
    for _ in 0..6 {
        assert_eq!(admit(&c, REQUEST, 1), AdmitResult::Served { queued: 0 });
    }
    // The same six requests, concurrent: the deadlock.
    assert!(matches!(
        admit(&c, REQUEST, 6),
        AdmitResult::Deadlocked { .. }
    ));
    // The smoke test that passed: one small request.
    assert!(serial_check(&c));
}

// ── Invariant 3: a latent deadlock looks healthy until load arrives ────────

#[test]
fn a_latent_deadlock_is_indistinguishable_from_a_healthy_configuration() {
    let c = cfg(4, POOL, true);
    // Until load arrives: over-committed, yet passing every serial check.
    assert!(latent(&c));
    assert!(serial_check(&c));
    // ...and then the load the agents produce arrives.
    assert!(matches!(
        admit(&c, REQUEST, 6),
        AdmitResult::Deadlocked { .. }
    ));
    // Healthy configurations are not latent.
    assert!(!latent(&cfg(4, POOL / 4, true)));
    assert!(!latent(&cfg(1, POOL, true)));
}

// ── Invariant 4: check every component that took the same change ───────────

/// The fleet from the incident: every worker carried the over-committed
/// configuration (slots=4, full-pool ceiling, unified). flash-next failed
/// first because the agent fleet hits its endpoints directly with 40k-token
/// prompts, so it reached the condition soonest.
fn incident_fleet() -> Vec<WorkerKv> {
    let names: &[(&str, &str)] = &[
        ("gw-flash", "qwen3.8-flash-next"),
        ("gw-27b-1", "qwen3.8-27b"),
        ("gw-27b-2", "qwen3.8-27b"),
        ("gw-27b-3", "qwen3.8-27b"),
        ("gw-27b-4", "qwen3.8-27b"),
        ("gw-vision", "qwen3.8-27b-vision"),
    ];
    names
        .iter()
        .map(|(worker, model)| WorkerKv {
            worker: (*worker).to_string(),
            model: (*model).to_string(),
            cfg: cfg(4, POOL, true),
        })
        .collect()
}

#[test]
fn the_fleet_audit_names_every_worker_that_took_the_change_not_just_the_victim() {
    let fleet = incident_fleet();
    let report = fleet_audit(&fleet);
    assert_eq!(report.total, 6);
    // All six carried the fault, not just the one that failed first.
    assert_eq!(report.overcommitted.len(), 6);
    assert!(report.overcommitted.contains(&"gw-flash".to_string()));
    assert!(report.overcommitted.contains(&"gw-vision".to_string()));
    // The other five looked healthy — they had not yet taken enough
    // concurrent large requests — while carrying the same fault.
    assert_eq!(report.looking_healthy, 6);
    assert!(!report.clean());

    // The report line names the workers, and says the latent deadlocks are
    // passing the serial check.
    let line = report.line();
    assert!(line.contains("gw-flash"), "line was: {line}");
    assert!(line.contains("6 over-committed"), "line was: {line}");
    assert!(
        line.contains("6 still pass the serial check"),
        "line was: {line}"
    );

    // The victim's configuration is the fleet's configuration: same change,
    // same fault, in all six.
    let same = same_change(&fleet, "gw-flash");
    assert_eq!(same.len(), 6);
    assert!(same.iter().all(|w| w.cfg == cfg(4, POOL, true)));
    // An unknown victim names nothing.
    assert!(same_change(&fleet, "gw-none").is_empty());
}

#[test]
fn a_fleet_where_only_the_victim_is_over_committed_reports_only_the_victim() {
    // Control: the same change applied to one worker and a fair share to
    // the rest — the audit must not over-report.
    let mut fleet = incident_fleet();
    for w in fleet.iter_mut().filter(|w| w.worker != "gw-flash") {
        w.cfg = cfg(4, POOL / 4, true);
    }
    let report = fleet_audit(&fleet);
    assert_eq!(report.overcommitted, vec!["gw-flash".to_string()]);
    assert_eq!(report.looking_healthy, 1);
    let same = same_change(&fleet, "gw-flash");
    assert_eq!(same.len(), 1);
    assert_eq!(same[0].worker, "gw-flash");
}

#[test]
fn a_fair_share_fleet_reports_clean() {
    let fleet: Vec<WorkerKv> = (0..6)
        .map(|i| WorkerKv {
            worker: format!("gw-{i}"),
            model: "qwen3.8-27b".to_string(),
            cfg: cfg(4, POOL / 4, true),
        })
        .collect();
    let report = fleet_audit(&fleet);
    assert!(report.clean());
    assert_eq!(
        report.line(),
        "fleet: 6 worker(s), no over-committed KV configuration"
    );
}
