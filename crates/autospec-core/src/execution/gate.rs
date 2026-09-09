//! Conversion gate policy (#3781): a hold record names every blocking
//! finding, not the first one the pipeline happened to hit.
//!
//! A gate's output is a durable record that someone — or some agent — acts
//! on later, possibly hours later, without re-running anything. Recording
//! only the first blocking finding makes such a record actively dangerous:
//! a flaky `cargo test --workspace` failure held a patch whose real defect
//! was a lint violation (an env var introduced without touching a doc file)
//! that only appeared further down the pipeline, at commit time. Disproving
//! the recorded reason then looked like proof the patch was fine, when it
//! only proved that the gate had stopped looking.
//!
//! The policy is encoded here as pure, testable primitives; callers execute
//! the checks in plan order and act on the verdict these functions return:
//!
//! 1. **Every independent check runs; a hold names every finding.**
//!    [`evaluate_gate`] collects all blocking findings rather than stopping
//!    at the first. A check is skipped only when a prior blocking finding
//!    makes it impossible to evaluate, and that skip is recorded as
//!    *not evaluated* — never as absence of a finding.
//! 2. **The hold record is ordered by confidence**, most confident first,
//!    so acting on it addresses the real blocker rather than the
//!    first-encountered one ([`GateVerdict::line`]).
//! 3. **Cheap before expensive.** Where checks must be sequenced for cost,
//!    the cheap deterministic ones (lint, formatting, documentation
//!    contracts) run before the expensive nondeterministic ones (the test
//!    suite, which can flake), so a flaky suite cannot mask a definite
//!    defect ([`GatePlan::new`]).

use std::collections::{BTreeMap, BTreeSet};

/// What a gate check costs to run, and how its failure can be trusted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckCost {
    /// Cheap and deterministic: lint, formatting, documentation contracts.
    /// The same input produces the same finding, every time.
    Deterministic,
    /// Expensive and nondeterministic: the full test suite, which can fail
    /// under parallel execution on a tree that is otherwise healthy.
    Nondeterministic,
}

impl CheckCost {
    /// How much a hold record can trust a finding from a check of this cost.
    pub fn confidence(self) -> Confidence {
        match self {
            Self::Deterministic => Confidence::Deterministic,
            Self::Nondeterministic => Confidence::Measured,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Deterministic => "deterministic",
            Self::Nondeterministic => "nondeterministic",
        }
    }
}

/// How much a hold record can trust one finding.
///
/// Ordered, most confident first: a deterministic finding sorts ahead of a
/// measured one, which is the order the hold record is written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Confidence {
    /// Same input, same finding, every time: a lint violation, a formatting
    /// failure, a documentation-contract breach.
    Deterministic,
    /// Observed once; a clean re-run may not reproduce it: a test failure
    /// under parallel load.
    Measured,
}

/// One gate check: a single thing the pipeline evaluates about a patch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateCheck {
    /// Stable name, e.g. `lint`, `format`, `tests`, `commit`.
    pub name: String,
    /// What the check costs, and how its finding can be trusted.
    pub cost: CheckCost,
    /// Checks that must report no blocking finding before this one can be
    /// evaluated at all: the pipeline never reaches this check once one of
    /// them blocks. Independent checks leave this empty.
    pub requires: Vec<String>,
}

impl GateCheck {
    /// A check that can be evaluated no matter what the other checks found.
    pub fn independent(name: impl Into<String>, cost: CheckCost) -> Self {
        Self {
            name: name.into(),
            cost,
            requires: Vec::new(),
        }
    }

    /// A check the pipeline reaches only after `required` are all clean.
    pub fn requires_all(
        name: impl Into<String>,
        cost: CheckCost,
        required: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        let name = name.into();
        Self {
            requires: required.into_iter().map(Into::into).collect(),
            name,
            cost,
        }
    }
}

/// The outcome of one executed check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckOutcome {
    /// The check ran and found nothing.
    Clean,
    /// The check ran and found at least one blocking defect.
    Blocking {
        /// The check's own detail line, carried verbatim into the record.
        detail: String,
    },
}

/// A gate run plan: every check exactly once, in the order it must run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatePlan {
    checks: Vec<GateCheck>,
}

impl GatePlan {
    /// Build the plan for the given checks.
    ///
    /// Declared order is reordered so that every cheap deterministic check
    /// runs before every expensive nondeterministic one (declared order is
    /// preserved within a class): a flaky suite must never get first say
    /// over a definite defect. Fails closed on empty plans, duplicate
    /// names, dependencies on unknown or self checks, and dependencies the
    /// cost order cannot satisfy.
    pub fn new(mut checks: Vec<GateCheck>) -> Result<Self, String> {
        if checks.is_empty() {
            return Err("a gate plan needs at least one check".to_string());
        }
        let mut names = BTreeSet::new();
        for check in &checks {
            if check.name.trim().is_empty() {
                return Err("gate check names must be nonempty".to_string());
            }
            if !names.insert(check.name.clone()) {
                return Err(format!("duplicate gate check: {}", check.name));
            }
        }
        for check in &mut checks {
            let mut required = BTreeSet::new();
            for dep in std::mem::take(&mut check.requires) {
                if dep == check.name {
                    return Err(format!("gate check {} requires itself", check.name));
                }
                if !names.contains(&dep) {
                    return Err(format!(
                        "gate check {} requires unknown check {}",
                        check.name, dep
                    ));
                }
                required.insert(dep);
            }
            check.requires = required.into_iter().collect();
        }
        // Stable partition: deterministic first, declared order within a class.
        checks.sort_by_key(|check| match check.cost {
            CheckCost::Deterministic => 0,
            CheckCost::Nondeterministic => 1,
        });
        let position: BTreeMap<String, usize> = checks
            .iter()
            .enumerate()
            .map(|(index, check)| (check.name.clone(), index))
            .collect();
        for check in &mut checks {
            check.requires.sort_by_key(|dep| position[dep.as_str()]);
            for dep in &check.requires {
                if position[dep.as_str()] > position[check.name.as_str()] {
                    return Err(format!(
                        "gate check {} requires {} but the cost order evaluates it first; \
                         a check cannot wait on the flaky suite and still run before it",
                        check.name, dep
                    ));
                }
            }
        }
        Ok(Self { checks })
    }

    /// Every check, in run order.
    pub fn checks(&self) -> &[GateCheck] {
        &self.checks
    }

    /// The check named `name`, if the plan has one.
    pub fn check(&self, name: &str) -> Option<&GateCheck> {
        self.checks.iter().find(|check| check.name == name)
    }
}

/// One blocking finding in a hold record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoldFinding {
    /// The check that produced the finding.
    pub check: String,
    /// How much the record can trust it.
    pub confidence: Confidence,
    /// The check's own detail line, verbatim.
    pub detail: String,
}

/// A check the pipeline did not evaluate because an earlier blocking
/// finding made it impossible. A skip is recorded — never silently dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotEvaluated {
    /// The check that was skipped.
    pub check: String,
    /// The check whose blocking finding made it impossible.
    pub because: String,
}

/// The gate's decision for one patch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateVerdict {
    /// Every evaluated check was clean; the patch may convert.
    Convert,
    /// At least one blocking finding. The record names *all* of them,
    /// most confident first, plus every check a prior failure made
    /// impossible to evaluate.
    Hold {
        findings: Vec<HoldFinding>,
        not_evaluated: Vec<NotEvaluated>,
    },
}

impl GateVerdict {
    /// Whether the patch is held.
    pub fn is_hold(&self) -> bool {
        matches!(self, Self::Hold { .. })
    }

    /// The names of the checks behind the blocking findings, record order.
    pub fn finding_checks(&self) -> Vec<&str> {
        match self {
            Self::Convert => Vec::new(),
            Self::Hold { findings, .. } => findings.iter().map(|f| f.check.as_str()).collect(),
        }
    }

    /// The durable hold record: every blocking finding, most confident
    /// first, followed by every recorded skip. An actor reading this line
    /// hours later finds the real blocker at the front, not the flake.
    pub fn line(&self) -> String {
        match self {
            Self::Convert => "gate clean: every evaluated check found nothing".to_string(),
            Self::Hold {
                findings,
                not_evaluated,
            } => {
                let mut parts = findings
                    .iter()
                    .map(|finding| format!("{}: {}", finding.check, finding.detail))
                    .collect::<Vec<_>>();
                for skipped in not_evaluated {
                    parts.push(format!(
                        "not evaluated: {} (blocked by {})",
                        skipped.check, skipped.because
                    ));
                }
                format!("HELD: {}", parts.join("; "))
            }
        }
    }
}

/// The internal decision recorded for one check while walking the plan.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Decision {
    Clean,
    /// The gate cannot get past this point; the named check is the one that
    /// actually failed. A skipped check carries its dependency's reason,
    /// so a transitive skip still points at the real blocker.
    Blocked(String),
}

/// Evaluate a gate run.
///
/// `outcomes` holds the result of every check the pipeline actually ran, in
/// no particular order. Every check in the plan either has an outcome or is
/// recorded as not evaluated against the first blocking finding (in plan
/// order) that makes it impossible — including transitively, through a
/// dependency that was itself skipped. A check with no outcome *and* no
/// such prior failure is a pipeline that stopped without a reason, which is
/// an error, not a silent pass or a silent skip.
///
/// The hold record collects every blocking finding, ordered by confidence
/// (deterministic first, plan order within a tier).
pub fn evaluate_gate(
    plan: &GatePlan,
    outcomes: &BTreeMap<String, CheckOutcome>,
) -> Result<GateVerdict, String> {
    for name in outcomes.keys() {
        if plan.check(name).is_none() {
            return Err(format!("gate outcome for unknown check: {name}"));
        }
    }
    let mut decided: BTreeMap<&str, Decision> = BTreeMap::new();
    let mut findings: Vec<HoldFinding> = Vec::new();
    let mut not_evaluated: Vec<NotEvaluated> = Vec::new();
    for check in plan.checks() {
        match outcomes.get(&check.name) {
            Some(CheckOutcome::Clean) => {
                decided.insert(check.name.as_str(), Decision::Clean);
            }
            Some(CheckOutcome::Blocking { detail }) => {
                findings.push(HoldFinding {
                    check: check.name.clone(),
                    confidence: check.cost.confidence(),
                    detail: detail.clone(),
                });
                decided.insert(check.name.as_str(), Decision::Blocked(check.name.clone()));
            }
            None => {
                let because = skip_reason(check, &decided)?;
                not_evaluated.push(NotEvaluated {
                    check: check.name.clone(),
                    because: because.clone(),
                });
                decided.insert(check.name.as_str(), Decision::Blocked(because));
            }
        }
    }
    // Stable: plan order (and thus declared order) is preserved within a tier.
    findings.sort_by_key(|finding| finding.confidence);
    if findings.is_empty() {
        Ok(GateVerdict::Convert)
    } else {
        Ok(GateVerdict::Hold {
            findings,
            not_evaluated,
        })
    }
}

/// The check whose blocking finding makes `check` impossible to evaluate.
/// A dependency that was itself skipped carries its own reason, so the
/// record always names the check that actually failed, not a check that
/// merely never ran.
fn skip_reason(check: &GateCheck, decided: &BTreeMap<&str, Decision>) -> Result<String, String> {
    let mut inherited = None;
    for dep in &check.requires {
        match decided.get(dep.as_str()) {
            Some(Decision::Blocked(reason)) => return Ok(reason.clone()),
            Some(Decision::Clean) => {}
            // A dependency with no recorded decision cannot explain a skip;
            // the plan guarantees dependencies run first, so this is only
            // reachable through a dependency that was itself skipped.
            None => {
                if inherited.is_none() {
                    inherited = Some(dep.clone());
                }
            }
        }
    }
    inherited.ok_or_else(|| {
        format!(
            "gate check {} was not evaluated and no prior blocking finding \
             makes it impossible; a skip is a recorded skip, not an unrecorded absence",
            check.name
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(name: &str, cost: CheckCost) -> GateCheck {
        GateCheck::independent(name, cost)
    }

    fn outcomes(pairs: &[(&str, CheckOutcome)]) -> BTreeMap<String, CheckOutcome> {
        pairs
            .iter()
            .map(|(name, outcome)| (name.to_string(), outcome.clone()))
            .collect()
    }

    #[test]
    fn plan_orders_cheap_deterministic_checks_before_the_flaky_suite() {
        // Declared the way the pipeline historically ran — the suite first —
        // the plan must still give the definite checks first say.
        let plan = GatePlan::new(vec![
            check("tests", CheckCost::Nondeterministic),
            check("lint", CheckCost::Deterministic),
            check("format", CheckCost::Deterministic),
            check("docs", CheckCost::Deterministic),
        ])
        .unwrap();
        let order: Vec<&str> = plan.checks().iter().map(|c| c.name.as_str()).collect();
        assert_eq!(order, vec!["lint", "format", "docs", "tests"]);
    }

    #[test]
    fn a_hold_names_the_lint_violation_behind_a_flaky_test_failure() {
        // Regression (#3781): the patch touched one shell script and no
        // Rust. The suite flaked under parallel execution; the lint
        // violation was the real defect. The hold must name the lint
        // violation — and lead with it, since it is the finding the record
        // can trust.
        let plan = GatePlan::new(vec![
            check("tests", CheckCost::Nondeterministic),
            check("lint", CheckCost::Deterministic),
        ])
        .unwrap();
        let outcome = outcomes(&[
            (
                "tests",
                CheckOutcome::Blocking {
                    detail:
                        "tests failed (--workspace): terminal_label::autonomous_executor_bridge \
                             919 passed; 1 failed"
                            .to_string(),
                },
            ),
            (
                "lint",
                CheckOutcome::Blocking {
                    detail: "DOC_OUT_OF_SYNC: scripts/autospec-explore.sh:38: env var introduced \
                             without touching a doc file"
                        .to_string(),
                },
            ),
        ]);
        let verdict = evaluate_gate(&plan, &outcome).unwrap();
        match &verdict {
            GateVerdict::Hold {
                findings,
                not_evaluated,
            } => {
                assert!(not_evaluated.is_empty(), "every check ran here");
                assert_eq!(findings.len(), 2);
                assert_eq!(findings[0].check, "lint");
                assert_eq!(findings[0].confidence, Confidence::Deterministic);
                assert!(findings[0].detail.starts_with("DOC_OUT_OF_SYNC"));
                assert_eq!(findings[1].check, "tests");
                assert_eq!(findings[1].confidence, Confidence::Measured);
            }
            other => panic!("expected a hold, got {other:?}"),
        }
        let line = verdict.line();
        assert!(
            line.contains("DOC_OUT_OF_SYNC"),
            "the hold record must name the lint violation: {line}"
        );
        let lint_at = line.find("DOC_OUT_OF_SYNC").unwrap();
        let tests_at = line.find("tests failed").unwrap();
        assert!(lint_at < tests_at, "the definite defect leads: {line}");
    }

    #[test]
    fn a_check_a_prior_failure_makes_impossible_is_recorded_not_evaluated() {
        // The suite cannot run at all if the build is broken. The pipeline
        // stops after the build — and the record must say the suite was
        // never evaluated, not simply omit it.
        let plan = GatePlan::new(vec![
            GateCheck::requires_all("tests", CheckCost::Nondeterministic, ["build"]),
            check("build", CheckCost::Deterministic),
        ])
        .unwrap();
        let outcome = outcomes(&[(
            "build",
            CheckOutcome::Blocking {
                detail: "workspace build failed: E0425".to_string(),
            },
        )]);
        let verdict = evaluate_gate(&plan, &outcome).unwrap();
        match &verdict {
            GateVerdict::Hold {
                findings,
                not_evaluated,
            } => {
                assert_eq!(findings.len(), 1);
                assert_eq!(findings[0].check, "build");
                assert_eq!(
                    not_evaluated,
                    &[NotEvaluated {
                        check: "tests".to_string(),
                        because: "build".to_string(),
                    }]
                );
            }
            other => panic!("expected a hold, got {other:?}"),
        }
        assert!(verdict
            .line()
            .contains("not evaluated: tests (blocked by build)"));
    }

    #[test]
    fn a_transitive_skip_is_recorded_against_the_check_that_blocked() {
        // Integration runs behind the suite, the suite behind the build. A
        // broken build skips both — and each skip in the record must point
        // at the check that actually failed, not at a check that merely
        // never ran.
        let plan = GatePlan::new(vec![
            check("build", CheckCost::Deterministic),
            GateCheck::requires_all("tests", CheckCost::Nondeterministic, ["build"]),
            GateCheck::requires_all("integration", CheckCost::Nondeterministic, ["tests"]),
        ])
        .unwrap();
        let outcome = outcomes(&[(
            "build",
            CheckOutcome::Blocking {
                detail: "workspace build failed: E0425".to_string(),
            },
        )]);
        let verdict = evaluate_gate(&plan, &outcome).unwrap();
        match &verdict {
            GateVerdict::Hold { not_evaluated, .. } => {
                assert_eq!(
                    not_evaluated,
                    &[
                        NotEvaluated {
                            check: "tests".to_string(),
                            because: "build".to_string(),
                        },
                        NotEvaluated {
                            check: "integration".to_string(),
                            because: "build".to_string(),
                        },
                    ],
                    "the transitive skip points at the check that actually blocked"
                );
            }
            other => panic!("expected a hold, got {other:?}"),
        }
    }

    #[test]
    fn a_clean_gate_converts() {
        let plan = GatePlan::new(vec![
            check("lint", CheckCost::Deterministic),
            check("tests", CheckCost::Nondeterministic),
        ])
        .unwrap();
        let outcome = outcomes(&[
            ("lint", CheckOutcome::Clean),
            ("tests", CheckOutcome::Clean),
        ]);
        let verdict = evaluate_gate(&plan, &outcome).unwrap();
        assert_eq!(verdict, GateVerdict::Convert);
        assert!(!verdict.is_hold());
        assert!(verdict.line().starts_with("gate clean"));
    }

    #[test]
    fn findings_are_stable_within_a_confidence_tier() {
        let plan = GatePlan::new(vec![
            check("tests", CheckCost::Nondeterministic),
            check("lint", CheckCost::Deterministic),
            check("format", CheckCost::Deterministic),
        ])
        .unwrap();
        let outcome = outcomes(&[
            (
                "lint",
                CheckOutcome::Blocking {
                    detail: "DOC_OUT_OF_SYNC: env var introduced".to_string(),
                },
            ),
            (
                "format",
                CheckOutcome::Blocking {
                    detail: "diff is not rustfmt-clean".to_string(),
                },
            ),
            (
                "tests",
                CheckOutcome::Blocking {
                    detail: "1 failed".to_string(),
                },
            ),
        ]);
        let verdict = evaluate_gate(&plan, &outcome).unwrap();
        assert_eq!(
            verdict.finding_checks(),
            vec!["lint", "format", "tests"],
            "deterministic first, declared order within a tier"
        );
    }

    #[test]
    fn a_missing_outcome_with_no_blocking_dependency_is_an_error() {
        let plan = GatePlan::new(vec![
            check("lint", CheckCost::Deterministic),
            check("tests", CheckCost::Nondeterministic),
        ])
        .unwrap();
        let outcome = outcomes(&[("lint", CheckOutcome::Clean)]);
        let error = evaluate_gate(&plan, &outcome).unwrap_err();
        assert!(error.contains("tests"), "{error}");
        assert!(error.contains("not evaluated"), "{error}");
    }

    #[test]
    fn an_outcome_for_an_unknown_check_is_an_error() {
        let plan = GatePlan::new(vec![check("lint", CheckCost::Deterministic)]).unwrap();
        let outcome = outcomes(&[("typos", CheckOutcome::Clean)]);
        let error = evaluate_gate(&plan, &outcome).unwrap_err();
        assert!(error.contains("unknown check"), "{error}");
    }

    #[test]
    fn the_plan_rejects_malformed_check_sets() {
        let duplicate = GatePlan::new(vec![
            check("lint", CheckCost::Deterministic),
            check("lint", CheckCost::Deterministic),
        ]);
        assert!(duplicate.is_err());
        assert!(GatePlan::new(vec![check(" ", CheckCost::Deterministic)]).is_err());
        assert!(GatePlan::new(Vec::new()).is_err());
        let unknown_dep = GatePlan::new(vec![GateCheck::requires_all(
            "commit",
            CheckCost::Deterministic,
            ["tests"],
        )]);
        assert!(unknown_dep.is_err());
        let self_dep = GatePlan::new(vec![GateCheck::requires_all(
            "commit",
            CheckCost::Deterministic,
            ["commit"],
        )]);
        assert!(self_dep.is_err());
    }

    #[test]
    fn the_plan_rejects_a_dependency_the_cost_order_cannot_satisfy() {
        // A deterministic check that waits on the flaky suite could never run
        // before it: the cost order puts the suite last, and the check
        // requires it. That is a contradictory plan, not one to guess at.
        let plan = GatePlan::new(vec![
            check("tests", CheckCost::Nondeterministic),
            GateCheck::requires_all("lint", CheckCost::Deterministic, ["tests"]),
        ]);
        let error = plan.unwrap_err();
        assert!(error.contains("cost order"), "{error}");
    }

    #[test]
    fn a_skip_is_never_reported_as_absence_of_a_finding() {
        // The heart of the invariant: when the pipeline stops early, the
        // record distinguishes "this check found a defect" from "this check
        // never ran". An actor hours later must be able to tell the two
        // apart from the record alone.
        let plan = GatePlan::new(vec![
            GateCheck::requires_all("tests", CheckCost::Nondeterministic, ["build"]),
            check("build", CheckCost::Deterministic),
        ])
        .unwrap();
        let outcome = outcomes(&[(
            "build",
            CheckOutcome::Blocking {
                detail: "1 error".to_string(),
            },
        )]);
        let line = evaluate_gate(&plan, &outcome).unwrap().line();
        let skipped = line
            .find("not evaluated: tests (blocked by build)")
            .unwrap();
        let findings_prefix = &line[..skipped];
        assert!(
            !findings_prefix.contains("tests"),
            "the skipped check is not reported as a finding: {line}"
        );
    }
}
