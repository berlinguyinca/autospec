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
//! 6. records a HELD line — one JSON `HoldRecord` line in the ledger format
//!    below — with the reason for failures. Never prose, never discards.
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
//! ## The HELD ledger format
//!
//! The HELD ledger (`--held-file`, default `<llm-root>/held.txt`) is a
//! machine-readable file this subcommand owns: one JSON line per hold, the
//! serde form of [`autospec_core::hold_memo::HoldRecord`] —
//! `{"issue":N,"patch_key":"...","base_sha":"...","depends_on":[...],
//! "reason":"..."}`. `issue` is the issue number; `patch_key` the patch
//! input key (the patch's mtime in seconds); `base_sha` the trunk tip the
//! hold was derived against; `depends_on` the file paths the hold depends
//! on (empty: the whole base); `reason` what the gate actually reported
//! (`clippy=2`, `test X FAILED`, `conflict in PATH`) — the gate's report, not
//! a sentence describing it, so the re-gate can decide whether the hold
//! still applies. Blank lines and `#` comment lines are skipped. Prose
//! commentary never lives in a parsed position: a converted prose reason
//! keeps its sentence in `reason`, and anything else goes in a sidecar
//! keyed by issue.
//!
//! Step 6 is the loop's instruction to an operator: on a failed conversion,
//! append a HELD line **in this format** — a JSON `HoldRecord` — never
//! prose. A record kept for a future consumer is written in that consumer's
//! format from the first entry, or the cost of the wrong choice is paid
//! retroactively across every entry ever written (#4494).
//!
//! `--convert-ledger PATH` is the one-off that turns a pre-existing prose
//! ledger (lines of `- <issue>  HELD <reason>`, reason may span lines) into
//! this format: one record per issue, the recorded reason preserved, stamped
//! with the current trunk tip and — where the patch is still on disk — its
//! current key, so the first plan after conversion reports `held=N` instead
//! of re-gating every entry at full gate cost.
//!
//! The command is side-effect-free by default: it plans the pass (enumerate +
//! select + report). `--apply` performs the real conversion (branch, gate,
//! PR, HELD).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use autospec_core::conflict_resolution::{classify_file, resolution_for, ResolutionPlan};
use autospec_core::conversion_gate::{test_count_contradiction, tests_added_by_patch};
use autospec_core::conversion_pass::{select_fresh, PassOutcome, PatchCandidate};
use autospec_core::failure_attribution::attribute;
use autospec_core::hold_memo::{re_gate, HoldRecord};
use autospec_core::prefilter_scope::derive_prefilter_scope;
use autospec_core::unfed_pass::PassCounters;
use autospec_core::verification::parse_test_run;
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
                     [--apply] [--json] [--convert-ledger PATH] [ISSUE ...]

PLAN (default): enumerate $LLM/*/out/issue-*/changes.patch, select the patches
not already attempted (a live branch/PR, or a recorded HELD entry whose
re-gate still holds, disqualify), and report the plan. No mutations.

--apply: perform the real conversion of each selected patch — branch off
origin/<base>, full gate (fmt --check, build, clippy, test --no-fail-fast),
open a PR per passing patch, and record a HELD line (a JSON HoldRecord in
the ledger format below, never prose) for failures.

OPTIONS:
    --llm-root DIR        the agent-patch root (default: $LLM)
    --repo OWNER/NAME     the GitHub repo for PR liveness (default: gh)
    --base BRANCH         trunk to branch off origin/<base> (default: main)
    --held-file PATH      the HELD ledger (default: <llm-root>/held.txt)
    --branch-prefix P     conversion branch prefix (default: conv-)
    --apply               perform the conversion, not just the plan
    --json                machine-readable plan
    --convert-ledger PATH one-off: convert a prose HELD ledger (lines of
                          `- <issue>  HELD <reason>`) into the JSON
                          HoldRecord lines the pass reads, one record per
                          issue, preserving the recorded reason; writes to
                          --held-file (default: <llm-root>/held.txt)
    ISSUE ...             restrict the pass to these issue numbers

HELD LEDGER (--held-file): one JSON line per hold — the serde form of
hold_memo::HoldRecord — fields issue, patch_key (patch mtime, seconds),
base_sha (trunk tip the hold was derived against), depends_on (file paths;
empty = the whole base), reason (what the gate reported: clippy=2, test X
FAILED, conflict in PATH — the report, not a sentence). Blank and # comment
lines are skipped. Prose belongs in reason (converted prose keeps its
sentence there) or in a sidecar keyed by issue; a HELD line is always
written in this format, never prose.";

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
    convert_ledger: Option<PathBuf>,
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
        convert_ledger: None,
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
            "--convert-ledger" => {
                opts.convert_ledger = Some(PathBuf::from(value(args, &mut i, &arg)?))
            }
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

/// One entry line of the prose ledger: `- <issue>  HELD <reason start>`
/// (the bullet is optional). Returns the issue number and the start of the
/// reason, or `None` when the line is not an entry. `HELD` must be a whole
/// word — a leading number that does not parse, or an issue of 0, is not an
/// entry either: an unrecognized line is prose, never an entry by guess.
fn prose_entry(line: &str) -> Option<(u64, String)> {
    let mut rest = line.trim_start();
    if let Some(stripped) = rest.strip_prefix(['-', '*']) {
        rest = stripped.trim_start();
    }
    let (number, remainder) = rest.split_once(char::is_whitespace)?;
    let issue: u64 = number.parse().ok()?;
    if issue == 0 {
        return None;
    }
    let after = remainder.trim_start().strip_prefix("HELD")?;
    // "HELD" must be a whole word: if a letter follows immediately, it was
    // not the marker (e.g. "HELDFOO"), so the line is not an entry.
    if after.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    Some((issue, after.trim_start().to_string()))
}

/// A pre-existing prose HELD ledger, parsed into the records it holds: one
/// entry per issue, keyed by issue number. An entry line is
/// `- <issue>  HELD <reason start>`; the lines that follow it, up to the
/// next entry, are its reason's continuation (a reason may span lines, as
/// the fourteen incident batches did). Blank and `#` comment lines are
/// skipped. A later entry for the same issue supersedes the earlier one: the
/// ledger is keyed by issue, last hold wins.
fn parse_prose_ledger(text: &str) -> BTreeMap<u64, String> {
    let mut entries: BTreeMap<u64, String> = BTreeMap::new();
    let mut current: Option<u64> = None;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        match prose_entry(trimmed) {
            Some((issue, start)) => {
                entries.insert(issue, start);
                current = Some(issue);
            }
            None => {
                // A continuation of the previous entry's reason (or a line
                // before any entry): prose, preserved as such, never a
                // parsed position.
                if let Some(issue) = current {
                    if let Some(record) = entries.get_mut(&issue) {
                        record.push('\n');
                        record.push_str(trimmed);
                    }
                }
            }
        }
    }
    entries
}

/// The trunk tip the converted records are stamped against: `origin/<base>`
/// when the remote ref resolves, else the local `HEAD` — resolved in the
/// repository the pass runs in (the cwd), the same way the pass resolves its
/// refs. Outside a git repository both fail — the records need a base sha to
/// become re-gate cache keys at all, so the conversion refuses rather than
/// stamps an empty one.
fn current_base_sha(base: &str) -> Result<String, CommandFailure> {
    if let Ok(sha) = run_git_capture(&["rev-parse", &format!("origin/{base}")]) {
        return Ok(sha);
    }
    run_git_capture(&["rev-parse", "HEAD"]).map_err(|_| {
        CommandFailure::status(
            "convert --convert-ledger: cannot resolve a base sha (not a git repository?) — \
             the converted records need one to become re-gate cache keys",
            2,
        )
    })
}

/// The one-off conversion (#4494): a pre-existing prose HELD ledger becomes
/// the JSON `HoldRecord` lines the pass reads. One record per issue, the
/// recorded reason preserved, stamped with the current trunk tip and — where
/// the patch is still on disk — its current key, so the first plan after
/// conversion reports `held=N` instead of re-gating every entry at full
/// gate cost. Where the patch is not on disk the record carries a sentinel
/// key and re-gates on the first pass — the safe direction (a re-offer,
/// never a stale "still held"). Existing JSON records in the target survive
/// the conversion; the prose entries replace any record for the same issue.
fn convert_ledger(opts: &Options, prose_path: &Path) -> Result<(), CommandFailure> {
    let text = fs::read_to_string(prose_path).map_err(|error| {
        CommandFailure::status(
            format!("cannot read prose ledger {}: {error}", prose_path.display()),
            2,
        )
    })?;
    let entries = parse_prose_ledger(&text);
    if entries.is_empty() {
        return Err(CommandFailure::status(
            format!(
                "no HELD entries recognized in {} — expected lines of the form \
                 `- <issue>  HELD <reason>`",
                prose_path.display()
            ),
            2,
        ));
    }
    let base_sha = current_base_sha(&opts.base)?;

    // The patch keys: the current mtime where the patch is still on disk.
    let mut patch_keys: BTreeMap<u64, String> = BTreeMap::new();
    if let Some(root) = resolve_llm_root(opts.llm_root.as_deref()) {
        if root.is_dir() {
            match enumerate_patches(&root) {
                Ok(patches) => {
                    for patch in patches {
                        patch_keys.insert(patch.issue, patch.patch_key.clone());
                    }
                }
                Err(_) => eprintln!(
                    "WARN: cannot enumerate {root:?} for patch keys; converted records \
                     get sentinel keys and re-gate on the first pass"
                ),
            }
        }
    }

    // The target: --held-file, else the pass's default ledger under the llm
    // root. A conversion without a target would write nothing the consumer
    // can find, which is the incident in reverse.
    let out_path = match &opts.held_file {
        Some(path) => path.clone(),
        None => match resolve_llm_root(opts.llm_root.as_deref()) {
            Some(root) => root.join("held.txt"),
            None => {
                return Err(CommandFailure::diagnostic(
                    "convert --convert-ledger: no --held-file and no llm root to default \
                     the target to — pass --held-file PATH",
                ))
            }
        },
    };

    let mut held = load_held(&out_path)?;
    for (issue, reason) in &entries {
        let patch_key = patch_keys
            .get(issue)
            .cloned()
            .unwrap_or_else(|| format!("unrecorded-{issue}"));
        let record = HoldRecord::new(*issue, patch_key, &base_sha, Vec::new(), reason)
            .expect("non-empty patch key and base sha by construction");
        held.insert(*issue, record);
    }
    if let Some(parent) = out_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let mut out = String::new();
    for record in held.values() {
        out.push_str(&serde_json::to_string(record).expect("HoldRecord serializes"));
        out.push('\n');
    }
    fs::write(&out_path, out).map_err(|error| {
        CommandFailure::status(
            format!("cannot write held ledger {}: {error}", out_path.display()),
            2,
        )
    })?;
    println!(
        "held ledger: converted {} prose entr{} -> {} (base_sha={base_sha})",
        entries.len(),
        if entries.len() == 1 { "y" } else { "ies" },
        out_path.display()
    );
    Ok(())
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

/// The gate scope tokens for the files a patch touches, derived from the
/// patch's touched crates by the shared definition
/// ([`autospec_core::prefilter_scope::derive_prefilter_scope`]) — never
/// hard-coded here (issue #4532): the crates the patch touches are gated
/// (`-p <crate>` per crate), and a patch that touches no resolvable crate
/// gates `--workspace`. Fail-closed: being slow is recoverable; being
/// narrow is not. The pre-filter and the gate use the same derivation, so
/// the two scopes cannot drift.
fn gate_packages(files: &[String]) -> Vec<String> {
    derive_prefilter_scope(files).tokens()
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
    if opts.apply && opts.convert_ledger.is_some() {
        return Err(CommandFailure::diagnostic(
            "--convert-ledger is the one-off ledger conversion; it does not combine with --apply",
        ));
    }
    if let Some(prose) = &opts.convert_ledger {
        return convert_ledger(&opts, prose);
    }
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

    // The gate's scope, derived from the patch's touched crates (the shared
    // definition — issue #4532): known before the patch is applied, because
    // the patch's paths are.
    let patch_text = fs::read_to_string(&patch.path).unwrap_or_default();
    let packages = gate_packages(&patch_files(&patch_text));

    // The unchanged-count contradiction (issue #4532) is only possible when
    // the patch adds test functions. For those, the baseline test count is
    // measured at the base — at the same scope the gate will use — before
    // the patch is applied. Being slow is recoverable; being narrow is not.
    let tests_added = tests_added_by_patch(&patch_text);
    let baseline_tests = if tests_added > 0 {
        match run_test_count(&worktree, &packages) {
            Some(count) => Some(count),
            None => {
                teardown_worktree(&worktree);
                return record_held_and_result(
                    plan,
                    base_sha,
                    patch,
                    "baseline test count undeterminable at the base (the test stage \
                     produced no test result line) — the unchanged-count \
                     contradiction cannot be checked",
                );
            }
        }
    } else {
        None
    };

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
            // Step 4: the full gate on the pinned toolchain, at the derived scope.
            let gate = run_gate(&worktree, &packages, tests_added, baseline_tests);
            match gate {
                GateResult::Pass => {
                    // Step 5: open a PR per passing patch.
                    let opened = open_pr(repo, &worktree, &branch, patch, &packages);
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
                GateResult::Contradiction(reason) => {
                    // The stages were green but the evidence contradicts the
                    // patch: hold it, the contradiction named (issue #4532).
                    teardown_worktree(&worktree);
                    record_held_and_result(plan, base_sha, patch, &reason)
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
    /// A stage failed; the stage's output.
    Fail(String),
    /// Every stage was green but the evidence contradicts the patch: the
    /// patch adds test functions and the test count is unchanged (issue
    /// #4532). The reason names the contradiction; it is the HELD reason.
    Contradiction(String),
}

/// The gate's test-stage argv at the given scope: `test --no-fail-fast`
/// plus the scope tokens. The baseline count and the gate's test stage run
/// this same argv, so they measure the same set of tests.
fn test_stage(packages: &[String]) -> Vec<String> {
    let mut stage = vec!["test".to_string(), "--no-fail-fast".to_string()];
    stage.extend_from_slice(packages);
    stage
}

/// The baseline test count at the given scope: the test stage run at the
/// base, before the patch is applied, summed across every `test result:`
/// line (passed + failed — every test that ran). `None` when the stage
/// produced no test result line (the base did not build, or no test ran) —
/// a count that cannot be measured is not a zero.
fn run_test_count(worktree: &Path, packages: &[String]) -> Option<u64> {
    let stage = test_stage(packages);
    let output = run_cargo(worktree, &stage)?;
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let aggregate = parse_test_run(&text);
    (aggregate.targets > 0).then(|| aggregate.passed.saturating_add(aggregate.failed))
}

/// The FULL gate at the given scope on the pinned toolchain: `fmt --check`,
/// `build`, `clippy`, `test --no-fail-fast`. Any failing stage fails the
/// gate; the failing stage's output is returned for the HELD line. A green
/// run is not yet a pass: when the patch adds test functions, the test
/// count must have moved from the baseline — identical figures mean the
/// gate's scope did not cover the change (issue #4532), and the gate fails
/// with the contradiction named.
fn run_gate(
    worktree: &Path,
    packages: &[String],
    tests_added: usize,
    baseline_tests: Option<u64>,
) -> GateResult {
    // The gate, in order: fmt --check, build, clippy, then the test stage
    // (handled separately — its output is the evidence for the
    // unchanged-count contradiction). Each stage is its own cargo argv; the
    // pinned toolchain is inherited from the environment (rust-toolchain.toml).
    let mut stages: Vec<Vec<String>> = vec![vec!["fmt".to_string(), "--check".to_string()]];
    let mut build = vec!["build".to_string()];
    build.extend_from_slice(packages);
    stages.push(build);
    let mut clippy = vec!["clippy".to_string(), "--all-targets".to_string()];
    clippy.extend_from_slice(packages);
    stages.push(clippy);

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

    let stage = test_stage(packages);
    let Some(output) = run_cargo(worktree, &stage) else {
        return GateResult::Fail(format!("cargo {stage:?}: failed to spawn"));
    };
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if output.status.code() != Some(0) {
        return GateResult::Fail(text);
    }

    // Green stages are not yet a pass: the unchanged-count contradiction
    // (issue #4532). `baseline_tests` is `Some` exactly when the patch adds
    // test functions and the baseline was measured.
    if let Some(baseline) = baseline_tests {
        let aggregate = parse_test_run(&text);
        let post = aggregate.passed.saturating_add(aggregate.failed);
        if let Some(contradiction) = test_count_contradiction(tests_added, baseline, post) {
            return GateResult::Contradiction(format!(
                "gate failed: {contradiction} (gate scope: {})",
                packages.join(" ")
            ));
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

fn open_pr(repo: &str, worktree: &Path, branch: &str, patch: &PatchLocation, scope: &[String]) -> bool {
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
                "Converted from the agent patch for issue #{}.\n\nGate scope: {} \
                 (derived from the patch's touched crates).\n\nSource spec: n/a \
                 (patch-to-PR conversion pass).",
                patch.issue,
                scope.join(" ")
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
    fn the_gate_scope_is_derived_from_the_touched_crate() {
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
            // The crate set is sorted (the shared `CheckScope` contract).
            vec![
                "-p".to_string(),
                "autospec-cli".to_string(),
                "-p".to_string(),
                "autospec-core".to_string(),
            ]
        );
        // No resolvable crate: the workspace (fail-closed default).
        assert_eq!(gate_packages(&["scripts/x.sh".to_string()]), vec!["--workspace".to_string()]);
        // The scope is the patch's touched crates, not the crates plus
        // "every other file": a crate patch with a docs-only file beside it
        // gates that crate (the shared definition, issue #4532).
        assert_eq!(
            gate_packages(&[
                "crates/autospec-core/src/a.rs".to_string(),
                "README.md".to_string()
            ]),
            vec!["-p".to_string(), "autospec-core".to_string()]
        );
        // No crate is hard-coded: a patch touching a crate this function
        // has never named gates that crate. The pre-#4532 code could only
        // produce autospec-core / autospec-cli / --workspace.
        assert_eq!(
            gate_packages(&["crates/autospec-foo/src/a.rs".to_string()]),
            vec!["-p".to_string(), "autospec-foo".to_string()]
        );
    }

    #[test]
    fn the_test_stage_carries_the_derived_scope() {
        // The incident's shape: a cli patch gates the test stage at
        // `-p autospec-cli`, never at a fixed crate (issue #4532).
        let scope = gate_packages(&["crates/autospec-cli/tests/a.rs".to_string()]);
        assert_eq!(
            test_stage(&scope),
            vec![
                "test".to_string(),
                "--no-fail-fast".to_string(),
                "-p".to_string(),
                "autospec-cli".to_string(),
            ]
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

    #[test]
    fn prose_entry_requires_the_marker_word() {
        assert_eq!(
            prose_entry("- 2755  HELD conflict at x"),
            Some((2755, "conflict at x".to_string()))
        );
        // The bullet is optional; a bare issue number still parses.
        assert_eq!(
            prose_entry("2755 HELD no bullet"),
            Some((2755, "no bullet".to_string()))
        );
        // "HELD" must be a whole word.
        assert_eq!(prose_entry("- 2755 HELDFOO bar"), None);
        // A lowercase marker is not the marker.
        assert_eq!(prose_entry("- 2755 held something"), None);
        assert_eq!(prose_entry("no number here HELD"), None);
        assert_eq!(prose_entry("- 0 HELD zero issue"), None);
    }

    #[test]
    fn the_prose_ledger_parses_keyed_by_issue_with_the_reason_preserved() {
        let prose = "\
# held ledger
- 3195  HELD regression. Applies clean, fmt/clippy clean, but adds a NEW test
  that main already ships under a different name.
- 2755  HELD conflict at `scripts/autospec-explore.sh:226`.
- 2755  HELD conflict at `scripts/autospec-explore.sh:231` (re-dispatch).
";
        let entries = parse_prose_ledger(prose);
        assert_eq!(entries.len(), 2);
        // A multi-line reason is preserved, line for line.
        assert_eq!(
            entries[&3195],
            "regression. Applies clean, fmt/clippy clean, but adds a NEW test\n\
             that main already ships under a different name."
        );
        // A later hold for the same issue supersedes the earlier one.
        assert_eq!(
            entries[&2755],
            "conflict at `scripts/autospec-explore.sh:231` (re-dispatch)."
        );
    }

    #[test]
    fn a_converted_record_round_trips_and_still_holds() {
        let entries = parse_prose_ledger("- 2755  HELD conflict at `x.sh:226`.\n");
        let (issue, reason) = entries.iter().next().unwrap();
        let record = HoldRecord::new(*issue, "1750000000", "abc1234", Vec::new(), reason)
            .unwrap();
        let mut line = serde_json::to_string(&record).unwrap();
        line.push('\n');
        let dir = std::env::temp_dir().join(format!("convert-converted-{issue}-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let held = dir.join("held.txt");
        fs::write(&held, line).unwrap();
        let loaded = load_held(&held).unwrap();
        assert_eq!(loaded[&2755].reason, "conflict at `x.sh:226`.");
        // Stamped with the current patch key and base, the re-gate still
        // holds: the first plan after conversion reports held, not fresh —
        // the AC that this issue is actually fixed.
        let decision = re_gate(&loaded[&2755], "1750000000", &[]);
        assert!(decision.is_still_held());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_one_off_converter_writes_a_ledger_the_pass_reads() {
        let dir = std::env::temp_dir().join(format!("convert-prose-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        // The base sha is resolved from the repository the pass runs in
        // (the test's cwd), the same way the pass resolves it.
        let expected_base = run_git_capture(&["rev-parse", "origin/main"])
            .or_else(|_| run_git_capture(&["rev-parse", "HEAD"]))
            .unwrap();

        // One agent patch on disk for a held issue: its mtime becomes the
        // record's key, so the pass's re-gate still holds for it.
        let patch_dir = dir.join("llm").join("node-a").join("out").join("issue-2755");
        fs::create_dir_all(&patch_dir).unwrap();
        fs::write(patch_dir.join("changes.patch"), "diff --git a/x b/x\n").unwrap();

        let prose = dir.join("held.prose");
        fs::write(&prose, "- 2755  HELD conflict at `x.sh:226`.\n- 9999  HELD clippy=2\n").unwrap();
        let out = dir.join("llm").join("held.txt");

        let opts = Options {
            llm_root: Some(dir.join("llm")),
            repo: None,
            base: "main".to_string(),
            held_file: Some(out.clone()),
            branch_prefix: DEFAULT_BRANCH_PREFIX.to_string(),
            apply: false,
            as_json: false,
            convert_ledger: Some(prose.clone()),
            issues: Vec::new(),
        };
        assert!(convert_ledger(&opts, &prose).is_ok());

        let held = load_held(&out).unwrap();
        assert_eq!(held.len(), 2);
        assert_eq!(held[&2755].reason, "conflict at `x.sh:226`.");
        assert_eq!(
            held[&2755].patch_key,
            patch_key(&patch_dir.join("changes.patch")).unwrap()
        );
        assert_eq!(held[&2755].base_sha, expected_base);
        // No patch on disk for 9999: sentinel key, reason still preserved.
        assert_eq!(held[&9999].patch_key, "unrecorded-9999");
        assert_eq!(held[&9999].reason, "clippy=2");
        let _ = fs::remove_dir_all(&dir);
    }
}
