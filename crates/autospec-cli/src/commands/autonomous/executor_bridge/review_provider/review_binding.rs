use super::super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::commands::autonomous::executor_bridge) struct BoundReviewEvidence {
    pub(in crate::commands::autonomous::executor_bridge) commit: String,
    pub(in crate::commands::autonomous::executor_bridge) inventory: ExecutorReviewInventory,
    pub(in crate::commands::autonomous::executor_bridge) requirements_digest: String,
    pub(in crate::commands::autonomous::executor_bridge) integration_evidence_digest:
        Option<String>,
    pub(in crate::commands::autonomous::executor_bridge) integration_command_records: Vec<String>,
}

impl BoundReviewEvidence {
    pub(in crate::commands::autonomous::executor_bridge) fn integration_citations(
        &self,
    ) -> Vec<String> {
        let Some(evidence_digest) = self.integration_evidence_digest.as_deref() else {
            return Vec::new();
        };
        let mut citations = vec![
            format!("requirements-digest:{}", self.requirements_digest),
            format!("integration-evidence-digest:{evidence_digest}"),
        ];
        citations.extend(
            self.integration_command_records
                .iter()
                .map(|record| format!("integration-record:{record}")),
        );
        citations
    }
}

pub(in crate::commands::autonomous::executor_bridge) fn load_bound_review_evidence(
    state: &PersistedInvocation,
    requirements: &ReviewRequirements,
    inventory: ExecutorReviewInventory,
) -> Result<BoundReviewEvidence, String> {
    let commit = state
        .head_oid
        .as_deref()
        .filter(|head| head.len() == 40 && head.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or_else(|| "executor bound review evidence requires a canonical head".to_string())?
        .to_string();
    let requirements_digest = canonical_review_requirements_digest(requirements);
    if !requirements.require_integration_smoke {
        return Ok(BoundReviewEvidence {
            commit,
            inventory,
            requirements_digest,
            integration_evidence_digest: None,
            integration_command_records: Vec::new(),
        });
    }
    load_sealed_integration_evidence(state, inventory, commit, requirements_digest)
}

fn load_sealed_integration_evidence(
    state: &PersistedInvocation,
    inventory: ExecutorReviewInventory,
    commit: String,
    requirements_digest: String,
) -> Result<BoundReviewEvidence, String> {
    let lane = PremergeLaneIdentity::new(
        state.identity.repository.clone(),
        state.identity.issue,
        state.identity.worker_id.clone(),
        state.identity.claim_id.clone(),
        state.identity.branch.clone(),
        commit.clone(),
    )?;
    let lane_root = state
        .identity
        .worktree
        .join(".autospec/evidence/premerge")
        .join(lane.lane_digest());
    let complete = read_private_json_object(
        &lane_root.join("complete.json"),
        &[
            "schema",
            "lane_digest",
            "attempt_path",
            "generation",
            "seal_digest",
        ],
        "completed premerge evidence",
    )?;
    if checked_u32(&complete, "schema")? != 2
        || text(&complete, "lane_digest")? != lane.lane_digest()
    {
        return Err("executor completed premerge evidence lane mismatch".to_string());
    }
    let attempt_relative = safe_relative_evidence_path(&text(&complete, "attempt_path")?)?;
    if attempt_relative.components().next()
        != Some(std::path::Component::Normal(OsStr::new("attempts")))
    {
        return Err("executor completed premerge attempt path is invalid".to_string());
    }
    let attempt_root = lane_root.join(&attempt_relative);
    let seal_path = attempt_root.join("seal.json");
    validate_private_state_file(&seal_path)
        .map_err(|error| format!("executor premerge evidence seal is unsafe: {error}"))?;
    let seal_body = fs::read_to_string(&seal_path)
        .map_err(|error| format!("read executor premerge evidence seal: {error}"))?;
    if sha256_hex(seal_body.as_bytes()) != text(&complete, "seal_digest")? {
        return Err("executor premerge evidence seal digest mismatch".to_string());
    }
    let seal = strict_object(
        serde_json::from_str(&seal_body)
            .map_err(|error| format!("parse executor premerge evidence seal: {error}"))?,
        &[
            "schema",
            "lane_digest",
            "intent_digest",
            "manifest_digest",
            "cleanup_digest",
            "qa_digest",
            "security_digest",
        ],
        "executor premerge evidence seal",
    )?;
    if checked_u32(&seal, "schema")? != 1 || text(&seal, "lane_digest")? != lane.lane_digest() {
        return Err("executor premerge evidence seal lane mismatch".to_string());
    }
    let observed_path = attempt_root.join("observed.json");
    validate_private_state_file(&observed_path)
        .map_err(|error| format!("executor observed premerge evidence is unsafe: {error}"))?;
    let observed_body = fs::read_to_string(&observed_path)
        .map_err(|error| format!("read executor observed premerge evidence: {error}"))?;
    if sha256_hex(observed_body.as_bytes()) != text(&seal, "manifest_digest")? {
        return Err("executor observed premerge evidence manifest digest mismatch".to_string());
    }
    let observed = parse_observed_manifest(&observed_body)?;
    if checked_u32(&observed, "schema")? != 2
        || text(&observed, "lane_digest")? != lane.lane_digest()
        || text(&observed, "base_oid")? != state.identity.base_oid
        || text(&observed, "intent_digest")? != text(&seal, "intent_digest")?
        || text(&observed, "review_requirements_digest")? != requirements_digest
    {
        return Err("executor observed integration evidence identity mismatch".to_string());
    }
    bind_observed_records(
        state,
        inventory,
        commit,
        requirements_digest,
        &attempt_root,
        &observed,
    )
}

fn parse_observed_manifest(body: &str) -> Result<JsonObject, String> {
    strict_object(
        serde_json::from_str(body)
            .map_err(|error| format!("parse executor observed premerge evidence: {error}"))?,
        &[
            "schema",
            "lane_digest",
            "base_oid",
            "intent_digest",
            "qa_run_id",
            "security_run_id",
            "review_requirements_digest",
            "integration_evidence_digest",
            "integration_records",
            "qa_records",
            "scanners",
            "artifacts",
        ],
        "executor observed premerge evidence",
    )
}

fn bind_observed_records(
    state: &PersistedInvocation,
    inventory: ExecutorReviewInventory,
    commit: String,
    requirements_digest: String,
    attempt_root: &Path,
    observed: &JsonObject,
) -> Result<BoundReviewEvidence, String> {
    let integration_evidence_digest = text(observed, "integration_evidence_digest")?;
    let record_values = required(observed, "integration_records")?
        .as_array()
        .filter(|records| !records.is_empty())
        .ok_or_else(|| {
            "executor integration evidence records must be a nonempty array".to_string()
        })?;
    let mut integration_command_records = Vec::with_capacity(record_values.len());
    let mut recomputed_digest = requirements_digest.clone();
    for value in record_values {
        let relative = value
            .as_str()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                "executor integration evidence records must contain paths".to_string()
            })?;
        let absolute = attempt_root.join(safe_relative_evidence_path(relative)?);
        validate_private_state_file(&absolute)
            .map_err(|error| format!("executor integration evidence record is unsafe: {error}"))?;
        let record = fs::read(&absolute)
            .map_err(|error| format!("read executor integration evidence record: {error}"))?;
        recomputed_digest.push('\0');
        recomputed_digest.push_str(&sha256_hex(&record));
        integration_command_records.push(
            absolute
                .strip_prefix(&state.identity.worktree)
                .map_err(|_| "executor integration evidence record escapes worktree".to_string())?
                .display()
                .to_string(),
        );
    }
    if sha256_hex(recomputed_digest.as_bytes()) != integration_evidence_digest {
        return Err("executor integration evidence digest mismatch".to_string());
    }
    Ok(BoundReviewEvidence {
        commit,
        inventory,
        requirements_digest,
        integration_evidence_digest: Some(integration_evidence_digest),
        integration_command_records,
    })
}

fn read_private_json_object(
    path: &Path,
    fields: &[&str],
    label: &str,
) -> Result<JsonObject, String> {
    validate_private_state_file(path).map_err(|error| format!("{label} is unsafe: {error}"))?;
    let body = fs::read_to_string(path).map_err(|error| format!("read {label}: {error}"))?;
    strict_object(
        serde_json::from_str(&body).map_err(|error| format!("parse {label}: {error}"))?,
        fields,
        label,
    )
}

fn safe_relative_evidence_path(value: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(value);
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err("executor integration evidence path is unsafe".to_string());
    }
    Ok(path)
}
