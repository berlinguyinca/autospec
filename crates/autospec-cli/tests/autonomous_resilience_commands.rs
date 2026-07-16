use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static FIXTURE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[test]
fn resilience_help_names_the_canonical_write_slug() {
    let output = Command::new(env!("CARGO_BIN_EXE_autospec"))
        .args(["autonomous", "resilience", "--help"])
        .output()
        .expect("run help");
    assert!(output.status.success());
    let help = String::from_utf8_lossy(&output.stdout);
    assert!(help.contains("resilience decide"));
    assert!(help.contains("owner__repo"));
}

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
fn resilience_decide_reads_underscore_and_hyphen_compatibility_layouts_without_migration() {
    let underscore = ResilienceFixture::new();
    underscore.write_state(
        "owner_repo",
        state_with_lock("owner/repo", "running", 1, 1, Some("different-host")),
    );

    let output = underscore.run(&["resilience", "decide", "--repo", "owner/repo"]);

    assert_eq!(output.status.code(), Some(20));
    assert_eq!(stdout(&output), "{\"decision\":\"held\"}\n");
    assert!(!underscore.canonical_state_path().exists());

    let hyphen = ResilienceFixture::new();
    hyphen.write_state(
        "owner-repo",
        state_with_lock("owner/repo", "running", 1, 1, Some("different-host")),
    );

    let output = hyphen.run(&["resilience", "decide", "--repo", "owner/repo"]);

    assert_eq!(output.status.code(), Some(20));
    assert_eq!(stdout(&output), "{\"decision\":\"held\"}\n");
    assert!(!hyphen.canonical_state_path().exists());
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
fn state_read_io_is_diagnostic_not_malformed_reject() {
    let fixture = ResilienceFixture::new();
    fs::create_dir_all(fixture.canonical_state_path()).expect("create state directory");

    let output = fixture.run(&["resilience", "decide", "--repo", "owner/repo"]);

    assert_eq!(output.status.code(), Some(2));
    assert!(stdout(&output).is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("resilience state"));
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
fn resilience_decide_reads_the_legacy_spend_root_despite_a_state_root_override() {
    let fixture = ResilienceFixture::new();
    fixture.write_default_spend("owner__repo", 10, 0);

    let output = fixture.run_without_spend_override(&[
        "resilience",
        "decide",
        "--repo",
        "owner/repo",
        "--budget-tokens",
        "10",
        "--budget-issues",
        "1",
    ]);

    assert_eq!(output.status.code(), Some(20));
    assert_eq!(
        stdout(&output),
        "{\"decision\":\"park\",\"reason\":\"usage_cap\"}\n"
    );
}

#[test]
fn resilience_decide_uses_legacy_lifetime_caps_when_environment_is_unset() {
    let usage = ResilienceFixture::new();
    usage.write_spend("owner__repo", 10_000_000, 0);

    let output = usage.run(&["resilience", "decide", "--repo", "owner/repo"]);

    assert_eq!(output.status.code(), Some(20));
    assert_eq!(
        stdout(&output),
        "{\"decision\":\"park\",\"reason\":\"usage_cap\"}\n"
    );

    let issues = ResilienceFixture::new();
    issues.write_spend("owner__repo", 0, 500);

    let output = issues.run(&["resilience", "decide", "--repo", "owner/repo"]);

    assert_eq!(output.status.code(), Some(20));
    assert_eq!(
        stdout(&output),
        "{\"decision\":\"park\",\"reason\":\"issue_cap\"}\n"
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
fn autonomous_start_rejects_unsafe_resilience_state_before_operator_writes() {
    for (name, state, reason) in [
        (
            "foreign",
            valid_state("other/repo", "running", 1),
            "foreign_state",
        ),
        ("malformed", "{not-json}".to_string(), "malformed_state"),
    ] {
        let fixture = ResilienceFixture::new();
        fixture.write_state("owner__repo", state);

        let output = fixture.run_autonomous(&["start", "--repo", "owner/repo"]);

        assert_eq!(output.status.code(), Some(3), "{name}");
        assert_eq!(
            stdout(&output),
            format!("{{\"decision\":\"reject\",\"reason\":\"{reason}\"}}\n"),
            "{name}"
        );
        assert!(
            !fixture.operator_lifecycle_path().exists(),
            "{name} must not create an operator lifecycle record"
        );
    }
}

#[test]
fn autonomous_start_prefers_a_persisted_stop_over_foreign_resilience_state() {
    let fixture = ResilienceFixture::new();
    fixture.write_state("owner__repo", valid_state("other/repo", "running", 1));
    write_file(
        &fixture.operator_stop_flag_path(),
        "immediate\noperator@test\n",
    );

    let output = fixture.run_autonomous(&["start", "--repo", "owner/repo"]);

    assert_eq!(output.status.code(), Some(20));
    assert_eq!(
        stdout(&output),
        "{\"decision\":\"stop\",\"mode\":\"immediate\"}\n"
    );
    assert!(!fixture.operator_lifecycle_path().exists());
}

#[test]
fn autonomous_restart_rejects_unsafe_resilience_state_without_clearing_stop() {
    let fixture = ResilienceFixture::new();
    fixture.write_state("owner__repo", valid_state("other/repo", "running", 1));
    let stop_flag = fixture.operator_stop_flag_path();
    write_file(&stop_flag, "graceful\noperator@test\n");
    let before = fs::read_to_string(&stop_flag).expect("read stop flag before restart");

    let output = fixture.run_autonomous(&["restart", "--repo", "owner/repo"]);

    assert_eq!(output.status.code(), Some(3));
    assert_eq!(
        stdout(&output),
        "{\"decision\":\"reject\",\"reason\":\"foreign_state\"}\n"
    );
    assert_eq!(
        fs::read_to_string(&stop_flag).expect("read stop flag after restart"),
        before
    );
    assert!(!fixture.operator_lifecycle_path().exists());
}

#[test]
fn autonomous_foreground_does_not_persist_when_final_selection_hits_failure_cap() {
    let fixture = ResilienceFixture::new();
    fixture.write_failures("owner__repo", 43, 3);

    let output = fixture.run_foreground_with_changed_selection();

    assert_eq!(
        output.status.code(),
        Some(3),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        stdout(&output),
        "{\"decision\":\"reject\",\"reason\":\"failure_cap\"}\n"
    );
    assert!(!fixture.operator_lifecycle_path().exists());
    assert!(!fixture.foreground_state_path().exists());
}

#[test]
fn autonomous_foreground_rejects_failure_cap_before_dispatch() {
    let fixture = ResilienceFixture::new();
    fixture.write_failures("owner__repo", 42, 3);

    let output =
        fixture.run_autonomous(&["run-foreground", "--repo", "owner/repo", "--issue", "42"]);

    assert_eq!(output.status.code(), Some(3));
    assert_eq!(
        stdout(&output),
        "{\"decision\":\"reject\",\"reason\":\"failure_cap\"}\n"
    );
    assert!(!fixture.operator_lifecycle_path().exists());
    assert!(!fixture.foreground_state_path().exists());
}

#[test]
fn autonomous_foreground_parks_capacity_before_dispatch() {
    let fixture = ResilienceFixture::new();
    fixture.write_spend("owner__repo", 10, 0);

    let output = fixture.run_autonomous(&[
        "run-foreground",
        "--repo",
        "owner/repo",
        "--budget-tokens",
        "10",
        "--budget-issues",
        "1",
    ]);

    assert_eq!(output.status.code(), Some(20));
    assert_eq!(
        stdout(&output),
        "{\"decision\":\"park\",\"reason\":\"budget_hard_cap\"}\n"
    );
    assert!(!fixture.operator_lifecycle_path().exists());
    assert!(!fixture.foreground_state_path().exists());
}

#[test]
fn autonomous_start_parks_a_held_resilience_lease_without_operator_writes() {
    let fixture = ResilienceFixture::new();
    fixture.write_state(
        "owner__repo",
        state_with_lock("owner/repo", "running", 1, 1, Some("different-host")),
    );

    let output = fixture.run_autonomous(&["start", "--repo", "owner/repo"]);

    assert_eq!(output.status.code(), Some(20));
    assert_eq!(
        stdout(&output),
        "{\"decision\":\"park\",\"reason\":\"conductor_lease_held\"}\n"
    );
    assert!(!fixture.operator_lifecycle_path().exists());
}

#[test]
fn autonomous_status_reads_legacy_cycle_suffix_without_writing() {
    let fixture = ResilienceFixture::new();
    fixture.write_state(
        "owner_repo",
        valid_state("owner/repo", "running:cycle-9", 1),
    );

    let output = fixture.run_autonomous(&["status", "--repo", "owner/repo", "--json"]);

    assert!(output.status.success());
    let body = stdout(&output);
    assert!(body.contains("\"state_status\":\"running:cycle-9\""));
    assert!(body.contains("\"last_cycle\":\"cycle-9\""));
    assert!(!fixture.canonical_state_path().exists());
    assert!(!fixture.operator_lifecycle_path().exists());
}

#[test]
fn autonomous_status_ignores_non_running_cycle_suffix() {
    let fixture = ResilienceFixture::new();
    fixture.write_state("owner_repo", valid_state("owner/repo", "paused:cycle-9", 1));

    let output = fixture.run_autonomous(&["status", "--repo", "owner/repo", "--json"]);

    assert!(output.status.success());
    assert!(stdout(&output).contains("\"last_cycle\":\"\""));
}

#[test]
fn autonomous_status_prefers_an_explicit_cycle_over_running_suffix() {
    let fixture = ResilienceFixture::new();
    fixture.write_state(
        "owner_repo",
        format!(
            "{{\"repo\":\"owner/repo\",\"status\":\"running:cycle-9\",\"heartbeat_at\":{},\"cycle\":44}}",
            now_secs()
        ),
    );

    let output = fixture.run_autonomous(&["status", "--repo", "owner/repo", "--json"]);

    assert!(output.status.success());
    assert!(stdout(&output).contains("\"last_cycle\":\"44\""));
}

#[test]
fn status_requires_matching_record_repo() {
    for (name, state, reason) in [
        (
            "missing",
            format!("{{\"status\":\"running\",\"heartbeat_at\":{}}}", now_secs()),
            "malformed_state",
        ),
        (
            "foreign",
            valid_state("other/repo", "running", 1),
            "foreign_state",
        ),
    ] {
        let fixture = ResilienceFixture::new();
        fixture.write_state("owner_repo", state);

        let output = fixture.run_autonomous(&["status", "--repo", "owner/repo", "--json"]);

        assert_eq!(output.status.code(), Some(3), "{name}");
        assert_eq!(
            stdout(&output),
            format!("{{\"decision\":\"reject\",\"reason\":\"{reason}\"}}\n"),
            "{name}"
        );
        assert!(
            !fixture.canonical_state_path().exists(),
            "{name} must not migrate state"
        );
        assert!(
            !fixture.operator_lifecycle_path().exists(),
            "{name} must not write operator state"
        );
    }
}

#[test]
fn empty_supplied_lifetime_budget_is_diagnostic() {
    for flag in ["--budget-tokens", "--budget-issues"] {
        for value in ["", "not-a-number", "-1"] {
            let fixture = ResilienceFixture::new();

            let output = fixture.run_autonomous(&[
                "start",
                "--repo",
                "owner/repo",
                flag,
                value,
                "--dry-run",
            ]);

            assert_eq!(output.status.code(), Some(2), "{flag}={value:?}");
            assert!(stdout(&output).is_empty(), "{flag}={value:?}");
            assert!(
                String::from_utf8_lossy(&output.stderr).contains(flag),
                "{flag}={value:?} must identify the supplied flag"
            );
            assert!(
                !fixture.canonical_state_path().exists(),
                "{flag}={value:?} must not create state"
            );
            assert!(
                !fixture.operator_lifecycle_path().exists(),
                "{flag}={value:?} must not write operator state"
            );
        }
    }
}

#[test]
fn explicit_zero_lifetime_budget_remains_valid() {
    for flag in ["--budget-tokens", "--budget-issues"] {
        let fixture = ResilienceFixture::new();

        let output =
            fixture.run_autonomous(&["start", "--repo", "owner/repo", flag, "0", "--dry-run"]);

        assert!(output.status.success(), "{flag}");
        assert!(stdout(&output).contains("dry-run"), "{flag}");
        assert!(
            !fixture.canonical_state_path().exists(),
            "{flag} must not create state"
        );
        assert!(
            !fixture.operator_lifecycle_path().exists(),
            "{flag} must not write operator state"
        );
    }
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
    operator_root: PathBuf,
    repo_dir: PathBuf,
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
        let fixture = Self {
            state_root: root.join("state"),
            spend_root: root.join("spend"),
            operator_root: root.join("operator"),
            repo_dir: root.join("repo"),
            root,
        };
        fixture.initialize_git_remote();
        fixture
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

    fn run_without_spend_override(&self, args: &[&str]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_autospec"));
        command
            .arg("autonomous")
            .env("AUTOSPEC_STATE_DIR", &self.state_root)
            .env("AUTOSPEC_HOST", "autospec-test-host")
            .env("HOME", &self.root)
            .args(args)
            .output()
            .expect("run resilience decision without a spend override")
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
            .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &self.operator_root)
            .env("HOME", &self.root);
        command
    }

    fn run_autonomous(&self, args: &[&str]) -> Output {
        self.command()
            .args(args)
            .arg("--repo-dir")
            .arg(&self.repo_dir)
            .env("AUTOSPEC_HOST", "autospec-test-host")
            .output()
            .expect("run autonomous command")
    }

    fn run_foreground_with_changed_selection(&self) -> Output {
        let bin = self.root.join("bin");
        let counter = self.root.join("ready-count");
        fs::create_dir_all(&bin).expect("create fake bin");
        fs::write(&counter, "0\n").expect("write ready counter");
        write_executable(
            &bin.join("gh"),
            r####"#!/bin/sh
set -eu

endpoint=""
for value in "$@"; do
  case "$value" in repos/*) endpoint="$value" ;; esac
done

issue() {
  printf '%s\n' "{\"number\":$1,\"title\":\"Safe issue\",\"body\":\"## Safety review\\n\\n<!-- autospec-safety:begin -->\\n- **decision:** \`SAFETY_PASS\`\\n<!-- autospec-safety:end -->\",\"labels\":[\"auto-implement\",\"safety:reviewed\"],\"author\":{\"login\":\"berlinguyinca\"}}"
}

if [ "$1" = api ] && [ "$2" = graphql ]; then
  printf '%s\n' '{"items":[],"page_info":{"has_next_page":false,"end_cursor":null}}'
  exit 0
fi

case "$endpoint" in
  repos/owner/repo/branches/main)
    printf '%s\n' '{}'
    ;;
  repos/owner/repo/commits/main/status)
    printf '%s\n' '{"state":"success","total_count":1,"statuses":[{"context":"ci","state":"success"}]}'
    ;;
  repos/owner/repo/issues/*/comments?*)
    printf '%s\n' '{"raw_count":0,"items":[]}'
    ;;
  repos/owner/repo/issues/*/comments)
    printf '%s\n' '[]'
    ;;
  *labels=auto-implement*)
    if [ "$(cat "$AUTOSPEC_RESILIENCE_READY_COUNTER")" = 0 ]; then
      printf '1\n' > "$AUTOSPEC_RESILIENCE_READY_COUNTER"
      issue 42
    else
      issue 43
    fi | {
      printf '%s' '{"raw_count":1,"items":['
      cat
      printf '%s\n' ']}'
    }
    ;;
  *labels=in-progress-by-bot*)
    printf '%s\n' '{"raw_count":0,"items":[]}'
    ;;
  '')
    printf 'unexpected gh invocation: %s\n' "$*" >&2
    exit 1
    ;;
  *)
    printf 'unexpected gh endpoint: %s\n' "$endpoint" >&2
    exit 1
    ;;
esac
"####,
        );
        self.command()
            .args([
                "run-foreground",
                "--repo",
                "owner/repo",
                "--branch",
                "main",
                "--repo-dir",
            ])
            .arg(&self.repo_dir)
            .env("AUTOSPEC_HOST", "autospec-test-host")
            .env("AUTOSPEC_RESILIENCE_READY_COUNTER", counter)
            .env("PATH", path_with(&bin))
            .output()
            .expect("run foreground with changing selection")
    }

    fn initialize_git_remote(&self) {
        let init = Command::new("git")
            .args([
                "init",
                "-q",
                self.repo_dir.to_str().expect("repo directory"),
            ])
            .output()
            .expect("initialize fixture repository");
        assert!(init.status.success());
        let remote = Command::new("git")
            .args([
                "-C",
                self.repo_dir.to_str().expect("repo directory"),
                "remote",
                "add",
                "origin",
                "https://github.com/owner/repo.git",
            ])
            .output()
            .expect("set fixture remote");
        assert!(remote.status.success());
    }

    fn canonical_state_path(&self) -> PathBuf {
        self.state_path("owner__repo")
    }

    fn operator_lifecycle_path(&self) -> PathBuf {
        self.operator_root.join("owner_repo/lifecycle.json")
    }

    fn operator_stop_flag_path(&self) -> PathBuf {
        self.operator_root.join("owner_repo/stop.flag")
    }

    fn foreground_state_path(&self) -> PathBuf {
        self.operator_root
            .join("owner_repo/foreground-conductor-repository.json")
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
        self.write_spend_at(&self.spend_root, slug, tokens, issues);
    }

    fn write_default_spend(&self, slug: &str, tokens: u64, issues: u64) {
        self.write_spend_at(
            &self.root.join(".autospec/autonomous-spend"),
            slug,
            tokens,
            issues,
        );
    }

    fn write_spend_at(&self, root: &Path, slug: &str, tokens: u64, issues: u64) {
        write_file(
            &root.join(slug).join("spend.json"),
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

fn write_executable(path: &Path, contents: &str) {
    fs::write(path, contents).expect("write fake executable");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("make fake executable");
}

fn path_with(bin: &Path) -> String {
    format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").expect("PATH is set")
    )
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
