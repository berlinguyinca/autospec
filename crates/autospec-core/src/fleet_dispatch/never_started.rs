//! The `AGENT-NEVER-STARTED` rule (spec 2026-10-14 §7 item 6).
//!
//! An exit code is never proof the agent ran. A runner that exits 0 while
//! leaving no session record, or a session with zero completed assistant
//! turns, started a process but never an agent. Classifying that as `OK` or
//! `NO-OUTPUT` would charge the issue an attempt for work nobody did.

/// Status: the runner exited 0 but the agent never started. Does not consume
/// an attempt.
pub const AGENT_NEVER_STARTED_STATUS: &str = "AGENT-NEVER-STARTED";

/// Whether this run's evidence says the agent never started.
///
/// True only for `agent_rc == Some(0)` together with an explicit negative
/// signal: `session_recorded == Some(false)` or `assistant_turns == Some(0)`.
/// A record that carries neither field (every legacy record) is never
/// reclassified, and a nonzero exit is a different failure with its own
/// status. `assistant_turns` counts completed turns; a turn that ended with
/// `stopReason:"error"` is not one.
pub fn never_started(
    agent_rc: Option<i32>,
    session_recorded: Option<bool>,
    assistant_turns: Option<u32>,
) -> bool {
    agent_rc == Some(0) && (session_recorded == Some(false) || assistant_turns == Some(0))
}
