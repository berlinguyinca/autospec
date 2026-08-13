// executor_bridge tests: continuation / event — 4 cases.
//
// Split out of tests.rs; see the note in that file.

use crate::commands::autonomous::executor_bridge as bridge;
use super::support_base::{git, git_stdout};
use super::support_invocation::implementation_proof_fixture;
use super::support_launch::{prepared_draft_transaction, DRAFT_ISSUE_BODY};
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::symlink;

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_pr_size_receipt_rejects_missing_stale_and_mismatched_evidence() {
    // Break caught: ready or merge trusting absent or non-exact patch-size evidence.
    for case in ["missing", "stale", "mismatch"] {
        let mut prepared = prepared_draft_transaction(&format!("pr-size-{case}"));
        let receipt = bridge::patch_size_receipt_path(&prepared.state_path);
        let admission = bridge::evaluate_patch_size_admission(
            &prepared.state,
            &prepared.proof.head_oid,
            DRAFT_ISSUE_BODY,
        )
        .expect("admission");
        if case != "missing" {
            bridge::persist_patch_size_admission(&prepared.state_path, &admission)
                .expect("receipt");
        }
        if case == "stale" {
            prepared.state.identity.base_oid = "b".repeat(40);
        } else if case == "mismatch" {
            let mut body: serde_json::Value =
                serde_json::from_str(&fs::read_to_string(&receipt).expect("receipt"))
                    .expect("receipt json");
            body["changed_lines"] = 399.into();
            fs::write(&receipt, body.to_string()).expect("tamper receipt");
        }
        prepared.state.phase = bridge::BridgePhase::DraftCreated;
        prepared.state.pr = Some(17);
        let lane = bridge::PremergeLaneIdentity::new(
            prepared.state.identity.repository.clone(),
            prepared.state.identity.issue,
            prepared.state.identity.worker_id.clone(),
            prepared.state.identity.claim_id.clone(),
            prepared.state.identity.branch.clone(),
            prepared.proof.head_oid.clone(),
        )
        .expect("lane");
        let pass = bridge::PremergeDecision::Pass {
            lane,
            evidence_digest: "evidence".into(),
        };
        fs::write(prepared.fixture.root.join("gh-calls"), "").expect("clear ledger");
        assert!(bridge::mark_exact_draft_ready(
            &prepared.state_path,
            &mut prepared.state,
            &pass,
            &prepared.adapter,
        )
        .expect_err(case)
        .contains("patch-size"));
        assert!(bridge::revalidate_merge_admission(
            &prepared.state_path,
            &prepared.state,
            &prepared.adapter,
        )
        .expect_err(case)
        .contains("patch-size"));
        assert!(fs::read_to_string(prepared.fixture.root.join("gh-calls"))
            .expect("ledger")
            .is_empty());
    }
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_continuation_event_thresholds_and_base_drift_generation() {
    // Break caught: capped or oversized exact-head work losing ordered continuation state.
    assert!(bridge::parse_closeout_criteria("Completed criteria: []").is_err());
    for (lines, unmet, expected) in [
        (319, "[\"second\",\"third\"]", None),
        (320, "[]", None),
        (320, "[\"second\",\"third\"]", Some("planned")),
        (401, "[\"second\",\"third\"]", Some("oversized_checkpoint")),
    ] {
        let (fixture, mut state, _, _) =
            implementation_proof_fixture(&format!("continuation-{lines}"));
        let state_path = fixture.root.join("state/invocation.json");
        let event_log = fixture.root.join("logs/../logs/executor.jsonl");
        let normalized_log = fixture.root.join("logs/executor.jsonl");
        fs::write(
            state.identity.worktree.join("slice.txt"),
            "changed\n".repeat(lines),
        )
        .expect("slice");
        git(&state.identity.worktree, &["add", "slice.txt"]);
        git(
            &state.identity.worktree,
            &["commit", "-m", "test: capped slice"],
        );
        let head = git_stdout(&state.identity.worktree, &["rev-parse", "HEAD"]);
        state.phase = bridge::BridgePhase::ImplementationProven;
        state.head_oid = Some(head.clone());
        let proof = bridge::ImplementationProof {
            head_oid: head.clone(),
            closeout_body: format!("## Closeout report\nResult: slice\nClaims: [verified] static slice\nProof type: static\nBefore/after: 0 to 1\nArtifacts: slice.txt; `git diff`\nScoped git status: slice.txt\nOne likely hidden failure: boundary\nCompleted criteria: [\"first\"]\nUnmet criteria: {unmet}\n"),
        };

        let checkpoint = bridge::require_continuation_checkpoint(
            &state_path,
            &event_log,
            &state,
            &proof,
            "",
            false,
        );
        if lines == 401 {
            assert!(checkpoint
                .expect_err("oversized checkpoint gate")
                .to_string()
                .contains("oversized continuation checkpoint"));
            assert!(!fixture.root.join("gh-calls").exists());
        } else {
            checkpoint.expect("checkpoint evaluation");
        }
        assert_eq!(
            git_stdout(&state.identity.worktree, &["rev-parse", "HEAD"]),
            head
        );
        let receipt_path =
            bridge::continuation_receipt_path(&state_path, &head).expect("receipt path");
        assert_eq!(receipt_path.exists(), expected.is_some());
        if let Some(status) = expected {
            let receipt =
                bridge::load_continuation_receipt(&state_path, &state).expect("typed receipt");
            assert_eq!(receipt.status.as_str(), status);
            assert_eq!(receipt.unmet, ["second", "third"]);
            if lines == 320 {
                let initial: serde_json::Value = serde_json::from_str(
                    fs::read_to_string(&event_log)
                        .expect("planned event")
                        .trim(),
                )
                .expect("planned JSON");
                assert_eq!(initial["event"], "continuation_planned");
                assert_eq!(initial["unmet"], serde_json::json!(["second", "third"]));
                assert_eq!(
                    [
                        initial["changed_lines"].as_u64(),
                        initial["raw_files"].as_u64(),
                        initial["logical_units"].as_u64()
                    ],
                    [Some(320), Some(1), Some(1)]
                );
                assert_eq!(initial["receipt_digest"], receipt.content_digest);
                assert_eq!(initial["receipt_path"], receipt_path.to_str().unwrap());
                assert_eq!(initial["base_oid"], state.identity.base_oid);
                assert_eq!(initial["head_oid"], proof.head_oid);
                assert_eq!(
                    initial["initiating_session_path"],
                    normalized_log.to_str().unwrap()
                );
                assert_eq!(
                    initial["initiating_session_digest"],
                    bridge::sha256_hex(normalized_log.to_str().unwrap().as_bytes())
                );
                let binding = initial["continuation_binding"].as_str().unwrap();
                let intent = bridge::continuation_event_marker_path(&state_path, binding, "intent");
                let intent_doc: serde_json::Value =
                    serde_json::from_slice(&fs::read(&intent).expect("event intent"))
                        .expect("intent JSON");
                assert_eq!(
                    intent_doc["initiating_session_path"],
                    initial["initiating_session_path"]
                );
                assert_eq!(
                    intent_doc["initiating_session_digest"],
                    initial["initiating_session_digest"]
                );
                let complete =
                    bridge::continuation_event_marker_path(&state_path, binding, "complete");
                let lock = bridge::continuation_event_marker_path(&state_path, binding, "lock");
                let lease =
                    bridge::acquire_continuation_event_lease(&lock).expect("first event lease");
                assert!(bridge::acquire_continuation_event_lease(&lock).is_err());
                drop(lease);
                assert!(bridge::require_continuation_checkpoint(
                    &state_path,
                    &fixture.root.join("logs/other.jsonl"),
                    &state,
                    &proof,
                    "",
                    false,
                )
                .is_err());
                fs::remove_file(&complete).expect("simulate append-before-complete");
                fs::rename(&event_log, event_log.with_extension("jsonl.1"))
                    .expect("rotate planned event");
                bridge::require_continuation_checkpoint(
                    &state_path,
                    &event_log,
                    &state,
                    &proof,
                    "",
                    false,
                )
                .expect("recover rotated event");
                bridge::require_continuation_checkpoint(
                    &state_path,
                    &event_log,
                    &state,
                    &proof,
                    "",
                    false,
                )
                .expect("second restart");
                let retained = format!(
                    "{}{}",
                    fs::read_to_string(event_log.with_extension("jsonl.1")).unwrap(),
                    fs::read_to_string(&event_log).unwrap()
                );
                assert_eq!(
                    retained
                        .matches("\"event\":\"continuation_planned\"")
                        .count(),
                    1
                );
                assert_eq!(
                    retained
                        .matches("\"event\":\"continuation_recovered\"")
                        .count(),
                    1
                );
                let complete_body = fs::read(&complete).expect("complete marker");
                fs::write(&complete, "tampered").expect("tamper complete");
                assert!(bridge::require_continuation_checkpoint(
                    &state_path,
                    &event_log,
                    &state,
                    &proof,
                    "",
                    false,
                )
                .is_err());
                fs::write(&complete, complete_body).expect("restore complete");
                fs::remove_file(&intent).expect("remove intent");
                symlink(&state_path, &intent).expect("symlink intent");
                assert!(bridge::require_continuation_checkpoint(
                    &state_path,
                    &event_log,
                    &state,
                    &proof,
                    "",
                    false,
                )
                .is_err());
                let first = fs::read(&receipt_path).expect("receipt");
                let worktree = state.identity.worktree.clone();
                state.identity.base_oid = head;
                fs::write(worktree.join("next.txt"), "next\n".repeat(320)).expect("next slice");
                git(&worktree, &["add", "next.txt"]);
                git(&worktree, &["commit", "-m", "next generation"]);
                let next_head = git_stdout(&worktree, &["rev-parse", "HEAD"]);
                state.head_oid = Some(next_head.clone());
                let next = bridge::ImplementationProof {
                    head_oid: next_head.clone(),
                    closeout_body: proof.closeout_body.clone(),
                };
                bridge::prepare_continuation_checkpoint(&state_path, &state, &next, "")
                    .expect("new generation");
                let next_path = bridge::continuation_receipt_path(&state_path, &next_head)
                    .expect("new receipt");
                let current = fs::read(&next_path).expect("current receipt");
                assert_eq!(
                    bridge::load_continuation_receipt(&state_path, &state)
                        .expect("current generation")
                        .head_oid,
                    next_head
                );
                bridge::prepare_continuation_checkpoint(&state_path, &state, &next, "")
                    .expect("new restart");
                assert!(receipt_path != next_path);
                assert_eq!(fs::read(receipt_path).expect("immutable old"), first);
                assert_eq!(fs::read(next_path).expect("reused current"), current);
            }
        }
        if lines == 401 {
            assert!(!bridge::remote_head_refs(&fixture.repo)
                .expect("remote refs")
                .contains_key(&format!("refs/heads/{}", state.identity.branch)));
        }
    }
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_continuation_event_exception_and_tamper_fail_closed() {
    // Break caught: invalid exceptions or forged receipt identity bypassing checkpoint policy.
    let (fixture, mut state, _, _) = implementation_proof_fixture("continuation-exception");
    let state_path = fixture.root.join("state/invocation.json");
    let event_log = fixture.root.join("logs/executor.jsonl");
    let migration = state.identity.worktree.join("db/migrations/001.sql");
    fs::create_dir_all(migration.parent().unwrap()).expect("migration dir");
    fs::write(
        &migration,
        format!("Generated by prisma\n{}", "changed\n".repeat(400)),
    )
    .expect("migration");
    git(&state.identity.worktree, &["add", "db/migrations/001.sql"]);
    git(
        &state.identity.worktree,
        &["commit", "-m", "test: migration"],
    );
    let head = git_stdout(&state.identity.worktree, &["rev-parse", "HEAD"]);
    state.head_oid = Some(head.clone());
    let proof = bridge::ImplementationProof {
        head_oid: head,
        closeout_body: "## Closeout report\nResult: migration\nClaims: [verified] static generated\nProof type: static\nBefore/after: 0 to 1\nArtifacts: db/migrations/001.sql; `git diff`\nScoped git status: db/migrations/001.sql\nOne likely hidden failure: generator\nCompleted criteria: []\nUnmet criteria: [\"publish\"]\n".into(),
    };
    let valid = "Guardian: skip-PR_SIZE # generated migration: prisma\n";
    assert!(!bridge::guardian_pr_size_attempt(
        "Docs mention skip-PR_SIZE."
    ));
    assert!(
        bridge::prepare_continuation_checkpoint(&state_path, &state, &proof, valid)
            .expect("valid exception")
            .is_none()
    );
    assert!(
        !bridge::continuation_receipt_path(&state_path, &proof.head_oid)
            .expect("receipt path")
            .exists()
    );
    assert!(bridge::require_continuation_checkpoint(
        &state_path,
        &event_log,
        &state,
        &proof,
        "Guardian: skip-PR_SIZE # generated migration: other\n",
        false,
    )
    .expect_err("invalid exception")
    .to_string()
    .contains("oversized continuation checkpoint"));
    let events = fs::read_to_string(&event_log).expect("oversized events");
    let oversized = events
        .find("\"event\":\"continuation_oversized_checkpoint\"")
        .expect("oversized event");
    let invalid = events
        .find("\"event\":\"continuation_invalid_exception\"")
        .expect("invalid exception event");
    assert!(oversized < invalid);
    assert!(!bridge::remote_head_refs(&fixture.repo)
        .expect("remote refs")
        .contains_key(&format!("refs/heads/{}", state.identity.branch)));
    assert!(!fixture.root.join("gh-calls").exists());

    let receipt =
        bridge::continuation_receipt_path(&state_path, &proof.head_oid).expect("receipt path");
    let mut body: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&receipt).expect("receipt")).expect("json");
    body["head_oid"] = "b".repeat(40).into();
    fs::write(&receipt, body.to_string()).expect("tamper");
    assert!(bridge::load_continuation_receipt(&state_path, &state).is_err());
    fs::remove_file(&receipt).expect("remove receipt");
    symlink(&state_path, &receipt).expect("receipt symlink");
    assert!(bridge::load_continuation_receipt(&state_path, &state).is_err());
}

#[test]
fn continuation_part_metadata_is_persisted_and_canonical() {
    let (_, mut state, _, _) = implementation_proof_fixture("continuation-part-body");
    state.umbrella = Some(42);
    state.current_child = Some(101);
    let restored = bridge::PersistedInvocation::from_json(&state.to_json().unwrap()).unwrap();
    assert_eq!(
        bridge::canonical_pull_request_body(&restored, "## Closeout report\n").unwrap(),
        "Part of #42\n\nCloses #101\n\n## Closeout report\n"
    );
    let mut invalid: serde_json::Value = serde_json::from_str(&state.to_json().unwrap()).unwrap();
    invalid["current_child"] = serde_json::Value::Null;
    assert!(bridge::PersistedInvocation::from_json(&invalid.to_string()).is_err());
    let object = invalid.as_object_mut().unwrap();
    object.remove("umbrella");
    object.remove("current_child");
    assert!(bridge::PersistedInvocation::from_json(&invalid.to_string()).is_ok());
}
