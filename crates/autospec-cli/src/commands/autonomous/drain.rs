use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Component, Path, PathBuf};
use std::process::{Child, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use autospec_core::autonomous::drain::{
    decide, DrainDecision, DrainExecutorInput, DrainObservation, DrainProgress,
};
use autospec_core::autonomous_lifecycle::RepositoryScope;

use super::{json_escape, Command, CommandFailure, Options, RunLayout};

const GITHUB_SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(15);
const OBSERVER_POLL_INTERVAL: Duration = Duration::from_millis(25);
const SESSION_RECONCILIATION_TIMEOUT_SECS: u64 = 300;
const SESSION_STARTUP_RETRY_LIMIT: u32 = 3;
const STARTUP_RETRY_STATE_FILE: &str = "drain-startup-retry.json";

enum ChildTermination {
    Exited(ExitStatus),
    Terminated,
}

enum GithubSnapshot {
    Available(String),
    ChildExited(ExitStatus),
    Unavailable,
}

enum GithubOutput {
    Available(String),
    ChildExited(ExitStatus),
    Unavailable,
}

struct OutputReaders {
    handles: Vec<JoinHandle<()>>,
    session_events: Receiver<SessionOutputEvent>,
}

struct DrainAttemptGuard {
    child: Child,
    readers: OutputReaders,
}

impl Drop for DrainAttemptGuard {
    fn drop(&mut self) {
        let _ = terminate_child(&mut self.child);
        join_readers(&mut self.readers);
    }
}

enum SessionOutputEvent {
    Started {
        session_id: String,
        observed_at: Instant,
        requires_reconciliation: bool,
    },
    Reconciled {
        session_id: String,
        observed_at: Instant,
    },
}

#[derive(Default)]
struct SessionStartupTracker {
    pending: HashMap<String, Instant>,
    reconciled: HashMap<String, Instant>,
    matched_reconciliation: bool,
}

enum StartupRetryDecision {
    Scheduled,
    Exhausted,
}

enum DrainAttemptEnd {
    Complete(ExitStatus),
    StartupTimedOut { session_id: String },
    Stalled { elapsed_secs: u64 },
}

#[derive(Default)]
struct StartupRetryState {
    failures: u32,
}

impl StartupRetryState {
    fn load(layout: &RunLayout) -> Result<Self, CommandFailure> {
        let path = layout.state_dir.join(STARTUP_RETRY_STATE_FILE);
        let raw = match fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => {
                return Err(CommandFailure::diagnostic(format!(
                    "cannot read {}: {error}",
                    path.display()
                )))
            }
        };
        let schema = super::extract_json_number(&raw, "schema");
        let repo = super::extract_json_string(&raw, "repo");
        let failures = super::extract_json_number(&raw, "failures")
            .and_then(|value| value.parse::<u32>().ok());
        if schema.as_deref() != Some("1") || repo.as_deref() != Some(&layout.repo) {
            return Err(CommandFailure::diagnostic(format!(
                "cannot recover startup retry state from {}",
                path.display()
            )));
        }
        failures.map(|failures| Self { failures }).ok_or_else(|| {
            CommandFailure::diagnostic(format!(
                "cannot recover startup retry state from {}",
                path.display()
            ))
        })
    }

    fn record_failure(&mut self) -> StartupRetryDecision {
        self.failures += 1;
        if self.failures <= SESSION_STARTUP_RETRY_LIMIT {
            StartupRetryDecision::Scheduled
        } else {
            StartupRetryDecision::Exhausted
        }
    }

    fn attempt(&self) -> u32 {
        self.failures + 1
    }

    fn exhausted(&self) -> bool {
        self.failures > SESSION_STARTUP_RETRY_LIMIT
    }

    fn persist(&self, layout: &RunLayout) -> Result<(), CommandFailure> {
        let path = layout.state_dir.join(STARTUP_RETRY_STATE_FILE);
        let body = format!(
            "{{\"schema\":1,\"repo\":\"{}\",\"failures\":{}}}\n",
            json_escape(&layout.repo),
            self.failures
        );
        super::atomic_write(&path, &body).map_err(CommandFailure::diagnostic)
    }

    fn reset(&mut self, layout: &RunLayout) -> Result<(), CommandFailure> {
        if self.failures == 0 {
            return Ok(());
        }
        let path = layout.state_dir.join(STARTUP_RETRY_STATE_FILE);
        match fs::remove_file(&path) {
            Ok(()) => {
                self.failures = 0;
                Ok(())
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                self.failures = 0;
                Ok(())
            }
            Err(error) => Err(CommandFailure::diagnostic(format!(
                "cannot clear {}: {error}",
                path.display()
            ))),
        }
    }
}

pub(super) fn run(options: Options) -> Result<(), CommandFailure> {
    let mut layout = RunLayout::new(&options).map_err(CommandFailure::diagnostic)?;
    let scope = RepositoryScope::try_from(layout.repo.as_str()).map_err(|reason| {
        CommandFailure::diagnostic(format!("autonomous drain invalid repo: {reason}"))
    })?;
    let canonical_repo = scope.as_str();
    let checkout_repo = super::git_remote_slug(&options.repo_dir).ok_or_else(|| {
        CommandFailure::diagnostic(
            "autonomous drain requires a supported GitHub origin for repository identity",
        )
    })?;
    if checkout_repo != canonical_repo {
        return Err(CommandFailure::diagnostic(
            "autonomous drain repo does not match checkout origin".to_string(),
        ));
    }
    let state_root = layout
        .state_dir
        .parent()
        .ok_or_else(|| CommandFailure::diagnostic("drain state root has no parent"))?;
    layout.state_dir = state_root.join(repository_progress_key(&canonical_repo));
    layout.scope = repository_progress_key(&canonical_repo);
    let artifact_paths = artifact_paths(&options.repo_dir)?;
    fs::create_dir_all(&layout.state_dir).map_err(|error| {
        CommandFailure::diagnostic(format!(
            "cannot create {}: {error}",
            layout.state_dir.display()
        ))
    })?;

    let startup_timeout =
        Duration::from_secs(SESSION_RECONCILIATION_TIMEOUT_SECS.min(options.drain_stall_secs));
    let mut startup_retry = StartupRetryState::load(&layout)?;
    if startup_retry.exhausted() {
        emit_recovered_retry_exhaustion(&options, startup_retry.failures);
        return Err(CommandFailure::status(String::new(), 124));
    }

    'attempt: loop {
        let mut child = spawn_child(&options)?;
        let output_progress = Arc::new(AtomicBool::new(false));
        let readers =
            match take_output_readers(&mut child, Arc::clone(&output_progress), options.json) {
                Ok(readers) => readers,
                Err(error) => {
                    let _ = terminate_child(&mut child);
                    return Err(error);
                }
            };
        let mut attempt = DrainAttemptGuard { child, readers };
        let mut session_tracker = SessionStartupTracker::default();
        let mut last_activity = Instant::now();
        let mut last_progress = DrainProgress::None;
        let mut warning_emitted = false;
        let mut heartbeat = heartbeat_signature(&layout.repo);
        let mut artifact = artifact_signature(&artifact_paths);
        let mut initial_exit = None;
        let mut github = match github_snapshot(&layout.repo, &mut attempt.child)? {
            GithubSnapshot::Available(snapshot) => Some(snapshot),
            GithubSnapshot::ChildExited(status) => {
                initial_exit = Some(status);
                None
            }
            GithubSnapshot::Unavailable => None,
        };

        let attempt_end = if let Some(status) = initial_exit {
            DrainAttemptEnd::Complete(status)
        } else {
            'supervise: loop {
                let startup_timeout_session = wait_for_session_deadline(
                    &layout,
                    &options,
                    &attempt.readers.session_events,
                    &mut session_tracker,
                    startup_timeout,
                    Duration::from_secs(options.drain_poll_secs),
                )?;
                if session_tracker.matched_reconciliation {
                    startup_retry.reset(&layout)?;
                }
                if let Some(status) = attempt.child.try_wait().map_err(child_status_error)? {
                    break 'supervise DrainAttemptEnd::Complete(status);
                }

                if let Some(session_id) = startup_timeout_session {
                    break 'supervise termination_attempt_end(
                        terminate_child(&mut attempt.child)?,
                        DrainAttemptEnd::StartupTimedOut { session_id },
                    );
                }

                let progress = if output_progress.swap(false, Ordering::AcqRel) {
                    Some(DrainProgress::ChildOutput)
                } else {
                    let next_artifact = artifact_signature(&artifact_paths);
                    if next_artifact != artifact {
                        artifact = next_artifact;
                        Some(DrainProgress::Artifact)
                    } else {
                        let next_heartbeat = heartbeat_signature(&layout.repo);
                        if next_heartbeat != heartbeat {
                            heartbeat = next_heartbeat;
                            Some(DrainProgress::Heartbeat)
                        } else {
                            None
                        }
                    }
                };

                if let Some(progress) = progress {
                    last_activity = Instant::now();
                    last_progress = progress;
                    if is_external(progress) && !warning_emitted {
                        warn_external_progress(&layout, &options, progress)?;
                        warning_emitted = true;
                    }
                    continue;
                }

                let elapsed_secs = last_activity.elapsed().as_secs();
                if elapsed_secs < options.drain_stall_secs {
                    continue;
                }
                match github_snapshot(&layout.repo, &mut attempt.child)? {
                    GithubSnapshot::Available(next_github)
                        if github
                            .as_ref()
                            .is_some_and(|previous| previous != &next_github) =>
                    {
                        github = Some(next_github);
                        last_activity = Instant::now();
                        last_progress = DrainProgress::Github;
                        if !warning_emitted {
                            warn_external_progress(&layout, &options, DrainProgress::Github)?;
                            warning_emitted = true;
                        }
                        continue;
                    }
                    GithubSnapshot::ChildExited(status) => {
                        break 'supervise DrainAttemptEnd::Complete(status)
                    }
                    GithubSnapshot::Available(_) | GithubSnapshot::Unavailable => {}
                }
                if let Some(status) = attempt.child.try_wait().map_err(child_status_error)? {
                    break 'supervise DrainAttemptEnd::Complete(status);
                }

                let observation = DrainObservation::live(
                    elapsed_secs,
                    options.drain_stall_secs,
                    DrainProgress::None,
                );
                debug_assert_eq!(decide(&observation), DrainDecision::TerminateStalled);
                break 'supervise termination_attempt_end(
                    terminate_child(&mut attempt.child)?,
                    DrainAttemptEnd::Stalled { elapsed_secs },
                );
            }
        };

        match attempt_end {
            DrainAttemptEnd::Complete(status) => {
                let _ = terminate_child(&mut attempt.child)?;
                let successful = status.success();
                let result = complete(
                    &layout,
                    &options,
                    status,
                    last_progress,
                    &mut attempt.readers,
                    &mut session_tracker,
                    startup_timeout,
                );
                if successful || session_tracker.matched_reconciliation {
                    startup_retry.reset(&layout)?;
                }
                return result;
            }
            DrainAttemptEnd::StartupTimedOut { session_id } => {
                let retry_attempt = startup_retry.attempt();
                let retry = startup_retry.record_failure();
                startup_retry.persist(&layout)?;
                persist_and_emit_startup_timeout(
                    &layout,
                    &options,
                    &session_id,
                    retry_attempt,
                    &retry,
                )?;
                join_readers(&mut attempt.readers);
                match retry {
                    StartupRetryDecision::Scheduled => continue 'attempt,
                    StartupRetryDecision::Exhausted => {
                        return Err(CommandFailure::status(String::new(), 124));
                    }
                }
            }
            DrainAttemptEnd::Stalled { elapsed_secs } => {
                persist_observation(
                    &layout,
                    DrainDecision::TerminateStalled,
                    last_progress,
                    false,
                )?;
                emit_termination(&layout, &options, elapsed_secs);
                join_readers(&mut attempt.readers);
                return Err(CommandFailure::status(String::new(), 124));
            }
        }
    }
}

fn termination_attempt_end(
    termination: ChildTermination,
    terminated: DrainAttemptEnd,
) -> DrainAttemptEnd {
    match termination {
        ChildTermination::Exited(status) => DrainAttemptEnd::Complete(status),
        ChildTermination::Terminated => terminated,
    }
}

fn spawn_child(options: &Options) -> Result<Child, CommandFailure> {
    let input = DrainExecutorInput::omx_autospec_run(&options.repo_dir)
        .map_err(CommandFailure::diagnostic)?;
    let mut command = Command::new(input.program());
    command
        .args(input.arguments())
        .current_dir(&options.repo_dir);
    #[cfg(unix)]
    command.process_group(0);
    command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            CommandFailure::diagnostic(format!("cannot start autonomous drain: {error}"))
        })
}

fn take_output_readers(
    child: &mut Child,
    progress: Arc<AtomicBool>,
    json: bool,
) -> Result<OutputReaders, CommandFailure> {
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| CommandFailure::diagnostic("autonomous drain stdout was not piped"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| CommandFailure::diagnostic("autonomous drain stderr was not piped"))?;
    let (session_sender, session_events) = mpsc::channel();
    Ok(OutputReaders {
        handles: vec![
            read_child_output(stdout, Arc::clone(&progress), json, session_sender.clone()),
            read_child_output(stderr, progress, true, session_sender),
        ],
        session_events,
    })
}

fn read_child_output<R>(
    reader: R,
    progress: Arc<AtomicBool>,
    is_stderr: bool,
    session_sender: Sender<SessionOutputEvent>,
) -> JoinHandle<()>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut reader = BufReader::new(reader);
        let mut buffer = Vec::new();
        loop {
            buffer.clear();
            match reader.read_until(b'\n', &mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(count) => {
                    progress.store(true, Ordering::Release);
                    record_session_output(&buffer[..count], &session_sender);
                    if is_stderr {
                        let _ = io::stderr().write_all(&buffer[..count]);
                    } else {
                        let _ = io::stdout().write_all(&buffer[..count]);
                    }
                }
            }
        }
    })
}

fn record_session_output(buffer: &[u8], sender: &Sender<SessionOutputEvent>) {
    let line = String::from_utf8_lossy(buffer);
    let observed_at = Instant::now();
    if let Some(start) = super::session_start_event(&line) {
        let _ = sender.send(SessionOutputEvent::Started {
            session_id: start.session_id,
            observed_at,
            requires_reconciliation: start.session_turns_zero && start.issue_claim_missing,
        });
    }
    if let Some(session_id) = super::session_start_reconciled_event(&line) {
        let _ = sender.send(SessionOutputEvent::Reconciled {
            session_id,
            observed_at,
        });
    }
}

fn record_session_events(
    layout: &RunLayout,
    options: &Options,
    events: &Receiver<SessionOutputEvent>,
    tracker: &mut SessionStartupTracker,
    timeout: Duration,
) -> Result<(), CommandFailure> {
    for event in events.try_iter() {
        record_session_event(layout, options, tracker, timeout, event)?;
    }
    Ok(())
}

fn record_session_event(
    layout: &RunLayout,
    options: &Options,
    tracker: &mut SessionStartupTracker,
    timeout: Duration,
    event: SessionOutputEvent,
) -> Result<(), CommandFailure> {
    match event {
        SessionOutputEvent::Started {
            session_id,
            observed_at,
            requires_reconciliation,
        } => {
            tracker.record_start(&session_id, observed_at, requires_reconciliation);
            persist_and_emit_session_state(
                layout,
                options,
                "session_start_observed",
                &session_id,
                "starting",
            )
        }
        SessionOutputEvent::Reconciled {
            session_id,
            observed_at,
        } => {
            let reconciled_in_time =
                tracker.record_reconciliation(&session_id, observed_at, timeout);
            if reconciled_in_time {
                persist_and_emit_session_state(
                    layout,
                    options,
                    "session_start_reconciled",
                    &session_id,
                    "claimed",
                )?;
            }
            Ok(())
        }
    }
}

fn wait_for_session_deadline(
    layout: &RunLayout,
    options: &Options,
    events: &Receiver<SessionOutputEvent>,
    tracker: &mut SessionStartupTracker,
    timeout: Duration,
    max_wait: Duration,
) -> Result<Option<String>, CommandFailure> {
    let poll_deadline = Instant::now() + max_wait;
    loop {
        record_session_events(layout, options, events, tracker, timeout)?;
        if let Some(session_id) = tracker.expired(timeout) {
            return Ok(Some(session_id));
        }
        let now = Instant::now();
        if now >= poll_deadline {
            return Ok(None);
        }
        let wait = tracker.until_deadline(timeout).map_or_else(
            || poll_deadline - now,
            |remaining| remaining.min(poll_deadline - now),
        );
        match events.recv_timeout(wait) {
            Ok(event) => record_session_event(layout, options, tracker, timeout, event)?,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return Ok(None),
        }
    }
}

impl SessionStartupTracker {
    fn record_start(
        &mut self,
        session_id: &str,
        observed_at: Instant,
        requires_reconciliation: bool,
    ) {
        if !requires_reconciliation {
            return;
        }
        let already_reconciled = self.reconciled.remove(session_id).is_some();
        if already_reconciled {
            self.matched_reconciliation = true;
        } else {
            self.pending.insert(session_id.to_string(), observed_at);
        }
    }

    fn record_reconciliation(
        &mut self,
        session_id: &str,
        observed_at: Instant,
        timeout: Duration,
    ) -> bool {
        self.reconciled.insert(session_id.to_string(), observed_at);
        let pending_start = self.pending.get(session_id).copied();
        let reconciled_in_time = pending_start
            .is_none_or(|started_at| observed_at.saturating_duration_since(started_at) <= timeout);
        if reconciled_in_time {
            self.pending.remove(session_id);
            if pending_start.is_some() {
                self.matched_reconciliation = true;
            }
        }
        reconciled_in_time
    }

    fn expired(&self, timeout: Duration) -> Option<String> {
        let now = Instant::now();
        self.pending
            .iter()
            .filter(|(_, started_at)| now.saturating_duration_since(**started_at) >= timeout)
            .min_by_key(|(_, started_at)| **started_at)
            .map(|(session_id, _)| session_id.clone())
    }

    fn until_deadline(&self, timeout: Duration) -> Option<Duration> {
        let now = Instant::now();
        self.pending
            .values()
            .map(|started_at| (*started_at + timeout).saturating_duration_since(now))
            .min()
    }
}

fn persist_and_emit_session_state(
    layout: &RunLayout,
    options: &Options,
    event: &str,
    session_id: &str,
    state: &str,
) -> Result<(), CommandFailure> {
    let body = format!(
        "{{\"schema\":1,\"repo\":\"{}\",\"event\":\"{event}\",\"session_id\":\"{}\",\"state\":\"{state}\"}}",
        json_escape(&layout.repo),
        json_escape(session_id),
    );
    persist_session_event(layout, &body)?;
    if options.json {
        println!("{body}");
    } else {
        eprintln!(
            "event={event} session_id={} state={state}",
            json_escape(session_id)
        );
    }
    Ok(())
}

fn persist_and_emit_startup_timeout(
    layout: &RunLayout,
    options: &Options,
    session_id: &str,
    attempt: u32,
    retry: &StartupRetryDecision,
) -> Result<(), CommandFailure> {
    let retry = match retry {
        StartupRetryDecision::Scheduled => "scheduled",
        StartupRetryDecision::Exhausted => "exhausted",
    };
    let body = format!(
        "{{\"schema\":1,\"repo\":\"{}\",\"event\":\"session_start_timeout\",\"session_id\":\"{}\",\"state\":\"failed-startup\",\"termination\":\"process_group\",\"retry\":\"{retry}\",\"attempt\":{attempt}}}",
        json_escape(&layout.repo),
        json_escape(session_id),
    );
    persist_session_event(layout, &body)?;
    if options.json {
        println!("{body}");
    } else {
        eprintln!(
            "event=session_start_timeout session_id={} state=failed-startup termination=process_group retry={retry} attempt={attempt}",
            json_escape(session_id)
        );
    }
    Ok(())
}

fn emit_recovered_retry_exhaustion(options: &Options, failures: u32) {
    if options.json {
        println!(
            "{{\"command\":\"autonomous\",\"subcommand\":\"drain\",\"decision\":\"startup_retry_exhausted\",\"attempt\":{failures}}}"
        );
    } else {
        eprintln!(
            "autospec autonomous drain: startup retry budget already exhausted at attempt {failures}"
        );
    }
}

fn persist_session_event(layout: &RunLayout, body: &str) -> Result<(), CommandFailure> {
    let path = layout.state_dir.join("drain-session-events.jsonl");
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|error| {
            CommandFailure::diagnostic(format!("cannot open {}: {error}", path.display()))
        })?;
    writeln!(file, "{body}").map_err(|error| {
        CommandFailure::diagnostic(format!("cannot write {}: {error}", path.display()))
    })
}

fn complete(
    layout: &RunLayout,
    options: &Options,
    status: ExitStatus,
    last_progress: DrainProgress,
    readers: &mut OutputReaders,
    session_tracker: &mut SessionStartupTracker,
    startup_timeout: Duration,
) -> Result<(), CommandFailure> {
    join_readers(readers);
    record_session_events(
        layout,
        options,
        &readers.session_events,
        session_tracker,
        startup_timeout,
    )?;
    let exit_code = status.code().unwrap_or(1);
    let decision = DrainDecision::Complete { exit_code };
    persist_observation(layout, decision, last_progress, false)?;
    if options.json {
        println!(
            "{{\"command\":\"autonomous\",\"subcommand\":\"drain\",\"decision\":\"complete\",\"exit_code\":{exit_code},\"last_progress\":\"{}\"}}",
            progress_name(last_progress)
        );
    } else {
        println!(
            "autospec autonomous drain: complete exit_code={exit_code} last_progress={}",
            progress_name(last_progress)
        );
    }
    if status.success() {
        Ok(())
    } else {
        Err(CommandFailure::status(String::new(), exit_code))
    }
}

fn warn_external_progress(
    layout: &RunLayout,
    options: &Options,
    progress: DrainProgress,
) -> Result<(), CommandFailure> {
    let decision = decide(&DrainObservation::live(
        options.drain_stall_secs,
        options.drain_stall_secs,
        progress,
    ));
    debug_assert_eq!(decision, DrainDecision::WarnExternalProgress);
    persist_observation(layout, decision, progress, true)?;
    if options.json {
        println!(
            "{{\"command\":\"autonomous\",\"subcommand\":\"drain\",\"warning\":\"quiet_stdout_external_progress\",\"progress\":\"{}\"}}",
            progress_name(progress)
        );
    } else {
        eprintln!(
            "autospec autonomous drain: warning quiet stdout while {} is advancing",
            progress_name(progress)
        );
    }
    Ok(())
}

fn emit_termination(layout: &RunLayout, options: &Options, elapsed_secs: u64) {
    if options.json {
        println!(
            "{{\"command\":\"autonomous\",\"subcommand\":\"drain\",\"decision\":\"terminate_stalled\",\"repo\":\"{}\",\"elapsed_secs\":{elapsed_secs}}}",
            json_escape(&layout.repo)
        );
    } else {
        eprintln!(
            "autospec autonomous drain: terminating stalled child for {} after {elapsed_secs}s",
            layout.repo
        );
    }
}

fn terminate_child(child: &mut Child) -> Result<ChildTermination, CommandFailure> {
    let pid = child.id();
    let process_group = format!("-{pid}");
    let leader_status = child.try_wait().map_err(child_status_error)?;
    if !process_group_is_alive(&process_group)? {
        return Ok(leader_status
            .or(child.try_wait().map_err(child_status_error)?)
            .map_or(ChildTermination::Terminated, ChildTermination::Exited));
    }
    let status = Command::new("kill")
        .args(["-TERM", "--", &process_group])
        .status()
        .map_err(|error| {
            CommandFailure::diagnostic(format!("cannot terminate drain child: {error}"))
        })?;
    if !status.success() {
        if !process_group_is_alive(&process_group)? {
            if let Some(status) = leader_status.or(child.try_wait().map_err(child_status_error)?) {
                return Ok(ChildTermination::Exited(status));
            }
            return Ok(ChildTermination::Terminated);
        }
        if let Some(status) = child.try_wait().map_err(child_status_error)? {
            return Ok(ChildTermination::Exited(status));
        }
        return Err(CommandFailure::diagnostic(
            "cannot terminate drain child".to_string(),
        ));
    }
    if wait_for_process_group_exit(child, &process_group)? {
        return Ok(leader_status.map_or(ChildTermination::Terminated, ChildTermination::Exited));
    }
    let status = Command::new("kill")
        .args(["-KILL", "--", &process_group])
        .status()
        .map_err(|error| CommandFailure::diagnostic(format!("cannot kill drain child: {error}")))?;
    if !status.success() {
        if !process_group_is_alive(&process_group)? {
            if let Some(status) = leader_status.or(child.try_wait().map_err(child_status_error)?) {
                return Ok(ChildTermination::Exited(status));
            }
            return Ok(ChildTermination::Terminated);
        }
        if let Some(status) = child.try_wait().map_err(child_status_error)? {
            return Ok(ChildTermination::Exited(status));
        }
        return Err(CommandFailure::diagnostic(
            "cannot kill drain child".to_string(),
        ));
    }
    if !wait_for_process_group_exit(child, &process_group)? {
        return Err(CommandFailure::diagnostic(
            "drain child process group did not exit".to_string(),
        ));
    }
    Ok(leader_status.map_or(ChildTermination::Terminated, ChildTermination::Exited))
}

fn wait_for_process_group_exit(
    child: &mut Child,
    process_group: &str,
) -> Result<bool, CommandFailure> {
    for _ in 0..20 {
        child.try_wait().map_err(child_status_error)?;
        if !process_group_is_alive(process_group)? {
            return Ok(true);
        }
        thread::sleep(OBSERVER_POLL_INTERVAL);
    }
    Ok(false)
}

fn process_group_is_alive(process_group: &str) -> Result<bool, CommandFailure> {
    Command::new("kill")
        .args(["-0", "--", process_group])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .map_err(|error| {
            CommandFailure::diagnostic(format!("cannot inspect drain process group: {error}"))
        })
}

fn join_readers(readers: &mut OutputReaders) {
    for reader in readers.handles.drain(..) {
        let _ = reader.join();
    }
}

fn heartbeat_signature(repo: &str) -> String {
    let mut paths = Vec::new();
    for directory in heartbeat_dirs(repo) {
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) == Some("json") {
                if let Some(signature) = typed_heartbeat_signature(&path, repo) {
                    paths.push(signature);
                }
            }
        }
        let sessions = directory.join("sessions");
        if let Ok(entries) = fs::read_dir(sessions) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|value| value.to_str()) == Some("json") {
                    if let Some(signature) = typed_heartbeat_signature(&path, repo) {
                        paths.push(signature);
                    }
                }
            }
        }
    }
    paths.sort();
    paths.join("|")
}

fn typed_heartbeat_signature(path: &Path, repo: &str) -> Option<String> {
    let raw = fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let object = value.as_object()?;
    let fields = ["issue", "branch", "step", "ts", "worker_id", "claim_id"];
    if object.get("repo").and_then(serde_json::Value::as_str) != Some(repo)
        || fields.iter().any(|field| !object.contains_key(*field))
    {
        return None;
    }
    Some(format!("{}:{}", path.display(), raw.trim()))
}

fn heartbeat_dirs(repo: &str) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(root) = super::super::claim::heartbeat_root() {
        roots.push(root);
    }
    if let Some(root) = std::env::var_os("AUTOSPEC_PROCESS_HEARTBEAT_DIR").map(PathBuf::from) {
        if !roots.contains(&root) {
            roots.push(root);
        }
    }
    if roots.is_empty() {
        roots.push(PathBuf::from(".autospec/process-heartbeats"));
    }
    roots
        .into_iter()
        .map(|root| root.join(repository_progress_key(repo)))
        .collect()
}

fn artifact_paths(repo_dir: &str) -> Result<Vec<PathBuf>, CommandFailure> {
    let repo_root = fs::canonicalize(repo_dir).map_err(|error| {
        CommandFailure::diagnostic(format!("cannot resolve drain repository root: {error}"))
    })?;
    let mut paths = vec![repo_root.join(".autospec/run-summary.md")];
    for variable in [
        "AUTOSPEC_AUTONOMOUS_DRAIN_LOG",
        "AUTOSPEC_AUTONOMOUS_DRAIN_LOG_FILE",
    ] {
        if let Some(path) = std::env::var_os(variable) {
            let path = contained_path(&repo_root, &PathBuf::from(path)).ok_or_else(|| {
                CommandFailure::diagnostic(format!(
                    "{variable} must remain inside the canonical repository root"
                ))
            })?;
            paths.push(path);
        }
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

fn artifact_signature(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|path| file_signature(path))
        .collect::<Vec<_>>()
        .join("|")
}

fn contained_path(root: &Path, candidate: &Path) -> Option<PathBuf> {
    let absolute = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        root.join(candidate)
    };
    let normalized = normalize_absolute(&absolute)?;
    let resolved = resolve_existing_prefix(&normalized)?;
    resolved.starts_with(root).then_some(resolved)
}

fn normalize_absolute(path: &Path) -> Option<PathBuf> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(Path::new("/")),
            Component::CurDir => {}
            Component::Normal(part) => normalized.push(part),
            Component::ParentDir if normalized.pop() => {}
            Component::ParentDir => return None,
        }
    }
    normalized.is_absolute().then_some(normalized)
}

fn resolve_existing_prefix(path: &Path) -> Option<PathBuf> {
    let mut ancestor = path;
    let mut suffix = Vec::new();
    while !ancestor.exists() {
        suffix.push(ancestor.file_name()?.to_owned());
        ancestor = ancestor.parent()?;
    }
    let mut resolved = fs::canonicalize(ancestor).ok()?;
    for component in suffix.into_iter().rev() {
        resolved.push(component);
    }
    Some(resolved)
}

pub(crate) fn repository_progress_key(repo: &str) -> String {
    let (owner, repository) = repo
        .split_once('/')
        .expect("validated repository scope has one separator");
    format!(
        "o{}_{}_r{}_{}",
        owner.len(),
        owner,
        repository.len(),
        repository
    )
}

fn file_signature(path: &Path) -> String {
    let Ok(metadata) = fs::metadata(path) else {
        return String::new();
    };
    let modified = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map(|value| value.as_nanos())
        .unwrap_or_default();
    format!("{}:{}:{modified}", path.display(), metadata.len())
}

fn github_snapshot(
    repo: &str,
    watched_child: &mut Child,
) -> Result<GithubSnapshot, CommandFailure> {
    let issues = match gh_output(
        [
            "issue",
            "list",
            "--repo",
            repo,
            "--state",
            "open",
            "--label",
            "in-progress-by-bot",
            "--json",
            "number,updatedAt",
        ],
        watched_child,
    )? {
        GithubOutput::Available(output) => output,
        GithubOutput::ChildExited(status) => return Ok(GithubSnapshot::ChildExited(status)),
        GithubOutput::Unavailable => return Ok(GithubSnapshot::Unavailable),
    };
    let pull_requests = match gh_output(
        [
            "pr",
            "list",
            "--repo",
            repo,
            "--state",
            "all",
            "--json",
            "number,state,updatedAt",
        ],
        watched_child,
    )? {
        GithubOutput::Available(output) => output,
        GithubOutput::ChildExited(status) => return Ok(GithubSnapshot::ChildExited(status)),
        GithubOutput::Unavailable => return Ok(GithubSnapshot::Unavailable),
    };
    Ok(GithubSnapshot::Available(format!(
        "{issues}\n{pull_requests}"
    )))
}

fn gh_output<const N: usize>(
    arguments: [&str; N],
    watched_child: &mut Child,
) -> Result<GithubOutput, CommandFailure> {
    let mut command = Command::new("gh");
    command.args(arguments);
    #[cfg(unix)]
    command.process_group(0);
    let mut observer = match command.stdout(Stdio::piped()).stderr(Stdio::null()).spawn() {
        Ok(child) => child,
        Err(_) => return Ok(GithubOutput::Unavailable),
    };
    let mut stdout = observer
        .stdout
        .take()
        .ok_or_else(|| CommandFailure::diagnostic("cannot capture drain GitHub output"))?;
    let reader = thread::spawn(move || {
        let mut output = Vec::new();
        stdout.read_to_end(&mut output).map(|_| output)
    });
    let started = Instant::now();
    loop {
        if let Some(status) = watched_child.try_wait().map_err(child_status_error)? {
            stop_observer(&mut observer);
            let _ = reader.join();
            return Ok(GithubOutput::ChildExited(status));
        }
        if let Some(status) = observer.try_wait().map_err(|error| {
            CommandFailure::diagnostic(format!("cannot inspect drain progress: {error}"))
        })? {
            let output = reader
                .join()
                .ok()
                .and_then(Result::ok)
                .map(|output| String::from_utf8_lossy(&output).to_string());
            return Ok(match (status.success(), output) {
                (true, Some(output)) => GithubOutput::Available(output),
                _ => GithubOutput::Unavailable,
            });
        }
        if started.elapsed() >= GITHUB_SNAPSHOT_TIMEOUT {
            stop_observer(&mut observer);
            let _ = reader.join();
            return Ok(GithubOutput::Unavailable);
        }
        thread::sleep(OBSERVER_POLL_INTERVAL);
    }
}

fn stop_observer(observer: &mut Child) {
    #[cfg(unix)]
    {
        let process_group = format!("-{}", observer.id());
        let _ = Command::new("kill")
            .args(["-KILL", "--", &process_group])
            .status();
    }
    let _ = observer.kill();
    let _ = observer.wait();
}

fn persist_observation(
    layout: &RunLayout,
    decision: DrainDecision,
    progress: DrainProgress,
    warning: bool,
) -> Result<(), CommandFailure> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| {
            CommandFailure::diagnostic(format!("cannot timestamp drain observation: {error}"))
        })?
        .as_secs();
    let body = format!(
        "{{\"schema\":1,\"repo\":\"{}\",\"timestamp\":{timestamp},\"child_state\":\"{}\",\"child_exit_code\":{},\"decision\":\"{}\",\"progress\":\"{}\",\"warning\":{warning}}}\n",
        json_escape(&layout.repo),
        child_state_name(decision),
        child_exit_code_json(decision),
        decision_name(decision),
        progress_name(progress),
    );
    let path = layout.state_dir.join("drain-observation.json");
    super::atomic_write(&path, &body).map_err(CommandFailure::diagnostic)
}

fn child_status_error(error: std::io::Error) -> CommandFailure {
    CommandFailure::diagnostic(format!("cannot inspect drain child: {error}"))
}

fn is_external(progress: DrainProgress) -> bool {
    matches!(progress, DrainProgress::Heartbeat | DrainProgress::Github)
}

fn progress_name(progress: DrainProgress) -> &'static str {
    match progress {
        DrainProgress::None => "none",
        DrainProgress::ChildOutput => "child_output",
        DrainProgress::Artifact => "artifact",
        DrainProgress::Heartbeat => "heartbeat",
        DrainProgress::Github => "github",
    }
}

fn decision_name(decision: DrainDecision) -> &'static str {
    match decision {
        DrainDecision::Wait => "wait",
        DrainDecision::WarnExternalProgress => "warn_external_progress",
        DrainDecision::Complete { .. } => "complete",
        DrainDecision::TerminateStalled => "terminate_stalled",
    }
}

fn child_state_name(decision: DrainDecision) -> &'static str {
    match decision {
        DrainDecision::Complete { .. } => "completed",
        DrainDecision::Wait | DrainDecision::WarnExternalProgress => "live",
        DrainDecision::TerminateStalled => "terminated",
    }
}

fn child_exit_code_json(decision: DrainDecision) -> String {
    match decision {
        DrainDecision::Complete { exit_code } => exit_code.to_string(),
        DrainDecision::Wait
        | DrainDecision::WarnExternalProgress
        | DrainDecision::TerminateStalled => "null".to_string(),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::fs;
    use std::time::{Duration, Instant};

    use super::{heartbeat_dirs, typed_heartbeat_signature, SessionStartupTracker};

    #[test]
    fn repository_aliases_cannot_share_heartbeat_progress_paths() {
        let first = heartbeat_dirs("owner/repo_name");
        let second = heartbeat_dirs("owner_repo/name");
        assert!(
            first.iter().all(|path| !second.contains(path)),
            "distinct canonical repositories must not share a progress path"
        );
    }

    #[test]
    fn malformed_heartbeat_never_counts_as_progress() {
        let path = std::env::temp_dir().join(format!(
            "autospec-malformed-heartbeat-{}.json",
            std::process::id()
        ));
        fs::write(&path, "not-json").expect("malformed heartbeat");
        assert_eq!(typed_heartbeat_signature(&path, "owner/repo"), None);
        fs::write(
            &path,
            r#"{"repo":"owner/repo","issue":"42","branch":"main","step":"claimed","ts":1,"worker_id":"worker","claim_id":"claim"}"#,
        )
        .expect("valid heartbeat");
        assert!(typed_heartbeat_signature(&path, "owner/repo").is_some());
        let _ = fs::remove_file(path);
    }

    #[test]
    fn reconciliation_from_one_pipe_cancels_a_start_observed_later_from_the_other_pipe() {
        let mut tracker = SessionStartupTracker::default();
        let started_at = Instant::now();
        let reconciled_at = started_at + Duration::from_millis(1);

        tracker.record_reconciliation("child-1850", reconciled_at, Duration::from_secs(1));
        tracker.record_start("child-1850", started_at, true);

        assert!(tracker.pending.is_empty());
    }
}
