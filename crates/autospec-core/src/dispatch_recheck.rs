//! The dispatcher must re-check that an issue is still open at dispatch
//! time (issue #3774).
//!
//! The autospec dispatcher checks, before launching a seven-hour GPU job
//! for an issue: is it already in flight, does a patch already exist, is
//! it on the hold list, does its spec file exist and exceed 800 bytes. It
//! does not check whether the issue is still open.
//!
//! An issue that was closed without producing a patch — closed as a
//! duplicate, closed because a human fixed it directly, closed as
//! wontfix — passes every existing guard and is dispatched again. And
//! again, on the next cycle, because nothing about the outcome changes:
//! the issue stays closed and no patch appears.
//!
//! The `changes.patch` guard happens to cover the closed issues that *did*
//! produce patches, but that is luck, not design: it is checking a
//! different property for a different reason. Closing without a patch is
//! the normal case this system produces — the more effective the human
//! supervision, the more issues land in exactly the shape that defeats
//! every existing guard.
//!
//! Three invariants, each a primitive here:
//!
//! 1. **Eligibility is re-evaluated against live tracker state at
//!    dispatch time, not only at enqueue time** ([`Recheck::run`]). The
//!    live state is an input to every cycle, never a property of the
//!    worklist.
//! 2. **A closed issue is never dispatched, and it leaves the worklist.**
//!    Encountering a closed issue removes it rather than skipping it
//!    silently each cycle, so the queue converges: the second cycle over
//!    the same worklist removes nothing ([`CycleReport::removed`] is
//!    empty, the worklist is unchanged).
//! 3. **The removal is logged with the issue number and the reason**
//!    ([`Removal::line`]) — the operator sees the delta instead of
//!    inferring it from a worklist that quietly shrinks.
//!
//! A tracker state that cannot be read is fail-closed: the entry is held,
//! not dispatched and not dropped — a check that cannot answer is unsafe,
//! never clear (closure was not established, and dispatching on an
//! unverifiable state is the defect this module exists to prevent).
//!
//! Everything here is pure: no I/O, no clock, no subprocess. The caller
//! fetches the live tracker state and records it; the module decides.

use std::collections::BTreeMap;

/// The staged spec file must exceed this many bytes to be dispatchable.
pub const MIN_SPEC_BYTES: u64 = 800;

/// The reason recorded when a closed issue is dropped from the worklist.
pub const CLOSED_AT_DISPATCH: &str = "closed on the tracker at dispatch time";

/// The live state of an issue on the tracker, read at dispatch time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackerState {
    /// The issue is open: work is still wanted.
    Open,
    /// The issue is closed: work is no longer wanted. Terminal — the
    /// entry is dropped from the worklist, never dispatched.
    Closed,
    /// The tracker was unreachable or unreadable. Fail-closed: the entry
    /// is held — never dispatched, and never dropped, because closure
    /// was not established.
    Unknown,
}

/// The dispatcher's existing enqueue-time guards for one issue.
///
/// Reproduced here so the dispatch-time check is seen *against* them: a
/// closed, patchless issue passes every one of these, which is the latent
/// defect #3774 names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EntryGuards {
    /// An agent is already running for this issue.
    pub in_flight: bool,
    /// A `changes.patch` already exists in the agent output directory.
    pub has_patch: bool,
    /// The issue is on the hold list.
    pub on_hold: bool,
    /// Size of the staged spec file in bytes; 0 means missing.
    pub spec_bytes: u64,
}

impl EntryGuards {
    /// The existing enqueue-time check: every guard must be clear.
    ///
    /// A closed issue with no patch passes this — the check says nothing
    /// about whether the work is still wanted.
    pub fn passes(&self) -> bool {
        self.failure().is_none()
    }

    /// The first guard that fails, in the dispatcher's check order.
    pub fn failure(&self) -> Option<GuardFailure> {
        if self.in_flight {
            Some(GuardFailure::InFlight)
        } else if self.has_patch {
            Some(GuardFailure::PatchExists)
        } else if self.on_hold {
            Some(GuardFailure::OnHold)
        } else if self.spec_bytes <= MIN_SPEC_BYTES {
            Some(GuardFailure::SpecTooSmall)
        } else {
            None
        }
    }
}

/// An enqueue-time guard that refuses dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardFailure {
    /// An agent is already running for the issue.
    InFlight,
    /// A `changes.patch` already exists.
    PatchExists,
    /// The issue is on the hold list.
    OnHold,
    /// The staged spec file is missing or does not exceed
    /// [`MIN_SPEC_BYTES`].
    SpecTooSmall,
}

impl GuardFailure {
    /// The skip reason, as logged.
    pub fn reason(self) -> &'static str {
        match self {
            Self::InFlight => "already in flight",
            Self::PatchExists => "patch already exists",
            Self::OnHold => "on the hold list",
            Self::SpecTooSmall => "spec file missing or not over 800 bytes",
        }
    }
}

/// The dispatcher's worklist: the issues queued for dispatch, in order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Worklist {
    entries: Vec<u64>,
}

impl Worklist {
    /// A worklist in dispatch order.
    pub fn new(entries: impl IntoIterator<Item = u64>) -> Self {
        Self {
            entries: entries.into_iter().collect(),
        }
    }

    /// The entries, in dispatch order.
    pub fn entries(&self) -> &[u64] {
        &self.entries
    }

    /// The number of queued issues.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the worklist is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Remove one entry; `true` when it was present.
    pub fn remove(&mut self, issue: u64) -> bool {
        let before = self.entries.len();
        self.entries.retain(|entry| *entry != issue);
        self.entries.len() != before
    }
}

/// One entry dropped from the worklist, with the reason the log must
/// carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Removal {
    /// The issue number that left the worklist.
    pub issue: u64,
    /// Why it left: [`CLOSED_AT_DISPATCH`].
    pub reason: &'static str,
}

impl Removal {
    /// The log line: the issue number and the reason, both.
    pub fn line(&self) -> String {
        format!("dropped #{} from worklist: {}", self.issue, self.reason)
    }
}

/// The outcome of one dispatch cycle over the worklist.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CycleReport {
    /// Entries dispatched: open on the live tracker and past every
    /// enqueue-time guard.
    dispatched: Vec<u64>,
    /// Entries dropped from the worklist, each with its reason.
    removed: Vec<Removal>,
    /// Entries held: the live tracker state was unreadable, so neither
    /// dispatch nor drop was safe.
    held: Vec<u64>,
    /// Open entries skipped by an enqueue-time guard, each with the
    /// guard that fired.
    skipped: Vec<(u64, GuardFailure)>,
}

impl CycleReport {
    /// The entries dispatched this cycle, in worklist order.
    pub fn dispatched(&self) -> &[u64] {
        &self.dispatched
    }

    /// The entries dropped from the worklist, in worklist order.
    pub fn removed(&self) -> &[Removal] {
        &self.removed
    }

    /// The entries held on unverifiable tracker state, in worklist order.
    pub fn held(&self) -> &[u64] {
        &self.held
    }

    /// The open entries an enqueue-time guard skipped, in worklist order.
    pub fn skipped(&self) -> &[(u64, GuardFailure)] {
        &self.skipped
    }

    /// The log lines: every removal carries the issue number and the
    /// reason, and every hold and skip is named too — a skip without a
    /// reason is the silent version of the bug this reports.
    pub fn lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        for removal in &self.removed {
            lines.push(removal.line());
        }
        for issue in &self.held {
            lines.push(format!(
                "holding #{issue}: tracker state unreadable; fail-closed, never dispatched"
            ));
        }
        for (issue, failure) in &self.skipped {
            lines.push(format!("skipping #{issue}: {}", failure.reason()));
        }
        for issue in &self.dispatched {
            lines.push(format!("dispatching #{issue}"));
        }
        lines
    }
}

/// One dispatch cycle's re-check: the worklist plus everything the cycle
/// was told — the enqueue-time guard facts and the live tracker state.
pub struct Recheck {
    worklist: Worklist,
    guards: BTreeMap<u64, EntryGuards>,
    live: BTreeMap<u64, TrackerState>,
}

impl Recheck {
    /// A re-check over the given worklist.
    pub fn new(worklist: Worklist) -> Self {
        Self {
            worklist,
            guards: BTreeMap::new(),
            live: BTreeMap::new(),
        }
    }

    /// The enqueue-time guard facts the dispatcher observed for one
    /// issue. An entry with no recorded guards fails closed: missing
    /// spec, not dispatchable.
    pub fn guards(&mut self, issue: u64, guards: EntryGuards) {
        self.guards.insert(issue, guards);
    }

    /// The live tracker state read for one issue, at dispatch time. An
    /// entry with no recorded state is held: fail-closed.
    pub fn live_state(&mut self, issue: u64, state: TrackerState) {
        self.live.insert(issue, state);
    }

    /// The worklist, after everything this re-check has run.
    pub fn worklist(&self) -> &Worklist {
        &self.worklist
    }

    /// One dispatch cycle: every entry is re-evaluated against the live
    /// tracker state before any resource is spent on it.
    ///
    /// A closed entry is removed from the worklist and logged (the queue
    /// converges — the next cycle removes nothing further). An
    /// unverifiable entry is held. An open entry faces the existing
    /// enqueue-time guards and dispatches only when every one is clear.
    ///
    /// The tracker check comes first on purpose: closure is terminal and
    /// outranks the guards. A closed issue that also has a patch is
    /// dropped, not skipped by the patch guard — the patch guard was
    /// covering it by accident, and the drop is the property that makes
    /// the worklist shrink.
    pub fn run(&mut self) -> CycleReport {
        let entries = self.worklist.entries.clone();
        let mut report = CycleReport::default();

        for issue in entries {
            let state = self
                .live
                .get(&issue)
                .copied()
                .unwrap_or(TrackerState::Unknown);
            match state {
                TrackerState::Closed => {
                    self.worklist.remove(issue);
                    report.removed.push(Removal {
                        issue,
                        reason: CLOSED_AT_DISPATCH,
                    });
                }
                TrackerState::Unknown => {
                    report.held.push(issue);
                }
                TrackerState::Open => {
                    let guards = self.guards.get(&issue).copied().unwrap_or_default();
                    match guards.failure() {
                        Some(failure) => report.skipped.push((issue, failure)),
                        None => report.dispatched.push(issue),
                    }
                }
            }
        }

        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open() -> EntryGuards {
        EntryGuards {
            in_flight: false,
            has_patch: false,
            on_hold: false,
            spec_bytes: MIN_SPEC_BYTES + 1,
        }
    }

    #[test]
    fn a_closed_issue_without_a_patch_passes_every_enqueue_guard() {
        // The shape that defeats every existing guard: closed, no patch,
        // nothing else wrong. The enqueue-time check says dispatch.
        let guards = open();
        assert!(guards.passes());
        assert_eq!(guards.failure(), None);
    }

    #[test]
    fn a_closed_issue_is_never_dispatched_and_leaves_the_worklist() {
        let mut recheck = Recheck::new(Worklist::new([42, 43]));
        recheck.guards(42, open());
        recheck.guards(43, open());
        recheck.live_state(42, TrackerState::Closed);
        recheck.live_state(43, TrackerState::Open);

        let report = recheck.run();

        assert_eq!(report.dispatched(), &[43]);
        assert!(report.held().is_empty());
        assert_eq!(
            report.removed(),
            &[Removal {
                issue: 42,
                reason: CLOSED_AT_DISPATCH
            }]
        );
        assert_eq!(recheck.worklist().entries(), &[43]);
    }

    #[test]
    fn the_removal_is_logged_with_the_issue_number_and_the_reason() {
        let removal = Removal {
            issue: 3774,
            reason: CLOSED_AT_DISPATCH,
        };
        let line = removal.line();
        assert!(line.contains("#3774"), "{line}");
        assert!(line.contains(CLOSED_AT_DISPATCH), "{line}");

        let mut recheck = Recheck::new(Worklist::new([3774]));
        recheck.guards(3774, open());
        recheck.live_state(3774, TrackerState::Closed);
        let report = recheck.run();
        let lines = report.lines();
        assert!(
            lines
                .iter()
                .any(|line| line.contains("#3774") && line.contains("dropped")),
            "{lines:?}"
        );
    }

    #[test]
    fn the_queue_converges_on_the_second_cycle() {
        let mut recheck = Recheck::new(Worklist::new([42, 43, 44]));
        recheck.guards(42, open());
        recheck.guards(43, open());
        recheck.guards(44, open());
        recheck.live_state(42, TrackerState::Closed);
        recheck.live_state(43, TrackerState::Open);
        recheck.live_state(44, TrackerState::Closed);

        let first = recheck.run();
        assert_eq!(
            first.removed(),
            &[
                Removal {
                    issue: 42,
                    reason: CLOSED_AT_DISPATCH
                },
                Removal {
                    issue: 44,
                    reason: CLOSED_AT_DISPATCH
                },
            ]
        );
        assert_eq!(recheck.worklist().entries(), &[43]);

        // Second cycle over the remaining worklist: nothing to remove,
        // nothing newly dropped. The queue has converged.
        let second = recheck.run();
        assert!(second.removed().is_empty());
        assert_eq!(second.dispatched(), &[43]);
        assert_eq!(recheck.worklist().entries(), &[43]);
    }

    #[test]
    fn closure_outranks_the_patch_guard() {
        // A closed issue that has a patch: the patch guard was covering
        // it by accident. The drop is what the worklist needs — closure
        // is terminal and outranks the guards.
        let mut recheck = Recheck::new(Worklist::new([7]));
        recheck.guards(
            7,
            EntryGuards {
                has_patch: true,
                ..open()
            },
        );
        recheck.live_state(7, TrackerState::Closed);

        let report = recheck.run();

        assert!(report.dispatched().is_empty());
        assert!(report.skipped().is_empty());
        assert_eq!(
            report.removed(),
            &[Removal {
                issue: 7,
                reason: CLOSED_AT_DISPATCH
            }]
        );
        assert!(recheck.worklist().is_empty());
    }

    #[test]
    fn an_unverifiable_tracker_state_is_held_not_dispatched_and_not_dropped() {
        // Fail-closed: the tracker was unreadable. The entry stays in the
        // worklist and is not dispatched.
        let mut recheck = Recheck::new(Worklist::new([9]));
        recheck.guards(9, open());
        recheck.live_state(9, TrackerState::Unknown);

        let report = recheck.run();

        assert!(report.dispatched().is_empty());
        assert!(report.removed().is_empty());
        assert_eq!(report.held(), &[9]);
        assert_eq!(recheck.worklist().entries(), &[9]);
    }

    #[test]
    fn an_entry_with_no_recorded_state_is_held_fail_closed() {
        let mut recheck = Recheck::new(Worklist::new([10]));
        recheck.guards(10, open());

        let report = recheck.run();

        assert_eq!(report.held(), &[10]);
        assert_eq!(recheck.worklist().entries(), &[10]);
    }

    #[test]
    fn an_open_issue_still_faces_the_enqueue_guards() {
        let mut recheck = Recheck::new(Worklist::new([11, 12, 13, 14]));
        recheck.guards(
            11,
            EntryGuards {
                in_flight: true,
                ..open()
            },
        );
        recheck.guards(
            12,
            EntryGuards {
                on_hold: true,
                ..open()
            },
        );
        recheck.guards(
            13,
            EntryGuards {
                spec_bytes: MIN_SPEC_BYTES,
                ..open()
            },
        );
        recheck.guards(14, open());
        for issue in [11, 12, 13, 14] {
            recheck.live_state(issue, TrackerState::Open);
        }

        let report = recheck.run();

        assert_eq!(report.dispatched(), &[14]);
        assert_eq!(
            report.skipped(),
            &[
                (11, GuardFailure::InFlight),
                (12, GuardFailure::OnHold),
                (13, GuardFailure::SpecTooSmall),
            ]
        );
        assert!(report.removed().is_empty());
        assert_eq!(recheck.worklist().entries(), &[11, 12, 13, 14]);
    }

    #[test]
    fn the_spec_guard_requires_exceeding_the_byte_floor() {
        assert_eq!(
            EntryGuards {
                spec_bytes: MIN_SPEC_BYTES,
                ..open()
            }
            .failure(),
            Some(GuardFailure::SpecTooSmall)
        );
        assert_eq!(
            EntryGuards {
                spec_bytes: MIN_SPEC_BYTES + 1,
                ..open()
            }
            .failure(),
            None
        );
    }
}
