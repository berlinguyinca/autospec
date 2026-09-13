//! The apply half of the conversion pass: the per-item loop that converts
//! what the plan selected, with the pass's own deadline sizing the batch
//! it starts (#4607).

use autospec_core::conversion_pass::PassOutcome;
use autospec_core::unfed_pass::PassCounters;

use super::git::{run_git, run_git_capture};
use super::{ApplyResult, ConvertPlan, ConversionBuffer, apply_one, infer_repo, report_outcome};
use super::language;
use super::progress;
use super::sizing;
use crate::commands::CommandFailure;

pub(super) fn run_apply(plan: &ConvertPlan) -> Result<(), CommandFailure> {
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
    let base_sha = run_git_capture(&["rev-parse", "HEAD"])?;

    let mut counters = PassCounters {
        examined: plan.candidates.len(),
        converted: 0,
        held: 0,
        skipped: selection.disqualified.len(),
        deferred: 0,
    };
    // Patches archived this run (superseded by the base): they leave the
    // buffer entirely — no patch on disk, so no queue entry held.
    let mut archived = 0;

    language::archive_held(plan, &selection.language_held, &mut counters, &mut archived);
    // The pass sizes its batch to its own deadline (#4607): it stops
    // *starting* new patches when the remaining time is less than what a
    // patch has been observed to cost in this pass, and finishes the one in
    // flight. The candidates it never reaches are deferred, not lost — they
    // stay on disk, keep their queue entries, and are re-offered next pass —
    // but the outcome says so: a pass that quietly did 1 of 12 and one that
    // did 12 of 12 must not print the same shape of line.
    let started = std::time::Instant::now();
    let fresh = &selection.fresh;
    let mut last_cost: Option<std::time::Duration> = None;
    for (index, c) in fresh.iter().enumerate() {
        if let Some(deadline_secs) = plan.opts.deadline {
            let remaining =
                std::time::Duration::from_secs(deadline_secs).saturating_sub(started.elapsed());
            if sizing::should_defer(remaining, last_cost) {
                counters.deferred = fresh.len() - index;
                progress::report_line(&format!(
                    "  DEFER  {} candidate(s) not started: the remaining time is below the \
                     observed per-item cost; they stay on disk for the next pass",
                    counters.deferred
                ));
                break;
            }
        }
        let patch = match plan.examined.iter().find(|p| p.issue == c.issue) {
            Some(p) => p,
            None => continue,
        };
        let item_started = std::time::Instant::now();
        match apply_one(plan, &repo, &base_ref, &base_sha, patch) {
            ApplyResult::Converted => {
                counters.converted += 1;
                progress::converted(patch.issue, &patch.patch_key);
            }
            ApplyResult::Held => counters.held += 1,
            // Not converted and not held: the pass could not tell whether this
            // patch is good, so it says so and leaves the patch alone.
            ApplyResult::BaseUnverifiable => counters.skipped += 1,
            ApplyResult::Archived => {
                counters.skipped += 1;
                archived += 1;
            }
        }
        last_cost = Some(item_started.elapsed());
    }

    let outcome = PassOutcome::Examined(counters);
    report_outcome(&outcome);

    // The buffer after the run, not just the run itself (#4558 ask 3):
    // converted patches leave the waiting count (their PR is live) but
    // stay on disk and still hold their queue entry; archived patches
    // leave both. A report of only what converted hides how much is still
    // waiting.
    let initial = plan.buffer();
    let buffer = ConversionBuffer {
        waiting: initial.waiting.saturating_sub(counters.converted).saturating_sub(archived),
        queue_entries_blocked: initial.queue_entries_blocked.saturating_sub(archived),
    };
    println!("{}", buffer.line(plan.opts.free_slots));
    if let Some(alarm) = buffer.alarm(plan.opts.free_slots) {
        println!("{alarm}");
    }
    Ok(())
}
