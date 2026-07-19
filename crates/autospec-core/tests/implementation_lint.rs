use std::collections::BTreeMap;

use autospec_core::lint::{
    directive_for, lint_implementation, parse_unified_diff, ImplementationLintContext,
    ImplementationLintOptions, ImplementationLintRule, ImplementationLintSeverity, RepositoryIndex,
};

#[derive(Default)]
struct TestRepository {
    helper_definitions: BTreeMap<String, String>,
    external_callers: BTreeMap<String, usize>,
    file_contents: BTreeMap<String, String>,
}

impl RepositoryIndex for TestRepository {
    fn helper_definition(&self, function_name: &str, excluding_path: &str) -> Option<String> {
        self.helper_definitions
            .get(function_name)
            .filter(|path| path.as_str() != excluding_path)
            .cloned()
    }

    fn external_caller_count(&self, stem: &str, _excluding_path: &str) -> Option<usize> {
        self.external_callers.get(stem).copied()
    }

    fn post_change_file(&self, path: &str) -> Option<String> {
        self.file_contents.get(path).cloned()
    }
}

fn lint(
    diff: &str,
    issue_body: Option<&str>,
    repository: &TestRepository,
    options: ImplementationLintOptions,
) -> autospec_core::lint::ImplementationLintResult {
    let diff = parse_unified_diff(diff).expect("synthetic unified diff parses");
    lint_implementation(
        &diff,
        ImplementationLintContext {
            issue_body,
            repository,
            options,
        },
    )
}

fn rules(result: &autospec_core::lint::ImplementationLintResult) -> Vec<&'static str> {
    result
        .findings
        .iter()
        .map(|finding| finding.rule_id())
        .collect()
}

fn new_file(path: &str, lines: &[&str]) -> String {
    let additions = lines
        .iter()
        .map(|line| format!("+{line}\n"))
        .collect::<String>();
    format!(
        "diff --git a/{path} b/{path}\nnew file mode 100644\n--- /dev/null\n+++ b/{path}\n@@ -0,0 +1,{} @@\n{additions}",
        lines.len()
    )
}

fn implementation_issue(outline: &str, tests: &str) -> String {
    format!(
        "## Goal\nAdd policy coverage.\n\n## Implementation outline\n{outline}\n\n## Tests required\n{tests}\n"
    )
}

#[test]
fn implementation_lint_reports_scope_and_missing_test_requirements() {
    let issue = implementation_issue("- `src/allowed.rs`", "- [ ] integration coverage");
    let diff = new_file("src/outside.rs", &["pub fn change() {}"]);

    let result = lint(
        &diff,
        Some(&issue),
        &TestRepository::default(),
        ImplementationLintOptions::default(),
    );

    assert_eq!(rules(&result), ["OUT_OF_SCOPE", "MISSING_TEST"]);
}

#[test]
fn implementation_lint_reads_outline_after_multiple_prior_sections() {
    let issue = "## Goal\nAdd coverage.\n\n## Context\nKeep the issue contract explicit.\n\n## Files to read first\n- `src/allowed.rs`\n\n## Implementation scope\n- Update the implementation.\n\n## Implementation outline\n- Update `src/allowed.rs`.\n\n## Tests required\n- `cargo test -p autospec-core --test implementation_lint`\n";
    let diff = new_file("src/allowed.rs", &["pub fn covered() {}"]);

    let result = lint(
        &diff,
        Some(issue),
        &TestRepository::default(),
        ImplementationLintOptions::default(),
    );

    assert!(!rules(&result).contains(&"OUT_OF_SCOPE"));
}

#[test]
fn implementation_lint_reports_complexity_security_todo_mock_and_docs() {
    let deeply_nested = "                    if nested {";
    let diff = format!(
        "{}{}{}{}",
        new_file(
            "src/policy.sh",
            &[
                deeply_nested,
                "eval(input)",
                "# TODO remove this",
                "command --new-policy enabled"
            ],
        ),
        new_file("tests/unit/db_test.rs", &["let database = mock(database);"],),
        new_file("src/extra.rs", &["let value = 1;"]),
        new_file("src/another.rs", &["let value = 2;"]),
    );

    let result = lint(
        &diff,
        None,
        &TestRepository::default(),
        ImplementationLintOptions::default(),
    );

    assert_eq!(
        rules(&result),
        [
            "COMPLEXITY",
            "SECURITY",
            "TODO_LEFT",
            "MOCK_DB",
            "DOC_OUT_OF_SYNC",
        ]
    );
}

#[test]
fn implementation_lint_ignores_removed_and_context_lines() {
    let diff = "diff --git a/src/clean.rs b/src/clean.rs\n--- a/src/clean.rs\n+++ b/src/clean.rs\n@@ -1,3 +1,3 @@\n-// TODO old\n // TODO context\n+let value = 1;\n";

    let result = lint(
        diff,
        None,
        &TestRepository::default(),
        ImplementationLintOptions::default(),
    );

    assert!(result.findings.is_empty());
}

#[test]
fn implementation_lint_ignores_heredoc_indentation_but_flags_real_nesting() {
    let heredoc = new_file(
        "scripts/render.sh",
        &[
            "render() {",
            "    cat <<'EOF'",
            "                            template content",
            "EOF",
            "}",
        ],
    );
    let nested = new_file("scripts/deep.sh", &["                    if nested {"]);

    let heredoc_result = lint(
        &heredoc,
        None,
        &TestRepository::default(),
        ImplementationLintOptions::default(),
    );
    let nested_result = lint(
        &nested,
        None,
        &TestRepository::default(),
        ImplementationLintOptions::default(),
    );

    assert!(!rules(&heredoc_result).contains(&"COMPLEXITY"));
    assert_eq!(rules(&nested_result), ["COMPLEXITY"]);
}

#[test]
fn implementation_lint_uses_injected_post_change_snapshots_for_file_complexity() {
    let mut repository = TestRepository::default();
    repository
        .file_contents
        .insert("src/large.rs".to_string(), "line\n".repeat(401));
    let diff = new_file("src/large.rs", &["pub fn touch() {}"]);

    let result = lint(
        &diff,
        None,
        &repository,
        ImplementationLintOptions::default(),
    );

    assert_eq!(rules(&result), ["COMPLEXITY"]);
}

#[test]
fn implementation_lint_reports_every_vacuous_test_rule() {
    let diff = format!(
        "{}{}{}{}{}{}",
        new_file(
            "tests/unit/test_grep.bats",
            &["grep -qv 'bad' result.txt || true"],
        ),
        new_file(
            "tests/unit/test_or_true.bats",
            &["[ \"$status\" -eq 0 ] || true"],
        ),
        new_file(
            "tests/unit/test_tautology.js",
            &["expect(true).toBe(true);"],
        ),
        new_file("tests/ac/test_stub.bats", &["skip \"auto-stub\""],),
        new_file("tests/unit/test_empty.js", &["it(\"empty\", () => {})"],),
        new_file(
            "tests/unit/test_no_assert.bats",
            &["@test \"no assert\" {", "  echo no-op", "}"],
        ),
    );
    let result = lint(
        &diff,
        None,
        &TestRepository::default(),
        ImplementationLintOptions {
            enable_vacuous_assertions: true,
            ..ImplementationLintOptions::default()
        },
    );

    assert_eq!(
        rules(&result),
        [
            "VACUOUS_GREP_INVERSE_OR_TRUE",
            "VACUOUS_OR_TRUE",
            "VACUOUS_TAUTOLOGY",
            "VACUOUS_AC_STUB",
            "VACUOUS_EMPTY_TEST",
            "VACUOUS_NO_ASSERT",
        ]
    );
}

#[test]
fn implementation_lint_reports_assertion_density_when_enabled() {
    let diff = new_file(
        "tests/unit/test_density.bats",
        &["@test \"density\" {", "  echo no assertion", "}"],
    );
    let result = lint(
        &diff,
        None,
        &TestRepository::default(),
        ImplementationLintOptions {
            enable_assertion_density: true,
            ..ImplementationLintOptions::default()
        },
    );

    assert_eq!(rules(&result), ["ASSERTION_DENSITY"]);
}

#[test]
fn implementation_lint_pre_commit_mode_enables_vacuous_and_density_gates() {
    let diff = format!(
        "{}{}",
        new_file(
            "tests/unit/test_pre_commit.bats",
            &["grep -qv 'bad' result.txt || true"],
        ),
        new_file(
            "tests/unit/test_density.bats",
            &["@test \"density\" {", "  echo no assertion", "}"],
        ),
    );
    let result = lint(
        &diff,
        None,
        &TestRepository::default(),
        ImplementationLintOptions {
            pre_commit_mode: true,
            ..ImplementationLintOptions::default()
        },
    );

    assert_eq!(
        rules(&result),
        [
            "VACUOUS_GREP_INVERSE_OR_TRUE",
            "VACUOUS_NO_ASSERT",
            "ASSERTION_DENSITY",
        ]
    );
}

#[test]
fn implementation_lint_reports_repository_indexed_reuse_rules() {
    let mut repository = TestRepository::default();
    repository.helper_definitions.insert(
        "parse_config".to_string(),
        "scripts/lib/config-utils.sh".to_string(),
    );
    repository
        .external_callers
        .insert("session-manager".to_string(), 1);
    let diff = format!(
        "{}{}{}",
        new_file("scripts/new-loader.sh", &["parse_config() { echo new; }"]),
        new_file("requirements.txt", &["requests==2.31.0"]),
        new_file("scripts/session-manager.sh", &["manage_session() { :; }"]),
    );

    let result = lint(
        &diff,
        None,
        &repository,
        ImplementationLintOptions {
            enable_reuse_lens: true,
            ..ImplementationLintOptions::default()
        },
    );

    assert_eq!(
        rules(&result),
        [
            "REINVENT_REPO_UTIL",
            "NEW_DEP_UNJUSTIFIED",
            "NEW_ABSTRACTION_SINGLE_CALLER",
        ]
    );
}

#[test]
fn implementation_lint_honors_valid_guardian_skips_and_functional_inline_allows() {
    let issue = "Guardian: skip-SECURITY # independently reviewed test fixture";
    let mut repository = TestRepository::default();
    repository.file_contents.insert(
        "requirements.txt".to_string(),
        "# linter:allow-NEW_DEP_UNJUSTIFIED fixture documents the dependency\nrequests==2.31.0\n"
            .to_string(),
    );
    let diff = format!(
        "{}{}",
        new_file("src/policy.sh", &["eval(input)"]),
        "diff --git a/requirements.txt b/requirements.txt\n--- a/requirements.txt\n+++ b/requirements.txt\n@@ -1 +1,2 @@\n # linter:allow-NEW_DEP_UNJUSTIFIED fixture documents the dependency\n+requests==2.31.0\n",
    );
    let result = lint(
        &diff,
        Some(issue),
        &repository,
        ImplementationLintOptions {
            enable_reuse_lens: true,
            ..ImplementationLintOptions::default()
        },
    );

    assert!(result
        .findings
        .iter()
        .all(|finding| finding.severity == ImplementationLintSeverity::Info));
    assert_eq!(rules(&result), ["SECURITY", "NEW_DEP_UNJUSTIFIED"]);
}

#[test]
fn implementation_lint_rejects_reasonless_guardian_skips() {
    let diff = new_file("src/policy.sh", &["eval(input)"]);
    let result = lint(
        &diff,
        Some("Guardian: skip-SECURITY"),
        &TestRepository::default(),
        ImplementationLintOptions::default(),
    );

    assert_eq!(rules(&result), ["SECURITY"]);
    assert_eq!(
        result.findings[0].severity,
        ImplementationLintSeverity::Error
    );
}

#[test]
fn implementation_lint_rejects_guardian_syntax_that_the_shell_does_not_accept() {
    let diff = new_file("src/policy.sh", &["eval(input)"]);
    for invalid in [
        "Guardian:skip-SECURITY # two characters",
        "Guardian: skip-SECURITY # x",
        "Guardian: skip-SECURITY, invalid # two characters",
    ] {
        let result = lint(
            &diff,
            Some(invalid),
            &TestRepository::default(),
            ImplementationLintOptions::default(),
        );
        assert_eq!(
            result.findings[0].severity,
            ImplementationLintSeverity::Error
        );
    }
}

#[test]
fn implementation_lint_matches_guardian_comma_and_reason_whitespace_rules() {
    let diff = new_file("src/policy.sh", &["eval(input)"]);
    let valid = lint(
        &diff,
        Some("Guardian: skip-SECURITY # x "),
        &TestRepository::default(),
        ImplementationLintOptions::default(),
    );
    assert_eq!(valid.findings[0].severity, ImplementationLintSeverity::Info);

    let invalid = lint(
        &diff,
        Some("Guardian: skip-SECURITY , skip-TODO_LEFT # two characters"),
        &TestRepository::default(),
        ImplementationLintOptions::default(),
    );
    assert_eq!(
        invalid.findings[0].severity,
        ImplementationLintSeverity::Error
    );
}

#[test]
fn implementation_lint_caps_repeated_rule_findings_without_dropping_the_truncation_marker() {
    let lines = (1..=15)
        .map(|index| format!("# TODO item {index}"))
        .collect::<Vec<_>>();
    let line_references = lines.iter().map(String::as_str).collect::<Vec<_>>();
    let diff = new_file("src/todos.sh", &line_references);
    let result = lint(
        &diff,
        None,
        &TestRepository::default(),
        ImplementationLintOptions::default(),
    );

    assert_eq!(rules(&result), vec!["TODO_LEFT"; 11]);
    assert_eq!(result.blocking_count, 11);
    assert!(result.findings[10].message.contains("+ more (truncated)"));
    assert_eq!(result.exit_code(), 11);
}

#[test]
fn implementation_lint_reports_scope_explosion_for_an_explicit_aggregate_cap() {
    let diff = new_file(
        "src/policy.sh",
        &[
            "eval(input)",
            "# TODO aggregate cap",
            "command --new-policy enabled",
        ],
    );
    let result = lint(
        &diff,
        None,
        &TestRepository::default(),
        ImplementationLintOptions {
            aggregate_hard_cap: 2,
            ..ImplementationLintOptions::default()
        },
    );

    assert!(result.scope_exploded);
    assert_eq!(result.exit_code(), 200);
    assert_eq!(result.blocking_count, 2);
    assert_eq!(rules(&result), ["SECURITY", "TODO_LEFT", "OUT_OF_SCOPE"]);
    assert_eq!(
        result.findings[2].message,
        "too many findings — likely scope explosion"
    );
}

#[test]
fn implementation_lint_uses_shell_security_pass_order_and_multiplicity() {
    let diff = format!(
        "{}{}",
        new_file("src/first.sh", &["exec(input)"]),
        new_file("src/second.sh", &["eval(input); exec(input)"]),
    );
    let result = lint(
        &diff,
        None,
        &TestRepository::default(),
        ImplementationLintOptions::default(),
    );

    assert_eq!(rules(&result), ["SECURITY", "SECURITY", "SECURITY"]);
    assert_eq!(
        result
            .findings
            .iter()
            .map(|finding| finding.path.as_str())
            .collect::<Vec<_>>(),
        ["src/second.sh", "src/first.sh", "src/second.sh"],
    );
}

#[test]
fn implementation_lint_keeps_plain_heredoc_open_until_a_column_zero_terminator() {
    let diff = new_file(
        "scripts/heredoc.sh",
        &[
            "cat <<EOF",
            "  EOF",
            "                    if still_literal {",
            "EOF",
            "                    if real_code {",
        ],
    );
    let result = lint(
        &diff,
        None,
        &TestRepository::default(),
        ImplementationLintOptions::default(),
    );

    assert_eq!(rules(&result), ["COMPLEXITY"]);
}

#[test]
fn implementation_lint_uses_python_control_nesting_not_plain_indentation() {
    let diff = new_file(
        "src/formatting.py",
        &[
            "def render():",
            "                    continuation = call(",
            "                        value,",
            "                    )",
        ],
    );
    let result = lint(
        &diff,
        None,
        &TestRepository::default(),
        ImplementationLintOptions::default(),
    );

    assert!(!rules(&result).contains(&"COMPLEXITY"));
}

#[test]
fn implementation_lint_uses_post_change_python_controls_for_nesting() {
    let diff = new_file("src/nesting.py", &["def nested():"]);
    let mut repository = TestRepository::default();
    repository.file_contents.insert(
        "src/nesting.py".to_string(),
        concat!(
            "def nested():\n",
            "    if first:\n",
            "        for second in values:\n",
            "            while third:\n",
            "                with fourth:\n",
            "                    if fifth:\n",
            "                        return\n",
        )
        .to_string(),
    );
    let result = lint(
        &diff,
        None,
        &repository,
        ImplementationLintOptions::default(),
    );

    assert_eq!(rules(&result), ["COMPLEXITY"]);
}

#[test]
fn implementation_lint_preserves_outer_python_controls_for_nested_definitions() {
    let diff = new_file("src/inherited.py", &["def placeholder():"]);
    let mut repository = TestRepository::default();
    repository.file_contents.insert(
        "src/inherited.py".to_string(),
        concat!(
            "if enabled:\n",
            "    def nested():\n",
            "        if first:\n",
            "            for second in values:\n",
            "                while third:\n",
            "                    with fourth:\n",
            "                        return\n",
        )
        .to_string(),
    );
    let result = lint(
        &diff,
        None,
        &repository,
        ImplementationLintOptions::default(),
    );

    assert_eq!(rules(&result), ["COMPLEXITY"]);
}

#[test]
fn implementation_lint_counts_python_definitions_and_elif_as_ast_depth() {
    let diff = new_file("src/elif_depth.py", &["def placeholder():"]);
    let mut repository = TestRepository::default();
    repository.file_contents.insert(
        "src/elif_depth.py".to_string(),
        concat!(
            "def nested():\n",
            "    if first:\n",
            "        pass\n",
            "    elif second:\n",
            "        if third:\n",
            "            for fourth in values:\n",
            "                return\n",
        )
        .to_string(),
    );
    let result = lint(
        &diff,
        None,
        &repository,
        ImplementationLintOptions::default(),
    );

    assert_eq!(rules(&result), ["COMPLEXITY"]);
}

#[test]
fn implementation_lint_counts_nested_async_definitions_as_ast_depth() {
    let diff = new_file("src/nested_defs.py", &["def placeholder():"]);
    let mut repository = TestRepository::default();
    repository.file_contents.insert(
        "src/nested_defs.py".to_string(),
        concat!(
            "def first():\n",
            "    def second():\n",
            "        def third():\n",
            "            def fourth():\n",
            "                async def fifth():\n",
            "                    return\n",
        )
        .to_string(),
    );
    let result = lint(
        &diff,
        None,
        &repository,
        ImplementationLintOptions::default(),
    );

    assert_eq!(rules(&result), ["COMPLEXITY"]);
}

#[test]
fn implementation_lint_does_not_count_python_match_as_ast_depth() {
    let diff = new_file("src/match_depth.py", &["def placeholder():"]);
    let mut repository = TestRepository::default();
    repository.file_contents.insert(
        "src/match_depth.py".to_string(),
        concat!(
            "def nested(value):\n",
            "    match value:\n",
            "        case _:\n",
            "            if first:\n",
            "                for second in values:\n",
            "                    while third:\n",
            "                        return\n",
        )
        .to_string(),
    );
    let result = lint(
        &diff,
        None,
        &repository,
        ImplementationLintOptions::default(),
    );

    assert!(!rules(&result).contains(&"COMPLEXITY"));
}

#[test]
fn implementation_lint_runs_python_snapshot_nesting_in_file_order() {
    let diff = format!(
        "{}{}",
        new_file("src/first.py", &["def nested():"]),
        new_file("scripts/second.sh", &["                    if nested {"]),
    );
    let mut repository = TestRepository::default();
    repository.file_contents.insert(
        "src/first.py".to_string(),
        concat!(
            "def nested():\n",
            "    if first:\n",
            "        for second in values:\n",
            "            while third:\n",
            "                with fourth:\n",
            "                    if fifth:\n",
        )
        .to_string(),
    );
    let result = lint(
        &diff,
        None,
        &repository,
        ImplementationLintOptions::default(),
    );

    assert_eq!(
        result
            .findings
            .iter()
            .map(|finding| finding.path.as_str())
            .collect::<Vec<_>>(),
        ["scripts/second.sh", "src/first.py"],
    );
}

#[test]
fn implementation_lint_keeps_global_complexity_phase_order_for_snapshots() {
    let diff = format!(
        "{}{}",
        new_file("scripts/first.sh", &["                    if first {"]),
        new_file("scripts/second.sh", &["                    if second {"]),
    );
    let mut repository = TestRepository::default();
    repository
        .file_contents
        .insert("scripts/first.sh".to_string(), "line\n".repeat(401));
    let result = lint(
        &diff,
        None,
        &repository,
        ImplementationLintOptions::default(),
    );

    assert_eq!(
        result
            .findings
            .iter()
            .map(|finding| finding.path.as_str())
            .collect::<Vec<_>>(),
        ["scripts/first.sh", "scripts/second.sh", "scripts/first.sh"],
    );
}

#[test]
fn implementation_lint_matches_manifest_tautology_and_cli_flag_boundaries() {
    let dependencies = format!(
        "{}{}{}{}{}{}",
        new_file("requirements.txt", &["requests==2.31.0"]),
        new_file("package.json", &["\"requests\": \"^1.0\""]),
        new_file("go.mod", &["example.dev/request v1.0.0"]),
        new_file("Cargo.toml", &["requests = \"1.0\""]),
        new_file("pyproject.toml", &["requests>=1.0"]),
        new_file("Gemfile", &["gem 'requests'"]),
    );
    let dependency_result = lint(
        &dependencies,
        None,
        &TestRepository::default(),
        ImplementationLintOptions {
            enable_reuse_lens: true,
            ..ImplementationLintOptions::default()
        },
    );
    assert_eq!(rules(&dependency_result), vec!["NEW_DEP_UNJUSTIFIED"; 6]);

    let tautology = new_file(
        "tests/unit/test_tautology.js",
        &[
            "expect(true).toStrictEqual(true);",
            "assert True # not shell tautology",
        ],
    );
    let tautology_result = lint(
        &tautology,
        None,
        &TestRepository::default(),
        ImplementationLintOptions {
            enable_vacuous_assertions: true,
            ..ImplementationLintOptions::default()
        },
    );
    assert_eq!(rules(&tautology_result), ["VACUOUS_TAUTOLOGY"]);
    assert_eq!(tautology_result.findings[0].line, Some(1));

    let flags = new_file(
        "src/flags.sh",
        &[
            "command --ab,",
            "command --not-a-flag,",
            "command ---not-a-flag=value",
            "command --valid-flag=value",
        ],
    );
    let flags_result = lint(
        &flags,
        None,
        &TestRepository::default(),
        ImplementationLintOptions::default(),
    );
    assert_eq!(rules(&flags_result), ["DOC_OUT_OF_SYNC"]);
}

#[test]
fn implementation_lint_rejects_near_miss_manifest_and_public_surface_patterns() {
    let dependencies = format!(
        "{}{}{}{}{}{}",
        new_file("requirements.txt", &["# comment"]),
        new_file("package.json", &["\"requests\": \"latest\""]),
        new_file("go.mod", &["example.dev/request latest"]),
        new_file("Cargo.toml", &["Requests = \"1.0\""]),
        new_file("pyproject.toml", &["requests value"]),
        new_file("Gemfile", &["source 'https://rubygems.org'"]),
    );
    let dependency_result = lint(
        &dependencies,
        None,
        &TestRepository::default(),
        ImplementationLintOptions {
            enable_reuse_lens: true,
            ..ImplementationLintOptions::default()
        },
    );
    assert!(dependency_result.findings.is_empty());

    let flags = new_file(
        "src/flags.sh",
        &[
            "command --not-a-flag,",
            "--ab,",
            "---not-a-flag=value",
            " export BUILD_MODE=debug",
        ],
    );
    let result = lint(
        &flags,
        None,
        &TestRepository::default(),
        ImplementationLintOptions::default(),
    );
    assert!(result.findings.is_empty());
}

#[test]
fn implementation_lint_matches_private_key_doc_and_pyproject_boundaries() {
    let keys = new_file(
        "docs/keys.txt",
        &[
            "-----BEGIN rsa PRIVATE KEY-----",
            "-----BEGIN RSA PRIVATE KEY-----",
        ],
    );
    let key_result = lint(
        &keys,
        None,
        &TestRepository::default(),
        ImplementationLintOptions::default(),
    );
    assert_eq!(rules(&key_result), ["SECURITY"]);

    let doc_surface = format!(
        "{}{}",
        new_file(
            "src/upper.sh",
            &["command --UPPER=value", "command --ab=value"]
        ),
        new_file("src/env.sh", &["FOO_=1"]),
    );
    let doc_result = lint(
        &doc_surface,
        None,
        &TestRepository::default(),
        ImplementationLintOptions::default(),
    );
    assert!(doc_result.findings.is_empty());

    let quoted_pyproject = new_file("pyproject.toml", &["\"requests>=1.0\""]);
    let pyproject_result = lint(
        &quoted_pyproject,
        None,
        &TestRepository::default(),
        ImplementationLintOptions {
            enable_reuse_lens: true,
            ..ImplementationLintOptions::default()
        },
    );
    assert_eq!(rules(&pyproject_result), ["NEW_DEP_UNJUSTIFIED"]);
}

#[test]
fn implementation_lint_only_treats_lower_or_titlecase_why_as_a_justification() {
    let diff = new_file(
        "requirements.txt",
        &["# WHY: uppercase is not shell syntax", "requests==2.31.0"],
    );
    let result = lint(
        &diff,
        None,
        &TestRepository::default(),
        ImplementationLintOptions {
            enable_reuse_lens: true,
            ..ImplementationLintOptions::default()
        },
    );

    assert_eq!(rules(&result), ["NEW_DEP_UNJUSTIFIED"]);
}

#[test]
fn implementation_lint_uses_configured_snapshot_complexity_thresholds() {
    let diff = new_file("src/threshold.py", &["def threshold():", "    if ready:"]);
    let mut repository = TestRepository::default();
    repository.file_contents.insert(
        "src/threshold.py".to_string(),
        "def threshold():\n    if ready:\n".to_string(),
    );
    let result = lint(
        &diff,
        None,
        &repository,
        ImplementationLintOptions {
            max_file_loc: 1,
            max_function_loc: 1,
            max_cyclomatic: 0,
            ..ImplementationLintOptions::default()
        },
    );

    assert_eq!(rules(&result), vec!["COMPLEXITY"; 3]);
}

#[test]
fn implementation_lint_exposes_the_legacy_directive_text() {
    assert_eq!(
        directive_for(ImplementationLintRule::Security),
        "Remove the flagged pattern: never hardcode secrets, never bypass git hooks or use destructive resets, validate all inputs."
    );
}

#[test]
fn implementation_lint_clean_fixture_has_no_findings() {
    let diff = new_file("src/clean.rs", &["pub fn clean() { println!(\"ok\"); }"]);
    let result = lint(
        &diff,
        None,
        &TestRepository::default(),
        ImplementationLintOptions {
            enable_vacuous_assertions: true,
            enable_reuse_lens: true,
            ..ImplementationLintOptions::default()
        },
    );

    assert!(result.findings.is_empty());
    assert_eq!(result.exit_code(), 0);
}
