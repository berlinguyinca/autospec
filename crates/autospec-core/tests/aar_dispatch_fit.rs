//! Integration tests for `aar::dispatch_fit` (issue #3694): context-class
//! fit at dispatch time. The card's declared pack must be compared against
//! the endpoint's slot context before dispatch; when nothing fits the issue
//! is held, not dispatched truncated, and an under-provisioned run does not
//! count as an attempt.

use autospec_core::aar::dispatch_fit::{
    dispatch, parse_context_class, redispatch, status_json_with_grant, ContextGrant,
    DispatchVerdict, Endpoint, NO_ENDPOINT_LARGE_ENOUGH,
};

/// The exact card line from the issue.
const CARD_LINE: &str = "context class `C75` (tier `tier-integration`, **65\u{2013}82K pack**)";

/// A 256K llama.cpp endpoint with `--parallel <slots>` slots and the
/// issue's `max_tokens=16384` output reservation.
fn endpoint(name: &str, slots: u32) -> Endpoint {
    Endpoint {
        name: name.to_string(),
        context_window_tokens: 262_144,
        parallel_slots: slots,
        output_reserve_tokens: 16_384,
    }
}

fn fleet() -> Vec<Endpoint> {
    vec![
        endpoint("8-slot", 8),
        endpoint("4-slot", 4),
        endpoint("2-slot", 2),
    ]
}

fn grant(verdict: &DispatchVerdict) -> &ContextGrant {
    match verdict {
        DispatchVerdict::Grant(g) => &g.record,
        DispatchVerdict::Hold { code, .. } => panic!("expected Grant, held with {code}"),
    }
}

fn held_code(verdict: &DispatchVerdict) -> &'static str {
    match verdict {
        DispatchVerdict::Hold { code, .. } => *code,
        DispatchVerdict::Grant(g) => panic!("expected Hold, granted {}", g.endpoint.name),
    }
}

#[test]
fn card_line_parses_name_tier_and_pack_range() {
    let class = parse_context_class(CARD_LINE).expect("the card line from the issue parses");
    assert_eq!(class.name, "C75");
    assert_eq!(class.tier.as_deref(), Some("tier-integration"));
    assert_eq!(class.pack_min_tokens, 65_000);
    assert_eq!(class.pack_max_tokens, 82_000);
    assert_eq!(class.floor_tokens(), 65_000);
}

#[test]
fn parsing_is_lenient_about_markdown_and_strict_about_numbers() {
    // Plain text, hyphen instead of en-dash, no backticks or bold.
    let class = parse_context_class("Context Class C75 (tier tier-integration, 65-82K pack)")
        .expect("plain-text card parses");
    assert_eq!(class.name, "C75");
    assert_eq!(class.tier.as_deref(), Some("tier-integration"));
    assert_eq!(
        (class.pack_min_tokens, class.pack_max_tokens),
        (65_000, 82_000)
    );

    // No class-shaped token: refuse to guess.
    assert_eq!(parse_context_class("no context declared here"), None);
    // Class but no pack range: refuse to guess the floor.
    assert_eq!(
        parse_context_class("context class C75 (pack unspecified)"),
        None
    );
    // Class-shaped token glued to letters is a class, not a pack size.
    assert_eq!(
        parse_context_class("class C75, no pack numbers at all"),
        None
    );
}

#[test]
fn a_65k_card_is_never_dispatched_to_a_32k_slot() {
    let class = parse_context_class(CARD_LINE).unwrap();
    let verdict = dispatch(&class, &[endpoint("8-slot", 8)]);
    assert_eq!(held_code(&verdict), NO_ENDPOINT_LARGE_ENOUGH);
}

#[test]
fn dispatch_prefers_the_largest_context_endpoint_regardless_of_order() {
    let class = parse_context_class(CARD_LINE).unwrap();
    let fleet = fleet();

    let forward = dispatch(&class, &fleet);
    let backward = dispatch(&class, &fleet.iter().rev().cloned().collect::<Vec<_>>());

    for verdict in [forward, backward] {
        let g = grant(&verdict);
        assert_eq!(g.endpoint, "2-slot");
        assert_eq!(g.granted_context_tokens, 131_072);
    }
}

#[test]
fn a_65k_floor_fits_a_65536_slot() {
    let class = parse_context_class(CARD_LINE).unwrap();
    let verdict = dispatch(&class, &[endpoint("4-slot", 4)]);
    let g = grant(&verdict);
    assert_eq!(g.endpoint, "4-slot");
    assert_eq!(g.granted_context_tokens, 65_536);
    assert!(g.fits_declared_floor(), "65,536 >= 65,000 floor");
}

#[test]
fn a_fleet_with_no_fitting_slot_holds_the_issue() {
    let class = parse_context_class(CARD_LINE).unwrap();
    let verdict = dispatch(&class, &[endpoint("8-slot-a", 8), endpoint("8-slot-b", 8)]);
    let DispatchVerdict::Hold { code, rationale } = verdict else {
        panic!("expected Hold, got {verdict:?}");
    };
    assert_eq!(code, NO_ENDPOINT_LARGE_ENOUGH);
    assert!(
        rationale.contains("32,768") || rationale.contains("32768"),
        "rationale names the largest available slot: {rationale}"
    );
}

#[test]
fn an_empty_fleet_holds_the_issue() {
    let class = parse_context_class(CARD_LINE).unwrap();
    let verdict = dispatch(&class, &[]);
    assert_eq!(held_code(&verdict), NO_ENDPOINT_LARGE_ENOUGH);
}

#[test]
fn the_grant_records_declared_and_granted_numbers() {
    let class = parse_context_class(CARD_LINE).unwrap();
    let verdict = dispatch(&class, &fleet());
    let g = grant(&verdict);
    assert_eq!(g.declared_class, "C75");
    assert_eq!(g.declared_tier.as_deref(), Some("tier-integration"));
    assert_eq!(g.declared_pack_min_tokens, 65_000);
    assert_eq!(g.declared_pack_max_tokens, 82_000);
    assert_eq!(g.endpoint, "2-slot");
    assert_eq!(g.granted_context_tokens, 131_072);
    assert_eq!(g.granted_working_context_tokens, 131_072 - 16_384);
}

#[test]
fn the_status_file_keeps_existing_keys_next_to_the_grant() {
    let class = parse_context_class(CARD_LINE).unwrap();
    let record = ContextGrant::for_dispatch(&class, &endpoint("2-slot", 2));
    let json = status_json_with_grant(Some(r#"{"status":"failed","exit_code":137}"#), &record)
        .expect("valid existing status");
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["status"], "failed");
    assert_eq!(value["exit_code"], 137);
    assert_eq!(value["context"]["declared_class"], "C75");
    assert_eq!(value["context"]["granted_context_tokens"], 131_072);
    // Also works when the runner has not written anything yet.
    let fresh = status_json_with_grant(None, &record).expect("no existing status");
    let value: serde_json::Value = serde_json::from_str(&fresh).unwrap();
    assert_eq!(value["context"]["endpoint"], "2-slot");
}

#[test]
fn the_status_writer_rejects_a_non_object_status() {
    let class = parse_context_class(CARD_LINE).unwrap();
    let record = ContextGrant::for_dispatch(&class, &endpoint("2-slot", 2));
    let err = status_json_with_grant(Some("[1,2]"), &record).unwrap_err();
    assert!(err.contains("not a JSON object"), "{err}");
}

#[test]
fn an_under_provisioned_run_does_not_count_as_an_attempt() {
    let class = parse_context_class(CARD_LINE).unwrap();
    let record = ContextGrant::for_dispatch(&class, &endpoint("8-slot", 8));
    assert_eq!(record.granted_context_tokens, 32_768);
    assert!(!record.fits_declared_floor());
    assert!(
        !record.counts_as_attempt(),
        "the TIMEOUT is evidence about the budget, not the task"
    );
}

#[test]
fn a_provisioned_run_counts_as_an_attempt() {
    let class = parse_context_class(CARD_LINE).unwrap();
    for slots in [4, 2] {
        let record = ContextGrant::for_dispatch(&class, &endpoint(&format!("{slots}-slot"), slots));
        assert!(record.fits_declared_floor());
        assert!(record.counts_as_attempt());
    }
}

#[test]
fn redispatch_excludes_the_class_of_endpoint_that_lost() {
    let class = parse_context_class(CARD_LINE).unwrap();
    let previous = ContextGrant::for_dispatch(&class, &endpoint("8-slot", 8));
    let verdict = redispatch(&class, &fleet(), &previous);
    let g = grant(&verdict);
    assert_eq!(g.endpoint, "2-slot");
    assert!(g.granted_context_tokens > previous.granted_context_tokens);
}

#[test]
fn redispatch_holds_when_only_the_lost_class_remains() {
    let class = parse_context_class(CARD_LINE).unwrap();
    let previous = ContextGrant::for_dispatch(&class, &endpoint("8-slot", 8));
    let verdict = redispatch(
        &class,
        &[endpoint("8-slot-a", 8), endpoint("8-slot-b", 8)],
        &previous,
    );
    assert_eq!(held_code(&verdict), NO_ENDPOINT_LARGE_ENOUGH);
    // A strictly larger slot does remain eligible even at the same floor.
    let verdict = redispatch(&class, &[endpoint("4-slot", 4)], &previous);
    assert_eq!(grant(&verdict).granted_context_tokens, 65_536);
}

#[test]
fn the_issue_correlation_table_holds_end_to_end() {
    let class = parse_context_class(CARD_LINE).unwrap();
    // 32,768 context -> the issue holds instead of a truncated run.
    assert_eq!(
        held_code(&dispatch(&class, &[endpoint("8-slot", 8)])),
        NO_ENDPOINT_LARGE_ENOUGH
    );
    // 65,536 context -> granted, fits the floor.
    assert_eq!(
        grant(&dispatch(&class, &[endpoint("4-slot", 4)])).granted_context_tokens,
        65_536
    );
    // 131,072 context -> granted, the largest slot wins.
    assert_eq!(
        grant(&dispatch(
            &class,
            &[endpoint("4-slot", 4), endpoint("2-slot", 2)]
        ))
        .granted_context_tokens,
        131_072
    );
}

#[test]
fn the_working_context_matches_the_issue_arithmetic() {
    // 8 slots: 32,768 per slot, 16,384 reserved for output, ~16K of working
    // room left for the specification, source files, and test output.
    let e8 = endpoint("8-slot", 8);
    assert_eq!(e8.context_per_slot(), 32_768);
    assert_eq!(e8.working_context_per_slot(), 16_384);
    let e4 = endpoint("4-slot", 4);
    assert_eq!(e4.context_per_slot(), 65_536);
    let e2 = endpoint("2-slot", 2);
    assert_eq!(e2.context_per_slot(), 131_072);
}
