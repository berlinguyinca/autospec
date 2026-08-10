use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
#[cfg(unix)]
use std::net::{TcpListener, TcpStream};
#[cfg(unix)]
use std::os::unix::fs::{symlink, PermissionsExt};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use autospec_core::runtime_env::{EnvironmentLifecycle, EnvironmentOwner};
#[cfg(target_os = "linux")]
use nix::sys::signal::Signal;

use super::{
    build_implementer_prompt, provision_issue_worktree, recover_invocation, resolve_base,
    runtime_session_adapter, supervise_harness, validate_trusted_ownership,
    write_invocation_atomic, BridgeIdentity, BridgePhase, ExecutorBridgeRequest, HarnessConfig,
    HarnessInvocation, HarnessKind, MutationSnapshot, PersistedInvocation, ProcessIdentity,
    ResolvedBase, SupervisionConfig, SupervisionOutcome, CLAUDE_BUILTIN_TOOLS,
    CLAUDE_FORBIDDEN_TOOLS, CLAUDE_LOCAL_TOOLS,
};

mod support_base;
use support_base::*;
mod support_invocation;
use support_invocation::*;
mod support_launch;
use support_launch::*;
mod attempt_generation;
mod worktree_post;
mod license_checker;
mod descendant_spawn;
mod full_suite;
mod reviewer_runtime;
mod draft_release;
mod json_identity;
mod quarantine_nested;
mod dispatcher_temporary;
mod generation_input;
mod cleanup_reap;
mod closeout_repairs;
mod snapshot_identity;
mod branch_predecessor;
mod runtime_fixture;
mod terminal_label;
mod reviewer_automatic;
mod prunable_zero;
mod codex_permission;
mod identity_reviewer;
mod repair_implementation;
mod closeout_harness;
mod pull_mutation;
mod scope_root;
mod production_entry;
mod commit_rust;
mod codex_sandbox;
mod closeout_remote;
mod sidecar_launch;
mod restart_direct;
mod adoption_cleanup;
mod ready_harness;
mod rust_commit;
mod result_reviewer;
mod remote_base;
mod sync_integration;
mod harness_supervisor;
mod continuation_event;
mod cleanup_restart;
mod ordered_publication;
mod proxy_direct;
mod attempt_retirement;
mod npm_inputs;
mod base_merge;
mod runtime_close;
mod harness_death;

#[test]
fn autonomous_executor_bridge_restart_reruns_all_scanner_results() {
    let _environment = TEST_ENVIRONMENT.lock().expect("test environment lock");
    // Break caught: successful scanner records recovered from disk becoming authoritative
    // security evidence without re-executing each scanner in the current process.
    let fixture = GitFixture::new("scanner-recovery");
    let bin = fixture.root.join("scanner-bin");
    fs::create_dir_all(&bin).expect("scanner bin");
    let mut paths = BTreeMap::new();
    for (scanner, output) in [
        ("gitleaks", "[]"),
        (
            "semgrep",
            r#"{"results":[],"errors":[],"paths":{"scanned":["feature.js"],"skipped":[]}}"#,
        ),
        ("trivy", r#"{"Results":[{"Target":"."}]}"#),
        ("license-checker", r#"{"fixture@1.0.0":{"licenses":"MIT"}}"#),
    ] {
        let executable = bin.join(scanner);
        let count = bin.join(format!("{scanner}.count"));
        let result_logic = if scanner == "gitleaks" {
            format!(
                "report=''\nwhile [ \"$#\" -gt 0 ]; do if [ \"$1\" = --report-path ]; then report=\"$2\"; shift 2; else shift; fi; done\nprintf '%s' '{}' > \"$report\"\n",
                output
            )
        } else {
            format!("printf '%s' '{}'\n", output)
        };
        write_executable(
            &executable,
            &format!(
                "#!/bin/sh\nset -eu\nn=0\n[ ! -f '{}' ] || n=$(cat '{}')\nn=$((n+1))\nprintf '%s' \"$n\" > '{}'\n{}",
                count.display(),
                count.display(),
                count.display(),
                result_logic
            ),
        );
        paths.insert(scanner.to_string(), executable);
    }
    let scanners = super::ScannerExecutables::from_paths(paths).expect("scanner paths");
    let artifact_root = fixture.root.join("scanner-evidence");

    let first = super::run_required_scanners(
        &fixture.repo,
        &git_stdout(&fixture.repo, &["rev-parse", "HEAD"]),
        &artifact_root,
        &scanners,
        None,
        Duration::from_secs(5),
    )
    .expect("first scanner pass");
    let second = super::run_required_scanners(
        &fixture.repo,
        &git_stdout(&fixture.repo, &["rev-parse", "HEAD"]),
        &artifact_root,
        &scanners,
        None,
        Duration::from_secs(5),
    )
    .expect("adopt scanner pass");

    assert_eq!(first.len(), second.len());
    for scanner in ["gitleaks", "semgrep", "trivy", "license-checker"] {
        assert_eq!(
            fs::read_to_string(bin.join(format!("{scanner}.count"))).expect("scanner count"),
            "2",
            "{scanner} did not rerun after its durable terminal record"
        );
    }
}

#[test]
fn autonomous_executor_bridge_policy_digest_prevents_command_replay() {
    // Break caught: an identical scanner argv recovering a terminal record created under
    // different generated-policy content.
    let fixture = GitFixture::new("scanner-policy-command-identity");
    let artifact_root = fixture.root.join("command-evidence");
    let command = |digest: &str| {
        let mut command = super::DirectCommand::success(vec!["/usr/bin/true".to_string()]);
        command.identity_digest = Some(digest.to_string());
        super::DirectCommandPlan {
            commands: vec![command],
        }
    };

    super::execute_direct_plan(
        &fixture.repo,
        &command(&"a".repeat(64)),
        &artifact_root,
        None,
        Duration::from_secs(5),
    )
    .expect("first policy-bound command");
    let error = super::execute_direct_plan(
        &fixture.repo,
        &command(&"b".repeat(64)),
        &artifact_root,
        None,
        Duration::from_secs(5),
    )
    .expect_err("different policy digest must not replay the old terminal record");

    assert!(error.contains("invocation intent"), "{error}");
}

#[test]
fn autonomous_executor_bridge_scanner_argv_is_direct_and_fail_closed() {
    let worktree = Path::new("/safe/worktree");
    let config = Path::new("/safe/evidence/gitleaks-policy.toml");
    let report = Path::new("/safe/evidence/gitleaks.json");
    let expected = [
        (
            "gitleaks",
            vec![
                "/scanner/gitleaks",
                "detect",
                "--no-git",
                "--no-banner",
                "--redact",
                "--source",
                "/safe/worktree",
                "--config",
                "/safe/evidence/gitleaks-policy.toml",
                "--report-format",
                "json",
                "--report-path",
                "/safe/evidence/gitleaks.json",
            ],
        ),
        (
            "semgrep",
            vec![
                "/scanner/semgrep",
                "scan",
                "--config",
                "p/default",
                "--metrics",
                "off",
                "--error",
                "--json",
                "--verbose",
                "--max-target-bytes",
                "0",
                "--timeout",
                "0",
                "--timeout-threshold",
                "0",
                "--baseline-commit",
                "base-oid",
                "/safe/worktree",
            ],
        ),
        (
            "trivy",
            vec![
                "/scanner/trivy",
                "fs",
                "--quiet",
                "--format",
                "json",
                "--exit-code",
                "1",
                "/safe/worktree",
            ],
        ),
        (
            "license-checker",
            vec![
                "/scanner/license-checker",
                "--json",
                "--production",
                "--start",
                "/safe/worktree",
            ],
        ),
    ];
    for (scanner, argv) in expected {
        assert_eq!(
            super::scanner_command(
                scanner,
                Path::new(argv[0]),
                worktree,
                "base-oid",
                config,
                report,
            )
            .expect("scanner command")
            .argv,
            argv
        );
    }
}

#[test]
fn autonomous_executor_bridge_scanner_command_semgrep_is_private_and_baseline_scoped() {
    // Break caught: `--config auto --metrics off` is rejected by Semgrep before scanning,
    // while an unscoped repository scan also blocks feature work on pre-existing findings.
    let command = super::scanner_command(
        "semgrep",
        Path::new("/scanner/semgrep"),
        Path::new("/safe/worktree"),
        "base-oid",
        Path::new("/safe/evidence/gitleaks-policy.toml"),
        Path::new("/safe/evidence/gitleaks.json"),
    )
    .expect("Semgrep command");

    assert!(
        command
            .argv
            .windows(2)
            .any(|pair| pair == ["--config", "p/default"]),
        "{:?}",
        command.argv
    );
    assert!(
        command
            .argv
            .iter()
            .any(|argument| argument == "--baseline-commit"),
        "{:?}",
        command.argv
    );
    assert!(
        command
            .argv
            .windows(2)
            .any(|pair| pair == ["--metrics", "off"]),
        "{:?}",
        command.argv
    );
    assert!(
        command
            .argv
            .windows(2)
            .any(|pair| pair == ["--max-target-bytes", "0"]),
        "{:?}",
        command.argv
    );
    assert_eq!(command.accepted_exit_codes, vec![0, 1]);
}

#[test]
fn autonomous_executor_bridge_scanner_command_semgrep_baseline_is_diff_scoped() {
    let _environment = TEST_ENVIRONMENT.lock().expect("test environment lock");
    // Break caught: a repository-wide scan blocking a feature on findings already present
    // in its claimed base commit, instead of evaluating only feature-introduced findings.
    let fixture = GitFixture::new("semgrep-baseline");
    let rule = fixture.root.join("semgrep-rule.yml");
    fs::write(
        &rule,
        r#"rules:
  - id: autospec-test-dangerous-call
languages:
  - generic
message: deterministic test finding
severity: ERROR
pattern-regex: dangerous_call
"#,
    )
    .expect("deterministic Semgrep rule");
    fs::write(fixture.repo.join("old.js"), "dangerous_call('old');\n")
        .expect("pre-existing finding");
    git(&fixture.repo, &["add", "old.js"]);
    git(&fixture.repo, &["commit", "-m", "baseline finding"]);
    let base_oid = git_stdout(&fixture.repo, &["rev-parse", "HEAD"]);
    fs::write(
        fixture.repo.join("feature.js"),
        "safe_call('feature');\n".repeat(60_000),
    )
    .expect("clean feature larger than Semgrep's default 1 MB limit");
    git(&fixture.repo, &["add", "feature.js"]);
    git(&fixture.repo, &["commit", "-m", "clean feature"]);
    let semgrep = super::resolve_direct_executable(&fixture.repo, "semgrep")
        .expect("real Semgrep")
        .program;
    let scan = |artifact: &str| {
        let mut command = super::DirectCommand::success(vec![
            semgrep.display().to_string(),
            "scan".to_string(),
            "--config".to_string(),
            rule.display().to_string(),
            "--metrics".to_string(),
            "off".to_string(),
            "--error".to_string(),
            "--json".to_string(),
            "--verbose".to_string(),
            "--max-target-bytes".to_string(),
            "0".to_string(),
            "--timeout".to_string(),
            "0".to_string(),
            "--timeout-threshold".to_string(),
            "0".to_string(),
            "--baseline-commit".to_string(),
            base_oid.clone(),
            ".".to_string(),
        ]);
        command.accepted_exit_codes = vec![0, 1];
        let observed = super::execute_direct_plan(
            &fixture.repo,
            &super::DirectCommandPlan {
                commands: vec![command],
            },
            &fixture.root.join(artifact),
            None,
            Duration::from_secs(30),
        )
        .expect("Semgrep process observation");
        let command = &observed[0];
        let stdout = fs::read(&command.stdout_path).expect("Semgrep JSON");
        let stderr = fs::read(&command.stderr_path).expect("Semgrep diagnostics");
        (
            command.exit_code().expect("Semgrep exit status"),
            stdout,
            stderr,
        )
    };

    let (exit_status, stdout, stderr) = scan("clean-scan");
    super::validate_scanner_result("semgrep", exit_status, &stdout, &stderr).unwrap_or_else(
        |error| {
            panic!(
                "pre-existing finding is outside the feature diff: {error}; {}",
                String::from_utf8_lossy(&stderr)
            )
        },
    );

    fs::write(
        fixture.repo.join("feature.js"),
        "dangerous_call('feature');\n",
    )
    .expect("new finding");
    git(&fixture.repo, &["add", "feature.js"]);
    git(
        &fixture.repo,
        &["commit", "-m", "introduce feature finding"],
    );
    let (exit_status, stdout, stderr) = scan("finding-scan");
    let error = super::validate_scanner_result("semgrep", exit_status, &stdout, &stderr)
        .expect_err("feature-introduced finding must block");
    assert!(error.contains("reported findings"), "{error}");
}

#[test]
fn autonomous_executor_bridge_command_artifact_digest_and_commit_tamper_block() {
    let fixture = GitFixture::new("command-evidence-tamper");
    let artifacts = fixture.root.join("evidence");
    let plan =
        super::parse_direct_command_plan("/usr/bin/printf exact").expect("direct command plan");
    let records = super::execute_direct_plan(
        &fixture.repo,
        &plan,
        &artifacts,
        None,
        Duration::from_secs(5),
    )
    .expect("observed command");
    super::validate_observed_command(&fixture.repo, &records[0])
        .expect("untampered observation");

    fs::write(&records[0].stdout_path, "tampered").expect("tamper command output");
    let error = super::validate_observed_command(&fixture.repo, &records[0])
        .expect_err("artifact digest tamper must fail");
    assert!(error.contains("digest"), "{error}");

    git(&fixture.repo, &["commit", "--allow-empty", "-m", "drift"]);
    let error = super::validate_observed_command(&fixture.repo, &records[0])
        .expect_err("commit drift must fail");
    assert!(error.contains("commit"), "{error}");
}

#[test]
fn autonomous_executor_bridge_observed_results_are_the_only_typed_pass_authority() {
    let fixture = GitFixture::new("typed-evidence");
    let commit = git_stdout(&fixture.repo, &["rev-parse", "HEAD"]);
    let lane = super::PremergeLaneIdentity::new(
        "test/repo",
        42,
        "worker-42",
        "claim-42",
        "main",
        commit.clone(),
    )
    .expect("typed lane");
    let mut qa = Vec::new();
    let mut scanners = Vec::new();
    for (index, scanner) in ["gitleaks", "semgrep", "trivy", "license-checker"]
        .into_iter()
        .enumerate()
    {
        let output = match scanner {
            "gitleaks" => "[]",
            "semgrep" => {
                r#"{"results":[],"errors":[],"paths":{"scanned":["feature.js"],"skipped":[]}}"#
            }
            "trivy" => r#"{"Results":[{"Target":"."}]}"#,
            "license-checker" => r#"{"fixture@1.0.0":{"licenses":"MIT"}}"#,
            _ => unreachable!(),
        };
        let plan = super::parse_direct_command_plan(&format!("/usr/bin/printf '{output}'"))
            .expect("native scanner JSON observation command");
        let records = super::execute_direct_plan(
            &fixture.repo,
            &plan,
            &fixture.root.join(format!("observed-{index}")),
            None,
            Duration::from_secs(5),
        )
        .expect("real observed process");
        if index == 0 {
            qa.push(records[0].clone());
        }
        scanners.push(super::ObservedScanner {
            name: scanner.to_string(),
            base_oid: git_stdout(&fixture.repo, &["rev-parse", "HEAD"]),
            command: records[0].clone(),
            result_path: records[0].stdout_path.clone(),
            result_digest: records[0].stdout_digest.clone(),
        });
    }

    let complete = super::typed_evidence_from_observed(
        &fixture.repo,
        &commit,
        &lane,
        Ok(&qa),
        Ok(&scanners),
        Some("PASS"),
        1_800_000_000,
    );
    assert!(matches!(complete.0.verdict, super::EvidenceVerdict::Pass));
    assert!(matches!(complete.1.verdict, super::EvidenceVerdict::Pass));

    let missing = super::typed_evidence_from_observed(
        &fixture.repo,
        &commit,
        &lane,
        Ok(&qa),
        Ok(&scanners[..3]),
        Some("PASS"),
        1_800_000_001,
    );
    assert!(
        !matches!(missing.1.verdict, super::EvidenceVerdict::Pass),
        "fabricated model Pass must not upgrade missing scanner evidence"
    );

    let failed = super::typed_evidence_from_observed(
        &fixture.repo,
        &commit,
        &lane,
        Err("full suite failed"),
        Ok(&scanners),
        Some("PASS"),
        1_800_000_002,
    );
    assert!(
        !matches!(failed.0.verdict, super::EvidenceVerdict::Pass),
        "fabricated model Pass must not upgrade failed QA evidence"
    );
    let lint_failed = super::typed_evidence_from_observed(
        &fixture.repo,
        &commit,
        &lane,
        Ok(&qa),
        Err("implementation lint failed"),
        Some("PASS"),
        1_800_000_003,
    );
    assert!(
        !matches!(lint_failed.1.verdict, super::EvidenceVerdict::Pass),
        "implementation-lint failure must block security Pass"
    );
}

#[test]
fn autonomous_executor_bridge_remote_snapshot_ignores_claim_ledger_refs() {
    let document = format!(
        "{}\trefs/heads/main\n{}\trefs/autospec/claims/issue-42\n",
        "a".repeat(40),
        "b".repeat(40)
    );

    assert_eq!(
        super::parse_bridge_remote_refs(&document).expect("claim ledger is separate authority"),
        BTreeMap::from([("refs/heads/main".to_string(), "a".repeat(40))])
    );
    assert!(
        super::parse_bridge_remote_refs(&format!(
            "{}\trefs/autospec/unowned/issue-42\n",
            "c".repeat(40)
        ))
        .is_err(),
        "only the exact claim-ledger namespace may be excluded"
    );
}

#[test]
fn autonomous_executor_bridge_requires_observed_exact_merged_state() {
    let document = r#"{
        "number":17,
        "state":"MERGED",
        "isDraft":false,
        "headRefOid":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "baseRefName":"main",
        "mergeCommit":{"oid":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}
    }"#;
    assert_eq!(
        super::parse_observed_merge(document, 17, &"a".repeat(40), "main",).expect("merged"),
        "b".repeat(40)
    );
    assert!(super::parse_observed_merge(
        &document.replace("\"MERGED\"", "\"OPEN\""),
        17,
        &"a".repeat(40),
        "main",
    )
    .is_err());
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_reconciles_merged_existing_worktree_before_stale_proof() {
    let _environment = TEST_ENVIRONMENT.lock().expect("test environment lock");
    let (fixture, mut state, _snapshot, closeout) =
        implementation_proof_fixture("merged-existing-worktree-entrypoint");
    commit_implementation(&state);
    let persisted_head = git_stdout(&state.identity.worktree, &["rev-parse", "HEAD"]);
    state.phase = super::BridgePhase::DraftCreated;
    state.pr = Some(17);
    state.head_oid = Some(persisted_head.clone());
    state.closeout_path = Some(fs::canonicalize(&closeout).expect("canonical closeout"));
    let closeout_body = fs::read_to_string(&closeout).expect("read closeout");
    state.closeout_digest = Some(super::sha256_hex(closeout_body.as_bytes()));
    state.identity.runtime_environment_dir = None;
    state.identity.runtime_session_id = None;
    state.supervisor = None;
    state.process = None;
    state.draft_process = None;
    super::record_worktree_creation_identity(
        &state.identity.repository_path,
        &state.identity.branch,
        &ResolvedBase {
            base_ref: state.identity.base_ref.clone(),
            base_oid: state.identity.base_oid.clone(),
            explore_mode: false,
        },
    )
    .expect("record worktree creation identity");
    let claimed = autospec_core::claim::RunStateRecord::new(
        state.identity.repository.clone(),
        state.identity.issue,
        state.identity.worker_id.clone(),
        "claimed",
        state.identity.branch.clone(),
        "",
        "claimed",
        Vec::new(),
        "2026-08-04T00:00:00Z",
        "2026-08-04T00:00:00Z",
        999_999,
    )
    .with_claim_id(state.identity.claim_id.clone());
    assert!(crate::commands::claim::advance_claim_ref_for_test(
        &state.identity.repository_path,
        &claimed,
    )
    .expect("seed claimed generation"));

    fs::write(
        state.identity.worktree.join("reviewer-follow-up.txt"),
        "reviewer follow-up\n",
    )
    .expect("reviewer follow-up");
    git(&state.identity.worktree, &["add", "reviewer-follow-up.txt"]);
    git(
        &state.identity.worktree,
        &["commit", "-m", "test: reviewer follow-up"],
    );
    let merged_head = git_stdout(&state.identity.worktree, &["rev-parse", "HEAD"]);

    let state_path = fixture.root.join("state/invocation.json");
    super::write_invocation_atomic(&state_path, &state).expect("persist stale draft state");
    let observation = fixture.root.join("merged-observation.json");
    fs::write(
        &observation,
        serde_json::json!({
            "number": 17,
            "state": "MERGED",
            "isDraft": false,
            "headRefName": state.identity.branch,
            "headRefOid": merged_head,
            "baseRefName": "main",
            "mergeCommit": {"oid": "b".repeat(40)},
            "body": super::canonical_pull_request_body(&state, &closeout_body).unwrap(),
        })
        .to_string(),
    )
    .expect("merged observation");
    let gh = fixture.root.join("gh");
    fs::write(
        &gh,
        format!(
            "#!/bin/sh\nset -eu\n\
             if [ \"$1 $2\" = 'pr view' ]; then cat '{}'; exit 0; fi\n\
             if [ \"$1 $2\" = 'issue view' ]; then printf '%s\\n' '{{\"labels\":[]}}'; exit 0; fi\n\
             if [ \"$1 $2\" = 'issue edit' ] || [ \"$1 $2\" = 'issue comment' ]; then exit 0; fi\n\
             if [ \"$1\" = 'api' ]; then printf '%s\\n' '[]'; exit 0; fi\n\
             exit 64\n",
            observation.display()
        ),
    )
    .expect("gh fixture");
    fs::set_permissions(&gh, fs::Permissions::from_mode(0o755)).expect("gh mode");
    let failpoint = fixture.root.join("merged-reconciliation-failpoint");
    let previous_path = std::env::var_os("PATH");
    let previous_claim = std::env::var_os("AUTOSPEC_TEST_EXACT_EVIDENCE_CLAIM");
    let previous_failpoint = std::env::var_os("AUTOSPEC_TEST_MERGED_RECONCILIATION_FAIL_ONCE");
    let previous_claim_remote = std::env::var_os("AUTOSPEC_CLAIM_GIT_REMOTE");
    let previous_claim_state = std::env::var_os("AUTOSPEC_CLAIM_GIT_STATE_DIR");
    let previous_retry_sleep = std::env::var_os("AUTOSPEC_CLAIM_RETRY_SLEEP_MS");
    std::env::set_var(
        "PATH",
        format!(
            "{}:{}",
            fixture.root.display(),
            previous_path
                .as_deref()
                .unwrap_or_default()
                .to_string_lossy()
        ),
    );
    std::env::set_var("AUTOSPEC_TEST_EXACT_EVIDENCE_CLAIM", "1");
    std::env::set_var("AUTOSPEC_TEST_MERGED_RECONCILIATION_FAIL_ONCE", &failpoint);
    std::env::set_var("AUTOSPEC_CLAIM_GIT_REMOTE", fixture.root.join("remote.git"));
    std::env::set_var(
        "AUTOSPEC_CLAIM_GIT_STATE_DIR",
        fixture.root.join("claim-state"),
    );
    std::env::set_var("AUTOSPEC_CLAIM_RETRY_SLEEP_MS", "0");
    let request = super::ExecutorBridgeRequest {
        repository: state.identity.repository.clone(),
        repository_path: state.identity.repository_path.clone(),
        issue: state.identity.issue,
        issue_title: "Retire merged executor".to_string(),
        issue_body: DRAFT_ISSUE_BODY.to_string(),
        worker_id: state.identity.worker_id.clone(),
        claim_id: state.identity.claim_id.clone(),
        invocation_id: state.identity.invocation_id.clone(),
        state_path: state_path.clone(),
        event_log: fixture.root.join("events.jsonl"),
    };

    let outcome = super::run_executor_bridge_with_codex_probe(&request, |_| {
        panic!("merged recovery must precede Codex probing")
    });
    let error = outcome.expect_err("failpoint stops after merged reconciliation");
    assert!(
        error.to_string().contains("injected executor crash"),
        "{error}"
    );
    let durable = super::PersistedInvocation::from_json(
        &fs::read_to_string(&state_path).expect("read reconciled invocation"),
    )
    .expect("parse reconciled invocation");
    assert_eq!(durable.phase, super::BridgePhase::Merged);
    assert_eq!(durable.head_oid.as_deref(), Some(merged_head.as_str()));
    assert_eq!(
        durable.terminal_result.as_deref(),
        Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
    );
    assert!(super::cleanup_record_path(&state_path, "merged-reconciliation").exists());
    assert!(
        state.identity.worktree.exists(),
        "failpoint must precede cleanup"
    );
    let receipt = super::run_executor_bridge_with_codex_probe(&request, |_| {
        panic!("merged restart must finalize before Codex probing")
    })
    .expect("restart finalizes the reconciled merge");
    let complete = super::PersistedInvocation::from_json(
        &fs::read_to_string(&state_path).expect("read completed invocation"),
    )
    .expect("parse completed invocation");

    for (key, previous) in [
        ("PATH", previous_path),
        ("AUTOSPEC_TEST_EXACT_EVIDENCE_CLAIM", previous_claim),
        (
            "AUTOSPEC_TEST_MERGED_RECONCILIATION_FAIL_ONCE",
            previous_failpoint,
        ),
        ("AUTOSPEC_CLAIM_GIT_REMOTE", previous_claim_remote),
        ("AUTOSPEC_CLAIM_GIT_STATE_DIR", previous_claim_state),
        ("AUTOSPEC_CLAIM_RETRY_SLEEP_MS", previous_retry_sleep),
    ] {
        match previous {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }

    assert!(matches!(
        receipt.status,
        super::BridgeRunStatus::Merged {
            pull_request: 17,
            ref head_oid,
            ref merge_oid,
        } if head_oid == &merged_head && merge_oid == &"b".repeat(40)
    ));
    assert_eq!(complete.phase, super::BridgePhase::Complete);
    assert!(!state.identity.worktree.exists());
    assert!(state_path.with_extension("terminal.json").is_file());
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_merged_reconciliation_is_exact_and_fail_closed() {
    let (fixture, mut state, _snapshot, _) =
        implementation_proof_fixture("merged-reconciliation-exact");
    commit_implementation(&state);
    let persisted_head = git_stdout(&state.identity.worktree, &["rev-parse", "HEAD"]);
    fs::write(
        state.identity.worktree.join("reviewer-follow-up.txt"),
        "reviewer follow-up\n",
    )
    .expect("reviewer follow-up");
    git(&state.identity.worktree, &["add", "reviewer-follow-up.txt"]);
    git(
        &state.identity.worktree,
        &["commit", "-m", "reviewer follow-up"],
    );
    let merged_head = git_stdout(&state.identity.worktree, &["rev-parse", "HEAD"]);
    state.phase = super::BridgePhase::DraftCreated;
    state.pr = Some(17);
    state.head_oid = Some(persisted_head.clone());
    state.supervisor = None;
    state.process = None;
    state.draft_process = None;
    state.umbrella = Some(42);
    state.current_child = Some(101);
    let closeout = "## Closeout report\n";
    state.closeout_digest = Some(autospec_core::autonomous::waterfall::sha256_hex(
        closeout.as_bytes(),
    ));
    let observation = fixture.root.join("merged-observation.json");
    let gh = fixture.root.join("gh-merged-reconciliation");
    fs::write(&gh, "#!/bin/sh\nset -eu\ncat \"$MERGED_OBSERVATION\"\n").expect("gh");
    fs::set_permissions(&gh, fs::Permissions::from_mode(0o755)).expect("gh mode");
    let adapter = super::DraftPrAdapter {
        gh,
        environment: BTreeMap::from([(
            "MERGED_OBSERVATION".into(),
            observation.clone().into_os_string(),
        )]),
    };
    let exact = serde_json::json!({
        "number": 17,
        "state": "MERGED",
        "isDraft": false,
        "headRefName": state.identity.branch,
        "headRefOid": merged_head,
        "baseRefName": "main",
        "mergeCommit": {"oid": "b".repeat(40)},
        "body": super::canonical_pull_request_body(&state, closeout).unwrap(),
    });
    for phase in [
        super::BridgePhase::Merged,
        super::BridgePhase::CleanupPending,
        super::BridgePhase::Complete,
    ] {
        state.phase = phase;
        for (body, accepted) in [("Closes #101", true), ("Closes #42", false)] {
            fs::write(&observation, exact.to_string().replace("Closes #101", body)).unwrap();
            assert_eq!(
                super::revalidate_live_canonical_pull_request(&state, &adapter).is_ok(),
                accepted
            );
        }
    }
    state.phase = super::BridgePhase::DraftCreated;
    fs::write(&observation, exact.to_string()).expect("exact observation");
    let state_path = fixture.root.join("state/exact.json");
    super::write_invocation_atomic(&state_path, &state).expect("persist pre-reconciliation");
    let mut reconciled = state.clone();
    assert!(super::reconcile_exact_merged_invocation_with_refresh(
        &state_path,
        &mut reconciled,
        &adapter,
        || Ok(super::BridgeClaimOwnership::Refreshed { ttl_seconds: 60 }),
    )
    .expect("exact merged reconciliation"));
    assert_eq!(reconciled.phase, super::BridgePhase::Merged);
    assert_eq!(reconciled.head_oid.as_deref(), Some(merged_head.as_str()));
    assert_eq!(
        reconciled.terminal_result.as_deref(),
        Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
    );
    super::validate_merged_reconciliation_record(&state_path, &reconciled)
        .expect("bound reconciliation record");
    let mut rebound = reconciled.clone();
    rebound.current_child = Some(102);
    assert!(super::validate_merged_reconciliation_record(&state_path, &rebound).is_err());

    for (name, mutated) in [
        ("open", serde_json::json!({"state": "OPEN"})),
        ("draft", serde_json::json!({"isDraft": true})),
        ("number", serde_json::json!({"number": 18})),
        (
            "branch",
            serde_json::json!({"headRefName": "feat/autonomous-issue-99"}),
        ),
        ("base", serde_json::json!({"baseRefName": "release"})),
        (
            "body",
            serde_json::json!({"body": format!("Closes #42\n\n{closeout}")}),
        ),
        ("head-oid", serde_json::json!({"headRefOid": "not-an-oid"})),
        (
            "merge-oid",
            serde_json::json!({"mergeCommit": {"oid": "not-an-oid"}}),
        ),
        (
            "local-head",
            serde_json::json!({"headRefOid": persisted_head}),
        ),
    ] {
        let mut document = exact.clone();
        let object = document.as_object_mut().expect("observation object");
        for (key, value) in mutated.as_object().expect("mutation object") {
            object.insert(key.clone(), value.clone());
        }
        fs::write(&observation, document.to_string()).expect("mutated observation");
        let mut candidate = state.clone();
        let candidate_path = fixture.root.join(format!("state/{name}.json"));
        let outcome = super::reconcile_exact_merged_invocation_with_refresh(
            &candidate_path,
            &mut candidate,
            &adapter,
            || Ok(super::BridgeClaimOwnership::Refreshed { ttl_seconds: 60 }),
        );
        if name == "open" {
            assert!(!outcome.expect("open PR is not terminal"));
        } else {
            assert!(outcome.is_err(), "{name} must fail closed");
        }
        assert_eq!(candidate, state, "{name} must not mutate invocation state");
        assert!(
            !candidate_path.exists(),
            "{name} must not publish terminal state"
        );
    }

    let reconciliation_path = super::cleanup_record_path(&state_path, "merged-reconciliation");
    let mut changed_record: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(&reconciliation_path).expect("read reconciliation"),
    )
    .expect("parse reconciliation");
    changed_record["persisted_head"] = serde_json::json!(state.identity.base_oid);
    fs::write(&reconciliation_path, changed_record.to_string())
        .expect("change reconciliation record");
    assert!(
        super::validate_merged_reconciliation_record(&state_path, &reconciled).is_err(),
        "changed persisted evidence must fail closed"
    );

    let base = state.identity.base_oid.clone();
    let tree = git_stdout(
        &state.identity.repository_path,
        &["rev-parse", &format!("{base}^{{tree}}")],
    );
    let divergent = git_stdout(
        &state.identity.repository_path,
        &["commit-tree", &tree, "-p", &base, "-m", "divergent head"],
    );
    git(
        &state.identity.repository_path,
        &[
            "update-ref",
            &format!("refs/heads/{}", state.identity.branch),
            &divergent,
            &merged_head,
        ],
    );
    let mut divergent_observation = exact.clone();
    divergent_observation["headRefOid"] = serde_json::json!(divergent);
    fs::write(&observation, divergent_observation.to_string()).expect("divergent observation");
    let mut nonancestor = state.clone();
    let nonancestor_path = fixture.root.join("state/nonancestor.json");
    let error = super::reconcile_exact_merged_invocation_with_refresh(
        &nonancestor_path,
        &mut nonancestor,
        &adapter,
        || Ok(super::BridgeClaimOwnership::Refreshed { ttl_seconds: 60 }),
    )
    .expect_err("non-ancestor merged head must fail closed");
    assert!(error.to_string().contains("not contained"), "{error}");
    assert_eq!(nonancestor, state);
    assert!(!nonancestor_path.exists());

    fs::write(&observation, exact.to_string()).expect("restore exact observation");
    let mut lost = state.clone();
    let lost_path = fixture.root.join("state/ownership-lost.json");
    let error = super::reconcile_exact_merged_invocation_with_refresh(
        &lost_path,
        &mut lost,
        &adapter,
        || Ok(super::BridgeClaimOwnership::Lost),
    )
    .expect_err("claim takeover must block reconciliation");
    assert!(error.to_string().contains("ownership"), "{error}");
    assert_eq!(lost, state);
    assert!(!lost_path.exists());
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_merged_reconciliation_waits_for_exact_live_process() {
    let (_fixture, mut state, _snapshot, _) =
        implementation_proof_fixture("merged-reconciliation-live-process");
    let args = vec!["30".to_string()];
    let executable = fs::canonicalize("/usr/bin/sleep").expect("sleep executable");
    let mut child = Command::new(&executable)
        .arg("30")
        .process_group(0)
        .spawn()
        .expect("spawn live executor fixture");
    let mut cleanup =
        DetachedForkedCleanup::new(child.id()).expect("arm live executor cleanup");
    let deadline = Instant::now() + Duration::from_secs(2);
    let identity = loop {
        if let Some(identity) =
            super::observe_process_identity(child.id(), &super::argv_digest(&args))
                .expect("observe live executor")
        {
            if identity.executable == executable
                && identity.argv_digest == super::argv_digest(&args)
                && identity.process_group == identity.pid
            {
                break identity;
            }
        }
        assert!(Instant::now() < deadline, "live executor was not observed");
        std::thread::sleep(Duration::from_millis(1));
    };
    cleanup.confirm_identity(identity.clone());
    state.supervisor = Some(identity);

    assert!(
        !super::executor_terminal_processes_are_quiescent(&state)
            .expect("inspect exact live process"),
        "remote terminal truth must not retire a generation while its exact process is live"
    );
    assert!(
        child
            .try_wait()
            .expect("inspect live executor fixture")
            .is_none(),
        "the quiescence gate must not mutate the live process"
    );
}
