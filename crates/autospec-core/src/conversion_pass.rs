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

use crate::patch_language::PatchLanguage;
use crate::unfed_pass::{examined_line, unfed_line, PassCounters};

/// The attempt state a candidate's branch carries, the external evidence of
/// whether the issue has already been worked. A branch alone is not evidence
/// of an attempt: the pass pushes a branch and only then opens the PR, so an
/// interruption in that window leaves a branch with no PR — and a liveness
/// check that read a bare branch as "attempted" silently retired the issue,
/// never converted, never held, never reported (#4499). The disqualifying
/// fact is a live attempt (a branch with an open or merged PR, or a checked-
/// out worktree); a branch without one is an interrupted attempt, re-offered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Attempt {
    /// No branch exists: nothing was ever attempted here. The default: most
    /// patches were never attempted, and `#[derive(Default)]` on
    /// `PatchCandidate` needs a default attempt state.
    #[default]
    Fresh,
    /// The attempt is in play: a branch with an open or merged pull request,
    /// or a local worktree holding the branch.
    Live,
    /// The branch exists but no open or merged pull request uses it: an
    /// interrupted or abandoned attempt. Re-offered, and reported as such.
    Interrupted,
    /// The liveness lookup could not complete. No verdict was reached; the
    /// selection folds this fail-closed like a live attempt, because
    /// offering a patch whose attempt state cannot be verified risks a
    /// duplicate PR.
    Unknown,
}

impl Attempt {
    /// The machine name used in reports and the JSON plan.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fresh => "fresh",
            Self::Live => "live",
            Self::Interrupted => "interrupted",
            Self::Unknown => "unknown",
        }
    }
}

/// Why a patch on disk is not offered to the pass: something already
/// attempted it. The attempt state and the HELD ledger each disqualify on
/// their own — the pass must not offer a patch that a live attempt or a
/// HELD entry already owns. A bare branch is not among them: see [`Attempt`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Disqualification {
    /// A live attempt owns the issue: a branch with an open or merged pull
    /// request, or a checked-out worktree. When the liveness lookup could
    /// not complete, the candidate is cited here too — fail-closed.
    Attempted,
    /// A recorded HELD entry owns this issue and its re-gate still holds.
    Held,
}

impl Disqualification {
    /// The machine name used in reports and the JSON plan.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Attempted => "attempted",
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
/// attempted. The pass selects on the attempt state and the HELD re-gate;
/// the presence of the patch file is not one of them.
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
    /// The attempt state the branch carries (the default, `Fresh`, is the
    /// common case: most patches were never attempted).
    pub attempt: Attempt,
    /// A recorded HELD entry owns this issue and its re-gate still holds.
    pub held_recorded: bool,
    /// The patch's changes are already in the base: the work is delivered,
    /// the patch is residue (#4501). Never offered, never gated.
    pub delivered: bool,
    /// The issue is closed: there is no pending work, so the patch is
    /// residue — archived and its hold released, never re-gated forever
    /// (#4626). Checked before the attempt state, because a merged-PR branch
    /// on a closed issue is residue too, and `Live` would hide it behind a
    /// disqualifier instead of letting the pass archive it. `false` when the
    /// state could not be read: unknown never authorises acting.
    pub closed: bool,
    /// The patch's language class ([`crate::patch_language::classify`])
    /// from its file list. Only [`PatchLanguage::RustGo`] is offerable: the
    /// Rust gate cannot fail on a shell-only or a neither patch, so a green
    /// gate there means the gate did not read the patch (issue #4559).
    pub language: PatchLanguage,
}

/// The disqualifier for a candidate, or `None` when the patch should be
/// offered. A live attempt — and an attempt state the lookup could not
/// classify (fail-closed) — disqualifies; a recorded HELD entry disqualifies
/// on its own. A bare branch does not: [`Attempt::Interrupted`] is offered
/// again, which is what the push-before-PR window needs, because redoing an
/// interrupted attempt overwrites the orphan branch (#4499).
pub fn disqualification(attempt: Attempt, held_recorded: bool) -> Option<Disqualification> {
    if matches!(attempt, Attempt::Live | Attempt::Unknown) {
        Some(Disqualification::Attempted)
    } else if held_recorded {
        Some(Disqualification::Held)
    } else {
        None
    }
}

/// A patch held for its language, not its attempt state: it will never
/// convert, so the hold is terminal (archivable), not a re-gate queue entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LanguageHold {
    pub candidate: Candidate,
    pub language: PatchLanguage,
}

/// The result of selecting the fresh patches to attempt.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Selection {
    /// The patches to attempt this pass: those with no disqualifier and an
    /// offerable language.
    pub fresh: Vec<Candidate>,
    /// The offered patches whose attempt was interrupted — a branch exists
    /// with no open or merged PR. They are re-offered (redoing the attempt
    /// overwrites the orphan branch) and reported as a distinct category, so
    /// an interrupted run leaves a record of what it left behind (#4499).
    pub interrupted: Vec<Candidate>,
    /// The patches not offered by attempt state, each paired with the
    /// disqualifier that excluded it.
    pub disqualified: Vec<(Candidate, Disqualification)>,
    /// The patches not offered by language (issue #4559): shell-only, mixed,
    /// or neither — the gate cannot evaluate them, so they are held
    /// unevaluated, never gated, never branched.
    pub language_held: Vec<LanguageHold>,
    /// The patches whose changes are already in the base (issue #4501): the
    /// work is delivered, the patch is residue. Never offered, never gated —
    /// reported as their own category so a pending backlog and a delivered
    /// one read differently.
    pub delivered: Vec<Candidate>,
    /// The patches whose issue is closed (issue #4626): a hold is a claim
    /// that work is pending; a closed issue has no pending work, so the patch
    /// is residue — archived and its hold released under `--apply`, never
    /// re-gated forever. Reported as their own category so a pending backlog
    /// and a closed-issue one read differently.
    pub closed: Vec<Candidate>,
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
        // Delivered wins over everything: the changes are in the base, so
        // neither the attempt state nor a stale hold has anything left to
        // say about the patch (#4501). It is reported, not offered.
        if candidate.delivered {
            selection.delivered.push(Candidate {
                issue: candidate.issue,
                patch_key: candidate.patch_key.clone(),
            });
            continue;
        }
        // A closed issue owns its patch next: the work it claimed is over,
        // so neither the attempt state (a merged PR is residue, not a live
        // attempt) nor a recorded hold (re-gating it is the waste #4626
        // measured) has anything left to say about it. Delivered is checked
        // first: for a closed issue whose changes are in the base, "the work
        // is in the base" is the more specific fact.
        if candidate.closed {
            selection.closed.push(Candidate {
                issue: candidate.issue,
                patch_key: candidate.patch_key.clone(),
            });
            continue;
        }
        // Attempt state wins: the attempt fact is cited before the language
        // verdict, which costs a patch read. A live-attempt shell patch is
        // reported as attempted; an interrupted one is re-offered and named.
        match disqualification(candidate.attempt, candidate.held_recorded) {
            None if candidate.language.offerable() => {
                let offered = Candidate {
                    issue: candidate.issue,
                    patch_key: candidate.patch_key.clone(),
                };
                if candidate.attempt == Attempt::Interrupted {
                    selection.interrupted.push(offered.clone());
                }
                selection.fresh.push(offered);
            }
            None => selection.language_held.push(LanguageHold {
                candidate: Candidate {
                    issue: candidate.issue,
                    patch_key: candidate.patch_key.clone(),
                },
                language: candidate.language,
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

    /// The patches whose changes are already in the base (#4501).
    pub fn delivered_count(&self) -> usize {
        self.delivered.len()
    }

    /// The patches whose issue is closed: residue to be archived, not work
    /// to be re-gated (#4626).
    pub fn closed_count(&self) -> usize {
        self.closed.len()
    }

    /// The not-offered patches grouped by disqualifier, in attempted → held
    /// order. The counts reconcile against the number examined:
    /// `fresh + disqualified == examined`.
    pub fn disqualified_counts(&self) -> [(Disqualification, usize); 2] {
        let count = |want: Disqualification| {
            self.disqualified
                .iter()
                .filter(|(_, reason)| *reason == want)
                .count()
        };
        [
            (
                Disqualification::Attempted,
                count(Disqualification::Attempted),
            ),
            (Disqualification::Held, count(Disqualification::Held)),
        ]
    }

    /// The not-offered patches grouped by language, in shell → mixed →
    /// neither order. Mixed is counted on its own deliberately: an agent
    /// asked for Rust that produced shell is a prompt signal, not just
    /// another hold (issue #4559).
    pub fn language_counts(&self) -> [(PatchLanguage, usize); 3] {
        let count = |want: PatchLanguage| {
            self.language_held
                .iter()
                .filter(|hold| hold.language == want)
                .count()
        };
        [
            (PatchLanguage::Shell, count(PatchLanguage::Shell)),
            (PatchLanguage::Mixed, count(PatchLanguage::Mixed)),
            (PatchLanguage::Neither, count(PatchLanguage::Neither)),
        ]
    }

    /// The one-line plan summary: how many were examined, how many are fresh
    /// (and how many of those re-offer an interrupted attempt), and how each
    /// disqualifier and language class accounted for the rest. `examined` is
    /// the size of the input the pass was handed — a zero fresh count read
    /// against it is what keeps an idle pass distinct from a broken one, and
    /// a pass that held everything must not print the idle counters
    /// (issue #4559). `delivered` and `closed` name the two residue
    /// categories on their own: a backlog of finished work and a backlog of
    /// closed issues must not read as pending work.
    pub fn line(&self, examined: usize) -> String {
        let [(attempted, attempted_n), (held, held_n)] = self.disqualified_counts();
        let [(shell, shell_n), (mixed, mixed_n), (neither, neither_n)] = self.language_counts();
        format!(
            "conversion pass: examined={examined} fresh={} interrupted={} delivered={} closed={} \
             ({} {} {} {}; language: {} {} {} {} {} {})",
            self.fresh.len(),
            self.interrupted.len(),
            self.delivered.len(),
            self.closed.len(),
            attempted_n,
            attempted.as_str(),
            held_n,
            held.as_str(),
            shell_n,
            shell.as_str(),
            mixed_n,
            mixed.as_str(),
            neither_n,
            neither.as_str(),
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
