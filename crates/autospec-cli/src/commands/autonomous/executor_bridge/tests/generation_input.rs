// executor_bridge tests: generation / input — 6 cases.
//
// Split out of tests.rs; see the note in that file.

use super::super::super::premerge;
use super::super::HarnessInvocation;
use super::support_base::{git, git_stdout, test_environment, GitFixture};
use super::support_invocation::shell_invocation;
use super::support_launch::{completed_generation_bundle, run_process_generation_producer};
use crate::commands::autonomous::executor_bridge as bridge;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

#[test]
fn autonomous_executor_bridge_partial_publication_allocates_fresh_generation() {
    // Break caught: observed/QA/security/seal create-once files in an incomplete generation
    // poisoning every restart instead of becoming an immutable diagnostic attempt.
    let environment = test_environment();
    let previous_claim_override = std::env::var_os("AUTOSPEC_TEST_EXACT_EVIDENCE_CLAIM");
    std::env::set_var("AUTOSPEC_TEST_EXACT_EVIDENCE_CLAIM", "1");
    for poisoned in ["observed.json", "qa.json", "security.json", "seal.json"] {
        let fixture = GitFixture::new(&format!("partial-generation-{poisoned}"));
        fs::write(fixture.repo.join(".gitignore"), ".autospec/\n")
            .expect("ignore evidence artifacts");
        fs::create_dir_all(fixture.repo.join("tests/smoke")).expect("smoke test directory");
        fs::write(
            fixture.repo.join("tests/smoke/generation.sh"),
            "#!/bin/sh\nset -eu\ntest -f README.md\n",
        )
        .expect("smoke test fixture");
        git(
            &fixture.repo,
            &["add", ".gitignore", "tests/smoke/generation.sh"],
        );
        git(&fixture.repo, &["commit", "-m", "ignore evidence"]);
        let execution_count = fixture.root.join("execution-count");
        let scanner_root = fixture.root.join("scanner-binaries");
        environment.launch(bridge::LaunchFailpoint::BeforeEvidenceBundle);
        run_process_generation_producer(&fixture.repo, &execution_count, &scanner_root)
            .expect_err("pre-bundle failure creates incomplete attempt");
        environment.launch(bridge::LaunchFailpoint::None);
        let lane_root = fs::read_dir(fixture.repo.join(".autospec/evidence/premerge"))
            .expect("premerge root")
            .flatten()
            .find(|entry| entry.path().join("active.json").is_file())
            .expect("active lane")
            .path();
        let active: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(lane_root.join("active.json")).expect("active attempt"),
        )
        .expect("active JSON");
        let old_attempt_path = active["attempt_path"]
            .as_str()
            .expect("active attempt path")
            .to_string();
        let old_attempt = lane_root.join(&old_attempt_path);
        let poison = old_attempt.join(poisoned);
        if !poison.exists() {
            fs::write(&poison, b"partial-publication\n").expect("poison publication artifact");
            fs::set_permissions(&poison, fs::Permissions::from_mode(0o600))
                .expect("private poison artifact");
        }

        let outcome =
            run_process_generation_producer(&fixture.repo, &execution_count, &scanner_root)
                .expect("restart archives poisoned generation and reaches Pass");
        let new_active: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(lane_root.join("active.json")).expect("new active attempt"),
        )
        .expect("new active JSON");

        assert!(matches!(
            outcome.decision,
            bridge::PremergeDecision::Pass { .. }
        ));
        assert_ne!(
            new_active["attempt_path"].as_str(),
            Some(old_attempt_path.as_str()),
            "{poisoned} did not force a fresh generation"
        );
        assert!(!old_attempt.exists(), "{poisoned} attempt was reused");
        assert!(
            fs::read_dir(lane_root.join("diagnostics"))
                .expect("diagnostic generations")
                .flatten()
                .any(|entry| entry.path().join(poisoned).is_file()),
            "{poisoned} attempt was not archived intact"
        );
    }
    match previous_claim_override {
        Some(value) => std::env::set_var("AUTOSPEC_TEST_EXACT_EVIDENCE_CLAIM", value),
        None => std::env::remove_var("AUTOSPEC_TEST_EXACT_EVIDENCE_CLAIM"),
    }
}

#[test]
fn autonomous_executor_bridge_partial_generation_rotation_resumes_crash_boundaries() {
    // Break caught: moving a poisoned generation before durably switching active.json leaves
    // restart free to recreate the old run identity or lose the diagnostic generation.
    let environment = test_environment();
    let previous_claim_override = std::env::var_os("AUTOSPEC_TEST_EXACT_EVIDENCE_CLAIM");
    std::env::set_var("AUTOSPEC_TEST_EXACT_EVIDENCE_CLAIM", "1");
    for boundary in [
        bridge::LaunchFailpoint::RotationAfterArchive,
        bridge::LaunchFailpoint::RotationAfterActive,
    ] {
        let fixture = GitFixture::new(&format!("generation-rotation-{boundary:?}"));
        fs::write(fixture.repo.join(".gitignore"), ".autospec/\n")
            .expect("ignore evidence artifacts");
        fs::create_dir_all(fixture.repo.join("tests/smoke")).expect("smoke test directory");
        fs::write(
            fixture.repo.join("tests/smoke/generation.sh"),
            "#!/bin/sh\nset -eu\ntest -f README.md\n",
        )
        .expect("smoke test fixture");
        git(
            &fixture.repo,
            &["add", ".gitignore", "tests/smoke/generation.sh"],
        );
        git(&fixture.repo, &["commit", "-m", "ignore evidence"]);
        let execution_count = fixture.root.join("execution-count");
        let scanner_root = fixture.root.join("scanner-binaries");
        environment.launch(bridge::LaunchFailpoint::BeforeEvidenceBundle);
        run_process_generation_producer(&fixture.repo, &execution_count, &scanner_root)
            .expect_err("pre-bundle failure creates incomplete attempt");
        environment.launch(bridge::LaunchFailpoint::None);
        let lane_root = fs::read_dir(fixture.repo.join(".autospec/evidence/premerge"))
            .expect("premerge root")
            .flatten()
            .find(|entry| entry.path().join("active.json").is_file())
            .expect("active lane")
            .path();
        let old_active: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(lane_root.join("active.json")).expect("old active"),
        )
        .expect("old active JSON");
        let old_attempt_path = old_active["attempt_path"]
            .as_str()
            .expect("old attempt path")
            .to_string();
        let old_attempt = lane_root.join(&old_attempt_path);
        let old_intent: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(old_attempt.join("intent.json")).expect("old intent"),
        )
        .expect("old intent JSON");
        fs::write(old_attempt.join("observed.json"), b"partial-publication\n")
            .expect("poison publication");
        fs::set_permissions(
            old_attempt.join("observed.json"),
            fs::Permissions::from_mode(0o600),
        )
        .expect("private poison");

        environment.launch(boundary);
        run_process_generation_producer(&fixture.repo, &execution_count, &scanner_root)
            .expect_err("rotation boundary interrupts transaction");
        environment.launch(bridge::LaunchFailpoint::None);
        assert!(
            lane_root.join("rotation.pending.json").is_file(),
            "rotation intent must survive {boundary:?}"
        );
        assert!(!old_attempt.exists(), "old attempt survived {boundary:?}");
        let interrupted_active: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(lane_root.join("active.json")).expect("interrupted active"),
        )
        .expect("interrupted active JSON");
        if boundary == bridge::LaunchFailpoint::RotationAfterArchive {
            assert_eq!(
                interrupted_active["attempt_path"].as_str(),
                Some(old_attempt_path.as_str())
            );
        } else {
            assert_ne!(
                interrupted_active["attempt_path"].as_str(),
                Some(old_attempt_path.as_str())
            );
        }
        let outcome =
            run_process_generation_producer(&fixture.repo, &execution_count, &scanner_root)
                .expect("restart resumes rotation and reaches Pass");
        let new_active: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(lane_root.join("active.json")).expect("new active"),
        )
        .expect("new active JSON");
        let new_attempt_path = new_active["attempt_path"]
            .as_str()
            .expect("new attempt path");
        let new_intent: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(lane_root.join(new_attempt_path).join("intent.json"))
                .expect("new intent"),
        )
        .expect("new intent JSON");

        assert!(matches!(
            outcome.decision,
            bridge::PremergeDecision::Pass { .. }
        ));
        assert_ne!(new_attempt_path, old_attempt_path);
        assert_ne!(new_intent["run_id"], old_intent["run_id"]);
        assert!(!lane_root.join("rotation.pending.json").exists());
        assert!(fs::read_dir(lane_root.join("diagnostics"))
            .expect("diagnostic generations")
            .flatten()
            .any(|entry| entry.path().join("observed.json").is_file()));
    }
    match previous_claim_override {
        Some(value) => std::env::set_var("AUTOSPEC_TEST_EXACT_EVIDENCE_CLAIM", value),
        None => std::env::remove_var("AUTOSPEC_TEST_EXACT_EVIDENCE_CLAIM"),
    }
}

#[test]
fn autonomous_executor_bridge_rotation_rebases_changed_input_after_crash() {
    let environment = test_environment();
    // Break caught: a transparently reprovisioned runtime reusing the stale replacement
    // generation reserved by a rotation that began under the previous runtime input.
    for boundary in [
        bridge::LaunchFailpoint::RotationAfterArchive,
        bridge::LaunchFailpoint::RotationAfterActive,
    ] {
        let fixture = GitFixture::new(&format!("rotation-rebase-{boundary:?}"));
        let lane_root = fixture.root.join("lane");
        bridge::ensure_private_directory(&lane_root).expect("lane root");
        bridge::ensure_private_directory(&lane_root.join("attempts")).expect("attempts root");
        let old_base = autospec_core::autonomous::waterfall::sha256_hex(b"runtime-session-old");
        let current_base =
            autospec_core::autonomous::waterfall::sha256_hex(b"runtime-session-current");
        let old_attempt_path = format!("attempts/{}", &old_base[..24]);
        let old_attempt = lane_root.join(&old_attempt_path);
        bridge::ensure_private_directory(&old_attempt).expect("old attempt");
        fs::write(old_attempt.join("observed.json"), b"partial\n").expect("partial publication");
        fs::set_permissions(
            old_attempt.join("observed.json"),
            fs::Permissions::from_mode(0o600),
        )
        .expect("private partial publication");
        let active = serde_json::json!({
            "schema": 2,
            "attempt_path": old_attempt_path,
            "input_digest": old_base,
            "base_input_digest": old_base,
            "intent_digest": "old",
            "runtime_session_id": "runtime-session-old",
        })
        .to_string();
        bridge::write_private_atomic(
            &lane_root.join("active.json"),
            active.as_bytes(),
            "old active fixture",
        )
        .expect("old active");

        environment.launch(boundary);
        bridge::select_evidence_generation(&lane_root, &old_base)
            .expect_err("rotation crash boundary");
        environment.launch(bridge::LaunchFailpoint::None);
        let pending: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(lane_root.join("rotation.pending.json")).expect("pending rotation"),
        )
        .expect("pending rotation JSON");
        let stale_new = pending["new_input_digest"]
            .as_str()
            .expect("stale replacement digest")
            .to_string();

        environment.launch(bridge::LaunchFailpoint::EvidenceAfterGenerationSelect);
        bridge::select_evidence_generation(&lane_root, &current_base)
            .expect_err("crash after pending removal before attempt intent");
        environment.launch(bridge::LaunchFailpoint::None);
        assert!(!lane_root.join("rotation.pending.json").exists());
        let handoff_active: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(lane_root.join("active.json")).expect("handoff active"),
        )
        .expect("handoff active JSON");
        assert_eq!(
            handoff_active["base_input_digest"].as_str(),
            Some(current_base.as_str())
        );
        let handoff_generation = handoff_active["input_digest"]
            .as_str()
            .expect("handoff generation")
            .to_string();
        let selected = bridge::select_evidence_generation(&lane_root, &current_base)
            .expect("restart selects current-input handoff");
        assert_eq!(selected, handoff_generation);
        let current_attempt = lane_root.join("attempts").join(&selected[..24]);
        bridge::ensure_private_directory(&current_attempt).expect("current attempt");
        let plan = bridge::parse_direct_command_plan("/usr/bin/true").expect("current plan");
        let commands = bridge::execute_direct_plan(
            &fixture.repo,
            &plan,
            &current_attempt.join("qa"),
            None,
            Duration::from_secs(5),
        )
        .expect("run command under current generation");
        let current_active: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(lane_root.join("active.json")).expect("current active"),
        )
        .expect("current active JSON");

        assert_ne!(selected, stale_new);
        assert_eq!(
            current_active["input_digest"].as_str(),
            Some(selected.as_str())
        );
        assert_eq!(commands[0].terminal, bridge::AttemptTerminal::Exited(0));
        assert!(current_attempt.join("qa/command-000.json").is_file());
        assert!(!lane_root.join("rotation.pending.json").exists());
        assert!(fs::read_dir(lane_root.join("diagnostics"))
            .expect("diagnostics")
            .flatten()
            .any(|entry| entry.path().join("observed.json").is_file()));
    }
}

#[test]
fn autonomous_executor_bridge_empty_active_generation_rebinds_current_input() {
    let fixture = GitFixture::new("empty-active-input-rebind");
    let lane_root = fixture.root.join("lane");
    bridge::ensure_private_directory(&lane_root).expect("lane root");
    let old_base = autospec_core::autonomous::waterfall::sha256_hex(b"empty-old-runtime");
    let current_base = autospec_core::autonomous::waterfall::sha256_hex(b"empty-current-runtime");
    let active = serde_json::json!({
        "schema": 2,
        "attempt_path": format!("attempts/{}", &old_base[..24]),
        "input_digest": old_base,
        "base_input_digest": old_base,
        "intent_digest": null,
        "runtime_session_id": "empty-old-runtime",
    })
    .to_string();
    bridge::write_private_atomic(
        &lane_root.join("active.json"),
        active.as_bytes(),
        "empty old active fixture",
    )
    .expect("old active");

    let selected = bridge::select_evidence_generation(&lane_root, &current_base)
        .expect("empty stale active replaced");
    let rebound: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(lane_root.join("active.json")).expect("rebound active"),
    )
    .expect("rebound active JSON");

    assert_ne!(selected, old_base);
    assert_eq!(
        rebound["base_input_digest"].as_str(),
        Some(current_base.as_str())
    );
    assert_eq!(rebound["input_digest"].as_str(), Some(selected.as_str()));
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_input_rebind_cleans_nested_live_command() {
    let _environment = test_environment();
    let fixture = GitFixture::new("nested-live-input-rebind");
    let lane_root = fixture.root.join("lane");
    bridge::ensure_private_directory(&lane_root).expect("lane root");
    let old_base = autospec_core::autonomous::waterfall::sha256_hex(b"nested-old-runtime");
    let current_base = autospec_core::autonomous::waterfall::sha256_hex(b"nested-current-runtime");
    let old_attempt_path = format!("attempts/{}", &old_base[..24]);
    let old_attempt = lane_root.join(&old_attempt_path);
    bridge::ensure_private_directory(&lane_root.join("attempts")).expect("attempts root");
    bridge::ensure_private_directory(&old_attempt).expect("old attempt root");
    let qa_root = old_attempt.join("qa");
    bridge::ensure_private_directory(&qa_root).expect("nested QA root");
    let paths = bridge::direct_attempt_paths(&qa_root, 0);
    let attempt_id = bridge::reserve_direct_attempt_id(&paths).expect("nested attempt id");
    let invocation = shell_invocation(&fixture.repo, "exec /usr/bin/sleep 30");
    let validated = bridge::validate_invocation(
        &HarnessInvocation {
            program: invocation.program.canonicalize().expect("canonical shell"),
            args: invocation.args,
            current_dir: invocation
                .current_dir
                .canonicalize()
                .expect("canonical repo"),
            requires_mutation_snapshots: false,
        },
        &fixture.repo.canonicalize().expect("canonical fixture repo"),
    )
    .expect("validate nested harness");
    let mut argv = vec![validated.program.display().to_string()];
    argv.extend(validated.args.clone());
    let intent = bridge::direct_intent_document(
        &attempt_id,
        &bridge::git_stdout(&fixture.repo, &["rev-parse", "--verify", "HEAD^{commit}"])
            .expect("fixture commit"),
        None,
        &validated.program,
        &argv,
    );
    bridge::write_private_create_once(&paths.intent, intent.as_bytes(), "nested direct intent")
        .expect("nested intent");
    let mut child = bridge::spawn_blocked_harness(&validated, &paths.sinks, Some(&attempt_id))
        .expect("spawn nested live command");
    let supervisor_pid = child.supervisor_birth().pid;
    child
        .release_launch_barrier()
        .expect("release nested command");
    drop(child);
    let active = serde_json::json!({
        "schema": 2,
        "attempt_path": old_attempt_path,
        "input_digest": old_base,
        "base_input_digest": old_base,
        "intent_digest": "old",
        "runtime_session_id": "nested-old-runtime",
    })
    .to_string();
    bridge::write_private_atomic(
        &lane_root.join("active.json"),
        active.as_bytes(),
        "nested old active fixture",
    )
    .expect("old active");

    let selected = bridge::select_evidence_generation(&lane_root, &current_base)
        .expect("clean nested ownership and select current input");
    let new_attempt = lane_root.join("attempts").join(&selected[..24]);
    bridge::ensure_private_directory(&new_attempt).expect("new attempt");
    let plan = bridge::parse_direct_command_plan("/usr/bin/true").expect("new command plan");
    let commands = bridge::execute_direct_plan(
        &fixture.repo,
        &plan,
        &new_attempt.join("qa"),
        None,
        Duration::from_secs(5),
    )
    .expect("one new command");

    assert!(
        bridge::observe_process_birth(supervisor_pid)
            .expect("observe old supervisor")
            .is_none(),
        "old nested supervisor survived input rebind"
    );
    assert!(!old_attempt.exists());
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].terminal, bridge::AttemptTerminal::Exited(0));
    assert!(fs::read_dir(lane_root.join("diagnostics"))
        .expect("diagnostics")
        .flatten()
        .any(|entry| entry.path().join("qa/command-000.intent.json").is_file()));
}

#[test]
fn autonomous_executor_bridge_completed_marker_selects_fresh_live_generation() {
    let _environment = test_environment();
    // Break caught: a durable completed attempt permanently vetoing every later in-process
    // producer after the process that wrote it died before returning Pass.
    let fixture = GitFixture::new("completed-generation-recovery");
    let commit = git_stdout(&fixture.repo, &["rev-parse", "HEAD"]);
    let lane =
        bridge::PremergeLaneIdentity::new("test/repo", 42, "worker", "claim", "main", commit)
            .expect("lane");
    let lane_root = fixture
        .repo
        .join(".autospec/evidence/premerge")
        .join(lane.lane_digest());
    bridge::ensure_private_directory(&lane_root).expect("lane root");
    let execution_count = fixture.root.join("execution-count");

    let first = completed_generation_bundle(&fixture, &lane, &lane_root, 1, &execution_count);
    premerge::set_complete_publication_failpoint(true);
    let discarded = premerge::persist_observed_bridge_evidence(&fixture.repo, &first);
    premerge::set_complete_publication_failpoint(false);
    let discarded =
        discarded.expect_err("first process must fail after publishing its durable locator");
    assert!(
        discarded.contains("after complete marker fsync"),
        "{discarded}"
    );
    assert!(lane_root.join("complete.json").is_file());

    let second = completed_generation_bundle(&fixture, &lane, &lane_root, 2, &execution_count);
    let recovered = premerge::persist_observed_bridge_evidence(&fixture.repo, &second)
        .expect("fresh process must replace the stale locator with live evidence");

    assert!(matches!(recovered, bridge::PremergeDecision::Pass { .. }));
    assert_eq!(
        fs::read_to_string(execution_count).expect("live execution count"),
        "2",
        "disk replay must not substitute for a fresh observation"
    );
    assert!(lane_root.join("completed").is_dir());
}
