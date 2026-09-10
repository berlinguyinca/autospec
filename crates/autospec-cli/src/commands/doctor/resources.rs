//! `autospec doctor resources` — §24.3 per-type resource health.
//!
//! Prints per-type counts for worktrees, branches, Docker containers,
//! images, volumes, and child processes, each bucketed into five buckets:
//!
//! | bucket           | meaning                                                                 |
//! |------------------|-------------------------------------------------------------------------|
//! | `active`         | attributable and current work (main worktree, unmerged branch, running managed resource) |
//! | `stale`          | attributable but finished: branch (or worktree branch) merged into `origin/main` |
//! | `orphaned`       | leftover metadata whose target is gone: prunable worktree, branch whose upstream is `[gone]` |
//! | `quarantined`    | operator-locked worktree                                                |
//! | `unattributable` | `External` ownership — nothing an AutoSpec run owns. **Always printed**, including 0: unlabeled containers must never be folded into `orphaned` |
//!
//! The estimated reclaimable disk sums `size_bytes` over **non-`External`**
//! observations only — an unlabeled (External) resource contributes 0 bytes
//! even when Docker reports a size for it.
//!
//! Read-only by construction: the git subcommands issued from here are
//! listing verbs only (`worktree list`, `for-each-ref`, `remote get-url`).
//! No destructive git or Docker subcommand is ever issued from anywhere under
//! `commands/doctor/` (the source scan in the test module below pins this).
//!
//! Fail-open like the rest of `doctor`: a missing repo, a missing
//! `origin/main`, a missing heartbeat directory, or an unavailable Docker
//! daemon each degrade to empty buckets and the command still exits 0 —
//! including in a repo with no resource ledger.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use autospec_core::resources::{
    observe_docker, observe_processes, ObservedResource, OwnershipClass, ResourceType,
};

/// The five §24.3 buckets. All five are always printed, even at zero.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Buckets {
    pub active: u64,
    pub stale: u64,
    pub orphaned: u64,
    pub quarantined: u64,
    pub unattributable: u64,
}

/// Everything `autospec doctor resources` reports for one repository.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Report {
    pub worktrees: Buckets,
    pub branches: Buckets,
    pub containers: Buckets,
    pub images: Buckets,
    pub volumes: Buckets,
    pub processes: Buckets,
    /// Sum of `size_bytes` over non-`External` observations only.
    pub estimated_reclaimable_bytes: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Options {
    json: bool,
    help: bool,
}

const USAGE: &str = "autospec doctor resources [--json]

Print \u{00a7}24.3 per-type resource health: worktrees, branches, Docker
containers, images, volumes, and processes, each bucketed into
active / stale / orphaned / quarantined / unattributable, plus the
estimated reclaimable disk (the sum of size_bytes over non-External
observations only; External resources count 0 bytes).

Options:
  --json    emit the same buckets as a parseable JSON object

Read-only: this command issues listing git subcommands only and never
issues a destructive git or Docker subcommand. It exits 0 in a repo with
no resource ledger.";

/// The only git subcommands this command issues — listing verbs.
const GIT_WORKTREE_LIST: &[&str] = &["worktree", "list", "--porcelain"];
const GIT_FOR_EACH_REF: &[&str] = &[
    "for-each-ref",
    "refs/heads",
    "--format=%(refname:short)%00%(upstream:track)",
];
const GIT_MERGED_INTO_MAIN: &[&str] = &[
    "for-each-ref",
    "--merged=origin/main",
    "refs/heads",
    "--format=%(refname:short)",
];
const GIT_REMOTE_ORIGIN: &[&str] = &["remote", "get-url", "origin"];

/// CLI entry point; resolves the repository root from the cwd and delegates
/// to `run_in` so tests can drive the same code against fixture repos.
pub fn run(args: &[String]) -> Result<(), String> {
    let root = std::env::current_dir()
        .map_err(|error| format!("could not resolve the current directory: {error}"))?;
    run_in(&root, args)
}

pub fn run_in(root: &Path, args: &[String]) -> Result<(), String> {
    let options = parse_args(args)?;
    if options.help {
        println!("{USAGE}");
        return Ok(());
    }
    let report = collect(root);
    let rendered = if options.json {
        render_json(&report)
    } else {
        render_text(&report)
    };
    println!("{rendered}");
    Ok(())
}

fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut options = Options::default();
    for argument in args {
        match argument.as_str() {
            "--json" => options.json = true,
            "--help" | "-h" => options.help = true,
            other => {
                return Err(format!("unknown argument `{other}`\n\n{USAGE}"));
            }
        }
    }
    Ok(options)
}

fn collect(root: &Path) -> Report {
    let merged = merged_branches(root);
    let worktrees = parse_worktrees(&git_output(root, GIT_WORKTREE_LIST).unwrap_or_default());
    let branches = parse_branches(&git_output(root, GIT_FOR_EACH_REF).unwrap_or_default());
    let docker = match observe_docker() {
        Ok(observations) => observations,
        Err(error) => {
            eprintln!("autospec doctor resources: docker observation skipped: {error}");
            Vec::new()
        }
    };
    let processes = process_observations(root);
    Report {
        worktrees: bucket_worktrees(&worktrees, &merged),
        branches: bucket_branches(&branches, &merged),
        containers: docker_buckets(&docker, ResourceType::DockerContainer),
        images: docker_buckets(&docker, ResourceType::DockerImage),
        volumes: docker_buckets(&docker, ResourceType::DockerVolume),
        processes: bucket_observations(&processes),
        estimated_reclaimable_bytes: reclaimable_bytes(&docker) + reclaimable_bytes(&processes),
    }
}

fn git_output(root: &Path, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Branch names fully merged into `origin/main` (the "merged stale" set).
/// Empty when `origin/main` does not exist.
fn merged_branches(root: &Path) -> HashSet<String> {
    git_output(root, GIT_MERGED_INTO_MAIN)
        .map(|names| {
            names
                .lines()
                .filter(|line| !line.trim().is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Child-process observations for this repository's heartbeat slug
/// directory, resolved the same way the drain loop resolves it.
fn process_observations(root: &Path) -> Vec<ObservedResource> {
    let Some(remote_url) = git_output(root, GIT_REMOTE_ORIGIN) else {
        return Vec::new();
    };
    let Some(heartbeat_root) = crate::commands::claim::heartbeat_root().ok() else {
        return Vec::new();
    };
    let Some(slug_dir) = process_heartbeat_dir(&remote_url, &heartbeat_root) else {
        return Vec::new();
    };
    match observe_processes(&slug_dir) {
        Ok(observations) => observations,
        Err(error) => {
            eprintln!("autospec doctor resources: process observation skipped: {error}");
            Vec::new()
        }
    }
}

/// `<heartbeat_root>/<owner-repo slug>` — the directory
/// `observe_processes` scans for this repository.
pub(crate) fn process_heartbeat_dir(remote_url: &str, heartbeat_root: &Path) -> Option<PathBuf> {
    let scope = repository_scope(remote_url)?;
    Some(
        heartbeat_root.join(crate::commands::autonomous::drain::repository_progress_key(
            &scope,
        )),
    )
}

/// `owner/repo` from an origin URL (`git@host:owner/repo(.git)` or
/// `https://host/owner/repo(.git)`); `None` when the URL has fewer than two
/// path segments after the host.
pub(crate) fn repository_scope(remote_url: &str) -> Option<String> {
    let url = remote_url.trim();
    let path = if let Some((_, after_host)) = url.split_once("://") {
        after_host.split_once('/').map(|(_, path)| path)?
    } else if let Some((_, after_colon)) = url.rsplit_once(':') {
        after_colon
    } else {
        url
    };
    let path = path.trim_matches('/').trim_end_matches(".git");
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let [.., owner, repository] = segments.as_slice() else {
        return None;
    };
    Some(format!("{owner}/{repository}"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Worktree {
    path: String,
    branch: Option<String>,
    is_main: bool,
    locked: bool,
    prunable: bool,
}

/// Parse `git worktree list --porcelain`. The first record is the main
/// worktree; a record without a `branch` line is detached.
pub(crate) fn parse_worktrees(porcelain: &str) -> Vec<Worktree> {
    let mut worktrees: Vec<Worktree> = Vec::new();
    for line in porcelain.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            worktrees.push(Worktree {
                path: path.trim_end().to_string(),
                branch: None,
                is_main: false,
                locked: false,
                prunable: false,
            });
        } else if let Some(worktree) = worktrees.last_mut() {
            if line == "locked" || line.starts_with("locked ") {
                worktree.locked = true;
            } else if line == "prunable" || line.starts_with("prunable ") {
                worktree.prunable = true;
            } else if let Some(refname) = line.strip_prefix("branch ") {
                worktree.branch = Some(shorten_ref(refname.trim()));
            }
        }
    }
    if let Some(first) = worktrees.first_mut() {
        first.is_main = true;
    }
    worktrees
}

fn shorten_ref(refname: &str) -> String {
    refname
        .strip_prefix("refs/heads/")
        .unwrap_or(refname)
        .to_string()
}

/// Worktree bucketing, in priority order: the main worktree is always
/// `active`; a prunable record (checkout path gone) is `orphaned` even if
/// also locked; a locked worktree is `quarantined`; a worktree on a branch
/// merged into `origin/main` is `stale`; a detached worktree with no branch
/// to attribute it to is `unattributable`; everything else is `active`.
pub(crate) fn bucket_worktrees(worktrees: &[Worktree], merged: &HashSet<String>) -> Buckets {
    let mut buckets = Buckets::default();
    for worktree in worktrees {
        let bucket = if worktree.is_main {
            &mut buckets.active
        } else if worktree.prunable {
            &mut buckets.orphaned
        } else if worktree.locked {
            &mut buckets.quarantined
        } else if worktree
            .branch
            .as_deref()
            .is_some_and(|name| merged.contains(name))
        {
            &mut buckets.stale
        } else if worktree.branch.is_none() {
            &mut buckets.unattributable
        } else {
            &mut buckets.active
        };
        *bucket += 1;
    }
    buckets
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Branch {
    name: String,
    upstream_gone: bool,
}

/// Parse `git for-each-ref` output formatted as `<name>\0<upstream:track>`.
/// `<upstream:track>` is `[gone]` exactly when the upstream ref no longer
/// exists; malformed lines (no NUL) are skipped.
pub(crate) fn parse_branches(foreach: &str) -> Vec<Branch> {
    foreach
        .lines()
        .filter_map(|line| {
            let (name, track) = line.split_once('\0')?;
            let name = name.trim();
            if name.is_empty() {
                return None;
            }
            Some(Branch {
                name: name.to_string(),
                upstream_gone: track.contains("gone"),
            })
        })
        .collect()
}

/// Branch bucketing: an upstream that is `[gone]` is `orphaned` (even when
/// the branch is also merged); a branch merged into `origin/main` is
/// `stale`; everything else is `active`.
pub(crate) fn bucket_branches(branches: &[Branch], merged: &HashSet<String>) -> Buckets {
    let mut buckets = Buckets::default();
    for branch in branches {
        let bucket = if branch.upstream_gone {
            &mut buckets.orphaned
        } else if merged.contains(&branch.name) {
            &mut buckets.stale
        } else {
            &mut buckets.active
        };
        *bucket += 1;
    }
    buckets
}

/// Observation bucketing for the core observers (Docker and processes):
/// `External` ownership is `unattributable`; every attributable
/// (non-External) observation is `active`. The observers report ownership
/// but no lifecycle state, so `stale`/`orphaned`/`quarantined` stay at zero
/// for these types rather than guessing.
pub(crate) fn bucket_observations(observations: &[ObservedResource]) -> Buckets {
    let mut buckets = Buckets::default();
    for observation in observations {
        let bucket = if matches!(observation.ownership, OwnershipClass::External) {
            &mut buckets.unattributable
        } else {
            &mut buckets.active
        };
        *bucket += 1;
    }
    buckets
}

fn docker_buckets(docker: &[ObservedResource], resource_type: ResourceType) -> Buckets {
    let selected: Vec<ObservedResource> = docker
        .iter()
        .filter(|observation| observation.resource_type == resource_type)
        .cloned()
        .collect();
    bucket_observations(&selected)
}

/// Estimated reclaimable disk: the sum of `size_bytes` over non-`External`
/// observations only. `External` entries never contribute, even when they
/// carry a size; `None` (unmeasured) contributes nothing.
pub(crate) fn reclaimable_bytes(observations: &[ObservedResource]) -> u64 {
    observations
        .iter()
        .filter(|observation| !matches!(observation.ownership, OwnershipClass::External))
        .filter_map(|observation| observation.size_bytes)
        .sum()
}

pub(crate) fn render_text(report: &Report) -> String {
    let mut text = String::from(
        "AutoSpec Resource Health\n\
         \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\
         \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\n\n",
    );
    push_section(&mut text, "Worktrees", &report.worktrees);
    push_section(&mut text, "Branches", &report.branches);
    push_section(&mut text, "Docker containers", &report.containers);
    push_section(&mut text, "Docker images", &report.images);
    push_section(&mut text, "Docker volumes", &report.volumes);
    push_section(&mut text, "Processes", &report.processes);
    text.push_str(&format!(
        "Estimated reclaimable disk: {}\n",
        human_size(report.estimated_reclaimable_bytes)
    ));
    text
}

fn push_section(text: &mut String, title: &str, buckets: &Buckets) {
    text.push_str(title);
    text.push('\n');
    for (label, count) in [
        ("active", buckets.active),
        ("stale", buckets.stale),
        ("orphaned", buckets.orphaned),
        ("quarantined", buckets.quarantined),
        ("unattributable", buckets.unattributable),
    ] {
        text.push_str(&format!("  {label:<24}{count}\n"));
    }
    text.push('\n');
}

pub(crate) fn render_json(report: &Report) -> String {
    serde_json::to_string(&serde_json::json!({
        "command": "doctor resources",
        "worktrees": report.worktrees.json(),
        "branches": report.branches.json(),
        "containers": report.containers.json(),
        "images": report.images.json(),
        "volumes": report.volumes.json(),
        "processes": report.processes.json(),
        "estimated_reclaimable_bytes": report.estimated_reclaimable_bytes,
        "estimated_reclaimable": human_size(report.estimated_reclaimable_bytes),
    }))
    .expect("static JSON shape serializes")
}

impl Buckets {
    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "active": self.active,
            "stale": self.stale,
            "orphaned": self.orphaned,
            "quarantined": self.quarantined,
            "unattributable": self.unattributable,
        })
    }
}

/// Human-readable size: `0 B` .. `1023 B`, then one decimal in
/// KB/MB/GB/TB/PB (1024-based).
pub(crate) fn human_size(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    const UNITS: [&str; 5] = ["KB", "MB", "GB", "TB", "PB"];
    let mut value = bytes as f64;
    for unit in UNITS {
        value /= 1024.0;
        if value < 1024.0 || unit == "PB" {
            return format!("{value:.1} {unit}");
        }
    }
    unreachable!("the unit loop always returns")
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------- helpers ----------

    /// A unique temp directory; the caller removes it with `cleanup_dir`.
    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "autospec-doctor-resources-{}-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
            std::thread::current()
                .name()
                .unwrap_or("t")
                .replace(' ', "_")
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn cleanup_dir(dir: impl AsRef<Path>) {
        let _ = std::fs::remove_dir_all(dir);
    }

    fn git(repo: &Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(repo)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GIT_AUTHOR_NAME", "autospec doctor resources test")
            .env("GIT_AUTHOR_EMAIL", "doctor-resources-test@example.com")
            .env("GIT_COMMITTER_NAME", "autospec doctor resources test")
            .env("GIT_COMMITTER_EMAIL", "doctor-resources-test@example.com")
            .output()
            .expect("run git");
        assert!(
            status.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&status.stderr)
        );
    }

    /// Fresh repo with one commit on `main`.
    fn fresh_repo(tag: &str) -> PathBuf {
        let root = temp_root(tag);
        git(&root, &["init", "-q", "-b", "main"]);
        std::fs::write(root.join("README.md"), "fixture\n").unwrap();
        git(&root, &["add", "."]);
        git(&root, &["commit", "-q", "-m", "init"]);
        root
    }

    fn commit_on(repo: &Path, file: &str, message: &str) {
        let path = repo.join(file);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, format!("{message}\n")).unwrap();
        git(repo, &["add", "."]);
        git(repo, &["commit", "-q", "-m", message]);
    }

    fn observation(
        resource_type: ResourceType,
        ownership: OwnershipClass,
        size: Option<u64>,
    ) -> ObservedResource {
        ObservedResource {
            resource_type,
            external_id: "fixture".to_string(),
            ownership,
            reasons: Vec::new(),
            size_bytes: size,
        }
    }

    // ---------- argument parsing ----------

    #[test]
    fn parse_args_defaults_to_text_no_help() {
        assert_eq!(parse_args(&[]), Ok(Options::default()));
    }

    #[test]
    fn parse_args_accepts_json() {
        let options = parse_args(&["--json".to_string()]).expect("parses");
        assert!(options.json);
        assert!(!options.help);
    }

    #[test]
    fn parse_args_accepts_help_forms() {
        for flag in ["--help", "-h"] {
            let options = parse_args(&[flag.to_string()]).expect("parses");
            assert!(options.help, "{flag} sets help");
        }
    }

    #[test]
    fn parse_args_rejects_unknown_arguments_with_usage() {
        let error = parse_args(&["--bogus".to_string()]).expect_err("must fail");
        assert!(error.contains("unknown argument `--bogus`"), "{error}");
        assert!(
            error.contains("autospec doctor resources"),
            "error carries usage"
        );
    }

    // ---------- worktree porcelain parsing ----------

    #[test]
    fn parse_worktrees_reads_main_branch_detached_locked_and_prunable() {
        let porcelain = "\
worktree /repo
HEAD aaa
branch refs/heads/main

worktree /repo/w1
HEAD bbb
branch refs/heads/feat/live

worktree /repo/w2
HEAD ccc
detached

worktree /repo/w3
HEAD ddd
branch refs/heads/feat/locked
locked operator pin

worktree /repo/w4
HEAD eee
branch refs/heads/feat/gone
prunable gitdir missing
";
        let worktrees = parse_worktrees(porcelain);
        assert_eq!(worktrees.len(), 5);
        assert!(worktrees[0].is_main);
        assert_eq!(worktrees[0].branch.as_deref(), Some("main"));
        assert_eq!(worktrees[1].branch.as_deref(), Some("feat/live"));
        assert_eq!(worktrees[2].branch, None, "detached has no branch");
        assert!(worktrees[3].locked);
        assert!(worktrees[4].prunable);
        assert!(!worktrees[1].is_main);
    }

    #[test]
    fn parse_worktrees_empty_input_is_empty() {
        assert!(parse_worktrees("").is_empty());
        assert!(parse_worktrees("   \n").is_empty());
    }

    #[test]
    fn shorten_ref_strips_heads_prefix_only() {
        assert_eq!(shorten_ref("refs/heads/feat/x"), "feat/x");
        assert_eq!(shorten_ref("other/ref"), "other/ref");
    }

    // ---------- worktree bucketing ----------

    fn worktree(branch: Option<&str>, main: bool, locked: bool, prunable: bool) -> Worktree {
        Worktree {
            path: "/tmp".to_string(),
            branch: branch.map(str::to_string),
            is_main: main,
            locked,
            prunable,
        }
    }

    #[test]
    fn bucket_worktrees_covers_every_bucket_with_priority() {
        let merged: HashSet<String> = ["merged-a".to_string()].into_iter().collect();
        let worktrees = vec![
            worktree(Some("main"), true, false, false), // main: always active
            worktree(Some("feat/live"), false, false, false), // active
            worktree(Some("merged-a"), false, false, false), // merged: stale
            worktree(Some("feat/broken"), false, true, true), // prunable beats locked: orphaned
            worktree(Some("feat/locked"), false, true, false), // quarantined
            worktree(None, false, false, false),        // detached: unattributable
        ];
        assert_eq!(
            bucket_worktrees(&worktrees, &merged),
            Buckets {
                active: 2,
                stale: 1,
                orphaned: 1,
                quarantined: 1,
                unattributable: 1,
            }
        );
    }

    #[test]
    fn bucket_worktrees_without_origin_main_has_no_stale() {
        let worktrees = vec![worktree(Some("main"), true, false, false)];
        let buckets = bucket_worktrees(&worktrees, &HashSet::new());
        assert_eq!(buckets.active, 1);
        assert_eq!(buckets.stale, 0);
    }

    // ---------- branch parsing and bucketing ----------

    #[test]
    fn parse_branches_reads_nul_separated_name_and_track() {
        let branches = parse_branches("main\0\nfeat/x\0[gone]\nfeat/y\0[ahead 1]\n");
        assert_eq!(branches.len(), 3);
        assert_eq!(branches[0].name, "main");
        assert!(!branches[0].upstream_gone);
        assert_eq!(branches[1].name, "feat/x");
        assert!(branches[1].upstream_gone);
        assert_eq!(branches[2].name, "feat/y");
        assert!(!branches[2].upstream_gone);
    }

    #[test]
    fn parse_branches_skips_malformed_lines() {
        assert!(parse_branches("").is_empty());
        let branches = parse_branches("line-without-nul\nfeat/x\0[ahead 1]\n");
        assert_eq!(branches.len(), 1);
        assert_eq!(branches[0].name, "feat/x");
    }

    #[test]
    fn bucket_branches_gone_beats_merged_and_stale_beats_active() {
        let merged: HashSet<String> = ["merged-a".to_string(), "both".to_string()]
            .into_iter()
            .collect();
        let branches = vec![
            Branch {
                name: "both".into(),
                upstream_gone: true,
            }, // orphaned, not stale
            Branch {
                name: "merged-a".into(),
                upstream_gone: false,
            }, // stale
            Branch {
                name: "live".into(),
                upstream_gone: false,
            }, // active
        ];
        assert_eq!(
            bucket_branches(&branches, &merged),
            Buckets {
                active: 1,
                stale: 1,
                orphaned: 1,
                quarantined: 0,
                unattributable: 0,
            }
        );
    }

    // ---------- observation bucketing and reclaimable bytes ----------

    #[test]
    fn thirteen_unlabeled_containers_are_unattributable_never_orphaned() {
        let unlabeled: Vec<ObservedResource> = (0..13)
            .map(|_| {
                observation(
                    ResourceType::DockerContainer,
                    OwnershipClass::External,
                    Some(1024),
                )
            })
            .collect();
        let buckets = bucket_observations(&unlabeled);
        assert_eq!(buckets.unattributable, 13);
        assert_eq!(buckets.orphaned, 0);
        assert_eq!(buckets.active, 0);
        assert_eq!(
            reclaimable_bytes(&unlabeled),
            0,
            "External entries count 0 bytes"
        );
    }

    #[test]
    fn managed_observations_are_active() {
        let managed = vec![
            observation(
                ResourceType::DockerContainer,
                OwnershipClass::RunExclusive,
                None,
            ),
            observation(
                ResourceType::DockerImage,
                OwnershipClass::RepoShared,
                Some(10),
            ),
            observation(
                ResourceType::DockerVolume,
                OwnershipClass::GlobalShared,
                Some(20),
            ),
        ];
        assert_eq!(
            bucket_observations(&managed),
            Buckets {
                active: 3,
                ..Default::default()
            }
        );
    }

    #[test]
    fn reclaimable_sums_non_external_sizes_only() {
        let observations = vec![
            observation(
                ResourceType::DockerImage,
                OwnershipClass::External,
                Some(999),
            ),
            observation(
                ResourceType::DockerImage,
                OwnershipClass::RunExclusive,
                Some(1024),
            ),
            observation(
                ResourceType::DockerImage,
                OwnershipClass::RunExclusive,
                None,
            ),
            observation(
                ResourceType::DockerImage,
                OwnershipClass::RepoShared,
                Some(2048),
            ),
        ];
        assert_eq!(reclaimable_bytes(&observations), 3072);
    }

    #[test]
    fn docker_buckets_filter_by_resource_type() {
        let docker = vec![
            observation(
                ResourceType::DockerContainer,
                OwnershipClass::External,
                None,
            ),
            observation(
                ResourceType::DockerContainer,
                OwnershipClass::RunExclusive,
                None,
            ),
            observation(
                ResourceType::DockerImage,
                OwnershipClass::External,
                Some(500),
            ),
            observation(
                ResourceType::DockerImage,
                OwnershipClass::RunExclusive,
                Some(500),
            ),
            observation(ResourceType::DockerVolume, OwnershipClass::External, None),
            observation(ResourceType::DockerNetwork, OwnershipClass::External, None),
        ];
        let containers = docker_buckets(&docker, ResourceType::DockerContainer);
        assert_eq!(containers.active, 1);
        assert_eq!(containers.unattributable, 1);
        let images = docker_buckets(&docker, ResourceType::DockerImage);
        assert_eq!(images.active, 1);
        assert_eq!(images.unattributable, 1);
        let volumes = docker_buckets(&docker, ResourceType::DockerVolume);
        assert_eq!(volumes.unattributable, 1);
    }

    // ---------- repository scope / heartbeat dir ----------

    #[test]
    fn repository_scope_parses_ssh_and_https_urls() {
        assert_eq!(
            repository_scope("git@github.com:autospec/core.git").as_deref(),
            Some("autospec/core")
        );
        assert_eq!(
            repository_scope("https://github.com/autospec/core.git").as_deref(),
            Some("autospec/core")
        );
        assert_eq!(repository_scope("not-a-url"), None);
        assert_eq!(
            repository_scope("https://github.com/only-one-segment.git"),
            None
        );
    }

    #[test]
    fn process_heartbeat_dir_uses_owner_repo_slug() {
        let root = PathBuf::from("/tmp/hb");
        let dir =
            process_heartbeat_dir("git@github.com:autospec/core.git", &root).expect("resolves");
        assert_eq!(dir, root.join("o8_autospec_r4_core"));
        assert!(
            process_heartbeat_dir("not-a-url", &root).is_none(),
            "unparseable URLs yield no heartbeat dir"
        );
    }

    // ---------- human_size ----------

    #[test]
    fn human_size_scales_with_one_decimal() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(1023), "1023 B");
        assert_eq!(human_size(1024), "1.0 KB");
        assert_eq!(human_size(1536), "1.5 KB");
        assert_eq!(human_size(2048), "2.0 KB");
        assert_eq!(human_size(1024 * 1024), "1.0 MB");
        assert_eq!(human_size(1024u64.pow(3)), "1.0 GB");
        assert_eq!(human_size(1024u64.pow(4)), "1.0 TB");
        assert_eq!(human_size(1024u64.pow(5)), "1.0 PB");
        assert_eq!(human_size(u64::MAX), "16384.0 PB");
    }

    #[test]
    fn human_size_renders_the_spec_example() {
        assert_eq!(human_size(41_568_102_220), "38.7 GB");
    }

    // ---------- rendering ----------

    fn sample_report() -> Report {
        Report {
            worktrees: Buckets {
                active: 1,
                stale: 2,
                orphaned: 1,
                quarantined: 1,
                unattributable: 1,
            },
            branches: Buckets {
                active: 3,
                stale: 7,
                orphaned: 2,
                ..Default::default()
            },
            containers: Buckets {
                unattributable: 13,
                ..Default::default()
            },
            images: Buckets {
                active: 1,
                unattributable: 4,
                ..Default::default()
            },
            volumes: Buckets::default(),
            processes: Buckets {
                active: 2,
                unattributable: 1,
                ..Default::default()
            },
            estimated_reclaimable_bytes: 41_568_102_220,
        }
    }

    #[test]
    fn render_text_prints_all_six_sections_and_every_bucket_including_zeros() {
        let text = render_text(&sample_report());
        for section in [
            "AutoSpec Resource Health",
            "Worktrees",
            "Branches",
            "Docker containers",
            "Docker images",
            "Docker volumes",
            "Processes",
            "Estimated reclaimable disk: 38.7 GB",
        ] {
            assert!(text.contains(section), "missing {section:?} in:\n{text}");
        }
        // Zero buckets are printed, not omitted.
        assert!(
            text.contains("  quarantined             0"),
            "zero quarantined bucket printed:\n{text}"
        );
        assert!(
            text.contains("  stale                   0"),
            "zero stale bucket printed:\n{text}"
        );
    }

    #[test]
    fn render_json_emits_parseable_buckets_matching_the_text_report() {
        let report = sample_report();
        let value: serde_json::Value =
            serde_json::from_str(&render_json(&report)).expect("valid JSON");
        assert_eq!(value["command"], "doctor resources");
        assert_eq!(value["worktrees"]["active"], 1);
        assert_eq!(value["worktrees"]["stale"], 2);
        assert_eq!(value["worktrees"]["orphaned"], 1);
        assert_eq!(value["worktrees"]["quarantined"], 1);
        assert_eq!(value["worktrees"]["unattributable"], 1);
        assert_eq!(value["branches"]["active"], 3);
        assert_eq!(value["containers"]["unattributable"], 13);
        assert_eq!(value["containers"]["orphaned"], 0);
        assert_eq!(value["volumes"]["active"], 0);
        assert_eq!(value["processes"]["active"], 2);
        assert_eq!(value["estimated_reclaimable_bytes"], 41_568_102_220u64);
        assert_eq!(value["estimated_reclaimable"], "38.7 GB");
        // Same five bucket keys everywhere.
        for key in [
            "worktrees",
            "branches",
            "containers",
            "images",
            "volumes",
            "processes",
        ] {
            let buckets = &value[key];
            for bucket in [
                "active",
                "stale",
                "orphaned",
                "quarantined",
                "unattributable",
            ] {
                assert!(buckets.get(bucket).is_some(), "{key} missing {bucket}");
            }
        }
    }

    // ---------- end-to-end against a real git repository ----------

    #[test]
    fn end_to_end_reports_every_worktree_and_branch_bucket() {
        let root = fresh_repo("e2e");
        commit_on(&root, "a.txt", "merged feature");
        git(&root, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
        // A branch merged into origin/main.
        git(&root, &["branch", "feat/merged", "HEAD"]);
        // An unmerged branch.
        commit_on(&root, "b.txt", "live feature");
        git(&root, &["branch", "feat/live"]);
        // A branch whose upstream is [gone].
        git(&root, &["branch", "feat/ghost"]);
        git(&root, &["config", "branch.feat/ghost.remote", "origin"]);
        git(
            &root,
            &["config", "branch.feat/ghost.merge", "refs/heads/ghost"],
        );
        // Worktrees: active, stale, quarantined, prunable (orphaned), detached (unattributable).
        // Paths are unique per run (derived from the temp root name) so parallel or
        // interrupted runs never collide on shared /tmp/wt-* names.
        let suffix = root.file_name().unwrap().to_string_lossy().to_string();
        let wt = |name: &str| {
            root.parent()
                .unwrap()
                .join(format!("{suffix}-{name}"))
                .to_string_lossy()
                .to_string()
        };
        for name in [
            "wt-detached",
            "wt-active",
            "wt-stale",
            "wt-locked",
            "wt-orphan",
        ] {
            cleanup_dir(wt(name));
        }
        git(
            &root,
            &[
                "worktree",
                "add",
                "--detach",
                wt("wt-detached").as_str(),
                "HEAD",
            ],
        );
        let main_sha = git_output(&root, &["rev-parse", "HEAD"]).expect("rev-parse");
        git(
            &root,
            &["worktree", "add", wt("wt-active").as_str(), "-b", "wt/live"],
        );
        git(
            &root,
            &["worktree", "add", wt("wt-stale").as_str(), "feat/merged"],
        );
        git(
            &root,
            &[
                "worktree",
                "add",
                wt("wt-locked").as_str(),
                "-b",
                "wt/locked",
                main_sha.as_str(),
            ],
        );
        git(&root, &["worktree", "lock", wt("wt-locked").as_str()]);
        git(
            &root,
            &[
                "worktree",
                "add",
                wt("wt-orphan").as_str(),
                "-b",
                "wt/orphan",
            ],
        );
        cleanup_dir(wt("wt-orphan"));
        // Simulate the same repo as the drain loop: a remote origin with owner/repo.
        git(
            &root,
            &[
                "remote",
                "add",
                "origin",
                "https://example.invalid/autospec/core.git",
            ],
        );
        let report = collect(&root);
        assert_eq!(
            report.worktrees,
            Buckets {
                active: 2,
                stale: 1,
                orphaned: 1,
                quarantined: 1,
                unattributable: 1,
            }
        );
        assert_eq!(
            report.branches,
            Buckets {
                active: 5,   // main, feat/live, wt/live, wt/locked, wt/orphan
                stale: 1,    // feat/merged
                orphaned: 1, // feat/ghost ([gone] upstream)
                ..Default::default()
            }
        );
        for name in ["wt-detached", "wt-active", "wt-stale", "wt-locked"] {
            cleanup_dir(wt(name));
        }
        cleanup_dir(&root);
    }

    #[test]
    fn collect_is_read_only_and_exits_clean_without_a_ledger() {
        let root = fresh_repo("nolegder");
        // No ledger, no origin, no heartbeat directory — every bucket is zero
        // except the main worktree and main branch, and the call succeeds.
        let report = collect(&root);
        assert_eq!(report.worktrees.active, 1);
        assert_eq!(report.branches.active, 1);
        assert_eq!(report.estimated_reclaimable_bytes, 0);
        let rendered = render_text(&report);
        assert!(rendered.contains("Estimated reclaimable disk: 0 B"));
        let json: serde_json::Value =
            serde_json::from_str(&render_json(&report)).expect("valid JSON");
        assert_eq!(json["estimated_reclaimable"], "0 B");
        cleanup_dir(&root);
    }

    #[test]
    fn run_in_renders_json_and_help_and_rejects_unknown_flags() {
        let root = fresh_repo("runin");
        let out = run_in(&root, &["--json".to_string()]);
        assert!(out.is_ok(), "{out:?}");
        assert!(run_in(&root, &["--help".to_string()]).is_ok());
        let error = run_in(&root, &["--wat".to_string()]).expect_err("unknown flag");
        assert!(error.contains("unknown argument"));
        cleanup_dir(&root);
    }

    // ---------- 6500-branch performance fixture ----------

    #[test]
    fn six_thousand_five_hundred_branches_collect_in_under_15_seconds() {
        let root = fresh_repo("perf6500");
        git(&root, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
        // 6000 merged branches: point straight at the origin/main tip by writing
        // loose refs directly (no commit per branch — this is the fast path).
        let tip = git_output(&root, &["rev-parse", "HEAD"]).expect("rev-parse");
        let refs_dir = root.join(".git").join("refs").join("heads");
        std::fs::create_dir_all(&refs_dir).expect("refs dir");
        let mut packed = String::new();
        for i in 0..6000u32 {
            packed.push_str(&format!("{tip} refs/heads/perf/merged-{i:04}\n"));
        }
        std::fs::write(root.join(".git").join("packed-refs"), packed).expect("packed-refs");
        // 499 unmerged branches need distinct commits.
        for i in 0..499u32 {
            commit_on(&root, &format!("perf/{i:03}.txt"), &format!("perf {i}"));
            git(&root, &["branch", &format!("perf/live-{i:03}")]);
        }
        // 6500 branches total: main + 6000 merged + 499 unmerged.
        let start = std::time::Instant::now();
        let report = collect(&root);
        let elapsed = start.elapsed();
        assert_eq!(
            report.branches.stale, 6000,
            "the 6000 perf/merged-* branches are merged into origin/main"
        );
        // main is 499 commits ahead of origin/main, so it stays active.
        assert_eq!(report.branches.active, 500);
        assert!(
            elapsed.as_secs_f64() < 15.0,
            "collect took {elapsed:?} (budget: 15s)"
        );
        eprintln!("6500-branch collect: {elapsed:?}");
        cleanup_dir(&root);
    }

    // ---------- read-only guard ----------

    /// The doctor command must never issue destructive git/docker operations.
    /// Scan the non-test source of every file in this command tree.
    #[test]
    fn doctor_source_contains_no_destructive_operations() {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        let doctor_dir = manifest.join("src").join("commands").join("doctor");
        let mut files = vec![manifest.join("src").join("commands").join("doctor.rs")];
        if let Ok(entries) = std::fs::read_dir(&doctor_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|ext| ext == "rs") {
                    files.push(path);
                }
            }
        }
        assert!(!files.is_empty(), "expected doctor sources to exist");
        for file in &files {
            let text = std::fs::read_to_string(file)
                .unwrap_or_else(|err| panic!("read {}: {err}", file.display()));
            // Test-only cleanup code may reference the external tools; scan the
            // production code only.
            let scanned = &text[..text.find("#[cfg(test)]").unwrap_or(text.len())];
            // Tokens are concatenated so this test itself does not trip the scan.
            let forbidden = [
                "git worktree rem".to_string() + "ove",
                "git branch -D".to_string(),
                "git branch -d".to_string(),
                "git push --dele".to_string() + "te",
                "docker r".to_string() + "m",
                "image pr".to_string() + "une",
                "volume pr".to_string() + "une",
                "system pr".to_string() + "une",
                "pr".to_string() + "une",
                "ki".to_string() + "ll",
            ];
            for token in &forbidden {
                assert!(
                    !scanned.contains(token.as_str()),
                    "{} contains destructive token {token:?}",
                    file.display()
                );
            }
        }
    }

    // ---------- docker (gated on availability) ----------

    fn docker_available() -> bool {
        std::process::Command::new("docker")
            .args(["version", "--format", "{{.Client.Version}}"])
            .output()
            .map(|out| out.status.success())
            .unwrap_or(false)
    }

    #[test]
    fn managed_docker_image_is_active_with_nonzero_reclaimable() {
        if !docker_available() {
            eprintln!("SKIP: docker not available");
            return;
        }
        let root = fresh_repo("docker");
        let build_dir = temp_root("docker-build");
        std::fs::write(build_dir.join("payload.txt"), "x".repeat(4096)).expect("payload");
        std::fs::write(
            build_dir.join("Dockerfile"),
            "FROM scratch\nCOPY payload.txt /payload.txt\n",
        )
        .expect("Dockerfile");
        let tag = format!(
            "autospec-doctor-resources-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        let status = std::process::Command::new("docker")
            .args([
                "build",
                "-q",
                "--label",
                "autospec.managed=true",
                "-t",
                &tag,
                ".",
            ])
            .current_dir(&build_dir)
            .status()
            .expect("run docker build");
        cleanup_dir(&build_dir);
        if !status.success() {
            eprintln!("SKIP: docker build failed");
            return;
        }
        // Verify the observer can actually see the label before asserting on it.
        let images = std::process::Command::new("docker")
            .args(["images", &tag, "--format", "{{json .}}"])
            .output()
            .expect("run docker images");
        let stdout = String::from_utf8_lossy(&images.stdout);
        if !stdout.contains("autospec.managed") {
            eprintln!("SKIP: docker image labels not surfaced");
            let _ = std::process::Command::new("docker")
                .args(["rmi", &tag])
                .status();
            cleanup_dir(&root);
            return;
        }
        let report = collect(&root);
        let _ = std::process::Command::new("docker")
            .args(["rmi", &tag])
            .status();
        assert!(
            report.images.active >= 1,
            "managed image must be active: {report:?}"
        );
        assert!(
            report.estimated_reclaimable_bytes > 0,
            "managed image must carry a size"
        );
        cleanup_dir(&root);
    }
}
