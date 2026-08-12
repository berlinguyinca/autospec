use super::*;
use autospec_core::autonomous::review_policy::{ReviewReasoning, ReviewRisk};

const REVIEW_RECEIPT_SCHEMA: u32 = 5;
const REVIEW_RECEIPT_FIELDS: &[&str] = &[
    "schema",
    "binding",
    "stdout_path",
    "stdout_digest",
    "stderr_path",
    "stderr_digest",
    "normalizer_path",
    "normalizer_digest",
    "inner_stdout_path",
    "inner_stdout_digest",
    "inner_stderr_path",
    "inner_stderr_digest",
    "result_path",
    "result_digest",
    "review_commit",
    "review_risk",
    "reviewer_harness",
    "reviewer_reasoning",
    "integration_shaped",
    "require_integration_smoke",
    "prefer_provider_diversity",
    "require_provider_diversity",
    "review_reasons",
    "provider_diversified",
    "selection_reason",
    "requirements_digest",
    "policy_digest",
    "changed_paths",
    "logical_components",
    "producer_surfaces",
    "consumer_surfaces",
    "integration_evidence_digest",
    "integration_command_records",
    "review_context_digest",
    "verdict_schema",
    "verdict",
    "surfaces_examined",
    "tests_examined",
    "integration_paths_checked",
    "blocking_findings",
    "verdict_digest",
];

pub(super) fn recover_existing_review_receipt(
    state_path: &Path,
    state: &mut PersistedInvocation,
) -> Result<bool, BridgeRunFailure> {
    let path = review_receipt_path(state_path, state)?;
    if !path.exists() {
        return Ok(false);
    }
    let schema = review_receipt_schema(&path)?;
    if schema != REVIEW_RECEIPT_SCHEMA {
        archive_legacy_review_receipt(&path, schema)?;
        state.phase = BridgePhase::CiPassed;
        state.progress_at = unix_now()?;
        write_invocation_atomic(state_path, state).map_err(BridgeRunFailure::invariant)?;
        return Ok(false);
    }
    validate_review_receipt(state_path, state)?;
    state.phase = BridgePhase::ReviewPassed;
    state.progress_at = unix_now()?;
    write_invocation_atomic(state_path, state).map_err(BridgeRunFailure::invariant)?;
    Ok(true)
}

fn review_receipt_schema(path: &Path) -> Result<u32, String> {
    validate_private_state_file(path)?;
    let value: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(path)
            .map_err(|error| format!("read executor independent review receipt: {error}"))?,
    )
    .map_err(|error| format!("parse executor independent review receipt: {error}"))?;
    value
        .as_object()
        .and_then(|object| object.get("schema"))
        .and_then(serde_json::Value::as_u64)
        .and_then(|schema| u32::try_from(schema).ok())
        .ok_or_else(|| "executor independent review receipt schema is invalid".to_string())
}

fn archive_legacy_review_receipt(path: &Path, schema: u32) -> Result<(), String> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "executor review receipt name is invalid".to_string())?;
    let archive = path.with_file_name(format!("{name}.legacy-schema-{schema}"));
    if archive.exists() {
        return Err("executor legacy review receipt archive already exists".to_string());
    }
    fs::rename(path, archive)
        .map_err(|error| format!("archive legacy executor review receipt: {error}"))
}

pub(super) fn review_binding(state: &PersistedInvocation) -> Result<String, String> {
    let head = review_head(state)?;
    Ok(sha256_hex(
        format!(
            "{}\0{}\0{}\0{}\0{}\0{}",
            state.identity.repository,
            state.identity.issue,
            state.identity.worker_id,
            state.identity.claim_id,
            state.identity.branch,
            head
        )
        .as_bytes(),
    ))
}

fn review_head(state: &PersistedInvocation) -> Result<&str, String> {
    state
        .head_oid
        .as_deref()
        .filter(|head| head.len() == 40 && head.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or_else(|| "executor review receipt requires a canonical head".to_string())
}

pub(super) fn review_receipt_path(
    state_path: &Path,
    state: &PersistedInvocation,
) -> Result<PathBuf, String> {
    Ok(state_path.with_extension(format!("review-{}.json", review_head(state)?)))
}

pub(super) fn canonical_review_policy_digest(policy: &ResolvedReviewPolicy) -> String {
    sha256_hex(
        format!(
            "review-policy-v1\0{}\0{}\0{}\0{}",
            canonical_review_requirements_digest(&policy.requirements),
            policy.reviewer_harness.as_str(),
            policy.provider_diversified,
            policy.selection_reason,
        )
        .as_bytes(),
    )
}

pub(super) fn canonical_review_context_digest(
    policy: &ResolvedReviewPolicy,
    evidence: &BoundReviewEvidence,
) -> String {
    sha256_hex(
        format!(
            "review-context-v1\0{}\0{}\0{}\0{}\0{}\0{}\0{}\0{}\0{}",
            canonical_review_policy_digest(policy),
            evidence.commit,
            evidence.requirements_digest,
            evidence.inventory.changed_paths.join("\0"),
            evidence.inventory.logical_components.join("\0"),
            evidence.inventory.producer_surfaces.join("\0"),
            evidence.inventory.consumer_surfaces.join("\0"),
            evidence
                .integration_evidence_digest
                .as_deref()
                .unwrap_or(""),
            evidence.integration_command_records.join("\0"),
        )
        .as_bytes(),
    )
}

pub(super) fn write_review_receipt(
    state_path: &Path,
    state: &PersistedInvocation,
    observation: &ObservedDirectCommand,
    reviewer: &IndependentReviewer,
    verdict: &ReviewVerdict,
) -> Result<(), String> {
    let automatic = reviewer.automatic.as_ref().ok_or_else(|| {
        "executor production review receipt requires structured automatic evidence".to_string()
    })?;
    let requirements = &reviewer.policy.requirements;
    let evidence =
        load_bound_review_evidence(state, requirements, executor_review_inventory(state)?)?;
    let body = serde_json::json!({
        "schema": REVIEW_RECEIPT_SCHEMA, "binding": review_binding(state)?,
        "stdout_path": observation.stdout_path, "stdout_digest": observation.stdout_digest,
        "stderr_path": observation.stderr_path, "stderr_digest": observation.stderr_digest,
        "normalizer_path": automatic.normalizer,
        "normalizer_digest": private_reviewer_artifact_digest(&automatic.normalizer)?,
        "inner_stdout_path": automatic.inner_stdout,
        "inner_stdout_digest": private_reviewer_artifact_digest(&automatic.inner_stdout)?,
        "inner_stderr_path": automatic.inner_stderr,
        "inner_stderr_digest": private_reviewer_artifact_digest(&automatic.inner_stderr)?,
        "result_path": automatic.result,
        "result_digest": private_reviewer_artifact_digest(&automatic.result)?,
        "review_commit": verdict.commit, "review_risk": risk_name(requirements.risk),
        "reviewer_harness": reviewer.policy.reviewer_harness.as_str(),
        "reviewer_reasoning": reasoning_name(requirements.reviewer_reasoning),
        "integration_shaped": requirements.integration_shaped,
        "require_integration_smoke": requirements.require_integration_smoke,
        "prefer_provider_diversity": requirements.prefer_provider_diversity,
        "require_provider_diversity": requirements.require_provider_diversity,
        "review_reasons": requirements.reasons,
        "provider_diversified": reviewer.policy.provider_diversified,
        "selection_reason": reviewer.policy.selection_reason,
        "requirements_digest": canonical_review_requirements_digest(requirements),
        "policy_digest": canonical_review_policy_digest(&reviewer.policy),
        "changed_paths": evidence.inventory.changed_paths,
        "logical_components": evidence.inventory.logical_components,
        "producer_surfaces": evidence.inventory.producer_surfaces,
        "consumer_surfaces": evidence.inventory.consumer_surfaces,
        "integration_evidence_digest": evidence.integration_evidence_digest,
        "integration_command_records": evidence.integration_command_records,
        "review_context_digest": canonical_review_context_digest(&reviewer.policy, &evidence),
        "verdict_schema": verdict.schema, "verdict": verdict.verdict,
        "surfaces_examined": verdict.surfaces_examined, "tests_examined": verdict.tests_examined,
        "integration_paths_checked": verdict.integration_paths_checked,
        "blocking_findings": verdict.blocking_findings,
        "verdict_digest": canonical_review_verdict_digest(verdict),
    });
    write_private_create_once(
        &review_receipt_path(state_path, state)?,
        format!("{body}\n").as_bytes(),
        "executor independent review receipt",
    )
}

pub(super) fn validate_review_receipt(
    state_path: &Path,
    state: &PersistedInvocation,
) -> Result<(), String> {
    let path = review_receipt_path(state_path, state)?;
    validate_private_state_file(&path)?;
    let value: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(&path)
            .map_err(|error| format!("read executor independent review receipt: {error}"))?,
    )
    .map_err(|error| format!("parse executor independent review receipt: {error}"))?;
    let object = strict_object(
        value,
        REVIEW_RECEIPT_FIELDS,
        "executor independent review receipt",
    )?;
    if checked_u32(&object, "schema")? != REVIEW_RECEIPT_SCHEMA
        || text(&object, "binding")? != review_binding(state)?
        || text(&object, "review_commit")? != review_head(state)?
    {
        return Err("executor independent review receipt identity mismatch".to_string());
    }
    validate_receipt_artifacts(&object)?;
    let requirements = receipt_requirements(&object)?;
    let requirements_digest = canonical_review_requirements_digest(&requirements);
    if requirements_digest != text(&object, "requirements_digest")? {
        return Err("executor independent review requirements digest mismatch".to_string());
    }
    let policy = ResolvedReviewPolicy {
        requirements,
        reviewer_harness: HarnessKind::parse(&text(&object, "reviewer_harness")?)?,
        provider_diversified: boolean(&object, "provider_diversified")?,
        selection_reason: text(&object, "selection_reason")?,
    };
    if canonical_review_policy_digest(&policy) != text(&object, "policy_digest")? {
        return Err("executor independent review policy digest mismatch".to_string());
    }
    let receipt_evidence = receipt_review_evidence(&object)?;
    if receipt_evidence.commit != review_head(state)?
        || receipt_evidence.requirements_digest != requirements_digest
        || canonical_review_context_digest(&policy, &receipt_evidence)
            != text(&object, "review_context_digest")?
    {
        return Err("executor independent review context digest mismatch".to_string());
    }
    let live_evidence = load_bound_review_evidence(
        state,
        &policy.requirements,
        executor_review_inventory(state)?,
    )?;
    if live_evidence != receipt_evidence {
        return Err("executor independent review context changed after admission".to_string());
    }
    let expected_integration_citations = receipt_evidence.integration_citations();
    let verdict = receipt_verdict(&object)?;
    validate_review_verdict(
        &verdict,
        review_head(state)?,
        &expected_integration_citations,
    )?;
    if canonical_review_verdict_digest(&verdict) != text(&object, "verdict_digest")? {
        return Err("executor independent review verdict digest mismatch".to_string());
    }
    let raw = fs::read_to_string(PathBuf::from(text(&object, "result_path")?))
        .map_err(|error| format!("read executor structured reviewer result: {error}"))?;
    if parse_review_verdict(&raw, review_head(state)?, &expected_integration_citations)? != verdict
    {
        return Err("executor independent review semantic result mismatch".to_string());
    }
    Ok(())
}

fn receipt_review_evidence(object: &JsonObject) -> Result<BoundReviewEvidence, String> {
    let integration_evidence_digest = match required(object, "integration_evidence_digest")? {
        serde_json::Value::Null => None,
        serde_json::Value::String(value) if !value.trim().is_empty() => Some(value.clone()),
        _ => {
            return Err(
                "executor independent review integration evidence digest is invalid".to_string(),
            );
        }
    };
    Ok(BoundReviewEvidence {
        commit: text(object, "review_commit")?,
        inventory: ExecutorReviewInventory {
            changed_paths: receipt_strings(object, "changed_paths")?,
            logical_components: receipt_strings(object, "logical_components")?,
            producer_surfaces: receipt_strings(object, "producer_surfaces")?,
            consumer_surfaces: receipt_strings(object, "consumer_surfaces")?,
        },
        requirements_digest: text(object, "requirements_digest")?,
        integration_evidence_digest,
        integration_command_records: receipt_strings(object, "integration_command_records")?,
    })
}

fn validate_receipt_artifacts(object: &JsonObject) -> Result<(), String> {
    let stdout = PathBuf::from(text(object, "stdout_path")?);
    let output = fs::read_to_string(&stdout)
        .map_err(|error| format!("read executor independent review artifact: {error}"))?;
    if private_reviewer_artifact_digest(&stdout)? != text(object, "stdout_digest")? {
        return Err("executor independent review artifact digest mismatch".to_string());
    }
    strict_lgtm(&output)?;
    let stderr = PathBuf::from(text(object, "stderr_path")?);
    if private_reviewer_artifact_digest(&stderr)? != text(object, "stderr_digest")?
        || fs::metadata(&stderr)
            .map_err(|error| error.to_string())?
            .len()
            != 0
    {
        return Err("executor independent review stderr artifact is not empty".to_string());
    }
    for (path_field, digest_field) in [
        ("normalizer_path", "normalizer_digest"),
        ("inner_stdout_path", "inner_stdout_digest"),
        ("inner_stderr_path", "inner_stderr_digest"),
        ("result_path", "result_digest"),
    ] {
        let path = PathBuf::from(text(object, path_field)?);
        if private_reviewer_artifact_digest(&path)? != text(object, digest_field)? {
            return Err(format!(
                "executor independent review {path_field} digest mismatch"
            ));
        }
    }
    Ok(())
}

fn receipt_requirements(object: &JsonObject) -> Result<ReviewRequirements, String> {
    Ok(ReviewRequirements {
        risk: parse_risk(&text(object, "review_risk")?)?,
        reviewer_reasoning: parse_reasoning(&text(object, "reviewer_reasoning")?)?,
        integration_shaped: boolean(object, "integration_shaped")?,
        require_integration_smoke: boolean(object, "require_integration_smoke")?,
        prefer_provider_diversity: boolean(object, "prefer_provider_diversity")?,
        require_provider_diversity: boolean(object, "require_provider_diversity")?,
        reasons: receipt_strings(object, "review_reasons")?,
    })
}

fn receipt_verdict(object: &JsonObject) -> Result<ReviewVerdict, String> {
    Ok(ReviewVerdict {
        schema: checked_u32(object, "verdict_schema")?,
        commit: text(object, "review_commit")?,
        verdict: text(object, "verdict")?,
        surfaces_examined: receipt_strings(object, "surfaces_examined")?,
        tests_examined: receipt_strings(object, "tests_examined")?,
        integration_paths_checked: receipt_strings(object, "integration_paths_checked")?,
        blocking_findings: receipt_strings(object, "blocking_findings")?,
    })
}

fn receipt_strings(object: &JsonObject, field: &str) -> Result<Vec<String>, String> {
    required(object, field)?
        .as_array()
        .ok_or_else(|| format!("{field} must be an array"))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| format!("{field} must contain strings"))
        })
        .collect()
}

fn boolean(object: &JsonObject, field: &str) -> Result<bool, String> {
    required(object, field)?
        .as_bool()
        .ok_or_else(|| format!("{field} must be a boolean"))
}

fn risk_name(risk: ReviewRisk) -> &'static str {
    match risk {
        ReviewRisk::Normal => "normal",
        ReviewRisk::High => "high",
        ReviewRisk::Integration => "integration",
        ReviewRisk::Critical => "critical",
    }
}

fn reasoning_name(reasoning: ReviewReasoning) -> &'static str {
    match reasoning {
        ReviewReasoning::Standard => "standard",
        ReviewReasoning::High => "high",
    }
}

fn parse_risk(value: &str) -> Result<ReviewRisk, String> {
    match value {
        "normal" => Ok(ReviewRisk::Normal),
        "high" => Ok(ReviewRisk::High),
        "integration" => Ok(ReviewRisk::Integration),
        "critical" => Ok(ReviewRisk::Critical),
        _ => Err("executor independent review risk is invalid".to_string()),
    }
}

fn parse_reasoning(value: &str) -> Result<ReviewReasoning, String> {
    match value {
        "standard" => Ok(ReviewReasoning::Standard),
        "high" => Ok(ReviewReasoning::High),
        _ => Err("executor independent review reasoning is invalid".to_string()),
    }
}
