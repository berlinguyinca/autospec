//! Is the recorded owner of a resilience lease still there?
//!
//! Two questions, both answered conservatively: an unknown answer means "still live", because
//! only proven absence may release another worker's lease. Split out of `resilience.rs`, which
//! is past the size ratchet and may not grow.

pub(super) fn same_known_host(recorded: &str, current: &str) -> bool {
    let recorded = recorded.trim();
    let current = current.trim();
    !recorded.is_empty()
        && !current.is_empty()
        && !recorded.eq_ignore_ascii_case("unknown")
        && !current.eq_ignore_ascii_case("unknown")
        && recorded == current
}

pub(super) fn pid_is_dead(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    #[cfg(unix)]
    if pid > i32::MAX as u32 {
        return false;
    }
    // A reaped process disappears, but a terminated one keeps its PID entry until somebody waits
    // on it, and identity observation still resolves for a zombie. The conductor arms a subreaper,
    // so an orphaned lease holder can stay unreaped for the life of the run: the lifecycle lease
    // would never free and every replacement conductor would defer instead of adopting the
    // repository. Sibling call site `observe_unit_process_identity` already asks this first.
    //
    // Unknown stays live. Only proven termination releases another worker's lease.
    if crate::commands::autonomous::executor_bridge::process_is_terminated(pid).unwrap_or(false) {
        return true;
    }
    matches!(
        crate::commands::autonomous::executor_bridge::observe_runtime_process_identity(pid),
        Ok(None)
    )
}
