use super::*;

#[cfg(any(target_os = "macos", target_os = "freebsd", windows))]
pub(in crate::commands::autonomous::executor_bridge) fn interrupted_direct_terminal(
) -> AttemptTerminal {
    AttemptTerminal::CleanupFailed(
        "interrupted attempt was quarantined before terminal publication".to_string(),
    )
}

#[cfg(any(target_os = "macos", target_os = "freebsd", windows))]
pub(in crate::commands::autonomous::executor_bridge) fn validate_platform_direct_quarantine(
    paths: &DirectAttemptPaths,
) -> Result<(), String> {
    let parent = paths
        .record
        .parent()
        .ok_or_else(|| "direct command record has no parent".to_string())?;
    let command = paths
        .record
        .file_stem()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "direct command record name is malformed".to_string())?;
    let prefix = format!("{command}.ownership-disproven-");
    for entry in fs::read_dir(parent)
        .map_err(|error| format!("inventory portable ownership quarantines: {error}"))?
    {
        let entry =
            entry.map_err(|error| format!("read portable ownership quarantine: {error}"))?;
        let name = entry.file_name();
        let Some(attempt_id) = name
            .to_str()
            .and_then(|name| name.strip_prefix(&prefix))
            .and_then(|name| name.strip_suffix(".json"))
        else {
            continue;
        };
        if !valid_direct_attempt_id(attempt_id) {
            return Err("portable ownership quarantine filename is malformed".to_string());
        }
        validate_private_state_file(&entry.path())
            .map_err(|error| format!("portable ownership quarantine is unsafe: {error}"))?;
        let body = fs::read_to_string(entry.path())
            .map_err(|error| format!("read portable ownership quarantine: {error}"))?;
        let value: serde_json::Value = serde_json::from_str(&body)
            .map_err(|error| format!("parse portable ownership quarantine: {error}"))?;
        let intent_digest = value
            .get("intent_digest")
            .and_then(serde_json::Value::as_str)
            .filter(|digest| valid_direct_attempt_id(digest))
            .ok_or_else(|| {
                "portable ownership quarantine intent digest is malformed".to_string()
            })?;
        if value.get("attempt_id").and_then(serde_json::Value::as_str) != Some(attempt_id) {
            return Err(
                "portable ownership quarantine attempt identity differs from its filename"
                    .to_string(),
            );
        }
        process_owner::DurableProcessOwner::from_document(&body, attempt_id, intent_digest)?;
    }
    Ok(())
}

#[cfg(any(target_os = "macos", target_os = "freebsd", windows))]
pub(in crate::commands::autonomous::executor_bridge) fn reconcile_direct_launch(
    paths: &DirectAttemptPaths,
    expected_intent_body: Option<&str>,
) -> Result<bool, String> {
    if resume_direct_retirement(paths)? {
        return Ok(true);
    }
    let intent_body = match fs::read_to_string(&paths.intent) {
        Ok(body) => body,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if paths.launch.exists() {
                return Err("direct launch exists without its durable intent".to_string());
            }
            return Ok(false);
        }
        Err(error) => return Err(format!("read direct command intent: {error}")),
    };
    validate_private_state_file(&paths.intent)
        .map_err(|error| format!("direct command intent is unsafe: {error}"))?;
    if expected_intent_body.is_some_and(|expected| expected != intent_body) {
        return Err("direct command intent differs from the requested argv".to_string());
    }
    let attempt_id = direct_intent_attempt_id(&paths.intent)?
        .ok_or_else(|| "direct command intent disappeared during reconciliation".to_string())?;
    validate_direct_attempt_id_reservation(paths, &attempt_id)?;
    if paths.launch.is_file() {
        validate_private_state_file(&paths.launch)
            .map_err(|error| format!("direct launch record is unsafe: {error}"))?;
        let launch = fs::read_to_string(&paths.launch)
            .map_err(|error| format!("read direct launch record: {error}"))?;
        let owner = process_owner::DurableProcessOwner::from_document(
            &launch,
            &attempt_id,
            &sha256_hex(intent_body.as_bytes()),
        )?;
        match process_owner::recover_owner(&owner) {
            process_owner::RecoveryDisposition::Completed => {
                retire_direct_launch(paths, &attempt_id)?;
            }
            process_owner::RecoveryDisposition::SidecarOwns => {
                return Err("reconstructed direct owner remains sidecar-owned".to_string());
            }
            process_owner::RecoveryDisposition::Quarantine => {
                return Err(
                    "reconstructed direct owner is quarantined; portable recovery cannot prove cleanup"
                        .to_string(),
                );
            }
        }
    }
    Ok(true)
}

#[cfg(any(target_os = "macos", target_os = "freebsd", windows))]
enum OwnedAttemptResult {
    Exited(std::process::ExitStatus),
    TimedOut(std::process::ExitStatus),
    OutputOverflow(std::process::ExitStatus),
    ReviewerLimit(std::process::ExitStatus),
}

#[cfg(any(target_os = "macos", target_os = "freebsd", windows))]
enum OwnedAttemptFailure {
    BeforeCleanup(String),
    ArtifactAfterCleanup(String),
    Cleanup(String),
}

#[cfg(any(target_os = "macos", target_os = "freebsd", windows))]
trait AttemptLifecycle {
    fn reviewer_at_limit(&self, capture: &ActiveReviewerCapture) -> Result<bool, String>;
    fn finalize_reviewer(&self, capture: &ActiveReviewerCapture) -> Result<bool, String>;
    fn bound_output(&self, paths: &DirectAttemptPaths) -> Result<(), String>;
}

#[cfg(any(target_os = "macos", target_os = "freebsd", windows))]
struct SystemAttemptLifecycle;

#[cfg(any(target_os = "macos", target_os = "freebsd", windows))]
impl AttemptLifecycle for SystemAttemptLifecycle {
    fn reviewer_at_limit(&self, capture: &ActiveReviewerCapture) -> Result<bool, String> {
        capture.at_limit()
    }

    fn finalize_reviewer(&self, capture: &ActiveReviewerCapture) -> Result<bool, String> {
        capture.finalize()
    }

    fn bound_output(&self, paths: &DirectAttemptPaths) -> Result<(), String> {
        bound_direct_output(paths)
    }
}

#[cfg(all(test, any(target_os = "macos", target_os = "freebsd")))]
struct InjectedAttemptLifecycle {
    fail_finalize: bool,
}

#[cfg(all(test, any(target_os = "macos", target_os = "freebsd")))]
impl InjectedAttemptLifecycle {
    fn fail_finalize() -> Self {
        Self {
            fail_finalize: true,
        }
    }
}

#[cfg(all(test, any(target_os = "macos", target_os = "freebsd")))]
impl AttemptLifecycle for InjectedAttemptLifecycle {
    fn reviewer_at_limit(&self, _capture: &ActiveReviewerCapture) -> Result<bool, String> {
        Ok(true)
    }

    fn finalize_reviewer(&self, _capture: &ActiveReviewerCapture) -> Result<bool, String> {
        if self.fail_finalize {
            Err("injected finalize failure".to_string())
        } else {
            Ok(false)
        }
    }

    fn bound_output(&self, paths: &DirectAttemptPaths) -> Result<(), String> {
        bound_direct_output(paths)
    }
}

#[cfg(any(target_os = "macos", target_os = "freebsd", windows))]
fn output_sizes(paths: &DirectAttemptPaths) -> Result<(u64, u64), String> {
    Ok((
        fs::metadata(&paths.stdout)
            .map_err(|error| format!("inspect direct stdout: {error}"))?
            .len(),
        fs::metadata(&paths.stderr)
            .map_err(|error| format!("inspect direct stderr: {error}"))?
            .len(),
    ))
}

#[cfg(any(target_os = "macos", target_os = "freebsd", windows))]
fn bound_direct_output(paths: &DirectAttemptPaths) -> Result<(), String> {
    for path in [&paths.stdout, &paths.stderr] {
        let file = OpenOptions::new()
            .write(true)
            .open(path)
            .map_err(|error| format!("open oversized direct output: {error}"))?;
        if file
            .metadata()
            .map_err(|error| format!("inspect oversized direct output: {error}"))?
            .len()
            > MAX_DIRECT_OUTPUT_BYTES
        {
            file.set_len(MAX_DIRECT_OUTPUT_BYTES)
                .map_err(|error| format!("bound direct output artifact: {error}"))?;
        }
    }
    Ok(())
}

#[cfg(any(target_os = "macos", target_os = "freebsd", windows))]
fn run_owned_attempt(
    owned: &mut process_owner::OwnedChildTree,
    paths: &DirectAttemptPaths,
    stall_timeout: Duration,
    reviewer_capture: Option<&ActiveReviewerCapture>,
) -> Result<OwnedAttemptResult, OwnedAttemptFailure> {
    run_owned_attempt_with_lifecycle(
        owned,
        paths,
        stall_timeout,
        reviewer_capture,
        &SystemAttemptLifecycle,
    )
}

#[cfg(any(target_os = "macos", target_os = "freebsd", windows))]
fn run_owned_attempt_with_lifecycle(
    owned: &mut process_owner::OwnedChildTree,
    paths: &DirectAttemptPaths,
    stall_timeout: Duration,
    reviewer_capture: Option<&ActiveReviewerCapture>,
    lifecycle: &impl AttemptLifecycle,
) -> Result<OwnedAttemptResult, OwnedAttemptFailure> {
    let mut last_progress = Instant::now();
    let mut observed = output_sizes(paths).map_err(OwnedAttemptFailure::BeforeCleanup)?;
    loop {
        if let Some(capture) = reviewer_capture {
            if lifecycle
                .reviewer_at_limit(capture)
                .map_err(OwnedAttemptFailure::BeforeCleanup)?
            {
                let status = owned.terminate().map_err(OwnedAttemptFailure::Cleanup)?;
                lifecycle
                    .finalize_reviewer(capture)
                    .map_err(OwnedAttemptFailure::ArtifactAfterCleanup)?;
                return Ok(OwnedAttemptResult::ReviewerLimit(status));
            }
        }
        if let Some(status) = owned
            .try_wait()
            .map_err(OwnedAttemptFailure::BeforeCleanup)?
        {
            if let Some(capture) = reviewer_capture {
                if lifecycle
                    .finalize_reviewer(capture)
                    .map_err(OwnedAttemptFailure::ArtifactAfterCleanup)?
                {
                    return Ok(OwnedAttemptResult::ReviewerLimit(status));
                }
            }
            return Ok(OwnedAttemptResult::Exited(status));
        }
        let sizes = output_sizes(paths).map_err(OwnedAttemptFailure::BeforeCleanup)?;
        if sizes.0 > MAX_DIRECT_OUTPUT_BYTES || sizes.1 > MAX_DIRECT_OUTPUT_BYTES {
            let status = owned.terminate().map_err(OwnedAttemptFailure::Cleanup)?;
            lifecycle
                .bound_output(paths)
                .map_err(OwnedAttemptFailure::ArtifactAfterCleanup)?;
            return Ok(OwnedAttemptResult::OutputOverflow(status));
        }
        if sizes != observed {
            observed = sizes;
            last_progress = Instant::now();
        } else if last_progress.elapsed() >= stall_timeout {
            return owned
                .terminate()
                .map(OwnedAttemptResult::TimedOut)
                .map_err(OwnedAttemptFailure::Cleanup);
        }
        thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(any(target_os = "macos", target_os = "freebsd", windows))]
fn finish_owned_attempt(
    owned: &mut process_owner::OwnedChildTree,
    result: Result<OwnedAttemptResult, OwnedAttemptFailure>,
) -> AttemptTerminal {
    match result {
        Ok(OwnedAttemptResult::Exited(status)) => match status.code() {
            Some(code) => AttemptTerminal::Exited(code),
            #[cfg(unix)]
            None => {
                use std::os::unix::process::ExitStatusExt;

                status
                    .signal()
                    .map_or(AttemptTerminal::Exited(1), AttemptTerminal::Signaled)
            }
            #[cfg(not(unix))]
            None => AttemptTerminal::Exited(1),
        },
        Ok(OwnedAttemptResult::TimedOut(_status)) => AttemptTerminal::TimedOut,
        Ok(OwnedAttemptResult::OutputOverflow(_status)) => AttemptTerminal::OutputOverflow,
        Ok(OwnedAttemptResult::ReviewerLimit(_status)) => AttemptTerminal::Exited(70),
        Err(OwnedAttemptFailure::ArtifactAfterCleanup(error)) => {
            AttemptTerminal::InfrastructureFailed(error)
        }
        Err(OwnedAttemptFailure::Cleanup(error)) => AttemptTerminal::CleanupFailed(error),
        Err(OwnedAttemptFailure::BeforeCleanup(error)) => match owned.terminate() {
            Ok(_) => AttemptTerminal::InfrastructureFailed(error),
            Err(cleanup) => AttemptTerminal::CleanupFailed(format!("{error}; {cleanup}")),
        },
    }
}

#[allow(clippy::too_many_arguments)]
#[cfg(any(target_os = "macos", target_os = "freebsd", windows))]
pub(in crate::commands::autonomous::executor_bridge) fn execute_supervised_direct_attempt(
    attempt_id: &str,
    worktree: &Path,
    program: &Path,
    direct: &DirectCommand,
    paths: &DirectAttemptPaths,
    stdout: &File,
    stderr: &File,
    environment_overrides: Vec<(OsString, OsString)>,
    intent_digest: &str,
    stall_timeout: Duration,
) -> AttemptTerminal {
    let reviewer_capture = match direct
        .review_capture
        .as_ref()
        .map(ActiveReviewerCapture::open)
        .transpose()
    {
        Ok(capture) => capture,
        Err(error) => return AttemptTerminal::InfrastructureFailed(error),
    };
    let stdout = match stdout.try_clone() {
        Ok(stdout) => stdout,
        Err(error) => {
            return AttemptTerminal::InfrastructureFailed(format!("clone direct stdout: {error}"))
        }
    };
    let stderr = match stderr.try_clone() {
        Ok(stderr) => stderr,
        Err(error) => {
            return AttemptTerminal::InfrastructureFailed(format!("clone direct stderr: {error}"))
        }
    };
    #[cfg(windows)]
    let null_input = "NUL";
    #[cfg(unix)]
    let null_input = "/dev/null";
    let spawn = File::open(null_input)
        .map_err(|error| format!("open null input: {error}"))
        .and_then(|stdin| {
            process_owner::PreparedLaunchSpec::credentialless(
                program.to_path_buf(),
                direct.argv.iter().map(OsString::from).collect(),
                Some(worktree.to_path_buf()),
                environment_overrides,
                paths
                    .stdout
                    .parent()
                    .ok_or_else(|| "direct command stdout has no parent".to_string())?
                    .join("credentialless-config"),
                false,
                Some(stdin),
                Some(stdout),
                Some(stderr),
            )
            .and_then(|spec| {
                process_owner::OwnedChildTree::spawn_prepared(spec, attempt_id.to_string())
            })
        });
    let mut owned = match spawn {
        Ok(owned) => owned,
        Err(error) => return AttemptTerminal::SpawnFailed(error),
    };
    let launch = owned.identity().document(attempt_id, intent_digest);
    if let Err(error) = write_private_create_once(
        &paths.launch,
        launch.as_bytes(),
        "portable direct command launch identity",
    ) {
        return match owned.terminate() {
            Ok(_) => AttemptTerminal::InfrastructureFailed(error),
            Err(cleanup) => AttemptTerminal::CleanupFailed(format!("{error}; {cleanup}")),
        };
    }
    let result = run_owned_attempt(&mut owned, paths, stall_timeout, reviewer_capture.as_ref());
    finish_owned_attempt(&mut owned, result)
}

#[cfg(all(test, any(target_os = "macos", target_os = "freebsd")))]
mod tests;
