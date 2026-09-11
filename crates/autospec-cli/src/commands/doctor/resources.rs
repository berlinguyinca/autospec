//! `autospec doctor resources` — per-type resource health (spec §24.3).
//!
//! Prints per-type counts for worktrees, branches, containers, images,
//! volumes, and processes, each broken into the §24.3 buckets (active,
//! stale, orphaned, quarantined, unattributable), plus an estimated
//! reclaimable-disk figure.
//!
//! Observation only. This module never deletes, prunes, kills, or mutates
//! anything:
//!
//! * git is queried with read-only subcommands (`worktree list --porcelain`,
//!   `for-each-ref`, `rev-parse --abbrev-ref`, `branch --merged`);
//! * Docker goes through `autospec_core::resources::observe_docker`, which
//!   issues read-only listing subcommands only and fails open (empty
//!   observation + `docker_unavailable`) when the CLI is missing;
//! * processes go through `autospec_core::resources::observe_processes`,
//!   which only reads heartbeat files.
//!
//! Bucketing rules (spec §19, §24.3, §36):
//!
//! * An `External` observation is always `unattributable` and is NEVER folded
//!   into `orphaned` — an external resource may be seen, it is never ours.
//! * Reclaimable disk sums `size_bytes` over non-`External` observations
//!   only. The sizes come from the #3190/#3191 observers; this command never
//!   recomputes a size, and an unlabeled (external) container contributes
//!   exactly 0 bytes.
//! * `merged stale` (branches only) counts local branches merged into
//!   `origin/main`.
//!
//! The command exits 0 even when no resource ledger exists: it aggregates
//! live observations and never requires the ledger DB.

use autospec_core::resources::{
    observe_docker, observe_processes, ObservedResource, OwnershipClass, ResourceType,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

const USAGE: &str = "autospec doctor resources [--json]\n\n\
Print per-type resource health (spec §24.3): worktrees, branches,\n\
containers, images, volumes, and processes, each broken into the buckets\n\
active / merged stale / stale / orphaned / quarantined / unattributable,\n\
plus an estimated reclaimable-disk figure.\n\n\
Observation only — nothing is deleted, pruned, or killed. Reclaimable disk\n\
sums the size_bytes carried by attributable (non-external) observations;\n\
unlabeled containers contribute 0 bytes.\n\n\
OPTIONS:\n\
    --json    emit the report as a single JSON object\n\n\
EXIT CODES:\n\
    0  report rendered (a missing ledger is not an error)\n\
    1  observations could not be gathered";

/// One §24.3 bucket set for a single resource type. `merged_stale` is only
/// meaningful for branches (merged into `origin/main`); it stays 0 elsewhere.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Buckets {
    pub active: u64,
    pub merged_stale: u64,
    pub stale: u64,
    pub orphaned: u64,
    pub quarantined: u64,
    pub unattributable: u64,
}

/// The full §24.3 report: one bucket set per observed type plus the
/// estimated reclaimable disk.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    pub worktrees: Buckets,
    pub branches: Buckets,
    pub containers: Buckets,
    pub images: Buckets,
    pub volumes: Buckets,
    pub processes: Buckets,
    pub reclaimable_bytes: u64,
}

/// Render the §24.3 report for `root` (a git worktree). `as_json` switches
/// the text table for a machine-readable object with the same buckets.
pub fn run(root: &Path, args: &[String]) -> Result<String, String> {
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        return Ok(USAGE.to_string());
    }
    let as_json = args.iter().any(|arg| arg == "--json");
    let report = gather(root)?;
    Ok(if as_json {
        render_json(&report)
    } else {
        render_text(&report)
    })
}

/// Gather every observation and fold it into one report. Read-only: git
/// listing subcommands, the core Docker observer, and the core process
/// observer. Never touches the resource ledger.
fn gather(root: &Path) -> Result<Report, String> {
    let mut report = Report::default();

    let (worktrees, branches) = observe_git(root)?;
    report.worktrees = bucket_worktrees(&worktrees);
    report.branches = bucket_branches(&branches);

    let docker = observe_docker().map_err(|error| format!("docker observation failed: {error}"))?;
    let mut attributable_docker: Vec<ObservedResource> = Vec::new();
    for observation in &docker {
        let slot = match observation.resource_type {
            ResourceType::DockerContainer => &mut report.containers,
            ResourceType::DockerImage => &mut report.images,
            ResourceType::DockerVolume => &mut report.volumes,
            // Networks and other types are out of scope for §24.3.
            _ => continue,
        };
        // External is observed but never attributable: it goes to its own
        // bucket and is never folded into orphaned (spec §19).
        if observation.ownership == OwnershipClass::External {
            slot.unattributable += 1;
        } else {
            slot.active += 1;
            attributable_docker.push(observation.clone());
        }
    }

    let processes = observe_process_observations(root)?;
    for observation in &processes {
        if observation.ownership == OwnershipClass::External {
            report.processes.unattributable += 1;
        } else {
            report.processes.active += 1;
        }
    }

    // Reclaimable disk: the sum of the size_bytes the observers already
    // carried, over attributable entries only. Never recomputed here, and an
    // external entry contributes exactly 0 bytes.
    report.reclaimable_bytes = attributable_docker
        .iter()
        .chain(processes.iter())
        .filter_map(|observation| observation.size_bytes)
        .sum();

    Ok(report)
}

/// A single worktree line from `git worktree list --porcelain`.
struct GitWorktree {
    prunable: bool,
}

/// Local-branch inventory from the read-only git listing commands.
struct GitBranches {
    /// The checked-out branch, if HEAD is not detached.
    current: Option<String>,
    /// Every local branch name.
    all: Vec<String>,
    /// The local branch names `git branch --merged origin/main` reported.
    merged_into_origin_main: HashSet<String>,
}

/// Read-only git observation: worktree list, branch list, current branch,
/// and branches merged into `origin/main`. Every subcommand is a listing
/// verb; this function never passes git a mutating argument.
fn observe_git(root: &Path) -> Result<(Vec<GitWorktree>, GitBranches), String> {
    let worktrees = parse_worktrees(&git_stdout(
        root,
        &["worktree", "list", "--porcelain"],
        "git worktree list",
    )?);

    let all = git_stdout(
        root,
        &["for-each-ref", "refs/heads", "--format=%(refname:short)"],
        "git for-each-ref",
    )?
    .lines()
    .map(|line| line.trim().to_string())
    .filter(|line| !line.is_empty())
    .collect();

    let head = git_stdout(
        root,
        &["rev-parse", "--abbrev-ref", "HEAD"],
        "git rev-parse",
    )?;
    let head = head.trim().to_string();
    let current = (head != "HEAD").then_some(head);

    // `origin/main` may not exist (fresh repo, no fetch). Merged-into is
    // then simply empty rather than an error: merged-stale is 0.
    let merged_into_origin_main = match git_stdout(
        root,
        &["branch", "--merged", "origin/main"],
        "git branch --merged origin/main",
    ) {
        Ok(output) => output
            .lines()
            .map(|line| line.trim_start_matches(['*', ' ']).trim().to_string())
            .filter(|line| !line.is_empty())
            .collect(),
        Err(_) => HashSet::new(),
    };

    Ok((
        worktrees,
        GitBranches {
            current,
            all,
            merged_into_origin_main,
        },
    ))
}

fn git_stdout(root: &Path, args: &[&str], label: &str) -> Result<String, String> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|error| format!("{label} failed: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "{label} exited {}: {}",
            output.status.code().unwrap_or(-1),
            stderr.trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Parse `git worktree list --porcelain`: blocks start at `worktree <path>`;
/// a `prunable` line marks a worktree whose directory is gone.
fn parse_worktrees(output: &str) -> Vec<GitWorktree> {
    let mut worktrees: Vec<GitWorktree> = Vec::new();
    for line in output.lines() {
        if line.starts_with("worktree ") {
            worktrees.push(GitWorktree { prunable: false });
        } else if line.starts_with("prunable") {
            if let Some(last) = worktrees.last_mut() {
                last.prunable = true;
            }
        }
    }
    worktrees
}

/// Worktree buckets: a worktree whose directory is still present (the
/// primary checkout included) is active; a registered-but-prunable
/// worktree is stale. Orphaned / quarantined / unattributable stay 0 —
/// those need the ledger and quarantine machinery, which are out of scope
/// for this subcommand.
fn bucket_worktrees(worktrees: &[GitWorktree]) -> Buckets {
    let mut buckets = Buckets::default();
    for worktree in worktrees {
        if worktree.prunable {
            buckets.stale += 1;
        } else {
            buckets.active += 1;
        }
    }
    buckets
}

/// Branch buckets: the checked-out branch is active; local branches merged
/// into `origin/main` are merged stale; every other local branch is stale.
fn bucket_branches(branches: &GitBranches) -> Buckets {
    let mut buckets = Buckets::default();
    if branches.current.is_some() {
        buckets.active = 1;
    }
    for name in &branches.all {
        if Some(name.as_str()) == branches.current.as_deref() {
            continue;
        }
        if branches.merged_into_origin_main.contains(name) {
            buckets.merged_stale += 1;
        } else {
            buckets.stale += 1;
        }
    }
    buckets
}

/// The heartbeat roots this command reads, in priority order: the
/// `AUTOSPEC_PROCESS_HEARTBEAT_DIR` override, the worktree's own
/// `.autospec/process-heartbeats`, and the per-user
/// `~/.autospec/process-heartbeats`.
fn process_heartbeat_roots(root: &Path) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(dir) = std::env::var("AUTOSPEC_PROCESS_HEARTBEAT_DIR") {
        if !dir.is_empty() {
            roots.push(PathBuf::from(dir));
        }
    }
    roots.push(root.join(".autospec").join("process-heartbeats"));
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            roots.push(
                PathBuf::from(home)
                    .join(".autospec")
                    .join("process-heartbeats"),
            );
        }
    }
    roots.dedup();
    roots
}

/// Observe child processes from every candidate heartbeat root. Each root
/// is read twice: for its immediate files and for each immediate
/// subdirectory (per-slug directories). Observations are deduplicated by
/// external id so overlapping roots never double-count.
fn observe_process_observations(root: &Path) -> Result<Vec<ObservedResource>, String> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut observations: Vec<ObservedResource> = Vec::new();
    for candidate in process_heartbeat_roots(root) {
        let mut targets = vec![candidate.clone()];
        if let Ok(entries) = std::fs::read_dir(&candidate) {
            for entry in entries.flatten() {
                if entry.path().is_dir() {
                    targets.push(entry.path());
                }
            }
        }
        for target in targets {
            let batch = observe_processes(&target).map_err(|error| {
                format!(
                    "process observation failed at {}: {error}",
                    target.display()
                )
            })?;
            for observation in batch {
                if seen.insert(observation.external_id.clone()) {
                    observations.push(observation);
                }
            }
        }
    }
    Ok(observations)
}

/// Render the §24.3 text table. Every section always prints an
/// `unattributable` row, even when it is 0.
fn render_text(report: &Report) -> String {
    let mut out = String::new();
    out.push_str("AutoSpec Resource Health\n");
    out.push_str("────────────────────────────────────────\n\n");
    push_section(&mut out, "Worktrees", &report.worktrees, false);
    push_section(&mut out, "Branches", &report.branches, true);
    push_section(&mut out, "Docker containers", &report.containers, false);
    push_section(&mut out, "Docker images", &report.images, false);
    push_section(&mut out, "Docker volumes", &report.volumes, false);
    push_section(&mut out, "Processes", &report.processes, false);
    out.push_str(&format!(
        "Estimated reclaimable disk: {}\n",
        human_size(report.reclaimable_bytes)
    ));
    out
}

fn push_section(out: &mut String, title: &str, buckets: &Buckets, show_merged_stale: bool) {
    out.push_str(title);
    out.push('\n');
    push_row(out, "active", buckets.active);
    if show_merged_stale {
        push_row(out, "merged stale", buckets.merged_stale);
    }
    push_row(out, "stale", buckets.stale);
    push_row(out, "orphaned", buckets.orphaned);
    push_row(out, "quarantined", buckets.quarantined);
    push_row(out, "unattributable", buckets.unattributable);
    out.push('\n');
}

fn push_row(out: &mut String, label: &str, count: u64) {
    out.push_str(&format!("  {:<18} {:>4}\n", label, count));
}

/// One JSON object with the same buckets the text table prints, plus the
/// reclaimable total in bytes and human form.
fn render_json(report: &Report) -> String {
    #[derive(Serialize)]
    struct JsonReport<'a> {
        worktrees: &'a Buckets,
        branches: &'a Buckets,
        containers: &'a Buckets,
        images: &'a Buckets,
        volumes: &'a Buckets,
        processes: &'a Buckets,
        reclaimable_bytes: u64,
        reclaimable_human: String,
    }
    serde_json::to_string_pretty(&JsonReport {
        worktrees: &report.worktrees,
        branches: &report.branches,
        containers: &report.containers,
        images: &report.images,
        volumes: &report.volumes,
        processes: &report.processes,
        reclaimable_bytes: report.reclaimable_bytes,
        reclaimable_human: human_size(report.reclaimable_bytes),
    })
    .expect("Report serializes to JSON")
}

/// Human-readable byte count: `0 B`, `512 B`, `55.7 MB`, `38.7 GB`.
fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];
    if bytes == 0 {
        return "0 B".to_string();
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git_available() -> bool {
        std::process::Command::new("git")
            .arg("--version")
            .output()
            .is_ok_and(|output| output.status.success())
    }

    fn require_git() {
        assert!(
            git_available(),
            "git is required for these tests (spec: real git repo, no mocks)"
        );
    }

    /// A unique scratch directory for a test, removed on drop.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "autospec-doctor-resources-{tag}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0)
            ));
            std::fs::create_dir_all(&path).expect("create scratch dir");
            Scratch(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn init_repo(dir: &Path) -> PathBuf {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["init", "-q", "-b", "main", "repo"])
            .output()
            .expect("spawn git init");
        assert!(
            out.status.success(),
            "git init failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let repo = dir.join("repo");
        git_run(&repo, &["config", "user.name", "Doctor Test"]);
        git_run(&repo, &["config", "user.email", "doctor@example.com"]);
        std::fs::write(repo.join("hello.txt"), "hello\n").expect("write file");
        git_run(&repo, &["add", "hello.txt"]);
        git_run(&repo, &["commit", "-q", "-m", "initial"]);
        repo
    }

    fn git_run(repo: &Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .output()
            .unwrap_or_else(|error| panic!("spawn git {:?} failed: {error}", args));
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn observation(
        id: &str,
        rtype: ResourceType,
        ownership: OwnershipClass,
        size_bytes: Option<u64>,
    ) -> ObservedResource {
        ObservedResource {
            external_id: id.to_string(),
            resource_type: rtype,
            ownership,
            size_bytes,
            reasons: Vec::new(),
        }
    }

    /// A real git repo with one commit, one tracked file, plus 6500 local
    /// branches (loose refs) all merged into `refs/remotes/origin/main`.
    fn fixture_repo_with_6500_branches(dir: &Path) -> PathBuf {
        require_git();
        let repo = init_repo(dir);
        let head =
            git_stdout(&repo, &["rev-parse", "HEAD"], "git rev-parse HEAD").expect("rev-parse");
        // Loose ref files are exactly what `git update-ref` writes; creating
        // them directly keeps the 6500-ref fixture cheap on old git versions
        // (no `update-ref --stdin` before 2.38).
        let refs_dir = repo.join(".git").join("refs").join("heads");
        std::fs::create_dir_all(&refs_dir).expect("create refs/heads");
        for i in 0..6500 {
            std::fs::write(refs_dir.join(format!("bulk-{i:05}")), format!("{head}\n"))
                .expect("write branch ref");
        }
        let origin_dir = repo
            .join(".git")
            .join("refs")
            .join("remotes")
            .join("origin");
        std::fs::create_dir_all(&origin_dir).expect("create refs/remotes/origin");
        std::fs::write(origin_dir.join("main"), format!("{head}\n")).expect("write origin/main");
        repo
    }

    #[test]
    fn six_thousand_fifty_branch_fixture_renders_in_under_fifteen_seconds() {
        let dir = Scratch::new("big");
        let repo = fixture_repo_with_6500_branches(&dir.0);
        let started = std::time::Instant::now();
        let rendered = run(&repo, &[]).expect("report renders");
        let elapsed = started.elapsed();
        assert!(
            elapsed.as_secs() < 15,
            "6500-branch fixture took {elapsed:?}, must render in under 15s"
        );
        let report: Report =
            serde_json::from_str(&run(&repo, &["--json".to_string()]).expect("json"))
                .expect("json parses");
        assert_eq!(report.branches.active, 1);
        assert_eq!(report.branches.merged_stale, 6500);
        assert!(rendered.contains("Estimated reclaimable disk:"));
        assert!(rendered.contains("unattributable"));
    }

    #[test]
    fn thirteen_unlabeled_containers_contribute_zero_reclaimable_bytes() {
        let mut report = Report::default();
        let containers: Vec<ObservedResource> = (0..13)
            .map(|i| {
                observation(
                    &format!("unlabeled-{i}"),
                    ResourceType::DockerContainer,
                    OwnershipClass::External,
                    Some(1024 * 1024),
                )
            })
            .collect();
        for observation in &containers {
            let slot = match observation.resource_type {
                ResourceType::DockerContainer => &mut report.containers,
                ResourceType::DockerImage => &mut report.images,
                ResourceType::DockerVolume => &mut report.volumes,
                _ => continue,
            };
            if observation.ownership == OwnershipClass::External {
                slot.unattributable += 1;
            } else {
                slot.active += 1;
            }
        }
        assert_eq!(report.containers.unattributable, 13);
        assert_eq!(
            report.containers.orphaned, 0,
            "external must never fold into orphaned"
        );
        let reclaimable: u64 = containers
            .iter()
            .filter(|o| o.ownership != OwnershipClass::External)
            .filter_map(|o| o.size_bytes)
            .sum();
        assert_eq!(reclaimable, 0, "unlabeled containers contribute 0 bytes");
    }

    #[test]
    fn attributable_nonzero_size_counts_toward_reclaimable() {
        let managed = observation(
            "img-managed",
            ResourceType::DockerImage,
            OwnershipClass::RunExclusive,
            Some(50 * 1024 * 1024),
        );
        let unlabeled = observation(
            "img-unlabeled",
            ResourceType::DockerImage,
            OwnershipClass::External,
            Some(4096 * 1024 * 1024),
        );
        let observations = vec![managed.clone(), unlabeled];
        let reclaimable: u64 = observations
            .iter()
            .filter(|o| o.ownership != OwnershipClass::External)
            .filter_map(|o| o.size_bytes)
            .sum();
        assert_eq!(reclaimable, 50 * 1024 * 1024);
        assert_eq!(human_size(reclaimable), "50.0 MB");
        assert_ne!(
            human_size(reclaimable),
            "0 B",
            "nonzero size must never render as 0"
        );
    }

    #[test]
    fn json_report_has_an_unattributable_count_for_every_type() {
        let dir = Scratch::new("json");
        let repo = init_repo(&dir.0);
        let rendered = run(&repo, &["--json".to_string()]).expect("json renders");
        let parsed: serde_json::Value = serde_json::from_str(&rendered).expect("json parses");
        for section in [
            "worktrees",
            "branches",
            "containers",
            "images",
            "volumes",
            "processes",
        ] {
            let buckets = parsed
                .get(section)
                .unwrap_or_else(|| panic!("{section} missing from {rendered}"));
            assert!(
                buckets.get("unattributable").is_some(),
                "{section} must carry an unattributable count in --json"
            );
            for key in ["active", "stale", "orphaned", "quarantined"] {
                assert!(buckets.get(key).is_some(), "{section} missing {key}");
            }
        }
        assert!(parsed.get("reclaimable_bytes").is_some());

        let text = run(&repo, &[]).expect("text renders");
        assert_eq!(
            text.matches("unattributable").count(),
            6,
            "every section prints an unattributable row:\n{text}"
        );
    }

    #[test]
    fn exits_ok_for_a_repo_with_no_ledger() {
        let dir = Scratch::new("noleader");
        let repo = init_repo(&dir.0);
        // No ledger DB, no .autospec directory, no heartbeats: still Ok.
        assert!(!repo.join(".autospec").exists());
        let rendered = run(&repo, &[]).expect("must exit 0 without a ledger");
        assert!(rendered.contains("AutoSpec Resource Health"));
        assert!(rendered.contains("Estimated reclaimable disk: 0 B"));
    }

    /// Static ratchet: the source of this file must never gain a mutating
    /// git or docker subcommand. The forbidden fragments are assembled by
    /// concatenation so this test's own source does not trip the scan.
    #[test]
    fn no_destructive_commands_in_source() {
        let source = include_str!("resources.rs");
        let forbidden: Vec<String> = vec![
            ["git", " worktree", " remove"].concat(),
            ["git", " branch", " -D"].concat(),
            ["git", " branch", " -d "].concat(),
            ["git", " push", " --force"].concat(),
            ["git", " reset", " --hard"].concat(),
            ["git", " clean", " -f"].concat(),
            ["docker", " rm"].concat(),
            ["docker", " rmi"].concat(),
            ["docker", " volume", " rm"].concat(),
            ["docker", " network", " remove"].concat(),
            ["docker", " system", " prune"].concat(),
            ["docker", " container", " prune"].concat(),
            ["docker", " image", " prune"].concat(),
            ["rm", " -rf"].concat(),
            ["kill", " -9"].concat(),
        ];
        for fragment in &forbidden {
            assert!(
                !source.contains(fragment.as_str()),
                "doctor/resources.rs must stay read-only, found {fragment:?}"
            );
        }
    }

    #[test]
    fn human_size_formats() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(1024), "1.0 KB");
        assert_eq!(human_size(50 * 1024 * 1024), "50.0 MB");
        let gb = 40u64 * 1024 * 1024 * 1024;
        assert!(human_size(gb).ends_with("GB"));
    }
}
