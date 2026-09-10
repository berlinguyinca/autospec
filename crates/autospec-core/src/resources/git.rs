//! Read-only observation of local git branches and worktrees.
//!
//! Spec anchors: `docs/specs/2026-08-16-resource-lifecycle-cleanup-design.md`
//! §13 (worktrees), §14 (branches), §36 (security invariants), §48 (legacy
//! compatibility).
//!
//! The observer is the Phase 1 half of the resource-lifecycle design: it
//! reports what exists and assigns an [`OwnershipClass`]. Every git call is a
//! read-only argument vector — never a shell string (Invariant 7) — and no
//! call can mutate repository state:
//!
//! - one `git for-each-ref` call lists every local branch (name + upstream);
//! - one `git branch --merged origin/main` call records merge state as a
//!   reason (spec §14.3), not a delete signal;
//! - one `git worktree list --porcelain` call inventories worktrees;
//! - one `git status --porcelain -b` call per worktree records dirty /
//!   unpushed state as reasons;
//! - `du -sb` records worktree disk usage as `size_bytes`, `None` when
//!   unknown — never `0` as a stand-in.
//!
//! Ownership rules (spec §14.1, §13.2, §48; Invariant 1 / 4):
//!
//! - A branch is run-exclusive only when it lives in the
//!   `autospec/<work-item>/<role>/<slug>` namespace. A user branch merely
//!   *named* with the word `autospec` (e.g. `feature/autospec-notes`) is
//!   **not** in the namespace and stays [`OwnershipClass::External`].
//! - A worktree is run-exclusive only when its path sits under a
//!   `.autospec/worktrees/` directory (in-repo or central layout, spec
//!   §13.2). Every other worktree — including the main checkout — is
//!   [`OwnershipClass::External`]. Dirtiness is recorded as a reason, never
//!   upgraded into ownership (spec §13.4, Invariant 1).
//!
//! Observers only report. Deletion is a separate subsystem decision that is
//! gated on Invariant 1 (never delete what you cannot establish ownership
//! of) and on the §13.3 / §14.3 safety checklists.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::model::{ObservedResource, OwnershipClass, ResourceType};
use crate::error::AutospecError;

const GIT_BIN: &str = "git";
const DU_BIN: &str = "du";

/// Branch namespace that marks AutoSpec-owned branches (spec §14.1).
pub const AUTOSPEC_BRANCH_PREFIX: &str = "autospec/";

/// Read-only: every local branch name and its upstream in a single call.
/// The two fields are joined with a NUL byte (`%00`); a ref name can never
/// contain NUL, so the split is unambiguous.
const FOR_EACH_REF_CMD: &[&str] = &[
    "for-each-ref",
    "--format=%(refname:short)%00%(upstream:short)",
    "refs/heads",
];

/// Read-only: local branches whose commits are reachable from `origin/main`
/// (merge-state evidence for spec §14.3; recorded, never acted on here).
const MERGED_BRANCHES_CMD: &[&str] = &["branch", "--merged", "origin/main"];

/// Read-only: worktree inventory (spec §13).
const WORKTREE_LIST_CMD: &[&str] = &["worktree", "list", "--porcelain"];

/// Read-only: per-worktree working-tree state. The `## ` head line carries
/// the `[ahead N]` marker; every other non-empty line is a local change.
const STATUS_CMD: &[&str] = &["status", "--porcelain", "-b"];

/// Read-only: recursive size in bytes (POSIX `du`).
const DU_CMD: &[&str] = &["-sb"];

// Reason tokens. Reasons are evidence for the cleanup subsystem; they are
// never delete signals by themselves.
const REASON_MERGED: &str = "merged_into_origin_main";
const REASON_NOT_MERGED: &str = "not_merged_into_origin_main";
const REASON_MERGE_STATE_UNKNOWN: &str = "merge_state_unknown_origin_main_unreadable";
const REASON_DIRTY: &str = "dirty";
const REASON_CLEAN: &str = "clean";
const REASON_UNPUSHED: &str = "has_unpushed_commits";
const REASON_STATUS_UNAVAILABLE: &str = "status_unavailable";
const REASON_PATH_MISSING: &str = "worktree_path_missing";
const REASON_DETACHED: &str = "detached_head";

/// Observe every local branch under `refs/heads` in `repo`.
///
/// Exactly one `git for-each-ref` call lists the refs (no per-branch
/// subprocess), plus one `git branch --merged origin/main` read for merge
/// state. Returns one [`ObservedResource`] per local ref, each carrying a
/// non-empty `reasons` vector.
pub fn observe_branches(repo: &Path) -> Result<Vec<ObservedResource>, AutospecError> {
    let stdout = run_git(repo, FOR_EACH_REF_CMD)?;
    let merged = merged_branch_names(repo);
    let mut observed = Vec::new();
    for line in stdout.lines() {
        if line.is_empty() {
            continue;
        }
        let (name, upstream) = match line.split_once('\0') {
            Some((name, upstream)) => (name, upstream),
            None => continue,
        };
        if name.is_empty() {
            continue;
        }
        let (ownership, ownership_reason) = classify_branch(name);
        let mut reasons = vec![format!("branch {name}"), ownership_reason];
        if !upstream.is_empty() {
            reasons.push(format!("upstream:{upstream}"));
        }
        let merge_reason = match &merged {
            Some(set) if set.contains(name) => REASON_MERGED,
            Some(_) => REASON_NOT_MERGED,
            None => REASON_MERGE_STATE_UNKNOWN,
        };
        reasons.push(merge_reason.to_string());
        observed.push(ObservedResource {
            resource_type: ResourceType::GitBranch,
            external_id: format!("refs/heads/{name}"),
            ownership,
            reasons,
            size_bytes: None,
        });
    }
    Ok(observed)
}

/// Observe every worktree known to the repository rooted at `repo`.
///
/// One `git worktree list --porcelain` call inventories the worktrees; each
/// worktree then gets one `git status` read (dirty / unpushed reasons) and a
/// `du` read (`size_bytes`). Every entry carries a non-empty `reasons`
/// vector and `size_bytes` is `Some(n > 0)` or `None`, never `Some(0)`.
pub fn observe_worktrees(repo: &Path) -> Result<Vec<ObservedResource>, AutospecError> {
    let stdout = run_git(repo, WORKTREE_LIST_CMD)?;
    let mut observed = Vec::new();
    for worktree in parse_worktree_porcelain(&stdout) {
        let (ownership, ownership_reason) = classify_worktree(&worktree.path);
        let mut reasons = vec![
            format!("worktree {}", worktree.path.display()),
            ownership_reason,
        ];
        if let Some(refname) = &worktree.branch {
            reasons.push(format!("branch:{refname}"));
        } else if worktree.detached {
            reasons.push(REASON_DETACHED.to_string());
        }
        push_status_reasons(&mut reasons, &worktree.path);
        observed.push(ObservedResource {
            resource_type: ResourceType::GitWorktree,
            external_id: worktree.path.display().to_string(),
            ownership,
            reasons,
            size_bytes: dir_size_bytes(&worktree.path),
        });
    }
    Ok(observed)
}

/// Run `git <args>` inside `repo` and return stdout.
///
/// Argument vectors only: the subcommand and every flag are fixed constants
/// in this module, so no caller-controlled text ever reaches a shell (Invariant
/// 7) and no call can mutate state (every verb is read-only).
fn run_git(repo: &Path, args: &[&str]) -> Result<String, AutospecError> {
    let repo_display = repo.display().to_string();
    let output = Command::new(GIT_BIN)
        .args(args)
        .current_dir(repo)
        .output()
        .map_err(|err| AutospecError::io("spawn git", repo_display.clone(), err))?;
    if !output.status.success() {
        return Err(AutospecError::other(format!(
            "git {} failed ({}): {}",
            args.join(" "),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// One read of which local branches are merged into `origin/main`.
///
/// Returns `None` when `origin/main` is unreadable (no remote yet, fresh
/// clone) — merge state is then *unknown*, deliberately distinct from *not
/// merged*.
fn merged_branch_names(repo: &Path) -> Option<BTreeSet<String>> {
    let stdout = run_git(repo, MERGED_BRANCHES_CMD).ok()?;
    Some(
        stdout
            .lines()
            .map(|line| line.trim().trim_start_matches('*').trim().to_string())
            .filter(|name| !name.is_empty())
            .collect(),
    )
}

/// Ownership for a local branch (spec §14.1, §48).
fn classify_branch(name: &str) -> (OwnershipClass, String) {
    if name.starts_with(AUTOSPEC_BRANCH_PREFIX) {
        (
            OwnershipClass::RunExclusive,
            "autospec/<work-item>/<role>/<slug> branch namespace (spec §14.1)".to_string(),
        )
    } else {
        (
            OwnershipClass::External,
            "no autospec branch namespace (spec §14.1); reported, never deleted (spec §48, Invariant 4)".to_string(),
        )
    }
}

/// Ownership for a worktree path (spec §13.2, §48).
fn classify_worktree(path: &Path) -> (OwnershipClass, String) {
    if under_autospec_worktrees(path) {
        (
            OwnershipClass::RunExclusive,
            ".autospec/worktrees/<run-id>/<purpose> path (spec §13.2)".to_string(),
        )
    } else {
        (
            OwnershipClass::External,
            "no autospec worktree path (spec §13.2); unattributable, reported never deleted (spec §48, Invariant 4)".to_string(),
        )
    }
}

/// True when `path` sits under a `.autospec/worktrees/` directory — either
/// the in-repo layout (`<repo>/.autospec/worktrees/...`) or the central
/// layout (`~/.autospec/worktrees/...`) from spec §13.2. Component-wise
/// matching avoids false positives like `my.autospec.notes/worktrees`.
fn under_autospec_worktrees(path: &Path) -> bool {
    let mut components = path.components();
    while let Some(component) = components.next() {
        if component.as_os_str() != ".autospec" {
            continue;
        }
        if let Some(next) = components.next() {
            if next.as_os_str() == "worktrees" {
                return true;
            }
        }
    }
    false
}

/// Record one worktree's working-tree state as reasons. A missing path or a
/// failed status read is itself evidence and is reported, never guessed.
fn push_status_reasons(reasons: &mut Vec<String>, worktree: &Path) {
    if !worktree.exists() {
        reasons.push(REASON_PATH_MISSING.to_string());
        return;
    }
    match worktree_dirty_and_unpushed(worktree) {
        Ok((dirty, unpushed)) => {
            reasons.push(if dirty { REASON_DIRTY } else { REASON_CLEAN }.to_string());
            if unpushed {
                reasons.push(REASON_UNPUSHED.to_string());
            }
        }
        Err(_) => reasons.push(REASON_STATUS_UNAVAILABLE.to_string()),
    }
}

struct RawWorktree {
    path: PathBuf,
    branch: Option<String>,
    detached: bool,
}

/// Parse `git worktree list --porcelain` output.
///
/// Each record starts with `worktree <path>` and is terminated by a blank
/// line (or end of output). `HEAD <sha>`, `bare`, `locked <reason>` and
/// `prunable <reason>` lines are recorded by git but carry no ownership
/// signal, so they are ignored here.
fn parse_worktree_porcelain(stdout: &str) -> Vec<RawWorktree> {
    let mut worktrees = Vec::new();
    let mut current: Option<RawWorktree> = None;
    for line in stdout.lines() {
        if line.is_empty() {
            if let Some(worktree) = current.take() {
                worktrees.push(worktree);
            }
            continue;
        }
        let worktree = current.get_or_insert_with(|| RawWorktree {
            path: PathBuf::new(),
            branch: None,
            detached: false,
        });
        if let Some(path) = line.strip_prefix("worktree ") {
            worktree.path = PathBuf::from(path);
        } else if let Some(refname) = line.strip_prefix("branch ") {
            worktree.branch = Some(refname.trim().to_string());
        } else if line == "detached" {
            worktree.detached = true;
        }
    }
    if let Some(worktree) = current.take() {
        worktrees.push(worktree);
    }
    worktrees
}

/// `(dirty, has_unpushed)` for one worktree from a single read-only
/// `git status --porcelain -b` call. The `## ` head line carries the
/// `[ahead N]` marker; any other non-empty line is a working-tree change.
fn worktree_dirty_and_unpushed(worktree: &Path) -> Result<(bool, bool), AutospecError> {
    let stdout = run_git(worktree, STATUS_CMD)?;
    let mut lines = stdout.lines();
    let mut unpushed = false;
    if let Some(head) = lines.next() {
        unpushed = head.starts_with("## ") && head.contains("[ahead");
    }
    let dirty = lines.any(|line| !line.is_empty());
    Ok((dirty, unpushed))
}

/// Disk usage of `path` in bytes via `du -sb`. Returns `None` when unknown
/// (missing binary, unreadable path) — never `0` as a stand-in.
fn dir_size_bytes(path: &Path) -> Option<u64> {
    let output = Command::new(DU_BIN).args(DU_CMD).arg(path).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let bytes = String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .next()?
        .parse::<u64>()
        .ok()?;
    (bytes > 0).then_some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    fn git(repo: &Path, args: &[&str]) -> String {
        let output = Command::new(GIT_BIN)
            .args(args)
            .current_dir(repo)
            .output()
            .expect("spawn git in test fixture");
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    fn temp_root(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!(
            "autospec-git-observe-{label}-{}-{nonce}",
            std::process::id()
        ))
    }

    /// A real git repository with one commit (issue #3190: real repos, no
    /// mocks).
    fn init_repo(label: &str) -> PathBuf {
        let dir = temp_root(label);
        std::fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init", "-q"]);
        git(&dir, &["config", "user.email", "observer@test"]);
        git(&dir, &["config", "user.name", "observer"]);
        std::fs::write(dir.join("f"), "base").unwrap();
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-qm", "init"]);
        dir
    }

    fn cleanup(dir: &Path) {
        let _ = std::fs::remove_dir_all(dir);
    }

    fn by_id<'a>(observed: &'a [ObservedResource], external_id: &str) -> &'a ObservedResource {
        observed
            .iter()
            .find(|r| r.external_id == external_id)
            .unwrap_or_else(|| panic!("missing observed resource {external_id}: {observed:?}"))
    }

    #[test]
    fn branch_observation_is_one_entry_per_local_ref() {
        let dir = init_repo("branches");
        git(&dir, &["branch", "autospec/417/implement/resource-manager"]);
        git(&dir, &["branch", "feature/autospec-notes"]);
        let head_ref = git(&dir, &["rev-parse", "--abbrev-ref", "HEAD"]);
        let default = head_ref.trim();

        let observed = observe_branches(&dir).unwrap();

        assert_eq!(observed.len(), 3);
        assert!(by_id(&observed, &format!("refs/heads/{default}"))
            .reasons
            .iter()
            .any(|r| r.starts_with("branch ")));
        assert!(by_id(
            &observed,
            "refs/heads/autospec/417/implement/resource-manager"
        )
        .reasons
        .iter()
        .any(|r| r.starts_with("branch ")));
        assert!(by_id(&observed, "refs/heads/feature/autospec-notes")
            .reasons
            .iter()
            .any(|r| r.starts_with("branch ")));
        for resource in &observed {
            assert_eq!(resource.resource_type, ResourceType::GitBranch);
            assert!(
                !resource.reasons.is_empty(),
                "every entry needs reasons: {resource:?}"
            );
        }
        cleanup(&dir);
    }

    #[test]
    fn autospec_namespace_is_run_exclusive_user_branch_stays_external() {
        let dir = init_repo("ownership");
        git(&dir, &["branch", "autospec/417/implement/x"]);
        git(&dir, &["branch", "feature/autospec-notes"]);

        let observed = observe_branches(&dir).unwrap();

        let owned = by_id(&observed, "refs/heads/autospec/417/implement/x");
        assert!(
            matches!(owned.ownership, OwnershipClass::RunExclusive),
            "{:?}",
            owned.reasons
        );
        // A user branch merely *named* with the word autospec is not in the
        // namespace (spec §14.1, §48).
        let user = by_id(&observed, "refs/heads/feature/autospec-notes");
        assert!(
            matches!(user.ownership, OwnershipClass::External),
            "feature/autospec-notes must stay External: {:?}",
            user.reasons
        );
        cleanup(&dir);
    }

    #[test]
    fn merged_state_is_recorded_as_reasons_not_delete_signals() {
        let dir = init_repo("merged");
        // Fake a remote-tracking `main` so `origin/main` is readable without
        // a real remote: the observer only reads refs.
        git(&dir, &["remote", "add", "origin", "/nonexistent"]);
        let head = git(&dir, &["rev-parse", "HEAD"]).trim().to_string();
        git(&dir, &["update-ref", "refs/remotes/origin/main", &head]);
        git(&dir, &["branch", "autospec/417/implement/merged-x"]);
        git(
            &dir,
            &["checkout", "-qb", "autospec/417/implement/unmerged-x"],
        );
        std::fs::write(dir.join("f"), "more").unwrap();
        git(&dir, &["commit", "-qam", "more"]);

        let observed = observe_branches(&dir).unwrap();

        let merged = by_id(&observed, "refs/heads/autospec/417/implement/merged-x");
        assert!(
            merged.reasons.iter().any(|r| r == REASON_MERGED),
            "{:?}",
            merged.reasons
        );
        let unmerged = by_id(&observed, "refs/heads/autospec/417/implement/unmerged-x");
        assert!(
            unmerged.reasons.iter().any(|r| r == REASON_NOT_MERGED),
            "{:?}",
            unmerged.reasons
        );
        assert!(
            !unmerged.reasons.iter().any(|r| r == REASON_MERGED),
            "{:?}",
            unmerged.reasons
        );
        cleanup(&dir);
    }

    #[test]
    fn six_thousand_five_hundred_branch_fixture_observes_under_ten_seconds() {
        let dir = init_repo("bulk");
        let sha = git(&dir, &["rev-parse", "HEAD"]).trim().to_string();
        // Loose ref files are the native storage format: 6,499 of them plus
        // the initial branch is a 6,500-branch fixture.
        let refs_dir = dir.join(".git").join("refs").join("heads");
        for i in 1..=6499 {
            std::fs::write(refs_dir.join(format!("bulk-{:05}", i)), format!("{sha}\n")).unwrap();
        }

        let start = std::time::Instant::now();
        let observed = observe_branches(&dir).unwrap();
        let elapsed = start.elapsed();

        assert_eq!(observed.len(), 6500);
        assert!(
            elapsed < Duration::from_secs(10),
            "observing 6,500 branches took {elapsed:?}"
        );
        for resource in observed.iter().step_by(733) {
            assert!(
                !resource.reasons.is_empty(),
                "every entry needs reasons: {resource:?}"
            );
        }
        cleanup(&dir);
    }

    #[test]
    fn worktree_observation_reports_paths_reasons_and_size() {
        let dir = init_repo("wt-basic");
        let linked = temp_root("wt-basic-linked");
        git(
            &dir,
            &[
                "worktree",
                "add",
                "-q",
                linked.to_str().unwrap(),
                "-b",
                "wt-branch",
            ],
        );

        let observed = observe_worktrees(&dir).unwrap();

        assert_eq!(observed.len(), 2);
        for resource in &observed {
            assert_eq!(resource.resource_type, ResourceType::GitWorktree);
            assert!(
                !resource.reasons.is_empty(),
                "every entry needs reasons: {resource:?}"
            );
            assert_ne!(resource.size_bytes, Some(0), "0 is forbidden as a stand-in");
        }
        // The main checkout is unattributable: External, never RunExclusive.
        let main = by_id(&observed, dir.to_str().unwrap());
        assert!(
            matches!(main.ownership, OwnershipClass::External),
            "main checkout must stay External: {:?}",
            main.reasons
        );
        let linked_entry = by_id(&observed, linked.to_str().unwrap());
        assert!(
            matches!(linked_entry.ownership, OwnershipClass::External),
            "{:?}",
            linked_entry.reasons
        );
        assert!(
            linked_entry
                .reasons
                .iter()
                .any(|r| r == "branch:refs/heads/wt-branch"),
            "{:?}",
            linked_entry.reasons
        );
        assert!(
            linked_entry.size_bytes.is_some(),
            "{:?}",
            linked_entry.size_bytes
        );
        cleanup(&dir);
        cleanup(&linked);
    }

    #[test]
    fn autospec_worktree_path_is_run_exclusive() {
        let dir = init_repo("wt-autospec");
        let linked = dir
            .join(".autospec")
            .join("worktrees")
            .join("as-20260816-221904-a31f")
            .join("implement");
        git(
            &dir,
            &[
                "worktree",
                "add",
                "-q",
                linked.to_str().unwrap(),
                "-b",
                "autospec/417/implement/x",
            ],
        );

        let observed = observe_worktrees(&dir).unwrap();

        let entry = by_id(&observed, linked.to_str().unwrap());
        assert!(
            matches!(entry.ownership, OwnershipClass::RunExclusive),
            "{:?}",
            entry.reasons
        );
        cleanup(&dir);
    }

    #[test]
    fn dirty_unattributable_worktree_stays_external_with_dirty_reason() {
        let dir = init_repo("wt-dirty");
        let linked = temp_root("wt-dirty-linked");
        git(
            &dir,
            &[
                "worktree",
                "add",
                "-q",
                linked.to_str().unwrap(),
                "-b",
                "user-branch",
            ],
        );
        std::fs::write(linked.join("untracked.txt"), "data").unwrap();

        let observed = observe_worktrees(&dir).unwrap();

        let entry = by_id(&observed, linked.to_str().unwrap());
        assert!(
            matches!(entry.ownership, OwnershipClass::External),
            "dirtiness never upgrades ownership (spec §13.4, Invariant 1): {:?}",
            entry.reasons
        );
        assert!(
            entry.reasons.iter().any(|r| r == REASON_DIRTY),
            "dirty state must be recorded as a reason: {:?}",
            entry.reasons
        );
        cleanup(&dir);
        cleanup(&linked);
    }

    #[test]
    fn unpushed_commits_are_recorded_as_reason_not_delete_signal() {
        let dir = init_repo("wt-ahead");
        let linked = temp_root("wt-ahead-linked");
        git(
            &dir,
            &[
                "worktree",
                "add",
                "-q",
                linked.to_str().unwrap(),
                "-b",
                "ahead-branch",
            ],
        );
        git(&dir, &["remote", "add", "origin", "/nonexistent"]);
        let base = git(&linked, &["rev-parse", "HEAD"]).trim().to_string();
        std::fs::write(linked.join("g"), "extra").unwrap();
        git(&linked, &["add", "."]);
        git(&linked, &["commit", "-qm", "extra"]);
        git(
            &linked,
            &["update-ref", "refs/remotes/origin/ahead-branch", &base],
        );
        git(&linked, &["config", "branch.ahead-branch.remote", "origin"]);
        git(
            &linked,
            &[
                "config",
                "branch.ahead-branch.merge",
                "refs/heads/ahead-branch",
            ],
        );

        let observed = observe_worktrees(&dir).unwrap();

        let entry = by_id(&observed, linked.to_str().unwrap());
        assert!(
            entry.reasons.iter().any(|r| r == REASON_UNPUSHED),
            "unpushed state must be recorded as a reason: {:?}",
            entry.reasons
        );
        cleanup(&dir);
        cleanup(&linked);
    }

    #[test]
    fn observing_a_non_git_directory_is_a_clean_error() {
        let dir = temp_root("not-git");
        std::fs::create_dir_all(&dir).unwrap();

        assert!(observe_branches(&dir).is_err());
        assert!(observe_worktrees(&dir).is_err());
        cleanup(&dir);
    }

    #[test]
    fn git_subcommands_are_read_only_argument_vectors() {
        // The observer may only ever call these read-only subcommands, as
        // argument vectors (Invariant 7: no shell, no interpolation).
        for cmd in [
            FOR_EACH_REF_CMD,
            MERGED_BRANCHES_CMD,
            WORKTREE_LIST_CMD,
            STATUS_CMD,
        ] {
            assert!(
                matches!(cmd[0], "for-each-ref" | "branch" | "worktree" | "status"),
                "unexpected git subcommand: {cmd:?}"
            );
            for arg in cmd.iter().copied() {
                assert!(
                    !matches!(
                        arg,
                        "-D" | "remove" | "prune" | "fetch" | "reset" | "push" | "delete"
                    ),
                    "destructive arg {arg:?} in {cmd:?}"
                );
            }
            assert!(
                !cmd.iter().any(|arg| *arg == "-c"),
                "-c would allow config injection: {cmd:?}"
            );
        }
    }

    #[test]
    fn source_contains_no_destructive_git_invocations() {
        // Split tokens so this test's own source does not trip a naive scan
        // of the file for these literals.
        let forbidden = [
            ["sh", " -c"].concat(),
            ["bra", "nch -D"].concat(),
            ["work", "tree remove"].concat(),
            ["git ", "reset"].concat(),
            ["git ", "push"].concat(),
            ["git ", "fetch"].concat(),
        ];
        let source = include_str!("git.rs");
        for token in &forbidden {
            assert!(
                !source.contains(token.as_str()),
                "destructive git in source: {token}"
            );
        }
    }
}
