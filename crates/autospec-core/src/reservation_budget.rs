//! Agent budget derivation against a Slurm reservation's walltime (issue #3690).
//!
//! An agent run is dispatched inside a Slurm reservation whose walltime is
//! known at submission time. The runner used to impose a fixed 45-minute
//! LIMIT regardless of the reservation, so every run was killed long before
//! the reservation ended and the patch it had been building was discarded.
//! Three of the issue's four rules are encoded here as pure, testable
//! primitives; the fourth lives in [`crate::failure_signatures`]
//! ([`crate::failure_signatures::TIMEOUT_NO_OUTPUT_SIGNATURE`]):
//!
//! 1. **The agent budget is derived from the reservation walltime**
//!    ([`derive_agent_budget`]): budget = walltime minus startup/teardown
//!    overhead. A reservation too small to host a minimum useful run is
//!    refused, not shrunk to a meaningless sliver.
//! 2. **A budget that does not fit is refused before the run starts**
//!    ([`validate_budget`]): requested budget plus overhead must not exceed
//!    the reservation walltime.
//! 3. **A timed-out run's building patch is never discarded**
//!    ([`classify_timeout_disposition`]).
//! 4. **A timeout with no output is reported distinctly** from an ordinary
//!    silent failure, so the frontier can react to "the budget was too
//!    small" instead of guessing.

/// Job startup plus teardown overhead reserved out of the walltime:
/// provisioning, environment setup, teardown and artifact collection.
pub const RESERVATION_OVERHEAD_SECS: u64 = 600;

/// Minimum useful agent budget. A reservation that would leave less than
/// this for the agent itself is too small to be worth dispatching into.
pub const MIN_AGENT_BUDGET_SECS: u64 = 900;

/// An agent budget derived from a reservation's walltime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentBudget {
    /// The reservation walltime the budget was derived from.
    pub walltime_secs: u64,
    /// Overhead subtracted from the walltime.
    pub overhead_secs: u64,
    /// Seconds the agent may run.
    pub budget_secs: u64,
}

/// A budget or reservation that cannot host an agent run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetError {
    /// The walltime minus overhead leaves less than
    /// [`MIN_AGENT_BUDGET_SECS`] for the agent.
    ReservationTooSmall {
        walltime_secs: u64,
        minimum_walltime_secs: u64,
    },
    /// The requested budget plus overhead exceeds the walltime.
    BudgetExceedsReservation {
        walltime_secs: u64,
        requested_budget_secs: u64,
        max_budget_secs: u64,
    },
    /// A budget of zero seconds was requested.
    ZeroBudget,
}

impl std::fmt::Display for BudgetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ReservationTooSmall {
                walltime_secs,
                minimum_walltime_secs,
            } => write!(
                f,
                "reservation walltime {}s leaves less than {}s for the agent \
                 after {}s overhead; minimum useful walltime is {}s",
                walltime_secs,
                MIN_AGENT_BUDGET_SECS,
                RESERVATION_OVERHEAD_SECS,
                minimum_walltime_secs
            ),
            Self::BudgetExceedsReservation {
                walltime_secs,
                requested_budget_secs,
                max_budget_secs,
            } => write!(
                f,
                "budget {}s does not fit a {}s reservation: the agent budget \
                 may be at most {}s (walltime minus {}s overhead)",
                requested_budget_secs, walltime_secs, max_budget_secs, RESERVATION_OVERHEAD_SECS
            ),
            Self::ZeroBudget => write!(f, "a zero-second agent budget cannot host a run"),
        }
    }
}

impl std::error::Error for BudgetError {}

/// What to do with a timed-out run's building patch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeoutDisposition {
    /// The run was still building a patch when it timed out: preserve it so
    /// the next run can resume instead of rebuilding from scratch.
    PreservePatch,
    /// The run held no patch: nothing to preserve.
    NoPatch,
}

impl TimeoutDisposition {
    /// True when the building patch must be kept.
    pub fn preserves_patch(self) -> bool {
        matches!(self, Self::PreservePatch)
    }
}

/// A timed-out run's building patch is never discarded (issue #3690).
pub fn classify_timeout_disposition(patch_present: bool) -> TimeoutDisposition {
    if patch_present {
        TimeoutDisposition::PreservePatch
    } else {
        TimeoutDisposition::NoPatch
    }
}

/// Parse a reservation walltime as plain seconds or Slurm's
/// `[[HH:]MM:]SS` form: one component is seconds, two are minutes and
/// seconds, three are hours, minutes and seconds.
pub fn parse_walltime(input: &str) -> Result<u64, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("empty walltime".to_string());
    }
    let components: Vec<&str> = trimmed.split(':').collect();
    if components.len() > 3 {
        return Err(format!(
            "walltime {trimmed:?} has {} components; expected seconds or [[HH:]MM:]SS",
            components.len()
        ));
    }
    let mut seconds = 0u64;
    for (position, component) in components.iter().enumerate() {
        if component.is_empty() || !component.chars().all(|c| c.is_ascii_digit()) {
            return Err(format!("walltime {trimmed:?} has a non-numeric component"));
        }
        let value: u64 = component
            .parse()
            .map_err(|_| format!("walltime {trimmed:?} is out of range"))?;
        // Seconds; MM:SS is minutes and seconds; HH:MM:SS hours, minutes and
        // seconds (Slurm's walltime forms).
        let scale = match components.len() {
            1 => 1u64,
            2 => [60, 1][position],
            3 => [3600, 60, 1][position],
            _ => unreachable!("component count checked above"),
        };
        let term = value
            .checked_mul(scale)
            .ok_or_else(|| format!("walltime {trimmed:?} is out of range"))?;
        seconds = seconds
            .checked_add(term)
            .ok_or_else(|| format!("walltime {trimmed:?} is out of range"))?;
    }
    Ok(seconds)
}

/// Format seconds as `H:MM:SS` for reports.
pub fn format_duration(total_secs: u64) -> String {
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;
    format!("{hours}:{minutes:02}:{seconds:02}")
}

/// Derive the agent budget from the reservation walltime: the walltime minus
/// startup/teardown overhead, refused when the remainder is below
/// [`MIN_AGENT_BUDGET_SECS`].
pub fn derive_agent_budget(walltime_secs: u64) -> Result<AgentBudget, BudgetError> {
    let maximum = walltime_secs.saturating_sub(RESERVATION_OVERHEAD_SECS);
    if maximum < MIN_AGENT_BUDGET_SECS {
        return Err(BudgetError::ReservationTooSmall {
            walltime_secs,
            minimum_walltime_secs: MIN_AGENT_BUDGET_SECS + RESERVATION_OVERHEAD_SECS,
        });
    }
    Ok(AgentBudget {
        walltime_secs,
        overhead_secs: RESERVATION_OVERHEAD_SECS,
        budget_secs: maximum,
    })
}

/// Refuse to start when the requested budget does not fit the reservation:
/// budget plus overhead must not exceed the walltime.
pub fn validate_budget(walltime_secs: u64, requested_budget_secs: u64) -> Result<(), BudgetError> {
    if requested_budget_secs == 0 {
        return Err(BudgetError::ZeroBudget);
    }
    let maximum = walltime_secs.saturating_sub(RESERVATION_OVERHEAD_SECS);
    if requested_budget_secs > maximum {
        return Err(BudgetError::BudgetExceedsReservation {
            walltime_secs,
            requested_budget_secs,
            max_budget_secs: maximum,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_seconds() {
        assert_eq!(parse_walltime("28800").unwrap(), 28800);
        assert_eq!(parse_walltime(" 45 ").unwrap(), 45);
        assert_eq!(parse_walltime("0").unwrap(), 0);
    }

    #[test]
    fn parses_slurm_mm_ss_and_hh_mm_ss() {
        assert_eq!(parse_walltime("45:00").unwrap(), 2700);
        assert_eq!(parse_walltime("1:30").unwrap(), 90);
        assert_eq!(parse_walltime("8:00:00").unwrap(), 8 * 3600);
        assert_eq!(parse_walltime("0:00:05").unwrap(), 5);
    }

    #[test]
    fn rejects_malformed_walltime() {
        assert!(parse_walltime("").is_err());
        assert!(parse_walltime("   ").is_err());
        assert!(parse_walltime(":30").is_err());
        assert!(parse_walltime("8:").is_err());
        assert!(parse_walltime("1:2:3:4").is_err());
        assert!(parse_walltime("8:oo:00").is_err());
        assert!(parse_walltime("-5").is_err());
    }

    #[test]
    fn derives_budget_from_eight_hour_reservation() {
        // The shape of the bug: 8h reservation, 45min fixed LIMIT.
        let budget = derive_agent_budget(8 * 3600).unwrap();
        assert_eq!(budget.walltime_secs, 28800);
        assert_eq!(budget.overhead_secs, RESERVATION_OVERHEAD_SECS);
        assert_eq!(budget.budget_secs, 28800 - RESERVATION_OVERHEAD_SECS);
    }

    #[test]
    fn derives_budget_at_the_minimum_walltime_boundary() {
        let walltime = MIN_AGENT_BUDGET_SECS + RESERVATION_OVERHEAD_SECS;
        let budget = derive_agent_budget(walltime).unwrap();
        assert_eq!(budget.budget_secs, MIN_AGENT_BUDGET_SECS);
        assert!(derive_agent_budget(walltime - 1).is_err());
    }

    #[test]
    fn refuses_reservations_too_small_to_host_an_agent() {
        let error = derive_agent_budget(1499).unwrap_err();
        assert_eq!(
            error,
            BudgetError::ReservationTooSmall {
                walltime_secs: 1499,
                minimum_walltime_secs: MIN_AGENT_BUDGET_SECS + RESERVATION_OVERHEAD_SECS,
            }
        );
        assert!(error.to_string().contains("900s"));
    }

    #[test]
    fn validate_accepts_budget_that_fits() {
        assert!(validate_budget(28800, 2700).is_ok());
        // Exactly at the maximum still fits.
        assert!(validate_budget(28800, 28800 - RESERVATION_OVERHEAD_SECS).is_ok());
    }

    #[test]
    fn validate_refuses_budget_over_the_reservation() {
        let error = validate_budget(45 * 60, 2700).unwrap_err();
        assert_eq!(
            error,
            BudgetError::BudgetExceedsReservation {
                walltime_secs: 2700,
                requested_budget_secs: 2700,
                max_budget_secs: 2700 - RESERVATION_OVERHEAD_SECS,
            }
        );
        assert!(error.to_string().contains("does not fit"));
    }

    #[test]
    fn validate_refuses_zero_budget() {
        assert_eq!(validate_budget(28800, 0), Err(BudgetError::ZeroBudget));
    }

    #[test]
    fn timed_out_run_with_a_building_patch_is_preserved() {
        assert_eq!(
            classify_timeout_disposition(true),
            TimeoutDisposition::PreservePatch
        );
        assert!(classify_timeout_disposition(true).preserves_patch());
    }

    #[test]
    fn timed_out_run_without_a_patch_has_nothing_to_preserve() {
        assert_eq!(
            classify_timeout_disposition(false),
            TimeoutDisposition::NoPatch
        );
        assert!(!classify_timeout_disposition(false).preserves_patch());
    }

    #[test]
    fn format_duration_renders_hh_mm_ss() {
        assert_eq!(format_duration(28800), "8:00:00");
        assert_eq!(format_duration(2700), "0:45:00");
        assert_eq!(format_duration(5), "0:00:05");
    }
}
