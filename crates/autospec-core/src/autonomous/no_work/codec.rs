use std::collections::BTreeMap;

use crate::state::json::{JsonParser, JsonValue};

use super::{
    evidence_reference, is_sealed_digest, DryReason, IdeationRequest, NoWorkDecision,
    NoWorkObservation, NoWorkState, NoWorkTier, TierEvidence, TierOutcome,
    IDEATION_CANDIDATE_LIMIT, IDEATION_DRY_PASS_THRESHOLD, NO_WORK_SCHEMA,
};

pub(super) fn state_json(state: &NoWorkState) -> String {
    let ideation_request = state
        .ideation_request()
        .map(ideation_request_json)
        .unwrap_or_else(|| "null".to_string());
    format!(
        "{{\"schema\":{},\"repo\":\"{}\",\"pass_id\":{},\"consecutive_dry_passes\":{},\"threshold\":{},\"tiers\":[{}],\"dry_pass_history\":[{}],\"decision\":\"{}\",\"reason_counts\":{},\"ideation_request\":{}}}",
        NO_WORK_SCHEMA,
        escape_json(&state.repo),
        state.pass_id,
        state.consecutive_dry_passes,
        IDEATION_DRY_PASS_THRESHOLD,
        tiers_json(state.pass_id, &state.evidence_digest, &state.tiers),
        state
            .dry_pass_history
            .iter()
            .map(observation_json)
            .collect::<Vec<_>>()
            .join(","),
        state.decision().as_str(),
        reason_counts_json(&state.tiers),
        ideation_request,
    )
}

pub(super) fn parse_state(input: &str) -> Result<NoWorkState, String> {
    let mut object = JsonParser::new(input)
        .parse()?
        .into_object("no-work state")?;
    require_only_keys(
        &object,
        &[
            "schema",
            "repo",
            "pass_id",
            "consecutive_dry_passes",
            "threshold",
            "tiers",
            "dry_pass_history",
            "decision",
            "reason_counts",
            "ideation_request",
        ],
        "no-work state",
    )?;
    let schema = take_required(&mut object, "schema", "no-work state")?
        .into_number("no-work state.schema")?;
    if schema != NO_WORK_SCHEMA {
        return Err(format!("unsupported no-work schema: {schema}"));
    }
    let repo =
        take_required(&mut object, "repo", "no-work state")?.into_string("no-work state.repo")?;
    let pass_id = take_required(&mut object, "pass_id", "no-work state")?
        .into_number("no-work state.pass_id")?;
    let consecutive_dry_passes =
        take_required(&mut object, "consecutive_dry_passes", "no-work state")?
            .into_number("no-work state.consecutive_dry_passes")?;
    let threshold = take_required(&mut object, "threshold", "no-work state")?
        .into_number("no-work state.threshold")?;
    if threshold != IDEATION_DRY_PASS_THRESHOLD {
        return Err("no-work state threshold does not match the closed policy".to_string());
    }
    let (tiers, evidence_digest) = parse_tiers(
        take_required(&mut object, "tiers", "no-work state")?,
        pass_id,
        "no-work state.tiers",
    )?;
    let dry_pass_history = parse_dry_pass_history(take_required(
        &mut object,
        "dry_pass_history",
        "no-work state",
    )?)?;
    let decision = NoWorkDecision::parse(
        &take_required(&mut object, "decision", "no-work state")?
            .into_string("no-work state.decision")?,
    )?;
    let reason_counts = parse_reason_counts(take_required(
        &mut object,
        "reason_counts",
        "no-work state",
    )?)?;
    let ideation_request = take_required(&mut object, "ideation_request", "no-work state")?;

    let state = NoWorkState {
        repo,
        pass_id,
        evidence_digest,
        consecutive_dry_passes,
        tiers,
        dry_pass_history,
    };
    state.validate()?;
    if decision != state.decision() {
        return Err("no-work state decision does not match the closed policy".to_string());
    }
    if reason_counts != reason_counts_for(&state.tiers) {
        return Err("no-work state reason counts do not match tier outcomes".to_string());
    }
    parse_ideation_request(ideation_request, state.ideation_request())?;
    Ok(state)
}

fn observation_json(observation: &NoWorkObservation) -> String {
    format!(
        "{{\"repo\":\"{}\",\"pass_id\":{},\"tiers\":[{}]}}",
        escape_json(&observation.repo),
        observation.pass_id,
        tiers_json(
            observation.pass_id,
            &observation.evidence_digest,
            &observation.tiers,
        )
    )
}

fn tiers_json(pass_id: u64, digest: &str, tiers: &[(NoWorkTier, TierOutcome)]) -> String {
    tiers
        .iter()
        .map(|tier| tier_json(pass_id, digest, tier))
        .collect::<Vec<_>>()
        .join(",")
}

fn tier_json(pass_id: u64, digest: &str, (tier, outcome): &(NoWorkTier, TierOutcome)) -> String {
    format!(
        concat!(
            "{{\"tier\":\"{}\",\"outcome\":{},",
            "\"evidence\":{{\"digest\":\"{}\",\"reference\":\"{}\"}}}}"
        ),
        tier.as_str(),
        outcome_json(outcome),
        digest,
        evidence_reference(pass_id, *tier),
    )
}

fn outcome_json(outcome: &TierOutcome) -> String {
    match outcome {
        TierOutcome::Produced { count } => format!("{{\"kind\":\"produced\",\"count\":{count}}}"),
        TierOutcome::Dry { reason } => {
            format!("{{\"kind\":\"dry\",\"reason\":\"{}\"}}", reason.as_str())
        }
        TierOutcome::NotRun { reason } => {
            format!(
                "{{\"kind\":\"not_run\",\"reason\":\"{}\"}}",
                escape_json(reason)
            )
        }
        TierOutcome::Failed { reason } => {
            format!(
                "{{\"kind\":\"failed\",\"reason\":\"{}\"}}",
                escape_json(reason)
            )
        }
    }
}

fn reason_counts_for(tiers: &[(NoWorkTier, TierOutcome)]) -> [(DryReason, u64); 5] {
    let mut counts = [(DryReason::NoProposalsGenerated, 0); 5];
    for (index, reason) in DryReason::ALL.into_iter().enumerate() {
        counts[index].0 = reason;
    }
    for (_, outcome) in tiers {
        if let TierOutcome::Dry { reason } = outcome {
            let (_, count) = counts
                .iter_mut()
                .find(|(candidate, _)| candidate == reason)
                .expect("closed dry reason has a count slot");
            *count += 1;
        }
    }
    counts
}

fn reason_counts_json(tiers: &[(NoWorkTier, TierOutcome)]) -> String {
    format!(
        "{{{}}}",
        reason_counts_for(tiers)
            .iter()
            .map(|(reason, count)| format!("\"{}\":{count}", reason.as_str()))
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn ideation_request_json(request: IdeationRequest) -> String {
    format!(
        "{{\"candidate_limit\":{},\"disposition\":\"{}\",\"remote_mutation\":\"{}\",\"score_fields\":[{}],\"questions\":[{}]}}",
        request.candidate_limit,
        request.disposition,
        request.remote_mutation,
        request
            .score_fields
            .iter()
            .map(|field| format!("\"{field}\""))
            .collect::<Vec<_>>()
            .join(","),
        request
            .questions
            .iter()
            .map(|question| format!("\"{}\"", escape_json(question)))
            .collect::<Vec<_>>()
            .join(","),
    )
}

fn parse_tiers(
    value: JsonValue,
    pass_id: u64,
    context: &str,
) -> Result<(Vec<(NoWorkTier, TierOutcome)>, String), String> {
    let mut digest = None;
    let tiers = value
        .into_array(context)?
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            let tier_context = format!("{context}[{index}]");
            let mut tier = value.into_object(&tier_context)?;
            require_only_keys(&tier, &["tier", "outcome", "evidence"], &tier_context)?;
            let tier_name = NoWorkTier::parse(
                &take_required(&mut tier, "tier", &tier_context)?
                    .into_string(&format!("{tier_context}.tier"))?,
            )?;
            let outcome = parse_outcome(
                take_required(&mut tier, "outcome", &tier_context)?,
                &tier_context,
            )?;
            let evidence = parse_tier_evidence(
                take_required(&mut tier, "evidence", &tier_context)?,
                pass_id,
                tier_name,
                &tier_context,
            )?;
            if let Some(existing) = &digest {
                if existing != &evidence.digest {
                    return Err(
                        "no-work tier evidence digests must match within a pass".to_string()
                    );
                }
            } else {
                digest = Some(evidence.digest);
            }
            Ok((tier_name, outcome))
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok((
        tiers,
        digest.ok_or_else(|| "no-work state tiers must not be empty".to_string())?,
    ))
}

fn parse_tier_evidence(
    value: JsonValue,
    pass_id: u64,
    tier: NoWorkTier,
    parent: &str,
) -> Result<TierEvidence, String> {
    let context = format!("{parent}.evidence");
    let mut object = value.into_object(&context)?;
    require_only_keys(&object, &["digest", "reference"], &context)?;
    let digest = take_required(&mut object, "digest", &context)?
        .into_string(&format!("{context}.digest"))?;
    if !is_sealed_digest(&digest) {
        return Err("no-work tier evidence digest is not sealed".to_string());
    }
    let reference = take_required(&mut object, "reference", &context)?
        .into_string(&format!("{context}.reference"))?;
    if reference != evidence_reference(pass_id, tier) {
        return Err(
            "no-work tier evidence reference is not derived from the pass and tier".to_string(),
        );
    }
    Ok(TierEvidence { digest, reference })
}

fn parse_dry_pass_history(value: JsonValue) -> Result<Vec<NoWorkObservation>, String> {
    value
        .into_array("no-work state.dry_pass_history")?
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            let context = format!("no-work state.dry_pass_history[{index}]");
            let mut object = value.into_object(&context)?;
            require_only_keys(&object, &["repo", "pass_id", "tiers"], &context)?;
            let repo = take_required(&mut object, "repo", &context)?
                .into_string(&format!("{context}.repo"))?;
            let pass_id = take_required(&mut object, "pass_id", &context)?
                .into_number(&format!("{context}.pass_id"))?;
            let (tiers, evidence_digest) = parse_tiers(
                take_required(&mut object, "tiers", &context)?,
                pass_id,
                &format!("{context}.tiers"),
            )?;
            Ok(NoWorkObservation {
                repo,
                pass_id,
                evidence_digest,
                tiers,
            })
        })
        .collect()
}

fn parse_outcome(value: JsonValue, parent: &str) -> Result<TierOutcome, String> {
    let context = format!("{parent}.outcome");
    let mut object = value.into_object(&context)?;
    let kind =
        take_required(&mut object, "kind", &context)?.into_string(&format!("{context}.kind"))?;
    match kind.as_str() {
        "produced" => {
            require_only_keys(&object, &["count"], &context)?;
            Ok(TierOutcome::Produced {
                count: take_required(&mut object, "count", &context)?
                    .into_number(&format!("{context}.count"))?,
            })
        }
        "dry" => {
            require_only_keys(&object, &["reason"], &context)?;
            Ok(TierOutcome::Dry {
                reason: DryReason::parse(
                    &take_required(&mut object, "reason", &context)?
                        .into_string(&format!("{context}.reason"))?,
                )?,
            })
        }
        "not_run" => {
            require_only_keys(&object, &["reason"], &context)?;
            Ok(TierOutcome::NotRun {
                reason: take_required(&mut object, "reason", &context)?
                    .into_string(&format!("{context}.reason"))?,
            })
        }
        "failed" => {
            require_only_keys(&object, &["reason"], &context)?;
            Ok(TierOutcome::Failed {
                reason: take_required(&mut object, "reason", &context)?
                    .into_string(&format!("{context}.reason"))?,
            })
        }
        _ => Err(format!("unknown no-work outcome kind: {kind}")),
    }
}

fn parse_reason_counts(value: JsonValue) -> Result<[(DryReason, u64); 5], String> {
    let context = "no-work state.reason_counts";
    let mut object = value.into_object(context)?;
    require_only_keys(&object, &DryReason::ALL.map(DryReason::as_str), context)?;
    let mut counts = [(DryReason::NoProposalsGenerated, 0); 5];
    for (index, reason) in DryReason::ALL.into_iter().enumerate() {
        let name = reason.as_str();
        counts[index] = (
            reason,
            take_required(&mut object, name, context)?.into_number(&format!("{context}.{name}"))?,
        );
    }
    Ok(counts)
}

fn parse_ideation_request(
    value: JsonValue,
    expected: Option<IdeationRequest>,
) -> Result<(), String> {
    match (value, expected) {
        (JsonValue::Null, None) => Ok(()),
        (JsonValue::Object(mut object), Some(expected)) => {
            let context = "no-work ideation request";
            require_only_keys(
                &object,
                &[
                    "candidate_limit",
                    "disposition",
                    "remote_mutation",
                    "score_fields",
                    "questions",
                ],
                context,
            )?;
            let candidate_limit = take_required(&mut object, "candidate_limit", context)?
                .into_number("no-work ideation request.candidate_limit")?;
            let disposition = take_required(&mut object, "disposition", context)?
                .into_string("no-work ideation request.disposition")?;
            let remote_mutation = take_required(&mut object, "remote_mutation", context)?
                .into_string("no-work ideation request.remote_mutation")?;
            let score_fields = parse_string_array(
                take_required(&mut object, "score_fields", context)?,
                "no-work ideation request.score_fields",
            )?;
            let questions = parse_string_array(
                take_required(&mut object, "questions", context)?,
                "no-work ideation request.questions",
            )?;
            let matches_policy = candidate_limit == IDEATION_CANDIDATE_LIMIT
                && disposition == expected.disposition
                && remote_mutation == expected.remote_mutation
                && score_fields
                    .iter()
                    .map(String::as_str)
                    .eq(expected.score_fields)
                && questions.iter().map(String::as_str).eq(expected.questions);
            matches_policy.then_some(()).ok_or_else(|| {
                "no-work ideation request does not match the closed policy".to_string()
            })
        }
        (JsonValue::Null, Some(_)) => Err("no-work ideation request is required".to_string()),
        (_, None) => {
            Err("no-work ideation request is not allowed before the dry-pass threshold".to_string())
        }
        _ => Err("no-work ideation request must be an object or null".to_string()),
    }
}

fn parse_string_array(value: JsonValue, context: &str) -> Result<Vec<String>, String> {
    value
        .into_array(context)?
        .into_iter()
        .enumerate()
        .map(|(index, value)| value.into_string(&format!("{context}[{index}]")))
        .collect()
}

fn require_only_keys(
    object: &BTreeMap<String, JsonValue>,
    expected: &[&str],
    context: &str,
) -> Result<(), String> {
    if let Some(key) = object.keys().find(|key| !expected.contains(&key.as_str())) {
        return Err(format!("unexpected {context} field: {key}"));
    }
    Ok(())
}

fn take_required(
    object: &mut BTreeMap<String, JsonValue>,
    key: &str,
    context: &str,
) -> Result<JsonValue, String> {
    object
        .remove(key)
        .ok_or_else(|| format!("missing {context} field: {key}"))
}

fn escape_json(value: &str) -> String {
    value
        .chars()
        .flat_map(|character| match character {
            '"' => "\\\"".chars().collect::<Vec<_>>(),
            '\\' => "\\\\".chars().collect::<Vec<_>>(),
            '\n' => "\\n".chars().collect::<Vec<_>>(),
            '\r' => "\\r".chars().collect::<Vec<_>>(),
            '\t' => "\\t".chars().collect::<Vec<_>>(),
            character if character.is_control() => format!("\\u{:04x}", character as u32)
                .chars()
                .collect::<Vec<_>>(),
            character => vec![character],
        })
        .collect()
}
