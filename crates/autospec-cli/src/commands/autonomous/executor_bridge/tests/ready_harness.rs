// executor_bridge tests: ready / harness — 12 cases.
//
// Split out of tests.rs; see the note in that file.

use super::support_base::{git, git_stdout, test_root, write_executable, GitFixture};
use super::support_invocation::{
    commit_implementation, implementation_proof_fixture, supervision_state,
};
use super::support_launch::{adapter_path, prepared_draft_transaction};
use crate::commands::autonomous::executor_bridge as bridge;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::Duration;

#[cfg(unix)]
#[test]
fn premerge_repair_updates_same_draft_with_descendant() {
    // Break caught: repaired H1 was compared to the prelaunch empty PR inventory,
    // so Rust rejected or duplicated the already-created draft at H0.
    let mut prepared = prepared_draft_transaction("same-pr-premerge-repair");
    assert_eq!(prepared.publish().expect("publish H0 draft"), 17);
    let h0 = prepared.proof.head_oid.clone();
    let lane = bridge::PremergeLaneIdentity::new(
        prepared.state.identity.repository.clone(),
        prepared.state.identity.issue,
        prepared.state.identity.worker_id.clone(),
        prepared.state.identity.claim_id.clone(),
        prepared.state.identity.branch.clone(),
        h0.clone(),
    )
    .expect("failed H0 lane");
    let failure_root = prepared
        .state
        .identity
        .worktree
        .join(".autospec/evidence/premerge")
        .join(lane.lane_digest())
        .join("attempts/test/qa/smoke");
    let failure_plan = bridge::DirectCommandPlan {
        commands: vec![bridge::DirectCommand::success(vec![
            "/usr/bin/false".to_string()
        ])],
    };
    let failure = bridge::execute_premerge_qa_plan(
        &prepared.state.identity.worktree,
        &failure_plan,
        &failure_root,
        None,
        Duration::from_secs(5),
    )
    .expect_err("H0 QA failure");
    let observation = failure
        .premerge_command_failure()
        .expect("typed H0 failure")
        .clone();
    bridge::prepare_premerge_command_repair(
        &prepared.state_path,
        &mut prepared.state,
        &prepared.proof,
        &observation,
    )
    .expect("prepare H0 repair");

    let protected = bridge::MutationSnapshot::capture(
        &prepared.state.identity.repository_path,
        &prepared.state.identity.branch,
    )
    .expect("protected snapshot");
    fs::write(
        prepared.state.identity.worktree.join("implementation.txt"),
        "repaired implementation\n",
    )
    .expect("repair implementation");
    fs::write(
        prepared
            .state
            .identity
            .worktree
            .join(".autospec/executor-closeout.md"),
        &prepared.proof.closeout_body,
    )
    .expect("replace repair closeout");
    git(
        &prepared.state.identity.worktree,
        &["add", "implementation.txt"],
    );
    git(
        &prepared.state.identity.worktree,
        &["commit", "-m", "fix: repair premerge command"],
    );
    prepared.state.phase = bridge::BridgePhase::ImplementationComplete;
    bridge::write_invocation_atomic(&prepared.state_path, &prepared.state)
        .expect("persist repaired completion");
    let closeout = prepared
        .state
        .identity
        .worktree
        .join(".autospec/executor-closeout.md");
    let h1_proof = bridge::prove_implementation(
        &prepared.state_path,
        &mut prepared.state,
        &protected,
        &closeout,
    )
    .expect("prove repaired H1");
    assert_ne!(h1_proof.head_oid, h0);
    let body = bridge::canonical_pull_request_body(&prepared.state, &h1_proof.closeout_body)
        .expect("canonical repaired PR body");
    let updated = prepared.fixture.root.join("updated-pull-request.json");
    fs::write(
        &updated,
        format!(
            "[{{\"number\":17,\"body\":{},\"headRefName\":\"{}\",\"headRefOid\":\"{}\",\"isDraft\":true,\"baseRefName\":\"main\"}}]",
            serde_json::to_string(&body).expect("encode repaired body"),
            prepared.state.identity.branch,
            h1_proof.head_oid,
        ),
    )
    .expect("updated PR fixture");
    prepared
        .adapter
        .environment
        .insert("GH_UPDATED_PR".into(), updated.into_os_string());

    assert_eq!(
        bridge::push_and_create_draft(
            &prepared.state_path,
            &mut prepared.state,
            &h1_proof,
            "Implement issue",
            super::support_launch::DRAFT_ISSUE_BODY,
            &prepared.adapter,
        )
        .expect("resume same-PR update after descendant push"),
        17,
    );
    assert_eq!(prepared.state.phase, bridge::BridgePhase::DraftCreated);
    assert_eq!(prepared.state.pr, Some(17));
    assert_eq!(
        prepared.state.head_oid.as_deref(),
        Some(h1_proof.head_oid.as_str())
    );
    let ancestor = std::process::Command::new("git")
        .args(["merge-base", "--is-ancestor", &h0, &h1_proof.head_oid])
        .current_dir(&prepared.state.identity.worktree)
        .status()
        .expect("verify repaired ancestry");
    assert!(ancestor.success());
    assert_eq!(
        git_stdout(
            &prepared.state.identity.worktree,
            &[
                "ls-remote",
                "origin",
                &format!("refs/heads/{}", prepared.state.identity.branch),
            ]
        )
        .split_whitespace()
        .next(),
        Some(h1_proof.head_oid.as_str())
    );
    let calls =
        fs::read_to_string(adapter_path(&prepared.adapter, "GH_CALLS")).expect("GH call log");
    assert_eq!(
        calls
            .lines()
            .filter(|call| call.starts_with("pr create "))
            .count(),
        1
    );
    assert_eq!(
        calls
            .lines()
            .filter(|call| call.starts_with("pr edit 17 "))
            .count(),
        1
    );
}

#[test]
fn autonomous_executor_bridge_primary_smoke_is_additional_to_full_suite() {
    let fixture = GitFixture::new("smoke-additional");
    let issue = "### Primary smoke test (inner loop)\n\n```bash\n/usr/bin/false\n```\n\n### Operator/full verification\n\n```bash\n/usr/bin/true\n```\n";
    let smoke = bridge::parse_primary_smoke(issue).expect("primary smoke");
    let full = bridge::resolve_full_suite(&fixture.repo, issue, &[], &BTreeMap::new())
        .expect("full suite");
    assert_eq!(smoke.commands[0].argv, vec!["/usr/bin/false"]);
    assert_eq!(full.plan.commands[0].argv, vec!["/usr/bin/true"]);
}

#[test]
fn autonomous_executor_bridge_marks_only_exact_passed_draft_ready() {
    let (_fixture, mut state, _snapshot, _) = implementation_proof_fixture("ready-pass-only");
    commit_implementation(&state);
    let head_oid = git_stdout(&state.identity.worktree, &["rev-parse", "HEAD"]);
    state.phase = bridge::BridgePhase::DraftCreated;
    state.pr = Some(17);
    state.head_oid = Some(head_oid.clone());
    let lane = bridge::PremergeLaneIdentity::new(
        state.identity.repository.clone(),
        state.identity.issue,
        state.identity.worker_id.clone(),
        state.identity.claim_id.clone(),
        state.identity.branch.clone(),
        head_oid.clone(),
    )
    .expect("lane");
    let pass = bridge::PremergeDecision::Pass {
        lane,
        evidence_digest: "evidence".into(),
    };

    let admission = bridge::ready_admission(&state, &pass).expect("exact Pass admits ready");
    assert_eq!(admission.pull_request, 17);
    assert_eq!(admission.head_oid, head_oid);
    assert_eq!(admission.evidence_digest, "evidence");

    let blocked = bridge::PremergeDecision::Blocked {
        lane: admission.lane.clone(),
        reason: "scanner".into(),
        evidence_digest: "blocked".into(),
        quarantine: autospec_core::autonomous::premerge::LaneQuarantine {
            lane: admission.lane,
            evidence_digest: "blocked".into(),
            finding_codes: vec!["scanner".into()],
        },
    };
    assert!(bridge::ready_admission(&state, &blocked).is_err());
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_observes_exact_draft_becoming_ready() {
    let mut prepared = prepared_draft_transaction("ready-success");
    prepared.bind_continuation();
    prepared.publish().expect("draft");
    let current_path = adapter_path(&prepared.adapter, "GH_PR_STATE");
    let ready_path = prepared.fixture.root.join("ready-success.json");
    fs::write(
        &ready_path,
        fs::read_to_string(&current_path)
            .expect("draft JSON")
            .replace("\"isDraft\":true", "\"isDraft\":false"),
    )
    .expect("ready JSON");
    prepared
        .adapter
        .environment
        .insert("GH_READY_PR".into(), ready_path.into_os_string());
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

    bridge::mark_exact_draft_ready_with_refresh(
        &prepared.state_path,
        &mut prepared.state,
        &pass,
        &prepared.adapter,
        || Ok(bridge::BridgeClaimOwnership::Refreshed { ttl_seconds: 60 }),
    )
    .expect("ready transition");

    assert_eq!(prepared.state.phase, bridge::BridgePhase::Ready);
    assert!(fs::read_to_string(current_path)
        .expect("observed ready")
        .contains("\"isDraft\":false"));
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_keeps_ready_inventory_outages_transient() {
    let mut prepared = prepared_draft_transaction("ready-inventory-outage");
    prepared.publish().expect("draft");
    let failing_gh = prepared.fixture.root.join("gh-inventory-outage");
    fs::write(&failing_gh, "#!/bin/sh\nexit 42\n").expect("failing gh");
    fs::set_permissions(&failing_gh, fs::Permissions::from_mode(0o755)).expect("failing gh mode");
    prepared.adapter.gh = failing_gh;
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

    let error = bridge::mark_exact_draft_ready_with_refresh(
        &prepared.state_path,
        &mut prepared.state,
        &pass,
        &prepared.adapter,
        || Ok(bridge::BridgeClaimOwnership::Refreshed { ttl_seconds: 60 }),
    )
    .expect_err("PR inventory outage must remain retryable");

    assert_eq!(error.kind, bridge::BridgeFailureKind::Transient);
    assert_eq!(prepared.state.phase, bridge::BridgePhase::DraftCreated);
    let durable = bridge::PersistedInvocation::from_json(
        &fs::read_to_string(&prepared.state_path).expect("durable invocation"),
    )
    .expect("parse durable invocation");
    assert_eq!(durable.phase, bridge::BridgePhase::DraftCreated);
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_claim_takeover_blocks_ready_mutation() {
    let mut prepared = prepared_draft_transaction("ready-takeover");
    prepared.publish().expect("draft");
    let current_path = PathBuf::from(
        prepared
            .adapter
            .environment
            .get(&OsString::from("GH_PR_STATE"))
            .expect("PR state"),
    );
    let ready_path = prepared.fixture.root.join("ready-pr.json");
    let current = fs::read_to_string(&current_path).expect("draft JSON");
    fs::write(
        &ready_path,
        current.replace("\"isDraft\":true", "\"isDraft\":false"),
    )
    .expect("ready JSON");
    prepared
        .adapter
        .environment
        .insert("GH_READY_PR".into(), ready_path.into_os_string());
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

    let error = bridge::mark_exact_draft_ready_with_refresh(
        &prepared.state_path,
        &mut prepared.state,
        &pass,
        &prepared.adapter,
        || Ok(bridge::BridgeClaimOwnership::Lost),
    )
    .expect_err("takeover blocks ready");
    assert!(error.contains("ownership"), "{error}");
    assert!(fs::read_to_string(current_path)
        .expect("preserved draft")
        .contains("\"isDraft\":true"));
}

#[test]
fn autonomous_executor_bridge_waits_for_every_non_advisory_required_check() {
    let checks = r#"[
        {"name":"unit","state":"SUCCESS"},
        {"name":"teamcity","state":"FAILURE"}
    ]"#;
    let advisory = bridge::BTreeSet::from(["^teamcity( .*)?$".to_string()]);
    assert_eq!(
        bridge::evaluate_required_checks(checks, &advisory).expect("checks"),
        bridge::RequiredChecksDecision::Pass
    );
    assert_eq!(
        bridge::evaluate_required_checks(r#"[{"name":"teamcity","state":"PENDING"}]"#, &advisory,)
            .expect("all required checks are explicitly advisory"),
        bridge::RequiredChecksDecision::Pass
    );
    assert!(
        bridge::evaluate_required_checks("[]", &bridge::BTreeSet::new()).is_err(),
        "truly missing required-check evidence must fail closed"
    );

    let pending = r#"[{"name":"unit","state":"PENDING"}]"#;
    assert_eq!(
        bridge::evaluate_required_checks(pending, &bridge::BTreeSet::new()).expect("pending"),
        bridge::RequiredChecksDecision::Pending
    );

    let failing = r#"[{"name":"unit","state":"FAILURE"}]"#;
    assert!(matches!(
        bridge::evaluate_required_checks(failing, &bridge::BTreeSet::new()).expect("failing"),
        bridge::RequiredChecksDecision::Failed { .. }
    ));
}

#[test]
fn autonomous_executor_bridge_refreshes_exact_claim_during_ci_polling() {
    let (_fixture, mut state, _snapshot, _) = implementation_proof_fixture("ci-refresh");
    state.phase = bridge::BridgePhase::Ready;
    state.pr = Some(17);
    let mut polls = vec![
        r#"[{"name":"unit","state":"PENDING"}]"#.to_string(),
        r#"[{"name":"unit","state":"SUCCESS"}]"#.to_string(),
    ]
    .into_iter();
    let mut refreshes = 0;
    let mut delays = Vec::new();
    bridge::wait_for_required_ci_with_delay(
        &state,
        2,
        &bridge::BTreeSet::new(),
        || Ok(polls.next().expect("bounded poll")),
        || {
            refreshes += 1;
            Ok(bridge::BridgeClaimOwnership::Refreshed { ttl_seconds: 60 })
        },
        Duration::from_secs(7),
        |duration| delays.push(duration),
    )
    .expect("pending check eventually succeeds");
    assert_eq!(refreshes, 2);
    assert_eq!(delays, vec![Duration::from_secs(7)]);
}

#[test]
fn autonomous_executor_bridge_reviewer_accepts_only_bare_lgtm() {
    assert!(bridge::strict_lgtm("LGTM\n").is_ok());
    for rejected in [
        "",
        "looks good",
        "LGTM\nfinding: race",
        "LGTM with reservations",
        "```text\nLGTM\n```",
    ] {
        assert!(
            bridge::strict_lgtm(rejected).is_err(),
            "accepted non-strict review: {rejected:?}"
        );
    }
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_reviewer_rejects_non_lgtm_harness_result() {
    // Break caught: accepting strict stdout while the harness result contains findings.
    let root = test_root("reviewer-harness-result");
    let result = root.join("harness-result.txt");
    fs::write(&result, "finding: unsafe boundary\n").expect("reviewer result");
    fs::set_permissions(&result, fs::Permissions::from_mode(0o600))
        .expect("private reviewer result");

    let error = bridge::strict_lgtm_harness_result(&result)
        .expect_err("harness findings must block review");

    assert!(error.contains("strict LGTM"), "{error}");
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_paginated_comments_flatten_exact_typed_values() {
    // Break caught: executor comment reads combining --slurp and --jq never reach GitHub.
    let fixture = GitFixture::new("paginated-comments");
    let state = supervision_state(&fixture);
    let gh = fixture.root.join("gh-paginated-comments");
    write_executable(
        &gh,
        r#"#!/bin/sh
set -eu
slurp=0
jq=0
for argument in "$@"; do
  [ "$argument" = --slurp ] && slurp=1
  [ "$argument" = --jq ] && jq=1
done
if [ "$slurp" -eq 1 ] && [ "$jq" -eq 1 ]; then
  printf '%s\n' 'the `--slurp` option is not supported with `--jq` or `--template`' >&2
  exit 64
fi
printf '%s\n' '[[{"id":100,"body":"page one","updated_at":"2026-07-27T00:00:00Z","user":{"login":"autospec"}}],[{"id":101,"body":null,"updated_at":null,"user":{"login":"operator"}}]]'
"#,
    );
    let adapter = bridge::DraftPrAdapter {
        gh,
        environment: BTreeMap::new(),
    };

    let comments =
        bridge::list_bridge_comments(&state, &adapter).expect("paginated comments parse");

    assert_eq!(
        comments,
        vec![
            autospec_core::claim::RemoteComment::new(100, "page one", "2026-07-27T00:00:00Z",),
            autospec_core::claim::RemoteComment::new(101, "", ""),
        ]
    );
}
