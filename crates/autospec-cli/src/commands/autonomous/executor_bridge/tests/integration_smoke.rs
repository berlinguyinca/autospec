// executor_bridge tests: integration-smoke review evidence.

use super::support_base::{git, GitFixture};
use super::support_invocation::supervision_state;
use crate::commands::autonomous::executor_bridge as bridge;
use autospec_core::autonomous::review_policy::{
    classify_review_requirements, ReviewPolicyInput, ReviewRequirements, ReviewRisk,
};
use std::fs;
use std::time::Duration;

fn sealed_integration_review_fixture(
    name: &str,
) -> (
    GitFixture,
    bridge::PersistedInvocation,
    ReviewRequirements,
    bridge::ExecutorReviewInventory,
    std::path::PathBuf,
) {
    let fixture = GitFixture::new(name);
    let mut state = supervision_state(&fixture);
    let commit = super::support_base::git_stdout(&fixture.repo, &["rev-parse", "HEAD"]);
    state.head_oid = Some(commit.clone());
    let requirements = integration_requirements();
    let inventory = bridge::ExecutorReviewInventory {
        changed_paths: vec!["crates/example/src/event_producer.rs".to_string()],
        logical_components: vec!["crates/example".to_string()],
        producer_surfaces: vec!["crates/example/src/event_producer.rs".to_string()],
        consumer_surfaces: vec!["crates/example/src/event_consumer.rs".to_string()],
    };
    let lane = bridge::PremergeLaneIdentity::new(
        state.identity.repository.clone(),
        state.identity.issue,
        state.identity.worker_id.clone(),
        state.identity.claim_id.clone(),
        state.identity.branch.clone(),
        commit,
    )
    .expect("review lane");
    let lane_root = state
        .identity
        .worktree
        .join(".autospec/evidence/premerge")
        .join(lane.lane_digest());
    let attempt_relative = "attempts/aaaaaaaaaaaaaaaaaaaaaaaa";
    let attempt_root = lane_root.join(attempt_relative);
    let record_relative = "qa/integration/command-000.json";
    let record_path = attempt_root.join(record_relative);
    bridge::ensure_private_directory(record_path.parent().expect("record parent"))
        .expect("private record directory");
    let record_body = b"{\"schema\":1,\"terminal\":\"exited:0\"}";
    bridge::write_private_create_once(&record_path, record_body, "integration record")
        .expect("integration record");
    let requirements_digest = bridge::canonical_review_requirements_digest(&requirements);
    let evidence_digest = autospec_core::autonomous::waterfall::sha256_hex(
        format!(
            "{}\0{}",
            requirements_digest,
            autospec_core::autonomous::waterfall::sha256_hex(record_body)
        )
        .as_bytes(),
    );
    let intent_digest = autospec_core::autonomous::waterfall::sha256_hex(b"intent");
    let observed = serde_json::json!({
        "schema": 2,
        "lane_digest": lane.lane_digest(),
        "base_oid": state.identity.base_oid,
        "intent_digest": intent_digest,
        "qa_run_id": "qa",
        "security_run_id": "security",
        "review_requirements_digest": requirements_digest,
        "integration_evidence_digest": evidence_digest,
        "integration_records": [record_relative],
        "qa_records": [],
        "scanners": [],
        "artifacts": [],
    })
    .to_string();
    bridge::write_private_create_once(
        &attempt_root.join("observed.json"),
        observed.as_bytes(),
        "observed integration evidence",
    )
    .expect("observed integration evidence");
    let seal = serde_json::json!({
        "schema": 1,
        "lane_digest": lane.lane_digest(),
        "intent_digest": intent_digest,
        "manifest_digest": autospec_core::autonomous::waterfall::sha256_hex(observed.as_bytes()),
        "cleanup_digest": autospec_core::autonomous::waterfall::sha256_hex(b"cleanup"),
        "qa_digest": autospec_core::autonomous::waterfall::sha256_hex(b"qa"),
        "security_digest": autospec_core::autonomous::waterfall::sha256_hex(b"security"),
    })
    .to_string();
    bridge::write_private_create_once(
        &attempt_root.join("seal.json"),
        seal.as_bytes(),
        "integration evidence seal",
    )
    .expect("integration evidence seal");
    let complete = serde_json::json!({
        "schema": 2,
        "lane_digest": lane.lane_digest(),
        "attempt_path": attempt_relative,
        "generation": "aaaaaaaaaaaaaaaaaaaaaaaa",
        "seal_digest": autospec_core::autonomous::waterfall::sha256_hex(seal.as_bytes()),
    })
    .to_string();
    bridge::write_private_create_once(
        &lane_root.join("complete.json"),
        complete.as_bytes(),
        "completed integration evidence",
    )
    .expect("completed integration evidence");
    (fixture, state, requirements, inventory, record_path)
}

fn integration_requirements() -> ReviewRequirements {
    classify_review_requirements(&ReviewPolicyInput {
        serialization_reasons: vec!["priority:high".to_string()],
        ..ReviewPolicyInput::default()
    })
}

#[test]
fn executor_review_classifies_a_producer_consumer_boundary_without_other_risk_signals() {
    // Break caught: executor classification permanently disabling the producer/consumer rule.
    let fixture = GitFixture::new("producer-consumer-review-risk");
    let producer = fixture.repo.join("crates/example/src/event_producer.rs");
    let consumer = fixture.repo.join("crates/example/src/event_consumer.rs");
    fs::create_dir_all(producer.parent().expect("producer parent")).expect("component directory");
    fs::write(&producer, "pub fn publish() {}\n").expect("producer source");
    fs::write(&consumer, "pub fn consume() {}\n").expect("consumer source");
    git(&fixture.repo, &["add", "."]);
    git(
        &fixture.repo,
        &["commit", "-m", "feat: connect example boundary"],
    );
    let state = supervision_state(&fixture);
    let request = super::support_invocation::reviewer_request(
        &state,
        fixture.root.join("state/invocation.json"),
    );

    let requirements = bridge::classify_executor_review_requirements(&request, &state)
        .expect("executor review requirements");

    assert_eq!(requirements.risk, ReviewRisk::Integration);
    assert_eq!(
        requirements.reasons,
        ["boundary:producer-consumer".to_string()]
    );
    assert!(requirements.require_integration_smoke);
}

#[test]
fn integration_shaped_issue_without_integration_smoke_fails_before_review() {
    let body = "### Primary smoke test (inner loop)\n\n```bash\n/usr/bin/true\n```\n";

    let error = bridge::parse_required_integration_smoke(body, &integration_requirements())
        .expect_err("integration-shaped work must carry integration evidence");

    assert!(error.contains("integration smoke"), "{error}");
}

#[test]
fn duplicate_integration_smoke_headings_fail_as_ambiguous() {
    let body = concat!(
        "### Primary smoke test (inner loop)\n\n```bash\n/usr/bin/true\n```\n\n",
        "### Integration smoke test (pre-merge)\n\n```bash\n/usr/bin/true\n```\n\n",
        "### Integration smoke test (pre-merge)\n\n```bash\n/usr/bin/true\n```\n",
    );

    let error = bridge::parse_required_integration_smoke(body, &integration_requirements())
        .expect_err("duplicate integration smoke headings must fail closed");

    assert!(error.contains("exactly one"), "{error}");
}

#[test]
fn integration_primary_smoke_rejects_a_noop_with_a_test_shaped_argument() {
    let body = "### Primary smoke test (inner loop)\n\n```bash\n/usr/bin/true tests/integration/review-policy.rs\n```\n";

    let error = bridge::parse_required_integration_smoke(body, &integration_requirements())
        .expect_err("a successful no-op does not exercise integration behavior");

    assert!(error.contains("integration smoke"), "{error}");
}

#[test]
fn integration_primary_smoke_accepts_a_repository_integration_test() {
    let body = "### Primary smoke test (inner loop)\n\n```bash\nbash tests/integration/review-policy.sh\n```\n";

    let plan = bridge::parse_required_integration_smoke(body, &integration_requirements())
        .expect("qualifying primary compatibility smoke")
        .expect("integration smoke plan");

    assert_eq!(
        plan.commands[0].argv,
        vec!["bash", "tests/integration/review-policy.sh"]
    );
}

#[test]
fn failing_integration_smoke_blocks_ci_passed_transition() {
    let fixture = GitFixture::new("integration-smoke-failure");
    let mut state = supervision_state(&fixture);
    state.phase = bridge::BridgePhase::DraftCreated;
    let script = fixture.repo.join("tests/integration/fail.sh");
    fs::create_dir_all(script.parent().expect("integration directory"))
        .expect("integration directory");
    fs::write(&script, "#!/usr/bin/env bash\nexit 1\n").expect("failing integration test");
    let body =
        "### Integration smoke test (pre-merge)\n\n```bash\nbash tests/integration/fail.sh\n```\n";
    let plan = bridge::parse_required_integration_smoke(body, &integration_requirements())
        .expect("strict integration smoke")
        .expect("required plan");

    let error = bridge::execute_required_integration_smoke(
        &state,
        &plan,
        &fixture.root.join("integration-failure"),
        None,
        Duration::from_secs(5),
    )
    .expect_err("failing integration evidence must block admission");

    assert!(error.contains("exit status 1"), "{error}");
    assert_eq!(state.phase, bridge::BridgePhase::DraftCreated);
}

#[test]
fn passing_integration_smoke_is_bound_into_premerge_evidence() {
    let fixture = GitFixture::new("integration-smoke-binding");
    let mut state = supervision_state(&fixture);
    state.phase = bridge::BridgePhase::DraftCreated;
    let requirements = integration_requirements();
    let script = fixture.repo.join("tests/integration/pass.sh");
    fs::create_dir_all(script.parent().expect("integration directory"))
        .expect("integration directory");
    fs::write(&script, "#!/usr/bin/env bash\nexit 0\n").expect("passing integration test");
    let body =
        "### Integration smoke test (pre-merge)\n\n```bash\nbash tests/integration/pass.sh\n```\n";
    let plan = bridge::parse_required_integration_smoke(body, &requirements)
        .expect("strict integration smoke")
        .expect("required plan");
    let artifact_root = fixture.root.join("integration-pass");
    let observations = bridge::execute_required_integration_smoke(
        &state,
        &plan,
        &artifact_root,
        None,
        Duration::from_secs(5),
    )
    .expect("passing integration evidence");

    let binding =
        bridge::bind_integration_smoke_evidence(&requirements, &artifact_root, &observations)
            .expect("commit-bound integration evidence");

    assert!(bridge::canonical_sha256(&binding.requirements_digest));
    assert!(bridge::canonical_sha256(&binding.evidence_digest));
    assert_eq!(binding.command_records.len(), 1);
    assert!(binding.command_records[0].ends_with("command-000.json"));
}

#[test]
fn sealed_integration_evidence_loads_exact_review_citations() {
    // Break caught: reviewer context dropping the immutable premerge evidence identity.
    let (_fixture, state, requirements, inventory, _) =
        sealed_integration_review_fixture("sealed-integration-review");

    let evidence = bridge::load_bound_review_evidence(&state, &requirements, inventory)
        .expect("sealed review evidence");

    assert_eq!(evidence.commit, state.head_oid.expect("review head"));
    assert_eq!(evidence.integration_command_records.len(), 1);
    let citations = evidence.integration_citations();
    assert_eq!(citations.len(), 3);
    assert!(citations[0].starts_with("requirements-digest:"));
    assert!(citations[1].starts_with("integration-evidence-digest:"));
    assert!(citations[2].starts_with("integration-record:.autospec/evidence/premerge/"));
    assert!(citations[2].ends_with("qa/integration/command-000.json"));
}

#[test]
fn sealed_integration_evidence_rejects_record_tampering() {
    // Break caught: review admission trusting a sealed manifest after its command record changed.
    let (_fixture, state, requirements, inventory, record_path) =
        sealed_integration_review_fixture("tampered-integration-review");
    fs::write(record_path, b"{\"schema\":1,\"terminal\":\"exited:1\"}")
        .expect("tamper integration record");

    let error = bridge::load_bound_review_evidence(&state, &requirements, inventory)
        .expect_err("tampered integration record must fail closed");

    assert!(
        error.contains("integration evidence digest mismatch"),
        "{error}"
    );
}
