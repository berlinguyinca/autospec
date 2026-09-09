//! Issue-DAG model: cycle detection, Kahn execution waves, and critical path.
//!
//! Spec: `docs/specs/2026-09-08-parallel-decomposition-fleet-saturation.md`
//! — §19 (DAG analyzer), §24 (execution waves), §28 (proposed Rust data
//! structures), §29 (algorithm requirements).
//!
//! This module owns graph *traversal* over [`IssueGraph`]; the metadata
//! shapes it consumes (`Ownership`, `ConcurrencyMetadata`,
//! `DependencyReason`) live in [`crate::graph::metadata`], and the aggregate
//! metrics in [`crate::graph::metrics`]. It sits beside — and does not
//! extend — `crate::graph::order`, which orders `SpecMetadata`.
//!
//! All traversals are iterative (explicit stacks/queues over `BTreeMap`/
//! `BTreeSet`), never recursive, so adversarial graphs (deep chains, dense
//! cycles) cannot overflow the stack. Ties are broken by sorted issue id, so
//! wave ordering and cycle extraction are deterministic across runs.

use crate::graph::metadata::{ConcurrencyMetadata, DependencyReason, Ownership};
use crate::graph::order::{GraphError, GraphErrorKind};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// A proposed set of issues with their hard dependency edges (§28).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IssueGraph {
    pub issues: Vec<PlannedIssue>,
    pub hard_edges: Vec<DependencyEdge>,
}

/// One planned issue and its declared metadata (§28).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlannedIssue {
    pub id: String,
    pub title: String,
    pub ownership: Ownership,
    pub concurrency: ConcurrencyMetadata,
}

/// One hard dependency: `predecessor` must complete before `successor` may
/// start (§28). `reason` and `artifact` carry the dependency justification
/// required by §16; a `None` artifact means the edge is typed but not
/// artifact-backed, which lowers the justification component of the
/// parallelization score.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DependencyEdge {
    pub predecessor: String,
    pub successor: String,
    pub reason: DependencyReason,
    pub artifact: Option<String>,
}

impl IssueGraph {
    /// Number of issues with zero unresolved hard dependencies (§20.1).
    pub fn root_count(&self) -> usize {
        self.topology()
            .predecessors
            .values()
            .filter(|preds| preds.is_empty())
            .count()
    }

    /// Number of issues no other issue depends on.
    pub fn leaf_count(&self) -> usize {
        self.topology()
            .successors
            .values()
            .filter(|succs| succs.is_empty())
            .count()
    }

    /// Detect a dependency cycle (§29.1) and return the node list of one
    /// cycle (each node exactly once, in dependency order) if any exists.
    ///
    /// Method: Kahn peel by indegree residue — every node removed by the
    /// peel is provably acyclic; if residue remains, an iterative DFS over
    /// the residue (starting from the smallest id, walking successors in
    /// sorted order) extracts one concrete cycle.
    pub fn detect_cycle(&self) -> Option<Vec<String>> {
        let topology = self.topology();
        let succs = &topology.successors;
        let preds = &topology.predecessors;
        let mut indegree: BTreeMap<&str, usize> =
            preds.iter().map(|(id, preds)| (*id, preds.len())).collect();

        let mut queue: VecDeque<&str> = indegree
            .iter()
            .filter(|(_, degree)| **degree == 0)
            .map(|(id, _)| *id)
            .collect();
        while let Some(id) = queue.pop_front() {
            for successor in succs.get(id).into_iter().flatten() {
                let degree = indegree
                    .get_mut(successor)
                    .expect("edges are filtered to known ids");
                *degree -= 1;
                if *degree == 0 {
                    queue.push_back(successor);
                }
            }
        }

        let residual: BTreeSet<&str> = indegree
            .iter()
            .filter(|(_, degree)| **degree > 0)
            .map(|(id, _)| *id)
            .collect();
        let &start = residual.iter().next()?;

        // Iterative DFS confined to the residue: every residual node has
        // indegree >= 1 inside the residue, so a cycle is always reachable
        // from the start node.
        const UNSEEN: u8 = 0;
        const VISITING: u8 = 1;
        const DONE: u8 = 2;
        let mut state: BTreeMap<&str, u8> = BTreeMap::new();
        state.insert(start, VISITING);
        let mut stack: Vec<&str> = vec![start];

        loop {
            let top = *stack.last().expect("stack is never empty in the loop");
            let mut back_edge: Option<&str> = None;
            let mut next: Option<&str> = None;
            for successor in succs.get(top).into_iter().flatten() {
                if !residual.contains(successor) {
                    continue;
                }
                match state.get(successor).copied().unwrap_or(UNSEEN) {
                    VISITING => {
                        back_edge = Some(successor);
                        break;
                    }
                    UNSEEN => {
                        next = Some(successor);
                        break;
                    }
                    DONE => {}
                    _ => {}
                }
            }
            if let Some(successor) = back_edge {
                // Back edge: the cycle is the stack suffix from `successor`.
                let position = stack
                    .iter()
                    .position(|node| *node == successor)
                    .expect("visiting node is on the stack");
                return Some(
                    stack[position..]
                        .iter()
                        .map(|node| (*node).to_string())
                        .collect(),
                );
            }
            if let Some(successor) = next {
                state.insert(successor, VISITING);
                stack.push(successor);
            } else {
                state.insert(top, DONE);
                stack.pop();
            }
        }
    }

    /// Kahn execution waves (§24, §29.2): wave 0 is the zero-indegree set,
    /// remove it, wave 1 is the new zero-indegree set, and so on. Each wave
    /// is sorted by issue id, so the projection is deterministic.
    ///
    /// Fails with [`GraphErrorKind::Cycle`] if the graph is cyclic; edges
    /// referencing unknown ids are ignored.
    pub fn waves(&self) -> Result<Vec<Vec<String>>, GraphError> {
        let topology = self.topology();
        let succs = &topology.successors;
        let preds = &topology.predecessors;
        let mut indegree: BTreeMap<&str, usize> =
            preds.iter().map(|(id, preds)| (*id, preds.len())).collect();

        let mut current: BTreeSet<&str> = indegree
            .iter()
            .filter(|(_, degree)| **degree == 0)
            .map(|(id, _)| *id)
            .collect();
        let mut waves: Vec<Vec<String>> = Vec::new();
        while !current.is_empty() {
            waves.push(current.iter().map(|id| (*id).to_string()).collect());
            let mut next: BTreeSet<&str> = BTreeSet::new();
            for id in &current {
                for successor in succs.get(id).into_iter().flatten() {
                    let degree = indegree
                        .get_mut(successor)
                        .expect("edges are filtered to known ids");
                    *degree -= 1;
                    if *degree == 0 {
                        next.insert(successor);
                    }
                }
            }
            current = next;
        }

        if waves.iter().map(Vec::len).sum::<usize>() != self.issues.len() {
            let cycle = self.detect_cycle().unwrap_or_default();
            return Err(GraphError {
                kind: GraphErrorKind::Cycle,
                spec_id: cycle.first().cloned(),
                dependency: None,
                message: format!("dependency cycle detected: {}", cycle.join(" -> ")),
                cycle,
            });
        }
        Ok(waves)
    }

    /// Longest hard-dependency chain, counting nodes (§20.3, §29.3):
    /// `distance[node] = 1 + max(distance[pred])`, maximum over all nodes.
    /// Zero for an empty graph and for cyclic graphs (no topological order).
    pub fn critical_path_length(&self) -> usize {
        let Ok(waves) = self.waves() else {
            return 0;
        };
        let topology = self.topology();
        let preds = &topology.predecessors;
        let mut distance: BTreeMap<&str, usize> = BTreeMap::new();
        let mut longest = 0;
        for wave in &waves {
            for node in wave {
                let preds = preds
                    .get(node.as_str())
                    .expect("every wave node is a known issue");
                let d = preds
                    .iter()
                    .map(|pred| distance.get(*pred).copied().unwrap_or(0))
                    .max()
                    .unwrap_or(0)
                    + 1;
                longest = longest.max(d);
                distance.insert(node.as_str(), d);
            }
        }
        longest
    }

    /// Adjacency over known ids only.
    fn topology(&self) -> Topology<'_> {
        let known: BTreeSet<&str> = self.issues.iter().map(|issue| issue.id.as_str()).collect();
        let mut successors: BTreeMap<&str, BTreeSet<&str>> =
            known.iter().map(|id| (*id, BTreeSet::new())).collect();
        let mut predecessors: BTreeMap<&str, Vec<&str>> =
            known.iter().map(|id| (*id, Vec::new())).collect();

        for edge in &self.hard_edges {
            let Some(predecessor) = known.get(edge.predecessor.as_str()) else {
                continue;
            };
            let Some(successor) = known.get(edge.successor.as_str()) else {
                continue;
            };
            if successors
                .get_mut(*predecessor)
                .expect("known id was inserted")
                .insert(*successor)
            {
                predecessors
                    .get_mut(*successor)
                    .expect("known id was inserted")
                    .push(*predecessor);
            }
        }
        for preds in predecessors.values_mut() {
            preds.sort_unstable();
        }
        Topology {
            successors,
            predecessors,
        }
    }
}

/// Sorted successor and predecessor adjacency for one graph, borrowed from
/// it. `successors` maps node -> sorted successor ids; `predecessors` maps
/// node -> sorted predecessor ids. Edges referencing unknown ids are dropped.
struct Topology<'a> {
    successors: BTreeMap<&'a str, BTreeSet<&'a str>>,
    predecessors: BTreeMap<&'a str, Vec<&'a str>>,
}
