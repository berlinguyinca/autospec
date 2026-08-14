use super::*;

mod control;
pub(super) use control::*;

#[cfg(test)]
mod terminal_tests;

pub(super) fn bind_accountability_epic(
    layout: &RunLayout,
    options: &Options,
    lease: &resilience::ConductorLease,
    successor: bool,
) -> Result<accountability::github::EpicBinding, CommandFailure> {
    use accountability::github::{EpicBindingRequest, GhCli, ResumePolicy};
    use accountability::{
        AccountabilityEvent, AccountabilityStore, EventKind, Evidence, LaunchDescriptor,
        LeaseGeneration, RepositoryIdentity, RunIdentity, RunNonce,
    };

    create_launch_directories(layout)?;
    let repository = RepositoryIdentity::parse(&layout.repo)
        .map_err(|error| CommandFailure::diagnostic(error.to_string()))?;
    let root = accountability_root(layout, options.epic)?;
    if options.epic.is_none() && root.exists() {
        let existing = AccountabilityStore::open(&root)
            .map_err(|error| CommandFailure::diagnostic(error.to_string()))?;
        if successor
            || matches!(
                existing.status().lifecycle_phase.as_str(),
                "spawned" | "terminal"
            )
        {
            drop(existing);
            archive_accountability_root(layout, &root)?;
        }
    }
    let mut store = AccountabilityStore::open(&root)
        .map_err(|error| CommandFailure::diagnostic(error.to_string()))?;
    if store.identity().is_none() && options.epic.is_none() {
        let identity = RunIdentity::derive(
            repository.clone(),
            RunNonce::parse(&random_run_nonce()?)
                .map_err(|error| CommandFailure::diagnostic(error.to_string()))?,
            LeaseGeneration::new(lease.generation())
                .map_err(|error| CommandFailure::diagnostic(error.to_string()))?,
        );
        store
            .begin_launch(
                LaunchDescriptor::new(
                    identity,
                    "Execute the autonomous backlog for this repository",
                    "The run needs one durable epic that explains each implementation decision",
                )
                .map_err(|error| CommandFailure::diagnostic(error.to_string()))?,
            )
            .and_then(|_| {
                store.append_event(AccountabilityEvent::new(
                    EventKind::RunStarted,
                    "Started a new accountable autonomous conductor generation",
                    "Every live autonomous run must be traceable before it can mutate work",
                    vec![Evidence::outcome(
                        "lifecycle lease acquired before conductor spawn",
                    )],
                )?)?;
                Ok(())
            })
            .map_err(|error: accountability::AccountabilityError| {
                CommandFailure::diagnostic(error.to_string())
            })?;
    }
    let request = EpicBindingRequest {
        repository,
        explicit_epic: options.epic,
        resume_policy: if options.subcommand == "resume" {
            ResumePolicy::ReopenClosed
        } else {
            ResumePolicy::ActiveOnly
        },
        project_number: accountability_project_number(),
        adopted_lease_generation: Some(lease.generation()),
    };
    let heartbeat = resilience::start_lifecycle_heartbeat(&layout.repo, lease)
        .map_err(resilience_lease_error)?;
    let mut github = GhCli;
    let binding_result =
        accountability::github::bind_epic(&mut store, &mut github, request, || {
            resilience::renew_lifecycle(&layout.repo, lease)
                .map_err(|_| "lifecycle lease renewal rejected".to_string())
        });
    let heartbeat_result = heartbeat.finish().map_err(resilience_lease_error);
    let binding = binding_result.map_err(|error| CommandFailure::diagnostic(error.to_string()))?;
    heartbeat_result?;
    if let Some(warning) = binding.project_warning.as_deref() {
        eprintln!(
            "autospec autonomous accountability: optional Project assignment failed: {warning}"
        );
    }
    Ok(binding)
}

pub(super) fn open_bound_accountability(
    layout: &RunLayout,
) -> Result<accountability::AccountabilityStore, CommandFailure> {
    let launch = read_launch_json(&layout.state_dir);
    let run_id = extract_json_string(&launch, "run_id").ok_or_else(|| {
        CommandFailure::diagnostic("launch metadata has no accountability run_id")
    })?;
    let root = find_accountability_root(layout, &run_id).map_err(CommandFailure::diagnostic)?;
    accountability::AccountabilityStore::open(root)
        .map_err(|error| CommandFailure::diagnostic(error.to_string()))
}

pub(super) fn record_accountability_event(
    layout: &RunLayout,
    event: accountability::AccountabilityEvent,
    project: bool,
) -> Result<(), CommandFailure> {
    let mut store = open_bound_accountability(layout)?;
    store
        .append_event(event)
        .map_err(|error| CommandFailure::diagnostic(error.to_string()))?;
    if project {
        let status = store.status();
        let epic = status.epic_number.ok_or_else(|| {
            CommandFailure::diagnostic("accountability journal has no verified epic binding")
        })?;
        let repository = accountability::RepositoryIdentity::parse(&layout.repo)
            .map_err(|error| CommandFailure::diagnostic(error.to_string()))?;
        let request = accountability::github::EpicBindingRequest {
            repository,
            explicit_epic: Some(epic),
            resume_policy: accountability::github::ResumePolicy::ActiveOnly,
            project_number: None,
            adopted_lease_generation: None,
        };
        let mut github = accountability::github::GhCli;
        if let Err(error) =
            accountability::github::bind_epic(&mut store, &mut github, request, || Ok(()))
        {
            if error.projection_disposition()
                == Some(accountability::ProjectionDisposition::DegradableTransport)
            {
                eprintln!(
                    "autospec autonomous accountability projection degraded: {error}; local event remains durable"
                );
            } else {
                return Err(CommandFailure::diagnostic(format!(
                    "accountability integrity check blocked autonomous mutation: {error}"
                )));
            }
        }
    }
    Ok(())
}

pub(super) fn record_accountability_event_once(
    layout: &RunLayout,
    event: accountability::AccountabilityEvent,
    project: bool,
) -> Result<(), CommandFailure> {
    let store = open_bound_accountability(layout)?;
    if store.has_event(&event.kind) {
        return Ok(());
    }
    drop(store);
    record_accountability_event(layout, event, project)
}

pub(super) fn mark_accountability_spawned(layout: &RunLayout) -> Result<(), CommandFailure> {
    let mut store = open_bound_accountability(layout)?;
    store
        .mark_spawned()
        .map_err(|error| CommandFailure::diagnostic(error.to_string()))
}

pub(super) fn record_foreground_terminal(
    layout: &RunLayout,
    result: &Result<ForegroundCompletion, ForegroundFailure>,
) -> Result<(), CommandFailure> {
    let completed = matches!(
        result,
        Ok(ForegroundCompletion::State(_))
            | Ok(ForegroundCompletion::Lifecycle(
                LifecycleDecision::Run { .. }
            ))
    );
    let (kind, what, why) = if completed {
        (
            accountability::EventKind::Completed,
            "Completed the native autonomous conductor run",
            "The conductor reached a successful terminal boundary before relinquishing ownership",
        )
    } else {
        (
            accountability::EventKind::Stopped,
            "Stopped the native autonomous conductor run",
            "A non-success terminal boundary must remain visible before lifecycle ownership is released",
        )
    };
    record_accountability_event_once(
        layout,
        accountability_event(
            kind,
            what,
            why,
            "terminal accountability event persisted before lifecycle lease release",
        )?,
        true,
    )?;
    let status = open_bound_accountability(layout)?.status();
    require_terminal_projection_ack(
        status.pending_projection_count,
        status.next_projection_retry_at,
    )
}

fn require_terminal_projection_ack(
    pending_projection_count: u64,
    next_projection_retry_at: Option<u64>,
) -> Result<(), CommandFailure> {
    if pending_projection_count == 0 {
        return Ok(());
    }
    Err(CommandFailure::diagnostic(format!(
        "terminal accountability projection remains pending; retained lifecycle ownership for retry at {}",
        next_projection_retry_at
            .map(|value| value.to_string())
            .unwrap_or_else(|| "the next conductor tick".to_owned())
    )))
}

pub(super) fn finish_accountability_boundary<T, R>(
    value: T,
    record: impl FnOnce(&T) -> Result<(), CommandFailure>,
    release: impl FnOnce() -> Result<(), CommandFailure>,
    emit: impl FnOnce(T) -> Result<R, CommandFailure>,
) -> Result<R, CommandFailure> {
    record(&value)?;
    release()?;
    emit(value)
}

pub(super) fn accountability_event(
    kind: accountability::EventKind,
    what: impl Into<String>,
    why: impl Into<String>,
    evidence: impl Into<String>,
) -> Result<accountability::AccountabilityEvent, CommandFailure> {
    accountability::AccountabilityEvent::new(
        kind,
        what,
        why,
        vec![accountability::Evidence::outcome(evidence)],
    )
    .map_err(|error| CommandFailure::diagnostic(error.to_string()))
}

#[cfg(target_os = "linux")]
pub(super) fn record_bridge_accountability_boundary(
    layout: &RunLayout,
    issue: u64,
    boundary: executor_bridge::BridgeLifecycleBoundary,
) -> Result<(), String> {
    let (kind, what, why, evidence) = match boundary {
        executor_bridge::BridgeLifecycleBoundary::PullRequestOpened { pull_request } => (
            accountability::EventKind::PullRequestOpened { pull_request },
            format!("Opened PR {pull_request} for issue {issue}"),
            "The implementation needs a reviewable branch and durable merge boundary".to_owned(),
            format!("PR {pull_request} linked to issue {issue}"),
        ),
        executor_bridge::BridgeLifecycleBoundary::ReviewStarted { pull_request } => (
            accountability::EventKind::ReviewStarted { pull_request },
            format!("Started guarded review for PR {pull_request}"),
            "Autonomous changes require review evidence before merge".to_owned(),
            format!("PR {pull_request} entered the guarded review path"),
        ),
        executor_bridge::BridgeLifecycleBoundary::Verified { pull_request } => (
            accountability::EventKind::PullRequestVerified { pull_request },
            format!("Verified PR {pull_request} against its merge gates"),
            "Only verified changes may advance to the merge mutation".to_owned(),
            format!("PR {pull_request} passed the executor merge gates"),
        ),
        executor_bridge::BridgeLifecycleBoundary::Merged { pull_request } => (
            accountability::EventKind::Merged { pull_request },
            format!("Merged PR {pull_request} into the target branch"),
            "A merged commit is the only lifecycle boundary counted as implemented".to_owned(),
            format!("terminal merged receipt recorded for PR {pull_request}"),
        ),
    };
    record_accountability_event_once(
        layout,
        accountability_event(kind, what, why, evidence).map_err(|error| error.to_string())?,
        true,
    )
    .map_err(|error| error.message)
}

pub(super) fn record_executor_accountability(
    layout: &RunLayout,
    issue: u64,
    receipt: &ExecutorReceipt,
) -> Result<(), CommandFailure> {
    match (&receipt.outcome, receipt.pull_request) {
        (ConductorOutcome::Succeeded, Some(pull_request)) => {
            for (kind, what, why, evidence, project) in [
                (
                    accountability::EventKind::PullRequestOpened { pull_request },
                    format!("Opened PR {pull_request} for issue {issue}"),
                    "The implementation needs a reviewable branch and durable merge boundary"
                        .to_string(),
                    format!("PR {pull_request} linked to issue {issue}"),
                    false,
                ),
                (
                    accountability::EventKind::ReviewStarted { pull_request },
                    format!("Reviewed PR {pull_request} before merge"),
                    "Autonomous changes require review and verification evidence before landing"
                        .to_string(),
                    format!("PR {pull_request} entered the guarded review path"),
                    false,
                ),
                (
                    accountability::EventKind::PullRequestVerified { pull_request },
                    format!("Verified PR {pull_request} against its merge gates"),
                    "Only verified changes may be represented as ready to land".to_string(),
                    format!("PR {pull_request} passed the executor merge gates"),
                    false,
                ),
                (
                    accountability::EventKind::Merged { pull_request },
                    format!("Merged PR {pull_request} into the target branch"),
                    "A merged commit is the only lifecycle boundary counted as implemented"
                        .to_string(),
                    format!("terminal merged receipt recorded for PR {pull_request}"),
                    true,
                ),
            ] {
                record_accountability_event_once(
                    layout,
                    accountability_event(kind, what, why, evidence)?,
                    project,
                )?;
            }
        }
        (ConductorOutcome::Retryable(reason), _) => record_accountability_event(
            layout,
            accountability_event(
                accountability::EventKind::Failed,
                format!("Issue {issue} failed with a retryable executor outcome"),
                "The failure is recorded before the conductor schedules another attempt",
                reason.clone(),
            )?,
            true,
        )?,
        (ConductorOutcome::Blocked(reason), _)
        | (ConductorOutcome::AllBlocked { reason, .. }, _)
        | (ConductorOutcome::VerifierUnavailable { reason }, _)
        | (ConductorOutcome::ResourcePark { reason }, _)
        | (ConductorOutcome::OperatorStop { reason }, _) => {
            let kind = if reason.contains("quarantin") {
                accountability::EventKind::Quarantined { issue }
            } else if matches!(&receipt.outcome, ConductorOutcome::ResourcePark { .. }) {
                accountability::EventKind::Parked
            } else if matches!(&receipt.outcome, ConductorOutcome::OperatorStop { .. }) {
                accountability::EventKind::Stopped
            } else {
                accountability::EventKind::Failed
            };
            record_accountability_event(
                layout,
                accountability_event(
                    kind,
                    format!("Issue {issue} stopped before merge"),
                    "Non-merged outcomes remain visible without being counted as implemented",
                    reason.clone(),
                )?,
                true,
            )?;
        }
        (ConductorOutcome::Succeeded, None) => {
            return Err(CommandFailure::diagnostic(
                "merged executor receipt has no pull request for accountability",
            ));
        }
    }
    Ok(())
}

pub(super) fn accountability_root(
    layout: &RunLayout,
    explicit_epic: Option<u64>,
) -> Result<PathBuf, CommandFailure> {
    let primary = layout.state_dir.join("accountability");
    let Some(epic) = explicit_epic else {
        return Ok(primary);
    };
    if primary.exists() {
        let store = accountability::AccountabilityStore::open(&primary)
            .map_err(|error| CommandFailure::diagnostic(error.to_string()))?;
        if store.status().epic_number == Some(epic) {
            return Ok(primary);
        }
    }
    Ok(layout
        .state_dir
        .join("accountability-resumes")
        .join(format!("epic-{epic}")))
}

pub(super) fn archive_accountability_root(
    layout: &RunLayout,
    root: &Path,
) -> Result<(), CommandFailure> {
    let store = accountability::AccountabilityStore::open(root)
        .map_err(|error| CommandFailure::diagnostic(error.to_string()))?;
    let run_id = store
        .status()
        .run_id
        .unwrap_or_else(|| "unbound".to_string());
    drop(store);
    let history = layout.state_dir.join("accountability-history");
    fs::create_dir_all(&history).map_err(|error| {
        CommandFailure::diagnostic(format!("cannot create {}: {error}", history.display()))
    })?;
    #[cfg(unix)]
    fs::set_permissions(&history, fs::Permissions::from_mode(0o700)).map_err(|error| {
        CommandFailure::diagnostic(format!("cannot privatize {}: {error}", history.display()))
    })?;
    let serial = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let destination = history.join(format!("{}-{serial}", &run_id[..run_id.len().min(16)]));
    fs::rename(root, &destination).map_err(|error| {
        CommandFailure::diagnostic(format!(
            "cannot archive prior accountability run {}: {error}",
            root.display()
        ))
    })
}

pub(super) fn random_run_nonce() -> Result<String, CommandFailure> {
    let mut bytes = [0_u8; 16];
    File::open("/dev/urandom")
        .and_then(|mut source| source.read_exact(&mut bytes))
        .map_err(|error| {
            CommandFailure::diagnostic(format!(
                "cannot obtain cryptographic run nonce from /dev/urandom: {error}"
            ))
        })?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

pub(super) fn accountability_project_number() -> Option<u64> {
    let path = env_path("AUTOSPEC_HOME", &[".autospec"]).join("project-map.yml");
    let source = fs::read_to_string(path).ok()?;
    source.lines().find_map(|line| {
        let line = line.trim();
        let value = line
            .strip_prefix("autospec:run-accountability")
            .or_else(|| line.strip_prefix("\"autospec:run-accountability\""))
            .or_else(|| line.strip_prefix("'autospec:run-accountability'"))?
            .trim_start()
            .strip_prefix(':')?;
        value.trim().parse::<u64>().ok()
    })
}

#[derive(Debug, Clone, Default)]
pub(super) struct AccountabilityView {
    run_id: Option<String>,
    epic_number: Option<u64>,
    epic_url: Option<String>,
    event_count: u64,
    pending_projection_count: u64,
    desired_high_watermark: u64,
    acknowledged_high_watermark: u64,
    accountability_state: Option<String>,
    last_projected_at: Option<u64>,
    next_projection_retry_at: Option<u64>,
    recovery_state: Option<String>,
    created_at: Option<u64>,
    updated_at: Option<u64>,
    error: Option<String>,
}

impl AccountabilityView {
    fn projection_state(&self) -> &'static str {
        if self.error.is_some() || self.pending_projection_count > 0 {
            "degraded"
        } else if self.run_id.is_some() {
            "current"
        } else {
            "unbound"
        }
    }

    pub(super) fn json(&self) -> String {
        format!(
            "{{\"run_id\":{},\"epic_number\":{},\"epic_url\":{},\"accountability_state\":{},\"recovery_state\":{},\"event_count\":{},\"pending_projection_count\":{},\"last_projected_at\":{},\"next_projection_retry_at\":{},\"desired_high_watermark\":{},\"acknowledged_high_watermark\":{},\"created_at\":{},\"updated_at\":{},\"projection_state\":\"{}\",\"error\":{}}}",
            optional_json_string(self.run_id.as_deref()),
            self.epic_number.map_or_else(|| "null".to_string(), |value| value.to_string()),
            optional_json_string(self.epic_url.as_deref()),
            optional_json_string(self.accountability_state.as_deref()),
            optional_json_string(self.recovery_state.as_deref()),
            self.event_count,
            self.pending_projection_count,
            self.last_projected_at.map_or_else(|| "null".to_string(), |value| value.to_string()),
            self.next_projection_retry_at.map_or_else(|| "null".to_string(), |value| value.to_string()),
            self.desired_high_watermark,
            self.acknowledged_high_watermark,
            self.created_at.map_or_else(|| "null".to_string(), |value| value.to_string()),
            self.updated_at.map_or_else(|| "null".to_string(), |value| value.to_string()),
            self.projection_state(),
            optional_json_string(self.error.as_deref())
        )
    }

    pub(super) fn print_human(&self) {
        println!("accountability: {}", self.projection_state());
        if let Some(epic_url) = &self.epic_url {
            println!("accountability epic: {epic_url}");
        }
        println!(
            "accountability events: {} (projected {}/{})",
            self.event_count, self.acknowledged_high_watermark, self.desired_high_watermark
        );
        if let Some(error) = &self.error {
            println!("accountability error: {error}");
        }
    }
}

pub(super) fn accountability_view(layout: &RunLayout) -> AccountabilityView {
    let launch = read_launch_json(&layout.state_dir);
    let Some(run_id) = extract_json_string(&launch, "run_id") else {
        return AccountabilityView::default();
    };
    match find_accountability_root(layout, &run_id)
        .and_then(|root| accountability::AccountabilityStore::open(root).map_err(|e| e.to_string()))
    {
        Ok(store) => {
            let status = store.status();
            AccountabilityView {
                run_id: status.run_id,
                epic_number: status.epic_number,
                epic_url: status.epic_url,
                event_count: status.event_count,
                pending_projection_count: status.pending_projection_count,
                desired_high_watermark: status.desired_high_watermark,
                acknowledged_high_watermark: status.acknowledged_high_watermark,
                accountability_state: Some(status.accountability_state),
                last_projected_at: status.last_projected_at,
                next_projection_retry_at: status.next_projection_retry_at,
                recovery_state: Some(format!("{:?}", status.recovery_state).to_ascii_lowercase()),
                created_at: Some(status.created_at),
                updated_at: Some(status.updated_at),
                error: None,
            }
        }
        Err(error) => AccountabilityView {
            run_id: Some(run_id),
            error: Some(error),
            ..AccountabilityView::default()
        },
    }
}
