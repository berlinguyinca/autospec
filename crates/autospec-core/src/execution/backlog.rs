//! The conversion backlog metric, with issue identity from authoritative fields (#3924).
//!
//! The backlog metric answers "which issues still need a PR". The original
//! implementation derived issue identity from branch-name shape by taking the
//! trailing digits, so every branch with a suffix (`conv/3845-fix2`,
//! `conv/3870-fix`) silently produced no match and its issue was counted as
//! not yet converted — including five issues merged within the previous hour.
//! The reported backlog was 220 against a true 215.
//!
//! The invariants this module enforces:
//!
//! 1. **Issue identity comes from an authoritative field, never from a
//!    name.** [`PrRecord::authoritative_issue`] uses the platform's linked
//!    issue first, then a `Closes #N` / `Fixes #N` / `Refs #N` trailer.
//!    Branch names are a human convenience only.
//! 2. **Every derived metric ships with a known-answer check.**
//!    [`known_answer_check`] asserts that issues known to be merged do not
//!    appear in the outstanding set, and fails loudly when they do. A metric
//!    that has never been run against a case whose answer is known is not yet
//!    a measurement (#3793).
//! 3. **A name-matching heuristic reports what it failed to match.**
//!    [`NameExtraction::unmatched_branches`] keeps the non-matching branch
//!    names visible instead of discarding them silently.
//! 4. **Counts that drive dispatch are reconcilable.**
//!    [`BacklogReport::summary_line`] prints `patches on disk`, `issues open`,
//!    `issues with a PR` and `convertible` together, and
//!    [`BacklogReport::discrepancies`] reports — rather than absorbs — any
//!    drift between the authoritative backlog and the name heuristic.
//! 5. **Eligibility is "is anyone working on this?", not "has anyone
//!    worked on this?"** (issue #4214). A patch held by an in-flight
//!    conversion pass has no PR yet and would otherwise read as convertible
//!    to the next overlapping pass. The claimed set
//!    ([`BacklogSnapshot::in_flight`], fed by
//!    [`crate::execution::conversion_claim::in_flight`]) is excluded from the
//!    outstanding set and reported — stale claims included — never absorbed.

use super::closure::{reconcile_tracker, reconciled_open_count, TrackerDiscrepancy};
use crate::false_negative::{self, Measured};
use std::collections::BTreeSet;

/// Conventional branch-name markers after which an issue number may appear.
/// An *anchored* marker, not a trailing-digit grab: this is the extraction
/// that reproduced the true figure (194 issues with a branch), where
/// `[0-9]+$` missed every suffixed branch and `[0-9]{4}` matched any 4-digit
/// run anywhere (334 against a true 194).
const BRANCH_ISSUE_MARKERS: [&str; 3] = ["issue-", "conv/", "fix/"];

/// Keywords that link a PR/commit to an issue in free text, per GitHub's
/// closing-keyword set plus `Refs` for non-closing association.
const LINK_TRAILER_KEYWORDS: [&str; 3] = ["Closes", "Fixes", "Refs"];

/// Extract an issue number from a branch name by convention.
///
/// **This is a heuristic, not an identifier** (#3924): branch names drift
/// the moment anyone adds a suffix or a retry marker. Use
/// [`PrRecord::authoritative_issue`] for association; use this only where a
/// name must be read (audits, diagnostics), and report non-matches via
/// [`extract_issue_numbers_from_branches`].
///
/// A match requires a marker from [`BRANCH_ISSUE_MARKERS`] followed by a run
/// of 3–5 ASCII digits terminated by end-of-name or a non-digit. The length
/// bound rejects unrelated digit runs (`conv/20260909-rerun` → `None`); the
/// anchor rejects digits that are not after a marker
/// (`chore/8080-sweep` → `None`).
///
/// ```text
/// fix/issue-3685        -> Some(3685)
/// conv/3845-fix2        -> Some(3845)   (suffix tolerated)
/// conv/3870-fix         -> Some(3870)
/// conv/20260909-rerun   -> None         (8-digit run is a date, not an issue)
/// chore/8080-sweep      -> None         (no marker)
/// ```
pub fn issue_number_from_branch(branch: &str) -> Option<u64> {
    for idx in branch.char_indices().map(|(i, _)| i) {
        let tail = &branch[idx..];
        for marker in BRANCH_ISSUE_MARKERS {
            let Some(after) = tail.strip_prefix(marker) else {
                continue;
            };
            let digit_end = after
                .find(|c: char| !c.is_ascii_digit())
                .unwrap_or(after.len());
            let digits = &after[..digit_end];
            if (3..=5).contains(&digits.len()) {
                return digits.parse().ok();
            }
            // Too short, or a run longer than 5 digits (a date, a port):
            // not an issue number. Keep scanning for a later marker.
        }
    }
    None
}

/// Parse a `Closes #N` / `Fixes #N` / `Refs #N` trailer from a PR body or
/// commit message. The keyword must start a line (trailer semantics, not a
/// mid-sentence mention); the first matching trailer wins. Keywords are
/// matched case-insensitively.
pub fn issue_number_from_trailer(text: &str) -> Option<u64> {
    for line in text.lines() {
        let line = line.trim_start();
        for keyword in LINK_TRAILER_KEYWORDS {
            let matched =
                line.len() >= keyword.len() && line[..keyword.len()].eq_ignore_ascii_case(keyword);
            if !matched {
                continue;
            }
            let rest = line[keyword.len()..].trim_start();
            let Some(after_hash) = rest.strip_prefix('#') else {
                continue;
            };
            let digit_end = after_hash
                .find(|c: char| !c.is_ascii_digit())
                .unwrap_or(after_hash.len());
            if digit_end > 0 {
                return after_hash[..digit_end].parse().ok();
            }
        }
    }
    None
}

/// The state of a pull request as far as the backlog metric cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrState {
    /// Still open: the issue has work in flight.
    Open,
    /// Merged: the issue's work has landed; it must never appear as
    /// outstanding (#3924's known-answer case).
    Merged,
    /// Closed without merging: the PR does not associate the issue with
    /// delivered work, so the issue stays outstanding.
    Closed,
}

/// One pull request, as the backlog metric sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrRecord {
    /// The head branch name. A human convenience: it is audited
    /// ([`extract_issue_numbers_from_branches`]) but never used for issue
    /// association (#3924 invariant 1).
    pub head_branch: String,
    /// The platform's linked issue number, when the platform reports one.
    /// The most authoritative field available.
    pub linked_issue: Option<u64>,
    /// PR body / commit message, scanned for `Closes #N` / `Fixes #N` /
    /// `Refs #N` trailers when no linked issue is reported.
    pub description: String,
    /// Whether the PR is open, merged, or closed-unmerged.
    pub state: PrState,
}

impl PrRecord {
    /// The issue this PR authoritatively belongs to: the platform's linked
    /// issue first, then a closing/refs trailer. **Never the branch name.**
    pub fn authoritative_issue(&self) -> Option<u64> {
        self.linked_issue
            .or_else(|| issue_number_from_trailer(&self.description))
    }
}

/// Result of running the name-based extraction over branch names: the match
/// count *and* the names that produced no issue number (#3924 invariant 3 —
/// the original extraction discarded non-matches silently, which is exactly
/// what hid the defect).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NameExtraction {
    /// Branch names that yielded an issue number.
    pub matched: usize,
    /// Branch names that yielded nothing, verbatim, for the report.
    pub unmatched_branches: Vec<String>,
    /// The issue numbers found, for cross-checking against authoritative
    /// identity. Private: read it through [`NameExtraction::matched_numbers`].
    matched_numbers: BTreeSet<u64>,
}

impl NameExtraction {
    /// How many branch names produced no issue number.
    pub fn unmatched_count(&self) -> usize {
        self.unmatched_branches.len()
    }

    /// The issue numbers the heuristic found across all matched names.
    /// Diagnostic only — association uses
    /// [`PrRecord::authoritative_issue`].
    pub fn matched_numbers(&self) -> &BTreeSet<u64> {
        &self.matched_numbers
    }
}

/// Run [`issue_number_from_branch`] over a set of branch names, keeping the
/// non-matches visible instead of dropping them silently.
pub fn extract_issue_numbers_from_branches<'a>(
    branches: impl IntoIterator<Item = &'a str>,
) -> NameExtraction {
    let mut extraction = NameExtraction::default();
    for name in branches {
        match issue_number_from_branch(name) {
            Some(number) => {
                extraction.matched += 1;
                extraction.matched_numbers.insert(number);
            }
            None => extraction.unmatched_branches.push(name.to_string()),
        }
    }
    extraction
}

/// Inputs to one backlog computation.
#[derive(Debug, Clone, Default)]
pub struct BacklogSnapshot {
    /// Issue numbers currently open.
    pub open_issues: BTreeSet<u64>,
    /// Issue numbers that have a patch on disk awaiting conversion.
    pub patched_issues: BTreeSet<u64>,
    /// Every PR in scope, open or settled.
    pub prs: Vec<PrRecord>,
    /// Known-answer set (#3924 invariant 2 / #3793): issues whose true state
    /// is known to the caller — e.g. merged in the current session. They
    /// must not appear in the outstanding set.
    pub known_merged: BTreeSet<u64>,
    /// Issue numbers claimed by in-flight conversion passes (issue #4214):
    /// a claim under `state/converting/` means a pass is converting the
    /// issue right now and it is not convertible by anyone else, PR or no PR.
    pub in_flight: BTreeSet<u64>,
}

/// The computed backlog, with its reconciliation and audit output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BacklogReport {
    /// Open, patched issues with no open-or-merged PR: the convertible
    /// backlog. Derived from authoritative issue identity only.
    pub outstanding: BTreeSet<u64>,
    /// The zero-control for `outstanding` (issue #4449): `Some` only when
    /// `outstanding` is empty. A proven zero is evidence the backlog is
    /// drained; an unmeasured zero means the measurement never saw a
    /// candidate — the report names that instead of printing a plausible
    /// "convertible 0" a reader cannot tell from a healthy idle pass.
    pub outstanding_zero: Option<Measured<u64>>,
    /// Issue numbers with an open or merged PR, per authoritative fields.
    pub issues_with_pr: BTreeSet<u64>,
    /// Patches on disk at snapshot time.
    pub patches_on_disk: usize,
    /// Open issues at snapshot time.
    pub issues_open: usize,
    /// What the name heuristic would have produced, kept as a cross-check
    /// only; never used for dispatch.
    pub heuristic_outstanding: BTreeSet<u64>,
    /// Claimed at snapshot time: issues a pass is converting right now,
    /// excluded from [`outstanding`](BacklogReport::outstanding) (issue
    /// #4214).
    pub in_flight: BTreeSet<u64>,
    /// Claims that name an issue with no open patch to convert: stale
    /// claims, reported rather than absorbed.
    pub stale_claims: BTreeSet<u64>,
    /// The branch-name audit: matched count and the unmatched names.
    pub branch_audit: NameExtraction,
    /// Tracker/merged-PR mismatches from reconciliation (#4044): open
    /// issues a merged PR delivers, and merged PRs whose issue is open with
    /// no closure decision. An empty list means the tracker and the merged
    /// work agree.
    pub tracker_discrepancies: Vec<TrackerDiscrepancy>,
    /// Open issues after reconciliation (#4044): the tracker count minus
    /// the issues a merged PR has already delivered. This — not
    /// `issues_open` — is the count capacity/throughput metrics may use,
    /// so a tracking lag cannot be misread as a backlog trend.
    pub issues_open_effective: usize,
}

impl BacklogReport {
    /// The number that drives dispatch: convertible patches still owed.
    pub fn convertible(&self) -> usize {
        self.outstanding.len()
    }

    /// One line with every dispatch-relevant count, so the sums can be
    /// checked by eye (#3924 invariant 4), with the open count after
    /// reconciliation (#4044) alongside the raw one:
    /// `backlog: convertible 215 (patches on disk 409, issues open 372, open after reconcile 372, issues with a PR 194, in flight 3)`.
    pub fn summary_line(&self) -> String {
        let mut line = format!(
            "backlog: convertible {} (patches on disk {}, issues open {}, open after reconcile {}, issues with a PR {}, in flight {})",
            self.convertible(),
            self.patches_on_disk,
            self.issues_open,
            self.issues_open_effective,
            self.issues_with_pr.len(),
            self.in_flight.len()
        );
        // An unmeasured zero must not pass for a drained backlog on the
        // line the conversion loop logs (issue #4449).
        if let Some(zero) = &self.outstanding_zero {
            if zero.is_unmeasured() {
                line.push_str(" — ");
                line.push_str(&zero.line());
            }
        }
        line
    }

    /// Every way this report's derivations disagree, reported rather than
    /// absorbed (#3924 invariant 4), plus the name heuristic's non-match
    /// count (invariant 3). An empty vector means nothing drifted.
    pub fn discrepancies(&self) -> Vec<String> {
        let mut found = Vec::new();
        if !self.branch_audit.unmatched_branches.is_empty() {
            found.push(format!(
                "{} branch name(s) produced no issue number: {}",
                self.branch_audit.unmatched_count(),
                self.branch_audit.unmatched_branches.join(", ")
            ));
        }
        if self.outstanding != self.heuristic_outstanding {
            let heuristic_only: Vec<String> = self
                .heuristic_outstanding
                .difference(&self.outstanding)
                .map(|n| n.to_string())
                .collect();
            let authoritative_only: Vec<String> = self
                .outstanding
                .difference(&self.heuristic_outstanding)
                .map(|n| n.to_string())
                .collect();
            found.push(format!(
                "name heuristic disagrees with authoritative identity: heuristic {} vs authoritative {} outstanding (heuristic-only: [{}], authoritative-only: [{}])",
                self.heuristic_outstanding.len(),
                self.outstanding.len(),
                heuristic_only.join(", "),
                authoritative_only.join(", ")
            ));
        }
        // Tracker/merged-PR mismatches are reported, not absorbed (#4044).
        found.extend(
            self.tracker_discrepancies
                .iter()
                .map(TrackerDiscrepancy::line),
        );
        // Stale conversion claims are reported, not absorbed (#4214):
        // a claim with no open patch to convert names a pass that is gone
        // or a patch that moved, and the queue must see that.
        if !self.stale_claims.is_empty() {
            found.push(format!(
                "{} conversion claim(s) name issue(s) with no open patch to convert (stale claims): {}",
                self.stale_claims.len(),
                self.stale_claims
                    .iter()
                    .map(|n| n.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        // An unmeasured zero backlog is reported, not absorbed (issue
        // #4449): "convertible 0" on an empty input is not a measurement.
        if let Some(zero) = &self.outstanding_zero {
            if zero.is_unmeasured() {
                found.push(zero.line());
            }
        }
        found
    }
}

/// The known-answer assertion (#3924 invariant 2): issues the caller *knows*
/// are merged must not appear in an outstanding set. Fails loudly — with
/// every offending issue named — because a metric contradicting a known
/// fact must stop the pipeline, not print a plausible number.
///
/// `compute_backlog` applies this to its authoritative result; run it
/// against any other derivation (a new extraction, a cached set) before
/// trusting that derivation (#3793).
pub fn known_answer_check(
    outstanding: &BTreeSet<u64>,
    known_merged: &BTreeSet<u64>,
) -> Result<(), String> {
    let violations: Vec<String> = known_merged
        .intersection(outstanding)
        .map(|n| n.to_string())
        .collect();
    if violations.is_empty() {
        return Ok(());
    }
    Err(format!(
        "known-answer check failed: issue(s) {} are known merged but appear in the outstanding backlog [{}]; identity was not taken from an authoritative field (#3924)",
        violations.join(", "),
        violations.join(", ")
    ))
}

/// Compute the conversion backlog from authoritative issue identity.
///
/// An issue is outstanding when it is open, has a patch on disk, has no
/// open or merged PR associated by [`PrRecord::authoritative_issue`], and is
/// not claimed by an in-flight conversion pass ([`BacklogSnapshot::in_flight`],
/// issue #4214 — the claim is the "is anyone working on this?" check and it
/// outranks the PR check, because a patch held by a running pass has no PR
/// yet). Closed-unmerged PRs do not associate: the work did not land, so the
/// issue stays on the list.
///
/// The branch-name heuristic runs alongside — as an audit and a
/// cross-check, never as the source of truth — and the authoritative
/// result is gated by [`known_answer_check`] before it is returned: a
/// snapshot in which a known-merged issue looks outstanding is an error,
/// not a number.
pub fn compute_backlog(snapshot: &BacklogSnapshot) -> Result<BacklogReport, String> {
    let issues_with_pr: BTreeSet<u64> = snapshot
        .prs
        .iter()
        .filter(|pr| pr.state != PrState::Closed)
        .filter_map(|pr| pr.authoritative_issue())
        .collect();
    // The candidate set goes through the positive-controlled set operation
    // (issue #4449): when it comes back empty, the verdict says which input
    // was empty — and that is what the zero report below carries.
    let open_patched_measured = false_negative::set_intersection(
        "open issues",
        snapshot.open_issues.iter().copied(),
        "patches on disk",
        snapshot.patched_issues.iter().copied(),
    );
    let open_patched = open_patched_measured.as_set().cloned().unwrap_or_default();
    // In-flight (issue #4214): a claim means a pass is converting the issue
    // right now — open, patched, no PR yet. A claim that a PR already
    // covers is subsumed by that PR (the pass finished; the release may lag
    // it) and is reported neither way. A claim naming an issue with no open
    // patch to convert is stale and is reported, never absorbed.
    let claimed_and_patched: BTreeSet<u64> = snapshot
        .in_flight
        .intersection(&open_patched)
        .copied()
        .collect();
    let in_flight: BTreeSet<u64> = claimed_and_patched
        .difference(&issues_with_pr)
        .copied()
        .collect();
    let stale_claims: BTreeSet<u64> = snapshot
        .in_flight
        .difference(&open_patched)
        .copied()
        .collect();
    let prless_measured = false_negative::set_difference(
        "open patched issues",
        open_patched.iter().copied(),
        "issues with a PR",
        issues_with_pr.iter().copied(),
    );
    let prless = prless_measured.as_set().cloned().unwrap_or_default();
    let outstanding_measured = false_negative::set_difference(
        "convertible candidates",
        prless.iter().copied(),
        "in-flight conversion claims",
        in_flight.iter().copied(),
    );
    let outstanding = outstanding_measured.as_set().cloned().unwrap_or_default();
    known_answer_check(&outstanding, &snapshot.known_merged)?;

    // The audit covers every branch name, whatever the PR state: it is a
    // statement about the names, while association above is a statement
    // about delivered work.
    let branch_audit =
        extract_issue_numbers_from_branches(snapshot.prs.iter().map(|pr| pr.head_branch.as_str()));
    let heuristic_outstanding: BTreeSet<u64> = open_patched
        .difference(branch_audit.matched_numbers())
        .copied()
        .collect();

    let tracker_discrepancies = reconcile_tracker(&snapshot.open_issues, &snapshot.prs);
    // The zero-control (issue #4449): a zero backlog is evidence only when
    // the measurement saw candidates to compare. Built on an empty candidate
    // set, it was never measured — the summary line and the discrepancies
    // say so instead of printing a "convertible 0" that reads like a
    // drained backlog.
    let outstanding_zero = if outstanding.is_empty() {
        Some(match &open_patched_measured {
            Measured::NonEmpty(candidates) => Measured::proven_empty(format!(
                "{} open issue(s) had a patch on disk and each was compared against the PR and in-flight sets",
                candidates.len()
            )),
            Measured::ProvenEmpty { control } => Measured::unmeasured(format!(
                "no open issue had a patch on disk — {control} — so 'convertible 0' was never measured against a candidate"
            )),
            Measured::Unmeasured { why } => Measured::unmeasured(format!(
                "no open issue had a patch on disk — {why}"
            )),
        })
    } else {
        None
    };
    Ok(BacklogReport {
        outstanding,
        outstanding_zero,
        issues_with_pr,
        in_flight,
        stale_claims,
        patches_on_disk: snapshot.patched_issues.len(),
        issues_open: snapshot.open_issues.len(),
        issues_open_effective: reconciled_open_count(&snapshot.open_issues, &tracker_discrepancies),
        heuristic_outstanding,
        branch_audit,
        tracker_discrepancies,
    })
}

#[cfg(test)]
#[path = "backlog_tests.rs"]
mod tests;
