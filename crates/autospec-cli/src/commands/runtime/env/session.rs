use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Child, ExitStatus};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use autospec_core::runtime_env::{
    random_session_token, read_json, write_json_atomic, ReleaseDecision, ResourcePlan,
    RuntimeContext, RuntimeState, SessionRecord, SessionSet,
};

use crate::commands::CommandFailure;

use super::state::EnvironmentLease;
use super::worker::ChildGuard;

enum SessionWait {
    Exited(ExitStatus),
    Interrupted(i32),
}

pub(super) struct SessionLease {
    _lock: File,
    lock_path: PathBuf,
    record_path: PathBuf,
    record: SessionRecord,
}

impl SessionLease {
    pub(super) fn register(environment_dir: &Path, harness: &str) -> Result<Self, CommandFailure> {
        let sessions = environment_dir.join("sessions");
        std::fs::create_dir_all(&sessions).map_err(|error| {
            diagnostic(format!(
                "could not create runtime session directory {}: {error}",
                sessions.display()
            ))
        })?;
        let session_id = token()?;
        let process_start = token()?;
        let lock_path = sessions.join(format!("{session_id}.lock"));
        let record_path = sessions.join(format!("{session_id}.json"));
        let lock = create_locked(&lock_path)?;
        let timestamp = unix_time_ms()?;
        let record = SessionRecord {
            schema_version: 1,
            session_id,
            pid: std::process::id(),
            process_start,
            harness: harness.to_string(),
            host: host(),
            started_at_unix_ms: timestamp,
            heartbeat_at_unix_ms: timestamp,
        };
        write_record(&record_path, &record)?;
        Ok(Self {
            _lock: lock,
            lock_path,
            record_path,
            record,
        })
    }

    pub(super) fn heartbeat(&mut self) -> Result<(), CommandFailure> {
        self.record.heartbeat_at_unix_ms = unix_time_ms()?;
        write_record(&self.record_path, &self.record)
    }

    pub(super) fn session_id(&self) -> &str {
        &self.record.session_id
    }

    pub(super) fn release(self) -> Result<(), CommandFailure> {
        remove_if_present(&self.record_path)?;
        drop(self._lock);
        remove_if_present(&self.lock_path)
    }
}

pub(super) fn run_session_command(
    command: &[String],
    context: &RuntimeContext,
    state: &RuntimeState,
    plan: &ResourcePlan,
    mut session_lease: SessionLease,
    keep_alive: bool,
    bypassed: bool,
) -> Result<(), CommandFailure> {
    let child = match super::worker::spawn_direct_command(
        command,
        &context.repo,
        Some((context, state)),
        bypassed,
        true,
    ) {
        Ok(child) => child,
        Err(error) => {
            let cleanup = cleanup_session(context, state, plan, session_lease, true);
            return session_result(Err(error), cleanup);
        }
    };
    let mut child = ChildGuard::new(child, cfg!(unix));
    let mut result = wait_for_session_child(child.child(), &mut session_lease);
    if matches!(result, Ok(SessionWait::Exited(_))) {
        child.disarm();
    } else if let Err(error) = child.terminate() {
        result = Err(add_secondary_failure(
            result.err(),
            "runtime harness termination also failed",
            error,
        ));
        child.wait_for_natural_group_exit();
    }
    let should_teardown = matches!(&result, Ok(SessionWait::Interrupted(_))) || !keep_alive;
    let cleanup = cleanup_session(context, state, plan, session_lease, should_teardown);
    session_result(result, cleanup)
}

fn session_result(
    result: Result<SessionWait, CommandFailure>,
    cleanup: Result<(), CommandFailure>,
) -> Result<(), CommandFailure> {
    let primary = match result {
        Ok(SessionWait::Interrupted(signal)) => Err(CommandFailure::status(
            String::new(),
            if signal == 2 { 130 } else { 143 },
        )),
        Ok(SessionWait::Exited(status)) => super::child_status(status),
        Err(error) => Err(error),
    };
    match (primary, cleanup) {
        (Ok(()), cleanup) => cleanup,
        (Err(primary), Ok(())) => Err(primary),
        (Err(primary), Err(cleanup)) => Err(add_secondary_failure(
            Some(primary),
            "runtime session cleanup also failed",
            cleanup,
        )),
    }
}

fn add_secondary_failure(
    primary: Option<CommandFailure>,
    label: &str,
    secondary: CommandFailure,
) -> CommandFailure {
    let Some(primary) = primary else {
        return secondary;
    };
    let separator = if primary.message.is_empty() { "" } else { "; " };
    let secondary_message = if secondary.message.is_empty() {
        format!("exit {}", secondary.exit_code)
    } else {
        secondary.message
    };
    CommandFailure::status(
        format!("{}{separator}{label}: {secondary_message}", primary.message),
        primary.exit_code,
    )
}

fn cleanup_session(
    context: &RuntimeContext,
    state: &RuntimeState,
    plan: &ResourcePlan,
    session_lease: SessionLease,
    should_teardown: bool,
) -> Result<(), CommandFailure> {
    let _lease = EnvironmentLease::acquire(&context.environment_dir)?;
    let session_id = session_lease.session_id().to_string();
    let mut sessions = SessionSet::default();
    for record in live_sessions(&context.environment_dir)? {
        sessions.register(record);
    }
    session_lease.release()?;
    let decision = sessions.release(&session_id);
    if should_teardown && decision == ReleaseDecision::TearDown {
        super::lifecycle::teardown_locked(context, Some(state), plan)?;
    }
    Ok(())
}

#[cfg(unix)]
fn wait_for_session_child(
    child: &mut Child,
    session_lease: &mut SessionLease,
) -> Result<SessionWait, CommandFailure> {
    let mut last_heartbeat = Instant::now();
    loop {
        let signal = super::received_session_signal();
        if signal == 2 || signal == 15 {
            return Ok(SessionWait::Interrupted(signal));
        }
        if let Some(status) = child.try_wait().map_err(wait_error)? {
            return Ok(SessionWait::Exited(status));
        }
        heartbeat_if_due(session_lease, &mut last_heartbeat)?;
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(not(unix))]
fn wait_for_session_child(
    child: &mut Child,
    _session_lease: &mut SessionLease,
) -> Result<SessionWait, CommandFailure> {
    child.wait().map(SessionWait::Exited).map_err(wait_error)
}

fn heartbeat_if_due(
    session_lease: &mut SessionLease,
    last_heartbeat: &mut Instant,
) -> Result<(), CommandFailure> {
    if last_heartbeat.elapsed() >= Duration::from_secs(1) {
        session_lease.heartbeat()?;
        *last_heartbeat = Instant::now();
    }
    Ok(())
}

fn wait_error(error: std::io::Error) -> CommandFailure {
    diagnostic(format!("could not wait for runtime child command: {error}"))
}

pub(super) fn live_sessions(environment_dir: &Path) -> Result<Vec<SessionRecord>, CommandFailure> {
    let sessions = environment_dir.join("sessions");
    let entries = match std::fs::read_dir(&sessions) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(diagnostic(format!(
                "could not read {}: {error}",
                sessions.display()
            )))
        }
    };
    let mut live = Vec::new();
    for entry in entries {
        let path = entry.map_err(|error| diagnostic(error.to_string()))?.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        inspect_record(&path, &mut live)?;
    }
    Ok(live)
}

fn inspect_record(path: &Path, live: &mut Vec<SessionRecord>) -> Result<(), CommandFailure> {
    let lock_path = path.with_extension("lock");
    let lock = open_lock(&lock_path)?;
    match lock.try_lock() {
        Ok(()) => {
            drop(lock);
            remove_if_present(path)?;
            remove_if_present(&lock_path)
        }
        Err(std::fs::TryLockError::WouldBlock) => {
            live.push(read_json(path).map_err(|error| diagnostic(error.to_string()))?);
            Ok(())
        }
        Err(std::fs::TryLockError::Error(error)) => Err(diagnostic(format!(
            "could not inspect runtime session lock {}: {error}",
            lock_path.display()
        ))),
    }
}

fn create_locked(path: &Path) -> Result<File, CommandFailure> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| diagnostic(format!("could not create {}: {error}", path.display())))?;
    file.lock()
        .map_err(|error| diagnostic(format!("could not lock {}: {error}", path.display())))?;
    Ok(file)
}

fn open_lock(path: &Path) -> Result<File, CommandFailure> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|error| diagnostic(format!("could not open {}: {error}", path.display())))
}

fn write_record(path: &Path, record: &SessionRecord) -> Result<(), CommandFailure> {
    write_json_atomic(path, record).map_err(|error| diagnostic(error.to_string()))
}

fn remove_if_present(path: &Path) -> Result<(), CommandFailure> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(diagnostic(format!(
            "could not remove {}: {error}",
            path.display()
        ))),
    }
}

fn unix_time_ms() -> Result<u64, CommandFailure> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| diagnostic(format!("system clock is before Unix epoch: {error}")))?;
    u64::try_from(elapsed.as_millis())
        .map_err(|_| diagnostic("runtime session timestamp exceeds u64"))
}

fn token() -> Result<String, CommandFailure> {
    random_session_token().map_err(|error| diagnostic(error.to_string()))
}

fn host() -> String {
    std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown-host".to_string())
}

fn diagnostic(message: impl Into<String>) -> CommandFailure {
    CommandFailure::diagnostic(message)
}
