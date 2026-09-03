use serde::Serialize;

use super::error::{CodeIntelError, CodeIntelErrorKind};

/// The stable AutoSpec-owned operation surface exposed to Pi agents.
///
/// Agents never receive raw LSP JSON-RPC; they name one of these operations and
/// the gateway normalizes whatever the backend returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Operation {
    FindSymbol,
    Definition,
    References,
    Implementations,
    Hover,
    Callers,
    Callees,
    TypeHierarchy,
    Diagnostics,
    Impact,
}

const OPERATIONS: &[(Operation, &str)] = &[
    (Operation::FindSymbol, "code.find_symbol"),
    (Operation::Definition, "code.definition"),
    (Operation::References, "code.references"),
    (Operation::Implementations, "code.implementations"),
    (Operation::Hover, "code.hover"),
    (Operation::Callers, "code.callers"),
    (Operation::Callees, "code.callees"),
    (Operation::TypeHierarchy, "code.type_hierarchy"),
    (Operation::Diagnostics, "code.diagnostics"),
    (Operation::Impact, "code.impact"),
];

impl Operation {
    pub fn as_api_name(self) -> &'static str {
        OPERATIONS
            .iter()
            .find(|(operation, _)| *operation == self)
            .map(|(_, name)| *name)
            .unwrap_or("code.unknown")
    }

    pub fn parse(name: &str) -> Result<Self, CodeIntelError> {
        OPERATIONS
            .iter()
            .find(|(_, api_name)| *api_name == name)
            .map(|(operation, _)| *operation)
            .ok_or_else(|| {
                CodeIntelError::new(
                    CodeIntelErrorKind::Unsupported,
                    format!("unknown code intelligence operation: {name}"),
                )
            })
    }

    pub fn all() -> Vec<Self> {
        OPERATIONS.iter().map(|(operation, _)| *operation).collect()
    }

    /// Operations a structural matcher (ast-grep) can approximate.
    ///
    /// Hover, type hierarchy, and diagnostics need a type checker: no structural
    /// or textual matcher can synthesize them, so they fail closed instead.
    pub fn has_structural_fallback(self) -> bool {
        matches!(
            self,
            Self::FindSymbol
                | Self::Definition
                | Self::References
                | Self::Implementations
                | Self::Callers
                | Self::Callees
                | Self::Impact
        )
    }

    /// Operations a textual matcher (ripgrep) can approximate.
    pub fn has_textual_fallback(self) -> bool {
        matches!(
            self,
            Self::FindSymbol | Self::References | Self::Definition | Self::Impact
        )
    }
}

/// Where a result came from, ordered best-first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ResultSource {
    Lsp,
    AstGrep,
    Ripgrep,
}

impl ResultSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lsp => "lsp",
            Self::AstGrep => "ast-grep",
            Self::Ripgrep => "ripgrep",
        }
    }

    /// The confidence a result inherits from its source. Agents are told this so
    /// they never treat a textual guess as a semantic fact.
    pub fn confidence(self) -> Confidence {
        match self {
            Self::Lsp => Confidence::Semantic,
            Self::AstGrep => Confidence::Structural,
            Self::Ripgrep => Confidence::Textual,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Confidence {
    Semantic,
    Structural,
    Textual,
}

impl Confidence {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Semantic => "semantic",
            Self::Structural => "structural",
            Self::Textual => "textual",
        }
    }
}

/// Attached to every result so a reader can tell which worktree, repository and
/// revision produced it, and how much to trust it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Provenance {
    pub workspace: String,
    pub repository: String,
    pub revision: String,
    pub backend: String,
    pub source: String,
    pub confidence: String,
}

impl Provenance {
    pub fn new(
        workspace: impl Into<String>,
        repository: impl Into<String>,
        revision: impl Into<String>,
        backend: impl Into<String>,
        source: ResultSource,
    ) -> Self {
        Self {
            workspace: workspace.into(),
            repository: repository.into(),
            revision: revision.into(),
            backend: backend.into(),
            source: source.as_str().to_string(),
            confidence: source.confidence().as_str().to_string(),
        }
    }

    pub fn is_semantic(&self) -> bool {
        self.confidence == Confidence::Semantic.as_str()
    }
}

/// A zero-based line/character position, matching LSP coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Position {
    pub line: u32,
    pub character: u32,
}

impl Position {
    pub fn new(line: u32, character: u32) -> Self {
        Self { line, character }
    }
}

/// A file range, always expressed as a worktree-relative path so results never
/// leak absolute host paths into a model prompt.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Location {
    pub path: String,
    pub start: Position,
    pub end: Position,
}

impl Location {
    pub fn new(path: impl Into<String>, start: Position, end: Position) -> Self {
        Self {
            path: path.into(),
            start,
            end,
        }
    }

    pub fn point(path: impl Into<String>, line: u32, character: u32) -> Self {
        let position = Position::new(line, character);
        Self::new(path, position, position)
    }
}

/// A normalized symbol. `kind` is the LSP symbol-kind name lowercased
/// (`function`, `struct`, `method`, ...) so agents can filter without knowing
/// numeric LSP constants.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Symbol {
    pub name: String,
    pub kind: String,
    pub container: Option<String>,
    pub location: Location,
    pub detail: Option<String>,
}

impl Symbol {
    pub fn new(name: impl Into<String>, kind: impl Into<String>, location: Location) -> Self {
        Self {
            name: name.into(),
            kind: kind.into(),
            container: None,
            location,
            detail: None,
        }
    }

    pub fn with_container(mut self, container: impl Into<String>) -> Self {
        self.container = Some(container.into());
        self
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// Stable identity used for cross-query set operations (impact comparison,
    /// changed-symbol summaries). Two symbols with the same qualified name and
    /// file are the same symbol even if their line numbers shifted.
    pub fn identity(&self) -> String {
        match &self.container {
            Some(container) => format!("{}::{}@{}", container, self.name, self.location.path),
            None => format!("{}@{}", self.name, self.location.path),
        }
    }
}

/// A single reference to a symbol, tagged with whether it writes to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Reference {
    pub location: Location,
    pub is_definition: bool,
    pub is_write: bool,
}

impl Reference {
    pub fn read(location: Location) -> Self {
        Self {
            location,
            is_definition: false,
            is_write: false,
        }
    }

    pub fn definition(location: Location) -> Self {
        Self {
            location,
            is_definition: true,
            is_write: false,
        }
    }

    pub fn write(location: Location) -> Self {
        Self {
            location,
            is_definition: false,
            is_write: true,
        }
    }
}

/// The envelope every gateway operation returns: a payload plus the provenance
/// that says how far to trust it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QueryResult<T> {
    pub operation: String,
    pub provenance: Provenance,
    pub degraded: bool,
    pub payload: T,
}

impl<T> QueryResult<T> {
    pub fn new(operation: Operation, provenance: Provenance, payload: T) -> Self {
        let degraded = !provenance.is_semantic();
        Self {
            operation: operation.as_api_name().to_string(),
            provenance,
            degraded,
            payload,
        }
    }
}

impl<T: Serialize> QueryResult<T> {
    pub fn to_json_string(&self) -> Result<String, CodeIntelError> {
        serde_json::to_string(self).map_err(|error| CodeIntelError::backend(error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_operation_round_trips_through_its_api_name() {
        for operation in Operation::all() {
            let name = operation.as_api_name();

            assert!(name.starts_with("code."), "{name} is not namespaced");
            assert_eq!(Operation::parse(name).unwrap(), operation);
        }
    }

    #[test]
    fn unknown_operation_names_are_rejected_as_unsupported() {
        let error = Operation::parse("code.rename").unwrap_err();

        assert_eq!(error.kind(), CodeIntelErrorKind::Unsupported);
    }

    #[test]
    fn type_dependent_operations_have_no_lower_confidence_fallback() {
        for operation in [
            Operation::Hover,
            Operation::TypeHierarchy,
            Operation::Diagnostics,
        ] {
            assert!(!operation.has_structural_fallback());
            assert!(!operation.has_textual_fallback());
        }
    }

    #[test]
    fn source_pins_confidence() {
        assert_eq!(ResultSource::Lsp.confidence(), Confidence::Semantic);
        assert_eq!(ResultSource::AstGrep.confidence(), Confidence::Structural);
        assert_eq!(ResultSource::Ripgrep.confidence(), Confidence::Textual);
    }

    #[test]
    fn results_from_a_fallback_source_are_marked_degraded() {
        let provenance = Provenance::new(
            "issue-421",
            "inferweave-gateway",
            "abc123",
            "agent-lsp",
            ResultSource::Ripgrep,
        );

        let result = QueryResult::new(Operation::References, provenance, Vec::<Reference>::new());

        assert!(result.degraded);
        assert_eq!(result.provenance.confidence, "textual");
    }

    #[test]
    fn semantic_results_are_not_degraded() {
        let provenance = Provenance::new(
            "issue-421",
            "inferweave-gateway",
            "abc123",
            "agent-lsp",
            ResultSource::Lsp,
        );

        let result = QueryResult::new(Operation::Definition, provenance, Vec::<Symbol>::new());

        assert!(!result.degraded);
        assert!(result.provenance.is_semantic());
    }

    #[test]
    fn symbol_identity_includes_container_and_file() {
        let symbol = Symbol::new(
            "resolve",
            "method",
            Location::point("src/gateway.rs", 10, 4),
        )
        .with_container("Gateway");

        assert_eq!(symbol.identity(), "Gateway::resolve@src/gateway.rs");
    }

    #[test]
    fn symbol_identity_survives_line_movement() {
        let before = Symbol::new(
            "resolve",
            "function",
            Location::point("src/gateway.rs", 10, 4),
        );
        let after = Symbol::new(
            "resolve",
            "function",
            Location::point("src/gateway.rs", 42, 4),
        );

        assert_eq!(before.identity(), after.identity());
    }

    #[test]
    fn query_results_serialize_with_provenance() {
        let provenance = Provenance::new(
            "issue-421",
            "inferweave-gateway",
            "abc123",
            "agent-lsp",
            ResultSource::Lsp,
        );
        let result = QueryResult::new(
            Operation::Definition,
            provenance,
            vec![Symbol::new(
                "Gateway",
                "struct",
                Location::point("src/gateway.rs", 3, 0),
            )],
        );

        let json = result.to_json_string().unwrap();

        assert!(json.contains("\"operation\":\"code.definition\""));
        assert!(json.contains("\"workspace\":\"issue-421\""));
        assert!(json.contains("\"confidence\":\"semantic\""));
    }
}
