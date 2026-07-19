use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::process::CommandExt;
use std::path::{Component, Path, PathBuf};
use std::process::{Child, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
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

    let mut child = spawn_child(&options)?;
    let output_progress = Arc::new(AtomicBool::new(false));
    let readers = take_output_readers(&mut child, Arc::clone(&output_progress), options.json)?;
    let mut last_activity = Instant::now();
    let mut last_progress = DrainProgress::None;
    let mut warning_emitted = false;
    let mut heartbeat = heartbeat_signature(&layout.repo);
    let mut artifact = artifact_signature(&artifact_paths);
    let mut github = match github_snapshot(&layout.repo, &mut child)? {
        GithubSnapshot::Available(snapshot) => Some(snapshot),
        GithubSnapshot::ChildExited(status) => {
            return complete(&layout, &options, status, last_progress, readers)
        }
        GithubSnapshot::Unavailable => None,
    };

    loop {
        if let Some(status) = child.try_wait().map_err(child_status_error)? {
            return complete(&layout, &options, status, last_progress, readers);
        }
        thread::sleep(Duration::from_secs(options.drain_poll_secs));
        if let Some(status) = child.try_wait().map_err(child_status_error)? {
            return complete(&layout, &options, status, last_progress, readers);
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
        match github_snapshot(&layout.repo, &mut child)? {
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
                return complete(&layout, &options, status, last_progress, readers)
            }
            GithubSnapshot::Available(_) | GithubSnapshot::Unavailable => {}
        }
        if let Some(status) = child.try_wait().map_err(child_status_error)? {
            return complete(&layout, &options, status, last_progress, readers);
        }

        let observation =
            DrainObservation::live(elapsed_secs, options.drain_stall_secs, DrainProgress::None);
        debug_assert_eq!(decide(&observation), DrainDecision::TerminateStalled);
        match terminate_child(&mut child)? {
            ChildTermination::Exited(status) => {
                return complete(&layout, &options, status, last_progress, readers)
            }
            ChildTermination::Terminated => {
                persist_observation(
                    &layout,
                    DrainDecision::TerminateStalled,
                    last_progress,
                    false,
                )?;
                emit_termination(&layout, &options, elapsed_secs);
                join_readers(readers);
                return Err(CommandFailure::status(String::new(), 124));
            }
        }
    }
}

fn spawn_child(options: &Options) -> Result<Child, CommandFailure> {
    let input = DrainExecutorInput::omx_autospec_run(&options.repo_dir)
        .map_err(CommandFailure::diagnostic)?;
    Command::new(input.program())
        .args(input.arguments())
        .current_dir(&options.repo_dir)
        .process_group(0)
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
) -> Result<Vec<JoinHandle<()>>, CommandFailure> {
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| CommandFailure::diagnostic("autonomous drain stdout was not piped"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| CommandFailure::diagnostic("autonomous drain stderr was not piped"))?;
    Ok(vec![
        read_child_output(stdout, Arc::clone(&progress), json),
        read_child_output(stderr, progress, true),
    ])
}

fn read_child_output<R>(mut reader: R, progress: Arc<AtomicBool>, is_stderr: bool) -> JoinHandle<()>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut buffer = [0; 4096];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(count) => {
                    progress.store(true, Ordering::Release);
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

fn complete(
    layout: &RunLayout,
    options: &Options,
    status: ExitStatus,
    last_progress: DrainProgress,
    readers: Vec<JoinHandle<()>>,
) -> Result<(), CommandFailure> {
    join_readers(readers);
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
    if let Some(status) = child.try_wait().map_err(child_status_error)? {
        return Ok(ChildTermination::Exited(status));
    }
    let pid = child.id();
    let process_group = format!("-{pid}");
    let status = Command::new("kill")
        .args(["-TERM", "--", &process_group])
        .status()
        .map_err(|error| {
            CommandFailure::diagnostic(format!("cannot terminate drain child: {error}"))
        })?;
    if !status.success() {
        if let Some(status) = child.try_wait().map_err(child_status_error)? {
            return Ok(ChildTermination::Exited(status));
        }
        return Err(CommandFailure::diagnostic(
            "cannot terminate drain child".to_string(),
        ));
    }
    if wait_for_process_group_exit(child, &process_group)? {
        return Ok(ChildTermination::Terminated);
    }
    let status = Command::new("kill")
        .args(["-KILL", "--", &process_group])
        .status()
        .map_err(|error| CommandFailure::diagnostic(format!("cannot kill drain child: {error}")))?;
    if !status.success() {
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
    Ok(ChildTermination::Terminated)
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

fn join_readers(readers: Vec<JoinHandle<()>>) {
    for reader in readers {
        let _ = reader.join();
    }
}

fn heartbeat_signature(repo: &str) -> String {
    let mut paths = Vec::new();
    for directory in heartbeat_dirs(repo) {
        let Ok(entries) = fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) == Some("json") {
                paths.push(file_signature(&path));
            }
        }
    }
    paths.sort();
    paths.join("|")
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
    let mut observer = match Command::new("gh")
        .args(arguments)
        .process_group(0)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
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
    let process_group = format!("-{}", observer.id());
    let _ = Command::new("kill")
        .args(["-KILL", "--", &process_group])
        .status();
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

#[cfg(test)]
mod tests {
    use super::heartbeat_dirs;

    #[test]
    fn repository_aliases_cannot_share_heartbeat_progress_paths() {
        let first = heartbeat_dirs("owner/repo_name");
        let second = heartbeat_dirs("owner_repo/name");
        assert!(
            first.iter().all(|path| !second.contains(path)),
            "distinct canonical repositories must not share a progress path"
        );
    }
}
