use std::collections::BTreeMap;

use crate::autonomous::waterfall::FunnelCounts;

use super::model::{
    bounded_text, Tier3AdapterEvidence, Tier3Evaluation, Tier3Failure, Tier3FailureCode,
    Tier3Finding, Tier3FindingKind, Tier3Input, Tier3Observation, Tier3PartialEvidence, Tier3Stage,
    Tier3StageResult, FIELD_LIMIT, TIER3_RANK_LIMIT, TIER3_SCHEMA,
};

pub fn evaluate_tier3(input: Tier3Input) -> Result<Tier3Evaluation, Tier3Failure> {
    match input {
        Tier3Input::DisabledByCheckedInPolicy => {
            Ok(Tier3Evaluation::NotRun(super::Tier3NotRun::disabled()))
        }
        Tier3Input::Enabled {
            architecture,
            coverage,
            debt,
        } => evaluate_enabled(architecture, coverage, debt).map_err(Tier3Failure::seal),
    }
}

fn evaluate_enabled(
    architecture: Tier3StageResult<Tier3AdapterEvidence>,
    coverage: Tier3StageResult<Tier3AdapterEvidence>,
    debt: Tier3StageResult<Tier3AdapterEvidence>,
) -> Result<Tier3Evaluation, Tier3Failure> {
    let architecture = complete(architecture, Tier3Stage::Architecture, no_predecessors())?;
    validate_adapter(
        &architecture,
        Tier3FindingKind::Architecture,
        Tier3Stage::Architecture,
    )?;
    let architecture_partial = Tier3PartialEvidence::architecture_complete(architecture.clone());
    let coverage = complete(coverage, Tier3Stage::Coverage, architecture_partial.clone())?;
    validate_adapter(&coverage, Tier3FindingKind::Coverage, Tier3Stage::Coverage)
        .map_err(|error| error.with_partial(architecture_partial.clone()))?;
    let coverage_partial =
        Tier3PartialEvidence::coverage_complete(architecture.clone(), coverage.clone());
    let debt = complete(debt, Tier3Stage::Debt, coverage_partial.clone())?;
    validate_adapter(&debt, Tier3FindingKind::Debt, Tier3Stage::Debt)
        .map_err(|error| error.with_partial(coverage_partial.clone()))?;

    let completed_partial = Tier3PartialEvidence::complete(
        architecture.clone(),
        coverage.clone(),
        debt.clone(),
        FunnelCounts::new(0, 0, 0, 0, 0).expect("zero funnel counts are valid"),
    );
    let observed = checked_count(
        architecture
            .findings
            .len()
            .checked_add(coverage.findings.len())
            .and_then(|count| count.checked_add(debt.findings.len()))
            .ok_or_else(|| {
                failure(
                    Tier3Stage::Ranking,
                    Tier3FailureCode::CountOverflow,
                    "finding count overflow",
                )
            })?,
    )
    .map_err(|error| error.with_partial(completed_partial.clone()))?;
    let all = architecture
        .findings
        .iter()
        .chain(&coverage.findings)
        .chain(&debt.findings)
        .cloned()
        .collect::<Vec<_>>();
    let deduplicated =
        deduplicate(all).map_err(|error| error.with_partial(completed_partial.clone()))?;
    let deduplicated_count = checked_count(deduplicated.len())
        .map_err(|error| error.with_partial(completed_partial.clone()))?;
    let ranked = rank(deduplicated.clone())
        .map_err(|error| error.with_partial(completed_partial.clone()))?;
    let ranked_count = checked_count(ranked.len())
        .map_err(|error| error.with_partial(completed_partial.clone()))?;
    let funnel = FunnelCounts::new(
        observed,
        deduplicated_count,
        deduplicated_count,
        deduplicated_count,
        ranked_count,
    )
    .map_err(|detail| {
        failure(Tier3Stage::Ranking, Tier3FailureCode::CountOverflow, detail)
            .with_partial(completed_partial)
    })?;
    Ok(Tier3Evaluation::Complete(Tier3Observation {
        architecture,
        coverage,
        debt,
        deduplicated,
        ranked,
        funnel,
    }))
}

fn complete<T>(
    result: Tier3StageResult<T>,
    stage: Tier3Stage,
    partial: Tier3PartialEvidence,
) -> Result<T, Tier3Failure> {
    match result {
        Tier3StageResult::Complete(value) => Ok(value),
        Tier3StageResult::Failed(error) => Err(error.rebind(stage).with_partial(partial)),
        Tier3StageResult::Missing => Err(failure(
            stage,
            Tier3FailureCode::MissingStageResult,
            "stage result was not supplied",
        )
        .with_partial(partial)),
    }
}

fn validate_adapter(
    adapter: &Tier3AdapterEvidence,
    expected_kind: Tier3FindingKind,
    stage: Tier3Stage,
) -> Result<(), Tier3Failure> {
    if adapter.schema_version != TIER3_SCHEMA
        || !bounded_text(&adapter.adapter_version, FIELD_LIMIT)
        || !bounded_text(&adapter.rule_version, FIELD_LIMIT)
    {
        return Err(failure(
            stage,
            Tier3FailureCode::InvalidAdapterEvidence,
            "adapter schema or identity is invalid",
        ));
    }
    let mut previous = None;
    let mut identities = BTreeMap::new();
    for finding in &adapter.findings {
        if finding.kind != expected_kind {
            return Err(failure(
                stage,
                Tier3FailureCode::WrongFindingKind,
                "finding kind does not match its adapter stage",
            ));
        }
        if !valid_finding(finding) {
            return Err(failure(
                stage,
                Tier3FailureCode::InvalidFinding,
                "finding fields are invalid",
            ));
        }
        let order = finding_order(finding);
        if previous.as_ref().is_some_and(|previous| order <= *previous) {
            return Err(failure(
                stage,
                Tier3FailureCode::NonCanonicalOrder,
                "findings must be sorted uniquely by canonical rank order",
            ));
        }
        previous = Some(order);
        let identity = finding_identity(finding);
        if let Some(existing) = identities.insert(identity, finding.clone()) {
            if existing != *finding {
                return Err(failure(
                    stage,
                    Tier3FailureCode::DuplicateConflict,
                    "finding identity has conflicting evidence",
                ));
            }
        }
    }
    Ok(())
}

fn valid_finding(finding: &Tier3Finding) -> bool {
    bounded_text(&finding.rule_id, FIELD_LIMIT)
        && bounded_text(&finding.message, FIELD_LIMIT)
        && valid_relative_path(&finding.path)
        && finding.line > 0
}

fn valid_relative_path(path: &str) -> bool {
    if !bounded_text(path, FIELD_LIMIT) || path.starts_with('/') || path.starts_with('\\') {
        return false;
    }
    path.split('/').all(|part| {
        !part.is_empty()
            && part != "."
            && part != ".."
            && !part.contains('\\')
            && !part.contains(':')
    })
}

fn deduplicate(findings: Vec<Tier3Finding>) -> Result<Vec<Tier3Finding>, Tier3Failure> {
    let mut rows = BTreeMap::new();
    for finding in findings {
        let key = finding_dedup_key(&finding);
        if rows.insert(key, finding).is_some() {
            return Err(failure(
                Tier3Stage::Ranking,
                Tier3FailureCode::DuplicateConflict,
                "validated findings duplicated during normalization",
            ));
        }
    }
    Ok(rows.into_values().collect())
}

fn rank(mut findings: Vec<Tier3Finding>) -> Result<Vec<Tier3Finding>, Tier3Failure> {
    findings.sort_by_key(finding_order);
    let limit = usize::try_from(TIER3_RANK_LIMIT).map_err(|_| {
        failure(
            Tier3Stage::Ranking,
            Tier3FailureCode::CountOverflow,
            "rank limit does not fit usize",
        )
    })?;
    findings.truncate(limit);
    Ok(findings)
}

fn checked_count(value: usize) -> Result<u64, Tier3Failure> {
    u64::try_from(value).map_err(|_| {
        failure(
            Tier3Stage::Ranking,
            Tier3FailureCode::CountOverflow,
            "finding count does not fit u64",
        )
    })
}

fn finding_order(finding: &Tier3Finding) -> (u64, String, String, u64, String, Tier3FindingKind) {
    (
        finding.severity.rank(),
        finding.rule_id.clone(),
        finding.path.clone(),
        finding.line,
        finding.message.clone(),
        finding.kind,
    )
}

fn finding_identity(finding: &Tier3Finding) -> (Tier3FindingKind, String, String, u64, String) {
    (
        finding.kind,
        finding.rule_id.clone(),
        finding.path.clone(),
        finding.line,
        finding.message.clone(),
    )
}

fn finding_dedup_key(finding: &Tier3Finding) -> (Tier3FindingKind, String, String, u64, String) {
    (
        finding.kind,
        finding.rule_id.clone(),
        finding.path.clone(),
        finding.line,
        finding.message.clone(),
    )
}

fn failure(stage: Tier3Stage, code: Tier3FailureCode, detail: impl Into<String>) -> Tier3Failure {
    Tier3Failure::new(stage, code, detail).expect("Tier 3 evaluator emits bounded failures")
}

fn no_predecessors() -> Tier3PartialEvidence {
    Tier3PartialEvidence::none()
}
