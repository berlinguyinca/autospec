//! AAR spec section 12: the InferWeave capability contract and routing.

use autospec_core::aar::inferweave::{
    admit, classify_probe, route, serves_model, AdmissionVerdict, CapabilityRequest, DeadReason,
    LatencyPriority, LivenessProbe, NodeOffer, PoolAction, ProbeCheck, ProbeSignal, ProbeVerdict,
    RefusalReason, RegistrationProbe, SessionSeat,
};

fn node(node_id: &str, free_context: u64, decode_tps: f64) -> NodeOffer {
    NodeOffer {
        node_id: node_id.to_string(),
        served_models: vec!["qwen3.8-27b".to_string()],
        model_classes: vec!["coding-local".to_string()],
        free_context_tokens: free_context,
        total_context_tokens: 131_072,
        is_local: true,
        warm_prefix_cache_keys: Vec::new(),
        affinity_session_id: None,
        utilization: 0.3,
        queue_depth: 0,
        observed_prefill_tokens_per_second: 1_000.0,
        observed_decode_tokens_per_second: decode_tps,
        network_cost: 0.0,
        qos_share_remaining: 1.0,
        overloaded: false,
    }
}

fn request() -> CapabilityRequest {
    CapabilityRequest {
        model_class: "coding-local".to_string(),
        model_allowlist: vec!["qwen3.8-27b".to_string()],
        minimum_context_free: 24_000,
        ..CapabilityRequest::default()
    }
}

/// The rule the spec states outright: context is a filter, speed is a score.
#[test]
fn a_faster_node_without_enough_free_context_loses_to_a_slower_eligible_one() {
    let offers = [
        node("fast-but-full", 8_000, 200.0),
        node("slower-with-room", 40_000, 40.0),
    ];

    let decision = route(&request(), &offers);

    assert_eq!(decision.selected.as_deref(), Some("slower-with-room"));
    assert!(decision
        .rejected
        .iter()
        .any(|(node_id, reason)| node_id == "fast-but-full" && reason.contains("free context")));
}

#[test]
fn a_node_that_does_not_serve_the_required_class_is_not_routed_to() {
    let mut wrong_class = node("other-class", 100_000, 200.0);
    wrong_class.model_classes = vec!["vision-local".to_string()];

    let decision = route(&request(), &[wrong_class, node("right", 40_000, 40.0)]);

    assert_eq!(decision.selected.as_deref(), Some("right"));
    assert!(decision.rejected[0].1.contains("does not serve"));
}

#[test]
fn a_node_that_does_not_serve_an_allowlisted_model_is_rejected() {
    let mut other_model = node("other-model", 100_000, 300.0);
    other_model.served_models = vec!["some-other-model".to_string()];

    let decision = route(&request(), &[other_model, node("right", 40_000, 40.0)]);

    assert_eq!(decision.selected.as_deref(), Some("right"));
}

#[test]
fn an_overloaded_node_is_rejected() {
    let mut overloaded = node("overloaded", 100_000, 300.0);
    overloaded.overloaded = true;

    let decision = route(&request(), &[overloaded, node("healthy", 40_000, 40.0)]);

    assert_eq!(decision.selected.as_deref(), Some("healthy"));
    assert!(decision.rejected[0].1.contains("overload"));
}

#[test]
fn a_node_with_exhausted_fair_share_is_rejected() {
    let mut exhausted = node("exhausted", 100_000, 300.0);
    exhausted.qos_share_remaining = 0.0;

    let decision = route(&request(), &[exhausted, node("healthy", 40_000, 40.0)]);

    assert_eq!(decision.selected.as_deref(), Some("healthy"));
    assert!(decision.rejected[0].1.contains("fair-share"));
}

#[test]
fn session_affinity_outweighs_raw_speed() {
    let mut affine = node("affine", 40_000, 30.0);
    affine.affinity_session_id = Some("session-7".to_string());
    let request = CapabilityRequest {
        session_id: "session-7".to_string(),
        session_affinity: true,
        ..request()
    };

    let decision = route(&request, &[affine, node("faster", 40_000, 120.0)]);

    assert_eq!(decision.selected.as_deref(), Some("affine"));
    assert!(decision.candidates[0]
        .reasons
        .iter()
        .any(|reason| reason.contains("session affinity")));
}

#[test]
fn a_warm_prefix_cache_is_preferred_over_a_cold_node() {
    let mut warm = node("warm", 40_000, 40.0);
    warm.warm_prefix_cache_keys = vec!["prefix-abc".to_string()];
    let request = CapabilityRequest {
        prefix_cache_key: "prefix-abc".to_string(),
        ..request()
    };

    let decision = route(&request, &[node("cold", 40_000, 60.0), warm]);

    assert_eq!(decision.selected.as_deref(), Some("warm"));
    assert!(decision.candidates[0]
        .reasons
        .iter()
        .any(|reason| reason.contains("warm prefix cache")));
}

#[test]
fn a_local_node_is_preferred_when_prefer_local_is_set() {
    let mut remote = node("remote", 100_000, 80.0);
    remote.is_local = false;
    remote.network_cost = 0.8;

    let decision = route(&request(), &[remote, node("local", 40_000, 60.0)]);

    assert_eq!(decision.selected.as_deref(), Some("local"));
}

/// A seat is not a slot: its demand includes projected growth and KV, so a
/// node sized for the current prompt alone is not eligible.
#[test]
fn the_seat_demand_raises_the_free_context_requirement() {
    let request = CapabilityRequest {
        minimum_context_free: 10_000,
        seat: SessionSeat {
            current_context_tokens: 20_000,
            projected_growth_tokens: 16_000,
            kv_tokens: 4_000,
        },
        ..request()
    };

    assert_eq!(request.required_free_context(), 40_000);
    let decision = route(&request, &[node("too-small", 30_000, 100.0)]);

    assert!(!decision.is_routed());
    assert!(decision.rejected[0].1.contains("< required 40000"));
}

#[test]
fn no_eligible_node_reports_an_unrouted_decision_with_reasons() {
    let decision = route(&request(), &[node("small", 1_000, 100.0)]);

    assert!(!decision.is_routed());
    assert!(decision
        .rationale
        .iter()
        .any(|reason| reason.contains("no eligible node")));
}

#[test]
fn latency_priority_shifts_the_weight_toward_prefill() {
    let mut prefill_heavy = node("prefill-heavy", 40_000, 20.0);
    prefill_heavy.observed_prefill_tokens_per_second = 4_000.0;
    let mut decode_heavy = node("decode-heavy", 40_000, 200.0);
    decode_heavy.observed_prefill_tokens_per_second = 100.0;

    let latency = route(
        &CapabilityRequest {
            latency_priority: LatencyPriority::Latency,
            ..request()
        },
        &[prefill_heavy.clone(), decode_heavy.clone()],
    );
    let throughput = route(
        &CapabilityRequest {
            latency_priority: LatencyPriority::Throughput,
            ..request()
        },
        &[prefill_heavy, decode_heavy],
    );

    assert_eq!(latency.selected.as_deref(), Some("prefill-heavy"));
    assert_eq!(throughput.selected.as_deref(), Some("decode-heavy"));
}

#[test]
fn the_request_renders_the_specification_yaml_shape() {
    let request = CapabilityRequest {
        prefix_cache_key: "abc123".to_string(),
        ..request()
    };

    let yaml = request.to_yaml();

    assert!(yaml.contains("model_class: coding-local"));
    assert!(yaml.contains("model_allowlist: [qwen3.8-27b]"));
    assert!(yaml.contains("minimum_context_free: 24000"));
    assert!(yaml.contains("prefer_local: true"));
    assert!(yaml.contains("session_affinity: true"));
    assert!(yaml.contains("prefix_cache_key: \"abc123\""));
    assert!(yaml.contains("latency_priority: balanced"));
}

/// The worker with fewer live agent jobs wins over a busier equally capable
/// one, every time the selector is asked.
#[test]
fn fewer_live_agent_jobs_win_over_a_busier_worker() {
    let mut busy = node("busy", 40_000, 50.0);
    busy.queue_depth = 3;
    let idle = node("idle", 40_000, 50.0);
    let offers = [busy, idle];

    for _ in 0..16 {
        let decision = route(&request(), &offers);
        assert_eq!(decision.selected.as_deref(), Some("idle"));
    }
}

/// The bug this selector used to have: a constant tiebreak sent every
/// simultaneous dispatch to the same worker. Repeated calls over identical
/// workers must not be constant.
#[test]
fn repeated_routing_over_identical_workers_is_not_constant() {
    let offers = [node("w1", 40_000, 50.0), node("w2", 40_000, 50.0)];

    let mut chosen = std::collections::BTreeSet::new();
    for _ in 0..64 {
        let decision = route(&request(), &offers);
        chosen.insert(decision.selected.clone().expect("routed"));
    }

    assert!(
        chosen.len() > 1,
        "selection was constant across 64 calls: {chosen:?}"
    );
}

/// N simultaneous dispatches spread across N identical workers instead of
/// converging on one of them.
#[test]
fn simultaneous_dispatches_distribute_across_workers() {
    let offers = [
        node("w1", 40_000, 50.0),
        node("w2", 40_000, 50.0),
        node("w3", 40_000, 50.0),
        node("w4", 40_000, 50.0),
    ];

    let mut counts: std::collections::BTreeMap<String, u32> = std::collections::BTreeMap::new();
    for _ in 0..128 {
        let decision = route(&request(), &offers);
        *counts
            .entry(decision.selected.clone().expect("routed"))
            .or_insert(0) += 1;
    }

    assert_eq!(
        counts.len(),
        4,
        "every worker must receive at least one dispatch in 128: {counts:?}"
    );
}

#[test]
fn every_latency_priority_round_trips_through_its_string_form() {
    for priority in [
        LatencyPriority::Latency,
        LatencyPriority::Balanced,
        LatencyPriority::Throughput,
    ] {
        assert_eq!(LatencyPriority::parse(priority.as_str()), Some(priority));
    }
}

// Liveness probe contract: a probe must not evict a worker it merely failed
// to reach in time.

fn cheap_probe() -> LivenessProbe {
    LivenessProbe {
        interval_secs: 300,
        deadline_secs: 30,
        liveness: ProbeCheck::Health,
        verification: None,
        expected_identity: Some("worker-7".to_string()),
    }
}

/// AC1: a liveness check must not consume the very resource whose exhaustion
/// it is supposed to survive. A completion runs the model, so it is rejected
/// from the eviction-gating slot.
#[test]
fn a_completion_check_cannot_gate_eviction() {
    let probe = LivenessProbe {
        liveness: ProbeCheck::Completion {
            starvation_argument: "why not".to_string(),
        },
        ..cheap_probe()
    };

    let err = probe.validate().unwrap_err();

    assert!(err.contains("constant-cost"), "got: {err}");
}

/// AC2: a timeout is not a failure verdict. A deadline miss is inconclusive,
/// and an inconclusive verdict keeps the worker in the pool.
#[test]
fn a_deadline_miss_is_inconclusive_and_keeps_the_worker() {
    let probe = cheap_probe();

    let verdict = classify_probe(&probe, &ProbeSignal::DeadlineExceeded);

    assert_eq!(verdict, ProbeVerdict::Inconclusive);
    assert_eq!(verdict.pool_action(), PoolAction::Keep);
}

/// AC2: the failures that are a definitive statement about the worker still
/// evict: refused, reset, error status, and identity mismatch.
#[test]
fn every_definitive_failure_evicts() {
    let probe = cheap_probe();
    let definitive = [
        (ProbeSignal::Refused, "refused"),
        (ProbeSignal::Reset, "reset"),
        (ProbeSignal::ErrorStatus { status: 503 }, "error status"),
        (
            ProbeSignal::Live {
                identity: Some("worker-9".to_string()),
            },
            "identity mismatch",
        ),
    ];

    for (signal, what) in definitive {
        let verdict = classify_probe(&probe, &signal);
        assert_eq!(
            verdict.pool_action(),
            PoolAction::Evict,
            "{what} must evict, got {verdict:?}"
        );
        assert!(matches!(verdict, ProbeVerdict::Dead { .. }), "{what}");
    }
}

#[test]
fn an_alive_worker_that_reports_the_expected_identity_is_kept() {
    let probe = cheap_probe();

    let verdict = classify_probe(
        &probe,
        &ProbeSignal::Live {
            identity: Some("worker-7".to_string()),
        },
    );

    assert_eq!(verdict, ProbeVerdict::Alive);
    assert_eq!(verdict.pool_action(), PoolAction::Keep);
}

/// No identity reported is not an identity mismatch: the worker answered,
/// and only a contradiction is definitive.
#[test]
fn an_absent_identity_is_not_a_mismatch() {
    let probe = cheap_probe();

    let verdict = classify_probe(&probe, &ProbeSignal::Live { identity: None });

    assert_eq!(verdict, ProbeVerdict::Alive);
}

/// AC3: if a check needs to be costly, it is separated from the cheap check,
/// and only the cheap one gates eviction. A completion is legal in the
/// verification slot, and a deadline miss on the liveness check still keeps
/// the worker even while the costly check is configured.
#[test]
fn a_completion_check_is_legal_only_in_the_verification_slot() {
    let probe = LivenessProbe {
        verification: Some(ProbeCheck::Completion {
            starvation_argument:
                "on a busy worker the probe queues behind production traffic, so its \
                 deadline miss measures the queue, not the worker; it must not evict."
                    .to_string(),
        }),
        ..cheap_probe()
    };

    probe
        .validate()
        .expect("completion is legal as verification");

    let verdict = classify_probe(&probe, &ProbeSignal::DeadlineExceeded);
    assert_eq!(verdict.pool_action(), PoolAction::Keep);
}

/// AC4: a check whose cost scales with load must carry an explicit
/// starvation argument alongside it.
#[test]
fn a_load_scaling_check_requires_a_starvation_argument() {
    let without_argument = LivenessProbe {
        verification: Some(ProbeCheck::Completion {
            starvation_argument: "   ".to_string(),
        }),
        ..cheap_probe()
    };
    let err = without_argument.validate().unwrap_err();
    assert!(err.contains("starvation argument"), "got: {err}");

    let with_argument = LivenessProbe {
        verification: Some(ProbeCheck::Completion {
            starvation_argument: "cost scales with load; must not gate eviction".to_string(),
        }),
        ..cheap_probe()
    };
    with_argument
        .validate()
        .expect("a non-blank starvation argument is accepted");
}

#[test]
fn a_probe_needs_positive_interval_and_deadline() {
    let no_interval = LivenessProbe {
        interval_secs: 0,
        ..cheap_probe()
    };
    assert!(no_interval.validate().is_err());

    let no_deadline = LivenessProbe {
        deadline_secs: 0,
        ..cheap_probe()
    };
    assert!(no_deadline.validate().is_err());
}

#[test]
fn the_model_list_check_is_constant_cost_and_legal_in_the_liveness_slot() {
    let probe = LivenessProbe {
        liveness: ProbeCheck::ModelList,
        ..cheap_probe()
    };

    probe.validate().expect("model list is constant-cost");
}

// Admission: the same probe signal drives keep/evict in the liveness loop
// and admit/refuse at (re-)registration. The interpretation lives in the
// one shared predicate (`classify_probe`) both paths consult, so the two
// verdicts cannot disagree.

/// The empty/unknown case, written first: the liveness step timed out (busy
/// is not dead) but the identity step itself established nothing. "Nothing
/// observed" must not cost the worker nothing — an endpoint that also timed
/// out on `/v1/models` must not be admitted having proved nothing at all.
#[test]
fn admission_refuses_when_identity_itself_was_never_established() {
    let probe = cheap_probe();

    // The identity step timed out as well: no model list was observed.
    let both_timed_out = RegistrationProbe {
        identity: ProbeSignal::DeadlineExceeded,
        observed_models: Vec::new(),
        liveness: ProbeSignal::DeadlineExceeded,
    };
    assert_eq!(
        admit(&probe, &both_timed_out, "qwen3.8-27b"),
        AdmissionVerdict::Refused {
            reason: RefusalReason::IdentityNeverEstablished
        }
    );

    // The identity step answered but reported no identity and listed no
    // models: still nothing established.
    let answered_but_empty = RegistrationProbe {
        identity: ProbeSignal::Live { identity: None },
        observed_models: Vec::new(),
        liveness: ProbeSignal::DeadlineExceeded,
    };
    assert_eq!(
        admit(&probe, &answered_but_empty, "qwen3.8-27b"),
        AdmissionVerdict::Refused {
            reason: RefusalReason::IdentityNeverEstablished
        }
    );
}

/// The incident this rule exists for: the busiest worker's completion probe
/// times out behind production traffic. Its identity was established, so
/// admission proceeds and it re-registers instead of expiring at its TTL
/// into `no worker for model`.
///
/// And (issue #4411) the admission is *provisional*: the liveness step never
/// completed, so the worker was never measured and the success-path checks
/// never ran against it. The bypass is not an exit — it carries the
/// obligation to be measured within a bounded window.
#[test]
fn a_busy_worker_whose_identity_was_established_may_re_register() {
    let probe = cheap_probe();
    let busy = RegistrationProbe {
        identity: ProbeSignal::Live {
            identity: Some("worker-7".to_string()),
        },
        observed_models: vec!["qwen3.8-27b".to_string()],
        liveness: ProbeSignal::DeadlineExceeded,
    };

    let verdict = admit(&probe, &busy, "qwen3.8-27b");

    assert_eq!(verdict, AdmissionVerdict::AdmittedProvisionally);
    assert!(verdict.is_admitted());
    assert!(verdict.requires_measurement());
    // And the liveness loop reads the same timeout the same way.
    assert_eq!(
        classify_probe(&probe, &ProbeSignal::DeadlineExceeded).pool_action(),
        PoolAction::Keep
    );
}

/// A worker whose liveness step completed was measured: its admission is
/// not provisional and carries no obligation.
#[test]
fn a_measured_admission_is_not_provisional() {
    let probe = cheap_probe();
    let measured = RegistrationProbe {
        identity: ProbeSignal::Live {
            identity: Some("worker-7".to_string()),
        },
        observed_models: vec!["qwen3.8-27b".to_string()],
        liveness: ProbeSignal::Live {
            identity: Some("worker-7".to_string()),
        },
    };

    let verdict = admit(&probe, &measured, "qwen3.8-27b");

    assert_eq!(verdict, AdmissionVerdict::Admitted);
    assert!(verdict.is_admitted());
    assert!(!verdict.requires_measurement());
}

/// A definitive failure on either step — identity or liveness — refuses,
/// regardless of what the other step established.
#[test]
fn a_definitive_failure_on_either_step_refuses() {
    let probe = cheap_probe();
    let signals = [
        ProbeSignal::Refused,
        ProbeSignal::Reset,
        ProbeSignal::ErrorStatus { status: 503 },
    ];

    for signal in signals {
        let liveness_dead = RegistrationProbe {
            identity: ProbeSignal::Live {
                identity: Some("worker-7".to_string()),
            },
            observed_models: vec!["qwen3.8-27b".to_string()],
            liveness: signal.clone(),
        };
        assert!(
            matches!(
                admit(&probe, &liveness_dead, "qwen3.8-27b"),
                AdmissionVerdict::Refused {
                    reason: RefusalReason::WorkerDead { .. }
                }
            ),
            "{signal:?} on the liveness step must refuse"
        );

        let identity_dead = RegistrationProbe {
            identity: signal.clone(),
            observed_models: vec!["qwen3.8-27b".to_string()],
            liveness: ProbeSignal::Live {
                identity: Some("worker-7".to_string()),
            },
        };
        assert!(
            matches!(
                admit(&probe, &identity_dead, "qwen3.8-27b"),
                AdmissionVerdict::Refused {
                    reason: RefusalReason::WorkerDead { .. }
                }
            ),
            "{signal:?} on the identity step must refuse"
        );
    }
}

/// An identity the worker reports that contradicts the expected one is
/// definitive in admission, as in the liveness loop.
#[test]
fn an_unexpected_identity_refuses_admission() {
    let probe = cheap_probe();
    let impostor = RegistrationProbe {
        identity: ProbeSignal::Live {
            identity: Some("worker-9".to_string()),
        },
        observed_models: vec!["qwen3.8-27b".to_string()],
        liveness: ProbeSignal::Live {
            identity: Some("worker-7".to_string()),
        },
    };

    assert_eq!(
        admit(&probe, &impostor, "qwen3.8-27b"),
        AdmissionVerdict::Refused {
            reason: RefusalReason::WorkerDead {
                reason: DeadReason::IdentityMismatch
            }
        }
    );
}

/// A worker whose observed model list lacks the requested model is not
/// admitted to serve it.
#[test]
fn a_worker_that_does_not_serve_the_requested_model_is_refused() {
    let probe = cheap_probe();
    let other = RegistrationProbe {
        identity: ProbeSignal::Live {
            identity: Some("worker-7".to_string()),
        },
        observed_models: vec!["some-other-model".to_string()],
        liveness: ProbeSignal::Live {
            identity: Some("worker-7".to_string()),
        },
    };

    assert_eq!(
        admit(&probe, &other, "qwen3.8-27b"),
        AdmissionVerdict::Refused {
            reason: RefusalReason::DoesNotServeModel
        }
    );
}

/// The permissive-default lesson, checked directly on every input class:
/// the identity predicate decides only from an observation.
#[test]
fn the_model_identity_check_decides_only_from_an_observation() {
    // Unknown — the list was never established — is never "anything
    // goes", not even for an unconstrained request.
    assert!(!serves_model(&[], ""));
    assert!(!serves_model(&[], "qwen3.8-27b"));
    // An observed list decides normally.
    assert!(serves_model(&["qwen3.8-27b".to_string()], ""));
    assert!(serves_model(&["qwen3.8-27b".to_string()], "qwen3.8-27b"));
    assert!(!serves_model(
        &["some-other-model".to_string()],
        "qwen3.8-27b"
    ));
}

/// The comparison that was never made before: for every signal class,
/// admission and the liveness loop agree. A signal that evicts refuses;
/// a signal that keeps admits, given an established identity that serves
/// the requested model.
#[test]
fn admission_and_the_liveness_loop_agree_on_every_signal() {
    let probe = cheap_probe();
    let signals = [
        ProbeSignal::Refused,
        ProbeSignal::Reset,
        ProbeSignal::ErrorStatus { status: 503 },
        ProbeSignal::Live {
            identity: Some("worker-7".to_string()),
        },
        ProbeSignal::Live {
            identity: Some("worker-9".to_string()),
        },
        ProbeSignal::Live { identity: None },
        ProbeSignal::DeadlineExceeded,
    ];

    for signal in &signals {
        let verdict = classify_probe(&probe, signal);
        let registration = RegistrationProbe {
            identity: signal.clone(),
            observed_models: vec!["qwen3.8-27b".to_string()],
            liveness: signal.clone(),
        };
        let admission = admit(&probe, &registration, "qwen3.8-27b");

        match verdict {
            ProbeVerdict::Dead { .. } => assert!(
                !admission.is_admitted(),
                "the pool evicts on {signal:?} but admission did not refuse"
            ),
            ProbeVerdict::Alive | ProbeVerdict::Inconclusive => assert!(
                admission.is_admitted(),
                "the pool keeps {signal:?} but admission refused: {admission:?}"
            ),
        }
    }
}
