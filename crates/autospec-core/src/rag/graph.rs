//! Graph RAG: the revision-aware knowledge graph (spec sections 14 and 15).
//!
//! Traversal is breadth-first with a configurable depth and edge filter, and
//! returns the path it took. The path is what makes a graph answer auditable:
//! "GatewayRouter reaches SchedulerFairnessTest" is a claim, and section 54
//! requires AutoSpec to be able to say *how*.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// What a graph node represents (spec section 14).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NodeKind {
    /// A repository.
    Repository,
    /// A file.
    File,
    /// A module or package.
    Module,
    /// A named symbol: function, type, or interface.
    Symbol,
    /// A test.
    Test,
    /// An HTTP or library API surface.
    Api,
    /// A GitHub issue.
    Issue,
    /// A pull request.
    PullRequest,
    /// A specification document.
    Specification,
    /// An architectural decision record.
    Adr,
    /// A deployed runtime service.
    Service,
    /// A model.
    Model,
    /// A dashboard surface.
    Dashboard,
}

impl NodeKind {
    /// Stable wire identifier.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Repository => "repository",
            Self::File => "file",
            Self::Module => "module",
            Self::Symbol => "symbol",
            Self::Test => "test",
            Self::Api => "api",
            Self::Issue => "issue",
            Self::PullRequest => "pull_request",
            Self::Specification => "specification",
            Self::Adr => "adr",
            Self::Service => "service",
            Self::Model => "model",
            Self::Dashboard => "dashboard",
        }
    }
}

/// A typed edge label (spec section 14).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Relation {
    /// Implements an interface.
    Implements,
    /// Calls a symbol.
    Calls,
    /// Imports a module.
    Imports,
    /// Depends on a component.
    DependsOn,
    /// Returns a type.
    Returns,
    /// Accepts a type.
    Accepts,
    /// Covered by a test.
    TestedBy,
    /// Described by documentation.
    DocumentedBy,
    /// Governed by a specification.
    SpecifiedBy,
    /// Supersedes an earlier decision.
    Supersedes,
    /// Configured by a settings surface.
    ConfiguredBy,
    /// Owned by a team.
    OwnedBy,
    /// Exposed by an API surface.
    ExposedBy,
    /// Consumed by a client.
    ConsumedBy,
    /// Deployed alongside a service.
    DeployedWith,
    /// Observed by telemetry.
    ObservedBy,
    /// Modified by a change.
    ModifiedBy,
}

/// Every relation, in a stable order.
pub const ALL_RELATIONS: [Relation; 17] = [
    Relation::Implements,
    Relation::Calls,
    Relation::Imports,
    Relation::DependsOn,
    Relation::Returns,
    Relation::Accepts,
    Relation::TestedBy,
    Relation::DocumentedBy,
    Relation::SpecifiedBy,
    Relation::Supersedes,
    Relation::ConfiguredBy,
    Relation::OwnedBy,
    Relation::ExposedBy,
    Relation::ConsumedBy,
    Relation::DeployedWith,
    Relation::ObservedBy,
    Relation::ModifiedBy,
];

impl Relation {
    /// Stable wire identifier.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Implements => "implements",
            Self::Calls => "calls",
            Self::Imports => "imports",
            Self::DependsOn => "depends_on",
            Self::Returns => "returns",
            Self::Accepts => "accepts",
            Self::TestedBy => "tested_by",
            Self::DocumentedBy => "documented_by",
            Self::SpecifiedBy => "specified_by",
            Self::Supersedes => "supersedes",
            Self::ConfiguredBy => "configured_by",
            Self::OwnedBy => "owned_by",
            Self::ExposedBy => "exposed_by",
            Self::ConsumedBy => "consumed_by",
            Self::DeployedWith => "deployed_with",
            Self::ObservedBy => "observed_by",
            Self::ModifiedBy => "modified_by",
        }
    }

    /// Parse a wire identifier.
    pub fn parse(text: &str) -> Result<Self, String> {
        ALL_RELATIONS
            .iter()
            .copied()
            .find(|relation| relation.as_str() == text)
            .ok_or_else(|| format!("unknown relation: {text}"))
    }
}

/// A node in the knowledge graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphNode {
    /// Stable identifier, unique within the graph.
    pub id: String,
    /// What the node represents.
    pub kind: NodeKind,
    /// Source URI, when the node has one.
    pub uri: Option<String>,
}

impl GraphNode {
    /// Build a node.
    pub fn new(id: impl Into<String>, kind: NodeKind) -> Self {
        Self {
            id: id.into(),
            kind,
            uri: None,
        }
    }

    /// Attach a source URI.
    pub fn with_uri(mut self, uri: impl Into<String>) -> Self {
        self.uri = Some(uri.into());
        self
    }
}

/// One hop taken during a traversal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraversalStep {
    /// Node the hop started from.
    pub from: String,
    /// Edge followed.
    pub relation: Relation,
    /// Node the hop reached.
    pub to: String,
}

/// A node reached by a traversal, with the path that reached it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReachedNode {
    /// The node.
    pub node: GraphNode,
    /// Hops from the traversal origin.
    pub depth: u32,
    /// The path taken, origin first.
    pub path: Vec<TraversalStep>,
}

/// A revision-aware knowledge graph over one source state.
///
/// The graph carries the revision it was built from so a traversal result can
/// be cached and invalidated with the same key discipline as evidence
/// (section 25).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnowledgeGraph {
    revision: String,
    nodes: BTreeMap<String, GraphNode>,
    edges: BTreeMap<String, Vec<(Relation, String)>>,
}

impl KnowledgeGraph {
    /// An empty graph at a revision.
    pub fn new(revision: impl Into<String>) -> Self {
        Self {
            revision: revision.into(),
            nodes: BTreeMap::new(),
            edges: BTreeMap::new(),
        }
    }

    /// The revision this graph indexes.
    pub fn revision(&self) -> &str {
        &self.revision
    }

    /// Insert or replace a node.
    pub fn add_node(&mut self, node: GraphNode) {
        self.nodes.insert(node.id.clone(), node);
    }

    /// Add a directed edge between two existing nodes.
    ///
    /// Both endpoints must already exist. A dangling edge is the graph-level
    /// form of a hallucinated reference (section 41.2), and rejecting it here
    /// means a traversal can never return a node that was never indexed.
    pub fn add_edge(&mut self, from: &str, relation: Relation, to: &str) -> Result<(), String> {
        if !self.nodes.contains_key(from) {
            return Err(format!("unknown edge source node: {from}"));
        }
        if !self.nodes.contains_key(to) {
            return Err(format!("unknown edge target node: {to}"));
        }
        let outgoing = self.edges.entry(from.to_string()).or_default();
        let edge = (relation, to.to_string());
        if !outgoing.contains(&edge) {
            outgoing.push(edge);
            outgoing.sort();
        }
        Ok(())
    }

    /// Look up a node.
    pub fn node(&self, id: &str) -> Option<&GraphNode> {
        self.nodes.get(id)
    }

    /// Number of indexed nodes.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Number of indexed edges.
    pub fn edge_count(&self) -> usize {
        self.edges.values().map(Vec::len).sum()
    }

    /// Immediate neighbours reached by one relation.
    pub fn neighbors(&self, id: &str, relation: Relation) -> Vec<&GraphNode> {
        self.edges
            .get(id)
            .map(|edges| {
                edges
                    .iter()
                    .filter(|(candidate, _)| *candidate == relation)
                    .filter_map(|(_, target)| self.nodes.get(target))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Breadth-first traversal from `origin`.
    ///
    /// `relations` filters which edges may be followed; an empty filter follows
    /// every edge. `max_depth` is a hop count, and `0` returns nothing but the
    /// origin's own existence check.
    pub fn traverse(
        &self,
        origin: &str,
        relations: &[Relation],
        max_depth: u32,
    ) -> Result<Vec<ReachedNode>, String> {
        if !self.nodes.contains_key(origin) {
            return Err(format!("unknown traversal origin: {origin}"));
        }
        let allowed: BTreeSet<Relation> = relations.iter().copied().collect();
        let mut visited = BTreeSet::new();
        visited.insert(origin.to_string());
        let mut queue = VecDeque::new();
        queue.push_back((origin.to_string(), 0_u32, Vec::<TraversalStep>::new()));
        let mut reached = Vec::new();

        while let Some((current, depth, path)) = queue.pop_front() {
            if depth >= max_depth {
                continue;
            }
            let Some(edges) = self.edges.get(&current) else {
                continue;
            };
            for (relation, target) in edges {
                if !allowed.is_empty() && !allowed.contains(relation) {
                    continue;
                }
                if !visited.insert(target.clone()) {
                    continue;
                }
                let mut next_path = path.clone();
                next_path.push(TraversalStep {
                    from: current.clone(),
                    relation: *relation,
                    to: target.clone(),
                });
                let node = self
                    .nodes
                    .get(target)
                    .expect("edge targets are validated on insert")
                    .clone();
                reached.push(ReachedNode {
                    node,
                    depth: depth + 1,
                    path: next_path.clone(),
                });
                queue.push_back((target.clone(), depth + 1, next_path));
            }
        }
        Ok(reached)
    }

    /// Return `true` when `id` names an indexed node.
    ///
    /// The existence check section 41.2 prescribes: a planner can confirm a
    /// symbol is real before spending a query on it.
    pub fn exists(&self, id: &str) -> bool {
        self.nodes.contains_key(id)
    }
}
