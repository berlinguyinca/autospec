use super::*;

#[cfg(not(target_os = "linux"))]
#[path = "portable_output.rs"]
mod portable_output;
#[cfg(not(target_os = "linux"))]
use portable_output::PortableOutputReaders;

#[cfg(all(test, not(target_os = "linux")))]
static PORTABLE_AFTER_CLEANUP_PROOF_FAILPOINT: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[cfg(all(test, not(target_os = "linux")))]
pub(in crate::commands::autonomous::executor_bridge) fn set_portable_after_cleanup_proof_failpoint(
    enabled: bool,
) {
    PORTABLE_AFTER_CLEANUP_PROOF_FAILPOINT.store(enabled, Ordering::SeqCst);
}

#[cfg(not(target_os = "linux"))]
pub(in crate::commands::autonomous::executor_bridge) fn create_draft_pull_request<Refresh>(
    state_path: &Path,
    state: &mut PersistedInvocation,
    body: &str,
    issue_title: &str,
    base: &str,
    adapter: &DraftPrAdapter,
    refresh: &mut Refresh,
) -> Result<(), BridgeRunFailure>
where
    Refresh: FnMut() -> Result<BridgeClaimOwnership, BridgeRunFailure>,
{
    if refresh()? == BridgeClaimOwnership::Lost {
        return Err(BridgeRunFailure::ownership_lost(
            "executor draft creation lost exact claim ownership",
        ));
    }
    let failpoint = |stage: &str| {
        adapter
            .environment
            .get(std::ffi::OsStr::new("AUTOSPEC_TEST_PORTABLE_DRAFT_FAIL"))
            .is_some_and(|value| value == std::ffi::OsStr::new(stage))
    };
    let body_path = state_path.with_file_name(format!(
        "draft-body-{}-{}.md",
        state.identity.invocation_id,
        INVOCATION_WRITE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    write_private_create_once(&body_path, body.as_bytes(), "executor draft body")?;
    let executable = resolve_draft_executable(adapter)?;
    let args = vec![
        "pr".into(),
        "create".into(),
        "--repo".into(),
        state.identity.repository.clone(),
        "--draft".into(),
        "--head".into(),
        state.identity.branch.clone(),
        "--base".into(),
        base.into(),
        "--title".into(),
        issue_title.into(),
        "--body-file".into(),
        body_path.to_string_lossy().into_owned(),
    ];

    // Portable process creation cannot use Linux's fork/pipe release barrier. Persist a
    // transaction identity and release guard before spawning instead: a crash after release may
    // strand a request, but can never authorize replay. Exact visible PR observation reconciles
    // it in `push_and_create_draft`; absent/delayed visibility remains quarantined.
    let process = ProcessIdentity {
        pid: u32::MAX - 1,
        process_group: u32::MAX - 1,
        executable: executable.clone(),
        argv_digest: argv_digest(&args),
        boot_id: "portable-draft-release-v1".to_string(),
        start_identity: state.identity.invocation_id.clone(),
    };
    state.draft_process = Some(process.clone());
    write_invocation_atomic(state_path, state)?;
    if failpoint("prepare") {
        let _ = fs::remove_file(&body_path);
        return Err("injected portable draft prepare failure".to_string().into());
    }
    write_draft_release_intent(state_path, state, &process)?;
    if failpoint("release") {
        let _ = fs::remove_file(&body_path);
        return Err("injected portable draft release failure".to_string().into());
    }
    write_private_create_once(
        &draft_release_receipt_path(state_path),
        draft_release_digest(state, &process).as_bytes(),
        "portable executor draft release receipt",
    )?;

    let null = if cfg!(windows) { "NUL" } else { "/dev/null" };
    let stderr_path = state_path.with_extension("draft.stderr");
    let stdout_path = state_path.with_extension("draft.stdout");
    let spawn = process_owner::OwnedChildTree::spawn_prepared(
        process_owner::PreparedLaunchSpec::inherited(
            executable.clone(),
            std::iter::once(executable.into_os_string())
                .chain(args.iter().map(OsString::from))
                .collect(),
            Some(state.identity.worktree.clone()),
            adapter.environment.clone().into_iter().collect(),
            Some(File::open(null).map_err(|error| format!("open draft null input: {error}"))?),
            Some(open_private_file(&stdout_path, true)?),
            Some(open_private_file(&stderr_path, true)?),
        ),
        format!("draft-{}", state.identity.invocation_id),
    );
    let mut owned = match spawn {
        Ok(owned) => owned,
        Err(error) => {
            let _ = fs::remove_file(&body_path);
            return Err(BridgeRunFailure::transient(format!(
                "launch executor draft pull request: {error}"
            )));
        }
    };
    let owner = owned.identity();
    let owner_path = state_path.with_extension("draft-owner.json");
    if let Err(error) = write_private_create_once(
        &owner_path,
        owner
            .document(
                &format!("draft-{}", state.identity.invocation_id),
                &draft_release_digest(state, &process),
            )
            .as_bytes(),
        "portable executor draft owner",
    ) {
        let cleanup = owned.terminate();
        let _ = fs::remove_file(&body_path);
        return Err(match cleanup {
            Ok(_) => error.into(),
            Err(cleanup) => format!("{error}; draft cleanup ambiguous: {cleanup}").into(),
        });
    }
    let status = owned.wait();
    let cleanup = owned.terminate();
    let _ = fs::remove_file(&body_path);
    let cleanup_status = cleanup.map_err(|error| {
        BridgeRunFailure::transient(format!(
            "portable executor draft cleanup is ambiguous: {error}"
        ))
    })?;
    write_private_create_once(
        &owner_path.with_extension("cleanup.json"),
        serde_json::json!({
            "schema": 1,
            "invocation_id": state.identity.invocation_id,
            "tree_cleanup": "proven",
            "exit_code": cleanup_status.code(),
        })
        .to_string()
        .as_bytes(),
        "portable executor draft cleanup evidence",
    )?;
    fs::remove_file(&owner_path)
        .map_err(|error| format!("retire portable executor draft owner: {error}"))?;
    let status = status.map_err(|error| {
        BridgeRunFailure::transient(format!("wait for executor draft pull request: {error}"))
    })?;
    if failpoint("post-request") {
        return Err("injected portable draft post-request failure"
            .to_string()
            .into());
    }
    if status.success() {
        Ok(())
    } else {
        let stderr = fs::read_to_string(&stderr_path).unwrap_or_default();
        Err(BridgeRunFailure::transient(format!(
            "create executor draft pull request failed: {}",
            stderr.trim()
        )))
    }
}

#[cfg(not(target_os = "linux"))]
struct PortableOwnedChildGuard {
    owned: process_owner::OwnedChildTree,
    armed: bool,
}

#[cfg(not(target_os = "linux"))]
impl PortableOwnedChildGuard {
    fn new(owned: process_owner::OwnedChildTree) -> Self {
        Self { owned, armed: true }
    }

    fn identity(&self) -> process_owner::DurableProcessOwner {
        self.owned.identity()
    }

    fn try_wait(&mut self) -> Result<Option<std::process::ExitStatus>, String> {
        self.owned.try_wait()
    }

    fn terminate(&mut self) -> Result<std::process::ExitStatus, String> {
        let status = self.owned.terminate()?;
        self.armed = false;
        Ok(status)
    }
}

#[cfg(not(target_os = "linux"))]
impl Drop for PortableOwnedChildGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.owned.terminate();
        }
    }
}

#[cfg(not(target_os = "linux"))]
pub(in crate::commands::autonomous::executor_bridge) fn supervise_validated_harness_with_claim_renewal(
    state_path: &Path,
    event_log: &Path,
    state: &mut PersistedInvocation,
    harness: Option<&ValidatedInvocation>,
    snapshot: &MutationSnapshot,
    config: SupervisionConfig,
    mut renewal: ClaimRenewalSchedule,
) -> Result<SupervisionOutcome, String> {
    if config.stall_timeout.is_zero() || config.poll_interval.is_zero() {
        return Err("executor supervision intervals must be non-zero".to_string());
    }
    let sinks = output_sink_paths_for_state(state_path, state)?;
    if state.supervisor.is_some() || state.process.is_some() || sinks.supervisor_identity.exists() {
        state.phase = BridgePhase::Interrupted;
        state.progress_at = unix_now()?;
        write_invocation_atomic(state_path, state)?;
        append_executor_event(
            event_log,
            state,
            "child_recovery_required",
            Some(serde_json::json!({
                "reason": "durable process identity has no live in-process owner; ownership quarantined without signalling"
            })),
        )?;
        return Err(
            "executor ownership is quarantined; portable recovery cannot adopt a PID".to_string(),
        );
    }
    if renewal.is_enabled() {
        match refresh_bridge_claim(state, ClaimRenewalPhase::Implementation) {
            Ok(BridgeClaimOwnership::Refreshed { ttl_seconds }) => {
                renewal.mark_refreshed(ttl_seconds)
            }
            Ok(BridgeClaimOwnership::Lost) => {
                record_claim_ownership_loss(state_path, event_log, state)?;
                return Ok(SupervisionOutcome::OwnershipLost);
            }
            Err(error) => return Ok(SupervisionOutcome::TransientFailure(error)),
        }
    }
    let harness = harness.ok_or_else(|| {
        "executor recovery exhausted durable identities before fresh harness resolution".to_string()
    })?;
    state.phase = BridgePhase::Pending;
    state.supervisor = None;
    state.process = None;
    state.progress_at = unix_now()?;
    write_invocation_atomic(state_path, state)?;
    let stdout = open_private_file(&sinks.stdout, true)?;
    let stderr = open_private_file(&sinks.stderr, true)?;
    let null = if cfg!(windows) { "NUL" } else { "/dev/null" };
    let mut argv = Vec::with_capacity(harness.args.len() + 1);
    argv.push(
        harness
            .argv_zero
            .clone()
            .unwrap_or_else(|| harness.program.clone().into_os_string()),
    );
    argv.extend(harness.args.iter().map(OsString::from));
    let credentialless_config = sinks
        .stdout
        .parent()
        .ok_or_else(|| "executor output sink has no parent".to_string())?
        .join("credentialless-config");
    let preserve_codex_host_auth = harness.args.first().is_some_and(|arg| arg == "exec")
        && harness
            .args
            .iter()
            .any(|arg| arg == "--output-last-message");
    let owned = process_owner::OwnedChildTree::spawn_prepared(
        process_owner::PreparedLaunchSpec::credentialless(
            harness.program.clone(),
            argv,
            Some(harness.current_dir.clone()),
            harness.environment_overrides.clone(),
            credentialless_config,
            preserve_codex_host_auth,
            Some(File::open(null).map_err(|error| format!("open executor null input: {error}"))?),
            Some(stdout),
            Some(stderr),
        )?,
        state.identity.invocation_id.clone(),
    )?;
    let mut owned = PortableOwnedChildGuard::new(owned);
    let mut readers = PortableOutputReaders::open(&sinks)?;
    let owner = owned.identity();
    #[cfg(test)]
    LAST_SPAWN_HARNESS.store(owner.pid, Ordering::SeqCst);
    let owner_document = owner.document(&state.identity.invocation_id, &argv_digest(&harness.args));
    let journal = fail_launch_at("journal-write").and_then(|()| {
        write_private_create_once(
            &sinks.supervisor_identity,
            owner_document.as_bytes(),
            "portable executor owner",
        )
    });
    enum PortableTerminal {
        Exited(i32),
        OwnershipLost,
        Transient(BridgeRunFailure),
        Stalled { last_progress_at: u64 },
    }

    let operation = journal.and_then(|()| {
        (|| -> Result<PortableTerminal, String> {
            fail_launch_at("persist")?;
            state.phase = BridgePhase::Implementing;
            state.progress_at = unix_now()?;
            write_invocation_atomic(state_path, state)?;
            fail_launch_at("log")?;
            append_executor_event(event_log, state, "child_started", None)?;
            let mut last_progress = Instant::now();
            let mut last_progress_at = state.progress_at;
            loop {
                thread::sleep(config.poll_interval);
                fail_launch_at("direct-poll")?;
                let consumed = readers.poll()?;
                if consumed > 0 {
                    fail_launch_at("adopt-flush")?;
                    last_progress = Instant::now();
                }
                if readers.flush_if_due(state_path, event_log, state, false)? {
                    last_progress = Instant::now();
                    last_progress_at = state.progress_at;
                }
                if let Some(status) = owned.try_wait()? {
                    return Ok(PortableTerminal::Exited(status.code().unwrap_or(1)));
                }
                if renewal.is_due() {
                    match refresh_bridge_claim(state, ClaimRenewalPhase::Implementation) {
                        Ok(BridgeClaimOwnership::Refreshed { ttl_seconds }) => {
                            renewal.mark_refreshed(ttl_seconds)
                        }
                        Ok(BridgeClaimOwnership::Lost) => {
                            return Ok(PortableTerminal::OwnershipLost)
                        }
                        Err(error) => return Ok(PortableTerminal::Transient(error)),
                    }
                }
                if last_progress.elapsed() >= config.stall_timeout {
                    return Ok(PortableTerminal::Stalled { last_progress_at });
                }
            }
        })()
    });

    // This is the single post-spawn finalizer. It consumes the live OS ownership authority on
    // every branch, including a leader that already exited while descendants remain. The durable
    // journal is retained unless tree cleanup, cleanup evidence, and its event are all proven.
    let cleanup_status = match owned.terminate() {
        Ok(status) => status,
        Err(cleanup) => {
            state.phase = BridgePhase::Interrupted;
            state.progress_at = unix_now().unwrap_or(state.progress_at);
            let _ = write_invocation_atomic(state_path, state);
            let _ = append_executor_event(
                event_log,
                state,
                "child_cleanup_ambiguous",
                Some(serde_json::json!({"reason": &cleanup})),
            );
            let operation = operation
                .err()
                .map(|error| format!("; operation failed: {error}"))
                .unwrap_or_default();
            return Err(format!(
                "executor portable child cleanup is ambiguous: {cleanup}{operation}"
            ));
        }
    };
    let cleanup_path = sinks.supervisor_identity.with_extension("cleanup.json");
    let cleanup_document = serde_json::json!({
        "schema": 1,
        "invocation_id": state.identity.invocation_id,
        "owner": owner_document,
        "exit_code": cleanup_status.code(),
        "tree_cleanup": "proven",
    })
    .to_string();
    write_private_create_once(
        &cleanup_path,
        cleanup_document.as_bytes(),
        "portable executor cleanup evidence",
    )?;
    append_executor_event(event_log, state, "child_cleanup_complete", None)?;
    #[cfg(test)]
    if PORTABLE_AFTER_CLEANUP_PROOF_FAILPOINT.swap(false, Ordering::SeqCst) {
        return Err("injected portable failure after cleanup proof".to_string());
    }

    let post_cleanup = (|| -> Result<SupervisionOutcome, String> {
        let terminal = operation?;
        if matches!(
            &terminal,
            PortableTerminal::Exited(_) | PortableTerminal::Stalled { .. }
        ) {
            match readers.drain_after_exit(state_path, event_log, state, &mut renewal)? {
                CompletionDrainOutcome::Drained => {}
                CompletionDrainOutcome::OwnershipLost => {
                    record_claim_ownership_loss(state_path, event_log, state)?;
                    return Ok(SupervisionOutcome::OwnershipLost);
                }
                CompletionDrainOutcome::TransientFailure(error) => {
                    return Ok(SupervisionOutcome::TransientFailure(error));
                }
            }
        }
        let outcome = match terminal {
            PortableTerminal::Exited(exit_code) => {
                fail_launch_at("pre-verify")?;
                snapshot.verify(&state.identity.repository_path, &state.identity.branch)?;
                state.phase = if exit_code == 0 {
                    BridgePhase::ImplementationComplete
                } else {
                    BridgePhase::Interrupted
                };
                state.progress_at = unix_now()?;
                write_invocation_atomic(state_path, state)?;
                append_executor_event(
                    event_log,
                    state,
                    "child_exited",
                    Some(serde_json::json!({"exit_code": exit_code, "adopted": false})),
                )?;
                SupervisionOutcome::Exited { exit_code }
            }
            PortableTerminal::OwnershipLost => {
                record_claim_ownership_loss(state_path, event_log, state)?;
                SupervisionOutcome::OwnershipLost
            }
            PortableTerminal::Transient(error) => {
                return Ok(SupervisionOutcome::TransientFailure(error));
            }
            PortableTerminal::Stalled { last_progress_at } => {
                state.phase = BridgePhase::Interrupted;
                state.progress_at = unix_now()?;
                write_invocation_atomic(state_path, state)?;
                append_executor_event(
                    event_log,
                    state,
                    "child_stalled",
                    Some(serde_json::json!({
                        "stall_timeout_ms": config.stall_timeout.as_millis(),
                        "last_progress_at": last_progress_at,
                    })),
                )?;
                SupervisionOutcome::Stalled
            }
        };
        Ok(outcome)
    })();
    let retirement = match fs::remove_file(&sinks.supervisor_identity) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("retire portable executor owner journal: {error}")),
    };
    match (post_cleanup, retirement) {
        (Ok(outcome), Ok(())) => Ok(outcome),
        (Err(error), Ok(())) | (Ok(_), Err(error)) => Err(error),
        (Err(operation_error), Err(retirement_error)) => {
            Err(format!("{operation_error}; {retirement_error}"))
        }
    }
}

pub(in crate::commands::autonomous::executor_bridge) fn resolve_executor_supervisor_executable(
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
