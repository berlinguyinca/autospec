//! Regression tests for the `BINARY_COMMITTED` implementation lint rule
//! (issue #4645): a commit that adds an unreviewable blob — a binary file, or
//! an executable (mode 100755) that is not a text script — is rejected.
//!
//! These live in their own file so the already-oversized
//! `implementation_lint.rs` is not grown (file-size ratchet).

use autospec_core::lint::{
    commit_blocking_rules, directive_for, lint_implementation, parse_unified_diff,
    ImplementationLintContext, ImplementationLintOptions, ImplementationLintResult,
    ImplementationLintRule, ImplementationLintSeverity, RepositoryIndex, UnifiedDiff,
};

/// A repository index that knows nothing; the binary/executable detector does
/// not consult it.
struct EmptyRepository;
impl RepositoryIndex for EmptyRepository {}

fn lint(diff: &str) -> ImplementationLintResult {
    let diff: UnifiedDiff = parse_unified_diff(diff).expect("synthetic diff parses");
    lint_implementation(
        &diff,
        ImplementationLintContext {
            issue_body: None,
            repository: &EmptyRepository,
            options: ImplementationLintOptions::default(),
        },
    )
}

/// A git diff for a new file declared with the given mode.
fn new_file(mode: &str, path: &str, additions: &str) -> String {
    format!(
        "diff --git a/{path} b/{path}\nnew file mode {mode}\n--- /dev/null\n+++ b/{path}\n@@ -0,0 +1,1 @@\n{additions}"
    )
}

fn rule_ids(result: &ImplementationLintResult) -> Vec<&'static str> {
    result.findings.iter().map(|f| f.rule_id()).collect()
}

#[test]
fn a_committed_binary_is_rejected() {
    let diff = "diff --git a/iw-slurm-exporter b/iw-slurm-exporter\nnew file mode 100755\n--- /dev/null\n+++ b/iw-slurm-exporter\nGIT binary patch\nliteral 8\nzcmeXz...\n";
    let result = lint(diff);

    assert!(rule_ids(&result).contains(&"BINARY_COMMITTED"));
    let binary = result
        .findings
        .iter()
        .find(|f| f.rule_id() == "BINARY_COMMITTED")
        .expect("BINARY_COMMITTED finding");
    assert_eq!(binary.severity, ImplementationLintSeverity::Error);
    assert!(binary.path.ends_with("iw-slurm-exporter"));
}

#[test]
fn a_binary_reported_as_binary_files_is_rejected() {
    let diff = "diff --git a/blob.dat b/blob.dat\nnew file mode 100644\n--- /dev/null\n+++ b/blob.dat\nBinary files /dev/null and b/blob.dat differ\n";
    let result = lint(diff);

    assert!(rule_ids(&result).contains(&"BINARY_COMMITTED"));
    let binary = result
        .findings
        .iter()
        .find(|f| f.rule_id() == "BINARY_COMMITTED")
        .expect("BINARY_COMMITTED finding");
    assert_eq!(binary.severity, ImplementationLintSeverity::Error);
    assert!(result.blocking_count >= 1);
}

#[test]
fn a_new_executable_without_a_shebang_is_rejected() {
    // `go build ./cmd/x` writes an ELF with mode 100755 and no shebang.
    let diff = new_file("100755", "cmd/x", "+not a shebang\n");
    let result = lint(&diff);

    assert_eq!(rule_ids(&result), ["BINARY_COMMITTED"]);
    assert_eq!(result.blocking_count, 1);
}

#[test]
fn an_executable_text_script_with_a_shebang_is_accepted() {
    let diff = new_file("100755", "scripts/deploy.sh", "+#!/usr/bin/env bash\n");
    let result = lint(&diff);

    assert!(
        rule_ids(&result).is_empty(),
        "a shebang text script is reviewable: {:?}",
        result.findings
    );
}

#[test]
fn a_new_regular_source_file_is_accepted() {
    let diff = new_file("100644", "src/lib.rs", "+pub fn main() {}\n");
    let result = lint(&diff);

    assert!(
        rule_ids(&result).is_empty(),
        "a plain source file is not a committed artefact: {:?}",
        result.findings
    );
}

#[test]
fn binary_committed_is_a_commit_blocking_rule() {
    let rules = commit_blocking_rules();
    let binary = rules
        .iter()
        .find(|rule| rule.rule_id == "BINARY_COMMITTED")
        .expect("BINARY_COMMITTED is commit-blocking");
    assert!(!binary.acceptance.trim().is_empty());
}

#[test]
fn the_directive_for_binary_committed_tells_the_agent_to_stage_by_name() {
    let directive = directive_for(ImplementationLintRule::BinaryCommitted);
    assert!(
        directive.contains("never `git add -A`"),
        "directive must advise staging by name: {directive}"
    );
    assert!(
        directive.contains(".gitignore"),
        "directive must name the .gitignore remedy: {directive}"
    );
}
