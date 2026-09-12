//! The patch-to-PR conversion pass (issue #4388).
//!
//! The pass existed as shell scripts in a session scratch directory
//! (`convpass.sh` / `convselect.sh`) and was lost when that directory was
//! cleared. This module rebuilds its *decisions* as pure, testable
//! primitives; the CLI subcommand (`autospec convert`) wires them to the
//! git/cargo/gh I/O the pass needs.
//!
//! The pass:
//!
//! 1. enumerates agent patches (`$LLM/*/out/issue-*/changes.patch`),
//! 2. selects only those NOT already attempted — a branch, an open/merged PR,
//!    or a recorded HELD entry all disqualify,
//! 3. applies each to a branch off current `origin/main`,
//! 4. runs the affected crate's full gate,
//! 5. opens a PR per passing patch,
//! 6. records a HELD line with the reason for failures — never discards.
//!
//! The decisions this module owns are the two the shell scripts got wrong and
//! whose correction *is* the tool's value:
//!
//! 1. **Selection, not counting.** The pass selects on three disqualifiers —
//!    a branch, an open/merged PR, or a recorded HELD entry — never on the
//!    number of patch files on disk. Counting produced a "121 patches
//!    awaiting conversion" report when only 11 were actually fresh
//!    ([`select_fresh`]).
//! 2. **A no-op pass is distinguishable from a broken one.** A pass handed no
//!    candidates and a pass that examined candidates and found nothing are
//!    different states and must print different lines. The one line that
//!    cannot tell them apart (`converted=0 held=0 skipped=0`) is the incident
//!    recorded in issue #4296; [`PassOutcome`] reuses
//!    [`crate::unfed_pass`] to keep the two lines apart.
//!
//! The pass's other must-survive behaviours live in the modules it already
//! reuses, not here: the authoritative `failures:` name list and its
//! declared-count cross-check are [`crate::failure_attribution`]; the
//! refusal of any conflict auto-resolution it cannot prove safe is
//! [`crate::conflict_resolution`]; the HELD-as-queue re-gate ("re-attempt
//! when the base moves, archive the stale") is [`crate::hold_memo`] and
//! [`crate::stored_output`].
//!
//! Everything here is pure: no I/O, no clock, no subprocesses. The caller
//! supplies each patch's attempted-state (the three disqualifier booleans)
//! and the pass's counters.

use serde::{Deserialize, Serialize};

use crate::unfed_pass::{examined_line, unfed_line, PassCounters};

/// Why a patch on disk is not offered to the pass: something already
/// attempted it. Any one of the three disqualifies — the pass must not offer
/// a patch that a branch, a PR, or a HELD entry already owns.
///
/// Precedence is the order the pass checks: a branch is the cheapest local
/// fact, a PR the remote one, a HELD entry the ledger. When several are set,
/// the first is the reason the report cites; the others are implied by it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Disqualification {
    /// A conversion branch for the issue already exists.
    Branch,
    /// An open or merged pull request for the issue already exists.
    PullRequest,
    /// A recorded HELD entry owns the issue and its re-gate still holds.
    Held,
}

impl Disqualification {
    /// The machine name used in reports and the JSON plan.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Branch => "branch",
            Self::PullRequest => "pull-request",
            Self::Held => "held",
        }
    }
}

/// A patch the pass may offer: identified by its issue and its input key.
///
/// `patch_key` is the "already attempted" memo key (a content hash or the
/// file's mtime), not the file's mere presence: a redispatched agent's fresh
/// work carries a new key and is re-offered even though a patch file for the
/// same issue is still on disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    pub issue: u64,
    /// The patch input key the attempt would be memoed against.
    pub patch_key: String,
}

/// A patch on disk, plus the external evidence of whether it has already been
/// attempted. The pass selects on these three booleans; the presence of the
/// patch file is not one of them.
///
/// [`held_recorded`](PatchCandidate::held_recorded) is the *output* of the
/// HELD-as-queue re-gate ([`crate::hold_memo::re_gate`]), not the raw "a HELD
/// entry exists" fact: a HELD entry whose patch changed, or whose dependent
/// files moved on the base since the hold, has been re-gated and is re-offered
/// — so the caller records `held_recorded = false` for it and the selection
/// offers it again. Only a still-held entry disqualifies.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PatchCandidate {
    pub issue: u64,
    pub patch_key: String,
    /// A conversion branch for this issue already exists.
    pub branch_exists: bool,
    /// An open or merged pull request for this issue already exists.
    pub pull_request_exists: bool,
    /// A recorded HELD entry owns this issue and its re-gate still holds.
    pub held_recorded: bool,
}

/// The disqualifier for a candidate's three booleans, or `None` when the
/// patch is fresh and should be offered.
pub fn disqualification(
    branch_exists: bool,
    pull_request_exists: bool,
    held_recorded: bool,
) -> Option<Disqualification> {
    if branch_exists {
        Some(Disqualification::Branch)
    } else if pull_request_exists {
        Some(Disqualification::PullRequest)
    } else if held_recorded {
        Some(Disqualification::Held)
    } else {
        None
    }
}

/// The result of selecting the fresh patches to attempt.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Selection {
    /// The patches to attempt this pass: those with no disqualifier.
    pub fresh: Vec<Candidate>,
    /// The patches not offered, each paired with the disqualifier that
    /// excluded it.
    pub disqualified: Vec<(Candidate, Disqualification)>,
}

/// The "11, not 121" selection (step 2 of the pass).
///
/// A patch is offered exactly when none of its three disqualifiers is set.
/// The old tool instead counted patch files on disk and reported every one of
/// them as awaiting conversion — 121 "fresh" when a branch, a PR, or a HELD
/// entry already owned 110 of them and only 11 were actually fresh.
pub fn select_fresh(candidates: &[PatchCandidate]) -> Selection {
    let mut selection = Selection::default();
    for candidate in candidates {
        match disqualification(
            candidate.branch_exists,
            candidate.pull_request_exists,
            candidate.held_recorded,
        ) {
            None => selection.fresh.push(Candidate {
                issue: candidate.issue,
                patch_key: candidate.patch_key.clone(),
            }),
            Some(reason) => selection.disqualified.push((
                Candidate {
                    issue: candidate.issue,
                    patch_key: candidate.patch_key.clone(),
                },
                reason,
            )),
        }
    }
    selection
}

impl Selection {
    /// The number of patches this pass will attempt.
    pub fn fresh_count(&self) -> usize {
        self.fresh.len()
    }

    /// The not-offered patches grouped by disqualifier, in branch → PR → HELD
    /// order. The three counts reconcile against the number examined:
    /// `fresh + disqualified == examined`.
    pub fn disqualified_counts(&self) -> [(Disqualification, usize); 3] {
        let count = |want: Disqualification| {
            self.disqualified
                .iter()
                .filter(|(_, reason)| *reason == want)
                .count()
        };
        [
            (Disqualification::Branch, count(Disqualification::Branch)),
            (
                Disqualification::PullRequest,
                count(Disqualification::PullRequest),
            ),
            (Disqualification::Held, count(Disqualification::Held)),
        ]
    }

    /// The one-line plan summary: how many were examined, how many are fresh,
    /// and how each disqualifier accounted for the rest. `examined` is the
    /// size of the input the pass was handed — a zero fresh count read against
    /// it is what keeps an idle pass distinct from a broken one.
    pub fn line(&self, examined: usize) -> String {
        let [(branch, branch_n), (pr, pr_n), (held, held_n)] = self.disqualified_counts();
        format!(
            "conversion pass: examined={examined} fresh={} ({} {} {} {} {} {})",
            self.fresh.len(),
            branch_n,
            branch.as_str(),
            pr_n,
            pr.as_str(),
            held_n,
            held.as_str(),
        )
    }
}

/// The outcome of a conversion pass. A pass handed no candidates (unfed) and
/// a pass that examined candidates and found nothing (idle) are different
/// states and print different lines.
///
/// The incident this exists to close: the shell pass took its work
/// positionally and, run bare, printed `converted=0 held=0 skipped=0` —
/// byte-identical to a healthy idle pass — while four patches waited. One
/// line that cannot tell the two apart reads as an empty backlog, and the
/// longer it runs the more normal it looks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PassOutcome {
    /// The caller handed the pass no candidates; it did no work. Its line
    /// must not look like an idle pass.
    Unfed,
    /// The pass examined its candidates and acted (or found nothing to act
    /// on). The counters are populated and must reconcile.
    Examined(PassCounters),
}

impl PassOutcome {
    /// The pass summary line. `tool`, `script`, and `selector` are the names
    /// the unfed line's remedy carries (see [`crate::unfed_pass::unfed_line`]).
    pub fn line(&self, tool: &str, script: &str, selector: &str) -> String {
        match self {
            PassOutcome::Unfed => unfed_line(tool, script, selector),
            PassOutcome::Examined(counters) => examined_line(tool, counters),
        }
    }

    /// The counters the outcome carries, or `None` when the pass was unfed
    /// and none were populated. A guarded/unfed exit must not restate
    /// counters that were never populated.
    pub fn counters(&self) -> Option<&PassCounters> {
        match self {
            PassOutcome::Unfed => None,
            PassOutcome::Examined(counters) => Some(counters),
        }
    }

    /// Whether the examined counters reconcile — a pass cannot act on more
    /// items than it examined. An unfed pass has nothing to reconcile.
    pub fn reconciles(&self) -> bool {
        match self {
            PassOutcome::Unfed => true,
            PassOutcome::Examined(counters) => counters.reconciles(),
        }
    }

    /// The incident, as a check: the unfed line and the all-idle line must
    /// differ. They always do — the unfed line names the empty-input branch
    /// and the selector, the idle line leads with `examined=` — so a pass
    /// that renders through this type can no longer collapse the two.
    pub fn unfed_and_idle_differ(tool: &str, script: &str, selector: &str) -> bool {
        let unfed = PassOutcome::Unfed.line(tool, script, selector);
        let idle = PassOutcome::Examined(PassCounters::default()).line(tool, script, selector);
        unfed != idle
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh(issue: u64) -> PatchCandidate {
        PatchCandidate {
            issue,
            patch_key: format!("patch-{issue}"),
            ..Default::default()
        }
    }

    fn with_flag(mut candidate: PatchCandidate, flag: fn(&mut PatchCandidate)) -> PatchCandidate {
        flag(&mut candidate);
        candidate
    }

    #[test]
    fn a_patch_with_no_disqualifier_is_offered() {
        let selection = select_fresh(&[fresh(1)]);
        assert_eq!(selection.fresh_count(), 1);
        assert!(selection.disqualified.is_empty());
        assert_eq!(selection.fresh[0].issue, 1);
    }

    #[test]
    fn each_disqualifier_excludes_its_patch() {
        let branch = with_flag(fresh(1), |c| c.branch_exists = true);
        let pr = with_flag(fresh(2), |c| c.pull_request_exists = true);
        let held = with_flag(fresh(3), |c| c.held_recorded = true);
        let selection = select_fresh(&[branch, pr, held]);
        assert!(selection.fresh.is_empty());
        let reasons: Vec<Disqualification> =
            selection.disqualified.iter().map(|(_, r)| *r).collect();
        assert_eq!(
            reasons,
            vec![
                Disqualification::Branch,
                Disqualification::PullRequest,
                Disqualification::Held
            ]
        );
    }

    #[test]
    fn the_pass_selects_not_counts() {
        // The incident: 121 patches on disk, but 110 already owned by a
        // branch, a PR, or a HELD entry, leaving 11 actually fresh.
        let candidates: Vec<PatchCandidate> = (1..=121)
            .map(|n| {
                let mut c = fresh(n);
                match n {
                    1..=70 => c.branch_exists = true,
                    71..=95 => c.pull_request_exists = true,
                    96..=110 => c.held_recorded = true,
                    _ => {}
                }
                c
            })
            .collect();
        let selection = select_fresh(&candidates);
        assert_eq!(selection.fresh_count(), 11, "11 fresh, not 121");
        let counts = selection.disqualified_counts();
        assert_eq!(counts[0], (Disqualification::Branch, 70));
        assert_eq!(counts[1], (Disqualification::PullRequest, 25));
        assert_eq!(counts[2], (Disqualification::Held, 15));
        // The counts reconcile against the input size.
        assert_eq!(
            selection.fresh_count() + selection.disqualified.len(),
            candidates.len()
        );
    }

    #[test]
    fn branch_wins_the_precedence_when_several_are_set() {
        let all = PatchCandidate {
            issue: 1,
            patch_key: "k".to_string(),
            branch_exists: true,
            pull_request_exists: true,
            held_recorded: true,
        };
        let selection = select_fresh(&[all]);
        assert_eq!(
            selection.disqualified[0].1,
            Disqualification::Branch,
            "branch is checked first and is the cited reason"
        );
    }

    #[test]
    fn disqualification_reports_each_flag_independently() {
        assert_eq!(disqualification(false, false, false), None);
        assert_eq!(
            disqualification(true, false, false),
            Some(Disqualification::Branch)
        );
        assert_eq!(
            disqualification(false, true, false),
            Some(Disqualification::PullRequest)
        );
        assert_eq!(
            disqualification(false, false, true),
            Some(Disqualification::Held)
        );
    }

    #[test]
    fn the_selection_line_names_examined_and_each_reason() {
        let selection = select_fresh(&[
            with_flag(fresh(1), |c| c.branch_exists = true),
            with_flag(fresh(2), |c| c.held_recorded = true),
            fresh(3),
            fresh(4),
        ]);
        let line = selection.line(4);
        assert!(line.contains("examined=4"), "{line}");
        assert!(line.contains("fresh=2"), "{line}");
        assert!(line.contains("1 branch"), "{line}");
        assert!(line.contains("1 held"), "{line}");
        assert!(line.contains("0 pull-request"), "{line}");
    }

    // --- unfed vs idle ------------------------------------------------------

    #[test]
    fn an_unfed_pass_and_an_idle_pass_print_different_lines() {
        let tool = "convert";
        let script = "autospec convert";
        let selector = "enumerate $LLM";
        assert!(PassOutcome::unfed_and_idle_differ(tool, script, selector));

        let unfed = PassOutcome::Unfed.line(tool, script, selector);
        let idle = PassOutcome::Examined(PassCounters {
            examined: 0,
            converted: 0,
            held: 0,
            skipped: 0,
        })
        .line(tool, script, selector);
        assert_ne!(unfed, idle, "the incident is the two lines being equal");
        // The idle line leads with the input size; the unfed line names the
        // empty-input branch and the selector that would feed it.
        assert!(idle.contains("examined=0"), "{idle}");
        assert!(unfed.contains("no issues given"), "{unfed}");
        assert!(unfed.contains(selector), "{unfed}");
    }

    #[test]
    fn an_unfed_outcome_carries_no_counters() {
        assert!(PassOutcome::Unfed.counters().is_none());
        let examined = PassOutcome::Examined(PassCounters {
            examined: 3,
            converted: 1,
            held: 1,
            skipped: 1,
        });
        assert_eq!(examined.counters().unwrap().examined, 3);
    }

    #[test]
    fn the_outcome_reconciles_its_counters() {
        assert!(PassOutcome::Unfed.reconciles());
        let ok = PassOutcome::Examined(PassCounters {
            examined: 3,
            converted: 1,
            held: 1,
            skipped: 1,
        });
        assert!(ok.reconciles());
        let impossible = PassOutcome::Examined(PassCounters {
            examined: 3,
            converted: 3,
            held: 1,
            skipped: 0,
        });
        assert!(
            !impossible.reconciles(),
            "acting on more than examined is impossible"
        );
    }

    #[test]
    fn the_disqualifier_round_trips_its_wire_form() {
        for (value, wire) in [
            (Disqualification::Branch, "branch"),
            (Disqualification::PullRequest, "pull-request"),
            (Disqualification::Held, "held"),
        ] {
            assert_eq!(value.as_str(), wire);
            let parsed: Disqualification = serde_json::from_str(&format!("\"{wire}\"")).unwrap();
            assert_eq!(parsed, value);
        }
    }
}
