use autospec_core::autonomous::no_work::NoWorkTier;
use autospec_core::autonomous::premerge::PremergeLaneIdentity;
use autospec_core::autonomous::waterfall::{sha256_hex, TierReceipt, TierStatus, WaterfallState};
use autospec_core::claim::{parse_remote_comments_json, parse_run_state_comment, RunStateRecord};
use autospec_core::coordination::{
    ConductorEvent, ConductorOutcome, ConductorPhase, ConductorScope, ConductorState,
};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

#[path = "support/autonomous_accountability_acquisition.rs"] mod autonomous_accountability_acquisition;
const EXECUTOR_CLAIM_ID: &str = "claim-generation-42";
const EXECUTOR_COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
const PREMERGE_RECEIPT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
static REAL_BRIDGE_E2E: Mutex<()> = Mutex::new(());

#[cfg(target_os = "linux")] #[path = "support/foreground_fixture_git.rs"] mod foreground_fixture_git;
#[cfg(target_os = "linux")] use foreground_fixture_git::seed_preserved_issue_branch;
fn workspace_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
}

fn is_generation_invocation_path(path: &Path, issue: u64) -> bool {
    let Some(stem) = path.file_stem().and_then(|value| value.to_str()) else {
        return false;
    };
    let Some(generation) = stem.strip_prefix(&format!("issue-{issue}-")) else {
        return false;
    };
    path.extension().and_then(|value| value.to_str()) == Some("json")
        && generation.len() == 16
        && generation
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
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
fn autonomous_help_documents_all_start_modes() {
    let output = ForegroundFixture::new()
        .configured_command()
        .args(["autonomous", "--help"])
        .output()
        .expect("print autonomous help");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--follow"));
    assert!(stdout.contains("--detach"));
    assert!(stdout.contains("--foreground"));
    assert!(stdout.contains("default"));
    assert!(stdout.contains("detached and supervised"));
    assert!(stdout.contains("Ctrl-C detaches only the follower"));
    assert!(stdout.contains("caller-owned"));
}

#[test]
fn start_rejects_conflicting_launch_modes_before_mutation() {
    let fixture = ForegroundFixture::new();
    let output = fixture
        .configured_command()
        .args(["autonomous", "start", "--follow", "--foreground"])
        .output()
        .expect("reject conflicting modes");
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("--follow, --detach, and --foreground are mutually exclusive"));
    assert!(!fixture.operator.exists());
}

#[test]
fn start_rejects_follow_force_before_mutation() {
    let fixture = ForegroundFixture::new();
    let output = fixture
        .configured_command()
        .args(["autonomous", "start", "--follow", "--force"])
        .output()
        .expect("reject follow with force");
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains(
        "--force cannot be combined with --follow; use autospec autonomous restart --force"
    ));
    assert!(!fixture.operator.exists());
}

#[test]
fn launch_modes_reject_non_start_subcommands_before_mutation() {
    let fixture = ForegroundFixture::new();
    let output = fixture
        .configured_command()
        .args(["autonomous", "status", "--follow"])
        .output()
        .expect("reject launch mode on status");
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("launch modes are valid only with autospec autonomous start, not status"));
    assert!(!fixture.operator.exists());
}

#[test]
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn restart_dry_run_is_strictly_read_only() {
    let fixture = ForegroundFixture::new();
    git_fixture(&fixture.repo_dir, &["init", "-q"]);
    fs::write(&fixture.accountability, "CLOSED\n").expect("seed closed accountability epic");
    fs::create_dir_all(fixture.scoped_dir()).expect("create autonomous scope");
    fs::write(
        fixture.scoped_stop_sentinel(),
        "immediate\n2026-08-14T00:00:00Z test@localhost\n",
    )
    .expect("seed immediate-stop sentinel");
    let mut conductor_command = Command::new("sh");
    conductor_command
        .args(["-c", "while :; do sleep 1; done"])
        .process_group(0);
    let mut conductor = conductor_command.spawn().expect("spawn conductor fixture");
    let identity = native_process_identity(conductor.id()).expect("capture conductor identity");
    fs::write(
        fixture.scoped_dir().join("conductor.pid"),
        format!(
            "{{\"pid\":{},\"repo\":\"test/repo\",\"scope\":\"test_repo\",\"pgid\":{},\"start_time_ticks\":{}}}\n",
            conductor.id(),
            identity.pgid,
            identity.start_time_ticks,
        ),
    )
    .expect("record conductor metadata");
    assert_authoritative_conductor_metadata(
        &fixture.scoped_dir().join("conductor.pid"),
        conductor.id(),
    );
    let before = snapshot_tree(&fixture.root);

    let output = fixture
        .detached_command("restart")
        .args(["--dry-run", "--json"])
        .output()
        .expect("preview restart");

    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(conductor.try_wait().unwrap().is_none());
    assert_eq!(snapshot_tree(&fixture.root), before);
    assert!(!fixture.calls.exists());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["subcommand"], "restart");
    assert_eq!(json["status"], "dry-run");

    let restarted = fixture
        .detached_command("restart")
        .arg("--json")
        .output()
        .expect("restart after preview");
    let mut conductor_terminated = false;
    for _ in 0..100 {
        if conductor.try_wait().unwrap().is_some() {
            conductor_terminated = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(
        conductor_terminated,
        "non-preview restart must recognize and terminate the owned conductor group; stdout={} stderr={}",
        String::from_utf8_lossy(&restarted.stdout),
        String::from_utf8_lossy(&restarted.stderr)
    );

    terminate_process_group(conductor.id());
    let _ = conductor.wait();
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

    let bridge = fs::read_to_string(
        workspace_root().join("crates/autospec-cli/src/commands/autonomous/executor_bridge.rs"),
    )
    .expect("read executor bridge source");
    for native_contract in [
        "provision_issue_worktree",
        "recover_invocation",
        "runtime_session_adapter",
    ] {
        assert!(
            bridge.contains(native_contract),
            "executor bridge must own {native_contract}"
        );
    }

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
fn native_executor_bridge_source_owns_child_supervision_contract() {
    let source = fs::read_to_string(
        workspace_root().join("crates/autospec-cli/src/commands/autonomous/executor_bridge.rs"),
    )
    .expect("read native executor bridge");

    for required in [
        "build_implementer_prompt",
        "supervise_harness",
        "setpgid(0, 0)",
        "observe_process_identity",
        "terminate_exact_process_group",
        "MutationSnapshot",
        "child_output",
        "BridgePhase::Interrupted",
    ] {
        assert!(
            source.contains(required),
            "native executor bridge omitted supervision contract: {required}"
        );
    }
    assert_no_forbidden_authority(
        &source,
        &[
            SourcePattern::Literal("autospec-autonomous.sh"),
            SourcePattern::Literal("autospec-run"),
            SourcePattern::Literal("omx autospec"),
        ],
        "native executor bridge",
    );
}

#[test]
fn native_executor_bridge_source_owns_ready_through_cleanup_contract() {
    let bridge = fs::read_to_string(
        workspace_root().join("crates/autospec-cli/src/commands/autonomous/executor_bridge.rs"),
    )
    .expect("read native executor bridge");
    for required in [
        "mark_exact_draft_ready",
        "poll_exact_required_ci",
        "run_strict_independent_reviewer",
        "reconcile_base_drift",
        "originate_and_accept_executor_result",
        "admin_squash_merge_exact",
        "finalize_merged_executor",
        "finalize_failed_executor",
    ] {
        assert!(
            bridge.contains(required),
            "native executor bridge omitted completion contract: {required}"
        );
    }

    let claim =
        fs::read_to_string(workspace_root().join("crates/autospec-cli/src/commands/claim.rs"))
            .expect("read claim command");
    for required in [
        "transition_bridge_claim",
        "BridgeClaimDisposition::Retryable",
        "BridgeClaimDisposition::NeedsHuman",
        "BridgeTerminalMode::Prepared",
        "BridgeTerminalMode::Complete",
    ] {
        assert!(
            claim.contains(required),
            "claim authority omitted completion contract: {required}"
        );
    }
}

#[test]
fn fabricated_premerge_producer_pass_is_not_admissible_evidence() {
    let root = temp_dir("fabricated-premerge-pass");
    let repo = root.join("repo");
    fs::create_dir_all(&repo).expect("create producer repo");
    git_fixture(&repo, &["init", "-b", "feat/evidence"]);
    git_fixture(&repo, &["config", "user.name", "Autospec Test"]);
    git_fixture(&repo, &["config", "user.email", "autospec@example.invalid"]);
    fs::write(repo.join("tracked.txt"), "baseline\n").expect("write producer fixture");
    git_fixture(&repo, &["add", "tracked.txt"]);
    git_fixture(&repo, &["commit", "-m", "fixture"]);

    let output = Command::new(env!("CARGO_BIN_EXE_autospec"))
        .args([
            "autonomous",
            "premerge",
            "produce",
            "--kind",
            "qa",
            "--repo",
            "test/repo",
            "--repo-dir",
            repo.to_str().expect("repo UTF-8"),
            "--issue",
            "42",
            "--worker-id",
            "worker-42",
            "--claim-id",
            "claim-42",
            "--run-id",
            "fabricated-model-pass",
            "--verdict",
            "pass",
        ])
        .output()
        .expect("reject fabricated producer Pass");

    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("observed bridge evidence"),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !repo.join(".autospec/evidence/premerge").exists(),
        "external Pass producer must not write admissible evidence"
    );
    fs::remove_dir_all(root).expect("remove producer fixture");
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
        "status={:?} stdout={} stderr={}",
        first.status.code(),
        String::from_utf8_lossy(&first.stdout),
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
    for forbidden in ["issue\nedit\n42", "issue\ncomment\n42", "executor_pending"] {
        assert!(
            !calls.contains(forbidden),
            "empty Tier 1 must not mutate implementation work: {forbidden}"
        );
    }
    assert!(
        calls.contains("issue\nedit\n999"),
        "the mandatory run epic remains the only permitted empty-queue mutation"
    );
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
fn foreground_resumes_no_ready_pause() {
    let fixture = ForegroundFixture::new();
    let paused = no_ready_paused_state();
    seed_foreground_state(&fixture, &paused);
    fs::write(&fixture.mode, "reviewed\n").expect("make issue queue-ready");

    let output = fixture.run_foreground();

    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let resumed = fixture.read_state();
    assert_ne!(resumed, paused, "ready work left the conductor parked");
    assert_eq!(resumed.selected_issue(), Some(42));
    assert_ne!(resumed.pause_reason(), Some("no_ready_issue_after_review"));
}

#[test]
fn foreground_still_empty_pause_polls_without_process_churn() {
    let fixture = ForegroundFixture::new();
    fixture.initialize_git_remote();
    let paused = no_ready_paused_state();
    seed_foreground_state(&fixture, &paused);
    let output = fixture
        .detached_command("start")
        .args(["--detach", "--branch", "main", "--poll-interval-sec", "1"])
        .env("AUTOSPEC_AUTONOMOUS_COMPANIONS", "0")
        .env("AUTOSPEC_FOREGROUND_EMPTY_QUEUE", "1")
        .output()
        .expect("start parked foreground conductor");
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    wait_for_file_contents(&fixture.calls, "labels=auto-implement");
    let conductor_pid = fixture
        .recorded_conductor_pid()
        .expect("recorded conductor pid");

    std::thread::sleep(std::time::Duration::from_millis(250));
    let same_process = fixture.recorded_conductor_pid() == Some(conductor_pid)
        && process_is_running(conductor_pid);
    let retained = fixture.read_state();
    fixture.terminate_recorded_conductor();

    assert!(
        same_process,
        "empty paused conductor exited instead of polling in process"
    );
    assert_eq!(retained, paused);
}

#[test]
fn foreground_rejects_ambiguous_no_ready_pause_resume_phase() {
    let fixture = ForegroundFixture::new();
    let ambiguous = ConductorState::new("test/repo", ConductorScope::Repository, 3)
        .expect("state")
        .transition(ConductorEvent::Pause {
            reason: "no_ready_issue_after_review".to_string(),
        })
        .expect("ambiguous pause");
    seed_foreground_state(&fixture, &ambiguous);

    let output = fixture.run_foreground();

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("no-ready foreground pause must resume from Select"),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fixture.read_state(), ambiguous);
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
            "executor_receipt_failed".to_string()
        ))
    );
    assert_eq!(state.pause_reason(), Some("executor_receipt_failed"));
    let calls = fs::read_to_string(&fixture.calls).expect("read GitHub calls");
    let review = calls
        .find("repos/test/repo/issues/42\n--jq")
        .expect("issue reread");
    let claim = calls.find("issue\nedit\n42").expect("claim label change");
    assert!(review < claim, "safety review must precede claim selection");
    assert!(!calls.contains("executor_pending"));
    let second = fixture.run_foreground();
    assert!(
        second.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&second.stderr)
    );
    assert_eq!(fixture.read_state(), state);
    assert!(!fs::read_to_string(&fixture.calls)
        .expect("calls after retained-state replay")
        .contains("\npr\ncreate\n"));
}

#[test]
fn foreground_closed_selected_issue_retires_before_receipt_recovery() {
    let fixture = ForegroundFixture::new();
    assert!(fixture.run_foreground().status.success());
    assert_eq!(
        fixture.read_state().pause_reason(),
        Some("executor_receipt_failed")
    );

    let output = fixture
        .command()
        .env("FOREGROUND_ISSUE_STATE", "closed")
        .output()
        .expect("retire closed receipt-failure selection");

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fixture.read_state().selected_issue(), None);
}

#[test]
fn foreground_executes_and_merges_selected_issue_through_native_bridge_once() {
    let _bridge_e2e = REAL_BRIDGE_E2E.lock().expect("real bridge E2E lock");
    let fixture = ForegroundFixture::new();
    let bridge = fixture.configure_real_bridge();
    let review_launches = bridge.safe_root.join("review-launches");
    let mut command = fixture.command();
    command
        .env("PATH", path_with(&bridge.bin))
        .env("AUTOSPEC_FOREGROUND_REAL_BRIDGE", "1")
        .env("AUTOSPEC_BRIDGE_REMOTE", &bridge.remote)
        .env("AUTOSPEC_BRIDGE_MERGED", &bridge.merged)
        .env("AUTOSPEC_CLAIM_GIT_REMOTE", &bridge.remote)
        .env("AUTOSPEC_HARNESS_RUNTIME_ALIASES", &bridge.aliases)
        .env("AUTOSPEC_HANDOFF_DISPATCHER_KIND", "codex")
        .env("AUTOSPEC_BRIDGE_REVIEW_LAUNCHES", &review_launches)
        .env_remove("AUTOSPEC_EXECUTOR_REVIEW_COMMAND");

    let first = command.output().expect("execute selected issue");
    assert!(
        first.status.success(),
        "status={:?} stdout={} stderr={}",
        first.status.code(),
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr)
    );
    let terminal = fs::read_dir(fixture.scoped_dir().join("executor"))
        .expect("executor state directory")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    name.starts_with("issue-42-") && name.ends_with(".terminal.json")
                })
        })
        .unwrap_or_else(|| {
            panic!(
                "generation-scoped terminal receipt missing; stderr={} state={:?} calls={}",
                String::from_utf8_lossy(&first.stderr),
                fixture.read_state(),
                fs::read_to_string(&fixture.calls).unwrap_or_default()
            )
        });
    assert!(
        terminal.is_file(),
        "bridge did not reach a terminal receipt; stderr={} state={:?} calls={}",
        String::from_utf8_lossy(&first.stderr),
        fixture.read_state(),
        fs::read_to_string(&fixture.calls).unwrap_or_default()
    );
    assert_eq!(
        git_fixture(
            &fixture.root,
            &[
                "--git-dir",
                bridge.remote.to_str().unwrap(),
                "show",
                "refs/heads/main:tests/smoke/generation.sh",
            ],
        ),
        "#!/bin/sh\nexit 0"
    );
    let calls = fs::read_to_string(&fixture.calls).expect("read bridge calls");
    assert_eq!(calls.matches("\npr\ncreate\n").count(), 1);
    assert_eq!(calls.matches("\npr\nmerge\n").count(), 1);
    assert!(calls.lines().any(|line| line == "--match-head-commit"));
    assert_eq!(
        fs::read_to_string(&review_launches)
            .expect("configured reviewer launch")
            .lines()
            .count(),
        1,
        "unset review override must invoke exactly one configured external harness"
    );
    let receipt = fs::read_to_string(&terminal).expect("terminal bridge receipt");
    assert!(receipt.contains("\"status\":\"merged\""));
    assert!(receipt.contains("\"claim_released\":true"));
    assert!(
        !Path::new("/tmp/autospec-executor")
            .join(format!("test_repo-{}", &sha256_hex(b"test/repo")[..12]))
            .join("issue-42")
            .exists(),
        "verified completion removes only the owned issue worktree"
    );

    let invocation = fs::read_dir(fixture.scoped_dir().join("executor"))
        .expect("executor state directory")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| is_generation_invocation_path(path, 42))
        .expect("generation-scoped invocation");
    let mut cleanup_window: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&invocation).unwrap()).unwrap();
    cleanup_window["phase"] = serde_json::json!("cleanup_pending");
    fs::write(&invocation, format!("{cleanup_window}\n")).expect("rewind to cleanup window");
    fs::remove_file(&terminal).expect("remove terminal publication after cleanup window");
    let dispatch = ConductorState::new("test/repo", ConductorScope::Repository, 3)
        .unwrap()
        .transition(ConductorEvent::ScanFoundWork)
        .unwrap()
        .transition(ConductorEvent::SafetyReviewed)
        .unwrap()
        .transition(ConductorEvent::Selected {
            issue: 42,
            serialization_reasons: Vec::new(),
        })
        .unwrap()
        .transition(ConductorEvent::Claimed)
        .unwrap()
        .transition(ConductorEvent::DispatchRecorded {
            outcome: ConductorOutcome::Succeeded,
        })
        .unwrap();
    fs::write(fixture.state_path(), dispatch.to_json()).expect("seed interrupted dispatch");
    let terminal_receipt: serde_json::Value =
        serde_json::from_str(&receipt).expect("parse terminal bridge receipt");
    fixture.seed_claim_acquisition_receipt(
        terminal_receipt["worker_id"]
            .as_str()
            .expect("terminal worker"),
        terminal_receipt["branch"]
            .as_str()
            .expect("terminal branch"),
        terminal_receipt["claim_id"]
            .as_str()
            .expect("terminal claim"),
    );
    let before = fs::read_to_string(&fixture.calls).expect("calls before replay");
    let second = fixture
        .command()
        .env("PATH", path_with(&bridge.bin))
        .env("AUTOSPEC_FOREGROUND_REAL_BRIDGE", "1")
        .env("AUTOSPEC_BRIDGE_REMOTE", &bridge.remote)
        .env("AUTOSPEC_BRIDGE_MERGED", &bridge.merged)
        .env("AUTOSPEC_CLAIM_GIT_REMOTE", &bridge.remote)
        .env("AUTOSPEC_HARNESS_RUNTIME_ALIASES", &bridge.aliases)
        .env("AUTOSPEC_HANDOFF_DISPATCHER_KIND", "codex")
        .env_remove("AUTOSPEC_EXECUTOR_REVIEW_COMMAND")
        .output()
        .expect("replay completed foreground");
    assert!(
        second.status.success() || second.status.code() == Some(3),
        "replay stderr={}",
        String::from_utf8_lossy(&second.stderr)
    );
    let after = fs::read_to_string(&fixture.calls).expect("calls after replay");
    assert_eq!(
        after.matches("\npr\ncreate\n").count(),
        before.matches("\npr\ncreate\n").count()
    );
    assert_eq!(
        after.matches("\npr\nmerge\n").count(),
        before.matches("\npr\nmerge\n").count()
    );
    assert_eq!(fixture.read_state().phase(), ConductorPhase::Scan);
}

#[test]
fn foreground_accepts_fast_forwarded_explore_head() {
    let _bridge_e2e = REAL_BRIDGE_E2E.lock().expect("real bridge E2E lock");
    let fixture = ForegroundFixture::new();
    let bridge = fixture.configure_real_bridge();
    let recorded_oid = git_fixture(&fixture.repo_dir, &["rev-parse", "HEAD"]);
    git_fixture(
        &fixture.repo_dir,
        &["checkout", "-b", "autospec/autonomous-main"],
    );
    git_fixture(
        &fixture.repo_dir,
        &["push", "-u", "origin", "autospec/autonomous-main"],
    );
    fs::write(fixture.repo_dir.join("integration.txt"), "advanced\n")
        .expect("advance integration branch");
    git_fixture(&fixture.repo_dir, &["add", "integration.txt"]);
    git_fixture(
        &fixture.repo_dir,
        &["commit", "-m", "advance integration branch"],
    );
    git_fixture(
        &fixture.repo_dir,
        &["push", "origin", "autospec/autonomous-main"],
    );
    let advanced_oid = git_fixture(&fixture.repo_dir, &["rev-parse", "HEAD"]);
    git_fixture(&fixture.repo_dir, &["checkout", "main"]);
    fs::create_dir_all(fixture.repo_dir.join(".autospec")).expect("create explore config");
    fs::write(
        fixture.repo_dir.join(".autospec/explore-mode.json"),
        format!("{{\"branch\":\"autospec/autonomous-main\",\"head_sha\":\"{recorded_oid}\"}}\n"),
    )
    .expect("write stale explore head");

    let output = fixture
        .command()
        .env("PATH", path_with(&bridge.bin))
        .env("AUTOSPEC_FOREGROUND_REAL_BRIDGE", "1")
        .env("AUTOSPEC_BRIDGE_REMOTE", &bridge.remote)
        .env("AUTOSPEC_BRIDGE_MERGED", &bridge.merged)
        .env("AUTOSPEC_BRIDGE_BASE_REF", "autospec/autonomous-main")
        .env("AUTOSPEC_CLAIM_GIT_REMOTE", &bridge.remote)
        .env("AUTOSPEC_HARNESS_RUNTIME_ALIASES", &bridge.aliases)
        .env("AUTOSPEC_HANDOFF_DISPATCHER_KIND", "codex")
        .env_remove("AUTOSPEC_EXECUTOR_REVIEW_COMMAND")
        .output()
        .expect("execute from fast-forwarded explore head");

    assert!(
        output.status.success(),
        "status={:?} stdout={} stderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        git_fixture(
            &fixture.root,
            &[
                "--git-dir",
                bridge.remote.to_str().unwrap(),
                "merge-base",
                "--is-ancestor",
                &advanced_oid,
                "refs/heads/autospec/autonomous-main",
            ],
        ),
        ""
    );
}

#[test]
fn foreground_recovers_complete_bridge_after_transient_terminal_observation_failure() {
    let _bridge_e2e = REAL_BRIDGE_E2E.lock().expect("real bridge E2E lock");
    let fixture = ForegroundFixture::new();
    let bridge = fixture.configure_real_bridge();
    let fail_once = fixture.root.join("terminal-observation.failed");
    let configure = |command: &mut Command| {
        command
            .env("PATH", path_with(&bridge.bin))
            .env("AUTOSPEC_FOREGROUND_REAL_BRIDGE", "1")
            .env("AUTOSPEC_BRIDGE_REMOTE", &bridge.remote)
            .env("AUTOSPEC_BRIDGE_MERGED", &bridge.merged)
            .env("AUTOSPEC_CLAIM_GIT_REMOTE", &bridge.remote)
            .env("AUTOSPEC_HARNESS_RUNTIME_ALIASES", &bridge.aliases)
            .env("AUTOSPEC_HANDOFF_DISPATCHER_KIND", "codex")
            .env_remove("AUTOSPEC_EXECUTOR_REVIEW_COMMAND")
            .env("AUTOSPEC_FOREGROUND_FAIL_TERMINAL_ONCE", &fail_once);
    };
    let mut first_command = fixture.command();
    configure(&mut first_command);
    let first = first_command
        .output()
        .expect("run through terminal failpoint");
    assert!(
        first.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(fail_once.exists());
    assert_eq!(
        fixture.read_state().phase(),
        ConductorPhase::Scan,
        "typed terminal observation outages retry the same completed invocation"
    );

    let calls = fs::read_to_string(&fixture.calls).expect("bridge calls");
    assert_eq!(calls.matches("\npr\ncreate\n").count(), 1);
    assert_eq!(calls.matches("\npr\nmerge\n").count(), 1);
    assert!(fs::read_dir(fixture.scoped_dir().join("executor"))
        .unwrap()
        .filter_map(Result::ok)
        .any(|entry| entry.path().to_string_lossy().ends_with(".terminal.json")));
}

#[test]
fn foreground_recovery_refuses_a_foreign_claim_without_a_local_acquisition_receipt() {
    let fixture = ForegroundFixture::new();
    fs::create_dir_all(fixture.state_path().parent().unwrap())
        .expect("create recovery state directory");
    fs::write(fixture.state_path(), selected_foreground_state().to_json())
        .expect("seed selected foreground state");
    fixture.seed_claim_state_with_id(
        "foreign-conductor",
        "feat/foreign-issue-42",
        "claimed",
        &fresh_iso_timestamp(),
        "foreign-generation",
    );

    let output = fixture.command().output().expect("run foreign recovery");

    assert!(!output.status.success());
    assert!(
        !fs::read_to_string(&fixture.calls)
            .expect("read GitHub calls")
            .contains("\npr\ncreate\n"),
        "foreign ownership must fail before executor launch"
    );
    assert_eq!(fixture.read_state().phase(), ConductorPhase::Claim);
}

#[test]
fn foreground_legacy_executor_pending_without_receipt_retires_to_scan() {
    let fixture = ForegroundFixture::new();
    fs::create_dir_all(fixture.state_path().parent().unwrap())
        .expect("create recovery state directory");
    fs::write(
        fixture.state_path(),
        legacy_executor_pending_state().to_json(),
    )
    .expect("seed legacy executor-pending state");

    let output = fixture.command().output().expect("retire legacy state");

    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let state = fixture.read_state();
    assert_eq!(state.phase(), ConductorPhase::Scan);
    assert_eq!(state.selected_issue(), None);
    assert!(
        !fs::read_to_string(&fixture.calls)
            .expect("read GitHub calls")
            .contains("\npr\ncreate\n"),
        "receiptless legacy ownership must retire before executor launch"
    );
}

#[test]
fn foreground_legacy_executor_pending_with_orphaned_receipt_retires_without_claiming() {
    let fixture = ForegroundFixture::new();
    fs::create_dir_all(fixture.state_path().parent().unwrap())
        .expect("create recovery state directory");
    fs::write(
        fixture.state_path(),
        legacy_executor_pending_state().to_json(),
    )
    .expect("seed legacy executor-pending state");
    fixture.seed_claim_acquisition_receipt(
        "rust-foreground-conductor-recovered",
        "feat/autonomous-issue-42",
        EXECUTOR_CLAIM_ID,
    );

    let output = fixture
        .command()
        .output()
        .expect("retire orphaned legacy receipt");

    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fixture.read_state().phase(), ConductorPhase::Scan);
    assert!(
        !fixture
            .state_path()
            .with_extension("claim-acquisition.json")
            .exists(),
        "orphaned local receipt must be retired"
    );
    let claim = Command::new("git")
        .args([
            "--git-dir",
            fixture.claim_remote.to_str().expect("claim remote"),
            "show-ref",
            "--verify",
            "--quiet",
            "refs/autospec/claims/issue-42",
        ])
        .status()
        .expect("inspect authoritative claim ref");
    assert!(!claim.success(), "recovery must not acquire a new claim");
    assert!(
        !fs::read_to_string(&fixture.calls)
            .expect("read GitHub calls")
            .contains("\npr\ncreate\n"),
        "orphaned receipt must retire before executor launch"
    );
}

#[test]
fn foreground_claim_phase_with_orphaned_receipt_retires_without_claiming() {
    let fixture = ForegroundFixture::new();
    fs::create_dir_all(fixture.state_path().parent().unwrap())
        .expect("create recovery state directory");
    fs::write(fixture.state_path(), selected_foreground_state().to_json())
        .expect("seed persisted claim-phase state");
    fixture.seed_claim_acquisition_receipt(
        "rust-foreground-conductor-recovered",
        "feat/autonomous-issue-42",
        EXECUTOR_CLAIM_ID,
    );

    let output = fixture
        .command()
        .output()
        .expect("retire orphaned claim-phase receipt");

    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fixture.read_state().phase(), ConductorPhase::Scan);
    assert!(
        !fixture
            .state_path()
            .with_extension("claim-acquisition.json")
            .exists(),
        "orphaned claim-phase receipt must be retired"
    );
    let claim = Command::new("git")
        .args([
            "--git-dir",
            fixture.claim_remote.to_str().expect("claim remote"),
            "show-ref",
            "--verify",
            "--quiet",
            "refs/autospec/claims/issue-42",
        ])
        .status()
        .expect("inspect authoritative claim ref");
    assert!(!claim.success(), "recovery must not acquire a new claim");
    assert!(
        !fs::read_to_string(&fixture.calls)
            .expect("read GitHub calls")
            .contains("\npr\ncreate\n"),
        "orphaned claim phase must retire before executor launch"
    );
}

#[test]
fn foreground_dispatch_recovery_retires_exact_released_claim() {
    let fixture = ForegroundFixture::new();
    let worker = "rust-foreground-conductor-released";
    let branch = "feat/autonomous-issue-42";
    let dispatch = selected_foreground_state()
        .transition(ConductorEvent::Claimed)
        .expect("enter dispatch phase");
    seed_foreground_state(&fixture, &dispatch);
    fixture.seed_claim_state_with_id(
        worker,
        branch,
        "released",
        &fresh_iso_timestamp(),
        EXECUTOR_CLAIM_ID,
    );
    fixture.seed_claim_acquisition_receipt(worker, branch, EXECUTOR_CLAIM_ID);

    let output = fixture
        .command()
        .output()
        .expect("retire released dispatch acquisition");

    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let recovered = fixture.read_state();
    assert_eq!(recovered.phase(), ConductorPhase::Scan);
    assert_eq!(recovered.selected_issue(), None);
    assert!(
        !fixture
            .state_path()
            .with_extension("claim-acquisition.json")
            .exists(),
        "exact released acquisition must retire durably"
    );
    assert!(
        !fs::read_to_string(&fixture.calls)
            .expect("read GitHub calls")
            .contains("\npr\ncreate\n"),
        "released dispatch recovery must retire before executor launch"
    );
}

#[test]
fn foreground_dispatch_recovery_rejects_mismatched_terminal_claim() {
    let fixture = ForegroundFixture::new();
    let worker = "rust-foreground-conductor-released";
    let branch = "feat/autonomous-issue-42";
    let dispatch = selected_foreground_state()
        .transition(ConductorEvent::Claimed)
        .expect("enter dispatch phase");
    seed_foreground_state(&fixture, &dispatch);
    fixture.seed_claim_state_with_id(
        worker,
        branch,
        "released",
        &fresh_iso_timestamp(),
        "successor-generation",
    );
    fixture.seed_claim_acquisition_receipt(worker, branch, EXECUTOR_CLAIM_ID);

    let output = fixture
        .command()
        .output()
        .expect("reject mismatched terminal claim");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("terminal claim does not match the durable local claim acquisition"));
    assert!(
        fixture
            .state_path()
            .with_extension("claim-acquisition.json")
            .exists(),
        "mismatched terminal claim must not retire the local acquisition"
    );
    assert!(
        !fs::read_to_string(&fixture.calls)
            .expect("read GitHub calls")
            .contains("\npr\ncreate\n"),
        "mismatched terminal claim must fail before executor launch"
    );
}

#[test]
fn foreground_legacy_executor_pending_resumes_exact_local_acquisition_receipt() {
    let _bridge_e2e = REAL_BRIDGE_E2E.lock().expect("real bridge E2E lock");
    let fixture = ForegroundFixture::new();
    let bridge = fixture.configure_real_bridge();
    fs::create_dir_all(fixture.state_path().parent().unwrap())
        .expect("create recovery state directory");
    fs::write(
        fixture.state_path(),
        legacy_executor_pending_state().to_json(),
    )
    .expect("seed legacy executor-pending state");
    fixture.seed_claim_state_with_id(
        "rust-foreground-conductor-recovered",
        "feat/autonomous-issue-42",
        "claimed",
        &fresh_iso_timestamp(),
        EXECUTOR_CLAIM_ID,
    );
    fixture.seed_claim_acquisition_receipt(
        "rust-foreground-conductor-recovered",
        "feat/autonomous-issue-42",
        EXECUTOR_CLAIM_ID,
    );
    fixture.copy_claim_ref_to(&bridge.remote);

    let output = fixture
        .command()
        .env("PATH", path_with(&bridge.bin))
        .env("AUTOSPEC_FOREGROUND_REAL_BRIDGE", "1")
        .env("AUTOSPEC_BRIDGE_REMOTE", &bridge.remote)
        .env("AUTOSPEC_BRIDGE_MERGED", &bridge.merged)
        .env("AUTOSPEC_CLAIM_GIT_REMOTE", &bridge.remote)
        .env("AUTOSPEC_HARNESS_RUNTIME_ALIASES", &bridge.aliases)
        .env("AUTOSPEC_HANDOFF_DISPATCHER_KIND", "codex")
        .env_remove("AUTOSPEC_EXECUTOR_REVIEW_COMMAND")
        .output()
        .expect("run exact recovery");

    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let calls = fs::read_to_string(&fixture.calls).expect("read bridge calls");
    assert_eq!(calls.matches("\npr\ncreate\n").count(), 1);
    assert_eq!(calls.matches("\npr\nmerge\n").count(), 1);
    assert!(
        !fixture
            .state_path()
            .with_extension("claim-acquisition.json")
            .exists(),
        "terminal recovery retires the local acquisition receipt"
    );
}

#[test]
fn foreground_recovery_rejects_a_tampered_local_acquisition_receipt() {
    let fixture = ForegroundFixture::new();
    fs::create_dir_all(fixture.state_path().parent().unwrap())
        .expect("create recovery state directory");
    fs::write(fixture.state_path(), selected_foreground_state().to_json())
        .expect("seed selected foreground state");
    fixture.seed_claim_state_with_id(
        "rust-foreground-conductor-recovered",
        "feat/autonomous-issue-42",
        "claimed",
        &fresh_iso_timestamp(),
        EXECUTOR_CLAIM_ID,
    );
    fixture.seed_claim_acquisition_receipt(
        "rust-foreground-conductor-recovered",
        "feat/autonomous-issue-42",
        "tampered-generation",
    );

    let output = fixture.command().output().expect("run tampered recovery");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("does not match the durable local acquisition"));
    assert!(
        !fs::read_to_string(&fixture.calls)
            .expect("read GitHub calls")
            .contains("\npr\ncreate\n"),
        "tampered ownership must fail before executor launch"
    );
    assert_eq!(fixture.read_state().phase(), ConductorPhase::Claim);
}

#[test]
fn foreground_recovery_rejects_a_nonprivate_local_acquisition_receipt() {
    let fixture = ForegroundFixture::new();
    fs::create_dir_all(fixture.state_path().parent().unwrap())
        .expect("create recovery state directory");
    fs::write(fixture.state_path(), selected_foreground_state().to_json())
        .expect("seed selected foreground state");
    fixture.seed_claim_state_with_id(
        "rust-foreground-conductor-recovered",
        "feat/autonomous-issue-42",
        "claimed",
        &fresh_iso_timestamp(),
        EXECUTOR_CLAIM_ID,
    );
    fixture.seed_claim_acquisition_receipt(
        "rust-foreground-conductor-recovered",
        "feat/autonomous-issue-42",
        EXECUTOR_CLAIM_ID,
    );
    fs::set_permissions(
        fixture
            .state_path()
            .with_extension("claim-acquisition.json"),
        fs::Permissions::from_mode(0o644),
    )
    .expect("make receipt nonprivate");

    let output = fixture.command().output().expect("run nonprivate recovery");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("receipt must be private"));
}

#[test]
fn foreground_recovery_rejects_a_symlinked_local_acquisition_receipt() {
    let fixture = ForegroundFixture::new();
    fs::create_dir_all(fixture.state_path().parent().unwrap())
        .expect("create recovery state directory");
    fs::write(fixture.state_path(), selected_foreground_state().to_json())
        .expect("seed selected foreground state");
    fixture.seed_claim_state_with_id(
        "rust-foreground-conductor-recovered",
        "feat/autonomous-issue-42",
        "claimed",
        &fresh_iso_timestamp(),
        EXECUTOR_CLAIM_ID,
    );
    let target = fixture.root.join("foreign-receipt.json");
    fs::write(&target, "{}\n").expect("write symlink target");
    fs::set_permissions(
        fixture.state_path().parent().unwrap(),
        fs::Permissions::from_mode(0o700),
    )
    .expect("make receipt parent private");
    std::os::unix::fs::symlink(
        &target,
        fixture
            .state_path()
            .with_extension("claim-acquisition.json"),
    )
    .expect("symlink acquisition receipt");

    let output = fixture.command().output().expect("run symlink recovery");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("receipt must be a regular file"));
}

fn selected_foreground_state() -> ConductorState {
    ConductorState::new("test/repo", ConductorScope::Repository, 3)
        .unwrap()
        .transition(ConductorEvent::ScanFoundWork)
        .unwrap()
        .transition(ConductorEvent::SafetyReviewed)
        .unwrap()
        .transition(ConductorEvent::Selected {
            issue: 42,
            serialization_reasons: Vec::new(),
        })
        .unwrap()
}

fn no_ready_paused_state() -> ConductorState {
    ConductorState::new("test/repo", ConductorScope::Repository, 3)
        .expect("state")
        .transition(ConductorEvent::ScanFoundWork)
        .expect("scan")
        .transition(ConductorEvent::SafetyReviewed)
        .expect("review")
        .transition(ConductorEvent::Pause {
            reason: "no_ready_issue_after_review".to_string(),
        })
        .expect("no-ready pause")
}

fn seed_foreground_state(fixture: &ForegroundFixture, state: &ConductorState) {
    fs::create_dir_all(fixture.state_path().parent().expect("state parent"))
        .expect("create foreground state directory");
    fs::write(fixture.state_path(), state.to_json()).expect("seed foreground state");
}

fn legacy_executor_pending_state() -> ConductorState {
    selected_foreground_state()
        .transition(ConductorEvent::Claimed)
        .unwrap()
        .transition(ConductorEvent::DispatchRecorded {
            outcome: ConductorOutcome::Blocked("implementation_executor_pending".to_string()),
        })
        .unwrap()
}

#[test]
fn foreground_closed_claim_phase_selection_retires_without_reacquisition() {
    let fixture = ForegroundFixture::new();
    let retry_claim = selected_foreground_state()
        .transition(ConductorEvent::Claimed)
        .expect("claim first generation")
        .transition(ConductorEvent::DispatchRecorded {
            outcome: ConductorOutcome::Retryable("executor_harness_exit_1".to_string()),
        })
        .expect("record retryable result")
        .transition(ConductorEvent::RetryScheduled)
        .expect("schedule successor claim");
    assert_eq!(retry_claim.phase(), ConductorPhase::Claim);
    seed_foreground_state(&fixture, &retry_claim);
    fs::write(&fixture.mode, "reviewed\n").expect("seed queue labels");

    let output = fixture
        .command()
        .env("FOREGROUND_ISSUE_STATE", "closed")
        .output()
        .expect("recover closed claim-phase selection");

    assert!(
        output.status.success(),
        "stdout={} stderr={} calls={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
        fs::read_to_string(&fixture.calls).unwrap_or_default()
    );
    let recovered = fixture.read_state();
    assert_eq!(recovered.phase(), ConductorPhase::Scan);
    assert_eq!(recovered.selected_issue(), None);
    let calls = fs::read_to_string(&fixture.calls).expect("GitHub calls");
    assert!(
        !calls.contains("issue\nedit\n42"),
        "a closed selected issue must not receive a successor claim"
    );
}

#[test]
fn foreground_recovers_released_executor_receipt_failure_and_other_claim_crash_windows() {
    let _bridge_e2e = REAL_BRIDGE_E2E.lock().expect("real bridge E2E lock");
    for (name, state, seeded_claim_state) in [
        (
            "claim-before-acquire",
            ConductorState::new("test/repo", ConductorScope::Repository, 3)
                .unwrap()
                .transition(ConductorEvent::ScanFoundWork)
                .unwrap()
                .transition(ConductorEvent::SafetyReviewed)
                .unwrap()
                .transition(ConductorEvent::Selected {
                    issue: 42,
                    serialization_reasons: Vec::new(),
                })
                .unwrap(),
            None,
        ),
        (
            "claim-after-acquire",
            ConductorState::new("test/repo", ConductorScope::Repository, 3)
                .unwrap()
                .transition(ConductorEvent::ScanFoundWork)
                .unwrap()
                .transition(ConductorEvent::SafetyReviewed)
                .unwrap()
                .transition(ConductorEvent::Selected {
                    issue: 42,
                    serialization_reasons: Vec::new(),
                })
                .unwrap(),
            Some("claimed"),
        ),
        (
            "retry-before-schedule",
            ConductorState::new("test/repo", ConductorScope::Repository, 3)
                .unwrap()
                .transition(ConductorEvent::ScanFoundWork)
                .unwrap()
                .transition(ConductorEvent::SafetyReviewed)
                .unwrap()
                .transition(ConductorEvent::Selected {
                    issue: 42,
                    serialization_reasons: Vec::new(),
                })
                .unwrap()
                .transition(ConductorEvent::Claimed)
                .unwrap()
                .transition(ConductorEvent::DispatchRecorded {
                    outcome: ConductorOutcome::Retryable("transient".to_string()),
                })
                .unwrap(),
            Some("released"),
        ),
        (
            "released-executor-receipt-failure",
            selected_foreground_state()
                .transition(ConductorEvent::Claimed)
                .unwrap()
                .transition(ConductorEvent::DispatchRecorded {
                    outcome: ConductorOutcome::Blocked("executor_receipt_failed".to_string()),
                })
                .unwrap(),
            Some("released"),
        ),
    ] {
        let fixture = ForegroundFixture::new();
        let bridge = fixture.configure_real_bridge();
        fs::create_dir_all(fixture.state_path().parent().unwrap())
            .expect("create crash-window state directory");
        fs::write(fixture.state_path(), state.to_json()).expect("seed crash-window state");
        if let Some(claim_state) = seeded_claim_state {
            let seeded_claim_id = if claim_state == "claimed" {
                EXECUTOR_CLAIM_ID
            } else {
                "terminal-retry-generation"
            };
            fixture.seed_claim_state_with_id(
                "rust-foreground-conductor-recovered",
                "feat/autonomous-issue-42",
                claim_state,
                &fresh_iso_timestamp(),
                seeded_claim_id,
            );
            fixture.seed_claim_acquisition_receipt(
                "rust-foreground-conductor-recovered",
                "feat/autonomous-issue-42",
                seeded_claim_id,
            );
            let seeded = git_fixture(
                &fixture.claim_repo,
                &[
                    "ls-remote",
                    "--refs",
                    fixture.claim_remote.to_str().expect("claim remote"),
                    "refs/autospec/claims/issue-42",
                ],
            );
            let seeded = seeded.split_whitespace().next().expect("seeded claim oid");
            git_fixture(
                &fixture.claim_repo,
                &[
                    "push",
                    bridge.remote.to_str().expect("bridge remote"),
                    &format!("{seeded}:refs/autospec/claims/issue-42"),
                ],
            );
            fs::write(
                &fixture.mode,
                if claim_state == "claimed" {
                    "claimed\n"
                } else {
                    "reviewed\n"
                },
            )
            .expect("seed claim label projection");
            if name == "released-executor-receipt-failure" {
                fixture.seed_interrupted_executor_invocation_without_cleanup(
                    "rust-foreground-conductor-recovered",
                    "feat/autonomous-issue-42",
                    seeded_claim_id,
                );
            }
        }
        let mut command = if name == "released-executor-receipt-failure" {
            let mut command = fixture.configured_command();
            command.args([
                "autonomous",
                "start",
                "--foreground",
                "--max-cycles",
                "2",
                "--poll-interval-sec",
                "0",
                "--repo",
                "test/repo",
                "--repo-dir",
                fixture.repo_dir.to_str().expect("repo path"),
                "--branch",
                "main",
            ]);
            command
        } else {
            fixture.command()
        };
        let output = command
            .env("PATH", path_with(&bridge.bin))
            .env("AUTOSPEC_FOREGROUND_REAL_BRIDGE", "1")
            .env("AUTOSPEC_BRIDGE_REMOTE", &bridge.remote)
            .env("AUTOSPEC_BRIDGE_MERGED", &bridge.merged)
            .env("AUTOSPEC_CLAIM_GIT_REMOTE", &bridge.remote)
            .env("AUTOSPEC_HARNESS_RUNTIME_ALIASES", &bridge.aliases)
            .env("AUTOSPEC_HANDOFF_DISPATCHER_KIND", "codex")
            .env_remove("AUTOSPEC_EXECUTOR_REVIEW_COMMAND")
            .output()
            .unwrap_or_else(|error| panic!("{name}: run recovered foreground: {error}"));
        assert!(
            output.status.success(),
            "{name}: stdout={} stderr={} calls={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
            fs::read_to_string(&fixture.calls).unwrap_or_default()
        );
        assert_eq!(fixture.read_state().phase(), ConductorPhase::Scan, "{name}");
        let calls = fs::read_to_string(&fixture.calls).expect("bridge calls");
        let expected_pr_actions = usize::from(name != "retry-before-schedule");
        assert_eq!(
            calls.matches("\npr\ncreate\n").count(),
            expected_pr_actions,
            "{name}"
        );
        assert_eq!(
            calls.matches("\npr\nmerge\n").count(),
            expected_pr_actions,
            "{name}"
        );
        assert!(
            !fixture
                .state_path()
                .with_extension("claim-acquisition.json")
                .exists(),
            "{name}: terminal or released claims retire the acquisition receipt"
        );
        if seeded_claim_state == Some("released") {
            let terminal_message = git_fixture(
                &fixture.root,
                &[
                    "--git-dir",
                    bridge.remote.to_str().expect("bridge remote"),
                    "show",
                    "-s",
                    "--format=%B",
                    "refs/autospec/claims/issue-42",
                ],
            );
            let terminal_record =
                parse_run_state_comment(&terminal_message).expect("parse final bridge claim");
            if name == "retry-before-schedule" {
                assert_eq!(
                    terminal_record.claim_id.as_deref(),
                    Some("terminal-retry-generation"),
                    "retry retirement should not dispatch a replacement claim"
                );
            } else {
                assert_ne!(
                    terminal_record.claim_id.as_deref(),
                    Some("terminal-retry-generation"),
                    "executor restart reused a terminal claim generation"
                );
            }
        }
        drop(bridge);
    }
}

#[test]
fn foreground_missing_failure_intent_requires_exact_retryable_release() {
    let _bridge_e2e = REAL_BRIDGE_E2E.lock().expect("real bridge E2E lock");
    let fixture = ForegroundFixture::new();
    let bridge = fixture.configure_real_bridge();
    fs::create_dir_all(fixture.state_path().parent().unwrap())
        .expect("create recovery state directory");
    fs::write(
        fixture.state_path(),
        selected_foreground_state()
            .transition(ConductorEvent::Claimed)
            .expect("seed claimed conductor")
            .transition(ConductorEvent::DispatchRecorded {
                outcome: ConductorOutcome::Blocked("executor_receipt_failed".to_string()),
            })
            .expect("seed receipt failure")
            .to_json(),
    )
    .expect("persist receipt-failed conductor state");
    let worker_id = "rust-foreground-conductor-recovered";
    let branch = "feat/autonomous-issue-42";
    let claim_id = "terminal-needs-human-generation";
    fixture.seed_claim_state_with_id(
        worker_id,
        branch,
        "failed",
        &fresh_iso_timestamp(),
        claim_id,
    );
    fixture.seed_claim_acquisition_receipt(worker_id, branch, claim_id);
    fixture.seed_interrupted_executor_invocation_without_cleanup(worker_id, branch, claim_id);
    let seeded = git_fixture(
        &fixture.claim_repo,
        &[
            "ls-remote",
            "--refs",
            fixture.claim_remote.to_str().expect("claim remote"),
            "refs/autospec/claims/issue-42",
        ],
    );
    let seeded = seeded.split_whitespace().next().expect("seeded claim oid");
    git_fixture(
        &fixture.claim_repo,
        &[
            "push",
            bridge.remote.to_str().expect("bridge remote"),
            &format!("{seeded}:refs/autospec/claims/issue-42"),
        ],
    );
    fs::write(&fixture.mode, "reviewed\n").expect("seed claim label projection");

    let output = fixture
        .command()
        .env("PATH", path_with(&bridge.bin))
        .env("AUTOSPEC_FOREGROUND_REAL_BRIDGE", "1")
        .env("AUTOSPEC_BRIDGE_REMOTE", &bridge.remote)
        .env("AUTOSPEC_BRIDGE_MERGED", &bridge.merged)
        .env("AUTOSPEC_CLAIM_GIT_REMOTE", &bridge.remote)
        .env("AUTOSPEC_HARNESS_RUNTIME_ALIASES", &bridge.aliases)
        .env("AUTOSPEC_HANDOFF_DISPATCHER_KIND", "codex")
        .env_remove("AUTOSPEC_EXECUTOR_REVIEW_COMMAND")
        .output()
        .expect("reject non-retryable terminal recovery");

    assert!(
        !output.status.success(),
        "stdout={} stderr={} calls={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
        fs::read_to_string(&fixture.calls).unwrap_or_default()
    );
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("missing failure cleanup intent requires an exact released claim"),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        fixture
            .state_path()
            .with_extension("claim-acquisition.json")
            .exists(),
        "non-retryable terminal state must retain its durable acquisition"
    );
}

#[test]
fn foreground_recovers_exact_immediate_stop_release_without_cleanup_intent() {
    let recovery = run_missing_cleanup_recovery(None);

    assert!(recovery.success, "stderr={}", recovery.stderr);
    assert!(
        !recovery.receipt_exists,
        "exact release must retire the acquisition"
    );
    assert!(
        !recovery.heartbeat_exists,
        "exact release must retire the live startup heartbeat"
    );
    assert!(
        recovery.heartbeat_archived,
        "exact release must retain the startup heartbeat handoff"
    );
    assert!(
        recovery.returned_to_scan,
        "exact release must resume queue scanning"
    );
    let fresh = recovery
        .fresh_run
        .expect("exact recovery must run the foreground path again");
    assert!(fresh.success, "stderr={}", fresh.stderr);
    assert!(
        !fresh.stderr.contains("heartbeat_write_failed"),
        "fresh generation heartbeat publication failed: {}",
        fresh.stderr
    );
    assert_eq!(
        fresh.heartbeat_count, 1,
        "the successor must publish exactly one live heartbeat"
    );
    assert!(
        fresh.generation_is_fresh,
        "the successor heartbeat must belong to a fresh claim generation"
    );
}

#[test]
fn foreground_missing_failure_intent_requires_exact_released_identity() {
    for mismatch in ["worker", "claim", "branch", "pr"] {
        let recovery = run_missing_cleanup_recovery(Some(mismatch));

        assert!(
            !recovery.success,
            "{mismatch}: recovery accepted foreign evidence"
        );
        assert!(
            recovery.receipt_exists,
            "{mismatch}: foreign evidence retired the local acquisition"
        );
        assert!(
            recovery.heartbeat_exists,
            "{mismatch}: foreign evidence retired the local startup heartbeat"
        );
    }
}

struct MissingCleanupRecovery {
    success: bool,
    stderr: String,
    receipt_exists: bool,
    returned_to_scan: bool,
    heartbeat_exists: bool,
    heartbeat_archived: bool,
    fresh_run: Option<FreshHeartbeatRun>,
}

struct FreshHeartbeatRun {
    success: bool,
    stderr: String,
    heartbeat_count: usize,
    generation_is_fresh: bool,
}

fn run_missing_cleanup_recovery(mismatch: Option<&str>) -> MissingCleanupRecovery {
    let _bridge_e2e = REAL_BRIDGE_E2E.lock().expect("real bridge E2E lock");
    let fixture = ForegroundFixture::new();
    let bridge = fixture.configure_real_bridge();
    fs::create_dir_all(fixture.state_path().parent().unwrap())
        .expect("create recovery state directory");
    fs::write(
        fixture.state_path(),
        selected_foreground_state()
            .transition(ConductorEvent::Claimed)
            .expect("seed claimed conductor")
            .transition(ConductorEvent::DispatchRecorded {
                outcome: ConductorOutcome::Blocked("executor_receipt_failed".to_string()),
            })
            .expect("seed receipt failure")
            .to_json(),
    )
    .expect("persist receipt-failed conductor state");
    let worker_id = "rust-foreground-conductor-recovered";
    let branch = "feat/autonomous-issue-42";
    let claim_id = "terminal-immediate-stop-generation";
    let updated_at = fresh_iso_timestamp();
    let mut remote = RunStateRecord::new(
        "test/repo",
        42,
        worker_id,
        "released",
        branch,
        "",
        "released",
        Vec::new(),
        &updated_at,
        &updated_at,
        10_800,
    )
    .with_claim_id(claim_id);
    match mismatch {
        Some("worker") => remote.worker_id = "foreign-worker".to_string(),
        Some("claim") => remote.claim_id = Some("foreign-claim".to_string()),
        Some("branch") => remote.branch = "feat/foreign".to_string(),
        Some("pr") => remote.pr = "17".to_string(),
        Some(other) => panic!("unknown claim mismatch {other}"),
        None => {}
    }
    fixture.transition_claim_ref(&remote);
    fixture.seed_claim_acquisition_receipt(worker_id, branch, claim_id);
    fixture.seed_expired_claim_heartbeat(worker_id, branch, claim_id);
    fixture.seed_interrupted_executor_invocation_without_cleanup(worker_id, branch, claim_id);
    let seeded = git_fixture(
        &fixture.claim_repo,
        &[
            "ls-remote",
            "--refs",
            fixture.claim_remote.to_str().expect("claim remote"),
            "refs/autospec/claims/issue-42",
        ],
    );
    let seeded = seeded.split_whitespace().next().expect("seeded claim oid");
    git_fixture(
        &fixture.claim_repo,
        &[
            "push",
            bridge.remote.to_str().expect("bridge remote"),
            &format!("{seeded}:refs/autospec/claims/issue-42"),
        ],
    );
    fs::write(&fixture.mode, "reviewed\n").expect("seed claim label projection");

    let configure = |command: &mut Command| {
        command
            .env("PATH", path_with(&bridge.bin))
            .env("AUTOSPEC_FOREGROUND_REAL_BRIDGE", "1")
            .env("AUTOSPEC_BRIDGE_REMOTE", &bridge.remote)
            .env("AUTOSPEC_BRIDGE_MERGED", &bridge.merged)
            .env("AUTOSPEC_CLAIM_GIT_REMOTE", &bridge.remote)
            .env("AUTOSPEC_HARNESS_RUNTIME_ALIASES", &bridge.aliases)
            .env("AUTOSPEC_HANDOFF_DISPATCHER_KIND", "codex")
            .env_remove("AUTOSPEC_EXECUTOR_REVIEW_COMMAND");
    };
    let mut command = fixture.command();
    configure(&mut command);
    let output = command.output().expect("run missing-cleanup recovery");
    let receipt_exists = fixture
        .state_path()
        .with_extension("claim-acquisition.json")
        .exists();
    let returned_to_scan = fixture.read_state().phase() == ConductorPhase::Scan;
    let heartbeat_exists = fixture.heartbeats.join("o4_test_r4_repo/42.json").exists();
    let heartbeat_archived = fs::read_dir(
        fixture
            .heartbeats
            .join("o4_test_r4_repo/quarantine/startup-heartbeat-handoffs"),
    )
    .ok()
    .into_iter()
    .flatten()
    .filter_map(Result::ok)
    .filter_map(|entry| fs::read_to_string(entry.path()).ok())
    .any(|document| document.contains(claim_id));
    let fresh_run = mismatch.is_none().then(|| {
        let mut command = fixture.command();
        configure(&mut command);
        let output = command.output().expect("run fresh foreground generation");
        let heartbeat_documents = fs::read_dir(fixture.heartbeats.join("o4_test_r4_repo"))
            .expect("read fresh heartbeat directory")
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "json")
            })
            .filter_map(|entry| fs::read_to_string(entry.path()).ok())
            .collect::<Vec<_>>();
        let generation_is_fresh = heartbeat_documents.iter().any(|document| {
            serde_json::from_str::<serde_json::Value>(document)
                .ok()
                .and_then(|heartbeat| heartbeat["claim_id"].as_str().map(str::to_string))
                .is_some_and(|fresh_claim_id| fresh_claim_id != claim_id)
        });
        FreshHeartbeatRun {
            success: output.status.success(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            heartbeat_count: heartbeat_documents.len(),
            generation_is_fresh,
        }
    });

    MissingCleanupRecovery {
        success: output.status.success(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        receipt_exists,
        returned_to_scan,
        heartbeat_exists,
        heartbeat_archived,
        fresh_run,
    }
}

#[test]
fn foreground_recovers_active_claim_without_executor_invocation() {
    let _bridge_e2e = REAL_BRIDGE_E2E.lock().expect("real bridge E2E lock");
    let fixture = ForegroundFixture::new();
    let bridge = fixture.configure_real_bridge();
    fs::create_dir_all(fixture.state_path().parent().unwrap())
        .expect("create recovery state directory");
    fs::write(
        fixture.state_path(),
        selected_foreground_state()
            .transition(ConductorEvent::Claimed)
            .expect("seed claimed conductor")
            .transition(ConductorEvent::DispatchRecorded {
                outcome: ConductorOutcome::Blocked("executor_receipt_failed".to_string()),
            })
            .expect("seed receipt failure")
            .to_json(),
    )
    .expect("persist receipt-failed conductor state");
    fixture.seed_claim_state_with_id(
        "rust-foreground-conductor-recovered",
        "feat/autonomous-issue-42",
        "claimed",
        &fresh_iso_timestamp(),
        EXECUTOR_CLAIM_ID,
    );
    fixture.seed_claim_acquisition_receipt(
        "rust-foreground-conductor-recovered",
        "feat/autonomous-issue-42",
        EXECUTOR_CLAIM_ID,
    );
    let seeded = git_fixture(
        &fixture.claim_repo,
        &[
            "ls-remote",
            "--refs",
            fixture.claim_remote.to_str().expect("claim remote"),
            "refs/autospec/claims/issue-42",
        ],
    );
    let seeded = seeded.split_whitespace().next().expect("seeded claim oid");
    git_fixture(
        &fixture.claim_repo,
        &[
            "push",
            bridge.remote.to_str().expect("bridge remote"),
            &format!("{seeded}:refs/autospec/claims/issue-42"),
        ],
    );
    fs::write(&fixture.mode, "claimed\n").expect("seed claim label projection");
    assert!(
        !fixture.scoped_dir().join("executor").exists(),
        "the crash window must precede exact executor invocation persistence"
    );

    let output = fixture
        .command()
        .env("PATH", path_with(&bridge.bin))
        .env("AUTOSPEC_FOREGROUND_REAL_BRIDGE", "1")
        .env("AUTOSPEC_BRIDGE_REMOTE", &bridge.remote)
        .env("AUTOSPEC_BRIDGE_MERGED", &bridge.merged)
        .env("AUTOSPEC_CLAIM_GIT_REMOTE", &bridge.remote)
        .env("AUTOSPEC_HARNESS_RUNTIME_ALIASES", &bridge.aliases)
        .env("AUTOSPEC_HANDOFF_DISPATCHER_KIND", "codex")
        .env_remove("AUTOSPEC_EXECUTOR_REVIEW_COMMAND")
        .output()
        .expect("recover claimed pre-invocation dispatch");

    assert!(
        output.status.success(),
        "stdout={} stderr={} calls={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
        fs::read_to_string(&fixture.calls).unwrap_or_default()
    );
    assert_eq!(
        fixture.read_state().phase(),
        ConductorPhase::Scan,
        "an active exact claim with no invocation must retry dispatch"
    );
    let calls = fs::read_to_string(&fixture.calls).expect("bridge calls");
    assert_eq!(calls.matches("\npr\ncreate\n").count(), 1);
    assert_eq!(calls.matches("\npr\nmerge\n").count(), 1);
}

#[test]
fn foreground_repeated_restart_observes_one_live_harness_until_merge() {
    let _bridge_e2e = REAL_BRIDGE_E2E.lock().expect("real bridge E2E lock");
    let fixture = ForegroundFixture::new();
    let bridge = fixture.configure_real_bridge();
    let started = fixture.root.join("slow-harness.started");
    let launches = fixture.root.join("harness-launches");
    let configure = |command: &mut Command| {
        command
            .env("PATH", path_with(&bridge.bin))
            .env("AUTOSPEC_FOREGROUND_REAL_BRIDGE", "1")
            .env("AUTOSPEC_BRIDGE_REMOTE", &bridge.remote)
            .env("AUTOSPEC_BRIDGE_MERGED", &bridge.merged)
            .env("AUTOSPEC_CLAIM_GIT_REMOTE", &bridge.remote)
            .env("AUTOSPEC_HARNESS_RUNTIME_ALIASES", &bridge.aliases)
            .env("AUTOSPEC_HANDOFF_DISPATCHER_KIND", "codex")
            .env_remove("AUTOSPEC_EXECUTOR_REVIEW_COMMAND")
            .env("AUTOSPEC_HOST", "bridge-e2e-host")
            .env("AUTOSPEC_CLAIM_LEASE_SECONDS", "1")
            .env("AUTOSPEC_BRIDGE_SLOW_HARNESS_MARKER", &started)
            .env("AUTOSPEC_BRIDGE_SLOW_HARNESS_SECONDS", "6")
            .env("AUTOSPEC_BRIDGE_HARNESS_LAUNCHES", &launches);
    };

    let mut first_command = fixture.command();
    configure(&mut first_command);
    let mut first = first_command.spawn().expect("spawn first conductor");
    let first_pid = first.id();
    wait_for_file_contents(&started, "");
    let harness_pid = fs::read_to_string(&started)
        .expect("harness marker")
        .trim()
        .parse::<u32>()
        .expect("harness PID");
    let invocation_path = fs::read_dir(fixture.scoped_dir().join("executor"))
        .expect("executor state directory")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| is_generation_invocation_path(path, 42))
        .expect("live executor invocation");
    let invocation: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(&invocation_path).expect("read live executor invocation"),
    )
    .expect("parse live executor invocation");
    let supervisor_pid = invocation["supervisor"]["pid"]
        .as_u64()
        .and_then(|pid| u32::try_from(pid).ok())
        .expect("persisted supervisor PID");
    let before_message = git_fixture(
        &fixture.root,
        &[
            "--git-dir",
            bridge.remote.to_str().expect("bridge remote"),
            "show",
            "-s",
            "--format=%B",
            "refs/autospec/claims/issue-42",
        ],
    );
    let before_claim = parse_run_state_comment(&before_message).expect("initial live claim");
    first.kill().expect("kill conductor in live-harness window");
    let _ = first.wait();

    let mut live_restart = fixture.command();
    configure(&mut live_restart);
    live_restart.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut replacement = live_restart.spawn().expect("restart while harness is live");
    std::thread::sleep(std::time::Duration::from_millis(2500));
    let refreshed_message = git_fixture(
        &fixture.root,
        &[
            "--git-dir",
            bridge.remote.to_str().expect("bridge remote"),
            "show",
            "-s",
            "--format=%B",
            "refs/autospec/claims/issue-42",
        ],
    );
    let refreshed_claim =
        parse_run_state_comment(&refreshed_message).expect("refreshed adopted claim");
    assert_eq!(refreshed_claim.claim_id, before_claim.claim_id);
    let replacement_status = replacement.try_wait().expect("inspect replacement");
    let replacement_stderr = if replacement_status.is_some() {
        let mut stderr = String::new();
        replacement
            .stderr
            .take()
            .expect("replacement stderr")
            .read_to_string(&mut stderr)
            .expect("read replacement stderr");
        stderr
    } else {
        String::new()
    };
    assert_ne!(
        refreshed_claim.updated_at, before_claim.updated_at,
        "replacement must renew the exact claim while the adopted harness remains live; status={replacement_status:?} stderr={replacement_stderr} invocation={} calls={}",
        fs::read_to_string(&invocation_path).unwrap_or_default(),
        fs::read_to_string(&fixture.calls).unwrap_or_default()
    );
    assert!(
        Path::new(&format!("/proc/{harness_pid}")).exists(),
        "harness must still be live when renewal is observed"
    );
    let completed = replacement
        .wait_with_output()
        .expect("wait for adopting replacement");
    assert!(
        completed.status.success(),
        "adopting restart status={:?} stdout={} stderr={} state={}",
        completed.status.code(),
        String::from_utf8_lossy(&completed.stdout),
        String::from_utf8_lossy(&completed.stderr),
        // Keep the persisted owner visible if dead-owner reclamation regresses.
        fs::read_to_string(fixture.resilience_state_path()).unwrap_or_else(|_| {
            format!("missing resilience state for killed conductor {first_pid}")
        })
    );
    assert_eq!(
        fixture.read_state().phase(),
        ConductorPhase::Scan,
        "replacement conductor must supervise the live harness through completion; stdout={} stderr={} calls={}",
        String::from_utf8_lossy(&completed.stdout),
        String::from_utf8_lossy(&completed.stderr),
        fs::read_to_string(&fixture.calls).unwrap_or_default()
    );
    assert_eq!(
        fs::read_to_string(&launches)
            .expect("harness launches")
            .lines()
            .count(),
        1
    );

    assert_eq!(
        fs::read_to_string(&launches)
            .expect("harness launches")
            .lines()
            .count(),
        1,
        "restarts must adopt the one persisted harness"
    );
    let calls = fs::read_to_string(&fixture.calls).expect("bridge calls");
    assert_eq!(calls.matches("\npr\ncreate\n").count(), 1);
    assert_eq!(calls.matches("\npr\nmerge\n").count(), 1);
    assert!(!Path::new(&format!("/proc/{supervisor_pid}")).exists());
    assert!(!Path::new(&format!("/proc/{harness_pid}")).exists());
}

fn assert_released_heartbeat_generation_handoff() {
    let _bridge_e2e = REAL_BRIDGE_E2E.lock().expect("real bridge E2E lock");
    let fixture = ForegroundFixture::new();
    let bridge = fixture.configure_real_bridge();
    let zero_effect_once = fixture.root.join("harness.zero-effect");
    let deleted_worktree = zero_effect_once.with_extension("deleted");
    let terminal_transition_once = fixture.root.join("terminal-transition.failed");
    let launches = fixture.root.join("harness-launches");
    let configure = |command: &mut Command| {
        command
            .env("PATH", path_with(&bridge.bin))
            .env("AUTOSPEC_FOREGROUND_REAL_BRIDGE", "1")
            .env("AUTOSPEC_BRIDGE_REMOTE", &bridge.remote)
            .env("AUTOSPEC_BRIDGE_MERGED", &bridge.merged)
            .env("AUTOSPEC_CLAIM_GIT_REMOTE", &bridge.remote)
            .env("AUTOSPEC_HARNESS_RUNTIME_ALIASES", &bridge.aliases)
            .env("AUTOSPEC_HANDOFF_DISPATCHER_KIND", "codex")
            .env_remove("AUTOSPEC_EXECUTOR_REVIEW_COMMAND")
            .env("AUTOSPEC_BRIDGE_ZERO_EFFECT_ONCE", &zero_effect_once)
            .env(
                "AUTOSPEC_TEST_FAILURE_TRANSITION_FAIL_ONCE",
                &terminal_transition_once,
            )
            .env("AUTOSPEC_BRIDGE_HARNESS_LAUNCHES", &launches);
    };
    let mut first_command = fixture.command();
    configure(&mut first_command);
    let first = first_command
        .output()
        .expect("seed exact zero-effect completion");

    assert!(
        first.status.success(),
        "stdout={} stderr={} calls={}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr),
        fs::read_to_string(&fixture.calls).unwrap_or_default()
    );
    assert!(
        zero_effect_once.exists(),
        "zero-effect fixture did not run; stdout={} stderr={} state={:?} calls={}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr),
        fixture.read_state(),
        fs::read_to_string(&fixture.calls).unwrap_or_default()
    );
    assert_eq!(fixture.read_state().phase(), ConductorPhase::Paused);
    assert_eq!(
        fixture.read_state().pause_reason(),
        Some("executor_receipt_failed")
    );
    wait_for_file_contents(&deleted_worktree, "");
    let invocation = fs::read_dir(fixture.scoped_dir().join("executor"))
        .expect("executor state directory")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| is_generation_invocation_path(path, 42))
        .expect("zero-effect invocation");
    let mut zero_effect_state: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(&invocation).expect("read zero-effect invocation"),
    )
    .expect("parse zero-effect invocation");
    let zero_effect_worktree = PathBuf::from(
        zero_effect_state["identity"]["worktree"]
            .as_str()
            .expect("zero-effect worktree"),
    );
    let missing_scope = zero_effect_worktree.parent().expect("zero-effect scope");
    fs::remove_dir_all(missing_scope).expect("remove complete zero-effect scope");
    let registry = git_fixture(&fixture.repo_dir, &["worktree", "list", "--porcelain"]);
    assert!(
        registry.contains(&format!(
            "worktree {}",
            zero_effect_state["identity"]["worktree"]
                .as_str()
                .expect("zero-effect worktree")
        )) && registry.contains("branch refs/heads/feat/autonomous-issue-42")
            && registry.lines().any(|line| line.starts_with("prunable ")),
        "real conductor fixture must enter the exact prunable branch state: {registry}"
    );
    zero_effect_state["phase"] = serde_json::json!("implementation_complete");
    fs::write(&invocation, format!("{zero_effect_state}\n"))
        .expect("seed exact post-child completion phase");

    let mut retry_command = fixture.command();
    configure(&mut retry_command);
    let output = retry_command
        .output()
        .expect("retry exact zero-effect completion");
    assert!(
        terminal_transition_once.is_file(),
        "terminal transition failpoint did not fire; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fixture.read_state().phase(),
        ConductorPhase::Paused,
        "terminal claim crash must preserve paused dispatch recovery"
    );
    let terminal_receipt = invocation.with_extension("terminal.json");
    assert!(
        invocation
            .with_extension("zero-effect-recovery.json")
            .is_file(),
        "terminal claim transition must preserve the zero-effect recovery marker"
    );
    assert!(
        !terminal_receipt.exists(),
        "terminal transition crash must precede receipt publication"
    );
    let mut completed_state: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(&invocation).expect("read released zero-effect invocation"),
    )
    .expect("parse released zero-effect invocation");
    completed_state["phase"] = serde_json::json!("complete");
    completed_state["terminal_result"] =
        serde_json::json!("retryable:executor_zero_effect_completion");
    fs::write(&invocation, format!("{completed_state}\n"))
        .expect("seed Complete invocation before terminal receipt publication");
    let transfer_path = missing_scope.join("issue-42.ownership-transfer.json");
    let transfer: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(&transfer_path).expect("read available ownership transfer"),
    )
    .expect("parse available ownership transfer");
    assert_eq!(
        transfer["state"], "available",
        "old generation must release the exact worktree before the fresh claim"
    );
    let old_claim_id = transfer["from_claim_id"]
        .as_str()
        .expect("old transfer claim")
        .to_string();
    let old_invocation_id = transfer["from_invocation_id"]
        .as_str()
        .expect("old transfer invocation")
        .to_string();
    assert!(
        zero_effect_worktree.is_dir(),
        "old-generation recovery must first restore the exact worktree"
    );
    fs::remove_dir_all(&zero_effect_worktree)
        .expect("make the recovered worktree registration prunable");
    let registry = git_fixture(&fixture.repo_dir, &["worktree", "list", "--porcelain"]);
    let expected_path = format!("worktree {}", zero_effect_worktree.display());
    let expected_branch = "branch refs/heads/feat/autonomous-issue-42";
    let prunable = registry.split("\n\n").any(|block| {
        block.lines().any(|line| line == expected_path)
            && block.lines().any(|line| line == expected_branch)
            && block.lines().any(|line| line.starts_with("prunable "))
    });
    assert!(
        prunable,
        "fresh generation must begin from the exact prunable registration: {registry}"
    );

    let mut terminal_recovery_command = fixture.command();
    configure(&mut terminal_recovery_command);
    let output = terminal_recovery_command
        .output()
        .expect("resume exact terminal failure intent");
    assert_eq!(
        fixture.read_state().phase(),
        ConductorPhase::Scan,
        "state={:?} stderr={} calls={}",
        fixture.read_state(),
        String::from_utf8_lossy(&output.stderr),
        fs::read_to_string(&fixture.calls).unwrap_or_default()
    );
    assert_eq!(
        fs::read_to_string(&launches)
            .expect("harness launch ledger")
            .lines()
            .count(),
        2,
        "one zero-effect launch must be followed by exactly one fresh-generation launch"
    );

    let terminal_receipts = fs::read_dir(fixture.scoped_dir().join("executor"))
        .expect("executor terminal receipts")
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            path.to_string_lossy()
                .ends_with(".terminal.json")
                .then(|| fs::read_to_string(path).ok())
                .flatten()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        terminal_receipts.len(),
        2,
        "retry and merged generations must both remain durable"
    );
    assert!(
        terminal_receipts.iter().any(|receipt| {
            receipt.contains("\"status\":\"retryable\"")
                && receipt.contains("\"reason\":\"executor_zero_effect_completion\"")
                && receipt.contains("\"claim_released\":true")
        }),
        "the exact old generation was not finalized as a released typed retry: {terminal_receipts:?}"
    );
    assert!(
        terminal_receipts
            .iter()
            .any(|receipt| receipt.contains("\"status\":\"merged\"")),
        "the fresh generation did not merge: {terminal_receipts:?}"
    );
    let merged_receipt = terminal_receipts
        .iter()
        .filter_map(|receipt| serde_json::from_str::<serde_json::Value>(receipt).ok())
        .find(|receipt| receipt["status"] == "merged")
        .expect("fresh merged terminal receipt");
    let adopted_transfer: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(&transfer_path).expect("read adopted ownership transfer"),
    )
    .expect("parse adopted ownership transfer");
    assert_eq!(adopted_transfer["state"], "adopted");
    assert_eq!(adopted_transfer["from_claim_id"], old_claim_id);
    assert_eq!(adopted_transfer["from_invocation_id"], old_invocation_id);
    assert_eq!(
        adopted_transfer["to_claim_id"], merged_receipt["claim_id"],
        "fresh merged claim must own the reclaimed worktree"
    );
    assert_eq!(
        adopted_transfer["to_invocation_id"], merged_receipt["invocation_id"],
        "fresh merged invocation must own the reclaimed worktree"
    );
    let archive = fixture
        .heartbeats
        .join("o4_test_r4_repo/quarantine/startup-heartbeat-handoffs");
    let archived = fs::read_dir(&archive)
        .expect("released heartbeat archive")
        .filter_map(Result::ok)
        .filter_map(|entry| fs::read_to_string(entry.path()).ok())
        .filter(|document| document.contains(&old_claim_id))
        .count();
    assert_eq!(archived, 1, "archive must retain the exact old generation");
}
#[test]
fn foreground_reclaims_prunable_zero_effect_branch_on_a_fresh_claim_generation() {
    assert_released_heartbeat_generation_handoff();
}
#[test]
fn released_heartbeat_generation_handoff() {
    assert_released_heartbeat_generation_handoff();
}

#[test]
fn immediate_stop_after_claim_prevents_retry_claim_and_executor() {
    let _bridge_e2e = REAL_BRIDGE_E2E.lock().expect("real bridge E2E lock");
    let fixture = ForegroundFixture::new();
    let bridge = fixture.configure_real_bridge();
    let fail_once = fixture.root.join("harness.failed");
    let harness_launches = fixture.root.join("harness.launches");
    let output = fixture
        .command()
        .env("PATH", path_with(&bridge.bin))
        .env("AUTOSPEC_FOREGROUND_REAL_BRIDGE", "1")
        .env("AUTOSPEC_BRIDGE_REMOTE", &bridge.remote)
        .env("AUTOSPEC_BRIDGE_MERGED", &bridge.merged)
        .env("AUTOSPEC_CLAIM_GIT_REMOTE", &bridge.remote)
        .env("AUTOSPEC_HARNESS_RUNTIME_ALIASES", &bridge.aliases)
        .env("AUTOSPEC_HANDOFF_DISPATCHER_KIND", "codex")
        .env_remove("AUTOSPEC_EXECUTOR_REVIEW_COMMAND")
        .env("AUTOSPEC_BRIDGE_FAIL_HARNESS_ONCE", &fail_once)
        .env("AUTOSPEC_BRIDGE_HARNESS_LAUNCHES", &harness_launches)
        .env(
            "FOREGROUND_STOP_ON_RETRYABLE_RELEASE",
            fixture.scoped_stop_sentinel(),
        )
        .output()
        .expect("run immediate stop at retry boundary");
    assert_eq!(output.status.code(), Some(20));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "{\"decision\":\"stop\",\"mode\":\"immediate\"}\n"
    );
    assert_eq!(
        fs::read_to_string(&harness_launches)
            .expect("harness launch ledger")
            .lines()
            .count(),
        1,
        "an immediate stop at the retry boundary must not dispatch a successor executor"
    );
    let calls = fs::read_to_string(&fixture.calls).expect("bridge calls");
    assert_eq!(
        calls.matches("--add-label\nin-progress-by-bot").count(),
        1,
        "an immediate stop at the retry boundary must not acquire a successor claim"
    );
    assert_eq!(calls.matches("\npr\ncreate\n").count(), 0);
    assert_eq!(calls.matches("\npr\nmerge\n").count(), 0);
    assert!(
        fs::read_to_string(fixture.resilience_state_path())
            .expect("read released lifecycle state")
            .contains("\"status\":\"released\""),
        "the stop boundary must release lifecycle ownership"
    );
}

#[test]
fn graceful_stop_after_claim_allows_retry_to_finish_issue() {
    let _bridge_e2e = REAL_BRIDGE_E2E.lock().expect("real bridge E2E lock");
    let fixture = ForegroundFixture::new();
    let bridge = fixture.configure_real_bridge();
    let fail_once = fixture.root.join("harness.failed");
    let harness_launches = fixture.root.join("harness.launches");
    let output = fixture
        .command()
        .env("PATH", path_with(&bridge.bin))
        .env("AUTOSPEC_FOREGROUND_REAL_BRIDGE", "1")
        .env("AUTOSPEC_BRIDGE_REMOTE", &bridge.remote)
        .env("AUTOSPEC_BRIDGE_MERGED", &bridge.merged)
        .env("AUTOSPEC_CLAIM_GIT_REMOTE", &bridge.remote)
        .env("AUTOSPEC_HARNESS_RUNTIME_ALIASES", &bridge.aliases)
        .env("AUTOSPEC_HANDOFF_DISPATCHER_KIND", "codex")
        .env_remove("AUTOSPEC_EXECUTOR_REVIEW_COMMAND")
        .env("AUTOSPEC_BRIDGE_FAIL_HARNESS_ONCE", &fail_once)
        .env("AUTOSPEC_BRIDGE_HARNESS_LAUNCHES", &harness_launches)
        .env(
            "FOREGROUND_STOP_ON_RETRYABLE_RELEASE",
            fixture.scoped_stop_sentinel(),
        )
        .env("FOREGROUND_STOP_MODE_ON_RETRYABLE_RELEASE", "graceful")
        .output()
        .expect("run graceful stop at retry boundary");
    assert!(
        output.status.success(),
        "stdout={} stderr={} calls={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
        fs::read_to_string(&fixture.calls).unwrap_or_default()
    );
    assert_eq!(
        fs::read_to_string(&harness_launches)
            .expect("harness launch ledger")
            .lines()
            .count(),
        2,
        "a graceful stop must allow the current issue's successor executor"
    );
    let calls = fs::read_to_string(&fixture.calls).expect("bridge calls");
    assert_eq!(
        calls.matches("--add-label\nin-progress-by-bot").count(),
        2,
        "a graceful stop must allow the current issue's successor claim"
    );
    assert_eq!(calls.matches("\npr\ncreate\n").count(), 1);
    assert_eq!(calls.matches("\npr\nmerge\n").count(), 1);
    assert_eq!(fixture.read_state().phase(), ConductorPhase::Scan);
}

#[test]
fn foreground_retry_preserves_dirty_wip_and_merges_on_a_fresh_claim_generation() {
    let _bridge_e2e = REAL_BRIDGE_E2E.lock().expect("real bridge E2E lock");
    let fixture = ForegroundFixture::new();
    let bridge = fixture.configure_real_bridge();
    let fail_once = fixture.root.join("harness.failed");
    let output = fixture
        .command()
        .env("PATH", path_with(&bridge.bin))
        .env("AUTOSPEC_FOREGROUND_REAL_BRIDGE", "1")
        .env("AUTOSPEC_BRIDGE_REMOTE", &bridge.remote)
        .env("AUTOSPEC_BRIDGE_MERGED", &bridge.merged)
        .env("AUTOSPEC_CLAIM_GIT_REMOTE", &bridge.remote)
        .env("AUTOSPEC_HARNESS_RUNTIME_ALIASES", &bridge.aliases)
        .env("AUTOSPEC_HANDOFF_DISPATCHER_KIND", "codex")
        .env_remove("AUTOSPEC_EXECUTOR_REVIEW_COMMAND")
        .env("AUTOSPEC_BRIDGE_FAIL_HARNESS_ONCE", &fail_once)
        .env("AUTOSPEC_BRIDGE_ADVANCE_MAIN_ON_FAIL", "1")
        .output()
        .expect("retry dirty executor WIP");
    assert!(
        output.status.success(),
        "stdout={} stderr={} calls={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
        fs::read_to_string(&fixture.calls).unwrap_or_default()
    );
    assert!(fail_once.exists());
    assert_eq!(
        fixture.read_state().phase(),
        ConductorPhase::Scan,
        "state={:?} stderr={} calls={}",
        fixture.read_state(),
        String::from_utf8_lossy(&output.stderr),
        fs::read_to_string(&fixture.calls).unwrap_or_default()
    );
    let calls = fs::read_to_string(&fixture.calls).expect("bridge calls");
    assert_eq!(calls.matches("\npr\ncreate\n").count(), 1);
    assert_eq!(calls.matches("\npr\nmerge\n").count(), 1);
    let receipts = fs::read_dir(fixture.scoped_dir().join("executor"))
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.path().to_string_lossy().ends_with(".terminal.json"))
        .count();
    assert_eq!(receipts, 2, "retry and merged generations are both durable");

    let executor_dir = fixture.scoped_dir().join("executor");
    let merged_terminal = fs::read_dir(&executor_dir)
        .expect("executor generations")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.to_string_lossy().ends_with(".terminal.json")
                && fs::read_to_string(path)
                    .ok()
                    .and_then(|body| serde_json::from_str::<serde_json::Value>(&body).ok())
                    .and_then(|receipt| receipt["status"].as_str().map(str::to_string))
                    .as_deref()
                    == Some("merged")
        })
        .expect("merged terminal receipt");
    let merged_receipt: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(&merged_terminal).expect("read merged terminal receipt"),
    )
    .expect("parse merged terminal receipt");
    fs::remove_file(&merged_terminal).expect("simulate crash before merged receipt publication");
    let replay_state = ConductorState::new("test/repo", ConductorScope::Repository, 3)
        .unwrap()
        .transition(ConductorEvent::ScanFoundWork)
        .unwrap()
        .transition(ConductorEvent::SafetyReviewed)
        .unwrap()
        .transition(ConductorEvent::Selected {
            issue: 42,
            serialization_reasons: Vec::new(),
        })
        .unwrap()
        .transition(ConductorEvent::Claimed)
        .unwrap();
    fs::write(fixture.state_path(), replay_state.to_json()).expect("seed merged replay crash");
    fixture.seed_claim_acquisition_receipt(
        merged_receipt["worker_id"].as_str().expect("merged worker"),
        merged_receipt["branch"].as_str().expect("merged branch"),
        merged_receipt["claim_id"].as_str().expect("merged claim"),
    );
    let calls_before = fs::read_to_string(&fixture.calls).expect("calls before replay");
    let replay = fixture
        .command()
        .env("PATH", path_with(&bridge.bin))
        .env("AUTOSPEC_FOREGROUND_REAL_BRIDGE", "1")
        .env("AUTOSPEC_BRIDGE_REMOTE", &bridge.remote)
        .env("AUTOSPEC_BRIDGE_MERGED", &bridge.merged)
        .env("AUTOSPEC_CLAIM_GIT_REMOTE", &bridge.remote)
        .env("AUTOSPEC_HARNESS_RUNTIME_ALIASES", &bridge.aliases)
        .env("AUTOSPEC_HANDOFF_DISPATCHER_KIND", "codex")
        .env_remove("AUTOSPEC_EXECUTOR_REVIEW_COMMAND")
        .output()
        .expect("replay exact merged generation");
    assert!(
        replay.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&replay.stdout),
        String::from_utf8_lossy(&replay.stderr)
    );
    assert_eq!(fixture.read_state().phase(), ConductorPhase::Scan);
    let calls_after = fs::read_to_string(&fixture.calls).expect("calls after replay");
    assert_eq!(
        calls_after.matches("\npr\ncreate\n").count(),
        calls_before.matches("\npr\ncreate\n").count()
    );
    assert_eq!(
        calls_after.matches("\npr\nmerge\n").count(),
        calls_before.matches("\npr\nmerge\n").count()
    );
}

#[test]
fn foreground_post_harness_gh_read_outage_resumes_the_exact_claim() {
    let _bridge_e2e = REAL_BRIDGE_E2E.lock().expect("real bridge E2E lock");
    let fixture = ForegroundFixture::new();
    let bridge = fixture.configure_real_bridge();
    let fail_once = fixture.root.join("bridge-gh-read.failed");
    let harness_done = fixture.root.join("bridge-harness.done");
    let harness_launches = fixture.root.join("bridge-harness.launches");
    let output = fixture
        .command()
        .env("PATH", path_with(&bridge.bin))
        .env("AUTOSPEC_FOREGROUND_REAL_BRIDGE", "1")
        .env("AUTOSPEC_BRIDGE_REMOTE", &bridge.remote)
        .env("AUTOSPEC_BRIDGE_MERGED", &bridge.merged)
        .env("AUTOSPEC_CLAIM_GIT_REMOTE", &bridge.remote)
        .env("AUTOSPEC_HARNESS_RUNTIME_ALIASES", &bridge.aliases)
        .env("AUTOSPEC_HANDOFF_DISPATCHER_KIND", "codex")
        .env_remove("AUTOSPEC_EXECUTOR_REVIEW_COMMAND")
        .env("AUTOSPEC_BRIDGE_FAIL_GH_READ_ONCE", &fail_once)
        .env("AUTOSPEC_BRIDGE_HARNESS_DONE", &harness_done)
        .env("AUTOSPEC_BRIDGE_HARNESS_LAUNCHES", &harness_launches)
        .env("AUTOSPEC_BRIDGE_FAIL_GH_AFTER_BRANCH", "1")
        .output()
        .expect("run transient bridge recovery");

    assert!(
        output.status.success(),
        "stdout={} stderr={} calls={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
        fs::read_to_string(&fixture.calls).unwrap_or_default()
    );
    assert!(fail_once.exists(), "bridge read failpoint must fire");
    assert_eq!(
        fixture.read_state().phase(),
        ConductorPhase::Scan,
        "state={:?} stdout={} stderr={} calls={}",
        fixture.read_state(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
        fs::read_to_string(&fixture.calls).unwrap_or_default()
    );
    let calls = fs::read_to_string(&fixture.calls).expect("bridge calls");
    assert_eq!(calls.matches("\npr\ncreate\n").count(), 1);
    assert_eq!(calls.matches("\npr\nmerge\n").count(), 1);
    assert_eq!(
        fs::read_to_string(&harness_launches)
            .expect("harness launch ledger")
            .lines()
            .count(),
        1,
        "late transport retries resume the exact invocation without rerunning the harness"
    );
}

#[test]
fn foreground_resumes_nonzero_draft_created_receipt_failure_without_second_harness() {
    let _bridge_e2e = REAL_BRIDGE_E2E.lock().expect("real bridge E2E lock");
    let fixture = ForegroundFixture::new();
    let bridge = fixture.configure_real_bridge();
    let harness_launches = fixture.root.join("draft-created-harness.launches");
    let configure = |command: &mut Command| {
        command
            .env("PATH", path_with(&bridge.bin))
            .env("AUTOSPEC_FOREGROUND_REAL_BRIDGE", "1")
            .env("AUTOSPEC_BRIDGE_REMOTE", &bridge.remote)
            .env("AUTOSPEC_BRIDGE_MERGED", &bridge.merged)
            .env("AUTOSPEC_CLAIM_GIT_REMOTE", &bridge.remote)
            .env("AUTOSPEC_HARNESS_RUNTIME_ALIASES", &bridge.aliases)
            .env("AUTOSPEC_HANDOFF_DISPATCHER_KIND", "codex")
            .env_remove("AUTOSPEC_EXECUTOR_REVIEW_COMMAND")
            .env("AUTOSPEC_BRIDGE_HARNESS_LAUNCHES", &harness_launches);
    };

    let mut first_command = fixture.command();
    configure(&mut first_command);
    first_command.env("AUTOSPEC_BRIDGE_FAIL_GH_AFTER_CREATE_ALWAYS", "1");
    let first = first_command
        .output()
        .expect("persist nonzero DraftCreated invocation");
    assert!(
        first.status.success(),
        "stdout={} stderr={} calls={}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr),
        fs::read_to_string(&fixture.calls).unwrap_or_default()
    );
    assert_eq!(fixture.read_state().phase(), ConductorPhase::Dispatch);
    let invocation = fs::read_dir(fixture.scoped_dir().join("executor"))
        .expect("executor state directory")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| is_generation_invocation_path(path, 42))
        .expect("generation-scoped invocation");
    let mut invocation_state: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(&invocation).expect("read DraftCreated invocation"),
    )
    .expect("parse DraftCreated invocation");
    assert_eq!(
        invocation_state["phase"], "draft_creating",
        "the injected authoritative reread outage must follow the real draft mutation"
    );
    assert!(
        invocation_state["draft_process"].is_object(),
        "successful draft creation must retain its exited gh process identity"
    );
    invocation_state["phase"] = serde_json::json!("draft_created");
    invocation_state["pr"] = serde_json::json!(17);
    fs::write(&invocation, format!("{invocation_state}\n"))
        .expect("seed exact DraftCreated gate-failure state");
    let acquisition_path = fixture
        .state_path()
        .with_extension("claim-acquisition.json");
    assert!(
        acquisition_path.is_file(),
        "exact local acquisition must survive the transient post-create outage"
    );
    let acquisition: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(&acquisition_path).expect("read exact local acquisition"),
    )
    .expect("parse exact local acquisition");
    for field in ["worker_id", "branch", "claim_id"] {
        assert_eq!(
            invocation_state["identity"][field], acquisition[field],
            "persisted invocation must retain the acquired {field}"
        );
    }
    fs::write(
        fixture.state_path(),
        selected_foreground_state()
            .transition(ConductorEvent::Claimed)
            .expect("seed claimed conductor")
            .transition(ConductorEvent::DispatchRecorded {
                outcome: ConductorOutcome::Blocked("executor_receipt_failed".to_string()),
            })
            .expect("seed receipt failure")
            .to_json(),
    )
    .expect("persist receipt-failed conductor state");
    let before = fs::read_to_string(&fixture.calls).expect("calls before recovery");
    assert_eq!(
        fs::read_to_string(&harness_launches)
            .expect("initial harness launch ledger")
            .lines()
            .count(),
        1
    );

    let mut recovery = fixture.command();
    configure(&mut recovery);
    let recovered = recovery
        .output()
        .expect("resume exact nonzero DraftCreated invocation");
    assert!(
        recovered.status.success(),
        "stdout={} stderr={} state={:?} calls={}",
        String::from_utf8_lossy(&recovered.stdout),
        String::from_utf8_lossy(&recovered.stderr),
        fixture.read_state(),
        fs::read_to_string(&fixture.calls).unwrap_or_default()
    );
    assert_eq!(fixture.read_state().phase(), ConductorPhase::Scan);
    assert_eq!(
        fs::read_to_string(&harness_launches)
            .expect("recovered harness launch ledger")
            .lines()
            .count(),
        1,
        "receipt recovery must resume the exact invocation without launching another implementer"
    );
    let after = fs::read_to_string(&fixture.calls).expect("calls after recovery");
    assert_eq!(
        after.matches("\npr\ncreate\n").count(),
        before.matches("\npr\ncreate\n").count(),
        "receipt recovery must adopt the existing draft"
    );
    assert_eq!(after.matches("\npr\nmerge\n").count(), 1);
}

#[test]
fn foreground_retires_exact_merged_draft_when_worktree_is_missing() {
    let _bridge_e2e = REAL_BRIDGE_E2E.lock().expect("real bridge E2E lock");
    let fixture = ForegroundFixture::new();
    let bridge = fixture.configure_real_bridge();
    let harness_launches = fixture.root.join("merged-draft-harness.launches");
    let configure = |command: &mut Command| {
        command
            .env("PATH", path_with(&bridge.bin))
            .env("AUTOSPEC_FOREGROUND_REAL_BRIDGE", "1")
            .env("AUTOSPEC_BRIDGE_REMOTE", &bridge.remote)
            .env("AUTOSPEC_BRIDGE_MERGED", &bridge.merged)
            .env("AUTOSPEC_CLAIM_GIT_REMOTE", &bridge.remote)
            .env("AUTOSPEC_HARNESS_RUNTIME_ALIASES", &bridge.aliases)
            .env("AUTOSPEC_HANDOFF_DISPATCHER_KIND", "codex")
            .env_remove("AUTOSPEC_EXECUTOR_REVIEW_COMMAND")
            .env("AUTOSPEC_BRIDGE_HARNESS_LAUNCHES", &harness_launches);
    };

    let mut first_command = fixture.command();
    configure(&mut first_command);
    first_command.env("AUTOSPEC_BRIDGE_FAIL_GH_AFTER_CREATE_ALWAYS", "1");
    let first = first_command
        .output()
        .expect("persist a DraftCreated invocation");
    assert!(
        first.status.success(),
        "stdout={} stderr={} calls={}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr),
        fs::read_to_string(&fixture.calls).unwrap_or_default()
    );

    let invocation = fs::read_dir(fixture.scoped_dir().join("executor"))
        .expect("executor state directory")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| is_generation_invocation_path(path, 42))
        .expect("generation-scoped invocation");
    let mut invocation_state: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(&invocation).expect("read DraftCreated invocation"),
    )
    .expect("parse DraftCreated invocation");
    let persisted_head = invocation_state["head_oid"]
        .as_str()
        .expect("persisted implementation head")
        .to_string();
    let worktree = PathBuf::from(
        invocation_state["identity"]["worktree"]
            .as_str()
            .expect("persisted executor worktree"),
    );
    invocation_state["phase"] = serde_json::json!("draft_created");
    invocation_state["pr"] = serde_json::json!(17);
    fs::write(&invocation, format!("{invocation_state}\n"))
        .expect("seed exact DraftCreated gate-failure state");

    fs::write(
        worktree.join("reviewer-follow-up.txt"),
        "reviewer-owned follow-up\n",
    )
    .expect("write reviewer follow-up");
    git_fixture(&worktree, &["add", "reviewer-follow-up.txt"]);
    git_fixture(&worktree, &["commit", "-m", "test: add reviewer follow-up"]);
    let merged_head = git_fixture(&worktree, &["rev-parse", "HEAD"]);
    assert_ne!(
        merged_head, persisted_head,
        "the regression requires an externally advanced merged head"
    );
    let merge_oid = merged_head.clone();
    let mut pull_requests: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(&fixture.pull_requests).expect("read pull request fixture"),
    )
    .expect("parse pull request fixture");
    pull_requests[0]["headRefOid"] = serde_json::json!(merged_head);
    pull_requests[0]["isDraft"] = serde_json::json!(false);
    fs::write(&fixture.pull_requests, format!("{pull_requests}\n"))
        .expect("advance exact pull request head");
    fs::write(&bridge.merged, format!("{merge_oid}\n")).expect("mark exact PR merged");
    let mut exact_claim = fixture.claim_record_from(&bridge.remote);
    exact_claim.pr = "17".to_string();
    exact_claim.step = "verification".to_string();
    fixture.transition_claim_ref_to(&exact_claim, &bridge.remote);

    let unrelated = bridge.executor_fixture.join("unrelated-prunable");
    git_fixture(
        &fixture.repo_dir,
        &[
            "worktree",
            "add",
            "-b",
            "feat/unrelated-prunable",
            unrelated.to_str().expect("unrelated worktree path"),
            "main",
        ],
    );
    fs::remove_dir_all(&unrelated).expect("make unrelated registration prunable");
    fs::remove_dir_all(&worktree).expect("remove exact executor worktree");
    let registry_before = git_fixture(&fixture.repo_dir, &["worktree", "list", "--porcelain"]);
    assert!(registry_before.contains(worktree.to_str().expect("worktree path")));
    assert!(registry_before.contains(unrelated.to_str().expect("unrelated path")));
    assert_eq!(
        fixture.claim_record_from(&bridge.remote).pr,
        "17",
        "the authoritative generation must bind the exact pull request"
    );

    fs::write(
        fixture.state_path(),
        selected_foreground_state()
            .transition(ConductorEvent::Claimed)
            .expect("seed claimed conductor")
            .transition(ConductorEvent::DispatchRecorded {
                outcome: ConductorOutcome::Blocked("executor_receipt_failed".to_string()),
            })
            .expect("seed receipt failure")
            .to_json(),
    )
    .expect("persist receipt-failed conductor state");
    let calls_before = fs::read_to_string(&fixture.calls).expect("calls before recovery");
    let reconciliation_crash = fixture.root.join("merged-reconciliation.crashed");

    let mut interrupted_recovery = fixture.command();
    configure(&mut interrupted_recovery);
    interrupted_recovery.env(
        "AUTOSPEC_TEST_MERGED_RECONCILIATION_FAIL_ONCE",
        &reconciliation_crash,
    );
    let interrupted = interrupted_recovery
        .output()
        .expect("interrupt merged reconciliation after durable state");
    assert!(
        !interrupted.status.success(),
        "the crash boundary must interrupt the first recovery"
    );
    assert!(reconciliation_crash.exists());
    let interrupted_state: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(&invocation).expect("read interrupted invocation"),
    )
    .expect("parse interrupted invocation");
    assert_eq!(interrupted_state["phase"], "merged");
    assert_eq!(
        fixture.claim_record_from(&bridge.remote).state,
        "claimed",
        "the reconciliation record and Merged state precede claim terminalization"
    );

    let claim_transition_crash = fixture.root.join("merged-claim-transition.crashed");
    let mut transition_recovery = fixture.command();
    configure(&mut transition_recovery);
    transition_recovery.env(
        "AUTOSPEC_TEST_MERGED_CLAIM_TRANSITION_FAIL_ONCE",
        &claim_transition_crash,
    );
    let transition_interrupted = transition_recovery
        .output()
        .expect("interrupt after exact claim terminalization");
    assert!(
        !transition_interrupted.status.success(),
        "the claim transition boundary must interrupt recovery"
    );
    assert!(claim_transition_crash.exists());
    assert_eq!(
        fixture.claim_record_from(&bridge.remote).state,
        "merged",
        "the claim transition must be durable before local cleanup"
    );
    let post_transition_state: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(&invocation).expect("read post-transition invocation"),
    )
    .expect("parse post-transition invocation");
    assert_eq!(post_transition_state["phase"], "merged");

    let mut final_recovery = fixture.command();
    configure(&mut final_recovery);
    let recovered = final_recovery
        .output()
        .expect("resume and retire exact already-merged invocation");
    assert!(
        recovered.status.success(),
        "stdout={} stderr={} state={:?} calls={}",
        String::from_utf8_lossy(&recovered.stdout),
        String::from_utf8_lossy(&recovered.stderr),
        fixture.read_state(),
        fs::read_to_string(&fixture.calls).unwrap_or_default()
    );
    assert_eq!(fixture.read_state().phase(), ConductorPhase::Scan);
    assert_eq!(fixture.claim_record_from(&bridge.remote).state, "merged");
    assert_eq!(
        fs::read_to_string(&harness_launches)
            .expect("harness launch ledger")
            .lines()
            .count(),
        1,
        "terminal retirement must not relaunch the implementation harness"
    );
    let calls_after = fs::read_to_string(&fixture.calls).expect("calls after recovery");
    assert_eq!(
        calls_after.matches("\npr\ncreate\n").count(),
        calls_before.matches("\npr\ncreate\n").count()
    );
    assert_eq!(
        calls_after.matches("\npr\nmerge\n").count(),
        calls_before.matches("\npr\nmerge\n").count(),
        "terminal reconciliation must not attempt a second merge"
    );

    let reconciliation_path = invocation.with_extension("cleanup-merged-reconciliation");
    let reconciliation: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(&reconciliation_path).expect("read merged reconciliation record"),
    )
    .expect("parse merged reconciliation record");
    assert_eq!(reconciliation["persisted_head"], persisted_head);
    assert_eq!(reconciliation["merged_head"], merged_head);
    assert_eq!(reconciliation["merge_oid"], merge_oid);
    assert_eq!(reconciliation["pr"], 17);
    let terminal: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(invocation.with_extension("terminal.json"))
            .expect("read terminal bridge receipt"),
    )
    .expect("parse terminal bridge receipt");
    assert_eq!(
        terminal["head_oid"], merged_head,
        "the terminal receipt reports the actual merged PR head"
    );

    let registry_after = git_fixture(&fixture.repo_dir, &["worktree", "list", "--porcelain"]);
    assert!(!registry_after.contains(worktree.to_str().expect("exact worktree path")));
    assert!(
        registry_after.contains(unrelated.to_str().expect("unrelated path")),
        "exact cleanup must preserve unrelated prunable registrations"
    );
}

#[test]
fn foreground_persistent_post_create_outage_stays_on_the_exact_claim() {
    let _bridge_e2e = REAL_BRIDGE_E2E.lock().expect("real bridge E2E lock");
    let fixture = ForegroundFixture::new();
    let bridge = fixture.configure_real_bridge();
    let harness_launches = fixture.root.join("persistent-outage-harness.launches");
    let configure = |command: &mut Command| {
        command
            .env("PATH", path_with(&bridge.bin))
            .env("AUTOSPEC_FOREGROUND_REAL_BRIDGE", "1")
            .env("AUTOSPEC_BRIDGE_REMOTE", &bridge.remote)
            .env("AUTOSPEC_BRIDGE_MERGED", &bridge.merged)
            .env("AUTOSPEC_CLAIM_GIT_REMOTE", &bridge.remote)
            .env("AUTOSPEC_HARNESS_RUNTIME_ALIASES", &bridge.aliases)
            .env("AUTOSPEC_HANDOFF_DISPATCHER_KIND", "codex")
            .env_remove("AUTOSPEC_EXECUTOR_REVIEW_COMMAND")
            .env("AUTOSPEC_BRIDGE_HARNESS_LAUNCHES", &harness_launches);
    };
    let mut first_command = fixture.command();
    configure(&mut first_command);
    first_command.env("AUTOSPEC_BRIDGE_FAIL_GH_AFTER_CREATE_ALWAYS", "1");
    let first = first_command.output().expect("run persistent outage");
    assert!(
        first.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr)
    );
    assert_eq!(fixture.read_state().phase(), ConductorPhase::Dispatch);
    assert!(
        fixture
            .state_path()
            .with_extension("claim-acquisition.json")
            .is_file(),
        "resumable transient must retain its exact acquisition"
    );
    let before = fs::read_to_string(&fixture.calls).expect("calls during outage");
    assert_eq!(before.matches("\npr\ncreate\n").count(), 1);
    assert_eq!(before.matches("\npr\nmerge\n").count(), 0);

    let mut recovery = fixture.command();
    configure(&mut recovery);
    let recovered = recovery.output().expect("resume persistent outage");
    assert!(
        recovered.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&recovered.stdout),
        String::from_utf8_lossy(&recovered.stderr)
    );
    assert_eq!(fixture.read_state().phase(), ConductorPhase::Scan);
    let after = fs::read_to_string(&fixture.calls).expect("calls after outage");
    assert_eq!(after.matches("\npr\ncreate\n").count(), 1);
    assert_eq!(after.matches("\npr\nmerge\n").count(), 1);
    assert_eq!(
        fs::read_to_string(harness_launches)
            .expect("harness launch ledger")
            .lines()
            .count(),
        1
    );
}

#[test]
fn foreground_receipt_retirement_crash_windows_resume_without_replay() {
    let _bridge_e2e = REAL_BRIDGE_E2E.lock().expect("real bridge E2E lock");
    for (scenario, failpoint, fail_harness, expected_phase, receipt_remains) in [
        (
            "retry-before-clear",
            "before-clear",
            true,
            ConductorPhase::Retry,
            true,
        ),
        (
            "retry-after-clear",
            "after-clear",
            true,
            ConductorPhase::Retry,
            false,
        ),
        (
            "terminal-before-clear",
            "before-clear",
            false,
            ConductorPhase::Paused,
            true,
        ),
        (
            "terminal-after-clear",
            "after-clear",
            false,
            ConductorPhase::Paused,
            false,
        ),
    ] {
        let fixture = ForegroundFixture::new();
        let bridge = fixture.configure_real_bridge();
        let marker = fixture.root.join(format!("{scenario}.failed"));
        let configure = |command: &mut Command| {
            command
                .env("PATH", path_with(&bridge.bin))
                .env("AUTOSPEC_FOREGROUND_REAL_BRIDGE", "1")
                .env("AUTOSPEC_BRIDGE_REMOTE", &bridge.remote)
                .env("AUTOSPEC_BRIDGE_MERGED", &bridge.merged)
                .env("AUTOSPEC_CLAIM_GIT_REMOTE", &bridge.remote)
                .env("AUTOSPEC_HARNESS_RUNTIME_ALIASES", &bridge.aliases)
                .env("AUTOSPEC_HANDOFF_DISPATCHER_KIND", "codex")
                .env_remove("AUTOSPEC_EXECUTOR_REVIEW_COMMAND");
        };
        let mut first_command = fixture.command();
        configure(&mut first_command);
        first_command
            .env("AUTOSPEC_FOREGROUND_RETIRE_FAILPOINT", failpoint)
            .env("AUTOSPEC_FOREGROUND_RETIRE_FAIL_ONCE", &marker);
        if fail_harness {
            first_command.env(
                "AUTOSPEC_BRIDGE_FAIL_HARNESS_ONCE",
                fixture.root.join(format!("{scenario}.harness-failed")),
            );
        }
        let first = first_command.output().expect("run retirement failpoint");
        assert!(
            !first.status.success(),
            "{scenario}: failpoint must interrupt retirement"
        );
        assert_eq!(
            fixture.read_state().phase(),
            expected_phase,
            "{scenario}: old-generation state must remain durable"
        );
        assert_eq!(
            fixture
                .state_path()
                .with_extension("claim-acquisition.json")
                .exists(),
            receipt_remains,
            "{scenario}: receipt retirement boundary"
        );
        let calls_before = fs::read_to_string(&fixture.calls).expect("calls before recovery");

        let mut recovery = fixture.command();
        configure(&mut recovery);
        let recovered = recovery.output().expect("resume retirement crash");
        assert!(
            recovered.status.success(),
            "{scenario}: stdout={} stderr={}",
            String::from_utf8_lossy(&recovered.stdout),
            String::from_utf8_lossy(&recovered.stderr)
        );
        let recovered_state = fixture.read_state();
        if fail_harness {
            assert_eq!(
                recovered_state.phase(),
                ConductorPhase::Scan,
                "{scenario}: state={recovered_state:?}"
            );
        } else {
            assert!(
                matches!(
                    recovered_state.phase(),
                    ConductorPhase::Scan | ConductorPhase::Paused
                ) && recovered_state.selected_issue().is_none(),
                "{scenario}: finalized recovery must leave the old issue; state={recovered_state:?}"
            );
        }
        assert!(
            !fixture
                .state_path()
                .with_extension("claim-acquisition.json")
                .exists(),
            "{scenario}: recovery retires the old acquisition receipt"
        );
        let calls_after = fs::read_to_string(&fixture.calls).expect("calls after recovery");
        assert_eq!(
            calls_after.matches("\npr\ncreate\n").count(),
            calls_before.matches("\npr\ncreate\n").count(),
            "{scenario}: retired generation must not create another PR"
        );
        assert_eq!(
            calls_after.matches("\npr\nmerge\n").count(),
            calls_before.matches("\npr\nmerge\n").count(),
            "{scenario}: retired generation must not merge twice"
        );
    }
}

#[test]
fn foreground_normal_fourth_retry_pauses_after_three_completed_retries() {
    let mut state = selected_foreground_state();
    for completed in 1..=3 {
        state = state
            .transition(ConductorEvent::Claimed)
            .expect("claim retry generation")
            .transition(ConductorEvent::DispatchRecorded {
                outcome: ConductorOutcome::Retryable("transient failure".to_string()),
            })
            .expect("record completed retry");
        assert_eq!(state.phase(), ConductorPhase::Retry);
        assert_eq!(state.retry_count(), completed);
        state = state
            .transition(ConductorEvent::RetryScheduled)
            .expect("schedule allowed retry");
    }

    let exhausted = state
        .transition(ConductorEvent::Claimed)
        .expect("claim fourth generation")
        .transition(ConductorEvent::DispatchRecorded {
            outcome: ConductorOutcome::Retryable("transient failure".to_string()),
        })
        .expect("record fourth retry");

    assert_eq!(exhausted.phase(), ConductorPhase::Paused);
    assert_eq!(exhausted.retry_count(), 4);
    assert_eq!(exhausted.pause_reason(), Some("retry_limit_exhausted"));
}

#[test]
fn foreground_exhausted_retry_recovers_after_receipt_retirement_crash() {
    let _bridge_e2e = REAL_BRIDGE_E2E.lock().expect("real bridge E2E lock");
    let fixture = ForegroundFixture::new();
    let bridge = fixture.configure_real_bridge();
    fs::create_dir_all(fixture.state_path().parent().unwrap())
        .expect("create recovery state directory");
    let dispatch = selected_foreground_state()
        .transition(ConductorEvent::Claimed)
        .expect("seed dispatch")
        .to_json()
        .replace("\"retry_count\":0", "\"retry_count\":3");
    fs::write(
        fixture.state_path(),
        ConductorState::parse_json(&dispatch)
            .expect("parse exhausted dispatch")
            .to_json(),
    )
    .expect("seed exhausted dispatch");
    fixture.seed_claim_state_with_id(
        "rust-foreground-conductor-recovered",
        "feat/autonomous-issue-42",
        "claimed",
        &fresh_iso_timestamp(),
        EXECUTOR_CLAIM_ID,
    );
    fixture.seed_claim_heartbeat(
        "rust-foreground-conductor-recovered",
        "feat/autonomous-issue-42",
        EXECUTOR_CLAIM_ID,
    );
    fixture.seed_claim_acquisition_receipt(
        "rust-foreground-conductor-recovered",
        "feat/autonomous-issue-42",
        EXECUTOR_CLAIM_ID,
    );
    fixture.copy_claim_ref_to(&bridge.remote);
    let crash_marker = fixture.root.join("exhausted-after-clear.failed");
    let configure = |command: &mut Command| {
        command
            .env("PATH", path_with(&bridge.bin))
            .env("AUTOSPEC_FOREGROUND_REAL_BRIDGE", "1")
            .env("AUTOSPEC_BRIDGE_REMOTE", &bridge.remote)
            .env("AUTOSPEC_BRIDGE_MERGED", &bridge.merged)
            .env("AUTOSPEC_CLAIM_GIT_REMOTE", &bridge.remote)
            .env("AUTOSPEC_HARNESS_RUNTIME_ALIASES", &bridge.aliases)
            .env("AUTOSPEC_HANDOFF_DISPATCHER_KIND", "codex")
            .env_remove("AUTOSPEC_EXECUTOR_REVIEW_COMMAND");
    };
    let mut first_command = fixture.command();
    configure(&mut first_command);
    let first = first_command
        .env(
            "AUTOSPEC_BRIDGE_FAIL_HARNESS_ONCE",
            fixture.root.join("exhausted.harness-failed"),
        )
        .env("AUTOSPEC_FOREGROUND_RETIRE_FAILPOINT", "after-clear")
        .env("AUTOSPEC_FOREGROUND_RETIRE_FAIL_ONCE", &crash_marker)
        .output()
        .expect("run exhausted retirement failpoint");
    assert!(!first.status.success(), "after-clear failpoint must fire");
    let retiring = fixture.read_state();
    assert_eq!(retiring.phase(), ConductorPhase::Paused);
    assert_eq!(
        retiring.pause_reason(),
        Some("executor_terminal_retirement")
    );
    assert!(
        !fixture
            .state_path()
            .with_extension("claim-acquisition.json")
            .exists(),
        "receipt was retired before the injected crash"
    );

    let mut recovery = fixture.command();
    configure(&mut recovery);
    let recovered = recovery.output().expect("recover exhausted retirement");
    assert!(
        recovered.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&recovered.stdout),
        String::from_utf8_lossy(&recovered.stderr)
    );
    let recovered_state = fixture.read_state();
    assert_eq!(recovered_state.phase(), ConductorPhase::Scan);
    assert_eq!(recovered_state.selected_issue(), None);
}

#[test]
fn foreground_ownership_retirement_recovers_across_receipt_clear_crashes() {
    let _bridge_e2e = REAL_BRIDGE_E2E.lock().expect("real bridge E2E lock");
    for (failpoint, receipt_remains) in [("before-clear", true), ("after-clear", false)] {
        let fixture = ForegroundFixture::new();
        let bridge = fixture.configure_real_bridge();
        let harness_done = fixture
            .root
            .join(format!("ownership-{failpoint}.harness-done"));
        let takeover_done = fixture
            .root
            .join(format!("ownership-{failpoint}.takeover-done"));
        let crash_marker = fixture.root.join(format!("ownership-{failpoint}.failed"));
        let foreign_claim = fixture.prepare_foreign_claim_object_in(&bridge.remote);
        let configure = |command: &mut Command| {
            command
                .env("PATH", path_with(&bridge.bin))
                .env("AUTOSPEC_FOREGROUND_REAL_BRIDGE", "1")
                .env("AUTOSPEC_BRIDGE_REMOTE", &bridge.remote)
                .env("AUTOSPEC_BRIDGE_MERGED", &bridge.merged)
                .env("AUTOSPEC_CLAIM_GIT_REMOTE", &bridge.remote)
                .env("AUTOSPEC_HARNESS_RUNTIME_ALIASES", &bridge.aliases)
                .env("AUTOSPEC_HANDOFF_DISPATCHER_KIND", "codex")
                .env_remove("AUTOSPEC_EXECUTOR_REVIEW_COMMAND");
        };
        let mut first_command = fixture.command();
        configure(&mut first_command);
        let first = first_command
            .env("AUTOSPEC_BRIDGE_HARNESS_DONE", &harness_done)
            .env("AUTOSPEC_BRIDGE_TAKEOVER_OID", &foreign_claim)
            .env("AUTOSPEC_BRIDGE_TAKEOVER_DONE", &takeover_done)
            .env("AUTOSPEC_FOREGROUND_RETIRE_FAILPOINT", failpoint)
            .env("AUTOSPEC_FOREGROUND_RETIRE_FAIL_ONCE", &crash_marker)
            .output()
            .expect("run ownership-loss conductor");
        assert!(
            !first.status.success(),
            "{failpoint}: failpoint must fire; stdout={} stderr={} state={:?} calls={}",
            String::from_utf8_lossy(&first.stdout),
            String::from_utf8_lossy(&first.stderr),
            fixture.read_state(),
            fs::read_to_string(&fixture.calls).unwrap_or_default()
        );
        assert!(takeover_done.exists(), "{failpoint}: takeover must occur");
        let retiring = fixture.read_state();
        assert_eq!(retiring.phase(), ConductorPhase::Paused);
        assert_eq!(
            retiring.pause_reason(),
            Some("executor_ownership_retirement")
        );
        assert_eq!(
            fixture
                .state_path()
                .with_extension("claim-acquisition.json")
                .exists(),
            receipt_remains,
            "{failpoint}: acquisition receipt boundary"
        );

        let mut recovery = fixture.command();
        configure(&mut recovery);
        let recovered = recovery.output().expect("recover ownership retirement");
        assert!(
            recovered.status.success(),
            "{failpoint}: stdout={} stderr={}",
            String::from_utf8_lossy(&recovered.stdout),
            String::from_utf8_lossy(&recovered.stderr)
        );
        let state = fixture.read_state();
        assert_eq!(state.phase(), ConductorPhase::Scan);
        assert_eq!(state.selected_issue(), None);
        assert!(!fixture
            .state_path()
            .with_extension("claim-acquisition.json")
            .exists());
    }
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
fn foreground_bridge_failure_does_not_publish_a_duplicate_executor_outcome() {
    let fixture = ForegroundFixture::new();
    let output = fixture
        .command()
        .env("AUTOSPEC_FOREGROUND_STEAL_ON_OUTCOME", "1")
        .output()
        .expect("run foreground");

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("executor bridge failed"));
    let record = fixture.claim_record();
    assert!(record.worker_id.starts_with("rust-foreground-conductor-"));
    assert_eq!(record.branch, "feat/autonomous-issue-42");
    assert_ne!(record.step, "executor_pending");
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

#[cfg(target_os = "linux")]
#[test]
fn foreground_retires_exact_released_predecessor_heartbeat_before_acquire() {
    let fixture = ForegroundFixture::new();
    fixture.initialize_empty_local_remote();
    let branch = "feat/autonomous-issue-42";
    seed_preserved_issue_branch(&fixture, branch);
    fixture.seed_claim_state_with_id(
        "predecessor-worker",
        branch,
        "released",
        &fresh_iso_timestamp(),
        "predecessor-claim",
    );
    fixture.seed_expired_claim_heartbeat("predecessor-worker", branch, "predecessor-claim");
    seed_foreground_state(&fixture, &selected_foreground_state());
    fs::write(&fixture.mode, "reviewed\n").expect("seed reviewed issue");

    let output = fixture.run_foreground();

    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !String::from_utf8_lossy(&output.stderr).contains("heartbeat_write_failed"),
        "successor publication collided with released predecessor heartbeat"
    );
    let successor = fixture.claim_record();
    assert_eq!(successor.state, "claimed");
    assert_ne!(successor.claim_id.as_deref(), Some("predecessor-claim"));
    let heartbeat = fs::read_to_string(fixture.heartbeats.join("o4_test_r4_repo/42.json"))
        .expect("successor heartbeat");
    assert!(heartbeat.contains(successor.claim_id.as_deref().expect("successor claim ID")));
    assert!(fs::read_dir(
        fixture
            .heartbeats
            .join("o4_test_r4_repo/quarantine/startup-heartbeat-handoffs")
    )
    .expect("predecessor heartbeat handoff")
    .filter_map(Result::ok)
    .filter_map(|entry| fs::read_to_string(entry.path()).ok())
    .any(|document| document.contains("\"claim_id\":\"predecessor-claim\"")));
}

#[cfg(target_os = "linux")]
#[test]
fn foreground_rejects_foreign_released_predecessor_heartbeat_before_acquire() {
    let fixture = ForegroundFixture::new();
    fixture.initialize_empty_local_remote();
    let branch = "feat/autonomous-issue-42";
    seed_preserved_issue_branch(&fixture, branch);
    fixture.seed_claim_state_with_id(
        "predecessor-worker",
        branch,
        "released",
        &fresh_iso_timestamp(),
        "predecessor-claim",
    );
    fixture.seed_expired_claim_heartbeat("foreign-worker", branch, "foreign-claim");
    seed_foreground_state(&fixture, &selected_foreground_state());
    fs::write(&fixture.mode, "reviewed\n").expect("seed reviewed issue");
    let predecessor = fixture.claim_record();

    let output = fixture.run_foreground();

    assert!(!output.status.success());
    assert_eq!(fixture.claim_record(), predecessor);
    let heartbeat = fs::read_to_string(fixture.heartbeats.join("o4_test_r4_repo/42.json"))
        .expect("foreign heartbeat remains live");
    assert!(heartbeat.contains("\"claim_id\":\"foreign-claim\""));
}

#[cfg(target_os = "linux")]
#[test]
fn foreground_rejects_live_released_predecessor_heartbeat_before_acquire() {
    let fixture = ForegroundFixture::new();
    fixture.initialize_empty_local_remote();
    let branch = "feat/autonomous-issue-42";
    seed_preserved_issue_branch(&fixture, branch);
    fixture.seed_claim_state_with_id(
        "predecessor-worker",
        branch,
        "released",
        &fresh_iso_timestamp(),
        "predecessor-claim",
    );
    fixture.seed_claim_heartbeat("predecessor-worker", branch, "predecessor-claim");
    seed_foreground_state(&fixture, &selected_foreground_state());
    fs::write(&fixture.mode, "reviewed\n").expect("seed reviewed issue");
    let predecessor = fixture.claim_record();

    let output = fixture.run_foreground();

    assert!(!output.status.success());
    assert_eq!(fixture.claim_record(), predecessor);
    assert!(fixture.heartbeats.join("o4_test_r4_repo/42.json").exists());
}

#[cfg(target_os = "linux")]
#[test]
fn foreground_resumes_pending_released_predecessor_heartbeat_handoff_before_acquire() {
    let fixture = ForegroundFixture::new();
    fixture.initialize_empty_local_remote();
    let branch = "feat/autonomous-issue-42";
    seed_preserved_issue_branch(&fixture, branch);
    fixture.seed_claim_state_with_id(
        "predecessor-worker",
        branch,
        "released",
        &fresh_iso_timestamp(),
        "predecessor-claim",
    );
    let (pending, completed) = fixture.seed_pending_claim_heartbeat_handoff(
        "predecessor-worker",
        branch,
        "predecessor-claim",
    );
    seed_foreground_state(&fixture, &selected_foreground_state());
    fs::write(&fixture.mode, "reviewed\n").expect("seed reviewed issue");

    let output = fixture.run_foreground();

    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!pending.exists());
    assert!(completed.exists());
    let successor = fixture.claim_record();
    assert_ne!(successor.claim_id.as_deref(), Some("predecessor-claim"));
    let heartbeat = fs::read_to_string(fixture.heartbeats.join("o4_test_r4_repo/42.json"))
        .expect("successor heartbeat");
    assert!(heartbeat.contains(successor.claim_id.as_deref().expect("successor claim ID")));
}

#[cfg(target_os = "linux")]
#[test]
fn foreground_rejects_unsafe_released_predecessor_heartbeat_root_before_acquire() {
    use std::os::unix::fs::symlink;

    let fixture = ForegroundFixture::new();
    fixture.initialize_empty_local_remote();
    let branch = "feat/autonomous-issue-42";
    seed_preserved_issue_branch(&fixture, branch);
    fixture.seed_claim_state_with_id(
        "predecessor-worker",
        branch,
        "released",
        &fresh_iso_timestamp(),
        "predecessor-claim",
    );
    let heartbeat_target = fixture.root.join("foreign-heartbeats");
    fs::create_dir(&heartbeat_target).expect("create foreign heartbeat root");
    symlink(&heartbeat_target, &fixture.heartbeats).expect("symlink heartbeat root");
    seed_foreground_state(&fixture, &selected_foreground_state());
    fs::write(&fixture.mode, "reviewed\n").expect("seed reviewed issue");
    let predecessor = fixture.claim_record();

    let output = fixture.run_foreground();

    assert!(!output.status.success());
    assert_eq!(fixture.claim_record(), predecessor);
    assert_eq!(fs::read_dir(&heartbeat_target).unwrap().count(), 0);
}

#[cfg(target_os = "linux")]
#[test]
fn foreground_reclaims_stale_heartbeat_pending_before_acquire() {
    // Break caught: foreground acquisition skipped stale-startup recovery, replaced the
    // stranded claim generation, and then failed to publish over its expired heartbeat.
    let fixture = ForegroundFixture::new();
    fixture.initialize_empty_local_remote();
    let branch = "feat/autonomous-issue-42";
    seed_preserved_issue_branch(&fixture, branch);
    let branch_oid = git_fixture(&fixture.repo_dir, &["rev-parse", branch]);
    let stale = RunStateRecord::new(
        "test/repo",
        42,
        "successor-worker",
        "claimed",
        branch,
        "",
        "heartbeat-pending:none",
        Vec::new(),
        "2000-01-01T00:00:00Z",
        "2000-01-01T00:00:00Z",
        1,
    )
    .with_claim_id("successor-claim");
    fixture.transition_claim_ref(&stale);
    fixture.seed_expired_claim_heartbeat("prior-worker", branch, "prior-claim");
    seed_foreground_state(&fixture, &selected_foreground_state());
    fs::write(&fixture.mode, "reviewed\n").expect("seed reviewed issue");

    let output = fixture.run_foreground();

    assert!(
        output.status.success(),
        "stdout={} stderr={} calls={} claim={:?} heartbeat={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
        fs::read_to_string(&fixture.calls).unwrap_or_default(),
        fixture.claim_record(),
        fs::read_to_string(fixture.heartbeats.join("o4_test_r4_repo/42.json")).unwrap_or_default(),
    );
    assert!(
        !String::from_utf8_lossy(&output.stderr).contains("heartbeat_write_failed"),
        "foreground attempted acquisition before stale-startup recovery"
    );
    let acquired = fixture.claim_record();
    assert!(acquired.worker_id.starts_with("rust-foreground-conductor-"));
    assert_ne!(acquired.claim_id.as_deref(), Some("successor-claim"));
    assert_eq!(
        git_fixture(&fixture.repo_dir, &["rev-parse", branch]),
        branch_oid
    );
    let heartbeat = fs::read_to_string(fixture.heartbeats.join("o4_test_r4_repo/42.json"))
        .expect("fresh foreground heartbeat");
    assert!(heartbeat.contains(&format!("\"worker_id\":{:?}", acquired.worker_id)));
    assert!(heartbeat.contains(&format!(
        "\"claim_id\":{:?}",
        acquired.claim_id.expect("fresh claim ID")
    )));
    assert!(fs::read_dir(
        fixture
            .heartbeats
            .join("o4_test_r4_repo/quarantine/startup-heartbeat-handoffs")
    )
    .expect("prior heartbeat handoff")
    .filter_map(Result::ok)
    .filter_map(|entry| fs::read_to_string(entry.path()).ok())
    .any(|document| document.contains("\"claim_id\":\"prior-claim\"")));

    for (case, updated_at, heartbeat_worker, heartbeat_branch, heartbeat_claim, live) in [
        (
            "fresh-claim",
            fresh_iso_timestamp(),
            "prior-worker",
            branch,
            "prior-claim",
            false,
        ),
        (
            "current-generation",
            "2000-01-01T00:00:00Z".to_string(),
            "blocked-worker",
            branch,
            "blocked-claim",
            false,
        ),
        (
            "live-prior",
            "2000-01-01T00:00:00Z".to_string(),
            "prior-worker",
            branch,
            "prior-claim",
            true,
        ),
        (
            "wrong-branch",
            "2000-01-01T00:00:00Z".to_string(),
            "prior-worker",
            "feat/foreign",
            "prior-claim",
            false,
        ),
    ] {
        let fixture = ForegroundFixture::new();
        fixture.initialize_empty_local_remote();
        seed_preserved_issue_branch(&fixture, branch);
        let blocked_branch_oid = git_fixture(&fixture.repo_dir, &["rev-parse", branch]);
        let blocked = RunStateRecord::new(
            "test/repo",
            42,
            "blocked-worker",
            "claimed",
            branch,
            "",
            "heartbeat-pending:none",
            Vec::new(),
            &updated_at,
            &updated_at,
            1,
        )
        .with_claim_id("blocked-claim");
        fixture.transition_claim_ref(&blocked);
        if live {
            fixture.seed_claim_heartbeat(heartbeat_worker, heartbeat_branch, heartbeat_claim);
        } else {
            fixture.seed_expired_claim_heartbeat(
                heartbeat_worker,
                heartbeat_branch,
                heartbeat_claim,
            );
        }
        seed_foreground_state(&fixture, &selected_foreground_state());
        fs::write(&fixture.mode, "reviewed\n").expect("seed reviewed issue");
        let heartbeat_path = fixture.heartbeats.join("o4_test_r4_repo/42.json");
        let heartbeat_before = fs::read_to_string(&heartbeat_path).expect("blocked heartbeat");
        let claim_before = fixture.claim_record();

        let output = fixture.run_foreground();

        assert!(
            !output.status.success(),
            "{case}: acquisition unexpectedly won"
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("\"reason\":\"claim_lost\""),
            "{case}: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            fixture.claim_record(),
            claim_before,
            "{case}: claim mutated"
        );
        assert_eq!(
            fs::read_to_string(&heartbeat_path).expect("preserved blocked heartbeat"),
            heartbeat_before,
            "{case}: heartbeat mutated"
        );
        assert_eq!(
            git_fixture(&fixture.repo_dir, &["rev-parse", branch]),
            blocked_branch_oid,
            "{case}: branch mutated"
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn foreground_recovers_with_integrated_inactive_local_branch() {
    for (case, unmerged, topic_head, should_recover) in [
        ("integrated", false, false, true),
        ("unmerged", true, false, false),
        ("topic-contains-branch", true, true, false),
    ] {
        let fixture = ForegroundFixture::new();
        fixture.initialize_empty_local_remote();
        git_fixture(
            &fixture.repo_dir,
            &["config", "user.name", "Autospec Branch Test"],
        );
        git_fixture(
            &fixture.repo_dir,
            &["config", "user.email", "autospec-branch-test@localhost"],
        );
        fs::write(fixture.repo_dir.join("README.md"), "baseline\n").expect("write baseline");
        git_fixture(&fixture.repo_dir, &["add", "README.md"]);
        git_fixture(&fixture.repo_dir, &["commit", "-m", "baseline"]);
        git_fixture(&fixture.repo_dir, &["push", "-u", "origin", "main"]);
        let branch = "feat/autonomous-issue-42";
        git_fixture(&fixture.repo_dir, &["branch", branch]);
        if unmerged {
            git_fixture(&fixture.repo_dir, &["checkout", branch]);
            fs::write(fixture.repo_dir.join("work.txt"), "unmerged\n")
                .expect("write unmerged work");
            git_fixture(&fixture.repo_dir, &["add", "work.txt"]);
            git_fixture(&fixture.repo_dir, &["commit", "-m", "unmerged work"]);
            if topic_head {
                git_fixture(&fixture.repo_dir, &["checkout", "-b", "topic"]);
            } else {
                git_fixture(&fixture.repo_dir, &["checkout", "main"]);
            }
        }
        let stale = RunStateRecord::new(
            "test/repo",
            42,
            "stale-worker",
            "claimed",
            branch,
            "",
            "heartbeat-pending:none",
            Vec::new(),
            "2000-01-01T00:00:00Z",
            "2000-01-01T00:00:00Z",
            1,
        )
        .with_claim_id("stale-claim");
        fixture.transition_claim_ref(&stale);
        fixture.seed_expired_claim_heartbeat("stale-worker", branch, "stale-claim");
        seed_foreground_state(&fixture, &selected_foreground_state());
        fs::write(&fixture.mode, "reviewed\n").expect("seed reviewed issue");
        let heartbeat_path = fixture.heartbeats.join("o4_test_r4_repo/42.json");
        let heartbeat_before = fs::read_to_string(&heartbeat_path).expect("stale heartbeat");

        let output = fixture.run_foreground();

        assert_eq!(
            output.status.success(),
            should_recover,
            "{case}: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        if should_recover {
            assert!(fixture
                .claim_record()
                .worker_id
                .starts_with("rust-foreground-conductor-"));
        } else {
            assert_eq!(fixture.claim_record(), stale);
            assert_eq!(
                fs::read_to_string(&heartbeat_path).expect("preserved heartbeat"),
                heartbeat_before
            );
        }
    }
}

#[test]
fn foreground_ignores_a_malformed_audit_projection_without_a_claim_ref() {
    let fixture = ForegroundFixture::new();
    fixture.seed_malformed_claim();

    let output = fixture.run_foreground();

    assert!(output.status.success());
    let record = fixture.claim_record();
    assert!(record.worker_id.starts_with("rust-foreground-conductor-"));
    assert_eq!(record.state, "claimed");
    let calls = fs::read_to_string(&fixture.calls).expect("read GitHub calls");
    assert!(
        calls.contains("issue\nedit\n42"),
        "an audit-only malformed comment must not block a new authoritative claim"
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
fn detached_start_survives_launcher_process_group_shutdown() {
    let fixture = ForegroundFixture::new();
    fixture.initialize_git_remote();
    let mut command = fixture.detached_command("start");
    command
        .env("AUTOSPEC_AUTONOMOUS_COMPANIONS", "0")
        .env("AUTOSPEC_FOREGROUND_BLOCK_GH", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.process_group(0);
    let launcher = command.spawn().expect("spawn detached launcher");
    let launcher_process_group = launcher.id();
    let output = launcher
        .wait_with_output()
        .expect("wait for detached launcher");

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let conductor_pid = fixture
        .recorded_conductor_pid()
        .expect("recorded conductor pid");
    assert!(process_is_running(conductor_pid));

    terminate_process_group(launcher_process_group);
    std::thread::sleep(std::time::Duration::from_millis(100));

    assert!(
        process_is_running(conductor_pid),
        "detached conductor {conductor_pid} exited with launcher process group {launcher_process_group}"
    );
}

#[test]
fn detached_start_runs_multiple_native_cycles_then_exits_at_max_cycles() {
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
            "--max-cycles",
            "2",
            "--poll-interval-sec",
            "1",
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_COMPANIONS", "0")
        .env("AUTOSPEC_FOREGROUND_EMPTY_QUEUE", "1")
        .output()
        .expect("start bounded detached foreground");

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let conductor_pid = fixture
        .recorded_conductor_pid()
        .expect("recorded conductor pid");
    for _ in 0..400 {
        if !process_is_running(conductor_pid) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    assert!(
        !process_is_running(conductor_pid),
        "bounded conductor {conductor_pid} did not exit after two cycles"
    );
    assert_eq!(
        fixture.read_state().no_progress_cycles(),
        2,
        "a detached start must keep the Rust conductor alive across completed cycles"
    );
    assert!(
        fs::read_to_string(fixture.resilience_state_path())
            .expect("read released lifecycle state")
            .contains("\"status\":\"released\""),
        "a max-cycle exit must release lifecycle ownership"
    );
}

#[test]
fn detached_start_observes_stop_before_the_next_native_cycle() {
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
            "--max-cycles",
            "3",
            "--poll-interval-sec",
            "5",
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_COMPANIONS", "0")
        .env("AUTOSPEC_FOREGROUND_EMPTY_QUEUE", "1")
        .output()
        .expect("start stoppable detached foreground");

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let conductor_pid = fixture
        .recorded_conductor_pid()
        .expect("recorded conductor pid");
    wait_for_file_contents(&fixture.state_path(), "\"no_progress_cycles\":1");
    fs::write(
        fixture.scoped_stop_sentinel(),
        "graceful\n2026-07-25T00:00:00Z test@localhost\n",
    )
    .expect("write scoped stop sentinel");

    for _ in 0..250 {
        if !process_is_running(conductor_pid) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    assert!(
        !process_is_running(conductor_pid),
        "conductor {conductor_pid} did not stop at the next cycle boundary"
    );
    assert_eq!(
        fixture.read_state().no_progress_cycles(),
        1,
        "the stop boundary must prevent a second cycle"
    );
    assert!(
        fs::read_to_string(fixture.resilience_state_path())
            .expect("read released lifecycle state")
            .contains("\"status\":\"released\""),
        "a stop exit must release lifecycle ownership"
    );
}

#[test]
fn session_follow_attaches_without_restarting_and_detaches_safely() {
    let fixture = ForegroundFixture::new();
    fixture.initialize_git_remote();
    fixture.start_blocked_detached();
    let conductor_pid = fixture.recorded_conductor_pid().expect("conductor pid");

    let mut follower = fixture.spawn_following_start(0);
    wait_for_file_contents(
        &fixture.root.join("follower.log"),
        "autospec autonomous attached",
    );
    assert!(process_is_running(follower.id()));
    assert_eq!(fixture.recorded_conductor_pid(), Some(conductor_pid));
    assert!(!fixture.scoped_stop_sentinel().exists());

    follower.terminate_and_wait();
    assert!(process_is_running(conductor_pid));
    assert_eq!(fixture.recorded_conductor_pid(), Some(conductor_pid));
    assert!(!fixture.scoped_stop_sentinel().exists());
    fixture.terminate_recorded_conductor();
}

#[test]
fn session_follow_fresh_start_uses_a_generation_log_and_enters_follow_mode() {
    let fixture = ForegroundFixture::new();
    fixture.initialize_git_remote();

    let mut follower = fixture.spawn_following_start(1);
    wait_for_file_contents(
        &fixture.root.join("follower.log"),
        "autospec autonomous started",
    );
    let status = follower.wait();

    assert!(status.success());
    assert!(process_is_running(
        fixture.recorded_conductor_pid().expect("conductor pid")
    ));
    let logpath = fixture.recorded_conductor_logpath();
    assert_ne!(
        logpath.file_name().and_then(|name| name.to_str()),
        Some("autospec-autonomous-conductor.log")
    );
    assert!(logpath.exists());
}

#[test]
fn session_follow_uses_a_one_second_cadence_without_a_poll_interval_override() {
    let fixture = ForegroundFixture::new();
    fixture.initialize_git_remote();
    fixture.start_blocked_detached();
    let logpath = fixture.recorded_conductor_logpath();

    let mut follower = fixture.spawn_following_start_without_interval(2);
    wait_for_file_contents(
        &fixture.root.join("follower.log"),
        "autospec autonomous attached",
    );
    fs::write(&logpath, "default-cadence-update\n").expect("write conductor update");
    wait_for_file_contents(&fixture.root.join("follower.log"), "default-cadence-update");

    assert!(follower.wait().success());
}

#[test]
fn session_follow_switches_to_repaired_conductor_log_from_offset_zero() {
    let fixture = ForegroundFixture::new();
    fixture.initialize_git_remote();
    fixture.start_blocked_detached();
    let initial_log = fixture.recorded_conductor_logpath();
    fs::write(&initial_log, "initial-generation\n").expect("write initial conductor log");

    let mut follower = fixture.spawn_following_start(0);
    wait_for_file_contents(&fixture.root.join("follower.log"), "initial-generation");

    let repaired_log = fixture.root.join("repaired-conductor.log");
    fs::write(&repaired_log, "repair-generation-from-zero\n").expect("write repaired log");
    fs::write(
        fixture.scoped_dir().join("conductor.logpath"),
        format!("{}\n", repaired_log.display()),
    )
    .expect("switch scoped log metadata");

    wait_for_file_contents(
        &fixture.root.join("follower.log"),
        "repair-generation-from-zero",
    );
    follower.terminate_and_wait();
    assert!(process_is_running(
        fixture.recorded_conductor_pid().expect("conductor pid")
    ));
}

#[test]
fn session_follow_reports_one_wait_per_supervisor_repair_outage() {
    let fixture = ForegroundFixture::new();
    fixture.initialize_git_remote();
    fixture.start_blocked_detached();
    let mut follower = fixture.spawn_following_start(5);
    wait_for_file_contents(
        &fixture.root.join("follower.log"),
        "autospec autonomous attached",
    );

    let mut supervisor = Command::new("sleep");
    supervisor.arg("300").process_group(0);
    let supervisor = supervisor.spawn().expect("spawn live supervisor");
    let supervisor_pid = supervisor.id();
    let mut supervisor = GuardedProcessGroup::new(supervisor);
    let (_, start_time_ticks) =
        process_identity(supervisor_pid).expect("supervisor process identity");
    fs::write(
        fixture.scoped_dir().join("supervisor.pid"),
        format!(
            "{{\"pid\":{supervisor_pid},\"repo\":\"test/repo\",\"scope\":\"test_repo\",\"pgid\":{supervisor_pid},\"start_time_ticks\":{start_time_ticks}}}\n"
        ),
    )
    .expect("record live supervisor");
    fixture.terminate_recorded_conductor();

    assert!(follower.wait().success());
    supervisor.terminate_and_wait();
    let output = fs::read_to_string(fixture.root.join("follower.log")).expect("read follower log");
    assert_eq!(
        output
            .matches("autospec autonomous follow: waiting for supervisor repair")
            .count(),
        1,
        "{output}"
    );
}

#[test]
fn session_follow_resets_explicit_log_cursor_after_live_pid_replacement() {
    let fixture = ForegroundFixture::new();
    fixture.initialize_git_remote();
    let explicit_log = fixture.root.join("explicit-conductor.log");
    fixture.start_blocked_detached_with_log(Some(&explicit_log));
    fs::write(&explicit_log, "old-generation-complete\n").expect("write initial explicit log");

    let mut follower = fixture.spawn_following_start(0);
    wait_for_file_contents(
        &fixture.root.join("follower.log"),
        "old-generation-complete",
    );

    fixture.terminate_recorded_conductor();
    let mut replacement_command = Command::new("sh");
    replacement_command
        .args(["-c", "while :; do sleep 1; done"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
    let replacement = GuardedProcessGroup::new(
        replacement_command
            .spawn()
            .expect("spawn replacement conductor"),
    );
    fs::write(
        fixture.scoped_dir().join("conductor.pid"),
        format!(
            "{{\"pid\":{},\"repo\":\"test/repo\",\"scope\":\"test_repo\"}}\n",
            replacement.id()
        ),
    )
    .expect("record replacement conductor pid");
    fs::write(
        &explicit_log,
        "new-generation-prefix-must-be-visible-even-when-this-generation-is-longer-than-the-old-one\n",
    )
    .expect("rewrite explicit log for replacement generation");
    wait_for_file_contents(&fixture.root.join("follower.log"), "new-generation-prefix");

    follower.terminate_and_wait();
    assert_eq!(fixture.recorded_conductor_logpath(), explicit_log);
    assert!(!fixture.scoped_stop_sentinel().exists());
    drop(replacement);
}

#[test]
fn session_follow_reads_log_growth_with_a_seek_cursor() {
    let source =
        fs::read_to_string(workspace_root().join("crates/autospec-cli/src/commands/autonomous.rs"))
            .expect("read autonomous command source");
    let growth = source
        .split_once("fn print_log_growth")
        .and_then(|(_, remainder)| remainder.split_once("\nfn "))
        .map(|(body, _)| body)
        .expect("print_log_growth function");

    assert!(growth.contains("SeekFrom::Start"));
    assert!(growth.contains("read_to_end"));
    assert!(!growth.contains("fs::read(logpath)"));
}

#[test]
fn session_follow_dry_run_reports_scoped_log_without_creating_state() {
    let fixture = ForegroundFixture::new();
    let output = fixture
        .detached_command("start")
        .args(["--follow", "--dry-run"])
        .output()
        .expect("dry-run following start");

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("follow: scoped conductor log"));
    assert!(!fixture.operator.exists());
}

#[test]
fn session_follow_rejects_json_before_creating_state() {
    let fixture = ForegroundFixture::new();
    let output = fixture
        .detached_command("start")
        .args(["--follow", "--json"])
        .output()
        .expect("reject mixed JSON and log stream");

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("--json is not supported with --follow; use autospec autonomous status --json"));
    assert!(!fixture.operator.exists());
}

#[test]
fn session_follow_waits_for_metadata_published_after_a_held_lease() {
    let fixture = ForegroundFixture::new();
    fixture.initialize_git_remote();
    fixture.start_blocked_detached();
    let original_pid = fixture.recorded_conductor_pid().expect("conductor pid");
    let pid_path = fixture.scoped_dir().join("conductor.pid");
    let logpath_path = fixture.scoped_dir().join("conductor.logpath");
    let pid_metadata = fs::read_to_string(&pid_path).expect("read conductor metadata");
    let logpath_metadata = fs::read_to_string(&logpath_path).expect("read log metadata");
    fs::remove_file(&pid_path).expect("hide conductor pid before follower start");
    fs::remove_file(&logpath_path).expect("hide conductor log before follower start");

    let mut follower = fixture.spawn_following_start(1);
    wait_for_file_contents(
        &fixture.root.join("follower.log"),
        "waiting for scoped conductor metadata after held lifecycle lease",
    );
    fs::write(&pid_path, pid_metadata).expect("publish conductor pid metadata");
    fs::write(&logpath_path, logpath_metadata).expect("publish conductor log metadata");

    assert!(follower.wait().success());
    assert_eq!(fixture.recorded_conductor_pid(), Some(original_pid));
    assert!(process_is_running(original_pid));
}

#[test]
fn session_follow_rejects_ambiguous_conductor_metadata_before_starting() {
    let fixture = ForegroundFixture::new();
    fixture.initialize_git_remote();
    let scope = fixture.operator.join("test_repo");
    fs::create_dir_all(&scope).expect("create scoped metadata");
    fs::write(scope.join("conductor.logpath"), "/tmp/ambiguous.log\n")
        .expect("write ambiguous log metadata");

    let output = fixture
        .detached_command("start")
        .args(["--follow", "--iterations", "1", "--branch", "main"])
        .env("AUTOSPEC_AUTONOMOUS_COMPANIONS", "0")
        .output()
        .expect("follow ambiguous conductor");

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("cannot follow ambiguous conductor metadata for test/repo"));
    assert!(!scope.join("conductor.pid").exists());
}

#[test]
fn detached_flag_returns_after_start_without_following() {
    let fixture = ForegroundFixture::new();
    fixture.initialize_git_remote();
    let output = fixture
        .detached_command("start")
        .arg("--detach")
        .env("AUTOSPEC_AUTONOMOUS_COMPANIONS", "0")
        .output()
        .expect("explicit detached start");
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("autospec autonomous started"));
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
fn immediate_stop_terminates_recorded_wrapper_descendants() {
    let fixture = ForegroundFixture::new();
    let child_pid_path = fixture.root.join("wrapper-child.pid");
    let mut wrapper = Command::new("sh");
    wrapper
        .arg("-c")
        .arg("sleep 300 & child=$!; printf '%s\n' \"$child\" > \"$CHILD_PID_FILE\"; wait")
        .env("CHILD_PID_FILE", &child_pid_path)
        .process_group(0);
    let mut wrapper = GuardedProcessGroup::new(wrapper.spawn().expect("spawn wrapper process"));
    wait_for_file_contents(&child_pid_path, "");
    let child_pid = fs::read_to_string(&child_pid_path)
        .expect("read wrapper child pid")
        .trim()
        .parse::<u32>()
        .expect("parse wrapper child pid");
    let scope = fixture.scoped_dir();
    fs::create_dir_all(&scope).expect("create scoped metadata");
    fs::write(
        scope.join("supervisor.pid"),
        format!(
            "{{\"pid\":{},\"repo\":\"test/repo\",\"scope\":\"test_repo\",\"pgid\":{},\"start_time_ticks\":{}}}\n",
            wrapper.id(),
            wrapper.id(),
            process_identity(wrapper.id()).expect("wrapper identity").1
        ),
    )
    .expect("record wrapper metadata");

    let output = fixture
        .detached_command("stop")
        .args(["--immediate", "--json"])
        .output()
        .expect("stop wrapper process group");
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    for _ in 0..100 {
        if !process_is_running(child_pid) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let child_survived = process_is_running(child_pid);
    wrapper.terminate_and_wait();

    assert!(
        !child_survived,
        "wrapper descendant {child_pid} survived immediate stop"
    );
}

#[test]
fn immediate_stop_terminates_descendants_after_the_recorded_leader_exits() {
    let fixture = ForegroundFixture::new();
    let child_pid_path = fixture.root.join("orphan-child.pid");
    let mut leader = Command::new("bash");
    leader
        .arg("-c")
        .arg(
            "trap '' HUP; bash -c 'trap \"\" HUP; exec -a autospec-autonomous-supervisor sleep 300' & child=$!; printf '%s\n' \"$child\" > \"$CHILD_PID_FILE\"",
        )
        .env("CHILD_PID_FILE", &child_pid_path)
        .process_group(0);
    let mut leader = leader.spawn().expect("spawn short-lived group leader");
    let leader_pid = leader.id();
    let (_, start_time_ticks) = process_identity(leader_pid).expect("leader identity");
    wait_for_file_contents(&child_pid_path, "");
    let child_pid = fs::read_to_string(&child_pid_path)
        .expect("read orphan child pid")
        .trim()
        .parse::<u32>()
        .expect("parse orphan child pid");
    assert!(leader.wait().expect("reap group leader").success());
    assert!(!process_is_running(leader_pid));
    assert!(process_is_running(child_pid));
    let scope = fixture.scoped_dir();
    fs::create_dir_all(&scope).expect("create scoped metadata");
    fs::write(
        scope.join("supervisor.pid"),
        format!(
            "{{\"pid\":{leader_pid},\"repo\":\"test/repo\",\"scope\":\"test_repo\",\"pgid\":{leader_pid},\"start_time_ticks\":{start_time_ticks}}}\n"
        ),
    )
    .expect("record exited leader metadata");

    let output = fixture
        .detached_command("stop")
        .args(["--immediate", "--json"])
        .output()
        .expect("stop orphaned process group");
    for _ in 0..100 {
        if !process_is_running(child_pid) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let child_survived = process_is_running(child_pid);
    terminate_process_group(leader_pid);
    terminate_process(child_pid);

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !child_survived,
        "orphaned descendant {child_pid} survived immediate stop"
    );
}

#[test]
fn immediate_stop_rejects_mismatched_process_start_identity() {
    let fixture = ForegroundFixture::new();
    let mut process = Command::new("sleep");
    process.arg("300").process_group(0);
    let process = process.spawn().expect("spawn unrelated process group");
    let process_pid = process.id();
    let mut process = GuardedProcessGroup::new(process);
    let (_, start_time_ticks) = process_identity(process_pid).expect("process identity");
    let scope = fixture.scoped_dir();
    fs::create_dir_all(&scope).expect("create scoped metadata");
    fs::write(
        scope.join("supervisor.pid"),
        format!(
            "{{\"pid\":{process_pid},\"repo\":\"test/repo\",\"scope\":\"test_repo\",\"pgid\":{process_pid},\"start_time_ticks\":{}}}\n",
            start_time_ticks + 1
        ),
    )
    .expect("record mismatched process metadata");

    let output = fixture
        .detached_command("stop")
        .args(["--immediate", "--json"])
        .output()
        .expect("reject mismatched process identity");
    let process_survived = process_is_running(process_pid);
    process.terminate_and_wait();

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("process identity mismatch"));
    assert!(process_survived, "mismatched process group was terminated");
}

#[test]
fn forced_restart_replaces_a_live_conductor_and_its_lease() {
    let fixture = ForegroundFixture::new();
    fixture.initialize_git_remote();
    let output = fixture
        .detached_command("start")
        .args(["--detach", "--branch", "main"])
        .env("AUTOSPEC_AUTONOMOUS_COMPANIONS", "0")
        .env("AUTOSPEC_FOREGROUND_BLOCK_GH", "1")
        .env("AUTOSPEC_HOST", "unknown")
        .output()
        .expect("start live conductor with an unknown host");
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    wait_for_file_contents(&fixture.calls, "repos/test/repo/branches/main");
    let original_pid = fixture
        .recorded_conductor_pid()
        .expect("original conductor");

    let output = fixture
        .detached_command("restart")
        .args(["--force", "--branch", "main", "--json"])
        .env("AUTOSPEC_AUTONOMOUS_COMPANIONS", "0")
        .env("AUTOSPEC_FOREGROUND_BLOCK_GH", "1")
        .env("AUTOSPEC_HOST", "unknown")
        .output()
        .expect("restart live conductor");

    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let replacement_pid = fixture
        .recorded_conductor_pid()
        .expect("replacement conductor");
    assert_ne!(replacement_pid, original_pid);
    assert!(!process_is_running(original_pid));
    assert!(process_is_running(replacement_pid));
}

#[test]
fn forced_restart_releases_an_unknown_host_lease_after_the_conductor_is_gone() {
    let fixture = ForegroundFixture::new();
    fixture.initialize_git_remote();
    let output = fixture
        .detached_command("start")
        .args(["--detach", "--branch", "main"])
        .env("AUTOSPEC_AUTONOMOUS_COMPANIONS", "0")
        .env("AUTOSPEC_FOREGROUND_BLOCK_GH", "1")
        .env("AUTOSPEC_HOST", "unknown")
        .output()
        .expect("start conductor with an unknown host");
    assert!(output.status.success());
    wait_for_file_contents(&fixture.calls, "repos/test/repo/branches/main");
    let original_pid = fixture
        .recorded_conductor_pid()
        .expect("original conductor");
    fixture.terminate_recorded_conductor();
    for _ in 0..100 {
        if !process_is_running(original_pid) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(!process_is_running(original_pid));

    let output = fixture
        .detached_command("restart")
        .args(["--force", "--branch", "main", "--json"])
        .env("AUTOSPEC_AUTONOMOUS_COMPANIONS", "0")
        .env("AUTOSPEC_FOREGROUND_BLOCK_GH", "1")
        .env("AUTOSPEC_HOST", "unknown")
        .output()
        .expect("restart after conductor loss");

    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let replacement_pid = fixture
        .recorded_conductor_pid()
        .expect("replacement conductor");
    assert_ne!(replacement_pid, original_pid);
    assert!(process_is_running(replacement_pid));
}

#[test]
fn autonomous_start_records_kernel_hostname_without_host_env() {
    let fixture = ForegroundFixture::new();
    fixture.initialize_git_remote();
    let expected_host = Command::new("hostname")
        .arg("-s")
        .output()
        .expect("read kernel hostname");
    assert!(expected_host.status.success());
    let expected_host = String::from_utf8(expected_host.stdout)
        .expect("hostname is UTF-8")
        .trim()
        .to_string();
    assert!(!expected_host.is_empty());

    let output = fixture
        .detached_command("start")
        .args(["--detach", "--branch", "main"])
        .env_remove("AUTOSPEC_HOST")
        .env_remove("HOSTNAME")
        .env("AUTOSPEC_AUTONOMOUS_COMPANIONS", "0")
        .env("AUTOSPEC_FOREGROUND_BLOCK_GH", "1")
        .output()
        .expect("start conductor without host environment");
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    wait_for_file_contents(&fixture.calls, "repos/test/repo/branches/main");
    let state = fs::read_to_string(fixture.resilience_state_path())
        .expect("read persisted lifecycle lease");
    fixture.terminate_recorded_conductor();

    let state: serde_json::Value = serde_json::from_str(&state).expect("parse lifecycle lease");
    assert_eq!(
        state.get("host").and_then(serde_json::Value::as_str),
        Some(expected_host.as_str())
    );
    assert_eq!(
        state.get("lock_host").and_then(serde_json::Value::as_str),
        Some(expected_host.as_str())
    );
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
    let calls = fs::read_to_string(&fixture.calls).expect("read GitHub calls");
    assert!(calls.contains("issue\nedit\n42"));
    assert!(!calls.contains("executor_pending"));
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
fn foreground_fixture_git_remote_has_a_real_main() {
    let fixture = ForegroundFixture::new();
    fixture.initialize_git_remote();

    let local = git_fixture(&fixture.repo_dir, &["rev-parse", "refs/heads/main"]);
    let remote = git_fixture(
        &fixture.repo_dir,
        &["ls-remote", "--heads", "origin", "main"],
    );

    assert!(remote.starts_with(&local), "local={local} remote={remote}");
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
    const UNRESOLVED_POLICY_DIGEST: &str = "autospec-main-health-policy-v1:66e6f0c0605153f689ec9b01bbbd3ada254ed0031573a196fed67c7aab401671";
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
    const UNRESOLVED_POLICY_DIGEST: &str = "autospec-main-health-policy-v1:66e6f0c0605153f689ec9b01bbbd3ada254ed0031573a196fed67c7aab401671";
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
            "--claim-id",
            EXECUTOR_CLAIM_ID,
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
    let mut expected = before;
    expected.step = "executor_blocked".to_string();
    expected.updated_at = record.updated_at.clone();
    assert_eq!(record, expected);
    assert!(fs::read_to_string(&fixture.comments)
        .expect("read executor evidence")
        .contains("<!-- autospec-executor-result:begin -->"));
}

#[test]
fn executor_result_blocked_and_retryable_require_expected_claim_id() {
    for (outcome, reason, old_exit) in [
        ("blocked", "waiting-for-review", 20),
        ("retryable", "transient-network-error", 10),
    ] {
        let fixture = ForegroundFixture::new();
        fixture.seed_claim("rust-foreground-conductor-1", "autonomous/issue-42");
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
                outcome,
                "--reason",
                reason,
            ])
            .output()
            .expect("record executor result without claim generation");

        assert_eq!(
            output.status.code(),
            Some(2),
            "{outcome} unexpectedly retained legacy exit {old_exit}: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("\"status\":\"malformed\""));
        assert!(String::from_utf8_lossy(&output.stdout).contains("--claim-id"));
    }
}

#[test]
fn stale_claim_generation_cannot_record_blocked_or_retryable_executor_result() {
    for (outcome, reason) in [
        ("blocked", "waiting-for-review"),
        ("retryable", "transient-network-error"),
    ] {
        let fixture = ForegroundFixture::new();
        fixture.seed_claim_with_id(
            "rust-foreground-conductor-1",
            "autonomous/issue-42",
            "claim-generation-b",
        );
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
                "--claim-id",
                "claim-generation-a",
                "--outcome",
                outcome,
                "--reason",
                reason,
            ])
            .output()
            .expect("record stale executor result");

        assert_eq!(output.status.code(), Some(3), "{outcome}");
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("\"status\":\"ownership_lost\""),
            "{outcome}: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert_eq!(fixture.claim_record(), before, "{outcome}");
    }
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
                "--claim-id",
                EXECUTOR_CLAIM_ID,
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
        r#"[{"number":17,"body":"Closes #42","headRefName":"autonomous/issue-42","headRefOid":"0123456789abcdef0123456789abcdef01234567","isDraft":false,"baseRefName":"main"}]"#,
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
        r#"[{"number":17,"body":"Closes #42\n\n## Closeout report\n\nResult: shipped","headRefName":"autonomous/issue-42","headRefOid":"0123456789abcdef0123456789abcdef01234567","isDraft":false,"baseRefName":"main"}]"#,
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
    let mut expected = before;
    expected.step = "executor_succeeded".to_string();
    expected.updated_at = record.updated_at.clone();
    assert_eq!(record, expected);
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
    let claim_before = fixture.claim_record();
    let comments_before = fs::read_to_string(&fixture.comments).expect("comments before child");
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
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"status\":\"accepted\""));
    assert_eq!(fixture.claim_record(), claim_before);
    assert_eq!(
        fs::read_to_string(&fixture.comments).expect("comments after child"),
        comments_before,
        "a compatibility artifact is nonterminal and cannot mutate claim authority"
    );
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
            "--claim-id",
            EXECUTOR_CLAIM_ID,
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
    let record = fixture.claim_record();
    let mut expected = before;
    expected.step = "executor_retryable".to_string();
    expected.updated_at = record.updated_at.clone();
    assert_eq!(record, expected);
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
        r#"[{"number":17,"body":"Closes #42\n\n## Closeout report\n\nResult: shipped","headRefName":"autonomous/issue-42","headRefOid":"0123456789abcdef0123456789abcdef01234567","isDraft":false,"baseRefName":"main"}]"#,
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
fn executor_result_ignores_a_terminal_audit_comment_without_a_terminal_claim_ref() {
    let fixture = ForegroundFixture::new();
    fixture.seed_claim("rust-foreground-conductor-1", "autonomous/issue-42");
    fixture.append_terminal_merged_marker();
    let before = fs::read_to_string(&fixture.comments).expect("read terminal claim");
    fixture.set_open_pull_requests(
        r#"[{"number":17,"body":"Closes #42\n\n## Closeout report\n\nResult: shipped","headRefName":"autonomous/issue-42","headRefOid":"0123456789abcdef0123456789abcdef01234567","isDraft":false,"baseRefName":"main"}]"#,
    );

    let output = fixture
        .explicit_success_command()
        .output()
        .expect("submit terminal result");

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).starts_with("{\"status\":\"accepted\""));
    assert_eq!(fixture.claim_record().state, "claimed");
    assert_ne!(
        fs::read_to_string(&fixture.comments).expect("read terminal audit after result"),
        before,
        "accepted result evidence is appended to the audit projection"
    );
}

#[test]
fn executor_result_rejects_a_valid_closeout_pr_from_a_foreign_branch() {
    let fixture = ForegroundFixture::new();
    fixture.seed_claim("rust-foreground-conductor-1", "autonomous/issue-42");
    let before = fs::read_to_string(&fixture.comments).expect("read claimed run state");
    fixture.set_open_pull_requests(
        r#"[{"number":17,"body":"Closes #42\n\n## Closeout report\n\nResult: shipped","headRefName":"foreign/issue-42","headRefOid":"0123456789abcdef0123456789abcdef01234567","isDraft":false,"baseRefName":"main"}]"#,
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
        r#"[{"number":17,"body":"Closes #42\n\n## Closeout report\n\nResult: shipped","headRefName":"autonomous/issue-42","headRefOid":"0123456789abcdef0123456789abcdef01234567","isDraft":false,"baseRefName":"main"}]"#,
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
        r#"[{"number":17,"body":"Closes #42\n\n## Closeout report\n\nResult: shipped","headRefName":"autonomous/issue-42","headRefOid":"0123456789abcdef0123456789abcdef01234567","isDraft":false,"baseRefName":"main"}]"#,
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
    assert_eq!(fixture.claim_record().step, "executor_succeeded");
    assert!(fs::read_to_string(&fixture.comments)
        .expect("read persisted but unconfirmed evidence")
        .contains("<!-- autospec-executor-result:begin -->"));
}

#[test]
fn executor_result_reports_a_pre_write_failure_as_blocked() {
    let fixture = ForegroundFixture::new();
    fixture.seed_claim("rust-foreground-conductor-1", "autonomous/issue-42");
    fixture.set_open_pull_requests(
        r#"[{"number":17,"body":"Closes #42\n\n## Closeout report\n\nResult: shipped","headRefName":"autonomous/issue-42","headRefOid":"0123456789abcdef0123456789abcdef01234567","isDraft":false,"baseRefName":"main"}]"#,
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

struct GuardedProcessGroup {
    child: Option<Child>,
}

impl GuardedProcessGroup {
    fn new(child: Child) -> Self {
        Self { child: Some(child) }
    }

    fn id(&self) -> u32 {
        self.child.as_ref().expect("live guarded child").id()
    }

    fn wait(&mut self) -> ExitStatus {
        let mut child = self.child.take().expect("live guarded child");
        child.wait().expect("wait for guarded child")
    }

    fn terminate_and_wait(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        terminate_process_group(child.id());
        let _ = child.wait();
    }
}

impl Drop for GuardedProcessGroup {
    fn drop(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        match child.try_wait() {
            Ok(Some(_)) => {}
            Ok(None) | Err(_) => {
                terminate_process_group(child.id());
                let _ = child.kill();
            }
        }
        let _ = child.wait();
    }
}

struct ForegroundFixture {
    root: PathBuf,
    repo_dir: PathBuf,
    bin: PathBuf,
    mode: PathBuf,
    comments: PathBuf,
    pull_requests: PathBuf,
    calls: PathBuf,
    accountability: PathBuf,
    operator: PathBuf,
    state: PathBuf,
    health: PathBuf,
    heartbeats: PathBuf,
    claim_repo: PathBuf,
    claim_remote: PathBuf,
    claim_state: PathBuf,
}

struct RealBridgeEnvironment {
    bin: PathBuf,
    aliases: PathBuf,
    remote: PathBuf,
    merged: PathBuf,
    safe_root: PathBuf,
    executor_fixture: PathBuf,
    executor_state_dir: PathBuf,
    _cross_process_lock: File,
}

impl Drop for RealBridgeEnvironment {
    fn drop(&mut self) {
        if let Ok(entries) = fs::read_dir(&self.executor_state_dir) {
            for entry in entries.filter_map(Result::ok) {
                let Ok(value) = fs::read_to_string(entry.path())
                    .ok()
                    .and_then(|body| serde_json::from_str::<serde_json::Value>(&body).ok())
                    .ok_or(())
                else {
                    continue;
                };
                for identity in ["supervisor", "process"] {
                    if let Some(process_group) = value
                        .get(identity)
                        .and_then(|value| value.get("process_group"))
                        .and_then(serde_json::Value::as_u64)
                        .and_then(|value| u32::try_from(value).ok())
                    {
                        terminate_process_group(process_group);
                    }
                }
            }
        }
        let _ = fs::remove_dir_all(&self.executor_fixture);
        let _ = fs::remove_dir_all(&self.safe_root);
    }
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
        let accountability = root.join("accountability-epic.md");
        let operator = root.join("operator");
        let state = root.join("state");
        let health = root.join("health");
        let heartbeats = root.join("heartbeats");
        let claim_repo = root.join("claim-repo");
        let claim_remote = root.join("claim-remote.git");
        let claim_state = root.join("claim-state");
        fs::create_dir_all(&repo_dir).expect("create repo directory");
        fs::create_dir_all(&bin).expect("create fake bin");
        git_fixture(&root, &["init", "--bare", claim_remote.to_str().unwrap()]);
        git_fixture(&root, &["init", claim_repo.to_str().unwrap()]);
        git_fixture(
            &claim_repo,
            &["remote", "add", "origin", claim_remote.to_str().unwrap()],
        );
        fs::write(&mode, "unreviewed\n").expect("write mode");
        fs::write(&comments, "[]\n").expect("write comments");
        fs::write(&pull_requests, "[]\n").expect("write pull requests");
        write_executable(
            &bin.join("gh"),
            r####"#!/bin/sh
set -eu
printf '%s\n' "$@" >> "$AUTOSPEC_FOREGROUND_CALLS"
if [ -n "${AUTOSPEC_FOREGROUND_ACCOUNTABILITY_HANDLER:-}" ]; then . "$AUTOSPEC_FOREGROUND_ACCOUNTABILITY_HANDLER"; fi
if [ "${AUTOSPEC_FOREGROUND_BLOCK_GH:-0}" = 1 ]; then
  while :; do sleep 1; done
fi
mode="$(cat "$AUTOSPEC_FOREGROUND_MODE")"
if [ "$1" = pr ] && [ "${2:-}" = list ] && [ -n "${AUTOSPEC_BRIDGE_TAKEOVER_OID:-}" ] && [ -e "${AUTOSPEC_BRIDGE_HARNESS_DONE:-/nonexistent}" ] && [ ! -e "${AUTOSPEC_BRIDGE_TAKEOVER_DONE:-/nonexistent}" ]; then
  git --git-dir "$AUTOSPEC_BRIDGE_REMOTE" update-ref refs/autospec/claims/issue-42 "$AUTOSPEC_BRIDGE_TAKEOVER_OID"
  : > "$AUTOSPEC_BRIDGE_TAKEOVER_DONE"
fi
if [ "$1" = pr ] && [ "${2:-}" = list ] && [ -n "${AUTOSPEC_BRIDGE_FAIL_GH_READ_ONCE:-}" ] && [ ! -e "$AUTOSPEC_BRIDGE_FAIL_GH_READ_ONCE" ] && { [ -z "${AUTOSPEC_BRIDGE_HARNESS_DONE:-}" ] || [ -e "$AUTOSPEC_BRIDGE_HARNESS_DONE" ]; } && { [ "${AUTOSPEC_BRIDGE_FAIL_GH_AFTER_BRANCH:-0}" != 1 ] || git --git-dir "$AUTOSPEC_BRIDGE_REMOTE" show-ref --verify --quiet refs/heads/feat/autonomous-issue-42; }; then
  : > "$AUTOSPEC_BRIDGE_FAIL_GH_READ_ONCE"
  exit 42
fi
if [ "$1" = pr ] && [ "${2:-}" = list ] && [ "${AUTOSPEC_BRIDGE_FAIL_GH_AFTER_CREATE_ALWAYS:-0}" = 1 ] && [ "$(cat "$AUTOSPEC_FOREGROUND_PULL_REQUESTS")" != "[]" ]; then
  exit 42
fi
issue() {
  if [ "${AUTOSPEC_FOREGROUND_REAL_BRIDGE:-0}" = 1 ]; then
    if [ "$mode" = claimed ]; then real_labels='[{"name":"in-progress-by-bot"},{"name":"safety:reviewed"}]'; elif [ "$mode" = terminal ]; then real_labels='[]'; else real_labels='[{"name":"auto-implement"},{"name":"safety:reviewed"}]'; fi
    printf '%s\n' "{\"number\":42,\"title\":\"Ship the bridge fixture\",\"body\":\"## Goal\\n\\nAdd \`tests/smoke/generation.sh\` proving the native executor bridge runs.\\n\\n## Safety review\\n\\n<!-- autospec-safety:begin -->\\n- **decision:** \`SAFETY_PASS\`\\n<!-- autospec-safety:end -->\\n\\n## Implementation outline\\n\\n- \`tests/smoke/generation.sh\`\\n\\n## Tests required\\n\\n- smoke\\n\\n### Primary smoke test (inner loop)\\n\\n\`\`\`bash\\n/usr/bin/test -s tests/smoke/generation.sh\\n\`\`\`\\n\\n### Operator/full verification\\n\\n\`\`\`bash\\n/usr/bin/test -s tests/smoke/generation.sh\\n\`\`\`\",\"labels\":$real_labels,\"author\":{\"login\":\"agent\"},\"state\":\"${FOREGROUND_ISSUE_STATE:-open}\"}"
  elif [ "$mode" = unreviewed ]; then
    printf '%s\n' '{"number":42,"title":"Add Rust foreground","body":"## Goal\n\nAdd the foreground adapter.","labels":[{"name":"auto-implement"}],"author":{"login":"agent"},"state":"'"${FOREGROUND_ISSUE_STATE:-open}"'"}'
  else
    printf '%s\n' '{"number":42,"title":"Add Rust foreground","body":"## Safety review\n\n<!-- autospec-safety:begin -->\n- **decision:** `SAFETY_PASS`\n<!-- autospec-safety:end -->","labels":[{"name":"auto-implement"},{"name":"safety:reviewed"}],"author":{"login":"agent"},"state":"'"${FOREGROUND_ISSUE_STATE:-open}"'"}'
  fi
}
claim_issue() {
  if [ "${AUTOSPEC_FOREGROUND_REAL_BRIDGE:-0}" = 1 ]; then
    case " $* " in
      *"{labels: [.labels[] | {name: .name}]}"*) if [ "$mode" = claimed ]; then labels='[{"name":"in-progress-by-bot"},{"name":"safety:reviewed"}]'; elif [ "$mode" = terminal ]; then labels='[]'; else labels='[{"name":"auto-implement"},{"name":"safety:reviewed"}]'; fi ;;
      *" --jq "*) if [ "$mode" = claimed ]; then labels='["in-progress-by-bot","safety:reviewed"]'; elif [ "$mode" = terminal ]; then labels='[]'; else labels='["auto-implement","safety:reviewed"]'; fi ;;
      *) if [ "$mode" = claimed ]; then labels='[{"name":"in-progress-by-bot"},{"name":"safety:reviewed"}]'; elif [ "$mode" = terminal ]; then labels='[]'; else labels='[{"name":"auto-implement"},{"name":"safety:reviewed"}]'; fi ;;
    esac
    if [ "$mode" = terminal ] && [ -n "${AUTOSPEC_FOREGROUND_FAIL_TERMINAL_ONCE:-}" ] && [ ! -e "$AUTOSPEC_FOREGROUND_FAIL_TERMINAL_ONCE" ]; then
      : > "$AUTOSPEC_FOREGROUND_FAIL_TERMINAL_ONCE"
      exit 1
    fi
    printf '%s\n' "{\"labels\":$labels}"
    return
  fi
  if [ "$mode" = claimed ]; then labels='["in-progress-by-bot","safety:reviewed"]'; else labels='["auto-implement","safety:reviewed"]'; fi
  printf '%s\n' "{\"labels\":$labels,\"title\":\"Add Rust foreground\",\"body\":\"## Safety review\\n\\n<!-- autospec-safety:begin -->\\n- **decision:** \`SAFETY_PASS\`\\n<!-- autospec-safety:end -->\",\"author\":\"agent\"}"
}
steal_claim() {
  reference=refs/autospec/claims/issue-42
  current=$(git --git-dir "$AUTOSPEC_CLAIM_GIT_REMOTE" rev-parse "$reference")
  tree=$(git --git-dir "$AUTOSPEC_CLAIM_GIT_REMOTE" mktree </dev/null)
  message="$AUTOSPEC_FOREGROUND_COMMENTS.claim-message"
  cat > "$message" <<'EOF'
autospec-claim-ledger-v1
generation=foreign-generation

<!-- autospec-run-state:begin -->
{"schema":1,"repo":"test/repo","issue":42,"worker_id":"foreign-worker","state":"claimed","branch":"foreign/issue-42","pr":"","step":"claimed","paths":[],"claimed_at":"2030-07-15T00:00:00Z","updated_at":"2030-07-15T00:00:00Z","ttl_seconds":10800,"claim_id":"foreign-claim"}
<!-- autospec-run-state:end -->
EOF
  oid=$(GIT_AUTHOR_NAME='Autospec Claim Test' \
    GIT_AUTHOR_EMAIL='autospec-claim-test@localhost' \
    GIT_COMMITTER_NAME='Autospec Claim Test' \
    GIT_COMMITTER_EMAIL='autospec-claim-test@localhost' \
    git --git-dir "$AUTOSPEC_CLAIM_GIT_REMOTE" commit-tree "$tree" -p "$current" -F "$message")
  git --git-dir "$AUTOSPEC_CLAIM_GIT_REMOTE" update-ref "$reference" "$oid" "$current"
  rm -f "$message"
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
    repos/test/repo/issues/42/comments*)
      if [ "${AUTOSPEC_FOREGROUND_FAIL_EVIDENCE_CONFIRM:-0}" = 1 ] && grep -q '<!-- autospec-executor-result:begin -->' "$AUTOSPEC_FOREGROUND_COMMENTS"; then
        exit 1
      fi
      cat "$AUTOSPEC_FOREGROUND_COMMENTS"
      exit 0 ;;
    repos/test/repo/pulls/17) printf '%s\n' '{}'; exit 0 ;;
    repos/test/repo/issues/17/comments*|repos/test/repo/pulls/17/reviews*|repos/test/repo/pulls/17/comments*)
      printf '%s\n' '[]'
      exit 0 ;;
    repos/test/repo/issues/42/labels) printf 'reviewed\n' > "$AUTOSPEC_FOREGROUND_MODE"; exit 0 ;;
    repos/test/repo/issues/42)
      if printf '%s\n' "$@" | grep -q PATCH; then printf 'reviewed\n' > "$AUTOSPEC_FOREGROUND_MODE"; else issue; fi
      exit 0 ;;
    repos/test/repo/issues/comments/100)
      body=""
      for value in "$@"; do case "$value" in body=*) body="${value#body=}" ;; esac; done
      if [ "${AUTOSPEC_FOREGROUND_STEAL_ON_OUTCOME:-0}" = 1 ] && printf '%s' "$body" | grep -q executor_pending; then
        steal_claim
      fi
      jq --arg body "$body" '.[0].body = $body | .[0].updated_at = "2026-07-15T00:00:00Z"' "$AUTOSPEC_FOREGROUND_COMMENTS" > "$AUTOSPEC_FOREGROUND_COMMENTS.tmp"
      mv "$AUTOSPEC_FOREGROUND_COMMENTS.tmp" "$AUTOSPEC_FOREGROUND_COMMENTS"
      exit 0 ;;
  esac
fi
if [ "$1" = issue ] && [ "$2" = view ]; then claim_issue "$@"; exit 0; fi
if [ "$1" = label ] && [ "$2" = create ]; then exit 0; fi
if [ "$1" = issue ] && [ "$2" = edit ]; then
  if [ "${AUTOSPEC_FOREGROUND_REAL_BRIDGE:-0}" = 1 ]; then
    case " $* " in
      *" --remove-label in-progress-by-bot "*" --add-label auto-implement "*) printf 'reviewed\n' > "$AUTOSPEC_FOREGROUND_MODE" ;;
      *" --remove-label in-progress-by-bot "*) printf 'terminal\n' > "$AUTOSPEC_FOREGROUND_MODE" ;;
      *" --add-label in-progress-by-bot "*) printf 'claimed\n' > "$AUTOSPEC_FOREGROUND_MODE" ;;
    esac
    if [ -n "${FOREGROUND_STOP_ON_RETRYABLE_RELEASE:-}" ] && [ "$(cat "$AUTOSPEC_FOREGROUND_MODE")" = reviewed ]; then
      mkdir -p "$(dirname "$FOREGROUND_STOP_ON_RETRYABLE_RELEASE")"
      printf '%s\n' "${FOREGROUND_STOP_MODE_ON_RETRYABLE_RELEASE:-immediate}" '2026-07-31T00:00:00Z test@localhost' > "$FOREGROUND_STOP_ON_RETRYABLE_RELEASE"
    fi
  else
    last=""
    for value in "$@"; do last="$value"; done
    case "$last" in
      auto-implement) printf 'reviewed\n' > "$AUTOSPEC_FOREGROUND_MODE" ;;
      *) printf 'claimed\n' > "$AUTOSPEC_FOREGROUND_MODE" ;;
    esac
  fi
  exit 0
fi
if [ "$1" = issue ] && [ "$2" = comment ]; then
  body=""; shift 2
  while [ "$#" -gt 0 ]; do case "$1" in --body) body="$2"; shift 2 ;; *) shift ;; esac; done
  if [ "${AUTOSPEC_FOREGROUND_FAIL_EVIDENCE_CREATE:-0}" = 1 ] && printf '%s' "$body" | grep -q '<!-- autospec-executor-result:begin -->'; then
    exit 1
  fi
  if [ "${AUTOSPEC_FOREGROUND_STEAL_ON_OUTCOME:-0}" = 1 ] && printf '%s' "$body" | grep -q executor_pending; then
    steal_claim
  fi
  jq --arg body "$body" '. + [{"id":((map(.id) | max // 99) + 1),"updated_at":"2026-07-15T00:00:00Z","body":$body}]' "$AUTOSPEC_FOREGROUND_COMMENTS" > "$AUTOSPEC_FOREGROUND_COMMENTS.tmp"
  mv "$AUTOSPEC_FOREGROUND_COMMENTS.tmp" "$AUTOSPEC_FOREGROUND_COMMENTS"
  exit 0
fi
if [ "$1" = pr ] && [ "$2" = list ]; then
  if [ "${AUTOSPEC_FOREGROUND_STEAL_ON_RESULT_VALIDATION:-0}" = 1 ]; then
    steal_claim
  fi
  cat "$AUTOSPEC_FOREGROUND_PULL_REQUESTS"
  exit 0
fi
if [ "${AUTOSPEC_FOREGROUND_REAL_BRIDGE:-0}" = 1 ] && [ "$1" = pr ] && [ "$2" = create ]; then
  head=$(git --git-dir "$AUTOSPEC_BRIDGE_REMOTE" rev-parse refs/heads/feat/autonomous-issue-42)
  base="${AUTOSPEC_BRIDGE_BASE_REF:-main}"
  body_file=""; previous=""
  for value in "$@"; do
    if [ "$previous" = --body-file ]; then body_file="$value"; fi
    previous="$value"
  done
  jq -n --rawfile body "$body_file" --arg head "$head" --arg base "$base" '[{"number":17,"body":$body,"headRefName":"feat/autonomous-issue-42","headRefOid":$head,"isDraft":true,"baseRefName":$base}]' > "$AUTOSPEC_FOREGROUND_PULL_REQUESTS"
  printf '%s\n' 'https://example.invalid/test/repo/pull/17'
  exit 0
fi
if [ "${AUTOSPEC_FOREGROUND_REAL_BRIDGE:-0}" = 1 ] && [ "$1" = pr ] && [ "$2" = ready ]; then
  jq '.[0].isDraft = false' "$AUTOSPEC_FOREGROUND_PULL_REQUESTS" > "$AUTOSPEC_FOREGROUND_PULL_REQUESTS.tmp"
  mv "$AUTOSPEC_FOREGROUND_PULL_REQUESTS.tmp" "$AUTOSPEC_FOREGROUND_PULL_REQUESTS"
  exit 0
fi
if [ "${AUTOSPEC_FOREGROUND_REAL_BRIDGE:-0}" = 1 ] && [ "$1" = pr ] && [ "$2" = view ]; then
  head=$(jq -r '.[0].headRefOid' "$AUTOSPEC_FOREGROUND_PULL_REQUESTS")
  body=$(jq -c '.[0].body' "$AUTOSPEC_FOREGROUND_PULL_REQUESTS")
  base="${AUTOSPEC_BRIDGE_BASE_REF:-main}"
  case " $* " in
    *" headRefOid,statusCheckRollup "*)
      printf '%s\n' "{\"headRefOid\":\"$head\",\"statusCheckRollup\":[{\"name\":\"ci\",\"status\":\"COMPLETED\",\"conclusion\":\"SUCCESS\"}]}" ;;
    *)
      if [ -e "$AUTOSPEC_BRIDGE_MERGED" ]; then
        merge=$(cat "$AUTOSPEC_BRIDGE_MERGED")
        case " $* " in
          *" number,state,isDraft,headRefName,headRefOid,baseRefName,mergeCommit"*)
            printf '%s\n' "{\"number\":17,\"state\":\"MERGED\",\"isDraft\":false,\"headRefName\":\"feat/autonomous-issue-42\",\"headRefOid\":\"$head\",\"baseRefName\":\"$base\",\"mergeCommit\":{\"oid\":\"$merge\"},\"body\":$body}" ;;
          *)
            printf '%s\n' "{\"number\":17,\"state\":\"MERGED\",\"isDraft\":false,\"headRefOid\":\"$head\",\"baseRefName\":\"$base\",\"mergeCommit\":{\"oid\":\"$merge\"},\"body\":$body}" ;;
        esac
      else
        case " $* " in
          *" number,state,isDraft,headRefName,headRefOid,baseRefName,mergeCommit"*)
            printf '%s\n' "{\"number\":17,\"state\":\"OPEN\",\"isDraft\":false,\"headRefName\":\"feat/autonomous-issue-42\",\"headRefOid\":\"$head\",\"baseRefName\":\"$base\",\"mergeCommit\":null,\"body\":$body}" ;;
          *)
            printf '%s\n' "{\"number\":17,\"state\":\"OPEN\",\"isDraft\":false,\"headRefOid\":\"$head\",\"baseRefName\":\"$base\",\"mergeCommit\":null,\"body\":$body}" ;;
        esac
      fi ;;
  esac
  exit 0
fi
if [ "${AUTOSPEC_FOREGROUND_REAL_BRIDGE:-0}" = 1 ] && [ "$1" = pr ] && [ "$2" = merge ]; then
  head=$(jq -r '.[0].headRefOid' "$AUTOSPEC_FOREGROUND_PULL_REQUESTS")
  base="${AUTOSPEC_BRIDGE_BASE_REF:-main}"
  case " $* " in *" --match-head-commit $head "*) ;; *) exit 74 ;; esac
  git --git-dir "$AUTOSPEC_BRIDGE_REMOTE" update-ref "refs/heads/$base" "$head"
  git --git-dir "$AUTOSPEC_BRIDGE_REMOTE" update-ref -d refs/heads/feat/autonomous-issue-42
  printf '%s\n' "$head" > "$AUTOSPEC_BRIDGE_MERGED"
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
            accountability,
            operator,
            state,
            health,
            heartbeats,
            claim_repo,
            claim_remote,
            claim_state,
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

    fn configure_real_bridge(&self) -> RealBridgeEnvironment {
        let fixture_root = PathBuf::from(std::env::var_os("HOME").expect("HOME for bridge tools"))
            .join(".autospec-test-fixtures");
        fs::create_dir_all(&fixture_root).expect("create bridge fixture root");
        let cross_process_lock = File::options()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(fixture_root.join("bridge-e2e.lock"))
            .expect("open bridge E2E lock");
        cross_process_lock
            .lock()
            .expect("acquire cross-process bridge E2E lock");
        let executor_fixture = Path::new("/tmp/autospec-executor")
            .join(format!("test_repo-{}", &sha256_hex(b"test/repo")[..12]));
        if executor_fixture.exists() {
            fs::remove_dir_all(&executor_fixture).expect("clear stale bridge test fixture");
        }
        let remote = self.root.join("bridge-origin.git");
        git_fixture(&self.root, &["init", "--bare", remote.to_str().unwrap()]);
        git_fixture(&self.repo_dir, &["init", "-b", "main"]);
        autonomous_accountability_acquisition::write_git_exclude(&self.repo_dir);
        git_fixture(
            &self.repo_dir,
            &["config", "user.name", "Autospec Bridge Test"],
        );
        git_fixture(
            &self.repo_dir,
            &["config", "user.email", "bridge-test@example.invalid"],
        );
        fs::write(self.repo_dir.join("README.md"), "bridge fixture\n")
            .expect("write bridge baseline");
        git_fixture(&self.repo_dir, &["add", "README.md"]);
        git_fixture(&self.repo_dir, &["commit", "-m", "bridge fixture"]);
        git_fixture(
            &self.repo_dir,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        git_fixture(&self.repo_dir, &["push", "-u", "origin", "main"]);
        git_fixture(&remote, &["symbolic-ref", "HEAD", "refs/heads/main"]);
        git_fixture(&self.repo_dir, &["remote", "set-head", "origin", "main"]);

        let safe_root = fixture_root.join(format!(
            "bridge-e2e-{}",
            self.root
                .file_name()
                .and_then(|name| name.to_str())
                .expect("fixture name")
        ));
        let bin = safe_root.join("bin");
        fs::create_dir_all(&bin).expect("create bridge tools");
        fs::copy(self.bin.join("gh"), bin.join("gh")).expect("copy bridge gh fixture");
        fs::set_permissions(bin.join("gh"), fs::Permissions::from_mode(0o755))
            .expect("make bridge gh executable");
        let review_launches = safe_root.join("review-launches");
        write_executable(
            &bin.join("codex-bridge-fixture"),
            &r#"#!/bin/sh
set -eu
[ "${1:-}" = "sandbox" ] && exit 0
artifact=""
previous=""
prompt=""
for value in "$@"; do
  if [ "$previous" = "--output-last-message" ]; then artifact="$value"; fi
  previous="$value"
  prompt="$value"
done
test -n "$artifact"
case "$prompt" in
  *"Return exactly one JSON object"*)
    /usr/bin/python3 -c 'import json,sys;p,a=sys.argv[1:];c,_=json.JSONDecoder().raw_decode(p.split("Commit-bound review context (inspect every cited immutable record before approval):\n",1)[1]);b=json.dumps({"schema":1,"commit":c["commit"],"verdict":"lgtm","surfaces_examined":c["changed_paths"],"tests_examined":["tests/smoke/generation.sh"],"integration_paths_checked":c["required_integration_citations"],"blocking_findings":[]},separators=(",",":"));print(b);open(a,"w",encoding="utf-8").write(b+"\n")' "$prompt" "$artifact"
    chmod 600 "$artifact"
    i=0
    while [ "$i" -lt 64 ]; do
      printf '%s\n' 'codex startup/tool trace: normal non-verdict diagnostic output' >&2
      i=$((i + 1))
    done
    printf '%s\n' "$$" >> '__AUTOSPEC_BRIDGE_REVIEW_LAUNCHES__'
    exit 0
    ;;
esac
mkdir -p tests/smoke "$(dirname "$artifact")"
if [ -n "${AUTOSPEC_BRIDGE_HARNESS_LAUNCHES:-}" ]; then
  printf '%s\n' "$$" >> "$AUTOSPEC_BRIDGE_HARNESS_LAUNCHES"
fi
if [ -n "${AUTOSPEC_BRIDGE_SLOW_HARNESS_MARKER:-}" ] && [ ! -e "$AUTOSPEC_BRIDGE_SLOW_HARNESS_MARKER" ]; then
  printf '%s\n' "$$" > "$AUTOSPEC_BRIDGE_SLOW_HARNESS_MARKER"
  sleep "${AUTOSPEC_BRIDGE_SLOW_HARNESS_SECONDS:-3}"
fi
if [ -n "${AUTOSPEC_BRIDGE_ZERO_EFFECT_ONCE:-}" ] && [ ! -e "$AUTOSPEC_BRIDGE_ZERO_EFFECT_ONCE" ]; then
  : > "$AUTOSPEC_BRIDGE_ZERO_EFFECT_ONCE"
  worktree=$PWD
  (
    sleep 0.2
    rm -rf "$worktree"
    : > "${AUTOSPEC_BRIDGE_ZERO_EFFECT_ONCE%.*}.deleted"
  ) </dev/null >/dev/null 2>&1 &
  exit 0
fi
if [ -n "${AUTOSPEC_BRIDGE_FAIL_HARNESS_ONCE:-}" ] && [ ! -e "$AUTOSPEC_BRIDGE_FAIL_HARNESS_ONCE" ]; then
  printf '%s\n' '#!/bin/sh' 'exit 42' > tests/smoke/generation.sh
  : > "$AUTOSPEC_BRIDGE_FAIL_HARNESS_ONCE"
  if [ "${AUTOSPEC_BRIDGE_ADVANCE_MAIN_ON_FAIL:-0}" = 1 ]; then
    base=$(git --git-dir "$AUTOSPEC_BRIDGE_REMOTE" rev-parse refs/heads/main)
    tree=$(git --git-dir "$AUTOSPEC_BRIDGE_REMOTE" rev-parse "$base^{tree}")
    advanced=$(printf '%s\n' 'advance main during dirty retry' | \
      GIT_AUTHOR_NAME='Autospec Bridge Test' \
      GIT_AUTHOR_EMAIL='bridge-test@example.invalid' \
      GIT_COMMITTER_NAME='Autospec Bridge Test' \
      GIT_COMMITTER_EMAIL='bridge-test@example.invalid' \
      git --git-dir "$AUTOSPEC_BRIDGE_REMOTE" commit-tree "$tree" -p "$base")
    git --git-dir "$AUTOSPEC_BRIDGE_REMOTE" update-ref refs/heads/main "$advanced" "$base"
  fi
  exit 42
fi
printf '%s\n' '#!/bin/sh' 'exit 0' > tests/smoke/generation.sh
chmod 755 tests/smoke/generation.sh
git add tests/smoke/generation.sh
git commit -m 'test: prove native bridge execution'
cat > "$artifact" <<'EOF'
## Closeout report

Result: Added the native bridge smoke artifact.
Claims: [verified] static tests/smoke/generation.sh is committed.
Proof type: static
Before/after: 0 to 1 bridge smoke artifacts.
Artifacts: tests/smoke/generation.sh; /usr/bin/test -s tests/smoke/generation.sh
Scoped git status: tests/smoke/generation.sh
One likely hidden failure: scanner behavior outside this hermetic fixture.
EOF
chmod 600 "$artifact"
if [ -n "${AUTOSPEC_BRIDGE_HARNESS_DONE:-}" ]; then
  : > "$AUTOSPEC_BRIDGE_HARNESS_DONE"
fi
"#
            .replace(
                "__AUTOSPEC_BRIDGE_REVIEW_LAUNCHES__",
                &review_launches.display().to_string(),
            ),
        );
        for (scanner, output) in [
            (
                "semgrep",
                r#"{"results":[],"errors":[],"paths":{"scanned":["tests/smoke/generation.sh"],"skipped":[]}}"#,
            ),
            ("trivy", r#"{"Results":[{"Target":"."}]}"#),
            ("license-checker", r#"{"fixture@1.0.0":{"licenses":"MIT"}}"#),
        ] {
            write_executable(
                &bin.join(scanner),
                &format!("#!/bin/sh\nset -eu\nprintf '%s\\n' '{output}'\n"),
            );
        }
        write_executable(
            &bin.join("gitleaks"),
            r#"#!/bin/sh
set -eu
report=""
previous=""
for value in "$@"; do
  if [ "$previous" = "--report-path" ]; then report="$value"; fi
  previous="$value"
done
printf '%s\n' '[]' > "$report"
"#,
        );
        let aliases = safe_root.join("harness-runtime-aliases.tsv");
        fs::write(
            &aliases,
            format!(
                "codex\t{}\tautospec-codex\tCodex bridge fixture\n",
                bin.join("codex-bridge-fixture").display()
            ),
        )
        .expect("write bridge alias table");
        RealBridgeEnvironment {
            bin,
            aliases,
            remote,
            merged: self.root.join("bridge-merged"),
            safe_root,
            executor_fixture,
            executor_state_dir: self.scoped_dir().join("executor"),
            _cross_process_lock: cross_process_lock,
        }
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
            .env("AUTOSPEC_FOREGROUND_ACCOUNTABILITY", &self.accountability)
            .env("AUTOSPEC_FOREGROUND_ACCOUNTABILITY_HANDLER", concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/foreground_accountability_gh.sh"))
            .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &self.operator)
            .env("AUTOSPEC_STATE_DIR", &self.state)
            .env("AUTOSPEC_AUTONOMOUS_SPEND_DIR", self.root.join("spend"))
            .env("AUTOSPEC_AUTONOMOUS_STATE_DIR", &self.health)
            .env("AUTOSPEC_HEARTBEAT_DIR", &self.heartbeats)
            .env("AUTOSPEC_CLAIM_GIT_REMOTE", &self.claim_remote)
            .env("AUTOSPEC_CLAIM_GIT_STATE_DIR", &self.claim_state)
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
        ]);
        command
    }

    fn start_blocked_detached(&self) {
        self.start_blocked_detached_with_log(None);
    }

    fn start_blocked_detached_with_log(&self, logpath: Option<&Path>) {
        let mut command = self.detached_command("start");
        command.arg("--detach").args(["--branch", "main"]);
        if let Some(logpath) = logpath {
            command.args(["--log", logpath.to_str().expect("log path")]);
        }
        let output = command
            .env("AUTOSPEC_AUTONOMOUS_COMPANIONS", "0")
            .env("AUTOSPEC_FOREGROUND_BLOCK_GH", "1")
            .output()
            .expect("start blocked detached conductor");
        assert!(
            output.status.success(),
            "stderr={}",
            String::from_utf8_lossy(&output.stderr)
        );
        wait_for_file_contents(&self.calls, "repos/test/repo/branches/main");
        let conductor_pid = self.recorded_conductor_pid().expect("conductor pid");
        assert!(process_is_running(conductor_pid));
    }

    fn spawn_following_start(&self, iterations: u64) -> GuardedProcessGroup {
        self.spawn_following_start_command(iterations, true)
    }

    fn spawn_following_start_without_interval(&self, iterations: u64) -> GuardedProcessGroup {
        self.spawn_following_start_command(iterations, false)
    }

    fn spawn_following_start_command(
        &self,
        iterations: u64,
        set_interval: bool,
    ) -> GuardedProcessGroup {
        let mut command = self.detached_command("start");
        let output = File::create(self.root.join("follower.log")).expect("create follower log");
        let errors = output.try_clone().expect("clone follower log");
        command.args([
            "--follow",
            "--iterations",
            &iterations.to_string(),
            "--branch",
            "main",
        ]);
        if set_interval {
            command.args(["--interval-sec", "1"]);
        }
        command
            .env("AUTOSPEC_AUTONOMOUS_COMPANIONS", "0")
            .env("AUTOSPEC_FOREGROUND_BLOCK_GH", "1")
            .stdout(Stdio::from(output))
            .stderr(Stdio::from(errors))
            .process_group(0);
        GuardedProcessGroup::new(command.spawn().expect("spawn following start"))
    }

    fn recorded_conductor_pid(&self) -> Option<u32> {
        let metadata = fs::read_to_string(self.operator.join("test_repo/conductor.pid")).ok()?;
        let (_, value) = metadata.split_once("\"pid\":")?;
        value
            .split(|character: char| !character.is_ascii_digit())
            .next()?
            .parse()
            .ok()
    }

    fn terminate_recorded_conductor(&self) {
        if let Some(pid) = self.recorded_conductor_pid() {
            terminate_process_group(pid);
            terminate_process(pid);
        }
    }

    fn recorded_conductor_logpath(&self) -> PathBuf {
        PathBuf::from(
            fs::read_to_string(self.scoped_dir().join("conductor.logpath"))
                .expect("read conductor logpath")
                .trim(),
        )
    }

    fn scoped_dir(&self) -> PathBuf {
        self.operator.join("test_repo")
    }

    fn scoped_stop_sentinel(&self) -> PathBuf {
        self.scoped_dir().join("stop.flag")
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
        let remote = self.root.join("github.com/test/repo.git");
        fs::create_dir_all(remote.parent().expect("integration remote parent"))
            .expect("create integration remote parent");
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

    fn initialize_empty_local_remote(&self) {
        git_fixture(&self.repo_dir, &["init", "-b", "main"]);
        git_fixture(
            &self.repo_dir,
            &[
                "remote",
                "add",
                "origin",
                self.claim_remote.to_str().expect("claim remote"),
            ],
        );
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
            match state {
                "released" => "retryable_released",
                "failed" => "needs_human",
                _ => "claimed",
            },
            Vec::new(),
            updated_at,
            updated_at,
            10_800,
        )
        .with_claim_id(claim_id);
        self.transition_claim_ref(&record);
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

    fn transition_claim_ref(&self, record: &RunStateRecord) {
        self.transition_claim_ref_to(record, &self.claim_remote);
    }

    fn transition_claim_ref_to(&self, record: &RunStateRecord, remote: &Path) {
        let reference = format!("refs/autospec/claims/issue-{}", record.issue);
        let remote = remote.to_str().expect("claim remote path");
        let current = git_fixture(
            &self.claim_repo,
            &["ls-remote", "--refs", remote, &reference],
        );
        let parent = current.split_whitespace().next().map(str::to_string);
        if parent.is_some() {
            git_fixture(
                &self.claim_repo,
                &["fetch", "--no-tags", remote, &reference],
            );
        }
        let tree = git_fixture(&self.claim_repo, &["mktree"]);
        let mut command = Command::new("git");
        command
            .arg("commit-tree")
            .arg(tree)
            .current_dir(&self.claim_repo)
            .env("GIT_AUTHOR_NAME", "Autospec Claim Test")
            .env("GIT_AUTHOR_EMAIL", "autospec-claim-test@localhost")
            .env("GIT_COMMITTER_NAME", "Autospec Claim Test")
            .env("GIT_COMMITTER_EMAIL", "autospec-claim-test@localhost")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped());
        if let Some(parent) = parent.as_deref() {
            command.args(["-p", parent]);
        }
        let mut child = command.spawn().expect("create claim ledger commit");
        write!(
            child.stdin.take().expect("claim commit stdin"),
            "autospec-claim-ledger-v1\ngeneration=fixture-{}\n\n{}\n",
            TEMP_DIR_SEQUENCE.fetch_add(1, Ordering::Relaxed),
            record.to_marked_comment()
        )
        .expect("write claim ledger commit");
        let output = child.wait_with_output().expect("claim commit output");
        assert!(
            output.status.success(),
            "create claim commit: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let oid = String::from_utf8_lossy(&output.stdout).trim().to_string();
        git_fixture(
            &self.claim_repo,
            &["push", remote, &format!("{oid}:{reference}")],
        );
    }

    fn prepare_foreign_claim_object_in(&self, remote: &Path) -> String {
        let timestamp = fresh_iso_timestamp();
        let record = RunStateRecord::new(
            "test/repo",
            42,
            "foreign-conductor",
            "claimed",
            "feat/autonomous-issue-42",
            "",
            "claimed",
            Vec::new(),
            &timestamp,
            &timestamp,
            10_800,
        )
        .with_claim_id("foreign-generation");
        self.transition_claim_ref(&record);
        let seeded = git_fixture(
            &self.claim_repo,
            &[
                "ls-remote",
                "--refs",
                self.claim_remote.to_str().expect("claim remote"),
                "refs/autospec/claims/issue-42",
            ],
        );
        let oid = seeded
            .split_whitespace()
            .next()
            .expect("foreign claim oid")
            .to_string();
        git_fixture(
            &self.claim_repo,
            &[
                "push",
                remote.to_str().expect("target claim remote"),
                &format!("{oid}:refs/autospec/test/foreign-claim"),
            ],
        );
        git_fixture(
            &self.claim_repo,
            &[
                "--git-dir",
                remote.to_str().expect("target claim remote"),
                "update-ref",
                "-d",
                "refs/autospec/test/foreign-claim",
            ],
        );
        oid
    }

    fn seed_claim_acquisition_receipt(&self, worker_id: &str, branch: &str, claim_id: &str) {
        let path = self.state_path().with_extension("claim-acquisition.json");
        fs::set_permissions(
            path.parent().expect("receipt parent"),
            fs::Permissions::from_mode(0o700),
        )
        .expect("make claim acquisition receipt parent private");
        fs::write(
            &path,
            format!(
                "{{\"schema\":1,\"repo\":\"test/repo\",\"issue\":42,\"worker_id\":{:?},\"branch\":{:?},\"claim_id\":{:?}}}\n",
                worker_id, branch, claim_id
            ),
        )
        .expect("seed claim acquisition receipt");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .expect("make claim acquisition receipt private");
    }

    fn seed_interrupted_executor_invocation_without_cleanup(
        &self,
        worker_id: &str,
        branch: &str,
        claim_id: &str,
    ) {
        let state_dir = self.scoped_dir().join("executor");
        fs::create_dir_all(&state_dir).expect("create executor state directory");
        fs::set_permissions(&state_dir, fs::Permissions::from_mode(0o700))
            .expect("make executor state directory private");
        let generation = &sha256_hex(claim_id.as_bytes())[..16];
        let path = state_dir.join(format!("issue-42-{generation}.json"));
        let body = serde_json::json!({
            "schema": 1,
            "identity": {
                "repository": "test/repo",
                "repository_path": self.repo_dir,
                "issue": 42,
                "worker_id": worker_id,
                "branch": branch,
                "claim_id": claim_id,
                "invocation_id": format!("42-{claim_id}"),
                "base_ref": "origin/main",
                "base_oid": "0".repeat(40),
                "worktree": self.root.join("interrupted-worktree"),
                "runtime_environment_dir": null,
                "runtime_session_id": null
            },
            "harness": "claude",
            "phase": "interrupted",
            "supervisor": null,
            "process": null,
            "progress_at": 1,
            "pr": null,
            "head_oid": null,
            "closeout_path": null,
            "closeout_digest": null,
            "remote_snapshot_digest": null,
            "draft_process": null,
            "terminal_result": null,
            "umbrella": null,
            "current_child": null
        });
        fs::write(&path, format!("{body}\n")).expect("seed interrupted executor invocation");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .expect("make interrupted executor invocation private");
        assert!(
            !path.with_extension("failure-cleanup.json").exists(),
            "the crash window must precede failure cleanup intent persistence"
        );
    }

    fn seed_claim_heartbeat(&self, worker_id: &str, branch: &str, claim_id: &str) {
        let pid = std::process::id();
        let (_, process_start) = process_identity(pid).expect("current process identity");
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("heartbeat clock")
            .as_secs();
        self.seed_claim_heartbeat_record(worker_id, branch, claim_id, now, pid, process_start);
    }

    fn seed_expired_claim_heartbeat(&self, worker_id: &str, branch: &str, claim_id: &str) {
        self.seed_claim_heartbeat_record(worker_id, branch, claim_id, 1, 2_147_483_647, 1);
    }

    fn seed_pending_claim_heartbeat_handoff(
        &self,
        worker_id: &str,
        branch: &str,
        claim_id: &str,
    ) -> (PathBuf, PathBuf) {
        self.seed_expired_claim_heartbeat(worker_id, branch, claim_id);
        let mut identity = Vec::new();
        for field in [
            "test/repo",
            "42",
            worker_id,
            branch,
            "",
            claim_id,
            "claimed",
        ] {
            identity.extend_from_slice(&(field.len() as u64).to_be_bytes());
            identity.extend_from_slice(field.as_bytes());
        }
        let digest = sha256_hex(&identity);
        let quarantine = self.heartbeats.join("o4_test_r4_repo/quarantine");
        let handoff = quarantine.join("startup-heartbeat-handoffs");
        fs::create_dir_all(&handoff).expect("create heartbeat handoff directory");
        for private in [&quarantine, &handoff] {
            fs::set_permissions(private, fs::Permissions::from_mode(0o700))
                .expect("make heartbeat handoff directory private");
        }
        let pending = handoff.join(format!("pending-42-{digest}.receipt"));
        let completed = handoff.join(format!("completed-42-{digest}.receipt"));
        let archive = handoff.join(format!("completed-42-{digest}.json"));
        fs::write(&pending, "").expect("seed pending heartbeat receipt");
        fs::set_permissions(&pending, fs::Permissions::from_mode(0o600))
            .expect("make pending heartbeat receipt private");
        fs::rename(self.heartbeats.join("o4_test_r4_repo/42.json"), archive)
            .expect("seed moved heartbeat archive");
        (pending, completed)
    }

    fn seed_claim_heartbeat_record(
        &self,
        worker_id: &str,
        branch: &str,
        claim_id: &str,
        timestamp: u64,
        pid: u32,
        process_start: u64,
    ) {
        let repo = "test/repo";
        let issue = 42_u64;
        let host = fs::read_to_string("/proc/sys/kernel/hostname")
            .expect("read current host")
            .trim()
            .to_string();
        let boot_id = fs::read_to_string("/proc/sys/kernel/random/boot_id")
            .expect("read current boot identity")
            .trim()
            .to_string();
        let mut nonce_frame = b"autospec-startup-heartbeat-nonce-v1".to_vec();
        for field in [repo, &issue.to_string(), claim_id] {
            nonce_frame.extend_from_slice(&(field.len() as u64).to_be_bytes());
            nonce_frame.extend_from_slice(field.as_bytes());
        }
        let directory = self.heartbeats.join("o4_test_r4_repo");
        fs::create_dir_all(&directory).expect("create claim heartbeat directory");
        for private in [&self.heartbeats, &directory] {
            fs::set_permissions(private, fs::Permissions::from_mode(0o700))
                .expect("make claim heartbeat directory private");
        }
        let path = directory.join("42.json");
        fs::write(
            &path,
            format!(
                "{{\"issue\":\"{issue}\",\"branch\":{branch:?},\"step\":\"claimed\",\"ts\":{},\"ttl_seconds\":10800,\"pid\":{pid},\"nonce\":\"{}\",\"host\":{host:?},\"boot_id\":{boot_id:?},\"process_start\":\"{process_start}\",\"pr\":\"\",\"repo\":\"{repo}\",\"worker_id\":{worker_id:?},\"claim_id\":{claim_id:?}}}\n",
                timestamp,
                sha256_hex(&nonce_frame),
            ),
        )
        .expect("seed claim heartbeat");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .expect("make claim heartbeat private");
    }

    fn copy_claim_ref_to(&self, remote: &Path) {
        let seeded = git_fixture(
            &self.claim_repo,
            &[
                "ls-remote",
                "--refs",
                self.claim_remote.to_str().expect("claim remote"),
                "refs/autospec/claims/issue-42",
            ],
        );
        let seeded = seeded.split_whitespace().next().expect("seeded claim oid");
        git_fixture(
            &self.claim_repo,
            &[
                "push",
                remote.to_str().expect("target claim remote"),
                &format!("{seeded}:refs/autospec/claims/issue-42"),
            ],
        );
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
            "[{{\"number\":17,\"body\":\"Closes #42\\n\\n## Closeout report\\n\\nResult: shipped\",\"headRefName\":\"autonomous/issue-42\",\"headRefOid\":\"{commit}\",\"isDraft\":false,\"baseRefName\":\"main\"}}]"
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
        self.claim_record_from(&self.claim_remote)
    }

    fn claim_record_from(&self, remote: &Path) -> RunStateRecord {
        let message = git_fixture(
            &self.root,
            &[
                "--git-dir",
                remote.to_str().expect("claim remote path"),
                "show",
                "-s",
                "--format=%B",
                "refs/autospec/claims/issue-42",
            ],
        );
        parse_run_state_comment(&message).expect("parse authoritative claim ledger record")
    }

    fn state_path(&self) -> PathBuf {
        self.operator
            .join("test_repo")
            .join("foreground-conductor-repository.json")
    }

    fn resilience_state_path(&self) -> PathBuf {
        self.state
            .join("autonomous")
            .join("test__repo")
            .join("state.json")
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
        self.terminate_recorded_conductor();
        let _ = fs::remove_dir_all(&self.root);
    }
}

static TEMP_DIR_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn snapshot_tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, path: &Path, snapshot: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(path).expect("read fixture tree") {
            let entry = entry.expect("read fixture entry");
            let path = entry.path();
            if path.is_dir() {
                visit(root, &path, snapshot);
            } else {
                snapshot.insert(
                    path.strip_prefix(root)
                        .expect("fixture entry below root")
                        .to_path_buf(),
                    fs::read(path).expect("read fixture file"),
                );
            }
        }
    }

    let mut snapshot = BTreeMap::new();
    visit(root, root, &mut snapshot);
    snapshot
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Debug, Eq, PartialEq)]
struct NativeProcessIdentity {
    pgid: u32,
    start_time_ticks: u64,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn assert_authoritative_conductor_metadata(path: &Path, pid: u32) {
    let metadata: serde_json::Value =
        serde_json::from_slice(&fs::read(path).expect("read conductor metadata"))
            .expect("parse conductor metadata");
    let observed = native_process_identity(pid).expect("capture live conductor identity");
    assert_eq!(metadata["pid"], pid);
    assert_eq!(metadata["pgid"], observed.pgid);
    assert_eq!(metadata["start_time_ticks"], observed.start_time_ticks);
}

#[cfg(target_os = "linux")]
fn native_process_identity(pid: u32) -> Option<NativeProcessIdentity> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (_, fields) = stat.rsplit_once(") ")?;
    let fields = fields.split_whitespace().collect::<Vec<_>>();
    let pgid = u32::try_from(nix::unistd::getpgid(Some(nix::unistd::Pid::from_raw(
        i32::try_from(pid).ok()?,
    )))
    .ok()?
    .as_raw())
    .ok()?;
    Some(NativeProcessIdentity {
        pgid,
        start_time_ticks: fields.get(19)?.parse().ok()?,
    })
}

#[cfg(target_os = "macos")]
fn native_process_identity(pid: u32) -> Option<NativeProcessIdentity> {
    let mut process = unsafe { std::mem::zeroed::<nix::libc::proc_bsdinfo>() };
    let process_size = std::mem::size_of::<nix::libc::proc_bsdinfo>();
    if unsafe {
        nix::libc::proc_pidinfo(
            i32::try_from(pid).ok()?,
            nix::libc::PROC_PIDTBSDINFO,
            0,
            &mut process as *mut _ as *mut _,
            i32::try_from(process_size).ok()?,
        )
    } != i32::try_from(process_size).ok()?
    {
        return None;
    }
    let start_time_ticks = process
        .pbi_start_tvsec
        .checked_mul(1_000_000)?
        .checked_add(process.pbi_start_tvusec)?;
    Some(NativeProcessIdentity {
        pgid: process.pbi_pgid,
        start_time_ticks,
    })
}

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

fn git_fixture(directory: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(directory)
        .output()
        .expect("run Git fixture command");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
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
    for _ in 0..1_000 {
        if fs::read_to_string(path).is_ok_and(|contents| contents.contains(expected)) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("{} did not contain {expected}", path.display());
}

fn process_is_running(pid: u32) -> bool {
    Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .is_ok_and(|output| {
            output.status.success()
                && !String::from_utf8_lossy(&output.stdout)
                    .trim_start()
                    .starts_with('Z')
        })
}

fn process_identity(pid: u32) -> Option<(u32, u64)> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (_, fields) = stat.rsplit_once(") ")?;
    let fields = fields.split_whitespace().collect::<Vec<_>>();
    Some((fields.get(2)?.parse().ok()?, fields.get(19)?.parse().ok()?))
}

fn terminate_process_group(pid: u32) {
    let _ = Command::new("kill")
        .args(["-KILL", "--", &format!("-{pid}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

fn terminate_process(pid: u32) {
    let _ = Command::new("kill")
        .args(["-KILL", "--", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}
