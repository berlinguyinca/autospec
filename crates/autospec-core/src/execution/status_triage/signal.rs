//! How an agent process ended, read from the exit code it left behind
//! (issue #4651).
//!
//! The incident: `iw-87` ran 3602 s and died with `agent_rc=143` mid-edit,
//! 12 files half-applied and `agent.out` empty. The runner bounds itself
//! with `timeout -k 60 "$LIMIT" pi …`, and a `timeout` expiry exits `124`,
//! so the runner's own limit had *not* fired: a different supervisor sent
//! the signal. The record distinguished the two cases perfectly well — and
//! the reader dropped the field that did it.
//!
//! The invariants this module holds:
//!
//! 1. **A termination is attributable, or it says so.** `124` names its
//!    sender (the runner's own bounding `timeout`); `128 + N` names a
//!    signal but no sender, and the difference between those is the whole
//!    diagnosis ([`Termination::attribution`]).
//! 2. **A signal is named, not numbered.** `143` is unreadable in a hold
//!    line; `SIGTERM` is not ([`signal_name`]).
//! 3. **The stall-kill code is not restated here.** It has one definition,
//!    [`crate::run_lifecycle::STALL_KILL_RC`]; this module names what that
//!    code *means*, and its tests pin that the two agree.
//!
//! Pure: it reads the fields a record already carries and never inspects a
//! process, a clock, or a supervisor.

use crate::run_status::Status;

/// The exit code a bounding `timeout` returns when its own limit fires.
///
/// This is the runner saying "I ended it." Anything else that ends a
/// process sends a signal, and a signal exit code is `128 + N` (#4651).
pub const OWN_TIMEOUT_RC: i32 = 124;

/// How an agent's exit code says the process ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Termination {
    /// `rc=124`: the runner's own bounding `timeout` fired. The sender is
    /// known, and the run is a `TIMEOUT`, which the triage re-dispatches.
    ///
    /// The converse does not hold: a harness that reports the child's
    /// death-signal rather than `timeout`'s exit code writes `143` where
    /// `timeout` exits `124`, so a signalling code is ambiguous about its
    /// sender while `124` never is. That asymmetry is why an ambiguous code
    /// is refused as *unattributed* rather than assumed to be the runner, and
    /// why #4651 asks the runner to record whether its own timeout fired
    /// instead of leaving the reader to infer it.
    OwnTimeout,
    /// `rc=128 + N`: a signal ended the process, so the runner's own
    /// `timeout` did *not* fire. The sender is not in the record, and that
    /// absence is the finding (#4651 invariant 1).
    Signalled {
        /// the signal, named rather than numbered.
        signal: &'static str,
    },
    /// The exit code says nothing about a signal: a normal non-zero exit,
    /// zero, or nothing recorded at all.
    Plain,
}

impl Termination {
    /// Whether this termination means the agent was killed by a signal.
    pub fn is_signalled(self) -> bool {
        matches!(self, Self::Signalled { .. })
    }

    /// Classify a recorded `agent_rc`. `None` (nothing recorded) is
    /// [`Termination::Plain`]: absence of a code is not evidence of a
    /// signal, and a triage must not invent one.
    pub fn classify(agent_rc: Option<i32>) -> Self {
        match agent_rc {
            Some(rc) if rc == OWN_TIMEOUT_RC => Self::OwnTimeout,
            Some(rc) => match signal_name(rc) {
                Some(signal) => Self::Signalled { signal },
                None => Self::Plain,
            },
            None => Self::Plain,
        }
    }

    /// Who ended the run, and whether the runner's own `timeout` fired —
    /// the attribution the record owes the reader (#4651 invariant 1).
    ///
    /// `recorded` is the `signal=` field, if the runner named one. When it
    /// disagrees with the exit code, both are reported rather than one
    /// silently winning: a record that says `SIGTERM` over an `rc=137` is
    /// describing two different events, and picking either hides the other.
    pub fn attribution(&self, recorded: Option<&str>, agent_rc: Option<i32>) -> String {
        let rc_part = match agent_rc {
            Some(rc) => format!("agent_rc={rc}"),
            None => "agent_rc unrecorded".to_string(),
        };
        match self {
            Self::OwnTimeout => format!("{rc_part}: the runner's own timeout fired"),
            Self::Signalled { signal } => {
                let named = match recorded {
                    Some(named) if named != *signal => {
                        format!(", the record names {named} instead")
                    }
                    _ => String::new(),
                };
                format!(
                    "{rc_part} is {signal}{named}: the runner's own timeout exits \
                     {OWN_TIMEOUT_RC}, so the signal came from elsewhere and its sender \
                     is unattributed"
                )
            }
            Self::Plain => format!("{rc_part}: no termination signal recorded"),
        }
    }
}

/// The signal behind a shell's `128 + N` exit code, for the signals a
/// supervisor actually sends.
///
/// Only signals a killer uses are named. Naming every signal the shell can
/// report would let `SIGWINCH` read as a stall kill, and `128 + N` is not
/// otherwise distinguishable from an application exiting with a large code.
pub fn signal_name(exit_code: i32) -> Option<&'static str> {
    match exit_code - 128 {
        1 => Some("SIGHUP"),
        2 => Some("SIGINT"),
        6 => Some("SIGABRT"),
        9 => Some("SIGKILL"),
        13 => Some("SIGPIPE"),
        14 => Some("SIGALRM"),
        15 => Some("SIGTERM"),
        _ => None,
    }
}

/// Whether a report describes a signalled termination: its label says so,
/// it named a signal, or its own exit code decodes to one.
///
/// Any one of the three is sufficient, because each can be present without
/// the others: the fleet writes `agent_rc=143` onto records whose `status=`
/// says something else entirely — `UNKNOWN-NO-BASELINE` among them — and a
/// runner that dies before writing its label leaves only the exit code. The
/// exit code is the field that cannot be wrong about how the process died
/// (#4651, and the #4206 rule that a code outranks a word).
pub fn is_signalled(
    status: Option<&str>,
    recorded_signal: Option<&str>,
    agent_rc: Option<i32>,
) -> bool {
    let label = status.and_then(crate::run_status::canonical_status) == Some(Status::Signalled);
    label || recorded_signal.is_some() || Termination::classify(agent_rc).is_signalled()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run_lifecycle::STALL_KILL_RC;

    #[test]
    fn the_stall_kill_code_is_the_signal_the_incident_recorded() {
        // iw-87: agent_rc=143. The runner's own `timeout` would have said 124.
        assert_eq!(STALL_KILL_RC, 143);
        assert_eq!(
            Termination::classify(Some(STALL_KILL_RC)),
            Termination::Signalled { signal: "SIGTERM" }
        );
        assert!(Termination::classify(Some(STALL_KILL_RC)).is_signalled());
    }

    #[test]
    fn the_runners_own_timeout_is_not_a_signal() {
        // The distinction the whole issue turns on: 124 is the runner, 143
        // is somebody else.
        assert_eq!(Termination::classify(Some(124)), Termination::OwnTimeout);
        assert!(!Termination::classify(Some(124)).is_signalled());
    }

    #[test]
    fn an_ordinary_exit_is_no_termination_at_all() {
        for rc in [0, 1, 101, 127, 126, 255] {
            assert_eq!(
                Termination::classify(Some(rc)),
                Termination::Plain,
                "rc={rc} must not read as a kill"
            );
        }
        // Absence of a code is not evidence of a signal.
        assert_eq!(Termination::classify(None), Termination::Plain);
    }

    #[test]
    fn supervisor_signals_are_named_not_numbered() {
        assert_eq!(signal_name(130), Some("SIGINT"));
        assert_eq!(signal_name(137), Some("SIGKILL"));
        assert_eq!(signal_name(143), Some("SIGTERM"));
        // A large application exit code is not a signal.
        assert_eq!(signal_name(200), None);
    }

    #[test]
    fn the_attribution_says_whether_our_own_timeout_fired() {
        let line = Termination::classify(Some(143)).attribution(Some("SIGTERM"), Some(143));
        assert!(line.contains("SIGTERM"), "{line}");
        assert!(line.contains("agent_rc=143"), "{line}");
        assert!(line.contains("124"), "{line}");
        assert!(line.contains("unattributed"), "{line}");
        // A known sender is named as such, and says nothing about a signal.
        let own = Termination::classify(Some(124)).attribution(None, Some(124));
        assert!(own.contains("own timeout"), "{own}");
        assert!(!own.contains("unattributed"), "{own}");
    }

    #[test]
    fn a_disagreeing_record_reports_both_halves() {
        // `signal=SIGTERM` written over `agent_rc=137` describes two events;
        // neither may be dropped silently.
        let line = Termination::classify(Some(137)).attribution(Some("SIGTERM"), Some(137));
        assert!(line.contains("SIGKILL"), "{line}");
        assert!(line.contains("names SIGTERM instead"), "{line}");
    }

    #[test]
    fn a_label_or_a_field_alone_makes_a_run_signalled() {
        assert!(is_signalled(Some("SIGNALLED"), None, None));
        assert!(is_signalled(
            Some("UNKNOWN-NO-BASELINE"),
            Some("SIGTERM"),
            None
        ));
        assert!(is_signalled(Some("NO-OUTPUT"), None, Some(STALL_KILL_RC)));
        // A recorded zero is an exit, not a kill; an empty record is neither.
        assert!(!is_signalled(Some("NO-OUTPUT"), None, Some(0)));
        assert!(!is_signalled(None, None, None));
    }
}
