use super::*;

pub(super) fn receipt_review_evidence(object: &JsonObject) -> Result<BoundReviewEvidence, String> {
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

pub(super) fn validate_receipt_artifacts(object: &JsonObject) -> Result<(), String> {
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

pub(super) fn receipt_requirements(object: &JsonObject) -> Result<ReviewRequirements, String> {
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

pub(super) fn receipt_verdict(object: &JsonObject) -> Result<ReviewVerdict, String> {
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

pub(super) fn receipt_strings(object: &JsonObject, field: &str) -> Result<Vec<String>, String> {
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

pub(super) fn boolean(object: &JsonObject, field: &str) -> Result<bool, String> {
    required(object, field)?
        .as_bool()
        .ok_or_else(|| format!("{field} must be a boolean"))
}

pub(super) fn risk_name(risk: ReviewRisk) -> &'static str {
    match risk {
        ReviewRisk::Normal => "normal",
        ReviewRisk::High => "high",
        ReviewRisk::Integration => "integration",
        ReviewRisk::Critical => "critical",
    }
}

pub(super) fn reasoning_name(reasoning: ReviewReasoning) -> &'static str {
    match reasoning {
        ReviewReasoning::Standard => "standard",
        ReviewReasoning::High => "high",
    }
}

pub(super) fn parse_risk(value: &str) -> Result<ReviewRisk, String> {
    match value {
        "normal" => Ok(ReviewRisk::Normal),
        "high" => Ok(ReviewRisk::High),
        "integration" => Ok(ReviewRisk::Integration),
        "critical" => Ok(ReviewRisk::Critical),
        _ => Err("executor independent review risk is invalid".to_string()),
    }
}

pub(super) fn parse_reasoning(value: &str) -> Result<ReviewReasoning, String> {
    match value {
        "standard" => Ok(ReviewReasoning::Standard),
        "high" => Ok(ReviewReasoning::High),
        _ => Err("executor independent review reasoning is invalid".to_string()),
    }
}
