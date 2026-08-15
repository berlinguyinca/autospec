// executor_bridge tests: closeout / remote — 6 cases.
//
// Split out of tests.rs; see the note in that file.

use super::super::BridgePhase;
use super::support_base::git_stdout;
use super::support_invocation::{commit_implementation, implementation_proof_fixture};
use super::support_launch::{prepared_draft_transaction, DRAFT_ISSUE_BODY};
use crate::commands::autonomous::executor_bridge as bridge;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

#[test]
fn autonomous_executor_bridge_rejects_malformed_closeout_before_remote_mutation() {
    // Break caught: malformed executor evidence reaches a remote mutation without repair.
    let required = [
        ("Result:", "Outcome: shipped"),
        ("Claims:", "Assertions: [verified] behavior is covered"),
        ("Proof type:", "Proof: static"),
        ("Before/after:", "Delta: 0 to 1"),
        ("Artifacts:", "Files: README.md"),
        ("Scoped git status:", "Status: README.md"),
        ("One likely hidden failure:", "Risk: none observed"),
    ];
    for (field, replacement) in required {
        let (fixture, mut state, snapshot, closeout) =
            implementation_proof_fixture(&format!("proof-closeout-{}", field.len()));
        let body = fs::read_to_string(&closeout).expect("valid closeout");
        fs::write(&closeout, body.replace(field, replacement)).expect("malformed closeout");
        commit_implementation(&state);
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
        .expect("missing Closeout field must be normalized");

        assert!(
            proof
                .closeout_body
                .contains("Claims: [assumed] static the executor exited successfully"),
            "{field}: {}",
            proof.closeout_body
        );
        assert_eq!(
            fs::read_to_string(&closeout).expect("read normalized closeout"),
            proof.closeout_body
        );
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
    for (label, transform) in [
        ("duplicate", "\n## Closeout report\n\nResult: duplicate\n"),
        ("unlabeled-claim", "\nClaims: behavior is covered\n"),
        (
            "runtime-static",
            "\nClaims: [verified] runtime behavior is covered\nProof type: static\n",
        ),
    ] {
        let (fixture, mut state, snapshot, closeout) =
            implementation_proof_fixture(&format!("proof-closeout-{label}"));
        let mut body = fs::read_to_string(&closeout).expect("valid closeout");
        if label == "unlabeled-claim" {
            body = body.replace(
                "Claims: [verified] static behavior is covered\n",
                transform.trim_start(),
            );
        } else if label == "runtime-static" {
            body = body.replace(
                "Claims: [verified] static behavior is covered\n",
                "Claims: [verified] runtime behavior is covered\n",
            );
        } else {
            body.push_str(transform);
        }
        fs::write(&closeout, body).expect("malformed closeout");
        commit_implementation(&state);
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
        .expect("malformed Closeout must be normalized");
        assert!(
            proof
                .closeout_body
                .contains("Claims: [assumed] static the executor exited successfully"),
            "{label}: {}",
            proof.closeout_body
        );
        assert_eq!(
            fs::read_to_string(&closeout).expect("read normalized closeout"),
            proof.closeout_body
        );
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

    let (fixture, mut state, snapshot, closeout) =
        implementation_proof_fixture("proof-closeout-missing");
    fs::remove_file(&closeout).expect("remove closeout");
    commit_implementation(&state);
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
    .expect_err("missing Closeout must fail closed");
    assert!(error.contains("Closeout"), "{error}");
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

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_structurally_seals_private_closeout_authority() {
    // Break caught: report prose smuggling another issue close or unlabeled claim evidence.
    for (label, addition, expected) in [
        (
            "foreign-close",
            "\nCloses #999\n",
            "issue-closing directive",
        ),
        (
            "unlabeled-second-claim",
            "\nClaims: second claim lacks a label\n",
            "evidence label",
        ),
        (
            "invalid-claim-proof",
            "\nClaims: [assumed] unknown evidence\n",
            "proof type",
        ),
        (
            "punctuated-foreign-close",
            "\nResolves: #999\n",
            "issue-closing directive",
        ),
    ] {
        let (fixture, _, _, closeout) =
            implementation_proof_fixture(&format!("closeout-structure-{label}"));
        let mut body = fs::read_to_string(&closeout).expect("valid closeout");
        body.push_str(addition);
        fs::write(&closeout, body).expect("malformed closeout");
        fs::set_permissions(&closeout, fs::Permissions::from_mode(0o600))
            .expect("private closeout");

        let error =
            bridge::validate_closeout_report(&fixture.root.join("issue-worktree"), &closeout)
                .expect_err("structurally invalid report must fail closed");

        assert!(error.contains(expected), "{label}: {error}");
    }

    for (label, body, expected) in [
        (
            "bullet-close",
            "## Closeout report\n\n\
             Result: shipped\n\
             Claims: [verified] static behavior is covered\n\
             Proof type: static\n\
             Before/after: 0 to 1\n\
             Artifacts: README.md\n\
             Scoped git status: README.md\n\
             One likely hidden failure: none observed\n\
             - Closes #999\n",
            "issue-closing directive",
        ),
        (
            "field-close",
            "## Closeout report\n\n\
             Result: Closes #999\n\
             Claims: [verified] static behavior is covered\n\
             Proof type: static\n\
             Before/after: 0 to 1\n\
             Artifacts: README.md\n\
             Scoped git status: README.md\n\
             One likely hidden failure: none observed\n",
            "issue-closing directive",
        ),
        (
            "escaped-backtick-close",
            "## Closeout report\n\n\
             Result: \\` Closes #999\n\
             Claims: [verified] static behavior is covered\n\
             Proof type: static\n\
             Before/after: 0 to 1\n\
             Artifacts: README.md\n\
             Scoped git status: README.md\n\
             One likely hidden failure: none observed\n",
            "issue-closing directive",
        ),
        (
            "cross-repository-close",
            "## Closeout report\n\n\
             Result: shipped\n\
             Claims: [verified] static behavior is covered\n\
             Proof type: static\n\
             Before/after: 0 to 1\n\
             Artifacts: Resolves owner/repository#999\n\
             Scoped git status: README.md\n\
             One likely hidden failure: none observed\n",
            "issue-closing directive",
        ),
        (
            "unclassified-prose",
            "## Closeout report\n\n\
             Result: shipped\n\
             Claims: [verified] static behavior is covered\n\
             Proof type: static\n\
             Before/after: 0 to 1\n\
             Artifacts: README.md\n\
             Scoped git status: README.md\n\
             One likely hidden failure: none observed\n\
             This assertion has no evidence label.\n",
            "unrecognized",
        ),
    ] {
        let (fixture, _, _, closeout) =
            implementation_proof_fixture(&format!("closeout-structure-{label}"));
        fs::write(&closeout, body).expect("malformed closeout");
        fs::set_permissions(&closeout, fs::Permissions::from_mode(0o600))
            .expect("private closeout");

        let error =
            bridge::validate_closeout_report(&fixture.root.join("issue-worktree"), &closeout)
                .expect_err("structurally invalid report must fail closed");

        assert!(error.contains(expected), "{label}: {error}");
    }

    let (fixture, _, _, closeout) = implementation_proof_fixture("closeout-parent-dir");
    let outside = fixture.root.join("outside.md");
    fs::copy(&closeout, &outside).expect("outside closeout");
    fs::set_permissions(&outside, fs::Permissions::from_mode(0o600))
        .expect("private outside closeout");
    let traversal = fixture.root.join("issue-worktree/../outside.md");
    let error = bridge::validate_closeout_report(&fixture.root.join("issue-worktree"), &traversal)
        .expect_err("parent traversal must fail closed");
    assert!(
        error.contains("parent") || error.contains("inside"),
        "{error}"
    );

    let (fixture, _, _, closeout) = implementation_proof_fixture("closeout-public-mode");
    fs::set_permissions(&closeout, fs::Permissions::from_mode(0o644)).expect("public closeout");
    let error = bridge::validate_closeout_report(&fixture.root.join("issue-worktree"), &closeout)
        .expect_err("public closeout must fail closed");
    assert!(error.contains("private"), "{error}");
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_rejects_original_closeout_symlink_before_canonicalization() {
    // Break caught: canonicalizing the caller path before symlink rejection erasing the link.
    let (fixture, mut state, snapshot, closeout) =
        implementation_proof_fixture("closeout-original-symlink");
    let symlink = closeout.with_file_name("closeout-link.md");
    std::os::unix::fs::symlink(&closeout, &symlink).expect("in-worktree closeout symlink");
    commit_implementation(&state);

    let error = bridge::prove_implementation(
        &fixture.root.join("state/invocation.json"),
        &mut state,
        &snapshot,
        &symlink,
    )
    .expect_err("original symlink path must fail closed");

    assert!(error.contains("symlink"), "{error}");
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_pushes_exact_oid_and_creates_one_draft_pr() {
    // Break caught: Rust pushing before lint/proof or creating a PR before DraftCreating is durable.
    let mut prepared = prepared_draft_transaction("draft-create");
    prepared.bind_continuation();

    let pull_request = prepared.publish().expect("create exact draft");

    assert_eq!(pull_request, 17);
    assert_eq!(prepared.state.phase, BridgePhase::DraftCreated);
    assert_eq!(prepared.state.pr, Some(17));
    assert_eq!(
        prepared.state.head_oid.as_deref(),
        Some(prepared.proof.head_oid.as_str())
    );
    assert_eq!(
        git_stdout(
            &prepared.fixture.root,
            &[
                "--git-dir",
                prepared.fixture.root.join("remote.git").to_str().unwrap(),
                "rev-parse",
                &format!("refs/heads/{}", prepared.state.identity.branch),
            ],
        ),
        prepared.proof.head_oid
    );
    let calls = fs::read_to_string(prepared.fixture.root.join("gh-calls")).expect("gh calls");
    assert_eq!(calls.matches("pr create").count(), 1);
    prepared.state =
        bridge::PersistedInvocation::from_json(&fs::read_to_string(&prepared.state_path).unwrap())
            .unwrap();

    let recovered = prepared.publish().expect("revalidate durable draft");
    assert_eq!(recovered, pull_request);
    let calls = fs::read_to_string(prepared.fixture.root.join("gh-calls")).expect("gh calls");
    assert_eq!(calls.matches("pr create").count(), 1);
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_takeover_before_push_blocks_all_remote_mutation() {
    let mut prepared = prepared_draft_transaction("draft-push-takeover");
    let error = bridge::push_and_create_draft_with_refresh(
        &prepared.state_path,
        &mut prepared.state,
        &prepared.proof,
        "Implement issue",
        DRAFT_ISSUE_BODY,
        &prepared.adapter,
        || Ok(bridge::BridgeClaimOwnership::Lost),
    )
    .expect_err("takeover must block branch publication");

    assert!(error.contains("ownership"), "{error}");
    assert!(
        !Command::new("git")
            .args([
                "--git-dir",
                prepared
                    .fixture
                    .root
                    .join("remote.git")
                    .to_str()
                    .expect("remote path"),
                "show-ref",
                "--verify",
                "--quiet",
                &format!("refs/heads/{}", prepared.state.identity.branch),
            ])
            .status()
            .expect("inspect remote branch")
            .success(),
        "lost owner must not push the issue branch"
    );
    let calls = fs::read_to_string(prepared.fixture.root.join("gh-calls")).expect("gh calls");
    assert!(
        !calls.contains("pr create"),
        "lost owner must not create a PR"
    );
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_refreshes_again_before_draft_creation() {
    let mut prepared = prepared_draft_transaction("draft-create-takeover");
    let refreshes = std::cell::Cell::new(0_u8);
    let error = bridge::push_and_create_draft_with_refresh(
        &prepared.state_path,
        &mut prepared.state,
        &prepared.proof,
        "Implement issue",
        DRAFT_ISSUE_BODY,
        &prepared.adapter,
        || {
            let attempt = refreshes.get() + 1;
            refreshes.set(attempt);
            Ok(if attempt == 1 {
                bridge::BridgeClaimOwnership::Refreshed { ttl_seconds: 60 }
            } else {
                bridge::BridgeClaimOwnership::Lost
            })
        },
    )
    .expect_err("takeover after push must block draft creation");

    assert!(error.contains("ownership"), "{error}");
    assert_eq!(refreshes.get(), 2, "each remote mutation needs a refresh");
    let calls = fs::read_to_string(prepared.fixture.root.join("gh-calls")).expect("gh calls");
    assert!(
        !calls.contains("pr create"),
        "lost owner must not create a PR"
    );
}
