use autospec_core::autonomous::config::Tier4Config;
use autospec_core::autonomous::tier4::{
    evaluate_tier4, Tier4Evaluation, Tier4Failure, Tier4Input, Tier4Observation,
};

#[allow(clippy::large_enum_variant)]
pub(super) enum Tier4Scan {
    NotRun,
    Complete(Tier4Observation),
    Failed(Tier4Failure),
}

pub(super) fn disabled_by_checked_in_policy(_config: &Tier4Config) -> Tier4Scan {
    match evaluate_tier4(Tier4Input::DisabledByCheckedInPolicy) {
        Ok(Tier4Evaluation::NotRun(_)) => Tier4Scan::NotRun,
        Ok(Tier4Evaluation::Complete(_)) => {
            unreachable!("disabled Tier 4 policy cannot discover candidates")
        }
        Err(failure) => Tier4Scan::Failed(failure),
    }
}

#[cfg(test)]
mod tests {
    use autospec_core::autonomous::config::Tier4Config;

    use super::disabled_by_checked_in_policy;

    #[test]
    fn disabled_policy_adapter_has_only_checked_in_policy_authority() {
        let source = include_str!("tier4.rs");
        let production = source
            .split("\n#[cfg(test)]")
            .next()
            .expect("production source");
        assert_eq!(
            production
                .matches("Tier4Input::DisabledByCheckedInPolicy")
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
            "branch",
            "worktree",
            "run_foreground",
            "scan_foreground",
            "dispatch",
            "ExecutorRequest",
        ] {
            assert!(
                !production.contains(forbidden),
                "Tier 4 adapter retains prohibited authority: {forbidden}"
            );
        }
        assert!(matches!(
            disabled_by_checked_in_policy(&Tier4Config::default()),
            super::Tier4Scan::NotRun
        ));
    }
}
