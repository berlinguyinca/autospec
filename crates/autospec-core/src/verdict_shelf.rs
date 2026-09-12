//! A verdict has a shelf life (issue #3673).
//!
//! One conversion pass, measured:
//!
//! | | |
//! |---|---|
//! | duration | **158 min** for 89 patches (1.8 min/patch) |
//! | commits to `main` during the pass | **49** |
//! | verdicts issued **before** the trunk moved | **80** |
//! | of those, `HELD` | **8** |
//!
//! The pass reports a single line — `converted=N held=N skipped=N` — as
//! though it described one state of the world. It does not. Eight of those
//! holds were decided against a trunk that has since gained 49 commits, and
//! a patch held on a build error against the old trunk may build cleanly
//! against the new one, or vice versa.
//!
//! Two mitigations already exist and worked, and this module does not
//! replace them: the hold memo is keyed on `(patch hash, base sha)`, so a
//! stale hold is *retested* on the next pass rather than replayed, and a
//! batch merge is followed by verifying the trunk itself. What is wrong is
//! the **report**: a reader of `held=15` reasonably believes those are
//! fifteen current facts about the current trunk. They are a mixture of
//! verdicts against at least two different trunks.
//!
//! Three invariants, each a primitive here (the fourth — a pass short
//! relative to the trunk's change rate — is throughput, #3635, and this
//! module does not fix it):
//!
//! 1. **Every verdict records the base sha it was decided against**, and
//!    the summary reports how many are against the *current* trunk versus
//!    stale: `held=15 (7 current, 8 against an older base, will be
//!    retested)`. A verdict with no recorded base is a defect, never a
//!    missing fact to default ([`Verdict::new`] returns `None`).
//! 2. **A pass that outlives the trunk's change interval says so**, rather
//!    than presenting an average over a moving target ([`pass_span`],
//!    [`PassSpan::warn_line`]).
//! 3. **Where a pass is long, cheap decisions are re-checked at the end**
//!    rather than trusted from the start: "PR already exists" and "issue
//!    closed" are seconds to recompute and are the most likely to have
//!    changed ([`due_rechecks`], [`recheck_line`]). Expensive verdicts
//!    (builds, gates) are not re-run mid-pass — the `(patch hash, base
//!    sha)` memo key re-tests them on the next pass automatically, and it
//!    is re-running them that made the pass 158 minutes.
//!
//! Everything here is pure: no I/O, no clock, no subprocesses. The caller
//! supplies the trunk tips, the commit count, and the durations.

use serde::{Deserialize, Serialize};

/// The kind of verdict a conversion pass issues for one patch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VerdictKind {
    /// The patch converted to a pull request: the verdict is a fact about
    /// the past and terminal — it is not retested, it is reported.
    Converted,
    /// The pass held the patch for a recorded reason: non-terminal, and
    /// due for retest on the next pass once the base has moved (the hold
    /// memo is keyed on `(patch hash, base sha)` and stops matching).
    Held,
    /// The pass skipped the patch (no finished patch, no-op):
    /// re-evaluated on the next pass.
    Skipped,
}

impl VerdictKind {
    /// The label the summary line carries.
    pub fn label(self) -> &'static str {
        match self {
            Self::Converted => "converted",
            Self::Held => "held",
            Self::Skipped => "skipped",
        }
    }
}

/// One pass verdict, bound to the trunk tip it was decided against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Verdict {
    /// The issue the verdict was about.
    pub issue: u64,
    /// What the pass decided.
    pub kind: VerdictKind,
    /// The trunk tip the verdict was decided against. Non-empty — enforced
    /// at construction: a verdict decided against no recorded base is
    /// exactly the defect this module exists to make visible.
    pub base_sha: String,
    /// Whether the decision is cheap to recompute in seconds ("PR already
    /// exists", "issue closed"). Expensive verdicts (builds, gates) are
    /// `false`: they are retested on the next pass via the memo key, not
    /// re-run at the end of this one.
    pub cheap: bool,
}

impl Verdict {
    /// Construct a verdict.
    ///
    /// `None` when the base sha is empty or whitespace: a summary without
    /// an expiry is the defect (#3673), so a verdict that cannot say which
    /// trunk it was decided against is refused, never defaulted.
    pub fn new(
        issue: u64,
        kind: VerdictKind,
        base_sha: impl Into<String>,
        cheap: bool,
    ) -> Option<Self> {
        let base_sha = base_sha.into();
        if base_sha.trim().is_empty() {
            return None;
        }
        Some(Self {
            issue,
            kind,
            base_sha,
            cheap,
        })
    }

    /// Whether this verdict is against the given trunk tip.
    pub fn is_current(&self, current_tip: &str) -> bool {
        self.base_sha == current_tip
    }
}

/// The split of one verdict bucket against a trunk tip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BucketShelf {
    /// All verdicts in the bucket.
    pub total: usize,
    /// Verdicts decided against the given (current) tip.
    pub current: usize,
    /// Verdicts decided against an older base: hypotheses, not decisions.
    pub stale: usize,
}

impl BucketShelf {
    /// Whether the bucket adds up: `current + stale == total`. A bucket
    /// that does not reconcile is reporting a state that cannot exist.
    pub fn reconciles(&self) -> bool {
        self.current + self.stale == self.total
    }
}

/// The verdicts a pass issued, and the trunk tips at its boundaries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PassSummary {
    /// The verdicts, in pass order.
    pub verdicts: Vec<Verdict>,
    /// The trunk tip at the start of the pass.
    pub start_sha: String,
    /// The trunk tip at the end of the pass.
    pub end_sha: String,
}

impl PassSummary {
    /// The per-bucket split of one verdict kind against a trunk tip.
    pub fn shelf(&self, kind: VerdictKind, current_tip: &str) -> BucketShelf {
        let mut shelf = BucketShelf {
            total: 0,
            current: 0,
            stale: 0,
        };
        for verdict in self.verdicts.iter().filter(|v| v.kind == kind) {
            shelf.total += 1;
            if verdict.is_current(current_tip) {
                shelf.current += 1;
            } else {
                shelf.stale += 1;
            }
        }
        shelf
    }

    /// The pass's summary line, carrying the expiry the bare counters did
    /// not.
    ///
    /// A bucket with no stale verdicts stays bare — `held=0` — because
    /// "all current" is the state the bare line honestly describes. A
    /// bucket with stale verdicts annotates the count: `held=15 (7
    /// current, 8 against an older base, will be retested)`. The
    /// "will be retested" clause is on `held` only: a hold's memo is
    /// keyed on `(patch hash, base sha)`, so it stops matching once the
    /// trunk has moved and the next pass re-tests it; a `converted`
    /// verdict is terminal and a `skipped` one is merely re-evaluated, so
    /// neither is retested in that sense.
    pub fn line(&self, current_tip: &str) -> String {
        [
            VerdictKind::Converted,
            VerdictKind::Held,
            VerdictKind::Skipped,
        ]
        .iter()
        .map(|kind| bucket_part(*kind, self.shelf(*kind, current_tip)))
        .collect::<Vec<_>>()
        .join(" ")
    }

    /// The pass's span relative to the trunk (invariant 2).
    ///
    /// `commits_during` is the trunk's advance count during the pass,
    /// `duration_secs` its wall-clock length, and `change_interval_secs`
    /// the trunk's measured change interval, when one is known.
    pub fn span(
        &self,
        commits_during: usize,
        duration_secs: u64,
        change_interval_secs: Option<u64>,
    ) -> PassSpan {
        pass_span(
            &self.start_sha,
            &self.end_sha,
            commits_during,
            duration_secs,
            change_interval_secs,
        )
    }
}

/// Whether a pass's summary describes one state of the world or an
/// average over a moving target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PassSpan {
    /// The trunk did not move during the pass, and the pass is shorter
    /// than the trunk's change interval: the summary describes one state
    /// of the world and needs no qualification.
    SingleTrunk,
    /// The trunk moved during the pass: the summary spans at least two
    /// trunks and must say so.
    TrunkMoved {
        /// Trunk advances during the pass, as measured; `0` when the tips
        /// differ but the advance count was not measured.
        commits: usize,
    },
    /// The trunk did not move, but the pass is at least as long as the
    /// trunk's change interval: the summary is an average over a moving
    /// target even though every verdict happens to be current.
    OutlivedInterval {
        /// The pass's wall-clock length, seconds.
        duration_secs: u64,
        /// The trunk's measured change interval, seconds.
        interval_secs: u64,
    },
}

/// One bucket of the summary line, per [`PassSummary::line`]: bare when
/// no verdict is stale, annotated otherwise, with the retest clause on
/// `held` only.
fn bucket_part(kind: VerdictKind, bucket: BucketShelf) -> String {
    if bucket.stale == 0 {
        return format!("{}={}", kind.label(), bucket.total);
    }
    let mut part = format!(
        "{}={} ({} current, {} against an older base",
        kind.label(),
        bucket.total,
        bucket.current,
        bucket.stale
    );
    if kind == VerdictKind::Held {
        part.push_str(", will be retested");
    }
    part.push(')');
    part
}

/// Classifies a pass's span relative to the trunk.
///
/// The trunk moving during the pass is the stronger fact and wins: a
/// pass that ran across a tip change is `TrunkMoved` even when it is also
/// longer than the change interval. A pass that did not move the trunk but
/// outlived the interval is `OutlivedInterval` — the moving target was
/// waiting, and the summary should say it is one. A zero or absent
/// interval is not a measurement and never trips the span.
pub fn pass_span(
    start_sha: &str,
    end_sha: &str,
    commits_during: usize,
    duration_secs: u64,
    change_interval_secs: Option<u64>,
) -> PassSpan {
    if start_sha != end_sha {
        return PassSpan::TrunkMoved {
            commits: commits_during,
        };
    }
    if let Some(interval) = change_interval_secs {
        if interval > 0 && duration_secs >= interval {
            return PassSpan::OutlivedInterval {
                duration_secs,
                interval_secs: interval,
            };
        }
    }
    PassSpan::SingleTrunk
}

impl PassSpan {
    /// The warning a non-`SingleTrunk` span prints beside the summary
    /// line; `None` for `SingleTrunk` — a pass that did not outlive the
    /// interval has nothing to qualify.
    pub fn warn_line(&self, duration_secs: u64) -> Option<String> {
        match self {
            Self::SingleTrunk => None,
            Self::TrunkMoved { commits: 0 } => Some(format!(
                "WARN: the trunk moved during the pass ({duration_secs}s); the summary spans more than one trunk"
            )),
            Self::TrunkMoved { commits } => Some(format!(
                "WARN: pass ran {duration_secs}s across {commits} trunk change(s); the summary spans more than one trunk"
            )),
            Self::OutlivedInterval {
                duration_secs,
                interval_secs,
            } => Some(format!(
                "WARN: pass duration {duration_secs}s meets the trunk change interval {interval_secs}s; the summary is an average over a moving target"
            )),
        }
    }
}

/// The cheap verdicts a pass should re-check at its end (invariant 3):
/// decided against a base that is no longer the tip, and cheap enough to
/// recompute in seconds ("PR already exists", "issue closed") — and the
/// most likely to have changed while the trunk moved.
///
/// Expensive verdicts are not in this set: the hold memo is keyed on
/// `(patch hash, base sha)`, so they are retested on the next pass
/// automatically, and re-running them mid-pass is what made the pass 158
/// minutes. Returns the issues, in pass order.
pub fn due_rechecks(verdicts: &[Verdict], current_tip: &str) -> Vec<u64> {
    verdicts
        .iter()
        .filter(|v| v.cheap && !v.is_current(current_tip))
        .map(|v| v.issue)
        .collect()
}

/// The line the pass prints before finishing, naming the cheap decisions
/// it re-checked at the end.
pub fn recheck_line(due: &[u64]) -> String {
    if due.is_empty() {
        "recheck at pass end: none due".to_string()
    } else {
        let named = due
            .iter()
            .map(|issue| format!("#{issue}"))
            .collect::<Vec<_>>()
            .join(" ");
        format!(
            "recheck at pass end: {} cheap decision(s) against an older base ({named})",
            due.len()
        )
    }
}
