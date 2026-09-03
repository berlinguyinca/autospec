pub mod agent_lsp;
mod json;

use serde::Serialize;

use super::error::CodeIntelError;
use super::schema::{Operation, Position};

/// What a query points at: a name to search for, or an exact cursor position.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum SymbolTarget {
    /// A symbol name, optionally qualified by its container.
    Name { name: String },
    /// A worktree-relative path and a zero-based position inside it.
    Position { path: String, position: Position },
    /// The whole workspace — used by `code.diagnostics`.
    Workspace,
}

impl SymbolTarget {
    pub fn name(name: impl Into<String>) -> Self {
        Self::Name { name: name.into() }
    }

    pub fn position(path: impl Into<String>, line: u32, character: u32) -> Self {
        Self::Position {
            path: path.into(),
            position: Position::new(line, character),
        }
    }

    /// Stable identity used to build a cache key for this target.
    pub fn identity(&self) -> String {
        match self {
            Self::Name { name } => format!("name:{name}"),
            Self::Position { path, position } => {
                format!("pos:{}:{}:{}", path, position.line, position.character)
            }
            Self::Workspace => "workspace".to_string(),
        }
    }
}

/// A gateway-owned request. It always names a workspace, so the backend can
/// never be asked a question that is not scoped to exactly one worktree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BackendRequest {
    pub workspace: String,
    pub root: String,
    pub revision: String,
    pub operation: String,
    pub target: SymbolTarget,
    pub timeout_ms: u64,
}

/// Default deadline for a single backend call. Long enough for a cold call
/// hierarchy on a large repository, short enough that a wedged server degrades
/// to a fallback rather than stalling the task.
pub const DEFAULT_TIMEOUT_MS: u64 = 15_000;

impl BackendRequest {
    pub fn new(
        workspace: impl Into<String>,
        root: impl Into<String>,
        revision: impl Into<String>,
        operation: Operation,
        target: SymbolTarget,
    ) -> Self {
        Self {
            workspace: workspace.into(),
            root: root.into(),
            revision: revision.into(),
            operation: operation.as_api_name().to_string(),
            target,
            timeout_ms: DEFAULT_TIMEOUT_MS,
        }
    }

    pub fn with_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = timeout_ms;
        self
    }

    pub fn operation(&self) -> Result<Operation, CodeIntelError> {
        Operation::parse(&self.operation)
    }

    /// Cache identity of this request, independent of workspace and revision —
    /// `SemanticWorkspace::cache_key` supplies those.
    pub fn identity(&self) -> String {
        self.target.identity()
    }

    pub fn to_json_string(&self) -> Result<String, CodeIntelError> {
        serde_json::to_string(self).map_err(|error| CodeIntelError::backend(error.to_string()))
    }
}

/// A pluggable code intelligence backend.
///
/// agent-lsp is the v1 implementation, but the gateway only ever speaks this
/// interface, so multilspy or lsproxy can replace it without touching any
/// caller. Implementations return the backend's own payload; normalization into
/// AutoSpec schemas is the adapter's job.
pub trait CodeIntelBackend {
    /// Identifier recorded in every result's provenance.
    fn name(&self) -> &str;

    /// The pinned backend version, recorded for reproducibility.
    fn version(&self) -> &str;

    /// Operations this backend implements. Anything absent degrades through the
    /// fallback chain rather than erroring at the call site.
    fn capabilities(&self) -> Vec<Operation>;

    /// Run one request, returning the backend's raw payload.
    fn execute(&self, request: &BackendRequest) -> Result<String, CodeIntelError>;

    fn supports(&self, operation: Operation) -> bool {
        self.capabilities().contains(&operation)
    }

    /// Reject an unsupported operation before spending a process round-trip.
    fn ensure_supports(&self, operation: Operation) -> Result<(), CodeIntelError> {
        if self.supports(operation) {
            return Ok(());
        }
        Err(CodeIntelError::unsupported(format!(
            "{} does not implement {}",
            self.name(),
            operation.as_api_name()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct StubBackend {
        capabilities: Vec<Operation>,
    }

    impl CodeIntelBackend for StubBackend {
        fn name(&self) -> &str {
            "stub"
        }

        fn version(&self) -> &str {
            "0.0.0"
        }

        fn capabilities(&self) -> Vec<Operation> {
            self.capabilities.clone()
        }

        fn execute(&self, _request: &BackendRequest) -> Result<String, CodeIntelError> {
            Ok("[]".to_string())
        }
    }

    fn request(operation: Operation, target: SymbolTarget) -> BackendRequest {
        BackendRequest::new("issue-421", "wt/a", "abc123", operation, target)
    }

    #[test]
    fn requests_carry_a_workspace_and_a_default_deadline() {
        let request = request(Operation::Definition, SymbolTarget::name("Gateway"));

        assert_eq!(request.workspace, "issue-421");
        assert_eq!(request.timeout_ms, DEFAULT_TIMEOUT_MS);
        assert_eq!(request.operation().unwrap(), Operation::Definition);
    }

    #[test]
    fn target_identity_distinguishes_names_from_positions() {
        let by_name = SymbolTarget::name("Gateway");
        let by_position = SymbolTarget::position("src/gateway.rs", 10, 4);

        assert_eq!(by_name.identity(), "name:Gateway");
        assert_eq!(by_position.identity(), "pos:src/gateway.rs:10:4");
        assert_ne!(by_name.identity(), by_position.identity());
    }

    #[test]
    fn positions_at_different_columns_are_different_requests() {
        let left = SymbolTarget::position("src/gateway.rs", 10, 4);
        let right = SymbolTarget::position("src/gateway.rs", 10, 9);

        assert_ne!(left.identity(), right.identity());
    }

    #[test]
    fn unsupported_operations_are_rejected_before_a_round_trip() {
        let backend = StubBackend {
            capabilities: vec![Operation::Definition],
        };

        let error = backend
            .ensure_supports(Operation::TypeHierarchy)
            .unwrap_err();

        assert!(error.message().contains("code.type_hierarchy"));
        assert!(error.is_degradable());
    }

    #[test]
    fn supported_operations_pass_the_capability_check() {
        let backend = StubBackend {
            capabilities: vec![Operation::Definition],
        };

        assert!(backend.ensure_supports(Operation::Definition).is_ok());
    }

    #[test]
    fn requests_serialize_for_the_backend_transport() {
        let json = request(Operation::References, SymbolTarget::name("Gateway"))
            .to_json_string()
            .unwrap();

        assert!(json.contains("\"operation\":\"code.references\""));
        assert!(json.contains("\"workspace\":\"issue-421\""));
    }

    #[test]
    fn timeouts_are_overridable_per_request() {
        let request =
            request(Operation::Impact, SymbolTarget::name("Gateway")).with_timeout_ms(60_000);

        assert_eq!(request.timeout_ms, 60_000);
    }
}
