use std::collections::{BTreeMap, BTreeSet};

use super::funnel_validation::{
    invalid_coverage, normalize_title, score, validate_collector, validate_generator,
    validate_verifier_metadata,
};
use super::model::{
    bounded_text, StrictCollectorEvidence, Tier2CandidateScore, Tier2Deduplication,
    Tier2DeduplicationGroup, Tier2Evaluation, Tier2Failure, Tier2FailureCode,
    Tier2GeneratedProposals, Tier2Input, Tier2Observation, Tier2PartialEvidence, Tier2Proposal,
    Tier2RankedProposal, Tier2RoiDecision, Tier2Stage, Tier2StageResult, Tier2Verification,
    Tier2VerifierVerdicts, FIELD_SCALAR_LIMIT, REASON_SCALAR_LIMIT, TIER2_RANK_LIMIT,
};
use crate::autonomous::waterfall::FunnelCounts;

pub fn evaluate_tier2(input: Tier2Input) -> Result<Tier2Evaluation, Tier2Failure> {
    match input {
        Tier2Input::DisabledByCheckedInPolicy => {
            Ok(Tier2Evaluation::NotRun(super::Tier2NotRun::disabled()))
        }
        Tier2Input::Enabled {
            collector,
            generator,
            verifier,
            roi_policy,
        } => evaluate_enabled(collector, generator, verifier, roi_policy),
    }
}

fn evaluate_enabled(
    collector: Tier2StageResult<StrictCollectorEvidence>,
    generator: Tier2StageResult<Tier2GeneratedProposals>,
    verifier: Tier2StageResult<Tier2VerifierVerdicts>,
    roi_policy: super::Tier2RoiPolicy,
) -> Result<Tier2Evaluation, Tier2Failure> {
    let collector = complete(collector, Tier2Stage::Collector, empty_partial())?;
    let collector_rows = validate_collector(&collector)?;
    let collector_partial = collector_partial(&collector, zero_funnel());
    let generator = complete(generator, Tier2Stage::Generator, collector_partial.clone())?;
    validate_generator(&generator, &collector_rows)
        .map_err(|error| error.with_partial(collector_partial.clone()))?;
    let observed = count(generator.proposals.len())
        .map_err(|error| error.with_partial(collector_partial.clone()))?;
    let generated_partial =
        generated_partial(&collector, &generator, funnel(observed, 0, 0, 0, 0)?);
    let (deduplication, winners) = deduplicate(&generator.proposals)
        .map_err(|error| error.with_partial(generated_partial.clone()))?;
    let deduplicated =
        count(winners.len()).map_err(|error| error.with_partial(generated_partial.clone()))?;
    let dedup_partial = dedup_partial(
        &collector,
        &generator,
        &deduplication,
        funnel(observed, deduplicated, 0, 0, 0)
            .map_err(|error| error.with_partial(generated_partial.clone()))?,
    );
    let verifier = complete(verifier, Tier2Stage::Verifier, dedup_partial.clone())?;
    validate_verifier_metadata(&verifier)
        .map_err(|error| error.with_partial(dedup_partial.clone()))?;
    let verdicts = verify_coverage(&verifier, &winners)
        .map_err(|error| error.with_partial(dedup_partial.clone()))?;
    let verified = count(
        winners
            .iter()
            .filter(|winner| verdicts[winner.proposal.stable_key.as_str()].survived())
            .count(),
    )
    .map_err(|error| error.with_partial(dedup_partial.clone()))?;
    let verified_partial = verified_partial(
        &collector,
        &generator,
        &deduplication,
        &verifier,
        funnel(observed, deduplicated, verified, 0, 0)
            .map_err(|error| error.with_partial(dedup_partial.clone()))?,
    );
    let (roi, ranked) = rank(&winners, &verdicts, &roi_policy)
        .map_err(|error| error.with_partial(verified_partial.clone()))?;
    let roi_approved = count(roi.iter().filter(|decision| decision.permitted).count())
        .map_err(|error| error.with_partial(verified_partial.clone()))?;
    let funnel = funnel(
        observed,
        deduplicated,
        verified,
        roi_approved,
        count(ranked.len()).map_err(|error| error.with_partial(verified_partial.clone()))?,
    )
    .map_err(|error| error.with_partial(verified_partial.clone()))?;
    Ok(Tier2Evaluation::Complete(Tier2Observation {
        collector,
        generated: generator,
        deduplication,
        verification: verifier,
        roi,
        ranked,
        funnel,
    }))
}

fn complete<T>(
    result: Tier2StageResult<T>,
    stage: Tier2Stage,
    partial: Tier2PartialEvidence,
) -> Result<T, Tier2Failure> {
    match result {
        Tier2StageResult::Complete(value) => Ok(value),
        Tier2StageResult::Failed(error) => Err(checked_failure(error).with_partial(partial)),
        Tier2StageResult::Missing => Err(failure(
            stage,
            Tier2FailureCode::MissingStageResult,
            "stage result was not supplied",
        )
        .with_partial(partial)),
    }
}

fn deduplicate(
    proposals: &[Tier2Proposal],
) -> Result<(Tier2Deduplication, Vec<Winner>), Tier2Failure> {
    let mut grouped = BTreeMap::<String, Vec<Tier2Proposal>>::new();
    for proposal in proposals {
        grouped
            .entry(format!(
                "{}\0{}",
                proposal.source.as_str(),
                normalize_title(&proposal.title)
            ))
            .or_default()
            .push(proposal.clone());
    }
    let mut groups = Vec::with_capacity(grouped.len());
    let mut winners = Vec::with_capacity(grouped.len());
    for (key, mut candidates) in grouped {
        candidates.sort_by(|left, right| left.stable_key.cmp(&right.stable_key));
        let first = &candidates[0];
        if candidates.iter().any(|candidate| {
            candidate.source != first.source
                || candidate.named_consumer != first.named_consumer
                || candidate.evidence != first.evidence
        }) {
            return Err(failure(
                Tier2Stage::Deduplicator,
                Tier2FailureCode::DuplicateConflict,
                "duplicate candidates disagree on consumer or evidence",
            ));
        }
        let score_quotients = candidates
            .iter()
            .map(|candidate| Tier2CandidateScore {
                stable_key: candidate.stable_key.clone(),
                score_quotient: score(candidate),
            })
            .collect();
        let winner_index = candidates
            .iter()
            .enumerate()
            .min_by(|(_, left), (_, right)| winner_order(left, right))
            .map(|(index, _)| index)
            .ok_or_else(|| invalid_ranking("deduplication group is empty"))?;
        let winner = candidates[winner_index].clone();
        let candidate_keys = candidates
            .iter()
            .map(|candidate| candidate.stable_key.clone())
            .collect::<Vec<_>>();
        let suppressed_keys = candidate_keys
            .iter()
            .filter(|candidate| *candidate != &winner.stable_key)
            .cloned()
            .collect();
        groups.push(Tier2DeduplicationGroup {
            key,
            candidate_keys,
            winner_key: winner.stable_key.clone(),
            suppressed_keys,
            score_quotients,
        });
        winners.push(Winner {
            quotient: score(&winner),
            proposal: winner,
        });
    }
    Ok((Tier2Deduplication { groups }, winners))
}

fn verify_coverage(
    verifier: &Tier2VerifierVerdicts,
    winners: &[Winner],
) -> Result<BTreeMap<String, Tier2Verification>, Tier2Failure> {
    let expected = winners
        .iter()
        .map(|winner| winner.proposal.stable_key.as_str())
        .collect::<BTreeSet<_>>();
    let mut coverage = BTreeMap::new();
    for verdict in &verifier.verdicts {
        if !bounded_text(verdict.stable_key(), FIELD_SCALAR_LIMIT)
            || !bounded_text(verdict.reason(), REASON_SCALAR_LIMIT)
            || !expected.contains(verdict.stable_key())
            || coverage
                .insert(verdict.stable_key().to_string(), verdict.clone())
                .is_some()
        {
            return Err(invalid_coverage("verdict coverage is invalid"));
        }
    }
    if coverage.len() != expected.len() {
        Err(invalid_coverage("verdict coverage is incomplete"))
    } else {
        Ok(coverage)
    }
}

fn rank(
    winners: &[Winner],
    verdicts: &BTreeMap<String, Tier2Verification>,
    policy: &super::Tier2RoiPolicy,
) -> Result<(Vec<Tier2RoiDecision>, Vec<Tier2RankedProposal>), Tier2Failure> {
    let mut survivors = winners
        .iter()
        .filter(|winner| verdicts[winner.proposal.stable_key.as_str()].survived())
        .collect::<Vec<_>>();
    survivors.sort_by(|left, right| left.proposal.stable_key.cmp(&right.proposal.stable_key));
    let roi = survivors
        .iter()
        .map(|winner| Tier2RoiDecision {
            stable_key: winner.proposal.stable_key.clone(),
            source: winner.proposal.source,
            permitted: policy.permits(winner.proposal.source),
        })
        .collect::<Vec<_>>();
    let mut approved = survivors
        .into_iter()
        .filter(|winner| policy.permits(winner.proposal.source))
        .collect::<Vec<_>>();
    approved.sort_by(rank_order);
    approved.truncate(
        usize::try_from(TIER2_RANK_LIMIT)
            .map_err(|_| invalid_ranking("rank limit does not fit usize"))?,
    );
    let ranked = approved
        .into_iter()
        .enumerate()
        .map(|(index, winner)| ranked_proposal(index, winner))
        .collect::<Result<Vec<_>, _>>()?;
    Ok((roi, ranked))
}

fn ranked_proposal(index: usize, winner: &Winner) -> Result<Tier2RankedProposal, Tier2Failure> {
    let rank = index
        .checked_add(1)
        .and_then(|value| u64::try_from(value).ok())
        .ok_or_else(|| {
            failure(
                Tier2Stage::RoiRank,
                Tier2FailureCode::CountOverflow,
                "rank overflow",
            )
        })?;
    Ok(Tier2RankedProposal {
        proposal: winner.proposal.clone(),
        score_numerator: u64::from(winner.proposal.confidence_millis),
        complexity_units: winner.proposal.complexity.units(),
        score_quotient: winner.quotient,
        severity_rank: winner.proposal.severity.rank(),
        stable_key: winner.proposal.stable_key.clone(),
        named_consumer: winner.proposal.named_consumer.clone(),
        rank,
    })
}

fn funnel(
    observed: u64,
    deduplicated: u64,
    verified: u64,
    roi_approved: u64,
    ranked: u64,
) -> Result<FunnelCounts, Tier2Failure> {
    FunnelCounts::new(observed, deduplicated, verified, roi_approved, ranked).map_err(|detail| {
        failure(
            Tier2Stage::RoiRank,
            Tier2FailureCode::InvalidRanking,
            detail,
        )
    })
}

fn count(value: usize) -> Result<u64, Tier2Failure> {
    u64::try_from(value).map_err(|_| {
        failure(
            Tier2Stage::RoiRank,
            Tier2FailureCode::CountOverflow,
            "count overflow",
        )
    })
}
fn zero_funnel() -> FunnelCounts {
    FunnelCounts::new(0, 0, 0, 0, 0).expect("zero funnel counts are valid")
}
fn empty_partial() -> Tier2PartialEvidence {
    Tier2PartialEvidence::None {
        funnel: zero_funnel(),
    }
}
fn collector_partial(
    collector: &StrictCollectorEvidence,
    funnel: FunnelCounts,
) -> Tier2PartialEvidence {
    Tier2PartialEvidence::Collector {
        collector: collector.clone(),
        funnel,
    }
}
fn generated_partial(
    collector: &StrictCollectorEvidence,
    generated: &Tier2GeneratedProposals,
    funnel: FunnelCounts,
) -> Tier2PartialEvidence {
    Tier2PartialEvidence::Generated {
        collector: collector.clone(),
        generated: generated.clone(),
        funnel,
    }
}
fn dedup_partial(
    collector: &StrictCollectorEvidence,
    generated: &Tier2GeneratedProposals,
    deduplication: &Tier2Deduplication,
    funnel: FunnelCounts,
) -> Tier2PartialEvidence {
    Tier2PartialEvidence::Deduplicated {
        collector: collector.clone(),
        generated: generated.clone(),
        deduplication: deduplication.clone(),
        funnel,
    }
}
fn verified_partial(
    collector: &StrictCollectorEvidence,
    generated: &Tier2GeneratedProposals,
    deduplication: &Tier2Deduplication,
    verification: &Tier2VerifierVerdicts,
    funnel: FunnelCounts,
) -> Tier2PartialEvidence {
    Tier2PartialEvidence::Verified {
        collector: collector.clone(),
        generated: generated.clone(),
        deduplication: deduplication.clone(),
        verification: verification.clone(),
        funnel,
    }
}
fn winner_order(left: &Tier2Proposal, right: &Tier2Proposal) -> std::cmp::Ordering {
    score(right)
        .cmp(&score(left))
        .then_with(|| left.severity.rank().cmp(&right.severity.rank()))
        .then_with(|| left.stable_key.cmp(&right.stable_key))
}
fn rank_order(left: &&Winner, right: &&Winner) -> std::cmp::Ordering {
    left.proposal
        .severity
        .rank()
        .cmp(&right.proposal.severity.rank())
        .then_with(|| right.quotient.cmp(&left.quotient))
        .then_with(|| left.proposal.stable_key.cmp(&right.proposal.stable_key))
}
fn invalid_ranking(detail: impl Into<String>) -> Tier2Failure {
    failure(
        Tier2Stage::RoiRank,
        Tier2FailureCode::InvalidRanking,
        detail,
    )
}
fn failure(stage: Tier2Stage, code: Tier2FailureCode, detail: impl Into<String>) -> Tier2Failure {
    Tier2Failure::initial(stage, code, detail)
}
fn checked_failure(error: Tier2Failure) -> Tier2Failure {
    if bounded_text(&error.detail, REASON_SCALAR_LIMIT) {
        error
    } else {
        failure(error.stage, error.code, "stage failure detail is invalid")
    }
}
#[derive(Debug, Clone)]
struct Winner {
    proposal: Tier2Proposal,
    quotient: u64,
}
