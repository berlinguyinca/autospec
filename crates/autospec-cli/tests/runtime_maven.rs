use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use autospec_core::runtime_env::{
    EnvironmentLifecycle, EnvironmentOwner, MavenArgs, ResourceInventory,
};

static NEXT_FIXTURE: AtomicUsize = AtomicUsize::new(0);

struct MavenFixture {
    root: PathBuf,
    state_root: PathBuf,
    fake: PathBuf,
}

impl MavenFixture {
    fn new(version: &str) -> Self {
        use std::os::unix::fs::PermissionsExt;
        let suffix = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "autospec-runtime-maven-{}-{suffix}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".autospec")).unwrap();
        std::fs::write(
            root.join(".autospec/runtime.yml"),
            "version: 1\ndefault_mode: local\nmodes:\n  local:\n    command: sh -c 'true'\n",
        )
        .unwrap();
        std::fs::write(root.join("pom.xml"), "<project/>").unwrap();
        let fake = root.join("fake-maven");
        std::fs::create_dir_all(fake.join("bin")).unwrap();
        let script = fake.join("bin/mvn");
        let dash = '-';
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nif [ \"$1\" = {dash}{dash}version ]; then printf '%s\\n' 'Apache Maven {version}'; exit 0; fi\nprintf '%s\\n' \"$MAVEN_ARGS\" > '{capture}'\ntouch '{queried}'\nmkdir -p '{repository}'\nprintf '%s\\n' '{repository}'\n",
                capture = fake.join("captured-args").display(),
                queried = fake.join("repository-queried").display(),
                repository = fake.join("repository").display(),
            ),
        )
        .unwrap();
        let mut permissions = std::fs::metadata(&script).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(script, permissions).unwrap();
        let state_root = root.join("state");
        Self {
            root,
            state_root,
            fake,
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_autospec"));
        let path = std::env::var_os("PATH").unwrap_or_default();
        command.env("AGENT_ENV_STATE_ROOT", &self.state_root).env(
            "PATH",
            std::env::join_paths(
                std::iter::once(self.fake.join("bin")).chain(std::env::split_paths(&path)),
            )
            .unwrap(),
        );
        command
    }

    fn up(&self) -> std::process::Output {
        self.command()
            .args(["runtime", "env", "up", "--repo"])
            .arg(&self.root)
            .output()
            .unwrap()
    }

    fn environment(&self) -> PathBuf {
        std::fs::read_dir(&self.state_root)
            .unwrap()
            .find_map(|entry| {
                let path = entry.unwrap().path();
                path.join("owner.json").is_file().then_some(path)
            })
            .unwrap()
    }

    fn inventory(&self) -> ResourceInventory {
        autospec_core::runtime_env::read_json(&self.environment().join("inventory.json")).unwrap()
    }
}

impl Drop for MavenFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn maven_four_preserves_caller_arguments_and_records_the_effective_prefix() {
    let fixture = MavenFixture::new("4.0.0-rc-5");
    let output = fixture
        .command()
        .env("MAVEN_ARGS", r#"-T 2 -s 'settings with spaces.xml'"#)
        .args(["runtime", "env", "up", "--repo"])
        .arg(&fixture.root)
        .output()
        .unwrap();

    assert_success(&output);
    let captured = std::fs::read_to_string(fixture.fake.join("captured-args")).unwrap();
    let inventory = fixture.inventory();
    let tokens = MavenArgs::parse(captured.trim())
        .unwrap()
        .tokens()
        .iter()
        .map(|token| token.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert_eq!(
        tokens,
        vec![
            "-T".to_string(),
            "2".to_string(),
            "-s".to_string(),
            "settings with spaces.xml".to_string(),
            "-Daether.lrm.enhanced.split=true".to_string(),
            "-Daether.lrm.enhanced.remotePrefix=cached".to_string(),
            format!(
                "-Daether.lrm.enhanced.localPrefix=autospec/{}",
                inventory.environment_id
            ),
            "-Daether.system.named.factory=file-lock".to_string(),
        ]
    );
    assert_eq!(
        inventory.maven_local_prefix.unwrap(),
        fixture
            .fake
            .join("repository/autospec")
            .join(inventory.environment_id)
    );
}

#[test]
fn maven_three_and_conflicting_properties_fail_before_repository_query() {
    for (version, arguments, diagnostic) in [
        (
            "3.9.9",
            "",
            "MAVEN_VERSION_UNSUPPORTED: Maven 4 is required, found 3.9.9",
        ),
        (
            "4.0.0-rc-5",
            "-Daether.lrm.enhanced.remotePrefix=private",
            "MAVEN_ARGUMENT_CONFLICT: managed Maven property aether.lrm.enhanced.remotePrefix has conflicting value \"private\"",
        ),
    ] {
        let fixture = MavenFixture::new(version);
        let output = fixture
            .command()
            .env("MAVEN_ARGS", arguments)
            .args(["runtime", "env", "up", "--repo"])
            .arg(&fixture.root)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert_eq!(stderr(&output), diagnostic);
        assert!(!fixture.fake.join("repository-queried").exists());
    }
}

#[test]
fn ordinary_down_retains_prefix_and_purge_removes_only_owned_content() {
    let fixture = MavenFixture::new("4.0.0-rc-5");
    assert_success(&fixture.up());
    let owned = fixture.inventory().maven_local_prefix.unwrap();
    std::fs::create_dir_all(&owned).unwrap();
    std::fs::write(owned.join("installed.bin"), "owned").unwrap();
    let cached = fixture.fake.join("repository/cached/remote.bin");
    std::fs::create_dir_all(cached.parent().unwrap()).unwrap();
    std::fs::write(&cached, "shared").unwrap();

    assert_success(&down(&fixture, false));
    assert!(owned.is_dir());
    assert_success(&down(&fixture, true));
    assert!(!owned.exists());
    assert_eq!(std::fs::read_to_string(cached).unwrap(), "shared");
}

#[test]
fn failed_mode_command_records_cleanup_failed_and_explicit_purge_recovers() {
    let fixture = MavenFixture::new("4.0.0-rc-5");
    std::fs::write(
        fixture.root.join(".autospec/runtime.yml"),
        "version: 1\ndefault_mode: local\nmodes:\n  local:\n    command: sh -c 'exit 41'\n",
    )
    .unwrap();

    let failed = fixture.up();
    assert_eq!(failed.status.code(), Some(41));
    let environment = fixture.environment();
    let owner: EnvironmentOwner =
        autospec_core::runtime_env::read_json(&environment.join("owner.json")).unwrap();
    assert_eq!(owner.lifecycle, EnvironmentLifecycle::CleanupFailed);
    let owned = fixture.inventory().maven_local_prefix.unwrap();
    std::fs::create_dir_all(&owned).unwrap();

    assert_success(&down(&fixture, true));
    assert!(!owned.exists());
    assert!(!environment.join("owner.json").exists());
}

#[test]
fn purge_fails_closed_when_retained_ownership_is_tampered_or_partial() {
    let fixture = MavenFixture::new("4.0.0-rc-5");
    assert_success(&fixture.up());
    let environment = fixture.environment();
    let mut inventory = fixture.inventory();
    let owned = inventory.maven_local_prefix.clone().unwrap();
    std::fs::create_dir_all(&owned).unwrap();
    assert_success(&down(&fixture, false));
    inventory.maven_local_prefix = Some(fixture.fake.join("repository/autospec/not-owned"));
    autospec_core::runtime_env::write_json_atomic(&environment.join("inventory.json"), &inventory)
        .unwrap();
    let rejected = down(&fixture, true);
    assert!(!rejected.status.success());
    assert_eq!(
        stderr(&rejected),
        "MAVEN_PURGE_IDENTITY_MISMATCH: inventory does not own the effective Maven prefix"
    );
    assert!(owned.is_dir());

    std::fs::remove_file(environment.join("owner.json")).unwrap();
    let rejected = down(&fixture, true);
    assert!(!rejected.status.success());
    assert_eq!(
        stderr(&rejected),
        format!(
            "RUNTIME_PARTIAL_STATE: owner.json, plan.json, and inventory.json must all exist under {}",
            environment.display()
        )
    );
    assert!(owned.is_dir());
}

#[test]
fn purge_rejects_symlinked_prefix_and_live_session() {
    use std::os::unix::fs::symlink;
    let fixture = MavenFixture::new("4.0.0-rc-5");
    assert_success(&fixture.up());
    let owned = fixture.inventory().maven_local_prefix.unwrap();
    let outside = fixture.root.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::create_dir_all(owned.parent().unwrap()).unwrap();
    symlink(&outside, &owned).unwrap();
    let rejected = down(&fixture, true);
    assert!(!rejected.status.success());
    assert_eq!(
        stderr(&rejected),
        format!(
            "MAVEN_PURGE_SYMLINK: refusing symlinked Maven prefix {}",
            owned.display()
        )
    );
    assert!(outside.is_dir());
    std::fs::remove_file(&owned).unwrap();

    let ready = fixture.root.join("ready");
    let release = fixture.root.join("release");
    let mut session = fixture
        .command()
        .args(["runtime", "env", "session", "--keep-alive", "--repo"])
        .arg(&fixture.root)
        .args(["--", "sh", "-c"])
        .arg(format!(
            "touch '{}'; while [ ! -f '{}' ]; do sleep 0.02; done",
            ready.display(),
            release.display()
        ))
        .spawn()
        .unwrap();
    wait_for(&ready);
    let rejected = down(&fixture, true);
    assert!(!rejected.status.success());
    assert_eq!(
        stderr(&rejected),
        "RUNTIME_LIVE_SESSIONS: 1 live runtime session(s) prevent teardown"
    );
    std::fs::write(release, "release").unwrap();
    assert!(session.wait().unwrap().success());
}

fn down(fixture: &MavenFixture, purge: bool) -> std::process::Output {
    let mut command = fixture.command();
    command.args(["runtime", "env", "down"]);
    if purge {
        command.arg("--purge-maven");
    }
    command.arg("--repo").arg(&fixture.root).output().unwrap()
}

fn wait_for(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !path.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(path.exists(), "timed out waiting for {}", path.display());
}

fn assert_success(output: &std::process::Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn stderr(output: &std::process::Output) -> String {
    String::from_utf8(output.stderr.clone())
        .unwrap()
        .trim()
        .to_string()
}
