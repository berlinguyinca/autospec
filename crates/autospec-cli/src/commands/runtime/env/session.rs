use std::collections::BTreeSet;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Child, ExitStatus};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

use autospec_core::runtime_env::{
    random_session_token, read_json, write_json_atomic, EnvironmentLifecycle, ReleaseDecision,
    ResourcePlan, RuntimeContext, RuntimeState, SessionRecord, SessionSet,
};

use crate::commands::CommandFailure;

use super::state::{
    layout_for_context, read_authoritative_state, write_lifecycle, EnvironmentLease,
};
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

    pub(super) fn reattach(
        environment_dir: &Path,
        expected_session_id: &str,
        harness: &str,
    ) -> Result<Self, CommandFailure> {
        let sessions = environment_dir.join("sessions");
        let lock_path = sessions.join(format!("{expected_session_id}.lock"));
        let record_path = sessions.join(format!("{expected_session_id}.json"));
        if !record_path.is_file() || !lock_path.is_file() {
            return Err(diagnostic(format!(
                "RUNTIME_PARTIAL_SESSION: {expected_session_id} requires both record and lock"
            )));
        }
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&lock_path)
            .map_err(|error| diagnostic(format!("open runtime session lock: {error}")))?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => {
                return Err(diagnostic(format!(
                    "RUNTIME_SESSION_LIVE: exact evidence session blocked by {expected_session_id}"
                )))
            }
            Err(std::fs::TryLockError::Error(error)) => {
                return Err(diagnostic(format!(
                    "could not reattach runtime session {expected_session_id}: {error}"
                )))
            }
        }
        let previous: SessionRecord = read_json(&record_path)
            .map_err(|error| diagnostic(format!("read runtime session record: {error}")))?;
        if previous.session_id != expected_session_id {
            return Err(diagnostic(
                "RUNTIME_SESSION_ID_MISMATCH: reattached record does not match filename",
            ));
        }
        let timestamp = unix_time_ms()?;
        let record = SessionRecord {
            schema_version: previous.schema_version,
            session_id: previous.session_id,
            pid: std::process::id(),
            process_start: token()?,
            harness: harness.to_string(),
            host: host(),
            started_at_unix_ms: previous.started_at_unix_ms,
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

    pub(super) fn verify_active(&self, expected_session_id: &str) -> Result<(), CommandFailure> {
        if self.record.session_id != expected_session_id {
            return Err(diagnostic(
                "runtime prepared session identity changed while active",
            ));
        }
        let observed: SessionRecord = read_json(&self.record_path).map_err(|error| {
            diagnostic(format!(
                "runtime prepared session record is unreadable: {error}"
            ))
        })?;
        if observed != self.record {
            return Err(diagnostic(
                "runtime prepared session durable record changed while active",
            ));
        }
        for path in [&self.record_path, &self.lock_path] {
            let metadata = std::fs::symlink_metadata(path).map_err(|error| {
                diagnostic(format!(
                    "runtime prepared session artifact {} is missing: {error}",
                    path.display()
                ))
            })?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(diagnostic(
                    "runtime prepared session artifact is not a regular file",
                ));
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::{MetadataExt, PermissionsExt};
                // SAFETY: geteuid has no arguments or memory-safety preconditions.
                let owner = unsafe { nix::libc::geteuid() };
                if metadata.uid() != owner || metadata.permissions().mode() & 0o077 != 0 {
                    return Err(diagnostic(
                        "runtime prepared session artifact ownership or mode is unsafe",
                    ));
                }
            }
        }
        Ok(())
    }

    pub(super) fn release(self) -> Result<(), CommandFailure> {
        remove_if_present(&self.record_path)?;
        drop(self._lock);
        remove_if_present(&self.lock_path)
    }
}

pub(super) fn reconcile_for_exclusive_session(
    environment_dir: &Path,
) -> Result<(), CommandFailure> {
    let sessions = environment_dir.join("sessions");
    std::fs::create_dir_all(&sessions)
        .map_err(|error| diagnostic(format!("create runtime sessions directory: {error}")))?;
    let mut ids = BTreeSet::new();
    for entry in std::fs::read_dir(&sessions)
        .map_err(|error| diagnostic(format!("inventory runtime sessions: {error}")))?
    {
        let path = entry.map_err(|error| diagnostic(error.to_string()))?.path();
        if matches!(
            path.extension().and_then(|extension| extension.to_str()),
            Some("json" | "lock")
        ) {
            let id = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .ok_or_else(|| diagnostic("runtime session artifact name is not UTF-8"))?;
            ids.insert(id.to_string());
        }
    }
    for id in ids {
        let record_path = sessions.join(format!("{id}.json"));
        let lock_path = sessions.join(format!("{id}.lock"));
        if !record_path.is_file() || !lock_path.is_file() {
            return Err(diagnostic(format!(
                "RUNTIME_PARTIAL_SESSION: {id} requires both record and lock"
            )));
        }
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&lock_path)
            .map_err(|error| diagnostic(format!("open runtime session lock: {error}")))?;
        match lock.try_lock() {
            Ok(()) => {
                let record: SessionRecord = read_json(&record_path)
                    .map_err(|error| diagnostic(format!("read stale session record: {error}")))?;
                if record.session_id != id {
                    return Err(diagnostic(
                        "RUNTIME_SESSION_ID_MISMATCH: stale record does not match filename",
                    ));
                }
                remove_if_present(&record_path)?;
                drop(lock);
                remove_if_present(&lock_path)?;
            }
            Err(std::fs::TryLockError::WouldBlock) => {
                return Err(diagnostic(format!(
                    "RUNTIME_SESSION_LIVE: exclusive evidence session blocked by {id}"
                )))
            }
            Err(std::fs::TryLockError::Error(error)) => {
                return Err(diagnostic(format!(
                    "could not reconcile runtime session {id}: {error}"
                )))
            }
        }
    }
    Ok(())
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

pub(super) fn add_secondary_failure(
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

pub(super) fn cleanup_session(
    context: &RuntimeContext,
    state: &RuntimeState,
    plan: &ResourcePlan,
    session_lease: SessionLease,
    should_teardown: bool,
) -> Result<(), CommandFailure> {
    let _lease = EnvironmentLease::acquire(&context.environment_dir)?;
    let cleanup = (|| {
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
    })();
    let Err(primary) = cleanup else {
        return Ok(());
    };
    super::wait_for_cleanup_failure_test_hook();
    match record_cleanup_failure(context) {
        Ok(()) => Err(primary),
        Err(evidence) => Err(add_secondary_failure(
            Some(primary),
            "persist cleanup-failure evidence also failed",
            evidence,
        )),
    }
}

fn record_cleanup_failure(context: &RuntimeContext) -> Result<(), CommandFailure> {
    let layout = layout_for_context(context);
    let Some(mut authoritative) = read_authoritative_state(&layout)? else {
        return Err(CommandFailure::diagnostic(
            "RUNTIME_PARTIAL_STATE: cleanup failure has no authoritative owner",
        ));
    };
    write_lifecycle(
        &layout,
        &mut authoritative.owner,
        EnvironmentLifecycle::CleanupFailed,
    )
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

pub(super) fn verify_session_released(
    environment_dir: &Path,
    session_id: &str,
) -> Result<(), CommandFailure> {
    let sessions = environment_dir.join("sessions");
    for path in [
        sessions.join(format!("{session_id}.json")),
        sessions.join(format!("{session_id}.lock")),
    ] {
        match std::fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Ok(_) => {
                return Err(diagnostic(format!(
                    "RUNTIME_SESSION_RELEASE_UNVERIFIED: {} still exists",
                    path.display()
                )))
            }
            Err(error) => {
                return Err(diagnostic(format!(
                    "RUNTIME_SESSION_RELEASE_UNVERIFIED: cannot inspect {}: {error}",
                    path.display()
                )))
            }
        }
    }
    Ok(())
}

pub(super) fn verify_environment_released(environment_dir: &Path) -> Result<(), CommandFailure> {
    for name in [
        "owner.json",
        "plan.json",
        "env",
        "inventory.json",
        "sessions",
    ] {
        let path = environment_dir.join(name);
        match std::fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Ok(_) => {
                return Err(diagnostic(format!(
                    "RUNTIME_RESOURCE_RELEASE_UNVERIFIED: {} still exists",
                    path.display()
                )))
            }
            Err(error) => {
                return Err(diagnostic(format!(
                    "RUNTIME_RESOURCE_RELEASE_UNVERIFIED: cannot inspect {}: {error}",
                    path.display()
                )))
            }
        }
    }
    Ok(())
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
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let file = options
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

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "autospec-session-reconcile-{name}-{}-{}",
            std::process::id(),
            unix_time_ms().expect("timestamp")
        ));
        std::fs::create_dir_all(root.join("sessions")).expect("sessions fixture");
        root
    }

    fn record(id: &str) -> SessionRecord {
        SessionRecord {
            schema_version: 1,
            session_id: id.to_string(),
            pid: 1,
            process_start: "start".into(),
            harness: "test".into(),
            host: "host".into(),
            started_at_unix_ms: 1,
            heartbeat_at_unix_ms: 1,
        }
    }

    #[test]
    fn exclusive_reconciliation_removes_exact_unlocked_stale_pair() {
        let root = fixture("stale");
        let sessions = root.join("sessions");
        write_record(&sessions.join("stale.json"), &record("stale")).expect("record");
        File::create(sessions.join("stale.lock")).expect("lock");

        reconcile_for_exclusive_session(&root).expect("stale reconciliation");

        assert!(!sessions.join("stale.json").exists());
        assert!(!sessions.join("stale.lock").exists());
        reconcile_for_exclusive_session(&root).expect("absent pair is idempotent");
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn exclusive_reconciliation_rejects_partial_record_or_lock() {
        for extension in ["json", "lock"] {
            let root = fixture(extension);
            let path = root.join("sessions").join(format!("partial.{extension}"));
            if extension == "json" {
                write_record(&path, &record("partial")).expect("partial record");
            } else {
                File::create(&path).expect("partial lock");
            }

            let error = reconcile_for_exclusive_session(&root)
                .expect_err("partial session evidence must fail");

            assert!(error.message.contains("RUNTIME_PARTIAL_SESSION"));
            std::fs::remove_dir_all(root).expect("cleanup");
        }
    }

    #[test]
    fn exclusive_reconciliation_rejects_live_held_lock() {
        let root = fixture("live");
        let sessions = root.join("sessions");
        write_record(&sessions.join("live.json"), &record("live")).expect("record");
        let lock = File::create(sessions.join("live.lock")).expect("lock");
        lock.lock().expect("hold live lock");

        let error =
            reconcile_for_exclusive_session(&root).expect_err("live session must block rotation");

        assert!(error.message.contains("RUNTIME_SESSION_LIVE"));
        drop(lock);
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn exact_stale_session_is_reattached_without_rotating_its_identity() {
        let root = fixture("reattach");
        let sessions = root.join("sessions");
        write_record(&sessions.join("durable.json"), &record("durable")).expect("record");
        File::create(sessions.join("durable.lock")).expect("lock");
        #[cfg(unix)]
        for path in [sessions.join("durable.json"), sessions.join("durable.lock")] {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
                .expect("private session artifact");
        }

        let lease =
            SessionLease::reattach(&root, "durable", "recovered-harness").expect("reattach");

        assert_eq!(lease.session_id(), "durable");
        lease
            .verify_active("durable")
            .expect("active exact session");
        let observed: SessionRecord =
            read_json(&sessions.join("durable.json")).expect("recovered record");
        assert_eq!(observed.session_id, "durable");
        assert_eq!(observed.harness, "recovered-harness");
        assert_eq!(observed.pid, std::process::id());
        lease.release().expect("release");
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn exact_live_session_cannot_be_stolen_by_reattachment() {
        let root = fixture("reattach-live");
        let original = SessionLease::register(&root, "original").expect("register");
        let session_id = original.session_id().to_string();

        let error = match SessionLease::reattach(&root, &session_id, "replacement") {
            Ok(_) => panic!("live session must remain exclusive"),
            Err(error) => error,
        };

        assert!(error.message.contains("RUNTIME_SESSION_LIVE"));
        original.release().expect("release");
        std::fs::remove_dir_all(root).expect("cleanup");
    }
}
