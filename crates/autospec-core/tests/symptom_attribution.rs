//! Symptom attribution: which component emitted the symptom, decided
//! before any metric is read (issue #4247).
//!
//! The incident's numbers pin the fixtures: `llm.metabolomics.us` is
//! `nginx -> edge-gateway -> tunnel -> hive-gateway -> worker` — five
//! hops, four of which can emit a 503. The hive gateway reported 10
//! workers, all four models warm, `served_503=0`, and was off the
//! user's path from the first command. The edge gateway's log held
//! 374 failed registrations in three hours, all for one model.

use autospec_core::symptom_attribution::{
    assess, duplicate_services, judge, judge_probe, select_probe, Diagnosis, Evidence, HealthCheck,
    Hop, Instance, PathError, Probe, ProbeSelection, ProbeSide, ProbeVerdict, RequestPath,
    TrapVerdict,
};

/// The incident's request path: five hops, four of which can emit the
/// 503. The tunnel relays bytes; it does not synthesize responses.
fn incident_path() -> RequestPath {
    RequestPath::new(vec![
        Hop::new("nginx", true),
        Hop::new("edge-gateway", true),
        Hop::new("tunnel", false),
        Hop::new("hive-gateway", true),
        Hop::new("worker", true),
    ])
    .unwrap()
}

/// The stack's instances: two gateways, the same service. Only the
/// edge one is on the user's path.
fn incident_instances() -> Vec<Instance> {
    vec![
        Instance {
            name: "edge-gateway".into(),
            service: "inferweave-gateway".into(),
            on_user_path: true,
        },
        Instance {
            name: "hive-gateway".into(),
            service: "inferweave-gateway".into(),
            on_user_path: false,
        },
    ]
}

/// The probes that were available during the incident: the credentialed
/// component the operator had habit for, and the public URL.
fn incident_probes() -> Vec<Probe> {
    vec![
        Probe::new(ProbeSide::ComponentSide, "hive-gateway (ssh tunnel)"),
        Probe::new(
            ProbeSide::SymptomSide,
            "https://llm.metabolomics.us/health (authenticated)",
        ),
    ]
}

// --- Invariant 1: trace the request path before reading any metric ------

#[test]
fn the_incident_path_is_five_hops_and_four_of_them_can_emit_the_503() {
    let path = incident_path();
    assert_eq!(path.hops().len(), 5);
    assert_eq!(
        path.emitters(),
        vec!["nginx", "edge-gateway", "hive-gateway", "worker"]
    );
}

#[test]
fn an_empty_path_is_rejected_the_users_request_arrives_somewhere() {
    assert_eq!(RequestPath::new(vec![]), Err(PathError::Empty));
}

#[test]
fn a_repeated_hop_name_is_rejected() {
    let err = RequestPath::new(vec![
        Hop::new("edge-gateway", true),
        Hop::new("tunnel", false),
        Hop::new("edge-gateway", true),
    ])
    .unwrap_err();
    assert_eq!(
        err,
        PathError::DuplicateHop {
            name: "edge-gateway".into()
        }
    );
}

#[test]
fn emitters_are_reported_in_arrival_order() {
    // Reversed input order: the path order is the argument order, not
    // the hop names' alphabetical order.
    let path = RequestPath::new(vec![
        Hop::new("worker", true),
        Hop::new("nginx", true),
        Hop::new("tunnel", false),
    ])
    .unwrap();
    assert_eq!(path.emitters(), vec!["worker", "nginx"]);
}

// --- Invariant 2: zero errors proves only that the component is healthy -

#[test]
fn a_clean_hive_gateway_is_component_scoped_not_decisive() {
    // The incident: 10 workers, all four models warm, served_503=0 —
    // true, and useless, because the edge gateway is also on the path
    // and can emit the 503.
    let check = HealthCheck {
        component: "hive-gateway".into(),
        errors_observed: 0,
    };
    assert_eq!(assess(&check, &incident_path()), Evidence::ComponentScoped);
}

#[test]
fn a_clean_check_on_the_sole_emitter_is_decisive() {
    // A stack whose only possible emitter is the component checked:
    // zero errors on it rules the symptom out on the user's path.
    let path = RequestPath::new(vec![
        Hop::new("tunnel", false),
        Hop::new("hive-gateway", true),
    ])
    .unwrap();
    let check = HealthCheck {
        component: "hive-gateway".into(),
        errors_observed: 0,
    };
    assert_eq!(assess(&check, &path), Evidence::Decisive);
}

#[test]
fn a_clean_check_off_the_path_is_no_evidence() {
    // A component that user traffic never reaches: clean in every
    // direction, relevant in none.
    let check = HealthCheck {
        component: "staging-gateway".into(),
        errors_observed: 0,
    };
    assert_eq!(assess(&check, &incident_path()), Evidence::OffPath);
}

#[test]
fn errors_on_a_path_component_attribute_the_symptom() {
    // The edge gateway's log: 374 failed registrations in three hours.
    // The emitter is located; the diagnosis ends there.
    let check = HealthCheck {
        component: "edge-gateway".into(),
        errors_observed: 374,
    };
    assert_eq!(
        assess(&check, &incident_path()),
        Evidence::Attributed {
            component: "edge-gateway".into()
        }
    );
}

#[test]
fn errors_off_the_path_do_not_attribute_the_users_symptom() {
    // A component not on the user's path can be on fire; the user's
    // 503 did not come from there.
    let check = HealthCheck {
        component: "staging-gateway".into(),
        errors_observed: 374,
    };
    assert_eq!(assess(&check, &incident_path()), Evidence::OffPath);
}

#[test]
fn a_clean_check_on_a_non_emitting_path_hop_is_not_decisive() {
    // The tunnel is on the path but cannot synthesize a 503: a clean
    // tunnel says nothing about the hops that can.
    let check = HealthCheck {
        component: "tunnel".into(),
        errors_observed: 0,
    };
    assert_eq!(assess(&check, &incident_path()), Evidence::ComponentScoped);
}

// --- Invariant 3: two instances of the same service is a standing trap --

#[test]
fn two_gateways_are_a_standing_trap() {
    assert_eq!(
        duplicate_services(&incident_instances()),
        vec!["inferweave-gateway".to_string()]
    );
}

#[test]
fn a_single_instance_service_is_not_a_trap() {
    let instances = vec![
        Instance {
            name: "edge-gateway".into(),
            service: "inferweave-gateway".into(),
            on_user_path: true,
        },
        Instance {
            name: "worker-1".into(),
            service: "worker".into(),
            on_user_path: true,
        },
    ];
    assert!(duplicate_services(&instances).is_empty());
}

#[test]
fn a_service_only_claim_on_a_trap_service_is_unattributed() {
    // The runbook line that did not exist: "the gateway is healthy"
    // does not say which gateway.
    let diagnosis = Diagnosis {
        service: "inferweave-gateway".into(),
        instance: None,
    };
    assert_eq!(
        judge(&diagnosis, &incident_instances()),
        TrapVerdict::Unattributed {
            instances: vec!["edge-gateway".into(), "hive-gateway".into()]
        }
    );
}

#[test]
fn a_claim_that_names_the_edge_gateway_resolves_the_trap() {
    let diagnosis = Diagnosis {
        service: "inferweave-gateway".into(),
        instance: Some("edge-gateway".into()),
    };
    assert_eq!(
        judge(&diagnosis, &incident_instances()),
        TrapVerdict::Attributed {
            instance: "edge-gateway".into()
        }
    );
}

#[test]
fn a_claim_naming_an_instance_of_another_service_is_not_an_instance() {
    let diagnosis = Diagnosis {
        service: "inferweave-gateway".into(),
        instance: Some("worker-1".into()),
    };
    let instances = incident_instances();
    assert_eq!(
        judge(&diagnosis, &instances),
        TrapVerdict::NotAnInstance {
            instance: "worker-1".into()
        }
    );
}

#[test]
fn a_claim_about_a_service_not_in_the_stack_is_unknown() {
    let diagnosis = Diagnosis {
        service: "load-balancer".into(),
        instance: None,
    };
    assert_eq!(
        judge(&diagnosis, &incident_instances()),
        TrapVerdict::UnknownService {
            service: "load-balancer".into()
        }
    );
}

#[test]
fn a_single_instance_service_needs_no_instance_named() {
    let diagnosis = Diagnosis {
        service: "worker".into(),
        instance: None,
    };
    let instances = vec![Instance {
        name: "worker-1".into(),
        service: "worker".into(),
        on_user_path: true,
    }];
    assert_eq!(judge(&diagnosis, &instances), TrapVerdict::NoTrap);
}

// --- Invariant 4: prefer the symptom-side probe --------------------------

#[test]
fn select_prefers_the_public_probe_over_the_reachable_component() {
    // The component-side probe was listed first — habit. The selection
    // does not follow the listing.
    let selection = select_probe(&incident_probes());
    assert_eq!(
        selection,
        ProbeSelection::SymptomSide {
            target: "https://llm.metabolomics.us/health (authenticated)".into()
        }
    );
}

#[test]
fn select_with_no_symptom_side_probe_falls_back_to_component_side() {
    let probes = vec![
        Probe::new(ProbeSide::ComponentSide, "gateway-a"),
        Probe::new(ProbeSide::ComponentSide, "gateway-b"),
    ];
    assert_eq!(
        select_probe(&probes),
        ProbeSelection::ComponentSideOnly {
            target: "gateway-a".into()
        }
    );
}

#[test]
fn select_with_no_probes_is_none() {
    assert_eq!(select_probe(&[]), ProbeSelection::None);
}

#[test]
fn probing_the_component_when_the_public_url_was_available_is_the_incident_error() {
    // The operator had credentials for the hive gateway and a habit of
    // checking it. One authenticated request to the public URL would
    // have identified the failing hop in seconds.
    let available = incident_probes();
    let chosen = &available[0]; // the hive gateway, over the tunnel
    assert_eq!(
        judge_probe(chosen, &available),
        ProbeVerdict::ComponentSideWhenSymptomSideAvailable {
            chosen: "hive-gateway (ssh tunnel)".into(),
            missed: "https://llm.metabolomics.us/health (authenticated)".into()
        }
    );
}

#[test]
fn a_component_side_probe_is_correct_when_no_public_probe_exists() {
    // Nothing better was available: the component-side check is the
    // right call, and the judge does not retroactively blame it.
    let available = vec![Probe::new(ProbeSide::ComponentSide, "gateway-a")];
    assert_eq!(
        judge_probe(&available[0], &available),
        ProbeVerdict::CorrectSide
    );
}

#[test]
fn a_symptom_side_probe_is_always_the_correct_side() {
    let available = incident_probes();
    assert_eq!(
        judge_probe(&available[1], &available),
        ProbeVerdict::CorrectSide
    );
}

// --- The incident, end to end --------------------------------------------

#[test]
fn the_incident_hour_is_the_three_verdicts_together() {
    let path = incident_path();
    let instances = incident_instances();
    let probes = incident_probes();

    // The probe that ran: the reachable component, not the user's entry.
    assert!(matches!(
        judge_probe(&probes[0], &probes),
        ProbeVerdict::ComponentSideWhenSymptomSideAvailable { .. }
    ));

    // The check that answered "all healthy": component-scoped, never
    // decisive, because the checked gateway is not the sole emitter.
    let check = HealthCheck {
        component: "hive-gateway".into(),
        errors_observed: 0,
    };
    assert_eq!(assess(&check, &path), Evidence::ComponentScoped);

    // The trap that made "the gateway is healthy" a meaningless claim.
    let diagnosis = Diagnosis {
        service: "inferweave-gateway".into(),
        instance: None,
    };
    assert!(matches!(
        judge(&diagnosis, &instances),
        TrapVerdict::Unattributed { .. }
    ));
}
