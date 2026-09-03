//! AAR spec section 12: the InferWeave capability contract and routing.

use autospec_core::aar::inferweave::{
    route, CapabilityRequest, LatencyPriority, NodeOffer, SessionSeat,
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

#[test]
fn routing_is_deterministic_when_scores_tie() {
    let offers = [node("b-node", 40_000, 50.0), node("a-node", 40_000, 50.0)];

    let first = route(&request(), &offers);
    let second = route(&request(), &offers);

    assert_eq!(first.selected, second.selected);
    assert_eq!(first.selected.as_deref(), Some("a-node"));
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
