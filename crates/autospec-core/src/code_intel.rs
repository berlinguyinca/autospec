//! Code Intelligence Gateway.
//!
//! AutoSpec owns this gateway; Pi agents call the stable `code.*` surface in
//! [`schema::Operation`] and never receive raw LSP JSON-RPC. A backend adapter
//! ([`backend::agent_lsp`]) translates to whatever the underlying orchestrator
//! speaks, so replacing agent-lsp with multilspy or lsproxy is a sibling
//! adapter rather than a change to any caller.
//!
//! Three invariants hold across the module:
//!
//! 1. **Isolation.** Every query names a workspace, and a workspace resolves to
//!    exactly one worktree root ([`workspace`]). Diagnostics and cache entries
//!    can never cross worktrees.
//! 2. **Provenance.** Every result says where it came from and how far to trust
//!    it ([`schema::Provenance`]), so a textual guess is never read as a
//!    semantic fact.
//! 3. **Fail-visible degradation.** A backend failure degrades through
//!    [`fallback::FallbackChain`] rather than crashing the task, but the
//!    degraded confidence travels with the result and into the gate outcome.

pub mod backend;
pub mod config;
pub mod diagnostics;
pub mod doctor;
pub mod error;
pub mod fallback;
pub mod gate;
pub mod impact;
pub mod language;
pub mod metrics;
pub mod schema;
pub mod workspace;

pub use backend::agent_lsp::AgentLspAdapter;
pub use backend::{BackendRequest, CodeIntelBackend, SymbolTarget};
pub use config::{CodeIntelConfig, CONFIG_PATH};
pub use diagnostics::{Diagnostic, DiagnosticDelta, DiagnosticSet, Severity};
pub use doctor::{DoctorReport, HostProbe};
pub use error::{CodeIntelError, CodeIntelErrorKind};
pub use fallback::FallbackChain;
pub use gate::{implementer_gate, planner_gate, reviewer_gate, GateOutcome, Role};
pub use impact::{ImpactComparison, ImpactSet};
pub use language::{detect, DetectedLanguage, Language};
pub use metrics::{MetricsRegistry, MetricsSnapshot, OperationRecord};
pub use schema::{
    Confidence, Location, Operation, Position, Provenance, QueryResult, Reference, ResultSource,
    Symbol,
};
pub use workspace::{
    workspace_id_for_worktree, SemanticWorkspace, WorkspaceRegistry, WorkspaceState,
};

#[cfg(test)]
mod tests {
    use super::*;

    fn ready_workspace(id: &str, root: &str) -> SemanticWorkspace {
        SemanticWorkspace::new(id, "autospec", root, "abc123")
            .unwrap()
            .with_state(WorkspaceState::Ready)
    }

    fn impact_for(workspace: &SemanticWorkspace, path: &str) -> ImpactSet {
        let provenance = Provenance::new(
            workspace.id.clone(),
            workspace.repository.clone(),
            workspace.revision.clone(),
            "agent-lsp",
            ResultSource::Lsp,
        );
        let mut impact = ImpactSet::new(provenance, "Gateway::resolve");
        impact.definitions.push(Symbol::new(
            "resolve",
            "function",
            Location::point(path, 1, 0),
        ));
        impact
    }

    #[test]
    fn concurrent_worktrees_keep_separate_semantic_state() {
        let mut registry = WorkspaceRegistry::new();
        registry
            .register(ready_workspace(
                "issue-421",
                ".autospec/worktrees/issue-421",
            ))
            .unwrap();
        registry
            .register(ready_workspace(
                "issue-422",
                ".autospec/worktrees/issue-422",
            ))
            .unwrap();

        let first = registry.resolve("issue-421").unwrap();
        let second = registry.resolve("issue-422").unwrap();

        assert_ne!(first.root, second.root);
        assert_ne!(
            first.cache_key("code.impact", "Gateway"),
            second.cache_key("code.impact", "Gateway")
        );
    }

    #[test]
    fn invalidating_one_worktree_leaves_its_neighbour_queryable() {
        let mut registry = WorkspaceRegistry::new();
        registry
            .register(ready_workspace("issue-421", "wt/a"))
            .unwrap();
        registry
            .register(ready_workspace("issue-422", "wt/b"))
            .unwrap();

        registry.invalidate("issue-421").unwrap();

        assert!(registry.resolve("issue-421").is_err());
        assert!(registry.resolve("issue-422").is_ok());
    }

    #[test]
    fn a_backend_failure_degrades_the_result_instead_of_failing_the_task() {
        let config = CodeIntelConfig::default();
        let chain = FallbackChain::resolve(Operation::References, &config.fallback);
        let failure = CodeIntelError::backend("rust-analyzer exited during indexing");

        let (source, step) = chain.degrade(ResultSource::Lsp, &failure, false).unwrap();
        let provenance = Provenance::new("issue-421", "autospec", "abc123", "agent-lsp", source);
        let result = QueryResult::new(Operation::References, provenance, Vec::<Reference>::new());

        assert!(result.degraded);
        assert_eq!(result.provenance.confidence, "structural");
        assert!(step.reason.contains("rust-analyzer exited"));
    }

    #[test]
    fn the_full_planner_to_reviewer_path_gates_on_semantic_evidence() {
        let config = CodeIntelConfig::default();
        let workspace = ready_workspace("issue-421", ".autospec/worktrees/issue-421");
        let planner_impact = impact_for(&workspace, "src/gateway.rs");
        let planned = planner_impact.affected_files();

        let planner = planner_gate(&config.workflow, Some(&planner_impact), &planned);
        let comparison = ImpactComparison::between(&planned, &planner_impact.affected_files());
        let baseline = DiagnosticSet::new("issue-421", "abc123", Vec::new());
        let delta = DiagnosticDelta::between(&baseline, &baseline);
        let implementer = implementer_gate(&config.workflow, &comparison, Some(&delta));
        let reviewer_impact = impact_for(&workspace, "src/router.rs");
        let reviewer = reviewer_gate(
            &config.workflow,
            Some(&planner_impact),
            Some(&reviewer_impact),
            Some(&delta),
        );

        assert!(planner.passed && implementer.passed && reviewer.passed);
        assert!(comparison.within_plan());
    }

    #[test]
    fn a_new_error_after_implementation_blocks_both_later_gates() {
        let config = CodeIntelConfig::default();
        let baseline = DiagnosticSet::new("issue-421", "abc123", Vec::new());
        let current = DiagnosticSet::new(
            "issue-421",
            "abc123",
            vec![Diagnostic::new(
                Location::point("src/gateway.rs", 7, 0),
                Severity::Error,
                "unresolved import",
            )],
        );
        let delta = DiagnosticDelta::between(&baseline, &current);
        let comparison = ImpactComparison::between(&[], &[]);
        let workspace = ready_workspace("issue-421", "wt/a");

        let implementer = implementer_gate(&config.workflow, &comparison, Some(&delta));
        let reviewer = reviewer_gate(
            &config.workflow,
            None,
            Some(&impact_for(&workspace, "src/gateway.rs")),
            Some(&delta),
        );

        assert!(!implementer.passed);
        assert!(!reviewer.passed);
    }

    #[test]
    fn a_configured_gateway_detects_languages_and_reports_health() {
        let config = CodeIntelConfig::default();
        let paths = vec!["Cargo.toml".to_string(), "src/lib.rs".to_string()];
        let languages = detect(&paths, &config.languages.overrides);
        let probe = HostProbe {
            available_servers: vec!["rust-analyzer".to_string()],
            available_fallback_tools: vec!["ast-grep".to_string(), "rg".to_string()],
            backend_present: true,
            backend_version: Some(backend::agent_lsp::PINNED_VERSION.to_string()),
        };

        let report = doctor::report(&config, &languages, &WorkspaceRegistry::new(), &probe);

        assert!(report.is_healthy());
        assert_eq!(report.languages[0].server, "rust-analyzer");
    }

    #[test]
    fn a_multi_repository_impact_set_names_every_repository_it_aggregated() {
        let gateway = ready_workspace("issue-421", "wt/gateway");
        let mut aggregate = impact_for(&gateway, "src/gateway.rs");
        let gui = SemanticWorkspace::new("issue-421-gui", "autospec-gui", "wt/gui", "def456")
            .unwrap()
            .with_state(WorkspaceState::Ready);

        aggregate.merge(impact_for(&gui, "gui/src/main.rs"));

        assert_eq!(
            aggregate.affected_files(),
            vec!["gui/src/main.rs".to_string(), "src/gateway.rs".to_string()]
        );
    }
}
