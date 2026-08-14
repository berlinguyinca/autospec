use super::*;

#[cfg(unix)]
use std::os::unix::process::CommandExt;

pub(super) fn spawn_input(
    options: &Options,
    input: &DrainExecutorInput,
) -> Result<Child, CommandFailure> {
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

pub(super) fn gh_output<const N: usize>(
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
