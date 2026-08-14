use std::ffi::OsString;
use std::fs::File;
use std::path::PathBuf;
use std::process::ExitStatus;
#[cfg(unix)]
use std::process::{Command, Stdio};

#[cfg(unix)]
mod unix_group;
#[cfg(windows)]
mod windows_job;

pub(super) struct OwnedChildTree {
    inner: PlatformOwnedChild,
    identity: DurableProcessOwner,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct DurableProcessOwner {
    pub pid: u32,
    pub container_id: String,
    pub process_start: String,
    pub launch_nonce: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RecoveryDisposition {
    Completed,
    SidecarOwns,
    Quarantine,
}

enum PlatformOwnedChild {
    #[cfg(unix)]
    Unix(unix_group::UnixOwnedChild),
    #[cfg(windows)]
    Windows(windows_job::WindowsJobChild),
}

pub(super) struct PreparedEnvironment {
    pub variables: Vec<(OsString, OsString)>,
}

pub(super) struct PreparedLaunchSpec {
    pub program: PathBuf,
    pub argv: Vec<OsString>,
    pub current_dir: Option<PathBuf>,
    pub environment: PreparedEnvironment,
    pub stdin: Option<File>,
    pub stdout: Option<File>,
    pub stderr: Option<File>,
}

impl PreparedLaunchSpec {
    pub(super) fn inherited(
        program: PathBuf,
        argv: Vec<OsString>,
        current_dir: Option<PathBuf>,
        overrides: Vec<(OsString, OsString)>,
        stdin: Option<File>,
        stdout: Option<File>,
        stderr: Option<File>,
    ) -> Self {
        let mut variables: Vec<_> = std::env::vars_os().collect();
        for (key, value) in overrides {
            #[cfg(windows)]
            let matches = |candidate: &OsString| {
                candidate
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&key.to_string_lossy())
            };
            #[cfg(not(windows))]
            let matches = |candidate: &OsString| candidate == &key;
            variables.retain(|(candidate, _)| !matches(candidate));
            variables.push((key, value));
        }
        Self {
            program,
            argv,
            current_dir,
            environment: PreparedEnvironment { variables },
            stdin,
            stdout,
            stderr,
        }
    }
}

impl OwnedChildTree {
    #[cfg(unix)]
    pub(super) fn spawn(command: &mut Command, launch_nonce: String) -> Result<Self, String> {
        let inner = unix_group::UnixOwnedChild::spawn(command)?;
        Self::from_unix(inner, launch_nonce)
    }

    pub(super) fn spawn_prepared(
        spec: PreparedLaunchSpec,
        launch_nonce: String,
    ) -> Result<Self, String> {
        if spec.argv.is_empty() {
            return Err("prepared autonomous launch argv is empty".to_string());
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            let mut command = Command::new(&spec.program);
            command
                .arg0(&spec.argv[0])
                .args(&spec.argv[1..])
                .env_clear();
            command.envs(spec.environment.variables);
            if let Some(current_dir) = spec.current_dir {
                command.current_dir(current_dir);
            }
            if let Some(stdin) = spec.stdin {
                command.stdin(Stdio::from(stdin));
            }
            if let Some(stdout) = spec.stdout {
                command.stdout(Stdio::from(stdout));
            }
            if let Some(stderr) = spec.stderr {
                command.stderr(Stdio::from(stderr));
            }
            let inner = unix_group::UnixOwnedChild::spawn(&mut command)?;
            Self::from_unix(inner, launch_nonce)
        }
        #[cfg(windows)]
        {
            let inner = windows_job::WindowsJobChild::spawn(spec)?;
            Self::from_windows(inner, launch_nonce)
        }
    }

    #[cfg(unix)]
    fn from_unix(
        mut inner: unix_group::UnixOwnedChild,
        launch_nonce: String,
    ) -> Result<Self, String> {
        let pid = inner.id();
        let (boot_id, process_start) = match super::process_birth_identity(pid) {
            Ok(Some(identity)) => identity,
            Ok(None) => {
                let reason =
                    "spawned process exited before its creation identity was captured".to_string();
                return match inner.terminate() {
                    Ok(_) => Err(reason),
                    Err(cleanup) => Err(format!("{reason}; cleanup failed: {cleanup}")),
                };
            }
            Err(reason) => {
                return match inner.terminate() {
                    Ok(_) => Err(reason),
                    Err(cleanup) => Err(format!("{reason}; cleanup failed: {cleanup}")),
                };
            }
        };
        Ok(Self {
            inner: PlatformOwnedChild::Unix(inner),
            identity: DurableProcessOwner {
                pid,
                container_id: pid.to_string(),
                process_start: format!("{boot_id}:{process_start}"),
                launch_nonce,
            },
        })
    }

    #[cfg(windows)]
    fn from_windows(
        inner: windows_job::WindowsJobChild,
        launch_nonce: String,
    ) -> Result<Self, String> {
        let pid = inner.id();
        let process_start = inner.creation_filetime().to_string();
        Ok(Self {
            inner: PlatformOwnedChild::Windows(inner),
            identity: DurableProcessOwner {
                pid,
                container_id: format!("windows-job:{pid}"),
                process_start: format!("{}:{process_start}", super::current_boot_identity()?),
                launch_nonce,
            },
        })
    }

    pub(super) fn identity(&self) -> DurableProcessOwner {
        self.identity.clone()
    }

    pub(super) fn try_wait(&mut self) -> Result<Option<ExitStatus>, String> {
        match &mut self.inner {
            #[cfg(unix)]
            PlatformOwnedChild::Unix(child) => child.try_wait(),
            #[cfg(windows)]
            PlatformOwnedChild::Windows(child) => child.try_wait(),
        }
    }

    pub(super) fn wait(&mut self) -> Result<ExitStatus, String> {
        match &mut self.inner {
            #[cfg(unix)]
            PlatformOwnedChild::Unix(child) => child.wait(),
            #[cfg(windows)]
            PlatformOwnedChild::Windows(child) => child.wait(),
        }
    }

    pub(super) fn terminate(&mut self) -> Result<ExitStatus, String> {
        match &mut self.inner {
            #[cfg(unix)]
            PlatformOwnedChild::Unix(child) => child.terminate(),
            #[cfg(windows)]
            PlatformOwnedChild::Windows(child) => child.terminate(),
        }
    }
}

pub(super) fn recover_owner(_identity: &DurableProcessOwner) -> RecoveryDisposition {
    RecoveryDisposition::Quarantine
}

impl DurableProcessOwner {
    pub(super) fn document(&self, attempt_id: &str, intent_digest: &str) -> String {
        serde_json::json!({
            "schema": 1,
            "attempt_id": attempt_id,
            "intent_digest": intent_digest,
            "owner": {
                "pid": self.pid,
                "container_id": self.container_id,
                "process_start": self.process_start,
                "launch_nonce": self.launch_nonce,
            },
        })
        .to_string()
    }

    pub(super) fn from_document(
        body: &str,
        expected_attempt_id: &str,
        expected_intent_digest: &str,
    ) -> Result<Self, String> {
        let value: serde_json::Value = serde_json::from_str(body)
            .map_err(|error| format!("parse durable process owner: {error}"))?;
        if value.get("schema").and_then(serde_json::Value::as_u64) != Some(1)
            || value.get("attempt_id").and_then(serde_json::Value::as_str)
                != Some(expected_attempt_id)
            || value
                .get("intent_digest")
                .and_then(serde_json::Value::as_str)
                != Some(expected_intent_digest)
        {
            return Err("durable process owner differs from its invocation intent".to_string());
        }
        let owner = value
            .get("owner")
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| "durable process owner is missing".to_string())?;
        let pid = owner
            .get("pid")
            .and_then(serde_json::Value::as_u64)
            .and_then(|pid| u32::try_from(pid).ok())
            .filter(|pid| *pid > 0)
            .ok_or_else(|| "durable process owner PID is invalid".to_string())?;
        let container_id = owner
            .get("container_id")
            .and_then(serde_json::Value::as_str)
            .filter(|id| *id == pid.to_string() || *id == format!("windows-job:{pid}"))
            .ok_or_else(|| "durable process owner container ID is invalid".to_string())?
            .to_string();
        let process_start = owner
            .get("process_start")
            .and_then(serde_json::Value::as_str)
            .filter(|identity| !identity.is_empty())
            .ok_or_else(|| "durable process owner start identity is invalid".to_string())?
            .to_string();
        if !process_start
            .split_once(':')
            .is_some_and(|(boot, start)| !boot.is_empty() && !start.is_empty())
        {
            return Err("durable process owner creation identity is malformed".to_string());
        }
        let launch_nonce = owner
            .get("launch_nonce")
            .and_then(serde_json::Value::as_str)
            .filter(|nonce| !nonce.is_empty())
            .ok_or_else(|| "durable process owner launch nonce is invalid".to_string())?
            .to_string();
        if launch_nonce != expected_attempt_id {
            return Err("durable process owner launch nonce differs from its attempt".to_string());
        }
        let identity = Self {
            pid,
            container_id,
            process_start,
            launch_nonce,
        };
        if identity.document(expected_attempt_id, expected_intent_digest) != body {
            return Err("durable process owner is non-canonical or has extra fields".to_string());
        }
        Ok(identity)
    }

    #[cfg(test)]
    fn fixture_for_current_process() -> Self {
        let (boot_id, process_start) = super::process_birth_identity(std::process::id())
            .expect("observe current process identity")
            .expect("current process is live");
        Self {
            pid: std::process::id(),
            container_id: std::process::id().to_string(),
            process_start: format!("{boot_id}:{process_start}"),
            launch_nonce: "fixture-nonce".to_string(),
        }
    }
}

#[cfg(test)]
mod tests;
