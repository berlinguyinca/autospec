//! The apply half of the conversion pass: the per-item loop that converts
//! what the plan selected, with the pass's own deadline sizing the batch
//! it starts (#4607).

use autospec_core::conversion_pass::{select_fresh, PassOutcome};
use autospec_core::unfed_pass::PassCounters;

use super::git::{run_git, run_git_capture};

use super::{
    buffer_from_candidates, Attempt, ApplyResult, ConvertPlan, ConversionBuffer, apply_one,
    gate_source, infer_repo, report_outcome,
};
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

    let gate_set = gate_source::resolve_gate(plan.opts.gate_registry.as_deref(), &repo)?;

    // Fetch the trunk so the pass branches off current origin/<base>.
    let base_ref = format!("origin/{}", plan.opts.base);
    run_git(&["fetch", "origin"])?;

    // The base the hold records will cite is the base the patches are gated
    // against — the tip of origin/<base>, not the checkout's HEAD (#4512).
    // HEAD can be any branch; citing it records a fact about the wrong ref.
    let base_sha = match run_git_capture(&["rev-parse", &base_ref]) {
        Ok(sha) => sha,
        Err(_) => {
            eprintln!(
                "WARN: {base_ref} could not be resolved; citing the checkout's HEAD in the                  hold records"
            );
            run_git_capture(&["rev-parse", "HEAD"])?
        }
    };

    // The classification is a fact about the base it was derived against
    // (#4512): if the base moved between the plan and now, revalidate before
    // mutating. A stale entry produces a reported reclassification, never a
    // silent act on the old state.
    let (candidates, flips, _current_base) = super::stale::revalidate(plan, &base_ref);
    for flip in &flips {
        progress::report_line(&format!(
            "  STALE #{issue}: {from} -> {to} (the base moved since the plan;              revalidated before acting)",
            issue = flip.issue,
            from = flip.from,
            to = flip.to
        ));
    }
    let selection = select_fresh(&candidates);

    let mut counters = PassCounters {
        examined: plan.candidates.len(),
        converted: 0,
        held: 0,
        skipped: selection.disqualified.len(),
        deferred: 0,
        delivered: 0,
    };
    // Patches archived this run (superseded by the base): they leave the
    // buffer entirely — no patch on disk, so no queue entry held.
    let mut archived = 0;

    language::archive_held(plan, &selection.language_held, &mut counters, &mut archived);

    // The delivered residue (#4501): reported and archived, never gated —
    // archiving releases the queue entry the patch held hostage.
    for c in &selection.delivered {
        let Some(patch) = plan.examined.iter().find(|p| p.issue == c.issue) else {
            continue;
        };
        progress::delivered(c.issue);
        if super::delivered::archive_patch(&patch.path) {
            archived += 1;
        }
        counters.delivered += 1;
    }
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
        // The issue is on the record before the first remote write: a run
        // killed anywhere below leaves START without DONE, and that is
        // exactly where it stopped (#4499).
        progress::started(patch.issue);
        // A candidate re-offered from the interrupted state owns the orphan branch:
        // its redo force-pushes over it (#4499).
        let redo_interrupted = candidates
            .iter()
            .find(|cand| cand.issue == c.issue)
            .map(|cand| cand.attempt == Attempt::Interrupted)
            .unwrap_or(false);
        match apply_one(plan, &repo, &base_ref, &base_sha, patch, redo_interrupted, &gate_set) {
            ApplyResult::Converted => {
                counters.converted += 1;
                progress::converted(patch.issue, &patch.patch_key);
                progress::finished(patch.issue, "converted");
            }
            ApplyResult::Held => {
                counters.held += 1;
                progress::finished(patch.issue, "held");
            }
            // Not converted and not held: the pass could not tell whether this
            // patch is good, so it says so and leaves the patch alone.
            ApplyResult::BaseUnverifiable => {
                counters.skipped += 1;
                progress::finished(patch.issue, "skipped: base unverifiable");
            }
            ApplyResult::Archived => {
                counters.skipped += 1;
                archived += 1;
                progress::finished(patch.issue, "archived");
            }
            ApplyResult::Delivered => {
                counters.delivered += 1;
                progress::finished(patch.issue, "delivered");
            }
        }
        last_cost = Some(item_started.elapsed());
    }

    let outcome = PassOutcome::Examined(counters);
    report_outcome(&outcome);

    // The buffer after the run (#4558 ask 3): converted patches leave the
    // waiting count (their PR is live) but hold their queue entry until
    // archived; archived patches leave both.
    let initial = buffer_from_candidates(&candidates);
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
