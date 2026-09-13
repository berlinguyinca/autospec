pub mod aar;
pub mod agent;
pub mod agent_watchdog;
pub mod aggregate_granularity;
pub mod argument_scope;
pub mod implementation_language;
pub mod shell_ratchet;
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
pub mod baseline_coverage;
pub mod benchmark_matrix;
pub mod capability_admission;
pub mod ci_gate_promotion;
pub mod ci_name_drift;
pub mod circuit_breaker;
pub mod claim;
pub mod claim_lifecycle;
pub mod code_intel;
pub mod config_fidelity;
pub mod conflict_resolution;
pub mod construction_sites;
pub mod context;
pub mod control_file;
pub mod conversion_gate;
pub mod conversion_pass;
pub mod convert_pass;
pub mod coordination;
pub mod cost;
pub mod cron_log_contract;
pub mod deadline_ratchet;
pub mod declaration_ownership;
pub mod dependency_gates;
pub mod deploy_drift;
pub mod diff_coverage;
pub mod dispatch_guard;
pub mod dispatch_outcomes;
pub mod dispatch_pipeline;
pub mod dispatch_recheck;
pub mod effect_verification;
pub mod entity_binding;
pub mod env_preconditions;
pub mod error;
pub mod evaluation;
pub mod evidence;
pub mod evidence_fidelity;
pub mod execution;
pub mod exit_guard;
pub mod explore;
pub mod failure_attribution;
pub mod failure_floor;
pub mod failure_signatures;
pub mod false_negative;
pub mod fix_surface;
pub mod fleet_dispatch;
pub mod fleet_models;
pub mod fleet_pipeline_health;
pub mod gate_coverage;
pub mod gate_hold;
pub mod gate_provenance;
pub mod gate_serialization;
pub mod gate_verdict;
pub mod grading;
pub mod graph;
pub mod growth;
pub mod guard_extraction;
pub mod heartbeat;
pub mod held_backlog;
pub mod held_conflicts;
pub mod hold_memo;
pub mod host_set;
// Unix-only by design: the module's core abstraction is the `current`
// symlink it swaps atomically, and `std::os::unix::fs::symlink` does not
// exist on Windows. Rather than invent Windows symlink semantics nobody
// has asked for, the module is simply absent there. autospec-cli does not
// reference it, so the Windows CLI check loses nothing.
#[cfg(unix)]
pub mod immutable_base;
pub mod initiative;
pub mod insights;
pub mod integration;
pub mod io_coverage;
pub mod issue_lock;
pub mod issue_skeleton;
pub mod item_sweep;
pub mod kv_overcommit;
pub mod lint;
pub mod log_freshness;
pub mod loop_actuation;
pub mod loop_reset;
pub mod managed_project;
pub mod measurement_assertion;
pub mod memo_key;
pub mod merge_gate;
pub mod name_scope;
pub mod negative_evidence;
pub mod not_reproducible;
pub mod planning;
pub mod platform_gate;

pub mod positive_control;
pub mod post_merge;
pub mod prefilter_scope;
pub mod procedure;
pub mod process_evidence;
// Unix-only by design: pattern-based termination is built on `ps`, `pgrep`,
// and `nix::sys::signal::kill` — the self-safety helper behind
// `autospec process-kill` (issue #4448). The platform matrix for this
// repository is Linux/macOS/BSD CI; there is no Windows process table to
// guard against, so the module is absent there rather than stubbed.
#[cfg(unix)]
pub mod process_termination;
pub mod progress_contract;
pub mod progress_signal;
pub mod prompt_blocks;
pub mod prose_closure;
pub mod queue_gap;
pub mod rag;
pub mod reader_fidelity;
pub mod rebase_review;
pub mod rebaseline;
pub mod recovery_path;
pub mod refresh_queue_contract;
pub mod repair_loop;
pub mod repairs;

pub mod reservation_budget;
pub mod resources;
pub mod restart_safety;
pub mod result_soundness;
pub mod review_checklist;
pub mod roll_selection;
pub mod run_lifecycle;
pub mod run_status;
pub mod runner_verdict;
pub mod runtime_env;
pub mod runtime_policy;
pub mod safe_publish;
pub mod safety;
pub mod scratch_home;
pub mod self_gate;
pub mod semantic_integration;
pub mod service_address;
pub mod service_timeout;
pub mod shared_write_target;
pub mod size_dependent_timeout;
pub mod spec;
pub mod spec_outcome;
pub mod spot_measurement;
pub mod staged_spec;
pub mod state;
pub mod stored_output;
pub mod summarization;
pub mod symptom_attribution;
pub mod threshold_calibration;
pub mod toolchain_gate;
pub mod tractability;
pub mod unfed_pass;
pub mod untested_public_items;
pub mod validation;
pub mod verdict_shelf;
pub mod verification;
pub mod warned_output;
pub mod watchdog_evidence;
pub mod weight_acquisition;
pub mod wire_fixture;
pub mod work_selection;
pub mod worker_disposition;
pub mod workload_validation;
pub mod worktree_lock;

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
