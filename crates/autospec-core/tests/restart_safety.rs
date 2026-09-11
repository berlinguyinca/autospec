//! Restart safety and dependent safety are different properties (issue
//! #4241).
//!
//! The incident: a gateway was restarted with every local check green. The
//! replacement came up healthy in ~90 s and the published address updated.
//! The outage came from the other side of the boundary: a second machine
//! kept an inbound SSH tunnel to the old host. The tunnel's process never
//! exited, so restart supervision never fired, and it forwarded to an address
//! that no longer existed for 2 days 23 hours while reporting
//! `active (running)`. The public endpoint lost all of the gateway's capacity
//! and answered `/healthz` 200 the whole time.
//!
//! The invariants, in the order the incident produced them:
//!
//! * the restart gate answers both "can this come back?" and "who depends
//!   on it?", and holds on the missing half (invariant 1);
//! * the dependent sweep searched for something, somewhere, and the
//!   enumeration is written down (invariant 2);
//! * cross-host dependents are recorded somewhere an operator of either side
//!   will see (invariant 3);
//! * a readiness report that cannot go red without capacity is a liveness
//!   probe wearing a readiness probe's name, and an active forwarder whose
//!   process never exits is invisible to restart supervision (invariant 4).

use autospec_core::restart_safety::{
    forwarder_assessment, health_is_truthful, restart_gate, Dependent, DependentSweep,
    ForwarderAssessment, ForwarderEvidence, HealthClaim, HealthVerdict, Locator, RestartVerdict,
};

/// The gateway's own host before the restart. The relocated service keeps its
/// name and gets a new host; the dependent's host is whichever machine it
/// lives on.
const SERVICE_HOST: &str = "hive-as-11-2-40";

/// The incident's sweep, done properly: grepped the published address file,
/// the hostname pattern, and the old host:port pair, on the cluster host and
/// the second machine, and wrote the enumeration down.
fn recorded_sweep() -> DependentSweep {
    DependentSweep {
        recorded_in: "runbook: gateway restart — dependent sweep".to_string(),
        locators: vec![
            Locator::AddressFile {
                path: "state/gateway-url".to_string(),
            },
            Locator::HostnamePattern {
                pattern: "hive-as-*".to_string(),
            },
            Locator::HostPort {
                host: "hive-as-11-2-40".to_string(),
                port: 20579,
            },
        ],
        hosts_searched: vec!["hive-as-11-2-40".to_string(), "fry".to_string()],
        dependents: vec![],
    }
}

// --- Invariant 1: the gate answers both questions -------------------------

#[test]
fn the_incident_state_is_not_proceed() {
    // Every local check passed: the build works, the preflight is clean, the
    // reconciler starts a replacement, the address file updates on restart.
    // `restart_safe` is therefore true. What was missing is the sweep, and
    // the gate must not proceed on the visible half alone.
    let verdict = restart_gate(true, None, SERVICE_HOST);
    assert_eq!(
        verdict,
        RestartVerdict::DependentsNotEnumerated,
        "{verdict:?}"
    );
    assert!(!verdict.proceeds(), "{verdict:?}");
    let line = verdict.line();
    assert!(line.contains("not enumerated"), "{line}");
}

#[test]
fn a_service_that_cannot_come_back_blocks_before_any_sweep() {
    // The locally visible half fails: the restart must not happen, with or
    // without a sweep.
    assert_eq!(
        restart_gate(false, None, SERVICE_HOST),
        RestartVerdict::NotRestartSafe
    );
    assert_eq!(
        restart_gate(false, Some(&recorded_sweep()), SERVICE_HOST),
        RestartVerdict::NotRestartSafe,
        "a good sweep does not save a restart that cannot come back"
    );
}

#[test]
fn a_complete_recorded_sweep_proceeds() {
    // Everything on the cluster reads the record and recovers by itself; no
    // dependents found off-cluster. Both questions answered: proceed.
    let verdict = restart_gate(true, Some(&recorded_sweep()), SERVICE_HOST);
    assert_eq!(verdict, RestartVerdict::Proceed, "{verdict:?}");
    assert!(verdict.proceeds(), "{verdict:?}");
}

#[test]
fn every_hold_is_a_distinct_state_with_a_distinct_line() {
    let holds: Vec<RestartVerdict> = vec![
        restart_gate(false, None, SERVICE_HOST),
        restart_gate(true, None, SERVICE_HOST),
        restart_gate(
            true,
            Some(&DependentSweep {
                recorded_in: "runbook: gateway restart".to_string(),
                locators: vec![Locator::AddressFile {
                    path: "state/gateway-url".to_string(),
                }],
                hosts_searched: vec!["fry".to_string()],
                dependents: vec![Dependent {
                    host: "fry".to_string(),
                    what: "inbound SSH tunnel".to_string(),
                    recorded_in: None,
                }],
            }),
            SERVICE_HOST,
        ),
        restart_gate(
            true,
            Some(&DependentSweep {
                recorded_in: String::new(),
                locators: vec![Locator::AddressFile {
                    path: "state/gateway-url".to_string(),
                }],
                hosts_searched: vec!["fry".to_string()],
                dependents: vec![],
            }),
            SERVICE_HOST,
        ),
    ];
    assert!(
        holds.iter().all(|v| !v.proceeds()),
        "every hold must block: {holds:?}"
    );
    // No two holds collapse: "cannot come back", "not enumerated", and
    // "enumerated but not recorded" are different reasons and different
    // follow-ups.
    for (i, a) in holds.iter().enumerate() {
        for b in holds.iter().skip(i + 1) {
            assert_ne!(a, b, "two holds collapsed: {a:?} == {b:?}");
            assert_ne!(a.line(), b.line(), "two hold lines identical");
        }
    }
}

#[test]
fn the_unrecorded_cross_host_hold_names_the_dependent() {
    let sweep = DependentSweep {
        recorded_in: "runbook: gateway restart".to_string(),
        locators: vec![Locator::HostPort {
            host: SERVICE_HOST.to_string(),
            port: 20579,
        }],
        hosts_searched: vec![SERVICE_HOST.to_string(), "fry".to_string()],
        dependents: vec![Dependent {
            host: "fry".to_string(),
            what: "inbound SSH tunnel re-registering the gateway's models on a 60 s timer"
                .to_string(),
            recorded_in: None,
        }],
    };
    let verdict = restart_gate(true, Some(&sweep), SERVICE_HOST);
    match &verdict {
        RestartVerdict::UnrecordedCrossHostDependents { unrecorded } => {
            assert_eq!(unrecorded.len(), 1, "{verdict:?}");
            assert!(unrecorded[0].contains("fry"), "{}", unrecorded[0]);
        }
        other => panic!("expected UnrecordedCrossHostDependents, got {other:?}"),
    }
    assert!(!verdict.proceeds(), "{verdict:?}");
    let line = verdict.line();
    assert!(line.contains("fry"), "{line}");
    assert!(line.contains("either side"), "{line}");
}

// --- Invariant 2: the enumeration, recorded --------------------------------

#[test]
fn a_sweep_that_searched_for_nothing_is_not_an_enumeration() {
    let sweep = DependentSweep {
        recorded_in: "runbook: gateway restart".to_string(),
        locators: vec![],
        hosts_searched: vec!["fry".to_string()],
        dependents: vec![],
    };
    assert!(!sweep.searched_anything(), "{sweep:?}");
    assert_eq!(
        restart_gate(true, Some(&sweep), SERVICE_HOST),
        RestartVerdict::DependentsNotEnumerated,
        "found nothing because searched for nothing"
    );
}

#[test]
fn a_sweep_that_covered_no_host_is_not_an_enumeration() {
    let sweep = DependentSweep {
        recorded_in: "runbook: gateway restart".to_string(),
        locators: vec![Locator::AddressFile {
            path: "state/gateway-url".to_string(),
        }],
        hosts_searched: vec![],
        dependents: vec![],
    };
    assert!(!sweep.searched_anything(), "{sweep:?}");
    assert_eq!(
        restart_gate(true, Some(&sweep), SERVICE_HOST),
        RestartVerdict::DependentsNotEnumerated,
        "a search that covered no host covered nothing"
    );
}

#[test]
fn an_enumeration_that_was_not_written_down_holds_the_gate() {
    let sweep = DependentSweep {
        recorded_in: String::new(),
        locators: vec![Locator::AddressFile {
            path: "state/gateway-url".to_string(),
        }],
        hosts_searched: vec![SERVICE_HOST.to_string(), "fry".to_string()],
        dependents: vec![],
    };
    assert!(!sweep.is_recorded(), "{sweep:?}");
    assert_eq!(
        restart_gate(true, Some(&sweep), SERVICE_HOST),
        RestartVerdict::EnumerationNotRecorded
    );
    // Whitespace-only is the same as empty: it was not written down.
    let blank = DependentSweep {
        recorded_in: "   ".to_string(),
        ..sweep
    };
    assert!(!blank.is_recorded(), "{blank:?}");
}

#[test]
fn the_locators_carry_their_grep_terms() {
    assert_eq!(
        Locator::AddressFile {
            path: "state/gateway-url".to_string(),
        }
        .grep_term(),
        "state/gateway-url"
    );
    assert_eq!(
        Locator::HostnamePattern {
            pattern: "hive-as-*".to_string(),
        }
        .grep_term(),
        "hive-as-*"
    );
    assert_eq!(
        Locator::HostPort {
            host: "hive-as-11-2-40".to_string(),
            port: 20579,
        }
        .grep_term(),
        "hive-as-11-2-40:20579"
    );
}

// --- Invariant 3: cross-host dependents are the ones you forget ------------

#[test]
fn a_cross_host_dependent_with_a_record_does_not_hold() {
    let sweep = DependentSweep {
        recorded_in: "runbook: gateway restart".to_string(),
        locators: vec![Locator::HostPort {
            host: SERVICE_HOST.to_string(),
            port: 20579,
        }],
        hosts_searched: vec![SERVICE_HOST.to_string(), "fry".to_string()],
        dependents: vec![Dependent {
            host: "fry".to_string(),
            what: "inbound SSH tunnel".to_string(),
            recorded_in: Some("fry deploy note + gateway runbook".to_string()),
        }],
    };
    assert!(
        sweep.unrecorded_cross_host(SERVICE_HOST).is_empty(),
        "{sweep:?}"
    );
    assert_eq!(
        restart_gate(true, Some(&sweep), SERVICE_HOST),
        RestartVerdict::Proceed,
        "a cross-host dependent written down where either side's operator sees it is accounted for"
    );
}

#[test]
fn an_unrecorded_local_dependent_does_not_hold() {
    // Local dependents read the record and recover by themselves; the
    // invariant is about the boundary a local grep does not cross.
    let sweep = DependentSweep {
        recorded_in: "runbook: gateway restart".to_string(),
        locators: vec![Locator::AddressFile {
            path: "state/gateway-url".to_string(),
        }],
        hosts_searched: vec![SERVICE_HOST.to_string()],
        dependents: vec![Dependent {
            host: SERVICE_HOST.to_string(),
            what: "cluster workers reading state/gateway-url".to_string(),
            recorded_in: None,
        }],
    };
    assert!(
        sweep.unrecorded_cross_host(SERVICE_HOST).is_empty(),
        "{sweep:?}"
    );
    assert_eq!(
        restart_gate(true, Some(&sweep), SERVICE_HOST),
        RestartVerdict::Proceed
    );
}

#[test]
fn the_relocated_host_is_a_different_host() {
    // The gateway moved hive-as-11-2-40 -> hive-as-11-3-63. A dependent on
    // the old host's successor is cross-host relative to the pre-restart
    // host: exact string comparison, no prefix or pattern matching.
    let sweep = DependentSweep {
        recorded_in: "runbook: gateway restart".to_string(),
        locators: vec![],
        hosts_searched: vec!["hive-as-11-2-40".to_string(), "hive-as-11-3-63".to_string()],
        dependents: vec![Dependent {
            host: "hive-as-11-3-63".to_string(),
            what: "replacement gateway".to_string(),
            recorded_in: None,
        }],
    };
    // The dependent's host differs from the service host it was swept
    // against, so it is cross-host — even though the two names share a
    // prefix.
    assert_eq!(
        sweep.unrecorded_cross_host("hive-as-11-2-40"),
        vec!["hive-as-11-3-63: replacement gateway".to_string()],
        "a shared name prefix is not the same host"
    );
}

// --- Invariant 4: "the service is up" is not "the service is serving" ------

#[test]
fn a_readiness_report_green_without_capacity_is_the_incident() {
    // The public endpoint lost all of the gateway's capacity and answered
    // /healthz 200 for 2 days 23 hours.
    let verdict = health_is_truthful(HealthClaim::Readiness, true, false);
    assert_eq!(
        verdict,
        HealthVerdict::LivenessWearingReadiness,
        "{verdict:?}"
    );
    assert!(verdict.is_dishonest(), "{verdict:?}");
}

#[test]
fn a_readiness_report_is_honest_when_capacity_matches() {
    assert_eq!(
        health_is_truthful(HealthClaim::Readiness, true, true),
        HealthVerdict::Honest,
        "green with capacity is the point of a readiness probe"
    );
    assert_eq!(
        health_is_truthful(HealthClaim::Readiness, false, false),
        HealthVerdict::Honest,
        "a red report is never a lie"
    );
    assert_eq!(
        health_is_truthful(HealthClaim::Readiness, false, true),
        HealthVerdict::Honest,
        "red with capacity is conservative, not dishonest"
    );
}

#[test]
fn a_liveness_report_is_truthful_but_not_evidence_of_serving() {
    // The tunnel answered "healthy" about its own process the whole time —
    // and was forwarding nowhere. A liveness claim that is green without
    // capacity is honest about liveness; the defect is treating it as
    // readiness, which is exactly what the endpoint's name invited.
    let verdict = health_is_truthful(HealthClaim::Liveness, true, false);
    assert_eq!(verdict, HealthVerdict::Honest, "{verdict:?}");
    assert!(!verdict.is_dishonest(), "{verdict:?}");
}

#[test]
fn the_incidents_tunnel_is_a_stale_forward() {
    // active (running), process alive, remote address gone.
    let evidence = ForwarderEvidence {
        unit_active: true,
        process_exited: false,
        remote_reachable: false,
    };
    let assessment = forwarder_assessment(&evidence);
    assert_eq!(
        assessment,
        ForwarderAssessment::StaleForward,
        "{assessment:?}"
    );
    assert!(
        !assessment.supervision_can_repair(),
        "Restart=always never fires: the process never exits: {assessment:?}"
    );
    let line = assessment.line();
    assert!(line.contains("never"), "{line}");
    assert!(line.contains("active is not serving"), "{line}");
}

#[test]
fn a_live_remote_is_serving() {
    let evidence = ForwarderEvidence {
        unit_active: true,
        process_exited: false,
        remote_reachable: true,
    };
    let assessment = forwarder_assessment(&evidence);
    assert_eq!(assessment, ForwarderAssessment::Serving, "{assessment:?}");
    assert!(assessment.supervision_can_repair(), "{assessment:?}");
}

#[test]
fn an_exited_process_is_restartable() {
    // The remote died AND the forwarder's process exited (e.g. SSH gave up):
    // now supervision has something to fire on.
    let evidence = ForwarderEvidence {
        unit_active: false,
        process_exited: true,
        remote_reachable: false,
    };
    let assessment = forwarder_assessment(&evidence);
    assert_eq!(
        assessment,
        ForwarderAssessment::Restartable,
        "{assessment:?}"
    );
    assert!(assessment.supervision_can_repair(), "{assessment:?}");
}

#[test]
fn a_down_unit_is_restartable() {
    let evidence = ForwarderEvidence {
        unit_active: false,
        process_exited: false,
        remote_reachable: false,
    };
    let assessment = forwarder_assessment(&evidence);
    assert_eq!(
        assessment,
        ForwarderAssessment::Restartable,
        "{assessment:?}"
    );
    assert!(assessment.supervision_can_repair(), "{assessment:?}");
}

#[test]
fn stale_and_restartable_are_distinct_blocks() {
    // One is "supervision may act" and the other is "supervision is
    // impotent": collapsing them is the fold that hid the incident for two
    // days.
    let stale = forwarder_assessment(&ForwarderEvidence {
        unit_active: true,
        process_exited: false,
        remote_reachable: false,
    });
    let restartable = forwarder_assessment(&ForwarderEvidence {
        unit_active: false,
        process_exited: true,
        remote_reachable: false,
    });
    assert_ne!(stale, restartable, "{stale:?} == {restartable:?}");
    assert_ne!(stale.line(), restartable.line());
}

// --- Serde: the states serialize to distinct, stable forms -----------------

#[test]
fn the_verdicts_serialize_to_distinct_stable_forms() {
    let cases = vec![
        (RestartVerdict::Proceed, "\"proceed\""),
        (RestartVerdict::NotRestartSafe, "\"not_restart_safe\""),
        (
            RestartVerdict::DependentsNotEnumerated,
            "\"dependents_not_enumerated\"",
        ),
        (
            RestartVerdict::EnumerationNotRecorded,
            "\"enumeration_not_recorded\"",
        ),
        (
            RestartVerdict::UnrecordedCrossHostDependents {
                unrecorded: vec!["fry: inbound SSH tunnel".to_string()],
            },
            "{\"unrecorded_cross_host_dependents\":{\"unrecorded\":[\"fry: inbound SSH tunnel\"]}}",
        ),
    ];
    for (verdict, json) in cases {
        let got = serde_json::to_string(&verdict).expect("serialize");
        assert_eq!(got, json, "{verdict:?}");
        let back: RestartVerdict = serde_json::from_str(&got).expect("deserialize");
        assert_eq!(back, verdict, "{verdict:?}");
    }
}

#[test]
fn the_sweep_round_trips_with_missing_records_omitted() {
    let sweep = DependentSweep {
        recorded_in: "runbook: gateway restart".to_string(),
        locators: vec![Locator::HostPort {
            host: SERVICE_HOST.to_string(),
            port: 20579,
        }],
        hosts_searched: vec!["fry".to_string()],
        dependents: vec![
            Dependent {
                host: "fry".to_string(),
                what: "inbound SSH tunnel".to_string(),
                recorded_in: None,
            },
            Dependent {
                host: "hive-as-11-2-40".to_string(),
                what: "cluster workers".to_string(),
                recorded_in: Some("runbook".to_string()),
            },
        ],
    };
    let got = serde_json::to_string(&sweep).expect("serialize");
    let back: DependentSweep = serde_json::from_str(&got).expect("deserialize");
    assert_eq!(back, sweep, "{got}");
    // The unrecorded dependent's key is omitted, not null: `None` means
    // "not written down", and the round trip must preserve that.
    assert!(!got.contains("null"), "{got}");
}

#[test]
fn the_health_and_forwarder_states_round_trip() {
    let health_cases = [
        (HealthVerdict::Honest, "\"honest\""),
        (
            HealthVerdict::LivenessWearingReadiness,
            "\"liveness_wearing_readiness\"",
        ),
    ];
    for (verdict, json) in health_cases {
        let got = serde_json::to_string(&verdict).expect("serialize");
        assert_eq!(got, json, "{verdict:?}");
        let back: HealthVerdict = serde_json::from_str(&got).expect("deserialize");
        assert_eq!(back, verdict, "{verdict:?}");
    }

    let forward_cases = [
        (ForwarderAssessment::Serving, "\"serving\""),
        (ForwarderAssessment::StaleForward, "\"stale_forward\""),
        (ForwarderAssessment::Restartable, "\"restartable\""),
    ];
    for (assessment, json) in forward_cases {
        let got = serde_json::to_string(&assessment).expect("serialize");
        assert_eq!(got, json, "{assessment:?}");
        let back: ForwarderAssessment = serde_json::from_str(&got).expect("deserialize");
        assert_eq!(back, assessment, "{assessment:?}");
    }
}
