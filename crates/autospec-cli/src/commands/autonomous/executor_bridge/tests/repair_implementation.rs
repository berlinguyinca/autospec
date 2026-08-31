// executor_bridge tests: repair / implementation — 10 cases.
//
// Split out of tests.rs; see the note in that file.

use super::super::{BridgePhase, HarnessConfig, HarnessKind, PersistedInvocation};
use super::support_base::{
    environment, git, git_stdout, installed_aliases, test_environment, test_root,
    write_alias_table, GitFixture, TEST_SEQUENCE,
};
use super::support_invocation::{
    implementation_proof_fixture, persisted_invocation, supervision_state,
};
use crate::commands::autonomous::executor_bridge as bridge;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::Duration;

#[test]
fn autonomous_executor_bridge_uses_complete_markers_and_alias_order_fallback() {
    // Break caught: CODEX_CI/OpenCode sessions and marker-free installed shells being missed.
    let root = test_root("marker-fallback");
    let table = write_alias_table(
        &root,
        "claude\tautospec-definitely-missing\t\tClaude Code\n\
         codex\tsh\t\tCodex CLI\n\
         opencode\tfalse\t\tOpenCode\n",
    );
    let mut env = environment(&table);
    env.insert("CODEX_CI".to_string(), OsString::from("1"));
    assert_eq!(
        HarnessConfig::load(&root, &env)
            .expect("load config")
            .resolve(&env)
            .expect("resolve CODEX_CI")
            .kind,
        HarnessKind::Codex
    );

    env.remove("CODEX_CI");
    env.insert(
        "OPENCODE_SESSION_ID".to_string(),
        OsString::from("session-1"),
    );
    assert_eq!(
        HarnessConfig::load(&root, &env)
            .expect("load config")
            .resolve(&env)
            .expect("resolve OpenCode session")
            .kind,
        HarnessKind::OpenCode
    );

    env.remove("OPENCODE_SESSION_ID");
    assert_eq!(
        HarnessConfig::load(&root, &env)
            .expect("load config")
            .resolve(&env)
            .expect("probe aliases in installed order")
            .kind,
        HarnessKind::Codex
    );
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_rejects_relative_and_non_executable_dispatchers() {
    // Break caught: relative configured paths or non-executable files reaching process launch.
    use std::os::unix::fs::PermissionsExt;

    let root = test_root("dispatcher-contract");
    let relative_table = write_alias_table(
        &root,
        "codex\t./relative/codex\t\tCodex CLI\n\
         claude\ttrue\t\tClaude Code\n\
         opencode\tfalse\t\tOpenCode\n",
    );
    let mut env = environment(&relative_table);
    env.insert("CODEX_THREAD_ID".to_string(), OsString::from("thread-1"));
    let error = HarnessConfig::load(&root, &env)
        .expect("load relative alias")
        .resolve(&env)
        .expect_err("relative dispatcher must fail closed");
    assert!(error.contains("absolute"), "unexpected error: {error}");

    let fixture_root = PathBuf::from(std::env::var_os("HOME").expect("HOME for test fixture"))
        .join(".autospec/test-fixtures")
        .join(format!(
            "non-executable-{}-{}",
            std::process::id(),
            TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
    fs::create_dir_all(&fixture_root).expect("create non-temporary fixture");
    let dispatcher = fixture_root.join("codex");
    fs::write(&dispatcher, "#!/bin/sh\nexit 0\n").expect("write dispatcher");
    fs::set_permissions(&dispatcher, fs::Permissions::from_mode(0o644))
        .expect("make dispatcher non-executable");
    write_alias_table(
        &root,
        &format!(
            "codex\t{}\t\tCodex CLI\nclaude\ttrue\t\tClaude Code\nopencode\tfalse\t\tOpenCode\n",
            dispatcher.display()
        ),
    );
    let error = HarnessConfig::load(&root, &env)
        .expect("load non-executable alias")
        .resolve(&env)
        .expect_err("non-executable dispatcher must fail closed");
    assert!(error.contains("executable"), "unexpected error: {error}");
    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(fixture_root);
}

#[test]
fn autonomous_executor_bridge_opencode_requires_and_uses_safe_adapter() {
    // Break caught: treating OpenCode --pure as built-in tool containment.
    let root = test_root("opencode-argv");
    let table = write_alias_table(&root, installed_aliases());
    let mut env = environment(&table);
    env.insert(
        "AUTOSPEC_HANDOFF_DISPATCHER_KIND".to_string(),
        OsString::from("opencode"),
    );
    let uncontained = HarnessConfig::load(&root, &env)
        .expect("load harness config")
        .resolve(&env)
        .expect("recognize OpenCode");

    let error = uncontained
        .invocation(
            Path::new("/safe/worktree"),
            Path::new("/safe/state/unused.txt"),
            "implement issue 42",
        )
        .expect_err("OpenCode without an adapter must fail closed");
    assert!(
        error.contains("executor_harness_uncontained"),
        "unexpected error: {error}"
    );

    env.insert(
        "AUTOSPEC_OPENCODE_CONTAINMENT_ADAPTER".to_string(),
        OsString::from("/usr/bin/true"),
    );
    let contained = HarnessConfig::load(&root, &env)
        .expect("load contained harness config")
        .resolve(&env)
        .expect("resolve contained OpenCode");
    let invocation = contained
        .invocation(
            Path::new("/safe/worktree"),
            Path::new("/safe/state/unused.txt"),
            "implement issue 42",
        )
        .expect("build contained OpenCode invocation");

    assert_eq!(
        invocation.program,
        fs::canonicalize("/usr/bin/true").expect("canonical adapter")
    );
    assert_eq!(
        invocation.args,
        vec![
            contained.executable.display().to_string(),
            "--pure".to_string(),
            "run".to_string(),
            "implement issue 42".to_string(),
        ]
    );
    assert!(!invocation.requires_mutation_snapshots);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn autonomous_executor_bridge_opencode_honors_tier_model_and_variant() {
    // Break caught: AGENTS.md defines OpenCode Tier A/B model selection but the
    // executor never passed --model/--variant, so the two-tier routing silently
    // did nothing for OpenCode.
    let root = test_root("opencode-model-variant");
    let table = write_alias_table(&root, installed_aliases());
    let mut env = environment(&table);
    env.insert(
        "AUTOSPEC_HANDOFF_DISPATCHER_KIND".to_string(),
        OsString::from("opencode"),
    );
    env.insert(
        "AUTOSPEC_OPENCODE_CONTAINMENT_ADAPTER".to_string(),
        OsString::from("/usr/bin/true"),
    );
    env.insert(
        "AUTOSPEC_OPENCODE_MODEL".to_string(),
        OsString::from("anthropic/claude-opus-5"),
    );
    env.insert(
        "AUTOSPEC_OPENCODE_VARIANT".to_string(),
        OsString::from("high"),
    );

    let resolved = HarnessConfig::load(&root, &env)
        .expect("load harness config")
        .resolve(&env)
        .expect("recognize OpenCode");

    let invocation = resolved
        .invocation(
            Path::new("/safe/worktree"),
            Path::new("/safe/state/unused.txt"),
            "implement issue 42",
        )
        .expect("build OpenCode invocation");

    assert_eq!(
        invocation.args,
        vec![
            resolved.executable.display().to_string(),
            "--pure".to_string(),
            "run".to_string(),
            "--model".to_string(),
            "anthropic/claude-opus-5".to_string(),
            "--variant".to_string(),
            "high".to_string(),
            "implement issue 42".to_string(),
        ]
    );

    // Without the env vars, the invocation keeps the harness default (no flags).
    let mut default_env = environment(&table);
    default_env.insert(
        "AUTOSPEC_HANDOFF_DISPATCHER_KIND".to_string(),
        OsString::from("opencode"),
    );
    default_env.insert(
        "AUTOSPEC_OPENCODE_CONTAINMENT_ADAPTER".to_string(),
        OsString::from("/usr/bin/true"),
    );
    let defaulted = HarnessConfig::load(&root, &default_env)
        .expect("load harness config")
        .resolve(&default_env)
        .expect("recognize OpenCode")
        .invocation(
            Path::new("/safe/worktree"),
            Path::new("/safe/state/unused.txt"),
            "implement issue 42",
        )
        .expect("build default OpenCode invocation");
    assert_eq!(
        defaulted.args,
        vec![
            defaulted.args[0].clone(),
            "--pure".to_string(),
            "run".to_string(),
            "implement issue 42".to_string(),
        ]
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn autonomous_executor_bridge_persisted_json_round_trips_strictly() {
    // Break caught: recovery accepting missing, mistyped, or silently defaulted identity data.
    let expected = persisted_invocation();
    let json = expected.to_json().expect("serialize invocation");
    let actual = PersistedInvocation::from_json(&json).expect("parse invocation");

    assert_eq!(actual, expected);
}

#[test]
fn implementation_repair_output_legacy_json_defaults_to_attempt_zero() {
    let expected = persisted_invocation();
    let mut value: serde_json::Value =
        serde_json::from_str(&expected.to_json().expect("serialize invocation"))
            .expect("parse invocation JSON");
    value
        .as_object_mut()
        .expect("invocation object")
        .remove("implementation_repair_attempt");

    let actual = PersistedInvocation::from_json(&value.to_string())
        .expect("load legacy invocation without repair attempt");

    assert_eq!(actual.implementation_repair_attempt, 0);
}

#[test]
fn implementation_repair_output_generations_are_disjoint_and_bounded() {
    let root = test_root("implementation-repair-output");
    let state_path = root.join("state/invocation.json");
    let mut state = persisted_invocation();
    let mut outputs = BTreeMap::new();
    for attempt in 0..=3 {
        state.implementation_repair_attempt = attempt;
        let paths = bridge::output_sink_paths_for_state(&state_path, &state)
            .expect("supported repair output generation");
        outputs.insert(paths.stdout.clone(), attempt);
    }

    assert_eq!(outputs.len(), 4, "repair attempts reused durable output");
    state.implementation_repair_attempt = 4;
    let error = bridge::output_sink_paths_for_state(&state_path, &state)
        .expect_err("attempt four must fail before launch");
    assert!(error.contains("exceeds 3"), "{error}");
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn implementation_repair_output_recovery_ignores_prior_attempt_exit_record() {
    let root = test_root("implementation-repair-recovery");
    let state_path = root.join("state/invocation.json");
    let mut state = persisted_invocation();
    state.supervisor = None;
    state.process = None;

    let prior =
        bridge::output_sink_paths_for_attempt(&state_path, &state.identity.invocation_id, 0)
            .expect("attempt zero output");
    let mut complete = [0_u8; 16];
    complete[..4].copy_from_slice(&0_i32.to_ne_bytes());
    complete[4..8].copy_from_slice(b"EXIT");
    complete[8..12].copy_from_slice(&0_i32.to_ne_bytes());
    complete[12..].copy_from_slice(b"DONE");
    fs::write(&prior.exit_status, complete).expect("prior durable exit");
    fs::set_permissions(&prior.exit_status, fs::Permissions::from_mode(0o600))
        .expect("private prior exit");

    state.implementation_repair_attempt = 1;
    assert!(
        !bridge::has_durable_harness_recovery_evidence(&state_path, &state)
            .expect("inspect current repair attempt"),
        "attempt zero completion must not satisfy attempt one recovery"
    );

    let current =
        bridge::output_sink_paths_for_state(&state_path, &state).expect("attempt one output");
    fs::write(&current.exit_status, complete).expect("current durable exit");
    fs::set_permissions(&current.exit_status, fs::Permissions::from_mode(0o600))
        .expect("private current exit");
    assert!(
        bridge::has_durable_harness_recovery_evidence(&state_path, &state)
            .expect("recover current repair attempt")
    );
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn implementation_lint_repair_persists_bound_evidence_and_cumulative_prompt() {
    let fixture = GitFixture::new("implementation-lint-repair");
    git(
        &fixture.repo,
        &["checkout", "-b", "feat/autonomous-issue-42"],
    );
    let mut state = supervision_state(&fixture);
    state.phase = BridgePhase::ImplementationComplete;
    let state_path = fixture.root.join("state/invocation.json");
    let closeout = fixture.repo.join(".autospec/executor-closeout.md");
    bridge::prepare_private_closeout_sink(&fixture.repo, &closeout).expect("closeout sink");
    fs::write(&closeout, "stale closeout").expect("stale closeout");
    fs::write(fixture.repo.join("implementation.txt"), "repair me\n").expect("implementation");
    git(&fixture.repo, &["add", "implementation.txt"]);
    fs::remove_file(fixture.repo.join("implementation.txt")).expect("remove staged-only file");
    let error = bridge::prepare_implementation_lint_repair(
        &state_path,
        &mut state,
        &closeout,
        &[bridge::ImplementationLintRule::Complexity],
    )
    .expect_err("staged-only bytes must be preserved");
    assert!(error.contains("differs from worktree"), "{error}");
    fs::write(fixture.repo.join("implementation.txt"), "repair me\n").expect("restore worktree");
    bridge::prepare_implementation_lint_repair(
        &state_path,
        &mut state,
        &closeout,
        &[
            bridge::ImplementationLintRule::Complexity,
            bridge::ImplementationLintRule::Security,
        ],
    )
    .expect("prepare first repair");

    assert_eq!(state.phase, BridgePhase::Interrupted);
    assert_eq!(state.implementation_repair_attempt, 1);
    assert!(fs::read_to_string(&closeout)
        .expect("cleared closeout")
        .is_empty());
    git(&fixture.repo, &["diff", "--cached", "--quiet"]);
    let prompt = bridge::implementation_repair_prompt(&state_path, &state).expect("repair prompt");
    assert!(prompt.contains("Fix COMPLEXITY:") && prompt.contains("Fix SECURITY:"));
    assert!(prompt.contains("Claim: claim-42"), "{prompt}");
    assert!(prompt.contains("MUST NOT push"), "{prompt}");
    let artifact = bridge::implementation_repair_artifact_path(&state_path, &state, 1)
        .expect("first repair artifact");
    let target = artifact.with_extension("target");
    fs::rename(&artifact, &target).expect("move repair artifact");
    symlink(&target, &artifact).expect("replace repair artifact with symlink");
    let error = bridge::implementation_repair_prompt(&state_path, &state)
        .expect_err("symlinked artifact must fail closed");
    assert!(error
        .split_whitespace()
        .any(|word| word.starts_with("symlink")));
}

#[cfg(unix)]
#[test]
fn premerge_command_failure_prepares_bound_repair() {
    // Break caught: a deterministic command failure at an already-pushed draft
    // stayed at DraftCreated and the conductor reran the same immutable head.
    let (fixture, mut state, snapshot, closeout) =
        implementation_proof_fixture("premerge-command-repair");
    super::support_invocation::commit_implementation(&state);
    let state_path = fixture.root.join("state/invocation.json");
    let proof = bridge::prove_implementation(&state_path, &mut state, &snapshot, &closeout)
        .expect("prove implementation");
    state.phase = BridgePhase::DraftCreated;
    state.pr = Some(17);
    bridge::write_invocation_atomic(&state_path, &state).expect("persist draft state");

    let plan = bridge::DirectCommandPlan {
        commands: vec![bridge::DirectCommand::success(vec![
            "/usr/bin/false".to_string()
        ])],
    };
    let lane = bridge::PremergeLaneIdentity::new(
        state.identity.repository.clone(),
        state.identity.issue,
        state.identity.worker_id.clone(),
        state.identity.claim_id.clone(),
        state.identity.branch.clone(),
        proof.head_oid.clone(),
    )
    .expect("failed command lane");
    let evidence_root = state
        .identity
        .worktree
        .join(".autospec/evidence/premerge")
        .join(lane.lane_digest())
        .join("attempts/test/qa/smoke");
    let error = bridge::execute_direct_plan(
        &state.identity.worktree,
        &plan,
        &evidence_root,
        None,
        Duration::from_secs(5),
    )
    .expect_err("false must produce durable failure evidence");
    assert_eq!(
        error,
        "executor direct command segment 0 failed with exit status 1"
    );
    let paths = bridge::direct_attempt_paths(&evidence_root, 0);
    let observation = bridge::recover_observed_command(
        &state.identity.worktree,
        &paths.record,
        &plan.commands[0],
        None,
    )
    .expect("recover exact failed command");

    assert_eq!(
        bridge::prepare_premerge_command_repair(&state_path, &mut state, &proof, &observation,)
            .expect("prepare deterministic repair"),
        bridge::PremergeCommandRepairOutcome::RetryPrepared,
    );

    assert_eq!(state.phase, BridgePhase::Interrupted);
    assert_eq!(state.implementation_repair_attempt, 1);
    assert_eq!(state.pr, Some(17));
    assert_eq!(state.head_oid.as_deref(), Some(proof.head_oid.as_str()));
    assert!(state.closeout_path.is_none());
    assert!(state.closeout_digest.is_none());
    assert!(
        fs::read_to_string(&closeout)
            .expect("cleared premerge repair closeout")
            .is_empty(),
        "the next implementer must replace the failed-head closeout"
    );
    let artifact = bridge::premerge_command_repair_artifact_path(&state_path, &state, 1)
        .expect("repair artifact path");
    let value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(artifact).expect("read repair artifact"))
            .expect("parse repair artifact");
    assert_eq!(value["claim_id"], state.identity.claim_id);
    assert_eq!(value["invocation_id"], state.identity.invocation_id);
    assert_eq!(value["branch"], state.identity.branch);
    assert_eq!(value["pull_request"], 17);
    assert_eq!(value["failed_head"], proof.head_oid);
    assert_eq!(value["record_digest"], observation.record_digest);
    assert_eq!(value["stdout_digest"], observation.stdout_digest);
    assert_eq!(value["stderr_digest"], observation.stderr_digest);
    assert!(value.get("stdout").is_none());
    assert!(value.get("stderr").is_none());

    let prompt =
        bridge::implementation_repair_prompt(&state_path, &state).expect("premerge repair prompt");
    let lines = prompt.lines().collect::<Vec<_>>();
    for expected in [
        "Deterministic premerge command repair attempt 1 of 3.".to_string(),
        format!("Failed HEAD: {}", proof.head_oid),
        format!("Record SHA-256: {}", observation.record_digest),
        format!("stdout SHA-256: {}", observation.stdout_digest),
        format!("stderr SHA-256: {}", observation.stderr_digest),
        "The authority boundary is unchanged: you MUST NOT push or mutate remote state."
            .to_string(),
    ] {
        assert!(lines.iter().any(|line| *line == expected), "{prompt}");
    }

    state.phase = BridgePhase::DraftCreated;
    state.implementation_repair_attempt = bridge::MAX_IMPLEMENTATION_REPAIR_ATTEMPTS;
    bridge::write_invocation_atomic(&state_path, &state).expect("persist exhausted draft state");
    assert_eq!(
        bridge::prepare_premerge_command_repair(&state_path, &mut state, &proof, &observation)
            .expect("seal exhausted deterministic repair"),
        bridge::PremergeCommandRepairOutcome::Exhausted,
    );
    let intent = bridge::read_failure_cleanup_intent(&state_path, &state)
        .expect("read crash-recoverable exhaustion intent");
    assert!(intent.exhausted);
    assert_eq!(intent.reason, "executor_premerge_command_repair_exhausted");
}

#[cfg(unix)]
#[test]
fn premerge_qa_nonzero_exit_propagates_typed_observation() {
    // Break caught: QA returned only a formatted String, so the caller could
    // neither distinguish deterministic rejection nor bind the durable record.
    let (fixture, mut state, snapshot, closeout) =
        implementation_proof_fixture("typed-premerge-command-repair");
    super::support_invocation::commit_implementation(&state);
    let state_path = fixture.root.join("state/invocation.json");
    let proof = bridge::prove_implementation(&state_path, &mut state, &snapshot, &closeout)
        .expect("prove implementation");
    state.phase = BridgePhase::DraftCreated;
    state.pr = Some(17);
    let lane = bridge::PremergeLaneIdentity::new(
        state.identity.repository.clone(),
        state.identity.issue,
        state.identity.worker_id.clone(),
        state.identity.claim_id.clone(),
        state.identity.branch.clone(),
        proof.head_oid.clone(),
    )
    .expect("QA lane");
    let artifact_root = state
        .identity
        .worktree
        .join(".autospec/evidence/premerge")
        .join(lane.lane_digest())
        .join("attempts/test/qa/smoke");
    let plan = bridge::DirectCommandPlan {
        commands: vec![bridge::DirectCommand::success(vec![
            "/usr/bin/false".to_string()
        ])],
    };

    let error = bridge::execute_premerge_qa_plan(
        &state.identity.worktree,
        &plan,
        &artifact_root,
        None,
        Duration::from_secs(5),
    )
    .expect_err("nonzero QA must propagate a typed repairable observation");
    let observation = error
        .premerge_command_failure()
        .expect("typed premerge command failure");
    assert_eq!(observation.commit_oid, proof.head_oid);
    assert_eq!(observation.terminal, bridge::AttemptTerminal::Exited(1));
    assert!(observation.record_path.is_file());
    assert_eq!(
        error.to_string(),
        "executor premerge QA command failed with exit status 1"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn implementation_lint_repair_exhaustion_persists_final_once_without_attempt_four_output() {
    let fixture = GitFixture::new("implementation-lint-repair-exhaustion");
    git(
        &fixture.repo,
        &["checkout", "-b", "feat/autonomous-issue-42"],
    );
    let mut state = supervision_state(&fixture);
    state.phase = BridgePhase::ImplementationComplete;
    let state_path = fixture.root.join("state/invocation.json");
    let closeout = fixture.repo.join(".autospec/executor-closeout.md");
    bridge::prepare_private_closeout_sink(&fixture.repo, &closeout).expect("closeout sink");
    fs::write(fixture.repo.join("implementation.txt"), "still blocked\n").expect("implementation");
    for (attempt, rule) in [
        bridge::ImplementationLintRule::Complexity,
        bridge::ImplementationLintRule::Security,
        bridge::ImplementationLintRule::TodoLeft,
    ]
    .into_iter()
    .enumerate()
    {
        state.phase = BridgePhase::ImplementationComplete;
        git(&fixture.repo, &["add", "implementation.txt"]);
        assert_eq!(
            bridge::prepare_implementation_lint_repair(
                &state_path,
                &mut state,
                &closeout,
                &[rule],
            )
            .expect("prepare bounded repair"),
            bridge::ImplementationLintRepairOutcome::RetryPrepared,
        );
        assert_eq!(state.implementation_repair_attempt, attempt as u32 + 1);
    }
    let prompt = bridge::implementation_repair_prompt(&state_path, &state)
        .expect("cumulative third repair prompt");
    for rule in ["COMPLEXITY", "SECURITY", "TODO_LEFT"] {
        assert!(prompt.contains(&format!("Fix {rule}:")), "{prompt}");
    }

    state.phase = BridgePhase::ImplementationComplete;
    git(&fixture.repo, &["add", "implementation.txt"]);

    for _ in 0..2 {
        assert_eq!(
            bridge::prepare_implementation_lint_repair(
                &state_path,
                &mut state,
                &closeout,
                &[bridge::ImplementationLintRule::Security],
            )
            .expect("seal exhausted repair"),
            bridge::ImplementationLintRepairOutcome::Exhausted,
        );
    }
    assert_eq!(state.phase, BridgePhase::ImplementationComplete);
    assert_eq!(
        state.implementation_repair_attempt,
        bridge::MAX_IMPLEMENTATION_REPAIR_ATTEMPTS
    );
    assert_eq!(
        git_stdout(&fixture.repo, &["diff", "--cached", "--name-only"]),
        "implementation.txt",
        "exhaustion must preserve the staged implementation"
    );
    assert!(bridge::output_sink_paths_for_attempt(
        &state_path,
        &state.identity.invocation_id,
        bridge::MAX_IMPLEMENTATION_REPAIR_ATTEMPTS + 1,
    )
    .is_err());
    let artifact = bridge::implementation_repair_artifact_path(
        &state_path,
        &state,
        bridge::MAX_IMPLEMENTATION_REPAIR_ATTEMPTS + 1,
    )
    .expect("final repair artifact");
    assert_eq!(
        bridge::read_implementation_repair_rules(
            &state_path,
            &state,
            bridge::MAX_IMPLEMENTATION_REPAIR_ATTEMPTS + 1,
        )
        .expect("read final findings"),
        vec![bridge::ImplementationLintRule::Security]
    );

    let foreign = artifact.with_extension("foreign-link");
    fs::hard_link(&artifact, &foreign).expect("create foreign hard link");
    let error = bridge::read_implementation_repair_rules(
        &state_path,
        &state,
        bridge::MAX_IMPLEMENTATION_REPAIR_ATTEMPTS + 1,
    )
    .expect_err("foreign hard link must fail closed");
    assert!(error.ends_with("foreign hard link"), "{error}");
}

#[test]
fn implementation_lint_repair_exhaustion_replays_needs_human_cleanup() {
    let environment = test_environment();
    let (fixture, mut state, _snapshot, _) =
        implementation_proof_fixture("implementation-lint-exhausted-cleanup");
    state.phase = bridge::BridgePhase::ImplementationComplete;
    state.identity.runtime_environment_dir = None;
    state.identity.runtime_session_id = None;
    let state_path = fixture.root.join("state/exhausted-lint.json");
    bridge::write_invocation_atomic(&state_path, &state).expect("persist failure state");

    environment.zero_effect_recovery(bridge::ZeroEffectRecoveryFailpoint::AfterClaimTransition);
    let interrupted = bridge::finalize_failed_executor_with_transition(
        &state_path,
        &mut state,
        None,
        None,
        true,
        true,
        "executor_implementation_lint_repair_exhausted",
        |_, disposition| {
            assert_eq!(disposition, bridge::BridgeClaimDisposition::NeedsHuman);
            Ok(bridge::BridgeClaimTransition::Transitioned)
        },
    )
    .expect_err("interrupt after needs-human transition");
    environment.zero_effect_recovery(bridge::ZeroEffectRecoveryFailpoint::None);
    assert!(interrupted.to_string().ends_with("after claim transition"));
    let durable = bridge::PersistedInvocation::from_json(
        &fs::read_to_string(&state_path).expect("read interrupted state"),
    )
    .expect("parse interrupted state");
    let intent = bridge::read_failure_cleanup_intent(&state_path, &durable)
        .expect("read exhausted cleanup intent");
    assert!(intent.exhausted);
    assert_eq!(
        intent.reason,
        "executor_implementation_lint_repair_exhausted"
    );

    bridge::finalize_failed_executor_with_transition(
        &state_path,
        &mut state,
        None,
        None,
        true,
        intent.exhausted,
        &intent.reason,
        |_, disposition| {
            assert_eq!(disposition, bridge::BridgeClaimDisposition::NeedsHuman);
            Ok(bridge::BridgeClaimTransition::Transitioned)
        },
    )
    .expect("resume exhausted terminalization");
    assert_eq!(state.phase, bridge::BridgePhase::Complete);
    assert_eq!(
        state.terminal_result.as_deref(),
        Some("needs-human:executor_implementation_lint_repair_exhausted")
    );
}
