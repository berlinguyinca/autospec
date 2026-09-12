//! The dispatch queue's gap reconciliation (#4450).
//!
//! The bug: 178 open issues carried the eligibility label, 90 were queued, 75
//! had a branch or a PR, and 37 were in none of the three sets — filed,
//! labelled, invisible. Nothing compared the sets, so nothing said so. These
//! tests pin the four counts, the zero case (reported, not suppressed), the
//! defect being reported rather than corrected, and a missing component being
//! an error that names what was missing.

use autospec_core::procedure::{CommandResolver, Procedure, Step};
use autospec_core::queue_gap::{missing_components, MissingComponent, QueueGap, MAX_LISTED_ISSUES};

#[test]
fn gap_is_the_three_way_difference() {
    // 1, 2 queued; 3 covered by a PR; 4 and 5 in neither set.
    let gap = QueueGap::new([1, 2, 3, 4, 5], [1, 2], [3]);
    assert_eq!(gap.eligible_count(), 5);
    assert_eq!(gap.queued_count(), 2);
    assert_eq!(gap.has_branch_or_pr_count(), 1);
    assert_eq!(gap.missing, vec![4, 5]);
    assert!(gap.is_defect());
    assert!(gap.reconciles());
}

#[test]
fn branch_or_pr_coverage_is_not_counted_as_missing() {
    // The #3927 reconciliation counted these as admitted-but-unschedulable;
    // work in flight is excluded on purpose, so it must not read as a gap.
    let gap = QueueGap::new([10, 11, 12], [10], [11, 12]);
    assert_eq!(gap.missing_count(), 0);
    assert!(!gap.is_defect());
    assert!(gap.line().contains("has_branch_or_pr 2"));
}

#[test]
fn zero_gap_is_still_reported_with_all_four_counts() {
    let gap = QueueGap::new([1, 2], [1, 2], []);
    assert!(!gap.is_defect());
    assert_eq!(
        gap.line(),
        "queue gap: eligible 2, queued 2, has_branch_or_pr 0, missing 0"
    );
}

#[test]
fn defect_line_names_the_counts_and_the_missing_issues() {
    let gap = QueueGap::new([1, 2, 3, 4], [1], [2]);
    let line = gap.line();
    assert!(line.starts_with("QUEUE GAP DEFECT:"), "{line}");
    assert!(
        line.contains("eligible 4, queued 1, has_branch_or_pr 1, missing 2"),
        "{line}"
    );
    assert!(line.contains("3, 4"), "{line}");
    // Reported, never corrected: nothing here appends to a queue.
    assert!(line.contains("reported, not corrected"), "{line}");
}

#[test]
fn long_gaps_state_the_full_count_beyond_the_listed_head() {
    let eligible: Vec<u64> = (1..=(MAX_LISTED_ISSUES as u64 + 5)).collect();
    let gap = QueueGap::new(eligible, [], []);
    let line = gap.line();
    assert_eq!(gap.missing_count(), MAX_LISTED_ISSUES + 5);
    assert!(line.contains("missing 25"), "{line}");
    assert!(line.contains("+5 more (25 named in total)"), "{line}");
}

#[test]
fn duplicate_and_overlapping_inputs_do_not_move_the_counts() {
    let gap = QueueGap::new([1, 1, 2, 3], [2, 2, 3], [3, 1]);
    assert_eq!(gap.eligible_count(), 3);
    assert_eq!(gap.queued_count(), 2);
    assert!(gap.missing.is_empty(), "{:?}", gap.missing);
    assert!(gap.reconciles());
}

#[test]
fn empty_eligible_set_is_a_zero_gap_not_an_error() {
    let gap = QueueGap::new([], [7, 8], []);
    assert!(!gap.is_defect());
    assert_eq!(gap.missing_count(), 0);
    // A stale queue is this type's blind spot by design: it reports what is
    // missing, and `dispatch reconcile` reports what is stale.
    assert_eq!(
        gap.line(),
        "queue gap: eligible 0, queued 2, has_branch_or_pr 0, missing 0"
    );
}

#[test]
fn reconciles_detects_a_missing_list_that_does_not_match_its_inputs() {
    let mut gap = QueueGap::new([1, 2, 3], [1], []);
    assert!(gap.reconciles());
    // A hand-edited gap report that dropped issue 3 is a counting defect.
    gap.missing = vec![2];
    assert!(!gap.reconciles());
}

#[test]
fn missing_component_line_names_the_step_and_the_command() {
    let gap = MissingComponent {
        step: "refresh dispatch queue".to_string(),
        command: "refresh-queue.sh".to_string(),
    };
    let line = gap.line();
    assert!(line.contains("MISSING COMPONENT"), "{line}");
    assert!(line.contains("refresh dispatch queue"), "{line}");
    assert!(line.contains("refresh-queue.sh"), "{line}");
    assert!(line.contains("never a no-op"), "{line}");
}

#[test]
fn missing_components_names_the_step_with_no_implementation() {
    // The #4450 incident, in miniature: the loop step names a script the fleet
    // does not have, and the check says so instead of skipping the step.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let temp = std::env::temp_dir().join(format!(
        "autospec-queue-gap-components-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&temp).expect("temp dir");
    std::fs::write(temp.join("topup.sh"), "#!/usr/bin/env bash\n").expect("topup written");

    let procedure = Procedure::new(
        "operator-loop",
        vec![
            Step::run("refresh dispatch queue", "topup.sh"),
            Step::run("repopulate the queue", "refresh-queue.sh"),
        ],
    );
    let resolver = CommandResolver::in_dir(&temp);
    let missing = missing_components(&procedure, &|command| resolver.resolves(command));

    assert_eq!(missing.len(), 1, "only the absent script is named");
    assert_eq!(missing[0].step, "repopulate the queue");
    assert_eq!(missing[0].command, "refresh-queue.sh");

    let all_present = Procedure::new(
        "operator-loop",
        vec![Step::run("refresh dispatch queue", "topup.sh")],
    );
    assert!(missing_components(&all_present, &|command| resolver.resolves(command)).is_empty());
    std::fs::remove_dir_all(&temp).ok();
}
