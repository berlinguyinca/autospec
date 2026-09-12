//! Rebase before review: a stale branch's diff describes the base's
//! movement, not the change (issue #3688).
//!
//! Two pull requests, reviewed the same way on the same day, both filed
//! against an older `main`:
//!
//! | PR | diff against main, as filed | after rebasing onto current main |
//! |---|---|---|
//! | a clippy fix | `7 files changed, 1,522 deletions` | **1 line** |
//! | a capacity feature | `60 files changed, 9,349 deletions` | conflicts in four core files |
//!
//! Neither number described the change. Both described how far the base had
//! moved — 2 commits in the first case, 40 in the second. The second is the
//! dangerous one: merged as filed it would have reverted a large amount of
//! live work, and **every gate would have stayed green**, because a branch is
//! internally consistent with the past it was cut from. The gates answer
//! "is this branch sound?", never "is this branch still about today's code?"
//!
//! The drift measurement, the re-baseline plan, and the stale-but-clean /
//! subject-matter-conflict classification already live in
//! [`crate::rebaseline`]. This module adds the two halves that live on the
//! review and the verdict side:
//!
//! 1. **No review is conducted against a diff whose base is behind.**
//!    [`review_standing`] voids a review performed against a moved base, and
//!    a review record renders the base it was performed against
//!    ([`ReviewRecord::line`], `[base=<sha>]`); a record that does not name
//!    its base is not a review anyone can grade ([`record_names_base`]).
//! 2. **A staleness class carries its action, and hand-resolution is not one
//!    of them.** [`StalenessAction::from_staleness`] maps a
//!    [`rebaseline::Staleness`] to the only action the class permits: current
//!    → verify and merge; stale-but-clean → rebase, re-run the **full gate
//!    set on the rebased tree** (a green result on the pre-rebase branch is
//!    evidence about a tree nobody will merge), then merge; conflicts in the
//!    change's own subject matter → hold with the conflicting paths recorded
//!    and re-dispatch the issue against current `main`. Hand-resolving the
//!    second class means re-deciding the design in a merge editor, with no
//!    spec, no tests for the new decisions, and no review — it is a
//!    re-dispatch, not a merge, and [`StalenessAction::allows_hand_resolution`]
//!    exists so that stays true mechanically.
//! 3. **Every conversion verdict reports its base drift.** A verdict line
//!    that carries the test result but not "N commits behind" is a stale
//!    verdict with its staleness invisible: [`ConversionVerdict::line`]
//!    renders the drift on the same line as the result, and
//!    [`verdict_reports_drift`] is the check a verdict line must pass.
//!
//! Everything here is pure: no I/O, no git, no clock. The caller measures
//! the base against the tip (the count the rebase or apply reported) and
//! acts on the verdict.

use crate::rebaseline::{classify_staleness, BaseDrift, MeasureError, Staleness};

// ---------------------------------------------------------------------------
// Invariant 1 — no review against a diff whose base is behind
// ---------------------------------------------------------------------------

/// Whether a review stands as recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewStanding {
    /// The base is the trunk tip the review was performed against: the diff
    /// describes the change.
    Valid,
    /// The base is behind: the diff describes the base's movement. The
    /// review is void — rebase, then re-review the rebased diff.
    StaleBase { commits_behind: u64 },
}

impl ReviewStanding {
    pub fn valid(&self) -> bool {
        matches!(self, Self::Valid)
    }

    /// The line a review log carries next to the verdict.
    pub fn line(&self) -> String {
        match self {
            Self::Valid => "review stands: base is the trunk tip".to_string(),
            Self::StaleBase { commits_behind } => format!(
                "review void: base is {commits_behind} commit(s) behind — rebase before reading the diff"
            ),
        }
    }
}

/// Grades a review by the base it was performed against.
///
/// The count is cross-checked against the two ids by
/// [`BaseDrift::measure`]: a review whose drift was assumed rather than
/// measured is refused, not graded.
pub fn review_standing(
    base_sha: &str,
    tip_sha: &str,
    commits_behind: u64,
) -> Result<ReviewStanding, MeasureError> {
    let drift = BaseDrift::measure(base_sha, tip_sha, commits_behind)?;
    if drift.at_tip() {
        Ok(ReviewStanding::Valid)
    } else {
        Ok(ReviewStanding::StaleBase {
            commits_behind: drift.commits_behind,
        })
    }
}

/// A review as recorded: which pull request, performed against which base,
/// when the trunk was where.
///
/// The base is the field that makes the rest of the record cheap: with it,
/// the next reader can grade the review without re-running it. A review
/// whose base is unknown is a review whose staleness nobody can compute —
/// the failure mode in #3688 was reading the grade without the base.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewRecord {
    /// The pull request the review was performed on.
    pub pr: u64,
    /// The base the diff was reviewed against.
    pub base_sha: String,
    /// The trunk tip at review time.
    pub tip_sha: String,
    /// How far `base_sha` sat behind `tip_sha`.
    pub commits_behind: u64,
}

impl ReviewRecord {
    /// Builds the record and cross-checks its drift at write time, so a
    /// record that is internally inconsistent is refused here rather than
    /// discovered by the next reader.
    pub fn new(
        pr: u64,
        base_sha: &str,
        tip_sha: &str,
        commits_behind: u64,
    ) -> Result<Self, MeasureError> {
        let drift = BaseDrift::measure(base_sha, tip_sha, commits_behind)?;
        if pr == 0 {
            return Err("pr id is zero: a review must name the pull request it reviews");
        }
        Ok(Self {
            pr,
            base_sha: drift.base_sha,
            tip_sha: drift.tip_sha,
            commits_behind: drift.commits_behind,
        })
    }

    /// Whether this review stands as recorded (invariant 1).
    pub fn standing(&self) -> ReviewStanding {
        review_standing(&self.base_sha, &self.tip_sha, self.commits_behind)
            .expect("cross-checked in `new`")
    }

    /// The line a review log must carry: `[base=<sha>]` and the standing on
    /// the same line, so a stale review is legible instead of something the
    /// next reader has to rediscover.
    pub fn line(&self) -> String {
        format!(
            "review PR {} [base={}] {}",
            self.pr,
            self.base_sha,
            self.standing().line()
        )
    }
}

/// The mechanical check: a review record names the base it was performed
/// against. Absence within a record is not absence of the gap — a review
/// line with no `[base=<sha>]` cannot be graded for staleness by anyone.
pub fn record_names_base(record: &str) -> bool {
    record
        .split("[base=")
        .nth(1)
        .is_some_and(|rest| rest.split(']').next().is_some_and(|s| !s.trim().is_empty()))
}

// ---------------------------------------------------------------------------
// Invariant 2 — a staleness class carries its action
// ---------------------------------------------------------------------------

/// The action a staleness class permits.
///
/// There is deliberately no hand-resolution variant: resolving a
/// subject-matter conflict by hand means re-deciding the design in a merge
/// editor, with no spec, no tests for the new decisions, and no review. That
/// is a re-dispatch, not a merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StalenessAction {
    /// The base is current: the review stands as filed. Verify and merge.
    VerifyAndMerge,
    /// Stale but clean: rebase onto the trunk tip, re-run the full gate set
    /// on the rebased tree, then merge. A green result on the pre-rebase
    /// branch is evidence about a tree nobody will merge.
    RebaseVerifyMerge {
        /// The trunk tip the rebase lands on.
        onto: String,
    },
    /// Conflicts in the change's own subject matter: hold with the
    /// conflicting paths recorded and re-dispatch the issue against current
    /// `main`.
    HoldAndRedispatch {
        /// The conflicting paths, as recorded on the hold.
        files: Vec<String>,
    },
}

impl StalenessAction {
    /// Maps a staleness classification to the only action the class permits.
    pub fn from_staleness(staleness: &Staleness, tip_sha: &str) -> Self {
        match staleness {
            Staleness::Current => StalenessAction::VerifyAndMerge,
            Staleness::StaleButClean => StalenessAction::RebaseVerifyMerge {
                onto: tip_sha.to_string(),
            },
            Staleness::SubjectMatterConflict { files } => StalenessAction::HoldAndRedispatch {
                files: files.clone(),
            },
        }
    }

    /// Hand-resolution is never a permitted action. A hold releases by
    /// re-dispatch against current `main`, not by merge-editor surgery.
    pub fn allows_hand_resolution(&self) -> bool {
        false
    }

    /// Whether the branch may merge on this action.
    pub fn mergeable(&self) -> bool {
        !matches!(self, Self::HoldAndRedispatch { .. })
    }

    /// The one line the hold or the merge record carries.
    pub fn line(&self) -> String {
        match self {
            Self::VerifyAndMerge => {
                "verify and merge: base is current; the diff describes the change".to_string()
            }
            Self::RebaseVerifyMerge { onto } => format!(
                "rebase onto {onto}, re-run the full gate set on the rebased tree, then merge: stale but clean"
            ),
            Self::HoldAndRedispatch { files } => format!(
                "hold: conflicts in the change's own subject matter ({}) — re-dispatch the issue against current main; never hand-resolve",
                files.join(", ")
            ),
        }
    }
}

/// The whole of the decision for a stale branch: the drift it was measured
/// against and the action that drift permits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StalenessVerdict {
    /// The measured base drift.
    pub drift: BaseDrift,
    /// The action the staleness class permits.
    pub action: StalenessAction,
}

impl StalenessVerdict {
    /// The line a conversion log must carry: the drift and the action on the
    /// same line.
    pub fn line(&self) -> String {
        format!("{}; {}", drift_line(&self.drift), self.action.line())
    }
}

/// The end-to-end decision for a stale branch (invariant 2): measure the
/// drift, classify the conflict, and name the action the class permits.
///
/// `conflicting_files` are the files the rebase (or apply) reported; an
/// empty list means the branch rebases cleanly. `touched_files` are the
/// files the branch itself changes.
pub fn decide(
    base_sha: &str,
    tip_sha: &str,
    commits_behind: u64,
    conflicting_files: &[String],
    touched_files: &[String],
) -> Result<StalenessVerdict, MeasureError> {
    let drift = BaseDrift::measure(base_sha, tip_sha, commits_behind)?;
    let staleness = classify_staleness(&drift, conflicting_files, touched_files);
    let action = StalenessAction::from_staleness(&staleness, tip_sha);
    Ok(StalenessVerdict { drift, action })
}

/// How a base drift renders on a verdict or review line.
///
/// Always names the base and the count, because "0 behind" is a measurement
/// like any other and the `[base=<sha>]` token is what
/// [`record_names_base`] and [`verdict_reports_drift`] check for.
pub fn drift_line(drift: &BaseDrift) -> String {
    if drift.at_tip() {
        format!(
            "[base={}] is the trunk tip (0 commit(s) behind main)",
            drift.base_sha
        )
    } else {
        format!(
            "[base={}] is {} commit(s) behind main",
            drift.base_sha, drift.commits_behind
        )
    }
}

// ---------------------------------------------------------------------------
// Invariant 3 — every conversion verdict reports its base drift
// ---------------------------------------------------------------------------

/// The test result a conversion verdict reports alongside its drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TestResult {
    pub passed: u32,
    pub failed: u32,
}

impl TestResult {
    pub fn new(passed: u32, failed: u32) -> Self {
        Self { passed, failed }
    }

    pub fn line(&self) -> String {
        format!("{} passed, {} failing", self.passed, self.failed)
    }
}

/// A conversion verdict: the test result and the base the branch was filed
/// against, measured against the trunk tip at conversion time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversionVerdict {
    /// The test result the gates produced.
    pub tests: TestResult,
    /// The base drift at conversion time.
    pub base: BaseDrift,
}

impl ConversionVerdict {
    pub fn new(tests: TestResult, base: BaseDrift) -> Self {
        Self { tests, base }
    }

    /// The line a conversion log must carry (invariant 3): the test result
    /// and the drift on the same line, so a stale verdict is legible
    /// instead of something the next reader has to rediscover.
    pub fn line(&self) -> String {
        format!("{} — {}", self.tests.line(), drift_line(&self.base))
    }
}

/// The check a verdict line must pass: it reports its base drift.
///
/// A line that carries the test result but not the base and the behind-count
/// is a green verdict that could belong to any base — the state that let a
/// branch 40 commits behind read as a healthy one.
pub fn verdict_reports_drift(line: &str) -> bool {
    record_names_base(line) && line.contains("commit(s) behind")
}
