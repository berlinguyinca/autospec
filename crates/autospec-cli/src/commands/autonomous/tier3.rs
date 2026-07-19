use autospec_core::autonomous::tier3::{
    evaluate_tier3, Tier3Evaluation, Tier3Failure, Tier3Input, Tier3Observation,
};

#[allow(clippy::large_enum_variant)]
pub(super) enum Tier3Scan {
    NotRun,
    Complete(Tier3Observation),
    Failed(Tier3Failure),
}

pub(super) fn disabled_by_checked_in_policy() -> Tier3Scan {
    match evaluate_tier3(Tier3Input::DisabledByCheckedInPolicy) {
        Ok(Tier3Evaluation::NotRun(_)) => Tier3Scan::NotRun,
        Ok(Tier3Evaluation::Complete(_)) => {
            unreachable!("disabled Tier 3 policy cannot observe metadata")
        }
        Err(failure) => Tier3Scan::Failed(failure),
    }
}

#[cfg(test)]
mod tests {
    use super::disabled_by_checked_in_policy;

    #[test]
    fn disabled_policy_adapter_has_no_execution_or_mutation_authority() {
        let source = include_str!("tier3.rs");
        let production = source
            .split("\n#[cfg(test)]")
            .next()
            .expect("production source");
        assert_eq!(
            production
                .matches("Tier3Input::DisabledByCheckedInPolicy")
                .count(),
            1
        );
        for forbidden in [
            "std::env",
            "std::fs",
            "fs::",
            "std::io",
            "io::",
            "OpenOptions",
            "File::",
            "std::process",
            "std::net",
            "Command",
            "bash",
            "zsh",
            "sh -c",
            "curl",
            "gh ",
            "github",
            "queue",
            "claim",
            "label",
            "branch",
            "worktree",
            "pull_request",
            "WaterfallStore",
            "run_foreground",
            "scan_foreground",
            "dispatch",
            "ExecutorRequest",
            "graphql",
        ] {
            assert!(
                !production.contains(forbidden),
                "Tier 3 adapter retains prohibited authority: {forbidden}"
            );
        }
        assert!(matches!(
            disabled_by_checked_in_policy(),
            super::Tier3Scan::NotRun
        ));
    }
}
