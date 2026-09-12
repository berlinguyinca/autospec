//! Prerequisites block dispatch; gates block release (issue #4015).
//!
//! autospec decomposes a spec into GitHub issues with a `## Dependencies`
//! section per issue, and a dispatcher treats every entry there as a hard
//! prerequisite: an issue is eligible only when all of them are closed.
//! Two different relations were being written into that one field:
//!
//! - **X is a technical prerequisite of Y** — Y cannot be *implemented*
//!   until X exists, because Y consumes an interface X produces. This must
//!   block dispatch.
//! - **X gates the release of a phase** — the phase must not *ship*
//!   unreviewed. This must block shipping, and is a label or milestone.
//!
//! Measured on a 61-issue roadmap: 82 open→open edges, maximum chain depth
//! **28**, five issues ready, twenty-one workers idle. The depth rested on
//! five phase-approval checkpoints ("Phase-1 soak, failure injection and
//! independent review", "Phase-2 private-alpha review and qualification",
//! …) each listed as a `## Dependencies` entry of the next phase's work.
//! Removing only those five edges — touching no technical dependency —
//! brought the depth to 7 and the frontier to 10. #53, an independent-review
//! checkpoint with no enrolled reviewer, transitively blocked **55 of the 61
//! open issues**. The dispatcher reported "nothing new is ready", which is
//! true, and read as "the chain is progressing normally" rather than "the
//! chain cannot progress at all".
//!
//! Five invariants, each a checkable primitive here:
//!
//! 1. **A decomposer distinguishes a prerequisite from a gate, and only
//!    prerequisites belong in the dispatch-blocking field.**
//!    [`classify`] decides per dependency; [`emit`] writes only
//!    prerequisites to the `## Dependencies` section and emits gates as
//!    labels.
//! 2. **A review/qualification/sign-off/approval checkpoint is a gate by
//!    default.** A checkpoint classified as a prerequisite must state what
//!    interface it produces that the dependent consumes
//!    ([`DependencySpec::produced_interface`]).
//! 3. **A generated graph is measured before it is filed.** [`measure`]
//!    reports maximum depth, width at the ready frontier, and the
//!    transitive downstream count of the top three blocking issues;
//!    [`depth_verdict`] rejects a graph deeper than a configurable
//!    threshold, naming the blocking issues, rather than filing it.
//! 4. **A dispatcher that finds zero eligible issues distinguishes an
//!    exhausted frontier from a frontier blocked on issues that carry no
//!    assignee and cannot be completed by an agent**, and reports the
//!    latter distinctly ([`frontier_verdict`]).
//! 5. **Before a metric is reported as an opportunity, compute what changes
//!    if you act on it.** Edge count and path depth are computed over the
//!    artifact; only how much becomes startable is a decision input. An edge
//!    is transitively redundant precisely because another path already
//!    implies the ordering, so removing it frees at most the issues whose
//!    sole open predecessor was that edge — never one issue per edge. The
//!    actionable quantity is the simulated change to the ready set
//!    ([`redundant_edges`], [`ready_set_change`]).
//!
//! Everything here is pure: no I/O, no subprocess. Traversals are iterative
//! over sorted containers, so deep or cyclic graphs cannot overflow the
//! stack and results are deterministic across runs.

use std::collections::{BTreeMap, BTreeSet};

/// Terms that mark an issue a review, qualification, sign-off, or approval
/// checkpoint. Matching is case-insensitive on word boundaries, after
/// normalising `_` to `-`, so `Sign-Off`, `SIGNOFF`, `sign_off`, and
/// `independent review` all match and `preview` does not.
pub const CHECKPOINT_TERMS: &[&str] = &[
    "review",
    "qualification",
    "sign-off",
    "signoff",
    "sign off",
    "approval",
    "checkpoint",
];

fn has_term(text: &str, term: &str) -> bool {
    let bytes = text.as_bytes();
    for (i, window) in text.match_indices(term) {
        let before_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
        let after = i + window.len();
        let after_ok = after == bytes.len() || !bytes[after].is_ascii_alphanumeric();
        if before_ok && after_ok {
            return true;
        }
    }
    false
}

/// Invariant 2: does the issue's title or body mark it a review,
/// qualification, sign-off, or approval checkpoint?
pub fn is_checkpoint(title: &str, body: &str) -> bool {
    let title = title.to_lowercase().replace('_', "-");
    let body = body.to_lowercase().replace('_', "-");
    CHECKPOINT_TERMS
        .iter()
        .any(|term| has_term(&title, term) || has_term(&body, term))
}

/// How a dependency is classified: dispatch-blocking or release-blocking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DependencyClass {
    /// Y cannot be implemented until X exists: Y consumes an interface X
    /// produces. Belongs in the dispatch-blocking `## Dependencies`
    /// section. `interface` is `Some` when the source is a checkpoint and
    /// the classification is justified by the interface it produces.
    Prerequisite { interface: Option<String> },
    /// X gates the release of a phase: the phase must not ship unreviewed.
    /// Emitted as a label, never as a dispatch-blocking dependency.
    Gate,
}

/// One dependency the decomposer emits for a dependent issue: `source`
/// must complete (prerequisite) or its phase must pass review (gate)
/// before the dependent may proceed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DependencySpec {
    pub source_id: String,
    pub source_title: String,
    pub source_body: String,
    /// For a checkpoint source classified as a prerequisite: the
    /// interface it produces that the dependent consumes. Without it a
    /// checkpoint is a gate by default (invariant 2).
    pub produced_interface: Option<String>,
}

/// Invariant 1: classify one emitted dependency.
///
/// A checkpoint source (invariant 2) is a `Gate` unless it states the
/// interface it produces that the dependent consumes; any non-checkpoint
/// source is a `Prerequisite`.
pub fn classify(spec: &DependencySpec) -> DependencyClass {
    if is_checkpoint(&spec.source_title, &spec.source_body) {
        match &spec.produced_interface {
            Some(interface) if !interface.trim().is_empty() => DependencyClass::Prerequisite {
                interface: Some(interface.clone()),
            },
            _ => DependencyClass::Gate,
        }
    } else {
        DependencyClass::Prerequisite { interface: None }
    }
}

/// Invariant 1, as emitted: the dispatch-blocking section entries and the
/// gate labels.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Emission {
    /// The `## Dependencies` entries for the dependent issue: the source
    /// ids of prerequisite dependencies only. Gates never appear here.
    pub dependencies: Vec<String>,
    /// One label per gate dependency: `gate:<source_id>`.
    pub gate_labels: Vec<String>,
}

/// Split a set of emitted dependencies into the dispatch-blocking entries
/// (prerequisites) and the gate labels. Input order is preserved, so the
/// emission is deterministic for a deterministic decomposer.
pub fn emit(specs: &[DependencySpec]) -> Emission {
    let mut out = Emission::default();
    for spec in specs {
        match classify(spec) {
            DependencyClass::Prerequisite { .. } => out.dependencies.push(spec.source_id.clone()),
            DependencyClass::Gate => {
                out.gate_labels.push(format!("gate:{}", spec.source_id));
            }
        }
    }
    out
}

/// Invariant 3: the numbers one pass over the graph takes, measured
/// before the graph is filed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphMeasurement {
    /// Longest dependency chain in nodes. 0 for an empty graph.
    pub max_depth: usize,
    /// Width at the ready frontier: issues with no unresolved
    /// prerequisites.
    pub frontier_width: usize,
    /// The top three blocking issues by transitive downstream count
    /// (`id, count`), sorted by count descending then id ascending. Only
    /// issues that block at least one other appear; the incident's
    /// `53 → 55` is the first entry.
    pub top_blockers: Vec<(String, usize)>,
}

impl GraphMeasurement {
    /// The report line: the three numbers, in order.
    pub fn line(&self) -> String {
        let blockers = if self.top_blockers.is_empty() {
            "none".to_string()
        } else {
            self.top_blockers
                .iter()
                .map(|(id, n)| format!("{id} (blocks {n})"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        format!(
            "graph: depth {}, frontier width {}, top blockers: {blockers}",
            self.max_depth, self.frontier_width
        )
    }
}

type Adj<'a> = BTreeMap<&'a str, BTreeSet<&'a str>>;

/// Predecessor/successor adjacency over `ids`, ignoring edges whose
/// endpoints are not in the graph and self-edges.
fn adjacency<'a>(ids: &[&'a str], edges: &[(&'a str, &'a str)]) -> (Adj<'a>, Adj<'a>) {
    let id_set: BTreeSet<&str> = ids.iter().copied().collect();
    let mut preds: BTreeMap<&str, BTreeSet<&str>> =
        ids.iter().map(|id| (*id, BTreeSet::new())).collect();
    let mut succs: BTreeMap<&str, BTreeSet<&str>> =
        ids.iter().map(|id| (*id, BTreeSet::new())).collect();
    for (pre, succ) in edges {
        if pre != succ && id_set.contains(pre) && id_set.contains(succ) {
            preds.get_mut(succ).unwrap().insert(pre);
            succs.get_mut(pre).unwrap().insert(succ);
        }
    }
    (preds, succs)
}

/// Transitive successor count of `start` (iterative; `start` is not
/// counted in its own set).
fn downstream_count<'a>(start: &'a str, succs: &BTreeMap<&'a str, BTreeSet<&'a str>>) -> usize {
    let mut stack = Vec::new();
    if let Some(first) = succs.get(start) {
        stack.extend(first.iter().copied());
    }
    let mut seen = BTreeSet::new();
    while let Some(node) = stack.pop() {
        if !seen.insert(node) {
            continue;
        }
        if let Some(next) = succs.get(node) {
            stack.extend(next.iter().copied());
        }
    }
    seen.len()
}

/// Longest dependency chain in nodes, Kahn order. Nodes left after the
/// peel (a cycle — the decomposer's own cycle detection should have caught
/// it) count as depth 1 rather than hanging the measurement.
fn max_depth<'a>(
    preds: &BTreeMap<&'a str, BTreeSet<&'a str>>,
    succs: &BTreeMap<&'a str, BTreeSet<&'a str>>,
) -> usize {
    let mut remaining: BTreeMap<&str, usize> = preds.iter().map(|(id, p)| (*id, p.len())).collect();
    let mut depth: BTreeMap<&str, usize> = BTreeMap::new();
    let mut frontier: Vec<&str> = remaining
        .iter()
        .filter(|(_, n)| **n == 0)
        .map(|(id, _)| *id)
        .collect();
    frontier.sort_unstable();
    let mut i = 0;
    while i < frontier.len() {
        let u = frontier[i];
        i += 1;
        let du = depth.get(u).copied().unwrap_or(1);
        for v in succs.get(u).into_iter().flatten() {
            let dv = depth.get(v).copied().unwrap_or(1).max(du + 1);
            depth.insert(*v, dv);
            let slot = remaining.get_mut(v).unwrap();
            *slot -= 1;
            if *slot == 0 {
                frontier.push(v);
                // Keep the walk deterministic: re-sort from the next index.
                frontier[i..].sort_unstable();
            }
        }
    }
    // Cyclic residue: depth 1 each, so max never underreports a cycle node.
    let mut max = 0usize;
    for id in preds.keys() {
        max = max.max(depth.get(id).copied().unwrap_or(1));
    }
    max
}

/// Invariant 3: measure the graph the decomposer produced, before it is
/// filed. One pass derives all three reported numbers.
pub fn measure<'a>(ids: &[&'a str], edges: &[(&'a str, &'a str)]) -> GraphMeasurement {
    let (preds, succs) = adjacency(ids, edges);
    if ids.is_empty() {
        return GraphMeasurement {
            max_depth: 0,
            frontier_width: 0,
            top_blockers: Vec::new(),
        };
    }
    let mut blockers: Vec<(String, usize)> = ids
        .iter()
        .map(|id| (id.to_string(), downstream_count(id, &succs)))
        .filter(|(_, n)| *n > 0)
        .collect();
    blockers.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    blockers.truncate(3);

    GraphMeasurement {
        max_depth: max_depth(&preds, &succs),
        frontier_width: preds.values().filter(|p| p.is_empty()).count(),
        top_blockers: blockers,
    }
}

/// Invariant 3: the configurable depth threshold a decomposed graph must
/// meet to be filed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DepthPolicy {
    pub max_depth: usize,
}

impl Default for DepthPolicy {
    /// The incident's graph, with only the five gate edges removed, had
    /// maximum depth 7 — a legitimately deep roadmap, not one no fleet of
    /// workers can make progress on.
    fn default() -> Self {
        Self { max_depth: 7 }
    }
}

/// Invariant 3: the file-or-reject verdict for one measured graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DepthVerdict {
    /// The graph is within the threshold: file it.
    File { depth: usize, limit: usize },
    /// The graph is deeper than the threshold: reject the decomposition
    /// and name the blocking issues (the top blockers) rather than file a
    /// structurally unschedulable roadmap.
    Reject {
        depth: usize,
        limit: usize,
        blockers: Vec<(String, usize)>,
    },
}

impl DepthVerdict {
    /// The rendered verdict line.
    pub fn line(&self) -> String {
        match self {
            DepthVerdict::File { depth, limit } => {
                format!("file: depth {depth} within threshold {limit}")
            }
            DepthVerdict::Reject {
                depth,
                limit,
                blockers,
            } => {
                let named = blockers
                    .iter()
                    .map(|(id, n)| format!("{id} (blocks {n})"))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("reject: depth {depth} exceeds threshold {limit}; blocking issues: {named}")
            }
        }
    }
}

/// Invariant 3, applied: reject a graph deeper than the policy, naming the
/// blocking issues, instead of filing it.
pub fn depth_verdict(measurement: &GraphMeasurement, policy: &DepthPolicy) -> DepthVerdict {
    if measurement.max_depth > policy.max_depth {
        DepthVerdict::Reject {
            depth: measurement.max_depth,
            limit: policy.max_depth,
            blockers: measurement.top_blockers.clone(),
        }
    } else {
        DepthVerdict::File {
            depth: measurement.max_depth,
            limit: policy.max_depth,
        }
    }
}

/// One open issue on the dispatcher's frontier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontierIssue {
    pub id: String,
    /// `None` when nobody — human or agent — is assigned to it.
    pub assignee: Option<String>,
    /// Whether an automated agent can complete the issue. Phase
    /// checkpoints that require an enrolled human reviewer are `false`.
    pub agent_completable: bool,
}

/// One open issue holding the frontier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blocker {
    pub id: String,
    /// Other open issues transitively downstream of this blocker.
    pub downstream: usize,
    /// Carries no assignee and cannot be completed by an agent: the block
    /// no automated run can clear.
    pub unassignable: bool,
}

/// Invariant 4: the dispatcher's frontier after it found zero eligible
/// issues (or some, reported for completeness).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrontierVerdict {
    /// `count` open issues have every prerequisite closed and an agent
    /// can complete them: dispatchable.
    Eligible { count: usize },
    /// No open issues remain: the frontier is genuinely exhausted.
    Exhausted,
    /// Open issues remain but none is eligible. `blockers` are the open
    /// issues that transitively hold at least one other open issue, or
    /// that no agent can complete (a lone gate with nothing downstream
    /// still blocks the frontier), sorted by downstream count descending
    /// then id ascending.
    Held { blockers: Vec<Blocker> },
}

impl FrontierVerdict {
    /// Invariant 4: the frontier is blocked on issues that carry no
    /// assignee and cannot be completed by an agent. This is the state the
    /// incident's "nothing new is ready" concealed: the chain cannot
    /// progress at all, and every component reports healthy.
    pub fn blocked_on_unassignable(&self) -> bool {
        matches!(self, FrontierVerdict::Held { blockers } if blockers.iter().any(|b| b.unassignable))
    }

    /// The rendered line. An unassignable block is reported distinctly —
    /// `frontier blocked on … an agent cannot complete` — never as the
    /// ordinary hold a progressing chain produces.
    pub fn line(&self) -> String {
        match self {
            FrontierVerdict::Eligible { count } => format!("frontier: {count} eligible"),
            FrontierVerdict::Exhausted => "frontier exhausted: no open issues remain".to_string(),
            FrontierVerdict::Held { blockers } => {
                let named = |b: &Blocker| format!("{} (holds {})", b.id, b.downstream);
                let un: Vec<&Blocker> = blockers.iter().filter(|b| b.unassignable).collect();
                let others: Vec<&Blocker> = blockers.iter().filter(|b| !b.unassignable).collect();
                if un.is_empty() {
                    format!(
                        "frontier held behind {} open issue(s): {}",
                        blockers.len(),
                        blockers.iter().map(named).collect::<Vec<_>>().join(", ")
                    )
                } else {
                    let mut line = format!(
                        "frontier blocked on {} unassignable issue(s) an agent cannot complete: {}",
                        un.len(),
                        un.iter().copied().map(named).collect::<Vec<_>>().join(", ")
                    );
                    if !others.is_empty() {
                        line.push_str(&format!(
                            "; held behind {} open issue(s): {}",
                            others.len(),
                            others
                                .iter()
                                .copied()
                                .map(named)
                                .collect::<Vec<_>>()
                                .join(", ")
                        ));
                    }
                    line
                }
            }
        }
    }
}

/// Invariant 4: decide the frontier. `edges` are prerequisite edges
/// (`predecessor, successor`) among the open issues; an issue is eligible
/// when no predecessor of its is still open.
pub fn frontier_verdict<'a>(
    issues: &[FrontierIssue],
    edges: &[(&'a str, &'a str)],
) -> FrontierVerdict {
    if issues.is_empty() {
        return FrontierVerdict::Exhausted;
    }
    let ids: Vec<&str> = issues.iter().map(|i| i.id.as_str()).collect();
    let (preds, succs) = adjacency(&ids, edges);
    let open: BTreeSet<&str> = issues.iter().map(|i| i.id.as_str()).collect();
    // Eligible for dispatch: every predecessor closed AND an agent can
    // complete the issue. A checkpoint with no prerequisites is open work
    // no agent may take, not an eligible dispatch.
    let eligible = issues
        .iter()
        .filter(|i| {
            i.agent_completable
                && preds
                    .get(i.id.as_str())
                    .map(|p| p.iter().all(|x| !open.contains(x)))
                    .unwrap_or(true)
        })
        .count();
    if eligible > 0 {
        return FrontierVerdict::Eligible { count: eligible };
    }
    // Blockers: held issues that are causes — they hold at least one other
    // open issue transitively, or they are themselves not agent-completable
    // (a lone gate with nothing downstream still blocks the frontier).
    let blockers: Vec<Blocker> = issues
        .iter()
        .map(|i| {
            let downstream = downstream_count(i.id.as_str(), &succs);
            // A held completable leaf holds nothing: it is held, not a
            // cause. A cause holds at least one other open issue, or is a
            // checkpoint no agent can complete (a lone gate with nothing
            // downstream still blocks the frontier).
            let cause = downstream > 0 || !i.agent_completable;
            (
                i.id.clone(),
                downstream,
                i.assignee.is_none() && !i.agent_completable,
                cause,
            )
        })
        .filter(|(_, _, _, cause)| *cause)
        .map(|(id, downstream, unassignable, _)| Blocker {
            id,
            downstream,
            unassignable,
        })
        .collect();
    FrontierVerdict::Held {
        blockers: sort_blockers(blockers),
    }
}

fn sort_blockers(mut blockers: Vec<Blocker>) -> Vec<Blocker> {
    blockers.sort_by(|a, b| {
        b.downstream
            .cmp(&a.downstream)
            .then_with(|| a.id.cmp(&b.id))
    });
    blockers
}

/// Invariant 5: how many issues are startable — the size of the ready set.
/// An issue is startable when it has no open predecessor. This is the
/// quantity an edge-level "opportunity" must be cashed into: edge counts and
/// path depth describe the artifact and are not decision inputs on their own.
fn ready_width<'a>(ids: &[&'a str], edges: &[(&'a str, &'a str)]) -> usize {
    let (preds, _) = adjacency(ids, edges);
    preds.values().filter(|p| p.is_empty()).count()
}

/// Invariant 5: the transitively-redundant edges of the graph. An edge is
/// redundant when its successor is still reachable from its predecessor once
/// the edge itself is removed — the ordering it enforces is already implied
/// by another path.
///
/// This is the artifact-level metric the pitfall reports ("38% of the
/// edges"). It is real and reproduces exactly, and it is inert on its own:
/// it counts the edges, not the outcome. Feed it to [`ready_set_change`] to
/// see what acting on it buys.
pub fn redundant_edges<'a>(
    ids: &[&'a str],
    edges: &[(&'a str, &'a str)],
) -> BTreeSet<(&'a str, &'a str)> {
    let (_, succs) = adjacency(ids, edges);
    edges
        .iter()
        .copied()
        .filter(|(pre, succ)| pre != succ && reaches_without_direct(pre, succ, &succs))
        .collect()
}

/// From `pre`, is `succ` reachable without traversing the direct
/// `pre -> succ` edge at all? True when a second path already orders the two
/// nodes. Iterative, so cyclic graphs cannot overflow the stack.
fn reaches_without_direct<'a>(pre: &'a str, succ: &'a str, succs: &Adj<'a>) -> bool {
    let mut stack = vec![pre];
    let mut seen = BTreeSet::new();
    while let Some(node) = stack.pop() {
        let Some(nexts) = succs.get(node) else {
            continue;
        };
        for &next in nexts {
            if node == pre && next == succ {
                continue; // never traverse the edge under test
            }
            if next == succ {
                return true;
            }
            if seen.insert(next) {
                stack.push(next);
            }
        }
    }
    false
}

/// Invariant 5: the change a set of edge removals makes to the ready set,
/// computed by simulation rather than by counting the edges removed. `removed`
/// is the set of edges the proposed action would drop; the result reports the
/// ready set before and after. This is the decision input — the number that
/// says "acting on the redundancy gains one issue" — not the count of edges in
/// `removed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadySetChange {
    /// Issues startable before the edit.
    pub before: usize,
    /// Issues startable after the edit.
    pub after: usize,
}

impl ReadySetChange {
    /// How many issues become startable as a result of the edit. Zero (or
    /// negative) is the tell of the structurally inert metric: a large
    /// artifact-level number that cashes into nothing.
    pub fn gained(&self) -> i64 {
        self.after as i64 - self.before as i64
    }

    /// The report line: the outcome, not the number of edges touched.
    pub fn line(&self) -> String {
        format!(
            "ready set {} -> {} (gains {})",
            self.before,
            self.after,
            self.gained()
        )
    }
}

/// Invariant 5, applied: remove `removed` from the graph and report the
/// simulated change to the ready set. Removing an edge only ever lowers a
/// successor's open-predecessor count, so the ready set can only grow — but
/// for a redundant edge it grows by nothing, because another path already
/// holds the successor.
pub fn ready_set_change<'a>(
    ids: &[&'a str],
    edges: &[(&'a str, &'a str)],
    removed: &BTreeSet<(&'a str, &'a str)>,
) -> ReadySetChange {
    let before = ready_width(ids, edges);
    let kept: Vec<(&str, &str)> = edges
        .iter()
        .copied()
        .filter(|edge| !removed.contains(edge))
        .collect();
    ReadySetChange {
        before,
        after: ready_width(ids, &kept),
    }
}
