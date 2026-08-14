// executor_bridge tests: shared fixtures (1 of 3).
//
// Split out of tests.rs; see the note in that file. These are the helpers more than
// one test module builds on, so they are `pub(super)` rather than private.

use super::super::{BridgePhase, PersistedInvocation, ProcessIdentity};
use super::support_invocation::supervision_state;
use crate::commands::autonomous::executor_bridge as bridge;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub(super) static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(in super::super) static TEST_ENVIRONMENT: Mutex<()> = Mutex::new(());

/// Restore every injected-fault switch to the value it was declared with.
///
/// The defaults are read off the declarations rather than assumed to be zero:
/// CLEANUP_FAILPOINT_PREVIOUS_SUBREAPER starts at 2, so storing 0 into it would quietly
/// change behaviour instead of restoring it. The two *_SEQUENCE counters are deliberately
/// absent — they are not failpoints and tests do not own them.
fn reset_failpoints() {
    for switch in [
        &bridge::BASE_DRIFT_FAILPOINT,
        &bridge::EMPTY_RETRY_BASE_FAILPOINT,
        &bridge::RUNTIME_CLOSE_FAILPOINT,
        &bridge::METADATA_WIP_FAILPOINT,
        &bridge::WORKTREE_REPAIR_FAILPOINT,
        &bridge::POST_CI_RECREATE_FAILPOINT,
        &bridge::PRUNABLE_RECLAIM_FAILPOINT,
        &bridge::ZERO_EFFECT_RECOVERY_FAILPOINT,
        &bridge::ZERO_EFFECT_SCOPE_PARENT_SYNC_FAILPOINT,
        &bridge::EXECUTOR_ROOT_HARDEN_FAILPOINT,
        &bridge::IMPLEMENTATION_COMMIT_FAILPOINT,
        &bridge::NPM_MANIFEST_OPEN_FAILPOINT,
        &bridge::PARENT_CAPTURE_FAILPOINT,
        &bridge::PARENT_REAP_FAILPOINT,
        &bridge::RAW_READ_INTERRUPTED_ONCE,
    ] {
        switch.store(0, Ordering::SeqCst);
    }
    let none = bridge::LaunchFailpoint::None as u8;
    bridge::LAUNCH_FAILPOINT.store(none, Ordering::SeqCst);
    bridge::CLEANUP_FAILPOINT.store(none, Ordering::SeqCst);
    bridge::LAST_SPAWN_SUPERVISOR.store(0, Ordering::SeqCst);
    bridge::LAST_SPAWN_HARNESS.store(0, Ordering::SeqCst);
    #[cfg(target_os = "linux")]
    bridge::CLEANUP_FAILPOINT_PREVIOUS_SUBREAPER.store(2, Ordering::SeqCst);
}

/// Orders the tests that arm process-wide failpoints, and disarms them on the way out.
///
/// Recovering a poisoned guard is right here: this mutex sequences tests and guards no data
/// invariant, so a holder that panicked has left nothing corrupt for the next test to find.
/// Without recovery a single genuine failure cascades into a run of spurious
/// `test environment lock` panics that hide it.
///
/// Dropping the guard resets the failpoints. Arming is a bare store, and a test that panics
/// between arming and disarming leaves an injected fault set for whatever launches next —
/// which reads as that test's bug, in a different file, with nothing linking the two. Drop
/// runs on unwind, so the fault cannot outlive the test that asked for it (#2981).
pub(super) struct TestEnvironment {
    _guard: std::sync::MutexGuard<'static, ()>,
    restore_harness_env: super::support_harness_env::HarnessEnvRestore,
}

/// Arming lives here and nowhere else.
///
/// Reaching any of these requires a value that owns the mutex guard, so a test cannot arm a
/// process-wide fault without first ordering itself against every other test that launches.
/// That was previously a convention, and eight tests did not follow it (#2989) — one of them
/// hung the whole suite. A convention that eight tests break is not a convention.
impl TestEnvironment {
    pub(super) fn launch(&self, failpoint: bridge::LaunchFailpoint) {
        bridge::set_launch_failpoint(failpoint);
    }

    pub(super) fn cleanup(&self, failpoint: bridge::LaunchFailpoint) {
        bridge::set_cleanup_failpoint(failpoint);
    }

    pub(super) fn parent_capture(&self, enabled: bool) {
        bridge::set_parent_capture_failpoint(enabled);
    }

    pub(super) fn parent_reap(&self, failpoint: bridge::ParentReapFailpoint) {
        bridge::set_parent_reap_failpoint(failpoint);
    }

    pub(super) fn zero_effect_recovery(&self, failpoint: bridge::ZeroEffectRecoveryFailpoint) {
        bridge::set_zero_effect_recovery_failpoint(failpoint);
    }
}

impl Drop for TestEnvironment {
    fn drop(&mut self) {
        reset_failpoints();
        super::support_harness_env::restore(&mut self.restore_harness_env);
    }
}

pub(super) fn test_environment() -> TestEnvironment {
    let guard = TEST_ENVIRONMENT
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    // Scrub inside the guard: these are process-wide, so the same lock that orders failpoint
    // arming has to order this too, and Drop restores them on unwind like the failpoints.
    TestEnvironment {
        _guard: guard,
        restore_harness_env: super::support_harness_env::scrub(),
    }
}

#[cfg(target_os = "linux")]
pub(super) struct DetachedSupervisorCleanup(pub(super) ProcessIdentity);

#[cfg(target_os = "linux")]
pub(super) struct DirectCrashFixtureCleanup {
    pub(super) parent: Option<std::process::Child>,
    pub(super) conductor: Option<bridge::OwnedProcessSet>,
    pub(super) launch: PathBuf,
    pub(super) supervisor: Option<ProcessIdentity>,
    pub(super) harness: Option<ProcessIdentity>,
}

#[cfg(target_os = "linux")]
impl DirectCrashFixtureCleanup {
    pub(super) fn new(mut parent: std::process::Child, launch: PathBuf) -> Self {
        let conductor = match bridge::OwnedProcessSet::from_forked_child(parent.id()) {
            Ok(conductor) => conductor,
            Err(error) => {
                let _ = parent.kill();
                let _ = parent.wait();
                panic!("arm exact crash conductor cleanup: {error}");
            }
        };
        Self {
            parent: Some(parent),
            conductor: Some(conductor),
            launch,
            supervisor: None,
            harness: None,
        }
    }

    pub(super) fn arm(&mut self, supervisor: ProcessIdentity, harness: ProcessIdentity) {
        self.supervisor = Some(supervisor);
        self.harness = Some(harness);
    }

    pub(super) fn crash_parent(&mut self) {
        let parent = self.parent.as_mut().expect("crash parent is armed");
        parent.kill().expect("crash command parent");
        parent.wait().expect("reap crashed command parent");
        self.parent = None;
    }

    pub(super) fn disarm(mut self) {
        self.parent = None;
        self.conductor = None;
        self.supervisor = None;
        self.harness = None;
    }

    pub(super) fn recover_launch_identities(&mut self) {
        if self.supervisor.is_some() && self.harness.is_some() {
            return;
        }
        let Ok(body) = fs::read_to_string(&self.launch) else {
            return;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&body) else {
            return;
        };
        if self.supervisor.is_none() {
            self.supervisor = value.get("supervisor").cloned().and_then(|value| {
                bridge::parse_process_identity(value, "fixture supervisor cleanup").ok()
            });
        }
        if self.harness.is_none() {
            self.harness = value.get("process").cloned().and_then(|value| {
                bridge::parse_process_identity(value, "fixture harness cleanup").ok()
            });
        }
    }

    pub(super) fn terminate_birth_tree(identity: &ProcessIdentity) {
        let Ok(leader) = bridge::OwnedProcess::capture_cleanup_instance(identity) else {
            return;
        };
        let mut processes = bridge::OwnedProcessSet {
            leader,
            descendants: BTreeMap::new(),
            exited_descendants: BTreeMap::new(),
        };
        let _ = processes.capture_descendants_while_leader_live();
        let _ = processes.terminate();
    }
}

#[cfg(target_os = "linux")]
impl Drop for DirectCrashFixtureCleanup {
    fn drop(&mut self) {
        if let Some(conductor) = self.conductor.as_mut() {
            let _ = conductor.capture_descendants_while_leader_live();
            let _ = conductor.terminate();
        }
        self.conductor = None;
        if let Some(parent) = self.parent.as_mut() {
            let _ = parent.kill();
            let _ = parent.wait();
        }
        self.parent = None;
        self.recover_launch_identities();
        if let Some(supervisor) = &self.supervisor {
            Self::terminate_birth_tree(supervisor);
        }
        if let Some(harness) = &self.harness {
            Self::terminate_birth_tree(harness);
        }
    }
}

/// Reap a fixture child, giving up after `limit` instead of blocking forever.
///
/// A bare `waitpid(pid, None)` waits without bound, and the PID reaching it comes from
/// LAST_SPAWN_SUPERVISOR — a process-global that any spawning test overwrites. Wait on a PID
/// that is not ours and the call never returns; do it while holding TEST_ENVIRONMENT and the
/// whole suite queues behind it until the harness is killed. That is the shape of #2981.
///
/// Serialising the armers stops the wrong PID arriving. This stops a wrong PID being fatal:
/// the test fails in seconds with something legible instead of hanging, which is the
/// difference between a bug you diagnose over lunch and one that costs an hour a sample.
/// Returns whether the child was actually reaped.
#[cfg(target_os = "linux")]
pub(super) fn reap_fixture_child_within(pid: u32, limit: Duration) -> bool {
    let pid = nix::unistd::Pid::from_raw(i32::try_from(pid).expect("fixture PID range"));
    let deadline = Instant::now() + limit;
    loop {
        match nix::sys::wait::waitpid(pid, Some(nix::sys::wait::WaitPidFlag::WNOHANG)) {
            Ok(nix::sys::wait::WaitStatus::StillAlive) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(nix::sys::wait::WaitStatus::StillAlive) => return false,
            Err(nix::errno::Errno::EINTR) => continue,
            Ok(_) => return true,
            // ECHILD/ESRCH: not ours, or already reaped. Either way, nothing to wait for.
            Err(_) => return false,
        }
    }
}

#[cfg(target_os = "linux")]
pub(super) fn reap_exited_fixture_child(pid: u32) {
    let pid = nix::unistd::Pid::from_raw(i32::try_from(pid).expect("fixture PID range"));
    loop {
        match nix::sys::wait::waitpid(pid, Some(nix::sys::wait::WaitPidFlag::WNOHANG)) {
            Ok(nix::sys::wait::WaitStatus::StillAlive)
            | Err(nix::errno::Errno::ECHILD)
            | Err(nix::errno::Errno::ESRCH) => {
                break;
            }
            Ok(_) => break,
            Err(nix::errno::Errno::EINTR) => continue,
            Err(error) => panic!("reap exited executor fixture child: {error}"),
        }
    }
}

#[cfg(target_os = "linux")]
impl Drop for DetachedSupervisorCleanup {
    fn drop(&mut self) {
        if let Ok(mut processes) = bridge::OwnedProcessSet::adopt(&self.0) {
            let _ = processes.terminate();
        }
        reap_exited_fixture_child(self.0.pid);
    }
}

#[cfg(target_os = "linux")]
pub(super) struct DetachedForkedCleanup {
    pub(super) processes: Option<bridge::OwnedProcessSet>,
    pub(super) stable_identity: Option<ProcessIdentity>,
}

#[cfg(target_os = "linux")]
impl DetachedForkedCleanup {
    pub(super) fn new(pid: u32) -> Result<Self, String> {
        Ok(Self {
            processes: Some(bridge::OwnedProcessSet::from_forked_child(pid)?),
            stable_identity: None,
        })
    }

    pub(super) fn confirm_identity(&mut self, identity: ProcessIdentity) {
        self.stable_identity = Some(identity);
    }
}

#[cfg(target_os = "linux")]
impl Drop for DetachedForkedCleanup {
    fn drop(&mut self) {
        let leader = self
            .processes
            .as_ref()
            .map(|processes| processes.leader.birth.pid);
        if let Some(processes) = self.processes.as_mut() {
            let _ = processes.terminate();
        }
        if let Some(leader) = leader {
            reap_exited_fixture_child(leader);
        }
    }
}

#[cfg(target_os = "linux")]
pub(super) fn observe_spawned_identity(pid: u32, args: &[String]) -> ProcessIdentity {
    let executable = fs::canonicalize("/bin/sh").expect("canonical fixture shell");
    for _ in 0..100 {
        if let Some(identity) = bridge::observe_process_identity(pid, &bridge::argv_digest(args))
            .expect("observe spawned fixture")
        {
            if identity.executable == executable
                && identity.argv_digest == bridge::argv_digest(args)
                && bridge::OwnedProcess::capture_identity(&identity).is_ok()
            {
                return identity;
            }
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    panic!("spawned fixture never reached its stable exec identity");
}

pub(super) fn test_root(label: &str) -> PathBuf {
    let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let root = std::env::current_dir()
        .expect("current executor bridge test directory")
        .join("target/executor-bridge-tests")
        .join(format!(
            "autospec-autonomous-executor-bridge-{label}-{}-{sequence}",
            std::process::id()
        ));
    fs::create_dir_all(&root).expect("create executor bridge test root");
    fs::canonicalize(root).expect("canonical executor bridge test root")
}

pub(super) fn write_alias_table(root: &Path, body: &str) -> PathBuf {
    let path = root.join("harness-runtime-aliases.tsv");
    fs::write(&path, body).expect("write harness alias table");
    path
}

pub(super) fn write_executable(path: &Path, body: &str) {
    fs::write(path, body).expect("write executable fixture");
    #[cfg(unix)]
    {
        let mut permissions = fs::metadata(path)
            .expect("executable fixture metadata")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).expect("executable fixture mode");
    }
}

pub(super) fn environment(table: &Path) -> BTreeMap<String, OsString> {
    BTreeMap::from([
        (
            "AUTOSPEC_HARNESS_RUNTIME_ALIASES".to_string(),
            table.as_os_str().to_os_string(),
        ),
        ("PATH".to_string(), OsString::from("/bin:/usr/bin")),
    ])
}

pub(super) fn installed_aliases() -> &'static str {
    "claude\ttrue\t--dangerously-skip-permissions\tClaude Code\n\
     codex\tsh\t--yolo\tCodex CLI\n\
     opencode\tfalse\t\tOpenCode\n"
}

pub(super) struct GitFixture {
    pub(super) root: PathBuf,
    pub(super) repo: PathBuf,
    pub(super) executor_scope_roots: Vec<PathBuf>,
}

impl GitFixture {
    pub(super) fn new(label: &str) -> Self {
        let root = test_root(label);
        let remote = root.join("remote.git");
        let seed = root.join("seed");
        let repo = root.join("repo");
        git(
            &root,
            &["init", "--bare", remote.to_str().expect("remote path")],
        );
        git(&root, &["init", seed.to_str().expect("seed path")]);
        git(&seed, &["config", "user.email", "autospec@example.invalid"]);
        git(&seed, &["config", "user.name", "Autospec Test"]);
        fs::write(seed.join("README.md"), "fixture\n").expect("write fixture");
        git(&seed, &["add", "README.md"]);
        git(&seed, &["commit", "-m", "fixture"]);
        git(&seed, &["branch", "-M", "main"]);
        git(
            &seed,
            &[
                "remote",
                "add",
                "origin",
                remote.to_str().expect("remote path"),
            ],
        );
        git(&seed, &["push", "-u", "origin", "main"]);
        git(
            &root,
            &[
                "--git-dir",
                remote.to_str().expect("remote path"),
                "symbolic-ref",
                "HEAD",
                "refs/heads/main",
            ],
        );
        git(
            &root,
            &[
                "clone",
                remote.to_str().expect("remote path"),
                repo.to_str().expect("repo path"),
            ],
        );
        git(&repo, &["config", "user.email", "autospec@example.invalid"]);
        git(&repo, &["config", "user.name", "Autospec Test"]);
        Self {
            root,
            repo,
            executor_scope_roots: Vec::new(),
        }
    }

    pub(super) fn branch(&self, branch: &str) -> String {
        git(&self.repo, &["checkout", "-b", branch]);
        let filename = format!("{}.txt", branch.replace('/', "_"));
        fs::write(self.repo.join(filename), branch).expect("write branch file");
        git(&self.repo, &["add", "."]);
        git(&self.repo, &["commit", "-m", branch]);
        git(&self.repo, &["push", "-u", "origin", branch]);
        git(&self.repo, &["checkout", "main"]);
        git_stdout(&self.repo, &["rev-parse", &format!("origin/{branch}")])
    }
}

impl Drop for GitFixture {
    fn drop(&mut self) {
        for scope_root in &self.executor_scope_roots {
            let _ = fs::remove_dir_all(scope_root);
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}

pub(super) fn git(directory: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(directory)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

pub(super) fn git_stdout(directory: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(directory)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("git UTF-8")
        .trim()
        .to_string()
}

pub(super) fn zero_effect_classifier_fixture(
    label: &str,
    changed_head: bool,
    remove_worktree: bool,
) -> (GitFixture, PersistedInvocation, PathBuf, PathBuf) {
    let mut fixture = GitFixture::new(label);
    let repository_scope = format!(
        "test/{label}-{}-{}",
        std::process::id(),
        TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let scope_root = bridge::executor_worktree_root()
        .join(bridge::safe_scope(&repository_scope).expect("safe repository scope"));
    bridge::ensure_private_directory(&scope_root).expect("private executor scope");
    fixture.executor_scope_roots.push(scope_root.clone());
    let worktree = scope_root.join("issue-42");
    git(
        &fixture.repo,
        &[
            "worktree",
            "add",
            "-b",
            "feat/autonomous-issue-42",
            worktree.to_str().expect("worktree path"),
            "origin/main",
        ],
    );
    if changed_head {
        fs::write(worktree.join("changed.txt"), "changed\n").expect("changed fixture");
        git(&worktree, &["add", "changed.txt"]);
        git(&worktree, &["commit", "-m", "changed fixture"]);
    }
    let mut state = supervision_state(&fixture);
    state.phase = BridgePhase::ImplementationComplete;
    state.identity.repository = repository_scope;
    state.identity.worktree = fs::canonicalize(&worktree).expect("canonical worktree");
    state.identity.runtime_environment_dir = None;
    state.identity.runtime_session_id = None;
    let state_path = fixture.root.join("state/zero-effect.json");
    let snapshot = format!(
        "{}\n",
        serde_json::json!({
            "schema": 1,
            "identity": {
                "repository": state.identity.repository,
                "repository_path": state.identity.repository_path,
                "issue": state.identity.issue,
                "worker_id": state.identity.worker_id,
                "branch": state.identity.branch,
                "claim_id": state.identity.claim_id,
                "invocation_id": state.identity.invocation_id,
                "base_ref": state.identity.base_ref,
                "base_oid": state.identity.base_oid,
                "worktree": state.identity.worktree,
                "local_head": state.identity.base_oid,
                "dirty_wip": false,
            },
            "refs": {
                "refs/heads/main": state.identity.base_oid,
            },
            "pull_requests": [],
        })
    );
    let snapshot_path = bridge::remote_snapshot_path(&state_path);
    fs::create_dir_all(snapshot_path.parent().expect("snapshot parent"))
        .expect("create snapshot parent");
    fs::write(&snapshot_path, &snapshot).expect("write prelaunch snapshot");
    fs::set_permissions(&snapshot_path, fs::Permissions::from_mode(0o600))
        .expect("private prelaunch snapshot");
    state.remote_snapshot_digest = Some(bridge::sha256_hex(snapshot.as_bytes()));
    bridge::write_invocation_atomic(&state_path, &state).expect("persist zero-effect state");
    let sinks = bridge::output_sink_paths(&state_path, &state.identity.invocation_id)
        .expect("zero-effect output sinks");
    fs::create_dir_all(sinks.exit_status.parent().expect("exit parent"))
        .expect("create exit parent");
    let mut exit = [0_u8; 16];
    exit[..4].copy_from_slice(&0_i32.to_ne_bytes());
    exit[4..8].copy_from_slice(b"EXIT");
    exit[8..12].copy_from_slice(&0_i32.to_ne_bytes());
    exit[12..].copy_from_slice(b"DONE");
    fs::write(&sinks.exit_status, exit).expect("write synced exit");
    fs::set_permissions(&sinks.exit_status, fs::Permissions::from_mode(0o600))
        .expect("private synced exit");
    if remove_worktree {
        fs::remove_dir_all(&worktree).expect("make worktree prunable");
    }
    (fixture, state, state_path, sinks.exit_status)
}
