//! When may a replacement conductor take a lease another conductor holds?
//!
//! Split out of `autonomous_lifecycle.rs`, which is past the size ratchet. The rule this module
//! exists to state: reclamation needs *proof*. An owner we cannot observe counts as live, so only
//! a pid proven absent or terminated on the recorded host releases a lease early.

use super::{ABANDONED_LEASE_SECS, STALE_LEASE_SECS};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConductorLeaseInput {
    claimed: bool,
    heartbeat_age_secs: Option<u64>,
    same_host_pid_dead: bool,
}

impl ConductorLeaseInput {
    pub fn claimed(heartbeat_age_secs: u64, same_host_pid_dead: bool) -> Self {
        Self {
            claimed: true,
            heartbeat_age_secs: Some(heartbeat_age_secs),
            same_host_pid_dead,
        }
    }

    pub fn running(heartbeat_age_secs: u64, same_host_pid_dead: bool) -> Self {
        Self {
            claimed: false,
            heartbeat_age_secs: Some(heartbeat_age_secs),
            same_host_pid_dead,
        }
    }

    pub fn missing_heartbeat(claimed: bool, same_host_pid_dead: bool) -> Self {
        Self {
            claimed,
            heartbeat_age_secs: None,
            same_host_pid_dead,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConductorLeaseReclaim {
    DeadSameHostPid,
    MissingHeartbeat,
    Abandoned,
    ClaimedExpired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConductorLeaseDecision {
    Held,
    Reclaim(ConductorLeaseReclaim),
}

pub fn decide_conductor_lease(input: ConductorLeaseInput) -> ConductorLeaseDecision {
    // A proven-dead owner on this host releases the lease whatever state it is in. The
    // `claimed` state used to be excluded, which parked the repository for STALE_LEASE_SECS
    // (5 minutes) whenever a conductor died inside the claim window -- and the store already
    // disagreed: `release_terminated_owner` treats `status == "claimed"` with a dead
    // `lock_pid` as an abandoned claim it may release. The owner's harness may still be
    // running; the replacement adopts it rather than starting a second one, which is what
    // `foreground_repeated_restart_observes_one_live_harness_until_merge` asserts end to end.
    //
    // `same_host_pid_dead` is only ever true for a pid on the recorded host that we proved
    // absent or terminated, so an unknown or foreign owner still reads as live.
    if input.same_host_pid_dead {
        return ConductorLeaseDecision::Reclaim(ConductorLeaseReclaim::DeadSameHostPid);
    }
    let Some(age) = input.heartbeat_age_secs else {
        return ConductorLeaseDecision::Reclaim(ConductorLeaseReclaim::MissingHeartbeat);
    };
    if age >= ABANDONED_LEASE_SECS {
        return ConductorLeaseDecision::Reclaim(ConductorLeaseReclaim::Abandoned);
    }
    if input.claimed && age >= STALE_LEASE_SECS {
        return ConductorLeaseDecision::Reclaim(ConductorLeaseReclaim::ClaimedExpired);
    }
    ConductorLeaseDecision::Held
}
