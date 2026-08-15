use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BridgeLifecycleBoundary {
    PullRequestOpened { pull_request: u64 },
    ReviewStarted { pull_request: u64 },
    Verified { pull_request: u64 },
    Merged { pull_request: u64 },
}

pub(crate) fn run_executor_bridge(
    request: &ExecutorBridgeRequest,
) -> Result<BridgeRunReceipt, BridgeRunFailure> {
    run_executor_bridge_observed(request, |_| Ok(()))
}

pub(crate) fn run_executor_bridge_observed(
    request: &ExecutorBridgeRequest,
    mut observe: impl FnMut(BridgeLifecycleBoundary) -> Result<(), String>,
) -> Result<BridgeRunReceipt, BridgeRunFailure> {
    run_executor_bridge_with_codex_probe_observed(request, preflight_codex_sandbox, &mut observe)
}

pub(crate) fn run_executor_bridge_with_codex_probe(
    request: &ExecutorBridgeRequest,
    codex_probe: impl FnOnce(&Path) -> Result<CodexSandboxPolicy, String>,
) -> Result<BridgeRunReceipt, BridgeRunFailure> {
    run_executor_bridge_with_codex_probe_observed(request, codex_probe, &mut |_| Ok(()))
}

pub(crate) fn observe_pull_request_and_review(
    state: &PersistedInvocation,
    observe: &mut dyn FnMut(BridgeLifecycleBoundary) -> Result<(), String>,
) -> Result<u64, BridgeRunFailure> {
    let pull_request = state
        .pr
        .ok_or_else(|| "executor review path has no pull request".to_string())?;
    observe(BridgeLifecycleBoundary::PullRequestOpened { pull_request }).map_err(|error| {
        BridgeRunFailure::invariant(format!(
            "accountability pull-request boundary rejected: {error}"
        ))
    })?;
    observe(BridgeLifecycleBoundary::ReviewStarted { pull_request }).map_err(|error| {
        BridgeRunFailure::invariant(format!("accountability review boundary rejected: {error}"))
    })?;
    Ok(pull_request)
}

pub(crate) fn observe_verified(
    pull_request: u64,
    observe: &mut dyn FnMut(BridgeLifecycleBoundary) -> Result<(), String>,
) -> Result<(), BridgeRunFailure> {
    observe(BridgeLifecycleBoundary::Verified { pull_request }).map_err(|error| {
        BridgeRunFailure::invariant(format!(
            "accountability verification boundary rejected: {error}"
        ))
    })
}

pub(crate) fn observe_merged(
    pull_request: u64,
    observe: &mut dyn FnMut(BridgeLifecycleBoundary) -> Result<(), String>,
) -> Result<(), BridgeRunFailure> {
    observe(BridgeLifecycleBoundary::Merged { pull_request }).map_err(|error| {
        BridgeRunFailure::invariant(format!("accountability merge boundary rejected: {error}"))
    })
}
