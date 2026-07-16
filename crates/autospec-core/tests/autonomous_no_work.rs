use autospec_core::autonomous::no_work::{
    DryReason, NoWorkDecision, NoWorkObservation, NoWorkState, NoWorkTier, TierOutcome,
    IDEATION_CANDIDATE_LIMIT, IDEATION_DRY_PASS_THRESHOLD,
};

const TEST_DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn complete_dry(pass_id: u64) -> NoWorkObservation {
    observation(
        pass_id,
        vec![
            (
                NoWorkTier::Tier1,
                TierOutcome::Dry {
                    reason: DryReason::NoProposalsGenerated,
                },
            ),
            (
                NoWorkTier::Tier1_5,
                TierOutcome::Dry {
                    reason: DryReason::Deduplicated,
                },
            ),
            (
                NoWorkTier::Tier2,
                TierOutcome::Dry {
                    reason: DryReason::VerificationRejected,
                },
            ),
            (
                NoWorkTier::Tier3,
                TierOutcome::Dry {
                    reason: DryReason::RoiFiltered,
                },
            ),
            (
                NoWorkTier::Tier4,
                TierOutcome::Dry {
                    reason: DryReason::AlreadyImplemented,
                },
            ),
        ],
    )
}

fn observation(pass_id: u64, tiers: Vec<(NoWorkTier, TierOutcome)>) -> NoWorkObservation {
    NoWorkObservation {
        repo: "owner/repo".to_string(),
        pass_id,
        evidence_digest: TEST_DIGEST.to_string(),
        tiers,
    }
}

fn with_replacement(pass_id: u64, tier: NoWorkTier, outcome: TierOutcome) -> NoWorkObservation {
    let mut observation = complete_dry(pass_id);
    let entry = observation
        .tiers
        .iter_mut()
        .find(|(candidate, _)| *candidate == tier)
        .expect("tier exists");
    entry.1 = outcome;
    observation
}

#[test]
fn second_consecutive_complete_dry_pass_requests_bounded_ideation() {
    let first = NoWorkState::record(None, complete_dry(1)).expect("first pass");
    let second = NoWorkState::record(Some(&first), complete_dry(2)).expect("second pass");

    assert_eq!(IDEATION_DRY_PASS_THRESHOLD, 2);
    assert_eq!(second.decision(), NoWorkDecision::RequestIdeation);
    assert_eq!(second.candidate_limit(), IDEATION_CANDIDATE_LIMIT);
    assert_eq!(second.candidate_limit(), 5);
}

#[test]
fn not_run_failed_and_produced_tiers_cannot_increment_a_dry_pass() {
    let observations = [
        with_replacement(
            1,
            NoWorkTier::Tier1,
            TierOutcome::NotRun {
                reason: "operator paused".to_string(),
            },
        ),
        with_replacement(
            1,
            NoWorkTier::Tier2,
            TierOutcome::Failed {
                reason: "typed producer failed".to_string(),
            },
        ),
        with_replacement(1, NoWorkTier::Tier3, TierOutcome::Produced { count: 1 }),
    ];

    for observation in observations {
        let state = NoWorkState::record(None, observation).expect("valid non-dry pass");
        assert_eq!(state.consecutive_dry_passes(), 0);
        assert_eq!(state.decision(), NoWorkDecision::IdleRescan);
    }
}

#[test]
fn each_exact_dry_reason_is_closed_and_preserved() {
    let expected = [
        (DryReason::NoProposalsGenerated, "no_proposals_generated"),
        (DryReason::Deduplicated, "deduplicated"),
        (DryReason::VerificationRejected, "verification_rejected"),
        (DryReason::RoiFiltered, "roi_filtered"),
        (DryReason::AlreadyImplemented, "already_implemented"),
    ];

    for (reason, name) in expected {
        assert_eq!(reason.as_str(), name);
        assert_eq!(
            DryReason::parse(name).expect("closed reason parses"),
            reason
        );
    }
    assert!(DryReason::parse("made_up_reason").is_err());

    let state = NoWorkState::record(None, complete_dry(1)).expect("all reasons are valid");
    for (_, name) in expected {
        assert!(state.to_json().contains(&format!("\"{name}\":1")));
    }
}

#[test]
fn requires_each_ordered_tier_exactly_once() {
    let missing = observation(
        1,
        complete_dry(1)
            .tiers
            .into_iter()
            .filter(|(tier, _)| *tier != NoWorkTier::Tier4)
            .collect(),
    );
    assert!(NoWorkState::record(None, missing)
        .expect_err("missing tier is rejected")
        .contains("missing tier"));

    let mut duplicate = complete_dry(1);
    duplicate.tiers.push((
        NoWorkTier::Tier1,
        TierOutcome::Dry {
            reason: DryReason::NoProposalsGenerated,
        },
    ));
    assert!(NoWorkState::record(None, duplicate)
        .expect_err("duplicate tier is rejected")
        .contains("duplicate tier"));

    let unordered = observation(1, complete_dry(1).tiers.into_iter().rev().collect());
    assert!(NoWorkState::record(None, unordered)
        .expect_err("unordered tier set is rejected")
        .contains("must be ordered"));
}

#[test]
fn validates_positive_pass_produced_and_text_reasons() {
    assert!(NoWorkState::record(None, complete_dry(0))
        .expect_err("zero pass is invalid")
        .contains("pass_id must be positive"));
    assert!(NoWorkState::record(
        None,
        with_replacement(1, NoWorkTier::Tier1, TierOutcome::Produced { count: 0 }),
    )
    .expect_err("zero produced is invalid")
    .contains("count must be positive"));
    assert!(NoWorkState::record(
        None,
        with_replacement(
            1,
            NoWorkTier::Tier1,
            TierOutcome::NotRun {
                reason: " \t".to_string(),
            },
        ),
    )
    .expect_err("blank not-run reason is invalid")
    .contains("reason must not be empty"));
    assert!(NoWorkState::record(
        None,
        with_replacement(
            1,
            NoWorkTier::Tier1,
            TierOutcome::Failed {
                reason: String::new(),
            },
        ),
    )
    .expect_err("blank failure reason is invalid")
    .contains("reason must not be empty"));
}

#[test]
fn duplicate_pass_is_idempotent_but_conflicts_and_stale_passes_fail_closed() {
    let first = NoWorkState::record(None, complete_dry(4)).expect("first pass");

    let duplicate =
        NoWorkState::record(Some(&first), complete_dry(4)).expect("same pass is idempotent");
    assert_eq!(duplicate, first);

    assert!(NoWorkState::record(
        Some(&first),
        with_replacement(4, NoWorkTier::Tier1, TierOutcome::Produced { count: 1 }),
    )
    .expect_err("conflicting duplicate is rejected")
    .contains("conflicting duplicate"));
    assert!(NoWorkState::record(Some(&first), complete_dry(3))
        .expect_err("stale pass is rejected")
        .contains("stale pass"));

    let foreign = NoWorkObservation {
        repo: "other/repo".to_string(),
        ..complete_dry(5)
    };
    assert!(NoWorkState::record(Some(&first), foreign)
        .expect_err("cross-repository history is rejected")
        .contains("repository"));
}

#[test]
fn requires_contiguous_passes_and_retains_the_bounded_dry_source_history() {
    let first = NoWorkState::record(None, complete_dry(1)).expect("first dry pass");
    assert_eq!(
        first
            .dry_pass_history()
            .iter()
            .map(|source| source.pass_id)
            .collect::<Vec<_>>(),
        vec![1]
    );

    let second = NoWorkState::record(Some(&first), complete_dry(2)).expect("second dry pass");
    assert_eq!(
        second
            .dry_pass_history()
            .iter()
            .map(|source| source.pass_id)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert!(NoWorkState::record(Some(&second), complete_dry(4))
        .expect_err("a later pass cannot skip a source pass")
        .contains("exactly next"));

    let third = NoWorkState::record(Some(&second), complete_dry(3)).expect("third dry pass");
    assert_eq!(third.consecutive_dry_passes(), 3);
    assert_eq!(
        third
            .dry_pass_history()
            .iter()
            .map(|source| source.pass_id)
            .collect::<Vec<_>>(),
        vec![2, 3]
    );
    assert_eq!(
        third.dry_pass_history()[1]
            .evidence_for(NoWorkTier::Tier1)
            .reference,
        "waterfall/3/tier1.json"
    );
    assert!(third
        .to_json()
        .contains("\"reference\":\"waterfall/3/tier1.json\""));

    let reset = NoWorkState::record(
        Some(&third),
        with_replacement(4, NoWorkTier::Tier4, TierOutcome::Produced { count: 1 }),
    )
    .expect("non-dry pass resets history");
    assert!(reset.dry_pass_history().is_empty());
}

#[test]
fn rejects_unsealed_evidence_digests_before_recording() {
    let mut malformed = complete_dry(1);
    malformed.evidence_digest = "NOT-A-SEALED-DIGEST".to_string();

    assert!(NoWorkState::record(None, malformed)
        .expect_err("non-hex evidence digest is rejected")
        .contains("evidence digest"));
}

#[test]
fn json_round_trip_rejects_unknown_schema_and_fields() {
    let first = NoWorkState::record(None, complete_dry(1)).expect("first pass");
    let second = NoWorkState::record(Some(&first), complete_dry(2)).expect("second pass");
    let json = second.to_json();

    assert_eq!(
        NoWorkState::parse_json(&json).expect("state parses"),
        second
    );
    assert!(
        NoWorkState::parse_json(&json.replace("\"schema\":1", "\"schema\":2"))
            .expect_err("unknown schema is rejected")
            .contains("unsupported no-work schema")
    );
    assert!(
        NoWorkState::parse_json(&json.replace('}', ",\"unexpected\":true}"))
            .expect_err("unknown field is rejected")
            .contains("unexpected no-work state field")
    );
}

#[test]
fn parse_json_rejects_overflowed_non_contiguous_retained_dry_history() {
    let first = NoWorkState::record(None, complete_dry(u64::MAX - 1)).expect("first pass");
    let second = NoWorkState::record(Some(&first), complete_dry(u64::MAX)).expect("second pass");
    let overflowed_history = second
        .to_json()
        .replace(
            &format!("\"pass_id\":{}", u64::MAX - 1),
            &format!("\"pass_id\":{}", u64::MAX),
        )
        .replace(
            &format!("waterfall/{}/", u64::MAX - 1),
            &format!("waterfall/{}/", u64::MAX),
        );

    assert!(NoWorkState::parse_json(&overflowed_history).is_err());
}

#[test]
fn request_projection_has_exact_six_questions_and_planning_only_authority() {
    let first = NoWorkState::record(None, complete_dry(1)).expect("first pass");
    let second = NoWorkState::record(Some(&first), complete_dry(2)).expect("second pass");
    let request = second
        .ideation_request()
        .expect("threshold requests ideation");

    assert_eq!(request.candidate_limit, 5);
    assert_eq!(request.disposition, "planning_only");
    assert_eq!(request.remote_mutation, "none");
    assert_eq!(
        request.score_fields,
        ["impact", "importance", "risk", "effort"]
    );
    assert_eq!(
        request.questions,
        [
            "What features are missing?",
            "What can we do here?",
            "Find 5 new features.",
            "Rank them by impact and importance.",
            "Which are safe to implement autonomously now?",
            "Which need a planning/spec issue first?",
        ]
    );
}
