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
//! 4. **Normalise before judging.** A gate must not reject work for a
//!    defect it could repair deterministically ([`Normaliser`]): fourteen of
//!    twenty held patches carried one identical reason — unformatted at
//!    source, never tested — a single mechanical defect outweighing every
//!    real defect combined. Any defect the gate can detect with a
//!    deterministic tool, it can also fix with no judgement and no risk of
//!    altering behaviour, so [`evaluate_gate`] repairs it first and judges
//!    what remains. Formatting alone is never a terminal verdict; the
//!    repair is recorded in the verdict ([`Normalised`]) so the patch that
//!    lands is the normalised one and the log says so, and a hold names
//!    only the defects that survive normalisation.
//! 5. **Unavailability is a third outcome, not a failure.** The caller
//!    asserts each tool's precondition before running it (a fresh clone
//!    has no `node_modules`, so `eslint` is `not found`); a check that
//!    cannot run is recorded as [`CheckOutcome::Unavailable`] carrying the
//!    tool's own message, and the verdict is [`GateVerdict::Unavailable`]
//!    — `GATE-UNAVAILABLE: <why>` — not a hold. A missing tool is a
//!    statement about the toolchain, not the patch: holding on it records
//!    a defect that was never observed and spends the patch's retry
//!    budget on nothing; converting on it ships code the gate never
//!    verified. So unavailable is neither — the patch returns to the queue
//!    untouched, and the pass summary reports converted, held, and
//!    unavailable as three separate counts ([`GateCounts`]) (#4250, where
//!    275 patches were held on `eslint: not found`).

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
    /// The check did not run: its tool or precondition was missing
    /// (`eslint: not found` in a fresh clone without `node_modules`). This
    /// is a state of the toolchain, not a finding about the patch — no
    /// defect was observed — so it must not hold the patch, spend its
    /// retry budget, or mark it known-bad; and it is not a clean run, so
    /// the patch cannot convert on its strength either. The reason is the
    /// tool's own message, verbatim (#4250).
    Unavailable { reason: String },
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

/// One check the gate could not run: its tool or precondition was missing.
/// Unlike a [`HoldFinding`], this is a statement about the toolchain, not
/// the patch — no defect was observed — so it never justifies a hold on
/// its own and consumes no retry budget or known-bad slot (#4250).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnavailableFinding {
    /// The check that could not run.
    pub check: String,
    /// The tool's own message, verbatim: `eslint: not found`.
    pub reason: String,
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

/// A deterministic repair the pipeline can apply to a candidate patch
/// before judging it: the formatter (`cargo fmt`), an import-ordering
/// pass, trailing-whitespace stripping.
///
/// A normaliser is registered against the single check whose blocking
/// findings it repairs. Because it is deterministic, total, and semantically
/// neutral, a finding it repairs is a property of the *submission*, not of
/// the *work* — the gate must not hold work over it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Normaliser {
    /// The normaliser's own name, e.g. `cargo fmt`.
    pub name: String,
    /// The check whose blocking findings this normaliser repairs,
    /// e.g. `format`.
    pub repairs: String,
}

/// One repair the gate applied before judging: the check that would have
/// blocked, and the normaliser that repaired it. The patch that lands is
/// the normalised one, and the verdict says so — nothing is silently
/// rewritten.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Normalised {
    /// The check that found the defect.
    pub check: String,
    /// The normaliser that repaired it.
    pub by: String,
}

/// The gate's decision for one patch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateVerdict {
    /// Every evaluated check was clean — after normalisation; the patch may
    /// convert. `normalised` names every repair applied, so the patch that
    /// lands is the normalised one and the record says so.
    Convert { normalised: Vec<Normalised> },
    /// At least one blocking finding that *survives normalisation*. The
    /// record names all of them, most confident first, plus every repair
    /// applied, every check the gate could not run, and every check a
    /// prior failure made impossible to evaluate. A real finding dominates
    /// a broken toolchain: `unavailable` still names the missing tools, so
    /// the operator sees the toolchain state in the same line that names
    /// the defect (#4250).
    Hold {
        findings: Vec<HoldFinding>,
        unavailable: Vec<UnavailableFinding>,
        not_evaluated: Vec<NotEvaluated>,
        normalised: Vec<Normalised>,
    },
    /// No blocking finding was observed, but at least one check could not
    /// run: its tool or precondition was missing. The record names every
    /// such check with the tool's own message, plus every repair applied
    /// and every check the missing tool made impossible to evaluate. The
    /// patch is not converted — it was never verified — and not held:
    /// nothing about it is wrong. It returns to the queue untouched, and
    /// no retry budget or known-bad slot is spent (#4250).
    Unavailable {
        unavailable: Vec<UnavailableFinding>,
        not_evaluated: Vec<NotEvaluated>,
        normalised: Vec<Normalised>,
    },
}

impl GateVerdict {
    /// Whether the patch is held.
    pub fn is_hold(&self) -> bool {
        matches!(self, Self::Hold { .. })
    }

    /// Whether the patch may convert.
    pub fn is_convert(&self) -> bool {
        matches!(self, Self::Convert { .. })
    }

    /// Whether the gate could not verify the patch because a tool or
    /// precondition was missing: neither held nor converted (#4250).
    pub fn is_unavailable(&self) -> bool {
        matches!(self, Self::Unavailable { .. })
    }

    /// The names of the checks behind the blocking findings, record order.
    /// A finding here always survived normalisation. An unavailable gate
    /// has none: no defect was observed.
    pub fn finding_checks(&self) -> Vec<&str> {
        match self {
            Self::Convert { .. } => Vec::new(),
            Self::Hold { findings, .. } => findings.iter().map(|f| f.check.as_str()).collect(),
            Self::Unavailable { .. } => Vec::new(),
        }
    }

    /// The names of the checks the gate could not run, record order.
    pub fn unavailable_checks(&self) -> Vec<&str> {
        match self {
            Self::Convert { .. } => Vec::new(),
            Self::Hold { unavailable, .. } => {
                unavailable.iter().map(|f| f.check.as_str()).collect()
            }
            Self::Unavailable { unavailable, .. } => {
                unavailable.iter().map(|f| f.check.as_str()).collect()
            }
        }
    }

    /// Every repair applied before judging, in plan order.
    pub fn normalised(&self) -> &[Normalised] {
        match self {
            Self::Convert { normalised } => normalised,
            Self::Hold { normalised, .. } => normalised,
            Self::Unavailable { normalised, .. } => normalised,
        }
    }

    /// The durable record: every blocking finding, most confident first,
    /// followed by every recorded repair and every recorded skip. An actor
    /// reading this line hours later finds the real blocker at the front,
    /// not the flake — and can tell a repaired defect from a held one.
    pub fn line(&self) -> String {
        match self {
            Self::Convert { normalised } => {
                if normalised.is_empty() {
                    "gate clean: every evaluated check found nothing".to_string()
                } else {
                    format!(
                        "gate clean: every evaluated check found nothing; {}",
                        normalised_clause(normalised)
                    )
                }
            }
            Self::Hold {
                findings,
                unavailable,
                not_evaluated,
                normalised,
            } => {
                let mut parts = findings
                    .iter()
                    .map(|finding| format!("{}: {}", finding.check, finding.detail))
                    .collect::<Vec<_>>();
                if !unavailable.is_empty() {
                    parts.push(unavailable_clause(unavailable));
                }
                if !normalised.is_empty() {
                    parts.push(normalised_clause(normalised));
                }
                for skipped in not_evaluated {
                    parts.push(format!(
                        "not evaluated: {} (blocked by {})",
                        skipped.check, skipped.because
                    ));
                }
                format!("HELD: {}", parts.join("; "))
            }
            Self::Unavailable {
                unavailable,
                not_evaluated,
                normalised,
            } => {
                let mut parts = unavailable
                    .iter()
                    .map(|finding| format!("{}: {}", finding.check, finding.reason))
                    .collect::<Vec<_>>();
                if !normalised.is_empty() {
                    parts.push(normalised_clause(normalised));
                }
                for skipped in not_evaluated {
                    parts.push(format!(
                        "not evaluated: {} (blocked by {})",
                        skipped.check, skipped.because
                    ));
                }
                format!("GATE-UNAVAILABLE: {}", parts.join("; "))
            }
        }
    }
}

/// The record clause for checks the gate could not run:
/// `unavailable: lint (eslint: not found)`.
fn unavailable_clause(unavailable: &[UnavailableFinding]) -> String {
    format!(
        "unavailable: {}",
        unavailable
            .iter()
            .map(|finding| format!("{} ({})", finding.check, finding.reason))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// The record clause for applied repairs: `normalised: format by cargo fmt`.
fn normalised_clause(normalised: &[Normalised]) -> String {
    format!(
        "normalised: {}",
        normalised
            .iter()
            .map(|record| format!("{} by {}", record.check, record.by))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// The internal decision recorded for one check while walking the plan.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Decision {
    Clean,
    /// The gate cannot get past this point; the named check is the one that
    /// actually failed. A skipped check carries its dependency's reason,
    /// so a transitive skip still points at the real blocker.
    Blocked(String),
    /// The gate could not run the named check at all: its tool or
    /// precondition was missing. A skipped check carries its dependency's
    /// name, so a transitive skip points at the check that never ran.
    Unavailable(String),
}

/// Validate the normalisers and map each repaired check to the normaliser
/// that repairs it.
///
/// A normaliser may only repair a *deterministic* check: a deterministic,
/// semantically neutral rewrite cannot undo a measured failure, so
/// registering one against the flaky suite would turn a real defect into a
/// silent pass. Fails closed on empty names, unknown checks, and two
/// normalisers claiming the same check.
fn normaliser_map<'a>(
    plan: &GatePlan,
    normalisers: &'a [Normaliser],
) -> Result<BTreeMap<&'a str, &'a str>, String> {
    let mut map: BTreeMap<&str, &str> = BTreeMap::new();
    for normaliser in normalisers {
        if normaliser.name.trim().is_empty() || normaliser.repairs.trim().is_empty() {
            return Err("normaliser names and repaired checks must be nonempty".to_string());
        }
        let check = plan.check(&normaliser.repairs).ok_or_else(|| {
            format!(
                "normaliser {} repairs unknown check {}",
                normaliser.name, normaliser.repairs
            )
        })?;
        if check.cost != CheckCost::Deterministic {
            return Err(format!(
                "normaliser {} cannot repair {}: a deterministic rewrite cannot undo a measured failure",
                normaliser.name,
                normaliser.repairs
            ));
        }
        if map
            .insert(normaliser.repairs.as_str(), normaliser.name.as_str())
            .is_some()
        {
            return Err(format!("two normalisers repair {}", normaliser.repairs));
        }
    }
    Ok(map)
}

/// Evaluate a gate run, normalising before judging.
///
/// `normalisers` are the deterministic repairs the pipeline can apply to a
/// candidate patch (formatter, import ordering, trailing whitespace), each
/// registered against the check it repairs. `outcomes` holds the result of
/// every check the pipeline actually ran, in no particular order.
///
/// The walk is the same as before, with one difference at the heart of the
/// invariant: a blocking finding on a check that has a normaliser is a
/// property of the *submission*, not of the *work*. It is repaired, recorded
/// as a [`Normalised`] entry in the verdict, and the check resolves clean —
/// so the checks that depend on it (the test stage) still run, instead of
/// being discarded along with the whitespace. Formatting alone is therefore
/// never a terminal verdict.
///
/// Every check in the plan either has an outcome or is recorded as not
/// evaluated against the first surviving blocking finding (in plan order)
/// that makes it impossible — including transitively, through a dependency
/// that was itself skipped. A check with no outcome *and* no such prior
/// failure is a pipeline that stopped without a reason, which is an error,
/// not a silent pass or a silent skip.
///
/// The hold record collects every blocking finding that *survives*
/// normalisation, ordered by confidence (deterministic first, plan order
/// within a tier), followed by the repair record and every recorded skip.
///
/// A check whose tool or precondition was missing reports
/// [`CheckOutcome::Unavailable`]. That is a statement about the toolchain,
/// not the patch: it is recorded, it makes its dependents not evaluated
/// (named against it), and it produces [`GateVerdict::Unavailable`] —
/// `GATE-UNAVAILABLE` — whenever no blocking finding survived. A blocking
/// finding still dominates: a real defect holds the patch, and the record
/// names the missing tools in the same line. An unavailable outcome with
/// an empty reason is an error: the record must name what was missing,
/// verbatim (#4250).
pub fn evaluate_gate(
    plan: &GatePlan,
    normalisers: &[Normaliser],
    outcomes: &BTreeMap<String, CheckOutcome>,
) -> Result<GateVerdict, String> {
    let repaired = normaliser_map(plan, normalisers)?;
    for name in outcomes.keys() {
        if plan.check(name).is_none() {
            return Err(format!("gate outcome for unknown check: {name}"));
        }
    }
    let mut decided: BTreeMap<&str, Decision> = BTreeMap::new();
    let mut findings: Vec<HoldFinding> = Vec::new();
    let mut unavailable: Vec<UnavailableFinding> = Vec::new();
    let mut not_evaluated: Vec<NotEvaluated> = Vec::new();
    let mut normalised: Vec<Normalised> = Vec::new();
    for check in plan.checks() {
        match outcomes.get(&check.name) {
            Some(CheckOutcome::Clean) => {
                decided.insert(check.name.as_str(), Decision::Clean);
            }
            Some(CheckOutcome::Blocking { detail }) => {
                match repaired.get(check.name.as_str()) {
                    Some(by) => {
                        // The defect is one the gate can repair
                        // deterministically: repair it, record it, and judge
                        // the rest of the patch as normal.
                        normalised.push(Normalised {
                            check: check.name.clone(),
                            by: (*by).to_string(),
                        });
                        decided.insert(check.name.as_str(), Decision::Clean);
                    }
                    None => {
                        findings.push(HoldFinding {
                            check: check.name.clone(),
                            confidence: check.cost.confidence(),
                            detail: detail.clone(),
                        });
                        decided.insert(check.name.as_str(), Decision::Blocked(check.name.clone()));
                    }
                }
            }
            Some(CheckOutcome::Unavailable { reason }) => {
                // The tool or its precondition was missing: a statement
                // about the toolchain, not a finding about the patch.
                // Recorded, never held on, never converted on.
                let reason = reason.trim();
                if reason.is_empty() {
                    return Err(format!(
                        "gate check {} is unavailable with no reason: the record must name \
                         what was missing, verbatim — a skip is a recorded skip, not an \
                         unrecorded absence",
                        check.name
                    ));
                }
                unavailable.push(UnavailableFinding {
                    check: check.name.clone(),
                    reason: reason.to_string(),
                });
                decided.insert(
                    check.name.as_str(),
                    Decision::Unavailable(check.name.clone()),
                );
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
    if !findings.is_empty() {
        // A real finding dominates a broken toolchain: the patch is held on
        // the defect, and the record still names the tools that were
        // missing, in the same line.
        Ok(GateVerdict::Hold {
            findings,
            unavailable,
            not_evaluated,
            normalised,
        })
    } else if unavailable.is_empty() {
        Ok(GateVerdict::Convert { normalised })
    } else {
        // No defect was observed, but the gate could not verify the patch:
        // neither converted (never verified) nor held (nothing is wrong).
        // The patch returns to the queue untouched; no retry budget or
        // known-bad slot is spent (#4250).
        Ok(GateVerdict::Unavailable {
            unavailable,
            not_evaluated,
            normalised,
        })
    }
}

/// The three outcomes of a conversion pass, counted separately: converted
/// (every check ran clean), held (a real finding), and unavailable (a tool
/// or precondition was missing). Unavailable is its own count — never
/// folded into held — because holding a patch on a broken toolchain spends
/// its retry budget and marks it known-bad on a state of the toolchain,
/// not the patch (#4250).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GateCounts {
    /// Patches the gate let through.
    pub converted: usize,
    /// Patches held on a blocking finding.
    pub held: usize,
    /// Patches the gate could not verify because a tool or precondition
    /// was missing.
    pub unavailable: usize,
}

impl GateCounts {
    /// Count the outcomes of one pass.
    pub fn of(verdicts: &[GateVerdict]) -> Self {
        let mut counts = Self {
            converted: 0,
            held: 0,
            unavailable: 0,
        };
        for verdict in verdicts {
            match verdict {
                GateVerdict::Convert { .. } => counts.converted += 1,
                GateVerdict::Hold { .. } => counts.held += 1,
                GateVerdict::Unavailable { .. } => counts.unavailable += 1,
            }
        }
        counts
    }

    /// The pass summary line: `31 converted / 4 held / 240 unavailable`.
    pub fn line(&self) -> String {
        format!(
            "{} converted / {} held / {} unavailable",
            self.converted, self.held, self.unavailable
        )
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
            Some(Decision::Unavailable(reason)) => return Ok(reason.clone()),
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
            "gate check {} was not evaluated and no prior blocking finding or unavailable \
             check makes it impossible; a skip is a recorded skip, not an unrecorded absence",
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
        let verdict = evaluate_gate(&plan, &[], &outcome).unwrap();
        match &verdict {
            GateVerdict::Hold {
                findings,
                unavailable,
                not_evaluated,
                normalised,
            } => {
                assert!(normalised.is_empty(), "nothing is normalised here");
                assert!(unavailable.is_empty(), "no tool was missing here");
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
        let verdict = evaluate_gate(&plan, &[], &outcome).unwrap();
        match &verdict {
            GateVerdict::Hold {
                findings,
                unavailable,
                not_evaluated,
                normalised,
            } => {
                assert!(normalised.is_empty(), "nothing is normalised here");
                assert!(unavailable.is_empty(), "no tool was missing here");
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
        let verdict = evaluate_gate(&plan, &[], &outcome).unwrap();
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
        let verdict = evaluate_gate(&plan, &[], &outcome).unwrap();
        assert_eq!(verdict, GateVerdict::Convert { normalised: vec![] });
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
        let verdict = evaluate_gate(&plan, &[], &outcome).unwrap();
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
        let error = evaluate_gate(&plan, &[], &outcome).unwrap_err();
        assert!(error.contains("tests"), "{error}");
        assert!(error.contains("not evaluated"), "{error}");
    }

    #[test]
    fn an_outcome_for_an_unknown_check_is_an_error() {
        let plan = GatePlan::new(vec![check("lint", CheckCost::Deterministic)]).unwrap();
        let outcome = outcomes(&[("typos", CheckOutcome::Clean)]);
        let error = evaluate_gate(&plan, &[], &outcome).unwrap_err();
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
        let line = evaluate_gate(&plan, &[], &outcome).unwrap().line();
        let skipped = line
            .find("not evaluated: tests (blocked by build)")
            .unwrap();
        let findings_prefix = &line[..skipped];
        assert!(
            !findings_prefix.contains("tests"),
            "the skipped check is not reported as a finding: {line}"
        );
    }
    // ---- Normalise before judging ----------------------------------------

    #[test]
    fn an_unformatted_but_correct_patch_reaches_the_test_stage_and_passes() {
        // Regression: fourteen of twenty held patches carried one identical
        // reason — "unformatted at source, no local run". The patch was never
        // evaluated; the gate declined to test it over whitespace. The test
        // stage depends on the format check, so without normalisation it never
        // runs; with it, the repair is recorded and the verdict is convert.
        let plan = GatePlan::new(vec![
            GateCheck::independent("format", CheckCost::Deterministic),
            GateCheck::requires_all("tests", CheckCost::Nondeterministic, ["format"]),
        ])
        .unwrap();
        let outcome = outcomes(&[
            (
                "format",
                CheckOutcome::Blocking {
                    detail: "diff is not rustfmt-clean".to_string(),
                },
            ),
            ("tests", CheckOutcome::Clean),
        ]);
        let normalisers = [Normaliser {
            name: "cargo fmt".to_string(),
            repairs: "format".to_string(),
        }];
        let verdict = evaluate_gate(&plan, &normalisers, &outcome).unwrap();
        assert!(
            !verdict.is_hold(),
            "formatting alone is never a terminal verdict: {}",
            verdict.line()
        );
        assert_eq!(
            verdict,
            GateVerdict::Convert {
                normalised: vec![Normalised {
                    check: "format".to_string(),
                    by: "cargo fmt".to_string(),
                }]
            }
        );
        let line = verdict.line();
        assert!(line.contains("cargo fmt"), "{line}");
    }

    #[test]
    fn a_hold_names_only_defects_that_survive_normalisation() {
        let plan = GatePlan::new(vec![
            GateCheck::independent("format", CheckCost::Deterministic),
            GateCheck::independent("lint", CheckCost::Deterministic),
            GateCheck::independent("tests", CheckCost::Nondeterministic),
        ])
        .unwrap();
        let outcome = outcomes(&[
            (
                "format",
                CheckOutcome::Blocking {
                    detail: "diff is not rustfmt-clean".to_string(),
                },
            ),
            (
                "lint",
                CheckOutcome::Blocking {
                    detail: "DOC_OUT_OF_SYNC: README".to_string(),
                },
            ),
            ("tests", CheckOutcome::Clean),
        ]);
        let normalisers = [Normaliser {
            name: "cargo fmt".to_string(),
            repairs: "format".to_string(),
        }];
        let verdict = evaluate_gate(&plan, &normalisers, &outcome).unwrap();
        assert!(verdict.is_hold());
        // The format defect was repaired, so it is not a finding — the hold
        // names only the lint defect that survived normalisation.
        assert_eq!(verdict.finding_checks(), vec!["lint"]);
        let line = verdict.line();
        assert!(line.contains("lint: DOC_OUT_OF_SYNC: README"), "{line}");
        assert!(line.contains("normalised: format by cargo fmt"), "{line}");
        assert!(
            !line.contains("format: diff is not rustfmt-clean"),
            "{line}"
        );
    }

    #[test]
    fn a_clean_check_is_not_recorded_as_normalised() {
        let plan = GatePlan::new(vec![GateCheck::independent(
            "format",
            CheckCost::Deterministic,
        )])
        .unwrap();
        let outcome = outcomes(&[("format", CheckOutcome::Clean)]);
        let normalisers = [Normaliser {
            name: "cargo fmt".to_string(),
            repairs: "format".to_string(),
        }];
        let verdict = evaluate_gate(&plan, &normalisers, &outcome).unwrap();
        // Nothing was repaired, so nothing is recorded: the record names
        // repairs actually applied, not normalisers merely available.
        assert_eq!(verdict, GateVerdict::Convert { normalised: vec![] });
    }

    #[test]
    fn a_normalised_patch_still_blocks_dependent_checks_on_a_real_defect() {
        // Format is repaired, but the test stage it unblocks fails for a real
        // reason: the hold names the surviving defect, and the record still
        // says the format repair happened.
        let plan = GatePlan::new(vec![
            GateCheck::independent("format", CheckCost::Deterministic),
            GateCheck::requires_all("tests", CheckCost::Nondeterministic, ["format"]),
        ])
        .unwrap();
        let outcome = outcomes(&[
            (
                "format",
                CheckOutcome::Blocking {
                    detail: "diff is not rustfmt-clean".to_string(),
                },
            ),
            (
                "tests",
                CheckOutcome::Blocking {
                    detail: "2 failures: gate_eval".to_string(),
                },
            ),
        ]);
        let normalisers = [Normaliser {
            name: "cargo fmt".to_string(),
            repairs: "format".to_string(),
        }];
        let verdict = evaluate_gate(&plan, &normalisers, &outcome).unwrap();
        assert!(verdict.is_hold());
        assert_eq!(verdict.finding_checks(), vec!["tests"]);
        let line = verdict.line();
        assert!(line.contains("tests: 2 failures: gate_eval"), "{line}");
        assert!(line.contains("normalised: format by cargo fmt"), "{line}");
    }

    #[test]
    fn a_normaliser_cannot_repair_a_nondeterministic_check() {
        // A deterministic, semantically neutral rewrite cannot undo a measured
        // failure. Registering one against the flaky suite would turn a real
        // defect into a silent pass — the exact failure mode this module exists
        // to prevent, so the configuration is rejected, fail-closed.
        let plan = GatePlan::new(vec![GateCheck::independent(
            "tests",
            CheckCost::Nondeterministic,
        )])
        .unwrap();
        let outcome = outcomes(&[(
            "tests",
            CheckOutcome::Blocking {
                detail: "flake".to_string(),
            },
        )]);
        let normalisers = [Normaliser {
            name: "retry".to_string(),
            repairs: "tests".to_string(),
        }];
        let error = evaluate_gate(&plan, &normalisers, &outcome).unwrap_err();
        assert!(error.contains("cannot repair tests"), "{error}");
    }

    #[test]
    fn normaliser_configuration_fails_closed() {
        let plan = GatePlan::new(vec![
            GateCheck::independent("format", CheckCost::Deterministic),
            GateCheck::independent("imports", CheckCost::Deterministic),
        ])
        .unwrap();
        let outcome = outcomes(&[
            ("format", CheckOutcome::Clean),
            ("imports", CheckOutcome::Clean),
        ]);

        let unknown = [Normaliser {
            name: "cargo fmt".to_string(),
            repairs: "lint".to_string(),
        }];
        let error = evaluate_gate(&plan, &unknown, &outcome).unwrap_err();
        assert!(error.contains("unknown check lint"), "{error}");

        let ambiguous = [
            Normaliser {
                name: "cargo fmt".to_string(),
                repairs: "format".to_string(),
            },
            Normaliser {
                name: "import-order".to_string(),
                repairs: "format".to_string(),
            },
        ];
        let error = evaluate_gate(&plan, &ambiguous, &outcome).unwrap_err();
        assert!(error.contains("two normalisers repair format"), "{error}");

        let empty = [Normaliser {
            name: "  ".to_string(),
            repairs: "format".to_string(),
        }];
        let error = evaluate_gate(&plan, &empty, &outcome).unwrap_err();
        assert!(error.contains("nonempty"), "{error}");
    }
    // ---- Unavailability is a third outcome, not a failure -----------------

    #[test]
    fn a_missing_tool_is_unavailable_not_held() {
        // Regression (#4250): the conversion pass ran `eslint .` in a fresh
        // clone without `node_modules` and recorded 275 patches as failed.
        // The tool was missing; no defect was observed. The verdict is
        // GATE-UNAVAILABLE, naming the tool's own message verbatim.
        let plan = GatePlan::new(vec![
            check("lint", CheckCost::Deterministic),
            check("build", CheckCost::Deterministic),
        ])
        .unwrap();
        let outcome = outcomes(&[
            (
                "lint",
                CheckOutcome::Unavailable {
                    reason: "eslint: not found".to_string(),
                },
            ),
            ("build", CheckOutcome::Clean),
        ]);
        let verdict = evaluate_gate(&plan, &[], &outcome).unwrap();
        assert!(verdict.is_unavailable());
        assert!(!verdict.is_hold(), "nothing about the patch is wrong");
        assert!(verdict.finding_checks().is_empty());
        let line = verdict.line();
        assert_eq!(
            line, "GATE-UNAVAILABLE: lint: eslint: not found",
            "the record names the tool's own message verbatim"
        );
    }

    #[test]
    fn a_real_defect_still_holds_when_a_tool_is_missing() {
        // A blocking finding dominates a broken toolchain: the patch is
        // held on the defect, and the missing tool is named in the same
        // line, so the operator sees the toolchain state next to the
        // finding.
        let plan = GatePlan::new(vec![
            check("lint", CheckCost::Deterministic),
            check("typecheck", CheckCost::Deterministic),
        ])
        .unwrap();
        let outcome = outcomes(&[
            (
                "lint",
                CheckOutcome::Unavailable {
                    reason: "eslint: not found".to_string(),
                },
            ),
            (
                "typecheck",
                CheckOutcome::Blocking {
                    detail: "src/x.ts(3,1): error TS2304: Cannot find name 'foo'".to_string(),
                },
            ),
        ]);
        let verdict = evaluate_gate(&plan, &[], &outcome).unwrap();
        assert!(verdict.is_hold());
        assert!(!verdict.is_unavailable());
        assert_eq!(verdict.finding_checks(), vec!["typecheck"]);
        assert_eq!(verdict.unavailable_checks(), vec!["lint"]);
        let line = verdict.line();
        assert!(line.starts_with("HELD:"), "{line}");
        assert!(line.contains("typecheck: src/x.ts(3,1)"), "{line}");
        assert!(
            line.contains("unavailable: lint (eslint: not found)"),
            "{line}"
        );
    }

    #[test]
    fn unavailable_dependents_are_recorded_not_evaluated() {
        // The typecheck stage cannot run without the lint tooling it sits
        // behind. The skip is recorded, named against the check that never
        // ran — never silently dropped, and never counted as a finding.
        let plan = GatePlan::new(vec![
            check("lint", CheckCost::Deterministic),
            GateCheck::requires_all("typecheck", CheckCost::Deterministic, ["lint"]),
        ])
        .unwrap();
        let outcome = outcomes(&[(
            "lint",
            CheckOutcome::Unavailable {
                reason: "eslint: not found".to_string(),
            },
        )]);
        let verdict = evaluate_gate(&plan, &[], &outcome).unwrap();
        match &verdict {
            GateVerdict::Unavailable {
                unavailable,
                not_evaluated,
                normalised,
            } => {
                assert!(normalised.is_empty());
                assert_eq!(
                    unavailable,
                    &[UnavailableFinding {
                        check: "lint".to_string(),
                        reason: "eslint: not found".to_string(),
                    }]
                );
                assert_eq!(
                    not_evaluated,
                    &[NotEvaluated {
                        check: "typecheck".to_string(),
                        because: "lint".to_string(),
                    }]
                );
            }
            other => panic!("expected unavailable, got {other:?}"),
        }
        let line = verdict.line();
        assert_eq!(
            line,
            "GATE-UNAVAILABLE: lint: eslint: not found; not evaluated: typecheck (blocked by lint)"
        );
    }

    #[test]
    fn a_missing_tool_consumes_no_retry_budget_or_known_bad_slot() {
        // The record must give the actor a basis for neither holding
        // (no finding exists) nor converting (the patch was never
        // verified): only the unavailable count moves, the patch returns
        // to the queue untouched.
        let plan = GatePlan::new(vec![
            check("lint", CheckCost::Deterministic),
            check("build", CheckCost::Deterministic),
        ])
        .unwrap();
        let outcome = outcomes(&[
            (
                "lint",
                CheckOutcome::Unavailable {
                    reason: "eslint: not found".to_string(),
                },
            ),
            ("build", CheckOutcome::Clean),
        ]);
        let verdict = evaluate_gate(&plan, &[], &outcome).unwrap();
        assert!(verdict.is_unavailable());
        assert!(!verdict.is_hold() && !verdict.is_convert());
        let counts = GateCounts::of(&[verdict]);
        assert_eq!(
            counts,
            GateCounts {
                converted: 0,
                held: 0,
                unavailable: 1
            }
        );
        assert_eq!(counts.line(), "0 converted / 0 held / 1 unavailable");
    }

    #[test]
    fn an_unavailable_outcome_without_a_reason_is_an_error() {
        // The record must name what was missing, verbatim. An unavailable
        // outcome with no reason is a pipeline that stopped without a
        // reason — an error, not a silent unavailable.
        let plan = GatePlan::new(vec![check("lint", CheckCost::Deterministic)]).unwrap();
        let outcome = outcomes(&[(
            "lint",
            CheckOutcome::Unavailable {
                reason: "  ".to_string(),
            },
        )]);
        let error = evaluate_gate(&plan, &[], &outcome).unwrap_err();
        assert!(error.contains("no reason"), "{error}");
    }

    #[test]
    fn a_missing_tool_is_not_repaired_by_a_normaliser() {
        // A normaliser repairs a defect the check observed; it cannot
        // conjure a missing tool. A registered normaliser for an
        // unavailable check is ignored: the verdict stays unavailable.
        let plan = GatePlan::new(vec![check("format", CheckCost::Deterministic)]).unwrap();
        let outcome = outcomes(&[(
            "format",
            CheckOutcome::Unavailable {
                reason: "rustfmt not found".to_string(),
            },
        )]);
        let normalisers = [Normaliser {
            name: "cargo fmt".to_string(),
            repairs: "format".to_string(),
        }];
        let verdict = evaluate_gate(&plan, &normalisers, &outcome).unwrap();
        assert!(verdict.is_unavailable());
        assert!(
            verdict.normalised().is_empty(),
            "nothing was repaired: no defect was observed"
        );
        assert_eq!(
            verdict.line(),
            "GATE-UNAVAILABLE: format: rustfmt not found"
        );
    }

    #[test]
    fn the_gate_counts_report_the_three_outcomes_separately() {
        // 31 converted, 4 held, 240 unavailable — never 275 held.
        let plan = GatePlan::new(vec![
            check("lint", CheckCost::Deterministic),
            check("build", CheckCost::Deterministic),
        ])
        .unwrap();
        let convert = evaluate_gate(
            &plan,
            &[],
            &outcomes(&[
                ("lint", CheckOutcome::Clean),
                ("build", CheckOutcome::Clean),
            ]),
        )
        .unwrap();
        let hold = evaluate_gate(
            &plan,
            &[],
            &outcomes(&[
                (
                    "lint",
                    CheckOutcome::Blocking {
                        detail: "DOC_OUT_OF_SYNC: env var introduced".to_string(),
                    },
                ),
                ("build", CheckOutcome::Clean),
            ]),
        )
        .unwrap();
        let unavailable = evaluate_gate(
            &plan,
            &[],
            &outcomes(&[
                (
                    "lint",
                    CheckOutcome::Unavailable {
                        reason: "eslint: not found".to_string(),
                    },
                ),
                ("build", CheckOutcome::Clean),
            ]),
        )
        .unwrap();
        let verdicts = vec![convert.clone(), convert, hold, unavailable];
        let counts = GateCounts::of(&verdicts);
        assert_eq!(
            counts,
            GateCounts {
                converted: 2,
                held: 1,
                unavailable: 1
            }
        );
        assert_eq!(counts.line(), "2 converted / 1 held / 1 unavailable");
    }
}
