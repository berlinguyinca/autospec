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
mod tests {
    use super::*;
    use std::process::Command;
    use std::thread;
    use std::time::Duration;

    #[cfg(windows)]
    use std::path::{Path, PathBuf};

    #[cfg(windows)]
    #[link(name = "Kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut std::ffi::c_void;
        fn GetProcessHandleCount(process: *mut std::ffi::c_void, count: *mut u32) -> i32;
    }

    #[cfg(windows)]
    fn current_handle_count() -> u32 {
        let mut count = 0;
        // SAFETY: GetCurrentProcess returns the current-process pseudo handle and count is writable.
        assert_ne!(
            unsafe { GetProcessHandleCount(GetCurrentProcess(), &mut count) },
            0
        );
        count
    }

    #[cfg(windows)]
    fn helper_command(mode: &str, marker: &Path) -> Command {
        let mut command = Command::new(std::env::current_exe().expect("current test binary"));
        command
            .args([
                "--exact",
                "commands::autonomous::executor_bridge::process_owner::tests::windows_child_helper",
                "--nocapture",
            ])
            .env("AUTOSPEC_WINDOWS_CHILD_MODE", mode)
            .env("AUTOSPEC_WINDOWS_CHILD_MARKER", marker);
        command
    }

    #[cfg(windows)]
    fn helper_spec(mode: &str, marker: &Path) -> PreparedLaunchSpec {
        let executable = std::env::current_exe().expect("current test binary");
        PreparedLaunchSpec::inherited(
            executable.clone(),
            vec![
                executable.into_os_string(),
                "--exact".into(),
                "commands::autonomous::executor_bridge::process_owner::tests::windows_child_helper"
                    .into(),
                "--nocapture".into(),
            ],
            None,
            vec![
                ("AUTOSPEC_WINDOWS_CHILD_MODE".into(), mode.into()),
                (
                    "AUTOSPEC_WINDOWS_CHILD_MARKER".into(),
                    marker.as_os_str().to_os_string(),
                ),
            ],
            None,
            None,
            None,
        )
    }

    #[cfg(windows)]
    struct TempPath(PathBuf);

    #[cfg(windows)]
    impl TempPath {
        fn new(name: &str) -> Self {
            Self(std::env::temp_dir().join(format!(
                "autospec-{name}-{}-{}",
                std::process::id(),
                super::super::DIRECT_TRANSACTION_SEQUENCE
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            )))
        }

        fn path(&self) -> &Path {
            &self.0
        }

        fn descendant_survived(&self) -> bool {
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            while std::time::Instant::now() < deadline {
                if self.0.exists() {
                    return true;
                }
                thread::sleep(Duration::from_millis(20));
            }
            false
        }
    }

    #[cfg(windows)]
    impl Drop for TempPath {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[cfg(windows)]
    struct TempDirectory(PathBuf);

    #[cfg(windows)]
    impl TempDirectory {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "autospec-{name}-{}-{}",
                std::process::id(),
                super::super::DIRECT_TRANSACTION_SEQUENCE
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).expect("create Windows launch fixture directory");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    #[cfg(windows)]
    impl Drop for TempDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_child_helper() {
        let Ok(mode) = std::env::var("AUTOSPEC_WINDOWS_CHILD_MODE") else {
            return;
        };
        let marker = PathBuf::from(
            std::env::var_os("AUTOSPEC_WINDOWS_CHILD_MARKER").expect("child marker path"),
        );
        match mode.as_str() {
            "exit-zero" => {}
            "exit-nonzero" => std::process::exit(17),
            "mark-after-delay" => {
                thread::sleep(Duration::from_millis(750));
                std::fs::write(marker, b"descendant escaped").expect("write descendant marker");
            }
            "spawn-descendant-and-wait" => {
                use std::os::windows::process::CommandExt;
                const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
                let mut descendant = helper_command("mark-after-delay", &marker);
                descendant.creation_flags(CREATE_BREAKAWAY_FROM_JOB);
                let Ok(mut descendant) = descendant.spawn() else {
                    std::fs::write(marker, b"blocked").expect("write breakaway handshake");
                    return;
                };
                let _ = descendant.wait();
            }
            "spawn-normal-descendant-and-exit" => {
                helper_command("mark-after-delay", &marker)
                    .spawn()
                    .expect("spawn normal descendant");
            }
            "environment-is-cleared" => {
                assert!(std::env::var_os("AUTOSPEC_FORBIDDEN_PARENT_ENV").is_none());
                assert_eq!(
                    std::env::var("AUTOSPEC_EXPECTED_ENV").as_deref(),
                    Ok("present")
                );
                std::fs::write(marker, b"environment-ok").expect("write environment marker");
            }
            "print-stdio" => {
                println!("prepared-stdout");
                eprintln!("prepared-stderr");
            }
            unknown => panic!("unknown Windows child helper mode: {unknown}"),
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_empty_environment_child() {
        if !Path::new(".autospec-empty-environment-child").is_file() {
            return;
        }
        let environment = std::env::vars_os().collect::<Vec<_>>();
        assert!(
            environment.is_empty(),
            "prepared empty environment inherited variables: {environment:?}"
        );
        let observation = serde_json::json!({
            "cwd": std::fs::canonicalize(std::env::current_dir().expect("read child cwd"))
                .expect("canonicalize child cwd"),
            "argv": std::env::args_os()
                .map(|argument| argument.to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
        });
        std::fs::write(
            "empty-environment-observation.json",
            observation.to_string(),
        )
        .expect("write empty-environment child observation");
    }

    #[cfg(unix)]
    fn shell_command(script: &str) -> Command {
        let mut command = Command::new("/bin/sh");
        command.arg("-c").arg(script);
        command
    }

    #[cfg(unix)]
    #[test]
    fn wait_preserves_zero_exit() {
        let mut command = shell_command("sleep 0.05; exit 0");
        let mut owned = OwnedChildTree::spawn(&mut command, "nonce-zero".into()).unwrap();
        assert!(owned.wait().expect("wait for owned child").success());
    }

    #[cfg(unix)]
    #[test]
    fn try_wait_preserves_nonzero_exit() {
        let mut command = shell_command("sleep 0.05; exit 17");
        let mut owned = OwnedChildTree::spawn(&mut command, "nonce-nonzero".into()).unwrap();
        let status = loop {
            if let Some(status) = owned.try_wait().expect("poll owned child") {
                break status;
            }
            thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(status.code(), Some(17));
    }

    #[cfg(unix)]
    #[test]
    fn terminate_reaps_the_owned_process_group() {
        let mut command = shell_command("sleep 30 & wait");
        let mut owned = OwnedChildTree::spawn(&mut command, "nonce-a".into()).unwrap();
        let identity = owned.identity();
        let status = owned.terminate().expect("terminate owned group");
        assert!(!status.success());
        assert_eq!(identity.pid.to_string(), identity.container_id);
    }

    #[cfg(unix)]
    #[test]
    fn terminate_signals_descendants_in_the_owned_process_group() {
        let marker = std::env::temp_dir().join(format!(
            "autospec-owned-descendant-{}-{}",
            std::process::id(),
            super::super::DIRECT_TRANSACTION_SEQUENCE
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let mut command = shell_command(
            "sh -c 'trap \"printf term >> \\\"$AUTOSPEC_DESCENDANT_MARKER\\\"; exit 0\" TERM; printf ready > \"$AUTOSPEC_DESCENDANT_MARKER\"; while :; do sleep 1; done' & wait",
        );
        command.env("AUTOSPEC_DESCENDANT_MARKER", &marker);
        let mut owned = OwnedChildTree::spawn(&mut command, "nonce-tree".into()).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !marker.exists() && std::time::Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(marker.exists(), "descendant did not publish readiness");
        let _ = owned.terminate().expect("terminate owned descendant tree");
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while !matches!(
            std::fs::read_to_string(&marker),
            Ok(ref body) if body == "readyterm"
        ) && std::time::Instant::now() < deadline
        {
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            std::fs::read_to_string(&marker).expect("descendant signal marker"),
            "readyterm"
        );
        std::fs::remove_file(marker).expect("remove descendant signal marker");
    }

    #[cfg(unix)]
    #[test]
    fn terminate_cleans_descendants_after_group_leader_exits() {
        // Break caught: caching the leader's exit and skipping group cleanup leaves background
        // harness descendants running after terminal publication.
        let marker = std::env::temp_dir().join(format!(
            "autospec-exited-leader-descendant-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&marker);
        let script = format!("sleep 30 & echo $! > '{}'; exit 0", marker.display());
        let mut command = Command::new("/bin/sh");
        command.args(["-c", &script]);
        let mut owned = OwnedChildTree::spawn(&mut command, "exited-leader".into())
            .expect("spawn leader with background descendant");

        let leader = owned.wait().expect("reap exited leader");
        assert!(leader.success());
        let descendant = std::fs::read_to_string(&marker)
            .expect("read descendant PID")
            .trim()
            .parse::<u32>()
            .expect("parse descendant PID");
        assert!(
            super::super::process_birth_identity(descendant)
                .expect("observe descendant before cleanup")
                .is_some(),
            "fixture descendant exited before cleanup"
        );

        owned
            .terminate()
            .expect("terminate descendants after leader exit");
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while std::time::Instant::now() < deadline
            && super::super::process_birth_identity(descendant)
                .expect("observe descendant after cleanup")
                .is_some()
        {
            thread::sleep(Duration::from_millis(20));
        }
        assert!(
            super::super::process_birth_identity(descendant)
                .expect("final descendant observation")
                .is_none(),
            "owned group cleanup left a background descendant running"
        );
        let _ = std::fs::remove_file(marker);
    }

    #[test]
    fn durable_identity_without_live_owner_cannot_signal() {
        let identity = DurableProcessOwner::fixture_for_current_process();
        assert_eq!(recover_owner(&identity), RecoveryDisposition::Quarantine);
    }

    #[cfg(windows)]
    #[test]
    fn breakaway_helper_succeeds_without_job_containment() {
        let marker = TempPath::new("windows-breakaway-control");
        assert!(helper_command("spawn-descendant-and-wait", marker.path())
            .status()
            .expect("run breakaway control")
            .success());
        assert_eq!(std::fs::read(marker.path()).unwrap(), b"descendant escaped");
    }

    #[cfg(windows)]
    #[test]
    fn job_owns_descendant_before_primary_thread_runs() {
        let marker = TempPath::new("windows-owned-child-marker");
        let mut owned = OwnedChildTree::spawn_prepared(
            helper_spec("spawn-descendant-and-wait", marker.path()),
            "nonce-win".into(),
        )
        .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !marker.path().exists() && std::time::Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(std::fs::read(marker.path()).unwrap(), b"blocked");
        owned.terminate().expect("terminate job");
    }

    #[cfg(windows)]
    #[test]
    fn windows_creation_filetime_is_part_of_durable_identity() {
        let mut owned = OwnedChildTree::spawn_prepared(
            helper_spec("exit-zero", Path::new(".")),
            "nonce-win".into(),
        )
        .unwrap();
        let identity = owned.identity();
        assert!(!identity.process_start.is_empty());
        let document = identity.document("nonce-win", "intent-win");
        let recovered = DurableProcessOwner::from_document(&document, "nonce-win", "intent-win")
            .expect("parse Windows durable owner");
        assert_eq!(recover_owner(&recovered), RecoveryDisposition::Quarantine);
        assert!(owned.wait().expect("wait for zero exit").success());
    }

    #[cfg(windows)]
    #[test]
    fn windows_wait_preserves_nonzero_exit_and_cleanup_is_idempotent() {
        let mut owned = OwnedChildTree::spawn_prepared(
            helper_spec("exit-nonzero", Path::new(".")),
            "nonce-win".into(),
        )
        .unwrap();
        assert_eq!(owned.wait().expect("wait for child").code(), Some(17));
        assert_eq!(owned.terminate().expect("repeat cleanup").code(), Some(17));
    }

    #[cfg(windows)]
    #[test]
    fn repeated_job_launches_release_process_thread_and_job_handles() {
        let before = current_handle_count();
        for sequence in 0..24 {
            let mut owned = OwnedChildTree::spawn_prepared(
                helper_spec("exit-zero", Path::new(".")),
                format!("nonce-handle-{sequence}"),
            )
            .unwrap();
            assert!(owned.wait().unwrap().success());
            drop(owned);
        }
        let after = current_handle_count();
        assert!(
            after <= before.saturating_add(12),
            "repeated job launches leaked handles: before={before}, after={after}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn terminate_kills_normal_descendant_after_primary_exits() {
        let marker = TempPath::new("windows-normal-descendant");
        let mut owned = OwnedChildTree::spawn_prepared(
            helper_spec("spawn-normal-descendant-and-exit", marker.path()),
            "nonce-win".into(),
        )
        .unwrap();
        assert!(owned.wait().expect("wait for primary").success());
        owned
            .terminate()
            .expect("terminate descendants after primary exit");
        assert!(!marker.descendant_survived());
    }

    #[cfg(windows)]
    #[test]
    fn prepared_launch_honors_clear_environment_and_stdio() {
        use std::io::Read;
        let marker = TempPath::new("windows-prepared-environment");
        let stdout_path = TempPath::new("windows-prepared-stdout");
        let stderr_path = TempPath::new("windows-prepared-stderr");
        struct RemoveForbiddenEnvironment;
        impl Drop for RemoveForbiddenEnvironment {
            fn drop(&mut self) {
                std::env::remove_var("AUTOSPEC_FORBIDDEN_PARENT_ENV");
            }
        }
        std::env::set_var("AUTOSPEC_FORBIDDEN_PARENT_ENV", "secret");
        let _environment_guard = RemoveForbiddenEnvironment;
        let executable = std::env::current_exe().unwrap();
        let mut spec = helper_spec("environment-is-cleared", marker.path());
        let mut exact_environment = vec![
            (
                "AUTOSPEC_WINDOWS_CHILD_MODE".into(),
                "environment-is-cleared".into(),
            ),
            (
                "AUTOSPEC_WINDOWS_CHILD_MARKER".into(),
                marker.path().as_os_str().to_os_string(),
            ),
            ("AUTOSPEC_EXPECTED_ENV".into(), "present".into()),
        ];
        if let Some(system_root) = std::env::var_os("SystemRoot") {
            exact_environment.push(("SystemRoot".into(), system_root));
        }
        spec.environment = PreparedEnvironment {
            variables: exact_environment,
        };
        spec.program = executable;
        spec.stdout = Some(File::create(stdout_path.path()).unwrap());
        spec.stderr = Some(File::create(stderr_path.path()).unwrap());
        let mut owned = OwnedChildTree::spawn_prepared(spec, "nonce-win".into()).unwrap();
        assert!(owned.wait().unwrap().success());
        assert_eq!(std::fs::read(marker.path()).unwrap(), b"environment-ok");

        let mut spec = helper_spec("print-stdio", Path::new("."));
        spec.stdout = Some(File::create(stdout_path.path()).unwrap());
        spec.stderr = Some(File::create(stderr_path.path()).unwrap());
        let mut owned = OwnedChildTree::spawn_prepared(spec, "nonce-win-stdio".into()).unwrap();
        assert!(owned.wait().unwrap().success());
        let mut stdout = String::new();
        File::open(stdout_path.path())
            .unwrap()
            .read_to_string(&mut stdout)
            .unwrap();
        let mut stderr = String::new();
        File::open(stderr_path.path())
            .unwrap()
            .read_to_string(&mut stderr)
            .unwrap();
        assert!(stdout.contains("prepared-stdout"));
        assert!(stderr.contains("prepared-stderr"));
    }

    #[cfg(windows)]
    #[test]
    fn prepared_launch_preserves_empty_environment_cwd_and_argv() {
        let fixture = TempDirectory::new("windows-empty-environment");
        std::fs::write(
            fixture.path().join(".autospec-empty-environment-child"),
            b"ready",
        )
        .unwrap();
        let current_dir = std::fs::canonicalize(fixture.path()).unwrap();
        let executable = std::env::current_exe().unwrap();
        let expected_argv = vec![
            executable.to_string_lossy().into_owned(),
            "--exact".to_string(),
            "commands::autonomous::executor_bridge::process_owner::tests::windows_empty_environment_child"
                .to_string(),
            "--nocapture".to_string(),
        ];
        let spec = PreparedLaunchSpec {
            program: executable,
            argv: expected_argv.iter().map(OsString::from).collect(),
            current_dir: Some(current_dir.clone()),
            environment: PreparedEnvironment {
                variables: Vec::new(),
            },
            stdin: None,
            stdout: None,
            stderr: None,
        };
        let mut owned = OwnedChildTree::spawn_prepared(spec, "nonce-win-empty-env".into()).unwrap();
        assert!(owned.wait().unwrap().success());

        let observation: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(fixture.path().join("empty-environment-observation.json"))
                .expect("read empty-environment child observation"),
        )
        .unwrap();
        assert_eq!(observation["cwd"], serde_json::json!(current_dir));
        assert_eq!(observation["argv"], serde_json::json!(expected_argv));
    }
}
