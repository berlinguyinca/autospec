//! Fast lane for delivery-mechanism changes (#3795).
//!
//! The delivery mechanism — pipeline, dispatcher, gates, agent runner —
//! was changed by the same queue it serves: a mechanism change queued
//! behind per-candidate work waited for the very gate it was trying to
//! improve, and the full per-candidate gate ran on a change whose effect
//! the mechanism's own fixture test could have shown in seconds. The
//! fixed point — the mechanism stops improving because improving it is
//! slower than waiting — was invisible because nothing reported the
//! mechanism's own landing rate.
//!
//! The policy is encoded here as pure, testable primitives, read from
//! [`config/fast-lane.yml`](FAST_LANE_CONFIG_PATH); callers classify,
//! schedule, gate and report on the plans these functions return:
//!
//! 1. **Mechanism work is distinguishable from work that passes through
//!    the mechanism** ([`FastLanePolicy::classify`]): a changed path
//!    under a mechanism prefix, or the mechanism label, marks the work.
//!
//! 2. **Mechanism work is scheduled separately, within a bounded cap**
//!    ([`FastLanePolicy::schedule`]): the fast lane processes at most
//!    `cap_per_cycle` mechanism changes per cycle; the excess returns to
//!    the head of the normal queue, so the lane can never starve the
//!    queue it serves.
//!
//! 3. **Mechanism work is gated by the mechanism's own tests**
//!    ([`FastLanePolicy::gates_for`]): the fixture tests that exercise
//!    the mechanism directly — seconds, not the full per-candidate gate.
//!
//! 4. **The system reports its own improvement rate**
//!    ([`ImprovementLedger`]): mechanism changes landed per unit time,
//!    so the fixed point is a number (0/h) rather than an absence.

use std::fmt;
use std::str::FromStr;

use yaml_edit::{Document, Mapping};

/// Repository-relative path of the fast-lane policy config.
pub const FAST_LANE_CONFIG_PATH: &str = "config/fast-lane.yml";

/// Committed fast-lane policy, embedded at compile time.
const REPOSITORY_CONFIG: &str = include_str!("../../../../config/fast-lane.yml");

/// A measurement or config value that cannot form a valid fast-lane
/// policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FastLanePolicyError {
    /// The config's `version` is missing or not `1`.
    UnsupportedVersion,
    /// A required field is missing or has the wrong YAML type.
    MalformedField,
    /// `mechanism.cap_per_cycle` is missing, not a positive integer, or
    /// not bounded (the lane's capacity must be a stated finite number).
    InvalidCap,
    /// `report.window_hours` is missing, negative, zero, or not finite.
    InvalidWindow,
    /// `gates.mechanism` or `gates.full` is missing or empty: a gate set
    /// that names no gate grades nothing.
    EmptyGateSet,
}

impl fmt::Display for FastLanePolicyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::UnsupportedVersion => "version must be 1",
            Self::MalformedField => "a required field is missing or malformed",
            Self::InvalidCap => "mechanism.cap_per_cycle must be a positive, bounded integer",
            Self::InvalidWindow => "report.window_hours must be a positive, finite number",
            Self::EmptyGateSet => "gates.mechanism and gates.full must each name at least one gate",
        };
        f.write_str(message)
    }
}

impl std::error::Error for FastLanePolicyError {}

/// Whether a unit of work changes the delivery mechanism or merely passes
/// through it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkClass {
    /// The work changes the mechanism itself: pipeline, dispatcher,
    /// gates, or agent runner.
    Mechanism,
    /// The work is delivered *by* the mechanism without changing it.
    Transit,
}

impl WorkClass {
    /// The class name, as it appears in schedules and reports.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mechanism => "mechanism",
            Self::Transit => "transit",
        }
    }
}

/// A unit of schedulable work: what it is, what it touches, what it is
/// labelled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Work {
    /// Stable identifier (issue number, patch id).
    pub id: String,
    /// Repository-relative paths the work changes.
    pub paths: Vec<String>,
    /// Labels on the work.
    pub labels: Vec<String>,
}

impl Work {
    /// A unit of work with the given id, touched paths, and labels.
    pub fn new(id: impl Into<String>, paths: Vec<String>, labels: Vec<String>) -> Self {
        Self {
            id: id.into(),
            paths,
            labels,
        }
    }
}

/// The outcome of scheduling a worklist: the bounded fast lane and the
/// normal queue.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Schedule {
    /// Mechanism work admitted to the fast lane this cycle, in input
    /// order. Never longer than the policy cap.
    pub fast_lane: Vec<String>,
    /// Everything else, in input order: transit work, then mechanism
    /// work the cap turned away (at the head, keeping its priority).
    pub normal: Vec<String>,
}

impl Schedule {
    /// True when no work was scheduled at all.
    pub fn is_empty(&self) -> bool {
        self.fast_lane.is_empty() && self.normal.is_empty()
    }
    /// A one-line rendering a caller can log so the split is visible
    /// rather than inferred: the lane's occupancy against its cap, and
    /// both queues by id.
    pub fn render(&self, cap: u32) -> String {
        format!(
            "fast lane: {} (cap {}); normal: {}",
            join_or_empty(&self.fast_lane),
            cap,
            join_or_empty(&self.normal),
        )
    }
}

fn join_or_empty(ids: &[String]) -> String {
    if ids.is_empty() {
        "-".to_string()
    } else {
        ids.join(", ")
    }
}

/// The gate set a work class is graded against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateSet {
    /// The mechanism's own tests: the fixture tests that exercise the
    /// mechanism directly. Seconds.
    Mechanism {
        /// The gate commands, in run order.
        gates: Vec<String>,
    },
    /// The full per-candidate gate.
    Full {
        /// The gate commands, in run order.
        gates: Vec<String>,
    },
}

impl GateSet {
    /// The gate commands this set names, in run order.
    pub fn gates(&self) -> &[String] {
        match self {
            Self::Mechanism { gates } => gates,
            Self::Full { gates } => gates,
        }
    }
    /// Whether this is the mechanism's own fast gate set.
    pub fn is_mechanism(&self) -> bool {
        matches!(self, Self::Mechanism { .. })
    }
    /// The set's name, as it appears in verdicts.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Mechanism { .. } => "mechanism",
            Self::Full { .. } => "full",
        }
    }
}

/// The fast-lane policy, parsed from `config/fast-lane.yml`.
#[derive(Debug, Clone, PartialEq)]
pub struct FastLanePolicy {
    /// The label that marks work as a mechanism change by itself.
    label: String,
    /// Path prefixes that mark work as a mechanism change by path.
    path_prefixes: Vec<String>,
    /// Reserved capacity of the fast lane, per dispatch cycle.
    cap_per_cycle: u32,
    /// The mechanism's own gate set (its fixture tests).
    mechanism_gates: Vec<String>,
    /// The full per-candidate gate set.
    full_gates: Vec<String>,
    /// The window the improvement rate is reported over, in hours.
    report_window_hours: f64,
}

impl FastLanePolicy {
    /// Parse a fast-lane policy from YAML.
    ///
    /// Fails closed: an unreadable or malformed policy is an error, never
    /// an empty policy — an empty policy would silently put every
    /// mechanism change behind the full per-candidate gate.
    pub fn parse(text: &str) -> Result<Self, FastLanePolicyError> {
        let doc = Document::from_str(text).map_err(|_| FastLanePolicyError::MalformedField)?;
        let root = doc
            .as_mapping()
            .ok_or(FastLanePolicyError::MalformedField)?;

        let Some(version) = optional_scalar(&root, "version")? else {
            return Err(FastLanePolicyError::UnsupportedVersion);
        };
        if version.trim() != "1" {
            return Err(FastLanePolicyError::UnsupportedVersion);
        }

        let mechanism_node = root
            .get("mechanism")
            .ok_or(FastLanePolicyError::MalformedField)?;
        let mechanism = mechanism_node
            .as_mapping()
            .ok_or(FastLanePolicyError::MalformedField)?;

        let label = required_scalar(mechanism, "label")?;

        let cap_raw = required_scalar(mechanism, "cap_per_cycle")?;
        let cap_raw: u64 = cap_raw
            .trim()
            .parse()
            .map_err(|_| FastLanePolicyError::InvalidCap)?;
        if cap_raw == 0 {
            // A lane with no capacity is not a fast lane; a policy that
            // states no bound is not bounded policy.
            return Err(FastLanePolicyError::InvalidCap);
        }
        let cap_per_cycle = u32::try_from(cap_raw).map_err(|_| FastLanePolicyError::InvalidCap)?;

        let path_prefixes = scalar_list(mechanism, "path_prefixes")?;
        if path_prefixes.is_empty() {
            // A mechanism section that claims no path and depends only on
            // the label is a typo waiting to happen; fail closed.
            return Err(FastLanePolicyError::MalformedField);
        }

        let gates_node = root
            .get("gates")
            .ok_or(FastLanePolicyError::MalformedField)?;
        let gates = gates_node
            .as_mapping()
            .ok_or(FastLanePolicyError::MalformedField)?;

        let mechanism_gates = gate_list(gates, "mechanism")?;
        let full_gates = gate_list(gates, "full")?;

        let report_node = root
            .get("report")
            .ok_or(FastLanePolicyError::InvalidWindow)?;
        let report = report_node
            .as_mapping()
            .ok_or(FastLanePolicyError::InvalidWindow)?;
        let window_raw = required_scalar(report, "window_hours")?;
        let report_window_hours: f64 = window_raw
            .trim()
            .parse()
            .map_err(|_| FastLanePolicyError::InvalidWindow)?;
        if !report_window_hours.is_finite() || report_window_hours <= 0.0 {
            return Err(FastLanePolicyError::InvalidWindow);
        }

        Ok(Self {
            label,
            path_prefixes,
            cap_per_cycle,
            mechanism_gates,
            full_gates,
            report_window_hours,
        })
    }

    /// The committed repository policy, embedded at compile time.
    pub fn repository() -> Result<Self, FastLanePolicyError> {
        Self::parse(REPOSITORY_CONFIG)
    }

    /// The label that marks work as a mechanism change by itself.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// The path prefixes that mark work as a mechanism change by path.
    pub fn path_prefixes(&self) -> &[String] {
        &self.path_prefixes
    }

    /// The lane's reserved capacity, per dispatch cycle.
    pub fn cap_per_cycle(&self) -> u32 {
        self.cap_per_cycle
    }

    /// The window the improvement rate is reported over, in hours.
    pub fn report_window_hours(&self) -> f64 {
        self.report_window_hours
    }

    /// Whether `path` falls under a mechanism path prefix.
    pub fn is_mechanism_path(&self, path: &str) -> bool {
        self.path_prefixes
            .iter()
            .any(|prefix| path.starts_with(prefix.as_str()))
    }

    /// Classify work: mechanism when it touches the mechanism by path or
    /// carries the mechanism label, transit otherwise.
    ///
    /// Either marker suffices: a path match catches the change even
    /// before it is labelled, and the label catches a mechanism change
    /// that touches no configured path (policy docs, gate-set
    /// definitions).
    pub fn classify(&self, paths: &[String], labels: &[String]) -> WorkClass {
        if labels.contains(&self.label) {
            return WorkClass::Mechanism;
        }
        if paths.iter().any(|path| self.is_mechanism_path(path)) {
            return WorkClass::Mechanism;
        }
        WorkClass::Transit
    }

    /// Schedule a worklist: mechanism work goes to the fast lane in
    /// input order, up to the cap; the excess returns to the head of the
    /// normal queue ahead of transit work.
    ///
    /// The cap is per cycle, so a backlog of mechanism work makes the
    /// lane full every cycle without ever holding the whole queue: the
    /// excess is still scheduled, still ahead of transit work, just not
    /// on the lane. Work ids must be unique in `work`.
    pub fn schedule(&self, work: &[Work]) -> Schedule {
        let mut fast_lane = Vec::new();
        let mut overflow = Vec::new();
        let mut normal = Vec::new();

        for item in work {
            if self.classify(&item.paths, &item.labels) == WorkClass::Mechanism {
                if (fast_lane.len() as u32) < self.cap_per_cycle {
                    fast_lane.push(item.id.clone());
                } else {
                    overflow.push(item.id.clone());
                }
            } else {
                normal.push(item.id.clone());
            }
        }

        normal.splice(0..0, overflow);

        Schedule { fast_lane, normal }
    }

    /// The gate set a work class is graded against.
    ///
    /// Mechanism work is gated by the mechanism's own tests — the
    /// fixture tests that exercise it directly — not by the full
    /// per-candidate gate it is trying to improve.
    pub fn gates_for(&self, class: WorkClass) -> GateSet {
        match class {
            WorkClass::Mechanism => GateSet::Mechanism {
                gates: self.mechanism_gates.clone(),
            },
            WorkClass::Transit => GateSet::Full {
                gates: self.full_gates.clone(),
            },
        }
    }

    /// The improvement-rate line for landed mechanism changes, reported
    /// over this policy's window: the count, the rate per hour, and the
    /// fixed-point marker when nothing landed.
    pub fn report_line(
        &self,
        ledger: &ImprovementLedger,
        now_hours: f64,
    ) -> Result<String, ImprovementError> {
        Ok(ledger.rate(now_hours, self.report_window_hours)?.line())
    }
}

fn optional_scalar(mapping: &Mapping, key: &str) -> Result<Option<String>, FastLanePolicyError> {
    let Some(node) = mapping.get(key) else {
        return Ok(None);
    };
    node.as_scalar()
        .map(|scalar| scalar.as_string())
        .map(|value| {
            if value.trim().is_empty() {
                None
            } else {
                Some(value)
            }
        })
        .ok_or(FastLanePolicyError::MalformedField)
}

fn required_scalar(mapping: &Mapping, key: &str) -> Result<String, FastLanePolicyError> {
    optional_scalar(mapping, key)?.ok_or(FastLanePolicyError::MalformedField)
}

fn scalar_list(mapping: &Mapping, key: &str) -> Result<Vec<String>, FastLanePolicyError> {
    let Some(node) = mapping.get(key) else {
        return Ok(Vec::new());
    };
    let sequence = node
        .as_sequence()
        .ok_or(FastLanePolicyError::MalformedField)?;
    sequence
        .values()
        .map(|item| {
            item.as_scalar()
                .map(|scalar| scalar.as_string())
                .filter(|value| !value.trim().is_empty())
                .ok_or(FastLanePolicyError::MalformedField)
        })
        .collect()
}

fn gate_list(gates: &Mapping, key: &str) -> Result<Vec<String>, FastLanePolicyError> {
    // A missing or null gate set means the work class is graded by no
    // gate: fail closed with the specific error rather than a silent
    // empty set.
    let Some(node) = gates.get(key) else {
        return Err(FastLanePolicyError::EmptyGateSet);
    };
    let sequence = node
        .as_sequence()
        .ok_or(FastLanePolicyError::EmptyGateSet)?;
    let gates: Vec<String> = sequence
        .values()
        .map(|item| {
            item.as_scalar()
                .map(|scalar| scalar.as_string())
                .filter(|value| !value.trim().is_empty())
                .ok_or(FastLanePolicyError::EmptyGateSet)
        })
        .collect::<Result<Vec<String>, FastLanePolicyError>>()?;
    if gates.is_empty() {
        return Err(FastLanePolicyError::EmptyGateSet);
    }
    Ok(gates)
}

/// An improvement-rate measurement that cannot be taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImprovementError {
    /// A timestamp, now, or window is negative, zero (for the window),
    /// or not a finite number.
    InvalidMeasurement,
}

impl fmt::Display for ImprovementError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(
            "a timestamp or window must be a finite, non-negative number (window: positive)",
        )
    }
}

impl std::error::Error for ImprovementError {}

/// The mechanism's own landing rate over a stated window: the report
/// that makes the fixed point a number.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImprovementRate {
    /// Mechanism changes that landed inside the window.
    landed: usize,
    /// The window the count is taken over, in hours.
    window_hours: f64,
}

impl ImprovementRate {
    /// Mechanism changes that landed inside the window.
    pub fn landed(self) -> usize {
        self.landed
    }
    /// The window the count is taken over, in hours.
    pub fn window_hours(self) -> f64 {
        self.window_hours
    }
    /// The landing rate, mechanism changes per hour.
    pub fn per_hour(self) -> f64 {
        self.landed as f64 / self.window_hours
    }
    /// The fixed point: the mechanism stopped improving. Visible as a
    /// number (0/h) rather than as an absence.
    pub fn is_fixed_point(self) -> bool {
        self.landed == 0
    }
    /// The report line: count, window, rate, and the fixed-point
    /// marker when the rate is zero.
    pub fn line(&self) -> String {
        if self.is_fixed_point() {
            format!(
                "improvement rate: 0 mechanism changes landed in the last {} h (0.00/h) — fixed point: the mechanism is not improving",
                self.window_hours
            )
        } else {
            format!(
                "improvement rate: {} mechanism change{} landed in the last {} h ({:.2}/h)",
                self.landed,
                if self.landed == 1 { "" } else { "s" },
                self.window_hours,
                self.per_hour(),
            )
        }
    }
}

/// Landed mechanism changes, timestamped. The ledger is append-only:
/// the rate is always taken from what actually landed, never from what
/// was scheduled.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImprovementLedger {
    /// Landing times, in hours (same time base as `now` at rate
    /// measurement).
    landed_at_hours: Vec<f64>,
}

impl ImprovementLedger {
    /// An empty ledger: nothing has landed yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a mechanism change landing at `at_hours`.
    pub fn record_landing(&mut self, at_hours: f64) -> Result<(), ImprovementError> {
        if !at_hours.is_finite() || at_hours < 0.0 {
            return Err(ImprovementError::InvalidMeasurement);
        }
        self.landed_at_hours.push(at_hours);
        Ok(())
    }

    /// Number of mechanism changes recorded as landed.
    pub fn landed_count(&self) -> usize {
        self.landed_at_hours.len()
    }

    /// The improvement rate over `(now - window, now]`: the mechanism's
    /// own landing rate, per unit time.
    ///
    /// A landing exactly at the window's back edge is outside the
    /// window; a landing at `now` is inside it.
    pub fn rate(
        &self,
        now_hours: f64,
        window_hours: f64,
    ) -> Result<ImprovementRate, ImprovementError> {
        if !now_hours.is_finite() || now_hours < 0.0 {
            return Err(ImprovementError::InvalidMeasurement);
        }
        if !window_hours.is_finite() || window_hours <= 0.0 {
            return Err(ImprovementError::InvalidMeasurement);
        }
        let back_edge = now_hours - window_hours;
        let landed = self
            .landed_at_hours
            .iter()
            .filter(|at| **at > back_edge && **at <= now_hours)
            .count();
        Ok(ImprovementRate {
            landed,
            window_hours,
        })
    }

    /// The improvement-rate report line over `(now - window, now]`.
    pub fn line(&self, now_hours: f64, window_hours: f64) -> Result<String, ImprovementError> {
        Ok(self.rate(now_hours, window_hours)?.line())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy_yaml() -> String {
        r#"
version: 1
mechanism:
  label: autospec:mechanism
  cap_per_cycle: 2
  path_prefixes:
    - crates/autospec-core/src/execution/
    - crates/autospec-core/src/dispatch_pipeline.rs
gates:
  mechanism:
    - cargo test -p autospec-core
  full:
    - cargo fmt --all --check
    - cargo test --workspace
report:
  window_hours: 12
"#
        .to_string()
    }

    fn policy() -> FastLanePolicy {
        FastLanePolicy::parse(&policy_yaml()).unwrap()
    }

    #[test]
    fn repository_policy_parses_the_committed_file() {
        let from_file = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/fast-lane.yml"),
        )
        .expect("read committed config");
        let policy = FastLanePolicy::repository().expect("repository policy");
        assert_eq!(
            policy,
            FastLanePolicy::parse(&from_file).expect("parse same text"),
            "embedded config must match the committed file"
        );
        // The committed policy states a bounded cap and distinct gate
        // sets: the fast lane is a different, smaller gate than the
        // per-candidate gate.
        assert!(policy.cap_per_cycle() >= 1);
        let mechanism = policy.gates_for(WorkClass::Mechanism);
        let full = policy.gates_for(WorkClass::Transit);
        assert!(mechanism.is_mechanism());
        assert!(!full.is_mechanism());
        assert_ne!(mechanism.gates(), full.gates());
        assert!(!mechanism.gates().is_empty());
        assert!(!full.gates().is_empty());
    }

    #[test]
    fn classify_by_mechanism_path_prefix() {
        let policy = policy();
        // Directory prefix: a descendant path matches.
        assert_eq!(
            policy.classify(
                &["crates/autospec-core/src/execution/queue.rs".to_string()],
                &[]
            ),
            WorkClass::Mechanism
        );
        // Exact-file prefix: the file itself matches.
        assert_eq!(
            policy.classify(
                &["crates/autospec-core/src/dispatch_pipeline.rs".to_string()],
                &[]
            ),
            WorkClass::Mechanism
        );
        // A sibling that merely shares the stem does not match.
        assert_eq!(
            policy.classify(
                &["crates/autospec-core/src/execution_backup.rs".to_string()],
                &[]
            ),
            WorkClass::Transit
        );
        // A path that only passes through the mechanism is transit.
        assert_eq!(
            policy.classify(&["apps/web/src/index.ts".to_string()], &[]),
            WorkClass::Transit
        );
        // No paths at all is transit.
        assert_eq!(policy.classify(&[], &[]), WorkClass::Transit);
    }

    #[test]
    fn classify_by_mechanism_label_even_without_path_match() {
        let policy = policy();
        assert_eq!(
            policy.classify(
                &["docs/gate-set-policy.md".to_string()],
                &["autospec:mechanism".to_string()]
            ),
            WorkClass::Mechanism
        );
        // An unrelated label does not mark the work.
        assert_eq!(
            policy.classify(
                &["apps/web/src/index.ts".to_string()],
                &["auto-implement".to_string()]
            ),
            WorkClass::Transit
        );
    }

    #[test]
    fn schedule_puts_mechanism_work_on_a_bounded_fast_lane() {
        let policy = policy(); // cap 2
        let work = vec![
            Work::new(
                "1",
                vec!["crates/autospec-core/src/execution/queue.rs".into()],
                vec![],
            ),
            Work::new("2", vec!["apps/web/src/a.ts".into()], vec![]),
            Work::new(
                "3",
                vec!["crates/autospec-core/src/dispatch_pipeline.rs".into()],
                vec![],
            ),
            Work::new(
                "4",
                vec!["crates/autospec-core/src/execution/gate.rs".into()],
                vec![],
            ),
            Work::new("5", vec!["apps/web/src/b.ts".into()], vec![]),
        ];

        let schedule = policy.schedule(&work);

        // Mechanism work first, in input order, up to the cap.
        assert_eq!(schedule.fast_lane, vec!["1".to_string(), "3".to_string()]);
        // The excess mechanism work ("4") returns to the head of the
        // normal queue, ahead of transit work: still scheduled, still
        // ahead, just not on the lane this cycle.
        assert_eq!(
            schedule.normal,
            vec!["4".to_string(), "2".to_string(), "5".to_string()]
        );
        assert!((schedule.fast_lane.len() as u32) <= policy.cap_per_cycle());
        // The split is visible rather than inferred.
        assert_eq!(
            schedule.render(policy.cap_per_cycle()),
            "fast lane: 1, 3 (cap 2); normal: 4, 2, 5"
        );
    }

    #[test]
    fn schedule_with_no_mechanism_work_leaves_the_lane_empty() {
        let policy = policy();
        let work = vec![Work::new("9", vec!["apps/web/src/a.ts".into()], vec![])];
        let schedule = policy.schedule(&work);
        assert!(schedule.fast_lane.is_empty());
        assert_eq!(schedule.normal, vec!["9".to_string()]);
        assert_eq!(
            schedule.render(policy.cap_per_cycle()),
            "fast lane: - (cap 2); normal: 9"
        );
    }

    #[test]
    fn schedule_empty_worklist_is_empty() {
        let policy = policy();
        let schedule = policy.schedule(&[]);
        assert!(schedule.is_empty());
    }

    #[test]
    fn mechanism_work_is_gated_by_the_mechanisms_own_tests() {
        let policy = policy();
        let mechanism = policy.gates_for(WorkClass::Mechanism);
        assert!(mechanism.is_mechanism());
        assert_eq!(
            mechanism.gates(),
            &["cargo test -p autospec-core".to_string()]
        );

        let full = policy.gates_for(WorkClass::Transit);
        assert!(!full.is_mechanism());
        assert_eq!(
            full.gates(),
            &[
                "cargo fmt --all --check".to_string(),
                "cargo test --workspace".to_string()
            ]
        );

        // The two sets are different: the lane's gate is not the
        // per-candidate gate.
        assert_ne!(mechanism.gates(), full.gates());
        assert_eq!(
            GateSet::Mechanism {
                gates: vec!["g".into()]
            }
            .as_str(),
            "mechanism"
        );
        assert_eq!(
            GateSet::Full {
                gates: vec!["g".into()]
            }
            .as_str(),
            "full"
        );
    }

    #[test]
    fn parse_rejects_malformed_policies() {
        let mut bad = policy_yaml();
        bad = bad.replace("version: 1", "version: 2");
        assert_eq!(
            FastLanePolicy::parse(&bad),
            Err(FastLanePolicyError::UnsupportedVersion)
        );

        bad = policy_yaml().replace("version: 1", "");
        assert_eq!(
            FastLanePolicy::parse(&bad),
            Err(FastLanePolicyError::UnsupportedVersion)
        );

        bad = policy_yaml().replace("  cap_per_cycle: 2", "  cap_per_cycle: 0");
        assert_eq!(
            FastLanePolicy::parse(&bad),
            Err(FastLanePolicyError::InvalidCap)
        );

        bad = policy_yaml().replace("  cap_per_cycle: 2", "  cap_per_cycle: -2");
        assert_eq!(
            FastLanePolicy::parse(&bad),
            Err(FastLanePolicyError::InvalidCap)
        );

        bad = policy_yaml().replace("  window_hours: 12", "  window_hours: 0");
        assert_eq!(
            FastLanePolicy::parse(&bad),
            Err(FastLanePolicyError::InvalidWindow)
        );

        bad = policy_yaml().replace("  window_hours: 12", "  window_hours: -3");
        assert_eq!(
            FastLanePolicy::parse(&bad),
            Err(FastLanePolicyError::InvalidWindow)
        );

        bad = policy_yaml().replace("    - cargo test -p autospec-core", "");
        assert_eq!(
            FastLanePolicy::parse(&bad),
            Err(FastLanePolicyError::EmptyGateSet)
        );

        bad = policy_yaml().replace("gates:", "gates_missing:");
        assert_eq!(
            FastLanePolicy::parse(&bad),
            Err(FastLanePolicyError::MalformedField)
        );

        assert_eq!(
            FastLanePolicy::parse("not: [valid yaml doc"),
            Err(FastLanePolicyError::MalformedField)
        );
    }

    #[test]
    fn ledger_reports_landing_rate_over_the_window() {
        let mut ledger = ImprovementLedger::new();
        // Window (now - 12, now] with now = 100 h: landings at 90, 99,
        // 100 are inside; 87.9 and 88.0 are outside (88.0 is exactly
        // the back edge).
        ledger.record_landing(90.0).unwrap();
        ledger.record_landing(99.0).unwrap();
        ledger.record_landing(100.0).unwrap();
        ledger.record_landing(87.9).unwrap();
        ledger.record_landing(88.0).unwrap();
        assert_eq!(ledger.landed_count(), 5);

        let rate = ledger.rate(100.0, 12.0).unwrap();
        assert_eq!(rate.landed(), 3);
        assert_eq!(rate.window_hours(), 12.0);
        assert!((rate.per_hour() - 0.25).abs() < f64::EPSILON);
        assert!(!rate.is_fixed_point());
        assert_eq!(
            rate.line(),
            "improvement rate: 3 mechanism changes landed in the last 12 h (0.25/h)"
        );
    }

    #[test]
    fn ledger_fixed_point_is_visible_as_a_number() {
        let ledger = ImprovementLedger::new();
        let rate = ledger.rate(50.0, 24.0).unwrap();
        assert_eq!(rate.landed(), 0);
        assert!((rate.per_hour()).abs() < f64::EPSILON);
        assert!(rate.is_fixed_point());
        assert_eq!(
            rate.line(),
            "improvement rate: 0 mechanism changes landed in the last 24 h (0.00/h) — fixed point: the mechanism is not improving"
        );
        // A single landing is not a fixed point, and the line is
        // singular.
        let mut one = ImprovementLedger::new();
        one.record_landing(50.0).unwrap();
        assert_eq!(
            one.line(50.0, 24.0).unwrap(),
            "improvement rate: 1 mechanism change landed in the last 24 h (0.04/h)"
        );
    }

    #[test]
    fn policy_report_line_uses_the_policy_window() {
        let policy = policy(); // window 12 h
        let mut ledger = ImprovementLedger::new();
        ledger.record_landing(95.0).unwrap();
        assert_eq!(
            policy.report_line(&ledger, 100.0).unwrap(),
            "improvement rate: 1 mechanism change landed in the last 12 h (0.08/h)"
        );
        assert_eq!(
            policy.report_line(&ImprovementLedger::new(), 100.0).unwrap(),
            "improvement rate: 0 mechanism changes landed in the last 12 h (0.00/h) — fixed point: the mechanism is not improving"
        );
    }

    #[test]
    fn ledger_rejects_invalid_measurements() {
        let mut ledger = ImprovementLedger::new();
        assert_eq!(
            ledger.record_landing(-1.0),
            Err(ImprovementError::InvalidMeasurement)
        );
        assert_eq!(
            ledger.record_landing(f64::NAN),
            Err(ImprovementError::InvalidMeasurement)
        );
        assert_eq!(
            ledger.record_landing(f64::INFINITY),
            Err(ImprovementError::InvalidMeasurement)
        );
        assert!(ledger.landed_count() == 0);

        assert_eq!(
            ledger.rate(-5.0, 12.0),
            Err(ImprovementError::InvalidMeasurement)
        );
        assert_eq!(
            ledger.rate(5.0, 0.0),
            Err(ImprovementError::InvalidMeasurement)
        );
        assert_eq!(
            ledger.rate(5.0, f64::NAN),
            Err(ImprovementError::InvalidMeasurement)
        );
    }
}
