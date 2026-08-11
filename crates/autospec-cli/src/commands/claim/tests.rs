use super::{
    advance_claim_ref_in, claim_settle_millis, create_claim_ref_commit,
    lifecycle_claim_evidence_from_record, private_claim_git_dir_in, publish_session_binding,
    read_claim_ref_in, validated_claim_remote, ClaimRefAdvance, SessionBindingIdentity,
};
use autospec_core::autonomous_lifecycle::{
    ClaimBranch, ClaimContext, ClaimEvidence, IssueNumber, LeaseFreshness, RepositoryScope,
    WorkerId,
};
use autospec_core::claim::{ExecutorResultEvidence, RemoteComment, RunStateRecord};
#[cfg(unix)]
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Barrier, Mutex};

mod support;
mod heartbeat_startup;
mod heartbeat_prior;
mod heartbeat_classify;
mod heartbeat_quarantine;
mod paginated_comments;
use support::*;

#[test]
fn bridge_terminal_projection_resumes_only_the_exact_terminal_generation() {
    let identity = super::ClaimMutationIdentity {
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
        super::bridge_terminal_mode(
            &terminal,
            identity,
            "merged",
            "merged",
            "17",
            "terminal_prepared:merged",
        ),
        super::BridgeTerminalMode::Complete
    );

    terminal.claim_id = Some("claim-takeover".into());
    assert_eq!(
        super::bridge_terminal_mode(
            &terminal,
            identity,
            "merged",
            "merged",
            "17",
            "terminal_prepared:merged",
        ),
        super::BridgeTerminalMode::Lost
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
        super::exact_successful_executor_result(
            &comments,
            super::ClaimMutationIdentity {
                repo: "owner/repo",
                issue: 42,
                worker_id: "worker-a",
                branch: "feat/worker-a",
                claim_id: "claim-a",
            },
            super::ExecutorResultAuthorityBinding {
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
        super::exact_successful_executor_result(
            &comments,
            super::ClaimMutationIdentity {
                repo: "owner/repo",
                issue: 42,
                worker_id: "worker-a",
                branch: "feat/worker-a",
                claim_id: "claim-successor",
            },
            super::ExecutorResultAuthorityBinding {
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
    let _guard = BRIDGE_TRANSITION_ENV.lock().expect("transition env");
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

    let old_identity = super::ClaimMutationIdentity {
        repo: "owner/repo",
        issue: 42,
        worker_id: "worker-a",
        branch: "feat/worker-a",
        claim_id: "claim-old",
    };
    assert_eq!(
        super::record_executor_result_with_receipt(
            old_identity,
            &autospec_core::coordination::ConductorOutcome::Succeeded,
            Some(17),
            Some(super::ExecutorSuccessBinding {
                claim_id: "claim-old",
                commit: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                premerge_receipt:
                    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            }),
            Some("bridge-old"),
        )
        .expect("publish stale result"),
        super::ExecutorResultRecord::OwnershipLost
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
        super::authoritative_executor_result(
            super::ClaimMutationIdentity {
                repo: "owner/repo",
                issue: 42,
                worker_id: "worker-a",
                branch: "feat/worker-a",
                claim_id: "claim-successor",
            },
            super::ExecutorResultAuthorityBinding {
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
    let _guard = BRIDGE_TRANSITION_ENV.lock().expect("transition env");
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
        super::BridgeClaimDisposition::Retryable,
        "released",
        "terminal_prepared:retryable_released",
        &retryable_edit,
        1,
        false,
    );
    assert_bridge_transition_projection(
        "bridge-needs-human",
        super::BridgeClaimDisposition::NeedsHuman,
        "failed",
        "terminal_prepared:needs_human",
        "issue edit 42 --repo owner/repo --remove-label in-progress-by-bot --remove-label auto-implement --add-label autospec:needs-human",
        1,
        false,
    );
    assert_bridge_transition_projection(
        "bridge-merged",
        super::BridgeClaimDisposition::Merged,
        "merged",
        "terminal_prepared:merged",
        "issue edit 42 --repo owner/repo --remove-label in-progress-by-bot",
        2,
        false,
    );
    assert_bridge_transition_projection(
        "bridge-retryable-prepared-restart",
        super::BridgeClaimDisposition::Retryable,
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

#[test]
fn claim_ref_rejects_a_stale_terminal_transition_after_takeover() {
    // Break caught: stale release publishing an absorbing terminal comment before ownership CAS.
    let fixture = ClaimRefFixture::new("stale-terminal");
    let original = claim_record("worker-a", "claim-a", "claimed");
    let initial = advance_claim_ref_in(
        Path::new("git"),
        &fixture.clients[0],
        fixture.remote.to_str().unwrap(),
        "owner/repo",
        42,
        None,
        &original,
    )
    .expect("initial claim");
    let ClaimRefAdvance::Won(parent) = initial else {
        panic!("initial claim must win");
    };
    let takeover = claim_record("worker-b", "claim-b", "claimed");
    assert!(matches!(
        advance_claim_ref_in(
            Path::new("git"),
            &fixture.clients[1],
            fixture.remote.to_str().unwrap(),
            "owner/repo",
            42,
            Some(&parent),
            &takeover,
        )
        .expect("takeover"),
        ClaimRefAdvance::Won(_)
    ));
    let terminal = claim_record("worker-a", "claim-a", "merged");
    assert_eq!(
        advance_claim_ref_in(
            Path::new("git"),
            &fixture.clients[0],
            fixture.remote.to_str().unwrap(),
            "owner/repo",
            42,
            Some(&parent),
            &terminal,
        )
        .expect("stale terminal"),
        ClaimRefAdvance::Lost
    );

    let head = read_claim_ref_in(
        Path::new("git"),
        &fixture.clients[0],
        fixture.remote.to_str().unwrap(),
        "owner/repo",
        42,
    )
    .expect("read winner")
    .expect("claim ref");
    assert_eq!(head.record.worker_id, "worker-b");
    assert_eq!(head.record.state, "claimed");
}

#[cfg(unix)]
#[test]
fn claim_ref_keeps_an_unchanged_failed_push_transient() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = ClaimRefFixture::new("unchanged-push-failure");
    let original = claim_record("worker-a", "claim-a", "claimed");
    let ClaimRefAdvance::Won(parent) = advance_claim_ref_in(
        Path::new("git"),
        &fixture.clients[0],
        fixture.remote.to_str().unwrap(),
        "owner/repo",
        42,
        None,
        &original,
    )
    .expect("seed claim") else {
        panic!("seed claim must win");
    };
    let wrapper = fixture.root.join("git-fail-push");
    std::fs::write(
        &wrapper,
        "#!/bin/sh\nfor arg in \"$@\"; do if [ \"$arg\" = push ]; then echo outage >&2; exit 75; fi; done\nexec git \"$@\"\n",
    )
    .expect("git wrapper");
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755))
        .expect("git wrapper mode");
    let refreshed = claim_record("worker-a", "claim-a", "claimed");

    let error = advance_claim_ref_in(
        &wrapper,
        &fixture.clients[0],
        fixture.remote.to_str().unwrap(),
        "owner/repo",
        42,
        Some(&parent),
        &refreshed,
    )
    .expect_err("unchanged failed push must remain retryable");

    assert_eq!(error.kind, crate::commands::CommandFailureKind::Transient);
    assert!(
        error.message.contains("without changing"),
        "{}",
        error.message
    );
}

#[test]
fn claim_ref_renewal_race_has_one_exact_parent_winner() {
    // Break caught: two renewals both extending the same generation.
    let fixture = ClaimRefFixture::new("renewal-race");
    let parent = seed_claim(&fixture);
    let mut renewal_a = parent.record.clone();
    renewal_a.updated_at = "2026-07-25T00:00:01Z".to_string();
    let mut renewal_b = parent.record.clone();
    renewal_b.updated_at = "2026-07-25T00:00:02Z".to_string();
    let results = race_claim_ref_transitions(&fixture, &parent, [renewal_a, renewal_b]);
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

#[test]
fn claim_ref_takeover_and_renewal_each_win_when_received_first() {
    // Break caught: eventual comment ordering allowing both a takeover and renewal.
    for takeover_first in [false, true] {
        let fixture = ClaimRefFixture::new(if takeover_first {
            "takeover-first"
        } else {
            "renewal-first"
        });
        let parent = seed_claim(&fixture);
        let mut renewal = parent.record.clone();
        renewal.updated_at = "2026-07-25T00:00:01Z".to_string();
        let takeover = claim_record("worker-b", "claim-b", "claimed");
        let ordered = if takeover_first {
            [takeover, renewal]
        } else {
            [renewal, takeover]
        };
        let first = advance_claim_ref_in(
            Path::new("git"),
            &fixture.clients[0],
            fixture.remote.to_str().unwrap(),
            "owner/repo",
            42,
            Some(&parent),
            &ordered[0],
        )
        .expect("first transition");
        assert!(matches!(first, ClaimRefAdvance::Won(_)));
        let second = advance_claim_ref_in(
            Path::new("git"),
            &fixture.clients[1],
            fixture.remote.to_str().unwrap(),
            "owner/repo",
            42,
            Some(&parent),
            &ordered[1],
        )
        .expect("second transition");
        assert_eq!(second, ClaimRefAdvance::Lost);
        let head = read_claim_ref_in(
            Path::new("git"),
            &fixture.clients[0],
            fixture.remote.to_str().unwrap(),
            "owner/repo",
            42,
        )
        .expect("read winner")
        .expect("claim ref");
        assert_eq!(head.record.worker_id, ordered[0].worker_id);
        assert_eq!(head.record.claim_id, ordered[0].claim_id);
    }
}

#[test]
fn claim_ref_ambiguous_push_rereads_without_a_second_mutation() {
    // Break caught: retrying a POST/push after receive-pack committed but the response vanished.
    use std::os::unix::fs::PermissionsExt;

    let fixture = ClaimRefFixture::new("ambiguous-push");
    let wrapper = fixture.root.join("ambiguous-git");
    let push_log = fixture.root.join("push.log");
    let script = format!(
        "#!/bin/sh\nif [ \"$1\" = push ]; then\n  /usr/bin/git \"$@\"\n  status=$?\n  printf 'push\\n' >> '{}'\n  [ \"$status\" -eq 0 ] && exit 73\n  exit \"$status\"\nfi\nexec /usr/bin/git \"$@\"\n",
        push_log.display()
    );
    std::fs::write(&wrapper, script).expect("write git wrapper");
    let mut permissions = std::fs::metadata(&wrapper)
        .expect("git wrapper metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&wrapper, permissions).expect("make git wrapper executable");

    let record = claim_record("worker-a", "claim-a", "claimed");
    assert!(matches!(
        advance_claim_ref_in(
            &wrapper,
            &fixture.clients[0],
            fixture.remote.to_str().unwrap(),
            "owner/repo",
            42,
            None,
            &record,
        )
        .expect("ambiguous transition"),
        ClaimRefAdvance::Won(_)
    ));
    assert_eq!(
        std::fs::read_to_string(push_log)
            .expect("push invocation log")
            .lines()
            .count(),
        1
    );
}

#[test]
fn claim_ref_terminal_transition_rejects_foreign_identity() {
    // Break caught: foreign worker, branch, or claim ID publishing a terminal state.
    let fixture = ClaimRefFixture::new("foreign-release");
    let parent = seed_claim(&fixture);
    for (worker, branch, claim_id) in [
        ("worker-b", "feat/worker-a", "claim-a"),
        ("worker-a", "feat/worker-b", "claim-a"),
        ("worker-a", "feat/worker-a", "claim-b"),
    ] {
        let mut terminal = parent.record.clone();
        terminal.state = "merged".to_string();
        terminal.step = "merged".to_string();
        terminal.worker_id = worker.to_string();
        terminal.branch = branch.to_string();
        terminal.claim_id = Some(claim_id.to_string());
        assert!(advance_claim_ref_in(
            Path::new("git"),
            &fixture.clients[0],
            fixture.remote.to_str().unwrap(),
            "owner/repo",
            42,
            Some(&parent),
            &terminal,
        )
        .is_err());
    }
    let head = read_claim_ref_in(
        Path::new("git"),
        &fixture.clients[0],
        fixture.remote.to_str().unwrap(),
        "owner/repo",
        42,
    )
    .expect("read claim")
    .expect("claim ref");
    assert_eq!(head, parent);
}

#[test]
fn claim_ref_https_push_uses_gh_credential_helper_without_a_token_argument() {
    // Break caught: relying on global `gh auth setup-git` or exposing a token in argv.
    let arguments = super::claim_remote_arguments(
        "https://github.enterprise.example/owner/repo.git",
        &["push", "origin", "abc123:refs/autospec/claims/issue-42"],
    );
    assert_eq!(
        arguments,
        [
            "-c",
            "credential.helper=!gh auth git-credential",
            "-c",
            "credential.useHttpPath=true",
            "push",
            "origin",
            "abc123:refs/autospec/claims/issue-42",
        ]
    );
    assert!(!arguments.iter().any(|argument| {
        argument.contains("token")
            || argument.contains("Authorization")
            || argument.starts_with("https://")
    }));
    for remote_command in [
        vec!["ls-remote", "--refs", "origin"],
        vec!["fetch", "--no-tags", "origin"],
    ] {
        let arguments = super::claim_remote_arguments(
            "https://github.enterprise.example/owner/repo.git",
            &remote_command,
        );
        assert_eq!(
            &arguments[..4],
            [
                "-c",
                "credential.helper=!gh auth git-credential",
                "-c",
                "credential.useHttpPath=true",
            ]
        );
        assert!(!arguments.iter().any(|argument| argument.contains("token")));
    }
    assert_eq!(
        super::claim_remote_arguments("/tmp/remote.git", &["fetch", "/tmp/remote.git"]),
        ["fetch", "/tmp/remote.git"]
    );
    assert_eq!(
        validated_claim_remote("owner/repo", "https://github.enterprise.example/owner/repo")
            .expect("enterprise clone URL"),
        "https://github.enterprise.example/owner/repo.git"
    );
    assert!(validated_claim_remote(
        "owner/repo",
        "https://github.enterprise.example/other/repo"
    )
    .is_err());
}

#[test]
fn validated_claim_remote_rejects_unsafe_https_shapes() {
    for url in [
        "https://github.example/prefix/owner/repo",
        "https://github.example/owner/repo?token=secret",
        "https://github.example/owner/repo#fragment",
        "https://attacker@example.com/owner/repo",
        "https://github.example/not-owner/repo",
        "https://github.example/owner/repo/extra",
    ] {
        assert!(
            validated_claim_remote("owner/repo", url).is_err(),
            "accepted unsafe claim remote {url}"
        );
    }
}

#[test]
fn claim_ref_private_git_dirs_are_distinct_and_leave_target_git_metadata_unchanged() {
    // Break caught: claim fetches mutating FETCH_HEAD/objects in a shared target worktree.
    use std::os::unix::fs::PermissionsExt;

    let fixture = ClaimRefFixture::new("private-git");
    let state_root = fixture.root.join("private-state");
    let first = private_claim_git_dir_in(&state_root, "owner/repo")
        .expect("first private claim Git dir");
    let second = private_claim_git_dir_in(&state_root, "owner/repo")
        .expect("second private claim Git dir");
    assert_ne!(first.path, second.path);
    assert_eq!(
        std::fs::metadata(&first.path)
            .expect("private claim dir")
            .permissions()
            .mode()
            & 0o777,
        0o700
    );

    let target = &fixture.clients[0];
    let status_before = git_stdout(target, &["status", "--porcelain=v1"]);
    let refs_before = git_stdout(
        target,
        &["for-each-ref", "--format=%(refname) %(objectname)"],
    );
    let fetch_head = target.join(".git/FETCH_HEAD");
    let fetch_head_before = std::fs::read(&fetch_head).ok();
    let record = claim_record("worker-a", "claim-a", "claimed");
    assert!(matches!(
        advance_claim_ref_in(
            Path::new("git"),
            &first.path,
            fixture.remote.to_str().unwrap(),
            "owner/repo",
            42,
            None,
            &record,
        )
        .expect("private claim transition"),
        ClaimRefAdvance::Won(_)
    ));
    assert_eq!(
        git_stdout(target, &["status", "--porcelain=v1"]),
        status_before
    );
    assert_eq!(
        git_stdout(
            target,
            &["for-each-ref", "--format=%(refname) %(objectname)"]
        ),
        refs_before
    );
    assert_eq!(std::fs::read(&fetch_head).ok(), fetch_head_before);
}

#[test]
fn claim_mutation_paths_use_the_ref_ledger_or_fail_closed() {
    // Break caught: legacy state subcommands mutating comments/labels around the CAS ledger.
    let source = include_str!("../claim.rs");
    for function in [
        "release",
        "acquire_record",
        "refresh_claim_generation",
        "upsert_record",
        "clear",
        "recover_authoritative_stale_startup",
        "record_executor_result",
    ] {
        assert!(
            source_function(source, function).contains("advance_claim_ref"),
            "{function} must advance the claim ref before audit projection"
        );
    }
    assert!(source_function(source, "clear").contains("has_exact_claim_generation"));
    let recovery = source_function(source, "recover_authoritative_stale_startup");
    assert!(recovery.contains("\"available\""));
    assert!(recovery.contains("\"stale_startup_recovered\""));
    for function in [
        "record_executor_result_with_step",
        "reconcile_linked_pr_record",
    ] {
        let body = source_function(source, function);
        assert!(
            body.contains("read_claim_ref") && body.contains("upsert_record"),
            "{function} must route through the exact-ref mutation helper"
        );
        assert!(
            !body.contains("select_run_state"),
            "{function} must not derive a transition from stale audit projection"
        );
    }
    let lease = source_function(source, "has_active_executor_claim");
    assert!(lease.contains("record.updated_at"));
    assert!(lease.contains("record.ttl_seconds"));
}

#[test]
fn prepared_recovery_counts_toward_capacity_until_labels_converge() {
    let record = RunStateRecord::new(
        "owner/repo",
        42,
        "worker-a",
        "available",
        "",
        "",
        "stale_startup_recovered",
        Vec::new(),
        "2999-01-01T00:00:00Z",
        "2999-01-01T00:00:00Z",
        300,
    )
    .with_claim_id("claim-a");

    assert!(
        super::active_record_counts_toward_worker_capacity("owner/repo", 42, &record, 300)
            .expect("classify prepared recovery")
    );
}
