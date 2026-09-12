//! `autospec dispatch` — liveness between filing an issue and dispatching an
//! agent (#3800).
//!
//! Subcommands:
//! - `check` — the consumer's gate. Reads the queue artifact and refuses to
//!   call a stale or unstamped queue "no work". Exit 0 proceed/idle, 1 hold.
//!   With `--admitted-file <PATH>`, an idle queue over admitted work is a fault,
//!   not silence (#3927).
//! - `reconcile` — the admission reconciliation: report the count of admitted
//!   but unschedulable issues; zero is the expected answer, nonzero a failure.
//!   Exit 0 clean, 1 defect (#3927).
//! - `stamp` — the producer's call, just before it renames the artifact into
//!   place: writes `refreshed-at`/`refreshed-by` atomically and beats for the
//!   producing hop.
//! - `beat` — record liveness for any other hop (file, top-up, dispatch).
//! - `status` — topology, credential-hosted steps, per-hop liveness, verdict.
//!   Exit 0 healthy, 1 any hop failed.
//! - `guard` — the pre-dispatch gate against unconverted output (#3764):
//!   the dispatch path destroys the issue's output directory, so before it
//!   does, the guard verifies the directory holds no unconverted patch. A
//!   check that cannot answer is unsafe, never clear. `--dry-run` reports
//!   the decision without touching the directory. Exit 0 authorized, 1 hold.
//! - `runs` — classify a dispatch batch of agent runs (#3918): each run
//!   becomes an `OK` / `NO-OUTPUT` / `INFRA-FAIL` / `LAUNCH-FAIL` status
//!   record (the `agent-status.tsv` row), transcripts under the size
//!   threshold are quoted verbatim, the batch is summarized, and repeated
//!   identical failures raise a fleet-level fault. Exit 0 no fault, 1 fault.
//! - `tick` — one dispatch tick over the queue (#3911, #4451): once the
//!   liveness gate authorizes the queue, reports per entry which entries are
//!   actionable (a fresh dispatch, or a conversion of a patch that is
//!   already produced) and which are skipped, with a reason for each skip.
//!   The tick enforces the dispatch bound: an entry dispatched repeatedly
//!   without a patch is held — with the count and the reason — not
//!   redispatched, and the run reports how many entries it skipped over the
//!   bound. The tick records its decisions in the lifecycle ledger (fresh
//!   dispatches flag the entry in flight; bound-exceeding entries are held),
//!   so the bound survives the dispatcher process. Exit 0 when anything is
//!   dispatched; 1 on a liveness hold or a stall (non-empty queue, nothing
//!   dispatched — every skip is named).
//! - `mark` — move one queue entry through its lifecycle (#3911, #4451):
//!   `produced` (the agent produced a patch), `converted` (the patch became
//!   a PR or commit — the only terminal state), `hold` (record why the
//!   entry is blocked), `release` (clear the hold and reset the dispatch
//!   attempt count; a released produced entry re-enters the next tick as a
//!   conversion, not a fresh dispatch), or `failed` (a run ended without a
//!   patch: advance the attempt count and clear the in-flight flag).
//!   Exit 0 accepted, 1 refused, 2 usage error.
//! - `schedule` — audit the refresh-queue schedule as a first-class
//!   component (#4320): the critical-path manifest covers every step it
//!   declares (or explicitly monitors as manual), the queue artifact is
//!   fresh against the limit its producer's mode implies, and every
//!   credential-bearing step is admitted to run unattended — a refusal names
//!   the step and where the credential belongs. Exit 0 healthy, 1 any
//!   finding.
//!
//! A cron line for the refresh step is the intended deployment:
//!
//! ```text
//! */10 * * * * refresh-queue.sh && autospec dispatch stamp || \
//!   echo "refresh-queue failed" >> ~/.autospec/logs/refresh-queue.log
//! ```

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use autospec_core::dispatch_guard::{self, CheckId, CheckReport};
use autospec_core::dispatch_pipeline::{
    DispatchPipeline, DispatchTick, EntryState, FreshnessPolicy, LifecycleLedger, LivenessLedger,
    PipelineReport, PipelineTopology, QueueFile, SchedulingReconciliation,
    DEFAULT_INTERVAL_SECS, DEFAULT_MAX_DISPATCH_ATTEMPTS, DEFAULT_MAX_STALE_INTERVALS,
    QUEUE_ARTIFACT,
};
use autospec_core::refresh_queue_contract::{
    admit_schedule, assess_artifact, ArtifactStaleness, ScheduleManifest, SchedulingVerdict,
    WalkSummary,
};
use autospec_core::fleet_dispatch::{
    classify_run, idle_subfleet_lines, summarize_batch, BatchSummary, FleetDispatchPolicy,
    RunRecord, RunStatusRecord, SubfleetState, TSV_HEADER,
};
use serde::{Deserialize, Serialize};

use super::CommandFailure;

/// The artifact is authoritative and every hop is live.
const OK_EXIT: i32 = 0;
/// A liveness failure: the artifact is not authoritative or a hop is silent.
const HOLD_EXIT: i32 = 1;

const SUBCOMMANDS: &[(&str, &str)] = &[
    (
        "check",
        "Gate on the queue artifact (exit 0 proceed/idle / 1 hold)",
    ),
    (
        "reconcile",
        "Reconciliation: admitted-but-unschedulable issues by count (exit 1 nonzero)",
    ),
    (
        "guard",
        "Gate on unconverted output before dispatch (#3764)",
    ),
    ("stamp", "Stamp the queue artifact after repopulating it"),
    ("beat", "Record a liveness beat for one hop"),
    (
        "status",
        "Print topology, credential hosts, hop liveness, verdict",
    ),
    (
        "stage",
        "Stage an issue and its comments as the dispatch spec (#3864)",
    ),
    (
        "freshness",
        "Gate on the staged spec matching the live issue revision (#3864)",
    ),
    (
        "preflight",
        "The single pre-dispatch gate: NO-SPEC (missing/empty spec), prompt assertion, freshness, spec receipt in status.txt (#3620)",
    ),
    ("runs", "Classify a dispatch batch and summarize it (#3918)"),
    (
        "tick",
        "One dispatch tick over the queue: per-entry dispatch / convert / skip, every skip with a reason, dispatch bound enforced (#3911, #4451)",
    ),
    (
        "mark",
        "Move one queue entry through its lifecycle: produced / converted / hold / release / failed (#3911, #4451)",
    ),
    (
        "schedule",
        "Audit the refresh-queue schedule: manifest coverage, artifact staleness, credential admission (#4320)",
    ),
];

pub fn run(args: &[String]) -> Result<(), CommandFailure> {
    let (subcommand, rest) = args
        .split_first()
        .ok_or_else(|| CommandFailure::diagnostic("usage: autospec dispatch <subcommand> ..."))?;
    match subcommand.as_str() {
        "-h" | "--help" => {
            print_help();
            Ok(())
        }
        "check" => check(rest),
        "reconcile" => reconcile(rest),
        "guard" => guard(rest),
        "stage" => super::dispatch_spec::stage(rest),
        "freshness" => super::dispatch_spec::freshness(rest),
        "preflight" => super::dispatch_spec::preflight(rest),
        "stamp" => stamp(rest),
        "beat" => beat(rest),
        "status" => status(rest),
        "runs" => runs(rest),
        "tick" => tick(rest),
        "mark" => mark(rest),
        "schedule" => schedule(rest),
        other => Err(CommandFailure::diagnostic(format!(
            "unknown dispatch subcommand: {other} (expected one of: {})",
            SUBCOMMANDS
                .iter()
                .map(|(name, _)| *name)
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

fn print_help() {
    println!("USAGE: autospec dispatch <subcommand> [options]");
    println!();
    println!("SUBCOMMANDS:");
    for (name, description) in SUBCOMMANDS {
        println!("    {name:<7} {description}");
    }
    println!("OPTIONS:");
    println!("    --queue <PATH>        Queue artifact (default $HOME/.autospec/{QUEUE_ARTIFACT})");
    println!("    --admitted-file <PATH>  check/reconcile: the tracker's admitted set, one issue number per line");
    println!("    --state-file <PATH>   Liveness ledger (default $HOME/.autospec/dispatch-liveness.json)");
    println!(
        "    --topology <PATH>     Topology JSON (default: built-in filing-to-dispatch chain)"
    );
    println!("    --step <NAME>         Hop the beat is for (required for beat)");
    println!(
        "    --issue <N>           Issue the guard, stage, freshness, or mark command acts on (required for mark)"
    );
    println!("    --issue-json <PATH>   stage: `gh api` issue payload (body, updatedAt, comments)");
    println!("    --comments-json <PATH> stage: `gh api .../comments` payload merged into the discussion");
    println!("    --body-file <PATH>    stage: verbatim body when no --issue-json is given");
    println!("    --source-updated-at <T> stage: live issue updatedAt (epoch or RFC 3339)");
    println!("    --out <PATH>          stage: staged spec to write (default $HOME/.autospec/dispatch/specs/<N>.md)");
    println!("    --staged <PATH>       freshness/preflight: staged spec to check (same default)");
    println!("    --prompt-file <PATH>  preflight: the assembled prompt; the dispatch is refused if it carries no issue text");
    println!("    --status-file <PATH>  freshness/preflight: append the spec receipt (byte count + sha256) to the run's status.txt");
    println!(
        "    --live-updated-at <T> freshness: the live issue updatedAt, when the caller read it"
    );
    println!("    --live-json <PATH>    freshness: issue payload to read updatedAt from");
    println!("    --repo <OWNER/NAME>   stage: read issue + comments live via `gh api` (else $AUTOSPEC_REPO)");
    println!("                          freshness: repository to query with `gh api`");
    println!("    --container-runtime <P> stage: runtime path, or 'absent' / 'not probed'");
    println!("    --database <VALUE>    stage: database availability, or 'absent' / 'not probed'");
    println!("    --registry <VALUE>    stage: registry reachability (never probed unless given)");
    println!("    --no-probe            stage: declare every environment fact as not probed");
    println!("    --out-dir <PATH>      Runner output root (default $HOME/.autospec/dispatch/out)");
    println!("    --patch-name <NAME>   Patch file under issue-<N> (default changes.patch)");
    println!("    --dry-run             guard: report the decision without touching the directory");
    println!(
        "    --by <NAME>           Producer named in the stamp (default: declared queue producer)"
    );
    println!(
        "    --runs <PATH>         runs: JSON of the batch (array of runs, or {{runs, subfleets}})"
    );
    println!(
        "    --duration-floor <S>  runs: seconds below which a run is LAUNCH-FAIL (default 30)"
    );
    println!("    --quote-bytes <N>     runs: transcripts of at most N bytes are quoted verbatim (default 4096)");
    println!("    --fault-threshold <N> runs: identical failures at/above N in one batch raise a fleet fault (default 3)");
    println!("    --out <PATH>          runs: where to write the agent-status.tsv record (default stdout)");
    println!("    --lifecycle <PATH>    Lifecycle ledger (default $HOME/.autospec/dispatch-lifecycle.json)");
    println!("    --action <A>          mark: produced / converted / hold / release / failed (required)");
    println!(
        "    --max-attempts <N>    tick: fresh dispatches without a patch tolerated before an entry is held, not redispatched (default {DEFAULT_MAX_DISPATCH_ATTEMPTS}, #4451)"
    );
    println!("    --reason <TEXT>       mark hold: why the entry is blocked (required for hold)");
    println!("    --at <EPOCH>          Beat timestamp in epoch seconds (default: current time)");
    println!("    --now <EPOCH>         Evaluate against this instant instead of the clock");
    println!("    --interval <SECONDS>  Interval for hops that declare none (default {DEFAULT_INTERVAL_SECS})");
    println!("    --max-intervals <N>   Missed intervals tolerated before a hold (default {DEFAULT_MAX_STALE_INTERVALS})");
    println!("    --json                Emit JSON");
    println!("    -h, --help            Print help");
    println!();
    println!("EXIT CODES:");
    println!("    {OK_EXIT}  ok        queue fresh (proceed or genuinely idle) and every hop live");
    println!("    {HOLD_EXIT}  hold      queue missing/unstamped/stale, clock rewind, silent hop, or topology defect");
    println!("    {HOLD_EXIT}  stall     tick: queue non-empty but nothing dispatched (every skip named); mark: stamp refused");
    println!("    2  diagnostic usage error, unreadable artifact, or unparseable ledger");
}

/// `check` — refuse to read a stale queue as "no work".
fn check(args: &[String]) -> Result<(), CommandFailure> {
    let pipeline = build_pipeline(args)?;
    let queue = read_queue(&queue_path(args)?)?;
    let now = eval_now(args)?;
    let outcome = match opt_string(args, "--admitted-file")? {
        Some(path) => pipeline.authorize_queue_with_admission(
            queue.as_ref(),
            read_admitted(&PathBuf::from(path))?,
            now,
        ),
        None => pipeline.authorize_queue(queue.as_ref(), now),
    };

    if super::is_json(args) {
        println!(
            "{}",
            serde_json::to_string_pretty(&outcome)
                .map_err(|error| CommandFailure::diagnostic(error.to_string()))?
        );
    } else {
        println!("{}", outcome.line());
    }

    verdict_exit(outcome.held())
}

/// `reconcile` — the periodic admission reconciliation (#3927): report the
/// admitted-but-unschedulable issues by count. Zero is the expected answer; any
/// other number is a failure. The label set is authoritative and the queue a
/// derived copy, so a queue entry no longer admitted is reported as stale, never
/// as the defect.
fn reconcile(args: &[String]) -> Result<(), CommandFailure> {
    let admitted_path = opt_string(args, "--admitted-file")?
        .map(PathBuf::from)
        .ok_or_else(|| CommandFailure::diagnostic("reconcile needs --admitted-file <PATH>"))?;
    let admitted = read_admitted(&admitted_path)?;
    let schedulable = match read_queue(&queue_path(args)?)? {
        Some(queue) => queue.entries,
        None => Vec::new(),
    };

    let recon = SchedulingReconciliation::new(admitted, schedulable);
    if super::is_json(args) {
        println!(
            "{}",
            serde_json::to_string_pretty(&recon)
                .map_err(|error| CommandFailure::diagnostic(error.to_string()))?
        );
    } else {
        println!("{}", recon.line());
    }

    verdict_exit(recon.is_defect())
}

/// Read the admitted set: the issue numbers the tracker labels as dispatchable,
/// one per line (comments and blanks ignored). The file is required — a missing
/// admitted set is a diagnostic, not an empty one, because the label is the
/// authoritative gate and an absent label set is not proof there is no work.
fn read_admitted(path: &Path) -> Result<Vec<u64>, CommandFailure> {
    let text = fs::read_to_string(path).map_err(|error| {
        CommandFailure::diagnostic(format!(
            "cannot read admitted file {path:?}: {error} (the label set is required)"
        ))
    })?;
    Ok(QueueFile::parse(&text).entries)
}

/// `guard` — the pre-dispatch gate against unconverted output (#3764).
///
/// The dispatch path destroys the issue's output directory (`issue-<N>`
/// under `--out-dir`) before re-dispatching, and this guard is what verifies
/// the directory holds no unconverted patch. The check fails closed: a `stat`
/// error is an unsafe answer, never a clear one. Without `--dry-run`, an
/// authorized dispatch removes the (verified patch-free) directory; a held
/// dispatch touches nothing.
fn guard(args: &[String]) -> Result<(), CommandFailure> {
    let issue = opt_string(args, "--issue")?
        .ok_or_else(|| CommandFailure::diagnostic("guard needs --issue <N>"))?;
    validate_issue_number(&issue)?;
    let out_dir = match opt_string(args, "--out-dir")? {
        Some(dir) => PathBuf::from(dir),
        None => autospec_home()?.join("dispatch").join("out"),
    };
    let patch_name =
        opt_string(args, "--patch-name")?.unwrap_or_else(|| "changes.patch".to_string());
    validate_patch_name(&patch_name)?;
    let dry_run = args.iter().any(|arg| arg == "--dry-run");

    let issue_dir = out_dir.join(format!("issue-{issue}"));
    let patch_path = issue_dir.join(&patch_name);
    let check = check_unconverted_patch(&patch_path);

    // Classify the artifact outcome when the check saw a dangerous state
    // (the patch exists). A patch whose run failed irrecoverably is archived
    // to free the dispatch slot (#3784); a patch that is convertible or
    // unrecorded holds with a named reason.
    let epoch = now_epoch()?;
    let archive_performed: Option<PathBuf> = if check.outcome.dangerous() && !dry_run {
        let outcome = classify_run_outcome(&issue_dir);
        match outcome {
            dispatch_guard::ArtifactOutcome::FailedRun { status } => {
                println!("DISPATCH issue {issue} archiving failed run (status: {status})");
                match archive_failed_run(&out_dir, &issue_dir, &issue, epoch) {
                    Ok(archive_path) => {
                        println!("  archived to {}", archive_path.display());
                        Some(archive_path)
                    }
                    Err(error) => {
                        return Err(CommandFailure::diagnostic(format!(
                            "guard classified run as failed (status: {status}) but \
                             archiving the artifact failed: {error}"
                        )));
                    }
                }
            }
            other => {
                // Convertible or Unrecorded: hold with a classified reason.
                let report = dispatch_guard::classified_hold(&issue, check, other);
                if super::is_json(args) {
                    println!("{}", report.to_json());
                } else {
                    println!("{}", report.line());
                }
                return Err(CommandFailure::status(String::new(), HOLD_EXIT));
            }
        }
    } else if check.outcome.dangerous() && dry_run {
        let outcome = classify_run_outcome(&issue_dir);
        match outcome {
            dispatch_guard::ArtifactOutcome::FailedRun { status } => {
                let candidate = candidate_archive_path(&out_dir, &issue, epoch);
                println!(
                    "DISPATCH issue {issue} would archive failed run (status: {status}) to {}",
                    candidate.display()
                );
                // Dry-run + FailedRun: authorized, would archive.
                None
            }
            other => {
                let report = dispatch_guard::classified_hold(&issue, check, other);
                for line in report.lines() {
                    println!("{line}");
                }
                return Err(CommandFailure::status(String::new(), HOLD_EXIT));
            }
        }
    } else {
        // Check is Clear or Failed (not Dangerous): normal decide path.
        let report = dispatch_guard::decide(&issue, std::slice::from_ref(&check));
        if super::is_json(args) {
            println!("{}", report.to_json());
        } else if dry_run {
            for line in report.lines() {
                println!("{line}");
            }
        } else {
            println!("{}", report.line());
            if !report.held() {
                // The guard verified the directory holds no unconverted
                // patch; whatever remains is stale debris, and removing it
                // is what `rm -rf issue-<N>` was always for.
                match fs::remove_dir_all(&issue_dir) {
                    Ok(()) => println!("removed stale output {}", issue_dir.display()),
                    Err(error) if error.kind() == ErrorKind::NotFound => {
                        println!("nothing to remove: {} is absent", issue_dir.display())
                    }
                    Err(error) => {
                        return Err(CommandFailure::diagnostic(format!(
                            "guard authorized dispatch but stale output could not be removed: {error}"
                        )));
                    }
                }
            }
        }
        if report.held() {
            return Err(CommandFailure::status(String::new(), HOLD_EXIT));
        }
        return Ok(());
    };

    // We reached here with a patch that was classified as FailedRun and
    // archived (or would-be in dry-run). The dispatch slot is now free.
    // If we actually archived, the issue_dir is gone (it was moved);
    // remove_dir_all will hit NotFound which is fine.
    if archive_performed.is_some() {
        match fs::remove_dir_all(&issue_dir) {
            Ok(()) => println!("removed stale output {}", issue_dir.display()),
            Err(error) if error.kind() == ErrorKind::NotFound => {
                // Expected: the archive moved the directory.
            }
            Err(error) => {
                return Err(CommandFailure::diagnostic(format!(
                    "guard archived failed run but could not clean up {}: {error}",
                    issue_dir.display()
                )));
            }
        }
    }
    Ok(())
}

/// Read the `status.txt` next to the patch and classify the artifact
/// outcome. Returns [`dispatch_guard::ArtifactOutcome`].
fn classify_run_outcome(issue_dir: &Path) -> dispatch_guard::ArtifactOutcome {
    let status_path = issue_dir.join("status.txt");
    let content = match fs::read_to_string(&status_path) {
        Ok(c) => c,
        Err(_) => return dispatch_guard::classify_artifact_outcome(None),
    };
    let report = match autospec_core::execution::status_triage::parse_agent_report(&content) {
        Ok(r) => r,
        Err(error) => {
            return dispatch_guard::ArtifactOutcome::Unrecorded {
                detail: format!("status.txt is unparseable: {error}"),
            }
        }
    };
    dispatch_guard::classify_artifact_outcome(report.status.as_deref())
}

/// Move the entire `issue_dir` to an archive location under `out_dir` so the
/// dispatch slot is freed. The archive path is
/// `out_dir/archive/issue-<N>-<epoch>/` with a counter suffix for
/// same-second collisions.
fn archive_failed_run(
    out_dir: &Path,
    issue_dir: &Path,
    issue: &str,
    epoch: u64,
) -> Result<PathBuf, String> {
    let archive_root = out_dir.join("archive");
    fs::create_dir_all(&archive_root)
        .map_err(|e| format!("create archive dir {}: {e}", archive_root.display()))?;
    let base = archive_root.join(format!("issue-{issue}-{epoch}"));
    // If the candidate already exists (same-second collision), append a
    // counter.
    let final_path = if base.exists() {
        let mut counter = 1u32;
        loop {
            let candidate = archive_root.join(format!("issue-{issue}-{epoch}-{counter}"));
            if !candidate.exists() {
                break candidate;
            }
            counter += 1;
        }
    } else {
        base
    };
    fs::rename(issue_dir, &final_path).map_err(|e| {
        format!(
            "rename {} -> {}: {e}",
            issue_dir.display(),
            final_path.display()
        )
    })?;
    Ok(final_path)
}

/// The candidate archive path for a failed-run artifact: `out_dir/archive/
/// issue-<N>-<epoch>/`.
fn candidate_archive_path(out_dir: &Path, issue: &str, epoch: u64) -> PathBuf {
    out_dir
        .join("archive")
        .join(format!("issue-{issue}-{epoch}"))
}

/// The unconverted-patch check, run against the local filesystem.
///
/// `symlink_metadata` on purpose: the question is whether the *artifact*
/// exists at its path, not what it points at — a symlink is unconverted
/// output too. Anything that is not a clean "not found" (permission errors,
/// the path component not being a directory, I/O errors) is a failed check:
/// a check that cannot answer is unsafe, never clear.
fn check_unconverted_patch(patch_path: &Path) -> CheckReport {
    match fs::symlink_metadata(patch_path) {
        Ok(_) => CheckReport::dangerous(
            CheckId::UnconvertedPatch,
            Some(patch_path.display().to_string()),
        ),
        Err(error) if error.kind() == ErrorKind::NotFound => CheckReport::clear(
            CheckId::UnconvertedPatch,
            Some(patch_path.display().to_string()),
        ),
        Err(error) => CheckReport::failed(
            CheckId::UnconvertedPatch,
            format!("stat {}: {error}", patch_path.display()),
        ),
    }
}

/// Issue numbers name a path component (`issue-<N>`); keep them numeric so
/// they cannot traverse.
pub(crate) fn validate_issue_number(issue: &str) -> Result<u64, CommandFailure> {
    match issue.parse::<u64>() {
        Ok(n) if n > 0 => Ok(n),
        _ => Err(CommandFailure::diagnostic(format!(
            "dispatch --issue must be a positive integer, got '{issue}'"
        ))),
    }
}

/// The patch name sits under a directory the guard may remove, so it must be
/// a plain file name.
fn validate_patch_name(name: &str) -> Result<(), CommandFailure> {
    let safe = !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\0')
        && name
            .bytes()
            .all(|b| matches!(b, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'.' | b'_' | b'-'));
    if !safe {
        return Err(CommandFailure::diagnostic(format!(
            "guard --patch-name must be a plain file name, got '{name}'"
        )));
    }
    Ok(())
}

/// `stamp` — write the freshness headers into the artifact and beat for the hop.
fn stamp(args: &[String]) -> Result<(), CommandFailure> {
    let pipeline = build_pipeline(args)?;
    let path = queue_path(args)?;
    let mut queue = read_queue(&path)?.unwrap_or_default();
    let writer = match opt_string(args, "--by")? {
        Some(name) => name,
        None => pipeline
            .queue_producer()
            .map(|step| step.name.clone())
            .ok_or_else(|| {
                CommandFailure::diagnostic(format!(
                    "no step in the topology produces {QUEUE_ARTIFACT}; pass --by <NAME>"
                ))
            })?,
    };
    validate_step_name(&writer)?;
    let at = opt_u64(args, "--at")?.unwrap_or(now_epoch()?);

    queue.stamp(at, &writer);
    write_atomic(&path, &queue.render())?;

    let ledger_path = state_file(args)?;
    let mut ledger = load_ledger(&ledger_path)?;
    ledger.record(&writer, at);
    save_ledger(&ledger_path, &ledger)?;

    println!(
        "stamped {QUEUE_ARTIFACT}: refreshed-by {writer} at {at} ({} entries)",
        queue.entry_count()
    );
    Ok(())
}

/// `beat` — record that one hop ran, without touching the artifact.
fn beat(args: &[String]) -> Result<(), CommandFailure> {
    let step = opt_string(args, "--step")?.ok_or_else(|| {
        CommandFailure::diagnostic("beat needs --step <NAME> (the hop that is alive)")
    })?;
    validate_step_name(&step)?;
    let at = opt_u64(args, "--at")?.unwrap_or(now_epoch()?);

    let path = state_file(args)?;
    let mut ledger = load_ledger(&path)?;
    ledger.record(&step, at);
    save_ledger(&path, &ledger)?;

    println!("beat recorded: hop {step} at {at}");
    Ok(())
}

/// `status` — the whole filing-to-dispatch chain in one report.
fn status(args: &[String]) -> Result<(), CommandFailure> {
    let pipeline = build_pipeline(args)?;
    let queue = read_queue(&queue_path(args)?)?;
    let report = pipeline.report(queue.as_ref(), eval_now(args)?);

    if super::is_json(args) {
        println!("{}", report.to_json());
    } else {
        print_human(&report, &pipeline);
    }

    verdict_exit(!report.healthy())
}

/// Exit 0 when the answer may be acted on, 1 when a liveness failure names the
/// hop that stopped moving.
pub(crate) fn verdict_exit(failed: bool) -> Result<(), CommandFailure> {
    if failed {
        return Err(CommandFailure::status(String::new(), HOLD_EXIT));
    }
    Ok(())
}

fn print_human(report: &PipelineReport, pipeline: &DispatchPipeline) {
    let credential_hosts: Vec<String> = pipeline
        .topology
        .credential_steps()
        .into_iter()
        .map(|step| format!("{}@{}", step.name, step.host.as_str()))
        .collect();
    println!(
        "credential-holding steps: {}",
        if credential_hosts.is_empty() {
            "none".to_string()
        } else {
            credential_hosts.join(", ")
        }
    );
    for line in report.lines() {
        println!("{line}");
    }
}

/// Assemble topology + ledger + policy from flags and on-disk state.
/// `tick` — one dispatch tick over the queue (#3911, #4451).
///
/// Once the liveness gate authorizes the queue, report per entry which
/// entries are actionable — a fresh dispatch, or a conversion of a patch
/// that is already produced — and which are skipped, with a reason for each
/// skip. A tick that dispatches nothing over a non-empty queue is a stall,
/// exit 1: a skip without a reason is the silent version of the bug this
/// reports.
///
/// The tick enforces the dispatch bound (#4451): an entry dispatched
/// `--max-attempts` times without a patch is held — with the count and the
/// reason — not redispatched, and the summary reports how many entries were
/// skipped over the bound. The tick records its decisions in the lifecycle
/// ledger — fresh dispatches flag the entry in flight, bound-exceeding
/// entries are held — so the bound is durable across dispatcher processes,
/// not a guard bolted onto the shell. A tick with nothing to record leaves
/// the ledger untouched.
fn tick(args: &[String]) -> Result<(), CommandFailure> {
    let pipeline = build_pipeline(args)?;
    let queue = read_queue(&queue_path(args)?)?;
    let now = eval_now(args)?;

    if let Some(failure) = pipeline.authorize_queue(queue.as_ref(), now).failure() {
        println!("{}", failure.line());
        return verdict_exit(true);
    }
    let Some(queue) = queue else {
        return Err(CommandFailure::diagnostic(
            "the queue disappeared between the liveness check and the tick".to_string(),
        ));
    };

    let max_attempts =
        opt_u64(args, "--max-attempts")?.unwrap_or(DEFAULT_MAX_DISPATCH_ATTEMPTS);
    if max_attempts == 0 {
        return Err(CommandFailure::diagnostic(
            "--max-attempts must be greater than zero".to_string(),
        ));
    }

    let ledger_path = lifecycle_file(args)?;
    let mut ledger = load_lifecycle(&ledger_path)?;
    let report = DispatchTick::run_bounded(&queue, &ledger, max_attempts);

    // Record the tick's decisions before reporting them: if the ledger
    // cannot be written, the tick fails and the dispatcher dispatches
    // nothing, rather than reporting a dispatch that was never recorded
    // (and would be repeated on the next run).
    let written = report.apply_to_ledger(&mut ledger, now);
    if written > 0 {
        save_lifecycle(&ledger_path, &ledger)?;
    }

    if super::is_json(args) {
        println!("{}", report.to_json());
    } else {
        // The walk summary explains the walk's outcome — required exactly
        // when a non-empty queue yields nothing eligible (#4320): a zero
        // without the partition is silence, and silence is the defect.
        let walk = WalkSummary::from_tick(&queue, &report);
        println!("{}", walk.line());
        for line in report.lines() {
            println!("{line}");
        }
        if written > 0 {
            println!("lifecycle ledger updated: {written} record(s) written");
        }
    }

    verdict_exit(!report.dispatched_anything() && !report.skipped().is_empty())
}

/// `schedule` — audit the refresh-queue schedule as a first-class component
/// (#4320).
///
/// Three checks, each with a named finding: the critical-path manifest
/// covers every step it declares (`ScheduleManifest::coverage_audit`), the
/// queue artifact is fresh against the limit its producer's mode implies
/// (`assess_artifact` on the filesystem's `mtime`), and every
/// credential-bearing step is admitted to run unattended (`admit_schedule`) —
/// a refusal names the step and the explicit answer about where the
/// credential lives. A non-empty queue whose artifact is stale and whose
/// walk yields nothing eligible is the incident this exists to name. Exit 0
/// healthy, 1 any finding.
fn schedule(args: &[String]) -> Result<(), CommandFailure> {
    let pipeline = build_pipeline(args)?;
    let now = eval_now(args)?;
    let max_intervals =
        opt_u64(args, "--max-intervals")?.unwrap_or(DEFAULT_MAX_STALE_INTERVALS);

    let manifest: ScheduleManifest = match opt_string(args, "--manifest")? {
        Some(path) => {
            let text = fs::read_to_string(&path).map_err(|err| {
                CommandFailure::diagnostic(format!("cannot read --manifest {path}: {err}"))
            })?;
            serde_json::from_str(&text).map_err(|err| {
                CommandFailure::diagnostic(format!("cannot parse --manifest {path}: {err}"))
            })?
        }
        None => ScheduleManifest::from_topology(&pipeline.topology),
    };

    let mut findings: Vec<String> = manifest.lines();

    // Staleness: the queue artifact's mtime against the limit its producer's
    // mode implies — scheduled: interval x max_intervals; manual: the
    // declared limit.
    let queue_path = queue_path(args)?;
    let mtime = fs::metadata(&queue_path)
        .ok()
        .and_then(|meta| meta.modified().ok())
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs());
    let producer = pipeline.queue_producer();
    let limit = manifest
        .max_stale_secs_for(QUEUE_ARTIFACT, max_intervals)
        .unwrap_or_else(|| pipeline.policy.threshold_for(producer));
    let staleness = assess_artifact(mtime, now, limit);
    if staleness.is_stale() {
        findings.push(staleness.line());
    }

    // Admission: every credential-bearing step must be admitted to run
    // unattended; a refusal carries the gap and the location answer.
    let present = credential_present();
    let verdicts: Vec<SchedulingVerdict> = pipeline
        .topology
        .steps()
        .iter()
        .map(|step| admit_schedule(step, present))
        .collect();
    for verdict in &verdicts {
        if let SchedulingVerdict::Rejected { gap } = verdict {
            findings.push(gap.line());
        }
    }

    if super::is_json(args) {
        #[derive(Serialize)]
        struct ScheduleJson<'a> {
            healthy: bool,
            findings: Vec<String>,
            manifest: &'a ScheduleManifest,
            queue_artifact: ArtifactStaleness,
            admission: Vec<SchedulingVerdict>,
        }
        println!(
            "{}",
            serde_json::to_string_pretty(&ScheduleJson {
                healthy: findings.is_empty(),
                findings: findings.clone(),
                manifest: &manifest,
                queue_artifact: staleness,
                admission: verdicts.clone(),
            })
            .unwrap_or_else(|err| panic!("cannot serialize schedule report: {err}"))
        );
    } else if findings.is_empty() {
        println!("SCHEDULE AUDIT: healthy — {} steps declared, queue fresh, credentials admitted", manifest.steps.len());
    } else {
        println!("SCHEDULE AUDIT: {} findings", findings.len());
        for line in &findings {
            println!("{line}");
        }
    }

    verdict_exit(!findings.is_empty())
}

/// Probe for the GitHub credential the refresh step needs: a non-empty
/// `GH_TOKEN` environment variable, or the `gh` CLI's hosts file in the
/// operator's private home. Neither is a cluster-shared location: that is
/// the point — the probe answers "is the credential where it belongs",
/// which is what [`admit_schedule`] needs.
fn credential_present() -> bool {
    if std::env::var("GH_TOKEN")
        .map(|token| !token.trim().is_empty())
        .unwrap_or(false)
    {
        return true;
    }
    std::env::var("HOME")
        .is_ok_and(|home| Path::new(&home).join(".config/gh/hosts.yml").is_file())
}

/// `mark` — move one queue entry through its lifecycle (#3911, #4451).
///
/// `produced` (the agent produced a patch; resets the dispatch attempt
/// count), `converted` (the patch became a PR or commit — the only terminal
/// state), `hold` (record why the entry is blocked; the state is preserved),
/// `release` (clear the hold and reset the attempt count — the explicit
/// re-arm after triaging a dispatch-bound entry; a released produced entry
/// re-enters the next tick as a conversion, not a fresh dispatch), or
/// `failed` (a run ended without a patch: advance the attempt count and
/// clear the in-flight flag). A refused stamp — terminal entry, backwards
/// timestamp — exits 1 and names why; a usage error exits 2.
fn mark(args: &[String]) -> Result<(), CommandFailure> {
    const USAGE: &str = "usage: autospec dispatch mark --action <produced|converted|hold|release|failed> --issue <N> [--reason <TEXT>] [--at <EPOCH>] [--lifecycle <PATH>]";
    let action = opt_string(args, "--action")?
        .ok_or_else(|| CommandFailure::diagnostic(USAGE.to_string()))?;
    let issue_text = opt_string(args, "--issue")?
        .ok_or_else(|| CommandFailure::diagnostic(USAGE.to_string()))?;
    let issue = validate_issue_number(&issue_text)?;
    let reason = opt_string(args, "--reason")?;
    let at = match opt_u64(args, "--at")? {
        Some(at) => at,
        None => now_epoch()?,
    };
    let path = lifecycle_file(args)?;
    let mut ledger = load_lifecycle(&path)?;

    let message = match action.as_str() {
        "produced" => mark_state(&mut ledger, issue, EntryState::Produced, at)?,
        "converted" => mark_state(&mut ledger, issue, EntryState::Converted, at)?,
        "hold" => mark_hold(&mut ledger, issue, &hold_reason(reason)?, at)?,
        "release" => mark_release(&mut ledger, issue, at)?,
        "failed" => mark_failed(&mut ledger, issue, at)?,
        other => {
            return Err(CommandFailure::diagnostic(format!(
                "unknown mark action {other:?} (expected produced, converted, hold, release, or failed)"
            )))
        }
    };

    save_lifecycle(&path, &ledger)?;
    println!("{message}");
    Ok(())
}

/// A hold without a reason is a usage error (exit 2): a hold that cannot say
/// why is the silence this feature exists to remove.
fn hold_reason(reason: Option<String>) -> Result<String, CommandFailure> {
    reason
        .filter(|reason| !reason.trim().is_empty())
        .ok_or_else(|| CommandFailure::diagnostic("mark hold needs a non-blank --reason <text>"))
}

/// A refused stamp is a verdict (exit 1), not a usage error: the ledger
/// already knows the entry's true state, and the refusal names it.
fn mark_state(
    ledger: &mut LifecycleLedger,
    issue: u64,
    state: EntryState,
    at: u64,
) -> Result<String, CommandFailure> {
    if !ledger.record(issue, state, at) {
        let recorded = ledger
            .record_of(issue)
            .map_or(at, |record| record.recorded_at);
        return Err(CommandFailure::status(
            format!(
                "mark: #{issue} refused — already stamped at {recorded} ({}); stamps are monotonic",
                ledger.state_of(issue).as_str()
            ),
            HOLD_EXIT,
        ));
    }
    Ok(format!("marked #{issue} as {} at {at}", state.as_str()))
}

fn mark_hold(
    ledger: &mut LifecycleLedger,
    issue: u64,
    reason: &str,
    at: u64,
) -> Result<String, CommandFailure> {
    if !ledger.hold(issue, reason, at) {
        let message = if ledger.state_of(issue).is_terminal() {
            format!("mark: #{issue} is already converted; a terminal entry cannot be held")
        } else {
            format!(
                "mark: #{issue} hold refused — a later stamp already exists; stamps are monotonic"
            )
        };
        return Err(CommandFailure::status(message, HOLD_EXIT));
    }
    Ok(format!(
        "held #{issue} [{}]: {reason}",
        ledger.state_of(issue).as_str()
    ))
}

fn mark_release(
    ledger: &mut LifecycleLedger,
    issue: u64,
    at: u64,
) -> Result<String, CommandFailure> {
    if !ledger.release(issue, at) {
        return Err(CommandFailure::status(
            format!("mark: #{issue} release refused — no non-terminal lifecycle record to release"),
            HOLD_EXIT,
        ));
    }
    Ok(format!(
        "released #{issue} [{}] (dispatch attempt count reset)",
        ledger.state_of(issue).as_str()
    ))
}

/// `failed` — a run ended without a patch (#4451). The attempt count
/// advances — this is the counter the dispatch bound is enforced on — and
/// the in-flight flag clears, so the next tick either dispatches the entry
/// again (within the bound) or holds it over the bound with the count.
fn mark_failed(
    ledger: &mut LifecycleLedger,
    issue: u64,
    at: u64,
) -> Result<String, CommandFailure> {
    match ledger.record_failed(issue, at) {
        Some(count) => Ok(format!(
            "recorded failed run for #{issue}: attempt {count} (no patch; in-flight flag cleared)"
        )),
        None => {
            let message = if ledger.state_of(issue).is_terminal() {
                format!("mark: #{issue} is already converted; a terminal entry cannot record a failed run")
            } else {
                let recorded = ledger
                    .record_of(issue)
                    .map_or(at, |record| record.recorded_at);
                format!(
                    "mark: #{issue} failed refused — a later stamp already exists (recorded {recorded}); stamps are monotonic"
                )
            };
            Err(CommandFailure::status(message, HOLD_EXIT))
        }
    }
}

fn build_pipeline(args: &[String]) -> Result<DispatchPipeline, CommandFailure> {
    let topology = load_topology(&topology_path(args)?)?;
    let liveness = load_ledger(&state_file(args)?)?;
    let policy = FreshnessPolicy::new(
        opt_u64(args, "--interval")?.unwrap_or(DEFAULT_INTERVAL_SECS),
        opt_u64(args, "--max-intervals")?.unwrap_or(DEFAULT_MAX_STALE_INTERVALS),
    )
    .ok_or_else(|| {
        CommandFailure::diagnostic("--interval and --max-intervals must both be greater than zero")
    })?;
    Ok(DispatchPipeline::new(topology, liveness, policy))
}

/// Read the artifact; `None` means it is not on disk, which is a verdict, not
/// an I/O error.
fn read_queue(path: &Path) -> Result<Option<QueueFile>, CommandFailure> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(QueueFile::parse(&text))),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(CommandFailure::diagnostic(format!(
            "cannot read queue {path:?}: {error}"
        ))),
    }
}

/// Write via a sibling temp file and rename, so a consumer never observes a
/// half-written artifact.
pub(crate) fn write_atomic(path: &Path, text: &str) -> Result<(), CommandFailure> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            CommandFailure::diagnostic(format!("cannot create {}: {error}", parent.display()))
        })?;
    }
    let temp = path.with_extension(format!("tmp-{}", std::process::id()));
    fs::write(&temp, text).map_err(|error| {
        CommandFailure::diagnostic(format!("cannot write {}: {error}", temp.display()))
    })?;
    fs::rename(&temp, path).map_err(|error| {
        let _ = fs::remove_file(&temp);
        CommandFailure::diagnostic(format!(
            "cannot move {} onto {path:?}: {error}",
            temp.display()
        ))
    })
}

/// A missing ledger is not a failure: it means no hop has beaten yet.
fn load_ledger(path: &Path) -> Result<LivenessLedger, CommandFailure> {
    match fs::read_to_string(path) {
        Ok(text) => LivenessLedger::from_json(&text).map_err(|error| {
            CommandFailure::diagnostic(format!(
                "liveness ledger {path:?} does not parse ({error}); refusing to start a fresh one over it"
            ))
        }),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(LivenessLedger::new()),
        Err(error) => Err(CommandFailure::diagnostic(format!(
            "cannot read liveness ledger {path:?}: {error}"
        ))),
    }
}

fn save_ledger(path: &Path, ledger: &LivenessLedger) -> Result<(), CommandFailure> {
    write_atomic(path, &ledger.to_json())
}

/// The lifecycle ledger is consumer-owned state, parallel to the liveness
/// ledger: `--lifecycle <PATH>` or `$HOME/.autospec/dispatch-lifecycle.json`.
fn lifecycle_file(args: &[String]) -> Result<PathBuf, CommandFailure> {
    match opt_string(args, "--lifecycle")? {
        Some(path) => Ok(PathBuf::from(path)),
        None => Ok(autospec_home()?.join("dispatch-lifecycle.json")),
    }
}

/// A missing lifecycle ledger is not a fault: every entry is simply `queued`.
fn load_lifecycle(path: &Path) -> Result<LifecycleLedger, CommandFailure> {
    match fs::read_to_string(path) {
        Ok(text) => LifecycleLedger::from_json(&text).map_err(|error| {
            CommandFailure::diagnostic(format!(
                "lifecycle ledger {path:?} does not parse ({error}); refusing to start a fresh one over it"
            ))
        }),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(LifecycleLedger::new()),
        Err(error) => Err(CommandFailure::diagnostic(format!(
            "cannot read lifecycle ledger {path:?}: {error}"
        ))),
    }
}

fn save_lifecycle(path: &Path, ledger: &LifecycleLedger) -> Result<(), CommandFailure> {
    write_atomic(path, &ledger.to_json())
}

/// No topology file means the built-in filing-to-dispatch chain.
fn load_topology(path: &Option<PathBuf>) -> Result<PipelineTopology, CommandFailure> {
    let Some(path) = path else {
        return Ok(PipelineTopology::reference());
    };
    let text = fs::read_to_string(path).map_err(|error| {
        CommandFailure::diagnostic(format!("cannot read topology {path:?}: {error}"))
    })?;
    serde_json::from_str(&text).map_err(|error| {
        CommandFailure::diagnostic(format!("topology {path:?} does not parse: {error}"))
    })
}

fn queue_path(args: &[String]) -> Result<PathBuf, CommandFailure> {
    match opt_string(args, "--queue")? {
        Some(path) => Ok(PathBuf::from(path)),
        None => Ok(autospec_home()?.join(QUEUE_ARTIFACT)),
    }
}

fn state_file(args: &[String]) -> Result<PathBuf, CommandFailure> {
    match opt_string(args, "--state-file")? {
        Some(path) => Ok(PathBuf::from(path)),
        None => Ok(autospec_home()?.join("dispatch-liveness.json")),
    }
}

fn topology_path(args: &[String]) -> Result<Option<PathBuf>, CommandFailure> {
    Ok(opt_string(args, "--topology")?.map(PathBuf::from))
}

pub(crate) fn autospec_home() -> Result<PathBuf, CommandFailure> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map_err(|_| {
            CommandFailure::diagnostic("no HOME set for the dispatch state path".to_string())
        })?;
    Ok(PathBuf::from(home).join(".autospec"))
}

pub(crate) fn now_epoch() -> Result<u64, CommandFailure> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .map_err(|error| {
            CommandFailure::diagnostic(format!("system clock is before the unix epoch: {error}"))
        })
}

/// `--now` lets a wrapper (or a test) evaluate staleness against a fixed instant.
fn eval_now(args: &[String]) -> Result<u64, CommandFailure> {
    match opt_u64(args, "--now")? {
        Some(now) => Ok(now),
        None => now_epoch(),
    }
}

/// Hop names land in a JSON ledger key and in every report line, so they are
/// restricted to a safe charset.
fn validate_step_name(name: &str) -> Result<(), CommandFailure> {
    let valid = !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'));
    if valid {
        Ok(())
    } else {
        Err(CommandFailure::diagnostic(format!(
            "invalid hop name {name:?}: use [a-z0-9._-] only"
        )))
    }
}

pub(crate) fn opt_string(args: &[String], flag: &str) -> Result<Option<String>, CommandFailure> {
    opt_raw(args, flag).map(|value| value.map(str::to_string))
}

fn opt_u64(args: &[String], flag: &str) -> Result<Option<u64>, CommandFailure> {
    opt_raw(args, flag).and_then(|value| match value {
        Some(text) => text.parse::<u64>().map(Some).map_err(|error| {
            CommandFailure::diagnostic(format!(
                "flag {flag} needs an integer, got {text:?} ({error})"
            ))
        }),
        None => Ok(None),
    })
}

fn opt_raw<'a>(args: &'a [String], flag: &str) -> Result<Option<&'a str>, CommandFailure> {
    let Some(index) = args.iter().position(|arg| arg == flag) else {
        return Ok(None);
    };
    let value = args
        .get(index + 1)
        .ok_or_else(|| CommandFailure::diagnostic(format!("flag needs a value: {flag}")))?;
    Ok(Some(value.as_str()))
}

/// Input shape for `runs`: either a bare array of runs (the common case,
/// where the caller has no sub-fleet view) or an object that also carries
/// the sub-fleet states for the idle-sub-fleet report.
#[derive(Deserialize)]
struct RunsInput {
    runs: Vec<RunRecord>,
    #[serde(default)]
    subfleets: Vec<SubfleetState>,
}

/// The `--json` report for `runs`: the classified records, the batch
/// summary, and the idle-sub-fleet report lines.
#[derive(Serialize)]
pub(crate) struct RunsReport {
    pub records: Vec<RunStatusRecord>,
    pub summary: BatchSummary,
    pub idle_subfleets: Vec<String>,
}

/// Parse the batch JSON: a bare array of runs, or an object with `runs`
/// and optional `subfleets`.
fn parse_runs_input(
    value: serde_json::Value,
    source: &str,
) -> Result<(Vec<RunRecord>, Vec<SubfleetState>), CommandFailure> {
    match value {
        serde_json::Value::Array(_) => {
            let run_records: Vec<RunRecord> = serde_json::from_value(value).map_err(|error| {
                CommandFailure::diagnostic(format!("runs file {source}: {error}"))
            })?;
            Ok((run_records, Vec::new()))
        }
        other => {
            let input: RunsInput = serde_json::from_value(other).map_err(|error| {
                CommandFailure::diagnostic(format!("runs file {source}: {error}"))
            })?;
            Ok((input.runs, input.subfleets))
        }
    }
}

/// Classify the batch and build the report (pure: no file or stdout access).
fn build_runs_report(
    run_records: Vec<RunRecord>,
    subfleets: Vec<SubfleetState>,
    policy: &FleetDispatchPolicy,
) -> RunsReport {
    let records: Vec<RunStatusRecord> = run_records
        .iter()
        .map(|record| classify_run(record, policy))
        .collect();
    let summary = summarize_batch(&records, policy);
    let idle_subfleets = idle_subfleet_lines(&subfleets);
    RunsReport {
        records,
        summary,
        idle_subfleets,
    }
}

/// The `runs` policy: each knob defaults independently, but whatever ends up
/// in the policy must be positive — `FleetDispatchPolicy::new` is the single
/// check for that.
fn runs_policy(args: &[String]) -> Result<FleetDispatchPolicy, CommandFailure> {
    let default = FleetDispatchPolicy::default();
    let duration_floor_secs =
        opt_u64(args, "--duration-floor")?.unwrap_or(default.duration_floor_secs);
    let quote_bytes: u64 =
        opt_u64(args, "--quote-bytes")?.unwrap_or(default.transcript_quote_bytes as u64);
    let fault_threshold: u64 =
        opt_u64(args, "--fault-threshold")?.unwrap_or(default.repeat_fault_threshold as u64);
    FleetDispatchPolicy::new(
        duration_floor_secs,
        quote_bytes.try_into().map_err(|_| {
            CommandFailure::diagnostic(format!(
                "--quote-bytes {quote_bytes} does not fit in a byte count"
            ))
        })?,
        fault_threshold.try_into().map_err(|_| {
            CommandFailure::diagnostic(format!(
                "--fault-threshold {fault_threshold} does not fit in a count"
            ))
        })?,
    )
    .ok_or_else(|| {
        CommandFailure::diagnostic(
            "runs: --duration-floor, --quote-bytes and --fault-threshold must all be positive",
        )
    })
}

/// `autospec dispatch runs` — classify a dispatch batch (#3918).
///
/// Reads a JSON file describing the batch (a run is
/// `{"issue", "duration_secs", "transcript"}`), classifies each run into
/// `OK` / `NO-OUTPUT` / `INFRA-FAIL` / `LAUNCH-FAIL`, writes the
/// `agent-status.tsv` record (to `--out`, or stdout), and prints the batch
/// summary plus any fleet-level faults and idle-sub-fleet report lines.
/// `--json` prints the full report instead of the TSV.
///
/// Exit 0 when no fleet-level fault was raised; exit 1 when one was. The
/// idle-sub-fleet report lines are informational and do not affect the exit
/// code.
fn runs(args: &[String]) -> Result<(), CommandFailure> {
    const USAGE: &str = "usage: autospec dispatch runs --runs <PATH> \
[--out <PATH>] [--duration-floor <SECS>] [--quote-bytes <N>] [--fault-threshold <N>] [--json] [--transcript-dir <PATH>]";
    let runs_path =
        opt_string(args, "--runs")?.ok_or_else(|| CommandFailure::diagnostic(USAGE.to_string()))?;
    let out_path = opt_string(args, "--out")?;
    let transcript_dir = opt_string(args, "--transcript-dir")?;
    let as_json = args.iter().any(|arg| arg == "--json");
    let policy = runs_policy(args)?;

    let raw = fs::read_to_string(&runs_path).map_err(|error| {
        CommandFailure::transient(format!("cannot read runs file {runs_path}: {error}"))
    })?;
    let value: serde_json::Value = serde_json::from_str(&raw).map_err(|error| {
        CommandFailure::diagnostic(format!("runs file {runs_path} is not valid JSON: {error}"))
    })?;
    let (run_records, subfleets) = parse_runs_input(value, &runs_path)?;

    // AC #4: even empty runs leave a transcript on disk for post-mortem.
    if let Some(dir) = &transcript_dir {
        fs::create_dir_all(dir).map_err(|error| {
            CommandFailure::transient(format!("cannot create transcript dir {dir}: {error}"))
        })?;
        for record in &run_records {
            let path = std::path::Path::new(dir).join(format!("{}.transcript", record.issue));
            fs::write(&path, record.transcript.as_bytes()).map_err(|error| {
                CommandFailure::transient(format!(
                    "cannot write transcript {}: {error}",
                    path.display()
                ))
            })?;
        }
    }

    let report = build_runs_report(run_records, subfleets, &policy);

    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).expect("report serializes")
        );
    } else {
        let tsv = std::iter::once(TSV_HEADER.to_string())
            .chain(report.records.iter().map(RunStatusRecord::tsv_line))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        match &out_path {
            Some(path) => fs::write(path, &tsv).map_err(|error| {
                CommandFailure::transient(format!("cannot write status record {path}: {error}"))
            })?,
            None => print!("{tsv}"),
        }
        for line in report.summary.lines() {
            println!("{line}");
        }
        for line in &report.idle_subfleets {
            println!("{line}");
        }
    }

    if report.summary.faults.is_empty() {
        Ok(())
    } else {
        Err(CommandFailure::status(
            format!(
                "fleet-level fault raised in the dispatch batch: {}",
                report
                    .summary
                    .faults
                    .iter()
                    .map(|fault| fault.signature.clone())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            1,
        ))
    }
}

#[cfg(test)]
mod runs_tests {
    use super::*;
    use autospec_core::fleet_dispatch::{
        DEFAULT_DURATION_FLOOR_SECS, DEFAULT_REPEAT_FAULT_THRESHOLD, DEFAULT_TRANSCRIPT_QUOTE_BYTES,
    };
    use std::sync::atomic::{AtomicU64, Ordering};

    static FIXTURE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn fixture_dir(label: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "autospec-dispatch-runs-{label}-{}-{}",
            std::process::id(),
            FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&directory).expect("fixture directory");
        directory
    }

    fn write(path: &Path, content: &str) {
        fs::write(path, content).expect("fixture write");
    }

    /// The acceptance test from #3918 at the CLI level: a bad-credential
    /// dispatch is `INFRA-FAIL`, never `NO-OUTPUT`, and does not consume an
    /// attempt.
    #[test]
    fn bad_credential_batch_reports_infra_fail_and_raises_fault() {
        let dir = fixture_dir("badcred");
        let runs_path = dir.join("runs.json");
        let out_path = dir.join("agent-status.tsv");
        write(
            &runs_path,
            r#"[
  {"issue": "51", "duration_secs": 40, "transcript": "401 Unauthorized: invalid api key"},
  {"issue": "52", "duration_secs": 38, "transcript": "401 Unauthorized: invalid api key"},
  {"issue": "53", "duration_secs": 41, "transcript": "401 Unauthorized: invalid api key"}
]"#,
        );

        let failure = runs(&[
            "--runs".to_string(),
            runs_path.to_string_lossy().into_owned(),
            "--out".to_string(),
            out_path.to_string_lossy().into_owned(),
        ])
        .expect_err("three identical infra failures must raise a fleet fault");
        assert_eq!(failure.exit_code, 1);

        let tsv = fs::read_to_string(&out_path).expect("status record written");
        let lines: Vec<&str> = tsv.lines().collect();
        assert_eq!(lines.len(), 4, "header + three runs: {tsv:?}");
        assert!(lines[0].starts_with("issue\tstatus\tduration_secs"));
        for line in &lines[1..] {
            let columns: Vec<&str> = line.split('\t').collect();
            assert_eq!(columns.len(), 7, "one row per run: {line:?}");
            assert_eq!(
                columns[1], "INFRA-FAIL",
                "a bad credential must never be NO-OUTPUT: {line:?}"
            );
            assert_eq!(
                columns[5], "false",
                "INFRA-FAIL must not consume an attempt: {line:?}"
            );
        }
        assert!(
            tsv.contains("INFRA-FAIL/auth/401 unauthorized"),
            "the signature must name the category and the matched pattern: {tsv:?}"
        );
        // Short transcripts ride along verbatim, escaped.
        assert!(
            lines[1..]
                .iter()
                .all(|line| line.contains("401 Unauthorized: invalid api key")),
            "transcripts under the quote threshold are verbatim: {tsv:?}"
        );
    }

    #[test]
    fn no_fault_batch_exits_cleanly_and_reports_idle_subfleets() {
        let dir = fixture_dir("clean");
        let runs_path = dir.join("runs.json");
        write(
            &runs_path,
            r#"{
  "runs": [
    {"issue": "60", "duration_secs": 120, "transcript": ""},
    {"issue": "61", "duration_secs": 300, "transcript": "patch generated"}
  ],
  "subfleets": [
    {"name": "gw-issue-51-53", "running_agents": 0, "open_eligible": 7}
  ]
}"#,
        );

        let result = runs(&[
            "--runs".to_string(),
            runs_path.to_string_lossy().into_owned(),
        ]);
        assert!(
            result.is_ok(),
            "no repeated failures means no fault: {result:?}"
        );
    }

    #[test]
    fn report_json_round_trips_the_classified_batch() {
        let (run_records, subfleets) = parse_runs_input(
            serde_json::json!([{
                "issue": "51",
                "duration_secs": 2,
                "transcript": "connection refused"
            }]),
            "test",
        )
        .expect("bare array parses");
        assert!(subfleets.is_empty());

        let report = build_runs_report(run_records, subfleets, &FleetDispatchPolicy::default());
        assert_eq!(
            report.records[0].status,
            autospec_core::fleet_dispatch::RunStatus::LaunchFail
        );
        assert_eq!(report.summary.launch_fail, 1);
        assert!(report.summary.faults.is_empty());

        let json = serde_json::to_string(&report).expect("report serializes");
        let value: serde_json::Value = serde_json::from_str(&json).expect("report deserializes");
        assert_eq!(value["records"][0]["status"], "LAUNCH-FAIL");
        assert_eq!(value["records"][0]["consumes_attempt"], false);
        assert_eq!(value["summary"]["launch_fail"], 1);
    }

    #[test]
    fn runs_policy_defaults_and_rejects_zero() {
        let default = runs_policy(&[]).expect("flags are all optional");
        assert_eq!(
            default,
            FleetDispatchPolicy {
                duration_floor_secs: DEFAULT_DURATION_FLOOR_SECS,
                transcript_quote_bytes: DEFAULT_TRANSCRIPT_QUOTE_BYTES,
                repeat_fault_threshold: DEFAULT_REPEAT_FAULT_THRESHOLD
            }
        );

        let zero = runs_policy(&["--duration-floor".to_string(), "0".to_string()])
            .expect_err("a zero floor is a misconfiguration");
        assert_eq!(zero.exit_code, 2);
    }

    #[test]
    fn runs_requires_the_runs_flag() {
        let failure = runs(&[]).expect_err("--runs is required");
        assert_eq!(failure.exit_code, 2);
        assert!(failure.message.starts_with("usage: autospec dispatch runs"));
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static FIXTURE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    const NOW: u64 = 1_800_000_000;

    fn fixture_dir(label: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "autospec-dispatch-lifecycle-{label}-{}-{}",
            std::process::id(),
            FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&directory).expect("fixture directory");
        directory
    }

    /// A queue artifact the liveness gate will authorize: stamped by the
    /// reference topology's producer within the freshness threshold.
    fn live_queue(dir: &Path, entries: &[u64], stamped_at: u64) -> PathBuf {
        let path = dir.join("queue.json");
        let queue = QueueFile {
            entries: entries.to_vec(),
            refreshed_at: Some(stamped_at),
            refreshed_by: Some("refresh-queue".to_string()),
        };
        fs::write(&path, queue.render()).expect("queue artifact");
        path
    }

    fn lifecycle_path(dir: &Path) -> PathBuf {
        dir.join("lifecycle.json")
    }

    fn mark_args(lifecycle: &Path, action: &str, issue: u64, at: u64) -> Vec<String> {
        vec![
            "--action".to_string(),
            action.to_string(),
            "--issue".to_string(),
            issue.to_string(),
            "--lifecycle".to_string(),
            lifecycle.to_string_lossy().into_owned(),
            "--at".to_string(),
            at.to_string(),
        ]
    }

    fn mark_hold_args(lifecycle: &Path, issue: u64, reason: &str, at: u64) -> Vec<String> {
        let mut args = mark_args(lifecycle, "hold", issue, at);
        args.extend(["--reason".to_string(), reason.to_string()]);
        args
    }

    fn tick_args(queue: &Path, lifecycle: &Path, now: u64) -> Vec<String> {
        vec![
            "--queue".to_string(),
            queue.to_string_lossy().into_owned(),
            "--state-file".to_string(),
            queue
                .with_file_name("liveness.json")
                .to_string_lossy()
                .into_owned(),
            "--lifecycle".to_string(),
            lifecycle.to_string_lossy().into_owned(),
            "--now".to_string(),
            now.to_string(),
        ]
    }

    fn read_lifecycle(path: &Path) -> serde_json::Value {
        let text = fs::read_to_string(path).expect("lifecycle ledger written");
        serde_json::from_str(&text).expect("lifecycle ledger parses")
    }

    #[test]
    fn tick_reports_fresh_and_convert_per_entry() {
        let dir = fixture_dir("mixed");
        let queue = live_queue(&dir, &[101, 102, 110], NOW - 30);
        let lifecycle = lifecycle_path(&dir);

        // 102 has a patch waiting; 110 already converted (stale in the queue).
        mark(&mark_args(&lifecycle, "produced", 102, NOW - 20)).expect("102 produced");
        mark(&mark_args(&lifecycle, "converted", 110, NOW - 19)).expect("110 converted");
        // A backwards stamp is refused, exit 1, naming the existing stamp.
        let refused = mark(&mark_args(&lifecycle, "produced", 102, NOW - 30))
            .expect_err("stamps are monotonic");
        assert_eq!(refused.exit_code, 1);
        assert!(refused.message.contains("monotonic"), "{}", refused.message);

        tick(&tick_args(&queue, &lifecycle, NOW)).expect("something is dispatched");

        let records = &read_lifecycle(&lifecycle)["records"];
        assert_eq!(records["102"]["state"], "produced");
        assert_eq!(records["110"]["state"], "converted");
        // 101 was dispatched fresh, so the tick recorded it: in flight,
        // with a no-patch count of zero (#4451) — the dispatch is durable,
        // not inferred from the absence of a patch file.
        let fresh = records.get("101").expect("a fresh dispatch is recorded");
        assert_eq!(fresh["attempts"], 0);
        assert_eq!(fresh["dispatched_at"], NOW);
    }

    #[test]
    fn tick_holds_bound_exceeded_entries_and_reports_the_count() {
        let dir = fixture_dir("bound");
        // Stamped well inside the freshness window for every --now below.
        let queue = live_queue(&dir, &[101], NOW - 600);
        let lifecycle = lifecycle_path(&dir);

        // Three dispatches, each ending in a failed run: the tick records
        // the dispatch, mark failed records the outcome.
        for offset in [0u64, 10, 20] {
            let at = NOW - 60 + offset;
            tick(&tick_args(&queue, &lifecycle, at)).expect("within the bound");
            mark(&mark_args(&lifecycle, "failed", 101, at + 5)).expect("failed recorded");
        }

        // The fourth dispatch is refused: the entry is held, with the count
        // and the reason, and the run reports the over-bound skip.
        let stall = tick(&tick_args(&queue, &lifecycle, NOW))
            .expect_err("the bound holds: nothing is dispatchable");
        assert_eq!(stall.exit_code, 1);

        let records = &read_lifecycle(&lifecycle)["records"];
        assert_eq!(records["101"]["attempts"], 3);
        let reason = records["101"]["held_reason"]
            .as_str()
            .expect("the hold is durable");
        assert!(reason.contains("3 dispatches with no patch"), "{reason}");
        assert!(reason.contains("bound 3"), "{reason}");

        // Releasing is the re-arm: the attempt count resets and the entry
        // dispatches again.
        mark(&mark_args(&lifecycle, "release", 101, NOW + 10)).expect("released");
        tick(&tick_args(&queue, &lifecycle, NOW + 20)).expect("rearmed entry dispatches");
        let records = &read_lifecycle(&lifecycle)["records"];
        assert_eq!(records["101"]["attempts"], 0);
    }

    #[test]
    fn tick_reports_in_flight_skips_and_does_not_redispatch() {
        let dir = fixture_dir("inflight");
        let queue = live_queue(&dir, &[101], NOW - 600);
        let lifecycle = lifecycle_path(&dir);

        tick(&tick_args(&queue, &lifecycle, NOW - 60)).expect("first dispatch");

        // The run is still going: the second tick must wait, not redispatch.
        let second = tick(&tick_args(&queue, &lifecycle, NOW - 50))
            .expect_err("in flight: nothing dispatchable");
        assert_eq!(second.exit_code, 1);

        // The outcome arrives with no patch: the count advances and the
        // in-flight flag clears.
        mark(&mark_args(&lifecycle, "failed", 101, NOW - 40)).expect("failed recorded");
        tick(&tick_args(&queue, &lifecycle, NOW)).expect("within the bound again");
        let records = &read_lifecycle(&lifecycle)["records"];
        assert_eq!(records["101"]["attempts"], 1);
        assert_eq!(records["101"]["dispatched_at"], NOW);
    }

    #[test]
    fn mark_failed_refuses_terminal_and_is_a_usage_error_without_action() {
        let dir = fixture_dir("failed-usage");
        let lifecycle = lifecycle_path(&dir);

        mark(&mark_args(&lifecycle, "converted", 110, NOW - 20)).expect("110 converted");
        let refused = mark(&mark_args(&lifecycle, "failed", 110, NOW - 10))
            .expect_err("a terminal entry cannot record a failed run");
        assert_eq!(refused.exit_code, 1);
        assert!(refused.message.contains("converted"), "{}", refused.message);

        let unknown = mark(&mark_args(&lifecycle, "nope", 101, NOW))
            .expect_err("unknown action is a usage error");
        assert_eq!(unknown.exit_code, 2);
        assert!(unknown.message.contains("failed"), "{}", unknown.message);
    }

    #[test]
    fn stall_tick_exits_nonzero_and_names_every_skip() {
        let dir = fixture_dir("stall");
        let queue = live_queue(&dir, &[108, 109, 110], NOW - 30);
        let lifecycle = lifecycle_path(&dir);

        mark(&mark_args(&lifecycle, "produced", 108, NOW - 25)).expect("108 produced");
        mark(&mark_hold_args(
            &lifecycle,
            108,
            "conversion blocked: branch dirty",
            NOW - 24,
        ))
        .expect("108 held");
        mark(&mark_args(&lifecycle, "produced", 109, NOW - 23)).expect("109 produced");
        mark(&mark_hold_args(
            &lifecycle,
            109,
            "target branch protected",
            NOW - 22,
        ))
        .expect("109 held");
        mark(&mark_args(&lifecycle, "converted", 110, NOW - 21)).expect("110 converted");

        let stall = tick(&tick_args(&queue, &lifecycle, NOW))
            .expect_err("nothing is dispatchable, so the tick is a stall");
        assert_eq!(stall.exit_code, 1);

        let records = &read_lifecycle(&lifecycle)["records"];
        assert_eq!(
            records["108"]["held_reason"],
            "conversion blocked: branch dirty"
        );
        assert_eq!(records["109"]["held_reason"], "target branch protected");
    }

    #[test]
    fn release_reenters_as_convert_not_fresh() {
        let dir = fixture_dir("release");
        let queue = live_queue(&dir, &[108], NOW - 30);
        let lifecycle = lifecycle_path(&dir);

        mark(&mark_args(&lifecycle, "produced", 108, NOW - 25)).expect("108 produced");
        mark(&mark_hold_args(&lifecycle, 108, "branch dirty", NOW - 24)).expect("108 held");
        mark(&mark_args(&lifecycle, "release", 108, NOW - 23)).expect("108 released");

        tick(&tick_args(&queue, &lifecycle, NOW))
            .expect("a released produced entry converts, so the tick dispatches");

        let record = &read_lifecycle(&lifecycle)["records"]["108"];
        assert_eq!(record["state"], "produced", "release preserves the state");
        assert!(
            record.get("held_reason").is_none(),
            "release clears the hold: {record:?}"
        );
    }

    #[test]
    fn stale_queue_tick_holds_rather_than_reads_no_work() {
        let dir = fixture_dir("stale");
        // Four intervals of silence against a tolerance of three.
        let queue = live_queue(&dir, &[7], NOW - 2_400);
        let lifecycle = lifecycle_path(&dir);

        let held = tick(&tick_args(&queue, &lifecycle, NOW))
            .expect_err("a stale queue must hold, not read as 'no work'");
        assert_eq!(held.exit_code, 1);
        assert!(
            !lifecycle.exists(),
            "a hold must not touch the lifecycle ledger"
        );
    }

    #[test]
    fn hold_without_reason_is_a_usage_error() {
        let dir = fixture_dir("usage");
        let lifecycle = lifecycle_path(&dir);

        let missing_reason =
            mark(&mark_args(&lifecycle, "hold", 108, NOW)).expect_err("hold needs a reason");
        assert_eq!(missing_reason.exit_code, 2);

        let missing_action =
            mark(&["--issue".to_string(), "108".to_string()]).expect_err("mark needs an action");
        assert_eq!(missing_action.exit_code, 2);
        assert!(missing_action
            .message
            .starts_with("usage: autospec dispatch mark"));
    }

    #[test]
    fn mark_refuses_hold_on_converted_entry() {
        let dir = fixture_dir("terminal");
        let lifecycle = lifecycle_path(&dir);

        mark(&mark_args(&lifecycle, "converted", 110, NOW - 20)).expect("110 converted");
        // Re-marking converted at the same instant is an idempotent no-op.
        mark(&mark_args(&lifecycle, "converted", 110, NOW - 20)).expect("idempotent");

        let refused = mark(&mark_hold_args(&lifecycle, 110, "anything", NOW - 19))
            .expect_err("a terminal entry cannot be held");
        assert_eq!(refused.exit_code, 1);
        assert!(refused.message.contains("converted"), "{}", refused.message);
    }
}
