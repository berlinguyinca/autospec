//! `autospec cleanup` — Phase 1 observation-only dry-run report (spec
//! `docs/specs/2026-08-16-resource-lifecycle-cleanup-design.md` §24.1, §25,
//! §36, §47 Phase 1).
//!
//! Phase 1 **executes nothing**. Without `--dry-run` the command fails closed
//! with exit code 2 and names Phase 1; with `--dry-run` it gathers read-only
//! observations (git worktrees + branches, Docker objects, child processes)
//! and renders the §25 dry-run report through
//! [`autospec_core::resources::dry_run::plan`]. No removal, quarantine move,
//! ledger write, or process signal is performed — the planner is the single
//! source of the WOULD REMOVE / WOULD QUARANTINE / WOULD REPORT mapping, and
//! this module only renders it.
//!
//! The command stays observation-only by construction: every observer it calls
//! issues only read-only subcommands (git listing verbs, Docker listing
//! verbs, heartbeat-file reads), and this module contains no destructive git
//! or Docker invocation (pinned by `source_contains_no_branch_delete_or_docker_rm`).
//! A dry-run therefore cannot mutate the tree: a test asserts `git
//! status --porcelain` is unchanged and the tree is byte-identical across a
//! dry-run.

use autospec_core::resources::dry_run::{plan, DryRunEntry, DryRunPlan, ProposedAction};
use autospec_core::resources::{
    observe_branches, observe_docker, observe_processes, observe_worktrees, ObservedResource,
    ResourceType,
};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::CommandFailure;

/// Render the §25 dry-run report for the observed resources and exit.
///
/// `--dry-run` is required: Phase 1 is observation-only, so a bare
/// `autospec cleanup` fails closed with a `CommandFailure::diagnostic`
/// (exit code 2) rather than attempting anything destructive.
pub fn run(args: &[String]) -> Result<(), CommandFailure> {
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_help();
        return Ok(());
    }
    let options = parse_options(args)?;
    if !options.dry_run {
        return Err(CommandFailure::diagnostic(
            "autospec cleanup: Phase 1 is observation-only; pass --dry-run to render the cleanup \
             plan (no removals or quarantine moves are executed yet).",
        ));
    }
    let repo = options.repo.unwrap_or_else(|| PathBuf::from("."));
    let observations = gather(&repo, &options.run)?;
    let plan = plan(&observations);
    let output = if options.json {
        render_json(&plan)
    } else {
        render_text(&plan)
    };
    println!("{output}");
    Ok(())
}

#[derive(Debug, Default)]
struct Options {
    dry_run: bool,
    run: Option<String>,
    repo: Option<PathBuf>,
    json: bool,
}

/// Parse the Phase-1 flag set. Unknown flags, a flag missing its value, and a
/// duplicated `--run`/`--repo` are all diagnostics (exit 2), never panics.
fn parse_options(args: &[String]) -> Result<Options, CommandFailure> {
    let mut options = Options::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--dry-run" => {
                options.dry_run = true;
                index += 1;
            }
            "--run" => {
                if options.run.is_some() {
                    return Err(CommandFailure::diagnostic("duplicate --run flag"));
                }
                options.run = Some(next_value(args, index, "--run")?);
                index += 2;
            }
            "--repo" => {
                if options.repo.is_some() {
                    return Err(CommandFailure::diagnostic("duplicate --repo flag"));
                }
                options.repo = Some(PathBuf::from(next_value(args, index, "--repo")?));
                index += 2;
            }
            "--json" => {
                options.json = true;
                index += 1;
            }
            other => {
                return Err(CommandFailure::diagnostic(format!(
                    "unknown autospec cleanup flag: {other}"
                )))
            }
        }
    }
    Ok(options)
}

fn next_value(args: &[String], index: usize, flag: &str) -> Result<String, CommandFailure> {
    args.get(index + 1).cloned().ok_or_else(|| {
        CommandFailure::diagnostic(format!(
            "{flag} requires a value (got none); expected: {flag} <value>"
        ))
    })
}

/// Gather every read-only observation over the given repository: git
/// worktrees and branches, Docker objects (which fail open to an empty list
/// when the binary is absent), and child processes. When `run` is `Some`, the
/// result is restricted to observations attributed to that run (fail-closed:
/// an observation whose run cannot be structurally determined is excluded,
/// never guessed — spec §36 Invariant 1).
fn gather(repo: &Path, run: &Option<String>) -> Result<Vec<ObservedResource>, CommandFailure> {
    let mut observations: Vec<ObservedResource> = Vec::new();
    observations.extend(
        observe_worktrees(repo)
            .map_err(|error| CommandFailure::diagnostic(error.to_string()))?,
    );
    observations.extend(
        observe_branches(repo)
            .map_err(|error| CommandFailure::diagnostic(error.to_string()))?,
    );
    observations.extend(
        observe_docker()
            .map_err(|error| CommandFailure::diagnostic(error.to_string()))?,
    );
    observations.extend(observe_processes_roots(repo)?);
    Ok(match run {
        Some(id) => observations
            .into_iter()
            .filter(|observation| run_id_of(observation).is_some_and(|run_id| run_id == id))
            .collect(),
        None => observations,
    })
}

/// The AutoSpec run id an observation belongs to, when it is determinable from
/// the observation alone.
///
/// Only a Git worktree under `.autospec/worktrees/<run-id>/` carries its run
/// id in its path (spec §13.2). No other observed resource type carries a run
/// id in the `ObservedResource` model, so those return `None`. This is
/// deliberately narrow: a `--run` filter must attribute a resource to a run
/// only when the attribution is structural, not inferred from a name or a
/// prefix (spec §36 Invariant 1). A worktree path that is not under
/// `.autospec/worktrees/` (including the primary checkout) has no run id.
fn run_id_of<'a>(observation: &'a ObservedResource) -> Option<&'a str> {
    if observation.resource_type != ResourceType::GitWorktree {
        return None;
    }
    let mut components = Path::new(&observation.external_id).components();
    while let Some(component) = components.next() {
        if component.as_os_str() != ".autospec" {
            continue;
        }
        if components.next().is_some_and(|next| next.as_os_str() == "worktrees") {
            return components.next().and_then(|run| run.as_os_str().to_str());
        }
    }
    None
}

/// Observe child processes from every candidate heartbeat root (the
/// `AUTOSPEC_PROCESS_HEARTBEAT_DIR` override, the repository's own
/// `.autospec/process-heartbeats`, and the per-user `~/.autospec/...`
/// directory), descending into per-slug subdirectories and deduplicating by
/// external id so overlapping roots never double-count. Each read is
/// read-only; a missing root simply yields no observations.
fn observe_processes_roots(repo: &Path) -> Result<Vec<ObservedResource>, CommandFailure> {
    let mut seen = HashSet::new();
    let mut observations: Vec<ObservedResource> = Vec::new();
    for candidate in process_heartbeat_roots(repo) {
        let mut targets = vec![candidate.clone()];
        if let Ok(entries) = std::fs::read_dir(&candidate) {
            for entry in entries.flatten() {
                if entry.path().is_dir() {
                    targets.push(entry.path());
                }
            }
        }
        for target in targets {
            let batch = observe_processes(&target)
                .map_err(|error| CommandFailure::diagnostic(error.to_string()))?;
            for observation in batch {
                if seen.insert(observation.external_id.clone()) {
                    observations.push(observation);
                }
            }
        }
    }
    Ok(observations)
}

/// The heartbeat roots this command reads, in priority order.
fn process_heartbeat_roots(repo: &Path) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(dir) = std::env::var("AUTOSPEC_PROCESS_HEARTBEAT_DIR") {
        if !dir.is_empty() {
            roots.push(PathBuf::from(dir));
        }
    }
    roots.push(repo.join(".autospec").join("process-heartbeats"));
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            roots.push(PathBuf::from(home).join(".autospec").join("process-heartbeats"));
        }
    }
    roots.dedup();
    roots
}

/// The §25 text report: entries grouped under WOULD REMOVE / WOULD QUARANTINE
/// / WOULD REPORT (each group omitted when it has no entries), followed by a
/// per-resource-type totals section. Every entry prints its resource, owner,
/// state, action, safety reason, and estimated bytes.
pub(crate) fn render_text(plan: &DryRunPlan) -> String {
    let mut out = String::new();
    out.push_str("autospec cleanup --dry-run (Phase 1: observation only; nothing is removed or quarantined)\n");
    if plan.entries.is_empty() {
        out.push_str("\n  (no resources)\n");
        return out;
    }
    let mut by_action: [(ProposedAction, Vec<&DryRunEntry>); 3] = [
        (ProposedAction::WouldRemove, Vec::new()),
        (ProposedAction::WouldQuarantine, Vec::new()),
        (ProposedAction::WouldReport, Vec::new()),
    ];
    for entry in &plan.entries {
        for slot in by_action.iter_mut() {
            if slot.0 == entry.action {
                slot.1.push(entry);
                break;
            }
        }
    }
    for (action, entries) in &by_action {
        if entries.is_empty() {
            continue;
        }
        out.push('\n');
        out.push_str(group_header(*action));
        out.push('\n');
        for entry in entries {
            out.push_str(&render_entry(entry));
        }
    }
    out.push('\n');
    out.push_str("TOTALS\n");
    for (resource_type, totals) in &plan.totals {
        out.push_str(&format!(
            "  {}: count={} would_remove={} would_quarantine={} would_report={} reclaimable_bytes={}\n",
            resource_type.as_str(),
            totals.count,
            totals.would_remove,
            totals.would_quarantine,
            totals.would_report,
            bytes_label(totals.reclaimable_bytes)
        ));
    }
    out
}

/// One entry block: the resource line plus the owner / state / action /
/// safety-reason / bytes fields the §25 report requires.
fn render_entry(entry: &DryRunEntry) -> String {
    let reasons = entry.safety_reasons.join("; ");
    format!(
        "  - {}: {}\n    owner: {}\n    state: {}\n    action: {}\n    safety reason: {}\n    bytes: {}\n",
        entry.resource_type.as_str(),
        entry.external_id,
        entry.owner.as_str(),
        entry.state,
        entry.action.as_str(),
        reasons,
        bytes_label(entry.reclaimable_bytes)
    )
}

fn group_header(action: ProposedAction) -> &'static str {
    match action {
        ProposedAction::WouldRemove => "WOULD REMOVE",
        ProposedAction::WouldQuarantine => "WOULD QUARANTINE",
        ProposedAction::WouldReport => "WOULD REPORT",
    }
}

/// `None` (unknown) renders as `unknown`, never `0` — `0` means "measured and
/// genuinely empty", a different fact from "not measured".
fn bytes_label(bytes: Option<u64>) -> String {
    match bytes {
        Some(count) => count.to_string(),
        None => "unknown".to_string(),
    }
}

/// The plan as one JSON object (`entries` plus per-type `totals`).
pub(crate) fn render_json(plan: &DryRunPlan) -> String {
    serde_json::to_string_pretty(plan).expect("DryRunPlan serializes to JSON")
}

fn print_help() {
    let help = help_text();
    println!("{help}");
}

fn help_text() -> &'static str {
    "autospec cleanup

Phase 1 (observation only): renders the resource-cleanup dry-run report
(spec §24.1, §25, §36, §47). No removals or quarantine moves are executed;
without --dry-run the command fails closed with exit code 2.

USAGE:
    autospec cleanup --dry-run [--run <run-id>] [--repo <path>] [--json]

FLAGS:
    --dry-run      render the WOULD REMOVE / WOULD QUARANTINE / WOULD REPORT plan
    --run <id>     report only the resources attributed to one run (worktree path)
    --repo <path>  the git repository to observe (default: current directory)
    --json         emit the plan as one JSON object
    -h, --help     print this help

EXIT CODES:
    0  dry-run report rendered
    2  --dry-run not passed (Phase 1 is observation-only), or a bad flag"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::CommandFailureKind;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn git(repo: &Path, args: &[&str]) -> String {
        let output = std::process::Command::new("git")
            .current_dir(repo)
            .args(args)
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
            "autospec-cleanup-cli-{label}-{}-{nonce}",
            std::process::id()
        ))
    }

    /// A real git repository with one commit (issue #3195: real repos, no
    /// mocks).
    fn init_repo(label: &str) -> PathBuf {
        let dir = temp_root(label);
        std::fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init", "-q"]);
        git(&dir, &["config", "user.email", "cleanup@test"]);
        git(&dir, &["config", "user.name", "cleanup"]);
        std::fs::write(dir.join("f"), "base").unwrap();
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-qm", "init"]);
        dir
    }

    /// Add a run-exclusive worktree under `<repo>/.autospec/worktrees/<run>/implement`.
    fn add_managed_worktree(repo: &Path, run_id: &str, branch: &str) -> PathBuf {
        let worktree = repo
            .join(".autospec")
            .join("worktrees")
            .join(run_id)
            .join("implement");
        git(
            repo,
            &[
                "worktree",
                "add",
                "-q",
                worktree.to_str().unwrap(),
                "-b",
                branch,
            ],
        );
        worktree
    }

    fn arg(value: &str) -> String {
        value.to_string()
    }

    /// Recursively snapshot (relative path, size, mtime) of a tree.
    fn snapshot_tree(root: &Path) -> Vec<(String, u64, SystemTime)> {
        let mut out: Vec<(String, u64, SystemTime)> = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();
                // `.git` is excluded deliberately: the dry-run reads repository state
                // by invoking git, and git rewrites `.git/index` as a side effect of
                // being read. Including it would assert that reading a repository does
                // not touch it -- false for reasons unrelated to this command. The
                // property under test is that the dry-run leaves the WORKING TREE alone.
                if path.file_name().is_some_and(|n| n == ".git") {
                    continue;
                }
                let meta = std::fs::metadata(&path).unwrap();
                if meta.is_dir() {
                    stack.push(path);
                } else {
                    let rel = path.strip_prefix(root).unwrap().display().to_string();
                    out.push((rel, meta.len(), meta.modified().unwrap()));
                }
            }
        }
        out.sort();
        out
    }

    #[test]
    fn without_dry_run_exits_2_and_names_phase_1() {
        let failure = run(&[]).expect_err("no --dry-run must fail closed");
        assert_eq!(failure.exit_code, 2);
        assert!(
            matches!(failure.kind, CommandFailureKind::Diagnostic),
            "the fail-closed refusal is a diagnostic: {failure:?}"
        );
        assert!(
            failure.message.contains("Phase 1"),
            "the refusal names Phase 1, got: {}",
            failure.message
        );
    }

    #[test]
    fn dry_run_exits_0_on_a_repo_with_no_managed_resources() {
        let repo = init_repo("empty");
        let result = run(&[arg("--dry-run"), arg("--repo"), arg(repo.to_str().unwrap())]);
        assert!(
            result.is_ok(),
            "a repo with no managed resources is a valid dry-run, not an error: {result:?}"
        );
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn dry_run_groups_entries_under_all_three_headers() {
        let repo = init_repo("groups");
        // A clean run-exclusive worktree -> WOULD REMOVE; a dirty one -> WOULD
        // QUARANTINE; the primary checkout (External) -> WOULD REPORT.
        add_managed_worktree(&repo, "as-clean", "autospec/1/implement/a");
        let dirty = add_managed_worktree(&repo, "as-dirty", "autospec/2/implement/b");
        std::fs::write(dirty.join("untracked.txt"), "data").unwrap();

        let observations = gather(&repo, &None).expect("gather observations");
        let plan = plan(&observations);
        let text = render_text(&plan);

        assert!(text.contains("WOULD REMOVE"), "\n{text}");
        assert!(text.contains("WOULD QUARANTINE"), "\n{text}");
        assert!(text.contains("WOULD REPORT"), "\n{text}");
        assert!(text.contains("as-clean"), "clean worktree under a header\n{text}");
        assert!(text.contains("as-dirty"), "dirty worktree under a header\n{text}");
        assert!(
            text.contains(repo.to_str().unwrap()),
            "primary checkout (unattributable) under WOULD REPORT\n{text}"
        );
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn unattributable_entries_never_appear_under_would_remove() {
        // Negative / misattribution case: every External observation lands
        // under WOULD REPORT, never WOULD REMOVE.
        let repo = init_repo("misattribute");
        // A user branch merely named with the word autospec is External.
        git(&repo, &["branch", "feature/autospec-notes"]);
        let observations = gather(&repo, &None).expect("gather observations");
        let plan = plan(&observations);
        for entry in &plan.entries {
            if entry.owner == autospec_core::resources::OwnershipClass::External {
                assert_ne!(
                    entry.action,
                    ProposedAction::WouldRemove,
                    "External {} was proposed for removal: {:?}",
                    entry.external_id,
                    entry.safety_reasons
                );
            }
        }
        let text = render_text(&plan);
        let would_remove = text
            .split("WOULD REMOVE")
            .nth(1)
            .and_then(|rest| rest.split("\n\n").next())
            .unwrap_or("");
        assert!(
            !would_remove.contains("feature/autospec-notes"),
            "the unattributable branch must not be listed under WOULD REMOVE:\n{text}"
        );
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn every_printed_entry_carries_owner_and_safety_reason() {
        let repo = init_repo("owner");
        add_managed_worktree(&repo, "as-a", "autospec/1/implement/a");
        let dirty = add_managed_worktree(&repo, "as-b", "autospec/2/implement/b");
        std::fs::write(dirty.join("u.txt"), "x").unwrap();

        let observations = gather(&repo, &None).expect("gather observations");
        let plan = plan(&observations);
        assert!(!plan.entries.is_empty(), "fixture must observe resources");
        let text = render_text(&plan);

        let owner_lines = text.lines().filter(|l| l.starts_with("    owner: ")).count();
        let reason_lines = text.lines().filter(|l| l.starts_with("    safety reason: ")).count();
        assert_eq!(
            owner_lines,
            plan.entries.len(),
            "every entry prints an owner field:\n{text}"
        );
        assert_eq!(
            reason_lines,
            plan.entries.len(),
            "every entry prints a safety reason field:\n{text}"
        );
        for line in text.lines().filter(|l| l.starts_with("    owner: ")) {
            assert!(
                !line["    owner: ".len()..].trim().is_empty(),
                "owner field must be non-empty: {line}"
            );
        }
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn dry_run_leaves_git_status_unchanged() {
        let repo = init_repo("status");
        add_managed_worktree(&repo, "as-s", "autospec/1/implement/s");
        // An uncommitted change makes the status non-trivial, so "unchanged"
        // is a real assertion, not two empty strings.
        std::fs::write(repo.join("untracked.txt"), "data").unwrap();
        let before = git(&repo, &["status", "--porcelain"]);
        assert!(
            before.contains("untracked.txt"),
            "fixture sanity: status must name the untracked file: {before}"
        );

        let result = run(&[arg("--dry-run"), arg("--repo"), arg(repo.to_str().unwrap())]);
        assert!(result.is_ok(), "{result:?}");

        let after = git(&repo, &["status", "--porcelain"]);
        assert_eq!(
            before, after,
            "a dry-run must not change `git status --porcelain`"
        );
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn dry_run_tree_stays_byte_identical() {
        let repo = init_repo("tree");
        add_managed_worktree(&repo, "as-t", "autospec/1/implement/t");
        let before = snapshot_tree(&repo);

        let result = run(&[arg("--dry-run"), arg("--repo"), arg(repo.to_str().unwrap())]);
        assert!(result.is_ok(), "{result:?}");

        let after = snapshot_tree(&repo);
        assert_eq!(
            before, after,
            "a dry-run must leave the tree byte-identical (path, size, mtime)"
        );
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn source_contains_no_branch_delete_or_docker_rm() {
        // Split tokens so this test's own source does not trip a naive scan of
        // the file for these literals.
        let source = include_str!("cleanup.rs");
        for fragment in [
            ["bra", "nch -D"].concat(),
            ["docker", " rm"].concat(),
        ] {
            assert!(
                !source.contains(fragment.as_str()),
                "cleanup.rs must not delete branches or docker containers: {fragment}"
            );
        }
    }

    #[test]
    fn dry_run_json_emits_a_single_plan_object() {
        let repo = init_repo("json");
        add_managed_worktree(&repo, "as-j", "autospec/1/implement/j");
        let observations = gather(&repo, &None).expect("gather observations");
        let plan = plan(&observations);
        let json = render_json(&plan);
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("plan json parses");
        assert!(parsed.is_object(), "the plan is one JSON object: {json}");
        assert!(parsed.get("entries").is_some(), "plan carries entries: {json}");
        assert!(parsed.get("totals").is_some(), "plan carries totals: {json}");
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn dry_run_json_flag_exits_0() {
        let repo = init_repo("jsonflag");
        let result = run(&[
            arg("--dry-run"),
            arg("--json"),
            arg("--repo"),
            arg(repo.to_str().unwrap()),
        ]);
        assert!(result.is_ok(), "{result:?}");
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn run_id_of_reads_run_id_from_the_worktree_path() {
        let repo = init_repo("runid");
        add_managed_worktree(&repo, "as-20260816-a31f", "autospec/9/implement/x");
        let worktrees = observe_worktrees(&repo).expect("observe worktrees");
        let managed = worktrees
            .iter()
            .find(|o| o.external_id.contains(".autospec/worktrees/as-20260816-a31f"))
            .expect("managed worktree observed");
        assert_eq!(run_id_of(managed), Some("as-20260816-a31f"));
        let main = worktrees
            .iter()
            .find(|o| o.external_id == repo.to_str().unwrap())
            .expect("primary checkout observed");
        assert_eq!(
            run_id_of(main),
            None,
            "the primary checkout has no run id"
        );
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn run_filter_restricts_to_the_matching_run_worktree() {
        let repo = init_repo("runfilter");
        let keep = add_managed_worktree(&repo, "as-keep", "autospec/1/implement/a");
        add_managed_worktree(&repo, "as-drop", "autospec/2/implement/b");

        let observations = gather(&repo, &Some("as-keep".to_string())).expect("gather filtered");
        let plan = plan(&observations);
        // Only the as-keep worktree remains: as-drop, the primary checkout, and
        // any non-worktree observations (whose run id is undeterminable) are
        // excluded, never guessed.
        assert_eq!(plan.entries.len(), 1, "one resource for one run: {plan:?}");
        assert_eq!(
            plan.entries[0].external_id,
            keep.to_str().unwrap(),
            "the kept entry is the requested run's worktree"
        );
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn unknown_flags_and_missing_values_are_diagnostics() {
        let cases: Vec<Vec<&str>> = vec![
            vec!["--dry-run", "--bogus"],
            vec!["--dry-run", "--run"],
            vec!["--dry-run", "--repo"],
            vec!["--dry-run", "--run", "a", "--run", "b"],
            vec!["--dry-run", "--repo", "x", "--repo", "y"],
        ];
        for case in &cases {
            let args: Vec<String> = case.iter().map(|s| s.to_string()).collect();
            let failure = run(&args).expect_err("these invocations must all fail: {case:?}");
            assert_eq!(failure.exit_code, 2);
            assert!(
                matches!(failure.kind, CommandFailureKind::Diagnostic),
                "case {case:?} must be a diagnostic, got: {failure:?}"
            );
            assert!(!failure.message.is_empty());
        }
    }

    #[test]
    fn dry_run_on_a_non_git_directory_is_a_diagnostic() {
        let dir = temp_root("notgit");
        std::fs::create_dir_all(&dir).unwrap();
        let failure = run(&[
            arg("--dry-run"),
            arg("--repo"),
            arg(dir.to_str().unwrap()),
        ])
        .expect_err("observing a non-git directory must fail closed");
        assert_eq!(failure.exit_code, 2);
        assert!(
            matches!(failure.kind, CommandFailureKind::Diagnostic),
            "{failure:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn help_lists_the_flags_and_exits_ok() {
        assert!(help_text().contains("--dry-run"));
        assert!(help_text().contains("--run"));
        assert!(help_text().contains("--repo"));
        assert!(help_text().contains("--json"));
        assert!(run(&[arg("--help")]).is_ok());
        assert!(run(&[arg("-h")]).is_ok());
    }
}
