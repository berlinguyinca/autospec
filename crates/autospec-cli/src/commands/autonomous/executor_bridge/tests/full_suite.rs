// executor_bridge tests: full / suite — 10 cases.
//
// Split out of tests.rs; see the note in that file.

use super::super::super::premerge;
use super::support_base::{git, git_stdout, test_environment, GitFixture};
use super::support_invocation::{implementation_proof_fixture, session_record_ids};
use crate::commands::autonomous::executor_bridge as bridge;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::process::Command;
use std::time::{Duration, Instant};

#[test]
fn autonomous_executor_bridge_direct_runner_kills_stalls_and_bounded_output() {
    let _environment = test_environment();
    let fixture = GitFixture::new("direct-bounds");
    let stalled = bridge::parse_direct_command_plan("/usr/bin/sleep 30").expect("sleep plan");
    let started = Instant::now();
    let error = bridge::execute_direct_plan(
        &fixture.repo,
        &stalled,
        &fixture.root.join("stalled"),
        None,
        Duration::from_millis(75),
    )
    .expect_err("quiet command must hit bounded stall policy");
    assert!(error.contains("stalled"), "{error}");
    assert!(started.elapsed() < Duration::from_secs(5));
    let stalled_record: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(fixture.root.join("stalled/command-000.json"))
            .expect("stalled command record"),
    )
    .expect("typed stalled record");
    assert_eq!(stalled_record["terminal"]["kind"], "timed_out");

    let noisy = bridge::parse_direct_command_plan("/usr/bin/yes").expect("noisy plan");
    let error = bridge::execute_direct_plan(
        &fixture.repo,
        &noisy,
        &fixture.root.join("noisy"),
        None,
        Duration::from_secs(2),
    )
    .expect_err("unbounded output must be terminated");
    assert!(error.contains("exceeded 1 MiB"), "{error}");
    let noisy_record: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(fixture.root.join("noisy/command-000.json"))
            .expect("overflow command record"),
    )
    .expect("typed overflow record");
    assert_eq!(noisy_record["terminal"]["kind"], "output_overflow");
}

#[test]
fn autonomous_executor_bridge_persists_spawn_and_signal_terminal_attempts() {
    let _environment = test_environment();
    let fixture = GitFixture::new("direct-terminal-attempts");
    let missing = bridge::parse_direct_command_plan("/definitely/missing/autospec-command")
        .expect("missing executable plan");
    let error = bridge::execute_direct_plan(
        &fixture.repo,
        &missing,
        &fixture.root.join("spawn"),
        None,
        Duration::from_secs(2),
    )
    .expect_err("spawn failure must fail");
    assert!(
        error.contains("missing") || error.contains("execute"),
        "{error}"
    );
    let spawn: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(fixture.root.join("spawn/command-000.json"))
            .expect("spawn failure record"),
    )
    .expect("typed spawn failure");
    assert_eq!(spawn["terminal"]["kind"], "spawn_failed");

    let signaled = bridge::parse_direct_command_plan(
        "/usr/bin/python3 -c 'import os,signal; os.kill(os.getpid(), signal.SIGTERM)'",
    )
    .expect("signaled command plan");
    let error = bridge::execute_direct_plan(
        &fixture.repo,
        &signaled,
        &fixture.root.join("signaled"),
        None,
        Duration::from_secs(2),
    )
    .expect_err("signaled command must fail");
    assert!(error.contains("signal 15"), "{error}");
    let signal: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(fixture.root.join("signaled/command-000.json")).expect("signal record"),
    )
    .expect("typed signal failure");
    assert_eq!(signal["terminal"]["kind"], "signaled");
    assert_eq!(signal["terminal"]["signal"], 15);
}

#[test]
fn autonomous_executor_bridge_full_suite_revalidation_is_commit_bound_and_repeatable() {
    let _environment = test_environment();
    let fixture = GitFixture::new("full-suite-revalidation");
    let commit = git_stdout(&fixture.repo, &["rev-parse", "HEAD"]);
    let env = BTreeMap::from([(
        "AUTOSPEC_FULL_TEST_COMMAND".to_string(),
        OsString::from("/usr/bin/true"),
    )]);
    for pass in 0..2 {
        let artifact_root = fixture.root.join(format!("full-{pass}"));
        let observed = bridge::revalidate_full_suite(bridge::FullSuiteRevalidationRequest {
            worktree: &fixture.repo,
            issue_body: "",
            spec_documents: &[],
            env: &env,
            artifact_root: &artifact_root,
            runtime: None,
            stall_timeout: Duration::from_secs(5),
            expected_base_ref: "origin/main",
            expected_base_oid: &commit,
            expected_commit: &commit,
        })
        .expect("repeat exact full suite");
        assert_eq!(observed.len(), 1);
        assert_eq!(observed[0].commit_oid, commit);
    }

    git(
        &fixture.repo,
        &["commit", "--allow-empty", "-m", "base update"],
    );
    let artifact_root = fixture.root.join("full-drift");
    let error = bridge::revalidate_full_suite(bridge::FullSuiteRevalidationRequest {
        worktree: &fixture.repo,
        issue_body: "",
        spec_documents: &[],
        env: &env,
        artifact_root: &artifact_root,
        runtime: None,
        stall_timeout: Duration::from_secs(5),
        expected_base_ref: "origin/main",
        expected_base_oid: &commit,
        expected_commit: &commit,
    })
    .expect_err("stale revalidation commit must fail closed");
    assert!(error.contains("commit mismatch"), "{error}");
}

#[test]
fn autonomous_executor_bridge_full_suite_accepts_descendant_remote_base_only() {
    let _environment = test_environment();
    // Break caught: main advancing after draft creation stranded evidence that remained
    // correctly bound to the sealed implementation and its still-ancestral base.
    let fixture = GitFixture::new("full-suite-descendant-base");
    let commit = git_stdout(&fixture.repo, &["rev-parse", "HEAD"]);
    let env = BTreeMap::from([(
        "AUTOSPEC_FULL_TEST_COMMAND".to_string(),
        OsString::from("/usr/bin/true"),
    )]);
    fs::write(fixture.root.join("seed/base-advance.txt"), "advance\n").expect("write base advance");
    git(&fixture.root.join("seed"), &["add", "base-advance.txt"]);
    git(
        &fixture.root.join("seed"),
        &["commit", "-m", "base advance"],
    );
    git(&fixture.root.join("seed"), &["push", "origin", "main"]);

    bridge::revalidate_full_suite(bridge::FullSuiteRevalidationRequest {
        worktree: &fixture.repo,
        issue_body: "",
        spec_documents: &[],
        env: &env,
        artifact_root: &fixture.root.join("full-descendant"),
        runtime: None,
        stall_timeout: Duration::from_secs(5),
        expected_base_ref: "origin/main",
        expected_base_oid: &commit,
        expected_commit: &commit,
    })
    .expect("descendant remote base must defer to later reconciliation");

    let tree = git_stdout(&fixture.root.join("seed"), &["rev-parse", "HEAD^{tree}"]);
    let unrelated = Command::new("git")
        .args(["commit-tree", &tree, "-m", "unrelated base"])
        .current_dir(fixture.root.join("seed"))
        .output()
        .expect("create unrelated base");
    assert!(unrelated.status.success());
    let unrelated = String::from_utf8(unrelated.stdout).expect("unrelated OID");
    let force_refspec = format!("{}:refs/heads/main", unrelated.trim());
    git(
        &fixture.root.join("seed"),
        &["push", "--force", "origin", &force_refspec],
    );

    let error = bridge::revalidate_full_suite(bridge::FullSuiteRevalidationRequest {
        worktree: &fixture.repo,
        issue_body: "",
        spec_documents: &[],
        env: &env,
        artifact_root: &fixture.root.join("full-unrelated"),
        runtime: None,
        stall_timeout: Duration::from_secs(5),
        expected_base_ref: "origin/main",
        expected_base_oid: &commit,
        expected_commit: &commit,
    })
    .expect_err("unrelated remote base must fail closed");
    assert!(error.contains("unrelated"), "{error}");
}

#[test]
fn autonomous_executor_bridge_executes_smoke_through_real_runtime_session() {
    let _environment = test_environment();
    let fixture = GitFixture::new("runtime-qa-execution");
    fs::create_dir_all(fixture.repo.join(".autospec")).expect("runtime manifest directory");
    fs::write(
        fixture.repo.join("runtime-up.py"),
        "import http.server, os\npid=os.fork()\nif not pid:\n http.server.HTTPServer(('127.0.0.1', int(os.environ['AGENT_FRONTEND_PORT'])), http.server.SimpleHTTPRequestHandler).serve_forever()\nelse:\n open('runtime.pid','w').write(str(pid))\n",
    )
    .expect("runtime up executable");
    fs::write(
        fixture.repo.join("runtime-down.py"),
        "import os, signal\npid=int(open('runtime.pid').read())\nos.kill(pid, signal.SIGTERM)\nos.remove('runtime.pid')\n",
    )
    .expect("runtime down executable");
    fs::write(
        fixture.repo.join(".autospec/runtime.yml"),
        "version: 1\ndefault_mode: local\nmodes:\n  local:\n    command: python3 runtime-up.py\n    down: python3 runtime-down.py\n",
    )
    .expect("runtime manifest");
    let state_root = fixture.root.join("runtime-state");
    let previous = std::env::var_os("AGENT_ENV_STATE_ROOT");
    std::env::set_var("AGENT_ENV_STATE_ROOT", &state_root);
    let adapter = bridge::DirectRuntimeAdapter::prepare(&fixture.repo).expect("runtime adapter");
    let session_id = adapter.session_id().to_string();
    let plan = bridge::parse_direct_command_plan("/usr/bin/printf runtime-session")
        .expect("runtime smoke plan");

    let observed = bridge::execute_direct_plan(
        &fixture.repo,
        &plan,
        &fixture.root.join("runtime-evidence"),
        Some(&adapter),
        Duration::from_secs(5),
    )
    .expect("execute through real runtime session");

    assert_eq!(
        fs::read(&observed[0].stdout_path).expect("runtime stdout"),
        b"runtime-session"
    );
    assert_eq!(
        observed[0].runtime_session_id.as_deref(),
        Some(session_id.as_str())
    );
    assert_eq!(session_record_ids(&state_root), vec![session_id]);
    adapter.close_verified().expect("verified close");
    assert!(session_record_ids(&state_root).is_empty());
    match previous {
        Some(value) => std::env::set_var("AGENT_ENV_STATE_ROOT", value),
        None => std::env::remove_var("AGENT_ENV_STATE_ROOT"),
    }
}

#[test]
fn autonomous_executor_bridge_detects_each_supported_ecosystem_full_suite() {
    let node = GitFixture::new("ecosystem-node");
    fs::write(
        node.repo.join("package.json"),
        r#"{"scripts":{"lint":"eslint .","typecheck":"tsc --noEmit","test":"vitest","build":"vite build"}}"#,
    )
    .expect("node manifest");
    let suite =
        bridge::resolve_full_suite(&node.repo, "", &[], &BTreeMap::new()).expect("node suite");
    assert_eq!(suite.plan.commands.len(), 4);

    let maven = GitFixture::new("ecosystem-maven");
    fs::write(maven.repo.join("pom.xml"), "<project/>").expect("maven manifest");
    let suite =
        bridge::resolve_full_suite(&maven.repo, "", &[], &BTreeMap::new()).expect("maven suite");
    assert_eq!(
        suite.plan.commands[0].argv,
        vec!["mvn", "--batch-mode", "verify"]
    );

    let gradle = GitFixture::new("ecosystem-gradle");
    fs::write(gradle.repo.join("build.gradle.kts"), "plugins {}").expect("gradle manifest");
    let suite =
        bridge::resolve_full_suite(&gradle.repo, "", &[], &BTreeMap::new()).expect("gradle suite");
    assert_eq!(
        suite.plan.commands[0].argv,
        vec!["gradle", "check", "build"]
    );

    let go = GitFixture::new("ecosystem-go");
    fs::write(
        go.repo.join("go.mod"),
        "module example.invalid/test\n\ngo 1.22\n",
    )
    .expect("go manifest");
    let suite = bridge::resolve_full_suite(&go.repo, "", &[], &BTreeMap::new()).expect("go suite");
    assert_eq!(suite.plan.commands.len(), 3);

    let python = GitFixture::new("ecosystem-python");
    fs::write(
        python.repo.join("pyproject.toml"),
        "[build-system]\nrequires=[]\n[tool.ruff]\n[tool.mypy]\n[tool.pytest.ini_options]\n",
    )
    .expect("python manifest");
    let suite = bridge::resolve_full_suite(&python.repo, "", &[], &BTreeMap::new())
        .expect("complete Python suite");
    assert_eq!(suite.plan.commands.len(), 4);
}

#[test]
fn autonomous_executor_bridge_inventories_mixed_and_nested_ecosystems_before_fallback() {
    let mixed = GitFixture::new("ecosystem-mixed");
    fs::write(
        mixed.repo.join("Cargo.toml"),
        "[package]\nname='fixture'\nversion='0.1.0'\n",
    )
    .expect("cargo manifest");
    fs::write(
        mixed.repo.join("pyproject.toml"),
        "[project]\nname='fixture'\n",
    )
    .expect("python manifest");
    let error = bridge::resolve_full_suite(&mixed.repo, "", &[], &BTreeMap::new())
        .expect_err("Cargo must not hide unsupported Python verification");
    assert!(error.contains("lint dimension"), "{error}");

    let nested = GitFixture::new("ecosystem-nested");
    fs::write(
        nested.repo.join("Cargo.toml"),
        "[package]\nname='fixture'\nversion='0.1.0'\n",
    )
    .expect("cargo manifest");
    fs::create_dir_all(nested.repo.join("tools/web")).expect("nested manifest directory");
    fs::write(
        nested.repo.join("tools/web/package.json"),
        r#"{"scripts":{"lint":"x","typecheck":"x","test":"x","build":"x"}}"#,
    )
    .expect("nested package manifest");
    let error = bridge::resolve_full_suite(&nested.repo, "", &[], &BTreeMap::new())
        .expect_err("root Cargo must not hide nested package area");
    assert!(error.contains("tools/web/package.json"), "{error}");
}

#[test]
fn autonomous_executor_bridge_ignores_next_build_manifest() {
    let fixture = GitFixture::new("ecosystem-next-build");
    fs::write(
        fixture.repo.join("package.json"),
        r#"{"scripts":{"lint":"eslint .","typecheck":"tsc --noEmit","test":"node test.mjs","build":"next build"}}"#,
    )
    .expect("root package manifest");
    fs::create_dir_all(fixture.repo.join(".next")).expect("Next.js build directory");
    fs::write(fixture.repo.join(".next/package.json"), "{}\n")
        .expect("generated Next.js package manifest");

    let suite = bridge::resolve_full_suite(&fixture.repo, "", &[], &BTreeMap::new())
        .expect("generated Next.js manifest must not require operator verification");

    assert_eq!(suite.plan.commands.len(), 4);
}

#[test]
fn autonomous_executor_bridge_exact_lane_cleanliness_excludes_only_owned_evidence() {
    let (fixture, state, _snapshot, _closeout) = implementation_proof_fixture("exact-lane-clean");
    let artifact_root = state
        .identity
        .worktree
        .join(".autospec/evidence/premerge/lane");
    fs::create_dir_all(&artifact_root).expect("owned evidence directory");
    fs::write(artifact_root.join("attempt.json"), "{}\n").expect("owned evidence");
    bridge::verify_clean_evidence_worktree(&state, &artifact_root)
        .expect("owned evidence is excluded");

    fs::write(state.identity.worktree.join("unowned.tmp"), "drift").expect("unowned artifact");
    let untracked = bridge::verify_clean_evidence_worktree(&state, &artifact_root)
        .expect_err("unowned untracked file must block");
    assert!(untracked.contains("unowned.tmp"), "{untracked}");
    fs::remove_file(state.identity.worktree.join("unowned.tmp")).expect("remove unowned artifact");

    fs::write(state.identity.worktree.join("README.md"), "tracked drift").expect("tracked drift");
    let tracked = bridge::verify_clean_evidence_worktree(&state, &artifact_root)
        .expect_err("tracked mutation must block");
    assert!(tracked.contains("README.md"), "{tracked}");
    drop(fixture);
}

#[test]
fn autonomous_executor_bridge_cleanup_failure_cannot_publish_complete_marker() {
    let fixture = GitFixture::new("cleanup-before-complete");
    let commit = git_stdout(&fixture.repo, &["rev-parse", "HEAD"]);
    let lane =
        bridge::PremergeLaneIdentity::new("test/repo", 42, "worker", "claim", "main", commit)
            .expect("lane");
    let artifact_root = fixture
        .repo
        .join(".autospec/evidence/premerge")
        .join(lane.lane_digest());
    bridge::ensure_private_directory(&artifact_root).expect("artifact root");
    let manifest_body = "{}".to_string();
    bridge::write_private_create_once(
        &artifact_root.join("observed.json"),
        manifest_body.as_bytes(),
        "test observed manifest",
    )
    .expect("observed manifest");
    let bundle = bridge::ObservedEvidenceBundle {
        qa: bridge::QaEvidence {
            lane: lane.clone(),
            run_id: "qa".into(),
            completed_at: 1,
            verdict: bridge::EvidenceVerdict::Pass,
        },
        security: bridge::SecurityAuditEvidence {
            lane,
            run_id: "security".into(),
            completed_at: 1,
            verdict: bridge::EvidenceVerdict::Pass,
        },
        artifact_root: artifact_root.clone(),
        attempt_root: artifact_root.clone(),
        intent_digest: "intent".into(),
        manifest_digest: autospec_core::autonomous::waterfall::sha256_hex(manifest_body.as_bytes()),
        manifest_body,
        runtime_session_id: Some("failed-cleanup-session".into()),
        runtime_environment_dir: Some(fixture.root.join("failed-runtime")),
        cleanup_digest: None,
    };

    let error = premerge::persist_observed_bridge_evidence(&fixture.repo, &bundle)
        .expect_err("unverified cleanup must prevent publication");

    assert!(error.contains("cleanup"), "{error}");
    assert!(!artifact_root.join("complete.json").exists());
    assert!(!artifact_root.join("qa.json").exists());
    assert!(!artifact_root.join("security.json").exists());
}
