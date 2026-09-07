//! Issue #3565 — real-git end to end for the integration phase.
//!
//! Builds a temporary repository in which:
//! - `feat/additive` adds a different line to `app.txt` than trunk does →
//!   an additive conflict that must union-resolve and land;
//! - `feat/semantic` rewrites the same line trunk rewrites → a genuine
//!   semantic disagreement that must halt, report the hunk, and leave the
//!   branch untouched (same commit sha as before the run).
//!
//! Also verifies: the repo is left clean (no in-progress rebase), the
//! trunk advances only with landed branches, and a failing verifier names
//! its branch and blocks the land.

use autospec_core::integration::{
    BranchOutcome, GitVcs, IntegrationPhase, NoopVerifier, Verifier, VerifyOutcome,
};
use std::path::{Path, PathBuf};
use std::process::Command;

static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("failed to run git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn new_repo(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "autospec-int-{}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst),
        tag
    ));
    std::fs::create_dir_all(&dir).expect("create temp repo");
    git(&dir, &["init", "-q", "-b", "main"]);
    git(&dir, &["config", "user.email", "autospec-test@example.com"]);
    git(&dir, &["config", "user.name", "autospec test"]);
    dir
}

/// Build the scenario: trunk `main`, `feat/additive` (additive vs trunk),
/// `feat/semantic` (semantic vs trunk).
fn scenario(dir: &Path) {
    std::fs::write(dir.join("app.txt"), "alpha\n").unwrap();
    git(dir, &["add", "app.txt"]);
    git(dir, &["commit", "-q", "-m", "base"]);

    // feat/additive: adds beta-additive.
    git(dir, &["checkout", "-q", "-b", "feat/additive"]);
    std::fs::write(dir.join("app.txt"), "alpha\nbeta-additive\n").unwrap();
    git(dir, &["commit", "-qam", "additive field"]);

    // feat/semantic: rewrites alpha (created from the base commit).
    git(dir, &["checkout", "-q", "main"]);
    git(dir, &["checkout", "-q", "-b", "feat/semantic"]);
    std::fs::write(dir.join("app.txt"), "ALPHA-REWRITTEN\n").unwrap();
    git(dir, &["commit", "-qam", "semantic rewrite"]);

    // Trunk: adds beta-main.
    git(dir, &["checkout", "-q", "main"]);
    std::fs::write(dir.join("app.txt"), "alpha\nbeta-main\n").unwrap();
    git(dir, &["commit", "-qam", "main field"]);

    // Leave the repo on trunk so the phase starts from a clean trunk.
}

#[test]
fn git_backend_lands_additive_and_halts_semantic() {
    let dir = new_repo("git-e2e");
    scenario(&dir);
    let semantic_before = git(&dir, &["rev-parse", "feat/semantic"]);

    let vcs = GitVcs::new(
        dir.clone(),
        "main".to_string(),
        vec!["feat/additive".to_string(), "feat/semantic".to_string()],
    );
    let report =
        IntegrationPhase::new(vcs, autospec_core::integration::UnionResolver, NoopVerifier).run();

    let additive = &report.results[0];
    let semantic = &report.results[1];
    assert_eq!(additive.branch, "feat/additive");
    assert_eq!(
        additive.outcome,
        BranchOutcome::Landed,
        "additive branch must land, got {:?}",
        additive.outcome
    );
    match &semantic.outcome {
        BranchOutcome::HaltedSemantic { conflicts } => {
            assert_eq!(conflicts.len(), 1, "{conflicts:?}");
            assert_eq!(conflicts[0].file, "app.txt");
            assert_eq!(conflicts[0].start, 0);
            assert_eq!(conflicts[0].ancestor, vec!["alpha".to_string()]);
            assert!(conflicts[0].reason.contains("branch side"));
        }
        other => panic!("expected HaltedSemantic, got {other:?}"),
    }

    // The trunk advanced with the additive branch and nothing else.
    let main_tree = git(&dir, &["show", "main:app.txt"]);
    assert!(main_tree.contains("alpha\n"), "{main_tree:?}");
    assert!(main_tree.contains("beta-additive\n"), "{main_tree:?}");
    assert!(main_tree.contains("beta-main\n"), "{main_tree:?}");
    assert!(!main_tree.contains("ALPHA-REWRITTEN"), "{main_tree:?}");

    // The halted branch is untouched: same commit as before the run.
    assert_eq!(git(&dir, &["rev-parse", "feat/semantic"]), semantic_before);

    // The repo is left clean: no in-progress rebase, no dirty files.
    let porcelain = git(&dir, &["status", "--porcelain"]);
    assert_eq!(porcelain, "", "repo must be left clean: {porcelain:?}");
    let rebase_dir = dir.join(".git/rebase-merge");
    assert!(!rebase_dir.exists(), "no in-progress rebase must remain");
    // The repository is handed back on the trunk, not on the halted branch.
    let head = git(&dir, &["symbolic-ref", "--short", "HEAD"]);
    assert_eq!(
        head.trim(),
        "main",
        "repo must be left on the trunk: {head:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// A verifier that fails for the named branch.
struct FailingVerifierFor {
    branch: String,
}

impl Verifier for FailingVerifierFor {
    fn verify(&self, branch: &str) -> VerifyOutcome {
        if branch == self.branch {
            VerifyOutcome::Failed {
                reason: "unit tests failed (3 failures)".to_string(),
            }
        } else {
            VerifyOutcome::Passed
        }
    }
}

#[test]
fn git_backend_verification_failure_names_branch_and_blocks_land() {
    let dir = new_repo("git-verify");
    scenario(&dir);
    let main_before = git(&dir, &["rev-parse", "main"]);

    // Only the additive branch is integrated, and the verifier fails for it.
    let vcs = GitVcs::new(
        dir.clone(),
        "main".to_string(),
        vec!["feat/additive".to_string()],
    );
    let report = IntegrationPhase::new(
        vcs,
        autospec_core::integration::UnionResolver,
        FailingVerifierFor {
            branch: "feat/additive".to_string(),
        },
    )
    .run();

    match &report.results[0].outcome {
        BranchOutcome::VerificationFailed { reason } => {
            assert!(reason.contains("unit tests failed"), "{reason}");
        }
        other => panic!("expected VerificationFailed, got {other:?}"),
    }
    // Trunk did not advance.
    assert_eq!(git(&dir, &["rev-parse", "main"]), main_before);
    // The branch is intact (rebase was completed, nothing landed).
    let porcelain = git(&dir, &["status", "--porcelain"]);
    assert_eq!(porcelain, "", "repo must be left clean: {porcelain:?}");

    let _ = std::fs::remove_dir_all(&dir);
}
