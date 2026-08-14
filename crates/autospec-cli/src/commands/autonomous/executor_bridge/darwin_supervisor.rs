use super::{
    argv_digest, codex_host_auth_environment_key, open_private_file, read_output_cursor,
    sensitive_executor_environment_key, write_output_cursor, OutputCursor, OutputSinkPaths,
    ProcessIdentity, ValidatedInvocation, OUTPUT_CURSOR_FILE_BYTES,
};
use crate::commands::autonomous::platform_process::{self, ProcessObservation};
use nix::errno::Errno;
use nix::sys::signal::{killpg, Signal};
use nix::unistd::Pid;
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::os::unix::process::CommandExt;
use std::os::unix::process::ExitStatusExt;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const TERM_GRACE: Duration = Duration::from_millis(500);
const KILL_GRACE: Duration = Duration::from_secs(5);
const OBSERVATION_INTERVAL: Duration = Duration::from_millis(10);

#[derive(Debug)]
pub(super) struct SpawnFailure {
    pub(super) reason: String,
}

impl From<String> for SpawnFailure {
    fn from(reason: String) -> Self {
        Self { reason }
    }
}

pub(super) struct DarwinOwnedGroup {
    leader: ProcessIdentity,
    child: Option<Child>,
    terminal: Option<i32>,
}

impl DarwinOwnedGroup {
    pub(super) fn spawn(
        harness: &ValidatedInvocation,
        sinks: &OutputSinkPaths,
    ) -> Result<Self, SpawnFailure> {
        Self::spawn_with_policy(harness, sinks, false)
    }

    pub(super) fn spawn_trusted(
        harness: &ValidatedInvocation,
        sinks: &OutputSinkPaths,
    ) -> Result<Self, SpawnFailure> {
        Self::spawn_with_policy(harness, sinks, true)
    }

    fn spawn_with_policy(
        harness: &ValidatedInvocation,
        sinks: &OutputSinkPaths,
        trusted_environment: bool,
    ) -> Result<Self, SpawnFailure> {
        let (stdout, stderr) = prepare_durable_sinks(sinks)?;
        let mut command = Command::new(&harness.program);
        if let Some(argv_zero) = harness.argv_zero.as_ref() {
            command.arg0(argv_zero);
        }
        let preserve_codex_host_auth = harness.args.first().is_some_and(|arg| arg == "exec")
            && harness
                .args
                .iter()
                .any(|arg| arg == "--output-last-message");
        let mut environment = std::env::vars_os()
            .filter(|(key, _)| {
                trusted_environment
                    || ((!sensitive_executor_environment_key(key)
                        || (preserve_codex_host_auth && codex_host_auth_environment_key(key)))
                        && key != "COMPOSE_PROJECT_NAME")
            })
            .collect::<BTreeMap<_, _>>();
        for (key, value) in &harness.environment_overrides {
            if !trusted_environment && sensitive_executor_environment_key(key) {
                return Err(format!(
                    "executor harness override may not restore credential authority: {}",
                    key.to_string_lossy()
                )
                .into());
            }
            environment.insert(key.clone(), value.clone());
        }
        if !trusted_environment {
            let credentialless_config = sinks
                .stdout
                .parent()
                .ok_or_else(|| "executor output sink has no parent".to_string())?
                .join("credentialless-config");
            super::ensure_private_directory(&credentialless_config)?;
            environment.insert(
                "GH_CONFIG_DIR".into(),
                credentialless_config.into_os_string(),
            );
            environment.insert("GIT_CONFIG_NOSYSTEM".into(), "1".into());
            environment.insert("GIT_CONFIG_GLOBAL".into(), "/dev/null".into());
            environment.insert("GIT_TERMINAL_PROMPT".into(), "0".into());
            environment.insert(
                "GIT_SSH_COMMAND".into(),
                "/usr/bin/ssh -F /dev/null -o IdentityAgent=none -o IdentitiesOnly=yes -o IdentityFile=/dev/null -o BatchMode=yes".into(),
            );
        }
        command
            .args(&harness.args)
            .current_dir(&harness.current_dir)
            .env_clear()
            .envs(environment)
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr));
        // SAFETY: setpgid is async-signal-safe and the closure performs no allocation or I/O.
        unsafe {
            command.pre_exec(|| {
                if nix::libc::setpgid(0, 0) == 0 {
                    Ok(())
                } else {
                    Err(std::io::Error::last_os_error())
                }
            });
        }
        let child = command
            .spawn()
            .map_err(|error| format!("spawn Darwin executor process group: {error}"))?;
        let pid = child.id();
        let before = platform_process::observe_birth(pid)
            .map_err(|error| format!("observe spawned Darwin executor leader: {error}"))?
            .ok_or_else(|| {
                "spawned Darwin executor leader exited before ownership capture".to_string()
            })?;
        let observed_group = platform_process::observe_process_group(pid)
            .map_err(|error| format!("verify spawned Darwin executor process group: {error}"))?
            .ok_or_else(|| {
                "spawned Darwin executor leader exited before group verification".to_string()
            })?;
        let after = platform_process::observe_birth(pid)
            .map_err(|error| format!("re-observe spawned Darwin executor leader: {error}"))?
            .ok_or_else(|| {
                "spawned Darwin executor leader exited before ownership persistence".to_string()
            })?;
        if before != after || observed_group != pid || after.process_group != pid {
            return Err("spawned Darwin executor process-group identity is unstable"
                .to_string()
                .into());
        }
        Ok(Self {
            leader: ProcessIdentity {
                pid,
                process_group: observed_group,
                executable: harness.program.clone(),
                argv_digest: argv_digest(&harness.args),
                boot_id: after.boot_id,
                start_identity: after.start_identity,
            },
            child: Some(child),
            terminal: None,
        })
    }

    pub(super) fn adopt(expected: &ProcessIdentity) -> Result<Self, String> {
        prove_exact_group(expected)?;
        Ok(Self {
            leader: expected.clone(),
            child: None,
            terminal: None,
        })
    }

    pub(super) fn poll(&mut self) -> Result<Option<i32>, String> {
        if let Some(code) = self.terminal {
            return if group_is_empty(self.leader.process_group)? {
                Ok(Some(code))
            } else {
                Ok(None)
            };
        }
        if let Some(child) = self.child.as_mut() {
            if let Some(status) = child
                .try_wait()
                .map_err(|error| format!("poll Darwin executor leader: {error}"))?
            {
                self.terminal = Some(
                    status
                        .code()
                        .unwrap_or_else(|| 128 + status.signal().unwrap_or(0)),
                );
                self.child = None;
                return self.poll();
            }
        }
        match platform_process::observe_expected(
            self.leader.pid,
            &self.leader.boot_id,
            &self.leader.start_identity,
        ) {
            ProcessObservation::Exact(birth)
                if birth.process_group == self.leader.process_group =>
            {
                Ok(None)
            }
            ProcessObservation::Dead if group_is_empty(self.leader.process_group)? => Ok(None),
            ProcessObservation::Dead => Err(
                "Darwin executor leader exited while process-group membership remains uncertain"
                    .to_string(),
            ),
            ProcessObservation::Exact(_)
            | ProcessObservation::Mismatch
            | ProcessObservation::Unknown(_) => {
                Err("executor process group ownership is unverified".to_string())
            }
        }
    }

    pub(super) fn terminate(mut self) -> Result<(), String> {
        signal_exact_group(&self.leader, Signal::SIGTERM)?;
        if wait_for_empty_group(self.leader.process_group, self.child.as_mut(), TERM_GRACE)? {
            return Ok(());
        }
        signal_exact_group(&self.leader, Signal::SIGKILL)?;
        if !wait_for_empty_group(self.leader.process_group, self.child.as_mut(), KILL_GRACE)? {
            return Err("Darwin executor process group survived SIGKILL".to_string());
        }
        Ok(())
    }

    pub(super) fn identity(&self) -> &ProcessIdentity {
        &self.leader
    }
}

pub(super) fn publish_output_cursors(paths: &OutputSinkPaths) -> Result<u64, String> {
    let mut published = 0_u64;
    for (sink, cursor_path, name) in [
        (&paths.stdout, &paths.stdout_writer_cursor, "stdout"),
        (&paths.stderr, &paths.stderr_writer_cursor, "stderr"),
    ] {
        let total = std::fs::metadata(sink)
            .map_err(|error| format!("inspect Darwin executor {name} sink: {error}"))?
            .len();
        let cursor = OpenOptions::new()
            .read(true)
            .write(true)
            .open(cursor_path)
            .map_err(|error| format!("open Darwin executor {name} cursor: {error}"))?;
        let mut current = read_output_cursor(&cursor)?;
        if total < current.total {
            return Err(format!("Darwin executor {name} sink regressed"));
        }
        published = published.saturating_add(total - current.total);
        current.total = total;
        write_output_cursor(&cursor, current)?;
    }
    Ok(published)
}

fn prepare_durable_sinks(paths: &OutputSinkPaths) -> Result<(File, File), SpawnFailure> {
    let stdout = open_private_file(&paths.stdout, true)?;
    let stderr = open_private_file(&paths.stderr, true)?;
    for path in [
        &paths.stdout_writer_cursor,
        &paths.stderr_writer_cursor,
        &paths.stdout_reader_cursor,
        &paths.stderr_reader_cursor,
    ] {
        let cursor = open_private_file(path, true)?;
        cursor
            .set_len(OUTPUT_CURSOR_FILE_BYTES)
            .map_err(|error| format!("size Darwin executor output cursor: {error}"))?;
        write_output_cursor(&cursor, OutputCursor::default())?;
    }
    let exit = open_private_file(&paths.exit_status, true)?;
    exit.set_len(16)
        .map_err(|error| format!("size Darwin executor exit record: {error}"))?;
    Ok((stdout, stderr))
}

fn prove_exact_group(expected: &ProcessIdentity) -> Result<(), String> {
    match platform_process::observe_expected(
        expected.pid,
        &expected.boot_id,
        &expected.start_identity,
    ) {
        ProcessObservation::Exact(birth) if birth.process_group == expected.process_group => Ok(()),
        ProcessObservation::Dead => Err("executor process group leader is dead".to_string()),
        ProcessObservation::Exact(_)
        | ProcessObservation::Mismatch
        | ProcessObservation::Unknown(_) => {
            Err("executor process group ownership is unverified".to_string())
        }
    }
}

fn signal_exact_group(expected: &ProcessIdentity, signal: Signal) -> Result<(), String> {
    prove_exact_group(expected)?;
    let group = Pid::from_raw(
        i32::try_from(expected.process_group)
            .map_err(|_| "executor process group is out of range".to_string())?,
    );
    killpg(group, signal).map_err(|error| format!("signal Darwin executor process group: {error}"))
}

pub(super) fn group_is_empty(process_group: u32) -> Result<bool, String> {
    let group = Pid::from_raw(
        i32::try_from(process_group)
            .map_err(|_| "executor process group is out of range".to_string())?,
    );
    match killpg(group, None) {
        Ok(()) => Ok(false),
        Err(Errno::ESRCH) => Ok(true),
        Err(Errno::EPERM) => {
            Err("Darwin executor process-group membership is permission-denied".to_string())
        }
        Err(error) => Err(format!(
            "observe Darwin executor process-group membership: {error}"
        )),
    }
}

fn wait_for_empty_group(
    process_group: u32,
    mut child: Option<&mut Child>,
    grace: Duration,
) -> Result<bool, String> {
    let deadline = Instant::now() + grace;
    loop {
        if let Some(child) = child.as_deref_mut() {
            child
                .try_wait()
                .map_err(|error| format!("reap Darwin executor leader: {error}"))?;
        }
        match group_is_empty(process_group) {
            Ok(true) => return Ok(true),
            Ok(false) => {}
            Err(error) if Instant::now() >= deadline => return Err(error),
            Err(_) => {}
        }
        if Instant::now() >= deadline {
            return Ok(false);
        }
        thread::sleep(OBSERVATION_INTERVAL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};

    fn fixture(script: &str) -> (DarwinOwnedGroup, PathBuf) {
        let root = std::env::current_dir()
            .expect("current Darwin fixture directory")
            .join("target/executor-bridge-tests")
            .join(format!(
                "autospec-darwin-owned-{}-{}",
                std::process::id(),
                super::super::unix_now().expect("clock")
            ));
        std::fs::create_dir_all(&root).expect("create Darwin group fixture");
        let sinks = OutputSinkPaths {
            stdout: root.join("stdout"),
            stderr: root.join("stderr"),
            stdout_writer_cursor: root.join("stdout.writer"),
            stderr_writer_cursor: root.join("stderr.writer"),
            stdout_reader_cursor: root.join("stdout.reader"),
            stderr_reader_cursor: root.join("stderr.reader"),
            exit_status: root.join("exit"),
            supervisor_identity: root.join("supervisor.json"),
        };
        let invocation = ValidatedInvocation {
            program: Path::new("/bin/sh").to_path_buf(),
            argv_zero: None::<OsString>,
            args: vec!["-c".into(), script.into()],
            current_dir: root.clone(),
            environment_overrides: Vec::new(),
        };
        (
            DarwinOwnedGroup::spawn(&invocation, &sinks).expect("spawn Darwin group fixture"),
            root,
        )
    }

    #[test]
    fn darwin_adoption_cleanup_requires_exact_boot_start_and_process_group() {
        let (group, root) = fixture("trap '' TERM; while :; do sleep 1; done");
        DarwinOwnedGroup::adopt(group.identity()).expect("adopt exact group");
        for field in ["boot", "start", "group"] {
            let mut mismatched = group.identity().clone();
            match field {
                "boot" => mismatched.boot_id.push_str("-wrong"),
                "start" => mismatched.start_identity.push_str("-wrong"),
                "group" => mismatched.process_group = mismatched.process_group.saturating_add(1),
                _ => unreachable!(),
            }
            assert!(
                DarwinOwnedGroup::adopt(&mismatched).is_err(),
                "{field} mismatch was accepted"
            );
        }
        group.terminate().expect("clean exact group");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn darwin_sidecar_launch_identity_mismatch_refuses_signal() {
        let (group, root) = fixture("trap '' TERM; while :; do sleep 1; done");
        let mut mismatched = group.identity().clone();
        mismatched.start_identity.push_str("-wrong");
        let forged = DarwinOwnedGroup {
            leader: mismatched,
            child: None,
            terminal: None,
        };
        assert_eq!(
            forged.terminate().unwrap_err(),
            "executor process group ownership is unverified"
        );
        assert!(matches!(
            platform_process::observe_expected(
                group.identity().pid,
                &group.identity().boot_id,
                &group.identity().start_identity
            ),
            ProcessObservation::Exact(_)
        ));
        group.terminate().expect("clean untouched exact group");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn darwin_restart_direct_completes_only_after_leader_and_group_exit() {
        let (mut group, root) = fixture("sleep 0.05 & exit 0");
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match group.poll().expect("poll Darwin group") {
                Some(0) => break,
                Some(code) => panic!("unexpected Darwin exit code {code}"),
                None if Instant::now() < deadline => thread::sleep(OBSERVATION_INTERVAL),
                None => panic!("Darwin group did not reach empty completion"),
            }
        }
        let _ = std::fs::remove_dir_all(root);
    }
}
