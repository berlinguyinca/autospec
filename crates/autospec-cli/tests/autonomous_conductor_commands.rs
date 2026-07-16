use autospec_core::claim::{parse_remote_comments_json, select_run_state, RunStateRecord};
use autospec_core::coordination::{ConductorOutcome, ConductorPhase, ConductorState};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

fn workspace_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
}

#[test]
fn foreground_source_has_no_legacy_shell_authority() {
    let source =
        fs::read_to_string(workspace_root().join("crates/autospec-cli/src/commands/autonomous.rs"))
            .expect("read autonomous command source");

    for forbidden in [
        "AUTOSPEC_AUTONOMOUS_SCRIPT",
        "AUTOSPEC_AUTONOMOUS_CONDUCTOR_CMD",
        "scripts/autospec-autonomous.sh",
        "Command::new(\"bash\")",
    ] {
        assert!(
            !source.contains(forbidden),
            "foreground command retains legacy authority: {forbidden}"
        );
    }
    assert!(source.contains("executor-result"));
    assert!(source.contains("ExecutorRequest"));
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
            "awaiting_typed_implementation_executor".to_string()
        ))
    );
    assert_eq!(
        state.pause_reason(),
        Some("awaiting_typed_implementation_executor")
    );
    let calls = fs::read_to_string(&fixture.calls).expect("read GitHub calls");
    let review = calls
        .find("repos/test/repo/issues/42\n--jq")
        .expect("issue reread");
    let claim = calls.find("issue\nedit\n42").expect("claim label change");
    assert!(review < claim, "safety review must precede claim selection");
    assert!(calls.contains("executor_deferred"));

    let second = fixture.run_foreground();
    assert!(
        second.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&second.stderr)
    );
    assert_eq!(fixture.read_state(), state);
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
        .contains("executor_deferred"));
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
        "{\"repo\":\"test/repo\",\"issue\":42,\"outcome\":\"blocked\",\"reason\":\"awaiting_typed_implementation_executor\"}\n"
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
        r#"[{"number":17,"body":"Closes #42","headRefName":"autonomous/issue-42"}]"#,
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
        r#"[{"number":17,"body":"Closes #42\n\n## Closeout report\n\nResult: shipped","headRefName":"autonomous/issue-42"}]"#,
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
    assert!(fs::read_to_string(&fixture.comments)
        .expect("read executor evidence")
        .contains("<!-- autospec-executor-result:begin -->"));
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
        r#"[{"number":17,"body":"Closes #42\n\n## Closeout report\n\nResult: shipped","headRefName":"autonomous/issue-42"}]"#,
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
        r#"[{"number":17,"body":"Closes #42\n\n## Closeout report\n\nResult: shipped","headRefName":"autonomous/issue-42"}]"#,
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
        r#"[{"number":17,"body":"Closes #42\n\n## Closeout report\n\nResult: shipped","headRefName":"foreign/issue-42"}]"#,
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
        r#"[{"number":17,"body":"Closes #42\n\n## Closeout report\n\nResult: shipped","headRefName":"autonomous/issue-42"}]"#,
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
        r#"[{"number":17,"body":"Closes #42\n\n## Closeout report\n\nResult: shipped","headRefName":"autonomous/issue-42"}]"#,
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
        r#"[{"number":17,"body":"Closes #42\n\n## Closeout report\n\nResult: shipped","headRefName":"autonomous/issue-42"}]"#,
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
if [ "$1" = api ] && [ "$2" = repos/test/repo/branches/main ]; then
  printf '%s\n' '{}'
  exit 0
fi
if [ "$1" = api ] && [ "$2" = repos/test/repo/branches/missing ]; then
  exit 1
fi
if [ "$1" = api ] && [ "$2" = repos/test/repo/commits/main/status ]; then
  printf '%s\n' '{"state":"success","total_count":1,"statuses":[{"context":"ci","state":"success"}]}'
  exit 0
fi
if [ "$1" = api ]; then
  endpoint=""
  for value in "$@"; do case "$value" in repos/*) endpoint="$value" ;; esac; done
  case "$endpoint" in
    repos/test/repo/issues\?*)
      case "$endpoint" in
        *labels=in-progress-by-bot*) printf '%s\n' '{"raw_count":0,"items":[]}' ;;
        *) printf '%s' '{"raw_count":1,"items":['; issue; printf '%s\n' ']}' ;;
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
      if [ "${AUTOSPEC_FOREGROUND_STEAL_ON_OUTCOME:-0}" = 1 ] && printf '%s' "$body" | grep -q executor_deferred; then
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
            .env("AUTOSPEC_CONFIG_FILE", self.root.join("missing.yml"));
        command
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
        self.seed_claim_at(worker_id, branch, &fresh_iso_timestamp());
    }

    fn seed_claim_at(&self, worker_id: &str, branch: &str, updated_at: &str) {
        self.seed_claim_state(worker_id, branch, "claimed", updated_at);
    }

    fn seed_claim_state(&self, worker_id: &str, branch: &str, state: &str, updated_at: &str) {
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
        );
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

    fn explicit_success_command(&self) -> Command {
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
