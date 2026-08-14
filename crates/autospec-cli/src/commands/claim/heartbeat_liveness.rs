use std::path::Path;

pub(super) fn startup_heartbeat_exists(repo: &str, issue: u64) -> bool {
    super::heartbeat_root().is_ok_and(|root| {
        let path = root
            .join(super::super::autonomous::drain::repository_progress_key(
                repo,
            ))
            .join(format!("{issue}.json"));
        path.is_file() && !startup_heartbeat_owner_is_gone(&path)
    })
}

/// True only when the heartbeat's owning process is provably gone.
///
/// Proof requires a heartbeat from this boot whose PID is natively observed as absent. Unreadable
/// evidence, identity mismatches, and failed probes keep blocking recovery so ambiguity never
/// transfers cleanup authority.
pub(super) fn startup_heartbeat_owner_is_gone(path: &Path) -> bool {
    let Ok(document) = std::fs::read(path) else {
        return false;
    };
    let Some(evidence) = super::parse_startup_heartbeat(&document) else {
        return false;
    };
    use super::super::autonomous::platform_process::{observe_expected, ProcessObservation};
    match observe_expected(evidence.pid, &evidence.boot_id, &evidence.process_start) {
        ProcessObservation::Dead => true,
        ProcessObservation::Exact(_)
        | ProcessObservation::Mismatch
        | ProcessObservation::Unknown(_) => false,
    }
}
