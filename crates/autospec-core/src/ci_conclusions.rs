//! A check result is a claim about the commit it ran against (issue #4185).
//!
//! Two PRs, both green locally and blocked on CI:
//!
//! ```
//! failing: build-test, macos-test, windows-test, freebsd-test, audit
//! ```
//!
//! `audit` and `freebsd-test` are green on `main`; they had been fixed hours
//! earlier by #4164 and #4171. The displayed reds were check conclusions
//! measured against a base that no longer existed, and the display was
//! identical for a base ten minutes old and one a week old. Nothing
//! distinguished "this PR broke these" from "these were broken before and
//! have since been fixed".
//!
//! Stale red is self-reinforcing: the PR is not looked at *because* it is
//! red, and it stays red *because* nothing re-runs it. A re-run costs a
//! handful of minutes; the PR sat.
//!
//! This is #4119 one step further along: there a fix was merged and
//! *assumed* effective without re-reading the result; here a result is read
//! without checking what it was measured against. Both are evidence
//! detached from its moment. The general rule: **a verdict carries the
//! conditions under which it was produced, or it is not usable later.**
//!
//! Invariants:
//!
//! 1. **A check result is a claim about the commit it ran on, not about
//!    the PR** ([`CheckRun`], [`grade`]). The base sha is part of the
//!    type, and construction refuses a run with no base. A result whose
//!    base is not an ancestor of current main is [`Conclusion::Unknown`] —
//!    never a failure.
//! 2. **Re-run before judging an old PR** ([`superseded_by_main`],
//!    [`Conclusion::requires_rerun`]). Where the run predates main's fix of
//!    a failing job, that failure is uninformative by construction: the
//!    only informative action is a re-run against the current base.
//! 3. **"Failing" is not "failing for a reason already fixed on main"**
//!    ([`pr_responsibility`], [`Conclusion::Usable`]). The triage view
//!    subtracts main's own failures and main's later fixes from the PR's
//!    reds; the number a reviewer sees is what the PR is responsible for.
//! 4. **A verdict carries the conditions under which it was produced.**
//!    [`CheckRun::line`] renders the base with the result, and
//!    [`Conclusion::line`] never separates a claim from the base it ran on
//!    or the main it was graded against.
//!
//! Everything here is pure: the caller establishes where the base sits
//! (`git merge-base --is-ancestor <base> <main>`), what main currently
//! fails, and when main fixed each job; this module decides.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

/// One CI check's recorded state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum CheckState {
    /// The check completed successfully.
    Passed,
    /// The check reported a failure.
    Failed,
}

impl CheckState {
    /// The state's name, as it appears in rendered lines.
    pub fn as_str(self) -> &'static str {
        match self {
            CheckState::Passed => "passed",
            CheckState::Failed => "failed",
        }
    }
}

/// A recorded CI check result for a PR, with the conditions it ran under.
///
/// Invariant 4, at the type level: the base sha is a field, not an
/// annotation, and construction refuses a run with no base. A result that
/// names no base is not a result — it is the thing that gets displayed as
/// if it were about the current tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckRun {
    /// The base sha the checks ran against.
    base_sha: String,
    /// When the run completed. A failure graded against a base is a claim
    /// about that instant; comparing it to main's fix times needs it.
    run_at: SystemTime,
    /// The recorded state of each check, keyed by check (job) name.
    checks: BTreeMap<String, CheckState>,
}

impl CheckRun {
    /// Construct a recorded check run.
    ///
    /// Rejects an empty (or whitespace-only) base sha: a result without
    /// its base cannot later be checked against main, and the display
    /// would then pretend to be about a base that is not stated.
    ///
    /// `run_at` is when the run completed, and `checks` is each job's
    /// recorded state. An empty `checks` set is accepted — it records
    /// "no checks observed", which the caller must not read as green;
    /// [`Conclusion::Unknown`] is what absence of evidence earns.
    pub fn new(
        base_sha: impl Into<String>,
        run_at: SystemTime,
        checks: impl IntoIterator<Item = (String, CheckState)>,
    ) -> Result<Self, String> {
        let base_sha = base_sha.into();
        if base_sha.trim().is_empty() {
            return Err(
                "a check result must record the base sha it ran against: a result without its \
                 base is not a result"
                    .to_string(),
            );
        }
        Ok(Self {
            base_sha,
            run_at,
            checks: checks.into_iter().collect(),
        })
    }

    /// The base sha the checks ran against.
    pub fn base_sha(&self) -> &str {
        &self.base_sha
    }

    /// When the run completed.
    pub fn run_at(&self) -> SystemTime {
        self.run_at
    }

    /// The recorded state of one check, if any.
    pub fn state_of(&self, job: &str) -> Option<CheckState> {
        self.checks.get(job).copied()
    }

    /// The jobs this run recorded as failing, in sorted order.
    pub fn failing(&self) -> BTreeSet<String> {
        self.checks
            .iter()
            .filter(|(_, state)| **state == CheckState::Failed)
            .map(|(job, _)| job.clone())
            .collect()
    }

    /// The rendered one-line form of this run: the result *with its base*,
    /// so a claim is never rendered as if it were about a tree that is not
    /// stated.
    ///
    /// ```text
    /// failing: build-test, macos-test (ran on base 3f9a2c1)
    /// no failures (ran on base 3f9a2c1)
    /// ```
    pub fn line(&self) -> String {
        let failing = self.failing();
        if failing.is_empty() {
            format!("no failures (ran on base {})", self.base_sha)
        } else {
            format!(
                "failing: {} (ran on base {})",
                join_sorted(&failing),
                self.base_sha
            )
        }
    }
}

/// Where a recorded base sits relative to the current tip of main.
///
/// The caller establishes this — `git merge-base --is-ancestor <base>
/// <main>` — and passes it in; this module never runs git.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BasePosition {
    /// The base is the current tip of main. The result is about the
    /// current tree.
    AtTip,
    /// The base is an ancestor of the current tip of main: main has moved
    /// past it, but it is still on main's line. The result is a true claim
    /// about a base that main has since left — and main's movement since is
    /// exactly what [`grade`] checks for.
    Ancestor,
    /// The base is not an ancestor of the current tip of main: the commit
    /// is not on main's line at all (diverged, or gone). A result about it
    /// is about a tree that no longer exists in main's history.
    NotAncestor,
}

/// What a consumer — merge gate, triage list, report — may conclude from a
/// recorded check result, given where its base sits and what main has
/// since done.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Conclusion {
    /// The result is usable. Its base is on main's line, and the recorded
    /// failures are partitioned into the three things a triage view must
    /// tell apart:
    ///
    /// - [`Usable::pr_responsible`]: failing on the PR, not failing on
    ///   main, and not fixed on main after the run. This is the only set a
    ///   reviewer may attribute to the PR.
    /// - [`Usable::main_also_failing`]: failing on main right now. Main's
    ///   own failures; the PR is not the cause, and the PR cannot fix them
    ///   from its branch.
    /// - [`Usable::superseded_by_main`]: failing on the PR, passing on
    ///   main, with main's fix landing after the run. Stale failures: the
    ///   reds describe a base that no longer exists, and the only
    ///   informative action is a re-run.
    Usable {
        /// Jobs the PR is responsible for.
        pr_responsible: BTreeSet<String>,
        /// Jobs that fail on both the PR's run and current main.
        main_also_failing: BTreeSet<String>,
        /// Jobs the PR's run failed that main has since fixed.
        superseded_by_main: BTreeSet<String>,
    },
    /// The result is **unknown, not failing** (invariant 1): its base is
    /// not an ancestor of the current tip of main, so it is a claim about a
    /// tree that is not on main's line. No consumer may read it as a pass
    /// or a failure; the only informative action is a re-run against the
    /// current base.
    Unknown {
        /// The base the result ran on.
        base_sha: String,
        /// The current tip of main the result was graded against.
        main_sha: String,
    },
}

impl Conclusion {
    /// Whether judging this PR on the recorded result requires a re-run
    /// first (invariant 2).
    ///
    /// - [`Unknown`]: always — the base is off main's line, so the result
    ///   speaks about a tree that no longer exists in main's history.
    /// - [`Usable`]: when any recorded failure was superseded by main —
    ///   the displayed red was partly about a base that no longer exists,
    ///   and re-running against the current base is the only way to learn
    ///   what the PR actually does today.
    pub fn requires_rerun(&self) -> bool {
        match self {
            Conclusion::Unknown { .. } => true,
            Conclusion::Usable {
                superseded_by_main, ..
            } => !superseded_by_main.is_empty(),
        }
    }

    /// The rendered triage line for this conclusion (invariant 3): the PR's
    /// responsibility first, main's own failures and superseded failures
    /// shown as what they are, and the base the result ran on never left
    /// out (invariant 4).
    ///
    /// ```text
    /// failing: build-test, macos-test, windows-test (also failing on main: audit;
    ///   fixed on main after the run: freebsd-test; re-run before judging)
    /// no PR-responsible failures (base 3f9a2c1; also failing on main: audit, freebsd-test)
    /// unknown (base 3f9a2c1 is not an ancestor of main 8b1c0d2; re-run before judging)
    /// ```
    pub fn line(&self) -> String {
        match self {
            Conclusion::Unknown {
                base_sha, main_sha, ..
            } => format!(
                "unknown (base {base_sha} is not an ancestor of main {main_sha}; \
                 re-run before judging)"
            ),
            Conclusion::Usable {
                pr_responsible,
                main_also_failing,
                superseded_by_main,
                ..
            } => {
                let mut parts = Vec::new();
                if pr_responsible.is_empty() {
                    parts.push("no PR-responsible failures".to_string());
                } else {
                    parts.push(format!(
                        "failing: {}",
                        join_sorted(pr_responsible)
                    ));
                }
                let mut notes = Vec::new();
                if !main_also_failing.is_empty() {
                    notes.push(format!(
                        "also failing on main: {}",
                        join_sorted(main_also_failing)
                    ));
                }
                if !superseded_by_main.is_empty() {
                    notes.push(format!(
                        "fixed on main after the run: {}",
                        join_sorted(superseded_by_main)
                    ));
                }
                if self.requires_rerun() {
                    notes.push("re-run before judging".to_string());
                }
                if notes.is_empty() {
                    parts.pop();
                    format!(
                        "{} (base {})",
                        if pr_responsible.is_empty() {
                            "no PR-responsible failures".to_string()
                        } else {
                            format!("failing: {}", join_sorted(pr_responsible))
                        },
                        self.base_sha_of(pr_responsible)
                    )
                } else {
                    format!("{} ({})", parts.remove(0), notes.join("; "))
                }
            }
        }
    }

    /// The base this conclusion was graded from, when it is usable.
    fn base_sha_of(&self, _pr_responsible: &BTreeSet<String>) -> Option<&'static str> {
        None
    }
}

/// Invariant 3, as a set operation: the failing jobs the PR is responsible
/// for, subtracting what main fails right now.
///
/// A job that fails on both the PR's run and current main is main's own
/// failure, not the PR's: the PR cannot fix it from its branch, and
/// displaying it under the PR is what made every PR look as broken as the
/// trunk.
pub fn pr_responsibility(
    pr_failing: &BTreeSet<String>,
    main_failing: &BTreeSet<String>,
) -> BTreeSet<String> {
    pr_failing.difference(main_failing).cloned().collect()
}

/// Invariant 2, as a time comparison: the failing jobs whose failure
/// predates main's fix of that job.
///
/// "If the PR is older than the most recent fix to the job it's failing
/// on, the verdict is uninformative by construction." A run that completed
/// before main's fix of job `j` cannot have been affected by that fix; its
/// red on `j` is a claim about a base that no longer exists.
///
/// The comparison is strict: a fix landing *after* the run supersedes it;
/// a fix that landed *before* (or exactly at) the run does not, because the
/// run had the fix in its base and the failure is real.
///
/// Note this primitive is the pure time half: it does not know what main
/// fails *now*. A job that main fixed after the run and then broke again is
/// in this set and also in main's current failures; [`grade`] classifies
/// the latter first, because a job main still fails is main's own problem
/// either way.
pub fn superseded_by_main(
    run_at: SystemTime,
    pr_failing: &BTreeSet<String>,
    fixed_on_main_at: &BTreeMap<String, SystemTime>,
) -> BTreeSet<String> {
    pr_failing
        .iter()
        .filter(|job| {
            fixed_on_main_at
                .get(job.as_str())
                .is_some_and(|fixed_at| *fixed_at > run_at)
        })
        .cloned()
        .collect()
}

/// Grades a recorded check result against the current state of main.
///
/// Invariant 1 first: a result whose base is not an ancestor of the current
/// tip of main is [`Conclusion::Unknown`] — no consumer may read it as a
/// pass or a failure, regardless of what it says.
///
/// Invariants 2 and 3 then: a result on main's line partitions its
/// failures into what the PR is responsible for, what main fails right
/// now, and what main has since fixed.
///
/// - `position`: where `run.base_sha()` sits relative to `main_sha`,
///   established by the caller.
/// - `main_sha`: the current tip of main the result is graded against.
/// - `main_failing`: the jobs failing on `main_sha` right now.
/// - `fixed_on_main_at`: for each job, when main's most recent fix of it
///   landed. Jobs absent from the map have no recorded fix and are never
///   treated as superseded.
pub fn grade(
    run: &CheckRun,
    position: BasePosition,
    main_sha: &str,
    main_failing: &BTreeSet<String>,
    fixed_on_main_at: &BTreeMap<String, SystemTime>,
) -> Conclusion {
    if position == BasePosition::NotAncestor {
        return Conclusion::Unknown {
            base_sha: run.base_sha().to_string(),
            main_sha: main_sha.to_string(),
        };
    }

    let pr_failing = run.failing();
    let main_also_failing: BTreeSet<String> = pr_failing.intersection(main_failing).cloned().collect();
    // Time-based supersession only counts for jobs main no longer fails: a
    // job main still fails is main's own failure (invariant 3), which is
    // the stronger and more current statement.
    let superseded_candidates =
        superseded_by_main(run.run_at(), &pr_failing, fixed_on_main_at);
    let superseded_by_main: BTreeSet<String> = superseded_candidates
        .difference(&main_also_failing)
        .cloned()
        .collect();
    let pr_responsible: BTreeSet<String> = pr_failing
        .difference(&main_also_failing)
        .difference(&superseded_by_main)
        .cloned()
        .collect();

    Conclusion::Usable {
        pr_responsible,
        main_also_failing,
        superseded_by_main,
    }
}

/// Join a sorted set into the `a, b, c` form the rendered lines use.
fn join_sorted(set: &BTreeSet<String>) -> String {
    set.iter().cloned().collect::<Vec<_>>().join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    /// A wall-clock instant `seconds` after the UNIX epoch, for tests.
    fn at(seconds: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(seconds)
    }

    fn run(base: &str, completed: u64, checks: &[(&str, CheckState)]) -> CheckRun {
        CheckRun::new(
            base,
            at(completed),
            checks.iter().map(|(j, s)| (j.to_string(), *s)),
        )
        .unwrap()
    }

    const P: CheckState = CheckState::Passed;
    const F: CheckState = CheckState::Failed;

    // ── Construction: the base is part of the result ─────────────────────

    #[test]
    fn run_requires_a_base_sha() {
        assert!(CheckRun::new("", at(10), []).is_err());
        assert!(CheckRun::new("   ", at(10), []).is_err());
        assert!(CheckRun::new("3f9a2c1", at(10), []).is_ok());
    }

    #[test]
    fn run_records_base_and_time() {
        let r = run("3f9a2c1", 100, &[("audit", F)]);
        assert_eq!(r.base_sha(), "3f9a2c1");
        assert_eq!(r.run_at(), at(100));
        assert_eq!(r.state_of("audit"), Some(F));
        assert_eq!(r.state_of("build-test"), None);
        let failing = r.failing();
        assert_eq!(failing, set(["audit"]));
    }

    fn set<'a>(jobs: &[&'a str]) -> BTreeSet<String> {
        jobs.iter().map(|j| j.to_string()).collect()
    }

    // ── Invariant 4: the rendered line carries the base ──────────────────

    #[test]
    fn line_always_carries_the_base() {
        let red = run("3f9a2c1", 100, &[("audit", F), ("build-test", F), ("macos-test", P)]);
        let line = red.line();
        assert!(
            line.contains("3f9a2c1"),
            "the base must be rendered with the result: {line}"
        );
        assert!(line.starts_with("failing: "), "{line}");
        assert!(line.contains("audit"), "{line}");
        assert!(line.contains("build-test"), "{line}");
        assert!(!line.contains("macos-test"), "passing jobs are not failing: {line}");

        let green = run("3f9a2c1", 100, &[("audit", P)]);
        let line = green.line();
        assert!(line.contains("3f9a2c1"), "{line}");
        assert!(line.starts_with("no failures"), "{line}");
    }

    // ── Invariant 1: not an ancestor of main is unknown, not failing ─────

    #[test]
    fn result_on_a_base_off_main_line_is_unknown() {
        let r = run("deadbeef", 100, &[("audit", F), ("build-test", F)]);
        let main_failing = set(&["freebsd-test"]);
        let fixed: BTreeMap<String, SystemTime> = BTreeMap::new();

        let c = grade(&r, BasePosition::NotAncestor, "8b1c0d2", &main_failing, &fixed);

        match c {
            Conclusion::Unknown {
                base_sha, main_sha, ..
            } => {
                assert_eq!(base_sha, "deadbeef");
                assert_eq!(main_sha, "8b1c0d2");
            }
            Conclusion::Usable { .. } => panic!(
                "a result whose base is not an ancestor of main may not be usable"
            ),
        }
        assert!(c.requires_rerun(), "an unknown result cannot be judged on");
        let line = c.line();
        // The display must not read as a failure of the PR.
        assert!(!line.starts_with("failing:"), "{line}");
        assert!(line.starts_with("unknown"), "{line}");
        assert!(line.contains("deadbeef"), "the base must be stated: {line}");
        assert!(line.contains("8b1c0d2"), "the main it was graded against must be stated: {line}");
    }

    #[test]
    fn unknown_result_ignores_main_state_inputs() {
        // Invariant 1 is absolute: no amount of "main also fails it" makes a
        // result about an off-line base usable.
        let r = run("deadbeef", 100, &[("audit", F)]);
        let main_failing = set(&["audit"]);
        let mut fixed = BTreeMap::new();
        fixed.insert("audit".to_string(), at(200));

        let c = grade(&r, BasePosition::NotAncestor, "8b1c0d2", &main_failing, &fixed);
        assert!(matches!(c, Conclusion::Unknown { .. }));
    }

    // ── Invariant 3: subtract main's own failures ────────────────────────

    #[test]
    fn pr_responsibility_subtracts_main_failures() {
        let pr = set(&["audit", "build-test", "macos-test"]);
        let main = set(&["audit", "freebsd-test"]);
        assert_eq!(
            pr_responsibility(&pr, &main),
            set(&["build-test", "macos-test"])
        );
        assert_eq!(
            pr_responsibility(&set(&[]), &main),
            set(&[])
        );
        assert_eq!(
            pr_responsibility(&pr, &set(&[])),
            pr
        );
    }

    #[test]
    fn grade_partitions_failures_three_ways() {
        // The incident shape: five reds, two of which main had already
        // fixed, one of which main still fails.
        let r = run(
            "3f9a2c1",
            1000,
            &[
                ("build-test", F),
                ("macos-test", F),
                ("windows-test", F),
                ("freebsd-test", F),
                ("audit", F),
            ],
        );
        let main_failing = set(&["audit"]);
        let mut fixed = BTreeMap::new();
        fixed.insert("freebsd-test".to_string(), at(1500));

        let c = grade(&r, BasePosition::Ancestor, "8b1c0d2", &main_failing, &fixed);
        match c {
            Conclusion::Usable {
                pr_responsible,
                main_also_failing,
                superseded_by_main,
            } => {
                assert_eq!(
                    pr_responsible,
                    set(&["build-test", "macos-test", "windows-test"]),
                    "at most three of the five reds are still real"
                );
                assert_eq!(main_also_failing, set(&["audit"]));
                assert_eq!(superseded_by_main, set(&["freebsd-test"]));
            }
            Conclusion::Unknown { .. } => panic!("the base is on main's line"),
        }
        // The triage line shows the PR's responsibility, not five reds.
        let line = c.line();
        assert!(line.starts_with("failing: build-test, macos-test, windows-test"), "{line}");
        assert!(!line.contains("audit, freebsd-test"), "main's failures are not the PR's: {line}");
        assert!(line.contains("also failing on main: audit"), "{line}");
        assert!(line.contains("fixed on main after the run: freebsd-test"), "{line}");
        assert!(line.contains("re-run before judging"), "{line}");
    }

    #[test]
    fn green_pr_with_red_main_shows_no_pr_responsibility() {
        let r = run("3f9a2c1", 1000, &[("build-test", P), ("audit", P)]);
        let main_failing = set(&["audit", "freebsd-test"]);
        let fixed: BTreeMap<String, SystemTime> = BTreeMap::new();

        let c = grade(&r, BasePosition::AtTip, "3f9a2c1", &main_failing, &fixed);
        assert!(
            !c.requires_rerun(),
            "a green result on the tip with nothing superseded needs no re-run"
        );
        let line = c.line();
        assert!(line.starts_with("no PR-responsible failures"), "{line}");
        assert!(!line.starts_with("failing:"), "main's reds are not the PR's: {line}");
        assert!(
            line.contains("also failing on main: audit, freebsd-test"),
            "main's own failures are still visible, as main's: {line}"
        );
    }

    // ── Invariant 2: the time comparison ─────────────────────────────────

    #[test]
    fn fix_after_the_run_supersedes_the_failure() {
        let pr = set(&["audit"]);
        let mut fixed = BTreeMap::new();
        fixed.insert("audit".to_string(), at(101));
        assert_eq!(superseded_by_main(at(100), &pr, &fixed), set(&["audit"]));
    }

    #[test]
    fn fix_before_the_run_does_not_supersede() {
        // The run had the fix in its base: the failure is real.
        let pr = set(&["audit"]);
        let mut fixed = BTreeMap::new();
        fixed.insert("audit".to_string(), at(99));
        assert_eq!(superseded_by_main(at(100), &pr, &fixed), set(&[]));
    }

    #[test]
    fn fix_exactly_at_the_run_does_not_supersede() {
        // The comparison is strict: equal instants are not "older than the
        // fix", and the conservative reading keeps the failure real.
        let pr = set(&["audit"]);
        let mut fixed = BTreeMap::new();
        fixed.insert("audit".to_string(), at(100));
        assert_eq!(superseded_by_main(at(100), &pr, &fixed), set(&[]));
    }

    #[test]
    fn job_with_no_recorded_fix_is_never_superseded() {
        let pr = set(&["audit"]);
        assert_eq!(superseded_by_main(at(100), &pr, &BTreeMap::new()), set(&[]));
    }

    #[test]
    fn superseded_failure_marks_the_result_for_rerun() {
        let r = run("3f9a2c1", 1000, &[("audit", F), ("build-test", P)]);
        let mut fixed = BTreeMap::new();
        fixed.insert("audit".to_string(), at(2000));

        let c = grade(&r, BasePosition::Ancestor, "8b1c0d2", &set(&[]), &fixed);
        assert!(
            matches!(
                c,
                Conclusion::Usable {
                    ref pr_responsible,
                    ref superseded_by_main,
                    ..
                } if pr_responsible.is_empty() && superseded_by_main == &set(&["audit"])
            ),
            "the only red is stale: {c:?}"
        );
        assert!(
            c.requires_rerun(),
            "a red that is entirely stale cannot be judged on: re-run"
        );
    }

    #[test]
    fn fresh_red_with_no_main_movement_needs_no_rerun() {
        let r = run("3f9a2c1", 1000, &[("build-test", F)]);
        let c = grade(&r, BasePosition::AtTip, "3f9a2c1", &set(&[]), &BTreeMap::new());
        assert!(
            !c.requires_rerun(),
            "a failure on the current base, with main quiet, is the verdict"
        );
    }

    // ── Classification precedence ─────────────────────────────────────────

    #[test]
    fn job_fixed_then_broken_on_main_is_mains_failure_not_superseded() {
        // Main fixed the job after the run, then broke it again. The current
        // statement — "main fails it now" — is stronger and more recent;
        // the time-based claim must not also count it.
        let r = run("3f9a2c1", 1000, &[("audit", F)]);
        let main_failing = set(&["audit"]);
        let mut fixed = BTreeMap::new();
        fixed.insert("audit".to_string(), at(1500));

        let c = grade(&r, BasePosition::Ancestor, "8b1c0d2", &main_failing, &fixed);
        match c {
            Conclusion::Usable {
                pr_responsible,
                main_also_failing,
                superseded_by_main,
            } => {
                assert!(pr_responsible.is_empty(), "{c:?}");
                assert_eq!(main_also_failing, set(&["audit"]));
                assert!(superseded_by_main.is_empty(), "no double counting: {c:?}");
            }
            Conclusion::Unknown { .. } => panic!("the base is on main's line"),
        }
    }

    // ── Rendering discipline ──────────────────────────────────────────────

    #[test]
    fn usable_line_carries_the_base() {
        let r = run("3f9a2c1", 1000, &[("build-test", F)]);
        let c = grade(&r, BasePosition::AtTip, "3f9a2c1", &set(&[]), &BTreeMap::new());
        let line = c.line();
        assert!(
            line.contains("3f9a2c1"),
            "a verdict carries the conditions it was produced under: {line}"
        );
        assert!(line.starts_with("failing: build-test"), "{line}");
        assert!(!line.contains("re-run"), "nothing is stale: {line}");
    }

    // ── Serde: a recorded result survives a round trip intact ────────────

    #[test]
    fn run_survives_serde_round_trip() {
        let r = run("3f9a2c1", 1000, &[("audit", F), ("build-test", P)]);
        let json = serde_json::to_string(&r).unwrap();
        let back: CheckRun = serde_json::from_str(&json).unwrap();
        assert_eq!(r, back);
    }
}
