//! Integration tests for `aar::dispatch_fit` (issue #3694, corrected by
//! #3749): context-class fit at dispatch time. The card's declared pack must
//! be compared against the endpoint's slot context before dispatch; when
//! nothing fits the issue is held, not dispatched truncated, and an
//! under-provisioned run does not count as an attempt. The fit check is
//! against the pack **ceiling** (#3749): a slot that holds the typical prompt
//! but truncates a ceiling-sized one is reported as a mismatched worker, not
//! granted.

use autospec_core::aar::dispatch_fit::{
    dispatch, mismatched_workers, parse_context_class, redispatch, slots_for_prompt,
    status_json_with_grant, status_json_with_prompt_size, ContextClass, ContextGrant,
    DispatchVerdict, Endpoint, MismatchedWorker, NO_ENDPOINT_LARGE_ENOUGH,
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
    assert_eq!(class.ceiling_tokens(), 82_000);
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

/// A slot between the pack floor and the ceiling is not enough: it holds the
/// typical prompt but truncates a ceiling-sized one, so the card is held and
/// the worker is reported as mismatched (#3749). The reported count is the
/// working context — per-slot minus the output reserve — the room the prompt
/// actually has (#4351).
#[test]
fn a_slot_between_floor_and_ceiling_holds_the_card() {
    let class = parse_context_class(CARD_LINE).unwrap();
    // 4 slots of 65,536 hold the 65,000 floor but not the 82,000 ceiling; the
    // working context is 65,536 minus the 16,384 output reserve.
    let verdict = dispatch(&class, &[endpoint("4-slot", 4)]);
    let DispatchVerdict::Hold {
        code, mismatched, ..
    } = verdict
    else {
        panic!("expected Hold, got {verdict:?}");
    };
    assert_eq!(code, NO_ENDPOINT_LARGE_ENOUGH);
    assert_eq!(mismatched.len(), 1);
    assert_eq!(mismatched[0].name, "4-slot");
    assert_eq!(mismatched[0].context_per_slot_tokens, 49_152);
    assert_eq!(mismatched[0].required_tokens, 82_000);
}

#[test]
fn a_fleet_with_no_fitting_slot_holds_the_issue() {
    let class = parse_context_class(CARD_LINE).unwrap();
    let verdict = dispatch(&class, &[endpoint("8-slot-a", 8), endpoint("8-slot-b", 8)]);
    let DispatchVerdict::Hold {
        code, rationale, ..
    } = verdict
    else {
        panic!("expected Hold, got {verdict:?}");
    };
    assert_eq!(code, NO_ENDPOINT_LARGE_ENOUGH);
    assert!(
        rationale.contains("16,384") || rationale.contains("16384"),
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
    // A strictly larger slot that is still below the ceiling is not
    // eligible: it would truncate a ceiling-sized prompt (#3749).
    let verdict = redispatch(&class, &[endpoint("4-slot", 4)], &previous);
    assert_eq!(held_code(&verdict), NO_ENDPOINT_LARGE_ENOUGH);
    let DispatchVerdict::Hold { mismatched, .. } = &verdict else {
        panic!("expected Hold, got {verdict:?}");
    };
    assert_eq!(mismatched.len(), 1);
    assert_eq!(mismatched[0].name, "4-slot");
    // The reported count is the working context: 65,536 minus the 16,384
    // output reserve (#4351).
    assert_eq!(mismatched[0].context_per_slot_tokens, 49_152);
}

#[test]
fn the_issue_correlation_table_holds_end_to_end() {
    let class = parse_context_class(CARD_LINE).unwrap();
    // 32,768 context -> the issue holds instead of a truncated run.
    assert_eq!(
        held_code(&dispatch(&class, &[endpoint("8-slot", 8)])),
        NO_ENDPOINT_LARGE_ENOUGH
    );
    // 65,536 context -> held: it fits the 65,000 floor but truncates the
    // 82,000 ceiling (#3749).
    assert_eq!(
        held_code(&dispatch(&class, &[endpoint("4-slot", 4)])),
        NO_ENDPOINT_LARGE_ENOUGH
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

/// `--parallel` is derived from the workload's prompt ceiling, not chosen as
/// a constant: `floor(window / ceiling)`, at least 1 (#3749).
#[test]
fn slots_for_prompt_derives_the_parallel_flag() {
    let window = 262_144;
    // A 256K window and a C75 ceiling of 82,000: floor(262144 / 82000) = 3
    // slots of 87,381 — every slot holds the ceiling. Four slots of 65,536
    // would truncate it, so 3 is the maximum that keeps every slot usable.
    assert_eq!(slots_for_prompt(window, 82_000), 3);
    // A 65,000 ceiling fits four slots of 65,536.
    assert_eq!(slots_for_prompt(window, 65_000), 4);
    // A ceiling larger than the window still yields 1 slot — the dispatch
    // fit check, not the slot count, holds the card.
    assert_eq!(slots_for_prompt(window, 262_145), 1);
    // No known ceiling: the undivided window, the safe default.
    assert_eq!(slots_for_prompt(window, 0), 1);
}

/// The consumer-side invariant of #3749, strengthened by #4351: for every
/// slot count the launcher might have chosen, dispatch grants exactly the
/// slots whose *working* context (per-slot minus the completion reserve)
/// holds the ceiling and holds the card for the rest — a slot that holds the
/// ceiling but not the ceiling plus the reserve 400s at the boundary.
#[test]
fn the_consumer_never_dispatches_below_the_prompt_ceiling_plus_completion() {
    let class = parse_context_class(CARD_LINE).unwrap();
    for slots in 1..=8u32 {
        let endpoint = endpoint(&format!("{slots}-slot"), slots);
        let working = endpoint.working_context_per_slot();
        let verdict = dispatch(&class, &[endpoint]);
        if working >= class.ceiling_tokens() {
            assert!(
                matches!(verdict, DispatchVerdict::Grant(_)),
                "a {slots}-slot endpoint ({working} working tokens/slot) holds \
                 the 82,000 ceiling plus its completion reserve and must be granted"
            );
        } else {
            assert_eq!(
                held_code(&verdict),
                NO_ENDPOINT_LARGE_ENOUGH,
                "a {slots}-slot endpoint ({working} working tokens/slot) cannot \
                 hold the ceiling plus its completion reserve and must hold"
            );
        }
    }
}

/// A slot that holds the prompt ceiling but not the ceiling plus the
/// completion reserve is held, not granted: granting it would accept the
/// request and then refuse it with a 400 at the boundary (issue #4351 — the
/// 67,529-token compaction request that 400'd on a 65,536 slot).
#[test]
fn a_slot_short_only_on_the_completion_reserve_is_held_not_granted() {
    // A 65,536 slot with a 4,096-token completion reserve leaves 61,440 of
    // working context. A 62,000-token prompt ceiling fits the full slot (the
    // old check granted it, then the gateway 400'd) but not the working
    // context, so it must be held.
    let class = ContextClass {
        name: "C62".to_string(),
        tier: None,
        pack_min_tokens: 60_000,
        pack_max_tokens: 62_000,
    };
    let slot = Endpoint {
        name: "edge-slot".to_string(),
        context_window_tokens: 65_536,
        parallel_slots: 1,
        output_reserve_tokens: 4_096,
    };
    assert_eq!(slot.context_per_slot(), 65_536);
    assert_eq!(slot.working_context_per_slot(), 61_440);
    // The full slot holds the ceiling, so the pre-#4351 check granted it.
    assert!(slot.context_per_slot() >= class.pack_max_tokens);
    // The working slot does not, so the card is held instead of 400'ing.
    assert!(slot.working_context_per_slot() < class.pack_max_tokens);
    assert_eq!(
        held_code(&dispatch(&class, &[slot])),
        NO_ENDPOINT_LARGE_ENOUGH
    );
}

/// Undersized capacity is reported as mismatched workers, not silently
/// skipped or counted as capacity (#3749).
#[test]
fn mismatched_workers_report_the_unusable_capacity() {
    let required = 82_000u32;
    // fleet(): 8-slot 32,768 / 4-slot 65,536 / 2-slot 131,072.
    let workers = mismatched_workers(&fleet(), required);
    // Each count is the working context (per-slot minus the 16,384 output
    // reserve), the room the prompt actually has (#4351).
    assert_eq!(
        workers,
        vec![
            MismatchedWorker {
                name: "4-slot".to_string(),
                context_per_slot_tokens: 49_152,
                required_tokens: required,
            },
            MismatchedWorker {
                name: "8-slot".to_string(),
                context_per_slot_tokens: 16_384,
                required_tokens: required,
            },
        ]
    );
    // Nothing below the requirement: the whole fleet is usable capacity.
    assert!(mismatched_workers(&[endpoint("2-slot", 2)], required).is_empty());
    // The report is stable regardless of fleet ordering.
    let reversed = mismatched_workers(&fleet().into_iter().rev().collect::<Vec<_>>(), required);
    assert_eq!(workers, reversed);
}

/// A hold names every mismatched worker in the rationale so the unusable
/// capacity is visible, not silently skipped (#3749).
#[test]
fn a_hold_names_every_mismatched_worker() {
    let class = parse_context_class(CARD_LINE).unwrap();
    let verdict = dispatch(&class, &[endpoint("8-slot-a", 8), endpoint("8-slot-b", 8)]);
    let DispatchVerdict::Hold {
        code,
        rationale,
        mismatched,
    } = verdict
    else {
        panic!("expected Hold, got {verdict:?}");
    };
    assert_eq!(code, NO_ENDPOINT_LARGE_ENOUGH);
    assert_eq!(mismatched.len(), 2);
    assert!(
        rationale.contains("8-slot-a (16384 tokens)"),
        "rationale names every mismatched worker: {rationale}"
    );
    assert!(
        rationale.contains("8-slot-b (16384 tokens)"),
        "rationale names every mismatched worker: {rationale}"
    );
    assert!(
        rationale.contains("82000"),
        "rationale names the required ceiling: {rationale}"
    );
}

/// A redispatch hold reports the mismatched workers over the caller's full
/// fleet view, not just the (filtered) strictly-larger set (#3749).
#[test]
fn redispatch_hold_reports_mismatched_over_the_full_fleet() {
    let class = parse_context_class(CARD_LINE).unwrap();
    // The previous grant was the fleet's largest slot; nothing strictly
    // larger remains.
    let previous = ContextGrant::for_dispatch(&class, &endpoint("2-slot", 2));
    let verdict = redispatch(&class, &fleet(), &previous);
    let DispatchVerdict::Hold {
        code, mismatched, ..
    } = verdict
    else {
        panic!("expected Hold, got {verdict:?}");
    };
    assert_eq!(code, NO_ENDPOINT_LARGE_ENOUGH);
    // The strictly-larger filter left zero endpoints, so the report must
    // come from the full fleet: its two undersized workers.
    assert_eq!(
        mismatched
            .iter()
            .map(|w| w.name.as_str())
            .collect::<Vec<_>>(),
        vec!["4-slot", "8-slot"]
    );
}

/// The status file records the actual prompt size next to the granted slot
/// context, so a zero-output run is auditable against the window it ran in
/// (#3749).
#[test]
fn the_status_file_records_the_actual_prompt_next_to_the_slot_context() {
    let class = parse_context_class(CARD_LINE).unwrap();
    let record = ContextGrant::for_dispatch(&class, &endpoint("3-slot", 3));
    let with_grant = status_json_with_grant(Some(r#"{"status":"running"}"#), &record)
        .expect("valid existing status");
    let with_prompt = status_json_with_prompt_size(Some(&with_grant), 81_500)
        .expect("grant status is valid JSON");
    let value: serde_json::Value = serde_json::from_str(&with_prompt).unwrap();
    assert_eq!(value["status"], "running");
    assert_eq!(value["context"]["declared_class"], "C75");
    assert_eq!(value["context"]["granted_context_tokens"], 87_381);
    assert_eq!(value["context"]["prompt_tokens"], 81_500);
    // The measured prompt fits the granted window.
    assert!(
        value["context"]["prompt_tokens"].as_u64()
            <= value["context"]["granted_context_tokens"].as_u64(),
        "prompt 81,500 must fit the granted 87,381"
    );
    // Works with no existing status and a re-measurement overwrites the
    // earlier one.
    let fresh = status_json_with_prompt_size(None, 40_000).expect("no existing status");
    let value: serde_json::Value = serde_json::from_str(&fresh).unwrap();
    assert_eq!(value["context"]["prompt_tokens"], 40_000);
    let remeasured = status_json_with_prompt_size(Some(&fresh), 55_000).unwrap();
    let value: serde_json::Value = serde_json::from_str(&remeasured).unwrap();
    assert_eq!(value["context"]["prompt_tokens"], 55_000);
    // A non-object status is rejected, like the grant writer.
    assert!(status_json_with_prompt_size(Some("[1,2]"), 100).is_err());
}
