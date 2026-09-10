//! Grading runs the same gate set as landing (#3925).
//!
//! The failure this module exists to close was invisible in the logs: a
//! patch graded as passing that the conversion pass then rejected, because
//! grading had run a *weaker* gate set than the one that decides whether
//! work lands. Clippy without `-D warnings` exits 0 with lints present, so
//! a gate that was merely *run* was not a gate that was *enforced* — and a
//! status line that tallied it as clean reported success for a gate that
//! never had a chance to fail the patch.
//!
//! Three primitives close the gap:
//!
//! 1. **The gate set is data** ([`Gate`], [`GateSet`],
//!    [`GateSet::rust_workspace`]). The commands that decide whether a
//!    patch lands are named in one place, so grading and landing cannot
//!    drift apart. A clippy invocation without `-D warnings` is a
//!    different gate, not a cheaper one.
//! 2. **A grade is a verdict, not a tally** ([`grade`], [`GradeVerdict`]).
//!    `Pass` means every gate in the set ran its specified command and
//!    found nothing. A gate that ran a different command — weaker,
//!    stronger, reordered — is [`FindingKind::Unenforced`]; a gate with no
//!    run at all is [`FindingKind::Skipped`]. Both fail the grade, and
//!    [`GradeVerdict::line`] names each finding, because a status line
//!    that reports success for a gate that was not enforced is the bug.
//! 3. **The staged spec names the gates** ([`GATES_SECTION`],
//!    [`gate_section`], [`staged_gates`]). When the caller knows the gate
//!    set, the spec a worker reads carries it as acceptance criteria, and
//!    reads it back fail-closed: a spec whose gate section is malformed
//!    parses to `None`, not to a partial set.
//!
//! Everything here is pure — no I/O, no clock, no subprocess. The caller
//! runs the commands and records the runs; the grade is computed from the
//! record.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// One gate: the name the grade reports it by, and the exact command that
/// enforces it.
///
/// Command identity is part of the gate's identity: the same tool invoked
/// with a different strictness flag is a different gate, and grading
/// against the weaker one is the drift this module exists to make
/// ungradeable as clean.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gate {
    pub name: String,
    pub command: String,
}

impl Gate {
    pub fn new(name: impl Into<String>, command: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            command: command.into(),
        }
    }
}

/// The full, ordered set of gates whose passing decides whether a patch
/// lands.
///
/// Order matters: it is the order findings appear in, so a grade names the
/// gates in the order the set declares them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateSet {
    gates: Vec<Gate>,
}

impl GateSet {
    /// A new gate set. Rejected: an empty set (grading against nothing
    /// passes everything), an empty name or command (a gate nobody can
    /// name or run cannot be enforced), and duplicate names (the second
    /// run would be ungradeable).
    pub fn new(gates: Vec<Gate>) -> Result<Self, String> {
        if gates.is_empty() {
            return Err(
                "a gate set is non-empty: grading against nothing passes everything".to_string(),
            );
        }
        let mut seen = BTreeSet::new();
        for gate in &gates {
            if gate.name.trim().is_empty() {
                return Err(
                    "a gate has an empty name: every gate must be nameable so a grade can \
                     name what it skipped"
                        .to_string(),
                );
            }
            if gate.command.trim().is_empty() {
                return Err(format!(
                    "gate {:?} has an empty command: a gate that cannot run cannot be enforced",
                    gate.name
                ));
            }
            if !seen.insert(gate.name.as_str()) {
                return Err(format!(
                    "gate name {:?} appears twice in the gate set: the second run would not be \
                     gradeable",
                    gate.name
                ));
            }
        }
        Ok(Self { gates })
    }

    /// The standard Rust workspace gate set, in landing order: build,
    /// test, clippy, fmt.
    ///
    /// The clippy command is the *strong* form — `-D warnings` — because
    /// the weak form exits 0 with lints present and is therefore a
    /// different gate, not a cheaper one.
    pub fn rust_workspace() -> Self {
        Self::new(vec![
            Gate::new("build", "cargo build --workspace"),
            Gate::new("test", "cargo test --workspace"),
            Gate::new(
                "clippy",
                "cargo clippy --workspace --all-targets -- -D warnings",
            ),
            Gate::new("fmt", "cargo fmt --check"),
        ])
        .expect("the standard gate set is well-formed")
    }

    /// The gates, in the order the set declares them.
    pub fn gates(&self) -> &[Gate] {
        &self.gates
    }

    /// The gate with the given name, if the set has one.
    pub fn gate(&self, name: &str) -> Option<&Gate> {
        self.gates.iter().find(|gate| gate.name == name)
    }

    /// The specified commands, in order: what a run must execute to count
    /// as enforcing each gate.
    pub fn commands(&self) -> Vec<String> {
        self.gates.iter().map(|gate| gate.command.clone()).collect()
    }
}

/// What one gate run found, given that it ran the specified command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateResult {
    /// The gate found nothing.
    Clean,
    /// The gate found something, named so the grade can name it.
    Failing { detail: String },
}

/// One gate run, recorded with the command that was actually executed.
///
/// The command is recorded rather than assumed, because the comparison
/// against the gate set's specified command is exactly what separates
/// "the gate ran" from "the gate was enforced". The load is recorded
/// rather than described (issue #3963), because "the thing I started is
/// done" is not "nothing is running": a long-lived background job in the
/// session is invisible to that inference, and a clean result produced
/// under contention is different evidence from one produced alone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateRun {
    pub command: String,
    pub result: GateResult,
    pub load: MachineLoad,
}

/// The machine's concurrent load, observed alongside a gate run (issue
/// #3963).
///
/// "No other test process running" is a check, not a description, and the
/// observation belongs in the result alongside the numbers.
///
/// `exclusive` is a lock held for the run's duration — a guarantee, not a
/// sample. `competing` is the number of competing test processes observed
/// while the run did not hold a lock; an observed zero is a snapshot taken
/// once, and says nothing about the processes that appeared after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct MachineLoad {
    /// Competing test processes observed while the gate ran.
    pub competing: u64,
    /// The run held an exclusivity lock for its full duration.
    pub exclusive: bool,
}

/// Why a gate did not count as passing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingKind {
    /// The gate ran its specified command and found a defect.
    Defect,
    /// A run was recorded, but under a command the gate set did not
    /// specify. Its result cannot be trusted — a clippy without
    /// `-D warnings` exits 0 with lints present — and the patch cannot
    /// be graded on its word.
    Unenforced,
    /// No run was recorded at all: the gate never had a chance to fail
    /// the patch.
    Skipped,
}

/// One finding in a failing grade.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GradeFinding {
    /// The gate this is about, by the gate set's name.
    pub gate: String,
    pub kind: FindingKind,
    /// What was found. For [`FindingKind::Unenforced`] this is the
    /// expected and recorded commands; for [`FindingKind::Defect`] the
    /// gate's own finding; empty for [`FindingKind::Skipped`].
    pub detail: String,
}

/// The machine's load across a passing grade, so the one line a status
/// log carries distinguishes the kind of evidence, not only its size
/// (issue #3963).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PassLoad {
    /// Every run held an exclusivity lock for its full duration.
    Exclusive,
    /// No run held a lock; `competing` is the highest competing-process
    /// count observed across the runs. Zero observed is a snapshot, not
    /// idleness.
    Observed { competing: u64 },
}

impl PassLoad {
    /// The load suffix of a pass line: how the machine's load qualifies
    /// the "found nothing".
    fn suffix(&self) -> String {
        match self {
            Self::Exclusive => " (exclusive: lock held for the full run)".to_string(),
            Self::Observed { competing: 0 } => {
                " (not exclusive: 0 competing process(es) observed)".to_string()
            }
            Self::Observed { competing } => {
                format!(" (contended: {competing} competing process(es) observed, no lock held)")
            }
        }
    }
}

/// The grade of a patch's gate runs against a gate set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GradeVerdict {
    /// Every gate in the set ran its specified command and found
    /// nothing. `gates` is how many, so the line can say how much
    /// passing it covers; `load` is what the machine was doing while
    /// the gates ran, so a clean result produced under contention is
    /// reported as different evidence from one produced alone.
    Pass { gates: usize, load: PassLoad },
    /// At least one gate failed the grade; every finding is named, in
    /// gate-set order.
    Fail { findings: Vec<GradeFinding> },
}

impl GradeVerdict {
    pub fn is_pass(&self) -> bool {
        matches!(self, Self::Pass { .. })
    }

    /// The one line for a status log.
    ///
    /// A pass names how many gates it covers. A fail names every finding
    /// in gate-set order, and each finding names the gate:
    /// `not run: clippy` for a skip, `unenforced: clippy (expected \`…\`,
    /// ran \`…\`)` for a command mismatch, `clippy: <detail>` for a
    /// defect. A grade that fails without saying which gate it is about
    /// cannot tell the operator what to re-run.
    pub fn line(&self) -> String {
        match self {
            Self::Pass { gates, load } => {
                let base = format!(
                    "GRADE PASS: {gates} of {gates} gates ran as specified and found nothing"
                );
                format!("{base}{}", load.suffix())
            }
            Self::Fail { findings } => {
                let segments: Vec<String> = findings
                    .iter()
                    .map(|finding| match finding.kind {
                        FindingKind::Defect => format!("{}: {}", finding.gate, finding.detail),
                        FindingKind::Unenforced => {
                            format!("unenforced: {} ({})", finding.gate, finding.detail)
                        }
                        FindingKind::Skipped => format!("not run: {}", finding.gate),
                    })
                    .collect();
                format!("GRADE FAIL: {}", segments.join("; "))
            }
        }
    }
}

/// Grade a patch's gate runs against a gate set.
///
/// The verdict is `Pass` only when every gate in the set ran its
/// specified command and found nothing. A run recorded under a different
/// command is [`FindingKind::Unenforced`] — the result is discarded, not
/// downgraded, because a weaker invocation may have exited clean while
/// the specified one would not have. A gate with no run is
/// [`FindingKind::Skipped`]. A run naming a gate the set does not
/// contain is a caller bug and an `Err`, because the grade would be
/// claiming coverage the set did not ask for.
pub fn grade(gate_set: &GateSet, runs: &BTreeMap<String, GateRun>) -> Result<GradeVerdict, String> {
    for name in runs.keys() {
        if gate_set.gate(name).is_none() {
            return Err(format!(
                "gate run for unknown gate {name:?}: the gate set names {}",
                gate_set
                    .gates()
                    .iter()
                    .map(|gate| gate.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    let mut findings = Vec::new();
    for gate in gate_set.gates() {
        match runs.get(&gate.name) {
            None => findings.push(GradeFinding {
                gate: gate.name.clone(),
                kind: FindingKind::Skipped,
                detail: String::new(),
            }),
            Some(run) if run.command != gate.command => findings.push(GradeFinding {
                gate: gate.name.clone(),
                kind: FindingKind::Unenforced,
                detail: format!("expected `{}`, ran `{}`", gate.command, run.command),
            }),
            Some(run) => match &run.result {
                GateResult::Clean => {}
                GateResult::Failing { detail } => findings.push(GradeFinding {
                    gate: gate.name.clone(),
                    kind: FindingKind::Defect,
                    detail: detail.clone(),
                }),
            },
        }
    }
    if findings.is_empty() {
        let load = if runs.values().all(|run| run.load.exclusive) {
            PassLoad::Exclusive
        } else {
            let competing = runs
                .values()
                .map(|run| run.load.competing)
                .max()
                .unwrap_or(0);
            PassLoad::Observed { competing }
        };
        Ok(GradeVerdict::Pass {
            gates: gate_set.gates().len(),
            load,
        })
    } else {
        Ok(GradeVerdict::Fail { findings })
    }
}

/// The staged-spec section that carries the gate set as acceptance
/// criteria.
pub const GATES_SECTION: &str = "## Gate set (run before completion)";

/// Render the gate set as a staged-spec section: one unchecked checkbox
/// per gate, in gate-set order, so the spec a worker reads names the same
/// commands that decide whether the patch lands.
///
/// An empty gate set renders nothing — a spec that does not name its gates
/// carries no gate section, and [`staged_gates`] reports that absence.
pub fn gate_section(gates: &[Gate]) -> String {
    if gates.is_empty() {
        return String::new();
    }
    let mut out = String::from(GATES_SECTION);
    out.push('\n');
    for gate in gates {
        out.push_str("- [ ] ");
        out.push_str(&gate.command);
        out.push('\n');
    }
    out
}

/// The gate commands a staged spec carries, in order.
///
/// `None` when the spec carries no gate section, or the section is
/// malformed: a non-blank line inside the section that is neither a
/// checkbox nor the next section heading. A spec whose gate set cannot be
/// read back is not graded as if it had one, and a section with no
/// checkboxes carries no gates rather than an empty success.
///
/// A checked box (`- [x]`) is accepted: it is a worker's mark that the
/// gate ran, not a malformed spec.
pub fn staged_gates(text: &str) -> Option<Vec<String>> {
    let mut gates: Vec<String> = Vec::new();
    let mut in_section = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("## ") {
            if in_section {
                break; // the next section ends the gate set
            }
            in_section = trimmed == GATES_SECTION;
            continue;
        }
        if !in_section {
            continue;
        }
        if trimmed.is_empty() {
            continue;
        }
        let command = trimmed
            .strip_prefix("- [ ] ")
            .or_else(|| trimmed.strip_prefix("- [x] "))
            .or_else(|| trimmed.strip_prefix("- [X] "))?;
        let command = command.trim();
        if command.is_empty() {
            return None;
        }
        gates.push(command.to_string());
    }
    (!gates.is_empty()).then_some(gates)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runs(pairs: &[(&str, &str, GateResult)]) -> BTreeMap<String, GateRun> {
        runs_with_load(pairs, MachineLoad::default())
    }

    fn runs_with_load(
        pairs: &[(&str, &str, GateResult)],
        load: MachineLoad,
    ) -> BTreeMap<String, GateRun> {
        pairs
            .iter()
            .map(|(name, command, result)| {
                (
                    name.to_string(),
                    GateRun {
                        command: (*command).to_string(),
                        result: result.clone(),
                        load,
                    },
                )
            })
            .collect()
    }

    /// The gate set's runs, each under the given load.
    fn all_runs(
        set: &GateSet,
        result: &GateResult,
        load: MachineLoad,
    ) -> BTreeMap<String, GateRun> {
        set.gates()
            .iter()
            .map(|gate| {
                (
                    gate.name.clone(),
                    GateRun {
                        command: gate.command.clone(),
                        result: result.clone(),
                        load,
                    },
                )
            })
            .collect()
    }

    /// Acceptance criterion 1: a patch that passes every gate as specified
    /// is graded passing, and the line names how many gates the pass
    /// covers.
    #[test]
    fn a_patch_that_passes_every_gate_as_specified_is_graded_passing() {
        let set = GateSet::rust_workspace();
        let all = |result: GateResult| {
            set.gates()
                .iter()
                .map(|gate| (gate.name.as_str(), gate.command.as_str(), result.clone()))
                .collect::<Vec<_>>()
        };
        let verdict = grade(&set, &runs(&all(GateResult::Clean))).expect("grades");

        assert!(verdict.is_pass(), "{}", verdict.line());
        // Default load is the honest one: no lock held, a zero-competitor
        // snapshot. The line says that, not "clean and serial".
        assert_eq!(
            verdict.line(),
            "GRADE PASS: 4 of 4 gates ran as specified and found nothing \
             (not exclusive: 0 competing process(es) observed)"
        );
    }

    /// Acceptance criterion 4, the populated #3793 case: a gate started
    /// while another test binary is running must report contention, not a
    /// clean serial result. The result is still a pass — the gates ran and
    /// found nothing — but the line that a status log carries must name
    /// the competing process and the missing lock, so the two kinds of
    /// evidence are no longer reported identically.
    #[test]
    fn a_pass_observed_under_contention_reports_contention_not_a_clean_serial_result() {
        let set = GateSet::rust_workspace();
        let contended = runs_with_load(
            &set.gates()
                .iter()
                .map(|gate| (gate.name.as_str(), gate.command.as_str(), GateResult::Clean))
                .collect::<Vec<_>>(),
            MachineLoad {
                competing: 1,
                exclusive: false,
            },
        );

        let verdict = grade(&set, &contended).expect("grades");

        assert!(verdict.is_pass(), "contention is a label, not a failure");
        assert_eq!(
            verdict,
            GradeVerdict::Pass {
                gates: 4,
                load: PassLoad::Observed { competing: 1 },
            }
        );
        let line = verdict.line();
        assert!(
            line.contains("contended: 1 competing process(es) observed, no lock held"),
            "{}",
            line
        );
        assert_ne!(
            verdict.line(),
            GradeVerdict::Pass {
                gates: 4,
                load: PassLoad::Exclusive,
            }
            .line(),
            "a contended pass must not read the same as an exclusive one"
        );
    }

    /// A pass whose runs held the exclusivity lock is labeled exclusive:
    /// the claim was acquired, and the line says so.
    #[test]
    fn a_pass_under_a_held_lock_is_labeled_exclusive() {
        let set = GateSet::rust_workspace();
        let table = all_runs(
            &set,
            &GateResult::Clean,
            MachineLoad {
                competing: 0,
                exclusive: true,
            },
        );

        let verdict = grade(&set, &table).expect("grades");

        assert_eq!(
            verdict,
            GradeVerdict::Pass {
                gates: 4,
                load: PassLoad::Exclusive,
            }
        );
        assert_eq!(
            verdict.line(),
            "GRADE PASS: 4 of 4 gates ran as specified and found nothing \
             (exclusive: lock held for the full run)"
        );
    }

    /// The highest competing count across the runs is what the pass
    /// reports: one clean gate run observed beside the 21-hour loop is
    /// enough to make the grade contended, however clean the others were.
    #[test]
    fn the_pass_reports_the_highest_competing_count_across_runs() {
        let set = GateSet::rust_workspace();
        let quiet = MachineLoad::default();
        let table: BTreeMap<String, GateRun> = set
            .gates()
            .iter()
            .map(|gate| {
                let load = if gate.name == "test" {
                    MachineLoad {
                        competing: 9,
                        exclusive: false,
                    }
                } else {
                    quiet
                };
                (
                    gate.name.clone(),
                    GateRun {
                        command: gate.command.clone(),
                        result: GateResult::Clean,
                        load,
                    },
                )
            })
            .collect();

        let verdict = grade(&set, &table).expect("grades");

        assert_eq!(
            verdict,
            GradeVerdict::Pass {
                gates: 4,
                load: PassLoad::Observed { competing: 9 },
            }
        );
        assert!(
            verdict.line().contains("contended: 9 competing"),
            "{}",
            verdict.line()
        );
    }

    /// Acceptance criterion 2: a gate run under a weaker command does not
    /// report success. Clippy without `-D warnings` exits 0 with lints
    /// present; the grade must call that unenforced, not clean.
    #[test]
    fn a_gate_run_with_a_weaker_command_is_unenforced_not_clean() {
        let set = GateSet::rust_workspace();
        let weak_clippy = "cargo clippy --workspace";
        // The runs map is keyed, not ordered: the grade must follow the
        // gate set's order, not the map's.
        let table = runs(&[
            ("clippy", weak_clippy, GateResult::Clean),
            ("fmt", "cargo fmt --check", GateResult::Clean),
            ("build", "cargo build --workspace", GateResult::Clean),
            ("test", "cargo test --workspace", GateResult::Clean),
        ]);

        let verdict = grade(&set, &table).expect("grades");

        assert!(!verdict.is_pass(), "{}", verdict.line());
        let GradeVerdict::Fail { findings } = &verdict else {
            panic!("a non-pass grade carries its findings");
        };
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].gate, "clippy");
        assert_eq!(findings[0].kind, FindingKind::Unenforced);
        let line = verdict.line();
        assert!(line.starts_with("GRADE FAIL: "), "{line}");
        assert!(line.contains("unenforced: clippy"), "{line}");
        assert!(
            line.contains("expected `cargo clippy --workspace --all-targets -- -D warnings`"),
            "{line}"
        );
        assert!(line.contains("ran `cargo clippy --workspace`"), "{line}");
    }

    /// Acceptance criterion 4: a partial run fails and names exactly the
    /// gates that were not run.
    #[test]
    fn a_partial_run_names_the_gates_that_were_not_run() {
        let set = GateSet::rust_workspace();
        let table = runs(&[
            ("build", "cargo build --workspace", GateResult::Clean),
            ("test", "cargo test --workspace", GateResult::Clean),
        ]);

        let verdict = grade(&set, &table).expect("grades");

        assert!(!verdict.is_pass());
        let GradeVerdict::Fail { findings } = &verdict else {
            panic!("a non-pass grade carries its findings");
        };
        assert_eq!(
            findings
                .iter()
                .map(|finding| finding.gate.as_str())
                .collect::<Vec<_>>(),
            vec!["clippy", "fmt"],
            "findings appear in gate-set order"
        );
        assert!(findings
            .iter()
            .all(|finding| finding.kind == FindingKind::Skipped));
        let line = verdict.line();
        assert!(line.contains("not run: clippy"), "{line}");
        assert!(line.contains("not run: fmt"), "{line}");
        // The gates that did run are not named: they are not the problem.
        assert!(!line.contains("build"), "{line}");
        assert!(!line.contains("test"), "{line}");
    }

    /// Acceptance criterion 5: a patch with one clippy lint is graded
    /// failing, and the finding is the lint, not a command complaint.
    #[test]
    fn a_single_clippy_lint_is_graded_failing() {
        let set = GateSet::rust_workspace();
        let table = runs(&[
            ("build", "cargo build --workspace", GateResult::Clean),
            ("test", "cargo test --workspace", GateResult::Clean),
            (
                "clippy",
                "cargo clippy --workspace --all-targets -- -D warnings",
                GateResult::Failing {
                    detail: "clippy::useless_conversion: 1 warning emitted".to_string(),
                },
            ),
            ("fmt", "cargo fmt --check", GateResult::Clean),
        ]);

        let verdict = grade(&set, &table).expect("grades");

        assert!(!verdict.is_pass());
        let GradeVerdict::Fail { findings } = &verdict else {
            panic!("a non-pass grade carries its findings");
        };
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].gate, "clippy");
        assert_eq!(findings[0].kind, FindingKind::Defect);
        assert_eq!(
            verdict.line(),
            "GRADE FAIL: clippy: clippy::useless_conversion: 1 warning emitted"
        );
    }

    /// A run for a gate the set does not contain is a caller bug, not a
    /// grade: accepting it would claim coverage the set never asked for.
    #[test]
    fn a_run_for_a_gate_outside_the_set_is_a_caller_bug() {
        let set = GateSet::rust_workspace();
        let table = runs(&[
            ("build", "cargo build --workspace", GateResult::Clean),
            ("lint", "cargo clippy --workspace", GateResult::Clean),
        ]);

        let error = grade(&set, &table).expect_err("unknown gate name is an error");

        assert!(error.contains(r#"lint"#), "{error}");
        assert!(
            error.contains("build") && error.contains("clippy"),
            "{error}"
        );
    }

    /// The gate set validates itself: an empty set, an empty name or
    /// command, and a duplicate name are all rejected, because each one
    /// is a way for a grade to pass or report on nothing.
    #[test]
    fn a_malformed_gate_set_is_rejected() {
        let empty = GateSet::new(Vec::new()).expect_err("empty set");
        assert!(empty.contains("non-empty"), "{empty}");

        let unnamed =
            GateSet::new(vec![Gate::new("", "cargo test --workspace")]).expect_err("no name");
        assert!(unnamed.contains("empty name"), "{unnamed}");

        let commandless = GateSet::new(vec![Gate::new("test", "   ")]).expect_err("no command");
        assert!(commandless.contains("empty command"), "{commandless}");

        let duplicated = GateSet::new(vec![
            Gate::new("test", "cargo test --workspace"),
            Gate::new("test", "cargo test --workspace --release"),
        ])
        .expect_err("duplicate name");
        assert!(duplicated.contains("twice"), "{duplicated}");
    }

    /// Acceptance criterion 3: the gate commands appear in the staged
    /// spec and read back, in order, as the same commands.
    #[test]
    fn the_gate_section_renders_and_reads_back() {
        let set = GateSet::rust_workspace();
        let section = gate_section(set.gates());
        assert!(section.starts_with(GATES_SECTION), "{section}");
        for gate in set.gates() {
            assert!(
                section.contains(&format!("- [ ] {}", gate.command)),
                "{section}"
            );
        }

        // Embedded in a spec-shaped document, with a section after it:
        // the reader stops at the next heading, not at the end of the
        // document.
        let spec = format!(
            "# Issue #3925\n\n{HEADER}\n\n{section}\n## After\n\n- [ ] a later checkbox\n",
            HEADER = "## Issue body"
        );
        assert_eq!(
            staged_gates(&spec),
            Some(set.commands()),
            "the reader returns the gate set's commands, in order"
        );

        // An empty gate set renders nothing, and a spec with no section
        // reads back as absent, not as an empty success.
        assert_eq!(gate_section(&[]), "");
        let bare = "# Issue #1\n\n## Issue body\n\nDo the thing.\n";
        assert_eq!(staged_gates(bare), None);
    }

    /// The reader is fail-closed on its own section: a malformed line
    /// inside the gate section, an empty checkbox, and a section with no
    /// checkboxes all parse to `None`.
    #[test]
    fn a_malformed_gate_section_does_not_parse_to_a_partial_set() {
        let prologue = "# Issue #1\n\n";
        assert_eq!(
            staged_gates(&format!(
                "{prologue}{GATES_SECTION}\n\n- [ ] cargo build --workspace\nprose line\n"
            )),
            None,
            "a non-checkbox line inside the section is malformed"
        );
        assert_eq!(
            staged_gates(&format!(
                "{prologue}{GATES_SECTION}\n\n- [ ] \n- [ ] cargo test\n"
            )),
            None,
            "an empty checkbox is malformed"
        );
        assert_eq!(
            staged_gates(&format!(
                "{prologue}{GATES_SECTION}\n\nA gate set, someday.\n"
            )),
            None,
            "a section with no checkboxes carries no gates"
        );
        // A checked box is the worker's mark that the gate ran: it still
        // reads back as the command.
        assert_eq!(
            staged_gates(&format!(
                "{prologue}{GATES_SECTION}\n\n- [x] cargo build --workspace\n"
            )),
            Some(vec!["cargo build --workspace".to_string()])
        );
    }

    /// The command check is an equality check, not a strictness check: a
    /// gate run under a *stronger* command is unenforced too, because the
    /// grade can only attest to the command the set specified.
    #[test]
    fn a_stronger_command_is_unenforced_too() {
        let set = GateSet::rust_workspace();
        let stronger = "cargo clippy --workspace --all-targets --all-features -- -D warnings";
        let table = runs(&[
            ("build", "cargo build --workspace", GateResult::Clean),
            ("test", "cargo test --workspace", GateResult::Clean),
            ("clippy", stronger, GateResult::Clean),
            ("fmt", "cargo fmt --check", GateResult::Clean),
        ]);

        let verdict = grade(&set, &table).expect("grades");

        assert!(!verdict.is_pass());
        let line = verdict.line();
        assert!(line.contains("unenforced: clippy"), "{line}");
        assert!(line.contains(&format!("ran `{stronger}`")), "{line}");
    }

    /// The standard gate set names the strong clippy form, because the
    /// weak form is a different gate.
    #[test]
    fn the_standard_rust_gate_set_names_the_strong_clippy_form() {
        let set = GateSet::rust_workspace();
        assert_eq!(
            set.gates()
                .iter()
                .map(|gate| gate.name.as_str())
                .collect::<Vec<_>>(),
            vec!["build", "test", "clippy", "fmt"]
        );
        let clippy = set.gate("clippy").expect("named clippy");
        assert!(
            clippy.command.ends_with("-- -D warnings"),
            "{}",
            clippy.command
        );
        assert_eq!(
            set.commands(),
            vec![
                "cargo build --workspace",
                "cargo test --workspace",
                "cargo clippy --workspace --all-targets -- -D warnings",
                "cargo fmt --check",
            ]
        );
    }
}
