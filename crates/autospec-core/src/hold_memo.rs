//! A deterministic hold is computed once per distinct input (issue #4360).
//!
//! Measured across one session's `convpass.log`:
//!
//! | | |
//! |---|---|
//! | issues ever held | **138** |
//! | total HELD events | **188** |
//! | redundant holds (beyond the first) | **50** — ~6h40m of repeated gating at ~8 min each |
//!
//! The repeat offenders:
//!
//! | issue | times held | distinct reasons |
//! |---|---|---|
//! | #4068 | 10 | 3 |
//! | #4015 | **6** | **1** |
//! | #4198 | 4 | 2 |
//! | #4018 | 4 | 2 |
//! | #3992 | 3 | 1 |
//!
//! #4015 was held six times and produced one distinct reason — the same
//! conflict in `queue_commands.rs` and `ready_queue.rs`, re-derived from
//! scratch on six separate passes. Each re-derivation applied the patch,
//! resolved what it could, ran a gate, and reached the identical
//! conclusion. Some repeats are legitimate: #4068's three distinct reasons
//! are real, because the resolver was fixed twice between runs, so the
//! outcome genuinely changed. That is the case the design must preserve.
//!
//! The existing memo (`memo_key`, #4260) does not prevent it. `convselect`
//! keys its "already attempted" memo on the patch's mtime, so a
//! redispatched agent's fresh work is correctly re-offered. But the patch
//! is only one of two inputs. The other is the base, and `main` moves
//! constantly, so every pass sees a new base and re-offers everything
//! previously held. For a conflict, a moved base is exactly what might
//! resolve it — which is why the re-run is not obviously wasteful, and why
//! it has gone unnoticed at 50 occurrences.
//!
//! The sharper predicate: a conflict hold depends on the patch and on the
//! conflicting files, **not on the whole base**. So:
//!
//! > Re-gate a held issue when the patch has changed, or when any file
//! > named in its recorded hold reason has changed on the base since that
//! > hold. Otherwise report `still held (unchanged since <sha>)` in one
//! > line.
//!
//! Invariants, each encoded as a checkable primitive:
//!
//! 1. **Record what a hold depended on, not just that it happened.** A
//!    reason naming files is already most of the way there; it needs the
//!    base sha alongside it to become a cache key (`HoldRecord`;
//!    [`HoldRecord::new`] refuses a key with no patch or base).
//! 2. **A deterministic outcome is computed once per distinct input.**
//!    Re-deriving it is not merely slow — it hides the interesting case,
//!    because a changed reason is the signal and it is buried among
//!    identical repeats (`re_gate`, [`ReGateDecision`]).
//! 3. **Report unchanged holds in one line, never silently skip them.**
//!    "Still held, unchanged since abc123" keeps the issue visible without
//!    paying for it, and preserves the distinction between idle and broken
//!    ([`ReGateDecision::line`]).
//! 4. **Count repeats and surface them.** 50 redundant events accumulated
//!    invisibly; nothing reported that the same conclusion was being
//!    reached over and over. Any loop that can reach the same outcome
//!    repeatedly should track how often it does, because that number is the
//!    measure of work it is wasting ([`OutcomeLedger`], [`wasted_secs`]).
//!
//! Everything here is pure: no I/O, no clock, no subprocesses. The caller
//! supplies the patch keys and the set of files that moved on the base
//! since the hold.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// A recorded hold: not just that it happened, but *what it depended on*
/// (invariant 1). A reason naming files is already most of the way there;
/// this records the base sha alongside so the pair becomes a cache key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HoldRecord {
    /// The issue the hold is about.
    pub issue: u64,
    /// The patch input key (content hash or mtime) the hold was derived
    /// against.
    pub patch_key: String,
    /// The base sha (trunk tip) the hold was derived against.
    pub base_sha: String,
    /// The files the hold depends on — the files named in the recorded
    /// hold reason. A conflict hold names the conflicting files; a
    /// test-failure hold names the failing test's file. Empty when the
    /// reason names no files: the hold then depends on the whole base
    /// (see [`re_gate`]).
    pub depends_on: BTreeSet<String>,
    /// The human-readable reason, on record.
    pub reason: String,
}

impl HoldRecord {
    /// Construct a hold record.
    ///
    /// `None` when the patch key or the base sha is empty or whitespace: a
    /// cache key with no base (or no patch) is exactly the defect this
    /// module exists to make visible — a hold that cannot say *against
    /// what* it was derived is refused, never defaulted (the same
    /// discipline as [`crate::verdict_shelf::Verdict::new`]).
    pub fn new(
        issue: u64,
        patch_key: impl Into<String>,
        base_sha: impl Into<String>,
        depends_on: impl IntoIterator<Item = String>,
        reason: impl Into<String>,
    ) -> Option<Self> {
        let patch_key = patch_key.into();
        let base_sha = base_sha.into();
        if patch_key.trim().is_empty() || base_sha.trim().is_empty() {
            return None;
        }
        let depends_on = depends_on
            .into_iter()
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect();
        Some(Self {
            issue,
            patch_key,
            base_sha,
            depends_on,
            reason: reason.into(),
        })
    }
}

/// The outcome of deciding whether a recorded hold must be re-derived this
/// pass (invariant 2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReGateDecision {
    /// The recorded outcome still stands: the patch is unchanged and no
    /// file the hold depends on moved on the base. Do not re-derive it —
    /// report it in one line (invariant 3).
    StillHeld {
        /// The base sha the recorded outcome is unchanged since.
        since_sha: String,
    },
    /// The outcome must be re-derived: the patch changed and/or a file the
    /// hold depends on moved on the base since the hold.
    ReGate {
        /// The patch input key differs from the recorded one.
        patch_changed: bool,
        /// The files the hold depends on that moved on the base since the
        /// hold, sorted.
        files_changed: Vec<String>,
    },
}

impl ReGateDecision {
    /// The one line the pass prints for this issue (invariant 3).
    ///
    /// An unchanged hold reports `still held (unchanged since <sha>)` —
    /// one line, never silently skipped, so the issue stays visible and the
    /// idle/broken distinction is preserved. A re-gated issue says why: the
    /// changed patch and/or the dependent files that moved.
    pub fn line(&self) -> String {
        match self {
            Self::StillHeld { since_sha } => format!("still held (unchanged since {since_sha})"),
            Self::ReGate {
                patch_changed,
                files_changed,
            } => {
                let mut parts: Vec<String> = Vec::new();
                if *patch_changed {
                    parts.push("patch changed".to_string());
                }
                if !files_changed.is_empty() {
                    parts.push(format!(
                        "{} dependent file(s) changed on base: {}",
                        files_changed.len(),
                        files_changed.join(" ")
                    ));
                }
                format!("re-gate: {}", parts.join("; "))
            }
        }
    }

    /// Whether the recorded outcome still stands (no re-derivation needed).
    pub fn is_still_held(&self) -> bool {
        matches!(self, Self::StillHeld { .. })
    }
}

/// The sharper predicate (invariant 2): re-gate a held issue when the
/// patch has changed, or when any file named in its recorded hold reason
/// has changed on the base since that hold. Otherwise the recorded outcome
/// stands unchanged.
///
/// `current_patch_key` is the patch input key on disk this pass;
/// `changed_files` is the set of files that moved on the base since
/// `record.base_sha` (the caller computes it, e.g. `git diff --name-only
/// <base>..`).
///
/// A hold that names no files (`depends_on` empty) depends on the whole
/// base: any base change re-gates it. Over-re-gating is the safe direction
/// — a needless re-derivation, never a stale "still held" — mirroring the
/// discipline of `platform_gate` and `construction_sites`.
pub fn re_gate(
    record: &HoldRecord,
    current_patch_key: &str,
    changed_files: &[String],
) -> ReGateDecision {
    let patch_changed = record.patch_key != current_patch_key;
    let changed: BTreeSet<&str> = changed_files.iter().map(String::as_str).collect();
    let files_changed: Vec<String> = if record.depends_on.is_empty() {
        // No files named: the hold depends on the whole base.
        let mut all = changed_files.to_vec();
        all.sort();
        all
    } else {
        record
            .depends_on
            .iter()
            .filter(|f| changed.contains(f.as_str()))
            .cloned()
            .collect()
    };
    if !patch_changed && files_changed.is_empty() {
        ReGateDecision::StillHeld {
            since_sha: record.base_sha.clone(),
        }
    } else {
        ReGateDecision::ReGate {
            patch_changed,
            files_changed,
        }
    }
}

/// Counts how often a loop reaches the same outcome for the same subject
/// (invariant 4). Any loop that can reach the same outcome repeatedly
/// should track how often it does, because that number is the measure of
/// work it is wasting.
///
/// A hold's outcome key is the signature of the recorded conclusion (e.g.
/// the reason, or a hash of the recorded hold inputs). Distinct keys for
/// the same subject are legitimate re-derivations — the outcome genuinely
/// changed, as #4068's three reasons did; identical keys are repeats, as
/// #4015's six identical conflict holds were.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct OutcomeLedger {
    /// `(subject, outcome_key)` in event order.
    events: Vec<(u64, String)>,
}

impl OutcomeLedger {
    /// An empty ledger.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one outcome reached for a subject.
    pub fn record(&mut self, subject: u64, outcome_key: impl Into<String>) {
        self.events.push((subject, outcome_key.into()));
    }

    /// The total number of outcomes recorded.
    pub fn total(&self) -> usize {
        self.events.len()
    }

    /// How many times the loop reached an outcome for this subject.
    pub fn times(&self, subject: u64) -> usize {
        self.events.iter().filter(|(s, _)| *s == subject).count()
    }

    /// How many distinct outcomes the loop reached for this subject.
    pub fn distinct(&self, subject: u64) -> usize {
        let mut keys = BTreeSet::new();
        for (s, k) in &self.events {
            if *s == subject {
                keys.insert(k.as_str());
            }
        }
        keys.len()
    }

    /// The number of distinct subjects the loop reached an outcome for.
    pub fn subjects(&self) -> usize {
        self.events
            .iter()
            .map(|(s, _)| *s)
            .collect::<BTreeSet<_>>()
            .len()
    }

    /// The number of distinct `(subject, outcome)` pairs the loop reached.
    pub fn distinct_outcomes(&self) -> usize {
        let mut seen = BTreeSet::new();
        for (s, k) in &self.events {
            seen.insert((*s, k.as_str()));
        }
        seen.len()
    }

    /// Holds beyond each subject's first: `total - subjects`. This is the
    /// issue's top-line "redundant holds (beyond the first)" — 50 in the
    /// incident. It is an *upper bound* on the work wasted: it also counts
    /// the legitimate re-derivations where the outcome changed.
    pub fn beyond_first(&self) -> usize {
        self.total() - self.subjects()
    }

    /// The work actually wasted: the same conclusion reached more than once
    /// for the same subject, `total - distinct_outcomes`. A re-derivation
    /// that produced a *different* outcome is not in this set — it was
    /// necessary, as #4068's three distinct reasons were.
    pub fn repeated_outcomes(&self) -> usize {
        self.total() - self.distinct_outcomes()
    }

    /// Subjects that reached the same outcome more than once, sorted. These
    /// are the offenders that should be surfaced, not left to accumulate
    /// invisibly.
    pub fn repeat_subjects(&self) -> Vec<u64> {
        let subjects: BTreeSet<u64> = self.events.iter().map(|(s, _)| *s).collect();
        let mut offenders: Vec<u64> = subjects
            .iter()
            .copied()
            .filter(|s| self.times(*s) > self.distinct(*s))
            .collect();
        offenders.sort();
        offenders
    }

    /// Whether the ledger's own numbers reconcile. `total` is both
    /// `subjects + beyond_first` and `distinct_outcomes +
    /// repeated_outcomes`, and a subject has at least one outcome, so
    /// `distinct_outcomes >= subjects`. A ledger that does not reconcile is
    /// reporting a state that cannot exist.
    pub fn reconciles(&self) -> bool {
        self.subjects() + self.beyond_first() == self.total()
            && self.distinct_outcomes() + self.repeated_outcomes() == self.total()
            && self.distinct_outcomes() >= self.subjects()
    }

    /// The summary line. The redundant counts are the numbers that must not
    /// accumulate invisibly; offenders are named with how often they
    /// re-derived the same outcome.
    pub fn line(&self) -> String {
        let mut line = format!(
            "outcomes: total={} subjects={} beyond_first={} repeated_outcomes={}",
            self.total(),
            self.subjects(),
            self.beyond_first(),
            self.repeated_outcomes()
        );
        let offenders = self.repeat_subjects();
        if !offenders.is_empty() {
            let named = offenders
                .iter()
                .map(|s| format!("#{s} x{}", self.times(*s)))
                .collect::<Vec<_>>()
                .join(" ");
            line.push_str(&format!(" ({named})"));
        }
        line
    }
}

/// The work a loop wastes reaching the same outcome repeatedly: the
/// redundant count times the per-derivation cost (invariant 4's general
/// form). 50 redundant holds at ~8 minutes each is ~6h40m of gating.
pub fn wasted_secs(redundant: usize, per_derivation_secs: u64) -> u64 {
    (redundant as u64).saturating_mul(per_derivation_secs)
}

/// Render a duration in seconds as `6h40m`, `8m`, `45s`, or `0s` — the
/// shape the incident's "~6h40m" is stated in.
pub fn format_duration(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    match (h, m, s) {
        (0, 0, s) => format!("{s}s"),
        (0, m, 0) => format!("{m}m"),
        (0, m, s) => format!("{m}m{s}s"),
        (h, 0, 0) => format!("{h}h"),
        (h, m, 0) => format!("{h}h{m}m"),
        (h, m, s) => format!("{h}h{m}m{s}s"),
    }
}
