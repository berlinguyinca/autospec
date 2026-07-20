use autospec_core::autonomous::no_work::NoWorkTier;
use autospec_core::autonomous::premerge::PremergeLaneIdentity;
use autospec_core::autonomous::waterfall::{sha256_hex, TierReceipt, TierStatus, WaterfallState};
use autospec_core::claim::{parse_remote_comments_json, select_run_state, RunStateRecord};
use autospec_core::coordination::{ConductorOutcome, ConductorPhase, ConductorState};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const EXECUTOR_CLAIM_ID: &str = "claim-generation-42";
const EXECUTOR_COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
const PREMERGE_RECEIPT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn workspace_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
}

/// How a forbidden-authority pattern must be located in source text.
///
/// Bare Rust substring matching is brittle: a needle like `"Command"` also
/// matches inside unrelated identifiers such as `SubCommand` or
/// `TierCommand`. Each table entry below states its own match primitive so
/// callers never fall back to an ambiguous raw `contains`.
enum SourcePattern {
    /// The needle must appear as a whole identifier token (no adjacent
    /// identifier characters on either side).
    Token(&'static str),
    /// The needle is an identifier-family prefix (e.g. `FOO_BAR_`): only the
    /// left boundary is checked, so suffixed family members still match.
    Prefix(&'static str),
    /// The needle is an unambiguous literal snippet (path or multi-token
    /// code fragment) where raw substring matching carries no synonym risk.
    Literal(&'static str),
}

impl SourcePattern {
    fn needle(&self) -> &'static str {
        match self {
            SourcePattern::Token(needle)
            | SourcePattern::Prefix(needle)
            | SourcePattern::Literal(needle) => needle,
        }
    }

    fn matches(&self, source: &str) -> bool {
        match self {
            SourcePattern::Token(needle) => contains_identifier_token(source, needle),
            SourcePattern::Prefix(needle) => contains_left_bounded(source, needle),
            SourcePattern::Literal(needle) => source.contains(needle),
        }
    }
}

fn is_identifier_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// True if `needle` occurs in `source` with no identifier character
/// immediately before or after the match (i.e. as a standalone token).
fn contains_identifier_token(source: &str, needle: &str) -> bool {
    source.match_indices(needle).any(|(start, matched)| {
        let before_is_boundary = source[..start]
            .chars()
            .next_back()
            .is_none_or(|c| !is_identifier_char(c));
        let end = start + matched.len();
        let after_is_boundary = source[end..]
            .chars()
            .next()
            .is_none_or(|c| !is_identifier_char(c));
        before_is_boundary && after_is_boundary
    })
}

/// True if `needle` occurs in `source` with no identifier character
/// immediately before the match; the character after the match is
/// unconstrained so identifier-family prefixes (e.g. `FOO_BAR_BAZ`) match.
fn contains_left_bounded(source: &str, needle: &str) -> bool {
    source.match_indices(needle).any(|(start, _)| {
        source[..start]
            .chars()
            .next_back()
            .is_none_or(|c| !is_identifier_char(c))
    })
}

fn assert_no_forbidden_authority(source: &str, patterns: &[SourcePattern], context: &str) {
    for pattern in patterns {
        assert!(
            !pattern.matches(source),
            "{context} retains legacy authority: {}",
            pattern.needle()
        );
    }
}

#[test]
fn foreground_source_has_no_legacy_shell_authority() {
    let source =
        fs::read_to_string(workspace_root().join("crates/autospec-cli/src/commands/autonomous.rs"))
            .expect("read autonomous command source");

    assert_no_forbidden_authority(
        &source,
        &[
            SourcePattern::Token("AUTOSPEC_AUTONOMOUS_SCRIPT"),
            SourcePattern::Token("AUTOSPEC_AUTONOMOUS_CONDUCTOR_CMD"),
            SourcePattern::Prefix("AUTOSPEC_MAIN_HEALTH_"),
            SourcePattern::Literal("scripts/autospec-autonomous.sh"),
            SourcePattern::Literal("scripts/autonomous-resilience.sh"),
            SourcePattern::Literal("Command::new(\"bash\")"),
            SourcePattern::Literal("Command::new(\"sh\")"),
        ],
        "foreground command",
    );
    assert!(source.contains("executor-result"));
    assert!(source.contains("ExecutorRequest"));

    let coordinator = fs::read_to_string(
        workspace_root()
            .join("crates/autospec-cli/src/commands/autonomous/waterfall_coordinator.rs"),
    )
    .expect("read native waterfall coordinator source");
    assert_no_forbidden_authority(
        &coordinator,
        &[
            SourcePattern::Token("ready_plan_for"),
            SourcePattern::Literal("NoWorkState::record"),
            SourcePattern::Literal("why-no-work.json"),
            SourcePattern::Literal("claim::"),
            SourcePattern::Token("Command"),
            SourcePattern::Literal("std::process"),
        ],
        "tier-one coordinator",
    );
    assert!(coordinator.contains("ConductorLease"));
}

#[test]
fn foreground_empty_repository_queue_records_tier_one_without_remote_mutation() {
    let fixture = ForegroundFixture::new();
    let first = fixture
        .command()
        .env("AUTOSPEC_FOREGROUND_EMPTY_QUEUE", "1")
        .output()
        .expect("run empty foreground queue");

    assert!(
        first.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert_eq!(fixture.read_state().phase(), ConductorPhase::Scan);
    let state = fixture.read_waterfall_state();
    assert_eq!(state.next_pass_id(), 1);
    assert_eq!(state.current_tier(), NoWorkTier::Tier1_5);
    let receipt = fixture.read_tier_one_receipt(1);
    assert!(matches!(receipt.status(), TierStatus::Exhausted { .. }));
    let receipt_source =
        fs::read_to_string(fixture.tier_one_receipt_path(1)).expect("read first sealed receipt");
    let evidence_source = fs::read_to_string(fixture.tier_one_evidence_path(1, "ready-page"))
        .expect("read sealed Tier 1 queue evidence");
    let evidence_digest = sha256_hex(evidence_source.as_bytes());
    assert!(
        receipt_source.contains(&format!("\"digest\":\"{evidence_digest}\"")),
        "the receipt must retain the persisted queue artifact digest"
    );
    let calls = fs::read_to_string(&fixture.calls).expect("read GitHub calls");
    assert_eq!(
        calls.matches("labels=auto-implement").count(),
        1,
        "foreground must reuse its captured ready-queue snapshot"
    );
    for forbidden in ["issue\nedit", "issue\ncomment", "executor_pending"] {
        assert!(
            !calls.contains(forbidden),
            "empty Tier 1 must not invoke remote mutation: {forbidden}"
        );
    }
    assert!(
        !fixture.operator.join("test_repo/why-no-work.json").exists(),
        "later waterfall tiers remain pending"
    );

    fs::remove_file(fixture.waterfall_state_path()).expect("remove cursor after receipt write");
    let replay = fixture
        .command()
        .env("AUTOSPEC_FOREGROUND_QUEUE_FAILURE", "1")
        .output()
        .expect("resume from an interrupted Tier 1 receipt despite a changed queue result");
    assert!(
        replay.status.success(),
        "an existing sealed receipt must replay before a new queue failure is recorded; stderr={}",
        String::from_utf8_lossy(&replay.stderr),
    );
    assert_eq!(fixture.read_waterfall_state(), state);
    assert_eq!(
        fs::read_to_string(fixture.tier_one_receipt_path(1)).expect("read replayed receipt"),
        receipt_source
    );

    let pending = fixture
        .command()
        .env("AUTOSPEC_FOREGROUND_QUEUE_FAILURE", "1")
        .output()
        .expect("leave later waterfall tiers pending");
    assert!(
        pending.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&pending.stderr)
    );
    assert_eq!(fixture.read_waterfall_state(), state);
    assert_eq!(
        fs::read_to_string(fixture.tier_one_receipt_path(1)).expect("read pending receipt"),
        receipt_source
    );
}

#[test]
fn foreground_rejects_tampered_tier_one_evidence_during_receipt_replay() {
    let fixture = ForegroundFixture::new();
    let first = fixture
        .command()
        .env("AUTOSPEC_FOREGROUND_EMPTY_QUEUE", "1")
        .output()
        .expect("record Tier 1 evidence");
    assert!(first.status.success());
    fs::remove_file(fixture.waterfall_state_path()).expect("remove cursor after receipt write");
    fs::write(
        fixture.tier_one_evidence_path(1, "ready-page"),
        "{\"schema\":1,\"tampered\":true}\n",
    )
    .expect("tamper Tier 1 evidence");

    let replay = fixture
        .command()
        .env("AUTOSPEC_FOREGROUND_EMPTY_QUEUE", "1")
        .output()
        .expect("resume Tier 1 receipt");

    assert!(!replay.status.success());
    assert!(String::from_utf8_lossy(&replay.stderr)
        .contains("early-tier evidence digest does not match its receipt"));
    assert!(
        !fixture.waterfall_state_path().exists(),
        "tampered evidence must not advance the cursor"
    );
}

#[test]
fn foreground_empty_slice_does_not_start_a_repository_waterfall_pass() {
    let fixture = ForegroundFixture::new();
    let output = fixture
        .command()
        .env("AUTOSPEC_FOREGROUND_EMPTY_QUEUE", "1")
        .env("AUTOSPEC_RUN_ONLY_ISSUES", "42")
        .output()
        .expect("run empty slice foreground queue");

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let slice_state = ConductorState::parse_json(
        &fs::read_to_string(fixture.slice_state_path("42")).expect("read slice state"),
    )
    .expect("parse slice state");
    assert_eq!(slice_state.phase(), ConductorPhase::SliceComplete);
    assert!(
        !fixture.waterfall_dir().exists(),
        "a slice-empty queue must not create repository waterfall state"
    );
}

#[test]
fn foreground_queue_failure_seals_tier_one_without_advancing_later_tiers() {
    let fixture = ForegroundFixture::new();
    let output = fixture
        .command()
        .env("AUTOSPEC_FOREGROUND_QUEUE_FAILURE", "1")
        .output()
        .expect("run failed foreground queue");

    assert!(!output.status.success());
    let receipt = fixture.read_tier_one_receipt(1);
    assert!(matches!(receipt.status(), TierStatus::Failed { .. }));
    assert!(
        !fixture.waterfall_state_path().exists(),
        "failed Tier 1 must not advance to Tier 1.5"
    );
    assert!(
        !fixture.operator.join("test_repo/why-no-work.json").exists(),
        "failed Tier 1 must not record no-work"
    );
}

#[test]
fn foreground_records_a_typed_deferred_receipt_and_keeps_the_selected_issue() {
    let fixture = ForegroundFixture::new();
    let first = fixture.run_foreground();
    assert!(
        first.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&first.stderr)
    );
    let state = fixture.read_state();
    assert_eq!(state.phase(), ConductorPhase::Paused);
    assert_eq!(state.selected_issue(), Some(42));
    assert_eq!(
        state.last_outcome(),
        Some(&ConductorOutcome::Blocked(
            "implementation_executor_pending".to_string()
        ))
    );
    assert_eq!(
        state.pause_reason(),
        Some("implementation_executor_pending")
    );
    let calls = fs::read_to_string(&fixture.calls).expect("read GitHub calls");
    let review = calls
        .find("repos/test/repo/issues/42\n--jq")
        .expect("issue reread");
    let claim = calls.find("issue\nedit\n42").expect("claim label change");
    assert!(review < claim, "safety review must precede claim selection");
    assert!(calls.contains("executor_pending"));
    let invocations = fs::read_dir(fixture.repo_dir.join(".autospec/executor-invocations"))
        .expect("invocation receipts")
        .count();
    assert_eq!(invocations, 1, "one invocation receipt is persisted");

    let second = fixture.run_foreground();
    assert!(
        second.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&second.stderr)
    );
    assert_eq!(fixture.read_state(), state);
    assert_eq!(
        fs::read_dir(fixture.repo_dir.join(".autospec/executor-invocations"))
            .expect("replayed invocation receipts")
            .count(),
        1,
        "restart replays the terminal invocation"
    );
}

#[test]
fn foreground_state_is_partitioned_by_run_scope() {
    let fixture = ForegroundFixture::new();
    let repository = fixture.run_foreground();
    assert!(repository.status.success());
    let repository_state = fixture.read_state();

    let slice = fixture
        .command()
        .env("AUTOSPEC_RUN_ONLY_ISSUES", "43")
        .output()
        .expect("run slice foreground");

    assert!(
        slice.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&slice.stderr)
    );
    assert_eq!(fixture.read_state(), repository_state);
    let slice_path = fixture.slice_state_path("43");
    assert!(slice_path.exists());
    let slice_state =
        ConductorState::parse_json(&fs::read_to_string(&slice_path).expect("read slice state"))
            .expect("parse slice state");
    assert_eq!(slice_state.phase(), ConductorPhase::SliceComplete);

    let repeated_slice = fixture
        .command()
        .env("AUTOSPEC_RUN_ONLY_ISSUES", "43")
        .output()
        .expect("rerun slice foreground");

    assert!(
        repeated_slice.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&repeated_slice.stderr)
    );
    assert_eq!(
        ConductorState::parse_json(&fs::read_to_string(&slice_path).expect("read slice state"))
            .expect("parse slice state"),
        slice_state
    );
}

#[test]
fn foreground_fails_closed_when_executor_outcome_loses_its_claim() {
    let fixture = ForegroundFixture::new();
    let output = fixture
        .command()
        .env("AUTOSPEC_FOREGROUND_STEAL_ON_OUTCOME", "1")
        .output()
        .expect("run foreground");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("claim ownership changed"));
    assert!(fs::read_to_string(&fixture.comments)
        .expect("read comments")
        .contains("foreign-worker"));
}

#[test]
fn foreground_rejects_a_terminal_run_state_before_claim_mutation() {
    let fixture = ForegroundFixture::new();
    fixture.seed_claim_state(
        "foreign-worker",
        "autonomous/issue-42",
        "merged",
        &fresh_iso_timestamp(),
    );

    let output = fixture.run_foreground();

    assert_eq!(output.status.code(), Some(3));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "{\"decision\":\"reject\",\"reason\":\"terminal_claim\"}\n"
    );
    let calls = fs::read_to_string(&fixture.calls).expect("read GitHub calls");
    assert!(
        !calls.contains("issue\nedit\n42"),
        "terminal state must be rejected before claim label mutation"
    );
}

#[test]
fn foreground_rejects_malformed_run_state_before_claim_mutation() {
    let fixture = ForegroundFixture::new();
    fixture.seed_malformed_claim();

    let output = fixture.run_foreground();

    assert_eq!(output.status.code(), Some(3));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "{\"decision\":\"reject\",\"reason\":\"malformed_claim\"}\n"
    );
    let calls = fs::read_to_string(&fixture.calls).expect("read GitHub calls");
    assert!(
        !calls.contains("issue\nedit\n42"),
        "malformed state must be rejected before claim label mutation"
    );
}

#[test]
fn detached_start_resolves_the_repository_before_launching_foreground() {
    let fixture = ForegroundFixture::new();
    fixture.initialize_git_remote();
    let output = fixture
        .configured_command()
        .args([
            "autonomous",
            "start",
            "--repo-dir",
            fixture.repo_dir.to_str().expect("repo directory"),
            "--branch",
            "main",
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_COMPANIONS", "0")
        .output()
        .expect("start detached foreground");

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    wait_for_file_contents(&fixture.calls, "repos/test/repo/branches/main");
    let calls = fs::read_to_string(&fixture.calls).expect("read GitHub calls");
    assert!(calls.contains("repos/test/repo/branches/main"));
    assert!(!calls.contains("repos/unknown"));
}

#[test]
fn detached_restart_resolves_the_repository_before_launching_foreground() {
    let fixture = ForegroundFixture::new();
    fixture.initialize_git_remote();
    let output = fixture
        .configured_command()
        .args([
            "autonomous",
            "restart",
            "--repo-dir",
            fixture.repo_dir.to_str().expect("repo directory"),
            "--branch",
            "main",
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_COMPANIONS", "0")
        .output()
        .expect("restart detached foreground");

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    wait_for_file_contents(&fixture.calls, "repos/test/repo/branches/main");
    let calls = fs::read_to_string(&fixture.calls).expect("read GitHub calls");
    assert!(calls.contains("repos/test/repo/branches/main"));
    assert!(!calls.contains("repos/unknown"));
}

#[test]
fn foreground_stops_before_executor_when_main_health_blocks() {
    let fixture = ForegroundFixture::new();
    let output = fixture
        .command()
        .args(["--branch", "missing"])
        .output()
        .expect("run");

    assert_eq!(output.status.code(), Some(20));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "{\"decision\":\"park\",\"reason\":\"health_halt\"}\n"
    );
    assert!(!fixture.state_path().exists());
    assert!(fixture.operator.join("test_repo/lifecycle.json").exists());
    assert!(!fs::read_to_string(&fixture.calls)
        .expect("read GitHub calls")
        .contains("executor_pending"));
}

#[test]
fn repository_configured_health_branch_precedes_github_default() {
    let fixture = ForegroundFixture::new();
    fixture.write_autonomous_config("main_health:\n  branch: master_ai\n");

    let output = fixture
        .unbranched_foreground_command()
        .output()
        .expect("run foreground with configured branch");

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let calls = fs::read_to_string(&fixture.calls).expect("read GitHub calls");
    assert!(calls.contains("repos/test/repo/branches/master_ai"));
    assert!(
        !calls.contains("repo\nview"),
        "config avoids default lookup"
    );
    assert!(!calls.contains("repos/test/repo/branches/main"));
}

#[test]
fn explicit_health_branch_overrides_repository_configuration() {
    let fixture = ForegroundFixture::new();
    fixture.write_autonomous_config("main_health:\n  branch: master_ai\n");

    let output = fixture.command().output().expect("run foreground override");

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let calls = fs::read_to_string(&fixture.calls).expect("read GitHub calls");
    assert!(calls.contains("repos/test/repo/branches/main"));
    assert!(!calls.contains("repos/test/repo/branches/master_ai"));
}

#[test]
fn ignored_failed_check_is_advisory_for_foreground_admission() {
    let fixture = ForegroundFixture::new();
    fixture.write_autonomous_config("main_health:\n  ignore_checks:\n    - Unit Tests\n");

    let output = fixture
        .command()
        .env("AUTOSPEC_FOREGROUND_HEALTH_CASE", "ignored_failure")
        .output()
        .expect("run foreground with advisory failure");

    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(fs::read_to_string(&fixture.calls)
        .expect("read GitHub calls")
        .contains("executor_pending"));
}

#[test]
fn malformed_repository_config_fails_before_foreground_dispatch() {
    let fixture = ForegroundFixture::new();
    fixture.write_autonomous_config("main_health:\n  ignore_checks: Unit Tests\n");

    let output = fixture.command().output().expect("run malformed config");

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("autonomous.yml"));
    assert!(
        !fixture.operator.exists(),
        "must fail before lease persistence"
    );
    assert!(!fixture.state_path().exists());
    let calls = fs::read_to_string(&fixture.calls).unwrap_or_default();
    assert!(!calls.contains("repos/test/repo/branches/"));
    assert!(!calls.contains("executor_pending"));
    assert!(!calls.contains("issue\nedit\n42"));
    assert!(!calls.contains("issue\ncomment\n42"));
}

#[test]
fn unreadable_repository_config_fails_before_foreground_dispatch() {
    let fixture = ForegroundFixture::new();
    let config_dir = fixture.repo_dir.join(".autospec");
    fs::create_dir_all(config_dir.join("autonomous.yml")).expect("create config directory");

    let output = fixture.command().output().expect("run unreadable config");

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("cannot read autonomous repository config"));
    assert!(
        !fixture.operator.exists(),
        "must fail before lease persistence"
    );
    assert!(!fixture.state_path().exists());
    let calls = fs::read_to_string(&fixture.calls).unwrap_or_default();
    assert!(!calls.contains("repos/test/repo/branches/"));
    assert!(!calls.contains("issue\nedit\n42"));
    assert!(!calls.contains("issue\ncomment\n42"));
    assert!(!calls.contains("executor_pending"));
}

#[test]
fn detached_start_rejects_invalid_config_before_lifecycle_mutation() {
    let fixture = ForegroundFixture::new();
    fixture.initialize_git_remote();
    fixture.write_autonomous_config("main_health:\n  ignore_checks: Unit Tests\n");

    let output = fixture
        .detached_command("start")
        .output()
        .expect("start with malformed config");

    assert_eq!(output.status.code(), Some(2));
    assert!(!fixture.operator.exists());
    assert!(!fixture.state_path().exists());
    assert!(!fixture.calls.exists());
}

#[test]
fn detached_restart_rejects_invalid_config_before_lifecycle_mutation() {
    let fixture = ForegroundFixture::new();
    fixture.initialize_git_remote();
    fixture.write_autonomous_config("main_health:\n  ignore_checks: Unit Tests\n");

    let output = fixture
        .detached_command("restart")
        .output()
        .expect("restart with malformed config");

    assert_eq!(output.status.code(), Some(2));
    assert!(!fixture.operator.exists());
    assert!(!fixture.state_path().exists());
    assert!(!fixture.calls.exists());
}

#[test]
fn detached_launches_reject_unreadable_config_before_lifecycle_mutation() {
    for subcommand in ["start", "restart"] {
        let fixture = ForegroundFixture::new();
        fixture.initialize_git_remote();
        let config_dir = fixture.repo_dir.join(".autospec");
        fs::create_dir_all(config_dir.join("autonomous.yml")).expect("create config directory");

        let output = fixture
            .detached_command(subcommand)
            .output()
            .expect("launch with unreadable config");

        assert_eq!(output.status.code(), Some(2), "{subcommand}");
        assert!(!fixture.operator.exists(), "{subcommand}");
        assert!(!fixture.state_path().exists(), "{subcommand}");
        assert!(!fixture.calls.exists(), "{subcommand}");
    }
}

#[test]
fn autonomous_health_config_is_isolated_to_its_repository() {
    let configured = ForegroundFixture::new();
    configured.write_autonomous_config("main_health:\n  branch: master_ai\n");
    let defaulted = ForegroundFixture::new();

    let configured_output = configured
        .unbranched_foreground_command()
        .output()
        .expect("run configured repository");
    let defaulted_output = defaulted
        .unbranched_foreground_command()
        .output()
        .expect("run default repository");

    assert!(configured_output.status.success());
    assert!(defaulted_output.status.success());
    assert!(fs::read_to_string(&configured.calls)
        .expect("read configured calls")
        .contains("repos/test/repo/branches/master_ai"));
    let defaulted_calls = fs::read_to_string(&defaulted.calls).expect("read default calls");
    assert!(defaulted_calls.contains("repos/test/repo/branches/main"));
    assert!(!defaulted_calls.contains("repos/test/repo/branches/master_ai"));
}

#[test]
fn main_health_reads_the_same_repository_config_as_foreground_admission() {
    let fixture = ForegroundFixture::new();
    fixture.write_autonomous_config("main_health:\n  branch: master_ai\n");

    let output = fixture
        .unbranched_main_health_command()
        .output()
        .expect("run main-health with repository configuration");

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"branch\":\"master_ai\""));
    let calls = fs::read_to_string(&fixture.calls).expect("read GitHub calls");
    assert!(calls.contains("repos/test/repo/branches/master_ai"));
    assert!(!calls.contains("repo\nview"));
}

#[test]
fn missing_default_branch_keeps_its_typed_policy_bound_health_receipt() {
    const UNRESOLVED_POLICY_DIGEST: &str =
        "autospec-main-health-policy-v1:66e6f0c0605153f689ec9b01bbbd3ada254ed0031573a196fed67c7aab401671";
    let fixture = ForegroundFixture::new();

    let output = fixture
        .unbranched_main_health_command()
        .env("AUTOSPEC_FOREGROUND_NO_DEFAULT_BRANCH", "1")
        .output()
        .expect("run main-health without GitHub default branch metadata");

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stdout).contains("default-branch-missing"));
    let receipts = fs::read_to_string(fixture.main_health_observations_path())
        .expect("missing branch health must retain a policy-bound observation");
    let lines = receipts.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("\"diagnostic\":\"default-branch-missing\""));
    assert_eq!(
        json_string_field(lines[0], "effective_policy_digest"),
        UNRESOLVED_POLICY_DIGEST
    );
}

#[test]
fn foreground_missing_default_branch_applies_typed_halt_after_recording_policy() {
    const UNRESOLVED_POLICY_DIGEST: &str =
        "autospec-main-health-policy-v1:66e6f0c0605153f689ec9b01bbbd3ada254ed0031573a196fed67c7aab401671";
    let fixture = ForegroundFixture::new();

    let output = fixture
        .unbranched_foreground_command()
        .env("AUTOSPEC_FOREGROUND_NO_DEFAULT_BRANCH", "1")
        .output()
        .expect("run foreground without GitHub default branch metadata");

    assert_eq!(output.status.code(), Some(20));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "{\"decision\":\"park\",\"reason\":\"health_halt\"}\n"
    );
    let receipts = fs::read_to_string(fixture.main_health_observations_path())
        .expect("foreground health halt must retain a policy-bound observation");
    let lines = receipts.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("\"diagnostic\":\"default-branch-missing\""));
    assert_eq!(
        json_string_field(lines[0], "effective_policy_digest"),
        UNRESOLVED_POLICY_DIGEST
    );
    assert!(!fixture.state_path().exists());
}

#[test]
fn retained_foreground_state_reloads_and_records_repository_policy() {
    let fixture = ForegroundFixture::new();
    fixture.write_autonomous_config(
        "main_health:\n  branch: main\n  ignore_checks:\n    - Unit Tests\n",
    );

    let first = fixture
        .unbranched_foreground_command()
        .env("AUTOSPEC_FOREGROUND_EMPTY_QUEUE", "1")
        .output()
        .expect("run first configured foreground cycle");
    assert!(
        first.status.success(),
        "status={:?} stdout={} stderr={}",
        first.status.code(),
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr)
    );
    let first_receipts = fs::read_to_string(fixture.main_health_observations_path())
        .expect("read first health observation");
    let first_lines = first_receipts.lines().collect::<Vec<_>>();
    assert_eq!(first_lines.len(), 1);
    let first_digest = json_string_field(first_lines[0], "effective_policy_digest");

    fixture.write_autonomous_config(
        "main_health:\n  branch: master_ai\n  ignore_checks:\n    - E2E Tests\n",
    );
    let second = fixture
        .unbranched_foreground_command()
        .env("AUTOSPEC_FOREGROUND_EMPTY_QUEUE", "1")
        .output()
        .expect("run retained foreground cycle with changed config");
    assert!(
        second.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&second.stderr)
    );
    let second_receipts = fs::read_to_string(fixture.main_health_observations_path())
        .expect("read reloaded health observations");
    let second_lines = second_receipts.lines().collect::<Vec<_>>();
    assert_eq!(
        second_lines.len(),
        2,
        "retained state must not suppress the next cycle policy receipt"
    );
    assert_ne!(
        first_digest,
        json_string_field(second_lines[1], "effective_policy_digest"),
        "changed effective repository policy must change its receipt binding"
    );

    fixture.write_autonomous_config("main_health:\n  ignore_checks: malformed\n");
    let before_malformed = second_receipts;
    let malformed = fixture
        .unbranched_foreground_command()
        .env("AUTOSPEC_FOREGROUND_EMPTY_QUEUE", "1")
        .output()
        .expect("run malformed repository config");
    assert_eq!(malformed.status.code(), Some(2));
    assert_eq!(
        fs::read_to_string(fixture.main_health_observations_path())
            .expect("read unchanged health observations"),
        before_malformed,
        "invalid config must fail before appending a policy receipt"
    );
}

#[test]
fn post_health_issue_admission_failure_keeps_the_evaluated_policy_receipt() {
    let fixture = ForegroundFixture::new();
    fs::write(&fixture.mode, "reviewed\n").expect("make the queued issue safety-ready");
    fixture.write_autonomous_config("main_health:\n  branch: main\n");

    let output = fixture
        .unbranched_foreground_command()
        .env("AUTOSPEC_FOREGROUND_CORRUPT_SPEND_AFTER_QUEUE", "1")
        .output()
        .expect("run foreground with a post-health admission failure");

    assert!(!output.status.success());
    let diagnostic = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        diagnostic.contains("malformed_spend"),
        "expected issue-admission failure, got {diagnostic}"
    );
    let receipts = fs::read_to_string(fixture.main_health_observations_path())
        .expect("health evaluation must be recorded before issue admission");
    let lines = receipts.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 1);
    assert!(!json_string_field(lines[0], "effective_policy_digest").is_empty());
}

#[test]
fn invalid_configured_branch_does_not_fall_back_to_github_default() {
    let fixture = ForegroundFixture::new();
    fixture.write_autonomous_config("main_health:\n  branch: missing\n");

    let output = fixture
        .unbranched_main_health_command()
        .output()
        .expect("run main-health with missing configured branch");

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stdout).contains("branch-not-found"));
    let calls = fs::read_to_string(&fixture.calls).expect("read GitHub calls");
    assert!(calls.contains("repos/test/repo/branches/missing"));
    assert!(!calls.contains("repo\nview"));
    assert!(!calls.contains("repos/test/repo/branches/main"));
}

#[test]
fn repository_root_config_is_used_when_repo_dir_is_a_subdirectory() {
    let fixture = ForegroundFixture::new();
    fixture.initialize_git_remote();
    fixture.write_autonomous_config("main_health:\n  branch: master_ai\n");
    let subdirectory = fixture.repo_dir.join("nested/work");
    fs::create_dir_all(&subdirectory).expect("create checkout subdirectory");
    let mut command = fixture.configured_command();
    command.current_dir(&subdirectory).args([
        "autonomous",
        "main-health",
        "--repo",
        "test/repo",
        "--repo-dir",
        subdirectory.to_str().expect("subdirectory path"),
        "--json",
    ]);

    let output = command
        .output()
        .expect("run health from checkout subdirectory");

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"branch\":\"master_ai\""));
    let calls = fs::read_to_string(&fixture.calls).expect("read GitHub calls");
    assert!(calls.contains("repos/test/repo/branches/master_ai"));
    assert!(!calls.contains("repo\nview"));
}

#[test]
fn executor_result_emits_the_typed_deferred_receipt() {
    let output = Command::new(env!("CARGO_BIN_EXE_autospec"))
        .args([
            "autonomous",
            "executor-result",
            "--repo",
            "test/repo",
            "--issue",
            "42",
        ])
        .output()
        .expect("run typed executor result");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "{\"repo\":\"test/repo\",\"issue\":42,\"outcome\":\"blocked\",\"reason\":\"implementation_executor_pending\"}\n"
    );
}

#[test]
fn executor_result_records_an_owner_verified_blocked_outcome() {
    let fixture = ForegroundFixture::new();
    fixture.seed_claim("rust-foreground-conductor-1", "autonomous/issue-42");
    let before = fixture.claim_record();

    let output = fixture
        .configured_command()
        .args([
            "autonomous",
            "executor-result",
            "--repo",
            "test/repo",
            "--issue",
            "42",
            "--worker-id",
            "rust-foreground-conductor-1",
            "--branch",
            "autonomous/issue-42",
            "--outcome",
            "blocked",
            "--reason",
            "waiting-for-review",
        ])
        .output()
        .expect("record blocked executor result");

    assert_eq!(output.status.code(), Some(20));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "{\"status\":\"blocked\",\"repo\":\"test/repo\",\"issue\":42,\"outcome\":\"blocked\",\"reason\":\"waiting-for-review\"}\n",
        "stderr={} calls={}",
        String::from_utf8_lossy(&output.stderr),
        fs::read_to_string(&fixture.calls).expect("read GitHub calls")
    );
    let record = fixture.claim_record();
    assert_eq!(record, before);
    assert!(fs::read_to_string(&fixture.comments)
        .expect("read executor evidence")
        .contains("<!-- autospec-executor-result:begin -->"));
}

#[test]
fn executor_result_rejects_malformed_protocol_input_without_mutating_claim_state() {
    let fixture = ForegroundFixture::new();
    fixture.seed_claim("rust-foreground-conductor-1", "autonomous/issue-42");
    let before = fs::read_to_string(&fixture.comments).expect("read initial comments");

    for args in [
        vec!["autonomous", "executor-result", "--repo", "test/repo"],
        vec!["autonomous", "executor-result", "--issue", "42"],
        vec![
            "autonomous",
            "executor-result",
            "--repo",
            "test/repo",
            "--issue",
            "42",
            "--repo",
            "test/repo",
        ],
        vec![
            "autonomous",
            "executor-result",
            "--repo",
            "test/repo",
            "--issue",
            "42",
            "--unexpected",
            "value",
        ],
        vec![
            "autonomous",
            "executor-result",
            "--repo",
            "test/repo",
            "--issue",
            "42",
            "--worker-id",
            "rust-foreground-conductor-1",
            "--branch",
            "autonomous/issue-42",
            "--outcome",
            "blocked",
            "--pr",
            "17",
            "--reason",
            "waiting-for-review",
        ],
    ] {
        let output = fixture
            .configured_command()
            .args(args)
            .output()
            .expect("run malformed executor result");

        assert_eq!(output.status.code(), Some(2));
        assert!(
            String::from_utf8_lossy(&output.stdout).starts_with("{\"status\":\"malformed\""),
            "stdout={}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert_eq!(
            fs::read_to_string(&fixture.comments).expect("read comments after malformed input"),
            before
        );
    }
}

#[test]
fn executor_result_rejects_foreign_worker_or_branch_without_mutating_claim_state() {
    let fixture = ForegroundFixture::new();
    fixture.seed_claim("rust-foreground-conductor-1", "autonomous/issue-42");
    let before = fs::read_to_string(&fixture.comments).expect("read initial comments");

    for (worker_id, branch) in [
        ("foreign-worker", "autonomous/issue-42"),
        ("rust-foreground-conductor-1", "foreign/issue-42"),
    ] {
        let output = fixture
            .configured_command()
            .args([
                "autonomous",
                "executor-result",
                "--repo",
                "test/repo",
                "--issue",
                "42",
                "--worker-id",
                worker_id,
                "--branch",
                branch,
                "--outcome",
                "blocked",
                "--reason",
                "waiting-for-review",
            ])
            .output()
            .expect("record foreign executor result");

        assert_eq!(output.status.code(), Some(3));
        assert!(
            String::from_utf8_lossy(&output.stdout).starts_with("{\"status\":\"ownership_lost\""),
            "stdout={}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert_eq!(
            fs::read_to_string(&fixture.comments).expect("read comments after ownership loss"),
            before
        );
    }
}

#[test]
fn executor_result_rejects_success_without_exactly_one_closeout_report() {
    let fixture = ForegroundFixture::new();
    fixture.seed_claim("rust-foreground-conductor-1", "autonomous/issue-42");
    fixture.set_open_pull_requests(
        r#"[{"number":17,"body":"Closes #42","headRefName":"autonomous/issue-42","headRefOid":"0123456789abcdef0123456789abcdef01234567"}]"#,
    );
    fixture.persist_pass_receipt(
        "pass",
        EXECUTOR_CLAIM_ID,
        "rust-foreground-conductor-1",
        "autonomous/issue-42",
        EXECUTOR_COMMIT,
        false,
    );
    let before = fs::read_to_string(&fixture.comments).expect("read initial comments");

    let output = fixture
        .configured_command()
        .args([
            "autonomous",
            "executor-result",
            "--repo",
            "test/repo",
            "--issue",
            "42",
            "--worker-id",
            "rust-foreground-conductor-1",
            "--branch",
            "autonomous/issue-42",
            "--outcome",
            "succeeded",
            "--pr",
            "17",
            "--claim-id",
            EXECUTOR_CLAIM_ID,
            "--premerge-receipt",
            PREMERGE_RECEIPT,
        ])
        .output()
        .expect("record unverified success");

    assert_eq!(output.status.code(), Some(20));
    assert!(
        String::from_utf8_lossy(&output.stdout).starts_with("{\"status\":\"blocked\""),
        "stdout={}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(
        fs::read_to_string(&fixture.comments).expect("read comments after unverified success"),
        before
    );
}

#[test]
fn executor_result_accepts_a_claim_owner_success_with_linked_closeout_evidence() {
    let fixture = ForegroundFixture::new();
    fixture.seed_claim("rust-foreground-conductor-1", "autonomous/issue-42");
    let before = fixture.claim_record();
    fixture.set_open_pull_requests(
        r#"[{"number":17,"body":"Closes #42\n\n## Closeout report\n\nResult: shipped","headRefName":"autonomous/issue-42","headRefOid":"0123456789abcdef0123456789abcdef01234567"}]"#,
    );
    fixture.persist_pass_receipt(
        "pass",
        EXECUTOR_CLAIM_ID,
        "rust-foreground-conductor-1",
        "autonomous/issue-42",
        EXECUTOR_COMMIT,
        false,
    );

    let output = fixture
        .configured_command()
        .args([
            "autonomous",
            "executor-result",
            "--repo",
            "test/repo",
            "--issue",
            "42",
            "--worker-id",
            "rust-foreground-conductor-1",
            "--branch",
            "autonomous/issue-42",
            "--outcome",
            "succeeded",
            "--pr",
            "17",
            "--claim-id",
            EXECUTOR_CLAIM_ID,
            "--premerge-receipt",
            PREMERGE_RECEIPT,
        ])
        .output()
        .expect("record verified success");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "{\"status\":\"accepted\",\"repo\":\"test/repo\",\"issue\":42,\"outcome\":\"succeeded\",\"pr\":17}\n"
    );
    let record = fixture.claim_record();
    assert_eq!(record, before);
    let comments = parse_remote_comments_json(
        &fs::read_to_string(&fixture.comments).expect("read executor evidence"),
    )
    .expect("parse executor evidence comments");
    assert!(comments.iter().any(|comment| comment.body.contains(&format!(
            "\"claim_id\":\"{EXECUTOR_CLAIM_ID}\",\"commit\":\"{EXECUTOR_COMMIT}\",\"premerge_receipt\":\"{PREMERGE_RECEIPT}\""
        ))));
}

#[test]
fn supervised_executor_child_accepts_a_typed_success_result() {
    let fixture = ForegroundFixture::new();
    fixture.seed_claim("rust-foreground-conductor-1", "autonomous/issue-42");
    fixture.set_valid_open_pull_request(EXECUTOR_COMMIT);
    fixture.persist_pass_receipt(
        "pass",
        EXECUTOR_CLAIM_ID,
        "rust-foreground-conductor-1",
        "autonomous/issue-42",
        EXECUTOR_COMMIT,
        false,
    );
    fs::create_dir_all(fixture.repo_dir.join(".autospec")).expect("executor state directory");
    fs::write(
        fixture.repo_dir.join(".autospec/executor-result.json"),
        format!(
            "{{\"repo\":\"test/repo\",\"issue\":42,\"worker_id\":\"rust-foreground-conductor-1\",\"branch\":\"autonomous/issue-42\",\"claim_id\":\"{EXECUTOR_CLAIM_ID}\",\"invocation_id\":\"42-claim-42\",\"expected_commit\":\"{EXECUTOR_COMMIT}\",\"outcome\":\"succeeded\",\"pr\":17,\"premerge_receipt\":\"{PREMERGE_RECEIPT}\"}}"
        ),
    )
    .expect("typed executor result");
    let output = fixture
        .configured_command()
        .args([
            "autonomous",
            "executor-child",
            "--repo",
            "test/repo",
            "--issue",
            "42",
            "--worker-id",
            "rust-foreground-conductor-1",
            "--branch",
            "autonomous/issue-42",
            "--claim-id",
            EXECUTOR_CLAIM_ID,
            "--expected-commit",
            EXECUTOR_COMMIT,
            "--invocation-id",
            "42-claim-42",
        ])
        .output()
        .expect("run typed executor child");
    assert!(output.status.success(), "stderr={}", String::from_utf8_lossy(&output.stderr));
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"status\":\"accepted\""));
}

#[test]
fn executor_result_success_requires_receipt_binding_flags_and_other_outcomes_reject_them() {
    let fixture = ForegroundFixture::new();
    fixture.seed_claim("rust-foreground-conductor-1", "autonomous/issue-42");
    let before = fs::read_to_string(&fixture.comments).expect("read initial comments");

    for args in [
        vec![
            "autonomous",
            "executor-result",
            "--repo",
            "test/repo",
            "--issue",
            "42",
            "--worker-id",
            "rust-foreground-conductor-1",
            "--branch",
            "autonomous/issue-42",
            "--outcome",
            "succeeded",
            "--pr",
            "17",
        ],
        vec![
            "autonomous",
            "executor-result",
            "--repo",
            "test/repo",
            "--issue",
            "42",
            "--worker-id",
            "rust-foreground-conductor-1",
            "--branch",
            "autonomous/issue-42",
            "--outcome",
            "blocked",
            "--reason",
            "waiting",
            "--claim-id",
            EXECUTOR_CLAIM_ID,
            "--premerge-receipt",
            PREMERGE_RECEIPT,
        ],
    ] {
        let output = fixture
            .configured_command()
            .args(args)
            .output()
            .expect("run receipt-binding protocol rejection");
        assert_eq!(output.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&output.stdout).starts_with("{\"status\":\"malformed\""));
        assert_eq!(fs::read_to_string(&fixture.comments).unwrap(), before);
    }
}

#[test]
fn executor_result_rejects_nonpass_quarantined_and_foreign_premerge_receipts() {
    for (decision, claim_id, worker_id, branch, quarantine) in [
        (
            "blocked",
            EXECUTOR_CLAIM_ID,
            "rust-foreground-conductor-1",
            "autonomous/issue-42",
            false,
        ),
        (
            "pass",
            EXECUTOR_CLAIM_ID,
            "rust-foreground-conductor-1",
            "autonomous/issue-42",
            true,
        ),
        (
            "pass",
            "other-claim",
            "rust-foreground-conductor-1",
            "autonomous/issue-42",
            false,
        ),
        (
            "pass",
            EXECUTOR_CLAIM_ID,
            "other-worker",
            "autonomous/issue-42",
            false,
        ),
        (
            "pass",
            EXECUTOR_CLAIM_ID,
            "rust-foreground-conductor-1",
            "other/issue-42",
            false,
        ),
    ] {
        let fixture = ForegroundFixture::new();
        fixture.seed_claim("rust-foreground-conductor-1", "autonomous/issue-42");
        fixture.set_valid_open_pull_request(EXECUTOR_COMMIT);
        fixture.persist_pass_receipt(
            decision,
            claim_id,
            worker_id,
            branch,
            EXECUTOR_COMMIT,
            quarantine,
        );
        let before = fs::read_to_string(&fixture.comments).expect("read claim");

        let output = fixture
            .explicit_success_command()
            .output()
            .expect("submit invalid receipt");

        assert_eq!(output.status.code(), Some(20));
        assert!(String::from_utf8_lossy(&output.stdout).contains("success_evidence_unavailable"));
        assert_eq!(fs::read_to_string(&fixture.comments).unwrap(), before);
    }
}

#[test]
fn executor_result_rejects_pr_head_mismatch_and_successor_claim_replay() {
    let mismatch = ForegroundFixture::new();
    mismatch.seed_claim("rust-foreground-conductor-1", "autonomous/issue-42");
    mismatch.set_valid_open_pull_request("ffffffffffffffffffffffffffffffffffffffff");
    mismatch.persist_pass_receipt(
        "pass",
        EXECUTOR_CLAIM_ID,
        "rust-foreground-conductor-1",
        "autonomous/issue-42",
        EXECUTOR_COMMIT,
        false,
    );
    assert_eq!(
        mismatch
            .explicit_success_command()
            .output()
            .unwrap()
            .status
            .code(),
        Some(20)
    );

    let replay = ForegroundFixture::new();
    replay.seed_claim_with_id(
        "rust-foreground-conductor-1",
        "autonomous/issue-42",
        "successor-claim",
    );
    replay.set_valid_open_pull_request(EXECUTOR_COMMIT);
    replay.persist_pass_receipt(
        "pass",
        EXECUTOR_CLAIM_ID,
        "rust-foreground-conductor-1",
        "autonomous/issue-42",
        EXECUTOR_COMMIT,
        false,
    );
    let output = replay
        .explicit_success_command()
        .output()
        .expect("replay prior receipt");
    assert_eq!(output.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&output.stdout).contains("claim_ownership_lost"));
}

#[test]
fn executor_result_rejects_a_self_declared_noncanonical_lane_digest() {
    const FORGED_LANE: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    let fixture = ForegroundFixture::new();
    fixture.seed_claim("rust-foreground-conductor-1", "autonomous/issue-42");
    fixture.set_valid_open_pull_request(EXECUTOR_COMMIT);
    fixture.persist_pass_receipt_in_lane(
        "pass",
        EXECUTOR_CLAIM_ID,
        "rust-foreground-conductor-1",
        "autonomous/issue-42",
        EXECUTOR_COMMIT,
        false,
        FORGED_LANE,
    );
    let before = fs::read_to_string(&fixture.comments).expect("read claim before forged receipt");

    let output = fixture
        .success_command()
        .output()
        .expect("submit self-declared forged lane receipt");

    assert_eq!(output.status.code(), Some(20));
    assert!(String::from_utf8_lossy(&output.stdout).contains("success_evidence_unavailable"));
    assert_eq!(fs::read_to_string(&fixture.comments).unwrap(), before);
}

#[test]
fn executor_result_records_an_owner_verified_retryable_outcome() {
    let fixture = ForegroundFixture::new();
    fixture.seed_claim("rust-foreground-conductor-1", "autonomous/issue-42");
    let before = fixture.claim_record();

    let output = fixture
        .configured_command()
        .args([
            "autonomous",
            "executor-result",
            "--repo",
            "test/repo",
            "--issue",
            "42",
            "--worker-id",
            "rust-foreground-conductor-1",
            "--branch",
            "autonomous/issue-42",
            "--outcome",
            "retryable",
            "--reason",
            "transient-network-error",
        ])
        .output()
        .expect("record retryable executor result");

    assert_eq!(
        output.status.code(),
        Some(10),
        "stdout={} stderr={} calls={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
        fs::read_to_string(&fixture.calls).expect("read GitHub calls")
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "{\"status\":\"retryable\",\"repo\":\"test/repo\",\"issue\":42,\"outcome\":\"retryable\",\"reason\":\"transient-network-error\"}\n"
    );
    assert_eq!(fixture.claim_record(), before);
}

#[test]
fn executor_result_rejects_an_expired_matching_lease_without_mutating_claim() {
    let fixture = ForegroundFixture::new();
    fixture.seed_claim_at(
        "rust-foreground-conductor-1",
        "autonomous/issue-42",
        "2000-01-01T00:00:00Z",
    );
    let before = fs::read_to_string(&fixture.comments).expect("read expired claim");
    fixture.set_open_pull_requests(
        r#"[{"number":17,"body":"Closes #42\n\n## Closeout report\n\nResult: shipped","headRefName":"autonomous/issue-42","headRefOid":"0123456789abcdef0123456789abcdef01234567"}]"#,
    );

    let output = fixture
        .explicit_success_command()
        .output()
        .expect("submit expired result");

    assert_eq!(output.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&output.stdout).starts_with("{\"status\":\"ownership_lost\""));
    assert_eq!(
        fs::read_to_string(&fixture.comments).expect("read expired claim after result"),
        before
    );
}

#[test]
fn executor_result_rejects_a_terminal_claim_without_mutating_claim() {
    let fixture = ForegroundFixture::new();
    fixture.seed_claim("rust-foreground-conductor-1", "autonomous/issue-42");
    fixture.append_terminal_merged_marker();
    let before = fs::read_to_string(&fixture.comments).expect("read terminal claim");
    fixture.set_open_pull_requests(
        r#"[{"number":17,"body":"Closes #42\n\n## Closeout report\n\nResult: shipped","headRefName":"autonomous/issue-42","headRefOid":"0123456789abcdef0123456789abcdef01234567"}]"#,
    );

    let output = fixture
        .explicit_success_command()
        .output()
        .expect("submit terminal result");

    assert_eq!(output.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&output.stdout).starts_with("{\"status\":\"ownership_lost\""));
    assert_eq!(
        fs::read_to_string(&fixture.comments).expect("read terminal claim after result"),
        before
    );
}

#[test]
fn executor_result_rejects_a_valid_closeout_pr_from_a_foreign_branch() {
    let fixture = ForegroundFixture::new();
    fixture.seed_claim("rust-foreground-conductor-1", "autonomous/issue-42");
    let before = fs::read_to_string(&fixture.comments).expect("read claimed run state");
    fixture.set_open_pull_requests(
        r#"[{"number":17,"body":"Closes #42\n\n## Closeout report\n\nResult: shipped","headRefName":"foreign/issue-42","headRefOid":"0123456789abcdef0123456789abcdef01234567"}]"#,
    );

    let output = fixture
        .explicit_success_command()
        .output()
        .expect("submit foreign branch result");

    assert_eq!(output.status.code(), Some(20));
    assert!(String::from_utf8_lossy(&output.stdout)
        .contains("\"reason\":\"success_evidence_unavailable\""));
    assert_eq!(
        fs::read_to_string(&fixture.comments).expect("read claim after foreign branch"),
        before
    );
}

#[test]
fn executor_result_does_not_overwrite_a_takeover_after_validating_the_result() {
    let fixture = ForegroundFixture::new();
    fixture.seed_claim("rust-foreground-conductor-1", "autonomous/issue-42");
    fixture.set_open_pull_requests(
        r#"[{"number":17,"body":"Closes #42\n\n## Closeout report\n\nResult: shipped","headRefName":"autonomous/issue-42","headRefOid":"0123456789abcdef0123456789abcdef01234567"}]"#,
    );

    let output = fixture
        .explicit_success_command()
        .env("AUTOSPEC_FOREGROUND_STEAL_ON_RESULT_VALIDATION", "1")
        .output()
        .expect("submit result during takeover");

    assert_eq!(output.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&output.stdout).starts_with("{\"status\":\"ownership_lost\""));
    let record = fixture.claim_record();
    assert_eq!(record.worker_id, "foreign-worker");
    assert_eq!(record.branch, "foreign/issue-42");
}

#[test]
fn executor_result_reports_a_post_write_confirmation_failure_as_blocked() {
    let fixture = ForegroundFixture::new();
    fixture.seed_claim("rust-foreground-conductor-1", "autonomous/issue-42");
    fixture.set_open_pull_requests(
        r#"[{"number":17,"body":"Closes #42\n\n## Closeout report\n\nResult: shipped","headRefName":"autonomous/issue-42","headRefOid":"0123456789abcdef0123456789abcdef01234567"}]"#,
    );

    let output = fixture
        .explicit_success_command()
        .env("AUTOSPEC_FOREGROUND_FAIL_EVIDENCE_CONFIRM", "1")
        .output()
        .expect("submit result with failed confirmation");

    assert_eq!(output.status.code(), Some(20));
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("\"reason\":\"result_recording_failed\"")
    );
    assert_eq!(fixture.claim_record().step, "claimed");
    assert!(fs::read_to_string(&fixture.comments)
        .expect("read persisted but unconfirmed evidence")
        .contains("<!-- autospec-executor-result:begin -->"));
}

#[test]
fn executor_result_reports_a_pre_write_failure_as_blocked() {
    let fixture = ForegroundFixture::new();
    fixture.seed_claim("rust-foreground-conductor-1", "autonomous/issue-42");
    fixture.set_open_pull_requests(
        r#"[{"number":17,"body":"Closes #42\n\n## Closeout report\n\nResult: shipped","headRefName":"autonomous/issue-42","headRefOid":"0123456789abcdef0123456789abcdef01234567"}]"#,
    );
    let before = fs::read_to_string(&fixture.comments).expect("read claim before failed evidence");

    let output = fixture
        .explicit_success_command()
        .env("AUTOSPEC_FOREGROUND_FAIL_EVIDENCE_CREATE", "1")
        .output()
        .expect("submit result with failed evidence write");

    assert_eq!(output.status.code(), Some(20));
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("\"reason\":\"result_recording_failed\"")
    );
    assert_eq!(
        fs::read_to_string(&fixture.comments).expect("read claim after failed evidence write"),
        before
    );
}

#[test]
fn foreground_fixture_directories_are_unique_within_one_process() {
    let first = ForegroundFixture::new();
    let second = ForegroundFixture::new();

    assert_ne!(first.root, second.root);
}

struct ForegroundFixture {
    root: PathBuf,
    repo_dir: PathBuf,
    bin: PathBuf,
    mode: PathBuf,
    comments: PathBuf,
    pull_requests: PathBuf,
    calls: PathBuf,
    operator: PathBuf,
    state: PathBuf,
    health: PathBuf,
    heartbeats: PathBuf,
}

impl ForegroundFixture {
    fn new() -> Self {
        let root = temp_dir("autospec-foreground-conductor");
        let repo_dir = root.join("repo");
        let bin = root.join("bin");
        let mode = root.join("mode");
        let comments = root.join("comments.json");
        let pull_requests = root.join("pull-requests.json");
        let calls = root.join("gh.log");
        let operator = root.join("operator");
        let state = root.join("state");
        let health = root.join("health");
        let heartbeats = root.join("heartbeats");
        fs::create_dir_all(&repo_dir).expect("create repo directory");
        fs::create_dir_all(&bin).expect("create fake bin");
        fs::write(&mode, "unreviewed\n").expect("write mode");
        fs::write(&comments, "[]\n").expect("write comments");
        fs::write(&pull_requests, "[]\n").expect("write pull requests");
        write_executable(
            &bin.join("gh"),
            r####"#!/bin/sh
set -eu
printf '%s\n' "$@" >> "$AUTOSPEC_FOREGROUND_CALLS"
mode="$(cat "$AUTOSPEC_FOREGROUND_MODE")"
issue() {
  if [ "$mode" = unreviewed ]; then
    printf '%s\n' '{"number":42,"title":"Add Rust foreground","body":"## Goal\n\nAdd the foreground adapter.","labels":[{"name":"auto-implement"}],"author":{"login":"agent"}}'
  else
    printf '%s\n' '{"number":42,"title":"Add Rust foreground","body":"## Safety review\n\n<!-- autospec-safety:begin -->\n- **decision:** `SAFETY_PASS`\n<!-- autospec-safety:end -->","labels":[{"name":"auto-implement"},{"name":"safety:reviewed"}],"author":{"login":"agent"}}'
  fi
}
claim_issue() {
  if [ "$mode" = claimed ]; then labels='["in-progress-by-bot","safety:reviewed"]'; else labels='["auto-implement","safety:reviewed"]'; fi
  printf '%s\n' "{\"labels\":$labels,\"title\":\"Add Rust foreground\",\"body\":\"## Safety review\\n\\n<!-- autospec-safety:begin -->\\n- **decision:** \`SAFETY_PASS\`\\n<!-- autospec-safety:end -->\",\"author\":\"agent\"}"
}
if [ "$1" = api ] && [ "$2" = graphql ]; then
  printf '%s\n' '{"items":[],"page_info":{"has_next_page":false,"end_cursor":null}}'
  exit 0
fi
if [ "$1" = repo ] && [ "$2" = view ]; then
  if [ "${AUTOSPEC_FOREGROUND_NO_DEFAULT_BRANCH:-0}" = 1 ]; then
    printf '\n'
  else
    printf '%s\n' main
  fi
  exit 0
fi
if [ "$1" = api ] && [ "$2" = repos/test/repo/branches/main ]; then
  printf '%s\n' '{}'
  exit 0
fi
if [ "$1" = api ] && [ "$2" = repos/test/repo/branches/master_ai ]; then
  printf '%s\n' '{}'
  exit 0
fi
if [ "$1" = api ] && [ "$2" = repos/test/repo/branches/missing ]; then
  exit 1
fi
if [ "$1" = api ] && { [ "$2" = repos/test/repo/commits/main/status ] || [ "$2" = repos/test/repo/commits/master_ai/status ]; }; then
  if [ "${AUTOSPEC_FOREGROUND_HEALTH_CASE:-success}" = ignored_failure ]; then
    printf '%s\n' '{"state":"failure","total_count":1,"statuses":[{"context":"Unit Tests","state":"failure"}]}'
  else
    printf '%s\n' '{"state":"success","total_count":1,"statuses":[{"context":"ci","state":"success"}]}'
  fi
  exit 0
fi
if [ "$1" = api ]; then
  endpoint=""
  for value in "$@"; do case "$value" in repos/*) endpoint="$value" ;; esac; done
  case "$endpoint" in
    repos/test/repo/issues\?*)
      case "$endpoint" in
        *labels=in-progress-by-bot*) printf '%s\n' '{"raw_count":0,"items":[]}' ;;
        *labels=auto-implement*)
          if [ "${AUTOSPEC_FOREGROUND_QUEUE_FAILURE:-0}" = 1 ]; then exit 1; fi
          if [ "${AUTOSPEC_FOREGROUND_CORRUPT_SPEND_AFTER_QUEUE:-0}" = 1 ]; then
            mkdir -p "$AUTOSPEC_AUTONOMOUS_SPEND_DIR/test_repo"
            printf '%s\n' '{malformed' > "$AUTOSPEC_AUTONOMOUS_SPEND_DIR/test_repo/spend.json"
          fi
          if [ "${AUTOSPEC_FOREGROUND_EMPTY_QUEUE:-0}" = 1 ]; then
            printf '%s\n' '{"raw_count":0,"items":[]}'
          else
            printf '%s' '{"raw_count":1,"items":['; issue; printf '%s\n' ']}'
          fi ;;
        *) printf '%s\n' '{"raw_count":0,"items":[]}' ;;
      esac
      exit 0 ;;
    repos/test/repo/issues/42/comments)
      if [ "${AUTOSPEC_FOREGROUND_FAIL_EVIDENCE_CONFIRM:-0}" = 1 ] && grep -q '<!-- autospec-executor-result:begin -->' "$AUTOSPEC_FOREGROUND_COMMENTS"; then
        exit 1
      fi
      cat "$AUTOSPEC_FOREGROUND_COMMENTS"
      exit 0 ;;
    repos/test/repo/issues/42/labels) printf 'reviewed\n' > "$AUTOSPEC_FOREGROUND_MODE"; exit 0 ;;
    repos/test/repo/issues/42)
      if printf '%s\n' "$@" | grep -q PATCH; then printf 'reviewed\n' > "$AUTOSPEC_FOREGROUND_MODE"; else issue; fi
      exit 0 ;;
    repos/test/repo/issues/comments/100)
      body=""
      for value in "$@"; do case "$value" in body=*) body="${value#body=}" ;; esac; done
      if [ "${AUTOSPEC_FOREGROUND_STEAL_ON_OUTCOME:-0}" = 1 ] && printf '%s' "$body" | grep -q executor_pending; then
        body='<!-- autospec-run-state:begin -->
{"schema":1,"repo":"test/repo","issue":42,"worker_id":"foreign-worker","state":"claimed","branch":"foreign/issue-42","pr":"","step":"claimed","paths":[],"claimed_at":"2026-07-15T00:00:00Z","updated_at":"2026-07-15T00:00:00Z","ttl_seconds":10800}
<!-- autospec-run-state:end -->'
      fi
      jq --arg body "$body" '.[0].body = $body | .[0].updated_at = "2026-07-15T00:00:00Z"' "$AUTOSPEC_FOREGROUND_COMMENTS" > "$AUTOSPEC_FOREGROUND_COMMENTS.tmp"
      mv "$AUTOSPEC_FOREGROUND_COMMENTS.tmp" "$AUTOSPEC_FOREGROUND_COMMENTS"
      exit 0 ;;
  esac
fi
if [ "$1" = issue ] && [ "$2" = view ]; then claim_issue; exit 0; fi
if [ "$1" = label ] && [ "$2" = create ]; then exit 0; fi
if [ "$1" = issue ] && [ "$2" = edit ]; then printf 'claimed\n' > "$AUTOSPEC_FOREGROUND_MODE"; exit 0; fi
if [ "$1" = issue ] && [ "$2" = comment ]; then
  body=""; shift 2
  while [ "$#" -gt 0 ]; do case "$1" in --body) body="$2"; shift 2 ;; *) shift ;; esac; done
  if [ "${AUTOSPEC_FOREGROUND_FAIL_EVIDENCE_CREATE:-0}" = 1 ] && printf '%s' "$body" | grep -q '<!-- autospec-executor-result:begin -->'; then
    exit 1
  fi
  jq --arg body "$body" '. + [{"id":100,"updated_at":"2026-07-15T00:00:00Z","body":$body}]' "$AUTOSPEC_FOREGROUND_COMMENTS" > "$AUTOSPEC_FOREGROUND_COMMENTS.tmp"
  mv "$AUTOSPEC_FOREGROUND_COMMENTS.tmp" "$AUTOSPEC_FOREGROUND_COMMENTS"
  exit 0
fi
if [ "$1" = pr ] && [ "$2" = list ]; then
  if [ "${AUTOSPEC_FOREGROUND_STEAL_ON_RESULT_VALIDATION:-0}" = 1 ]; then
    body='<!-- autospec-run-state:begin -->
{"schema":1,"repo":"test/repo","issue":42,"worker_id":"foreign-worker","state":"claimed","branch":"foreign/issue-42","pr":"","step":"claimed","paths":[],"claimed_at":"2026-07-15T00:00:00Z","updated_at":"2026-07-15T00:00:00Z","ttl_seconds":10800}
<!-- autospec-run-state:end -->'
    jq --arg body "$body" '.[0].body = $body | .[0].updated_at = "2026-07-15T00:00:00Z"' "$AUTOSPEC_FOREGROUND_COMMENTS" > "$AUTOSPEC_FOREGROUND_COMMENTS.tmp"
    mv "$AUTOSPEC_FOREGROUND_COMMENTS.tmp" "$AUTOSPEC_FOREGROUND_COMMENTS"
  fi
  cat "$AUTOSPEC_FOREGROUND_PULL_REQUESTS"
  exit 0
fi
printf 'unexpected gh invocation: %s\n' "$*" >&2
exit 1
"####,
        );
        Self {
            root,
            repo_dir,
            bin,
            mode,
            comments,
            pull_requests,
            calls,
            operator,
            state,
            health,
            heartbeats,
        }
    }

    fn command(&self) -> Command {
        let mut command = self.configured_command();
        command.args([
            "autonomous",
            "run-foreground",
            "--repo",
            "test/repo",
            "--repo-dir",
            self.repo_dir.to_str().expect("repo path"),
            "--branch",
            "main",
        ]);
        command
    }

    fn configured_command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_autospec"));
        command
            .current_dir(&self.repo_dir)
            .env("PATH", path_with(&self.bin))
            .env("AUTOSPEC_FOREGROUND_MODE", &self.mode)
            .env("AUTOSPEC_FOREGROUND_COMMENTS", &self.comments)
            .env("AUTOSPEC_FOREGROUND_PULL_REQUESTS", &self.pull_requests)
            .env("AUTOSPEC_FOREGROUND_CALLS", &self.calls)
            .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &self.operator)
            .env("AUTOSPEC_STATE_DIR", &self.state)
            .env("AUTOSPEC_AUTONOMOUS_SPEND_DIR", self.root.join("spend"))
            .env("AUTOSPEC_AUTONOMOUS_STATE_DIR", &self.health)
            .env("AUTOSPEC_HEARTBEAT_DIR", &self.heartbeats)
            .env("AUTOSPEC_CLAIM_CONFIRM_READS", "1")
            .env("AUTOSPEC_CLAIM_SETTLE_MILLIS", "0")
            .env_remove("AUTOSPEC_CONFIG_FILE");
        command
    }

    fn unbranched_foreground_command(&self) -> Command {
        let mut command = self.configured_command();
        command.args([
            "autonomous",
            "run-foreground",
            "--repo",
            "test/repo",
            "--repo-dir",
            self.repo_dir.to_str().expect("repo path"),
        ]);
        command
    }

    fn unbranched_main_health_command(&self) -> Command {
        let mut command = self.configured_command();
        command.args([
            "autonomous",
            "main-health",
            "--repo",
            "test/repo",
            "--repo-dir",
            self.repo_dir.to_str().expect("repo path"),
            "--json",
        ]);
        command
    }

    fn detached_command(&self, subcommand: &str) -> Command {
        let mut command = self.configured_command();
        command.args([
            "autonomous",
            subcommand,
            "--repo",
            "test/repo",
            "--repo-dir",
            self.repo_dir.to_str().expect("repo path"),
            "--json",
        ]);
        command
    }

    fn write_autonomous_config(&self, source: &str) {
        let config_dir = self.repo_dir.join(".autospec");
        fs::create_dir_all(&config_dir).expect("create autonomous config directory");
        fs::write(config_dir.join("autonomous.yml"), source).expect("write autonomous config");
    }

    fn main_health_observations_path(&self) -> PathBuf {
        self.health
            .join("test_repo")
            .join("main-health-observations.jsonl")
    }

    fn initialize_git_remote(&self) {
        let init = Command::new("git")
            .args([
                "init",
                "-q",
                self.repo_dir.to_str().expect("repo directory"),
            ])
            .output()
            .expect("initialize git repository");
        assert!(init.status.success());
        let remote = Command::new("git")
            .args([
                "-C",
                self.repo_dir.to_str().expect("repo directory"),
                "remote",
                "add",
                "origin",
                "https://github.com/test/repo.git",
            ])
            .output()
            .expect("set git remote");
        assert!(remote.status.success());
    }

    fn run_foreground(&self) -> std::process::Output {
        self.command().output().expect("run foreground")
    }

    fn seed_claim(&self, worker_id: &str, branch: &str) {
        self.seed_claim_with_id(worker_id, branch, EXECUTOR_CLAIM_ID);
    }

    fn seed_claim_with_id(&self, worker_id: &str, branch: &str, claim_id: &str) {
        self.seed_claim_state_with_id(
            worker_id,
            branch,
            "claimed",
            &fresh_iso_timestamp(),
            claim_id,
        );
    }

    fn seed_claim_at(&self, worker_id: &str, branch: &str, updated_at: &str) {
        self.seed_claim_state(worker_id, branch, "claimed", updated_at);
    }

    fn seed_claim_state(&self, worker_id: &str, branch: &str, state: &str, updated_at: &str) {
        self.seed_claim_state_with_id(worker_id, branch, state, updated_at, EXECUTOR_CLAIM_ID);
    }

    fn seed_claim_state_with_id(
        &self,
        worker_id: &str,
        branch: &str,
        state: &str,
        updated_at: &str,
        claim_id: &str,
    ) {
        let record = RunStateRecord::new(
            "test/repo",
            42,
            worker_id,
            state,
            branch,
            "",
            "claimed",
            Vec::new(),
            updated_at,
            updated_at,
            10_800,
        )
        .with_claim_id(claim_id);
        fs::write(
            &self.comments,
            format!(
                "[{{\"id\":100,\"updated_at\":{:?},\"body\":{:?}}}]\n",
                updated_at,
                record.to_marked_comment(),
            ),
        )
        .expect("seed claimed run-state comment");
    }

    fn seed_malformed_claim(&self) {
        let body = "<!-- autospec-run-state:begin -->\nnot-json\n<!-- autospec-run-state:end -->";
        fs::write(
            &self.comments,
            format!(
                "[{{\"id\":100,\"updated_at\":\"{}\",\"body\":{:?}}}]\n",
                fresh_iso_timestamp(),
                body
            ),
        )
        .expect("seed malformed run-state comment");
    }

    fn set_open_pull_requests(&self, pull_requests: &str) {
        fs::write(&self.pull_requests, pull_requests).expect("write open pull requests");
    }

    fn set_valid_open_pull_request(&self, commit: &str) {
        self.set_open_pull_requests(&format!(
            "[{{\"number\":17,\"body\":\"Closes #42\\n\\n## Closeout report\\n\\nResult: shipped\",\"headRefName\":\"autonomous/issue-42\",\"headRefOid\":\"{commit}\"}}]"
        ));
    }

    fn persist_pass_receipt(
        &self,
        decision: &str,
        claim_id: &str,
        worker_id: &str,
        branch: &str,
        commit: &str,
        quarantine: bool,
    ) {
        let lane_digest =
            PremergeLaneIdentity::new("test/repo", 42, worker_id, claim_id, branch, commit)
                .expect("valid premerge receipt lane")
                .lane_digest();
        self.persist_pass_receipt_in_lane(
            decision,
            claim_id,
            worker_id,
            branch,
            commit,
            quarantine,
            &lane_digest,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn persist_pass_receipt_in_lane(
        &self,
        decision: &str,
        claim_id: &str,
        worker_id: &str,
        branch: &str,
        commit: &str,
        quarantine: bool,
        lane_digest: &str,
    ) {
        let lane = self
            .operator
            .join("test_repo/premerge/lanes")
            .join(lane_digest);
        let decisions = lane.join("decisions");
        fs::create_dir_all(&decisions).expect("create premerge receipt directory");
        let receipt = format!(
            "{{\"schema\":1,\"decision\":\"{decision}\",\"repo\":\"test/repo\",\"issue\":42,\"worker_id\":\"{worker_id}\",\"claim_id\":\"{claim_id}\",\"branch\":\"{branch}\",\"commit\":\"{commit}\",\"lane_digest\":\"{lane_digest}\",\"evidence_digest\":\"{PREMERGE_RECEIPT}\",\"reason\":\"\",\"finding_codes\":[]}}\n"
        );
        fs::write(decisions.join(format!("{PREMERGE_RECEIPT}.json")), &receipt)
            .expect("persist immutable Pass receipt");
        if quarantine {
            fs::write(lane.join("quarantine.json"), receipt).expect("persist lane quarantine");
        }
    }

    fn explicit_success_command(&self) -> Command {
        let lane_digest = PremergeLaneIdentity::new(
            "test/repo",
            42,
            "rust-foreground-conductor-1",
            EXECUTOR_CLAIM_ID,
            "autonomous/issue-42",
            EXECUTOR_COMMIT,
        )
        .expect("valid executor lane")
        .lane_digest();
        let receipt = self
            .operator
            .join("test_repo/premerge/lanes")
            .join(lane_digest)
            .join("decisions")
            .join(format!("{PREMERGE_RECEIPT}.json"));
        if !receipt.exists() {
            self.persist_pass_receipt(
                "pass",
                EXECUTOR_CLAIM_ID,
                "rust-foreground-conductor-1",
                "autonomous/issue-42",
                EXECUTOR_COMMIT,
                false,
            );
        }
        self.success_command()
    }

    fn success_command(&self) -> Command {
        let mut command = self.configured_command();
        command.args([
            "autonomous",
            "executor-result",
            "--repo",
            "test/repo",
            "--issue",
            "42",
            "--worker-id",
            "rust-foreground-conductor-1",
            "--branch",
            "autonomous/issue-42",
            "--outcome",
            "succeeded",
            "--pr",
            "17",
            "--claim-id",
            EXECUTOR_CLAIM_ID,
            "--premerge-receipt",
            PREMERGE_RECEIPT,
        ]);
        command
    }

    fn append_terminal_merged_marker(&self) {
        let terminal = "<!-- autospec-run-terminal:begin -->\n{\"state\":\"merged\"}\n<!-- autospec-run-terminal:end -->";
        let existing = fs::read_to_string(&self.comments).expect("read comments before terminal");
        let prefix = existing.trim().strip_suffix(']').expect("comments array");
        let separator = if prefix.trim_end().ends_with('[') {
            ""
        } else {
            ","
        };
        fs::write(
            &self.comments,
            format!(
                "{prefix}{separator}{{\"id\":101,\"updated_at\":\"2026-07-15T00:00:00Z\",\"body\":{:?}}}]\n",
                terminal
            ),
        )
        .expect("append terminal marker");
    }

    fn claim_record(&self) -> RunStateRecord {
        let comments = parse_remote_comments_json(
            &fs::read_to_string(&self.comments).expect("read claimed run-state comments"),
        )
        .expect("parse claimed run-state comments");
        select_run_state(&comments, "test/repo", 42)
            .expect("select claimed run-state")
            .record
    }

    fn state_path(&self) -> PathBuf {
        self.operator
            .join("test_repo")
            .join("foreground-conductor-repository.json")
    }

    fn slice_state_path(&self, issue_scope: &str) -> PathBuf {
        let encoded = issue_scope
            .bytes()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        self.operator
            .join("test_repo")
            .join(format!("foreground-conductor-slice-{encoded}.json"))
    }

    fn read_state(&self) -> ConductorState {
        let source = fs::read_to_string(self.state_path()).expect("read foreground state");
        ConductorState::parse_json(&source).expect("parse foreground state")
    }

    fn waterfall_dir(&self) -> PathBuf {
        self.operator.join("test_repo/waterfall")
    }

    fn waterfall_state_path(&self) -> PathBuf {
        self.waterfall_dir().join("waterfall-state.json")
    }

    fn read_waterfall_state(&self) -> WaterfallState {
        let source = fs::read_to_string(self.waterfall_state_path()).expect("read waterfall state");
        WaterfallState::parse_json(&source, "test/repo").expect("parse waterfall state")
    }

    fn tier_one_receipt_path(&self, pass_id: u64) -> PathBuf {
        self.waterfall_dir()
            .join(format!("waterfall/{pass_id}/tier1.json"))
    }

    fn tier_one_evidence_path(&self, pass_id: u64, artifact: &str) -> PathBuf {
        self.waterfall_dir()
            .join(format!("waterfall/{pass_id}/tier1/{artifact}.json"))
    }

    fn read_tier_one_receipt(&self, pass_id: u64) -> TierReceipt {
        let source = fs::read_to_string(self.tier_one_receipt_path(pass_id))
            .expect("read Tier 1 waterfall receipt");
        TierReceipt::parse_json(&source, "test/repo", pass_id, NoWorkTier::Tier1)
            .expect("parse Tier 1 waterfall receipt")
    }
}

fn json_string_field(document: &str, field: &str) -> String {
    let marker = format!("\"{field}\":\"");
    document
        .split_once(&marker)
        .and_then(|(_, remainder)| remainder.split_once('"'))
        .map(|(value, _)| value.to_string())
        .unwrap_or_else(|| panic!("missing string field {field} in {document}"))
}

impl Drop for ForegroundFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

static TEMP_DIR_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn fresh_iso_timestamp() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_secs();
    let days = (seconds / 86_400) as i64;
    let seconds_of_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        seconds_of_day / 3_600,
        (seconds_of_day % 3_600) / 60,
        seconds_of_day % 60
    )
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let days = days + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    (year + i64::from(month <= 2), month as u32, day as u32)
}

fn temp_dir(prefix: &str) -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let sequence = TEMP_DIR_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "{prefix}-{}-{suffix}-{sequence}",
        std::process::id()
    ));
    fs::create_dir_all(&path).expect("create fixture directory");
    path
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

fn wait_for_file_contents(path: &Path, expected: &str) {
    for _ in 0..300 {
        if fs::read_to_string(path).is_ok_and(|contents| contents.contains(expected)) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("{} did not contain {expected}", path.display());
}
