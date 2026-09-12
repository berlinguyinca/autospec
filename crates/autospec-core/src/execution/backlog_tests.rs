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
        in_flight: BTreeSet::new(),
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
    // Every reconcilable count on one line (#3924 invariant 4), the raw
    // open count and the open count after reconciliation (#4044) side by
    // side: the tracker lists 25 open, but 2 (3870, 3845) were delivered
    // by merged PRs that the tracker never closed.
    assert!(line.contains("convertible 19"), "{line}");
    assert!(line.contains("patches on disk 27"), "{line}");
    assert!(line.contains("issues open 25"), "{line}");
    assert!(line.contains("open after reconcile 23"), "{line}");
    assert!(line.contains("issues with a PR 6"), "{line}");
    // Branch names all match and the heuristic agrees, but reconciliation
    // finds the tracker/merged-PR mismatches (#4044): 2 delivered-but-open
    // (3870, 3845) and 3 no-closure-decision (3849, 3883, 3888).
    assert_eq!(report.tracker_discrepancies.len(), 5);
    assert_eq!(
        report.discrepancies().len(),
        5,
        "{:?}",
        report.discrepancies()
    );
    assert!(
        report
            .discrepancies()
            .iter()
            .any(|d| d.contains("#3870") && d.contains("conv/3870-fix")),
        "{:?}",
        report.discrepancies()
    );
    assert!(
        report
            .discrepancies()
            .iter()
            .any(|d| d.contains("#3888") && d.contains("conv/3888-retry-2")),
        "{:?}",
        report.discrepancies()
    );
}

// ── tracker reconciliation: merged PR vs open issue (#4044) ────────────

#[test]
fn reconciliation_names_both_directions_of_the_mismatch() {
    let report = compute_backlog(&evidence_snapshot()).unwrap();
    let found = &report.tracker_discrepancies;
    // Delivered but the tracker never closed: the Closes PR merged, the
    // issue is still open.
    let delivered: Vec<u64> = found
        .iter()
        .filter(|d| d.kind == super::super::closure::TrackerDiscrepancyKind::DeliveredButOpen)
        .map(|d| d.issue)
        .collect();
    assert_eq!(delivered, vec![3845, 3870], "{found:?}");
    // Merged with no closing decision: linked-only, or a bare Refs — the
    // defect that left 51 issues open on delivered work.
    let undecided: Vec<u64> = found
        .iter()
        .filter(|d| d.kind == super::super::closure::TrackerDiscrepancyKind::NoClosureDecision)
        .map(|d| d.issue)
        .collect();
    assert_eq!(undecided, vec![3849, 3883, 3888], "{found:?}");
    // The open PR is work in progress, never a mismatch.
    assert!(found.iter().all(|d| d.issue != 3905), "{found:?}");
}

#[test]
fn effective_open_count_subtracts_only_delivered_issues() {
    let report = compute_backlog(&evidence_snapshot()).unwrap();
    // Only tracking lag drops out: 3870 and 3845 are delivered. The
    // no-decision issues still need work and stay in the count.
    assert_eq!(report.issues_open, 25);
    assert_eq!(report.issues_open_effective, 23);
}

#[test]
fn explicit_partial_fix_is_neither_a_discrepancy_nor_a_closure() {
    let snapshot = BacklogSnapshot {
        open_issues: set(&[4050]),
        patched_issues: set(&[4050]),
        prs: vec![merged_pr(
            "conv/4050-partial",
            None,
            "first slice\n\nRefs #4050 (does not close it)",
        )],
        known_merged: BTreeSet::new(),
        in_flight: BTreeSet::new(),
    };
    let report = compute_backlog(&snapshot).unwrap();
    // The stated decision is respected: no mismatch reported.
    assert!(
        report.tracker_discrepancies.is_empty(),
        "{:?}",
        report.tracker_discrepancies
    );
    assert!(
        report.discrepancies().is_empty(),
        "{:?}",
        report.discrepancies()
    );
    // The merged PR still associates the issue for backlog purposes
    // (#3924 invariant 1/2: a merged PR keeps its issue off the
    // outstanding list); the next slice lands as new work. But the issue
    // is not closed in the tracker, and the counts say so: raw and
    // effective open counts agree.
    assert!(report.outstanding.is_empty());
    assert!(report.issues_with_pr.contains(&4050));
    assert_eq!(report.issues_open, 1);
    assert_eq!(report.issues_open_effective, 1);
}

#[test]
fn generated_conversion_bodies_authorize_closure_partial_fixes_do_not() {
    use super::super::closure::{conversion_pr_body, has_partial_fix_marker, partial_fix_pr_body};
    use crate::evidence_fidelity::closure_authorized;
    // AC1/AC4: every body the conversion pass generates carries a closing
    // keyword; the partial-fix body states its decision explicitly instead.
    for issue in [7u64, 3888, 4044] {
        let full = conversion_pr_body(issue, "converted the patch");
        assert!(
            closure_authorized(&full, issue).is_authorized(),
            "full: {full:?}"
        );
        let partial = partial_fix_pr_body(issue, "partial sweep");
        assert!(
            !closure_authorized(&partial, issue).is_authorized(),
            "{partial:?}"
        );
        assert!(has_partial_fix_marker(&partial, issue), "{partial:?}");
    }
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
        in_flight: BTreeSet::new(),
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
        in_flight: BTreeSet::new(),
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

// ── in-flight conversion claims: "is anyone working on this?" (#4214) ──

#[test]
fn a_claimed_issue_is_excluded_from_outstanding_and_reported_in_flight() {
    let snapshot = BacklogSnapshot {
        open_issues: set(&[4200, 4201, 4202]),
        patched_issues: set(&[4200, 4201, 4202]),
        prs: Vec::new(),
        known_merged: BTreeSet::new(),
        // A running pass holds 4201's patch: no PR yet, work in progress.
        in_flight: set(&[4201]),
    };
    let report = compute_backlog(&snapshot).unwrap();
    // The claim outranks the PR check: 4201 is not convertible by anyone
    // else, even though nothing on GitHub says so.
    assert_eq!(report.outstanding, set(&[4200, 4202]));
    assert_eq!(report.in_flight, set(&[4201]));
    assert!(report.stale_claims.is_empty(), "{:?}", report.stale_claims);
    // The count is visible on the summary line, next to convertible.
    let line = report.summary_line();
    assert!(line.contains("convertible 2"), "{line}");
    assert!(line.contains("in flight 1"), "{line}");
}

#[test]
fn a_claim_on_an_issue_with_an_open_pr_is_not_in_flight_and_not_stale() {
    // The pass finished and opened the PR but the trap did not fire yet: the
    // claim is subsumed by the PR. It is not "in flight" (the PR says so)
    // and it is not stale (the patch is still open and patched); it is
    // simply reported neither way.
    let snapshot = BacklogSnapshot {
        open_issues: set(&[4203]),
        patched_issues: set(&[4203]),
        prs: vec![open_pr("conv/4203-fix", Some(4203), "")],
        known_merged: BTreeSet::new(),
        in_flight: set(&[4203]),
    };
    let report = compute_backlog(&snapshot).unwrap();
    assert!(report.outstanding.is_empty());
    assert!(report.in_flight.is_empty(), "{:?}", report.in_flight);
    assert!(report.stale_claims.is_empty(), "{:?}", report.stale_claims);
    let line = report.summary_line();
    assert!(line.contains("in flight 0"), "{line}");
}

// ── zero backlog: proven vs unmeasured (issue #4449) ───────────────────

#[test]
fn a_zero_backlog_built_on_seen_candidates_is_proven() {
    let snapshot = BacklogSnapshot {
        open_issues: set(&[4300]),
        patched_issues: set(&[4300]),
        prs: vec![open_pr("conv/4300-fix", Some(4300), "")],
        known_merged: BTreeSet::new(),
        in_flight: BTreeSet::new(),
    };
    let report = compute_backlog(&snapshot).unwrap();
    assert!(report.outstanding.is_empty());
    let zero = report
        .outstanding_zero
        .as_ref()
        .expect("a zero backlog carries its zero-control");
    assert!(!zero.is_unmeasured(), "{}", zero.line());
    assert!(
        zero.line().starts_with("empty (proven):"),
        "{}",
        zero.line()
    );
    // A proven zero reads as an ordinary count on the summary line.
    assert!(
        !report.summary_line().contains("UNMEASURED"),
        "{}",
        report.summary_line()
    );
    assert!(
        report.discrepancies().is_empty(),
        "{:?}",
        report.discrepancies()
    );
}

#[test]
fn a_zero_backlog_that_never_saw_a_candidate_is_unmeasured_and_reported() {
    // The comm incident: the loop would have logged a byte-identical
    // "convertible 0" for a broken enumeration and a drained backlog. The
    // line now says which it is.
    let snapshot = BacklogSnapshot {
        open_issues: BTreeSet::new(),
        patched_issues: BTreeSet::new(),
        prs: Vec::new(),
        known_merged: BTreeSet::new(),
        in_flight: BTreeSet::new(),
    };
    let report = compute_backlog(&snapshot).unwrap();
    assert!(report.outstanding.is_empty());
    let zero = report
        .outstanding_zero
        .as_ref()
        .expect("a zero backlog carries its zero-control");
    assert!(zero.is_unmeasured(), "{}", zero.line());
    assert!(
        report.summary_line().contains("UNMEASURED"),
        "{}",
        report.summary_line()
    );
    assert!(
        report
            .discrepancies()
            .iter()
            .any(|d| d.contains("UNMEASURED")),
        "{:?}",
        report.discrepancies()
    );
}

#[test]
fn a_zero_backlog_with_patches_but_no_open_issues_is_unmeasured() {
    // Patches on disk but the open-issue enumeration came back empty: the
    // intersection could not have returned a candidate, so the zero is not
    // a measurement of a drained backlog.
    let snapshot = BacklogSnapshot {
        open_issues: BTreeSet::new(),
        patched_issues: set(&[4400]),
        prs: Vec::new(),
        known_merged: BTreeSet::new(),
        in_flight: BTreeSet::new(),
    };
    let report = compute_backlog(&snapshot).unwrap();
    assert!(report.outstanding.is_empty());
    let zero = report
        .outstanding_zero
        .as_ref()
        .expect("zero carries its control");
    assert!(zero.is_unmeasured(), "{}", zero.line());
    assert!(zero.line().contains("open issues"), "{}", zero.line());
}

#[test]
fn a_non_empty_backlog_carries_no_zero_control() {
    let report = compute_backlog(&evidence_snapshot()).unwrap();
    assert!(!report.outstanding.is_empty());
    assert!(report.outstanding_zero.is_none());
}

#[test]
fn a_claim_naming_an_issue_with_no_open_patch_is_reported_as_stale() {
    // The pass is gone or the patch moved: the claim is evidence of drift,
    // reported rather than absorbed.
    let snapshot = BacklogSnapshot {
        open_issues: set(&[4204]),
        patched_issues: set(&[4204]),
        prs: Vec::new(),
        known_merged: BTreeSet::new(),
        in_flight: set(&[4204, 9999]),
    };
    let report = compute_backlog(&snapshot).unwrap();
    assert_eq!(report.outstanding, BTreeSet::new());
    assert_eq!(report.in_flight, set(&[4204]));
    assert_eq!(report.stale_claims, set(&[9999]));
    let discrepancies = report.discrepancies();
    assert!(
        discrepancies
            .iter()
            .any(|d| d.contains("stale claims") && d.contains("9999")),
        "{discrepancies:?}"
    );
}
