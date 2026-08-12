use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct IntegrationSmokeEvidenceBinding {
    pub(crate) requirements_digest: String,
    pub(crate) evidence_digest: String,
    pub(crate) command_records: Vec<String>,
}

#[derive(Default)]
pub(crate) struct IntegrationSmokeEvidenceOutcome {
    pub(crate) canonical_plan: Option<DirectCommandPlan>,
    pub(crate) observations: Vec<ObservedDirectCommand>,
    pub(crate) binding: Option<IntegrationSmokeEvidenceBinding>,
}

pub(crate) fn parse_primary_smoke(issue_body: &str) -> Result<DirectCommandPlan, String> {
    let line = first_fenced_command_under(issue_body, "primary smoke test (inner loop)")?;
    parse_direct_command_plan(line)
}

pub(crate) fn parse_required_integration_smoke(
    issue_body: &str,
    requirements: &ReviewRequirements,
) -> Result<Option<DirectCommandPlan>, String> {
    if !requirements.require_integration_smoke {
        return Ok(None);
    }
    let explicit_count = issue_body
        .lines()
        .filter(|line| {
            normalized_level_three_heading(line).as_deref()
                == Some("integration smoke test (pre-merge)")
        })
        .count();
    match explicit_count {
        0 => compatible_primary_integration_smoke(issue_body),
        1 => parse_explicit_integration_smoke(issue_body).map(Some),
        _ => Err(
            "executor integration smoke test (pre-merge) requires exactly one heading".to_string(),
        ),
    }
}

fn parse_explicit_integration_smoke(issue_body: &str) -> Result<DirectCommandPlan, String> {
    let line = first_fenced_command_under(issue_body, "integration smoke test (pre-merge)")?;
    let plan = parse_direct_command_plan(line)?;
    if plan.commands.len() != 1 {
        return Err(
            "executor integration smoke test (pre-merge) requires exactly one direct command"
                .to_string(),
        );
    }
    if !is_integration_test_command(&plan.commands[0]) {
        return Err(
            "executor integration smoke must invoke a repository integration, smoke, or e2e test"
                .to_string(),
        );
    }
    Ok(plan)
}

fn compatible_primary_integration_smoke(
    issue_body: &str,
) -> Result<Option<DirectCommandPlan>, String> {
    let primary = parse_primary_smoke(issue_body)?;
    if primary.commands.iter().any(is_integration_test_command) {
        Ok(Some(primary))
    } else {
        Err("executor integration smoke is required before independent review".to_string())
    }
}

fn is_integration_test_command(command: &DirectCommand) -> bool {
    let program = Path::new(&command.argv[0])
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let recognized_runner = matches!(
        program,
        "bash" | "sh" | "bats" | "pytest" | "python" | "python3"
    );
    let repository_test = command
        .argv
        .iter()
        .skip(1)
        .any(|argument| repository_integration_test_path(argument));
    let direct_repository_test = repository_integration_test_path(&command.argv[0]);
    (recognized_runner && repository_test) || direct_repository_test
}

fn repository_integration_test_path(argument: &str) -> bool {
    let normalized = argument.trim_start_matches("./").replace('\\', "/");
    !normalized.split('/').any(|part| part == "..")
        && ["tests/integration/", "tests/smoke/", "tests/e2e/"]
            .iter()
            .any(|prefix| normalized.starts_with(prefix))
}

pub(crate) fn produce_integration_smoke_evidence(
    request: &DeterministicEvidenceRequest<'_>,
    primary_plan: &DirectCommandPlan,
    primary_observations: &[ObservedDirectCommand],
    attempt_root: &Path,
) -> Result<IntegrationSmokeEvidenceOutcome, String> {
    let Some(plan) =
        parse_required_integration_smoke(request.issue_body, &request.review_requirements)?
    else {
        return Ok(IntegrationSmokeEvidenceOutcome::default());
    };
    if &plan == primary_plan {
        let binding = bind_integration_smoke_evidence(
            &request.review_requirements,
            attempt_root,
            primary_observations,
        )?;
        return Ok(IntegrationSmokeEvidenceOutcome {
            binding: Some(binding),
            ..IntegrationSmokeEvidenceOutcome::default()
        });
    }
    let observations = execute_required_integration_smoke(
        request.state,
        &plan,
        &attempt_root.join("qa/integration"),
        request.runtime,
        request.stall_timeout,
    )?;
    let binding =
        bind_integration_smoke_evidence(&request.review_requirements, attempt_root, &observations)?;
    Ok(IntegrationSmokeEvidenceOutcome {
        canonical_plan: Some(plan),
        observations,
        binding: Some(binding),
    })
}

pub(crate) fn execute_required_integration_smoke(
    state: &PersistedInvocation,
    plan: &DirectCommandPlan,
    artifact_root: &Path,
    runtime: Option<&DirectRuntimeAdapter>,
    stall_timeout: Duration,
) -> Result<Vec<ObservedDirectCommand>, String> {
    if state.phase != BridgePhase::DraftCreated || plan.commands.len() != 1 {
        return Err(
            "executor integration smoke requires one command at the created draft phase"
                .to_string(),
        );
    }
    execute_direct_plan(
        &state.identity.worktree,
        plan,
        artifact_root,
        runtime,
        stall_timeout,
    )
}

pub(crate) fn bind_integration_smoke_evidence(
    requirements: &ReviewRequirements,
    artifact_root: &Path,
    observations: &[ObservedDirectCommand],
) -> Result<IntegrationSmokeEvidenceBinding, String> {
    if !requirements.require_integration_smoke || observations.is_empty() {
        return Err("executor integration smoke binding requires observed evidence".to_string());
    }
    let mut command_records = Vec::with_capacity(observations.len());
    for observation in observations {
        validate_private_state_file(&observation.record_path)
            .map_err(|error| format!("executor integration smoke record is unsafe: {error}"))?;
        let record = fs::read(&observation.record_path)
            .map_err(|error| format!("read integration smoke record: {error}"))?;
        if sha256_hex(&record) != observation.record_digest {
            return Err("executor integration smoke record digest changed".to_string());
        }
        let relative = observation
            .record_path
            .strip_prefix(artifact_root)
            .map_err(|_| "executor integration smoke record escapes evidence root".to_string())?;
        command_records.push(relative.display().to_string());
    }
    let requirements_digest = canonical_review_requirements_digest(requirements);
    let mut digest_input = requirements_digest.clone();
    for observation in observations {
        digest_input.push('\0');
        digest_input.push_str(&observation.record_digest);
    }
    Ok(IntegrationSmokeEvidenceBinding {
        requirements_digest,
        evidence_digest: sha256_hex(digest_input.as_bytes()),
        command_records,
    })
}
