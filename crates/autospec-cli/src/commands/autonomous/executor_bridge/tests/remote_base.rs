// executor_bridge tests: remote / base — 8 cases.
//
// Split out of tests.rs; see the note in that file.

use super::super::{BridgePhase, MutationSnapshot};
use super::support_base::{git, git_stdout, GitFixture};
use super::support_invocation::{
    commit_implementation, implementation_proof_fixture, supervision_state,
};
use crate::commands::autonomous::executor_bridge as bridge;
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

#[test]
fn autonomous_executor_bridge_proves_descendant_base_drift_before_reconciliation() {
    // Break caught: an ordinary main advance stranding a committed implementation before
    // the later base-drift reconciliation phase can merge the new base.
    let (fixture, mut state, snapshot, closeout) = implementation_proof_fixture("proof-base-drift");
    commit_implementation(&state);
    fs::write(fixture.root.join("seed/base-drift.txt"), "drift\n").expect("base drift");
    git(&fixture.root.join("seed"), &["add", "base-drift.txt"]);
    git(&fixture.root.join("seed"), &["commit", "-m", "base drift"]);
    git(&fixture.root.join("seed"), &["push", "origin", "main"]);
    let remote_before = git_stdout(
        &fixture.root,
        &[
            "--git-dir",
            fixture.root.join("remote.git").to_str().unwrap(),
            "show-ref",
        ],
    );

    let proof = bridge::prove_implementation(
        &fixture.root.join("state/invocation.json"),
        &mut state,
        &snapshot,
        &closeout,
    )
    .expect("descendant base drift must remain eligible for later reconciliation");

    assert_eq!(proof.head_oid, state.head_oid.clone().unwrap());
    assert_eq!(state.phase, BridgePhase::ImplementationProven);
    assert_eq!(
        git_stdout(
            &fixture.root,
            &[
                "--git-dir",
                fixture.root.join("remote.git").to_str().unwrap(),
                "show-ref"
            ]
        ),
        remote_before
    );
}

#[test]
fn autonomous_executor_bridge_rejects_unrelated_base_drift_before_remote_mutation() {
    // Break caught: treating a force-pushed unrelated base as an ordinary main advance.
    let (fixture, mut state, snapshot, closeout) =
        implementation_proof_fixture("proof-unrelated-base-drift");
    commit_implementation(&state);
    let tree = git_stdout(&fixture.root.join("seed"), &["rev-parse", "HEAD^{tree}"]);
    let unrelated = Command::new("git")
        .args(["commit-tree", &tree, "-m", "unrelated base"])
        .current_dir(fixture.root.join("seed"))
        .output()
        .expect("create unrelated base");
    assert!(unrelated.status.success());
    let unrelated = String::from_utf8(unrelated.stdout).unwrap();
    let force_refspec = format!("{}:refs/heads/main", unrelated.trim());
    git(
        &fixture.root.join("seed"),
        &["push", "--force", "origin", &force_refspec],
    );
    let remote_before = git_stdout(
        &fixture.root,
        &[
            "--git-dir",
            fixture.root.join("remote.git").to_str().unwrap(),
            "show-ref",
        ],
    );

    let error = bridge::prove_implementation(
        &fixture.root.join("state/invocation.json"),
        &mut state,
        &snapshot,
        &closeout,
    )
    .expect_err("unrelated base drift must fail closed");

    assert!(error.contains("unrelated"), "{error}");
    assert_eq!(
        git_stdout(
            &fixture.root,
            &[
                "--git-dir",
                fixture.root.join("remote.git").to_str().unwrap(),
                "show-ref",
            ],
        ),
        remote_before
    );
}

#[test]
fn autonomous_executor_bridge_rejects_head_without_base_ancestry_before_remote_mutation() {
    // Break caught: an unrelated root commit replacing rather than extending the validated base.
    let (fixture, mut state, snapshot, closeout) =
        implementation_proof_fixture("proof-unbased-head");
    commit_implementation(&state);
    let tree = git_stdout(&state.identity.worktree, &["rev-parse", "HEAD^{tree}"]);
    let head = Command::new("git")
        .args(["commit-tree", &tree, "-m", "unrelated root"])
        .current_dir(&state.identity.worktree)
        .output()
        .expect("create unrelated root");
    assert!(head.status.success());
    let head = String::from_utf8(head.stdout).unwrap();
    git(&state.identity.worktree, &["reset", "--hard", head.trim()]);
    let remote_before = git_stdout(
        &fixture.root,
        &[
            "--git-dir",
            fixture.root.join("remote.git").to_str().unwrap(),
            "show-ref",
        ],
    );

    let error = bridge::prove_implementation(
        &fixture.root.join("state/invocation.json"),
        &mut state,
        &snapshot,
        &closeout,
    )
    .expect_err("unbased head must fail closed");

    assert!(error.contains("ancestor"), "{error}");
    assert_eq!(
        git_stdout(
            &fixture.root,
            &[
                "--git-dir",
                fixture.root.join("remote.git").to_str().unwrap(),
                "show-ref"
            ]
        ),
        remote_before
    );
}

#[test]
fn autonomous_executor_bridge_rejects_primary_head_mutation_before_remote_mutation() {
    // Break caught: the harness advancing the operator's primary checkout while its issue commit is valid.
    let (fixture, mut state, snapshot, closeout) =
        implementation_proof_fixture("proof-primary-head");
    commit_implementation(&state);
    fs::write(fixture.repo.join("primary.txt"), "mutated\n").expect("primary mutation");
    git(&fixture.repo, &["add", "primary.txt"]);
    git(&fixture.repo, &["commit", "-m", "primary mutation"]);
    let remote_before = git_stdout(
        &fixture.root,
        &[
            "--git-dir",
            fixture.root.join("remote.git").to_str().unwrap(),
            "show-ref",
        ],
    );

    let error = bridge::prove_implementation(
        &fixture.root.join("state/invocation.json"),
        &mut state,
        &snapshot,
        &closeout,
    )
    .expect_err("primary checkout mutation must fail closed");

    assert!(error.contains("primary checkout"), "{error}");
    assert_eq!(
        git_stdout(
            &fixture.root,
            &[
                "--git-dir",
                fixture.root.join("remote.git").to_str().unwrap(),
                "show-ref"
            ]
        ),
        remote_before
    );
}

#[test]
fn autonomous_executor_bridge_rejects_foreign_worktree_head_mutation_before_remote_mutation() {
    // Break caught: globally ignoring worktree HEAD lines hiding mutation of an unrelated worktree.
    let (fixture, mut state, _snapshot, closeout) =
        implementation_proof_fixture("proof-foreign-worktree-head");
    let foreign = fixture.root.join("foreign-worktree");
    git(
        &fixture.repo,
        &[
            "worktree",
            "add",
            "--detach",
            foreign.to_str().unwrap(),
            "origin/main",
        ],
    );
    let snapshot =
        MutationSnapshot::capture(&fixture.repo, &state.identity.branch).expect("snapshot");
    commit_implementation(&state);
    fs::write(foreign.join("foreign.txt"), "mutated\n").expect("foreign mutation");
    git(&foreign, &["add", "foreign.txt"]);
    git(&foreign, &["commit", "-m", "foreign mutation"]);
    let remote_before = git_stdout(
        &fixture.root,
        &[
            "--git-dir",
            fixture.root.join("remote.git").to_str().unwrap(),
            "show-ref",
        ],
    );

    let error = bridge::prove_implementation(
        &fixture.root.join("state/invocation.json"),
        &mut state,
        &snapshot,
        &closeout,
    )
    .expect_err("foreign worktree HEAD mutation must fail closed");

    assert!(error.contains("worktree registry"), "{error}");
    assert_eq!(
        git_stdout(
            &fixture.root,
            &[
                "--git-dir",
                fixture.root.join("remote.git").to_str().unwrap(),
                "show-ref"
            ]
        ),
        remote_before
    );
}

#[test]
fn autonomous_executor_bridge_allows_only_active_attributed_sibling_worktrees() {
    let (fixture, mut state, _snapshot, _closeout) =
        implementation_proof_fixture("proof-concurrent-sibling");
    let repository_scope = format!(
        "owner/repo-concurrent-{}-{}",
        std::process::id(),
        bridge::unix_now().expect("test clock")
    );
    state.identity.repository = repository_scope.clone();
    let state_dir = fixture.root.join("state/executor");
    bridge::ensure_private_directory(&state_dir).expect("private executor state");
    let current_state_path = state_dir.join("issue-42-current.json");
    let snapshot = MutationSnapshot::capture(&fixture.repo, &state.identity.branch)
        .expect("baseline snapshot")
        .with_sibling_state_dir(&current_state_path, &repository_scope)
        .expect("bind sibling state directory");

    let sibling_scope = bridge::executor_worktree_root()
        .join(bridge::safe_scope(&repository_scope).expect("safe sibling scope"));
    bridge::ensure_private_directory(&sibling_scope).expect("private sibling scope");
    let sibling_path = sibling_scope.join("issue-43");
    git(
        &fixture.repo,
        &[
            "worktree",
            "add",
            "-b",
            "feat/autonomous-issue-43",
            sibling_path.to_str().expect("sibling path"),
            "origin/main",
        ],
    );
    let mut sibling = state.clone();
    sibling.identity.issue = 43;
    sibling.identity.worker_id = "worker-43".to_string();
    sibling.identity.branch = "feat/autonomous-issue-43".to_string();
    sibling.identity.claim_id = "claim-43".to_string();
    sibling.identity.invocation_id = "invocation-43".to_string();
    sibling.identity.worktree = sibling_path
        .canonicalize()
        .expect("canonical sibling worktree");
    sibling.phase = BridgePhase::Implementing;
    bridge::write_invocation_atomic(&state_dir.join("issue-43-1111111111111111.json"), &sibling)
        .expect("persist sibling invocation");

    snapshot
        .verify_with_claim_lookup(&fixture.repo, &state.identity.branch, |candidate| {
            Ok(candidate.identity.issue == 43 && candidate.identity.claim_id == "claim-43")
        })
        .expect("active attributed sibling must not quarantine executor");
    let inactive = snapshot
        .verify_with_claim_lookup(&fixture.repo, &state.identity.branch, |_| Ok(false))
        .expect_err("persisted metadata without an active claim is insufficient");
    assert!(inactive.contains("worktree registry"), "{inactive}");

    let unowned_path = sibling_scope.join("issue-44");
    git(
        &fixture.repo,
        &[
            "worktree",
            "add",
            "-b",
            "feat/autonomous-issue-44",
            unowned_path.to_str().expect("unowned path"),
            "origin/main",
        ],
    );
    let unowned = snapshot
        .verify_with_claim_lookup(&fixture.repo, &state.identity.branch, |_| Ok(true))
        .expect_err("canonical-looking worktree without invocation must remain forbidden");
    assert!(unowned.contains("worktree registry"), "{unowned}");

    git(
        &fixture.repo,
        &[
            "worktree",
            "remove",
            "--force",
            unowned_path.to_str().unwrap(),
        ],
    );
    git(
        &fixture.repo,
        &[
            "worktree",
            "remove",
            "--force",
            sibling_path.to_str().unwrap(),
        ],
    );
    let _ = fs::remove_dir_all(sibling_scope);
}

#[test]
fn autonomous_executor_bridge_projects_only_active_attributed_sibling_remote_deltas() {
    let fixture = GitFixture::new("remote-concurrent-sibling");
    let mut current = supervision_state(&fixture);
    current.identity.repository = format!(
        "owner/remote-concurrent-{}-{}",
        std::process::id(),
        bridge::unix_now().expect("test clock")
    );
    let state_dir = fixture.root.join("state/executor");
    bridge::ensure_private_directory(&state_dir).expect("private executor state");
    let state_path = state_dir.join("issue-42-current.json");
    let mut sibling = current.clone();
    sibling.identity.issue = 43;
    sibling.identity.worker_id = "worker-43".to_string();
    sibling.identity.branch = "feat/autonomous-issue-43".to_string();
    sibling.identity.claim_id = "claim-43".to_string();
    sibling.identity.invocation_id = "invocation-43".to_string();
    sibling.head_oid = Some("b".repeat(40));
    sibling.pr = Some(43);
    sibling.phase = BridgePhase::DraftCreated;
    let closeout = "## Closeout report\n";
    sibling.closeout_digest = Some(bridge::sha256_hex(closeout.as_bytes()));
    bridge::write_invocation_atomic(&state_dir.join("issue-43-1111111111111111.json"), &sibling)
        .expect("persist sibling remote invocation");
    let baseline = bridge::RemoteMutationSnapshot {
        refs: BTreeMap::from([("refs/heads/main".to_string(), "a".repeat(40))]),
        pull_requests: Vec::new(),
    };
    let sibling_pr = bridge::OpenPullRequest {
        number: 43,
        body: bridge::canonical_pull_request_body(&sibling, closeout)
            .expect("canonical sibling PR body"),
        head_ref_name: sibling.identity.branch.clone(),
        head_ref_oid: "b".repeat(40),
        is_draft: true,
        base_ref_name: "main".to_string(),
    };
    let observed = bridge::RemoteMutationSnapshot {
        refs: BTreeMap::from([
            ("refs/heads/main".to_string(), "a".repeat(40)),
            (
                "refs/heads/feat/autonomous-issue-43".to_string(),
                "b".repeat(40),
            ),
        ]),
        pull_requests: vec![sibling_pr],
    };

    let normalized = bridge::normalize_authorized_sibling_remote_deltas_with_claim_lookup(
        &state_path,
        &current,
        &baseline,
        observed.clone(),
        |candidate| Ok(candidate.identity.claim_id == "claim-43"),
    )
    .expect("active attributed sibling remote delta");
    assert_eq!(normalized, baseline);

    let inactive = bridge::normalize_authorized_sibling_remote_deltas_with_claim_lookup(
        &state_path,
        &current,
        &baseline,
        observed.clone(),
        |_| Ok(false),
    )
    .expect("inactive sibling remains visible");
    assert_eq!(inactive, observed);

    let mut unowned = observed;
    unowned.refs.insert(
        "refs/heads/feat/autonomous-issue-44".to_string(),
        "c".repeat(40),
    );
    let unowned = bridge::normalize_authorized_sibling_remote_deltas_with_claim_lookup(
        &state_path,
        &current,
        &baseline,
        unowned,
        |_| Ok(true),
    )
    .expect("unowned remote delta remains visible");
    assert!(unowned
        .refs
        .contains_key("refs/heads/feat/autonomous-issue-44"));
}

#[test]
fn autonomous_executor_bridge_normalizes_descendant_base_remote_delta() {
    // Break caught: ordinary main progress invalidating a create-once remote snapshot before
    // the later Rust-owned base reconciliation phase can merge it.
    let fixture = GitFixture::new("remote-descendant-base");
    let current = supervision_state(&fixture);
    let state_dir = fixture.root.join("state/executor");
    bridge::ensure_private_directory(&state_dir).expect("private executor state");
    let baseline = bridge::RemoteMutationSnapshot {
        refs: BTreeMap::from([(
            "refs/heads/main".to_string(),
            current.identity.base_oid.clone(),
        )]),
        pull_requests: Vec::new(),
    };
    fs::write(fixture.root.join("seed/descendant.txt"), "descendant\n")
        .expect("write descendant base");
    git(&fixture.root.join("seed"), &["add", "descendant.txt"]);
    git(
        &fixture.root.join("seed"),
        &["commit", "-m", "descendant base"],
    );
    git(&fixture.root.join("seed"), &["push", "origin", "main"]);
    let descendant = git_stdout(&fixture.root.join("seed"), &["rev-parse", "HEAD"]);
    let observed = bridge::RemoteMutationSnapshot {
        refs: BTreeMap::from([("refs/heads/main".to_string(), descendant)]),
        pull_requests: Vec::new(),
    };

    let normalized = bridge::normalize_authorized_sibling_remote_deltas_with_claim_lookup(
        &state_dir.join("issue-42-current.json"),
        &current,
        &baseline,
        observed,
        |_| Ok(false),
    )
    .expect("descendant base advance must be deferred to reconciliation");

    assert_eq!(normalized, baseline);

    let tree = git_stdout(&fixture.root.join("seed"), &["rev-parse", "HEAD^{tree}"]);
    let unrelated = Command::new("git")
        .args(["commit-tree", &tree, "-m", "unrelated base"])
        .current_dir(fixture.root.join("seed"))
        .output()
        .expect("create unrelated base");
    assert!(unrelated.status.success());
    let unrelated = String::from_utf8(unrelated.stdout).unwrap();
    let force_refspec = format!("{}:refs/heads/main", unrelated.trim());
    git(
        &fixture.root.join("seed"),
        &["push", "--force", "origin", &force_refspec],
    );
    let observed = bridge::RemoteMutationSnapshot {
        refs: BTreeMap::from([("refs/heads/main".to_string(), unrelated.trim().to_string())]),
        pull_requests: Vec::new(),
    };

    let normalized = bridge::normalize_authorized_sibling_remote_deltas_with_claim_lookup(
        &state_dir.join("issue-42-current.json"),
        &current,
        &baseline,
        observed.clone(),
        |_| Ok(false),
    )
    .expect("unrelated base remains visible to draft admission");

    assert_eq!(normalized, observed);
}
