//! `AS-DAG-001` through `AS-DAG-010` lint rule catalogue.
//!
//! Source: `docs/specs/2026-09-08-parallel-decomposition-fleet-saturation.md`
//! §22 (DAG Lint Rules), §12 (reason codes), §16 (dependency section), §18
//! (conflict risk is not dependency).
//!
//! Every predicate here is pure: it takes issue text plus plain integer graph
//! metrics computed by the caller. It never takes a graph type and does no
//! graph traversal — traversal and metric computation belong to the DAG
//! analyzer (§19), not to this catalogue.
//!
//! Text predicates scan linearly (no regex), so authored rationale text cannot
//! trigger regex denial of service.

use std::collections::BTreeSet;

/// Severity of a [`DagLintFinding`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DagLintSeverity {
    /// Advisory: the graph can still be scheduled, but a reviewer should look.
    Warning,
    /// The dependency metadata is invalid; the graph must not be accepted.
    Error,
    /// The graph is unschedulable.
    Fatal,
}

/// The ten DAG lint rules with stable ids (dashboard-facing; do not renumber).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DagLintRule {
    /// AS-DAG-001: dependency has no recognized reason code or artifact.
    UnjustifiedDependency,
    /// AS-DAG-002: rationale describes ordering/convenience, not a prerequisite.
    OrderOnlyDependency,
    /// AS-DAG-003: child references no artifact produced by the predecessor.
    ArtificialSerialization,
    /// AS-DAG-004: issue depends on more than the fan-in threshold.
    ExcessiveFanIn,
    /// AS-DAG-005: critical path too long for the issue count.
    ExcessiveCriticalPath,
    /// AS-DAG-006: initial ready width too low for capacity and issue count.
    LowInitialWidth,
    /// AS-DAG-007: root issues declare overlapping primary write ownership.
    SharedWriteHotspot,
    /// AS-DAG-008: siblings connected only because they came from one split.
    SplitCreatedOrdering,
    /// AS-DAG-009: machine metadata and Markdown dependency section differ.
    MetadataDependencyMismatch,
    /// AS-DAG-010: the dependency graph contains a cycle.
    Cycle,
}

impl DagLintRule {
    /// Stable rule id, `AS-DAG-001` through `AS-DAG-010`.
    pub fn id(self) -> &'static str {
        match self {
            Self::UnjustifiedDependency => "AS-DAG-001",
            Self::OrderOnlyDependency => "AS-DAG-002",
            Self::ArtificialSerialization => "AS-DAG-003",
            Self::ExcessiveFanIn => "AS-DAG-004",
            Self::ExcessiveCriticalPath => "AS-DAG-005",
            Self::LowInitialWidth => "AS-DAG-006",
            Self::SharedWriteHotspot => "AS-DAG-007",
            Self::SplitCreatedOrdering => "AS-DAG-008",
            Self::MetadataDependencyMismatch => "AS-DAG-009",
            Self::Cycle => "AS-DAG-010",
        }
    }

    /// Per-rule severity (spec §22: 003/005/006 warning, 010 fatal; 001 must
    /// fail lint per §12 "unsupported reason codes MUST fail lint").
    pub fn severity(self) -> DagLintSeverity {
        match self {
            Self::UnjustifiedDependency => DagLintSeverity::Error,
            Self::MetadataDependencyMismatch => DagLintSeverity::Error,
            Self::Cycle => DagLintSeverity::Fatal,
            Self::OrderOnlyDependency
            | Self::ArtificialSerialization
            | Self::ExcessiveFanIn
            | Self::ExcessiveCriticalPath
            | Self::LowInitialWidth
            | Self::SharedWriteHotspot
            | Self::SplitCreatedOrdering => DagLintSeverity::Warning,
        }
    }
}

/// One lint finding for a proposed issue dependency graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DagLintFinding {
    pub rule: DagLintRule,
    pub subject: String,
    pub message: String,
}

impl DagLintFinding {
    fn new(rule: DagLintRule, subject: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            rule,
            subject: subject.into(),
            message: message.into(),
        }
    }

    /// Severity of the finding's rule.
    pub fn severity(&self) -> DagLintSeverity {
        self.rule.severity()
    }

    /// Stable rule id of the finding.
    pub fn rule_id(&self) -> &'static str {
        self.rule.id()
    }
}

/// AS-DAG-004 default: an issue depending on more than this many predecessors
/// needs explicit justification per edge.
pub const DEFAULT_FAN_IN_THRESHOLD: usize = 5;

/// AS-DAG-007 default: this many root issues sharing primary write ownership
/// is a hotspot.
pub const DEFAULT_SHARED_WRITE_THRESHOLD: usize = 3;

/// Recognized machine reason codes (spec §12). Unsupported codes fail lint.
pub const RECOGNIZED_REASON_CODES: [&str; 9] = [
    "consumes-new-interface",
    "consumes-new-type",
    "consumes-new-schema",
    "consumes-migration",
    "consumes-generated-artifact",
    "requires-structural-migration",
    "requires-new-protocol",
    "verification-requires-predecessor",
    "external-prerequisite",
];

/// Ordering/convenience wording that describes how work is sequenced rather
/// than a technical prerequisite (spec §22 AS-DAG-002 examples, plus
/// "do first", the same convenience claim without a verb swap).
const ORDER_ONLY_PHRASES: [&str; 5] = [
    "implement first",
    "do first",
    "foundation",
    "do before ui",
    "easier if",
];

/// AS-DAG-001: fire when a dependency edge carries neither a recognized
/// reason code nor a named artifact.
pub fn unjustified_dependency(
    reason_code: Option<&str>,
    artifact: Option<&str>,
) -> Option<DagLintFinding> {
    let justified = reason_code.is_some_and(|code| RECOGNIZED_REASON_CODES.contains(&code))
        || artifact.is_some_and(|value| !value.trim().is_empty());
    if justified {
        return None;
    }

    let detail = match reason_code {
        Some(code) => {
            format!("reason code '{code}' is not a recognized code and no artifact is named")
        }
        None => "no reason code and no artifact named".to_string(),
    };
    Some(DagLintFinding::new(
        DagLintRule::UnjustifiedDependency,
        "dependency edge",
        format!("dependency is unjustified: {detail}"),
    ))
}

/// AS-DAG-002: fire when the dependency rationale is ordering/convenience
/// wording rather than a technical prerequisite.
///
/// Linear word-start-boundary scan, case-insensitive; no regex, so adversarial
/// rationale text cannot cause catastrophic backtracking.
pub fn order_only_dependency(rationale: &str) -> Option<DagLintFinding> {
    let lowered = rationale.to_lowercase();
    let phrase = ORDER_ONLY_PHRASES
        .into_iter()
        .find(|phrase| contains_word_start(&lowered, phrase))?;

    Some(DagLintFinding::new(
        DagLintRule::OrderOnlyDependency,
        rationale.trim().to_string(),
        format!("rationale uses ordering wording '{phrase}' instead of a technical prerequisite"),
    ))
}

/// AS-DAG-003: fire when the child issue text never references the artifact
/// the predecessor supposedly produces (warning unless the caller has
/// deterministic proof, which this pure predicate cannot have).
pub fn artificial_serialization(child_text: &str, artifact: &str) -> Option<DagLintFinding> {
    let artifact = artifact.trim();
    if artifact.is_empty() {
        return None;
    }
    if child_text.contains(artifact) {
        return None;
    }

    Some(DagLintFinding::new(
        DagLintRule::ArtificialSerialization,
        artifact.to_string(),
        format!(
            "child never references artifact '{artifact}' produced by its predecessor and appears independently implementable"
        ),
    ))
}

/// AS-DAG-004: fire when `count` exceeds `threshold` (default
/// [`DEFAULT_FAN_IN_THRESHOLD`]).
pub fn excessive_fan_in(count: usize, threshold: usize) -> Option<DagLintFinding> {
    if count <= threshold {
        return None;
    }

    Some(DagLintFinding::new(
        DagLintRule::ExcessiveFanIn,
        format!("fan-in {count}"),
        format!(
            "issue depends on {count} predecessors (threshold {threshold}); justify each edge or remove one"
        ),
    ))
}

/// AS-DAG-005: for `issue_count >= 10`, fire when
/// `critical_path > max(5, ceil(issue_count * 0.30))`.
pub fn excessive_critical_path(issue_count: usize, critical_path: usize) -> Option<DagLintFinding> {
    if issue_count < 10 {
        return None;
    }
    let limit = 5.max(ceil_ratio(issue_count, 3, 10));
    if critical_path <= limit {
        return None;
    }

    Some(DagLintFinding::new(
        DagLintRule::ExcessiveCriticalPath,
        format!("critical path {critical_path}"),
        format!("critical path {critical_path} exceeds limit {limit} for {issue_count} issues"),
    ))
}

/// AS-DAG-006: for `issue_count >= 10` and `capacity >= 10`, fire when
/// `initial_width < min(capacity, max(4, ceil(issue_count * 0.25)))`.
pub fn low_initial_width(
    issue_count: usize,
    capacity: usize,
    initial_width: usize,
) -> Option<DagLintFinding> {
    if issue_count < 10 || capacity < 10 {
        return None;
    }
    let floor = capacity.min(4.max(ceil_ratio(issue_count, 1, 4)));
    if initial_width >= floor {
        return None;
    }

    Some(DagLintFinding::new(
        DagLintRule::LowInitialWidth,
        format!("initial width {initial_width}"),
        format!(
            "only {initial_width} of {issue_count} issues are initially ready (floor {floor} at capacity {capacity}); one concurrency-review retry is forced"
        ),
    ))
}

/// AS-DAG-007: fire when `root_overlaps` (root issues declaring overlapping
/// primary write ownership) reaches [`DEFAULT_SHARED_WRITE_THRESHOLD`].
pub fn shared_write_hotspot(root_overlaps: usize) -> Option<DagLintFinding> {
    if root_overlaps < DEFAULT_SHARED_WRITE_THRESHOLD {
        return None;
    }

    Some(DagLintFinding::new(
        DagLintRule::SharedWriteHotspot,
        format!("overlap {root_overlaps}"),
        format!(
            "{root_overlaps} root issues declare overlapping primary write ownership (threshold {DEFAULT_SHARED_WRITE_THRESHOLD}); split ownership"
        ),
    ))
}

/// AS-DAG-009: fire when the machine metadata dependency set and the Markdown
/// `## Dependencies` section disagree. Order and duplicates do not matter.
pub fn metadata_mismatch(yaml_deps: &[u64], markdown_deps: &[u64]) -> Option<DagLintFinding> {
    let yaml: BTreeSet<u64> = yaml_deps.iter().copied().collect();
    let markdown: BTreeSet<u64> = markdown_deps.iter().copied().collect();
    if yaml == markdown {
        return None;
    }

    Some(DagLintFinding::new(
        DagLintRule::MetadataDependencyMismatch,
        "dependencies",
        format!(
            "machine metadata declares {yaml:?} but the Markdown dependency section declares {markdown:?}"
        ),
    ))
}

/// AS-DAG-010: a cycle makes the graph unschedulable; always fatal.
pub fn cycle(path: &[String]) -> DagLintFinding {
    DagLintFinding::new(
        DagLintRule::Cycle,
        path.join(" -> "),
        "dependency cycle detected; the graph is unschedulable",
    )
}

/// True when `phrase` occurs in `text` at a word start (start of text or
/// preceded by a non-alphanumeric). Linear time.
fn contains_word_start(text: &str, phrase: &str) -> bool {
    let bytes = text.as_bytes();
    let mut offset = 0;
    while let Some(found) = text[offset..].find(phrase) {
        let start = offset + found;
        let at_word_start = start == 0 || !bytes[start - 1].is_ascii_alphanumeric();
        if at_word_start {
            return true;
        }
        offset = start + phrase.len();
    }
    false
}

/// `ceil(numerator * a / b)` in integer arithmetic.
fn ceil_ratio(numerator: usize, a: usize, b: usize) -> usize {
    (numerator * a).div_ceil(b)
}
