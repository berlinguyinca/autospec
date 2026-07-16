use autospec_core::autonomous::tier2::{
    evaluate_tier2, Tier2Failure, Tier2FailureCode, Tier2Input, Tier2Observation, Tier2RoiPolicy,
    Tier2Stage, Tier2StageResult,
};
use autospec_core::explore::specialists::{StrictCollectorError, StrictCollectorErrorCode};

#[allow(clippy::large_enum_variant)] // Preserves the plan's explicit typed injection seam.
pub(super) enum Tier2Scan {
    NotRun,
    Complete(Tier2Observation),
    Failed(Tier2Failure),
}

pub(super) fn disabled_by_checked_in_policy() -> Tier2Scan {
    match evaluate_tier2(Tier2Input::DisabledByCheckedInPolicy) {
        Ok(_) => Tier2Scan::NotRun,
        Err(failure) => Tier2Scan::Failed(failure),
    }
}

pub(super) fn strict_collector_failure(error: StrictCollectorError) -> Tier2Failure {
    let code = match error.code {
        StrictCollectorErrorCode::InvalidRoot => Tier2FailureCode::InvalidRoot,
        StrictCollectorErrorCode::InvalidCollectorSchema => {
            Tier2FailureCode::InvalidCollectorSchema
        }
        StrictCollectorErrorCode::PathEscapesRoot => Tier2FailureCode::PathEscapesRoot,
        StrictCollectorErrorCode::ReadDirectory => Tier2FailureCode::ReadDirectory,
        StrictCollectorErrorCode::ReadFile => Tier2FailureCode::ReadFile,
        StrictCollectorErrorCode::InvalidUtf8 => Tier2FailureCode::InvalidUtf8,
    };
    let detail = bounded_detail(&error.detail);
    let initial = Tier2Failure::new(Tier2Stage::Collector, code, detail)
        .expect("bounded strict collector detail is valid");
    match evaluate_tier2(Tier2Input::Enabled {
        collector: Tier2StageResult::Failed(initial),
        generator: Tier2StageResult::Missing,
        verifier: Tier2StageResult::Missing,
        roi_policy: Tier2RoiPolicy::v1(),
    }) {
        Err(failure) => failure,
        Ok(_) => unreachable!("a collector failure cannot evaluate successfully"),
    }
}

fn bounded_detail(detail: &str) -> String {
    let bounded = detail.chars().take(240).collect::<String>();
    if bounded.trim().is_empty() {
        "strict collector returned an empty diagnostic".to_string()
    } else {
        bounded
    }
}

#[cfg(test)]
mod tests {
    use autospec_core::autonomous::tier2::Tier2FailureCode;
    use autospec_core::explore::specialists::{StrictCollectorError, StrictCollectorErrorCode};

    use super::strict_collector_failure;

    #[test]
    fn strict_collector_codes_map_exhaustively_to_sealed_tier_two_failures() {
        for (strict, expected) in [
            (
                StrictCollectorErrorCode::InvalidRoot,
                Tier2FailureCode::InvalidRoot,
            ),
            (
                StrictCollectorErrorCode::InvalidCollectorSchema,
                Tier2FailureCode::InvalidCollectorSchema,
            ),
            (
                StrictCollectorErrorCode::PathEscapesRoot,
                Tier2FailureCode::PathEscapesRoot,
            ),
            (
                StrictCollectorErrorCode::ReadDirectory,
                Tier2FailureCode::ReadDirectory,
            ),
            (
                StrictCollectorErrorCode::ReadFile,
                Tier2FailureCode::ReadFile,
            ),
            (
                StrictCollectorErrorCode::InvalidUtf8,
                Tier2FailureCode::InvalidUtf8,
            ),
        ] {
            let failure = strict_collector_failure(StrictCollectorError {
                code: strict,
                detail: "strict collector diagnostic".to_string(),
            });
            assert_eq!(failure.code(), expected);
            assert_eq!(failure.stage().as_str(), "collector");
            assert!(failure.documents().is_some());
        }
    }

    #[test]
    fn strict_collector_oversized_diagnostic_is_bounded_without_becoming_dry() {
        let failure = strict_collector_failure(StrictCollectorError {
            code: StrictCollectorErrorCode::ReadFile,
            detail: "é".repeat(241),
        });

        assert_eq!(failure.detail().chars().count(), 240);
        assert_eq!(failure.status_reason(), "tier2_collector_read_file");
        assert!(failure.documents().is_some());
    }
}
