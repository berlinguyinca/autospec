//! Durable and operator-visible mainline-health evidence.

use std::fs::{self, OpenOptions};
use std::io::Write;

use autospec_core::autonomous::mainline_health::{MainlineHealth, MainlineHealthOutcome};

use super::{env_path, RunLayout};

pub(super) fn persist(
    layout: &RunLayout,
    health: &MainlineHealth,
    policy_digest: &str,
) -> Result<(), String> {
    let dir = env_path(
        "AUTOSPEC_AUTONOMOUS_STATE_DIR",
        &[".autospec", "autonomous"],
    )
    .join(&layout.scope);
    fs::create_dir_all(&dir)
        .map_err(|error| format!("cannot create {}: {error}", dir.display()))?;
    let path = dir.join("main-health-observations.jsonl");
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|error| format!("cannot open {}: {error}", path.display()))?;
    writeln!(
        file,
        "{}",
        health.to_json_with_policy_digest(&layout.repo, policy_digest)
    )
    .map_err(|error| format!("cannot write {}: {error}", path.display()))
}

/// Return the receipt that accompanies a machine-readable park decision.
pub(super) fn blocking_receipt(repo: &str, health: &MainlineHealth) -> Option<String> {
    (!matches!(health.outcome, MainlineHealthOutcome::Continue)).then(|| health.to_json(repo))
}

#[cfg(test)]
mod tests {
    use super::blocking_receipt;
    use autospec_core::autonomous::mainline_health::{
        MainlineHealth, MainlineHealthDiagnostic, MainlineHealthOutcome,
    };

    #[test]
    fn blocking_health_exposes_its_diagnostic() {
        let health = MainlineHealth::diagnostic(
            "main",
            MainlineHealthOutcome::Wait,
            MainlineHealthDiagnostic::GhApiFailed,
        );

        let receipt = blocking_receipt("owner/repo", &health).expect("blocking receipt");

        assert!(receipt.contains("\"outcome\":\"wait\""));
        assert!(receipt.contains("\"diagnostic\":\"gh-api-failed\""));
    }
}
