//! The patch-to-PR conversion pass (issue #4388).
//!
//! Rebuilds the lost shell pass (`convpass.sh` / `convselect.sh`) as a Rust
//! subcommand. The pass:
//!
//! 1. enumerates agent patches (`$LLM/*/out/issue-*/changes.patch`),
//! 2. selects only those NOT already attempted — a live branch/PR, or a
//!    recorded HELD entry whose re-gate still holds, all disqualify,
//! 3. applies each to a branch off current `origin/<base>`,
//! 4. runs the affected crate's full gate (`fmt --check`, `build`, `clippy`,
//!    `test --no-fail-fast`) on the pinned toolchain,
//! 5. opens a PR per passing patch,
//! 6. records a HELD line with the reason for failures — never discards.
//!
//! The pass's *decisions* live in [`autospec_core::conversion_pass`]
//! (selection, and the unfed-vs-idle outcome). Its must-survive behaviours
//! are reused, not re-implemented: the authoritative `failures:` name list and
//! its declared-count cross-check is [`autospec_core::failure_attribution`];
//! the refusal of any conflict auto-resolution the pass cannot prove safe is
//! [`autospec_core::conflict_resolution`]; the HELD-as-queue re-gate
//! (re-attempt when the base moves, archive the stale) is
//! [`autospec_core::hold_memo`] and [`autospec_core::stored_output`].
//!
//! The command is side-effect-free by default: it plans the pass (enumerate +
//! select + report). `--apply` performs the real conversion (branch, gate,
//! PR, HELD).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use autospec_core::conflict_resolution::{classify_file, resolution_for, ResolutionPlan};
use autospec_core::conversion_pass::{select_fresh, PassOutcome, PatchCandidate};
use autospec_core::failure_attribution::attribute;
use autospec_core::hold_memo::{re_gate, HoldRecord};
use autospec_core::unfed_pass::PassCounters;
use serde_json::{json, Value};

use super::claim::{branch_liveness, BranchLiveness};
use super::CommandFailure;

/// The schema emitted by `autospec convert --json`.
pub const CONVERT_PLAN_SCHEMA: &str = "autospec.convert-plan.v1";

/// The branch-name prefix the pass uses for a conversion attempt.
pub const DEFAULT_BRANCH_PREFIX: &str = "conv-";

const USAGE: &str = "\
USAGE:
    autospec convert [--llm-root DIR] [--repo OWNER/NAME] [--base BRANCH]
                     [--held-file PATH] [--branch-prefix PREFIX]
                     [--apply] [--json] [ISSUE ...]

PLAN (default): enumerate $LLM/*/out/issue-*/changes.patch, select the patches
not already attempted (a live branch/PR, or a recorded HELD entry whose
re-gate still holds, disqualify), and report the plan. No mutations.

--apply: perform the real conversion of each selected patch — branch off
origin/<base>, full gate (fmt --check, build, clippy, test --no-fail-fast),
open a PR per passing patch, and record a HELD line for failures.

OPTIONS:
    --llm-root DIR        the agent-patch root (default: $LLM)
    --repo OWNER/NAME     the GitHub repo for PR liveness (default: gh)
    --base BRANCH         trunk to branch off origin/<base> (default: main)
    --held-file PATH      the HELD ledger (default: <llm-root>/held.txt)
    --branch-prefix P     conversion branch prefix (default: conv-)
    --apply               perform the conversion, not just the plan
    --json                machine-readable plan
    ISSUE ...             restrict the pass to these issue numbers";

pub fn run(args: &[String]) -> Result<(), CommandFailure> {
    // A bare `autospec convert` is NOT a help request: it is the incident
    // shape — the pass handed no work — and must run the unfed/plan logic so
    // it refuses naming the enumeration (or plans against `$LLM`) instead of
    // printing the idle `converted=0 held=0 skipped=0` line.
    match args.first().map(String::as_str) {
        Some("--help") | Some("-h") => {
            print_usage();
            Ok(())
        }
        _ => convert(args),
    }
}

fn print_usage() {
    println!("autospec convert — the patch-to-PR conversion pass\n\n{USAGE}");
}

#[derive(Debug, Clone)]
struct Options {
    llm_root: Option<PathBuf>,
    repo: Option<String>,
    base: String,
    held_file: Option<PathBuf>,
    branch_prefix: String,
    apply: bool,
    as_json: bool,
    issues: Vec<u64>,
}

fn parse_options(args: &[String]) -> Result<Options, CommandFailure> {
    let mut opts = Options {
        llm_root: None,
        repo: None,
        base: "main".to_string(),
        held_file: None,
        branch_prefix: DEFAULT_BRANCH_PREFIX.to_string(),
        apply: false,
        as_json: false,
        issues: Vec::new(),
    };
    let value = |args: &[String], i: &mut usize, flag: &str| -> Result<String, CommandFailure> {
        *i += 1;
        args.get(*i)
            .cloned()
            .ok_or_else(|| CommandFailure::diagnostic(format!("missing value for {flag}")))
    };
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].clone();
        match arg.as_str() {
            "--llm-root" => opts.llm_root = Some(PathBuf::from(value(args, &mut i, &arg)?)),
            "--repo" => opts.repo = Some(value(args, &mut i, &arg)?),
            "--base" => opts.base = value(args, &mut i, &arg)?,
            "--held-file" => opts.held_file = Some(PathBuf::from(value(args, &mut i, &arg)?)),
            "--branch-prefix" => opts.branch_prefix = value(args, &mut i, &arg)?,
            "--apply" => opts.apply = true,
            "--json" => opts.as_json = true,
            other if other.starts_with("--") => {
                return Err(CommandFailure::diagnostic(format!(
                    "unknown autospec convert option: {other}\n{USAGE}"
                )))
            }
            other => {
                let issue = other.parse::<u64>().map_err(|_| {
                    CommandFailure::diagnostic(format!(
                        "ISSUE must be a positive integer, got {other:?}\n{USAGE}"
                    ))
                })?;
                if issue == 0 {
                    return Err(CommandFailure::diagnostic(format!(
                        "ISSUE must be a positive integer, got 0\n{USAGE}"
                    )));
                }
                opts.issues.push(issue);
            }
        }
        i += 1;
    }
    Ok(opts)
}

/// The enumeration source for the pass: `--llm-root`, else `$LLM`. `None`
/// when there is no source at all — the pass was handed no work, which is a
/// different state from an idle pass that examined work and found nothing.
fn resolve_llm_root(explicit: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = explicit {
        return Some(path.to_path_buf());
    }
    std::env::var("LLM")
        .ok()
        .and_then(|v| {
            let trimmed = v.trim();
            (!trimmed.is_empty()).then(|| PathBuf::from(trimmed))
        })
}

/// One agent patch on disk: its node, issue, path, and input key.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PatchLocation {
    node: String,
    issue: u64,
    path: PathBuf,
    patch_key: String,
}

/// `issue-<N>` → `N`. Anything that is not exactly that shape is not a patch
/// directory and is skipped, never guessed.
fn parse_issue_name(name: &str) -> Option<u64> {
    let number = name.strip_prefix("issue-")?;
    let number = number.parse::<u64>().ok()?;
    (number > 0).then_some(number)
}

/// The patch's input key: its mtime (seconds since the epoch). The shell
/// selector memoed "already attempted" on the mtime, and a redispatched
/// agent's fresh work carries a new mtime and is re-offered.
fn patch_key(path: &Path) -> Result<String, CommandFailure> {
    let modified = fs::metadata(path)
        .and_then(|m| m.modified())
        .map_err(|error| CommandFailure::diagnostic(format!("stat {path:?}: {error}")))?;
    let secs = modified
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    Ok(secs.to_string())
}

/// Enumerate `$LLM/*/out/issue-*/changes.patch`. A missing or unreadable
/// `out` directory is skipped (the node simply has no patches); a missing
/// `changes.patch` in an issue directory is skipped. The result is sorted by
/// issue so the pass attempts patches in a stable order.
fn enumerate_patches(root: &Path) -> Result<Vec<PatchLocation>, CommandFailure> {
    let mut out: Vec<PatchLocation> = Vec::new();
    let nodes = fs::read_dir(root).map_err(|error| {
        CommandFailure::status(
            format!(
                "conversion pass: cannot read llm root {}: {error} — a broken enumeration \
                 source is not an idle pass",
                root.display()
            ),
            2,
        )
    })?;
    for node in nodes {
        let node = match node {
            Ok(node) => node,
            Err(error) => {
                return Err(CommandFailure::status(
                    format!("conversion pass: cannot read an llm root entry: {error}"),
                    2,
                ))
            }
        };
        let node_path = node.path();
        if !node_path.is_dir() {
            continue;
        }
        let node_name = node_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let out_dir = node_path.join("out");
        let Ok(issues) = fs::read_dir(&out_dir) else {
            continue;
        };
        for issue_dir in issues.flatten() {
            let issue_path = issue_dir.path();
            let Some(name) = issue_path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let Some(issue) = parse_issue_name(name) else {
                continue;
            };
            let patch = issue_path.join("changes.patch");
            if !patch.is_file() {
                continue;
            }
            let patch_key = patch_key(&patch)?;
            out.push(PatchLocation {
                node: node_name.clone(),
                issue,
                path: patch,
                patch_key,
            });
        }
    }
    out.sort_by(|a, b| a.issue.cmp(&b.issue));
    Ok(out)
}

/// The HELD ledger: one JSON line per recorded hold, parseable into a
/// [`HoldRecord`]. The pass reads it to decide which held patches still
/// disqualify (their re-gate holds) versus which are re-offered (the base
/// moved). Appending a HELD line is the pass's "never discard" step.
fn load_held(path: &Path) -> Result<BTreeMap<u64, HoldRecord>, CommandFailure> {
    let Ok(text) = fs::read_to_string(path) else {
        return Ok(BTreeMap::new());
    };
    let mut held = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Ok(record) = serde_json::from_str::<HoldRecord>(line) else {
            // A line that is not a HoldRecord is not a hold; skip it rather
            // than fail the whole pass on a stray line.
            continue;
        };
        held.insert(record.issue, record);
    }
    Ok(held)
}

/// The files that moved on the base since a hold's recorded base sha — the
/// input to the HELD-as-queue re-gate. `None` when the diff could not be run
/// (unknown base sha, not a git repo, no network): the caller then
/// over-re-gates, the safe direction (a needless re-derivation, never a
/// stale "still held").
fn base_changed_files(base_sha: &str, base_ref: &str) -> Option<Vec<String>> {
    let output = Command::new("git")
        .args(["diff", "--name-only", base_sha, base_ref])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect(),
    )
}

/// The two attempt-flags a patch carries, decided from the branch's liveness.
///
/// `branch_liveness` is the single definition of a live conversion attempt
/// (a checked-out worktree, or an open/merged pull request); a live attempt
/// disqualifies the patch. A dead branch (missing, or abandoned — closed
/// unmerged with no worktree) does not: redoing it overwrites the stale
/// branch, so the pass re-offers it. `Unknown` fails closed (disqualify):
/// offering a patch whose attempt state cannot be verified risks a duplicate.
fn attempt_flags(repo: Option<&str>, branch: &str) -> (bool, bool) {
    let Some(repo) = repo else {
        // No repo to ask: the liveness is not checked (a plan-only gap the
        // caller reports), so the patch is not disqualified on this axis.
        return (false, false);
    };
    match branch_liveness(repo, branch) {
        BranchLiveness::Live => (false, true),
        BranchLiveness::Dead => (false, false),
        BranchLiveness::Unknown => (false, true),
    }
}

/// The gate packages for the files a patch touches: `-p` one affected crate,
/// or `--workspace` when the patch spans crates or touches the root. The
/// gate is the affected crate's FULL gate, not a whole-repo gate — a
/// TypeScript-only patch must not be marked verified on 471 passing Rust
/// tests, and a single-crate patch should not pay for a whole-workspace run.
fn gate_packages(files: &[String]) -> Vec<String> {
    let mut core = false;
    let mut cli = false;
    let mut other = false;
    for file in files {
        if file.starts_with("crates/autospec-core/") {
            core = true;
        } else if file.starts_with("crates/autospec-cli/") {
            cli = true;
        } else {
            other = true;
        }
    }
    if (core || cli) && !other {
        if core && !cli {
            vec!["-p".to_string(), "autospec-core".to_string()]
        } else if cli && !core {
            vec!["-p".to_string(), "autospec-cli".to_string()]
        } else {
            vec![
                "-p".to_string(),
                "autospec-core".to_string(),
                "-p".to_string(),
                "autospec-cli".to_string(),
            ]
        }
    } else {
        vec!["--workspace".to_string()]
    }
}

/// The file paths a `changes.patch` touches, from its `+++ b/<path>` lines.
/// Binary additions (`+++ /dev/null` is a deletion; `+++ b/<path>` is the
/// new path) are read by their `b/` side.
fn patch_files(patch: &str) -> Vec<String> {
    patch
        .lines()
        .filter_map(|line| {
            line.strip_prefix("+++ b/").map(|p| p.trim().to_string())
        })
        .filter(|p| !p.is_empty() && p != "/dev/null")
        .collect()
}

/// The HELD reason for a conflict, naming each conflicted file and whether
/// its shape is a proven-safe keep-both merge or a refusal (single-value,
/// generated, or unclassifiable). The pass never writes a resolution it
/// cannot prove safe; this reason is what a human or a proven-safe resolver
/// acts on when the base next moves and the patch is re-offered.
fn conflict_reason(path: &str, content: &str) -> String {
    let shape = classify_file(path, content);
    match resolution_for(&shape) {
        ResolutionPlan::KeepBothInOrder | ResolutionPlan::KeepBothDeduplicated => {
            format!("{path}: shape {} (resolvable keep-both)", shape.as_str())
        }
        ResolutionPlan::Regenerate { .. } => {
            format!("{path}: shape {} (regenerate from source, not a merge)", shape.as_str())
        }
        ResolutionPlan::Refuse { reason } => {
            format!("{path}: shape {} (refused: {reason})", shape.as_str())
        }
    }
}

struct ConvertPlan {
    opts: Options,
    llm_root: PathBuf,
    examined: Vec<PatchLocation>,
    candidates: Vec<PatchCandidate>,
    outcome: PassOutcome,
}

impl ConvertPlan {
    fn selection(&self) -> autospec_core::conversion_pass::Selection {
        select_fresh(&self.candidates)
    }
}

fn build_plan(opts: Options, llm_root: PathBuf) -> Result<ConvertPlan, CommandFailure> {
    if !llm_root.is_dir() {
        return Err(CommandFailure::status(
            format!(
                "conversion pass: llm root {} does not exist — a broken enumeration source is \
                 not an idle pass",
                llm_root.display()
            ),
            2,
        ));
    }
    let enumerated = enumerate_patches(&llm_root)?;
    let examined: Vec<PatchLocation> = if opts.issues.is_empty() {
        enumerated
    } else {
        enumerated
            .into_iter()
            .filter(|p| opts.issues.contains(&p.issue))
            .collect()
    };

    let held_path = opts
        .held_file
        .clone()
        .unwrap_or_else(|| llm_root.join("held.txt"));
    let held = load_held(&held_path)?;
    let base_ref = format!("origin/{}", opts.base);

    // The repo for the liveness check: --repo, else gh inference (best-effort
    // for a plan; --apply requires it, checked at the apply stage).
    let repo = opts.repo.clone().or_else(infer_repo);
    if repo.is_none() && !opts.apply {
        eprintln!(
            "WARN: no --repo and gh could not infer one; PR liveness is not checked in this plan"
        );
    }

    let mut candidates = Vec::new();
    for patch in &examined {
        let branch = format!("{}{}", opts.branch_prefix, patch.issue);
        let (branch_exists, pull_request_exists) = attempt_flags(repo.as_deref(), &branch);

        // HELD is a queue: a recorded hold disqualifies only while its
        // re-gate still holds. A changed patch, or a dependent file that
        // moved on the base since the hold, re-offers it.
        let held_recorded = match held.get(&patch.issue) {
            Some(record) => {
                let changed = base_changed_files(&record.base_sha, &base_ref)
                    .unwrap_or_else(|| vec!["<base: unknown — over-re-gate>".to_string()]);
                re_gate(record, &patch.patch_key, &changed).is_still_held()
            }
            None => false,
        };

        candidates.push(PatchCandidate {
            issue: patch.issue,
            patch_key: patch.patch_key.clone(),
            branch_exists,
            pull_request_exists,
            held_recorded,
        });
    }

    let outcome = PassOutcome::Examined(PassCounters {
        examined: candidates.len(),
        converted: 0,
        held: 0,
        skipped: 0,
    });

    Ok(ConvertPlan {
        opts,
        llm_root,
        examined,
        candidates,
        outcome,
    })
}

/// The bare/unfed refusal: no enumeration source (`--llm-root` and `$LLM`)
/// means the pass was handed no work. It prints the unfed line — never the
/// idle `converted=0 held=0 skipped=0` line — and exits `2`.
fn unfed() -> CommandFailure {
    let line = PassOutcome::Unfed.line(
        "convert",
        "autospec convert",
        "enumerate $LLM/*/out/issue-*/changes.patch",
    );
    CommandFailure::status(line, 2)
}

fn convert(args: &[String]) -> Result<(), CommandFailure> {
    let opts = parse_options(args)?;
    let Some(llm_root) = resolve_llm_root(opts.llm_root.as_deref()) else {
        return Err(unfed());
    };
    let plan = build_plan(opts.clone(), llm_root)?;

    if plan.opts.apply {
        return run_apply(&plan);
    }
    render_plan(&plan)
}

fn render_plan(plan: &ConvertPlan) -> Result<(), CommandFailure> {
    let selection = plan.selection();
    let selection_line = selection.line(plan.candidates.len());

    if plan.opts.as_json {
        let disqualified: Vec<Value> = selection
            .disqualified
            .iter()
            .map(|(c, reason)| {
                json!({ "issue": c.issue, "reason": reason.as_str() })
            })
            .collect();
        let fresh: Vec<Value> = selection
            .fresh
            .iter()
            .map(|c| json!({ "issue": c.issue, "patch_key": c.patch_key }))
            .collect();
        let value = json!({
            "schema": CONVERT_PLAN_SCHEMA,
            "llm_root": plan.llm_root.display().to_string(),
            "base": plan.opts.base,
            "apply": false,
            "examined": plan.candidates.len(),
            "fresh": fresh,
            "disqualified": disqualified,
            "summary": selection_line,
        });
        println!("{value}");
        return Ok(());
    }

    println!("{selection_line}");
    for c in &selection.fresh {
        println!("  FRESH #{issue} {patch_key}", issue = c.issue, patch_key = c.patch_key);
    }
    for (c, reason) in &selection.disqualified {
        println!(
            "  SKIP  #{issue} ({reason}) {patch_key}",
            issue = c.issue,
            reason = reason.as_str(),
            patch_key = c.patch_key
        );
    }
    println!("{}", plan.outcome.line("convert", "autospec convert", "enumerate $LLM"));
    Ok(())
}

/// The real conversion of each selected patch (steps 3-6). Side effects are
/// confined to the pass's own branches/PRs and the HELD ledger.
fn run_apply(plan: &ConvertPlan) -> Result<(), CommandFailure> {
    if plan.opts.repo.is_none() {
        return Err(CommandFailure::diagnostic(
            "autospec convert --apply requires --repo OWNER/NAME (or a gh-inferable repo) to \
             check PR liveness and open PRs",
        ));
    }
    let repo: String = plan.opts.repo.clone().or_else(infer_repo).unwrap_or_default();

    // Fetch the trunk so the pass branches off current origin/<base>.
    let base_ref = format!("origin/{}", plan.opts.base);
    run_git(&["fetch", "origin"])?;

    let selection = plan.selection();
    let held_path = plan
        .opts
        .held_file
        .clone()
        .unwrap_or_else(|| plan.llm_root.join("held.txt"));
    let base_sha = run_git_capture(&["rev-parse", "HEAD"])?;

    let mut counters = PassCounters {
        examined: plan.candidates.len(),
        converted: 0,
        held: 0,
        skipped: selection.disqualified.len(),
    };

    for c in &selection.fresh {
        let patch = match plan.examined.iter().find(|p| p.issue == c.issue) {
            Some(p) => p,
            None => continue,
        };
        match apply_one(plan, &repo, &base_ref, &base_sha, patch) {
            ApplyResult::Converted => counters.converted += 1,
            ApplyResult::Held => counters.held += 1,
            ApplyResult::Archived => counters.skipped += 1,
        }
    }

    let outcome = PassOutcome::Examined(counters);
    println!("{}", outcome.line("convert", "autospec convert", "enumerate $LLM"));
    let _ = held_path; // the HELD ledger is written inside apply_one
    Ok(())
}

enum ApplyResult {
    /// The patch passed the gate and a PR was opened.
    Converted,
    /// The patch failed (conflict or gate failure); a HELD line was recorded.
    Held,
    /// The patch no longer applies to the base; it was archived, never
    /// discarded, and its issue returns to the eligible pool.
    Archived,
}

fn apply_one(
    plan: &ConvertPlan,
    repo: &str,
    base_ref: &str,
    base_sha: &str,
    patch: &PatchLocation,
) -> ApplyResult {
    let branch = format!("{}{}", plan.opts.branch_prefix, patch.issue);
    let worktree = std::env::temp_dir().join(format!("autospec-conv-{branch}"));
    let _ = fs::remove_dir_all(&worktree);
    // `-B` creates the conversion branch off current origin/<base>, resetting
    // it if it already exists. A fresh candidate has no live attempt, so any
    // existing branch is abandoned — redoing it overwrites the stale branch,
    // which is exactly the recovery #4146 prescribes.
    if let Err(error) = run_git_in(
        &std::env::current_dir().ok().unwrap_or_default(),
        &[
            "worktree",
            "add",
            "-B",
            &branch,
            worktree.to_str().unwrap_or_default(),
            base_ref,
        ],
    ) {
        eprintln!("WARN: worktree add for #{issue} failed: {error}; holding", issue = patch.issue);
        return record_held_and_result(plan, base_sha, patch, &format!("worktree add failed: {error}"));
    }

    // Step 3: apply to the branch off current origin/<base>.
    let apply_output = run_capture_in(&worktree, &["apply", "--3way", patch.path.to_str().unwrap_or_default()]);
    let apply_rc = apply_output.as_ref().and_then(|o| o.status.code());
    let apply_stdout = apply_output
        .as_ref()
        .map(|o| format!("{}\n{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)))
        .unwrap_or_default();

    let lifecycle = classify_apply(&apply_stdout, apply_rc);
    let result = match lifecycle {
        ApplyLifecycle::Superseded => {
            // Stale: main moved past it. Archive, never discard.
            let _ = fs::create_dir_all(patch.path.parent().unwrap_or(Path::new(".")).join("superseded"));
            let archive = autospec_core::stored_output::superseded_archive_path(
                patch.path.parent().unwrap_or(Path::new(".")),
                patch.path.file_name().unwrap_or_default().to_str().unwrap_or_default(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0),
            );
            let _ = fs::rename(&patch.path, &archive);
            teardown_worktree(&worktree);
            ApplyResult::Archived
        }
        ApplyLifecycle::Conflict => {
            // Refuse any auto-resolution the pass cannot prove safe (behavior
            // 3): a conflict is HELD with the conflicted files' shapes, never
            // merged blind.
            let reason = conflict_summary(&worktree);
            teardown_worktree(&worktree);
            record_held_and_result(plan, base_sha, patch, &reason)
        }
        ApplyLifecycle::Applied => {
            // Step 4: the affected crate's full gate on the pinned toolchain.
            let files = read_patch_files(&patch.path);
            let packages = gate_packages(&files);
            let gate = run_gate(&worktree, &packages);
            match gate {
                GateResult::Pass => {
                    // Step 5: open a PR per passing patch.
                    let opened = open_pr(repo, &worktree, &branch, patch);
                    teardown_worktree(&worktree);
                    if opened {
                        ApplyResult::Converted
                    } else {
                        // A PR that could not be opened is not a pass: hold it.
                        record_held_and_result(plan, base_sha, patch, "PR could not be opened")
                    }
                }
                GateResult::Fail(output) => {
                    // Step 6: record a HELD line with the failing-test set from
                    // the authoritative failures: block, never the progress
                    // stream (behavior 2).
                    let failure_note = failing_tests_note(&output);
                    teardown_worktree(&worktree);
                    record_held_and_result(plan, base_sha, patch, &format!("gate failed: {failure_note}"))
                }
            }
        }
    };
    result
}

/// How a `git apply --3way` attempt went: it applied, it conflicted, or the
/// patch no longer applies to the base (superseded).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApplyLifecycle {
    Applied,
    Conflict,
    Superseded,
}

fn classify_apply(output: &str, rc: Option<i32>) -> ApplyLifecycle {
    let lower = output.to_ascii_lowercase();
    if lower.contains("patch does not apply")
        || lower.contains("already applied")
        || lower.contains("no diff")
    {
        return ApplyLifecycle::Superseded;
    }
    if rc == Some(0) {
        return ApplyLifecycle::Applied;
    }
    if lower.contains("conflict") || lower.contains("could not apply") || lower.contains("unmerged") {
        return ApplyLifecycle::Conflict;
    }
    // A nonzero apply with no recognized signature: fail closed as a
    // conflict (hold it), never read as applied.
    ApplyLifecycle::Conflict
}

/// The files a worktree left in a conflicted (unmerged) state, each with its
/// shape — the refusal reason for the pass's conservative no-auto-resolve
/// policy.
fn conflict_summary(worktree: &Path) -> String {
    let output = run_capture_in(worktree, &["diff", "--name-only", "--diff-filter=U"]);
    let names: Vec<String> = output
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect()
        })
        .unwrap_or_default();
    if names.is_empty() {
        return "conflict: could not enumerate conflicted files".to_string();
    }
    let mut parts = Vec::new();
    for name in names {
        let content = fs::read_to_string(worktree.join(&name)).unwrap_or_default();
        parts.push(conflict_reason(&name, &content));
    }
    format!("conflict (refusing auto-resolution): {}", parts.join("; "))
}

enum GateResult {
    Pass,
    Fail(String),
}

/// The affected crate's FULL gate on the pinned toolchain: `fmt --check`,
/// `build`, `clippy`, `test --no-fail-fast`. Any failing stage fails the
/// gate; the failing stage's output is returned for the HELD line.
fn run_gate(worktree: &Path, packages: &[String]) -> GateResult {
    // The affected crate's FULL gate, in order: fmt --check, build, clippy,
    // test --no-fail-fast. Each stage is its own cargo argv; the pinned
    // toolchain is inherited from the environment (rust-toolchain.toml).
    let mut stages: Vec<Vec<String>> = vec![vec!["fmt".to_string(), "--check".to_string()]];
    let mut build = vec!["build".to_string()];
    build.extend_from_slice(packages);
    stages.push(build);
    let mut clippy = vec!["clippy".to_string(), "--all-targets".to_string()];
    clippy.extend_from_slice(packages);
    stages.push(clippy);
    let mut test = vec!["test".to_string(), "--no-fail-fast".to_string()];
    test.extend_from_slice(packages);
    stages.push(test);

    for stage in &stages {
        let Some(output) = run_cargo(worktree, stage) else {
            return GateResult::Fail(format!("cargo {stage:?}: failed to spawn"));
        };
        if output.status.code() != Some(0) {
            let text = format!(
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return GateResult::Fail(text);
        }
    }
    GateResult::Pass
}

/// The failing-test set from the authoritative `failures:` block (behavior
/// 2), cross-checked against each suite's declared count. A shortfall — the
/// run declares failures the block cannot name — is a loud HELD reason, never
/// a short, silently-tolerated set.
fn failing_tests_note(output: &str) -> String {
    let report = attribute(output);
    if !report.harness_ran {
        return "no test result line; the gate failed before any suite ran".to_string();
    }
    let findings = report.findings();
    let names = report.names();
    let base = format!(
        "gate failed: {} failing test(s) attributed from {} declared",
        names.len(),
        report.declared
    );
    if findings.is_empty() {
        format!("{base}: {}", names.join(", "))
    } else {
        let details = findings.join("; ");
        format!("{base} — {details}")
    }
}

fn open_pr(repo: &str, worktree: &Path, branch: &str, patch: &PatchLocation) -> bool {
    if let Err(error) = run_git_in(worktree, &["add", "-A"]) {
        eprintln!("WARN: git add for #{issue} failed: {error}", issue = patch.issue);
        return false;
    }
    let message = format!("auto-implement: convert issue #{}", patch.issue);
    if run_git_in(worktree, &["commit", "-m", &message]).is_err() {
        // An empty commit (no changes) is not an error to hold on: nothing to
        // open a PR for. Treat it as not-converted.
        return false;
    }
    if let Err(error) = run_git_in(worktree, &["push", "origin", branch]) {
        eprintln!("WARN: push for #{issue} failed: {error}", issue = patch.issue);
        return false;
    }
    let pr_title = format!("auto-implement: issue #{}", patch.issue);
    let opened = Command::new("gh")
        .args([
            "pr",
            "create",
            "--repo",
            repo,
            "--head",
            branch,
            "--title",
            &pr_title,
            "--body",
            &format!(
                "Converted from the agent patch for issue #{}.\n\nSource spec: n/a (patch-to-PR \
                 conversion pass).",
                patch.issue
            ),
            "--label",
            "auto-implement",
        ])
        .status();
    match opened {
        Ok(status) if status.success() => true,
        Ok(status) => {
            eprintln!("WARN: gh pr create for #{issue} exited {status:?}", issue = patch.issue);
            false
        }
        Err(error) => {
            eprintln!("WARN: gh pr create for #{issue} failed: {error}", issue = patch.issue);
            false
        }
    }
}

/// Append a HELD line (a JSON [`HoldRecord`]) to the ledger — the pass's
/// "never discard" step — and return the held result.
fn record_held_and_result(
    plan: &ConvertPlan,
    base_sha: &str,
    patch: &PatchLocation,
    reason: &str,
) -> ApplyResult {
    let held_path = plan
        .opts
        .held_file
        .clone()
        .unwrap_or_else(|| plan.llm_root.join("held.txt"));
    let record = HoldRecord::new(
        patch.issue,
        patch.patch_key.clone(),
        base_sha,
        Vec::new(),
        reason.to_string(),
    );
    if let Some(record) = record {
        if let Some(parent) = held_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(mut line) = serde_json::to_string(&record) {
            line.push('\n');
            use std::io::Write;
            if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(&held_path) {
                let _ = file.write_all(line.as_bytes());
            }
        }
    }
    println!("  HELD  #{issue}: {reason}", issue = patch.issue);
    ApplyResult::Held
}

fn teardown_worktree(worktree: &Path) {
    let _ = run_git(&["worktree", "remove", "--force", worktree.to_str().unwrap_or_default()]);
    let _ = run_git(&["worktree", "prune"]);
    let _ = fs::remove_dir_all(worktree);
}

/// A captured git/cargo run: `Ok(None)` on a spawn error, else the `Output`.
fn run_capture_in(dir: &Path, args: &[&str]) -> Option<Output> {
    Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .ok()
}

fn run_cargo(dir: &Path, stage: &[String]) -> Option<Output> {
    Command::new("cargo")
        .args(stage)
        .current_dir(dir)
        .output()
        .ok()
}

fn run_git(args: &[&str]) -> Result<(), CommandFailure> {
    let output = Command::new("git").args(args).output().map_err(|error| {
        CommandFailure::transient(format!("could not run git {args:?}: {error}"))
    })?;
    if !output.status.success() {
        return Err(CommandFailure::status(
            format!("git {args:?} failed: {}", String::from_utf8_lossy(&output.stderr).trim()),
            output.status.code().unwrap_or(1),
        ));
    }
    Ok(())
}

fn run_git_in(dir: &Path, args: &[&str]) -> Result<(), CommandFailure> {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|error| CommandFailure::transient(format!("could not run git in {dir:?}: {error}")))?;
    if !output.status.success() {
        return Err(CommandFailure::status(
            format!("git {args:?} failed: {}", String::from_utf8_lossy(&output.stderr).trim()),
            output.status.code().unwrap_or(1),
        ));
    }
    Ok(())
}

fn run_git_capture(args: &[&str]) -> Result<String, CommandFailure> {
    let output = Command::new("git").args(args).output().map_err(|error| {
        CommandFailure::transient(format!("could not run git {args:?}: {error}"))
    })?;
    if !output.status.success() {
        return Err(CommandFailure::status(
            format!("git {args:?} failed"),
            output.status.code().unwrap_or(1),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn read_patch_files(path: &Path) -> Vec<String> {
    fs::read_to_string(path).map(|p| patch_files(&p)).unwrap_or_default()
}

/// Infer `OWNER/NAME` from `gh`, best-effort. `None` on any failure — the
/// caller decides whether that is fatal (apply) or a reported gap (plan).
fn infer_repo() -> Option<String> {
    let output = Command::new("gh")
        .args(["repo", "view", "--json", "nameWithOwner", "--jq", ".nameWithOwner"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let repo = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!repo.is_empty()).then_some(repo)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_unfed_line_is_not_the_idle_line() {
        // The incident: a bare pass printed converted=0 held=0 skipped=0,
        // identical to an idle pass. The unfed refusal must differ.
        let unfed = unfed();
        assert_eq!(unfed.exit_code, 2);
        assert!(unfed.message.contains("no issues given"), "{}", unfed.message);
        let idle = PassOutcome::Examined(PassCounters {
            examined: 0,
            converted: 0,
            held: 0,
            skipped: 0,
        })
        .line("convert", "autospec convert", "enumerate $LLM");
        assert_ne!(unfed.message, idle);
    }

    #[test]
    fn issue_names_parse_strictly() {
        assert_eq!(parse_issue_name("issue-42"), Some(42));
        assert_eq!(parse_issue_name("issue-0"), None);
        assert_eq!(parse_issue_name("issue-abc"), None);
        assert_eq!(parse_issue_name("issue-"), None);
        assert_eq!(parse_issue_name("patch-42"), None);
        assert_eq!(parse_issue_name("issue-42x"), None);
    }

    #[test]
    fn the_gate_is_scoped_to_the_affected_crate() {
        assert_eq!(
            gate_packages(&["crates/autospec-core/src/a.rs".to_string()]),
            vec!["-p".to_string(), "autospec-core".to_string()]
        );
        assert_eq!(
            gate_packages(&["crates/autospec-cli/src/a.rs".to_string()]),
            vec!["-p".to_string(), "autospec-cli".to_string()]
        );
        assert_eq!(
            gate_packages(&[
                "crates/autospec-core/src/a.rs".to_string(),
                "crates/autospec-cli/src/b.rs".to_string(),
            ]),
            vec![
                "-p".to_string(),
                "autospec-core".to_string(),
                "-p".to_string(),
                "autospec-cli".to_string(),
            ]
        );
        // A root or out-of-crate file widens the gate to the workspace.
        assert_eq!(gate_packages(&["scripts/x.sh".to_string()]), vec!["--workspace".to_string()]);
        assert_eq!(
            gate_packages(&[
                "crates/autospec-core/src/a.rs".to_string(),
                "README.md".to_string()
            ]),
            vec!["--workspace".to_string()]
        );
    }

    #[test]
    fn patch_files_read_the_b_side_of_the_diff() {
        let patch = "\
diff --git a/crates/autospec-core/src/a.rs b/crates/autospec-core/src/a.rs
index 000..111 100644
--- a/crates/autospec-core/src/a.rs
+++ b/crates/autospec-core/src/a.rs
@@ -1 +1 @@
diff --git a/crates/autospec-cli/src/b.rs b/crates/autospec-cli/src/b.rs
--- /dev/null
+++ b/crates/autospec-cli/src/b.rs
";
        let files = patch_files(patch);
        assert_eq!(
            files,
            vec![
                "crates/autospec-core/src/a.rs",
                "crates/autospec-cli/src/b.rs"
            ]
        );
    }

    #[test]
    fn the_apply_lifecycle_reads_the_git_signature() {
        assert_eq!(
            classify_apply("Applied patch to crates/x.rs", Some(0)),
            ApplyLifecycle::Applied
        );
        assert_eq!(
            classify_apply("error: patch does not apply", Some(1)),
            ApplyLifecycle::Superseded
        );
        assert_eq!(
            classify_apply("CONFLICT (content): Merge conflict in a.rs", Some(1)),
            ApplyLifecycle::Conflict
        );
        // An unrecognized failure fails closed as a conflict (hold), never as
        // applied.
        assert_eq!(classify_apply("mystery error", Some(3)), ApplyLifecycle::Conflict);
    }

    #[test]
    fn the_conflict_reason_refuses_the_unprovable_shapes() {
        // A rules document (AGENTS.md) is unclassifiable: "keep both" would
        // write two contradictory directives, so it is refused.
        let agents = conflict_reason("AGENTS.md", "# Title\n");
        assert!(agents.contains("refused"), "{agents}");
        // A single-value golden is regenerated, not merged.
        let golden = conflict_reason("x.sha256", "abc\n");
        assert!(golden.contains("regenerate"), "{golden}");
        // An append-only changelog is a proven-safe keep-both.
        let changelog = conflict_reason("CHANGELOG.md", "# 1.0\n");
        assert!(changelog.contains("keep-both"), "{changelog}");
    }

    #[test]
    fn a_failing_test_set_is_read_from_the_failures_block() {
        // The run declares 2 failed; the failures: block names 2. The note
        // names them and reports no shortfall.
        let log = "\
running 3 tests
test ok_a ... ok
test bad_b ... FAILED
test bad_c ... FAILED

failures:

---- bad_b stdout ----
---- bad_c stdout ----

failures:
    bad_b
    bad_c

test result: FAILED. 1 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
";
        let note = failing_tests_note(log);
        assert!(note.contains("2 failing test(s)"), "{note}");
        assert!(note.contains("bad_b") && note.contains("bad_c"), "{note}");
        assert!(!note.contains("FAILURE_ATTRIBUTION"), "{note}");
    }

    #[test]
    fn a_shortfall_in_the_failures_block_is_loud() {
        // The run declares 3 failed but the failures: block names only 1:
        // the two unnamed cannot be compared against a baseline, so the note
        // carries the attribution finding instead of a short, silent set.
        let log = "\
running 4 tests
test ok_a ... ok
test bad_b ... FAILED
test bad_c ... FAILED
test bad_d ... FAILED

failures:
    bad_b

test result: FAILED. 1 passed; 3 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
";
        let note = failing_tests_note(log);
        assert!(note.contains("FAILURE_ATTRIBUTION"), "{note}");
    }

    #[test]
    fn the_held_ledger_round_trips_a_record() {
        let dir = std::env::temp_dir().join(format!("convert-held-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let held = dir.join("held.txt");
        let record = HoldRecord::new(
            4015,
            "patch-4015",
            "abc123",
            vec!["a.rs".to_string()],
            "conflict in a.rs",
        )
        .unwrap();
        let mut line = serde_json::to_string(&record).unwrap();
        line.push('\n');
        fs::write(&held, line).unwrap();

        let issues = load_held(&held).unwrap();
        assert!(issues.contains_key(&4015));
        assert_eq!(issues[&4015].reason, "conflict in a.rs");
        let _ = fs::remove_dir_all(&dir);
    }
}
