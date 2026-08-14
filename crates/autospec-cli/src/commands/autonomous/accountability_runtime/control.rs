use super::*;

pub(crate) fn retry_pending_accountability_projection(
    layout: &RunLayout,
    lease: &resilience::ConductorLease,
) -> Result<(), CommandFailure> {
    let mut store = open_bound_accountability(layout)?;
    let status = store.status();
    if status.pending_projection_count == 0 || status.next_projection_retry_at.is_none() {
        return Ok(());
    }
    let request = accountability::github::EpicBindingRequest {
        repository: accountability::RepositoryIdentity::parse(&layout.repo)
            .map_err(|error| CommandFailure::diagnostic(error.to_string()))?,
        explicit_epic: None,
        resume_policy: accountability::github::ResumePolicy::ActiveOnly,
        project_number: None,
        adopted_lease_generation: Some(lease.generation()),
    };
    let mut github = accountability::github::GhCli;
    match accountability::github::bind_epic(&mut store, &mut github, request, || {
        resilience::renew_lifecycle(&layout.repo, lease)
            .map_err(|_| "lifecycle lease renewal rejected".to_string())
    }) {
        Ok(_) => Ok(()),
        Err(error)
            if error.projection_disposition()
                == Some(accountability::ProjectionDisposition::DegradableTransport) =>
        {
            eprintln!(
                "autospec autonomous accountability retry degraded: {error}; next retry remains durable"
            );
            Ok(())
        }
        Err(error) => Err(CommandFailure::diagnostic(format!(
            "accountability retry integrity check failed: {error}"
        ))),
    }
}

pub(crate) fn repair_stopped_conductor(
    layout: &RunLayout,
    options: &Options,
) -> Result<RepairOutcome, String> {
    let _repair_lock = acquire_supervisor_repair_lock(layout)?;
    if persisted_stop_mode(layout)?.is_some() {
        return Ok(RepairOutcome::StopRequested);
    }
    let recorded = read_unit("conductor", layout);
    if recorded.metadata_state == UnitMetadataState::Live {
        return Ok(RepairOutcome::AlreadyRunning(recorded.pid));
    }
    if recorded.metadata_state == UnitMetadataState::Ambiguous {
        return Err("conductor metadata is ambiguous".to_string());
    }
    // Clear stale records while the terminated owner's lease still fences new launches. If the
    // lease were released first, another supervisor could publish replacement metadata here.
    remove_stale_unit_metadata(&recorded)?;
    if !recorded.pid.is_empty() {
        reap_terminated_child(&recorded.pid)?;
        release_terminated_owner(layout, &recorded.pid)?;
    }
    let (lifecycle, lease) = acquire_lifecycle_start(layout, options, LifecycleTransition::Start)
        .map_err(|error| error.into_command_failure().message)?;
    let heartbeat = resilience::start_lifecycle_heartbeat(&layout.repo, &lease)
        .map_err(|error| resilience_lease_error(error).message)?;
    let repaired = (|| {
        resilience::assert_lifecycle_before_spawn(&layout.repo, &lease)
            .map_err(|error| resilience_lease_error(error).message)?;
        create_launch_directories(layout).map_err(|error| error.message)?;
        persist_lifecycle_decision(layout, &lifecycle)?;
        verify_existing_accountability(layout, &lease)?;
        if !accountability_allows_relaunch(
            open_bound_accountability(layout)
                .map_err(|error| error.message)?
                .status()
                .recovery_state,
        ) {
            return Ok(None);
        }
        let command = foreground_command(options)?;
        resilience::assert_lifecycle_before_spawn(&layout.repo, &lease)
            .map_err(|error| resilience_lease_error(error).message)?;
        spawn_unit(
            "conductor",
            &command,
            &options.repo_dir,
            layout,
            &layout.log_dir,
            log_override_for("conductor", options),
            Some(lease.token()),
        )
        .map(Some)
    })();
    let heartbeat_result = heartbeat
        .finish()
        .map_err(|error| resilience_lease_error(error).message);
    match (repaired, heartbeat_result) {
        (Ok(Some(unit)), Ok(())) => Ok(RepairOutcome::Restarted(unit)),
        (Ok(Some(unit)), Err(error)) => {
            let _ = terminate_process_group(&unit.pid);
            release_launch_lease(&layout.repo, &lease).map_err(|error| error.message)?;
            Err(error)
        }
        (Ok(None), heartbeat_result) => {
            heartbeat_result?;
            release_launch_lease(&layout.repo, &lease).map_err(|error| error.message)?;
            Ok(RepairOutcome::TerminalAccountability)
        }
        (Err(error), _) => {
            release_launch_lease(&layout.repo, &lease).map_err(|error| error.message)?;
            Err(error)
        }
    }
}

pub(super) fn accountability_allows_relaunch(
    recovery_state: accountability::RecoveryState,
) -> bool {
    recovery_state == accountability::RecoveryState::Active
}

pub(crate) fn verify_existing_accountability(
    layout: &RunLayout,
    lease: &resilience::ConductorLease,
) -> Result<(), String> {
    let run_id = serde_json::from_str::<serde_json::Value>(&read_launch_json(&layout.state_dir))
        .ok()
        .and_then(|value| {
            value
                .get("accountability")?
                .get("run_id")?
                .as_str()
                .map(str::to_owned)
        })
        .ok_or_else(|| "launch metadata has no accountability binding".to_string())?;
    let root = find_accountability_root(layout, &run_id)?;
    let mut store =
        accountability::AccountabilityStore::open(root).map_err(|error| error.to_string())?;
    let repository = accountability::RepositoryIdentity::parse(&layout.repo)
        .map_err(|error| error.to_string())?;
    let request = accountability::github::EpicBindingRequest {
        repository,
        explicit_epic: None,
        resume_policy: accountability::github::ResumePolicy::ActiveOnly,
        project_number: None,
        adopted_lease_generation: Some(lease.generation()),
    };
    let mut github = accountability::github::GhCli;
    accountability::github::bind_epic(&mut store, &mut github, request, || {
        resilience::renew_lifecycle(&layout.repo, lease)
            .map_err(|_| "lifecycle lease renewal rejected".to_string())
    })
    .map(|_| ())
    .map_err(|error| error.to_string())
}

pub(crate) fn find_accountability_root(
    layout: &RunLayout,
    run_id: &str,
) -> Result<PathBuf, String> {
    let primary = layout.state_dir.join("accountability");
    let resumes = layout.state_dir.join("accountability-resumes");
    let mut candidates = vec![primary];
    if let Ok(entries) = fs::read_dir(resumes) {
        candidates.extend(entries.flatten().map(|entry| entry.path()));
    }
    let mut matches = Vec::new();
    for candidate in candidates.into_iter().filter(|path| path.is_dir()) {
        let store = accountability::AccountabilityStore::open(&candidate)
            .map_err(|error| error.to_string())?;
        if store.status().run_id.as_deref() == Some(run_id) {
            matches.push(candidate);
        }
    }
    match matches.len() {
        1 => Ok(matches.remove(0)),
        0 => Err("accountability state for launch run_id is missing".to_string()),
        _ => Err("multiple local accountability stores match launch run_id".to_string()),
    }
}

pub(crate) fn acquire_supervisor_repair_lock(layout: &RunLayout) -> Result<File, String> {
    fs::create_dir_all(&layout.state_dir)
        .map_err(|error| format!("cannot create {}: {error}", layout.state_dir.display()))?;
    let path = layout.state_dir.join("supervisor-repair.lock");
    if let Ok(metadata) = fs::symlink_metadata(&path) {
        if !metadata.file_type().is_file() {
            return Err(format!(
                "supervisor repair lock is not a regular file: {}",
                path.display()
            ));
        }
    }
    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true);
    #[cfg(unix)]
    options.mode(0o600).custom_flags(nix::libc::O_NOFOLLOW);
    let lock = options.open(&path).map_err(|error| {
        format!(
            "cannot open supervisor repair lock {}: {error}",
            path.display()
        )
    })?;
    if !lock
        .metadata()
        .map_err(|error| format!("cannot inspect supervisor repair lock: {error}"))?
        .is_file()
    {
        return Err("supervisor repair lock is not a regular file".to_string());
    }
    lock.lock()
        .map_err(|error| format!("cannot acquire supervisor repair lock: {error}"))?;
    Ok(lock)
}

pub(crate) fn wait_for_follow_target_after_held(
    layout: &RunLayout,
) -> Result<Option<UnitStatus>, String> {
    const ATTEMPTS: usize = 50;
    const RETRY_INTERVAL: Duration = Duration::from_millis(100);

    println!("autospec autonomous follow: waiting for scoped conductor metadata after held lifecycle lease");
    io::stdout()
        .flush()
        .map_err(|error| format!("cannot flush stdout: {error}"))?;
    for _ in 0..ATTEMPTS {
        if let Some(conductor) = live_follow_target(layout)? {
            return Ok(Some(conductor));
        }
        thread::sleep(RETRY_INTERVAL);
    }
    Ok(None)
}

pub(crate) fn verify_follow_accountability(
    layout: &RunLayout,
    requested_epic: Option<u64>,
) -> Result<(), String> {
    let launch = read_launch_json(&layout.state_dir);
    let run_id = extract_json_string(&launch, "run_id")
        .ok_or_else(|| "live conductor launch has no accountability run_id".to_string())?;
    let launch_epic = extract_json_number(&launch, "epic_number")
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| "live conductor launch has no accountability epic".to_string())?;
    if requested_epic.is_some_and(|requested| requested != launch_epic) {
        return Err(format!(
            "live conductor is bound to epic {launch_epic}, not requested epic {}",
            requested_epic.unwrap()
        ));
    }
    let root = find_accountability_root(layout, &run_id)?;
    let store =
        accountability::AccountabilityStore::open(root).map_err(|error| error.to_string())?;
    if store.status().epic_number != Some(launch_epic) {
        return Err("live conductor launch and local accountability binding disagree".to_string());
    }
    Ok(())
}

pub(crate) fn record_accountability_event_command(args: &[String]) -> Result<(), CommandFailure> {
    let mut repo = None;
    let mut kind = None;
    let mut issue = None;
    let mut pull_request = None;
    let mut what = None;
    let mut why = None;
    let mut evidence = None;
    let mut project = false;
    let mut index = 1;
    while index < args.len() {
        let flag = args[index].as_str();
        if flag == "--project" {
            project = true;
            index += 1;
            continue;
        }
        let value = args
            .get(index + 1)
            .ok_or_else(|| CommandFailure::diagnostic(format!("{flag} requires a value")))?
            .clone();
        match flag {
            "--repo" => repo = Some(value),
            "--kind" => kind = Some(value),
            "--issue" => issue = value.parse::<u64>().ok(),
            "--pr" => pull_request = value.parse::<u64>().ok(),
            "--what" => what = Some(value),
            "--why" => why = Some(value),
            "--evidence" => evidence = Some(value),
            _ => {
                return Err(CommandFailure::diagnostic(format!(
                    "unknown accountability-event option: {flag}"
                )))
            }
        }
        index += 2;
    }
    let repo = repo.ok_or_else(|| CommandFailure::diagnostic("--repo is required"))?;
    let kind_name = kind.ok_or_else(|| CommandFailure::diagnostic("--kind is required"))?;
    let kind = match kind_name.as_str() {
        "selected" => accountability::EventKind::WorkSelected { issue },
        "claimed" => accountability::EventKind::IssueClaimed {
            issue: issue.ok_or_else(|| CommandFailure::diagnostic("claimed requires --issue"))?,
        },
        "pr-opened" => accountability::EventKind::PullRequestOpened {
            pull_request: pull_request
                .ok_or_else(|| CommandFailure::diagnostic("pr-opened requires --pr"))?,
        },
        "review" => accountability::EventKind::ReviewStarted {
            pull_request: pull_request
                .ok_or_else(|| CommandFailure::diagnostic("review requires --pr"))?,
        },
        "verified" => accountability::EventKind::Verified,
        "merged" => accountability::EventKind::Merged {
            pull_request: pull_request
                .ok_or_else(|| CommandFailure::diagnostic("merged requires --pr"))?,
        },
        "failed" => accountability::EventKind::Failed,
        "quarantined" => accountability::EventKind::Quarantined {
            issue: issue
                .ok_or_else(|| CommandFailure::diagnostic("quarantined requires --issue"))?,
        },
        "parked" => accountability::EventKind::Parked,
        "stopped" => accountability::EventKind::Stopped,
        "completed" => accountability::EventKind::Completed,
        _ => {
            return Err(CommandFailure::diagnostic(format!(
                "unknown accountability event kind: {kind_name}"
            )))
        }
    };
    let layout = RunLayout::new(&Options {
        repo,
        ..Options::default()
    })
    .map_err(CommandFailure::diagnostic)?;
    record_accountability_event(
        &layout,
        accountability_event(
            kind,
            what.ok_or_else(|| CommandFailure::diagnostic("--what is required"))?,
            why.ok_or_else(|| CommandFailure::diagnostic("--why is required"))?,
            evidence.ok_or_else(|| CommandFailure::diagnostic("--evidence is required"))?,
        )?,
        project,
    )?;
    println!("{{\"event\":\"accountability_recorded\",\"projection_requested\":{project}}}");
    Ok(())
}
