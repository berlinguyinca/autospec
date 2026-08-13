// executor_bridge tests: ordered / publication — 5 cases.
//
// Split out of tests.rs; see the note in that file.

use crate::commands::autonomous::executor_bridge as bridge;
use super::super::BridgePhase;
use super::support_base::{git, git_stdout, test_environment, write_executable};
use super::support_invocation::implementation_proof_fixture;
use super::support_launch::{adapter_path, draft_pr_adapter_fixture, prepared_draft_transaction};
use std::fs;

#[cfg(unix)]
#[test]
fn bound_continuation_publication_is_ordered_and_restart_safe() {
    // Break caught: proactive receipts existed locally but never became ordered GitHub work.
    let _environment = test_environment();
    let (fixture, mut state, _, _) = implementation_proof_fixture("continuation-publication");
    let state_path = fixture.root.join("state/invocation.json");
    let event_log = fixture.root.join("events.jsonl");
    let store = fixture.root.join("continuation-gh");
    let bin = fixture.root.join("continuation-bin");
    fs::create_dir_all(store.join("issues")).expect("issue store");
    fs::create_dir_all(store.join("comments")).expect("comment store");
    fs::create_dir_all(&bin).expect("fake gh bin");
    fs::write(store.join("next"), "100").expect("issue sequence");
    write_executable(
        &bin.join("gh"),
        r#"#!/bin/sh
set -eu; printf '%s\n' "$*" >> "$GH_CALLS"
case "$1 $2" in
"issue view") issue=$3; case "$*" in
*"--json comments"*) test ! -f "$GH_STORE/comments/$issue" || cat "$GH_STORE/comments/$issue";;
*"--json body"*) cat "$GH_STORE/issues/$issue.body";;
*"--json state"*) printf 'OPEN\n';;
*) exit 64;;
  esac;;
"issue list") marker=
  while [ "$#" -gt 0 ]; do [ "$1" != "--search" ] || marker=$2; shift; done; marker=${marker% in:body}
  for body in "$GH_STORE"/issues/*.body; do if [ -f "$body" ] && grep -Fq "$marker" "$body"; then basename "$body" .body; fi; done;;
"issue create") body= title=; while [ "$#" -gt 0 ]; do case "$1" in --body) body=$2; shift;; --title) title=$2; shift;; esac; shift; done
  number=$(cat "$GH_STORE/next"); number=$((number + 1)); printf '%s' "$number" > "$GH_STORE/next"
  printf '%s\n' "$body" > "$GH_STORE/issues/$number.body"; printf '%s\n' "$title" > "$GH_STORE/issues/$number.title"; printf 'https://example.invalid/issues/%s\n' "$number";;
"issue comment") issue=$3; body=
  while [ "$#" -gt 0 ]; do [ "$1" != "--body" ] || { body=$2; shift; }; shift; done; printf '%s\n' "$body" > "$GH_STORE/comments/$issue";;
"api user") printf 'berlinguyinca\n';;
*) exit 64;;
esac
"#,
    );
    let previous_path = std::env::var_os("PATH");
    let previous_store = std::env::var_os("GH_STORE");
    let previous_calls = std::env::var_os("GH_CALLS");
    std::env::set_var("PATH", format!("{}:/usr/bin:/bin", bin.display()));
    std::env::set_var("GH_STORE", &store);
    std::env::set_var("GH_CALLS", store.join("calls"));
    let quoted = store.join("comments/42");
    fs::write(&quoted, "Ordinary issue quoting Part of #9").unwrap();
    assert_eq!(bridge::continuation_parent("owner/repo", 42).unwrap(), None);
    fs::remove_file(quoted).unwrap();
    for name in [
        "skills/a/SKILL.md",
        "skills/a/codex/prompt.md",
        "skills/a/opencode/agent.md",
        "skills/b/SKILL.md",
        "skills/b/codex/prompt.md",
        "skills/b/opencode/agent.md",
        "slice.txt",
    ] {
        fs::create_dir_all(state.identity.worktree.join(name).parent().unwrap())
            .expect("proactive directory");
        fs::write(state.identity.worktree.join(name), "changed\n").expect("proactive file");
        git(&state.identity.worktree, &["add", name]);
    }
    git(
        &state.identity.worktree,
        &["commit", "-m", "test: proactive continuation"],
    );
    let head = git_stdout(&state.identity.worktree, &["rev-parse", "HEAD"]);
    state.head_oid = Some(head.clone());
    let proof = bridge::ImplementationProof {
        head_oid: head,
        closeout_body: "## Closeout report\nResult: slice\nClaims: [verified] static slice\nProof type: static\nBefore/after: 0 to 1\nArtifacts: slice-0.txt; `git diff`\nScoped git status: slice files\nOne likely hidden failure: boundary\nCompleted criteria: [\"first current slice criterion with a reproducible command and exact artifact path\",\"second current slice criterion with a reproducible command and exact artifact path\"]\nUnmet criteria: [\"Run `scripts/continuation-second.sh` and verify café café café café café café café café café café café\",\"Run `scripts/continuation-third.sh` once\"]\n".into(),
    };
    bridge::require_continuation_checkpoint(&state_path, &event_log, &state, &proof, "", true)
        .expect("proactive continuation");
    let bound =
        bridge::PersistedInvocation::from_json(&fs::read_to_string(&state_path).unwrap()).unwrap();
    assert_eq!((bound.umbrella, bound.current_child), (Some(42), Some(101)));
    let parent = fs::read_to_string(store.join("comments/42")).unwrap();
    assert!(parent.contains("append-only-parent-extension"));
    let mut sibling_misbinding = bound.clone();
    sibling_misbinding.current_child = Some(102);
    assert!(bridge::recover_bound_continuation(&sibling_misbinding).is_err());
    let receipt =
        bridge::continuation_receipt_path(&state_path, &proof.head_oid).expect("receipt path");
    assert!(receipt.exists(), "seven files must plan a continuation");
    let bodies = [101, 102, 103]
        .map(|number| fs::read_to_string(store.join(format!("issues/{number}.body"))).unwrap());
    assert!(bodies[0].contains("ordinal=1"));
    assert!(bodies[1].contains("Depends on issue #101"));
    assert!(bodies[2].contains("Depends on issue #102"));
    fs::write(store.join("calls"), "").expect("clear calls");
    bridge::require_continuation_checkpoint(&state_path, &event_log, &state, &proof, "", true)
        .expect("publication restart");
    assert!(!fs::read_to_string(store.join("calls"))
        .expect("restart calls")
        .contains("issue create"));

    git(
        &state.identity.worktree,
        &[
            "push",
            "origin",
            &format!("{}:refs/heads/{}", proof.head_oid, state.identity.branch),
        ],
    );
    let mut rebound = bound.clone();
    rebound.phase = bridge::BridgePhase::DraftCreated;
    rebound.pr = Some(17);
    bridge::write_invocation_atomic(&state_path, &rebound).expect("bound draft state");
    fs::write(
        fixture.root.join("seed/continuation-base.txt"),
        "base drift\n",
    )
    .expect("advance continuation base");
    git(
        &fixture.root.join("seed"),
        &["add", "continuation-base.txt"],
    );
    git(
        &fixture.root.join("seed"),
        &["commit", "-m", "test: advance continuation base"],
    );
    git(&fixture.root.join("seed"), &["push", "origin", "main"]);
    let old_base = rebound.identity.base_oid.clone();
    assert!(
        bridge::reconcile_base_drift_with_refresh(&state_path, &mut rebound, || Ok(
            bridge::BridgeClaimOwnership::Refreshed { ttl_seconds: 60 }
        ))
        .expect("reconcile bound continuation base")
    );
    assert_ne!(rebound.identity.base_oid, old_base);
    let next_head = rebound.head_oid.clone().expect("reconciled head");
    let next_proof = bridge::ImplementationProof {
        head_oid: next_head,
        closeout_body: proof.closeout_body.clone(),
    };
    fs::write(store.join("calls"), "").expect("clear calls");
    bridge::require_continuation_checkpoint(
        &state_path,
        &event_log,
        &rebound,
        &next_proof,
        "",
        true,
    )
    .expect("bound exact-head recovery");
    let recovery_calls = fs::read_to_string(store.join("calls")).expect("recovery calls");
    assert!(!recovery_calls.contains("issue create"));
    assert_eq!(fs::read_to_string(store.join("next")).unwrap(), "103");

    let mut adverse = bridge::load_continuation_receipt(&state_path, &state).expect("receipt");
    adverse.unmet = vec!["Improve quality should feel nice".into()];
    adverse.content_digest = adverse.digest();
    fs::write(store.join("calls"), "").expect("clear calls");
    assert!(bridge::publish_continuation_children(&state_path, &adverse).is_err());
    assert!(!fs::read_to_string(store.join("calls"))
        .unwrap()
        .contains("issue create"));
    fs::write(
        store.join("comments/43"),
        "<!-- autospec-parent:10 -->\nChild issue #43 belongs to parent issue #10.\n",
    )
    .expect("existing parent marker");
    fs::write(
        store.join("comments/10"),
        "<!-- autospec-parent-decomposition:begin -->\nParent issue #10 was decomposed\n- #41\n- #43\n- #45\n<!-- autospec-parent-decomposition:end -->\n",
    )
    .expect("existing order");
    let mut existing = bridge::load_continuation_receipt(&state_path, &state).expect("receipt");
    existing.issue = 43;
    existing.content_digest = existing.digest();
    bridge::publish_continuation_children(&state_path, &existing).expect("parent extension");
    let first = fs::read_to_string(store.join("issues/104.body")).unwrap();
    assert!(first.contains("Depends on issue #43"));
    assert!(fs::read_to_string(store.join("issues/105.body"))
        .unwrap()
        .contains("Depends on issue #104"));
    assert!(fs::read_to_string(store.join("comments/43"))
        .unwrap()
        .contains("autospec-parent:10"));
    let title = fs::read_to_string(store.join("issues/102.title")).unwrap();
    assert!(title.trim().len() <= 120);

    let remote_before_hard = bridge::remote_head_refs(&fixture.repo).expect("remote refs");
    state.identity.issue = 44;
    let worktree = &state.identity.worktree;
    fs::write(worktree.join("hard.txt"), "changed\n".repeat(401)).expect("hard slice");
    git(worktree, &["add", "hard.txt"]);
    git(worktree, &["commit", "-m", "test: hard continuation"]);
    let hard_head = git_stdout(worktree, &["rev-parse", "HEAD"]);
    state.head_oid = Some(hard_head.clone());
    let hard_proof = bridge::ImplementationProof {
        head_oid: hard_head,
        closeout_body: proof.closeout_body.clone(),
    };
    let publish = || {
        bridge::require_continuation_checkpoint(
            &state_path,
            &event_log,
            &state,
            &hard_proof,
            "",
            true,
        )
    };
    fs::write(store.join("calls"), "").expect("clear calls");
    let error = publish().expect_err("hard continuation invariant");
    assert!(error
        .to_string()
        .contains("oversized continuation checkpoint"));
    let children = bridge::continuation_child_list(&state.identity.repository, 44)
        .expect("authoritative hard child order");
    assert_eq!(children, [106, 107, 108]);
    for (ordinal, child) in children.iter().enumerate() {
        let body =
            fs::read_to_string(store.join(format!("issues/{child}.body"))).expect("hard child");
        assert!(body.contains(&format!("ordinal={}", ordinal + 1)));
    }
    let calls = fs::read_to_string(store.join("calls")).expect("hard calls");
    assert_eq!(calls.matches("issue create").count(), 3);
    assert!(!calls.contains("pr create"));
    let refs = bridge::remote_head_refs(&fixture.repo).expect("remote refs");
    assert_eq!(refs, remote_before_hard);
    fs::write(store.join("calls"), "").expect("clear calls");
    assert!(publish().is_err());
    assert!(!fs::read_to_string(store.join("calls"))
        .expect("hard restart calls")
        .contains("issue create"));
    for (key, previous) in [
        ("PATH", previous_path),
        ("GH_STORE", previous_store),
        ("GH_CALLS", previous_calls),
    ] {
        match previous {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }
}

#[test]
fn autonomous_executor_bridge_reuse_lens_follows_exact_environment_contract() {
    // Break caught: draft publication enabling reuse-only detectors when the shell gate is off.
    let original = std::env::var_os("AUTOSPEC_REUSE_LENS");
    std::env::remove_var("AUTOSPEC_REUSE_LENS");
    assert!(!bridge::implementation_lint_options().enable_reuse_lens);
    std::env::set_var("AUTOSPEC_REUSE_LENS", "off");
    assert!(!bridge::implementation_lint_options().enable_reuse_lens);
    std::env::set_var("AUTOSPEC_REUSE_LENS", "1");
    assert!(bridge::implementation_lint_options().enable_reuse_lens);
    match original {
        Some(value) => std::env::set_var("AUTOSPEC_REUSE_LENS", value),
        None => std::env::remove_var("AUTOSPEC_REUSE_LENS"),
    }
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_lints_the_proven_tree_not_mutable_worktree() {
    // Break caught: an uncommitted allow marker suppressing an unsafe committed diff.
    let (fixture, mut state, snapshot, closeout) =
        implementation_proof_fixture("draft-lint-exact-tree");
    let state_path = fixture.root.join("state/invocation.json");
    let adapter = draft_pr_adapter_fixture(&fixture, &state_path, "[]");
    state.phase = BridgePhase::Pending;
    bridge::write_invocation_atomic(&state_path, &state).expect("pending invocation");
    bridge::RemoteMutationSnapshot::capture_and_persist(&state_path, &mut state, &adapter)
        .expect("prelaunch remote");
    state.phase = BridgePhase::ImplementationComplete;
    let unsafe_path = state.identity.worktree.join("unsafe.rs");
    let unsafe_source = ["fn unsafe_change() { ev", "al(input); }\n"].concat();
    fs::write(&unsafe_path, unsafe_source).expect("unsafe change");
    git(&state.identity.worktree, &["add", "."]);
    git(
        &state.identity.worktree,
        &["commit", "-m", "feat: unsafe fixture"],
    );
    let proof = bridge::prove_implementation(&state_path, &mut state, &snapshot, &closeout)
        .expect("prove unsafe committed implementation");
    let mutable_allow_marker = [
        "// linter:allow-SECURITY fixture marker is not committed\nev",
        "al(input);\n",
    ]
    .concat();
    fs::write(&unsafe_path, mutable_allow_marker).expect("mutable allow marker");

    let error = bridge::push_and_create_draft(
        &state_path,
        &mut state,
        &proof,
        "Implement issue",
        "## Implementation outline\n\n- unsafe.rs\n- .autospec/closeout.md\n",
        &adapter,
    )
    .expect_err("mutable worktree cannot suppress exact-tree lint");

    assert!(error.contains("worktree must be clean"), "{error}");
    assert!(git_stdout(
        &fixture.root,
        &[
            "--git-dir",
            fixture.root.join("remote.git").to_str().unwrap(),
            "show-ref",
        ],
    )
    .lines()
    .all(|line| !line.ends_with(&format!("refs/heads/{}", state.identity.branch))));
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_rejects_nonexact_authoritative_drafts() {
    // Break caught: treating partial PR identity or additional PRs as the one owned draft.
    for variant in [
        "missing",
        "multiple",
        "wrong-head",
        "wrong-base",
        "ready",
        "wrong-body",
        "extra",
    ] {
        let mut prepared = prepared_draft_transaction(&format!("draft-invalid-{variant}"));
        let created_path = adapter_path(&prepared.adapter, "GH_CREATED_PR");
        let exact: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&created_path).unwrap()).unwrap();
        let mut pull_requests = exact.as_array().unwrap().clone();
        match variant {
            "missing" => pull_requests.clear(),
            "multiple" => {
                let mut duplicate = pull_requests[0].clone();
                duplicate["number"] = 18.into();
                pull_requests.push(duplicate);
            }
            "wrong-head" => {
                pull_requests[0]["headRefOid"] = "ffffffffffffffffffffffffffffffffffffffff".into()
            }
            "wrong-base" => pull_requests[0]["baseRefName"] = "other".into(),
            "ready" => pull_requests[0]["isDraft"] = false.into(),
            "wrong-body" => pull_requests[0]["body"] = "## Closeout report\n".into(),
            "extra" => {
                pull_requests.push(serde_json::json!({
                    "number": 18,
                    "body": "Closes #99",
                    "headRefName": "foreign",
                    "headRefOid": "ffffffffffffffffffffffffffffffffffffffff",
                    "isDraft": true,
                    "baseRefName": "main"
                }));
            }
            _ => unreachable!(),
        }
        fs::write(
            &created_path,
            serde_json::Value::Array(pull_requests).to_string(),
        )
        .expect("invalid authoritative PR fixture");

        let error = bridge::push_and_create_draft(
            &prepared.state_path,
            &mut prepared.state,
            &prepared.proof,
            "Implement issue",
            "## Implementation outline\n\n- implementation.txt\n- .autospec/executor-closeout.md\n",
            &prepared.adapter,
        )
        .expect_err("nonexact draft must fail closed");

        assert!(
            error.contains("exact") || error.contains("extra"),
            "{variant}: {error}"
        );
        assert_eq!(prepared.state.phase, BridgePhase::DraftCreating);
    }
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_rejects_prelaunch_remote_snapshot_drift() {
    // Break caught: harness-created extra refs or PRs being mistaken for Rust-owned mutations.
    let mut branch = prepared_draft_transaction("draft-extra-branch");
    git(
        &branch.fixture.root.join("seed"),
        &["push", "origin", "HEAD:refs/heads/foreign"],
    );
    let error = bridge::push_and_create_draft(
        &branch.state_path,
        &mut branch.state,
        &branch.proof,
        "Implement issue",
        "## Implementation outline\n\n- implementation.txt\n- .autospec/closeout.md\n",
        &branch.adapter,
    )
    .expect_err("extra branch must fail closed");
    assert!(error.contains("mutated remote"), "{error}");

    let mut tag = prepared_draft_transaction("draft-extra-tag");
    git(&tag.fixture.root.join("seed"), &["tag", "foreign-tag"]);
    git(
        &tag.fixture.root.join("seed"),
        &["push", "origin", "refs/tags/foreign-tag"],
    );
    let error = tag
        .publish()
        .expect_err("extra safe-namespace tag must fail closed");
    assert!(error.contains("mutated remote"), "{error}");

    let mut pull_request = prepared_draft_transaction("draft-extra-pr");
    fs::write(
        adapter_path(&pull_request.adapter, "GH_PR_STATE"),
        r#"[{"number":99,"body":"Closes #99","headRefName":"foreign","headRefOid":"ffffffffffffffffffffffffffffffffffffffff","isDraft":true,"baseRefName":"main"}]"#,
    )
    .expect("extra PR");
    let error = bridge::push_and_create_draft(
        &pull_request.state_path,
        &mut pull_request.state,
        &pull_request.proof,
        "Implement issue",
        "## Implementation outline\n\n- implementation.txt\n- .autospec/closeout.md\n",
        &pull_request.adapter,
    )
    .expect_err("extra PR must fail closed");
    assert!(error.contains("mutated remote"), "{error}");

    let mut stale = prepared_draft_transaction("draft-stale-snapshot");
    let snapshot_path = stale.state_path.with_extension("prelaunch-remote.json");
    let mut snapshot: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&snapshot_path).unwrap()).unwrap();
    snapshot["identity"]["invocation_id"] = "stale-invocation".into();
    fs::write(&snapshot_path, snapshot.to_string()).expect("tamper snapshot identity");
    let error = bridge::push_and_create_draft(
        &stale.state_path,
        &mut stale.state,
        &stale.proof,
        "Implement issue",
        "## Implementation outline\n\n- implementation.txt\n- .autospec/closeout.md\n",
        &stale.adapter,
    )
    .expect_err("stale remote snapshot must fail closed");
    assert!(error.contains("identity"), "{error}");

    let mut during_create = prepared_draft_transaction("draft-ref-during-create");
    during_create.adapter.environment.insert(
        "GH_MUTATE_REF".into(),
        during_create.proof.head_oid.clone().into(),
    );
    let error = during_create
        .publish()
        .expect_err("remote ref mutation during create must fail closed");
    assert!(error.contains("remote refs"), "{error}");
    assert_ne!(during_create.state.phase, BridgePhase::DraftCreated);
}
