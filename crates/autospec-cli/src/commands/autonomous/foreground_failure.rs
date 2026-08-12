//! How a foreground cycle fails, and which failures may be contained.
//!
//! The distinction matters more than it looks. A claim the conductor did not win is
//! one candidate's problem; a lifecycle lease reject means another conductor owns the
//! repository. Both used to arrive as the same variant, so containing one would have
//! silently contained the other and left two conductors mutating one checkout.

use super::claim;
use super::CommandFailure;

pub(super) enum ForegroundFailure {
    Diagnostic(CommandFailure),
    Deferred {
        json: String,
        exit_code: i32,
    },
    /// A claim the conductor did not win. Only `ConductorClaimError` raises this, so
    /// lifecycle admission and lease rejects -- including `Held`, where a second
    /// conductor owns the repo -- keep ending the run through `Deferred`.
    CandidateDeferred {
        json: String,
        exit_code: i32,
    },
}

impl From<CommandFailure> for ForegroundFailure {
    fn from(error: CommandFailure) -> Self {
        Self::Diagnostic(error)
    }
}

impl From<claim::ConductorClaimError> for ForegroundFailure {
    fn from(error: claim::ConductorClaimError) -> Self {
        match error {
            claim::ConductorClaimError::Diagnostic(error) => Self::Diagnostic(error),
            claim::ConductorClaimError::Deferred { json, exit_code } => {
                Self::CandidateDeferred { json, exit_code }
            }
        }
    }
}
