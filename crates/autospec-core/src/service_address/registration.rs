//! The three answers a registration attempt can give (issue #3776).

use std::fmt;

/// What one registration attempt reported.
///
/// The three variants are the three log lines of the incident. They are kept
/// distinct because they have different causes: `Rejected` proves the address
/// is right and the auth is wrong; `Unreachable` (`000`, connection refused) is
/// the signature of a service that has moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistrationOutcome {
    /// 2xx — the gateway now knows this worker exists.
    Registered,
    /// The service answered with a non-success status.
    Rejected { status: u16 },
    /// Nothing answered (`000`, connection refused, connection reset).
    Unreachable,
}

impl RegistrationOutcome {
    /// Map an HTTP status to an outcome. `0` is the no-response status `curl`
    /// reports for a transport failure, not a status any server sends.
    pub fn from_status(status: u16) -> Self {
        match status {
            0 => Self::Unreachable,
            200..=299 => Self::Registered,
            code => Self::Rejected { status: code },
        }
    }

    /// True when the pool now knows about this component.
    pub fn succeeded(&self) -> bool {
        matches!(self, Self::Registered)
    }

    /// True when this failure is explained by the address being stale. A
    /// `Rejected` outcome proves reachability, so it is not itself evidence of
    /// a move — but the cache is invalidated on it anyway, because re-reading
    /// the record is one `read` and the alternative is a permanent wrong guess.
    pub fn address_may_be_stale(&self) -> bool {
        matches!(self, Self::Unreachable)
    }
}

impl fmt::Display for RegistrationOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Registered => f.write_str("registered"),
            Self::Rejected { status } => write!(f, "registration rejected ({status})"),
            Self::Unreachable => f.write_str("registration transport failed (000)"),
        }
    }
}
