use super::*;

pub(super) fn validate_launch_mode(options: &Options) -> Result<LaunchMode, String> {
    let selected =
        usize::from(options.follow) + usize::from(options.detach) + usize::from(options.foreground);
    if selected > 1 {
        return Err("--follow, --detach, and --foreground are mutually exclusive".to_string());
    }
    if (options.follow || options.detach || options.foreground) && options.subcommand != "start" {
        return Err(format!(
            "launch modes are valid only with autospec autonomous start, not {}",
            options.subcommand
        ));
    }
    if options.follow && options.force {
        return Err(
            "--force cannot be combined with --follow; use autospec autonomous restart --force"
                .to_string(),
        );
    }
    if options.follow && options.json {
        return Err(
            "--json is not supported with --follow; use autospec autonomous status --json"
                .to_string(),
        );
    }
    if options.subcommand == "resume" && options.epic.is_none() {
        return Err("autospec autonomous resume requires --epic N".to_string());
    }
    if options.subcommand == "resume" && options.force {
        return Err("--force is not valid with resume".to_string());
    }
    if options.epic.is_some() && !matches!(options.subcommand.as_str(), "start" | "resume") {
        return Err("--epic is valid only with autospec autonomous start or resume".to_string());
    }
    Ok(if options.follow {
        LaunchMode::Follow
    } else if options.foreground {
        LaunchMode::Foreground
    } else {
        LaunchMode::Detached
    })
}

pub(super) fn spawn_unit(
    name: &str,
    command: &ForegroundCommand,
    repo_dir: &str,
    layout: &RunLayout,
    log_dir: &Path,
    log_override: Option<&str>,
    lease_token: Option<&str>,
) -> Result<UnitRecord, String> {
    let logpath = log_override
        .map(PathBuf::from)
        .unwrap_or_else(|| default_unit_logpath(name, log_dir));
    if let Some(parent) = logpath.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    }
    let log = File::create(&logpath)
        .map_err(|error| format!("cannot create {}: {error}", logpath.display()))?;
    let err_log = log
        .try_clone()
        .map_err(|error| format!("cannot clone {}: {error}", logpath.display()))?;
    let mut process = Command::new(&command.program);
    process
        .args(&command.args)
        .current_dir(repo_dir)
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(err_log));
    if let Some(token) = lease_token {
        process
            .env("AUTOSPEC_CONDUCTOR_LEASE_TOKEN", token)
            .env("AUTOSPEC_ACCOUNTABILITY_REQUIRED", "1");
    }
    #[cfg(unix)]
    process.process_group(0);
    let mut child = process
        .spawn()
        .map_err(|error| format!("cannot spawn {name} command: {error}"))?;
    let pid = child.id().to_string();
    let identity = process_identity(&pid).ok_or_else(|| {
        let _ = child.kill();
        let _ = child.wait();
        format!("cannot verify {name} process identity for pid {pid}")
    })?;
    let pid_file = layout.state_dir.join(format!("{name}.pid"));
    let logpath_file = layout.state_dir.join(format!("{name}.logpath"));
    let pid_metadata = format!(
        "{{\"pid\":{},\"repo\":\"{}\",\"scope\":\"{}\",\"pgid\":{},\"start_time_ticks\":{}}}\n",
        pid,
        json_escape(&layout.repo),
        json_escape(&layout.scope),
        identity.pgid,
        identity.start_time_ticks
    );
    if let Err(error) = fs::write(&pid_file, pid_metadata) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!("cannot write {}: {error}", pid_file.display()));
    }
    if let Err(error) = fs::write(&logpath_file, format!("{}\n", logpath.display())) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!("cannot write {}: {error}", logpath_file.display()));
    }
    Ok(UnitRecord {
        pid,
        pid_file,
        logpath,
        logpath_file,
    })
}

pub(super) fn default_unit_logpath(name: &str, log_dir: &Path) -> PathBuf {
    if name != "conductor" {
        return log_dir.join(format!("autospec-autonomous-{name}.log"));
    }
    let generation = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = ATOMIC_WRITE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    log_dir.join(format!(
        "autospec-autonomous-conductor-{generation}-{}-{sequence}.log",
        std::process::id()
    ))
}

pub(super) fn launch_units(
    layout: &RunLayout,
    options: &Options,
    foreground: &ForegroundCommand,
    commands: &LaunchCommands,
    lease: &resilience::ConductorLease,
) -> Result<(UnitRecord, UnitRecord, UnitRecord), CommandFailure> {
    let heartbeat = resilience::start_lifecycle_heartbeat(&layout.repo, lease)
        .map_err(resilience_lease_error)?;
    let launch_result =
        launch_units_with_lease_checks(layout, options, foreground, commands, lease);
    let heartbeat_result = heartbeat.finish().map_err(resilience_lease_error);
    let units = launch_result?;
    heartbeat_result?;
    Ok(units)
}

pub(super) fn mark_spawned_or_terminate(
    layout: &RunLayout,
    units: &(UnitRecord, UnitRecord, UnitRecord),
) -> Result<(), CommandFailure> {
    mark_spawned_with_cleanup(
        || mark_accountability_spawned(layout),
        || terminate_launched_units(units),
    )
}

fn mark_spawned_with_cleanup(
    mark: impl FnOnce() -> Result<(), CommandFailure>,
    cleanup: impl FnOnce(),
) -> Result<(), CommandFailure> {
    mark().inspect_err(|_| cleanup())
}

fn terminate_launched_units(units: &(UnitRecord, UnitRecord, UnitRecord)) {
    for unit in [&units.2, &units.1, &units.0] {
        if !unit.pid.is_empty() {
            let _ = terminate_process_group(&unit.pid);
        }
    }
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn failed_spawn_marker_terminates_launched_units() {
        let cleaned = Cell::new(false);
        let result = mark_spawned_with_cleanup(
            || Err(CommandFailure::diagnostic("durable marker failed")),
            || cleaned.set(true),
        );

        assert!(result.is_err());
        assert!(cleaned.get(), "cleanup must run before the error escapes");
    }
}

pub(super) fn launch_units_with_lease_checks(
    layout: &RunLayout,
    options: &Options,
    foreground: &ForegroundCommand,
    commands: &LaunchCommands,
    lease: &resilience::ConductorLease,
) -> Result<(UnitRecord, UnitRecord, UnitRecord), CommandFailure> {
    resilience::assert_lifecycle_before_spawn(&layout.repo, lease)
        .map_err(resilience_lease_error)?;
    let conductor = spawn_unit(
        "conductor",
        foreground,
        &options.repo_dir,
        layout,
        &layout.log_dir,
        log_override_for("conductor", options),
        Some(lease.token()),
    )
    .map_err(CommandFailure::diagnostic)?;
    if !companions_enabled() {
        return Ok((
            conductor,
            empty_unit("monitor", &layout.state_dir, &layout.log_dir),
            empty_unit("supervisor", &layout.state_dir, &layout.log_dir),
        ));
    }

    if let Err(error) = resilience::assert_lifecycle_before_spawn(&layout.repo, lease) {
        let _ = terminate_process_group(&conductor.pid);
        return Err(resilience_lease_error(error));
    }
    let monitor = match spawn_unit(
        "monitor",
        &commands.monitor,
        &options.repo_dir,
        layout,
        &layout.log_dir,
        log_override_for("monitor", options),
        None,
    ) {
        Ok(unit) => unit,
        Err(error) => {
            let _ = terminate_process_group(&conductor.pid);
            return Err(CommandFailure::diagnostic(error));
        }
    };
    if let Err(error) = resilience::assert_lifecycle_before_spawn(&layout.repo, lease) {
        let _ = terminate_process_group(&monitor.pid);
        let _ = terminate_process_group(&conductor.pid);
        return Err(resilience_lease_error(error));
    }
    let supervisor = match spawn_unit(
        "supervisor",
        &commands.supervisor,
        &options.repo_dir,
        layout,
        &layout.log_dir,
        log_override_for("supervisor", options),
        None,
    ) {
        Ok(unit) => unit,
        Err(error) => {
            let _ = terminate_process_group(&monitor.pid);
            let _ = terminate_process_group(&conductor.pid);
            return Err(CommandFailure::diagnostic(error));
        }
    };
    Ok((conductor, monitor, supervisor))
}
