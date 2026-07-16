use super::model::{
    StrictCollectorEvidence, Tier2Deduplication, Tier2Evaluation, Tier2Failure,
    Tier2GeneratedProposals, Tier2Observation, Tier2PartialEvidence, Tier2Proposal,
    Tier2RankedProposal, Tier2Verification, Tier2VerifierVerdicts, TIER2_SCHEMA,
};

pub fn render_tier2_evaluation_json(evaluation: &Tier2Evaluation) -> String {
    match evaluation {
        Tier2Evaluation::NotRun(not_run) => format!(
            "{{\"schema\":{TIER2_SCHEMA},\"kind\":\"tier2_evaluation\",\"result\":\"not_run\",\"reason\":{}}}\n",
            text(&not_run.reason)
        ),
        Tier2Evaluation::Complete(observation) => format!(
            "{{\"schema\":{TIER2_SCHEMA},\"kind\":\"tier2_evaluation\",\"result\":\"complete\",\"funnel\":{},\"ranked\":[{}]}}\n",
            funnel_json(&observation.funnel),
            observation.ranked.iter().map(ranked_json).collect::<Vec<_>>().join(",")
        ),
    }
}

pub fn render_tier2_collector_json(collector: &StrictCollectorEvidence) -> String {
    let domains = collector
        .domains
        .iter()
        .map(domain_json)
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{{\"schema\":{TIER2_SCHEMA},\"kind\":\"tier2_collector\",\"collector_version\":{},\"canonical_repo_scope\":{},\"domains\":[{domains}]}}\n",
        text(&collector.collector_version),
        text(&collector.canonical_repo_scope),
    )
}

pub fn render_tier2_generated_json(
    generated: &Tier2GeneratedProposals,
    predecessor_digest: &str,
) -> Result<String, String> {
    let predecessor = digest(predecessor_digest)?;
    let mut proposals = generated.proposals.clone();
    proposals.sort_by(|left, right| left.stable_key.cmp(&right.stable_key));
    Ok(format!(
        "{{\"schema\":{TIER2_SCHEMA},\"kind\":\"tier2_generated\",\"predecessor_digest\":{predecessor},\"generator_identity\":{},\"generator_protocol_version\":{},\"proposals\":[{}]}}\n",
        text(&generated.generator_identity),
        text(&generated.generator_protocol_version),
        proposals.iter().map(proposal_json).collect::<Vec<_>>().join(","),
    ))
}

pub fn render_tier2_deduplication_json(
    deduplication: &Tier2Deduplication,
    predecessor_digest: &str,
) -> Result<String, String> {
    let predecessor = digest(predecessor_digest)?;
    let groups = deduplication
        .groups
        .iter()
        .map(dedup_json)
        .collect::<Vec<_>>()
        .join(",");
    Ok(format!(
        "{{\"schema\":{TIER2_SCHEMA},\"kind\":\"tier2_dedup\",\"predecessor_digest\":{predecessor},\"groups\":[{groups}]}}\n"
    ))
}

pub fn render_tier2_verification_json(
    verification: &Tier2VerifierVerdicts,
    predecessor_digest: &str,
) -> Result<String, String> {
    let predecessor = digest(predecessor_digest)?;
    let mut verdicts = verification.verdicts.clone();
    verdicts.sort_by(|left, right| left.stable_key().cmp(right.stable_key()));
    Ok(format!(
        "{{\"schema\":{TIER2_SCHEMA},\"kind\":\"tier2_verification\",\"predecessor_digest\":{predecessor},\"verifier_identity\":{},\"verifier_protocol_version\":{},\"verdicts\":[{}]}}\n",
        text(&verification.verifier_identity),
        text(&verification.verifier_protocol_version),
        verdicts.iter().map(verdict_json).collect::<Vec<_>>().join(","),
    ))
}

pub fn render_tier2_roi_rank_json(
    observation: &Tier2Observation,
    predecessor_digest: &str,
) -> Result<String, String> {
    let predecessor = digest(predecessor_digest)?;
    let roi = observation
        .roi
        .iter()
        .map(roi_json)
        .collect::<Vec<_>>()
        .join(",");
    let ranked = observation
        .ranked
        .iter()
        .map(ranked_json)
        .collect::<Vec<_>>()
        .join(",");
    Ok(format!(
        "{{\"schema\":{TIER2_SCHEMA},\"kind\":\"tier2_roi_rank\",\"predecessor_digest\":{predecessor},\"funnel\":{},\"roi\":[{roi}],\"ranked\":[{ranked}]}}\n",
        funnel_json(&observation.funnel),
    ))
}

pub fn render_tier2_failure_json(
    failure: &Tier2Failure,
    predecessor_digest: Option<&str>,
) -> Result<String, String> {
    let expected_predecessor = !matches!(
        failure.partial_evidence(),
        Tier2PartialEvidence::None { .. }
    );
    let predecessor = match (expected_predecessor, predecessor_digest) {
        (false, None) => "null".to_string(),
        (true, Some(value)) => digest(value)?,
        _ => return Err("failure predecessor digest does not match completed stages".to_string()),
    };
    Ok(format!(
        "{{\"schema\":{TIER2_SCHEMA},\"kind\":\"tier2_failure\",\"predecessor_digest\":{predecessor},\"stage\":{},\"code\":{},\"status_reason\":{},\"detail\":{},\"funnel\":{}}}\n",
        text(failure.stage.as_str()),
        text(failure.code.as_str()),
        text(&failure.status_reason()),
        text(&failure.detail),
        funnel_json(failure.partial_evidence().funnel()),
    ))
}

fn domain_json(domain: &crate::explore::specialists::DetectedDomain) -> String {
    format!(
        "{{\"name\":{},\"score\":{},\"evidence\":[{}]}}",
        text(&domain.name),
        domain.score,
        domain
            .evidence
            .iter()
            .map(evidence_json)
            .collect::<Vec<_>>()
            .join(","),
    )
}

fn proposal_json(proposal: &Tier2Proposal) -> String {
    format!(
        "{{\"stable_key\":{},\"title\":{},\"source\":{},\"evidence\":[{}],\"severity\":{},\"confidence_millis\":{},\"complexity\":{},\"named_consumer\":{}}}",
        text(&proposal.stable_key), text(&proposal.title), text(proposal.source.as_str()),
        proposal.evidence.iter().map(evidence_json).collect::<Vec<_>>().join(","),
        text(proposal.severity.as_str()), proposal.confidence_millis, text(proposal.complexity.as_str()), text(&proposal.named_consumer),
    )
}

fn evidence_json(evidence: &crate::explore::specialists::FileLineEvidence) -> String {
    format!(
        "{{\"file\":{},\"line\":{},\"match\":{}}}",
        text(&evidence.file),
        evidence.line,
        text(&evidence.r#match)
    )
}

fn dedup_json(group: &super::Tier2DeduplicationGroup) -> String {
    let candidates = group
        .candidate_keys
        .iter()
        .map(|key| text(key))
        .collect::<Vec<_>>()
        .join(",");
    let suppressed = group
        .suppressed_keys
        .iter()
        .map(|key| text(key))
        .collect::<Vec<_>>()
        .join(",");
    let scores = group
        .score_quotients
        .iter()
        .map(|score| {
            format!(
                "{{\"stable_key\":{},\"score_quotient\":{}}}",
                text(&score.stable_key),
                score.score_quotient
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!("{{\"key\":{},\"candidate_keys\":[{candidates}],\"winner_key\":{},\"suppressed_keys\":[{suppressed}],\"score_quotients\":[{scores}]}}", text(&group.key), text(&group.winner_key))
}

fn verdict_json(verdict: &Tier2Verification) -> String {
    format!(
        "{{\"stable_key\":{},\"result\":{},\"reason\":{}}}",
        text(verdict.stable_key()),
        text(verdict.as_str()),
        text(verdict.reason())
    )
}

fn roi_json(decision: &super::Tier2RoiDecision) -> String {
    format!(
        "{{\"stable_key\":{},\"source\":{},\"permitted\":{}}}",
        text(&decision.stable_key),
        text(decision.source.as_str()),
        decision.permitted
    )
}

fn ranked_json(ranked: &Tier2RankedProposal) -> String {
    format!("{{\"rank\":{},\"stable_key\":{},\"severity_rank\":{},\"score_numerator\":{},\"complexity_units\":{},\"score_quotient\":{},\"named_consumer\":{}}}", ranked.rank, text(&ranked.stable_key), ranked.severity_rank, ranked.score_numerator, ranked.complexity_units, ranked.score_quotient, text(&ranked.named_consumer))
}

fn funnel_json(funnel: &crate::autonomous::waterfall::FunnelCounts) -> String {
    format!(
        "{{\"observed\":{},\"deduplicated\":{},\"verified\":{},\"roi_approved\":{},\"ranked\":{}}}",
        funnel.observed, funnel.deduplicated, funnel.verified, funnel.roi_approved, funnel.ranked
    )
}

fn digest(value: &str) -> Result<String, String> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(text(value))
    } else {
        Err("predecessor digest must be a sealed lowercase SHA-256 value".to_string())
    }
}

fn text(value: &str) -> String {
    let mut rendered = String::from("\"");
    for character in value.chars() {
        match character {
            '"' => rendered.push_str("\\\""),
            '\\' => rendered.push_str("\\\\"),
            '\n' => rendered.push_str("\\n"),
            '\r' => rendered.push_str("\\r"),
            '\t' => rendered.push_str("\\t"),
            character if character.is_control() => {
                rendered.push_str(&format!("\\u{:04x}", character as u32))
            }
            character => rendered.push(character),
        }
    }
    rendered.push('"');
    rendered
}
