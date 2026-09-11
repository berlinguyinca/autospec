//! Self-gating of gatekeeping automation (issue #4263).
//!
//! Three defects in the operational tooling:
//!
//! 1. **The watchdog reaped in queue order under a cap.** The reaper walked
//!    `squeue` output in list order and killed up to `MAX_KILLS=5`. Two
//!    agents at 4h01m and 3h51m survived three sweeps while younger
//!    offenders were killed in each — the cap was consumed by whoever came
//!    first in the list, and the worst offenders were never at the front.
//! 2. **The conversion selector keyed "already attempted" on the issue.**
//!    A new patch for an issue whose earlier patch had been attempted was
//!    never selected: the attempt was a fact about the patch it was made
//!    on, not a verdict on the issue.
//! 3. **The same selector counted issue directories, not patches.**
//!    Directories are created when an agent *starts*; patches are what an
//!    agent may never produce. The selector reported "15" when there were
//!    11 issues and 14 patches — the denominator was the wrong unit, and
//!    nobody could have caught it, because the selection predicate lived
//!    in a one-line `jq` in a script that died with the session.
//!
//! The common fact: the gatekeeping automation — the code that decides
//! what the system does — was the only code in the system with no gates of
//! its own. No dry run before the first production run, no denominator
//! reported, no file a reviewer could read, no test. It was "the only code
//! that was never wrong" because it had never been checked.
//!
//! Five rules, each checkable:
//!
//! 1. **Destructive scripts need a dry-run mode, used before the first
//!    production run** — [`gate_destructive_run`].
//! 2. **Selection predicates report their denominator and are reviewable
//!    as a file** — [`SelectionReport`].
//! 3. **Automation that decides which work to run is itself work**: it
//!    lives in a file, every condition carries a comment naming the bug it
//!    prevents, and one fixture test exercises it — [`AutomationArtifact`].
//! 4. **The blast radius of destructive automation is capped, and a
//!    deferred offender is named, never silently skipped** —
//!    [`select_reaps`], [`ReapPlan`].
//! 5. **Recency is not reliability.** "I wrote this helper an hour ago and
//!    it worked once" is evidence the code is *more* likely to be wrong
//!    (new, unexercised by the edge cases), never a reason to relax a
//!    gate. The only evidence that justifies trusting automation is that
//!    it is gated — [`trust_verdict`].
//!
//! Defects 1–3 above bind the rules to concrete primitives:
//! [`select_reaps`] (defect 1), [`AttemptLedger`] (defect 2) and
//! [`WorkState::is_conversion_candidate`] (defect 3).

use std::collections::BTreeSet;
use std::fmt::Write as _;

// ── Rule 1: dry-run gate ────────────────────────────────────────────────────

/// How a destructive script is being invoked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunMode {
    /// The script is running in dry-run mode: it reports what it would do
    /// and changes nothing. A dry run is always allowed — it *is* the gate.
    DryRun,
    /// The script is running for real.
    Production,
}

/// The outcome of gating a run of a destructive script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateVerdict {
    /// The run may proceed.
    Proceed,
    /// The run is refused; `reason` names the missing piece — the refusal
    /// is both the fix and the diagnosis (who reads it knows which file to
    /// edit or which command to run).
    Refused { reason: String },
}

/// Rule 1: a destructive script needs a dry-run mode, and the first
/// production run must be preceded by a dry run.
///
/// `has_dry_run_flag` is a fact about the script's source (it declares a
/// `--dry-run` mode); `dry_run_performed` is a fact about this deployment
/// (someone ran it in dry-run mode and reviewed the output). Neither may be
/// assumed from the other: a script that has no dry-run mode can never
/// satisfy the gate, and a script that has one still owes the operator the
/// dry run before it acts on live work.
pub fn gate_destructive_run(
    has_dry_run_flag: bool,
    mode: RunMode,
    dry_run_performed: bool,
) -> GateVerdict {
    match mode {
        // The dry run is the gate; it is never refused for being first.
        RunMode::DryRun => GateVerdict::Proceed,
        RunMode::Production => {
            if !has_dry_run_flag {
                return GateVerdict::Refused {
                    reason: "script declares no --dry-run mode; a destructive script must offer one before it may run in production"
                        .to_string(),
                };
            }
            if !dry_run_performed {
                return GateVerdict::Refused {
                    reason: "dry run never performed; run the script with --dry-run first and review what it would do"
                        .to_string(),
                };
            }
            GateVerdict::Proceed
        }
    }
}

// ── Rule 2: denominator and reviewable-as-file ─────────────────────────────

/// A selection predicate's report of what it picked and out of what.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectionReport {
    /// What was selected.
    pub numerator: usize,
    /// The population the predicate inspected — the unit it actually
    /// iterated, not a neighbouring quantity that happens to be close.
    pub denominator: usize,
    /// The report was written to a file: an artefact a reviewer can read
    /// after the process that computed it is gone.
    pub written_to_file: bool,
}

impl SelectionReport {
    /// The report line. The denominator always appears — a bare "selected
    /// 3" is a numerator without a base, and a base-less count is the
    /// quantity that was "15 when there were 11 issues and 14 patches".
    pub fn line(&self) -> String {
        format!("{} of {} selected", self.numerator, self.denominator)
    }

    /// Findings over the two halves of the rule.
    pub fn findings(&self) -> Vec<String> {
        let mut findings = Vec::new();
        if self.numerator > self.denominator {
            findings.push(format!(
                "selection report claims {} selected out of {} inspected — more were picked than exist; the predicate is counting the wrong unit or double-counting",
                self.numerator, self.denominator
            ));
        }
        if !self.written_to_file {
            findings.push(
                "selection report was never written to a file; the denominator is not reviewable after the run"
                    .to_string(),
            );
        }
        findings
    }
}

// ── Rule 3: automation that decides work is itself work ────────────────────

/// One condition in a selection predicate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Condition {
    /// What the condition selects on (e.g. `patch exists`, `not attempted`).
    pub name: String,
    /// The comment naming the bug this condition prevents. A condition
    /// without one is a filter whose absence nobody would notice — the
    /// comment is the record of why the line exists at all.
    pub bug_comment: Option<String>,
}

/// Rule 3: automation that decides which work to run is itself work, and
/// work gets the gates work gets: a home in a file, a reason per condition,
/// a test.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutomationArtifact {
    /// The file the predicate lives in. `None` means inline — a one-line
    /// `jq` in a wrapper script that dies with the session.
    pub file: Option<String>,
    /// The predicate's conditions, each of which must name the bug it
    /// prevents.
    pub conditions: Vec<Condition>,
    /// One fixture test exercises the predicate against recorded input.
    pub fixture_test: bool,
}

impl AutomationArtifact {
    /// Findings over the three gates of the rule.
    pub fn findings(&self) -> Vec<String> {
        let mut findings = Vec::new();
        match &self.file {
            None => findings.push(
                "selection predicate lives inline, not in a file; it is not reviewable, not testable, and dies with the session"
                    .to_string(),
            ),
            Some(path) => {
                for condition in &self.conditions {
                    if condition.bug_comment.is_none() {
                        findings.push(format!(
                            "condition '{name}' in {path} has no comment naming the bug it prevents",
                            name = condition.name
                        ));
                    }
                }
            }
        }
        if !self.fixture_test {
            findings.push(
                "selection predicate has no fixture test; one test over recorded input is the minimum that makes a wrong denominator a failure instead of a surprise"
                    .to_string(),
            );
        }
        findings
    }
}

// ── Rule 4 + defect 1: capped reaping in severity order ────────────────────

/// One offender past its limit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReapCandidate {
    /// The offender's identity (agent id, job id).
    pub id: String,
    /// How far past the limit the offender is, in seconds.
    pub over_by_secs: u64,
}

/// The result of one reap sweep under a cap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReapPlan {
    /// Who is reaped this sweep: the worst offenders, up to the cap.
    pub selected: Vec<ReapCandidate>,
    /// Who survives this sweep *because the cap filled up before them*.
    /// They are reported, never silently skipped — a deferred offender is
    /// still an offender, and the operator must see that the cap, not the
    /// detector, is what is letting them run.
    pub deferred: Vec<ReapCandidate>,
}

/// Rule 4 with defect 1 bound to it: a capped reaper selects the *worst*
/// offenders first, not the first in the list.
///
/// Queue order is not severity order. Under a cap, reaping in list order
/// starves whoever sits past the cap in the list: with cap 5 and seven
/// offenders, the two worst at list positions 6 and 7 survive every sweep
/// while the younger offenders in front of them are killed again and again.
/// The cap does not make the reaper gentle; it makes the reaper's *ordering*
/// load-bearing.
///
/// A cap of zero is a refusal, not a no-op: a destructive reaper with no
/// cap to fill is a configuration the operator should see, not a silent
/// pass.
pub fn select_reaps(candidates: &[ReapCandidate], cap: usize) -> Result<ReapPlan, String> {
    if cap == 0 {
        return Err(
            "reap cap is zero; a destructive reaper with nothing it may do is a configuration to surface, not a silent pass — raise the cap or stop the reaper"
                .to_string(),
        );
    }
    let mut ordered: Vec<ReapCandidate> = candidates.iter().cloned().collect();
    // Worst first: most seconds over the limit; ties broken by id so the
    // plan is deterministic for a given queue.
    ordered.sort_by(|a, b| {
        b.over_by_secs
            .cmp(&a.over_by_secs)
            .then_with(|| a.id.cmp(&b.id))
    });
    let take = cap.min(ordered.len());
    Ok(ReapPlan {
        deferred: ordered.split_off(take),
        selected: ordered[..take].to_vec(),
    })
}

impl ReapPlan {
    /// The report line. The denominator is the full offender population
    /// (selected + deferred), and every deferred offender is named.
    pub fn line(&self) -> String {
        let mut line = format!(
            "reaping {} of {} offenders",
            self.selected.len(),
            self.selected.len() + self.deferred.len()
        );
        if !self.deferred.is_empty() {
            let names: Vec<&str> = self.deferred.iter().map(|c| c.id.as_str()).collect();
            let _ = write!(
                line,
                " ({} deferred by the cap: {})",
                names.len(),
                names.join(", ")
            );
        }
        line
    }

    /// The offender ids reaped this sweep, worst first.
    pub fn selected_ids(&self) -> Vec<&str> {
        self.selected.iter().map(|c| c.id.as_str()).collect()
    }
}

// ── Defect 2: the attempt ledger is keyed by patch ─────────────────────────

/// Defect 2: "already attempted" is a fact about the *patch*, not the
/// issue.
///
/// The ledger records patch ids only. An issue whose first patch was
/// attempted still has a fresh, unattempted second patch — and that second
/// patch is a conversion candidate. Keying the attempt on the issue turns
/// one failed attempt into a permanent exclusion of the issue's whole
/// future, which is a verdict the attempt never earned.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AttemptLedger {
    attempted: BTreeSet<String>,
}

impl AttemptLedger {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record an attempt made on a patch.
    pub fn record_attempt(&mut self, patch_id: &str) {
        self.attempted.insert(patch_id.to_string());
    }

    /// A patch is attempted exactly when *that patch* was attempted.
    pub fn is_attempted(&self, patch_id: &str) -> bool {
        self.attempted.contains(patch_id)
    }

    /// The number of distinct patches attempted.
    pub fn attempted_count(&self) -> usize {
        self.attempted.len()
    }

    /// The selector line. The denominator is reported in **patches** —
    /// the unit the predicate actually inspects — with the issue count
    /// alongside, because conflating the two units is how the denominator
    /// was "15" when there were 11 issues and 14 patches.
    pub fn selector_line(&self, all_patches: &[&str], issue_count: usize) -> String {
        let attempted = all_patches.iter().filter(|p| self.is_attempted(p)).count();
        format!(
            "attempted {attempted} of {} patches ({} issues)",
            all_patches.len(),
            issue_count
        )
    }
}

// ── Defect 3: a conversion candidate is a patch, not a directory ───────────

/// Defect 3: what a piece of work has produced.
///
/// A working directory exists in every state — it is created when the
/// agent *starts*. A patch exists only when the agent has produced
/// something convertible. A selector that counts directories counts work
/// that may never produce output; the candidate unit is the patch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkState {
    /// The agent created a working directory; no output yet.
    InProgress { directory: String },
    /// A patch file exists on top of the directory, not yet converted.
    HasPatch { directory: String, patch: String },
    /// The patch was already converted; nothing left to select.
    Converted { directory: String, patch: String },
}

impl WorkState {
    /// The working directory. Present in every state — which is exactly
    /// why it cannot be the candidate unit.
    pub fn directory(&self) -> &str {
        match self {
            WorkState::InProgress { directory }
            | WorkState::HasPatch { directory, .. }
            | WorkState::Converted { directory, .. } => directory,
        }
    }

    /// The patch, if the work has produced one.
    pub fn patch(&self) -> Option<&str> {
        match self {
            WorkState::InProgress { .. } => None,
            WorkState::HasPatch { patch, .. } | WorkState::Converted { patch, .. } => {
                Some(patch.as_str())
            }
        }
    }

    /// A conversion candidate is a patch that exists and has not been
    /// converted. `InProgress` has a directory but no output; `Converted`
    /// has output but it is already done. Only `HasPatch` is both.
    pub fn is_conversion_candidate(&self) -> bool {
        matches!(self, WorkState::HasPatch { .. })
    }
}

// ── Rule 5: recency is not reliability ─────────────────────────────────────

/// What is being offered as the reason to trust a piece of automation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReliabilityEvidence {
    /// The automation is gated: dry-run mode, denominator reported, lives
    /// in a file with per-condition comments, capped, fixture-tested.
    Gated,
    /// The automation has run successfully `times` times, recently.
    RanRecently { times: u32 },
}

/// Rule 5: the only evidence that justifies trusting automation is that it
/// is gated.
///
/// "Worked once an hour ago" is evidence the code is *more* likely to be
/// wrong — it is new, and the edge cases are exactly what one happy run
/// does not exercise. A thousand successful runs on the happy path are the
/// same evidence at a larger number: still no edge case, still no gate.
/// No amount of recent success is a trust input; the gates are the trust
/// inputs.
pub fn trust_verdict(evidence: ReliabilityEvidence) -> bool {
    matches!(evidence, ReliabilityEvidence::Gated)
}
