use std::collections::{BTreeMap, BTreeSet};

use crate::autonomous::no_work::DryReason;
use crate::autonomous::waterfall::FunnelCounts;

use super::candidate::{
    deduplicate, failure, validate_generated, validate_roi_policy, validate_source_policy,
    validate_sources, validate_verifier,
};
use super::model::{
    zero_funnel, Tier4Deduplication, Tier4GeneratedCandidates, Tier4Input, Tier4Observation,
    Tier4RankedCandidate, Tier4RoiDecision, Tier4RoiPolicy, Tier4SourceEnvelope, Tier4SourcePolicy,
    Tier4Stage, Tier4StageResult, Tier4Terminal, Tier4Verification, Tier4VerifierVerdicts,
};
use super::{
    Tier4Evaluation, Tier4Failure, Tier4FailureCode, Tier4PartialEvidence, TIER4_RANK_LIMIT,
};

pub fn evaluate_tier4(input: Tier4Input) -> Result<Tier4Evaluation, Tier4Failure> {
    match input {
        Tier4Input::DisabledByCheckedInPolicy => {
            Ok(Tier4Evaluation::NotRun(super::Tier4NotRun::disabled()))
        }
        Tier4Input::Enabled {
            source_policy,
            sources,
            generated,
            verifier,
            roi_policy,
        } => evaluate_enabled(source_policy, sources, generated, verifier, roi_policy)
            .map_err(Tier4Failure::seal),
    }
}

fn evaluate_enabled(
    source_policy: Tier4SourcePolicy,
    sources: Vec<Tier4StageResult<Tier4SourceEnvelope>>,
    generated: Tier4StageResult<Tier4GeneratedCandidates>,
    verifier: Tier4StageResult<Tier4VerifierVerdicts>,
    roi_policy: Tier4RoiPolicy,
) -> Result<Tier4Evaluation, Tier4Failure> {
    validate_source_policy(&source_policy)?;
    let policy_partial =
        Tier4PartialEvidence::after_source_policy(source_policy.clone(), zero_funnel());
    let mut sources = complete_sources(sources, &source_policy, policy_partial.clone())?;
    let facts = validate_sources(&source_policy, &mut sources)
        .map_err(|error| error.with_partial(policy_partial.clone()))?;
    let sources_partial =
        Tier4PartialEvidence::after_sources(source_policy.clone(), sources.clone(), zero_funnel());
    let generated = complete_stage(generated, Tier4Stage::Generator, sources_partial.clone())?;
    validate_generated(&generated, &facts)
        .map_err(|error| error.with_partial(sources_partial.clone()))?;
    let observed = checked_count(generated.candidates.len())
        .map_err(|error| error.with_partial(sources_partial.clone()))?;
    let generated_partial = Tier4PartialEvidence::after_generated(
        source_policy.clone(),
        sources.clone(),
        generated.clone(),
        funnel(observed, 0, 0, 0, 0)
            .map_err(|error| error.with_partial(sources_partial.clone()))?,
    );
    let deduplication = deduplicate(&generated.candidates)
        .map_err(|error| error.with_partial(generated_partial.clone()))?;
    let deduplicated = checked_count(deduplication.groups.len())
        .map_err(|error| error.with_partial(generated_partial.clone()))?;
    let dedup_partial = Tier4PartialEvidence::after_deduplication(
        source_policy.clone(),
        sources.clone(),
        generated.clone(),
        deduplication.clone(),
        funnel(observed, deduplicated, 0, 0, 0)
            .map_err(|error| error.with_partial(generated_partial.clone()))?,
    );
    let verifier = complete_stage(verifier, Tier4Stage::Verifier, dedup_partial.clone())?;
    let expected = deduplication
        .groups
        .iter()
        .map(|group| group.stable_key.clone())
        .collect::<BTreeSet<_>>();
    let verdicts = validate_verifier(&verifier, &expected)
        .map_err(|error| error.with_partial(dedup_partial.clone()))?;
    let verified = checked_count(
        verdicts
            .values()
            .filter(|verdict| verdict.accepted())
            .count(),
    )
    .map_err(|error| error.with_partial(dedup_partial.clone()))?;
    let verified_partial = Tier4PartialEvidence::after_verification(
        source_policy.clone(),
        sources.clone(),
        generated.clone(),
        deduplication.clone(),
        verifier.clone(),
        funnel(observed, deduplicated, verified, 0, 0)
            .map_err(|error| error.with_partial(dedup_partial.clone()))?,
    );
    validate_roi_policy(roi_policy)
        .map_err(|error| error.with_partial(verified_partial.clone()))?;
    let (roi, ranked) = rank(&deduplication, &verdicts, roi_policy)
        .map_err(|error| error.with_partial(verified_partial.clone()))?;
    let roi_approved = checked_count(roi.iter().filter(|decision| decision.permitted).count())
        .map_err(|error| error.with_partial(verified_partial.clone()))?;
    let ranked_count =
        checked_count(ranked.len()).map_err(|error| error.with_partial(verified_partial))?;
    let funnel = funnel(observed, deduplicated, verified, roi_approved, ranked_count)?;
    let terminal = terminal(observed, verified, roi_approved, ranked_count);

    Ok(Tier4Evaluation::Complete(Tier4Observation {
        source_policy,
        sources,
        generated,
        deduplication,
        verification: verifier,
        roi,
        ranked,
        funnel,
        terminal,
    }))
}

fn complete_sources(
    results: Vec<Tier4StageResult<Tier4SourceEnvelope>>,
    policy: &Tier4SourcePolicy,
    partial: Tier4PartialEvidence,
) -> Result<Vec<Tier4SourceEnvelope>, Tier4Failure> {
    if results.len() != policy.descriptors.len() {
        return Err(failure(
            Tier4Stage::Sources,
            Tier4FailureCode::InvalidSourceCoverage,
            "source stage result count does not match source policy",
        )
        .with_partial(partial));
    }
    results
        .into_iter()
        .map(|result| complete_stage(result, Tier4Stage::Sources, partial.clone()))
        .collect()
}

fn complete_stage<T>(
    result: Tier4StageResult<T>,
    stage: Tier4Stage,
    partial: Tier4PartialEvidence,
) -> Result<T, Tier4Failure> {
    match result {
        Tier4StageResult::Complete(value) => Ok(value),
        Tier4StageResult::Failed(error) => Err(error.rebind(stage).with_partial(partial)),
        Tier4StageResult::Missing => Err(failure(
            stage,
            Tier4FailureCode::MissingStageResult,
            "stage result was not supplied",
        )
        .with_partial(partial)),
    }
}

fn rank(
    deduplication: &Tier4Deduplication,
    verdicts: &BTreeMap<String, Tier4Verification>,
    policy: Tier4RoiPolicy,
) -> Result<(Vec<Tier4RoiDecision>, Vec<Tier4RankedCandidate>), Tier4Failure> {
    let mut roi = Vec::with_capacity(deduplication.groups.len());
    let mut approved = Vec::new();
    for group in &deduplication.groups {
        let verdict = verdicts.get(&group.stable_key).ok_or_else(|| {
            failure(
                Tier4Stage::Verifier,
                Tier4FailureCode::InvalidVerdictCoverage,
                "deduplicated candidate is missing a verifier verdict",
            )
        })?;
        let verified = verdict.accepted();
        let roi_millis = verdict.roi_millis();
        let permitted = roi_millis.is_some_and(|roi| roi >= policy.threshold_millis);
        roi.push(Tier4RoiDecision {
            stable_key: group.stable_key.clone(),
            verified,
            roi_millis,
            permitted,
            reason: verdict.reason().to_string(),
        });
        if let Some(roi_millis) = roi_millis.filter(|roi| *roi >= policy.threshold_millis) {
            approved.push((group, roi_millis));
        }
    }
    approved.sort_by(|(left, left_roi), (right, right_roi)| {
        right_roi
            .cmp(left_roi)
            .then_with(|| left.stable_key.cmp(&right.stable_key))
    });
    approved.truncate(rank_limit()?);
    let ranked = approved
        .into_iter()
        .enumerate()
        .map(|(index, (group, roi_millis))| {
            let rank = index
                .checked_add(1)
                .and_then(|value| u64::try_from(value).ok())
                .ok_or_else(|| {
                    failure(
                        Tier4Stage::RoiRank,
                        Tier4FailureCode::CountOverflow,
                        "rank cannot be represented as u64",
                    )
                })?;
            Ok(Tier4RankedCandidate {
                rank,
                stable_key: group.stable_key.clone(),
                roi_millis,
                title: group.title.clone(),
                rationale: group.rationale.clone(),
                references: group.references.clone(),
            })
        })
        .collect::<Result<Vec<_>, Tier4Failure>>()?;
    Ok((roi, ranked))
}

fn terminal(observed: u64, verified: u64, roi_approved: u64, ranked: u64) -> Tier4Terminal {
    if observed == 0 {
        Tier4Terminal::Exhausted {
            reason: DryReason::NoProposalsGenerated,
        }
    } else if verified == 0 {
        Tier4Terminal::Exhausted {
            reason: DryReason::VerificationRejected,
        }
    } else if roi_approved == 0 {
        Tier4Terminal::Exhausted {
            reason: DryReason::RoiFiltered,
        }
    } else {
        Tier4Terminal::Produced { count: ranked }
    }
}

fn funnel(
    observed: u64,
    deduplicated: u64,
    verified: u64,
    roi_approved: u64,
    ranked: u64,
) -> Result<FunnelCounts, Tier4Failure> {
    FunnelCounts::new(observed, deduplicated, verified, roi_approved, ranked).map_err(|detail| {
        failure(
            Tier4Stage::RoiRank,
            Tier4FailureCode::InvalidRanking,
            detail,
        )
    })
}

fn checked_count(value: usize) -> Result<u64, Tier4Failure> {
    u64::try_from(value).map_err(|_| {
        failure(
            Tier4Stage::RoiRank,
            Tier4FailureCode::CountOverflow,
            "count cannot be represented as u64",
        )
    })
}

fn rank_limit() -> Result<usize, Tier4Failure> {
    usize::try_from(TIER4_RANK_LIMIT).map_err(|_| {
        failure(
            Tier4Stage::RoiRank,
            Tier4FailureCode::InvalidRanking,
            "rank limit cannot be represented as usize",
        )
    })
}
