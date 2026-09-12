//! Per-patch gate scoping and per-gate timing for the conversion pass (#3738).
//!
//! The per-patch conversion loop runs a fixed set of gates — build, clippy,
//! test, validate. A gate whose cost is **independent of the change** sets
//! the pipeline's throughput: `validate` runs its 159 checks (plus its own
//! release build) and costs the same ~6 minutes whether the patch touches
//! one line or a thousand, while most patches touch only `crates/` and the
//! checks inspect scripts, docs, skills, contracts and generated artifacts
//! the patch cannot affect.
//!
//! The rule: **scope every gate to the blast radius of the change, or
//! accept that its cost sets the pipeline's throughput.** The test gate
//! already did this — it runs `-p <crate>` for the crates the patch
//! touches, not `--workspace` — but the principle lived there as an
//! implementation detail, not a stated rule, and it had not been carried
//! to `validate`.
//!
//! So every gate in the loop now carries a stated scoping rule
//! ([`Gate::scoping_rule`]), and the plan for a patch ([`plan_gates`])
//! records a reason for every skip: an unexplained missing gate is
//! indistinguishable from an oversight, so a skip without a reason is
//! refused here and in [`GateTiming::skipped`]. The pass reports per-gate
//! wall-clock ([`GateReport`]) so a newly added expensive gate is visible
//! in the report, not inferred from the total runtime.
//!
//! Generalised: every gate added to a per-item loop should declare at
//! review time what makes its cost proportional to the item. If nothing
//! does, it belongs in a batch pass over the merged result, not in the
//! loop.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

/// The gates of the per-patch conversion loop, in execution order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Gate {
    /// `cargo build` for the crates the patch touched.
    Build,
    /// `cargo clippy` for the crates the patch touched.
    Clippy,
    /// `cargo test -p <crate>` for the crates the patch touched.
    Test,
    /// `autospec-cli validate`: the full check set over the repository.
    Validate,
}

impl Gate {
    /// Every gate in the loop, in execution order.
    pub const ALL: [Self; 4] = [Self::Build, Self::Clippy, Self::Test, Self::Validate];

    /// Stable machine name for reports and logs.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Build => "build",
            Self::Clippy => "clippy",
            Self::Test => "test",
            Self::Validate => "validate",
        }
    }

    /// The stated scoping rule for this gate: what makes its cost
    /// proportional to the patch. A gate with no such property belongs in
    /// a batch pass over the merged result, not in the per-patch loop.
    pub fn scoping_rule(self) -> &'static str {
        match self {
            Self::Build => {
                "runs for the crates the patch touched; skipped when the patch \
                 touches no crate"
            }
            Self::Clippy => {
                "runs for the crates the patch touched; skipped when the patch \
                 touches no crate"
            }
            Self::Test => {
                "runs `-p <crate>` for the crates the patch touched, never \
                 `--workspace`; skipped when the patch touches no crate"
            }
            Self::Validate => {
                "runs the full check set only when the patch touches files \
                 outside crates/; a crate-only change is covered by \
                 build+clippy+test"
            }
        }
    }

    /// The noun used in this gate's recorded skip reason
    /// ("nothing to build" / "nothing to lint" / "nothing to test").
    fn skip_noun(self) -> &'static str {
        match self {
            Self::Build => "build",
            Self::Clippy => "lint",
            Self::Test => "test",
            Self::Validate => "validate",
        }
    }
}

/// What a gate covers when it runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateScope {
    /// The whole repository: a fixed cost, independent of the patch.
    WholeRepo,
    /// Exactly the crates the patch touched.
    Crates(Vec<String>),
}

impl GateScope {
    /// `repo`, or `crates[<name>,<name>…]` in sorted order.
    pub fn render(&self) -> String {
        match self {
            Self::WholeRepo => "repo".to_string(),
            Self::Crates(crates) => format!("crates[{}]", crates.join(",")),
        }
    }
}

/// The decision for one gate on one patch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateDecision {
    /// The gate runs against this scope.
    Run(GateScope),
    /// The gate is deliberately skipped. The reason is recorded in the PR
    /// body: an unexplained missing gate is indistinguishable from an
    /// oversight, so the reason is mandatory.
    Skip { reason: String },
}

/// The plan for one gate on one patch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatePlan {
    /// The gate.
    pub gate: Gate,
    /// Run against a scope, or skip with a recorded reason.
    pub decision: GateDecision,
}

impl GatePlan {
    /// The stated scoping rule for the gate this plan covers.
    pub fn scoping_rule(&self) -> &'static str {
        self.gate.scoping_rule()
    }
}

/// Normalise a path for scoping: forward slashes, no surrounding blanks.
fn normalize(path: &str) -> String {
    path.trim().replace('\\', "/")
}

/// True when `path` lies inside the repository's `crates/` directory,
/// i.e. names a crate: `crates/<name>` or `crates/<name>/…`.
fn inside_crates(path: &str) -> bool {
    let mut components = path.split('/');
    components.next() == Some("crates") && components.next().is_some()
}

/// The crates a patch touches: the `crates/<name>` component of every path
/// under `crates/`, deduplicated and sorted.
pub fn touched_crates(paths: impl IntoIterator<Item = impl AsRef<str>>) -> Vec<String> {
    let mut crates = BTreeSet::new();
    for path in paths {
        let path = normalize(path.as_ref());
        let mut components = path.split('/');
        let name = if components.next() == Some("crates") {
            components.next().filter(|name| !name.is_empty())
        } else {
            None
        };
        if let Some(name) = name {
            crates.insert(name.to_string());
        }
    }
    crates.into_iter().collect()
}

/// Plan the per-patch gates for a patch that touches `paths`.
///
/// Every gate gets a stated decision: run against its blast-radius scope,
/// or skip with a reason that lands in the PR body. The decisions encode
/// the scoping rule of each gate:
///
/// - build, clippy, test — scoped to the crates the patch touched; a patch
///   that touches no crate cannot affect the Rust build or tests, so the
///   gate is skipped with a recorded reason rather than run workspace-wide
///   at a fixed cost.
/// - validate — the full check set, whose cost is independent of the
///   change; it runs only when the patch touches files outside `crates/`,
///   which is where its checks actually apply. A crate-only change is
///   covered by build + clippy + test.
pub fn plan_gates(paths: impl IntoIterator<Item = impl AsRef<str>>) -> Vec<GatePlan> {
    let paths: Vec<String> = paths
        .into_iter()
        .map(|path| normalize(path.as_ref()))
        .filter(|path| !path.is_empty())
        .collect();
    let crates = touched_crates(&paths);
    let touches_outside = paths.iter().any(|path| !inside_crates(path));
    Gate::ALL
        .iter()
        .map(|&gate| GatePlan {
            gate,
            decision: gate_decision(gate, &crates, touches_outside, !paths.is_empty()),
        })
        .collect()
}

/// The stated decision for one gate, given the patch's touched crates and
/// whether the patch touches anything outside `crates/`.
fn gate_decision(
    gate: Gate,
    crates: &[String],
    touches_outside: bool,
    touches_anything: bool,
) -> GateDecision {
    if !touches_anything {
        return GateDecision::Skip {
            reason: "patch touches no files; nothing to gate".to_string(),
        };
    }
    if gate == Gate::Validate {
        return if touches_outside {
            GateDecision::Run(GateScope::WholeRepo)
        } else {
            GateDecision::Skip {
                reason: "patch touches only crates/; covered by build+clippy+test".to_string(),
            }
        };
    }
    if crates.is_empty() {
        return GateDecision::Skip {
            reason: format!("patch touches no crate; nothing to {}", gate.skip_noun()),
        };
    }
    GateDecision::Run(GateScope::Crates(crates.to_vec()))
}

/// A gate report cannot accept this measurement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateTimingError {
    /// A duration that is not finite or is negative.
    InvalidDuration,
    /// A skipped gate without a recorded reason.
    MissingSkipReason,
    /// A gate the plan said would run has no measured duration: a missing
    /// gate with no explanation is indistinguishable from an oversight.
    MissingGate { gate: Gate },
    /// A gate was measured more than once.
    DuplicateGate { gate: Gate },
}

/// The observed cost of one gate for one patch.
#[derive(Debug, Clone, PartialEq)]
pub struct GateTiming {
    /// The gate.
    pub gate: Gate,
    /// The scope it ran against; `None` when the gate was skipped.
    pub scope: Option<GateScope>,
    /// The recorded reason when the gate was skipped; `None` when it ran.
    pub skip_reason: Option<String>,
    /// Wall-clock seconds the gate consumed (0.0 when skipped).
    pub seconds: f64,
}

impl GateTiming {
    /// A gate that ran against `scope` and consumed `seconds`.
    pub fn ran(gate: Gate, scope: GateScope, seconds: f64) -> Result<Self, GateTimingError> {
        check_duration(seconds)?;
        Ok(Self {
            gate,
            scope: Some(scope),
            skip_reason: None,
            seconds,
        })
    }

    /// A gate that was deliberately skipped. The reason is mandatory and
    /// non-blank: it is what the PR body records in place of a missing
    /// gate.
    pub fn skipped(gate: Gate, reason: impl Into<String>) -> Result<Self, GateTimingError> {
        let reason = reason.into();
        if reason.trim().is_empty() {
            return Err(GateTimingError::MissingSkipReason);
        }
        Ok(Self {
            gate,
            scope: None,
            skip_reason: Some(reason),
            seconds: 0.0,
        })
    }

    /// The timing row for `plan`, given the measured seconds of a gate that
    /// ran. Skipped gates always cost 0.0; the measured value is ignored.
    pub fn from_plan(plan: &GatePlan, seconds: f64) -> Result<Self, GateTimingError> {
        match &plan.decision {
            GateDecision::Run(scope) => Self::ran(plan.gate, scope.clone(), seconds),
            GateDecision::Skip { reason } => Self::skipped(plan.gate, reason.clone()),
        }
    }
}

/// The per-gate timing report for one patch.
///
/// One row per gate, in loop order, each with the wall-clock seconds it
/// consumed: a newly added expensive gate shows up as a line in this
/// report, not as an unexplained increase in the total runtime.
#[derive(Debug, Clone, PartialEq)]
pub struct GateReport {
    rows: Vec<GateTiming>,
}

impl GateReport {
    /// Assemble a report from timing rows, keeping the given order.
    pub fn new(rows: Vec<GateTiming>) -> Self {
        Self { rows }
    }

    /// Build the report for a plan: one row per gate in the plan, with the
    /// measured seconds for the gates that ran. Every gate the plan said
    /// would run must appear in `measured` — a missing row is a missing
    /// gate, and this constructor refuses it.
    pub fn from_plan(
        plan: &[GatePlan],
        measured: impl IntoIterator<Item = (Gate, f64)>,
    ) -> Result<Self, GateTimingError> {
        let mut durations = BTreeMap::new();
        for (gate, seconds) in measured {
            if durations.insert(gate, seconds).is_some() {
                return Err(GateTimingError::DuplicateGate { gate });
            }
        }
        let mut rows = Vec::with_capacity(plan.len());
        for entry in plan {
            let seconds = duration_for(entry, &durations)?;
            rows.push(GateTiming::from_plan(entry, seconds)?);
        }
        Ok(Self { rows })
    }

    /// The rows, in report order.
    pub fn rows(&self) -> &[GateTiming] {
        &self.rows
    }

    /// Whether the report has no rows at all.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Total wall-clock seconds across all rows.
    pub fn total_seconds(&self) -> f64 {
        self.rows.iter().map(|row| row.seconds).sum()
    }

    /// One line per gate: name, status, scope or recorded skip reason, and
    /// the seconds it consumed.
    pub fn render(&self) -> String {
        self.rows
            .iter()
            .map(render_row)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// The wall-clock seconds a planned gate consumed: the measured value when
/// the plan ran it, 0.0 when the plan skipped it. A gate the plan ran but
/// nobody measured is refused: a missing gate with no explanation is
/// indistinguishable from an oversight.
fn duration_for(entry: &GatePlan, durations: &BTreeMap<Gate, f64>) -> Result<f64, GateTimingError> {
    match &entry.decision {
        GateDecision::Skip { .. } => Ok(0.0),
        GateDecision::Run(_) => durations
            .get(&entry.gate)
            .copied()
            .ok_or(GateTimingError::MissingGate { gate: entry.gate }),
    }
}

/// One report line for one gate: name, status, scope or recorded skip
/// reason, and the seconds it consumed.
fn render_row(row: &GateTiming) -> String {
    match &row.scope {
        Some(scope) => format!(
            "gate={} status=ran scope={} seconds={:.1}",
            row.gate.as_str(),
            scope.render(),
            row.seconds
        ),
        None => format!(
            "gate={} status=skipped reason={} seconds={:.1}",
            row.gate.as_str(),
            row.skip_reason.as_deref().unwrap_or_default(),
            row.seconds
        ),
    }
}

/// Refuse a duration the report cannot stand on.
fn check_duration(seconds: f64) -> Result<(), GateTimingError> {
    if seconds.is_finite() && seconds >= 0.0 {
        Ok(())
    } else {
        Err(GateTimingError::InvalidDuration)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decision_of<'a>(plan: &'a [GatePlan], gate: Gate) -> &'a GateDecision {
        let entry = plan
            .iter()
            .find(|entry| entry.gate == gate)
            .expect("every gate in the loop gets a plan entry");
        &entry.decision
    }

    #[test]
    fn touched_crates_extracts_names_deduplicated_and_sorted() {
        let crates = touched_crates([
            "crates/autospec-core/src/lib.rs",
            "crates/autospec-cli/src/main.rs",
            "crates/autospec-core/Cargo.toml",
            "docs/specs/something.md",
        ]);
        assert_eq!(crates, vec!["autospec-cli", "autospec-core"]);
    }

    #[test]
    fn touched_crates_ignores_paths_outside_crates() {
        assert_eq!(
            touched_crates([
                "scripts/lint.sh",
                "docs/a.md",
                "skills/autospec-run/SKILL.md"
            ]),
            Vec::<String>::new()
        );
    }

    #[test]
    fn every_gate_has_a_stated_scoping_rule() {
        // Acceptance: each gate in the per-patch loop has a stated
        // scoping rule, or a stated reason none is needed.
        for gate in Gate::ALL {
            assert!(
                !gate.scoping_rule().trim().is_empty(),
                "{gate:?} has no stated scoping rule"
            );
        }
    }

    #[test]
    fn crate_only_patch_scopes_crate_gates_and_skips_validate() {
        let plan = plan_gates(["crates/autospec-core/src/foo.rs"]);
        for gate in [Gate::Build, Gate::Clippy, Gate::Test] {
            assert_eq!(
                decision_of(&plan, gate),
                &GateDecision::Run(GateScope::Crates(vec!["autospec-core".to_string()])),
            );
        }
        assert_eq!(
            decision_of(&plan, Gate::Validate),
            &GateDecision::Skip {
                reason: "patch touches only crates/; covered by build+clippy+test".to_string(),
            },
        );
    }

    #[test]
    fn validate_runs_only_when_the_patch_touches_files_outside_crates() {
        let plan = plan_gates(["crates/autospec-core/src/foo.rs", "scripts/lint.sh"]);
        assert_eq!(
            decision_of(&plan, Gate::Validate),
            &GateDecision::Run(GateScope::WholeRepo),
        );
        assert_eq!(
            decision_of(&plan, Gate::Test),
            &GateDecision::Run(GateScope::Crates(vec!["autospec-core".to_string()])),
        );
    }

    #[test]
    fn docs_only_patch_skips_crate_gates_with_recorded_reasons_and_runs_validate() {
        let plan = plan_gates(["docs/specs/something.md"]);
        assert_eq!(
            decision_of(&plan, Gate::Build),
            &GateDecision::Skip {
                reason: "patch touches no crate; nothing to build".to_string(),
            },
        );
        assert_eq!(
            decision_of(&plan, Gate::Clippy),
            &GateDecision::Skip {
                reason: "patch touches no crate; nothing to lint".to_string(),
            },
        );
        assert_eq!(
            decision_of(&plan, Gate::Test),
            &GateDecision::Skip {
                reason: "patch touches no crate; nothing to test".to_string(),
            },
        );
        assert_eq!(
            decision_of(&plan, Gate::Validate),
            &GateDecision::Run(GateScope::WholeRepo),
        );
    }

    #[test]
    fn empty_patch_skips_every_gate_with_a_recorded_reason() {
        let plan = plan_gates(Vec::<String>::new());
        assert_eq!(plan.len(), Gate::ALL.len());
        let expected = GateDecision::Skip {
            reason: "patch touches no files; nothing to gate".to_string(),
        };
        for entry in &plan {
            assert_eq!(entry.decision, expected.clone());
        }
    }

    #[test]
    fn every_skip_carries_a_non_blank_reason() {
        // A skip without a reason is unrepresentable in the plan: the
        // decision type requires one, and every path through the planner
        // fills it.
        let plans: Vec<GatePlan> = [
            vec!["crates/autospec-core/src/foo.rs".to_string()],
            vec!["docs/a.md".to_string()],
            Vec::<String>::new(),
        ]
        .into_iter()
        .map(|paths| plan_gates(&paths))
        .flatten()
        .collect();
        for entry in plans {
            if let GateDecision::Skip { reason } = entry.decision {
                assert!(!reason.trim().is_empty());
            }
        }
    }

    #[test]
    fn gate_timing_ran_refuses_bad_durations() {
        let scope = GateScope::Crates(vec!["autospec-core".to_string()]);
        assert_eq!(
            GateTiming::ran(Gate::Test, scope.clone(), -1.0),
            Err(GateTimingError::InvalidDuration),
        );
        assert_eq!(
            GateTiming::ran(Gate::Test, scope.clone(), f64::NAN),
            Err(GateTimingError::InvalidDuration),
        );
        assert_eq!(
            GateTiming::ran(Gate::Test, scope, f64::INFINITY),
            Err(GateTimingError::InvalidDuration),
        );
        assert!(GateTiming::ran(Gate::Test, GateScope::WholeRepo, 0.0).is_ok());
    }

    #[test]
    fn gate_timing_skipped_requires_a_recorded_reason() {
        assert_eq!(
            GateTiming::skipped(Gate::Validate, ""),
            Err(GateTimingError::MissingSkipReason),
        );
        assert_eq!(
            GateTiming::skipped(Gate::Validate, "   "),
            Err(GateTimingError::MissingSkipReason),
        );
        let row = GateTiming::skipped(Gate::Validate, "patch touches only crates/").unwrap();
        assert_eq!(row.scope, None);
        assert_eq!(row.seconds, 0.0);
    }

    #[test]
    fn report_from_plan_renders_one_line_per_gate_with_its_seconds() {
        let plan = plan_gates(["crates/autospec-core/src/foo.rs"]);
        let report = GateReport::from_plan(
            &plan,
            [
                (Gate::Build, 45.2),
                (Gate::Clippy, 30.1),
                (Gate::Test, 124.7),
            ],
        )
        .unwrap();
        assert_eq!(report.rows().len(), Gate::ALL.len());
        let rendered = report.render();
        let lines: Vec<&str> = rendered.lines().collect();
        assert_eq!(
            lines,
            vec![
                "gate=build status=ran scope=crates[autospec-core] seconds=45.2",
                "gate=clippy status=ran scope=crates[autospec-core] seconds=30.1",
                "gate=test status=ran scope=crates[autospec-core] seconds=124.7",
                "gate=validate status=skipped reason=patch touches only crates/; \
                 covered by build+clippy+test seconds=0.0",
            ],
        );
        assert!((report.total_seconds() - 200.0).abs() < 1e-9);
    }

    #[test]
    fn report_refuses_a_planned_gate_without_a_measured_duration() {
        let plan = plan_gates(["crates/autospec-core/src/foo.rs"]);
        let err = GateReport::from_plan(&plan, [(Gate::Build, 45.0), (Gate::Clippy, 30.0)])
            .expect_err("test was planned to run but was never measured");
        assert_eq!(err, GateTimingError::MissingGate { gate: Gate::Test });
    }

    #[test]
    fn report_refuses_a_gate_measured_twice() {
        let plan = plan_gates(["crates/autospec-core/src/foo.rs"]);
        let err = GateReport::from_plan(
            &plan,
            [
                (Gate::Build, 45.0),
                (Gate::Build, 46.0),
                (Gate::Clippy, 30.0),
                (Gate::Test, 100.0),
            ],
        )
        .expect_err("build was measured twice");
        assert_eq!(err, GateTimingError::DuplicateGate { gate: Gate::Build });
    }

    #[test]
    fn report_makes_an_expensive_gate_visible_as_its_own_line() {
        // Acceptance: the pass reports per-gate timing, so a newly added
        // expensive gate is visible immediately rather than inferred from
        // the total runtime.
        let plan = plan_gates(["scripts/lint.sh"]);
        let report = GateReport::from_plan(&plan, [(Gate::Validate, 360.0)]).unwrap();
        let rendered = report.render();
        assert!(
            rendered
                .lines()
                .any(|line| line == "gate=validate status=ran scope=repo seconds=360.0"),
            "the expensive gate must be visible as its own line: {rendered}"
        );
    }
}
