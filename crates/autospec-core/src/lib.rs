pub mod aar;
pub mod agent;
pub mod agent_watchdog;
pub mod autonomous {
    pub mod audit;
    pub mod blast_radius;
    pub mod config;
    pub mod drain;
    pub mod executor;
    pub mod mainline_health;
    pub mod no_work;
    pub mod premerge;
    pub mod quality_balance;
    pub mod regrade;
    pub mod review_policy;
    pub mod test_gate;
    pub mod tier15;
    pub mod tier2;
    pub mod tier3;
    pub mod tier4;
    pub mod timeout_triage;
    pub mod verdict_validity;
    pub mod waterfall;
}
pub mod autonomous_lifecycle;
pub mod benchmark_matrix;
pub mod ci_gate_promotion;
pub mod claim;
pub mod code_intel;
pub mod config_fidelity;
pub mod conflict_resolution;
pub mod context;
pub mod conversion_gate;
pub mod coordination;
pub mod cost;
pub mod deadline_ratchet;
pub mod declaration_ownership;
pub mod dispatch_guard;
pub mod dispatch_outcomes;
pub mod dispatch_pipeline;
pub mod env_preconditions;
pub mod error;
pub mod evaluation;
pub mod evidence;
pub mod evidence_fidelity;
pub mod execution;
pub mod exit_guard;
pub mod explore;
pub mod failure_signatures;
pub mod fix_surface;
pub mod fleet_dispatch;
pub mod fleet_models;
pub mod gate_provenance;
pub mod gate_serialization;
pub mod gate_verdict;
pub mod grading;
pub mod graph;
pub mod growth;
pub mod heartbeat;
pub mod host_set;
pub mod immutable_base;
pub mod initiative;
pub mod insights;
pub mod integration;
pub mod issue_lock;
pub mod item_sweep;
pub mod lint;
pub mod managed_project;
pub mod not_reproducible;
pub mod planning;
pub mod post_merge;
pub mod procedure;
pub mod prompt_blocks;
pub mod rag;
pub mod rebaseline;
pub mod repair_loop;
pub mod repairs;
pub mod resources;
pub mod restart_safety;
pub mod review_checklist;
pub mod runtime_env;
pub mod runtime_policy;
pub mod safe_publish;
pub mod safety;
pub mod semantic_integration;
pub mod service_address;
pub mod service_timeout;
pub mod spec;
pub mod staged_spec;
pub mod state;
pub mod stored_output;
pub mod symptom_attribution;
pub mod threshold_calibration;
pub mod tractability;
pub mod validation;
pub mod verification;

// Test-only fixture-executable publisher (issue #3500). Compiled only when the
// `test-support` feature is enabled, which happens solely in `autospec-cli`
// test builds via its `[dev-dependencies]`; it is never part of the production
// `autospec` binary.
#[cfg(feature = "test-support")]
pub mod test_support;

pub use error::AutospecError;
pub use safety::{prepare_session_start_git_exclude, SessionStartGitExcludeOutcome};

pub const WORKSPACE_NAME: &str = "autospec";
pub const RUST_CORE_CHECK: &str = "rust-core-workspace";

pub fn doctor_report_json() -> String {
    format!(
        "{{\"status\":\"ok\",\"workspace\":\"{}\",\"checks\":[\"{}\"]}}\n",
        WORKSPACE_NAME, RUST_CORE_CHECK
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doctor_report_names_workspace_and_core_check() {
        let report = doctor_report_json();

        assert!(report.contains("\"status\":\"ok\""));
        assert!(report.contains("\"workspace\":\"autospec\""));
        assert!(report.contains("\"rust-core-workspace\""));
    }
}
