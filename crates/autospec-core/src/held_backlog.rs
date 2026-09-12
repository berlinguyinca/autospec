//! The backlog is held patches, not conversion capacity (issue #4366).
//!
//! Asked where the pipeline was stuck, the first answer was "121 patches
//! awaiting conversion, ~26 hours of serial gating — parallelise the
//! converter". That count was computed by listing queue entries with a
//! `changes.patch` on disk; a patch on disk means an agent finished, and
//! says nothing about whether conversion has already been attempted. The
//! selector that actually drives the work (`convselect`) excludes
//! `have_pr`, `closed_issue` and `attempted`, and offered 11 candidates.
//! Two sources disagreed, and the one computed by hand was the one that
//! was wrong — its accounting line was on screen in the same session.
//!
//! The real breakdown, of the 116 open `auto-implement` issues that never
//! had a PR:
//!
//! ```text
//! 109  held   (56 conflicts, 17 bats, 9 test, 6 build, 8 misc; 13 unrecorded)
//!   5  an agent running now
//!   2  never implemented
//! ```
//!
//! Nothing was waiting for converter capacity. The 11 candidates were; the
//! other 109 had already been conversion-attempted and stopped for a
//! specific, recorded reason.
//!
//! Invariants, each encoded as a checkable primitive:
//!
//! 1. **Staleness, not throughput, is what makes a fan-out pipeline
//!    stall.** Every agent works from a base that ages while it runs; the
//!    fix is shortening that window or rebasing on arrival, not adding
//!    capacity downstream. A capacity fix reaches only the candidates the
//!    selector offers — it converts no held patch ([`stall_cause`],
//!    [`ProposedFix`], [`fix_population`], [`wrong_bottleneck_finding`]).
//! 2. **Attempt a rebase before declaring a conflict.** `git apply --3way`
//!    against a moved base fails where `git rebase` onto the current base
//!    would succeed, because the latter replays intent rather than
//!    matching context ([`RebaseAttempt`], [`conflict_without_rebase_finding`]).
//! 3. **Measure the queue with the selector that drives the work.** Any
//!    count computed separately will disagree with what actually gets
//!    processed, and the separate count is the one that is wrong
//!    ([`separate_count_finding`]).
//! 4. **A held item is not a queued item.** They need opposite responses —
//!    one needs judgement or a fix, the other needs capacity — and
//!    conflating them points effort at the wrong bottleneck
//!    ([`BacklogState`], [`Response`], [`response_for`],
//!    [`held_counted_as_queued`]).

/// The recorded reason a conversion attempt was held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldReason {
    /// The patch no longer applies to the (moved) base.
    Conflict,
    /// A bats suite failed on the patched tree.
    BatsFailure,
    /// A real new test failure.
    TestFailure,
    /// The patched tree does not build.
    BuildFailure,
    /// Anything else that was recorded.
    Other,
}

impl HoldReason {
    /// The report label used in the breakdown line.
    pub fn label(self) -> &'static str {
        match self {
            HoldReason::Conflict => "conflicts",
            HoldReason::BatsFailure => "bats",
            HoldReason::TestFailure => "test",
            HoldReason::BuildFailure => "build",
            HoldReason::Other => "misc",
        }
    }
}

/// Per-reason counts over the held patches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HeldReasons {
    pub conflicts: usize,
    pub bats: usize,
    pub test: usize,
    pub build: usize,
    pub misc: usize,
}

impl HeldReasons {
    /// How many held patches carry a recorded reason.
    pub fn total(&self) -> usize {
        self.conflicts + self.bats + self.test + self.build + self.misc
    }

    /// The per-reason counts are a partition of a subset of the held set:
    /// they may not exceed the held count.
    pub fn reconciles(&self, held: usize) -> bool {
        self.total() <= held
    }

    /// Held patches with no recorded reason.
    pub fn unrecorded(&self, held: usize) -> usize {
        held.saturating_sub(self.total())
    }
}

/// The breakdown of open `auto-implement` issues that never had a PR.
///
/// A patch on disk is not one of these buckets: it is only evidence an
/// agent finished, and the patch's issue is in exactly one of
/// [`held`](Self::held), [`running`](Self::running) or
/// [`never_implemented`](Self::never_implemented).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BacklogBreakdown {
    /// Conversion was attempted and HELD for a recorded (or unrecorded)
    /// reason.
    pub held: usize,
    /// An agent is running now.
    pub running: usize,
    /// Never implemented: no patch exists.
    pub never_implemented: usize,
    /// What the selector currently offers: the only part of the backlog a
    /// capacity decision is about (invariant 4).
    pub candidates: usize,
}

impl BacklogBreakdown {
    /// The breakdown accounts for every open issue in scope.
    pub fn reconciles(&self, open_total: usize) -> bool {
        self.held + self.running + self.never_implemented == open_total
    }

    /// How many items in the backlog are not in motion: held, plus never
    /// implemented.
    pub fn stalled(&self) -> usize {
        self.held + self.never_implemented
    }
}

/// The per-reason held counts, in fixed order.
pub fn reason_counts_line(reasons: &HeldReasons) -> String {
    let counts = [
        (HoldReason::Conflict, reasons.conflicts),
        (HoldReason::BatsFailure, reasons.bats),
        (HoldReason::TestFailure, reasons.test),
        (HoldReason::BuildFailure, reasons.build),
        (HoldReason::Other, reasons.misc),
    ];
    counts
        .into_iter()
        .map(|(reason, n)| format!("{}={n}", reason.label()))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The one line the backlog is reported in: the breakdown with the
/// per-reason held counts, and the selector's candidate count last,
/// because the candidates are the only part of the backlog a capacity
/// decision is about.
pub fn breakdown_line(breakdown: &BacklogBreakdown, reasons: &HeldReasons) -> String {
    format!(
        "backlog: held={} ({}; unrecorded={}) running={} not_implemented={} candidates={}",
        breakdown.held,
        reason_counts_line(reasons),
        reasons.unrecorded(breakdown.held),
        breakdown.running,
        breakdown.never_implemented,
        breakdown.candidates,
    )
}

/// Invariant 4: a state an item of the backlog can be in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BacklogState {
    /// Conversion was attempted and held for a recorded reason.
    Held,
    /// Offered by the selector, waiting to be converted.
    Queued,
}

/// Invariant 4: the response a state needs. Held and queued need
/// opposite responses; conflating the two states points effort at the
/// wrong bottleneck.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Response {
    /// Judgement or a fix: a rebase, test triage, a decision. Capacity
    /// does nothing for this state.
    JudgementOrFix,
    /// Capacity: the work is eligible and waiting to be processed.
    Capacity,
}

/// Invariant 4: the response a backlog state needs.
pub fn response_for(state: BacklogState) -> Response {
    match state {
        BacklogState::Held => Response::JudgementOrFix,
        BacklogState::Queued => Response::Capacity,
    }
}

/// Invariant 4, as a check: the finding for a report that counts held
/// patches as awaiting conversion.
///
/// `claimed` is what the report said ("121 patches awaiting conversion");
/// `breakdown.candidates` is what the selector offers. A claim above the
/// selector's count can only come from counting items that are not
/// candidates — held patches — and it answers "where is the bottleneck"
/// with "capacity", while the population the claim mostly covers needs
/// the other response.
pub fn held_counted_as_queued(claimed: usize, breakdown: &BacklogBreakdown) -> Vec<String> {
    if claimed <= breakdown.candidates {
        return Vec::new();
    }
    let extra = claimed - breakdown.candidates;
    vec![format!(
        "HELD_COUNTED_AS_QUEUED: the report claims {claimed} patches awaiting conversion but the selector offers {} — {} of the claim are not candidates; held patches need judgement or a fix, not capacity, and counting them as queued points the capacity fix at the wrong bottleneck ({} of the backlog is held)",
        breakdown.candidates,
        extra,
        breakdown.held
    )]
}

/// Invariant 3, as a check: the finding for a queue count computed
/// separately from the selector.
///
/// The count that drives the work is the selector's. A count computed
/// beside it — queue entries with a patch on disk, issue directories, a
/// glob — disagrees with what actually gets processed, and the separate
/// count is the one that is wrong.
pub fn separate_count_finding(computed: usize, selector_candidates: usize) -> Vec<String> {
    if computed == selector_candidates {
        return Vec::new();
    }
    vec![format!(
        "SEPARATE_COUNT: the separately computed queue count is {computed} but the selector that drives the work offers {selector_candidates} — measure the queue with the selector, not a count computed beside it"
    )]
}

/// What was done with a patch that failed to apply to the current base,
/// before a conflict was recorded for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebaseAttempt {
    /// The patch was replayed onto the current base (`git rebase`) and now
    /// applies: the apply failure was a context match against a moved
    /// base, not a conflict. The patch is convertible again.
    Rescued,
    /// The patch was replayed onto the current base and still conflicts:
    /// a genuine conflict that needs judgement.
    GenuineConflict,
    /// No rebase was attempted: the conflict was declared from the failed
    /// `git apply --3way` alone.
    NotAttempted,
}

/// Invariant 2, as a check: the finding for a conflict declared without a
/// rebase attempt.
///
/// `git apply --3way` against a moved base fails where `git rebase` onto
/// the current base would succeed, because the latter replays intent
/// rather than matching context. A conflict recorded from the apply alone
/// is not a conflict yet — it is a hold whose release was never tried.
pub fn conflict_without_rebase_finding(attempt: RebaseAttempt) -> Vec<String> {
    match attempt {
        RebaseAttempt::NotAttempted => vec![
            "CONFLICT_WITHOUT_REBASE: the conflict was declared without attempting a rebase — git apply --3way against a moved base fails where git rebase onto the current base would succeed (the latter replays intent rather than matching context); attempt the rebase before recording the hold".to_string(),
        ],
        RebaseAttempt::Rescued | RebaseAttempt::GenuineConflict => Vec::new(),
    }
}

/// What a fan-out pipeline stall is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bottleneck {
    /// The agents' base ages while they run and patches stop applying on
    /// arrival. The fix shortens the window or rebases on arrival.
    Staleness,
    /// The selector offers more candidates than the converter can process.
    /// The fix adds capacity.
    Throughput,
    /// Neither population is non-zero: nothing is stalled by either cause,
    /// and no fix for either population is misdirected.
    NoStall,
}

/// Invariant 1: classify a stall from the breakdown and the held reasons.
///
/// The population a capacity fix can reach is the selector's candidates;
/// the population a staleness fix can reach is the held-conflict count.
/// When the conflicts outnumber the candidates, the dominant stalled
/// population is unreachable by any converter capacity, and the stall is
/// staleness: more converter speed would convert the 11 and do nothing
/// for the 109. A tie is `Throughput`: the capacity fix reaches at least
/// as much, and the caller can verify with [`fix_population`].
pub fn stall_cause(breakdown: &BacklogBreakdown, reasons: &HeldReasons) -> Bottleneck {
    if reasons.conflicts == 0 && breakdown.candidates == 0 {
        Bottleneck::NoStall
    } else if reasons.conflicts > breakdown.candidates {
        Bottleneck::Staleness
    } else {
        Bottleneck::Throughput
    }
}

/// A fix proposed for a stalled backlog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProposedFix {
    /// Dispatch agents from fresher bases: shorten the base-age window.
    ShorterWindow,
    /// Rebase arriving patches onto the current base before converting.
    RebaseOnArrival,
    /// Add converter capacity (parallelise the serial gate).
    MoreCapacity,
}

/// How many stalled items a proposed fix reaches.
///
/// A capacity fix converts candidates and nothing else; a staleness fix
/// reaches the held conflicts. The number is what the report should carry
/// next to the fix, so "parallelise the converter" is read against "11 of
/// 111".
pub fn fix_population(
    fix: ProposedFix,
    breakdown: &BacklogBreakdown,
    reasons: &HeldReasons,
) -> usize {
    match fix {
        ProposedFix::ShorterWindow | ProposedFix::RebaseOnArrival => reasons.conflicts,
        ProposedFix::MoreCapacity => breakdown.candidates,
    }
}

/// Invariant 1, as a check: the finding for a fix that does not address
/// the classified stall.
///
/// A capacity fix proposed for a staleness stall leaves the dominant
/// population — the held conflicts — untouched; a staleness fix proposed
/// for a throughput stall leaves the waiting candidates untouched. The
/// finding names the population the fix does reach, against the stalled
/// total, so the misdirection is visible in the line itself.
pub fn wrong_bottleneck_finding(
    cause: Bottleneck,
    fix: ProposedFix,
    breakdown: &BacklogBreakdown,
    reasons: &HeldReasons,
) -> Vec<String> {
    match (cause, fix) {
        (Bottleneck::Staleness, ProposedFix::MoreCapacity) => vec![format!(
            "WRONG_BOTTLENECK: the stall is staleness but the proposed fix adds converter capacity — the capacity fix reaches {} of {} stalled items; {} are held on base staleness and need a rebase or a shorter window, not converter capacity",
            fix_population(fix, breakdown, reasons),
            breakdown.stalled(),
            reasons.conflicts,
        )],
        (Bottleneck::Throughput, ProposedFix::ShorterWindow)
        | (Bottleneck::Throughput, ProposedFix::RebaseOnArrival) => vec![format!(
            "WRONG_BOTTLENECK: the stall is throughput but the proposed fix targets base staleness — it reaches {} held conflicts while {} candidates the selector offers need capacity",
            fix_population(fix, breakdown, reasons),
            breakdown.candidates,
        )],
        _ => Vec::new(),
    }
}
