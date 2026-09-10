//! Deadline ratchet: `Duration::from_millis` below a threshold in
//! assertion paths is a load-sensitive false negative (issue #4080).
//!
//! A test that asserts against a 10 ms deadline passes on an idle host and
//! fails under inference load. 92 such deadlines in this repository were
//! the source of the false-negative verdicts that #4080 closes. The ratchet
//! makes that load-sensitivity visible and shrink-only: it counts
//! `Duration::from_millis(<n>)` sites below the threshold in test code, and
//! the frozen baseline is the ceiling. A new sub-threshold deadline in an
//! assertion path is a violation until the author either raises the deadline
//! past the threshold or deletes a baseline line to absorb it.
//!
//! 1. **The threshold** is [`DEADLINE_THRESHOLD_MS`] (50 ms). A deadline at
//!    or above it is load-tolerant and is not counted; below it is a
//!    candidate false negative.
//! 2. **Scope is test code**, decided per file: a path under a `tests/`
//!    directory, a `*_test`/`*_tests` file stem, or a source carrying a
//!    `#[test]`/`#[cfg(test)]` marker. Production deadlines are out of
//!    scope — the ratchet is about verdicts, not about production timing.
//! 3. **The baseline is shrink-only** ([`DEADLINE_BASELINE`]): a
//!    per-file count ceiling frozen at the time of introduction. A file
//!    found above its baseline entry is a [`RatchetViolation`]; a file found
//!    below it is progress. Growing the baseline requires deleting a line by
//!    hand — the same discipline as [`crate::validation::external::bats_registration_baseline::BATS_REGISTRATION_BASELINE`].
//!
//! Everything here is pure text parsing — no I/O, no clock, no subprocess —
//! except the in-test repository scan, which reads only files under the
//! workspace.

use std::collections::BTreeMap;

/// A deadline at or above this many milliseconds is considered
/// load-tolerant and is not counted by the ratchet.
pub const DEADLINE_THRESHOLD_MS: u64 = 50;

/// The per-file count ceiling, frozen at introduction. Shrink-only: a
/// violation names the file and the excess; absorbing it means deleting this
/// line, not adding to it.
pub const DEADLINE_BASELINE: &[(&str, u64)] = &[
    // (path, allowed sub-threshold deadline sites)
    // Re-derived by `deadline_ratchet::tests::repository_scan_matches_baseline`.
    ("crates/autospec-cli/src/commands/autonomous.rs", 4),
    ("crates/autospec-cli/src/commands/autonomous/drain.rs", 2),
    ("crates/autospec-cli/src/commands/autonomous/executor_bridge.rs", 7),
    ("crates/autospec-cli/src/commands/autonomous/executor_bridge/portability/direct_attempt/tests.rs", 2),
    ("crates/autospec-cli/src/commands/autonomous/executor_bridge/process_owner/tests.rs", 6),
    ("crates/autospec-cli/src/commands/autonomous/executor_bridge/process_owner/unix_group.rs", 2),
    ("crates/autospec-cli/src/commands/autonomous/executor_bridge/tests/adoption_cleanup.rs", 8),
    ("crates/autospec-cli/src/commands/autonomous/executor_bridge/tests/cleanup_reap.rs", 3),
    ("crates/autospec-cli/src/commands/autonomous/executor_bridge/tests/codex_sandbox.rs", 1),
    ("crates/autospec-cli/src/commands/autonomous/executor_bridge/tests/descendant_spawn.rs", 1),
    ("crates/autospec-cli/src/commands/autonomous/executor_bridge/tests/draft_release.rs", 2),
    ("crates/autospec-cli/src/commands/autonomous/executor_bridge/tests/harness_death.rs", 7),
    ("crates/autospec-cli/src/commands/autonomous/executor_bridge/tests/harness_supervisor.rs", 9),
    ("crates/autospec-cli/src/commands/autonomous/executor_bridge/tests/json_identity.rs", 1),
    ("crates/autospec-cli/src/commands/autonomous/executor_bridge/tests/merged_reconciliation.rs", 1),
    ("crates/autospec-cli/src/commands/autonomous/executor_bridge/tests/production_entry.rs", 1),
    ("crates/autospec-cli/src/commands/autonomous/executor_bridge/tests/pull_mutation.rs", 2),
    ("crates/autospec-cli/src/commands/autonomous/executor_bridge/tests/restart_direct.rs", 3),
    ("crates/autospec-cli/src/commands/autonomous/executor_bridge/tests/reviewer_runtime.rs", 1),
    ("crates/autospec-cli/src/commands/autonomous/executor_bridge/tests/rust_commit.rs", 1),
    ("crates/autospec-cli/src/commands/autonomous/executor_bridge/tests/sidecar_launch.rs", 5),
    ("crates/autospec-cli/src/commands/autonomous/executor_bridge/tests/snapshot_identity.rs", 1),
    ("crates/autospec-cli/src/commands/autonomous/executor_bridge/tests/supervision_family_lock.rs", 1),
    ("crates/autospec-cli/src/commands/autonomous/executor_bridge/tests/support_base.rs", 2),
    ("crates/autospec-cli/src/commands/autonomous/executor_bridge/tests/support_invocation.rs", 2),
    ("crates/autospec-cli/src/commands/autonomous/executor_bridge/tests/support_shim.rs", 1),
    ("crates/autospec-cli/src/commands/autonomous/resilience.rs", 4),
    ("crates/autospec-cli/src/commands/autonomous/resilience/heartbeat_tests.rs", 3),
    ("crates/autospec-cli/src/commands/autonomous/resilience/startup_transaction.rs", 1),
    ("crates/autospec-cli/src/commands/autonomous/tier2_runner.rs", 1),
    ("crates/autospec-cli/src/commands/autonomous/tier2_runner_tests.rs", 3),
    ("crates/autospec-cli/src/commands/autonomous/waterfall.rs", 1),
    ("crates/autospec-cli/src/commands/autonomous/waterfall_tests.rs", 1),
    ("crates/autospec-cli/src/commands/runtime/env.rs", 1),
    ("crates/autospec-cli/src/commands/runtime/env/session.rs", 1),
    ("crates/autospec-cli/tests/autonomous_accountability_github/binding.rs", 1),
    ("crates/autospec-cli/tests/autonomous_accountability_github/contracts.rs", 1),
    ("crates/autospec-cli/tests/autonomous_conductor_commands.rs", 3),
    ("crates/autospec-cli/tests/autonomous_resilience_commands.rs", 3),
    ("crates/autospec-cli/tests/claim_commands.rs", 1),
    ("crates/autospec-cli/tests/cli_commands.rs", 7),
    ("crates/autospec-cli/tests/runtime_commands.rs", 3),
    ("crates/autospec-cli/tests/runtime_maven.rs", 1),
    ("crates/autospec-cli/tests/runtime_session_security.rs", 3),
    ("crates/autospec-cli/tests/runtime_sessions.rs", 4),
    ("crates/autospec-cli/tests/runtime_state_reconciliation.rs", 2),
    ("crates/autospec-cli/tests/runtime_terminal.rs", 5),
    ("crates/autospec-cli/tests/support/autonomous_conductor_process.rs", 2),
    ("crates/autospec-cli/tests/support/autonomous_lease_fixture.rs", 1),
    ("crates/autospec-cli/tests/support/autonomous_recovery_accountability.rs", 1),
    ("crates/autospec-cli/tests/support/autonomous_restart_dry_run.rs", 1),
    ("crates/autospec-cli/tests/support/resilience_fixture_support.rs", 1),
    ("crates/autospec-core/src/runtime_env/ports.rs", 1),
];

/// Find every `from_millis(<literal>)` in the source and return the literal
/// value in order of appearance. The parser is deliberately hand-rolled:
/// the crate has no regex dependency. It handles `from_millis(10)`,
/// `from_millis(2_000)`, and surrounding whitespace.
pub fn from_millis_values(source: &str) -> Vec<u64> {
    let mut values = Vec::new();
    let mut rest = source;
    const NEEDLE: &str = "from_millis(";
    while let Some(at) = rest.find(NEEDLE) {
        let after = &rest[at + NEEDLE.len()..];
        let digits: String = after
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '_')
            .collect();
        if digits.is_empty() {
            // No literal argument — a variable or expression. Not a
            // fixed deadline the ratchet can bound.
            rest = &after[..];
            continue;
        }
        if let Ok(value) = digits.replace('_', "").parse::<u64>() {
            values.push(value);
        }
        rest = &after[digits.len()..];
    }
    values
}

/// The sub-threshold deadline sites in a source, in order of appearance.
pub fn deadline_sites(source: &str) -> Vec<u64> {
    from_millis_values(source)
        .into_iter()
        .filter(|value| *value < DEADLINE_THRESHOLD_MS)
        .collect()
}

/// A file lives in test code by its path: under a `tests/` directory, or a
/// `*_test` / `*_tests` stem.
pub fn is_test_path(path: &str) -> bool {
    let normalized = path.replace('\\', "/");
    let under_tests = normalized.contains("/tests/")
        || normalized.starts_with("tests/")
        || normalized.contains("src/tests/");
    if under_tests {
        return true;
    }
    let stem = normalized.rsplit('/').next().unwrap_or(&normalized);
    let stem = stem.strip_suffix(".rs").unwrap_or(stem);
    stem.ends_with("_test") || stem.ends_with("_tests")
}

/// A file lives in test code by its contents: it carries a `#[test]` or
/// `#[cfg(test)]` marker (or a tokio test).
pub fn is_test_source(source: &str) -> bool {
    source.contains("#[test]")
        || source.contains("#[cfg(test)]")
        || source.contains("#[tokio::test]")
}

/// A file is in scope for the ratchet if its path or its contents mark it
/// as test code.
pub fn is_in_scope(path: &str, source: &str) -> bool {
    is_test_path(path) || is_test_source(source)
}

/// One baseline breach: a file found with more sub-threshold deadline sites
/// than its frozen baseline allows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RatchetViolation {
    /// The repository-relative path.
    pub path: String,
    /// The count found now.
    pub found: u64,
    /// The count the baseline allows.
    pub allowed: u64,
}

/// Compare current per-file counts against the frozen baseline. A file above
/// its allowed count is a violation; a file absent from the baseline is a
/// violation with an allowed count of zero. Files at or below their baseline
/// entry — and files no longer carrying any sites — are clean.
pub fn ratchet_violations(
    findings: &[(String, u64)],
    baseline: &[(&str, u64)],
) -> Vec<RatchetViolation> {
    let allowed: BTreeMap<&str, u64> = baseline
        .iter()
        .map(|(path, count)| (*path, *count))
        .collect();
    findings
        .iter()
        .filter_map(|(path, found)| {
            let cap = allowed.get(path.as_str()).copied().unwrap_or(0);
            if *found > cap {
                Some(RatchetViolation {
                    path: path.clone(),
                    found: *found,
                    allowed: cap,
                })
            } else {
                None
            }
        })
        .collect()
}

/// The report line for a set of violations, in the `line()` style of the
/// other verdict modules.
pub fn line(violations: &[RatchetViolation]) -> String {
    if violations.is_empty() {
        "deadline ratchet: no new sub-threshold deadlines in assertion paths".to_string()
    } else {
        format!(
            "deadline ratchet: {} file(s) above baseline: {}",
            violations.len(),
            violations
                .iter()
                .map(|v| format!("{} (found {}, allowed {})", v.path, v.found, v.allowed))
                .collect::<Vec<_>>()
                .join("; ")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    /// The parser reads fixed literals and skips non-literal arguments.
    #[test]
    fn the_parser_reads_literals_and_skips_expressions() {
        let source = "a: Duration::from_millis(10), b: Duration::from_millis(2_000), c: Duration::from_millis(n), d: Duration::from_millis(25)";
        assert_eq!(
            from_millis_values(source),
            vec![10, 2000, 25],
            "digit literals with underscores parse; the bare variable is skipped"
        );
    }

    /// The threshold filters: below 50 counts, at or above does not.
    #[test]
    fn the_threshold_filters_at_fifty() {
        assert_eq!(deadline_sites("x from_millis(40)"), vec![40]);
        assert_eq!(deadline_sites("x from_millis(50)"), Vec::<u64>::new());
        assert_eq!(
            deadline_sites("x from_millis(10) y from_millis(100)"),
            vec![10]
        );
    }

    /// Scope is decided by path or by content, and production code is out.
    #[test]
    fn scope_is_decided_by_path_or_content() {
        assert!(is_test_path("crates/autospec-core/tests/foo.rs"));
        assert!(is_test_path("tests/unit/bar.rs"));
        assert!(is_test_path("src/foo_test.rs"));
        assert!(!is_test_path("crates/autospec-core/src/foo.rs"));
        assert!(is_test_source("mod tests { #[test] fn a() {} }"));
        assert!(!is_test_source("pub fn a() {}"));
        assert!(is_in_scope("src/foo.rs", "#[test] fn a() {}"));
    }

    /// A file above its baseline is a violation; at or below is clean; a new
    /// file (absent from the baseline) is a violation with allowed zero.
    #[test]
    fn violations_are_files_above_their_baseline() {
        let findings = vec![
            ("a.rs".to_string(), 5),
            ("b.rs".to_string(), 3),
            ("c.rs".to_string(), 0),
            ("d.rs".to_string(), 1),
        ];
        let baseline = &[("a.rs", 3), ("b.rs", 3), ("c.rs", 2)];
        let violations = ratchet_violations(&findings, baseline);
        assert_eq!(
            violations,
            vec![
                RatchetViolation {
                    path: "a.rs".to_string(),
                    found: 5,
                    allowed: 3
                },
                RatchetViolation {
                    path: "d.rs".to_string(),
                    found: 1,
                    allowed: 0
                },
            ]
        );
        assert!(line(&violations).contains("a.rs (found 5, allowed 3)"));
    }

    /// A clean scan reports exactly that.
    #[test]
    fn a_clean_scan_reports_no_violations() {
        assert_eq!(
            line(&[]),
            "deadline ratchet: no new sub-threshold deadlines in assertion paths"
        );
    }

    /// Repository scan (AC2): every in-scope file's sub-threshold count is at
    /// or below its frozen baseline entry. This test reads only the files
    /// under the workspace. A failure lists the offending files and counts so
    /// the baseline can be re-derived, or the deadline raised.
    #[test]
    fn repository_scan_matches_baseline() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."))
            .join("..");
        let root = root.canonicalize().expect("workspace root resolves");

        let mut findings: BTreeMap<String, u64> = BTreeMap::new();
        scan_dir(&root, &root, &mut findings);

        let findings: Vec<(String, u64)> = findings
            .into_iter()
            .filter(|(_, count)| *count > 0)
            .collect();

        let violations = ratchet_violations(&findings, DEADLINE_BASELINE);
        if !violations.is_empty() {
            let detail = violations
                .iter()
                .map(|v| format!("{}: found {}, allowed {}", v.path, v.found, v.allowed))
                .collect::<Vec<_>>()
                .join("\n  ");
            panic!(
                "deadline ratchet violations (raise the deadline, or delete the baseline line to absorb it):\n  {detail}"
            );
        }
    }

    /// Recursively scan `.rs` files under `base`, recording per-file
    /// sub-threshold deadline counts for in-scope files, keyed by
    /// repository-relative path. `deadline_ratchet.rs` itself is excluded so
    /// the baseline does not count its own test fixtures.
    fn scan_dir(root: &std::path::Path, dir: &std::path::Path, out: &mut BTreeMap<String, u64>) {
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(_) => return,
        };
        let mut children: Vec<_> = entries.filter_map(|e| e.ok()).collect();
        children.sort_by_key(|e| e.file_name());
        for entry in children {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if path.is_dir() {
                if name == "target" || name == ".git" || name == "node_modules" {
                    continue;
                }
                scan_dir(root, &path, out);
            } else if name.ends_with(".rs") {
                let relative = path
                    .strip_prefix(root)
                    .expect("child under root")
                    .to_string_lossy()
                    .replace('\\', "/");
                if relative == "crates/autospec-core/src/deadline_ratchet.rs" {
                    continue;
                }
                let Ok(source) = fs::read_to_string(&path) else {
                    continue;
                };
                if is_in_scope(&relative, &source) {
                    let count = deadline_sites(&source).len() as u64;
                    if count > 0 {
                        *out.entry(relative).or_insert(0) += count;
                    }
                }
            }
        }
    }
}
