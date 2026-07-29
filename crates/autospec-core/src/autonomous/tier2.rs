mod evidence;
mod funnel;
mod funnel_validation;
mod model;
mod partial;

pub use evidence::Tier2EvidenceDocuments;
pub use model::{
    StrictCollectorEvidence, Tier2CandidateScore, Tier2Complexity, Tier2Deduplication,
    Tier2DeduplicationGroup, Tier2Evaluation, Tier2ExclusionPolicy, Tier2ExclusionReport,
    Tier2Failure, Tier2FailureCode, Tier2GeneratedProposals, Tier2Input, Tier2NotRun,
    Tier2Observation, Tier2PollutionCode, Tier2PollutionFinding, Tier2Proposal,
    Tier2RankedProposal, Tier2RoiDecision, Tier2RoiPolicy, Tier2Severity, Tier2Source, Tier2Stage,
    Tier2StageResult, Tier2Verification, Tier2VerifierVerdicts, TIER2_NORMALIZATION_VERSION,
    TIER2_RANK_LIMIT, TIER2_SCHEMA,
};
pub use partial::Tier2PartialEvidence;

pub const DISABLED_REASON: &str = "tier2_local_discovery_disabled_by_policy";

impl Default for Tier2ExclusionPolicy {
    fn default() -> Self {
        Self::with_repository_additions(std::iter::empty::<&str>())
            .expect("checked-in Tier 2 exclusions are valid")
    }
}

pub fn evaluate_tier2(input: Tier2Input) -> Result<Tier2Evaluation, Tier2Failure> {
    evaluate_tier2_with_policy(input, Tier2ExclusionPolicy::default())
}

pub fn evaluate_tier2_with_policy(
    input: Tier2Input,
    policy: Tier2ExclusionPolicy,
) -> Result<Tier2Evaluation, Tier2Failure> {
    let (input, report) = apply_exclusions(input, &policy)?;
    funnel::evaluate_tier2(input, report.clone())
        .map_err(|failure| failure.with_exclusion_report(report))
}

fn apply_exclusions(
    input: Tier2Input,
    policy: &Tier2ExclusionPolicy,
) -> Result<(Tier2Input, Tier2ExclusionReport), Tier2Failure> {
    let Tier2Input::Enabled {
        collector,
        generator,
        verifier,
        roi_policy,
    } = input
    else {
        return Ok((input, empty_report(policy)));
    };
    let collector = match collector {
        Tier2StageResult::Complete(collector) => {
            let (collector, report) = filter_collector(collector, policy)?;
            return Ok((
                Tier2Input::Enabled {
                    collector: Tier2StageResult::Complete(collector),
                    generator,
                    verifier,
                    roi_policy,
                },
                report,
            ));
        }
        other => other,
    };
    Ok((
        Tier2Input::Enabled {
            collector,
            generator,
            verifier,
            roi_policy,
        },
        empty_report(policy),
    ))
}

fn filter_collector(
    mut collector: StrictCollectorEvidence,
    policy: &Tier2ExclusionPolicy,
) -> Result<(StrictCollectorEvidence, Tier2ExclusionReport), Tier2Failure> {
    let mut findings = Vec::new();
    for domain in &mut collector.domains {
        let findings_before = findings.len();
        for evidence in &domain.evidence {
            validate_repository_relative(&evidence.file)?;
        }
        domain.evidence.retain(|evidence| {
            let Some(component) = policy.matching_component(&evidence.file) else {
                return true;
            };
            findings.push(Tier2PollutionFinding {
                path: evidence.file.clone(),
                excluded_component: component.to_string(),
                finding: Tier2PollutionCode::ProhibitedVendorPath,
            });
            false
        });
        domain.score = domain
            .score
            .saturating_sub(findings.len() - findings_before)
            .max(usize::from(!domain.evidence.is_empty()));
    }
    collector
        .domains
        .retain(|domain| !domain.evidence.is_empty());
    let excluded_path_count = u64::try_from(findings.len()).map_err(|_| {
        Tier2Failure::initial(
            Tier2Stage::Collector,
            Tier2FailureCode::CountOverflow,
            "excluded path count exceeds u64",
        )
    })?;
    Ok((
        collector,
        Tier2ExclusionReport {
            policy_digest: policy.digest().to_string(),
            excluded_path_count,
            pollution_findings: findings,
        },
    ))
}

fn validate_repository_relative(path: &str) -> Result<(), Tier2Failure> {
    let valid = !path.starts_with('/')
        && !path.contains('\\')
        && path
            .split('/')
            .all(|component| !component.is_empty() && !matches!(component, "." | ".."));
    if valid {
        return Ok(());
    }
    Err(Tier2Failure::initial(
        Tier2Stage::Collector,
        Tier2FailureCode::PathEscapesRoot,
        "collector evidence path is outside repository root",
    )
    .seal())
}

fn empty_report(policy: &Tier2ExclusionPolicy) -> Tier2ExclusionReport {
    Tier2ExclusionReport {
        policy_digest: policy.digest().to_string(),
        excluded_path_count: 0,
        pollution_findings: Vec::new(),
    }
}
