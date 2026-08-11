use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ResolvedReviewPolicy {
    pub(crate) requirements: ReviewRequirements,
    pub(crate) reviewer_harness: HarnessKind,
    pub(crate) provider_diversified: bool,
    pub(crate) selection_reason: String,
}

pub(crate) fn resolve_review_policy(
    config: &HarnessConfig,
    requirements: ReviewRequirements,
    implementer_harness: HarnessKind,
    environment: &BTreeMap<String, OsString>,
) -> Result<ResolvedReviewPolicy, String> {
    let available = available_review_harnesses(config, environment);
    let implementer_available = available.contains(&implementer_harness);

    if let Some((reviewer_harness, provider_diversified, selection_reason)) = risk_review_selection(
        &requirements,
        &available,
        implementer_harness,
        implementer_available,
    )? {
        return Ok(ResolvedReviewPolicy {
            requirements,
            reviewer_harness,
            provider_diversified,
            selection_reason: selection_reason.to_string(),
        });
    }

    let reviewer_harness = if implementer_available {
        implementer_harness
    } else {
        available.first().copied().ok_or_else(|| {
            "executor_harness_unknown: no configured reviewer harness is available on PATH"
                .to_string()
        })?
    };
    let selection_reason = if reviewer_harness == implementer_harness {
        "normal:implementer-provider"
    } else {
        "normal:available-provider"
    };
    Ok(ResolvedReviewPolicy {
        requirements,
        reviewer_harness,
        provider_diversified: reviewer_harness != implementer_harness,
        selection_reason: selection_reason.to_string(),
    })
}

fn risk_review_selection(
    requirements: &ReviewRequirements,
    available: &[HarnessKind],
    implementer: HarnessKind,
    implementer_available: bool,
) -> Result<Option<(HarnessKind, bool, &'static str)>, String> {
    if !requirements.prefer_provider_diversity {
        return Ok(None);
    }
    if let Some(alternate) = alternate_harness(available, implementer) {
        return Ok(Some((alternate, true, "risk:provider-diversified")));
    }
    if requirements.require_provider_diversity {
        return Err(
            "critical review requires an alternate provider; none is configured and available"
                .to_string(),
        );
    }
    if implementer_available {
        return Ok(Some((
            implementer,
            false,
            "risk:same-provider-high-reasoning-fallback",
        )));
    }
    Ok(None)
}

fn alternate_harness(available: &[HarnessKind], implementer: HarnessKind) -> Option<HarnessKind> {
    available.iter().copied().find(|kind| *kind != implementer)
}

fn available_review_harnesses(
    config: &HarnessConfig,
    environment: &BTreeMap<String, OsString>,
) -> Vec<HarnessKind> {
    let mut available = Vec::new();
    for alias in &config.aliases {
        if available.contains(&alias.kind) {
            continue;
        }
        if resolve_review_harness(config, alias.kind, environment).is_ok() {
            available.push(alias.kind);
        }
    }
    available
}

fn resolve_review_harness(
    config: &HarnessConfig,
    kind: HarnessKind,
    environment: &BTreeMap<String, OsString>,
) -> Result<ResolvedHarness, String> {
    let mut selected_environment = environment.clone();
    selected_environment.insert(
        "AUTOSPEC_HANDOFF_DISPATCHER_KIND".to_string(),
        OsString::from(kind.as_str()),
    );
    config.resolve(&selected_environment)
}

pub(super) fn resolve_independent_reviewer(
    request: &ExecutorBridgeRequest,
    state: &PersistedInvocation,
    environment: &BTreeMap<String, OsString>,
    artifact_root: &Path,
) -> Result<IndependentReviewer, String> {
    if environment.contains_key("AUTOSPEC_EXECUTOR_REVIEW_COMMAND") {
        return Err(
            "unstructured review commands cannot authorize production review; configure a structured harness alias"
                .to_string(),
        );
    }

    let config = HarnessConfig::load(&state.identity.repository_path, environment)?;
    let policy = resolve_review_policy(
        &config,
        classify_executor_review_requirements(request, state)?,
        state.harness,
        environment,
    )?;
    let resolved = resolve_review_harness(&config, policy.reviewer_harness, environment)?;
    validate_external_reviewer_executable(state, &resolved.executable)?;
    ensure_private_directory(artifact_root)?;
    let artifact_root = fs::canonicalize(artifact_root)
        .map_err(|error| format!("canonicalize reviewer artifact root: {error}"))?;
    validate_external_reviewer_artifact_root(state, &artifact_root)?;
    let harness_artifact = artifact_root.join("harness-result.txt");
    if resolved.kind == HarnessKind::Codex {
        prepare_private_reviewer_result(&harness_artifact)?;
    }
    let prompt = independent_reviewer_prompt(request, state)?;
    let invocation =
        resolved.review_invocation(&state.identity.worktree, &harness_artifact, &prompt)?;
    let mut validated = validate_invocation(&invocation, &state.identity.worktree)?;
    validated.environment_overrides =
        sanitized_reviewer_environment(resolved.kind, environment, state, &artifact_root)?;
    let automatic =
        prepare_automatic_reviewer_normalizer(resolved.kind, &validated, &artifact_root)?;
    validate_external_reviewer_executable(state, &automatic.normalizer)?;
    Ok(IndependentReviewer {
        plan: DirectCommandPlan {
            commands: vec![DirectCommand::automatic_reviewer(
                vec![automatic.normalizer.display().to_string()],
                &automatic,
            )?],
        },
        automatic: Some(automatic),
        policy,
    })
}
