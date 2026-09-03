pub mod agent;
pub mod autonomous {
    pub mod audit;
    pub mod blast_radius;
    pub mod config;
    pub mod drain;
    pub mod executor;
    pub mod mainline_health;
    pub mod no_work;
    pub mod premerge;
    pub mod review_policy;
    pub mod tier15;
    pub mod tier2;
    pub mod tier3;
    pub mod tier4;
    pub mod waterfall;
}
pub mod autonomous_lifecycle;
pub mod claim;
pub mod code_intel;
pub mod context;
pub mod coordination;
pub mod error;
pub mod evidence;
pub mod execution;
pub mod explore;
pub mod graph;
pub mod growth;
pub mod lint;
pub mod managed_project;
pub mod runtime_env;
pub mod runtime_policy;
pub mod safety;
pub mod spec;
pub mod state;
pub mod validation;

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
