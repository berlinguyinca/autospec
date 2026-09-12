//! A per-item loop must reset to a known-clean state (issue #4395).
//!
//! The regression tests run in the configuration the incident required:
//! a conversion loop that ended each iteration with `git add -A`, whose
//! next iteration began with `git checkout --detach` and `git clean -fd`
//! — a plan that leaves the previous iteration's staged files behind, so
//! the patch failed to apply and the gate measured a tree containing two
//! patches. Issue 4368 was held for a formatting failure that belonged to
//! the previous patch's files, and nothing in the output distinguished the
//! contaminated verdict from a true one.

use autospec_core::loop_reset::{
    git_reset_plan, repeat_run_findings, reset_coverage_findings, ItemVerdict, Mutation, ResetStep,
};

/// What an iteration that ends in `git add -A` can leave behind: the
/// modification and the new file are both staged.
const LEAVES_STAGED: [Mutation; 1] = [Mutation::Staged];

/// Everything an iteration that can stage, modify, and create files leaves
/// behind.
const LEAVES_EVERYTHING: [Mutation; 3] =
    [Mutation::Staged, Mutation::Unstaged, Mutation::Untracked];

/// The incident's reset plan: detach, then clean. Neither step removes
/// staged files.
const INCIDENT_PLAN: [ResetStep; 2] = [ResetStep::Checkout, ResetStep::Clean];

// --- Invariant 1: a reset operation has defined coverage ------------------

#[test]
fn the_coverage_table_is_the_invariant() {
    // A hard reset undoes staged AND unstaged; clean removes untracked; a
    // checkout undoes nothing.
    assert_eq!(
        ResetStep::HardReset.undoes(),
        &[Mutation::Staged, Mutation::Unstaged]
    );
    assert_eq!(ResetStep::Clean.undoes(), &[Mutation::Untracked]);
    assert!(ResetStep::Checkout.undoes().is_empty());
}

// --- Invariant 2: the plan must cover everything the iteration can do -----

#[test]
fn incident_reset_leaves_staged_state_behind() {
    // The incident: iteration N ended with `git add -A`; iteration N+1
    // began with a detach and `git clean -fd`. Neither removes staged
    // files, so the patch failed to apply ("already exists in working
    // directory") and the gate measured a tree containing two patches.
    let findings = reset_coverage_findings(&LEAVES_STAGED, &INCIDENT_PLAN);
    assert_eq!(findings.len(), 1);
    assert!(findings[0].starts_with("RESET_INCOMPLETE:"));
    assert!(findings[0].contains("staged"));
    // The finding names both steps the reader would have trusted.
    assert!(findings[0].contains("`git clean` alone is not a reset"));
    assert!(findings[0].contains("`git checkout` alone is not a reset"));
    assert!(findings[0].contains("hard reset to the base sha"));
}

#[test]
fn clean_alone_is_not_a_reset() {
    // `git clean` covers only untracked: staged and unstaged both leak.
    let findings = reset_coverage_findings(&LEAVES_EVERYTHING, &[ResetStep::Clean]);
    assert_eq!(findings.len(), 2);
    assert!(findings.iter().all(|f| f.starts_with("RESET_INCOMPLETE:")));
    assert!(findings[0].contains("staged") || findings[1].contains("staged"));
}

#[test]
fn checkout_alone_is_not_a_reset() {
    // A checkout moves HEAD and carries everything across: all three
    // classes leak.
    let findings = reset_coverage_findings(&LEAVES_EVERYTHING, &[ResetStep::Checkout]);
    assert_eq!(findings.len(), 3);
    assert!(findings
        .iter()
        .all(|f| f.contains("`git checkout` alone is not a reset")));
}

#[test]
fn hard_reset_alone_leaves_untracked_files() {
    let findings = reset_coverage_findings(&LEAVES_EVERYTHING, &[ResetStep::HardReset]);
    assert_eq!(findings.len(), 1);
    assert!(findings[0].contains("untracked"));
    assert!(findings[0].contains("git clean -fd"));
}

#[test]
fn no_reset_at_all_is_a_finding() {
    let findings = reset_coverage_findings(&LEAVES_STAGED, &[]);
    assert_eq!(findings.len(), 1);
    assert!(findings[0].contains("(none)"));
    assert!(findings[0].contains("hard reset to the base sha"));
}

#[test]
fn coverage_is_per_class_the_iteration_can_produce() {
    // A loop that only stages and modifies tracked files, and never creates
    // new ones, needs no clean: only the classes the iteration can produce
    // have to be covered.
    let findings = reset_coverage_findings(
        &[Mutation::Staged, Mutation::Unstaged],
        &[ResetStep::HardReset],
    );
    assert!(findings.is_empty());
}

// --- The canonical plan ----------------------------------------------------

#[test]
fn the_canonical_plan_is_hard_reset_then_clean() {
    let plan = git_reset_plan();
    assert_eq!(plan, [ResetStep::HardReset, ResetStep::Clean]);
    assert_eq!(
        plan.map(|step| step.command("785447cf")),
        [
            "git reset --hard 785447cf".to_string(), // linter:allow-SECURITY test fixture: the plan's expected command string, not an executed command
            "git clean -fd".to_string(),
        ]
    );
}

#[test]
fn the_canonical_plan_covers_everything_an_iteration_can_do() {
    let findings = reset_coverage_findings(&LEAVES_EVERYTHING, &git_reset_plan());
    assert!(findings.is_empty());
}

// --- Invariant 3: prove the restore by running the same item twice --------

#[test]
fn a_clean_loop_gives_the_same_verdict_both_times() {
    assert!(repeat_run_findings("4368", ItemVerdict::Held, ItemVerdict::Held).is_empty());
    assert!(repeat_run_findings("4368", ItemVerdict::Mergeable, ItemVerdict::Mergeable).is_empty());
}

#[test]
fn a_repeat_run_mismatch_is_state_leak() {
    // The order-dependent shape: the same item HELD on one pass because the
    // previous patch's files were still in the tree, mergeable when the
    // reset actually restored the workspace.
    let findings = repeat_run_findings("4368", ItemVerdict::Held, ItemVerdict::Mergeable);
    assert_eq!(findings.len(), 1);
    assert!(findings[0].starts_with("REPEAT_RUN_MISMATCH:"));
    assert!(findings[0].contains("4368"));
    assert!(findings[0].contains("iteration order"));
}

#[test]
fn the_mismatch_is_symmetric_in_direction() {
    // Worse in the other direction: an item passes because a previous item
    // supplied what it was missing. Same finding, reversed verdicts.
    let findings = repeat_run_findings("4368", ItemVerdict::Mergeable, ItemVerdict::Held);
    assert_eq!(findings.len(), 1);
    assert!(findings[0].starts_with("REPEAT_RUN_MISMATCH:"));
}

// --- The fixed loop, end to end -------------------------------------------

#[test]
fn the_fixed_loop_end_to_end() {
    // The fixed shape: the loop can stage, modify, and create; its reset is
    // the canonical plan, which covers all three classes; the same item
    // judged twice gives the same verdict.
    let plan = git_reset_plan();
    assert!(reset_coverage_findings(&LEAVES_EVERYTHING, &plan).is_empty());
    // 4368 genuinely fails fmt (the re-run with a proper reset showed it),
    // so the honest verdict is HELD — and the repeat run agrees.
    let verdict = ItemVerdict::Held;
    assert!(repeat_run_findings("4368", verdict, verdict).is_empty());
    // And the incident's plan still fails the same audit.
    assert_eq!(
        reset_coverage_findings(&LEAVES_STAGED, &INCIDENT_PLAN).len(),
        1
    );
}
