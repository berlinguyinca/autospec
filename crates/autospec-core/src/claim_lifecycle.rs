//! A lock's lifecycle is acquire *and* release (issue #4220).
//!
//! A per-issue claim was added to stop two conversion passes from
//! converting the same patch (#4214). After launching, the claim was
//! verified:
//!
//! ```text
//! claim 3813 held by pid 815956 (alive: yes)
//! claim 4183 held by pid 815956 (alive: yes)
//! ```
//!
//! Two claims, correct owner, live process. The lock works.
//!
//! Later, with both issues long finished — 3813 held on conflicts, 4183
//! converted and merged — the claims were still there:
//!
//! ```text
//! held claims: 3813 4131 4183
//! ```
//!
//! Only 4131 was actually being worked on. The claim lived in a single
//! variable (`_held_lock`) that was overwritten each iteration, so the
//! exit trap released only the last claim; the rest survived until a
//! cleanup line at the summary.
//!
//! **Acquisition was verified and release was inferred.** Those are
//! different properties, and the one skipped is the one that makes a lock
//! a lock rather than a one-way marker. The consequence is narrow but
//! real: a completed issue is harmless — it has a PR, so the existing
//! check catches it anyway. A *held* issue is not: it is exactly the case
//! a retry should pick up, and a stale claim blocks that for the
//! remainder of the run. The bug is invisible in the common path and only
//! bites the recovery path.
//!
//! The issue's invariants 1, 2 and 4 are primitives here (invariant 3 —
//! a guard added to fix a concurrency bug deserves the same scrutiny as
//! the bug — is the discipline that makes invariant 1 mandatory for
//! guards, and it lives in `AGENTS.md`):
//!
//! 1. **A lock's lifecycle is acquire *and* release; testing one is
//!    testing half.** The assertion that matters is that the claim is
//!    gone once the work is done — check the claim directory after an
//!    item completes, not only after it starts
//!    ([`lifecycle_coverage`], [`LifecycleCoverage`]).
//! 2. **A resource held in a loop must be released at the top of the next
//!    iteration or at the end of the body, never only at function exit.**
//!    A single variable holding "the current lock" silently converts N
//!    locks into one released lock and N−1 leaks. Where the loop can
//!    `continue` from many places, releasing at the top of the next
//!    iteration is the form with one edit site instead of six
//!    ([`settle_claims`], [`LoopClaimShape::release_edit_sites`]). The
//!    leaked claims that actually block the recovery path are the held
//!    ones ([`blocked_retries`]).
//! 4. **Prefer the shape that cannot leak.** A claim file whose name
//!    encodes the owning PID, checked for liveness on read, degrades
//!    safely on crash without any release path at all — the fleet's
//!    `desired.sh` uses one-file-per-claim for the same reason.
//!    Designing the stale state to be *detectable* beats remembering to
//!    clean it up ([`stale_verdict`], [`after_crash`]).
//!
//! Everything here is pure: no I/O, no clock, no subprocesses. The
//! caller supplies the loop shape, the iterations, and the liveness set.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Invariant 1: a lock's lifecycle is acquire and release
// ---------------------------------------------------------------------------

/// When a lock verification looked at the claim directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CheckPoint {
    /// Right after the item started: proves the claim was taken.
    AfterStart,
    /// After the item completed: proves the claim is gone. This is the
    /// check that makes a lock a lock rather than a one-way marker.
    AfterCompletion,
    /// After the run ended. Counts for neither side: it is too late for
    /// the release side (the in-run retry path the claim protects was
    /// already blocked for the whole run), and it proves nothing about
    /// the acquire side (a claim taken and released again is equally
    /// invisible at run end). The incident's own summary-time cleanup
    /// emptied the directory while every in-run retry stayed blocked.
    AfterRun,
}

/// Whether a lock verification covers both sides of the lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LifecycleCoverage {
    /// Both sides verified at the right points: the claim was seen held
    /// after start and seen gone after completion.
    Complete,
    /// Only acquisition verified — "verified the lock was acquired and
    /// inferred it was released" (the incident).
    AcquireInferredRelease,
    /// Only release verified: the lock goes away, but it was never
    /// confirmed to be there, so the race it exists to prevent is
    /// unproven.
    ReleaseWithoutAcquire,
    /// Nothing verified.
    Unverified,
}

/// Classifies what a set of verification checkpoints actually proves.
///
/// A checkpoint at [`CheckPoint::AfterRun`] satisfies neither side — see
/// that variant. A guard added to fix a concurrency bug is a new lock and
/// owes this same coverage for its own claim (invariant 4 in the issue).
pub fn lifecycle_coverage(checks: &[CheckPoint]) -> LifecycleCoverage {
    let acquired = checks.contains(&CheckPoint::AfterStart);
    let released = checks.contains(&CheckPoint::AfterCompletion);
    match (acquired, released) {
        (true, true) => LifecycleCoverage::Complete,
        (true, false) => LifecycleCoverage::AcquireInferredRelease,
        (false, true) => LifecycleCoverage::ReleaseWithoutAcquire,
        (false, false) => LifecycleCoverage::Unverified,
    }
}

impl LifecycleCoverage {
    /// Whether the verification covers the whole lifecycle.
    pub fn is_complete(self) -> bool {
        self == LifecycleCoverage::Complete
    }

    /// The status line for the verification, whatever it covers.
    pub fn line(self) -> String {
        match self {
            Self::Complete => {
                "lock lifecycle: acquire verified after start, release verified after completion"
                    .to_string()
            }
            Self::AcquireInferredRelease => {
                "lock lifecycle: acquire verified after start; release inferred, never checked"
                    .to_string()
            }
            Self::ReleaseWithoutAcquire => {
                "lock lifecycle: release verified after completion; acquire inferred, never checked"
                    .to_string()
            }
            Self::Unverified => "lock lifecycle: neither side verified".to_string(),
        }
    }

    /// The warning a non-complete verification prints; `None` for
    /// `Complete` — a fully verified lifecycle has nothing to qualify.
    pub fn warn_line(self) -> Option<String> {
        if self.is_complete() {
            return None;
        }
        Some(format!("WARN: {}", self.line()))
    }
}

// ---------------------------------------------------------------------------
// Invariant 2: release in the loop, never only at function exit
// ---------------------------------------------------------------------------

/// Where a loop's claim gets released.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReleaseSite {
    /// At the top of the next iteration: every path through the loop —
    /// including every `continue` — passes it. The one-edit-site form.
    TopOfLoop,
    /// At the end of the body: every `continue` above the release jumps
    /// over it, so those iterations' claims leak.
    EndOfBody,
    /// Only at function exit (the exit trap): the trap holds a single
    /// variable with the last claim, so N−1 earlier claims stay on disk
    /// for the remainder of the run. The incident's shape.
    FunctionExit,
}

/// One iteration of the claiming loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimIteration {
    /// The issue the claim was taken for.
    pub issue: u64,
    /// Whether the iteration left the body via `continue` rather than
    /// falling through to the end of the body.
    pub exited_via_continue: bool,
}

/// The shape of a claiming loop: where the release lives and how many
/// `continue` statements can jump over the end of the body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopClaimShape {
    /// Where the claim gets released.
    pub release_site: ReleaseSite,
    /// The number of `continue` statements in the body. The incident's
    /// loop had six.
    pub continues: usize,
}

impl LoopClaimShape {
    /// How many distinct release points the code must carry for every
    /// exit path to release its claim.
    ///
    /// `TopOfLoop` is the form with one: a single release at the top of
    /// the loop is reached by every path, `continue` included. `EndOfBody`
    /// owes one site at the body end plus one before each `continue` that
    /// jumps over it — `continues + 1`. `FunctionExit` has no in-loop
    /// release at all and owes the same `continues + 1` to become safe.
    /// "One edit site instead of six" is this number, made countable.
    pub fn release_edit_sites(self) -> usize {
        match self.release_site {
            ReleaseSite::TopOfLoop => 1,
            ReleaseSite::EndOfBody | ReleaseSite::FunctionExit => self.continues + 1,
        }
    }
}

/// The state of the claim directory: which claims the release discipline
/// actually releases and which stay on disk.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ClaimLedger {
    /// Claims released by the loop's discipline, in pass order.
    pub released: Vec<u64>,
    /// Claims still on disk after the work they guarded is done, in pass
    /// order. These are what block the retry path for the remainder of
    /// the run; a summary-time cleanup does not unblock them in time.
    pub leaked: Vec<u64>,
}

/// Settles a claiming loop into released and leaked claims.
///
/// The exit trap always releases whatever the single variable last held —
/// the last completed iteration's claim — so that one claim is released
/// under every release site. Beyond that, only the release site decides:
/// `TopOfLoop` releases each claim at the top of the next iteration
/// (`continue` included), `EndOfBody` releases the claims of iterations
/// that fell through to the end of the body, and `FunctionExit` releases
/// nothing in the loop at all. Everything not released is leaked, in pass
/// order.
pub fn settle_claims(iterations: &[ClaimIteration], shape: &LoopClaimShape) -> ClaimLedger {
    let mut released = Vec::new();
    let mut released_seen = BTreeSet::new();
    for (i, iteration) in iterations.iter().enumerate() {
        match shape.release_site {
            ReleaseSite::TopOfLoop => {
                // The top of this iteration releases the previous
                // iteration's claim: `continue` still gets here.
                if i > 0 && released_seen.insert(iterations[i - 1].issue) {
                    released.push(iterations[i - 1].issue);
                }
            }
            ReleaseSite::EndOfBody => {
                if !iteration.exited_via_continue && released_seen.insert(iteration.issue) {
                    released.push(iteration.issue);
                }
            }
            ReleaseSite::FunctionExit => {}
        }
    }
    // The exit trap releases whatever the single variable last held.
    if let Some(last) = iterations.last() {
        if released_seen.insert(last.issue) {
            released.push(last.issue);
        }
    }
    let leaked = iterations
        .iter()
        .map(|it| it.issue)
        .filter(|issue| !released_seen.contains(issue))
        .collect();
    ClaimLedger { released, leaked }
}

impl ClaimLedger {
    /// The claim-directory line, carrying the leaks the bare
    /// "held claims: …" listing hid: `claims: released=1 leaked=1 (#3813)`.
    pub fn line(&self) -> String {
        if self.leaked.is_empty() {
            return format!("claims: released={} leaked=0", self.released.len());
        }
        let named = self
            .leaked
            .iter()
            .map(|issue| format!("#{issue}"))
            .collect::<Vec<_>>()
            .join(" ");
        format!(
            "claims: released={} leaked={} ({named})",
            self.released.len(),
            self.leaked.len()
        )
    }

    /// The warning a leaky ledger prints; `None` when nothing leaked —
    /// "no leaks" is the state the bare listing honestly describes.
    pub fn warn_line(&self) -> Option<String> {
        if self.leaked.is_empty() {
            return None;
        }
        Some(format!(
            "WARN: {} — a leaked claim on a held issue blocks its retry for the remainder of the run",
            self.line()
        ))
    }
}

/// The state of an issue whose claim was leaked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IssueState {
    /// The issue is still open and the patch held: a retry should pick it
    /// up — a leaked claim blocks it. The narrow but real damage.
    Held,
    /// The issue already has a PR or is merged: the existing PR check
    /// catches the leaked claim anyway. Masked, harmless for this run.
    AlreadyHasPr,
}

/// Of the leaked claims, the ones that actually block the recovery path:
/// the held issues. A leaked claim on an issue that already has a PR is
/// masked by the existing check — which is why the leak was invisible in
/// the common path (completed issues) and only bit the recovery path
/// (held ones). Issues whose state is unknown are not reported here.
pub fn blocked_retries(leaked: &[u64], states: &BTreeMap<u64, IssueState>) -> Vec<u64> {
    leaked
        .iter()
        .copied()
        .filter(|issue| states.get(issue) == Some(&IssueState::Held))
        .collect()
}

/// The line reporting which leaked claims block the retry path.
pub fn blocked_retries_line(leaked: &[u64], states: &BTreeMap<u64, IssueState>) -> String {
    let blocked = blocked_retries(leaked, states);
    if blocked.is_empty() {
        "leaked claims: none block the retry path".to_string()
    } else {
        let named = blocked
            .iter()
            .map(|issue| format!("#{issue}"))
            .collect::<Vec<_>>()
            .join(" ");
        format!("leaked claims block the retry path for: {named}")
    }
}

// ---------------------------------------------------------------------------
// Invariant 4: prefer the shape that cannot leak
// ---------------------------------------------------------------------------

/// How the claim is stored and read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ClaimFileDesign {
    /// The claim file's name encodes the owning PID: the reader can tell
    /// who owns the claim from the name alone, without reading content.
    pub pid_in_name: bool,
    /// The reader checks the owning PID for liveness on read: a claim
    /// whose owner is dead is treated as stale and the reader proceeds.
    pub liveness_checked_on_read: bool,
}

/// Whether a crashed owner's leftover claim is detectable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StaleVerdict {
    /// The claim names its owner and the reader checks liveness: a
    /// crashed owner leaves a claim the next reader recognizes as stale
    /// and proceeds. Degrades safely on crash with no release path at
    /// all.
    Detectable,
    /// The claim carries the owner PID but the reader never checks
    /// liveness: the stale state exists, nothing sees it.
    PidNeverChecked,
    /// The claim does not name its owner: a crashed owner's claim is
    /// indistinguishable from a live one — the retry path blocks until a
    /// human (or a later cleanup) removes it.
    Ownerless,
}

/// Classifies a claim file design's stale state.
///
/// A PID the reader never checks is the same as no PID as far as the
/// reader can tell, but the two are reported differently because the
/// remediation differs: `PidNeverChecked` adds one check to the reader,
/// `Ownerless` has to change the claim's encoding. Either way, designing
/// the stale state to be detectable beats remembering to clean it up.
pub fn stale_verdict(design: &ClaimFileDesign) -> StaleVerdict {
    if !design.pid_in_name {
        StaleVerdict::Ownerless
    } else if !design.liveness_checked_on_read {
        StaleVerdict::PidNeverChecked
    } else {
        StaleVerdict::Detectable
    }
}

impl StaleVerdict {
    /// The status line for the design, whatever it detects.
    pub fn line(self) -> String {
        match self {
            Self::Detectable => {
                "stale-claim state: detectable on read (owner pid in name, liveness checked)"
                    .to_string()
            }
            Self::PidNeverChecked => {
                "stale-claim state: pid in name but never checked — stale state is never seen"
                    .to_string()
            }
            Self::Ownerless => {
                "stale-claim state: ownerless claim — a crashed owner's claim is indistinguishable from a live one"
                    .to_string()
            }
        }
    }
}

/// What a leftover claim looks like to the next reader after the owner
/// process died.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AfterCrash {
    /// The owner is alive: the claim is live either way; there is no
    /// stale state to detect.
    Live,
    /// The owner is dead and the stale state is detectable on read: the
    /// reader proceeds. The shape that cannot leak.
    StaleDetected,
    /// The owner is dead and the stale state is not detectable: the
    /// claim looks live and blocks the retry path. The incident's
    /// degradation mode.
    StaleUndetected,
}

/// Resolves the leftover-claim question after the owner process died.
pub fn after_crash(design: &ClaimFileDesign, owner_alive: bool) -> AfterCrash {
    if owner_alive {
        return AfterCrash::Live;
    }
    match stale_verdict(design) {
        StaleVerdict::Detectable => AfterCrash::StaleDetected,
        StaleVerdict::PidNeverChecked | StaleVerdict::Ownerless => AfterCrash::StaleUndetected,
    }
}

/// The claim file name that encodes the owning PID: `claim-<issue>-<pid>`.
///
/// The name alone tells the reader who owns the claim — no content read,
/// no side state — which is what makes the liveness check possible on
/// read.
pub fn claim_file_name(issue: u64, pid: u32) -> String {
    format!("claim-{issue}-{pid}")
}

/// Parses a `claim-<issue>-<pid>` file name back into its issue and
/// owner PID. `None` for names without an encoded PID (a claim that
/// cannot name its owner cannot be checked for one).
pub fn parse_claim_file(name: &str) -> Option<(u64, u32)> {
    let rest = name.strip_prefix("claim-")?;
    let (issue, pid) = rest.rsplit_once('-')?;
    Some((issue.parse().ok()?, pid.parse().ok()?))
}
