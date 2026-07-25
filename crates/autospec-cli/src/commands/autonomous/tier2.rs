use autospec_core::autonomous::tier2::{evaluate_tier2, Tier2Failure, Tier2Input};
use autospec_core::explore::specialists::StrictCollectorError;

pub(super) use super::tier2_runner::{scan_native, Tier2Scan};

pub(super) fn strict_collector_failure(error: StrictCollectorError) -> Tier2Failure {
    super::tier2_runner::strict_collector_failure(error)
}

pub(super) fn disabled_by_checked_in_policy() -> Tier2Scan {
    match evaluate_tier2(Tier2Input::DisabledByCheckedInPolicy) {
        Ok(_) => Tier2Scan::NotRun,
        Err(failure) => Tier2Scan::Failed(failure),
    }
}
