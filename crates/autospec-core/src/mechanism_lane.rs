//! The bounded fast lane for changes to the delivery mechanism (issue #3795).
//!
//! The pipeline is the slowest consumer of its own fixes. A change to the
//! dispatcher is scheduled as payload — crushed against the candidate queue,
//! gated by the full per-candidate gate it is waiting to change — so it waits
//! behind the work it would have sped up. The queue of work that fixes the
//! queue is served by the queue: a fixed point.
//!
//! A fixed point is stable, self-amplifying, and invisible from inside. The
//! scheduler does not distinguish "this change is served by the mechanism"
//! from "this change is served *as* the mechanism", so the class does not
//! appear in any queue report, and the system has no incentive to fix the
//! thing that determines how fast the system improves.
//!
//! The remedy is a declared lane with a bounded budget:
//!
//! 1. **The class is declared, never inferred from ordering.** A change is
//!    mechanism work when its paths touch a declared component of the delivery
//!    mechanism, or when its labels name it ([`MechanismSurface`],
//!    [`Classification::classify`]). The declaration is the input; the
//!    scheduler's order is an output.
//! 2. **The lane's capacity is stated policy, and it is bounded.**
//!    [`LanePolicy`] names the slots the lane may take from a batch and the
//!    slots it may never take (the payload reserve). The lane cannot become
//!    the whole queue, and the queue cannot starve because the lane exists:
//!    mechanism work pushed out of a full lane rejoins the normal queue in
//!    its original position rather than waiting for the next window
//!    ([`schedule`]).
//! 3. **A mechanism change is gated by the mechanism's own tests.**
//!    [`gate_set`] selects the fixture gate — the fixture commands each
//!    declared component names, budgeted in seconds — instead of the full
//!    per-candidate gate budgeted in tens of minutes. A mechanism change whose
//!    components name no fixture falls back to the full gate: a gate that
//!    cannot answer is not a cheap gate, it is no gate.
//! 4. **The improvement rate is a number, not a feeling.**
//!    [`ImprovementLedger`] records what actually landed and
//!    [`ImprovementLedger::improvement_rate`] reports mechanism changes per
//!    unit time against the reservation. A backlog with zero mechanism changes
//!    landed in the window is reported as [`ImprovementVerdict::FixedPoint`]
//!    — the fixed point is visible as a number rather than inferred from
//!    frustration.
//!
//! Everything here is pure: no I/O, no clock, no subprocesses. The caller
//! supplies `now`, reads the worklist, runs the gate the plan names, and
//! appends to the ledger when a change lands.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// The label that declares an issue to be delivery-mechanism work, for a
/// change whose paths a surface rule does not catch (a fix filed against the
/// monitor's prompt text, a skill body, a deployment unit).
///
/// The label is a declaration, not a guess: an issue carries it because a
/// human or the classifier said the change is the mechanism.
pub const MECHANISM_LABEL: &str = "delivery-mechanism";

/// How many slots of a batch the lane may take. The reservation exists so the
/// pipeline can fix itself within a window instead of waiting for spare
/// capacity that a full queue never leaves free.
pub const DEFAULT_LANE_CAPACITY: usize = 2;

/// How many slots of a batch the lane may **never** take. The lane is a
/// reservation inside the delivery system, not a takeover of it: while
/// mechanism work is waiting, payload still ships in the same batch.
pub const DEFAULT_PAYLOAD_RESERVE: usize = 1;

/// The window the reservation is stated over, and the window the improvement
/// rate is reported over: one hour of dispatch ticks.
pub const DEFAULT_WINDOW_SECS: u64 = 3_600;

/// The full per-candidate gate budget (#3782: ~15 minutes per candidate).
pub const FULL_GATE_BUDGET_SECS: u64 = 900;

/// What the mechanism's own fixture suite is expected to cost. It is a
/// budget, not a measurement: a fixture gate that runs past it is the fixture
/// suite growing into the full gate it replaced, and the caller should treat
/// the overrun as a defect in the fixture suite.
pub const FIXTURE_GATE_BUDGET_SECS: u64 = 120;

/// The commands that make up the full per-candidate gate.
pub const FULL_GATE_COMMANDS: &[&str] = &[
    "cargo build --workspace --all-targets",
    "cargo test --workspace --no-fail-fast",
];

// ── The declared mechanism surface ───────────────────────────────────────

/// One declared component of the delivery mechanism, with the paths that make
/// a change a change to it and the fixture tests that gate it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MechanismComponent {
    /// The component's declared name (`pipeline`, `dispatcher`, …).
    pub name: String,
    /// Repo-relative path prefixes. A path under one of them is a change to
    /// this component. Prefixes are compared on `/`-normalised paths, so a
    /// directory prefix owns its descendants and a file prefix owns only it.
    pub prefixes: Vec<String>,
    /// The component's own tests: the fixture commands that gate a change to
    /// it. A component that names none leaves the change on the full gate.
    pub fixture_commands: Vec<String>,
}

impl MechanismComponent {
    /// Whether `path` is a change to this component.
    pub fn claims(&self, path: &str) -> bool {
        let normalized = path.trim().replace('\\', "/");
        let normalized = normalized.trim_start_matches("./");
        self.prefixes.iter().any(|prefix| {
            let prefix = prefix.trim().replace('\\', "/");
            let prefix = prefix.trim_start_matches("./");
            !prefix.is_empty() && normalized.starts_with(prefix)
        })
    }
}

/// The declared set of mechanism components ([`MechanismSurface::repository`])
/// — the surface whose changes the lane is reserved for.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MechanismSurface {
    components: Vec<MechanismComponent>,
}

/// The pipeline: the queue, its frontier, and the code that turns an admitted
/// issue into scheduled work.
const COMPONENT_PIPELINE: &str = "pipeline";
/// The dispatcher: the CLI surface that hands scheduled work to an agent.
const COMPONENT_DISPATCHER: &str = "dispatcher";
/// The gate: what a candidate must pass before it is a PR.
const COMPONENT_GATE: &str = "gate";
/// The agent runner: the conductor and monitor that execute a dispatched run.
const COMPONENT_AGENT_RUNNER: &str = "agent-runner";

impl MechanismSurface {
    /// A surface from explicit components. Order is preserved for reporting.
    pub fn new(components: impl IntoIterator<Item = MechanismComponent>) -> Self {
        Self {
            components: components.into_iter().collect(),
        }
    }

    /// The surface this repository declares: the pipeline, the dispatcher,
    /// the gate, and the agent runner, each with its own fixture tests.
    ///
    /// This is the shipped policy, embedded so the tested value and the
    /// documented value cannot drift apart. `docs/cli-reference.md` and
    /// `docs/invariants.md` describe it; the tests here pin it.
    pub fn repository() -> Self {
        Self::new([
            MechanismComponent {
                name: COMPONENT_PIPELINE.to_string(),
                prefixes: [
                    "crates/autospec-core/src/dispatch_pipeline.rs",
                    "crates/autospec-core/src/mechanism_lane.rs",
                    "crates/autospec-core/src/coordination/",
                    "crates/autospec-core/src/execution/",
                    "crates/autospec-core/src/issue_lint.rs",
                ]
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
                fixture_commands: [
                    "cargo test -p autospec-core --test mechanism_lane",
                    "cargo test -p autospec-core --test dispatch_pipeline",
                    "cargo test -p autospec-core --test ready_queue",
                ]
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
            },
            MechanismComponent {
                name: COMPONENT_DISPATCHER.to_string(),
                prefixes: [
                    "crates/autospec-cli/src/commands/dispatch.rs",
                    "crates/autospec-cli/src/commands/queue.rs",
                    "crates/autospec-cli/src/commands/dispatch_spec.rs",
                ]
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
                fixture_commands: ["cargo test -p autospec-cli --test dispatch_commands"]
                    .iter()
                    .map(|s| (*s).to_string())
                    .collect(),
            },
            MechanismComponent {
                name: COMPONENT_GATE.to_string(),
                prefixes: [
                    "crates/autospec-core/src/validation/",
                    "crates/autospec-core/src/conversion_gate.rs",
                    "crates/autospec-core/src/grading.rs",
                    "config/gate-scope.yml",
                    "scripts/lint-implementation.sh",
                ]
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
                fixture_commands: [
                    "cargo test -p autospec-core --test validation_catalog",
                    "cargo test -p autospec-core --test implementation_lint",
                    "bats tests/unit/test_lint_implementation.bats",
                ]
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
            },
            MechanismComponent {
                name: COMPONENT_AGENT_RUNNER.to_string(),
                prefixes: [
                    "crates/autospec-cli/src/commands/autonomous/",
                    "crates/autospec-core/src/agent/",
                    "scripts/autospec-run-monitor.sh",
                ]
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
                fixture_commands: [
                    "cargo test -p autospec-cli --test autonomous_conductor_commands",
                ]
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
            },
        ])
    }

    /// The declared components, in declaration order.
    pub fn components(&self) -> &[MechanismComponent] {
        &self.components
    }

    /// The components that claim `path`, in declaration order.
    pub fn components_for(&self, path: &str) -> Vec<&MechanismComponent> {
        self.components
            .iter()
            .filter(|component| component.claims(path))
            .collect()
    }

    /// The fixture commands the given components name, deduplicated and sorted
    /// so two runs of the same classification emit the same gate. Components
    /// that name none contribute nothing — an empty command list is what makes
    /// [`gate_set`] fall back to the full gate rather than pass on no evidence.
    pub fn fixture_commands(&self, components: &[&str]) -> Vec<String> {
        let wanted: BTreeSet<&str> = components.iter().map(|s| s.as_ref()).collect();
        let mut commands: BTreeSet<String> = BTreeSet::new();
        for component in &self.components {
            if wanted.contains(component.name.as_str()) {
                commands.extend(component.fixture_commands.iter().cloned());
            }
        }
        commands.into_iter().collect()
    }
}

// ── Classification ───────────────────────────────────────────────────────

/// Which side of the lane a change belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ChangeClass {
    /// A change to the delivery mechanism: lane-scheduled, fixture-gated.
    Mechanism,
    /// Everything else: queue-scheduled, gated by the full candidate gate.
    Payload,
}

impl ChangeClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mechanism => "mechanism",
            Self::Payload => "payload",
        }
    }
}

/// The classification of one change: its class, the mechanism components it
/// touched, and how the class was decided.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Classification {
    /// Mechanism or payload.
    pub class: ChangeClass,
    /// The declared components the paths matched, in declaration order.
    pub components: Vec<String>,
    /// The paths that matched, in input order.
    pub matched_paths: Vec<String>,
    /// Whether the change carried [`MECHANISM_LABEL`].
    pub label_declared: bool,
}

impl Classification {
    /// Classify a change by its labels and its paths.
    ///
    /// Either signal is sufficient — the label is the declaration for changes
    /// a path rule cannot reach, the paths are the declaration for an issue
    /// nobody relabelled — and when both speak, the paths name the components
    /// whose fixtures run.
    pub fn classify(
        surface: &MechanismSurface,
        labels: impl IntoIterator<Item = impl AsRef<str>>,
        paths: impl IntoIterator<Item = impl AsRef<str>>,
    ) -> Self {
        let label_declared = labels
            .into_iter()
            .any(|label| label.as_ref().trim() == MECHANISM_LABEL);
        let mut matched: BTreeSet<String> = BTreeSet::new();
        let mut matched_paths: Vec<String> = Vec::new();
        for path in paths {
            let path = path.as_ref().trim();
            let hits = surface.components_for(path);
            if hits.is_empty() {
                continue;
            }
            matched_paths.push(path.to_string());
            matched.extend(hits.into_iter().map(|component| component.name.clone()));
        }
        // Reported in declaration order, so a change touching two components
        // names them the same way whatever order the diff came in.
        let components: Vec<String> = surface
            .components()
            .iter()
            .map(|component| component.name.clone())
            .filter(|name| matched.contains(name))
            .collect();
        Self {
            class: if label_declared || !components.is_empty() {
                ChangeClass::Mechanism
            } else {
                ChangeClass::Payload
            },
            components,
            matched_paths,
            label_declared,
        }
    }

    pub fn is_mechanism(&self) -> bool {
        self.class == ChangeClass::Mechanism
    }

    /// The declaration and the diff disagree: the issue says mechanism work
    /// and no changed path belongs to a declared component. The class stands
    /// (the label is authoritative — a prompt or deployment change has no
    /// path rule), but the report says so, because a label applied to a
    /// payload change spends the lane's bounded budget on payload.
    pub fn label_without_paths(&self) -> bool {
        self.label_declared && self.components.is_empty()
    }

    /// The one-line report: `mechanism [pipeline, gate]` or `payload`.
    pub fn line(&self) -> String {
        let mut line = self.class.as_str().to_string();
        if !self.components.is_empty() {
            line.push_str(&format!(" [{}]", self.components.join(", ")));
        }
        if self.label_without_paths() {
            line.push_str(" (label only: no changed path is on the declared surface)");
        }
        line
    }
}

// ── The worklist ─────────────────────────────────────────────────────────

/// One candidate waiting to be scheduled, with the two facts the lane
/// decision needs: what it is labelled, and what it would change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkItem {
    /// The issue number.
    pub issue: u64,
    /// The issue's labels.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    /// The paths the change would touch.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
}

impl WorkItem {
    pub fn new(issue: u64) -> Self {
        Self {
            issue,
            labels: Vec::new(),
            paths: Vec::new(),
        }
    }

    /// With labels and paths attached.
    pub fn with(mut self, labels: &[&str], paths: &[&str]) -> Self {
        self.labels = labels.iter().map(|s| (*s).to_string()).collect();
        self.paths = paths.iter().map(|s| (*s).to_string()).collect();
        self
    }

    /// Classify this item against `surface`.
    pub fn classify(&self, surface: &MechanismSurface) -> Classification {
        Classification::classify(surface, &self.labels, &self.paths)
    }
}

/// A worklist read from a manifest: `<issue>\t<labels-csv>\t<paths-csv>` per
/// line, `#` comments and blank lines skipped.
///
/// The manifest is what the tracker hands the scheduler — labels and
/// files-touched are already on the issue; this is their projection. A line
/// whose issue column is not a number is reported in [`Worklist::malformed`]
/// rather than dropped: a worklist that silently loses a candidate is the
/// #3927 failure (filed work that never runs) in a smaller hat.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Worklist {
    /// The candidates, in file order.
    pub items: Vec<WorkItem>,
    /// Lines that could not be read, verbatim, with their line numbers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub malformed: Vec<String>,
}

impl Worklist {
    /// Parse a manifest. Fields may be empty (an issue with no labels and no
    /// known paths is payload); separators may be tabs or runs of spaces.
    pub fn parse(text: &str) -> Self {
        let mut worklist = Self::default();
        for (index, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let columns = split_columns(line);
            let Some(first) = columns.first() else {
                worklist.malformed.push(format!("{}: {line}", index + 1));
                continue;
            };
            let Ok(issue) = first.parse::<u64>() else {
                worklist.malformed.push(format!("{}: {line}", index + 1));
                continue;
            };
            let parse_list = |value: &str| -> Vec<String> {
                value
                    .split(',')
                    .map(|entry| entry.trim().to_string())
                    .filter(|entry| !entry.is_empty())
                    .collect()
            };
            let labels = columns.get(1).map(|s| parse_list(s)).unwrap_or_default();
            let paths = columns.get(2).map(|s| parse_list(s)).unwrap_or_default();
            worklist.items.push(WorkItem {
                issue,
                labels,
                paths,
            });
        }
        worklist
    }

    /// Render back to the manifest format.
    pub fn render(&self) -> String {
        let mut out = String::new();
        for item in &self.items {
            out.push_str(&format!(
                "{}\t{}\t{}\n",
                item.issue,
                item.labels.join(","),
                item.paths.join(",")
            ));
        }
        out
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

fn split_columns(line: &str) -> Vec<String> {
    if line.contains('\t') {
        return line.split('\t').map(|s| s.trim().to_string()).collect();
    }
    line.split_whitespace().map(|s| s.to_string()).collect()
}

// ── The lane policy ──────────────────────────────────────────────────────

/// The declared reservation: how much of a batch the lane may take, how much
/// it may never take, and the window both are stated over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LanePolicy {
    /// Slots the lane may take from one batch.
    pub capacity: usize,
    /// Slots the lane may never take: payload's guaranteed share.
    pub payload_reserve: usize,
    /// The window the reservation and the improvement rate are stated over.
    pub window_secs: u64,
}

impl Default for LanePolicy {
    fn default() -> Self {
        Self {
            capacity: DEFAULT_LANE_CAPACITY,
            payload_reserve: DEFAULT_PAYLOAD_RESERVE,
            window_secs: DEFAULT_WINDOW_SECS,
        }
    }
}

impl LanePolicy {
    /// The policy with an explicit cap, reserve, and window.
    pub fn new(capacity: usize, payload_reserve: usize, window_secs: u64) -> Self {
        Self {
            capacity,
            payload_reserve,
            window_secs,
        }
    }

    /// Whether the policy is a bounded reservation rather than a takeover.
    ///
    /// A zero-cap lane is no lane (the fixed point restored); a zero-reserve
    /// lane is the queue inverted, where payload waits behind the mechanism
    /// that serves it; a zero window makes the rate a division by nothing.
    pub fn validate(&self) -> Result<(), String> {
        if self.capacity == 0 {
            return Err("lane capacity is 0: no capacity is reserved, so mechanism work keeps waiting behind the queue it serves".to_string());
        }
        if self.payload_reserve == 0 {
            return Err("payload reserve is 0: the lane may take the whole batch, which inverts the queue instead of shortening it".to_string());
        }
        if self.window_secs == 0 {
            return Err(
                "window is 0s: the reservation and the improvement rate have no unit of time"
                    .to_string(),
            );
        }
        Ok(())
    }

    /// How many slots the lane may take from a batch of `batch_size`.
    ///
    /// The bound is structural: never more than `capacity`, never more than
    /// what is left after the payload reserve. A batch of 1 with a reserve of
    /// 1 leaves the lane nothing — that is the reserve doing its job, and the
    /// plan reports the mechanism entry as deferred rather than pretending the
    /// lane was free.
    pub fn lane_slots(&self, batch_size: usize) -> usize {
        batch_size
            .saturating_sub(self.payload_reserve)
            .min(self.capacity)
    }

    /// How many mechanism changes the reservation allows per hour: the number
    /// the observed improvement rate is read against.
    pub fn reservation_per_hour(&self) -> f64 {
        (self.capacity as f64) * 3_600.0 / (self.window_secs as f64)
    }

    /// The one-line statement of the policy.
    pub fn line(&self) -> String {
        format!(
            "lane: up to {} slot(s) per batch, {} reserved for payload, window {}s ({} mechanism change(s)/hour reserved)",
            self.capacity,
            self.payload_reserve,
            self.window_secs,
            trim_number(self.reservation_per_hour())
        )
    }
}

/// A number rendered without a trailing `.0`.
fn trim_number(value: f64) -> String {
    if (value - value.round()).abs() < 0.005 {
        format!("{}", value.round() as i64)
    } else {
        format!("{value:.2}")
    }
}

// ── Gate selection ───────────────────────────────────────────────────────

/// The gate a change must pass, with the commands and the budget they are
/// expected to cost.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateSet {
    /// `mechanism-fixture` or `full-candidate`.
    pub name: String,
    /// The commands, in the order the caller should run them.
    pub commands: Vec<String>,
    /// The budget the gate is expected to fit inside.
    pub budget_secs: u64,
    /// Why this gate: travels with the report so the choice is auditable.
    pub reason: String,
}

impl GateSet {
    /// The full per-candidate gate.
    pub fn full_candidate() -> Self {
        Self {
            name: "full-candidate".to_string(),
            commands: FULL_GATE_COMMANDS
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
            budget_secs: FULL_GATE_BUDGET_SECS,
            reason: "payload work: gated by the full per-candidate gate".to_string(),
        }
    }

    /// The mechanism's own fixture gate over the named components.
    pub fn mechanism_fixture(components: &[String], commands: Vec<String>) -> Self {
        Self {
            name: "mechanism-fixture".to_string(),
            commands,
            budget_secs: FIXTURE_GATE_BUDGET_SECS,
            reason: format!(
                "delivery-mechanism work in [{}]: gated by the mechanism's own fixture tests",
                components.join(", ")
            ),
        }
    }

    pub fn is_fixture(&self) -> bool {
        self.name == "mechanism-fixture"
    }

    /// The one-line report: name, budget, and how many commands run.
    pub fn line(&self) -> String {
        format!(
            "gate {} ({}s budget, {} command(s))",
            self.name,
            self.budget_secs,
            self.commands.len()
        )
    }
}

/// Which gate a classified change runs.
///
/// A mechanism change runs the fixture gate of the components it touched —
/// seconds, not the ~15-minute full gate it is waiting to change. A change
/// whose mechanism components name no fixture, or which was labelled
/// mechanism without touching a declared component, runs the full gate: the
/// lane is a scheduling decision, but a gate is evidence, and evidence that
/// cannot answer is never accepted as a pass.
pub fn gate_set(surface: &MechanismSurface, classification: &Classification) -> GateSet {
    if !classification.is_mechanism() {
        return GateSet::full_candidate();
    }
    let commands = surface.fixture_commands(
        &classification
            .components
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>(),
    );
    if commands.is_empty() {
        let mut fallback = GateSet::full_candidate();
        fallback.reason = format!(
            "mechanism work naming no fixture ({}): the full gate runs because a gate that cannot answer is not a cheap gate",
            if classification.components.is_empty() {
                "no declared component matched".to_string()
            } else {
                classification.components.join(", ")
            }
        );
        return fallback;
    }
    GateSet::mechanism_fixture(&classification.components, commands)
}

// ── The schedule ─────────────────────────────────────────────────────────

/// One entry of a [`LanePlan`]: what it is, and the gate it must pass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanEntry {
    pub issue: u64,
    /// The classification that put the entry where it is.
    pub classification: Classification,
    /// The gate the caller must run before this entry becomes a PR.
    pub gate: GateSet,
}

impl PlanEntry {
    /// The one-line report for the entry.
    pub fn line(&self) -> String {
        format!(
            "#{} {} — {}",
            self.issue,
            self.classification.line(),
            self.gate.line()
        )
    }
}

/// The separation of a worklist into lane and queue, with the budget the
/// split was made under and the mechanism work the budget could not admit.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LanePlan {
    /// Mechanism work admitted to the reserved slots, in worklist order.
    pub lane: Vec<PlanEntry>,
    /// Payload work, plus mechanism work the lane could not admit, in
    /// worklist order. Nothing is dropped and nothing waits for a window.
    pub queue: Vec<PlanEntry>,
    /// Slots the lane was allowed for this batch.
    pub lane_slots: usize,
    /// Slots still unused after this plan was built.
    pub lane_remaining: usize,
    /// Mechanism entries the lane did not admit — because its slots were gone
    /// or because no fixture gate was available for them. The bound, visible:
    /// these ride the normal queue, they are never dropped and never wait for
    /// a fresh window.
    pub deferred: Vec<u64>,
}

impl LanePlan {
    pub fn is_empty(&self) -> bool {
        self.lane.is_empty() && self.queue.is_empty()
    }

    pub fn len(&self) -> usize {
        self.lane.len() + self.queue.len()
    }

    /// Every entry that acts this batch, lane first, then the queue.
    pub fn scheduled(&self) -> Vec<&PlanEntry> {
        self.lane.iter().chain(self.queue.iter()).collect()
    }

    /// The report lines: the policy, the lane, then the queue.
    pub fn lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        lines.push(format!(
            "lane: {}/{} slot(s) used, {} remaining; queue: {} entry(s)",
            self.lane.len(),
            self.lane_slots,
            self.lane_remaining,
            self.queue.len()
        ));
        for entry in &self.lane {
            lines.push(format!("  lane  {}", entry.line()));
        }
        for entry in &self.queue {
            lines.push(format!("  queue {}", entry.line()));
        }
        if !self.deferred.is_empty() {
            lines.push(format!(
                "  lane not admitted: mechanism {} queued behind payload (no slot left, or no fixture gate) — not starved, not lost",
                join_issues(&self.deferred)
            ));
        }
        lines
    }
}

fn join_issues(issues: &[u64]) -> String {
    issues
        .iter()
        .map(u64::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Split a worklist into the reserved lane and the normal queue.
///
/// Mechanism work is considered first and takes at most
/// [`LanePolicy::lane_slots`] slots minus what this window has already used;
/// everything else keeps its position in the worklist. The lane therefore
/// reorders mechanism work to the front without reordering payload work, and
/// mechanism work that does not fit is queued where it stood — the lane has a
/// budget, not a veto.
pub fn schedule(
    worklist: &Worklist,
    surface: &MechanismSurface,
    policy: &LanePolicy,
    ledger: &ImprovementLedger,
    now: u64,
) -> LanePlan {
    let slots = policy.lane_slots(worklist.items.len());
    let used = ledger.landed_in_window(LandedClass::Mechanism, now, policy.window_secs);
    let remaining = slots.saturating_sub(used);

    let mut classified: Vec<(usize, Classification)> = worklist
        .items
        .iter()
        .enumerate()
        .map(|(index, item)| (index, item.classify(surface)))
        .collect();
    // Mechanism first, everything else after; both halves keep input order.
    classified.sort_by_key(|(index, classification)| (!classification.is_mechanism(), *index));

    let mut plan = LanePlan {
        lane_slots: slots,
        lane_remaining: remaining,
        ..Default::default()
    };
    let mut left = remaining;
    for (index, classification) in classified {
        let item = &worklist.items[index];
        let entry = PlanEntry {
            issue: item.issue,
            gate: gate_set(surface, &classification),
            classification,
        };
        // The lane admits mechanism work that actually has a fixture gate:
        // taking a reserved slot for a change that still runs the full gate
        // would spend the reservation without saving the wait.
        let admitted = entry.classification.is_mechanism() && entry.gate.is_fixture();
        if admitted && left > 0 {
            plan.lane.push(entry);
            left -= 1;
        } else {
            if entry.classification.is_mechanism() {
                plan.deferred.push(item.issue);
            }
            plan.queue.push(entry);
        }
    }
    plan.lane_remaining = left;
    // Both halves are reported in worklist order: the lane is a reservation,
    // not a re-shuffle of the queue's own ordering.
    plan.deferred.sort_unstable();
    plan
}

// ── The improvement ledger ───────────────────────────────────────────────

/// What landed, on which side of the lane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LandedClass {
    /// A change to the delivery mechanism.
    Mechanism,
    /// Everything else.
    Payload,
}

impl LandedClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mechanism => "mechanism",
            Self::Payload => "payload",
        }
    }

    /// Read a class from its name (`mechanism` / `payload`), as passed on the
    /// command line or read back from a ledger written by another tool.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "mechanism" | "mech" => Some(Self::Mechanism),
            "payload" => Some(Self::Payload),
            _ => None,
        }
    }
}

/// One landed change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LandedRecord {
    pub issue: u64,
    pub class: LandedClass,
    /// Epoch seconds at which the change landed.
    pub landed_at: u64,
}

/// The append-only journal of landed work that the improvement rate is
/// computed from.
///
/// The ledger answers the question the queue cannot: not "how much work is
/// waiting" but "how fast is the thing that moves the work getting better".
/// One record per issue — a change lands once — and an issue that was recorded
/// as one class cannot be re-recorded as the other: reclassifying history to
/// move a number is exactly what the rate must not be able to do.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImprovementLedger {
    records: Vec<LandedRecord>,
}

impl ImprovementLedger {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a landed change. Returns `false` when the issue is already in
    /// the ledger — the record stands as first written.
    pub fn record(&mut self, issue: u64, class: LandedClass, landed_at: u64) -> bool {
        if self.records.iter().any(|record| record.issue == issue) {
            return false;
        }
        self.records.push(LandedRecord {
            issue,
            class,
            landed_at,
        });
        self.records
            .sort_by_key(|record| (record.landed_at, record.issue));
        true
    }

    /// The records in landing order.
    pub fn records(&self) -> &[LandedRecord] {
        &self.records
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// The class a recorded issue landed as.
    pub fn class_of(&self, issue: u64) -> Option<LandedClass> {
        self.records
            .iter()
            .find(|record| record.issue == issue)
            .map(|record| record.class)
    }

    /// How many changes of `class` landed within `window_secs` of `now`.
    ///
    /// Records in the future relative to `now` are excluded: a ledger read at
    /// `now` reports what has happened by `now`, and a mis-stamped record must
    /// not inflate a rate.
    pub fn landed_in_window(&self, class: LandedClass, now: u64, window_secs: u64) -> usize {
        self.records
            .iter()
            .filter(|record| {
                record.class == class
                    && record.landed_at <= now
                    && now - record.landed_at < window_secs
            })
            .count()
    }

    /// The improvement rate over the policy's window.
    pub fn improvement_rate(
        &self,
        policy: &LanePolicy,
        backlog: usize,
        now: u64,
    ) -> ImprovementRate {
        let mechanism_landed =
            self.landed_in_window(LandedClass::Mechanism, now, policy.window_secs);
        let payload_landed = self.landed_in_window(LandedClass::Payload, now, policy.window_secs);
        let per_hour = round2(mechanism_landed as f64 * 3_600.0 / (policy.window_secs as f64));
        let verdict = if backlog == 0 {
            ImprovementVerdict::NoBacklog
        } else if mechanism_landed == 0 {
            ImprovementVerdict::FixedPoint
        } else if mechanism_landed < policy.capacity {
            ImprovementVerdict::ReservationUnused
        } else {
            ImprovementVerdict::Improving
        };
        ImprovementRate {
            window_secs: policy.window_secs,
            mechanism_landed,
            payload_landed,
            per_hour,
            reservation_per_hour: round2(policy.reservation_per_hour()),
            backlog,
            verdict,
        }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_string())
    }

    pub fn from_json(text: &str) -> Result<Self, String> {
        serde_json::from_str(text).map_err(|error| error.to_string())
    }
}

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

/// The reported rate of self-improvement, and what it means.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ImprovementRate {
    /// The window the counts were taken over.
    pub window_secs: u64,
    /// Mechanism changes landed in the window.
    pub mechanism_landed: usize,
    /// Payload changes landed in the same window (the lane's trade-off, in
    /// the same units).
    pub payload_landed: usize,
    /// Mechanism changes per hour.
    pub per_hour: f64,
    /// What the reservation would allow per hour.
    pub reservation_per_hour: f64,
    /// Changes waiting — mechanism work plus queue depth, supplied by the
    /// caller, because a rate with no backlog is not a fixed point.
    pub backlog: usize,
    /// The verdict.
    pub verdict: ImprovementVerdict,
}

impl ImprovementRate {
    /// The one-line report.
    pub fn line(&self) -> String {
        format!(
            "improvement rate: {} mechanism change(s) in {}s ({} per hour, {} per hour reserved), {} payload, backlog {}: {}",
            self.mechanism_landed,
            self.window_secs,
            trim_number(self.per_hour),
            trim_number(self.reservation_per_hour),
            self.payload_landed,
            self.backlog,
            self.verdict.as_str()
        )
    }
}

/// What an improvement rate says about the system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ImprovementVerdict {
    /// Work is waiting and the mechanism is changing: the system is fixing
    /// the thing that fixes the rest.
    Improving,
    /// Work is waiting and fewer mechanism changes landed in the window than
    /// the reservation allows: the lane exists and something other than its
    /// cap is holding the work back.
    ReservationUnused,
    /// Work is waiting and nothing mechanism-side has landed in the window:
    /// the fixed point, named. Self-improvement is zero while the queue that
    /// would fix the queue is served by the queue.
    FixedPoint,
    /// Nothing is waiting; the rate has nothing to say.
    NoBacklog,
}

impl ImprovementVerdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Improving => "improving",
            Self::ReservationUnused => "reservation-unused",
            Self::FixedPoint => "fixed-point",
            Self::NoBacklog => "no-backlog",
        }
    }

    /// Whether the verdict is a fault the caller should exit non-zero on:
    /// work waiting with the mechanism frozen is the #3795 state.
    pub fn is_fault(self) -> bool {
        self == Self::FixedPoint
    }
}
