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
/// Proof requires a heartbeat from this boot and either an absent PID or a live PID whose start
/// identity differs from the recorded owner. Unreadable evidence and failed probes keep blocking
/// recovery so a live worker's claim is never stolen.
pub(super) fn startup_heartbeat_owner_is_gone(path: &Path) -> bool {
    let Ok(document) = std::fs::read(path) else {
        return false;
    };
    let Some(evidence) = super::parse_startup_heartbeat(&document) else {
        return false;
    };
    let Ok(boot_id) = super::super::autonomous::current_boot_identity() else {
        return false;
    };
    if evidence.boot_id != boot_id {
        return false;
    }
    match super::super::autonomous::process_birth_identity(evidence.pid) {
        Ok(None) => true,
        Ok(Some((_, start_identity))) => start_identity != evidence.process_start,
        Err(_) => false,
    }
}
