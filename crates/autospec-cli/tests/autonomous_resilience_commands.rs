use autospec_core::coordination::{ConductorEvent, ConductorPhase, ConductorScope, ConductorState};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

#[path = "support/resilience_fixture_support.rs"]
mod resilience_fixture_support;
use resilience_fixture_support::{
    dead_child_pid, fixture_root, git_fixture, now_secs, path_with, seed_bound_accountability,
    stdout, wait_for_path, write_executable, write_file, ACCOUNTABILITY_ONLY_GH,
};

static FIXTURE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[test]
fn resilience_fixture_git_remote_has_a_real_main() {
    let fixture = ResilienceFixture::new();
    let local = git_fixture(&fixture.repo_dir, &["rev-parse", "refs/heads/main"]);
    let remote = git_fixture(
        &fixture.repo_dir,
        &["ls-remote", "--heads", "origin", "main"],
    );

    assert!(remote.starts_with(&local), "local={local} remote={remote}");
}

/// Returns the first non-empty line following a `LABEL:` header in `--help`
/// output. Reusing the labeled-block parsing pattern from
/// `cli_commands.rs::help_usage_invocation` (which reads the line after
/// `USAGE:` rather than substring-searching the whole blob) avoids false
/// matches between the intended structured field and incidental text
/// elsewhere in the help output.
fn help_section_first_line<'a>(help: &'a str, label: &str) -> Option<&'a str> {
    help.lines()
        .skip_while(|line| line.trim() != label)
        .skip(1)
        .map(str::trim)
        .find(|line| !line.is_empty())
}

#[test]
fn resilience_help_names_the_canonical_write_slug() {
    let output = Command::new(env!("CARGO_BIN_EXE_autospec"))
        .args(["autonomous", "resilience", "--help"])
        .output()
        .expect("run help");
    assert!(output.status.success());
    let help = String::from_utf8_lossy(&output.stdout);

    assert_eq!(
        help_section_first_line(&help, "USAGE:"),
        Some(
            "autospec autonomous resilience decide --repo OWNER/REPO [--issue N] [--budget-tokens N] [--budget-issues N]"
        ),
        "USAGE section did not name the `resilience decide` invocation verbatim"
    );

    let writes_line = help
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("Writes resilience state only to the canonical "));
    assert_eq!(
        writes_line,
        Some("Writes resilience state only to the canonical owner__repo layout."),
        "STATE section did not name the canonical owner__repo write slug verbatim"
    );
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
    assert_eq!(
        stdout(&output),
        "{\"decision\":\"held\",\"spend\":{\"tokens\":0,\"issues\":0,\"filed_issues\":0,\"budget_issues\":0}}\n"
    );
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
    assert_eq!(
        stdout(&output),
        "{\"decision\":\"held\",\"spend\":{\"tokens\":0,\"issues\":0,\"filed_issues\":0,\"budget_issues\":0}}\n"
    );
    assert!(!underscore.canonical_state_path().exists());

    let hyphen = ResilienceFixture::new();
    hyphen.write_state(
        "owner-repo",
        state_with_lock("owner/repo", "running", 1, 1, Some("different-host")),
    );

    let output = hyphen.run(&["resilience", "decide", "--repo", "owner/repo"]);

    assert_eq!(output.status.code(), Some(20));
    assert_eq!(
        stdout(&output),
        "{\"decision\":\"held\",\"spend\":{\"tokens\":0,\"issues\":0,\"filed_issues\":0,\"budget_issues\":0}}\n"
    );
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
fn resilience_decide_rejects_malformed_token_bearing_state_without_writing() {
    for (name, state) in [
        (
            "claimed-without-lock-pid",
            token_state(
                "claimed",
                Some(now_secs()),
                None,
                Some("autospec-test-host"),
                Some(now_secs()),
                Some("lease-1"),
                Some(1),
            ),
        ),
        (
            "running-without-lock-host",
            token_state(
                "running",
                Some(now_secs()),
                Some(1),
                None,
                Some(now_secs()),
                Some("lease-1"),
                Some(1),
            ),
        ),
        (
            "claimed-without-lock-acquired-at",
            token_state(
                "claimed",
                Some(now_secs()),
                Some(1),
                Some("autospec-test-host"),
                None,
                Some("lease-1"),
                Some(1),
            ),
        ),
        (
            "released-with-active-token",
            token_state(
                "released",
                Some(now_secs()),
                None,
                None,
                None,
                Some("lease-1"),
                Some(1),
            ),
        ),
        (
            "released-with-active-lock",
            token_state(
                "released",
                Some(now_secs()),
                Some(1),
                Some("autospec-test-host"),
                Some(now_secs()),
                None,
                Some(1),
            ),
        ),
        (
            "running-without-heartbeat",
            token_state(
                "running",
                None,
                Some(1),
                Some("autospec-test-host"),
                Some(now_secs()),
                Some("lease-1"),
                Some(1),
            ),
        ),
    ] {
        let fixture = ResilienceFixture::new();
        fixture.write_state("owner_repo", &state);
        let source_path = fixture.state_path("owner_repo");

        let output = fixture.run(&["resilience", "decide", "--repo", "owner/repo"]);

        assert_eq!(output.status.code(), Some(3), "{name}");
        assert_eq!(
            stdout(&output),
            "{\"decision\":\"reject\",\"reason\":\"malformed_state\"}\n",
            "{name}"
        );
        assert_eq!(
            fs::read_to_string(source_path).expect("read source state"),
            state,
            "{name}"
        );
        assert!(
            !fixture.canonical_state_path().exists(),
            "{name} must not write canonical state"
        );
    }
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
        "{\"decision\":\"reclaim\",\"reason\":\"claimed_expired\",\"spend\":{\"tokens\":0,\"issues\":0,\"filed_issues\":0,\"budget_issues\":0}}\n"
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
        "{\"decision\":\"reclaim\",\"reason\":\"abandoned\",\"spend\":{\"tokens\":0,\"issues\":0,\"filed_issues\":0,\"budget_issues\":0}}\n"
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
        "{\"decision\":\"reject\",\"reason\":\"failure_cap\",\"spend\":{\"tokens\":0,\"issues\":0,\"filed_issues\":0,\"budget_issues\":0}}\n"
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
        "{\"decision\":\"park\",\"reason\":\"usage_cap\",\"spend\":{\"tokens\":10,\"issues\":4,\"filed_issues\":0,\"budget_issues\":4}}\n"
    );
}

#[test]
fn resilience_decide_reports_filed_and_budget_issue_counters_separately() {
    let fixture = ResilienceFixture::new();
    fixture.write_spend_counters("owner__repo", 0, 1, 1);

    let first = fixture.run(&[
        "resilience",
        "decide",
        "--repo",
        "owner/repo",
        "--budget-issues",
        "2",
    ]);

    assert!(first.status.success());
    assert_eq!(
        stdout(&first),
        "{\"decision\":\"available\",\"spend\":{\"tokens\":0,\"issues\":1,\"filed_issues\":1,\"budget_issues\":1}}\n"
    );

    fixture.write_spend_counters("owner__repo", 0, 5, 2);

    let second = fixture.run(&[
        "resilience",
        "decide",
        "--repo",
        "owner/repo",
        "--budget-issues",
        "3",
    ]);

    assert!(
        second.status.success(),
        "the budget cap must use budget_issues=2, not filed_issues=5"
    );
    assert_eq!(
        stdout(&second),
        "{\"decision\":\"available\",\"spend\":{\"tokens\":0,\"issues\":2,\"filed_issues\":5,\"budget_issues\":2}}\n"
    );

    let status = fixture.run(&["status", "--repo", "owner/repo", "--json"]);
    let status_stdout = stdout(&status);

    assert!(status.status.success());
    assert!(
        status_stdout.contains(
            "\"spend\":{\"tokens\":0,\"issues\":2,\"filed_issues\":5,\"budget_issues\":2}"
        ),
        "status JSON must expose both issue counters by distinct names: {status_stdout}"
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
        "{\"decision\":\"park\",\"reason\":\"usage_cap\",\"spend\":{\"tokens\":10,\"issues\":0,\"filed_issues\":0,\"budget_issues\":0}}\n"
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
        "{\"decision\":\"park\",\"reason\":\"usage_cap\",\"spend\":{\"tokens\":10000000,\"issues\":0,\"filed_issues\":0,\"budget_issues\":0}}\n"
    );

    let issues = ResilienceFixture::new();
    issues.write_spend("owner__repo", 0, 500);

    let output = issues.run(&["resilience", "decide", "--repo", "owner/repo"]);

    assert_eq!(output.status.code(), Some(20));
    assert_eq!(
        stdout(&output),
        "{\"decision\":\"park\",\"reason\":\"issue_cap\",\"spend\":{\"tokens\":0,\"issues\":500,\"filed_issues\":0,\"budget_issues\":500}}\n"
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
        "{\"decision\":\"reject\",\"reason\":\"failure_cap\",\"spend\":{\"tokens\":0,\"issues\":0,\"filed_issues\":0,\"budget_issues\":0}}\n"
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
    let dead_pid = dead_child_pid();
    same_host.write_state(
        "owner__repo",
        state_with_lock(
            "owner/repo",
            "running",
            1,
            dead_pid,
            Some("autospec-test-host"),
        ),
    );

    let output = same_host.run(&["resilience", "decide", "--repo", "owner/repo"]);

    assert!(output.status.success());
    assert_eq!(
        stdout(&output),
        "{\"decision\":\"reclaim\",\"reason\":\"dead_same_host_pid\",\"spend\":{\"tokens\":0,\"issues\":0,\"filed_issues\":0,\"budget_issues\":0}}\n"
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
    assert_eq!(
        stdout(&output),
        "{\"decision\":\"held\",\"spend\":{\"tokens\":0,\"issues\":0,\"filed_issues\":0,\"budget_issues\":0}}\n"
    );
}

#[test]
fn resilience_decide_treats_legacy_released_lock_as_available() {
    let fixture = ResilienceFixture::new();
    fixture.write_state("owner__repo", valid_state("owner/repo", "running", 1));

    let output = fixture.run(&["resilience", "decide", "--repo", "owner/repo"]);

    assert!(output.status.success());
    assert_eq!(
        stdout(&output),
        "{\"decision\":\"available\",\"spend\":{\"tokens\":0,\"issues\":0,\"filed_issues\":0,\"budget_issues\":0}}\n"
    );
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
fn autonomous_foreground_releases_an_adopted_lease_when_a_stop_is_persisted() {
    let fixture = ResilienceFixture::new();
    fixture.write_state(
        "owner__repo",
        token_state(
            "claimed",
            Some(now_secs()),
            Some(42),
            Some("autospec-test-host"),
            Some(now_secs()),
            Some("parent-lease-token"),
            Some(1),
        ),
    );
    fixture.seed_bound_accountability(1);
    write_file(
        &fixture.operator_stop_flag_path(),
        "graceful\noperator@test\n",
    );

    let output = fixture
        .command()
        .args(["run-foreground", "--repo", "owner/repo"])
        .env("AUTOSPEC_CONDUCTOR_LEASE_TOKEN", "parent-lease-token")
        .output()
        .expect("run stopped foreground child");

    assert_eq!(output.status.code(), Some(20));
    assert_eq!(
        stdout(&output),
        "{\"decision\":\"stop\",\"mode\":\"graceful\"}\n"
    );
    assert_eq!(
        fs::read_to_string(fixture.operator_lifecycle_path()).expect("read terminal lifecycle"),
        "{\"version\":1,\"repo\":\"owner/repo\",\"result\":{\"decision\":\"stop\",\"mode\":\"graceful\"}}\n"
    );
    assert!(
        fs::read_to_string(fixture.canonical_state_path())
            .expect("read released conductor lease")
            .contains("\"status\":\"released\""),
        "a stopped child must release its inherited lease token"
    );
}

#[test]
fn autonomous_foreground_releases_an_adopted_lease_when_the_stop_record_is_invalid() {
    let fixture = ResilienceFixture::new();
    fixture.write_state(
        "owner__repo",
        token_state(
            "claimed",
            Some(now_secs()),
            Some(42),
            Some("autospec-test-host"),
            Some(now_secs()),
            Some("parent-lease-token"),
            Some(1),
        ),
    );
    fixture.seed_bound_accountability(1);
    write_file(
        &fixture.operator_stop_flag_path(),
        "unknown\noperator@test\n",
    );

    let output = fixture
        .command()
        .args(["run-foreground", "--repo", "owner/repo"])
        .env("AUTOSPEC_CONDUCTOR_LEASE_TOKEN", "parent-lease-token")
        .output()
        .expect("run foreground child with invalid stop record");

    assert_eq!(output.status.code(), Some(2));
    assert!(stdout(&output).is_empty());
    assert!(
        fs::read_to_string(fixture.canonical_state_path())
            .expect("read released conductor lease")
            .contains("\"status\":\"released\""),
        "a stop-record diagnostic must release the inherited lease token"
    );
}

#[test]
fn autonomous_foreground_releases_an_inherited_lease_when_config_turns_invalid() {
    let fixture = ResilienceFixture::new();
    fixture.write_state(
        "owner__repo",
        token_state(
            "claimed",
            Some(now_secs()),
            Some(42),
            Some("autospec-test-host"),
            Some(now_secs()),
            Some("parent-lease-token"),
            Some(1),
        ),
    );
    fixture.seed_bound_accountability(1);
    let config_dir = fixture.repo_dir.join(".autospec");
    fs::create_dir_all(&config_dir).expect("create config directory");
    fs::write(
        config_dir.join("autonomous.yml"),
        "main_health:\n  ignore_checks: Unit Tests\n",
    )
    .expect("write malformed autonomous config");

    let output = fixture
        .command()
        .args(["run-foreground", "--repo", "owner/repo", "--repo-dir"])
        .arg(&fixture.repo_dir)
        .env("AUTOSPEC_CONDUCTOR_LEASE_TOKEN", "parent-lease-token")
        .output()
        .expect("run foreground child with malformed config after parent preflight");

    assert_eq!(output.status.code(), Some(2));
    assert!(stdout(&output).is_empty());
    assert!(
        fs::read_to_string(fixture.canonical_state_path())
            .expect("read released conductor lease")
            .contains("\"status\":\"released\""),
        "a config diagnostic must release the inherited lease token"
    );
}

#[test]
fn autonomous_foreground_releases_an_adopted_lease_after_admission_diagnostic() {
    let fixture = ResilienceFixture::new();
    fixture.write_state(
        "owner__repo",
        token_state(
            "claimed",
            Some(now_secs()),
            Some(42),
            Some("autospec-test-host"),
            Some(now_secs()),
            Some("parent-lease-token"),
            Some(1),
        ),
    );
    fixture.seed_bound_accountability(1);
    fs::create_dir_all(
        fixture
            .state_root
            .join("autonomous/owner__repo/issues/42.json"),
    )
    .expect("create unreadable failure record path");

    let output = fixture
        .command()
        .args(["run-foreground", "--repo", "owner/repo", "--issue", "42"])
        .env("AUTOSPEC_CONDUCTOR_LEASE_TOKEN", "parent-lease-token")
        .output()
        .expect("run malformed-admission foreground child");

    assert_eq!(output.status.code(), Some(2));
    assert!(stdout(&output).is_empty());
    assert!(
        fs::read_to_string(fixture.canonical_state_path())
            .expect("read released conductor lease")
            .contains("\"status\":\"released\""),
        "an admission diagnostic must release the inherited lease token"
    );
}

#[test]
fn autonomous_foreground_persists_an_inherited_lease_rejection_before_release() {
    let fixture = ResilienceFixture::new();
    fixture.write_state(
        "owner__repo",
        token_state(
            "claimed",
            Some(now_secs()),
            Some(42),
            Some("autospec-test-host"),
            Some(now_secs()),
            Some("parent-lease-token"),
            Some(1),
        ),
    );
    fixture.seed_bound_accountability(1);
    fixture.write_failures("owner__repo", 42, 3);

    let output = fixture
        .command()
        .args(["run-foreground", "--repo", "owner/repo", "--issue", "42"])
        .env("AUTOSPEC_CONDUCTOR_LEASE_TOKEN", "parent-lease-token")
        .output()
        .expect("run inherited failure-capped foreground child");

    assert_eq!(output.status.code(), Some(3));
    assert_eq!(
        stdout(&output),
        "{\"decision\":\"reject\",\"reason\":\"failure_cap\"}\n"
    );
    assert_eq!(
        fs::read_to_string(fixture.operator_lifecycle_path()).expect("read terminal lifecycle"),
        "{\"version\":1,\"repo\":\"owner/repo\",\"result\":{\"decision\":\"reject\",\"reason\":\"failure_cap\"}}\n"
    );
    assert!(
        fs::read_to_string(fixture.canonical_state_path())
            .expect("read released conductor lease")
            .contains("\"status\":\"released\""),
        "the inherited lease must release after terminal lifecycle persistence"
    );
}

#[test]
fn autonomous_foreground_release_diagnostic_emits_no_decision_json() {
    let fixture = ResilienceFixture::new();
    fixture.write_state(
        "owner__repo",
        token_state(
            "claimed",
            Some(now_secs()),
            Some(42),
            Some("autospec-test-host"),
            Some(now_secs()),
            Some("parent-lease-token"),
            Some(1),
        ),
    );
    fixture.seed_bound_accountability(1);
    let stop_flag = fixture.operator_stop_flag_path();
    fs::create_dir_all(stop_flag.parent().expect("stop flag parent"))
        .expect("create stop flag parent");
    assert!(
        Command::new("mkfifo")
            .arg(&stop_flag)
            .status()
            .expect("create stop fifo")
            .success(),
        "create stop fifo"
    );
    let state_dir = fixture
        .canonical_state_path()
        .parent()
        .expect("canonical state parent")
        .to_path_buf();
    let child = fixture
        .command()
        .args(["run-foreground", "--repo", "owner/repo"])
        .env("AUTOSPEC_CONDUCTOR_LEASE_TOKEN", "parent-lease-token")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start foreground child with release failure");
    let adopted_pid = child.id();
    for _ in 0..80 {
        if fs::read_to_string(fixture.canonical_state_path())
            .map(|state| state.contains(&format!("\"lock_pid\":{adopted_pid}")))
            .unwrap_or(false)
        {
            break;
        }
        thread::sleep(Duration::from_millis(25));
    }
    assert!(
        fs::read_to_string(fixture.canonical_state_path())
            .expect("read adopted conductor lease")
            .contains(&format!("\"lock_pid\":{adopted_pid}")),
        "the child must adopt before its stop read blocks"
    );
    fs::set_permissions(&state_dir, fs::Permissions::from_mode(0o555))
        .expect("make state directory read-only");

    fs::write(&stop_flag, "graceful\noperator@test\n").expect("unblock stop read");
    let output = child
        .wait_with_output()
        .expect("wait for foreground child with release failure");

    fs::set_permissions(&state_dir, fs::Permissions::from_mode(0o755))
        .expect("restore state directory permissions");
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stdout(&output).is_empty(),
        "a release diagnostic must not follow a printed decision"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("cannot create"),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn autonomous_foreground_token_replacement_during_release_emits_no_decision_json() {
    let fixture = ResilienceFixture::new();
    fixture.write_state(
        "owner__repo",
        token_state(
            "claimed",
            Some(now_secs()),
            Some(42),
            Some("autospec-test-host"),
            Some(now_secs()),
            Some("parent-lease-token"),
            Some(1),
        ),
    );
    fixture.seed_bound_accountability(1);
    let stop_flag = fixture.operator_stop_flag_path();
    fs::create_dir_all(stop_flag.parent().expect("stop flag parent"))
        .expect("create stop flag parent");
    assert!(
        Command::new("mkfifo")
            .arg(&stop_flag)
            .status()
            .expect("create stop fifo")
            .success(),
        "create stop fifo"
    );
    let child = fixture
        .command()
        .args(["run-foreground", "--repo", "owner/repo"])
        .env("AUTOSPEC_CONDUCTOR_LEASE_TOKEN", "parent-lease-token")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start foreground child with a replaceable lease");
    let adopted_pid = child.id();
    for _ in 0..80 {
        if fs::read_to_string(fixture.canonical_state_path())
            .map(|state| state.contains(&format!("\"lock_pid\":{adopted_pid}")))
            .unwrap_or(false)
        {
            break;
        }
        thread::sleep(Duration::from_millis(25));
    }
    assert!(
        fs::read_to_string(fixture.canonical_state_path())
            .expect("read adopted conductor lease")
            .contains(&format!("\"lock_pid\":{adopted_pid}")),
        "the child must adopt before its stop read blocks"
    );
    let replacement = token_state(
        "claimed",
        Some(now_secs()),
        Some(1),
        Some("autospec-test-host"),
        Some(now_secs()),
        Some("replacement-owner-token"),
        Some(2),
    );
    fixture.write_state("owner__repo", &replacement);

    fs::write(&stop_flag, "graceful\noperator@test\n").expect("unblock stop read");
    let output = child
        .wait_with_output()
        .expect("wait for foreground child with replaced lease");

    assert_eq!(output.status.code(), Some(2));
    assert!(
        stdout(&output).is_empty(),
        "a failed release must not render a token-mismatch decision"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("cannot release conductor lease"),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_to_string(fixture.canonical_state_path()).expect("read replacement lease"),
        replacement
    );
}

#[test]
fn autonomous_foreground_midrun_rejection_emits_no_decision_json_when_release_fails() {
    let fixture = ResilienceFixture::new();
    fixture.write_failures("owner__repo", 43, 3);
    let final_selection_ready = fixture.root.join("final-selection-ready");
    let state_dir = fixture
        .canonical_state_path()
        .parent()
        .expect("canonical state parent")
        .to_path_buf();
    let child = fixture
        .foreground_with_changed_selection_command()
        .env(
            "AUTOSPEC_RESILIENCE_FINAL_SELECTION_READY",
            &final_selection_ready,
        )
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start foreground child with mid-run release failure");
    assert!(
        wait_for_path(&final_selection_ready, Duration::from_secs(5)),
        "foreground command must acquire its lease before final selection waits"
    );
    fs::set_permissions(&state_dir, fs::Permissions::from_mode(0o555))
        .expect("make state directory read-only");
    let output = child
        .wait_with_output()
        .expect("wait for foreground child with release failure");

    fs::set_permissions(&state_dir, fs::Permissions::from_mode(0o755))
        .expect("restore state directory permissions");
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stdout(&output).is_empty(),
        "a failed release must suppress a mid-run terminal decision"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("cannot create"),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn concurrent_foreground_command_parks_while_the_first_command_owns_the_lease() {
    let fixture = ResilienceFixture::new();
    let bin = fixture.root.join("bin");
    let entered = fixture.root.join("first-foreground-entered");
    fs::create_dir_all(&bin).expect("create fake bin");
    write_executable(
        &bin.join("gh"),
        r#"#!/bin/sh
. "$AUTOSPEC_FOREGROUND_ACCOUNTABILITY_HANDLER"
touch "$AUTOSPEC_TEST_FOREGROUND_ENTERED"
sleep 2
exit 1
"#,
    );

    let mut first = fixture.command();
    let mut first = first
        .args(["run-foreground", "--repo", "owner/repo", "--repo-dir"])
        .arg(&fixture.repo_dir)
        .env("AUTOSPEC_HOST", "autospec-test-host")
        .env("AUTOSPEC_TEST_FOREGROUND_ENTERED", &entered)
        .env("PATH", path_with(&bin))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start first foreground command");
    assert!(
        wait_for_path(&entered, Duration::from_secs(10)),
        "first foreground command must hold the lease"
    );

    let second = fixture
        .command()
        .args(["run-foreground", "--repo", "owner/repo", "--repo-dir"])
        .arg(&fixture.repo_dir)
        .env("AUTOSPEC_HOST", "autospec-test-host")
        .env("PATH", path_with(&bin))
        .output()
        .expect("run competing foreground command");

    assert_eq!(second.status.code(), Some(20));
    assert_eq!(
        stdout(&second),
        "{\"decision\":\"park\",\"reason\":\"conductor_lease_held\"}\n"
    );
    assert!(!first.wait().expect("wait for first foreground").success());
    assert!(
        fs::read_to_string(fixture.canonical_state_path())
            .expect("read released conductor lease")
            .contains("\"status\":\"released\""),
        "the first command must release its lease after its terminal diagnostic"
    );
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
fn fresh_lease_blocks_restart_before_kill_or_stop_clear() {
    let fixture = ResilienceFixture::new();
    let mut conductor = Command::new("sleep")
        .arg("30")
        .spawn()
        .expect("start dummy conductor");
    let stop_flag = fixture.operator_stop_flag_path();
    let state = token_state(
        "claimed",
        Some(now_secs()),
        Some(1),
        Some("different-host"),
        Some(now_secs()),
        Some("fresh-owner-token"),
        Some(1),
    );
    fixture.write_state("owner__repo", state);
    write_file(
        &fixture.operator_root.join("owner_repo/conductor.pid"),
        &format!("{}\n", conductor.id()),
    );
    write_file(&stop_flag, "graceful\noperator@test\n");
    let stop_before = fs::read_to_string(&stop_flag).expect("read stop before restart");

    let output = fixture.run_autonomous(&["restart", "--repo", "owner/repo"]);
    let conductor_survived = conductor
        .try_wait()
        .expect("inspect dummy conductor")
        .is_none();
    let _ = conductor.kill();
    let _ = conductor.wait();

    assert_eq!(
        output.status.code(),
        Some(20),
        "stdout={} stderr={}",
        stdout(&output),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        stdout(&output),
        "{\"decision\":\"park\",\"reason\":\"conductor_lease_held\"}\n"
    );
    assert!(
        conductor_survived,
        "restart must not signal before it owns a lease"
    );
    assert_eq!(
        fs::read_to_string(&stop_flag).expect("read stop after restart"),
        stop_before,
        "restart must not clear a stop flag while the lease is held"
    );
    assert!(!fixture.operator_lifecycle_path().exists());
}

#[test]
fn restart_releases_new_lease_when_owned_process_termination_is_rejected() {
    let fixture = ResilienceFixture::new();
    let mut conductor = Command::new("sleep")
        .arg("30")
        .spawn()
        .expect("start unowned conductor");
    write_file(
        &fixture.operator_root.join("owner_repo/conductor.pid"),
        &format!("{}\n", conductor.id()),
    );

    let output = fixture.run_autonomous(&["restart", "--repo", "owner/repo"]);
    let conductor_survived = conductor
        .try_wait()
        .expect("inspect unowned conductor")
        .is_none();
    let _ = conductor.kill();
    let _ = conductor.wait();

    assert_eq!(
        output.status.code(),
        Some(2),
        "stdout={} stderr={}",
        stdout(&output),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("process group ownership is unverified")
    );
    assert!(conductor_survived, "restart must not signal an unowned PID");
    assert!(
        fs::read_to_string(fixture.canonical_state_path())
            .expect("read released restart lease")
            .contains("\"status\":\"released\""),
        "a post-acquisition termination error must release the exact new lease"
    );
}

#[test]
fn autonomous_start_passes_lease_token_only_through_the_child_environment() {
    let fixture = ResilienceFixture::new();
    let bin = fixture.root.join("bin");
    let token_capture = fixture.root.join("child-token");
    let log_root = fixture.root.join("logs");
    fs::create_dir_all(&bin).expect("create fake bin");
    write_executable(
        &bin.join("gh"),
        r#"#!/bin/sh
. "$AUTOSPEC_FOREGROUND_ACCOUNTABILITY_HANDLER"
printf '%s' "${AUTOSPEC_CONDUCTOR_LEASE_TOKEN:-}" > "$AUTOSPEC_TEST_CHILD_TOKEN"
exit 1
"#,
    );

    let output = fixture
        .command()
        .args(["start", "--repo", "owner/repo", "--repo-dir"])
        .arg(&fixture.repo_dir)
        .env("AUTOSPEC_HOST", "autospec-test-host")
        .env("AUTOSPEC_AUTONOMOUS_COMPANIONS", "0")
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_root)
        .env("AUTOSPEC_TEST_CHILD_TOKEN", &token_capture)
        .env("PATH", path_with(&bin))
        .output()
        .expect("start native foreground child");
    let conductor_logpath_file = fixture.operator_root.join("owner_repo/conductor.logpath");

    for _ in 0..80 {
        if token_capture.exists() && conductor_logpath_file.exists() {
            break;
        }
        thread::sleep(Duration::from_millis(25));
    }
    let token = fs::read_to_string(&token_capture).expect("capture child lease token");
    let launch = fs::read_to_string(fixture.operator_root.join("owner_repo/launch.json"))
        .expect("read launch metadata");
    let conductor_log_path =
        fs::read_to_string(conductor_logpath_file).expect("read foreground log path");
    let conductor_log = fs::read_to_string(conductor_log_path.trim()).expect("read foreground log");

    assert!(output.status.success());
    assert!(
        !token.is_empty(),
        "native foreground child must receive its lease token"
    );
    assert!(
        !launch.contains(&token),
        "lease token must not appear in launch metadata"
    );
    assert!(
        !conductor_log.contains(&token),
        "lease token must not appear in foreground logs"
    );
}

#[test]
fn malformed_restart_state_rejects_before_process_or_stop_mutation() {
    let fixture = ResilienceFixture::new();
    let mut conductor = Command::new("sleep")
        .arg("30")
        .spawn()
        .expect("start dummy conductor");
    let stop_flag = fixture.operator_stop_flag_path();
    fixture.write_state("owner__repo", "{not-json}");
    write_file(
        &fixture.operator_root.join("owner_repo/conductor.pid"),
        &format!("{}\n", conductor.id()),
    );
    write_file(&stop_flag, "graceful\noperator@test\n");
    let stop_before = fs::read_to_string(&stop_flag).expect("read stop before restart");

    let output = fixture.run_autonomous(&["restart", "--repo", "owner/repo"]);
    let conductor_survived = conductor
        .try_wait()
        .expect("inspect dummy conductor")
        .is_none();
    let _ = conductor.kill();
    let _ = conductor.wait();

    assert_eq!(
        output.status.code(),
        Some(3),
        "stdout={} stderr={}",
        stdout(&output),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        stdout(&output),
        "{\"decision\":\"reject\",\"reason\":\"malformed_state\"}\n"
    );
    assert!(
        conductor_survived,
        "restart must reject malformed resilience state before signaling units"
    );
    assert_eq!(
        fs::read_to_string(&stop_flag).expect("read stop after restart"),
        stop_before,
        "restart must not clear a stop flag after a malformed-state rejection"
    );
    assert!(!fixture.operator_lifecycle_path().exists());
}

#[test]
fn delayed_child_with_replaced_token_exits_before_foreground_mutation() {
    let fixture = ResilienceFixture::new();
    let bin = fixture.root.join("bin");
    let github_calls = fixture.root.join("github-calls");
    fs::create_dir_all(&bin).expect("create fake bin");
    write_executable(
        &bin.join("gh"),
        r#"#!/bin/sh
. "$AUTOSPEC_FOREGROUND_ACCOUNTABILITY_HANDLER"
printf 'called\n' > "$AUTOSPEC_TEST_GITHUB_CALLS"
exit 1
"#,
    );
    let replacement = token_state(
        "claimed",
        Some(now_secs()),
        Some(1),
        Some("autospec-test-host"),
        Some(now_secs()),
        Some("replacement-owner-token"),
        Some(2),
    );
    fixture.write_state("owner__repo", &replacement);

    let output = fixture
        .command()
        .args(["run-foreground", "--repo", "owner/repo", "--repo-dir"])
        .arg(&fixture.repo_dir)
        .env("AUTOSPEC_HOST", "autospec-test-host")
        .env("AUTOSPEC_CONDUCTOR_LEASE_TOKEN", "delayed-child-token")
        .env("AUTOSPEC_TEST_GITHUB_CALLS", &github_calls)
        .env("PATH", path_with(&bin))
        .output()
        .expect("run delayed foreground child");

    assert_eq!(output.status.code(), Some(3));
    assert_eq!(
        stdout(&output),
        "{\"decision\":\"reject\",\"reason\":\"conductor_lease_token_mismatch\"}\n"
    );
    assert_eq!(
        fs::read_to_string(fixture.canonical_state_path()).expect("read replacement lease"),
        replacement
    );
    assert!(
        !fixture.operator_lifecycle_path().exists(),
        "a stale child token must not write lifecycle state"
    );
    assert!(
        !fixture.foreground_state_path().exists(),
        "a stale child token must not write foreground state"
    );
    assert!(
        !github_calls.exists(),
        "a stale child token must not invoke the GitHub adapter"
    );
}

#[test]
fn autonomous_foreground_persists_terminal_lifecycle_before_releasing_lease_when_final_selection_hits_failure_cap(
) {
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
    assert_eq!(
        fs::read_to_string(fixture.operator_lifecycle_path()).expect("read terminal lifecycle"),
        "{\"version\":1,\"repo\":\"owner/repo\",\"result\":{\"decision\":\"reject\",\"reason\":\"failure_cap\"}}\n"
    );
    assert!(
        fs::read_to_string(fixture.canonical_state_path())
            .expect("read released conductor lease")
            .contains("\"status\":\"released\""),
        "the owned lease must release only after terminal lifecycle persistence"
    );
    assert!(
        !fixture.foreground_state_path().exists(),
        "final-selection rejection must not claim or dispatch the issue"
    );
}

#[test]
fn autonomous_foreground_persists_initial_preview_rejection_before_releasing_lease() {
    let fixture = ResilienceFixture::new();
    fixture.write_failures("owner__repo", 42, 3);

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
    assert_eq!(
        fs::read_to_string(fixture.operator_lifecycle_path()).expect("read terminal lifecycle"),
        "{\"version\":1,\"repo\":\"owner/repo\",\"result\":{\"decision\":\"reject\",\"reason\":\"failure_cap\"}}\n"
    );
    assert!(
        fs::read_to_string(fixture.canonical_state_path())
            .expect("read released conductor lease")
            .contains("\"status\":\"released\""),
        "the owned lease must release only after terminal lifecycle persistence"
    );
}

#[test]
fn obsolete_paused_selection_retires_after_authoritative_issue_reread() {
    for issue_state in ["closed", "unlabeled"] {
        let fixture = ResilienceFixture::new();
        fixture.write_paused_foreground_state(1600);

        let output = fixture.run_foreground_with_issue_state(issue_state);

        assert!(
            output.status.success(),
            "{issue_state}: stderr={}",
            String::from_utf8_lossy(&output.stderr)
        );
        let state = fixture.read_foreground_state();
        assert_eq!(state.phase(), ConductorPhase::Scan, "{issue_state}");
        assert_eq!(state.selected_issue(), None, "{issue_state}");
    }
}

#[test]
fn obsolete_paused_selection_reread_failure_preserves_paused_state() {
    let fixture = ResilienceFixture::new();
    fixture.write_paused_foreground_state(1600);
    let before = fs::read_to_string(fixture.foreground_state_path()).expect("read paused state");

    let output = fixture.run_foreground_with_issue_state("failure");

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("gh issue reread 1600 failed"),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_to_string(fixture.foreground_state_path()).expect("reread paused state"),
        before
    );
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
fn autonomous_status_json_reports_unavailable_when_state_file_is_missing() {
    let fixture = ResilienceFixture::new();
    let logpath = fixture.root.join("conductor.log");
    write_file(
        &logpath,
        "[conductor] cycle 99 repo=owner/repo\n[conductor] waterfall decision tier=4 action=run-explore-once-internet\n",
    );
    write_file(
        &fixture.operator_root.join("owner_repo/conductor.logpath"),
        &format!("{}\n", logpath.display()),
    );

    let output = fixture.run_autonomous(&["status", "--repo", "owner/repo", "--json"]);

    assert!(output.status.success());
    let body: serde_json::Value = serde_json::from_str(&stdout(&output)).expect("status json");
    assert_eq!(body["state_outcome"], "Unavailable");
    assert_ne!(body["state_status"], "running");
    assert_ne!(body["current_cycle"], "99");
    assert_eq!(body["heartbeat_age_secs"], serde_json::Value::Null);
}

#[test]
fn autonomous_status_json_reports_malformed_state_when_legacy_state_cannot_parse() {
    let fixture = ResilienceFixture::new();
    let legacy_root = fixture.root.join("legacy-state");
    write_file(&legacy_root.join("owner_repo/state.json"), "{not-json}");

    let output = fixture
        .command()
        .args(["status", "--repo", "owner/repo", "--json", "--repo-dir"])
        .arg(&fixture.repo_dir)
        .env("AUTOSPEC_HOST", "autospec-test-host")
        .env("AUTOSPEC_AUTONOMOUS_STATE_DIR", &legacy_root)
        .output()
        .expect("run autonomous status against malformed legacy state");

    assert!(output.status.success());
    let body: serde_json::Value = serde_json::from_str(&stdout(&output)).expect("status json");
    assert_eq!(body["state_outcome"], "MalformedState");
    assert_eq!(body["state_status"], serde_json::Value::Null);
    assert_eq!(body["heartbeat_age_secs"], serde_json::Value::Null);
}

#[test]
fn autonomous_status_json_reports_healthy_legacy_state_fields() {
    let fixture = ResilienceFixture::new();
    let legacy_root = fixture.root.join("legacy-state");
    let heartbeat = now_secs().saturating_sub(4);
    write_file(
        &legacy_root.join("owner_repo/state.json"),
        &format!(
            r#"{{"status":"running","heartbeat_at":{heartbeat},"cycle":12,"current_cycle":13,"current_tier":"2","current_action":"run-explore-once","last_blocker":"none"}}"#
        ),
    );

    let output = fixture
        .command()
        .args(["status", "--repo", "owner/repo", "--json", "--repo-dir"])
        .arg(&fixture.repo_dir)
        .env("AUTOSPEC_HOST", "autospec-test-host")
        .env("AUTOSPEC_AUTONOMOUS_STATE_DIR", &legacy_root)
        .output()
        .expect("run autonomous status against healthy legacy state");

    assert!(output.status.success());
    let body: serde_json::Value = serde_json::from_str(&stdout(&output)).expect("status json");
    assert_eq!(body["state_outcome"], "Ok");
    assert_eq!(body["state_status"], "running");
    assert_eq!(body["last_cycle"], "12");
    assert_eq!(body["current_cycle"], "13");
    assert_eq!(body["current_tier"], "2");
    assert_eq!(body["current_action"], "run-explore-once");
    assert_eq!(body["last_blocker"], "none");
    assert!(
        body["heartbeat_age_secs"].as_u64().is_some(),
        "heartbeat age should be derived from state heartbeat: {body}"
    );
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
fn autonomous_monitor_reads_canonical_resilience_status_for_heartbeat_age() {
    let fixture = ResilienceFixture::new();
    let heartbeat = now_secs().saturating_sub(5);
    fixture.write_state(
        "owner__repo",
        format!(
            r#"{{"repo":"owner/repo","status":"running","heartbeat_at":{heartbeat},"cycle":12}}"#
        ),
    );

    let output = fixture.run_autonomous(&["monitor", "--repo", "owner/repo", "--json", "--once"]);

    assert!(output.status.success());
    let body: serde_json::Value = serde_json::from_str(&stdout(&output)).expect("monitor json");
    assert_eq!(body["state_status"], "running");
    assert_eq!(body["current_cycle"], "12");
    assert!(
        body["heartbeat_age_secs"].as_u64().is_some(),
        "monitor should report heartbeat age from canonical resilience state: {body}"
    );
}

#[test]
fn autonomous_monitor_does_not_invent_dry_result_for_tier_errors() {
    let fixture = ResilienceFixture::new();
    let logpath = fixture.root.join("conductor.log");
    write_file(
        &logpath,
        "[conductor] cycle 9 repo=owner/repo
[conductor] waterfall decision tier=2 action=run-explore-once reason=local-discovery
[conductor] Tier 2 explore result: ERROR helper unavailable
",
    );
    write_file(
        &fixture.operator_root.join("owner_repo/conductor.logpath"),
        &format!(
            "{}
",
            logpath.display()
        ),
    );

    let output = fixture.run_autonomous(&["monitor", "--repo", "owner/repo", "--json", "--once"]);

    assert!(output.status.success());
    let body: serde_json::Value = serde_json::from_str(&stdout(&output)).expect("monitor json");
    assert_eq!(body["current_tier"], "2");
    assert_eq!(body["tier_dry_result"], serde_json::Value::Null);
}

#[test]
fn autonomous_monitor_does_not_fall_back_to_stale_dry_result_after_tier_error() {
    let fixture = ResilienceFixture::new();
    let logpath = fixture.root.join("conductor.log");
    write_file(
        &logpath,
        "[conductor] cycle 8 repo=owner/repo
[conductor] Tier 2 explore result: dry=true filed=0
[conductor] cycle 9 repo=owner/repo
[conductor] waterfall decision tier=2 action=run-explore-once reason=local-discovery
[conductor] Tier 2 explore result: ERROR helper unavailable
",
    );
    write_file(
        &fixture.operator_root.join("owner_repo/conductor.logpath"),
        &format!(
            "{}
",
            logpath.display()
        ),
    );

    let output = fixture.run_autonomous(&["monitor", "--repo", "owner/repo", "--json", "--once"]);

    assert!(output.status.success());
    let body: serde_json::Value = serde_json::from_str(&stdout(&output)).expect("monitor json");
    assert_eq!(body["current_tier"], "2");
    assert_eq!(body["tier_dry_result"], serde_json::Value::Null);
}

#[test]
fn autonomous_monitor_distinguishes_launch_dry_run_from_tier_dry_result() {
    let fixture = ResilienceFixture::new();
    let logpath = fixture.root.join("conductor.log");
    write_file(
        &logpath,
        "[conductor] cycle 7 repo=owner/repo\n\
[conductor] waterfall decision tier=2 action=run-explore-once reason=local-discovery\n\
[conductor] Tier 2 explore result: dry=true filed=0\n",
    );
    write_file(
        &fixture.operator_root.join("owner_repo/conductor.pid"),
        "999999\n",
    );
    write_file(
        &fixture.operator_root.join("owner_repo/conductor.logpath"),
        &format!("{}\n", logpath.display()),
    );
    write_file(
        &fixture.operator_root.join("owner_repo/launch.json"),
        "{\"repo\":\"owner/repo\",\"repo_dir\":\".\",\"argv\":[\"autospec\",\"autonomous\",\"start\",\"--repo\",\"owner/repo\",\"--dry-run\"],\"conductor_argv\":[\"autospec\",\"autonomous\",\"run-foreground\",\"--dry-run\"]}\n",
    );

    let output = fixture.run_autonomous(&["monitor", "--repo", "owner/repo", "--json", "--once"]);

    assert!(output.status.success());
    let body: serde_json::Value = serde_json::from_str(&stdout(&output)).expect("monitor json");
    assert_eq!(body["run_mode"], "dry-run");
    assert_eq!(body["current_tier"], "2");
    assert_eq!(body["current_action"], "run-explore-once");
    assert_eq!(body["tier_dry_result"]["dry"], true);
    assert_eq!(body["tier_dry_result"]["filed"], 0);
    assert!(body["tier_dry_result"]["explanation"]
        .as_str()
        .expect("dry explanation")
        .contains("tier produced no filed candidates"));
}

#[test]
fn autonomous_monitor_falls_back_to_logs_and_discovery_artifacts() {
    let fixture = ResilienceFixture::new();
    let logpath = fixture.root.join("conductor.log");
    write_file(
        &logpath,
        "[conductor] cycle 8 repo=owner/repo\n\
[conductor] waterfall decision tier=4 action=run-explore-once-internet reason=internet-discovery\n\
[conductor] Tier 4 explore result: dry=false filed=2\n",
    );
    write_file(
        &fixture.operator_root.join("owner_repo/conductor.logpath"),
        &format!("{}\n", logpath.display()),
    );
    write_file(
        &fixture.operator_root.join("owner_repo/launch.json"),
        "{\"repo\":\"owner/repo\",\"repo_dir\":\".\",\"argv\":[\"autospec\",\"autonomous\",\"start\",\"--repo\",\"owner/repo\"],\"conductor_argv\":[\"autospec\",\"autonomous\",\"run-foreground\"]}\n",
    );
    write_file(
        &fixture.repo_dir.join(".autospec/explore-once-101/research.json"),
        "{\"verification_mode\":\"verified\",\"proposals\":[{\"title\":\"Fix flaky drain\"},{\"title\":\"Improve monitor output\"}]}",
    );
    write_file(
        &fixture
            .repo_dir
            .join(".autospec/explore-once-101/candidates.json"),
        "[{\"title\":\"Fix flaky drain\"},{\"title\":\"Improve monitor output\"}]",
    );

    let output = fixture.run_autonomous(&["monitor", "--repo", "owner/repo", "--json", "--once"]);

    assert!(output.status.success());
    let body: serde_json::Value = serde_json::from_str(&stdout(&output)).expect("monitor json");
    assert_eq!(body["run_mode"], "real");
    assert_eq!(body["current_cycle"], "8");
    assert_eq!(body["current_tier"], "4");
    assert_eq!(body["current_action"], "run-explore-once-internet");
    assert_eq!(body["discovery_artifacts"][0]["proposals"], 2);
    assert_eq!(body["discovery_artifacts"][0]["candidates"], 2);
    assert_eq!(
        body["discovery_artifacts"][0]["verification_mode"],
        "verified"
    );
    assert_eq!(
        body["discovery_artifacts"][0]["filed_issue_titles"][0],
        "Fix flaky drain"
    );
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
        let root = fixture_root(sequence);
        fs::create_dir_all(&root).expect("create fixture root");
        let bin = root.join("bin");
        fs::create_dir_all(&bin).expect("create fixture bin");
        let fixture = Self {
            state_root: root.join("state"),
            spend_root: root.join("spend"),
            operator_root: root.join("operator"),
            repo_dir: root.join("repo"),
            root,
        };
        fixture.initialize_git_remote();
        write_executable(&bin.join("gh"), ACCOUNTABILITY_ONLY_GH);
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
            .env("AUTOSPEC_NO_SELF_UPDATE", "1")
            .env(
                "AUTOSPEC_FOREGROUND_ACCOUNTABILITY",
                self.root.join("accountability.md"),
            )
            .env("AUTOSPEC_FOREGROUND_ACCOUNTABILITY_REPO", "owner/repo")
            .env(
                "AUTOSPEC_FOREGROUND_ACCOUNTABILITY_HANDLER",
                concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/support/foreground_accountability_gh.sh"
                ),
            )
            .env("PATH", path_with(&self.root.join("bin")))
            .env("HOME", &self.root)
            .env_remove("AUTOSPEC_STOP_FLAG_FILE")
            .env_remove("AUTOSPEC_RUN_ONLY_ISSUES")
            .env_remove("AUTOSPEC_RUN_CMD")
            .env_remove("AUTOSPEC_EXPLORE_CMD")
            .env_remove("AUTOSPEC_EXPLORE_VERIFY_CMD");
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

    fn foreground_with_changed_selection_command(&self) -> Command {
        let bin = self.root.join("bin");
        let counter = self.root.join("ready-count");
        fs::create_dir_all(&bin).expect("create fake bin");
        fs::write(&counter, "0\n").expect("write ready counter");
        write_executable(
            &bin.join("gh"),
            r####"#!/bin/sh
set -eu
. "$AUTOSPEC_FOREGROUND_ACCOUNTABILITY_HANDLER"

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
      if [ -n "${AUTOSPEC_RESILIENCE_FINAL_SELECTION_READY:-}" ]; then
        : > "$AUTOSPEC_RESILIENCE_FINAL_SELECTION_READY"
        sleep 2
      fi
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
        let mut command = self.command();
        command
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
            .env("PATH", path_with(&bin));
        command
    }

    fn run_foreground_with_changed_selection(&self) -> Output {
        self.foreground_with_changed_selection_command()
            .output()
            .expect("run foreground with changing selection")
    }

    fn run_foreground_with_issue_state(&self, issue_state: &str) -> Output {
        let bin = self.root.join("bin");
        fs::create_dir_all(&bin).expect("create fake bin");
        write_executable(
            &bin.join("gh"),
            r####"#!/bin/sh
set -eu
. "$AUTOSPEC_FOREGROUND_ACCOUNTABILITY_HANDLER"
endpoint=""
for value in "$@"; do
  case "$value" in repos/*) endpoint="$value" ;; esac
done
case "$endpoint" in
  repos/owner/repo/branches/main) printf '%s\n' '{}' ;;
  repos/owner/repo/commits/main/status)
    printf '%s\n' '{"state":"success","total_count":1,"statuses":[{"context":"ci","state":"success"}]}'
    ;;
  *labels=auto-implement*|*labels=in-progress-by-bot*)
    printf '%s\n' '{"raw_count":0,"items":[]}'
    ;;
  repos/owner/repo/issues/1600)
    case "$AUTOSPEC_TEST_ISSUE_STATE" in
      closed) printf '%s\n' '{"labels":["auto-implement"],"state":"closed"}' ;;
      unlabeled) printf '%s\n' '{"labels":[],"state":"open"}' ;;
      failure) printf '%s\n' 'authoritative reread failed' >&2; exit 1 ;;
    esac
    ;;
  repos/owner/repo/issues/1600/comments?*)
    printf '%s\n' '{"raw_count":0,"items":[]}'
    ;;
  '') printf '%s\n' '{"items":[],"page_info":{"has_next_page":false,"end_cursor":null}}' ;;
  *) printf 'unexpected gh endpoint: %s\n' "$endpoint" >&2; exit 1 ;;
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
            .env("AUTOSPEC_CLAIM_GIT_REMOTE", &self.repo_dir)
            .env("AUTOSPEC_TEST_ISSUE_STATE", issue_state)
            .env("PATH", path_with(&bin))
            .output()
            .expect("run foreground against authoritative issue state")
    }

    fn write_paused_foreground_state(&self, issue: u64) {
        let state = ConductorState::new("owner/repo", ConductorScope::Repository, 3)
            .expect("state")
            .transition(ConductorEvent::ScanFoundWork)
            .expect("scan")
            .transition(ConductorEvent::SafetyReviewed)
            .expect("review")
            .transition(ConductorEvent::Selected {
                issue,
                serialization_reasons: Vec::new(),
            })
            .expect("select")
            .transition(ConductorEvent::Pause {
                reason: "operator_wait".to_string(),
            })
            .expect("pause");
        write_file(&self.foreground_state_path(), &state.to_json());
    }

    fn read_foreground_state(&self) -> ConductorState {
        let source =
            fs::read_to_string(self.foreground_state_path()).expect("read foreground state");
        ConductorState::parse_json(&source).expect("parse foreground state")
    }

    fn initialize_git_remote(&self) {
        let remote = self.root.join("github.com/owner/repo.git");
        fs::create_dir_all(remote.parent().expect("integration remote parent"))
            .expect("create integration remote parent");
        fs::create_dir_all(&self.repo_dir).expect("create fixture repository");
        git_fixture(&self.root, &["init", "--bare", remote.to_str().unwrap()]);
        git_fixture(&self.repo_dir, &["init", "-b", "main"]);
        git_fixture(&self.repo_dir, &["config", "user.name", "Autospec Test"]);
        git_fixture(
            &self.repo_dir,
            &["config", "user.email", "autospec@example.invalid"],
        );
        fs::write(self.repo_dir.join("README.md"), "fixture\n").expect("write Git fixture");
        git_fixture(&self.repo_dir, &["add", "README.md"]);
        git_fixture(&self.repo_dir, &["commit", "-m", "fixture"]);
        git_fixture(
            &self.repo_dir,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        git_fixture(&self.repo_dir, &["push", "-u", "origin", "main"]);
    }

    fn seed_bound_accountability(&self, lease_generation: u64) {
        seed_bound_accountability(
            &self.operator_root,
            &self.root.join("accountability.md"),
            lease_generation,
        );
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

    fn write_spend_counters(&self, slug: &str, tokens: u64, filed_issues: u64, budget_issues: u64) {
        write_file(
            &self.spend_root.join(slug).join("spend.json"),
            &format!(
                "{{\"schema\":1,\"tokens\":{tokens},\"filed_issues\":{filed_issues},\"budget_issues\":{budget_issues},\"parked\":false}}"
            ),
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

fn token_state(
    status: &str,
    heartbeat_at: Option<u64>,
    lock_pid: Option<u32>,
    lock_host: Option<&str>,
    lock_acquired_at: Option<u64>,
    lease_token: Option<&str>,
    lease_generation: Option<u64>,
) -> String {
    format!(
        "{{\"repo\":\"owner/repo\",\"slug\":\"owner__repo\",\"status\":\"{status}\",\"host\":\"autospec-test-host\",\"session\":\"test\",\"heartbeat_at\":{},\"lock_pid\":{},\"lock_host\":{},\"lock_session\":null,\"lock_acquired_at\":{},\"lease_token\":{},\"lease_generation\":{}}}",
        heartbeat_at.map(|timestamp| timestamp.to_string()).unwrap_or_else(|| "null".to_string()),
        lock_pid.map(|pid| pid.to_string()).unwrap_or_else(|| "null".to_string()),
        lock_host.map(|host| format!("\"{host}\"")).unwrap_or_else(|| "null".to_string()),
        lock_acquired_at.map(|timestamp| timestamp.to_string()).unwrap_or_else(|| "null".to_string()),
        lease_token.map(|token| format!("\"{token}\"")).unwrap_or_else(|| "null".to_string()),
        lease_generation.map(|generation| generation.to_string()).unwrap_or_else(|| "null".to_string()),
    )
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
}
