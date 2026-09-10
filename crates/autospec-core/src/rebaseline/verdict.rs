//! What a verdict is actually worth (issue #3708).
//!
//! Two measurements, two judgements. A patch that applies cleanly onto a
//! trunk it did not run against is stale, not conflicted ([`Staleness`]), and
//! a `HELD` recorded against a trunk that has since moved is an experiment
//! that is due, not a decision that was made ([`hold_status`]). The last
//! section is the rate that made both of them necessary: how often the base
//! moves relative to the length of a run ([`DriftExposure`]).

use crate::rebaseline::{BaseDrift, MeasureError};

// ---------------------------------------------------------------------------
// Staleness: stale-but-clean is not a conflict
// ---------------------------------------------------------------------------

/// What a drift measurement plus an apply attempt actually says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Staleness {
    /// The base is the tip. There is no drift to blame: a patch that does not
    /// apply here does not apply, and the hold is a decision.
    Current,
    /// The trunk moved and the patch still applies — or the only files that
    /// conflict are files the patch does not touch. This is staleness, not
    /// conflict: re-baseline mechanically, do not re-dispatch the run.
    StaleButClean,
    /// The trunk moved and the conflict is inside the patch's own subject
    /// matter. The hold is a decision; the work has to be redone against the
    /// new trunk, not replayed onto it.
    SubjectMatterConflict { files: Vec<String> },
}

/// Distinguishes "stale but clean" from "conflicts in its own subject
/// matter" — the distinction #3688 asks for, and the one a hold cannot make
/// for itself, because a hold records only that the patch did not apply.
///
/// `conflicting_files` are the files the apply attempt reported; an empty
/// list means the patch applies. `touched_files` are the files the patch
/// itself changes.
pub fn classify_staleness(
    drift: &BaseDrift,
    conflicting_files: &[String],
    touched_files: &[String],
) -> Staleness {
    if drift.at_tip() {
        return Staleness::Current;
    }
    let touched: Vec<&str> = touched_files.iter().map(String::as_str).collect();
    let shared: Vec<String> = conflicting_files
        .iter()
        .filter(|file| touched.contains(&file.as_str()))
        .cloned()
        .collect();
    if shared.is_empty() {
        Staleness::StaleButClean
    } else {
        Staleness::SubjectMatterConflict { files: shared }
    }
}

// ---------------------------------------------------------------------------
// A HELD is a hypothesis, not a decision
// ---------------------------------------------------------------------------

/// A hold older than this many trunk advances is a hypothesis, not a
/// decision ([`hold_status`]). Three is the smallest number that separates
/// "the trunk moved once while I was deciding" from "the trunk moved several
/// times and every one of them was a chance for this hold to become true by
/// accident". Callers may pass a different threshold; this is the default.
pub const DEFAULT_HELD_HYPOTHESIS_COMMITS: u64 = 3;

/// Whether a recorded hold is still a decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldStatus {
    /// The hold stands: re-affirming it would be the whole of the re-test, so
    /// it is kept as the verdict it is.
    Decision,
    /// The trunk has moved at least `threshold` commits since the hold was
    /// recorded. The hold was true of a tree that no longer exists: it is a
    /// hypothesis, and the cheap action is to re-run the test, not to re-cite
    /// the verdict.
    Hypothesis {
        /// Trunk advances since the hold was recorded.
        commits_stale: u64,
        /// The threshold it was graded against.
        threshold: u64,
    },
}

/// Grades a hold against the trunk's movement since it was recorded.
///
/// A hold re-cited instead of re-tested is the failure this rule exists for:
/// the session in #3708 held ten patches whose base had already moved on, and
/// the second `does not apply` costs the same GPU run as the first while
/// proving strictly less.
pub fn hold_status(
    hold_tip: &str,
    current_tip: &str,
    commits_since: u64,
    threshold: u64,
) -> Result<HoldStatus, MeasureError> {
    let drift = BaseDrift::measure(hold_tip, current_tip, commits_since)?;
    if drift.commits_behind > 0 && drift.commits_behind >= threshold {
        Ok(HoldStatus::Hypothesis {
            commits_stale: drift.commits_behind,
            threshold,
        })
    } else {
        Ok(HoldStatus::Decision)
    }
}

// ---------------------------------------------------------------------------
// How fast the base moves relative to a run
// ---------------------------------------------------------------------------

/// Whether a run finishes before the trunk moves under it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExposureClass {
    /// The trunk moves far less than once per run: drift is a rarity.
    Sustainable,
    /// More than one run in five finishes onto a moved trunk: drift holds are
    /// becoming background noise in the queue.
    Even,
    /// At least a coin flip that a run finalises onto a base that no longer
    /// exists. Re-baselining is no longer an optimisation but the only way a
    /// patch lands.
    Invalidating,
}

/// The ratio of trunk movement to run length.
///
/// This is the number that makes the loop visible: the numerator is the
/// project's own merge rate, so the ratio worsens as the project succeeds,
/// and nothing in the pipeline worsens it more slowly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DriftExposure {
    /// Wall-clock length of a typical run.
    pub run_secs: u64,
    /// Mean interval between trunk advances.
    pub merge_interval_secs: u64,
}

impl DriftExposure {
    /// Builds an exposure from two measured durations. Returns `None` when
    /// either is zero: a zero-length run or a trunk that merges instantly is
    /// not a measurement, and a ratio computed from one would be a number
    /// that happens to be comparable to other numbers.
    pub fn new(run_secs: u64, merge_interval_secs: u64) -> Option<Self> {
        if run_secs == 0 || merge_interval_secs == 0 {
            return None;
        }
        Some(Self {
            run_secs,
            merge_interval_secs,
        })
    }

    /// Expected trunk advances during one run.
    pub fn advances_per_run(&self) -> f64 {
        self.run_secs as f64 / self.merge_interval_secs as f64
    }

    /// Probability that at least one trunk advance lands during one run,
    /// treating merges as arriving at a constant mean rate (`1 - e^-ratio`).
    ///
    /// The mean alone understates the exposure: the trunk in #3708 moved 0.83
    /// times per run, which does not leave 17% of runs alone, it leaves the
    /// runs with no merge in them, which is `e^-0.83` of them.
    pub fn invalidation_probability(&self) -> f64 {
        1.0 - (-self.advances_per_run()).exp()
    }

    /// Grades the exposure by that probability: invalidating at a coin flip,
    /// even at one run in five.
    pub fn classify(&self) -> ExposureClass {
        let p = self.invalidation_probability();
        if p >= 0.5 {
            ExposureClass::Invalidating
        } else if p >= 0.2 {
            ExposureClass::Even
        } else {
            ExposureClass::Sustainable
        }
    }

    /// One log line: the ratio, the probability and the class.
    pub fn line(&self) -> String {
        format!(
            "drift exposure {:.2} trunk advances per run, {:.0}% of runs finalise onto a moved trunk ({}s run, {}s merge interval): {:?}",
            self.advances_per_run(),
            self.invalidation_probability() * 100.0,
            self.run_secs,
            self.merge_interval_secs,
            self.classify(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rebaseline::support::{drift, BASE, TIP};

    // --- staleness -----------------------------------------------------------

    #[test]
    fn a_clean_apply_on_a_moved_trunk_is_staleness() {
        assert_eq!(
            classify_staleness(&drift(6), &[], &["src/a.rs".into()]),
            Staleness::StaleButClean
        );
    }

    #[test]
    fn a_conflict_outside_the_patch_surface_is_staleness() {
        assert_eq!(
            classify_staleness(
                &drift(6),
                &["src/generated/bindings.rs".into()],
                &["src/a.rs".into()]
            ),
            Staleness::StaleButClean
        );
    }

    #[test]
    fn a_conflict_inside_the_patch_surface_is_a_conflict() {
        assert_eq!(
            classify_staleness(
                &drift(6),
                &["src/a.rs".into()],
                &["src/a.rs".into(), "src/b.rs".into()]
            ),
            Staleness::SubjectMatterConflict {
                files: vec!["src/a.rs".into()]
            }
        );
    }

    #[test]
    fn a_current_base_blames_nothing_but_the_patch() {
        assert_eq!(
            classify_staleness(&drift(0), &["src/a.rs".into()], &["src/a.rs".into()]),
            Staleness::Current
        );
    }

    // --- a HELD is a hypothesis ----------------------------------------------

    #[test]
    fn a_hold_against_the_current_tip_is_a_decision() {
        assert_eq!(hold_status(TIP, TIP, 0, 3), Ok(HoldStatus::Decision));
    }

    #[test]
    fn a_young_hold_is_still_a_decision() {
        assert_eq!(
            hold_status(
                BASE,
                TIP,
                DEFAULT_HELD_HYPOTHESIS_COMMITS - 1,
                DEFAULT_HELD_HYPOTHESIS_COMMITS
            ),
            Ok(HoldStatus::Decision)
        );
    }

    #[test]
    fn an_old_hold_is_a_hypothesis() {
        assert_eq!(
            hold_status(
                BASE,
                TIP,
                DEFAULT_HELD_HYPOTHESIS_COMMITS,
                DEFAULT_HELD_HYPOTHESIS_COMMITS
            ),
            Ok(HoldStatus::Hypothesis {
                commits_stale: DEFAULT_HELD_HYPOTHESIS_COMMITS,
                threshold: DEFAULT_HELD_HYPOTHESIS_COMMITS
            })
        );
        assert!(matches!(
            hold_status(BASE, TIP, 14, 3),
            Ok(HoldStatus::Hypothesis { .. })
        ));
    }

    #[test]
    fn a_hold_measures_its_age_like_any_drift() {
        assert!(hold_status(BASE, TIP, 0, 3).is_err());
        assert!(hold_status("", TIP, 5, 3).is_err());
    }

    // --- exposure -------------------------------------------------------------

    #[test]
    fn exposure_needs_two_real_durations() {
        assert_eq!(DriftExposure::new(0, 1440), None);
        assert_eq!(DriftExposure::new(1200, 0), None);
        assert!(DriftExposure::new(1200, 1440).is_some());
    }

    #[test]
    fn a_run_shorter_than_the_merge_interval_is_sustainable() {
        // A 4-minute run against the 24-minute merge interval #3708 measured.
        let e = DriftExposure::new(4 * 60, 24 * 60).unwrap();
        assert!(e.advances_per_run() < 0.2);
        assert_eq!(e.classify(), ExposureClass::Sustainable);
    }

    #[test]
    fn the_observed_incident_classifies_as_invalidating() {
        // The run lengths #3708 measured (7-45 minutes, mean 20) against the
        // merge rate it measured (7 merges in 2h50m, one every ~24 minutes).
        // The mean run is under one advance per run, which is why reading the
        // mean alone called this acceptable.
        let e = DriftExposure::new(20 * 60, 24 * 60).unwrap();
        assert!(e.advances_per_run() < 1.0);
        assert!(e.invalidation_probability() > 0.5);
        assert_eq!(e.classify(), ExposureClass::Invalidating);
        assert!(e.line().contains("Invalidating"), "{}", e.line());
        // The longest run in that window (45 minutes) was nearly certain to
        // be invalidated; the shortest (7 minutes) was still one run in four.
        assert!(
            DriftExposure::new(45 * 60, 24 * 60)
                .unwrap()
                .invalidation_probability()
                > 0.8
        );
        assert_eq!(
            DriftExposure::new(7 * 60, 24 * 60).unwrap().classify(),
            ExposureClass::Even
        );
    }

    #[test]
    fn a_mean_below_one_still_invalidates_many_runs() {
        // 0.25 advances per run is a 22% chance of finalising onto a moved
        // base: past the one-in-five line, so no longer sustainable, but far
        // from the 63% that one advance per run reaches.
        let e = DriftExposure::new(100, 400).unwrap();
        assert!(e.invalidation_probability() < 0.25);
        assert_eq!(e.classify(), ExposureClass::Even);
        assert_eq!(
            DriftExposure::new(100, 100).unwrap().classify(),
            ExposureClass::Invalidating
        );
    }
}
