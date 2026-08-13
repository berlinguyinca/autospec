use super::*;

#[cfg(not(target_os = "linux"))]
const LINUX_EXECUTOR_REQUIRED: &str = "executor supervision requires Linux pidfd ownership";

#[cfg(not(target_os = "linux"))]
fn require_linux_executor_supervision() -> Result<(), String> {
    Err(LINUX_EXECUTOR_REQUIRED.to_string())
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn run_executor_bridge(
    _request: &ExecutorBridgeRequest,
) -> Result<BridgeRunReceipt, BridgeRunFailure> {
    require_linux_executor_supervision().map_err(BridgeRunFailure::from)?;
    unreachable!("non-Linux executor admission always fails")
}

#[cfg(not(target_os = "linux"))]
pub(super) fn reconcile_direct_launch(
    _paths: &DirectAttemptPaths,
    _expected_intent_body: Option<&str>,
) -> Result<bool, String> {
    require_linux_executor_supervision()?;
    unreachable!("non-Linux executor admission always fails")
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn execute_direct_plan(
    _worktree: &Path,
    _plan: &DirectCommandPlan,
    _artifact_root: &Path,
    _runtime: Option<&DirectRuntimeAdapter>,
    _stall_timeout: Duration,
) -> Result<Vec<ObservedDirectCommand>, String> {
    require_linux_executor_supervision()?;
    unreachable!("non-Linux executor admission always fails")
}

#[cfg(not(target_os = "linux"))]
pub(super) fn create_draft_pull_request<Refresh>(
    _state_path: &Path,
    _state: &mut PersistedInvocation,
    _body: &str,
    _issue_title: &str,
    _base: &str,
    _adapter: &DraftPrAdapter,
    _refresh: &mut Refresh,
) -> Result<(), BridgeRunFailure>
where
    Refresh: FnMut() -> Result<BridgeClaimOwnership, BridgeRunFailure>,
{
    require_linux_executor_supervision().map_err(BridgeRunFailure::from)?;
    unreachable!("non-Linux executor admission always fails")
}

#[cfg(not(target_os = "linux"))]
pub(super) fn supervise_validated_harness_with_claim_renewal(
    _state_path: &Path,
    _event_log: &Path,
    _state: &mut PersistedInvocation,
    _harness: Option<&ValidatedInvocation>,
    _snapshot: &MutationSnapshot,
    _config: SupervisionConfig,
    _renewal: ClaimRenewalSchedule,
) -> Result<SupervisionOutcome, String> {
    require_linux_executor_supervision()?;
    unreachable!("non-Linux executor admission always fails")
}

pub(super) fn resolve_executor_supervisor_executable(
    current_executable: Result<PathBuf, String>,
    argv_zero: Option<&OsStr>,
) -> Result<PathBuf, String> {
    let primary_error = match current_executable {
        Ok(path) => match fs::canonicalize(&path) {
            Ok(canonical) => return Ok(canonical),
            Err(error) => format!("canonicalize executor supervisor executable: {error}"),
        },
        Err(error) => error,
    };
    let fallback = argv_zero
        .map(Path::new)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| {
            format!(
                "{primary_error}; executor supervisor argv-zero fallback is not an absolute path"
            )
        })?;
    let _canonical = fs::canonicalize(fallback).map_err(|error| {
        format!(
            "{primary_error}; canonicalize executor supervisor argv-zero fallback {}: {error}",
            fallback.display()
        )
    })?;
    #[cfg(target_os = "linux")]
    {
        let running = fs::metadata("/proc/self/exe").map_err(|error| {
            format!("{primary_error}; inspect running executor supervisor image: {error}")
        })?;
        let candidate = fs::metadata(&_canonical).map_err(|error| {
            format!(
                "{primary_error}; inspect executor supervisor argv-zero fallback {}: {error}",
                _canonical.display()
            )
        })?;
        if running.dev() != candidate.dev() || running.ino() != candidate.ino() {
            return Err(format!(
                "{primary_error}; executor supervisor argv-zero fallback does not identify the running image"
            ));
        }
        Ok(_canonical)
    }
    #[cfg(not(target_os = "linux"))]
    Err(format!(
        "{primary_error}; executor supervisor argv-zero fallback cannot prove running-image identity on this platform"
    ))
}

#[cfg(all(test, not(target_os = "linux")))]
mod tests {
    use super::*;

    #[test]
    fn executor_bridge_fails_closed_before_state_mutation_without_linux_pidfds() {
        assert_eq!(
            require_linux_executor_supervision().unwrap_err(),
            "executor supervision requires Linux pidfd ownership"
        );
    }

    #[test]
    fn executor_bridge_keeps_harness_alias_parsing_portable() {
        let aliases = HarnessConfig::parse_alias_table("codex\tcodex\t--yolo\tCodex CLI\n")
            .expect("parse portable harness alias");
        assert_eq!(aliases.len(), 1);
        assert_eq!(aliases[0].kind, HarnessKind::Codex);
    }

    #[test]
    fn executor_bridge_keeps_primary_supervisor_resolution_portable() {
        let executable = std::env::current_exe().expect("current test executable");
        let resolved = resolve_executor_supervisor_executable(Ok(executable.clone()), None)
            .expect("resolve primary executable");
        assert_eq!(
            resolved,
            fs::canonicalize(executable).expect("canonical test executable")
        );
    }
}
