//! Memoized decisions are keyed on the input, not on the subject
//! (issue #4260).
//!
//! The conversion/selector loop kept a set of issues whose patches had
//! "already been attempted" and used it to exclude future candidates.
//! The set was keyed on the *issue identifier*. When an agent was killed
//! and redispatched, it produced a brand-new patch for the same issue —
//! different content, different mtime — and the filter saw "already
//! attempted" and excluded the fresh work forever. The selector's
//! denominator made the defect look healthy:
//!
//! ```text
//! considered=427 finished_patches=427 have_pr=314 closed_issue=265
//!   attempted=236 -> candidates=0
//! ```
//!
//! Every number checked out: the pass ran, the filters ran, and the
//! pipeline was stuck. The denominator proves the filter ran; it does
//! not prove the filter is correct.
//!
//! Three invariants, each a primitive here:
//!
//! 1. **A memoized decision is keyed on the input, not the subject.**
//!    An exclusion only holds while the input it was recorded against is
//!    still the input on disk ([`input_keyed_excluded`]). A memo entry
//!    that never matched the current artifact must not be used to skip
//!    it. The subject-keyed filter is kept as a named reference for the
//!    failure mode ([`subject_keyed_excluded`]).
//! 2. **Record *what* was attempted alongside the fact that it was
//!    attempted.** [`AttemptRecord`] carries the input key — content
//!    hash or mtime stamp — with the attempt. A record without a key is
//!    legacy and never excludes: re-attempting a patch is cheap,
//!    losing a fresh one is not.
//! 3. **Recency overrides history (the cheap version).** A patch whose
//!    mtime is within the fresh window is a candidate even when its key
//!    matches a recorded attempt ([`is_fresh`], [`select_candidates`]).
//!    The override is visible in the report as its own dimension
//!    (`fresh=N`), as is the set the old filter would have wrongly
//!    excluded (`stale=N`).
//!
//! And the reconciliation that would have caught this: per-stage
//! denominators prove each filter ran, not that the work reached its
//! destination. [`reconcile`] asserts the end-to-end invariant — work
//! completed within the window must appear either as a PR or as a held
//! line — and names what did not arrive.
//!
//! Everything here is pure: no I/O, no clock, no subprocesses. The
//! caller supplies "now" as a unix timestamp in seconds and the
//! artifact's key and mtime.

use serde::{Deserialize, Serialize};

/// Default recency window, seconds: a patch whose mtime is within this
/// window of now is a candidate even when its key matches a recorded
/// attempt.
pub const DEFAULT_FRESH_MIN: u64 = 10 * 60;

/// Default reconciliation window, seconds: work completed within this
/// window of now must appear either as a PR or as a held line.
pub const DEFAULT_RECONCILE_WINDOW: u64 = 60 * 60;

/// A recorded attempt: the fact that the subject was attempted, and —
/// the half the old filter dropped — *what* was attempted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttemptRecord {
    /// The subject identifier (issue number) the attempt was against.
    pub subject: u64,
    /// The input the attempt was recorded against — the artifact's
    /// content hash or mtime stamp. `None` for legacy records written
    /// before the key was captured; a legacy record never excludes.
    pub input_key: Option<String>,
    /// Unix timestamp (seconds) when the attempt was recorded.
    pub attempted_at: u64,
}

impl AttemptRecord {
    /// An attempt recorded against a known input.
    pub fn new(subject: u64, input_key: impl Into<String>, attempted_at: u64) -> Self {
        Self {
            subject,
            input_key: Some(input_key.into()),
            attempted_at,
        }
    }

    /// A legacy attempt with no captured input: the subject was attempted
    /// at `attempted_at`, but against what is not on record.
    pub fn legacy(subject: u64, attempted_at: u64) -> Self {
        Self {
            subject,
            input_key: None,
            attempted_at,
        }
    }

    /// Whether this record was made against the given input.
    pub fn key_matches(&self, current: &str) -> bool {
        self.input_key.as_deref() == Some(current)
    }
}

/// The failure mode, kept as a named reference: exclude the subject if it
/// was *ever* attempted, whatever the input on disk is now. A redispatch
/// that produced a brand-new patch is excluded forever by this filter.
pub fn subject_keyed_excluded(records: &[AttemptRecord], subject: u64) -> bool {
    records.iter().any(|r| r.subject == subject)
}

/// The invariant: exclude the subject only if a recorded attempt matches
/// the *current* input. A record for the subject whose key differs — or
/// that carries no key — is a memo entry about a different artifact and
/// must not skip this one.
pub fn input_keyed_excluded(records: &[AttemptRecord], subject: u64, current_key: &str) -> bool {
    records
        .iter()
        .any(|r| r.subject == subject && r.key_matches(current_key))
}

/// Whether an artifact is fresh: its mtime is within `window` of `now`.
/// An mtime in the future (clock skew between the producing host and the
/// selector) is fresh, not an error — failing open here costs one
/// re-attempt, failing closed costs the patch.
pub fn is_fresh(mtime: u64, now: u64, window: u64) -> bool {
    mtime >= now.saturating_sub(window)
}

/// A finished patch the selector is deciding over.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FinishedPatch {
    /// The issue identifier the patch was produced for.
    pub issue: u64,
    /// The artifact's input key: content hash or mtime stamp.
    pub input_key: String,
    /// The artifact's mtime, unix timestamp in seconds.
    pub mtime: u64,
    /// The issue already has a pull request: terminal, never a candidate.
    pub has_pr: bool,
    /// The issue is closed: terminal, never a candidate.
    pub issue_closed: bool,
}

/// Why the selector decided as it did for one patch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Decision {
    /// The issue already has a PR.
    HavePr,
    /// The issue is closed.
    ClosedIssue,
    /// A recorded attempt matches the current input and the artifact is
    /// not fresh: the memo decision holds.
    Attempted,
    /// A recorded attempt exists for the issue but none matches the
    /// current input: the subject-keyed filter would have excluded this
    /// patch, the input-keyed filter admits it.
    Stale,
    /// A recorded attempt matches the current input but the artifact is
    /// fresh: recency overrides history.
    FreshOverride,
    /// No recorded attempt for the issue.
    Unattempted,
}

impl Decision {
    /// Whether this decision admits the patch as a candidate.
    pub fn is_candidate(self) -> bool {
        matches!(self, Self::Stale | Self::FreshOverride | Self::Unattempted)
    }
}

/// The selector's report. The buckets are disjoint and reconcile:
/// `considered == finished_patches - have_pr - closed_issue` and
/// `candidates == considered - attempted == stale + fresh + unattempted`.
/// A report that does not reconcile is reporting a state that cannot
/// exist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectionReport {
    /// Finished patches seen on disk.
    pub finished_patches: usize,
    /// Patches that reached the attempted filter: `finished_patches`
    /// minus the terminal buckets.
    pub considered: usize,
    /// Patches whose issue already has a PR.
    pub have_pr: usize,
    /// Patches whose issue is closed.
    pub closed_issue: usize,
    /// Patches excluded because a recorded attempt matches the current
    /// input and the artifact is not fresh.
    pub attempted: usize,
    /// Patches admitted although the subject has a recorded attempt,
    /// because no attempt matches the current input — the set the
    /// subject-keyed filter would have wrongly excluded.
    pub stale: usize,
    /// Patches admitted by recency override: a matching attempt, but the
    /// artifact is fresh.
    pub fresh: usize,
    /// Patches with no recorded attempt.
    pub unattempted: usize,
    /// Patches admitted: `stale + fresh + unattempted`.
    pub candidates: usize,
}

impl SelectionReport {
    /// The selector's log line, with the `stale` and `fresh` dimensions
    /// the old line did not have: `stale=N` is the blast radius of the
    /// subject-keyed bug made visible, `fresh=N` is the recency override
    /// in action.
    pub fn line(&self) -> String {
        format!(
            "finished_patches={} considered={} have_pr={} closed_issue={} \
             attempted={} stale={} fresh={} unattempted={} -> candidates={}",
            self.finished_patches,
            self.considered,
            self.have_pr,
            self.closed_issue,
            self.attempted,
            self.stale,
            self.fresh,
            self.unattempted,
            self.candidates
        )
    }

    /// Whether the report's own numbers reconcile. A selector whose
    /// report does not reconcile is hiding a patch in the difference.
    pub fn reconciles(&self) -> bool {
        self.considered == self.finished_patches - self.have_pr - self.closed_issue
            && self.candidates == self.considered - self.attempted
            && self.candidates == self.stale + self.fresh + self.unattempted
    }
}

/// The selector's outcome: which patches are candidates and the report
/// that says where everything else went.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Selection {
    /// The issue numbers admitted as candidates, in input order.
    pub candidates: Vec<u64>,
    /// The per-patch decisions, in input order.
    pub decisions: Vec<Decision>,
    /// The report.
    pub report: SelectionReport,
}

/// Select candidates from finished patches, keyed on the input, with the
/// recency override. The attempted filter excludes a patch only when a
/// recorded attempt matches its *current* input; an artifact whose mtime
/// is within `fresh_window` is a candidate even then.
pub fn select_candidates(
    patches: &[FinishedPatch],
    records: &[AttemptRecord],
    now: u64,
    fresh_window: u64,
) -> Selection {
    let mut report = SelectionReport {
        finished_patches: patches.len(),
        considered: 0,
        have_pr: 0,
        closed_issue: 0,
        attempted: 0,
        stale: 0,
        fresh: 0,
        unattempted: 0,
        candidates: 0,
    };
    let mut candidates = Vec::new();
    let mut decisions = Vec::new();
    for patch in patches {
        let decision = if patch.has_pr {
            report.have_pr += 1;
            Decision::HavePr
        } else if patch.issue_closed {
            report.closed_issue += 1;
            Decision::ClosedIssue
        } else {
            report.considered += 1;
            let matching = records
                .iter()
                .any(|r| r.subject == patch.issue && r.key_matches(&patch.input_key));
            if matching {
                if is_fresh(patch.mtime, now, fresh_window) {
                    report.fresh += 1;
                    Decision::FreshOverride
                } else {
                    report.attempted += 1;
                    Decision::Attempted
                }
            } else if records.iter().any(|r| r.subject == patch.issue) {
                report.stale += 1;
                Decision::Stale
            } else {
                report.unattempted += 1;
                Decision::Unattempted
            }
        };
        if decision.is_candidate() {
            report.candidates += 1;
            candidates.push(patch.issue);
        }
        decisions.push(decision);
    }
    Selection {
        candidates,
        decisions,
        report,
    }
}

/// Work an agent finished: the subject, the artifact it produced, and
/// when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompletedWork {
    pub issue: u64,
    pub input_key: String,
    /// Unix timestamp (seconds) when the work completed.
    pub completed_at: u64,
}

/// A pull request that exists for a subject.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrOutcome {
    pub issue: u64,
    pub pr: u64,
}

/// A held line: the subject is not a candidate, and the reason is on
/// record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeldLine {
    pub issue: u64,
    pub reason: String,
}

/// Work that completed within the window and reached neither
/// destination: no PR, no held line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissingWork {
    pub issue: u64,
    pub input_key: String,
    pub completed_at: u64,
    /// Seconds since completion; a future timestamp (clock skew) is age 0.
    pub age: u64,
}

/// The end-to-end reconciliation: per-stage counters prove each filter
/// ran; this asserts the work actually reached its destination. Work
/// completed within the window must appear either as a PR or as a held
/// line — a PR takes precedence when both exist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReconcileReport {
    /// The reconciliation window, seconds.
    pub window: u64,
    /// Work completed within the window.
    pub completed: usize,
    /// Of that, appearing as a PR.
    pub as_pr: usize,
    /// Of that, with no PR but a held line.
    pub as_held: usize,
    /// Of that, appearing as neither: the defect, named.
    pub missing: Vec<MissingWork>,
    /// Work older than the window: outside this check.
    pub outside_window: usize,
}

impl ReconcileReport {
    /// The reconciliation line. A missing entry names the issue and how
    /// long it has been nowhere.
    pub fn line(&self) -> String {
        let base = format!(
            "reconciled window={}s completed={} pr={} held={} missing={}",
            self.window,
            self.completed,
            self.as_pr,
            self.as_held,
            self.missing.len()
        );
        if self.missing.is_empty() {
            base
        } else {
            let named = self
                .missing
                .iter()
                .map(|m| format!("#{issue} age={age}s", issue = m.issue, age = m.age))
                .collect::<Vec<_>>()
                .join(" ");
            format!("{base} [{named}]")
        }
    }

    /// Whether the report's own numbers reconcile: every completed work
    /// is a PR, a held line, or named missing.
    pub fn reconciles(&self) -> bool {
        self.completed == self.as_pr + self.as_held + self.missing.len()
    }
}

/// Assert the end-to-end invariant: work completed within `window` of
/// `now` appears either as a PR or as a held line. A PR and a held line
/// for the same issue count as a PR. Work older than the window (or
/// stamped in the future) is outside / age-0 respectively, never an
/// error.
pub fn reconcile(
    completed: &[CompletedWork],
    prs: &[PrOutcome],
    held: &[HeldLine],
    now: u64,
    window: u64,
) -> ReconcileReport {
    let horizon = now.saturating_sub(window);
    let mut report = ReconcileReport {
        window,
        completed: 0,
        as_pr: 0,
        as_held: 0,
        missing: Vec::new(),
        outside_window: 0,
    };
    for work in completed {
        if work.completed_at < horizon {
            report.outside_window += 1;
            continue;
        }
        report.completed += 1;
        if prs.iter().any(|p| p.issue == work.issue) {
            report.as_pr += 1;
        } else if held.iter().any(|h| h.issue == work.issue) {
            report.as_held += 1;
        } else {
            report.missing.push(MissingWork {
                issue: work.issue,
                input_key: work.input_key.clone(),
                completed_at: work.completed_at,
                age: now.saturating_sub(work.completed_at),
            });
        }
    }
    report
}
