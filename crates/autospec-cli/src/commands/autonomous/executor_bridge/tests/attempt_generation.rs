// executor_bridge tests: attempt / generation — 6 cases.
//
// Split out of tests.rs; see the note in that file.

use super::super::super::premerge;
use super::support_base::{git, git_stdout, test_environment, GitFixture};
use super::support_invocation::supervision_state;
use super::support_launch::run_process_generation_producer;
use crate::commands::autonomous::executor_bridge as bridge;
use std::collections::BTreeMap;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

#[test]
fn autonomous_executor_bridge_runtime_session_selects_stable_disjoint_attempt_generation() {
    let fixture = GitFixture::new("evidence-generations");
    let mut state = supervision_state(&fixture);
    let commit = git_stdout(&fixture.repo, &["rev-parse", "HEAD"]);
    state.identity.branch = "main".into();
    state.identity.base_oid = commit.clone();
    state.head_oid = Some(commit.clone());
    state.identity.runtime_session_id = Some("implementation-session-old".into());
    let proof = bridge::ImplementationProof {
        head_oid: commit.clone(),
        closeout_body: String::new(),
    };
    let lane = bridge::PremergeLaneIdentity::new(
        state.identity.repository.clone(),
        state.identity.issue,
        state.identity.worker_id.clone(),
        state.identity.claim_id.clone(),
        "main",
        commit,
    )
    .expect("lane");
    let scanner_paths = bridge::ScannerExecutables {
        paths: BTreeMap::new(),
    };
    let evidence_env = BTreeMap::new();
    let lane_root = fixture.root.join("lane-evidence");
    bridge::ensure_private_directory(&lane_root).expect("lane root");
    let adapter = |session_id: &str| bridge::DirectRuntimeAdapter {
        repo: fixture.repo.clone(),
        session_id: session_id.to_string(),
        environment_dir: fixture.root.join("runtime-environment"),
        session: std::cell::RefCell::new(None),
    };
    let first = adapter("session-one");
    let first_request = bridge::DeterministicEvidenceRequest {
        state: &state,
        proof: &proof,
        review_requirements: autospec_core::autonomous::review_policy::classify_review_requirements(
            &autospec_core::autonomous::review_policy::ReviewPolicyInput::default(),
        ),
        issue_body: "issue",
        spec_documents: &[],
        env: &evidence_env,
        scanners: &scanner_paths,
        artifact_root: &lane_root,
        runtime: Some(&first),
        model_output: None,
        stall_timeout: Duration::from_secs(1),
    };
    let (_, first_digest) =
        bridge::evidence_input_digests(&lane, &first_request).expect("first evidence digests");
    let first_root = lane_root.join("attempts").join(&first_digest[..24]);
    bridge::ensure_private_directory(&first_root).expect("first attempt root");
    let first_intent =
        bridge::load_or_create_evidence_intent(&first_root, &lane, &first_request, &first_digest)
            .expect("first intent");
    let adopted =
        bridge::load_or_create_evidence_intent(&first_root, &lane, &first_request, &first_digest)
            .expect("same-session intent adoption");
    assert_eq!(first_intent.digest, adopted.digest);
    assert_eq!(first_intent.completed_at, adopted.completed_at);

    let second = adapter("session-two");
    let second_request = bridge::DeterministicEvidenceRequest {
        runtime: Some(&second),
        ..first_request
    };
    let (_, second_digest) =
        bridge::evidence_input_digests(&lane, &second_request).expect("second evidence digests");
    let second_root = lane_root.join("attempts").join(&second_digest[..24]);
    bridge::ensure_private_directory(&second_root).expect("second attempt root");
    bridge::load_or_create_evidence_intent(&second_root, &lane, &second_request, &second_digest)
        .expect("new session gets a new attempt");

    assert_ne!(first_root, second_root);
    assert!(first_root.join("intent.json").is_file());
    assert!(second_root.join("intent.json").is_file());
}

#[test]
fn autonomous_executor_bridge_scanner_policy_digest_rotates_failed_evidence_once() {
    // Break caught: a failed attempt created with the immediately previous scanner schema
    // replaying after scanner argv changes, or rotating repeatedly on the same schema.
    let fixture = GitFixture::new("evidence-scanner-policy-generation");
    let mut state = supervision_state(&fixture);
    let commit = git_stdout(&fixture.repo, &["rev-parse", "HEAD"]);
    state.identity.branch = "main".into();
    state.identity.base_oid = commit.clone();
    state.head_oid = Some(commit.clone());
    let proof = bridge::ImplementationProof {
        head_oid: commit.clone(),
        closeout_body: String::new(),
    };
    let lane = bridge::PremergeLaneIdentity::new(
        state.identity.repository.clone(),
        state.identity.issue,
        state.identity.worker_id.clone(),
        state.identity.claim_id.clone(),
        "main",
        commit,
    )
    .expect("lane");
    let scanner_paths = bridge::ScannerExecutables {
        paths: BTreeMap::new(),
    };
    let evidence_env = BTreeMap::new();
    let request = bridge::DeterministicEvidenceRequest {
        state: &state,
        proof: &proof,
        review_requirements: autospec_core::autonomous::review_policy::classify_review_requirements(
            &autospec_core::autonomous::review_policy::ReviewPolicyInput::default(),
        ),
        issue_body: "issue",
        spec_documents: &[],
        env: &evidence_env,
        scanners: &scanner_paths,
        artifact_root: &fixture.root,
        runtime: None,
        model_output: None,
        stall_timeout: Duration::from_secs(1),
    };
    let gitleaks_policy_digest =
        bridge::gitleaks_policy_digest(&fixture.repo).expect("current Gitleaks policy digest");
    let previous_policy_schema =
        "gitleaks-next-v1;semgrep-p-default-baseline-complete-v2;trivy-v1;license-checker-v1";
    let previous_semantic = autospec_core::autonomous::waterfall::sha256_hex(
        format!(
            "{}\0{}\0{}\0{}\0{}\0{}\0{}",
            lane.lane_digest(),
            request.state.identity.base_ref,
            request.state.identity.base_oid,
            request.issue_body,
            request.spec_documents.join("\0"),
            gitleaks_policy_digest,
            previous_policy_schema,
        )
        .as_bytes(),
    );
    let previous_digest = autospec_core::autonomous::waterfall::sha256_hex(
        format!("{previous_semantic}\0").as_bytes(),
    );
    let (_, policy_digest) =
        bridge::evidence_input_digests(&lane, &request).expect("policy evidence digests");
    assert_ne!(
        policy_digest, previous_digest,
        "scanner policy schema must change the stable evidence input"
    );

    let lane_root = fixture.root.join("lane-evidence");
    bridge::ensure_private_directory(&lane_root).expect("lane root");
    let old_relative = format!("attempts/{}", &previous_digest[..24]);
    let old_root = lane_root.join(&old_relative);
    bridge::ensure_private_directory(&old_root).expect("old attempt");
    bridge::write_private_create_once(
        &old_root.join("intent.json"),
        b"{\"failed\":true}",
        "old failed evidence",
    )
    .expect("old failed artifact");
    let active = serde_json::json!({
        "schema": 2,
        "attempt_path": old_relative,
        "input_digest": previous_digest,
        "base_input_digest": previous_digest,
        "intent_digest": "old",
        "runtime_session_id": serde_json::Value::Null,
    })
    .to_string();
    bridge::write_private_atomic(
        &lane_root.join("active.json"),
        active.as_bytes(),
        "old active evidence",
    )
    .expect("old active marker");

    let first = bridge::select_evidence_generation(&lane_root, &policy_digest)
        .expect("policy change rotates failed evidence");
    let diagnostics = || {
        fs::read_dir(lane_root.join("diagnostics"))
            .expect("diagnostics directory")
            .count()
    };
    assert_ne!(first, previous_digest);
    assert_eq!(diagnostics(), 1);

    let second = bridge::select_evidence_generation(&lane_root, &policy_digest)
        .expect("same policy reuses rotated generation");
    assert_eq!(second, first);
    assert_eq!(diagnostics(), 1, "same policy rotated more than once");
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_generation_selection_repairs_stale_owned_attempt_before_probe() {
    // Break caught: strict artifact probing rejecting same-owner attempt directories created
    // by an older release before the production generation selector can repair them.
    let fixture = GitFixture::new("generation-select-legacy-evidence");
    let lane_root = fixture.root.join("lane");
    bridge::ensure_private_directory(&lane_root).expect("lane root");
    let old_base = autospec_core::autonomous::waterfall::sha256_hex(b"legacy-input");
    let current_base = autospec_core::autonomous::waterfall::sha256_hex(b"current-input");
    let old_relative = format!("attempts/{}", &old_base[..24]);
    let old_attempt = lane_root.join(&old_relative);
    let nested = old_attempt.join("qa/smoke");
    fs::create_dir_all(&nested).expect("legacy nested attempt");
    bridge::write_private_create_once(
        &nested.join("result.json"),
        b"{\"exit\":0}",
        "legacy evidence artifact",
    )
    .expect("legacy artifact");
    for directory in [&old_attempt, &old_attempt.join("qa"), &nested] {
        fs::set_permissions(directory, fs::Permissions::from_mode(0o775))
            .expect("make legacy attempt directory");
    }
    let active = serde_json::json!({
        "schema": 2,
        "attempt_path": old_relative,
        "input_digest": old_base,
        "base_input_digest": old_base,
        "intent_digest": "legacy",
        "runtime_session_id": serde_json::Value::Null,
    })
    .to_string();
    bridge::write_private_atomic(
        &lane_root.join("active.json"),
        active.as_bytes(),
        "legacy active evidence",
    )
    .expect("legacy active marker");

    let selected = bridge::select_evidence_generation(&lane_root, &current_base)
        .expect("repair and rotate stale owned attempt");

    assert_ne!(selected, old_base);
    assert!(!old_attempt.exists(), "stale attempt was not archived");
    let archived = fs::read_dir(lane_root.join("diagnostics"))
        .expect("diagnostic generations")
        .flatten()
        .next()
        .expect("archived legacy generation")
        .path();
    for directory in [&archived, &archived.join("qa"), &archived.join("qa/smoke")] {
        assert_eq!(
            fs::metadata(directory)
                .expect("repaired directory metadata")
                .permissions()
                .mode()
                & 0o777,
            0o700,
            "{}",
            directory.display()
        );
    }
    assert!(
        archived.join("qa/smoke/result.json").is_file(),
        "rotation lost the repaired legacy artifact"
    );
}

#[test]
fn autonomous_executor_bridge_attempt_lock_serializes_root_pointer_publication() {
    let _environment = test_environment();
    let fixture = GitFixture::new("evidence-attempt-lock");
    let lane_root = fixture.root.join("lane");
    bridge::ensure_private_directory(&lane_root).expect("lane root");
    let first = bridge::acquire_evidence_attempt_lease(&lane_root).expect("first lease");

    let error = bridge::acquire_evidence_attempt_lease(&lane_root)
        .expect_err("concurrent attempt must not acquire publication authority");
    assert!(error.contains("another evidence attempt"), "{error}");

    drop(first);
    bridge::acquire_evidence_attempt_lease(&lane_root).expect("lease is recoverable after drop");
}

#[test]
fn autonomous_executor_bridge_post_complete_process_crash_reruns_real_producer() {
    let _environment = test_environment();
    if let Some(repo) = std::env::var_os("AUTOSPEC_TEST_GENERATION_REPO") {
        let repo = PathBuf::from(repo);
        let count = PathBuf::from(
            std::env::var_os("AUTOSPEC_TEST_GENERATION_COUNT").expect("generation count"),
        );
        let scanners = PathBuf::from(
            std::env::var_os("AUTOSPEC_TEST_GENERATION_SCANNERS").expect("generation scanners"),
        );
        if std::env::var_os("AUTOSPEC_TEST_GENERATION_CRASH").is_some() {
            let terminate = std::process::exit;
            premerge::set_complete_publication_failpoint(true);
            let error = run_process_generation_producer(&repo, &count, &scanners)
                .expect_err("post-fsync failpoint must prevent returning Pass");
            if !error.contains("after complete marker fsync") {
                eprintln!("{error}");
                terminate(87);
            }
            terminate(86);
        }
        let outcome = run_process_generation_producer(&repo, &count, &scanners)
            .expect("fresh process producer reaches Pass");
        let bridge::PremergeDecision::Pass { .. } = outcome.decision else {
            panic!("fresh process producer returned a non-Pass decision");
        };
        return;
    }

    let fixture = GitFixture::new("producer-process-generation");
    fs::write(fixture.repo.join(".gitignore"), ".autospec/\n").expect("ignore evidence artifacts");
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
    let scanners = fixture.root.join("scanner-binaries");
    let executable = std::env::current_exe().expect("test executable");
    let test_name = "commands::autonomous::executor_bridge::tests::attempt_generation::autonomous_executor_bridge_post_complete_process_crash_reruns_real_producer";
    let first = Command::new(&executable)
        .args(["--exact", test_name, "--nocapture"])
        .env("AUTOSPEC_TEST_EXACT_EVIDENCE_CLAIM", "1")
        .env("AUTOSPEC_TEST_GENERATION_REPO", &fixture.repo)
        .env("AUTOSPEC_TEST_GENERATION_COUNT", &execution_count)
        .env("AUTOSPEC_TEST_GENERATION_SCANNERS", &scanners)
        .env("AUTOSPEC_TEST_GENERATION_CRASH", "1")
        .status()
        .expect("first producer process");
    assert_eq!(
        first.code(),
        Some(86),
        "first producer must die after fsync"
    );
    let second = Command::new(executable)
        .args(["--exact", test_name, "--nocapture"])
        .env("AUTOSPEC_TEST_EXACT_EVIDENCE_CLAIM", "1")
        .env("AUTOSPEC_TEST_GENERATION_REPO", &fixture.repo)
        .env("AUTOSPEC_TEST_GENERATION_COUNT", &execution_count)
        .env("AUTOSPEC_TEST_GENERATION_SCANNERS", &scanners)
        .status()
        .expect("fresh producer process");
    assert!(second.success(), "fresh producer process must return Pass");
    assert_eq!(
        fs::read_to_string(&execution_count).expect("producer execution count"),
        "2"
    );
    let completed = fixture.repo.join(".autospec/evidence/premerge");
    assert!(
        completed
            .read_dir()
            .expect("premerge roots")
            .filter_map(Result::ok)
            .any(|entry| entry.path().join("completed").is_dir()),
        "fresh producer must archive the stale completed generation"
    );
}

#[test]
fn autonomous_executor_bridge_prebundle_crash_reruns_every_diagnostic_record() {
    // Break caught: a crash after every command/scanner record but before bundle creation
    // allowing recovered disk bytes to originate Pass on restart.
    let environment = test_environment();
    let previous_claim_override = std::env::var_os("AUTOSPEC_TEST_EXACT_EVIDENCE_CLAIM");
    std::env::set_var("AUTOSPEC_TEST_EXACT_EVIDENCE_CLAIM", "1");
    let fixture = GitFixture::new("producer-prebundle-crash");
    fs::write(fixture.repo.join(".gitignore"), ".autospec/\n").expect("ignore evidence artifacts");
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
    let interrupted =
        run_process_generation_producer(&fixture.repo, &execution_count, &scanner_root);
    environment.launch(bridge::LaunchFailpoint::None);
    let interrupted = interrupted.expect_err("pre-bundle failpoint interrupts production");
    assert!(
        interrupted.contains("evidence-before-bundle"),
        "{interrupted}"
    );
    assert_eq!(
        fs::read_to_string(&execution_count).expect("first QA count"),
        "1"
    );

    let outcome = run_process_generation_producer(&fixture.repo, &execution_count, &scanner_root)
        .expect("restart reruns diagnostics and reaches Pass");
    assert!(matches!(
        outcome.decision,
        bridge::PremergeDecision::Pass { .. }
    ));
    assert_eq!(
        fs::read_to_string(&execution_count).expect("rerun QA count"),
        "2"
    );
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
    let attempt_root = lane_root.join(
        active["attempt_path"]
            .as_str()
            .expect("active attempt path"),
    );
    for scanner in ["gitleaks", "semgrep", "trivy", "license-checker"] {
        assert!(
            fs::read_dir(attempt_root.join("security").join(scanner).join("process"))
                .expect("scanner process artifacts")
                .flatten()
                .any(|entry| entry.file_name().to_string_lossy().contains(".archive-")),
            "{scanner} diagnostic record was not archived before rerun"
        );
    }
    match previous_claim_override {
        Some(value) => std::env::set_var("AUTOSPEC_TEST_EXACT_EVIDENCE_CLAIM", value),
        None => std::env::remove_var("AUTOSPEC_TEST_EXACT_EVIDENCE_CLAIM"),
    }
}
