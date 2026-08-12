use super::*;

const REVIEW_VERDICT_SCHEMA: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReviewVerdict {
    pub(crate) schema: u32,
    pub(crate) commit: String,
    pub(crate) verdict: String,
    pub(crate) surfaces_examined: Vec<String>,
    pub(crate) tests_examined: Vec<String>,
    pub(crate) integration_paths_checked: Vec<String>,
    pub(crate) blocking_findings: Vec<String>,
}

pub(crate) fn parse_review_verdict(
    body: &str,
    expected_commit: &str,
    expected_integration_citations: &[String],
) -> Result<ReviewVerdict, String> {
    let value: serde_json::Value = serde_json::from_str(body)
        .map_err(|error| format!("parse executor structured review verdict: {error}"))?;
    let object = strict_object(
        value,
        &[
            "schema",
            "commit",
            "verdict",
            "surfaces_examined",
            "tests_examined",
            "integration_paths_checked",
            "blocking_findings",
        ],
        "executor structured review verdict",
    )?;
    let verdict = ReviewVerdict {
        schema: checked_u32(&object, "schema")?,
        commit: text(&object, "commit")?,
        verdict: text(&object, "verdict")?,
        surfaces_examined: string_array(&object, "surfaces_examined")?,
        tests_examined: string_array(&object, "tests_examined")?,
        integration_paths_checked: string_array(&object, "integration_paths_checked")?,
        blocking_findings: string_array(&object, "blocking_findings")?,
    };
    validate_review_verdict(&verdict, expected_commit, expected_integration_citations)?;
    Ok(verdict)
}

fn string_array(object: &JsonObject, field: &str) -> Result<Vec<String>, String> {
    required(object, field)?
        .as_array()
        .ok_or_else(|| format!("{field} must be an array"))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .filter(|value| !value.trim().is_empty())
                .map(str::to_string)
                .ok_or_else(|| format!("{field} must contain nonempty strings"))
        })
        .collect()
}

pub(super) fn validate_review_verdict(
    verdict: &ReviewVerdict,
    expected_commit: &str,
    expected_integration_citations: &[String],
) -> Result<(), String> {
    if verdict.schema != REVIEW_VERDICT_SCHEMA {
        return Err("executor structured review verdict schema is unsupported".to_string());
    }
    if verdict.commit != expected_commit {
        return Err("executor structured review commit mismatch".to_string());
    }
    if verdict.verdict != "lgtm" {
        return Err("executor structured review verdict must be lgtm".to_string());
    }
    if verdict.surfaces_examined.is_empty() {
        return Err("executor structured review surfaces_examined must be nonempty".to_string());
    }
    if verdict.tests_examined.is_empty() {
        return Err("executor structured review tests_examined must be nonempty".to_string());
    }
    if verdict.integration_paths_checked != expected_integration_citations {
        return Err(
            "executor structured review integration evidence citations mismatch".to_string(),
        );
    }
    if !verdict.blocking_findings.is_empty() {
        return Err("executor structured review blocking_findings must be empty".to_string());
    }
    Ok(())
}

pub(super) fn canonical_review_verdict_digest(verdict: &ReviewVerdict) -> String {
    sha256_hex(
        format!(
            "review-verdict-v1\0{}\0{}\0{}\0{}\0{}\0{}\0{}",
            verdict.schema,
            verdict.commit,
            verdict.verdict,
            verdict.surfaces_examined.join("\0"),
            verdict.tests_examined.join("\0"),
            verdict.integration_paths_checked.join("\0"),
            verdict.blocking_findings.join("\0"),
        )
        .as_bytes(),
    )
}

pub(super) fn read_structured_review_verdict(
    state: &PersistedInvocation,
    reviewer: &IndependentReviewer,
) -> Result<ReviewVerdict, String> {
    let result = reviewer
        .automatic
        .as_ref()
        .ok_or_else(|| "executor production review requires structured evidence".to_string())?
        .result
        .as_path();
    let body = fs::read_to_string(result)
        .map_err(|error| format!("read executor structured reviewer result: {error}"))?;
    let head = state
        .head_oid
        .as_deref()
        .ok_or_else(|| "executor structured review requires a stable head".to_string())?;
    let evidence = load_bound_review_evidence(
        state,
        &reviewer.policy.requirements,
        executor_review_inventory(state)?,
    )?;
    parse_review_verdict(&body, head, &evidence.integration_citations())
}
