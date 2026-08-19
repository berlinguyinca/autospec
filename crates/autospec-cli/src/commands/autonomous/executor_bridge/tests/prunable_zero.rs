// executor_bridge tests: prunable / zero — 6 cases.
//
// Split out of tests.rs; see the note in that file.

use super::super::{provision_issue_worktree, resolve_base, ResolvedBase};
use super::support_base::{git, git_stdout, test_environment, GitFixture, TEST_SEQUENCE};
use super::support_invocation::prunable_zero_effect_branch_fixture;
use crate::commands::autonomous::executor_bridge as bridge;
use std::collections::BTreeMap;
use std::fs;
use std::sync::atomic::Ordering;

#[test]
fn autonomous_executor_bridge_prunable_zero_effect_branch_resumes_both_crash_boundaries() {
    let _environment = test_environment();
    for (mode, boundary) in ["prunable", "orphan"]
        .into_iter()
        .flat_map(|mode| [1, 2].map(move |boundary| (mode, boundary)))
    {
        let label = format!("{mode}-zero-effect-crash-{boundary}");
        let (fixture, scope, worktree, advanced) =
            prunable_zero_effect_branch_fixture(&label, false);
        if mode == "orphan" {
            git(&fixture.repo, &["worktree", "prune", "--expire", "now"]);
        }
        let recorded_head = git_stdout(
            &fixture.repo,
            &[
                "config",
                "--get",
                &format!("branch.{}.autospecBaseOid", worktree.branch),
            ],
        );
        bridge::PRUNABLE_RECLAIM_FAILPOINT.store(boundary, Ordering::SeqCst);
        let interrupted = bridge::provision_issue_worktree_for_claim(
            &fixture.repo,
            &scope,
            42,
            &advanced,
            Some(("claim-fresh", "invocation-fresh")),
        )
        .expect_err("interrupt zero-effect reclaim");
        assert!(interrupted.contains("injected executor prunable reclaim crash"));
        let intent = worktree
            .path
            .parent()
            .expect("scope root")
            .join("issue-42.prunable-reclaim-intent.json");
        assert!(intent.is_file());
        assert_eq!(
            git_stdout(
                &fixture.repo,
                &[
                    "rev-parse",
                    "--verify",
                    &format!("refs/heads/{}^{{commit}}", worktree.branch),
                ],
            ),
            recorded_head,
            "reclaim must never rewrite the branch ref"
        );
        assert_eq!(worktree.path.exists(), boundary == 2);

        if boundary == 2 {
            let competing = fixture.root.join(format!("{mode}-competing-worktree"));
            git(
                &fixture.repo,
                &[
                    "worktree",
                    "add",
                    "--force",
                    "--quiet",
                    competing.to_str().expect("competing worktree path"),
                    &worktree.branch,
                ],
            );
            let registry_before = git_stdout(&fixture.repo, &["worktree", "list", "--porcelain"]);
            let error = bridge::provision_issue_worktree_for_claim(
                &fixture.repo,
                &scope,
                42,
                &advanced,
                Some(("claim-fresh", "invocation-fresh")),
            )
            .expect_err("competing live branch registration must block resume");
            assert!(error.contains("conflicting live registration"), "{error}");
            assert_eq!(
                git_stdout(&fixture.repo, &["worktree", "list", "--porcelain"]),
                registry_before,
                "a rejected resume must not mutate the worktree registry"
            );
            assert!(intent.is_file(), "a rejected resume must retain intent");
            git(
                &fixture.repo,
                &[
                    "worktree",
                    "remove",
                    competing.to_str().expect("competing worktree path"),
                ],
            );
        }

        let resumed = bridge::provision_issue_worktree_for_claim(
            &fixture.repo,
            &scope,
            42,
            &advanced,
            Some(("claim-fresh", "invocation-fresh")),
        )
        .expect("resume zero-effect reclaim");
        assert!(!intent.exists());
        assert_eq!(
            git_stdout(&resumed.path, &["rev-parse", "--verify", "HEAD^{commit}"]),
            advanced.base_oid
        );
        let transfer: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(bridge::ownership_transfer_path(
                resumed.path.parent().expect("scope root"),
                42,
            ))
            .expect("fresh ownership"),
        )
        .expect("parse fresh ownership");
        assert_eq!(transfer["state"], "adopted");
        assert_eq!(transfer["to_claim_id"], "claim-fresh");

        git(
            &fixture.repo,
            &[
                "worktree",
                "remove",
                resumed.path.to_str().expect("worktree path"),
            ],
        );
        let _ = fs::remove_dir_all(resumed.path.parent().expect("scope root"));
    }
}

#[test]
fn autonomous_executor_bridge_prunable_zero_effect_branch_near_misses_do_not_mutate() {
    for mode in [
        "changed-head",
        "remote-branch",
        "config-mismatch",
        "transfer-mismatch",
    ] {
        let label = format!("prunable-zero-effect-near-{mode}");
        let (fixture, scope, worktree, advanced) =
            prunable_zero_effect_branch_fixture(&label, true);
        match mode {
            "changed-head" => git(
                &fixture.repo,
                &[
                    "update-ref",
                    &format!("refs/heads/{}", worktree.branch),
                    &advanced.base_oid,
                ],
            ),
            "remote-branch" => git(
                &fixture.repo,
                &[
                    "push",
                    "origin",
                    &format!(
                        "refs/heads/{}:refs/heads/{}",
                        worktree.branch, worktree.branch
                    ),
                ],
            ),
            "config-mismatch" => git(
                &fixture.repo,
                &[
                    "config",
                    &format!("branch.{}.autospecRepositoryPath", worktree.branch),
                    fixture.root.to_str().expect("foreign repository path"),
                ],
            ),
            "transfer-mismatch" => {
                let transfer_path = bridge::ownership_transfer_path(
                    worktree.path.parent().expect("scope root"),
                    42,
                );
                let mut transfer: serde_json::Value = serde_json::from_str(
                    &fs::read_to_string(&transfer_path).expect("read ownership transfer"),
                )
                .expect("parse ownership transfer");
                transfer["head_oid"] = serde_json::json!(advanced.base_oid);
                fs::write(
                    transfer_path,
                    format!(
                        "{}\n",
                        serde_json::to_string(&transfer)
                            .expect("serialize mismatched ownership transfer")
                    ),
                )
                .expect("write mismatched ownership transfer");
            }
            _ => unreachable!(),
        }
        let registry_before = git_stdout(&fixture.repo, &["worktree", "list", "--porcelain"]);
        let head_before = git_stdout(
            &fixture.repo,
            &[
                "rev-parse",
                "--verify",
                &format!("refs/heads/{}^{{commit}}", worktree.branch),
            ],
        );

        bridge::provision_issue_worktree_for_claim(
            &fixture.repo,
            &scope,
            42,
            &advanced,
            Some(("claim-fresh", "invocation-fresh")),
        )
        .expect_err("near miss must fail closed");

        assert_eq!(
            git_stdout(&fixture.repo, &["worktree", "list", "--porcelain"]),
            registry_before
        );
        assert_eq!(
            git_stdout(
                &fixture.repo,
                &[
                    "rev-parse",
                    "--verify",
                    &format!("refs/heads/{}^{{commit}}", worktree.branch),
                ],
            ),
            head_before
        );
        assert!(!worktree
            .path
            .parent()
            .expect("scope root")
            .join("issue-42.prunable-reclaim-intent.json")
            .exists());
        let _ = fs::remove_dir_all(worktree.path.parent().expect("scope root"));
    }
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_prunable_zero_effect_branch_path_near_misses_do_not_mutate() {
    use std::os::unix::fs::symlink;

    for mode in ["present", "symlink"] {
        let label = format!("prunable-zero-effect-path-{mode}");
        let (fixture, scope, worktree, advanced) =
            prunable_zero_effect_branch_fixture(&label, false);
        let head_before = git_stdout(
            &fixture.repo,
            &[
                "rev-parse",
                "--verify",
                &format!("refs/heads/{}^{{commit}}", worktree.branch),
            ],
        );
        if mode == "present" {
            fs::create_dir(&worktree.path).expect("create foreign path");
        } else {
            symlink(fixture.root.join("missing-target"), &worktree.path)
                .expect("create dangling worktree symlink");
        }

        bridge::provision_issue_worktree_for_claim(
            &fixture.repo,
            &scope,
            42,
            &advanced,
            Some(("claim-fresh", "invocation-fresh")),
        )
        .expect_err("present path must fail closed");

        assert_eq!(
            git_stdout(
                &fixture.repo,
                &[
                    "rev-parse",
                    "--verify",
                    &format!("refs/heads/{}^{{commit}}", worktree.branch),
                ],
            ),
            head_before
        );
        assert!(!worktree
            .path
            .parent()
            .expect("scope root")
            .join("issue-42.prunable-reclaim-intent.json")
            .exists());
        if mode == "present" {
            fs::remove_dir(&worktree.path).expect("remove foreign path");
        } else {
            fs::remove_file(&worktree.path).expect("remove worktree symlink");
        }
        let _ = fs::remove_dir_all(worktree.path.parent().expect("scope root"));
    }
}

#[test]
fn autonomous_executor_bridge_prunable_zero_effect_branch_foreign_registration_does_not_mutate() {
    let (fixture, scope, worktree, advanced) =
        prunable_zero_effect_branch_fixture("prunable-zero-effect-foreign-registration", true);
    git(
        &fixture.repo,
        &[
            "worktree",
            "remove",
            worktree.path.to_str().expect("worktree path"),
        ],
    );
    git(
        &fixture.repo,
        &["branch", "foreign-registration", &advanced.base_oid],
    );
    git(
        &fixture.repo,
        &[
            "worktree",
            "add",
            "--quiet",
            worktree.path.to_str().expect("worktree path"),
            "foreign-registration",
        ],
    );
    fs::remove_dir_all(&worktree.path).expect("make foreign registration prunable");
    let registry_before = git_stdout(&fixture.repo, &["worktree", "list", "--porcelain"]);
    let head_before = git_stdout(
        &fixture.repo,
        &[
            "rev-parse",
            "--verify",
            &format!("refs/heads/{}^{{commit}}", worktree.branch),
        ],
    );

    bridge::provision_issue_worktree_for_claim(
        &fixture.repo,
        &scope,
        42,
        &advanced,
        Some(("claim-fresh", "invocation-fresh")),
    )
    .expect_err("foreign registration must fail closed");

    assert_eq!(
        git_stdout(&fixture.repo, &["worktree", "list", "--porcelain"]),
        registry_before
    );
    assert_eq!(
        git_stdout(
            &fixture.repo,
            &[
                "rev-parse",
                "--verify",
                &format!("refs/heads/{}^{{commit}}", worktree.branch),
            ],
        ),
        head_before
    );
    assert!(!worktree
        .path
        .parent()
        .expect("scope root")
        .join("issue-42.prunable-reclaim-intent.json")
        .exists());
    let _ = fs::remove_dir_all(worktree.path.parent().expect("scope root"));
}

#[test]
fn autonomous_executor_bridge_retry_reconciles_preserved_wip_to_advanced_base() {
    // Break caught: validating current-base metadata before adopting exact retry WIP.
    let fixture = GitFixture::new("retry-base-adoption");
    let original = resolve_base(&fixture.repo, &BTreeMap::new()).expect("resolve original base");
    let scope = format!(
        "retry_base_{}_{}",
        std::process::id(),
        TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let worktree = provision_issue_worktree(&fixture.repo, &scope, 44, &original)
        .expect("provision original worktree");
    fs::write(worktree.path.join("implementation.txt"), "preserved WIP\n")
        .expect("write preserved implementation");
    git(&worktree.path, &["add", "implementation.txt"]);
    git(
        &worktree.path,
        &["commit", "-m", "feat: preserve retry WIP"],
    );
    let preserved_head = git_stdout(&worktree.path, &["rev-parse", "HEAD"]);
    git(
        &worktree.path,
        &[
            "push",
            "origin",
            &format!("{preserved_head}:refs/heads/{}", worktree.branch),
        ],
    );

    fs::write(fixture.repo.join("base-advance.txt"), "advanced\n").expect("write base advance");
    git(&fixture.repo, &["add", "base-advance.txt"]);
    git(&fixture.repo, &["commit", "-m", "feat: advance base"]);
    git(&fixture.repo, &["push", "origin", "main"]);
    let advanced = resolve_base(&fixture.repo, &BTreeMap::new()).expect("resolve advanced base");
    assert_ne!(advanced.base_oid, original.base_oid);

    let adopted = provision_issue_worktree(&fixture.repo, &scope, 44, &advanced)
        .expect("reconcile exact preserved WIP");
    let reconciled_head = git_stdout(&adopted.path, &["rev-parse", "HEAD"]);
    let restarted = provision_issue_worktree(&fixture.repo, &scope, 44, &advanced)
        .expect("adopt reconciled WIP after restart");

    assert_eq!(adopted.base_oid, advanced.base_oid);
    assert_eq!(restarted, adopted);
    assert_eq!(
        git_stdout(&restarted.path, &["rev-parse", "HEAD"]),
        reconciled_head
    );
    assert_eq!(
        git_stdout(
            &fixture.repo,
            &[
                "config",
                "--get",
                &format!("branch.{}.autospecBaseOid", adopted.branch),
            ],
        ),
        advanced.base_oid
    );
    assert_eq!(
        git_stdout(&adopted.path, &["show", "-s", "--format=%P", "HEAD"]),
        format!("{preserved_head} {}", advanced.base_oid)
    );
    assert_eq!(
        git_stdout(
            &fixture.root,
            &[
                "--git-dir",
                fixture.root.join("remote.git").to_str().unwrap(),
                "rev-parse",
                &format!("refs/heads/{}", adopted.branch),
            ],
        ),
        reconciled_head
    );
    assert!(
        fs::read_dir(adopted.path.parent().expect("scope root"))
            .expect("read scope root")
            .filter_map(Result::ok)
            .any(|entry| entry.file_name().to_string_lossy().contains("base-drift-")),
        "base transition must have a durable intent before metadata advances"
    );

    git(
        &fixture.repo,
        &[
            "worktree",
            "remove",
            adopted.path.to_str().expect("worktree path"),
        ],
    );
    let _ = fs::remove_dir_all(adopted.path.parent().expect("scope root"));
}

#[test]
fn autonomous_executor_bridge_rejects_adoption_from_a_different_base() {
    let fixture = GitFixture::new("wrong-base-adoption");
    let original = resolve_base(&fixture.repo, &BTreeMap::new()).expect("resolve main base");
    let other_oid = fixture.branch("other-base");
    let other = ResolvedBase {
        base_ref: "origin/other-base".to_string(),
        base_oid: other_oid,
        explore_mode: false,
    };
    let scope = format!(
        "wrong_base_{}",
        TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let worktree = provision_issue_worktree(&fixture.repo, &scope, 43, &original)
        .expect("provision from original base");

    let error = provision_issue_worktree(&fixture.repo, &scope, 43, &other)
        .expect_err("clean worktree from another base must not be adopted");

    assert!(error.contains("base"), "{error}");
    git(
        &fixture.repo,
        &[
            "worktree",
            "remove",
            worktree.path.to_str().expect("worktree path"),
        ],
    );
    let _ = fs::remove_dir_all(worktree.path.parent().expect("scope root"));
}
