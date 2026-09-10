//! Base-result reuse for conversion gates (#3782).
//!
//! The conversion pass re-ran the full gate set on every candidate: 107
//! patches, 132-304s of verification each, most of it in gates whose
//! verdict the patch could not have changed. A gate's declared inputs are
//! not asserted but *derived from the checks themselves*: if a change
//! touches none of a gate's inputs, the gate's verdict cannot have
//! changed, and the gate reuses the base result instead of re-running.
//!
//! The policy is encoded here as pure, testable primitives; callers execute
//! the gates the decisions name and act on the records these functions
//! return:
//!
//! 1. **A gate's inputs are derived from its checks** ([`Gate::inputs`]):
//!    the union of what the checks read. A per-gate input list maintained by
//!    hand can drift from the checks; a union cannot.
//! 2. **A gate whose inputs cannot be derived is not skippable**
//!    ([`Check::opaque`]): an opaque check is assumed to read everything,
//!    fail-closed.
//! 3. **A change touching none of a gate's inputs reuses the base result**
//!    ([`RunDecision::ReuseBase`]), and a base result is computed at most
//!    once per base revision ([`BaseResultCache`]).
//! 4. **A reused gate is recorded as skipped, never as passed**
//!    ([`GateRecord`]): the record names the revision the decision rests
//!    on and the outcome it reuses, because a bare `skipped` reads green.
//! 5. **The pass reports what it spent** ([`RunCost`]) and the projected
//!    drain of the candidate queue is derivable from the observed
//!    per-candidate cost ([`projected_drain`]).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// The set of files a check reads, derived from the check.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inputs {
    /// Exact repository-relative paths: `Cargo.toml`, `scripts/x.sh`.
    files: BTreeSet<String>,
    /// Directory prefixes, ending in `/`: `crates/`, `apps/web/`.
    directories: BTreeSet<String>,
}

impl Inputs {
    /// Derive the inputs from the check: every file it reads.
    ///
    /// A path ending in `/` is a directory prefix; any other path is an
    /// exact file. All paths are repository-relative.
    pub fn reading(paths: impl IntoIterator<Item = impl Into<String>>) -> Self {
        let mut inputs = Self::default();
        for path in paths {
            let path: String = path.into();
            if path.ends_with('/') {
                inputs.directories.insert(path);
            } else {
                inputs.files.insert(path);
            }
        }
        inputs
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty() && self.directories.is_empty()
    }

    /// Whether any of `changed` falls inside these inputs.
    pub fn touches(&self, changed: &[String]) -> bool {
        changed.iter().any(|path| self.matches(path))
    }

    /// Whether one repository-relative path falls inside these inputs.
    pub fn matches(&self, path: &str) -> bool {
        self.files.contains(path)
            || self
                .directories
                .iter()
                .any(|dir| path.starts_with(dir.as_str()))
    }

    /// The union with `other`: the files the checks of one gate read.
    pub fn union(&self, other: &Inputs) -> Inputs {
        Inputs {
            files: &self.files | &other.files,
            directories: &self.directories | &other.directories,
        }
    }

    pub fn files(&self) -> &BTreeSet<String> {
        &self.files
    }

    pub fn directories(&self) -> &BTreeSet<String> {
        &self.directories
    }
}

/// One check of a gate: what it is called and what it reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Check {
    name: String,
    /// `None` when the check's inputs cannot be derived from the check.
    inputs: Option<Inputs>,
}

impl Check {
    /// A check whose inputs are derived: it names every file it reads.
    pub fn reading(name: impl Into<String>, inputs: Inputs) -> Self {
        Self {
            name: name.into(),
            inputs: Some(inputs),
        }
    }

    /// A check whose inputs cannot be derived from the check itself
    /// (#3782): the check is opaque, and its gate is not skippable.
    pub fn opaque(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            inputs: None,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// The files this check reads, `None` when they cannot be derived.
    pub fn inputs(&self) -> Option<&Inputs> {
        self.inputs.as_ref()
    }

    pub fn is_opaque(&self) -> bool {
        self.inputs.is_none()
    }
}

/// A gate: the named set of checks whose combined verdict decides the patch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gate {
    name: String,
    checks: Vec<Check>,
}

impl Gate {
    pub fn new(
        name: impl Into<String>,
        checks: impl IntoIterator<Item = Check>,
    ) -> Result<Self, String> {
        let name = name.into();
        if name.is_empty() {
            return Err("a gate needs a name".to_string());
        }
        let checks: Vec<Check> = checks.into_iter().collect();
        if checks.is_empty() {
            return Err(format!("gate {name} needs at least one check"));
        }
        let mut seen = BTreeSet::new();
        for check in &checks {
            if !seen.insert(check.name()) {
                return Err(format!("gate {name} lists check {} twice", check.name()));
            }
        }
        Ok(Self { name, checks })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn checks(&self) -> &[Check] {
        &self.checks
    }

    /// The files this gate's verdict depends on: the union of the inputs of
    /// its checks, derived, not asserted.
    ///
    /// `None` when any check's inputs cannot be derived (#3782): the gate
    /// is not skippable. Failing closed means the gate runs - never that an
    /// opaque gate reuses a base result it cannot be shown not to depend on.
    pub fn inputs(&self) -> Option<Inputs> {
        let mut inputs = Inputs::default();
        for check in &self.checks {
            let check_inputs = check.inputs()?;
            inputs = inputs.union(check_inputs);
        }
        Some(inputs)
    }

    /// Whether `changed` can change this gate's verdict. A gate whose
    /// inputs cannot be derived can always be changed: it must run.
    pub fn can_change(&self, changed: &[String]) -> bool {
        match self.inputs() {
            Some(inputs) => inputs.touches(changed),
            None => true,
        }
    }

    /// The decision for a patch, against the base revision.
    pub fn decide(&self, changed: &[String]) -> RunDecision {
        if self.can_change(changed) {
            RunDecision::Run
        } else {
            RunDecision::ReuseBase
        }
    }
}

/// How a gate is decided for a candidate patch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RunDecision {
    /// The patch can change the gate's verdict - it touches an input the
    /// gate reads, or the gate's inputs cannot be derived: the gate runs.
    Run,
    /// The patch touches none of the gate's inputs: the verdict cannot
    /// have changed, and the base result stands.
    ReuseBase,
}

impl RunDecision {
    pub fn runs(self) -> bool {
        matches!(self, Self::Run)
    }
}

/// The outcome a base result carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BaseOutcome {
    Passed,
    Failed,
}

impl BaseOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
        }
    }
}

/// A gate's result on the base revision: the result a candidate patch
/// reuses when it touches none of the gate's inputs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaseResult {
    base_revision: String,
    gate: String,
    outcome: BaseOutcome,
}

impl BaseResult {
    /// A base result is only ever computed for the revision it names.
    pub fn computed_on(
        base_revision: impl Into<String>,
        gate: impl Into<String>,
        outcome: BaseOutcome,
    ) -> Result<Self, String> {
        let base_revision = base_revision.into();
        if base_revision.is_empty() {
            return Err("a base result needs the revision it was computed on".to_string());
        }
        let gate = gate.into();
        if gate.is_empty() {
            return Err("a base result needs the gate it was computed for".to_string());
        }
        Ok(Self {
            base_revision,
            gate,
            outcome,
        })
    }

    pub fn base_revision(&self) -> &str {
        &self.base_revision
    }

    pub fn gate(&self) -> &str {
        &self.gate
    }

    pub fn outcome(&self) -> BaseOutcome {
        self.outcome
    }
}

/// The once-per-base-revision cache of base results.
///
/// A base result is computed at most once per gate per base revision: the
/// first lookup runs the computation and stores the result, and every
/// later lookup returns the stored result. A cache belongs to one revision:
/// a result computed on one revision is not a result on another, so when
/// the base moves the caller opens a new cache rather than reusing this
/// one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaseResultCache {
    base_revision: String,
    by_gate: BTreeMap<String, BaseResult>,
    /// How many results this cache computed: one per gate, never more.
    computed: usize,
}

impl BaseResultCache {
    pub fn for_base(base_revision: impl Into<String>) -> Result<Self, String> {
        let base_revision = base_revision.into();
        if base_revision.is_empty() {
            return Err("a base result cache needs the base revision it belongs to".to_string());
        }
        Ok(Self {
            base_revision,
            by_gate: BTreeMap::new(),
            computed: 0,
        })
    }

    pub fn base_revision(&self) -> &str {
        &self.base_revision
    }

    /// The base result for `gate`, computed at most once per base revision.
    pub fn base_result_for<F: FnOnce() -> BaseOutcome>(
        &mut self,
        gate: &str,
        compute: F,
    ) -> Result<&BaseResult, String> {
        if gate.is_empty() {
            return Err("a base result needs the gate it was computed for".to_string());
        }
        if self.by_gate.get(gate).is_none() {
            let outcome = compute();
            self.computed += 1;
            let result = BaseResult {
                base_revision: self.base_revision.clone(),
                gate: gate.to_string(),
                outcome,
            };
            self.by_gate.insert(gate.to_string(), result);
        }
        Ok(self
            .by_gate
            .get(gate)
            .expect("stored in this call or already present"))
    }

    /// How many base results this cache computed: one per gate, never more.
    pub fn computed(&self) -> usize {
        self.computed
    }

    /// The stored base result for `gate`, if the cache holds one.
    pub fn get(&self, gate: &str) -> Option<&BaseResult> {
        self.by_gate.get(gate)
    }
}

/// The decision for a patch, given the base result cache: the gate reuses
/// the base only when the base result exists. On the first run the gate
/// runs and its result becomes the base result for the revision.
pub fn decide_with_base(gate: &Gate, changed: &[String], base: &BaseResultCache) -> RunDecision {
    match gate.decide(changed) {
        RunDecision::ReuseBase if base.get(gate.name()).is_some() => RunDecision::ReuseBase,
        _ => RunDecision::Run,
    }
}

/// How a gate is recorded after a pass: it ran, or it reused the base.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GateRecord {
    /// The gate ran and passed.
    Passed,
    /// The gate ran and failed.
    Failed { detail: String },
    /// The gate did not run: the patch touches none of its inputs, and the
    /// base result - computed on the base revision - stands.
    ///
    /// A skip is a skip: it is never recorded as a pass, and it names the
    /// revision the decision rests on and the outcome it reuses, because a
    /// bare `skipped` reads green (#3782).
    ReusedBase {
        base_revision: String,
        base: BaseOutcome,
    },
}

impl GateRecord {
    /// The effective outcome: what the pass may rely on. A reused gate
    /// carries the base outcome, so a patch that cannot change a failing
    /// gate's verdict does not clear the failure.
    pub fn outcome(&self) -> BaseOutcome {
        match self {
            Self::Passed => BaseOutcome::Passed,
            Self::Failed { .. } => BaseOutcome::Failed,
            Self::ReusedBase { base, .. } => *base,
        }
    }

    /// Whether the gate ran in this pass.
    pub fn ran(&self) -> bool {
        !matches!(self, Self::ReusedBase { .. })
    }

    /// Whether this record counts as evidence that the gate's target is
    /// healthy: only a gate that ran and passed. A skip never counts, even
    /// when the base result it reuses was a pass - a decision that rests on
    /// a base revision is not a decision this pass earned.
    pub fn counts_as_passed_evidence(&self) -> bool {
        matches!(self, Self::Passed)
    }

    /// This gate's line in the verdict record.
    pub fn line(&self) -> String {
        match self {
            Self::Passed => "passed".to_string(),
            Self::Failed { detail } => format!("failed ({detail})"),
            Self::ReusedBase {
                base_revision,
                base,
            } => format!("skipped (reused base {base_revision}: {})", base.as_str()),
        }
    }
}

/// The per-gate record of a pass, in gate order: the shape the verdict
/// flows in, including to the PR body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateReport {
    gates: Vec<(String, GateRecord)>,
}

impl GateReport {
    pub fn new(gates: impl IntoIterator<Item = (String, GateRecord)>) -> Result<Self, String> {
        let gates: Vec<(String, GateRecord)> = gates.into_iter().collect();
        if gates.is_empty() {
            return Err("a gate report needs at least one gate".to_string());
        }
        let mut seen = BTreeSet::new();
        for (name, _) in &gates {
            if name.is_empty() {
                return Err("a gate report names every gate".to_string());
            }
            if !seen.insert(name.as_str()) {
                return Err(format!("gate report lists {name} twice"));
            }
        }
        Ok(Self { gates })
    }

    pub fn gates(&self) -> &[(String, GateRecord)] {
        &self.gates
    }

    pub fn gate(&self, name: &str) -> Option<&GateRecord> {
        self.gates.iter().find(|(n, _)| n == name).map(|(_, r)| r)
    }

    /// The gates that did not run in this pass: the skips.
    pub fn skipped(&self) -> impl Iterator<Item = (&str, &GateRecord)> + '_ {
        self.gates
            .iter()
            .filter(|(_, r)| !r.ran())
            .map(|(n, r)| (n.as_str(), r))
    }

    /// Whether the pass holds: any gate's effective outcome is a failure,
    /// including a gate that reused a failing base.
    pub fn is_hold(&self) -> bool {
        self.gates
            .iter()
            .any(|(_, r)| r.outcome() == BaseOutcome::Failed)
    }

    /// The verdict line: the gates that ran, then - in a section of its
    /// own, so a skip can never read as a pass - the gates that did not
    /// run and what they rest on.
    pub fn line(&self) -> String {
        let ran: Vec<String> = self
            .gates
            .iter()
            .filter(|(_, r)| r.ran())
            .map(|(n, r)| format!("{n}: {}", r.line()))
            .collect();
        let skipped: Vec<String> = self
            .skipped()
            .map(|(n, r)| format!("{n} {}", r.line()))
            .collect();
        let mut parts = Vec::new();
        parts.push(if ran.is_empty() {
            "none ran".to_string()
        } else {
            ran.join(", ")
        });
        if !skipped.is_empty() {
            parts.push(format!(
                "skipped (not run, base stands): {}",
                skipped.join(", ")
            ));
        }
        format!("gates: {}", parts.join(" | "))
    }
}

/// One gate's contribution to a pass's verification cost.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateCost {
    gate: String,
    /// The gate's result matched the base revision: its run re-answered a
    /// question the base had already answered.
    matched_base: bool,
    seconds: u64,
}

impl GateCost {
    pub fn new(gate: impl Into<String>, matched_base: bool, seconds: u64) -> Result<Self, String> {
        let gate = gate.into();
        if gate.is_empty() {
            return Err("a gate cost needs the gate it was measured on".to_string());
        }
        Ok(Self {
            gate,
            matched_base,
            seconds,
        })
    }

    pub fn gate(&self) -> &str {
        &self.gate
    }

    pub fn matched_base(&self) -> bool {
        self.matched_base
    }

    pub fn seconds(&self) -> u64 {
        self.seconds
    }
}

/// What a pass reports about its verification cost (#3782): the total time
/// spent in gates, and the subset spent in gates that matched the base -
/// the seconds the base result had already answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunCost {
    total_seconds: u64,
    base_matched_seconds: u64,
}

impl RunCost {
    pub fn from_gates<'a>(gates: impl IntoIterator<Item = &'a GateCost>) -> Self {
        let mut total_seconds = 0u64;
        let mut base_matched_seconds = 0u64;
        for gate in gates {
            total_seconds += gate.seconds;
            if gate.matched_base {
                base_matched_seconds += gate.seconds;
            }
        }
        Self {
            total_seconds,
            base_matched_seconds,
        }
    }

    pub fn total_seconds(&self) -> u64 {
        self.total_seconds
    }

    pub fn base_matched_seconds(&self) -> u64 {
        self.base_matched_seconds
    }

    /// The share of the pass's verification time spent in gates that
    /// matched the base, in `[0, 1]`.
    ///
    /// `None` when the pass spent no time in gates at all: a pass with no
    /// measurement has no share, and reporting 0% for it would read as
    /// "nothing wasted" rather than "nothing measured" (#3782).
    pub fn base_matched_share(&self) -> Option<f64> {
        if self.total_seconds == 0 {
            return None;
        }
        Some(self.base_matched_seconds as f64 / self.total_seconds as f64)
    }

    /// This pass's line in the run log.
    pub fn line(&self) -> String {
        match self.base_matched_share() {
            Some(share) => format!(
                "verification: {}s total, {}s on gates matching base ({:.0}%)",
                self.total_seconds,
                self.base_matched_seconds,
                share * 100.0
            ),
            None => "verification: no gate time measured".to_string(),
        }
    }
}

/// The projected remaining drain of the candidate queue, in seconds.
///
/// Derived from the observed per-candidate cost, not from a model: the
/// median of the observed costs times the candidates remaining. The median
/// rather than the mean because the incident pass measured 132-304s per
/// candidate with 741-1189s outliers, and a projection has to survive the
/// outliers, not chase them. For an even count the lower of the two middle
/// observations is used: a projection that overstates the drain hides
/// queue time, and understating is visible when the projection is wrong.
///
/// `None` when there is no observation - a projection with no evidence is
/// a guess, and a pass must not report a guess as a projection - or when
/// the product overflows.
pub fn projected_drain(remaining: u64, observed_costs_seconds: &[u64]) -> Option<u64> {
    if observed_costs_seconds.is_empty() {
        return None;
    }
    if remaining == 0 {
        return Some(0);
    }
    let mut observed = observed_costs_seconds.to_vec();
    observed.sort_unstable();
    let median = observed[(observed.len() - 1) / 2];
    remaining.checked_mul(median)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo_gates() -> (Gate, Gate) {
        let validate = Gate::new(
            "validate",
            [
                Check::reading("cargo fmt", Inputs::reading(["crates/", "Cargo.toml"])),
                Check::reading(
                    "cargo test",
                    Inputs::reading(["crates/", "Cargo.toml", "Cargo.lock"]),
                ),
            ],
        )
        .unwrap();
        let workflow = Gate::new(
            "workflow",
            [Check::reading(
                "workflow parse",
                Inputs::reading([".github/workflows/"]),
            )],
        )
        .unwrap();
        (validate, workflow)
    }

    fn docs_patch() -> Vec<String> {
        vec!["docs/specs/x.md".to_string()]
    }

    #[test]
    fn patch_outside_every_gate_reuses_base() {
        let (validate, workflow) = repo_gates();
        assert_eq!(validate.decide(&docs_patch()), RunDecision::ReuseBase);
        assert_eq!(workflow.decide(&docs_patch()), RunDecision::ReuseBase);
    }

    #[test]
    fn patch_touching_a_declared_input_runs() {
        let (validate, workflow) = repo_gates();
        let rust_patch = vec!["crates/autospec-core/src/lib.rs".to_string()];
        assert_eq!(validate.decide(&rust_patch), RunDecision::Run);
        assert_eq!(workflow.decide(&rust_patch), RunDecision::ReuseBase);
    }

    /// The regression the issue exists for: a check that reads a path
    /// previously thought irrelevant extends the gate's inputs, and the
    /// scoping decision changes with it. Inputs are derived from the
    /// checks, so a per-gate input list maintained by hand - which the new
    /// check would not have updated - cannot pass this test.
    #[test]
    fn added_check_extends_gate_inputs() {
        let gate = Gate::new(
            "validate",
            [Check::reading(
                "cargo fmt",
                Inputs::reading(["crates/", "Cargo.toml"]),
            )],
        )
        .unwrap();
        assert_eq!(gate.decide(&docs_patch()), RunDecision::ReuseBase);

        let gate = Gate::new(
            "validate",
            [
                Check::reading("cargo fmt", Inputs::reading(["crates/", "Cargo.toml"])),
                Check::reading("markdown lint", Inputs::reading(["docs/"])),
            ],
        )
        .unwrap();
        assert_eq!(gate.decide(&docs_patch()), RunDecision::Run);
    }

    /// Inputs not derivable from the checks: the gate is not skippable.
    #[test]
    fn opaque_check_makes_gate_unskippable() {
        let gate = Gate::new(
            "validate",
            [
                Check::reading("cargo fmt", Inputs::reading(["crates/"])),
                Check::opaque("mystery check"),
            ],
        )
        .unwrap();
        assert_eq!(gate.inputs(), None);
        // Even a patch touching nothing the gate can be shown to read runs.
        assert_eq!(gate.decide(&[]), RunDecision::Run);
    }

    /// The gate reuses the base only when the base result exists.
    #[test]
    fn reuse_requires_a_base_result() {
        let gate = repo_gates().0;
        let empty = BaseResultCache::for_base("1a2b3c4").unwrap();
        assert_eq!(
            decide_with_base(&gate, &docs_patch(), &empty),
            RunDecision::Run
        );

        let mut cached = BaseResultCache::for_base("1a2b3c4").unwrap();
        cached
            .base_result_for("validate", || BaseOutcome::Passed)
            .unwrap();
        assert_eq!(
            decide_with_base(&gate, &docs_patch(), &cached),
            RunDecision::ReuseBase
        );
    }

    /// A base result is computed at most once per gate per base revision,
    /// and a moved base is a new cache, not a stale hit.
    #[test]
    fn base_result_computed_once_per_revision() {
        let mut cache = BaseResultCache::for_base("1a2b3c4").unwrap();
        let mut runs = 0u64;
        cache
            .base_result_for("validate", || {
                runs += 1;
                BaseOutcome::Passed
            })
            .unwrap();
        cache
            .base_result_for("validate", || {
                runs += 1;
                BaseOutcome::Passed
            })
            .unwrap();
        cache
            .base_result_for("validate", || {
                runs += 1;
                BaseOutcome::Passed
            })
            .unwrap();
        assert_eq!(runs, 1);
        assert_eq!(cache.computed(), 1);

        let mut moved = BaseResultCache::for_base("5d6e7f8").unwrap();
        let mut runs_moved = 0u64;
        let compute_moved = || {
            runs_moved += 1;
            BaseOutcome::Passed
        };
        moved.base_result_for("validate", compute_moved).unwrap();
        assert_eq!(runs_moved, 1);
        assert!(cache.get("validate").is_some());
        assert!(moved.get("validate").is_some());
    }

    /// A skip is recorded as a skip, never as a pass.
    #[test]
    fn skipped_is_recorded_distinctly_from_passed() {
        let record = GateRecord::ReusedBase {
            base_revision: "1a2b3c4".to_string(),
            base: BaseOutcome::Passed,
        };
        assert!(!record.ran());
        assert!(!record.counts_as_passed_evidence());
        assert_eq!(record.outcome(), BaseOutcome::Passed);
        assert_eq!(record.line(), "skipped (reused base 1a2b3c4: passed)");

        let passed = GateRecord::Passed;
        assert!(passed.ran());
        assert!(passed.counts_as_passed_evidence());

        let report = GateReport::new([
            ("build".to_string(), GateRecord::Passed),
            ("validate".to_string(), record),
        ])
        .unwrap();
        let line = report.line();
        // The ran section precedes the skip section: a skip can never be
        // read as a pass.
        assert_eq!(
            line,
            "gates: build: passed | skipped (not run, base stands): \
             validate skipped (reused base 1a2b3c4: passed)"
        );
        let skipped_names: Vec<&str> = report.skipped().map(|(n, _)| n).collect();
        assert_eq!(skipped_names, vec!["validate"]);
        assert!(!report.is_hold());
    }

    /// A reused failure still holds: reusing the base does not clear a
    /// failure the base had.
    #[test]
    fn reused_failure_holds() {
        let report = GateReport::new([(
            "validate".to_string(),
            GateRecord::ReusedBase {
                base_revision: "1a2b3c4".to_string(),
                base: BaseOutcome::Failed,
            },
        )])
        .unwrap();
        assert!(report.is_hold());
        assert_eq!(
            report.gate("validate").unwrap().line(),
            "skipped (reused base 1a2b3c4: failed)"
        );
    }

    #[test]
    fn run_cost_reports_total_and_base_matched() {
        let gates = [
            GateCost::new("build", true, 180).unwrap(),
            GateCost::new("clippy", false, 60).unwrap(),
            GateCost::new("validate", true, 0).unwrap(), // reused: zero seconds
        ];
        let cost = RunCost::from_gates(gates.iter());
        assert_eq!(cost.total_seconds(), 240);
        assert_eq!(cost.base_matched_seconds(), 180);
        assert!((cost.base_matched_share().unwrap() - 0.75).abs() < f64::EPSILON);
        assert_eq!(
            cost.line(),
            "verification: 240s total, 180s on gates matching base (75%)"
        );

        let empty = RunCost::from_gates(std::iter::empty::<&GateCost>());
        assert_eq!(empty.total_seconds(), 0);
        assert_eq!(empty.base_matched_share(), None);
        assert_eq!(empty.line(), "verification: no gate time measured");
    }

    #[test]
    fn projected_drain_from_observed_costs() {
        // No observation: no projection, not a guess.
        assert_eq!(projected_drain(107, &[]), None);
        // Nothing left: the queue is drained.
        assert_eq!(projected_drain(0, &[132]), Some(0));
        // Odd count: the middle observation.
        assert_eq!(projected_drain(3, &[300, 100, 200]), Some(3 * 200));
        // Even count with outliers: the lower middle survives them.
        assert_eq!(projected_drain(10, &[132, 304, 741, 1189]), Some(10 * 304));
        // Overflow: no projection rather than a wrong one.
        assert_eq!(projected_drain(u64::MAX, &[u64::MAX]), None);
    }

    #[test]
    fn exact_file_and_directory_prefix_match() {
        let inputs = Inputs::reading(["Cargo.toml", "crates/"]);
        assert!(inputs.matches("Cargo.toml"));
        assert!(inputs.matches("crates/autospec-core/src/lib.rs"));
        assert!(!inputs.matches("Cargo.toml.orig"));
        assert!(!inputs.matches("crates-x/lib.rs"));
        assert!(inputs.touches(&["docs/a.md".to_string(), "crates/a.rs".to_string()]));
        assert!(!Inputs::default().touches(&["crates/a.rs".to_string()]));
    }

    #[test]
    fn fail_closed_constructors() {
        assert!(Gate::new("", [Check::opaque("c")]).is_err());
        assert!(Gate::new("validate", []).is_err());
        assert!(Gate::new(
            "validate",
            [
                Check::reading("a", Inputs::default()),
                Check::reading("a", Inputs::default()),
            ]
        )
        .is_err());
        assert!(BaseResultCache::for_base("").is_err());
        assert!(BaseResult::computed_on("", "g", BaseOutcome::Passed).is_err());
        assert!(BaseResult::computed_on("r", "", BaseOutcome::Passed).is_err());
        assert!(GateCost::new("", false, 1).is_err());
        assert!(GateReport::new([]).is_err());
        assert!(GateReport::new([
            ("a".to_string(), GateRecord::Passed),
            ("a".to_string(), GateRecord::Passed),
        ])
        .is_err());
    }

    #[test]
    fn serde_round_trip() {
        let gate = repo_gates().0;
        let round: Gate = serde_json::from_str(&serde_json::to_string(&gate).unwrap()).unwrap();
        assert_eq!(round, gate);

        let cache = {
            let mut cache = BaseResultCache::for_base("1a2b3c4").unwrap();
            cache
                .base_result_for("validate", || BaseOutcome::Passed)
                .unwrap();
            cache
        };
        let round: BaseResultCache =
            serde_json::from_str(&serde_json::to_string(&cache).unwrap()).unwrap();
        assert_eq!(round, cache);
        assert_eq!(round.computed(), 1);

        let record = GateRecord::ReusedBase {
            base_revision: "1a2b3c4".to_string(),
            base: BaseOutcome::Failed,
        };
        let round: GateRecord =
            serde_json::from_str(&serde_json::to_string(&record).unwrap()).unwrap();
        assert_eq!(round, record);

        let cost = RunCost::from_gates(std::iter::once(&GateCost::new("g", true, 5).unwrap()));
        let round: RunCost = serde_json::from_str(&serde_json::to_string(&cost).unwrap()).unwrap();
        assert_eq!(round, cost);
    }
}
