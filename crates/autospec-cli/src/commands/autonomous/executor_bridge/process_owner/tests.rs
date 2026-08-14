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

    let leader = owned
        .wait()
        .expect("drain owned group before reaping exited leader");
    assert!(leader.success());
    let descendant = std::fs::read_to_string(&marker)
        .expect("read descendant PID")
        .trim()
        .parse::<u32>()
        .expect("parse descendant PID");
    owned
        .terminate()
        .expect("completed ownership is consumed exactly once");
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
