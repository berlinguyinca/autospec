//! A correction applies to every instance of the method, not just where the
//! flaw was found (issue #4428).
//!
//! The incident: a conversion gate could not judge a patch by pass/fail on a
//! red main, so the gate was rebuilt as a baseline-diff gate — compare the
//! patch's failing-test set against main's. The correction was applied to
//! `autospec-cli`, the crate where the red main had been discovered. For
//! `autospec-core` the gate kept pass/fail, having never asked whether that
//! crate was also red. It was: `runner_executes_the_newly_registered_bats_suites`
//! fails on main. Three patches — 4070, 4015 and 4094 — were recorded HELD
//! with that test named as their failure. All three were innocent; re-gated
//! against the measured baseline, all three passed and are merged.
//!
//! The held reasons were written confidently and were wrong, and they would
//! have stayed wrong: a HELD entry is not re-examined, so the patches would
//! have sat indefinitely with a plausible-looking explanation attached.
//!
//! The fix was built as a *response to a discovered problem* rather than as
//! a *correct method*. The question "is pass/fail ever a valid gate against
//! a repository whose main is not green?" was never asked, and its answer is
//! no — for any crate. Same session, same shape: a throughput floor was
//! corrected three times, each fix addressing the instance in front rather
//! than the class (#4411).
//!
//! The invariants, checkable here:
//!
//! 1. **A correction applies to every instance of the method, not the
//!    instance where the flaw was found.** After fixing a check, enumerate
//!    the other call sites and apply it there before moving on
//!    ([`CorrectionAudit`]). The instance that revealed the bug is rarely
//!    the only one affected — it is just the one that happened to be
//!    noticed.
//! 2. **Establish the baseline for every crate a gate runs against, at the
//!    start of the pass** ([`GatePass`]). "Main is green here" is a
//!    measurement, never an assumption: a crate without a measured baseline
//!    cannot be judged at all ([`Judgment::NoBaseline`]) — neither admitted
//!    nor held.
//! 3. **Pass/fail is never a valid gate on a red main** ([`judge`]). On a
//!    crate whose measured main fails, the only valid judgment is the
//!    baseline diff: the patch's new failures are `patch − main`. A pass/fail
//!    run over a red crate is an invalid judgment, whatever it printed.
//! 4. **A HELD entry is examined against the measured baseline, or not at
//!    all** ([`reexamine_held`]). A hold whose cited failures all fail on
//!    main was main's, not the patch's — the entry is re-gated, not kept.
//!    An entry on a crate with no measured baseline cannot be examined: it
//!    keeps its explanation — possibly wrong — until the baseline exists,
//!    which is the incident's failure mode.
//!
//! Everything here is pure: the caller supplies the measured baselines and
//! the patch failure sets; this module decides what each judgment is and
//! what a report line says.

use std::collections::{BTreeMap, BTreeSet};

/// The measured state of main on one crate: a measurement, never an
/// assumption.
///
/// There is no "presumed green" variant on purpose: the absence of a
/// [`MainStatus`] in a [`GatePass`] *is* the "never measured" state, and
/// judging on it is [`Judgment::NoBaseline`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MainStatus {
    /// Measured: main's full suite passed.
    Green,
    /// Measured: main's suite failed; the set of failing test ids.
    Red { failing: BTreeSet<String> },
}

impl MainStatus {
    /// Classify a measured run: an empty failing set is green, anything
    /// else red. A run that never happened cannot be classified — that is
    /// the absence of a `MainStatus`, not a value of it.
    pub fn from_failures(failing: impl IntoIterator<Item = impl Into<String>>) -> Self {
        let failing: BTreeSet<String> = failing.into_iter().map(Into::into).collect();
        if failing.is_empty() {
            Self::Green
        } else {
            Self::Red { failing }
        }
    }

    /// Whether the measured main fails.
    pub fn is_red(&self) -> bool {
        matches!(self, Self::Red { .. })
    }

    /// The measured failing set (empty for a green main).
    pub fn failing_set(&self) -> &BTreeSet<String> {
        match self {
            Self::Green => &EMPTY_SET,
            Self::Red { failing } => failing,
        }
    }

    /// One-word label for report lines: `green` or `red (n failing)`.
    pub fn label(&self) -> String {
        match self {
            Self::Green => "green".to_string(),
            Self::Red { failing } => format!("red ({} failing)", failing.len()),
        }
    }
}

static EMPTY_SET: BTreeSet<String> = BTreeSet::new();

/// How a patch's test run is judged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JudgmentMethod {
    /// Pass/fail: any failure holds the patch. Valid only on a measured
    /// green main — on a red main it holds the innocent, which is the
    /// incident.
    PassFail,
    /// Baseline diff: the patch's failures are compared against main's
    /// measured failing set; only `patch − main` can hold the patch.
    BaselineDiff,
}

/// A gate pass over a set of crates, with baselines measured at the start.
///
/// The pass's scope — every crate the gate runs against — is what the
/// baseline must cover. A baseline for a crate outside the pass is refused
/// ([`GatePass::establish_baseline`]): a measurement keyed on a crate the
/// gate never runs is a measurement nobody reads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GatePass {
    /// Every crate the gate runs against.
    crates: BTreeSet<String>,
    /// Measured main status per crate. A missing entry means the baseline
    /// was never measured — not that main is green.
    baselines: BTreeMap<String, MainStatus>,
}

impl GatePass {
    /// A pass over the given crates, with no baselines measured yet.
    pub fn new(crates: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            crates: crates.into_iter().map(Into::into).collect(),
            baselines: BTreeMap::new(),
        }
    }

    /// Record the measured main status for one crate in the pass.
    ///
    /// Refuses a crate outside the pass's scope: baselines are keyed on the
    /// pass, and a key the pass never reads is a silent hole.
    pub fn establish_baseline(&mut self, crate_name: &str, main: MainStatus) -> Result<(), String> {
        if !self.crates.contains(crate_name) {
            return Err(format!(
                "crate '{crate_name}' is not in the pass's scope (the gate runs against: {})",
                self.crates.iter().cloned().collect::<Vec<_>>().join(", ")
            ));
        }
        self.baselines.insert(crate_name.to_string(), main);
        Ok(())
    }

    /// The measured baseline for one crate, if it was measured.
    pub fn baseline(&self, crate_name: &str) -> Option<&MainStatus> {
        self.baselines.get(crate_name)
    }

    /// Crates the gate runs against with no measured baseline, sorted.
    ///
    /// Invariant 2: this list must be empty before any patch is judged.
    pub fn unbaselined(&self) -> Vec<String> {
        self.crates
            .iter()
            .filter(|c| !self.baselines.contains_key(*c))
            .cloned()
            .collect()
    }

    /// One line stating the baseline coverage of the pass.
    pub fn coverage_line(&self) -> String {
        let n = self.crates.len();
        let k = self.baselines.len();
        if k == n {
            format!(
                "OK: baseline coverage {k} of {n} crate(s) measured at the start of the pass: {}",
                self.crates.iter().cloned().collect::<Vec<_>>().join(", ")
            )
        } else {
            format!(
                "FAIL: baseline coverage {k} of {n} crate(s) measured at the start of the pass; unbaselined: {} — \"main is green here\" is an assumption, not a measurement",
                self.unbaselined().join(", ")
            )
        }
    }
}

/// The outcome of judging one patch on one crate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Judgment {
    /// No new failures against the measured main: the patch may proceed.
    Admitted { crate_name: String },
    /// The patch is held by failures it introduced itself (`patch − main`).
    Held {
        crate_name: String,
        new_failures: BTreeSet<String>,
    },
    /// The crate has no measured baseline: the patch is neither admitted
    /// nor held — the harness did not prepare the run. A harness fault,
    /// not a property of the patch.
    NoBaseline { crate_name: String },
    /// Pass/fail was applied to a crate whose measured main is red: the
    /// judgment is invalid, whatever the run printed.
    InvalidJudgment {
        crate_name: String,
        main_failures: BTreeSet<String>,
    },
}

impl Judgment {
    /// One line for a gate log, a closeout, or a review.
    pub fn line(&self) -> String {
        match self {
            Self::Admitted { crate_name } => {
                format!("OK: {crate_name} admitted — no new failures against the measured main")
            }
            Self::Held {
                crate_name,
                new_failures,
            } => format!(
                "HOLD: {crate_name} held by {} new failure(s) against the measured main: {}",
                new_failures.len(),
                new_failures.iter().cloned().collect::<Vec<_>>().join(", ")
            ),
            Self::NoBaseline { crate_name } => format!(
                "FAIL: {crate_name} has no measured baseline — \"main is green here\" is an assumption, not a measurement; the patch is neither admitted nor held (harness fault, not a property of the patch)"
            ),
            Self::InvalidJudgment {
                crate_name,
                main_failures,
            } => format!(
                "FAIL: {crate_name} judged by pass/fail on a red main ({} failing there: {}) — pass/fail cannot judge a patch on a red main, for any crate; re-gate by baseline diff",
                main_failures.len(),
                main_failures.iter().cloned().collect::<Vec<_>>().join(", ")
            ),
        }
    }
}

/// Judge one patch on one crate against the pass's measured baseline.
///
/// Invariants 2 and 3: a crate with no measured baseline is
/// [`Judgment::NoBaseline`] under *either* method — green is never assumed.
/// On a measured red main, `PassFail` is [`Judgment::InvalidJudgment`]
/// regardless of what the patch's run printed; only the baseline diff
/// admits or holds, and only on `patch − main`. On a measured green main
/// both methods are valid and coincide: `main`'s failing set is empty, so
/// every patch failure is new.
pub fn judge(
    pass: &GatePass,
    crate_name: &str,
    method: JudgmentMethod,
    patch_failures: &BTreeSet<String>,
) -> Judgment {
    match pass.baseline(crate_name) {
        None => Judgment::NoBaseline {
            crate_name: crate_name.to_string(),
        },
        Some(MainStatus::Green) => {
            if patch_failures.is_empty() {
                Judgment::Admitted {
                    crate_name: crate_name.to_string(),
                }
            } else {
                Judgment::Held {
                    crate_name: crate_name.to_string(),
                    new_failures: patch_failures.clone(),
                }
            }
        }
        Some(MainStatus::Red { failing }) => match method {
            JudgmentMethod::PassFail => Judgment::InvalidJudgment {
                crate_name: crate_name.to_string(),
                main_failures: failing.clone(),
            },
            JudgmentMethod::BaselineDiff => {
                let new_failures: BTreeSet<String> =
                    patch_failures.difference(failing).cloned().collect();
                if new_failures.is_empty() {
                    Judgment::Admitted {
                        crate_name: crate_name.to_string(),
                    }
                } else {
                    Judgment::Held {
                        crate_name: crate_name.to_string(),
                        new_failures,
                    }
                }
            }
        },
    }
}

/// A correction that touched some instances of a method (invariant 1):
/// the set it fixed, against the set of all instances of the method.
///
/// The set of all instances is supplied by the caller's enumeration — the
/// text search, the crate list, the call-site scan. This struct cannot
/// enumerate for you; it can only show you what you enumerated against
/// what you fixed, and name the gap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorrectionAudit {
    all: BTreeSet<String>,
    fixed: BTreeSet<String>,
}

impl CorrectionAudit {
    /// Audit a correction: `fixed` instances against `all` instances.
    pub fn new(
        all: impl IntoIterator<Item = impl Into<String>>,
        fixed: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            all: all.into_iter().map(Into::into).collect(),
            fixed: fixed.into_iter().map(Into::into).collect(),
        }
    }

    /// Instances the correction has not reached, sorted. Non-empty means
    /// the correction is not done: apply it there before moving on.
    pub fn unfixed(&self) -> Vec<String> {
        self.all.difference(&self.fixed).cloned().collect()
    }

    /// Whether the correction reached every instance of the method.
    pub fn complete(&self) -> bool {
        self.unfixed().is_empty()
    }

    /// One line stating the correction's coverage.
    pub fn line(&self) -> String {
        if self.complete() {
            format!(
                "OK: correction applied to all {} instance(s) of the method: {}",
                self.all.len(),
                self.all.iter().cloned().collect::<Vec<_>>().join(", ")
            )
        } else {
            format!(
                "FAIL: correction applied to {} of {} instance(s); unfixed: {} — a correction applies to every instance of the method, not just where the flaw was found",
                self.fixed.len(),
                self.all.len(),
                self.unfixed().join(", ")
            )
        }
    }
}

/// A HELD entry: a patch recorded as held with named failing tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldEntry {
    /// The patch identifier (e.g. `"4070"`).
    pub patch: String,
    /// The crate the hold was recorded under.
    pub crate_name: String,
    /// The failing tests the hold names as its reason.
    pub cited_failures: BTreeSet<String>,
}

/// The outcome of re-examining one HELD entry against the measured
/// baseline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReexamineVerdict {
    /// Every cited failure fails on the measured main: the hold was
    /// main's, not the patch's. Re-gated; the entry is dropped.
    Innocent {
        patch: String,
        crate_name: String,
        /// The main failures the hold had been attributed to.
        main_failures: BTreeSet<String>,
    },
    /// At least one cited failure does not fail on the measured main: the
    /// hold stands, now attributed to the failures the patch introduced.
    /// An entry that cites no failure also lands here: a hold with no
    /// named failure cannot be exonerated by a baseline.
    StillHeld {
        patch: String,
        crate_name: String,
        /// The cited failures that are the patch's, not main's.
        new_failures: BTreeSet<String>,
    },
    /// The crate has no measured baseline: the entry cannot be examined.
    /// It keeps its explanation — possibly wrong — until the baseline
    /// exists.
    NoBaseline { patch: String, crate_name: String },
}

impl ReexamineVerdict {
    /// One line for the re-examination report.
    pub fn line(&self) -> String {
        match self {
            Self::Innocent {
                patch,
                crate_name,
                main_failures,
            } => format!(
                "OK: patch {patch} ({crate_name}) re-gated against the measured main baseline — every cited failure ({}) fails on main; the hold was main's, not the patch's",
                main_failures.iter().cloned().collect::<Vec<_>>().join(", ")
            ),
            Self::StillHeld {
                patch,
                crate_name,
                new_failures,
            } => format!(
                "HOLD: patch {patch} ({crate_name}) still held — {} new failure(s): {}",
                new_failures.len(),
                new_failures.iter().cloned().collect::<Vec<_>>().join(", ")
            ),
            Self::NoBaseline { patch, crate_name } => format!(
                "FAIL: patch {patch} ({crate_name}) cannot be re-examined without a measured baseline — the held reason keeps standing, possibly wrong, until the baseline exists"
            ),
        }
    }
}

/// Re-examine HELD entries against the pass's measured baselines
/// (invariant 4).
///
/// The attribution is set-theoretic against the measured main: a cited
/// failure that fails on main is main's, one that does not is the patch's.
/// An entry whose citations are all main's is innocent — the incident's
/// three patches (4070, 4015, 4094) all land here once core's baseline is
/// measured. An entry on a crate with no measured baseline is
/// [`ReexamineVerdict::NoBaseline`], not guessed either way.
pub fn reexamine_held(entries: &[HeldEntry], pass: &GatePass) -> Vec<ReexamineVerdict> {
    entries
        .iter()
        .map(|e| match pass.baseline(&e.crate_name) {
            None => ReexamineVerdict::NoBaseline {
                patch: e.patch.clone(),
                crate_name: e.crate_name.clone(),
            },
            Some(main) => {
                let main_failures = main.failing_set();
                let new_failures: BTreeSet<String> = e
                    .cited_failures
                    .iter()
                    .filter(|f| !main_failures.contains(*f))
                    .cloned()
                    .collect();
                if e.cited_failures.is_empty() {
                    ReexamineVerdict::StillHeld {
                        patch: e.patch.clone(),
                        crate_name: e.crate_name.clone(),
                        new_failures: BTreeSet::new(),
                    }
                } else if new_failures.is_empty() {
                    ReexamineVerdict::Innocent {
                        patch: e.patch.clone(),
                        crate_name: e.crate_name.clone(),
                        main_failures: main_failures.clone(),
                    }
                } else {
                    ReexamineVerdict::StillHeld {
                        patch: e.patch.clone(),
                        crate_name: e.crate_name.clone(),
                        new_failures,
                    }
                }
            }
        })
        .collect()
}
