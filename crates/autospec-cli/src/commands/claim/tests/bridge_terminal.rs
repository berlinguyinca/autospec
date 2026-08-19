// claim tests: bridge / terminal — 6 cases.
//
// Split out of tests.rs; see the note in that file.

use super::super::{advance_claim_ref_in, create_claim_ref_commit, ClaimRefAdvance};
use super::support::{
    assert_bridge_transition_projection, claim_record, git, ClaimRefFixture, lock_heartbeat_env,
};
use crate::commands::claim;
use autospec_core::claim::{ExecutorResultEvidence, RemoteComment};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::{Arc, Barrier};

#[test]
fn bridge_terminal_projection_resumes_only_the_exact_terminal_generation() {
    let identity = claim::ClaimMutationIdentity {
        repo: "owner/repo",
        issue: 42,
        worker_id: "worker-a",
        branch: "feat/worker-a",
        claim_id: "claim-a",
    };
    let mut terminal = claim_record("worker-a", "claim-a", "merged");
    terminal.step = "merged".into();
    terminal.pr = "17".into();
    assert_eq!(
        claim::bridge_terminal_mode(
            &terminal,
            identity,
            "merged",
            "merged",
            "17",
            "terminal_prepared:merged",
        ),
        claim::BridgeTerminalMode::Complete
    );

    terminal.claim_id = Some("claim-takeover".into());
    assert_eq!(
        claim::bridge_terminal_mode(
            &terminal,
            identity,
            "merged",
            "merged",
            "17",
            "terminal_prepared:merged",
        ),
        claim::BridgeTerminalMode::Lost
    );
}

#[test]
fn bridge_result_recovery_collapses_only_identical_publication_retries() {
    let evidence = ExecutorResultEvidence::new(
        "owner/repo",
        42,
        "worker-a",
        "feat/worker-a",
        "succeeded",
        Some(17),
        "executor_succeeded",
        "stable-receipt",
        Some("claim-a".into()),
        Some("a".repeat(40)),
        Some("b".repeat(64)),
    );
    let body = evidence.to_marked_comment();
    let comments = vec![
        RemoteComment::new(1, body.clone(), "2026-07-26T00:00:00Z"),
        RemoteComment::new(2, body, "2026-07-26T00:00:01Z"),
    ];
    assert_eq!(
        claim::exact_successful_executor_result(
            &comments,
            claim::ClaimMutationIdentity {
                repo: "owner/repo",
                issue: 42,
                worker_id: "worker-a",
                branch: "feat/worker-a",
                claim_id: "claim-a",
            },
            claim::ExecutorResultAuthorityBinding {
                pull_request: 17,
                commit: &"a".repeat(40),
                premerge_receipt: &"b".repeat(64),
                receipt_id: "stable-receipt",
            },
        ),
        Some(evidence)
    );
}

#[test]
fn bridge_result_authority_filters_exact_generation_before_uniqueness() {
    let old = ExecutorResultEvidence::new(
        "owner/repo",
        42,
        "worker-a",
        "feat/worker-a",
        "succeeded",
        Some(17),
        "executor_succeeded",
        "bridge-old",
        Some("claim-old".into()),
        Some("a".repeat(40)),
        Some("b".repeat(64)),
    );
    let successor = ExecutorResultEvidence::new(
        "owner/repo",
        42,
        "worker-a",
        "feat/worker-a",
        "succeeded",
        Some(17),
        "executor_succeeded",
        "bridge-successor",
        Some("claim-successor".into()),
        Some("a".repeat(40)),
        Some("b".repeat(64)),
    );
    let comments = vec![
        RemoteComment::new(1, old.to_marked_comment(), "2026-07-26T00:00:00Z"),
        RemoteComment::new(2, successor.to_marked_comment(), "2026-07-26T00:00:01Z"),
        RemoteComment::new(3, successor.to_marked_comment(), "2026-07-26T00:00:02Z"),
    ];

    assert_eq!(
        claim::exact_successful_executor_result(
            &comments,
            claim::ClaimMutationIdentity {
                repo: "owner/repo",
                issue: 42,
                worker_id: "worker-a",
                branch: "feat/worker-a",
                claim_id: "claim-successor",
            },
            claim::ExecutorResultAuthorityBinding {
                pull_request: 17,
                commit: &"a".repeat(40),
                premerge_receipt: &"b".repeat(64),
                receipt_id: "bridge-successor",
            },
        ),
        Some(successor)
    );
}

#[cfg(unix)]
#[test]
fn executor_result_takeover_after_renewal_is_inert_for_successor_authority() {
    let _guard = lock_heartbeat_env();
    let fixture = ClaimRefFixture::new("executor-result-takeover");
    let bin = fixture.root.join("bin");
    let gh = bin.join("gh");
    let comments = fixture.root.join("comments.json");
    let observed_renewal = fixture.root.join("observed-renewal.txt");
    std::fs::create_dir(&bin).expect("bin");
    std::fs::write(&comments, "[]").expect("comments");
    std::fs::write(
        &gh,
        "#!/bin/sh\n\
         set -eu\n\
         if [ \"$1\" = api ]; then cat \"$GH_COMMENTS\"; exit 0; fi\n\
         if [ \"$1 $2\" = 'pr list' ]; then cat <<'EOF'\n\
         [{\"number\":17,\"body\":\"Closes #42\\n\\n## Closeout report\\nResult\",\"headRefName\":\"feat/worker-a\",\"headRefOid\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"isDraft\":false,\"baseRefName\":\"main\"}]\n\
         EOF\n\
         exit 0; fi\n\
         if [ \"$1 $2\" = 'issue comment' ]; then\n\
           body=''; while [ \"$#\" -gt 0 ]; do if [ \"$1\" = --body ]; then shift; body=$1; fi; shift || true; done\n\
           jq --arg body \"$body\" '. + [{id:(length + 1),body:$body,updated_at:\"2026-07-26T00:00:00Z\"}]' \"$GH_COMMENTS\" > \"$GH_COMMENTS.tmp\"\n\
           mv \"$GH_COMMENTS.tmp\" \"$GH_COMMENTS\"\n\
           current=$(git --git-dir=\"$GH_REMOTE\" rev-parse refs/autospec/claims/issue-42)\n\
           git --git-dir=\"$GH_REMOTE\" show -s --format=%B \"$current\" > \"$GH_OBSERVED_RENEWAL\"\n\
           git --git-dir=\"$GH_REMOTE\" update-ref refs/autospec/claims/issue-42 \"$GH_TAKEOVER_OID\"\n\
           exit 0\n\
         fi\n\
         exit 64\n",
    )
    .expect("gh");
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).expect("gh mode");

    let mut original = claim_record("worker-a", "claim-old", "claimed");
    original.ttl_seconds = u64::MAX;
    let initial = match advance_claim_ref_in(
        Path::new("git"),
        &fixture.clients[0],
        fixture.remote.to_str().unwrap(),
        "owner/repo",
        42,
        None,
        &original,
    )
    .expect("seed claim")
    {
        ClaimRefAdvance::Won(head) => head,
        ClaimRefAdvance::Lost => panic!("initial claim must win"),
    };
    git(
        &fixture.clients[1],
        &[
            "fetch",
            fixture.remote.to_str().unwrap(),
            "refs/autospec/claims/issue-42",
        ],
    );
    let mut successor = claim_record("worker-a", "claim-successor", "claimed");
    successor.ttl_seconds = u64::MAX;
    let successor_oid = create_claim_ref_commit(
        Path::new("git"),
        &fixture.clients[1],
        Some(&initial),
        "successor-generation",
        &successor,
    )
    .expect("create successor claim");
    git(
        &fixture.clients[1],
        &[
            "push",
            fixture.remote.to_str().unwrap(),
            &format!("{successor_oid}:refs/autospec/test/successor"),
        ],
    );

    let old_path = std::env::var_os("PATH");
    let old_remote = std::env::var_os("AUTOSPEC_CLAIM_GIT_REMOTE");
    let old_state = std::env::var_os("AUTOSPEC_CLAIM_GIT_STATE_DIR");
    let old_comments = std::env::var_os("GH_COMMENTS");
    let old_gh_remote = std::env::var_os("GH_REMOTE");
    let old_takeover = std::env::var_os("GH_TAKEOVER_OID");
    let old_observed = std::env::var_os("GH_OBSERVED_RENEWAL");
    let old_retries = std::env::var_os("AUTOSPEC_GH_API_RETRIES");
    std::env::set_var(
        "PATH",
        format!(
            "{}:{}",
            bin.display(),
            old_path.as_deref().unwrap_or_default().to_string_lossy()
        ),
    );
    std::env::set_var("AUTOSPEC_CLAIM_GIT_REMOTE", &fixture.remote);
    std::env::set_var(
        "AUTOSPEC_CLAIM_GIT_STATE_DIR",
        fixture.root.join("claim-state"),
    );
    std::env::set_var("GH_COMMENTS", &comments);
    std::env::set_var("GH_REMOTE", &fixture.remote);
    std::env::set_var("GH_TAKEOVER_OID", &successor_oid);
    std::env::set_var("GH_OBSERVED_RENEWAL", &observed_renewal);
    std::env::set_var("AUTOSPEC_GH_API_RETRIES", "1");

    let old_identity = claim::ClaimMutationIdentity {
        repo: "owner/repo",
        issue: 42,
        worker_id: "worker-a",
        branch: "feat/worker-a",
        claim_id: "claim-old",
    };
    assert_eq!(
        claim::record_executor_result_with_receipt(
            old_identity,
            &autospec_core::coordination::ConductorOutcome::Succeeded,
            Some(17),
            Some(claim::ExecutorSuccessBinding {
                claim_id: "claim-old",
                commit: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                premerge_receipt:
                    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            }),
            Some("bridge-old"),
        )
        .expect("publish stale result"),
        claim::ExecutorResultRecord::OwnershipLost
    );
    let renewal = std::fs::read_to_string(&observed_renewal).expect("renewal observation");
    assert!(
        renewal.contains("\"step\":\"executor_succeeded\""),
        "{renewal}"
    );
    assert!(!renewal.contains("\"updated_at\":\"2026-07-25T00:00:00Z\""));

    let successor_evidence = ExecutorResultEvidence::new(
        "owner/repo",
        42,
        "worker-a",
        "feat/worker-a",
        "succeeded",
        Some(17),
        "executor_succeeded",
        "bridge-successor",
        Some("claim-successor".into()),
        Some("a".repeat(40)),
        Some("b".repeat(64)),
    );
    let mut published: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&comments).expect("comments"))
            .expect("comments JSON");
    published
        .as_array_mut()
        .expect("comment array")
        .push(serde_json::json!({
            "id": 2,
            "body": successor_evidence.to_marked_comment(),
            "updated_at": "2026-07-26T00:00:01Z"
        }));
    std::fs::write(
        &comments,
        serde_json::to_vec(&published).expect("serialize comments"),
    )
    .expect("publish successor evidence");

    assert_eq!(
        claim::authoritative_executor_result(
            claim::ClaimMutationIdentity {
                repo: "owner/repo",
                issue: 42,
                worker_id: "worker-a",
                branch: "feat/worker-a",
                claim_id: "claim-successor",
            },
            claim::ExecutorResultAuthorityBinding {
                pull_request: 17,
                commit: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                premerge_receipt:
                    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                receipt_id: "bridge-successor",
            },
        )
        .expect("read successor authority"),
        Some(successor_evidence)
    );

    for (key, value) in [
        ("PATH", old_path),
        ("AUTOSPEC_CLAIM_GIT_REMOTE", old_remote),
        ("AUTOSPEC_CLAIM_GIT_STATE_DIR", old_state),
        ("GH_COMMENTS", old_comments),
        ("GH_REMOTE", old_gh_remote),
        ("GH_TAKEOVER_OID", old_takeover),
        ("GH_OBSERVED_RENEWAL", old_observed),
        ("AUTOSPEC_GH_API_RETRIES", old_retries),
    ] {
        match value {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }
}

#[cfg(unix)]
#[test]
fn bridge_terminal_transitions_prepare_before_labels_and_restarts_do_not_relabel() {
    let _guard = lock_heartbeat_env();
    let retryable_edit = [
        "issue edit 42 ",
        "repo owner/repo ",
        "remove-label in-progress-by-bot ",
        "remove-label autospec:needs-human ",
        "add-label auto-implement",
    ]
    .join("--");
    assert_bridge_transition_projection(
        "bridge-retryable",
        claim::BridgeClaimDisposition::Retryable,
        "released",
        "terminal_prepared:retryable_released",
        &retryable_edit,
        1,
        false,
    );
    assert_bridge_transition_projection(
        "bridge-needs-human",
        claim::BridgeClaimDisposition::NeedsHuman,
        "failed",
        "terminal_prepared:needs_human",
        "issue edit 42 --repo owner/repo --remove-label in-progress-by-bot --remove-label auto-implement --add-label autospec:needs-human",
        1,
        false,
    );
    assert_bridge_transition_projection(
        "bridge-merged",
        claim::BridgeClaimDisposition::Merged,
        "merged",
        "terminal_prepared:merged",
        "issue edit 42 --repo owner/repo --remove-label in-progress-by-bot",
        2,
        false,
    );
    assert_bridge_transition_projection(
        "bridge-retryable-prepared-restart",
        claim::BridgeClaimDisposition::Retryable,
        "released",
        "terminal_prepared:retryable_released",
        &retryable_edit,
        1,
        true,
    );
}

#[test]
fn claim_ref_initial_creation_has_exactly_one_receive_pack_winner() {
    // Break caught: two absent-ref acquisitions both succeeding on eventually-consistent comments.
    let fixture = ClaimRefFixture::new("initial-race");
    let barrier = Arc::new(Barrier::new(3));
    let handles = fixture
        .clients
        .clone()
        .into_iter()
        .enumerate()
        .map(|(index, client)| {
            let barrier = Arc::clone(&barrier);
            let remote = fixture.remote.clone();
            std::thread::spawn(move || {
                let worker = format!("worker-{index}");
                let record = claim_record(&worker, &format!("claim-{index}"), "claimed");
                barrier.wait();
                advance_claim_ref_in(
                    Path::new("git"),
                    &client,
                    remote.to_str().unwrap(),
                    "owner/repo",
                    42,
                    None,
                    &record,
                )
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();
    let results = handles
        .into_iter()
        .map(|handle| {
            handle
                .join()
                .expect("claim publisher")
                .expect("claim result")
        })
        .collect::<Vec<_>>();

    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, ClaimRefAdvance::Won(_)))
            .count(),
        1
    );
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, ClaimRefAdvance::Lost))
            .count(),
        1
    );
}
