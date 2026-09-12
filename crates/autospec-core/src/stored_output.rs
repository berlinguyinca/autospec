//! Stored-agent-output lifecycle, guard release paths, and blocker
//! escalation (issue #4170).
//!
//! A safety guard that refuses to destroy an issue's output directory is
//! right to refuse — until something else decides the output is expendable.
//! Nothing did, so the guard, once armed, never disarmed: two issues sat
//! undispatched for two days behind a hold that was accurate on every run and
//! released by no one. The patch behind one of them had been fully superseded
//! (every hunk already on `main`; `git apply --check` rejects it outright);
//! archiving that output released the issue immediately.
//!
//! Four invariants, each a primitive here:
//!
//! 1. **A guard that prevents an action must name the action that releases
//!    it.** [`Release`] is the other half of the policy: every held state has
//!    a named release action, and a hold whose release is [`Release::None`]
//!    is rendered as a deadlock, never as an ordinary hold.
//! 2. **Stored output has a lifecycle.** Each output directory is in a known
//!    state — [`OutputState::AwaitingConversion`],
//!    [`OutputState::Converted`], [`OutputState::Superseded`], or
//!    [`OutputState::Failed`] — decided by [`classify`] from cheap evidence.
//!    The cheap test for "superseded" is the one the guard already has: the
//!    patch no longer applies to `main` ([`ApplyCheck::Rejected`]). A check
//!    that cannot answer is never read as "superseded" — fail-closed, the
//!    output stays live.
//! 3. **A blocker that persists across runs escalates.** [`BlockerLedger`]
//!    records when a blocker was first seen (the caller persists it between
//!    runs) and renders its age, so "blocked 2 days by stored output" is
//!    never rendered the same as an ordinary idle cycle.
//! 4. **`ready` and `dispatchable` are different counts and are reported
//!    separately.** [`FrontierCounts::line`] prints the ready count, the
//!    dispatchable count, and the blocked count with reasons — a summary
//!    whose two numbers cannot both be acted on is a summary hiding a third.
//! 5. **A produced patch has a lifecycle, not a terminal state** (issue
//!    #3994). A dispatcher that skips any issue with a patch on disk treats
//!    "produced" as a terminal state with no exit: the patch leaves the
//!    issue's directory only by being deleted, nothing deletes it, and a
//!    patch that `main` has moved past holds its issue out of the queue
//!    forever. The exit that matters is the cheap one the guard already
//!    computes — does the patch still apply to the current base?
//!    [`eligibility`] separates "waiting for me" ([`Eligibility::
//!    AwaitingConversion`]) from "waiting for nothing"
//!    ([`Eligibility::RetireSuperseded`]), and the boundary the conversion
//!    pass actually uses is 3-way merge, not strict `--check`: a patch that
//!    strict `git apply --check` rejects but `git apply --3way` applies
//!    ([`ApplyCheck::AppliesUnder3way`]) is still live. Retirement is
//!    archival ([`superseded_archive_path`]) — never deletion — and an idle
//!    pass over a fully-blocked queue names its dominant skip reason
//!    ([`dominant_skip_line`]).
//!
//! Everything here is pure: no I/O, no clock, no subprocesses. The caller
//! runs `git apply --check` and reports it as [`ApplyCheck`]; the caller
//! supplies "now" as a [`Duration`] measured from a fixed epoch.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// How long a blocker must persist before its line carries an age
/// ("blocked for 2d") instead of being rendered like an ordinary idle cycle.
pub const DEFAULT_ESCALATION_AFTER: Duration = Duration::from_secs(24 * 60 * 60);

/// The lifecycle state of an issue's stored output directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputState {
    /// A patch exists, has not been converted, and still applies to the
    /// trunk. This is the only state a guard must protect: re-dispatch would
    /// destroy live work.
    AwaitingConversion,
    /// The output has been converted to a PR (or nothing unconverted is on
    /// record). Nothing protects a re-dispatch.
    Converted,
    /// A patch exists but no longer applies to the trunk: its changes are
    /// already landed. The stored output has no remaining value and is
    /// expendable — archiving it releases the guard.
    Superseded,
    /// A conversion attempt failed and no live patch remains. A human
    /// decides what happened; the guard must not guess.
    Failed,
}

impl OutputState {
    /// The machine name used in reports and ledger keys.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AwaitingConversion => "awaiting_conversion",
            Self::Converted => "converted",
            Self::Superseded => "superseded",
            Self::Failed => "failed",
        }
    }
}

impl fmt::Display for OutputState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The outcome of the apply checks of the stored patch against the trunk tip
/// — the cheap superseded test. It costs nothing, and it is the only test a
/// guard needs to ask "is this output still live?".
///
/// The boundary that matters is the one the conversion pass actually uses
/// (issue #3994): it applies with `git apply --3way`, not strict `--check`.
/// A patch that strict `--check` rejects can still be converted when `--3way`
/// applies it, so it is live, not superseded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplyCheck {
    /// The patch applies with strict `git apply --check`: the output is live.
    Applies,
    /// Strict `git apply --check` rejected the patch, but `git apply --3way`
    /// would apply it — the boundary between live and superseded. The output
    /// is still convertible and is *not* superseded (issue #3994).
    AppliesUnder3way,
    /// The patch no longer applies even under `--3way` merge: its changes are
    /// already landed or unreachable. The output is superseded.
    Rejected,
    /// The check itself failed to run (missing patch file, transport error,
    /// unparseable output). This is not [`ApplyCheck::Rejected`]: a check
    /// that cannot answer is never read as "no longer applies".
    Unrunnable { detail: String },
}

/// What the caller observed about an issue's stored output.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputEvidence {
    /// A patch file is present in the output directory.
    pub patch_present: bool,
    /// The patch has been converted to a PR.
    pub converted: bool,
    /// A conversion attempt failed (recorded, not inferred).
    pub conversion_failed: bool,
    /// `git apply --check` of the patch against the trunk tip, when it was
    /// run. `None` means it was not run — and not run is never read as
    /// superseded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub apply_check: Option<ApplyCheck>,
}

impl OutputEvidence {
    /// Evidence with no apply check run yet.
    pub fn new(patch_present: bool, converted: bool, conversion_failed: bool) -> Self {
        Self {
            patch_present,
            converted,
            conversion_failed,
            apply_check: None,
        }
    }
}

/// The state of the stored output, or `None` when there is no stored output
/// at all (nothing unconverted, nothing failed) — in which case the guard
/// was never armed and there is nothing to release.
pub fn classify(evidence: &OutputEvidence) -> Option<OutputState> {
    if evidence.converted {
        return Some(OutputState::Converted);
    }
    if evidence.patch_present {
        return Some(match evidence.apply_check {
            Some(ApplyCheck::Rejected) => OutputState::Superseded,
            // Applies, Unrunnable, or never run: fail-closed. A check that
            // cannot answer is not "no longer applies".
            _ => OutputState::AwaitingConversion,
        });
    }
    if evidence.conversion_failed {
        return Some(OutputState::Failed);
    }
    None
}

/// The action that releases a held dispatch.
///
/// "I will not destroy this" is only half a policy; the other half is who
/// decides it may be destroyed, and when. A hold whose release is
/// [`Release::None`] is a deadlock reporting itself as normal operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Release {
    /// Nothing is held; dispatch may proceed.
    None,
    /// Convert the stored patch to a PR. Only a converted patch clears the
    /// guard through the front door.
    ConvertPatch,
    /// Archive the stored output: the patch no longer applies to the trunk,
    /// so it has no remaining value. Archiving releases the guard.
    ArchiveSuperseded,
    /// Review the failed conversion: a human decides what the guard must not
    /// guess.
    ReviewFailure,
}

impl Release {
    /// The imperative phrase naming the release action.
    pub fn line(self) -> &'static str {
        match self {
            Self::None => "no release needed: dispatch may proceed",
            Self::ConvertPatch => "convert the stored patch to a PR",
            Self::ArchiveSuperseded => "archive the superseded output",
            Self::ReviewFailure => "review the failed conversion",
        }
    }
}

/// The release action for a classified state. Every held state names its
/// release; only un-held states have none.
pub fn release(state: Option<OutputState>) -> Release {
    match state {
        None | Some(OutputState::Converted) => Release::None,
        Some(OutputState::AwaitingConversion) => Release::ConvertPatch,
        Some(OutputState::Superseded) => Release::ArchiveSuperseded,
        Some(OutputState::Failed) => Release::ReviewFailure,
    }
}

/// The dispatch eligibility of an issue, decided from its stored output
/// (issue #3994).
///
/// This is the lifecycle exit the dispatcher's old "a patch exists, therefore
/// not eligible" rule lacked. A produced patch is not a terminal state: it is
/// either awaiting conversion (still applies to the current base — skip) or
/// superseded (no longer applies even under 3-way merge — retire and
/// re-dispatch). The single cheap check — does the patch still apply? —
/// separates "waiting for me" from "waiting for nothing".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Eligibility {
    /// No unconverted patch blocks a fresh run: dispatch an agent.
    Dispatch,
    /// A patch exists and still applies (strictly, or under `--3way`): skip —
    /// the work is awaiting conversion, not a fresh dispatch.
    AwaitingConversion,
    /// A patch exists but no longer applies even under `--3way` merge: retire
    /// it (archive, never delete) and return the issue to the eligible pool so
    /// a fresh run can produce one against current `main`.
    RetireSuperseded,
}

impl Eligibility {
    /// Whether the issue may be dispatched on the next pass (issue #3994
    /// AC4): no unconverted patch blocks a fresh run, or the superseded patch
    /// is retired (and the re-dispatch the retirement unblocks). Only a
    /// patch that still applies keeps the issue out of the eligible set.
    pub fn is_dispatchable(self) -> bool {
        !matches!(self, Self::AwaitingConversion)
    }

    /// The one-word machine name used in reports.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dispatch => "dispatch",
            Self::AwaitingConversion => "awaiting_conversion",
            Self::RetireSuperseded => "retire_superseded",
        }
    }
}

impl fmt::Display for Eligibility {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Decide dispatch eligibility from the issue's stored output. The two
/// branches that matter (#3994): a patch that exists and applies is a skip
/// (awaiting conversion), a patch that exists and does not apply even under
/// `--3way` is retired and re-dispatched. A check that cannot answer is
/// fail-closed — never read as "does not apply".
pub fn eligibility(evidence: &OutputEvidence) -> Eligibility {
    if evidence.converted {
        return Eligibility::Dispatch;
    }
    if evidence.patch_present {
        return match evidence.apply_check {
            Some(ApplyCheck::Rejected) => Eligibility::RetireSuperseded,
            // Applies, AppliesUnder3way, Unrunnable, or never run: the patch
            // is still live — fail-closed, never "does not apply".
            _ => Eligibility::AwaitingConversion,
        };
    }
    // No unconverted patch on record (a failed conversion leaves nothing to
    // protect), so a fresh run is not blocked.
    Eligibility::Dispatch
}

/// The archive path a superseded patch is retired to:
/// `out/issue-<N>/superseded/<stem>-<timestamp>.patch` (issue #3994).
///
/// `issue_dir` is the issue's output directory (`out/issue-<N>` in the
/// default layout) and `patch_name` the patch's file name (default
/// `changes.patch`), so the retired patch keeps its stem and gains the
/// retirement timestamp. Retirement is archival, never deletion: the patch is
/// *moved* under the issue's `superseded/` directory, so no patch content is
/// lost and the issue returns to the eligible pool.
pub fn superseded_archive_path(issue_dir: &Path, patch_name: &str, timestamp: u64) -> PathBuf {
    let stem = patch_name
        .rsplit_once('.')
        .map(|(stem, _ext)| stem)
        .unwrap_or(patch_name);
    issue_dir
        .join("superseded")
        .join(format!("{stem}-{timestamp}.patch"))
}

/// One skip reason observed in a dispatch pass, and how many entries it
/// accounted for (issue #3994 AC3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkipReasonCount {
    pub reason: String,
    pub count: usize,
}

/// Group per-entry skip reasons into reason → count, keeping first-seen
/// order. Blank reasons are dropped — a skip without a reason is the silent
/// version of the bug this reports, not an entry to count.
pub fn skip_reason_counts(
    reasons: impl IntoIterator<Item = impl AsRef<str>>,
) -> Vec<SkipReasonCount> {
    let mut groups: Vec<SkipReasonCount> = Vec::new();
    for reason in reasons {
        let reason = reason.as_ref().trim();
        if reason.is_empty() {
            continue;
        }
        match groups.iter_mut().find(|g| g.reason == reason) {
            Some(group) => group.count += 1,
            None => groups.push(SkipReasonCount {
                reason: reason.to_string(),
                count: 1,
            }),
        }
    }
    groups
}

/// The one line a dispatch pass logs when it ends with slots free and zero
/// eligible entries (issue #3994 AC3): it names the dominant skip reason and
/// its count, so a fleet idle because its queue is fully blocked is visible
/// in the log without a manual audit.
///
/// Returns `None` when nothing was skipped — a pass that skipped nothing is
/// not the case this line exists for, and "all eligible" needs no reason.
pub fn dominant_skip_line(counts: &[SkipReasonCount]) -> Option<String> {
    let total: usize = counts.iter().map(|c| c.count).sum();
    if total == 0 {
        return None;
    }
    let dominant = counts
        .iter()
        .max_by_key(|c| c.count)
        .expect("a non-empty total has a dominant reason");
    Some(format!(
        "dispatch pass: 0 eligible with slots free — blocked by {} ({}/{})",
        dominant.reason, dominant.count, total
    ))
}

fn reason_for(state: OutputState) -> &'static str {
    match state {
        OutputState::AwaitingConversion => "live unconverted output (patch still applies to main)",
        OutputState::Superseded => "superseded output (patch no longer applies to main)",
        OutputState::Failed => "failed conversion with no live patch",
        OutputState::Converted => "converted output",
    }
}

/// One issue the frontier could not dispatch, with the reason, the release
/// action, and the age of the blocker (`None` on the first run that saw it).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockedIssue {
    pub issue: String,
    pub reason: String,
    /// The action that releases this hold. [`Release::None`] here is a
    /// defect, not a state: a hold without a named release is a deadlock,
    /// and the line says so instead of pretending.
    pub release: Release,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub age: Option<Duration>,
}

impl BlockedIssue {
    /// The one-line hold report: state, release action, and — when the
    /// blocker is persistent — its age.
    pub fn line(&self) -> String {
        let release_phrase = match self.release {
            Release::None => {
                "release: UNNAMED — this hold has no release path (deadlock)".to_string()
            }
            other => format!("release: {}", other.line()),
        };
        let mut line = format!(
            "#{} NOT dispatched -- {}; {}",
            self.issue, self.reason, release_phrase
        );
        if let Some(age) = self.age {
            line.push_str(&format!(" (blocked for {})", format_age(age)));
        }
        line
    }
}

/// The one-line hold for an issue whose stored output keeps a dispatch from
/// proceeding, or `None` when nothing is held. This is the line a frontier
/// prints instead of the identical-on-every-run line nobody acts on: it
/// names the release action (invariant 1) and, when the blocker is
/// persistent, its age (invariant 3).
pub fn held_line(issue: &str, evidence: &OutputEvidence, age: Option<Duration>) -> Option<String> {
    let state = classify(evidence)?;
    if state == OutputState::Converted {
        return None;
    }
    Some(
        BlockedIssue {
            issue: issue.to_string(),
            reason: reason_for(state).to_string(),
            release: release(Some(state)),
            age,
        }
        .line(),
    )
}

/// One blocker's persistence, across runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockerEntry {
    pub key: String,
    /// When the blocker was first observed. The caller persists the ledger
    /// between runs; without persistence the age dies with the run that
    /// first saw the blocker, and every run looks like the first.
    pub first_seen: Duration,
    pub last_seen: Duration,
}

/// Who has been blocked and since when. Keys are stable per blocker (the
/// issue number and the blocker kind), never per run.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockerLedger {
    entries: BTreeMap<String, BlockerEntry>,
}

impl BlockerLedger {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that `key` is blocked at time `now`. The first observation
    /// stamps `first_seen`; later ones only move `last_seen`, so the age is
    /// measured from when the blocker started, not from the last run.
    pub fn observe(&mut self, key: &str, now: Duration) -> &BlockerEntry {
        let entry = self.entries.entry(key.to_string()).or_insert(BlockerEntry {
            key: key.to_string(),
            first_seen: now,
            last_seen: now,
        });
        entry.last_seen = now;
        entry
    }

    /// The blocker cleared; forget it. The next observation starts a new
    /// age.
    pub fn resolve(&mut self, key: &str) -> Option<BlockerEntry> {
        self.entries.remove(key)
    }

    pub fn get(&self, key: &str) -> Option<&BlockerEntry> {
        self.entries.get(key)
    }

    /// How long `key` has been blocked, saturating at zero: a clock that
    /// rewinds, or a ledger written by a newer clock, is never an age that
    /// underflows.
    pub fn age(&self, key: &str, now: Duration) -> Option<Duration> {
        self.entries
            .get(key)
            .map(|entry| now.saturating_sub(entry.first_seen))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }

    /// The escalation phrase for a persistent blocker ("blocked for 2d"),
    /// or `None` while the blocker is younger than `threshold` — the window
    /// in which a hold may still be ordinary idleness.
    pub fn escalation_phrase(
        &self,
        key: &str,
        now: Duration,
        threshold: Duration,
    ) -> Option<String> {
        let age = self.age(key, now)?;
        (age >= threshold).then(|| format!("blocked for {}", format_age(age)))
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("BlockerLedger serializes")
    }

    pub fn from_json(raw: &str) -> Result<Self, String> {
        serde_json::from_str(raw).map_err(|err| format!("blocker ledger is not valid JSON: {err}"))
    }
}

/// `2d 3h`, `3h 15m`, `45m`, `30s` — coarse on purpose. A blocker's age is
/// for a human deciding whether to act, not for a scheduler.
pub fn format_age(age: Duration) -> String {
    let secs = age.as_secs();
    let (d, h, m, s) = (
        secs / 86_400,
        (secs % 86_400) / 3_600,
        (secs % 3_600) / 60,
        secs % 60,
    );
    let mut parts: Vec<String> = Vec::new();
    if d > 0 {
        parts.push(format!("{d}d"));
        if h > 0 {
            parts.push(format!("{h}h"));
        }
    } else if h > 0 {
        parts.push(format!("{h}h"));
        if m > 0 {
            parts.push(format!("{m}m"));
        }
    } else if m > 0 {
        parts.push(format!("{m}m"));
    } else {
        parts.push(format!("{s}s"));
    }
    parts.join(" ")
}

/// The frontier's summary counts. `ready` and `dispatchable` are different
/// counts and stay separate (invariant 4): *ready* is "no unmet
/// prerequisites", *dispatchable* is "ready and no armed guard". `blocked`
/// carries the third number the old summary hid: how many are blocked, and
/// by what.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrontierCounts {
    pub open: u32,
    pub ready: u32,
    pub dispatchable: u32,
    #[serde(default)]
    pub blocked: Vec<BlockedIssue>,
    pub staged: u32,
    pub dispatched: u32,
}

impl FrontierCounts {
    /// The counts reconcile: every ready issue is either dispatchable or
    /// blocked. A frontier whose numbers do not reconcile is reporting a
    /// state that cannot exist.
    pub fn is_consistent(&self) -> bool {
        self.ready == self.dispatchable + self.blocked.len() as u32
    }

    /// The summary line. Prints the ready count, the dispatchable count,
    /// and — when anything is blocked — the blocked count grouped by reason,
    /// so a "2 ready (0 dispatchable)" frontier can no longer pass for an
    /// idle one.
    pub fn line(&self) -> String {
        let mut line = format!(
            "{} open, {} ready ({} dispatchable), staged {}, dispatched {}",
            self.open, self.ready, self.dispatchable, self.staged, self.dispatched
        );
        if !self.blocked.is_empty() {
            line.push_str(&format!(
                ", {} blocked ({})",
                self.blocked.len(),
                Self::blocked_summary(&self.blocked)
            ));
        }
        line
    }

    fn blocked_summary(blocked: &[BlockedIssue]) -> String {
        let mut groups: Vec<(String, Vec<&str>)> = Vec::new();
        for entry in blocked {
            match groups
                .iter_mut()
                .find(|(reason, _)| *reason == entry.reason)
            {
                Some((_, issues)) => issues.push(&entry.issue),
                None => groups.push((entry.reason.clone(), vec![&entry.issue])),
            }
        }
        groups
            .into_iter()
            .map(|(reason, issues)| {
                let numbered: Vec<String> =
                    issues.iter().map(|issue| format!("#{issue}")).collect();
                format!("{}x {reason} ({})", issues.len(), numbered.join(", "))
            })
            .collect::<Vec<_>>()
            .join(", ")
    }
}
