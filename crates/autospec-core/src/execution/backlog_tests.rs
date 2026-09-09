//! Tests for the authoritative-field backlog metric (#3924).
//!
//! Mounted via `#[path = "backlog_tests.rs"] mod tests;` in
//! `backlog.rs` so the module under test stays within the file-size
//! budget.

use super::*;

fn merged_pr(branch: &str, linked: Option<u64>, body: &str) -> PrRecord {
    PrRecord {
        head_branch: branch.to_string(),
        linked_issue: linked,
        description: body.to_string(),
        state: PrState::Merged,
    }
}

fn open_pr(branch: &str, linked: Option<u64>, body: &str) -> PrRecord {
    PrRecord {
        head_branch: branch.to_string(),
        linked_issue: linked,
        description: body.to_string(),
        state: PrState::Open,
    }
}

fn closed_pr(branch: &str, linked: Option<u64>) -> PrRecord {
    PrRecord {
        head_branch: branch.to_string(),
        linked_issue: linked,
        description: String::new(),
        state: PrState::Closed,
    }
}

fn set(values: &[u64]) -> BTreeSet<u64> {
    values.iter().copied().collect()
}

// ── name heuristic: anchored marker, suffixes tolerated, unrelated digit
//    runs rejected ──────────────────────────────────────────────────────

#[test]
fn branch_heuristic_handles_suffixed_names_the_original_missed() {
    assert_eq!(issue_number_from_branch("fix/issue-3685"), Some(3685));
    assert_eq!(issue_number_from_branch("conv/3845-fix2"), Some(3845));
    assert_eq!(issue_number_from_branch("conv/3870-fix"), Some(3870));
    assert_eq!(
        issue_number_from_branch("feat/issue-4123-backfill"),
        Some(4123)
    );
}

#[test]
fn branch_heuristic_rejects_unrelated_digit_runs() {
    // A date after the marker: 8 digits is not an issue number.
    assert_eq!(issue_number_from_branch("conv/20260909-rerun"), None);
    // Digits with no marker anywhere anchored: not an issue number.
    assert_eq!(issue_number_from_branch("chore/8080-sweep"), None);
    assert_eq!(issue_number_from_branch("fix/retry-2026"), None);
    // Two-digit run is below the 3-digit floor.
    assert_eq!(issue_number_from_branch("conv/12-x"), None);
    assert_eq!(issue_number_from_branch("main"), None);
}

#[test]
fn extraction_reports_what_it_failed_to_match() {
    let audit = extract_issue_numbers_from_branches([
        "conv/3845-fix2",
        "chore/8080-sweep",
        "conv/20260909-rerun",
    ]);
    assert_eq!(audit.matched, 1);
    assert_eq!(audit.matched_numbers(), &set(&[3845]));
    // The non-matches are named, not silently dropped (#3924 invariant 3).
    assert_eq!(audit.unmatched_count(), 2);
    assert_eq!(
        audit.unmatched_branches,
        vec!["chore/8080-sweep", "conv/20260909-rerun"]
    );
}

// ── authoritative identity: linked issue, then trailer, never the name ─

#[test]
fn trailer_parsing_finds_closing_and_refs_keywords() {
    assert_eq!(issue_number_from_trailer("Closes #3870"), Some(3870));
    assert_eq!(issue_number_from_trailer("fixes #3845."), Some(3845));
    assert_eq!(
        issue_number_from_trailer("subject line\n\nRefs #3883\n"),
        Some(3883)
    );
    // A mid-sentence mention is not a trailer.
    assert_eq!(
        issue_number_from_trailer("this closes #1 someday"),
        None,
        "keyword must start the line"
    );
    assert_eq!(issue_number_from_trailer("Closes without a number"), None);
}

#[test]
fn authoritative_issue_prefers_linked_field_and_never_reads_the_branch() {
    let with_link = open_pr("conv/111-fix", Some(3845), "Closes #999");
    assert_eq!(with_link.authoritative_issue(), Some(3845));

    let trailer_only = open_pr("some-branch", None, "do the thing\n\nCloses #3845");
    assert_eq!(trailer_only.authoritative_issue(), Some(3845));

    // A branch name full of numbers associates nothing without a field.
    let unlinked = open_pr("conv/3845-fix2", None, "no trailer here");
    assert_eq!(unlinked.authoritative_issue(), None);
}

// ── the populated case from the evidence (#3793) ──────────────────────

/// The exact scenario from 2026-09-09: issues 3870, 3849, 3845, 3888 and
/// 3883 merged within the hour on suffixed branches. The original
/// `[0-9]+$` extraction matched none of them and listed all five as
/// outstanding.
fn evidence_snapshot() -> BacklogSnapshot {
    let merged = set(&[3870, 3849, 3845, 3888, 3883]);
    let still_open: BTreeSet<u64> = (3900..3920).collect();
    let mut open_issues = merged.clone();
    open_issues.extend(still_open.iter().copied());
    // Every open issue has a patch on disk.
    let mut patched = open_issues.clone();
    patched.extend([3990, 3991]); // patches whose issue is already closed
    BacklogSnapshot {
        open_issues,
        patched_issues: patched,
        prs: vec![
            merged_pr("conv/3870-fix", None, "converted\n\nCloses #3870"),
            merged_pr("conv/3845-fix2", None, "converted\n\nCloses #3845"),
            merged_pr("conv/3849-sweep", Some(3849), "no trailer"),
            merged_pr("conv/3888-retry-2", None, "Refs #3888"),
            merged_pr("fix/issue-3883", Some(3883), ""),
            open_pr("conv/3905-fix", Some(3905), ""),
        ],
        known_merged: merged,
    }
}

#[test]
fn merged_this_session_never_appears_as_outstanding() {
    let report = compute_backlog(&evidence_snapshot()).expect("authoritative backlog computes");
    for issue in [3870u64, 3849, 3845, 3888, 3883] {
        assert!(
            !report.outstanding.contains(&issue),
            "merged issue {issue} listed as outstanding"
        );
    }
    // The open issue with an open PR is also off the list.
    assert!(!report.outstanding.contains(&3905));
    assert_eq!(
        report.outstanding,
        set(&[
            3900, 3901, 3902, 3903, 3904, 3906, 3907, 3908, 3909, 3910, 3911, 3912, 3913, 3914,
            3915, 3916, 3917, 3918, 3919
        ])
    );
    assert_eq!(report.convertible(), 19);
    assert_eq!(
        report.issues_with_pr,
        set(&[3870, 3845, 3849, 3888, 3883, 3905])
    );
}

#[test]
fn summary_line_reports_every_dispatch_count_together() {
    let report = compute_backlog(&evidence_snapshot()).unwrap();
    let line = report.summary_line();
    // All four reconcilable counts on one line (#3924 invariant 4).
    assert!(line.contains("convertible 19"), "{line}");
    assert!(line.contains("patches on disk 27"), "{line}");
    assert!(line.contains("issues open 25"), "{line}");
    assert!(line.contains("issues with a PR 6"), "{line}");
    // This snapshot's PR branches all match, so nothing to reconcile.
    assert!(
        report.discrepancies().is_empty(),
        "{:?}",
        report.discrepancies()
    );
}

#[test]
fn known_answer_check_fails_loudly_on_the_naive_extraction() {
    // The original trailing-digits extraction: `[0-9]+$` on
    // `conv/3845-fix2` matches nothing, so 3845 looks unconverted.
    let naive = |branch: &str| -> Option<u64> {
        let digits: String = branch
            .chars()
            .rev()
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        digits.parse().ok()
    };
    // The trailing-digits grab reads the `2` of the `fix2` suffix, not
    // the issue number: 3845 is never produced, which is the defect.
    assert_ne!(
        naive("conv/3845-fix2"),
        Some(3845),
        "the documented original defect"
    );
    assert_eq!(naive("conv/3870-fix"), None, "suffix kills the match");
    let naive_outstanding: BTreeSet<u64> = [3845u64, 3849, 3870].into_iter().collect();
    let known_merged = set(&[3845, 3870]);
    let err = known_answer_check(&naive_outstanding, &known_merged)
        .expect_err("a merged issue in the outstanding set must fail the check");
    assert!(err.contains("3845"), "{err}");
    assert!(err.contains("3870"), "{err}");
}

#[test]
fn known_answer_check_passes_when_identity_is_authoritative() {
    let snapshot = evidence_snapshot();
    let report = compute_backlog(&snapshot).unwrap();
    known_answer_check(&report.outstanding, &snapshot.known_merged).unwrap();
}

#[test]
fn compute_backlog_refuses_a_snapshot_where_a_known_merged_issue_is_outstanding() {
    let mut snapshot = evidence_snapshot();
    // Simulate a broken association: the merged PR lost its linked issue
    // and its trailer, so nothing ties it back to 3870.
    for pr in &mut snapshot.prs {
        if pr.head_branch == "conv/3870-fix" {
            pr.linked_issue = None;
            pr.description = String::new();
        }
    }
    let err = compute_backlog(&snapshot).expect_err("known-answer must gate the metric");
    assert!(err.contains("3870"), "{err}");
}

#[test]
fn closed_pr_does_not_associate_the_issue_stays_outstanding() {
    let snapshot = BacklogSnapshot {
        open_issues: set(&[4001]),
        patched_issues: set(&[4001]),
        prs: vec![closed_pr("conv/4001-fix", Some(4001))],
        known_merged: BTreeSet::new(),
    };
    let report = compute_backlog(&snapshot).unwrap();
    assert_eq!(report.outstanding, set(&[4001]));
    // The closed PR's branch still enters the name audit.
    assert_eq!(report.branch_audit.matched, 1);
}

#[test]
fn discrepancy_surfaces_unmatched_branches_and_heuristic_drift() {
    let snapshot = BacklogSnapshot {
        open_issues: set(&[4100, 4101]),
        patched_issues: set(&[4100, 4101]),
        prs: vec![
            // Open PR, authoritative link present, but branch name has
            // no extractable number: the heuristic would count 4100 as
            // outstanding while the authoritative report does not.
            open_pr("chore/manual-kickoff-8080", Some(4100), ""),
        ],
        known_merged: BTreeSet::new(),
    };
    let report = compute_backlog(&snapshot).unwrap();
    assert_eq!(report.outstanding, set(&[4101]));
    assert!(report.heuristic_outstanding.contains(&4100));
    let discrepancies = report.discrepancies();
    assert_eq!(discrepancies.len(), 2, "{discrepancies:?}");
    assert!(
        discrepancies
            .iter()
            .any(|d| d.contains("1 branch name(s) produced no issue number")),
        "{discrepancies:?}"
    );
    assert!(
        discrepancies
            .iter()
            .any(|d| d.contains("heuristic-only: [4100]")),
        "{discrepancies:?}"
    );
}
