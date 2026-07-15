use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static FIXTURE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[test]
fn resilience_decide_prefers_canonical_layout_over_legacy_fallbacks() {
    let fixture = ResilienceFixture::new();
    fixture.write_state(
        "owner__repo",
        state_with_lock("owner/repo", "claimed", 1, 1, Some("different-host")),
    );
    fixture.write_state("owner_repo", valid_state("owner/repo", "running", 1));

    let output = fixture.run(&["resilience", "decide", "--repo", "owner/repo"]);

    assert_eq!(output.status.code(), Some(20));
    assert_eq!(stdout(&output), "{\"decision\":\"held\"}\n");
    assert!(fixture.state_path("owner__repo").exists());
}

#[test]
fn resilience_decide_reads_underscore_and_hyphen_compatibility_layouts() {
    let underscore = ResilienceFixture::new();
    underscore.write_state(
        "owner_repo",
        state_with_lock("owner/repo", "running", 1, 1, Some("different-host")),
    );

    let output = underscore.run(&["resilience", "decide", "--repo", "owner/repo"]);

    assert_eq!(output.status.code(), Some(20));
    assert_eq!(stdout(&output), "{\"decision\":\"held\"}\n");
    assert!(underscore.canonical_state_path().exists());
    let canonical = fs::read_to_string(underscore.canonical_state_path()).expect("read migration");
    for field in [
        "\"slug\":\"owner__repo\"",
        "\"host\":\"autospec-test-host\"",
        "\"session\":\"test\"",
        "\"lock_session\":null",
        "\"lock_acquired_at\":null",
    ] {
        assert!(canonical.contains(field), "missing migrated field: {field}");
    }

    let hyphen = ResilienceFixture::new();
    hyphen.write_state(
        "owner-repo",
        state_with_lock("owner/repo", "running", 1, 1, Some("different-host")),
    );

    let output = hyphen.run(&["resilience", "decide", "--repo", "owner/repo"]);

    assert_eq!(output.status.code(), Some(20));
    assert_eq!(stdout(&output), "{\"decision\":\"held\"}\n");
    assert!(hyphen.canonical_state_path().exists());
}

#[test]
fn resilience_decide_rejects_malformed_or_foreign_state_without_a_canonical_write() {
    let malformed = ResilienceFixture::new();
    malformed.write_state("owner_repo", "{not-json}");

    let output = malformed.run(&["resilience", "decide", "--repo", "owner/repo"]);

    assert_eq!(output.status.code(), Some(3));
    assert_eq!(
        stdout(&output),
        "{\"decision\":\"reject\",\"reason\":\"malformed_state\"}\n"
    );
    assert!(!malformed.canonical_state_path().exists());

    let foreign = ResilienceFixture::new();
    foreign.write_state("owner_repo", valid_state("other/repo", "running", 1));

    let output = foreign.run(&["resilience", "decide", "--repo", "owner/repo"]);

    assert_eq!(output.status.code(), Some(3));
    assert_eq!(
        stdout(&output),
        "{\"decision\":\"reject\",\"reason\":\"foreign_state\"}\n"
    );
    assert!(!foreign.canonical_state_path().exists());
}

#[test]
fn resilience_decide_rejects_zero_lock_pid_without_a_canonical_write() {
    let fixture = ResilienceFixture::new();
    fixture.write_state(
        "owner_repo",
        format!(
            "{{\"repo\":\"owner/repo\",\"slug\":\"owner__repo\",\"status\":\"running\",\"host\":\"autospec-test-host\",\"session\":\"test\",\"heartbeat_at\":{},\"lock_pid\":0,\"lock_host\":\"autospec-test-host\",\"lock_session\":null,\"lock_acquired_at\":null}}",
            now_secs()
        ),
    );

    let output = fixture.run(&["resilience", "decide", "--repo", "owner/repo"]);

    assert_eq!(output.status.code(), Some(3));
    assert_eq!(
        stdout(&output),
        "{\"decision\":\"reject\",\"reason\":\"malformed_state\"}\n"
    );
    assert!(!fixture.canonical_state_path().exists());
}

#[test]
fn resilience_decide_reclaims_expired_leases_at_the_documented_boundaries() {
    let claimed = ResilienceFixture::new();
    claimed.write_state(
        "owner__repo",
        state_with_lock("owner/repo", "claimed", 300, 1, Some("different-host")),
    );

    let output = claimed.run(&["resilience", "decide", "--repo", "owner/repo"]);

    assert!(output.status.success());
    assert_eq!(
        stdout(&output),
        "{\"decision\":\"reclaim\",\"reason\":\"claimed_expired\"}\n"
    );

    let abandoned = ResilienceFixture::new();
    abandoned.write_state(
        "owner__repo",
        state_with_lock("owner/repo", "running", 10_800, 1, Some("different-host")),
    );

    let output = abandoned.run(&["resilience", "decide", "--repo", "owner/repo"]);

    assert!(output.status.success());
    assert_eq!(
        stdout(&output),
        "{\"decision\":\"reclaim\",\"reason\":\"abandoned\"}\n"
    );
}

#[test]
fn resilience_decide_parks_at_failure_cap_and_usage_precedes_issue_cap() {
    let failures = ResilienceFixture::new();
    failures.write_failures("owner__repo", 41, 3);

    let output = failures.run(&[
        "resilience",
        "decide",
        "--repo",
        "owner/repo",
        "--issue",
        "41",
    ]);

    assert_eq!(output.status.code(), Some(3));
    assert_eq!(
        stdout(&output),
        "{\"decision\":\"reject\",\"reason\":\"failure_cap\"}\n"
    );

    let capacity = ResilienceFixture::new();
    capacity.write_spend("owner__repo", 10, 4);

    let output = capacity.run(&[
        "resilience",
        "decide",
        "--repo",
        "owner/repo",
        "--budget-tokens",
        "10",
        "--budget-issues",
        "4",
    ]);

    assert_eq!(output.status.code(), Some(20));
    assert_eq!(
        stdout(&output),
        "{\"decision\":\"park\",\"reason\":\"usage_cap\"}\n"
    );
}

#[test]
fn resilience_decide_validates_compatibility_failure_and_spend_before_migrating_state() {
    let malformed_failure = ResilienceFixture::new();
    malformed_failure.write_state("owner_repo", valid_state("owner/repo", "running", 1));
    write_file(
        &malformed_failure
            .state_root
            .join("autonomous/owner_repo/issues/41.json"),
        "{not-json}",
    );

    let output = malformed_failure.run(&[
        "resilience",
        "decide",
        "--repo",
        "owner/repo",
        "--issue",
        "41",
    ]);

    assert_eq!(output.status.code(), Some(3));
    assert_eq!(
        stdout(&output),
        "{\"decision\":\"reject\",\"reason\":\"malformed_failure\"}\n"
    );
    assert!(!malformed_failure.canonical_state_path().exists());

    let malformed_spend = ResilienceFixture::new();
    malformed_spend.write_state("owner_repo", valid_state("owner/repo", "running", 1));
    write_file(
        &malformed_spend.spend_root.join("owner_repo/spend.json"),
        "{not-json}",
    );

    let output = malformed_spend.run(&["resilience", "decide", "--repo", "owner/repo"]);

    assert_eq!(output.status.code(), Some(3));
    assert_eq!(
        stdout(&output),
        "{\"decision\":\"reject\",\"reason\":\"malformed_spend\"}\n"
    );
    assert!(!malformed_spend.canonical_state_path().exists());
}

#[test]
fn resilience_decide_rejects_incomplete_or_incompatible_spend_before_migration() {
    for (name, spend) in [
        ("partial", "{\"schema\":1,\"tokens\":0}"),
        ("null", "{\"schema\":1,\"tokens\":null,\"issues\":0}"),
        ("missing-schema", "{\"tokens\":0,\"issues\":0}"),
        ("wrong-schema", "{\"schema\":2,\"tokens\":0,\"issues\":0}"),
        (
            "non-numeric-tokens",
            "{\"schema\":1,\"tokens\":\"0\",\"issues\":0}",
        ),
    ] {
        let fixture = ResilienceFixture::new();
        fixture.write_state("owner_repo", valid_state("owner/repo", "running", 1));
        write_file(&fixture.spend_root.join("owner_repo/spend.json"), spend);

        let output = fixture.run(&["resilience", "decide", "--repo", "owner/repo"]);

        assert_eq!(output.status.code(), Some(3), "{name}");
        assert_eq!(
            stdout(&output),
            "{\"decision\":\"reject\",\"reason\":\"malformed_spend\"}\n",
            "{name}"
        );
        assert!(
            !fixture.canonical_state_path().exists(),
            "{name} must not migrate state"
        );
    }
}

#[test]
fn resilience_decide_accepts_legacy_decimal_string_failure_issue_identifier() {
    let fixture = ResilienceFixture::new();
    write_file(
        &fixture
            .state_root
            .join("autonomous/owner__repo/issues/41.json"),
        "{\"issue\":\"41\",\"failures\":3,\"updated_at\":1}",
    );

    let output = fixture.run(&[
        "resilience",
        "decide",
        "--repo",
        "owner/repo",
        "--issue",
        "41",
    ]);

    assert_eq!(output.status.code(), Some(3));
    assert_eq!(
        stdout(&output),
        "{\"decision\":\"reject\",\"reason\":\"failure_cap\"}\n"
    );

    let mismatch = ResilienceFixture::new();
    write_file(
        &mismatch
            .state_root
            .join("autonomous/owner__repo/issues/41.json"),
        "{\"issue\":\"42\",\"failures\":3,\"updated_at\":1}",
    );

    let output = mismatch.run(&[
        "resilience",
        "decide",
        "--repo",
        "owner/repo",
        "--issue",
        "41",
    ]);

    assert_eq!(output.status.code(), Some(3));
    assert_eq!(
        stdout(&output),
        "{\"decision\":\"reject\",\"reason\":\"malformed_failure\"}\n"
    );
}

#[test]
fn resilience_decide_rejects_invalid_environment_budget_before_migration() {
    for (name, key) in [
        ("tokens", "AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS"),
        ("issues", "AUTOSPEC_AUTONOMOUS_LIFETIME_ISSUES"),
    ] {
        let fixture = ResilienceFixture::new();
        fixture.write_state("owner_repo", valid_state("owner/repo", "running", 1));

        let output = fixture.run_with_env(
            key,
            "invalid",
            &["resilience", "decide", "--repo", "owner/repo"],
        );

        assert_eq!(output.status.code(), Some(2), "{name}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(key),
            "{name} must identify the invalid environment variable"
        );
        assert!(
            !fixture.canonical_state_path().exists(),
            "{name} must not migrate state"
        );
    }
}

#[test]
fn resilience_decide_reclaims_only_a_known_same_host_dead_pid() {
    let same_host = ResilienceFixture::new();
    same_host.write_state(
        "owner__repo",
        state_with_lock(
            "owner/repo",
            "running",
            1,
            u32::MAX,
            Some("autospec-test-host"),
        ),
    );

    let output = same_host.run(&["resilience", "decide", "--repo", "owner/repo"]);

    assert!(output.status.success());
    assert_eq!(
        stdout(&output),
        "{\"decision\":\"reclaim\",\"reason\":\"dead_same_host_pid\"}\n"
    );
}

#[test]
fn resilience_decide_holds_unknown_host_identity_even_when_pid_is_dead() {
    let fixture = ResilienceFixture::new();
    fixture.write_state(
        "owner__repo",
        state_with_lock("owner/repo", "running", 1, u32::MAX, Some("unknown")),
    );

    let output =
        fixture.run_with_host("unknown", &["resilience", "decide", "--repo", "owner/repo"]);

    assert_eq!(output.status.code(), Some(20));
    assert_eq!(stdout(&output), "{\"decision\":\"held\"}\n");
}

#[test]
fn resilience_decide_treats_legacy_released_lock_as_available() {
    let fixture = ResilienceFixture::new();
    fixture.write_state("owner__repo", valid_state("owner/repo", "running", 1));

    let output = fixture.run(&["resilience", "decide", "--repo", "owner/repo"]);

    assert!(output.status.success());
    assert_eq!(stdout(&output), "{\"decision\":\"available\"}\n");
}

#[test]
fn resilience_source_keeps_shell_resilience_authority_out_of_rust() {
    let source = fs::read_to_string(
        workspace_root().join("crates/autospec-cli/src/commands/autonomous/resilience.rs"),
    )
    .expect("read resilience adapter source");

    for forbidden in [
        "scripts/autonomous-resilience.sh",
        "Command::new(\"sh\")",
        "Command::new(\"bash\")",
    ] {
        assert!(
            !source.contains(forbidden),
            "resilience adapter retains legacy shell authority: {forbidden}"
        );
    }
}

struct ResilienceFixture {
    root: PathBuf,
    state_root: PathBuf,
    spend_root: PathBuf,
}

impl ResilienceFixture {
    fn new() -> Self {
        let sequence = FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "autospec-resilience-test-{}-{}",
            std::process::id(),
            sequence
        ));
        fs::create_dir_all(&root).expect("create fixture root");
        Self {
            state_root: root.join("state"),
            spend_root: root.join("spend"),
            root,
        }
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command()
            .args(args)
            .env("AUTOSPEC_HOST", "autospec-test-host")
            .output()
            .expect("run resilience decision")
    }

    fn run_with_host(&self, host: &str, args: &[&str]) -> Output {
        self.command()
            .args(args)
            .env("AUTOSPEC_HOST", host)
            .output()
            .expect("run resilience decision with a host")
    }

    fn run_with_env(&self, key: &str, value: &str, args: &[&str]) -> Output {
        self.command()
            .args(args)
            .env("AUTOSPEC_HOST", "autospec-test-host")
            .env(key, value)
            .output()
            .expect("run resilience decision with an environment override")
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_autospec"));
        command
            .arg("autonomous")
            .env("AUTOSPEC_STATE_DIR", &self.state_root)
            .env("AUTOSPEC_AUTONOMOUS_SPEND_DIR", &self.spend_root)
            .env("HOME", &self.root);
        command
    }

    fn canonical_state_path(&self) -> PathBuf {
        self.state_path("owner__repo")
    }

    fn state_path(&self, slug: &str) -> PathBuf {
        self.state_root
            .join("autonomous")
            .join(slug)
            .join("state.json")
    }

    fn write_state(&self, slug: &str, value: impl AsRef<str>) {
        write_file(&self.state_path(slug), value.as_ref());
    }

    fn write_failures(&self, slug: &str, issue: u64, failures: u8) {
        write_file(
            &self
                .state_root
                .join("autonomous")
                .join(slug)
                .join("issues")
                .join(format!("{issue}.json")),
            &format!("{{\"issue\":{issue},\"failures\":{failures},\"updated_at\":1}}"),
        );
    }

    fn write_spend(&self, slug: &str, tokens: u64, issues: u64) {
        write_file(
            &self.spend_root.join(slug).join("spend.json"),
            &format!("{{\"schema\":1,\"tokens\":{tokens},\"issues\":{issues},\"parked\":false}}"),
        );
    }
}

impl Drop for ResilienceFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn valid_state(repo: &str, status: &str, age_secs: u64) -> String {
    state_with_lock(repo, status, age_secs, 0, None)
}

fn state_with_lock(
    repo: &str,
    status: &str,
    age_secs: u64,
    lock_pid: u32,
    lock_host: Option<&str>,
) -> String {
    let heartbeat_at = now_secs().saturating_sub(age_secs);
    format!(
        "{{\"repo\":\"{repo}\",\"slug\":\"owner__repo\",\"status\":\"{status}\",\"host\":\"autospec-test-host\",\"session\":\"test\",\"heartbeat_at\":{heartbeat_at},\"lock_pid\":{},\"lock_host\":{},\"lock_session\":null,\"lock_acquired_at\":null}}",
        if lock_pid == 0 { "null".to_string() } else { lock_pid.to_string() },
        lock_host.map(|host| format!("\"{host}\"")).unwrap_or_else(|| "null".to_string()),
    )
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("current time")
        .as_secs()
}

fn write_file(path: &Path, contents: &str) {
    fs::create_dir_all(path.parent().expect("parent directory")).expect("create parent directory");
    fs::write(path, contents).expect("write fixture state");
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
}
