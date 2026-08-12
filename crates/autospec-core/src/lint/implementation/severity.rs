//! Which rules report instead of blocking, decided once for the whole run.
//!
//! `COMPLEXITY` is advisory unless `AUTOSPEC_COMPLEXITY_ENFORCE=1`. As a veto the limits
//! froze oversized files against even a one-line safe edit: the ratchet waives *file*-LOC
//! for a file that does not grow, but a long function or a file-level cyclomatic score is
//! neither waived nor suppressible, so `fleet-gui-server.py` could not accept a three-line
//! fix at all (#2961). `scripts/lint-implementation.sh` made the same change; this module
//! is what keeps the two implementations from disagreeing, which they did for one release —
//! the pre-commit path honoured the policy while `autospec lint implementation` and the
//! Phase 4 repair loop in `executor_bridge.rs` still blocked (#3056).
//!
//! Expressed by adding the rule to the run's skip set rather than by branching inside
//! `FindingCollector::emit`. The collector already renders a skipped rule as `INFO:` and
//! leaves it out of `blocking_count`, which is exactly the wanted behaviour, so every
//! caller — present and future — inherits the policy without knowing it exists.
//!
//! `PR_SIZE` is removed from the skip set for the opposite reason: it governs commit shape,
//! a review contract rather than a style heuristic, and an issue may not opt out of it.
//! Rationale for both: `docs/superpowers/specs/2026-08-05-lint-gate-satisfiability-design.md`
//! Fix 5.

use std::collections::BTreeSet;

use super::{parse_guardian_skips, ImplementationLintOptions, ImplementationLintRule};

/// Build the run's skip set: the issue's own `Guardian: skip-` opt-outs, minus the rules
/// that cannot be opted out of, plus the rules that are advisory by policy.
pub(super) fn advisory_skip_set(
    options: &ImplementationLintOptions,
    issue_body: Option<&str>,
) -> BTreeSet<String> {
    let mut skipped = issue_body.map(parse_guardian_skips).unwrap_or_default();
    skipped.remove(ImplementationLintRule::PrSize.id());
    if !options.complexity_enforced {
        skipped.insert(ImplementationLintRule::Complexity.id().to_string());
    }
    skipped
}

/// Reads the environment once, so `ImplementationLintOptions::default()` carries the policy
/// and a caller that spreads `..Default::default()` cannot forget it. Tests set the field
/// directly instead of mutating the environment: `set_var` is process-global, and a lint
/// suite that races itself across threads is worse than no suite.
pub(super) fn complexity_enforced_from_env() -> bool {
    std::env::var("AUTOSPEC_COMPLEXITY_ENFORCE").is_ok_and(|value| value == "1")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lint::implementation::{
        lint_implementation, ImplementationLintContext, ImplementationLintResult,
        ImplementationLintSeverity, RepositoryIndex,
    };
    use crate::lint::parse_unified_diff;

    /// A one-line edit to a file that is already too long — the #2961 case. The rule reads
    /// the post-change file through the index, so the length is declared, not written to disk.
    fn small_edit_to_an_oversized_file(enforced: bool) -> ImplementationLintResult {
        let diff = concat!(
            "diff --git a/src/big.py b/src/big.py\n",
            "--- a/src/big.py\n",
            "+++ b/src/big.py\n",
            "@@ -1,2 +1,2 @@\n",
            "-    x = 0\n",
            "+    x = 1\n",
        );
        let diff = parse_unified_diff(diff).expect("synthetic unified diff parses");
        lint_implementation(
            &diff,
            ImplementationLintContext {
                issue_body: None,
                repository: &OversizedFile,
                options: ImplementationLintOptions {
                    complexity_enforced: enforced,
                    ..options_without_env()
                },
            },
        )
    }

    struct OversizedFile;
    impl RepositoryIndex for OversizedFile {
        fn post_change_file(&self, path: &str) -> Option<String> {
            (path == "src/big.py").then(|| {
                let mut body = String::from("def f():\n");
                for _ in 0..700 {
                    body.push_str("    x = 1\n");
                }
                body
            })
        }
    }

    /// `Default` consults the environment, which a test must not depend on: a developer
    /// exporting `AUTOSPEC_COMPLEXITY_ENFORCE=1` would otherwise flip these assertions.
    fn options_without_env() -> ImplementationLintOptions {
        ImplementationLintOptions {
            complexity_enforced: false,
            ..ImplementationLintOptions::default()
        }
    }

    fn complexity_findings(
        result: &ImplementationLintResult,
    ) -> Vec<(&'static str, ImplementationLintSeverity)> {
        result
            .findings
            .iter()
            .filter(|finding| finding.rule == ImplementationLintRule::Complexity)
            .map(|finding| (finding.rule_id(), finding.severity))
            .collect()
    }

    #[test]
    fn complexity_is_advisory_unless_enforcement_is_requested() {
        let advisory = small_edit_to_an_oversized_file(false);
        let reported = complexity_findings(&advisory);
        assert!(
            !reported.is_empty(),
            "the finding must still be reported, only not blocking"
        );
        assert!(
            reported
                .iter()
                .all(|(_, severity)| *severity == ImplementationLintSeverity::Info),
            "{reported:?}"
        );
        assert_eq!(advisory.blocking_count, 0);
        assert_eq!(advisory.exit_code(), 0);
    }

    #[test]
    fn enforcement_restores_the_blocking_finding() {
        let enforced = small_edit_to_an_oversized_file(true);
        let reported = complexity_findings(&enforced);
        assert!(
            reported
                .iter()
                .any(|(_, severity)| *severity == ImplementationLintSeverity::Error),
            "{reported:?}"
        );
        assert!(enforced.blocking_count > 0);
        assert!(enforced.exit_code() > 0);
    }

    #[test]
    fn the_two_implementations_agree_on_which_rules_an_issue_may_wave_through() {
        // PR_SIZE governs commit shape and is not opt-outable, whatever the issue says —
        // the shell drops it from the skip set for the same reason.
        let body = "Guardian: skip-PR_SIZE # generated migration\nGuardian: skip-TODO_LEFT # tracked\n";
        let advisory = advisory_skip_set(&options_without_env(), Some(body));
        assert!(!advisory.contains(ImplementationLintRule::PrSize.id()));
        assert!(advisory.contains(ImplementationLintRule::TodoLeft.id()));
        assert!(advisory.contains(ImplementationLintRule::Complexity.id()));

        let enforced = advisory_skip_set(
            &ImplementationLintOptions {
                complexity_enforced: true,
                ..options_without_env()
            },
            Some(body),
        );
        assert!(!enforced.contains(ImplementationLintRule::Complexity.id()));
        assert!(enforced.contains(ImplementationLintRule::TodoLeft.id()));
    }
}
