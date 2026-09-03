use serde_json::Value;

use super::super::diagnostics::{Diagnostic, Severity};
use super::super::error::CodeIntelError;
use super::super::impact::ImpactSet;
use super::super::schema::{Operation, Provenance, Reference, ResultSource, Symbol};
use super::json::{
    code_of, entries, flag, group, kind_of, location_from, parse_json, required_str, string_group,
};
use super::{BackendRequest, CodeIntelBackend};

/// The upstream release this adapter is written against.
///
/// agent-lsp's tool names are an upstream contract, not a standard. Pinning the
/// commit here and keeping every name in `TOOL_NAMES` means an upgrade is one
/// table to revalidate, not a search across the codebase.
pub const PINNED_VERSION: &str = "0.1.0";

/// AutoSpec operation -> agent-lsp tool name.
///
/// This table is the ONLY place upstream tool names appear. Swapping in
/// multilspy or lsproxy means writing a sibling adapter, not editing callers.
const TOOL_NAMES: &[(Operation, &str)] = &[
    (Operation::FindSymbol, "find_symbol"),
    (Operation::Definition, "goto_definition"),
    (Operation::References, "find_references"),
    (Operation::Implementations, "find_implementations"),
    (Operation::Hover, "hover"),
    (Operation::Callers, "incoming_calls"),
    (Operation::Callees, "outgoing_calls"),
    (Operation::TypeHierarchy, "type_hierarchy"),
    (Operation::Diagnostics, "diagnostics"),
    (Operation::Impact, "blast_radius"),
];

/// Translates between the AutoSpec API and one agent-lsp process.
///
/// The adapter holds no process handle: it is a pure translation layer, so the
/// normalization it performs is testable against recorded payloads without a
/// language server on the machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentLspAdapter {
    version: String,
    capabilities: Vec<Operation>,
}

impl Default for AgentLspAdapter {
    fn default() -> Self {
        Self::new(PINNED_VERSION, Operation::all())
    }
}

impl AgentLspAdapter {
    pub fn new(version: impl Into<String>, capabilities: Vec<Operation>) -> Self {
        Self {
            version: version.into(),
            capabilities,
        }
    }

    /// The upstream tool name for an operation.
    pub fn tool_name(operation: Operation) -> Result<&'static str, CodeIntelError> {
        TOOL_NAMES
            .iter()
            .find(|(candidate, _)| *candidate == operation)
            .map(|(_, name)| *name)
            .ok_or_else(|| {
                CodeIntelError::unsupported(format!(
                    "agent-lsp has no tool for {}",
                    operation.as_api_name()
                ))
            })
    }

    pub fn provenance(&self, request: &BackendRequest, repository: &str) -> Provenance {
        Provenance::new(
            request.workspace.clone(),
            repository,
            request.revision.clone(),
            self.name(),
            ResultSource::Lsp,
        )
    }

    /// Normalize a symbol-shaped payload (definitions, implementations, call
    /// hierarchy, workspace symbols).
    pub fn normalize_symbols(payload: &str) -> Result<Vec<Symbol>, CodeIntelError> {
        entries(payload)?.iter().map(symbol_from).collect()
    }

    pub fn normalize_references(payload: &str) -> Result<Vec<Reference>, CodeIntelError> {
        entries(payload)?.iter().map(reference_from).collect()
    }

    pub fn normalize_diagnostics(payload: &str) -> Result<Vec<Diagnostic>, CodeIntelError> {
        entries(payload)?.iter().map(diagnostic_from).collect()
    }

    /// Normalize agent-lsp's blast-radius payload into an `ImpactSet`.
    ///
    /// Missing groups normalize to empty rather than failing: an upstream that
    /// omits `exports` for a language should degrade that one group, not the
    /// whole analysis.
    pub fn normalize_impact(
        payload: &str,
        provenance: Provenance,
        target: &str,
    ) -> Result<ImpactSet, CodeIntelError> {
        let value = parse_json(payload)?;
        let mut impact = ImpactSet::new(provenance, target);
        impact.definitions = symbol_group(&value, "definitions")?;
        impact.callers = symbol_group(&value, "callers")?;
        impact.callees = symbol_group(&value, "callees")?;
        impact.implementations = symbol_group(&value, "implementations")?;
        impact.exports = symbol_group(&value, "exports")?;
        impact.references = reference_group(&value)?;
        impact.related_tests = string_group(&value, "related_tests");
        impact.dependent_modules = string_group(&value, "dependent_modules");
        impact.diagnostics = diagnostic_group(&value)?;
        Ok(impact)
    }
}

impl CodeIntelBackend for AgentLspAdapter {
    fn name(&self) -> &str {
        "agent-lsp"
    }

    fn version(&self) -> &str {
        &self.version
    }

    fn capabilities(&self) -> Vec<Operation> {
        self.capabilities.clone()
    }

    /// The adapter does not own a transport; a supervisor injects one. Calling
    /// it directly is a programming error, reported rather than silently
    /// returning an empty result that a caller would read as "no references".
    fn execute(&self, request: &BackendRequest) -> Result<String, CodeIntelError> {
        Err(CodeIntelError::backend(format!(
            "agent-lsp adapter has no transport bound for {}",
            request.operation
        )))
    }
}

fn symbol_group(value: &Value, key: &str) -> Result<Vec<Symbol>, CodeIntelError> {
    group(value, key)?.iter().map(symbol_from).collect()
}

fn diagnostic_group(value: &Value) -> Result<Vec<Diagnostic>, CodeIntelError> {
    group(value, "diagnostics")?
        .iter()
        .map(diagnostic_from)
        .collect()
}

fn reference_group(value: &Value) -> Result<Vec<Reference>, CodeIntelError> {
    group(value, "references")?
        .iter()
        .map(reference_from)
        .collect()
}

fn symbol_from(value: &Value) -> Result<Symbol, CodeIntelError> {
    let mut symbol = Symbol::new(
        required_str(value, "name")?,
        kind_of(value),
        location_from(value)?,
    );
    if let Some(container) = value.get("container").and_then(Value::as_str) {
        symbol = symbol.with_container(container);
    }
    if let Some(detail) = value.get("detail").and_then(Value::as_str) {
        symbol = symbol.with_detail(detail);
    }
    Ok(symbol)
}

fn reference_from(value: &Value) -> Result<Reference, CodeIntelError> {
    let location = location_from(value)?;
    Ok(Reference {
        location,
        is_definition: flag(value, "is_definition"),
        is_write: flag(value, "is_write"),
    })
}

fn diagnostic_from(value: &Value) -> Result<Diagnostic, CodeIntelError> {
    let severity = value
        .get("severity")
        .and_then(Value::as_u64)
        .map(Severity::from_lsp_code)
        .unwrap_or(Severity::Error);
    let mut diagnostic = Diagnostic::new(
        location_from(value)?,
        severity,
        required_str(value, "message")?,
    );
    if let Some(code) = code_of(value) {
        diagnostic = diagnostic.with_code(code);
    }
    if let Some(source) = value.get("source").and_then(Value::as_str) {
        diagnostic = diagnostic.with_source(source);
    }
    Ok(diagnostic)
}

#[cfg(test)]
mod tests {
    use super::super::super::schema::Position;
    use super::*;

    fn provenance() -> Provenance {
        Provenance::new(
            "issue-421",
            "autospec",
            "abc123",
            "agent-lsp",
            ResultSource::Lsp,
        )
    }

    #[test]
    fn every_operation_maps_to_exactly_one_upstream_tool() {
        let mut names = Vec::new();
        for operation in Operation::all() {
            let name = AgentLspAdapter::tool_name(operation).unwrap();

            assert!(!name.is_empty());
            names.push(name);
        }
        names.sort_unstable();
        let unique = names.len();
        names.dedup();

        assert_eq!(names.len(), unique, "upstream tool names must be distinct");
    }

    #[test]
    fn impact_maps_to_the_upstream_blast_radius_tool() {
        assert_eq!(
            AgentLspAdapter::tool_name(Operation::Impact).unwrap(),
            "blast_radius"
        );
    }

    #[test]
    fn symbols_normalize_with_container_and_range() {
        let payload = r#"[{
            "name": "resolve",
            "kind": "Method",
            "container": "Gateway",
            "path": "src/gateway.rs",
            "range": {"start": {"line": 10, "character": 4}, "end": {"line": 18, "character": 5}}
        }]"#;

        let symbols = AgentLspAdapter::normalize_symbols(payload).unwrap();

        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].kind, "method");
        assert_eq!(symbols[0].identity(), "Gateway::resolve@src/gateway.rs");
        assert_eq!(symbols[0].location.end.line, 18);
    }

    #[test]
    fn a_missing_range_normalizes_to_the_file_start() {
        let payload = r#"[{"name": "Gateway", "kind": "Struct", "path": "src/gateway.rs"}]"#;

        let symbols = AgentLspAdapter::normalize_symbols(payload).unwrap();

        assert_eq!(symbols[0].location.start, Position::new(0, 0));
    }

    #[test]
    fn a_missing_kind_normalizes_to_unknown() {
        let payload = r#"[{"name": "Gateway", "path": "src/gateway.rs"}]"#;

        let symbols = AgentLspAdapter::normalize_symbols(payload).unwrap();

        assert_eq!(symbols[0].kind, "unknown");
    }

    #[test]
    fn a_symbol_without_a_name_is_rejected_not_defaulted() {
        let payload = r#"[{"kind": "Struct", "path": "src/gateway.rs"}]"#;

        let error = AgentLspAdapter::normalize_symbols(payload).unwrap_err();

        assert!(error.message().contains("missing name"));
    }

    #[test]
    fn a_symbol_without_a_path_is_rejected() {
        let payload = r#"[{"name": "Gateway", "kind": "Struct"}]"#;

        assert!(AgentLspAdapter::normalize_symbols(payload).is_err());
    }

    #[test]
    fn malformed_payloads_are_rejected_rather_than_silently_empty() {
        let error = AgentLspAdapter::normalize_symbols("{not json").unwrap_err();

        assert!(error.message().contains("malformed agent-lsp payload"));
    }

    #[test]
    fn a_non_array_payload_is_rejected() {
        let error = AgentLspAdapter::normalize_symbols(r#"{"name":"x"}"#).unwrap_err();

        assert!(error.message().contains("must be a JSON array"));
    }

    #[test]
    fn references_carry_definition_and_write_flags() {
        let payload = r#"[
            {"path": "src/gateway.rs", "is_definition": true},
            {"path": "src/router.rs", "is_write": true},
            {"path": "src/router.rs"}
        ]"#;

        let references = AgentLspAdapter::normalize_references(payload).unwrap();

        assert!(references[0].is_definition);
        assert!(references[1].is_write);
        assert!(!references[2].is_definition && !references[2].is_write);
    }

    #[test]
    fn diagnostics_normalize_numeric_and_string_codes() {
        let payload = r#"[
            {"path": "src/a.rs", "severity": 1, "message": "mismatch", "code": "E0308"},
            {"path": "src/b.rs", "severity": 2, "message": "unused", "code": 1234}
        ]"#;

        let diagnostics = AgentLspAdapter::normalize_diagnostics(payload).unwrap();

        assert_eq!(diagnostics[0].severity, Severity::Error);
        assert_eq!(diagnostics[0].code.as_deref(), Some("E0308"));
        assert_eq!(diagnostics[1].severity, Severity::Warning);
        assert_eq!(diagnostics[1].code.as_deref(), Some("1234"));
    }

    #[test]
    fn a_diagnostic_without_a_severity_is_treated_as_an_error() {
        let payload = r#"[{"path": "src/a.rs", "message": "unresolved import"}]"#;

        let diagnostics = AgentLspAdapter::normalize_diagnostics(payload).unwrap();

        assert!(diagnostics[0].is_error());
    }

    #[test]
    fn blast_radius_normalizes_into_a_full_impact_set() {
        let payload = r#"{
            "definitions": [{"name": "resolve", "kind": "Function", "path": "src/gateway.rs"}],
            "callers": [{"name": "dispatch", "kind": "Function", "path": "src/router.rs"}],
            "references": [{"path": "src/router.rs"}],
            "related_tests": ["tests/gateway_test.rs"],
            "dependent_modules": ["router"],
            "diagnostics": [{"path": "src/gateway.rs", "severity": 1, "message": "mismatch"}]
        }"#;

        let impact =
            AgentLspAdapter::normalize_impact(payload, provenance(), "Gateway::resolve").unwrap();

        assert_eq!(impact.definitions.len(), 1);
        assert_eq!(impact.callers.len(), 1);
        assert_eq!(impact.related_tests, vec!["tests/gateway_test.rs"]);
        assert_eq!(impact.dependent_modules, vec!["router"]);
        assert_eq!(impact.diagnostics.len(), 1);
        assert_eq!(impact.affected_files().len(), 3);
    }

    #[test]
    fn missing_impact_groups_normalize_to_empty() {
        let impact =
            AgentLspAdapter::normalize_impact("{}", provenance(), "Gateway::resolve").unwrap();

        assert!(impact.is_empty());
        assert!(impact.implementations.is_empty());
        assert!(impact.exports.is_empty());
    }

    #[test]
    fn an_impact_group_of_the_wrong_shape_is_rejected() {
        let error = AgentLspAdapter::normalize_impact(r#"{"callers": 3}"#, provenance(), "target")
            .unwrap_err();

        assert!(error.message().contains("must be an array"));
    }

    #[test]
    fn the_adapter_reports_its_pinned_version_and_backend_name() {
        let adapter = AgentLspAdapter::default();

        assert_eq!(adapter.name(), "agent-lsp");
        assert_eq!(adapter.version(), PINNED_VERSION);
        assert_eq!(adapter.capabilities().len(), Operation::all().len());
    }

    #[test]
    fn an_unbound_adapter_reports_a_backend_failure_instead_of_an_empty_result() {
        let request = BackendRequest::new(
            "issue-421",
            "wt/a",
            "abc123",
            Operation::References,
            super::super::SymbolTarget::name("Gateway"),
        );

        let error = AgentLspAdapter::default().execute(&request).unwrap_err();

        assert!(error.message().contains("no transport bound"));
        assert!(error.is_degradable());
    }
}
