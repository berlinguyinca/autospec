use autospec_core::claim::RunStateRecord;
use std::io::Write;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::symlink;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static EXECUTABLE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn autospec() -> Command {
    Command::new(env!("CARGO_BIN_EXE_autospec"))
}

fn help_command_names(help: &str) -> Vec<&str> {
    help.lines()
        .skip_while(|line| line.trim() != "COMMANDS:")
        .skip(1)
        .take_while(|line| !line.trim().is_empty() && line.trim() != "OPTIONS:")
        .filter_map(|line| line.split_whitespace().next())
        .collect()
}

fn help_usage_invocation(help: &str) -> Option<&str> {
    help.lines()
        .skip_while(|line| line.trim() != "USAGE:")
        .skip(1)
        .map(str::trim)
        .find(|line| !line.is_empty())
}

/// Parses lint finding lines of the form `CODE:path:line: message` and
/// returns the set of finding codes present. Parsing the structured
/// `CODE:` prefix (rather than searching for the code as a raw substring
/// anywhere in stdout) avoids false matches between codes that share a
/// suffix, e.g. `VACUOUS_GREP_INVERSE_OR_TRUE` vs `VACUOUS_OR_TRUE`.
fn lint_finding_codes(stdout: &str) -> std::collections::HashSet<&str> {
    stdout
        .lines()
        .filter_map(|line| line.split_once(':').map(|(code, _)| code))
        .collect()
}

#[test]
fn help_command_table_parser_returns_command_column_only() {
    let help = "COMMANDS:\n    init           Initialize AutoSpec metadata\n    growth-report  Render metrics\n\nOPTIONS:\n    -h, --help       Print help\n";

    assert_eq!(help_command_names(help), ["init", "growth-report"]);
}

#[test]
fn help_usage_parser_returns_first_usage_invocation() {
    let help = "USAGE:\n    autospec lint issue [--json] <BODY_PATH>\n\nOPTIONS:\n    -h, --help       Print help\n";

    assert_eq!(
        help_usage_invocation(help),
        Some("autospec lint issue [--json] <BODY_PATH>")
    );
}

#[test]
fn cli_commands_help_lists_required_commands() {
    let output = autospec().arg("--help").output().expect("autospec runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert_eq!(
        help_command_names(&stdout),
        [
            "init",
            "lint",
            "claim",
            "parent",
            "queue",
            "doctor",
            "status",
            "autonomous",
            "plan",
            "validate",
            "run",
            "runtime",
            "resume",
            "report",
            "showcase",
            "benchmark",
            "growth-report",
        ]
    );
}

#[test]
fn lint_issue_file_reports_text_findings_to_stderr() {
    let body = issue_body(
        "improve the issue body.",
        "- [ ] `cargo test issue_lint` passes.",
        "cargo test issue_lint",
    );
    let path = write_issue_body("autospec-lint-issue-text", &body);

    let output = autospec()
        .args(["lint", "issue", path.to_str().unwrap()])
        .output()
        .expect("autospec lint issue runs");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "GOAL_VAGUE: Bare vague verb 'improve' used without a concrete object (path, backtick term, number, or UPPER_SNAKE label)\n"
    );
}

#[test]
fn lint_issue_json_writes_ordered_findings_to_stdout() {
    let body = issue_body(
        "improve the issue should be clear.",
        "- [ ] `cargo test issue_lint` passes.",
        "cargo test issue_lint",
    );
    let path = write_issue_body("autospec-lint-issue-json", &body);

    let output = autospec()
        .args(["lint", "issue", "--json", path.to_str().unwrap()])
        .output()
        .expect("autospec lint issue runs");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "[\n  {\"rule\":\"GOAL_VAGUE\",\"description\":\"Bare vague verb 'improve' used without a concrete object (path, backtick term, number, or UPPER_SNAKE label)\"},\n  {\"rule\":\"GOAL_HEDGE\",\"description\":\"Hedging word 'should' found in Goal section; state the outcome flatly\"}\n]\n"
    );
}

#[test]
fn lint_issue_reads_stdin_when_body_path_is_dash() {
    let body = issue_body(
        "improve the issue body.",
        "- [ ] `cargo test issue_lint` passes.",
        "cargo test issue_lint",
    );

    let output = autospec_with_stdin(["lint", "issue", "-"], &body);

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).starts_with("GOAL_VAGUE: "));
}

#[test]
fn lint_issue_help_exits_successfully_without_diagnostics() {
    let output = autospec()
        .args(["lint", "issue", "--help"])
        .output()
        .expect("autospec lint issue help runs");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(
        help_usage_invocation(&String::from_utf8_lossy(&output.stdout)),
        Some("autospec lint issue [--json] <BODY_PATH>")
    );
}

#[test]
fn lint_issue_rejects_malformed_options() {
    let output = autospec()
        .args(["lint", "issue", "--unsupported"])
        .output()
        .expect("autospec lint issue starts");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "unknown autospec lint issue option: --unsupported\n"
    );
}

#[test]
fn lint_issue_caps_the_exit_status_without_dropping_findings() {
    let ac = std::iter::once("- [ ] `cargo test issue_lint` passes.".to_string())
        .chain((1..=65).map(|index| format!("prose criterion {index}")))
        .collect::<Vec<_>>()
        .join("\n");
    let body = issue_body(
        "Add `lint_issue_body` parity fixtures.",
        &ac,
        "cargo test issue_lint",
    );
    let path = write_issue_body("autospec-lint-issue-cap", &body);

    let output = autospec()
        .args(["lint", "issue", path.to_str().unwrap()])
        .output()
        .expect("autospec lint issue runs");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(output.status.code(), Some(64));
    assert!(output.stdout.is_empty());
    assert_eq!(stderr.lines().count(), 65);
    assert!(stderr.lines().all(|line| line.starts_with("AC_PROSE: ")));
}

#[test]
fn lint_implementation_reads_an_offline_diff_file_and_streams_findings() {
    let diff = write_implementation_diff(
        "autospec-lint-implementation-diff-file",
        &new_diff("scripts/policy.sh", &["eval(input)"]),
    );

    let output = autospec()
        .args([
            "lint",
            "implementation",
            "--diff-file",
            diff.to_str().unwrap(),
        ])
        .output()
        .expect("autospec lint implementation runs");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "SECURITY:scripts/policy.sh:2: eval() usage — potential code injection\n"
    );
}

#[test]
fn lint_implementation_contract_blocks_unlisted_paths_and_missing_regression_evidence() {
    let issue = write_issue_body(
        "autospec-lint-implementation-contract-blocking",
        "## Implementation outline\n\n- `src/allowed.rs`\n\n## Tests required\n\n- integration: real CLI regression\n",
    );
    let diff = write_implementation_diff(
        "autospec-lint-implementation-contract-blocking",
        &new_diff("scripts/unlisted.mjs", &["export const value = 1;"]),
    );

    let output = autospec()
        .args([
            "lint",
            "implementation-contract",
            "--issue-body-file",
            issue.to_str().unwrap(),
            "--diff-file",
            diff.to_str().unwrap(),
        ])
        .output()
        .expect("autospec lint implementation-contract runs");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "OUT_OF_SCOPE:scripts/unlisted.mjs:-: file not listed in ## Implementation outline\n\
MISSING_TEST:tests/integration/:-: required test tier 'integration' not present in diff\n"
    );
}

#[test]
fn lint_implementation_contract_accepts_an_outlined_project_native_regression_artifact() {
    let path = "scripts/test-autonomous-status-panel.mjs";
    let issue = write_issue_body(
        "autospec-lint-implementation-contract-passing",
        &format!(
            "## Implementation outline\n\n- `{path}`\n\n## Tests required\n\n- integration: real CLI regression\n"
        ),
    );
    let diff = write_implementation_diff(
        "autospec-lint-implementation-contract-passing",
        &new_diff(path, &["export const value = 1;"]),
    );

    let output = autospec()
        .args([
            "lint",
            "implementation-contract",
            "--issue-body-file",
            issue.to_str().unwrap(),
            "--diff-file",
            diff.to_str().unwrap(),
        ])
        .output()
        .expect("autospec lint implementation-contract runs");

    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
}

#[test]
fn lint_implementation_contract_blocks_missing_or_pathless_outlines() {
    let path = "scripts/test-autonomous-status-panel.mjs";
    let bodies = [
        "## Tests required\n\n- integration: real CLI regression\n",
        "## Implementation outline\n\n## Tests required\n\n- integration: real CLI regression\n",
        "## Implementation outline\n\n- Implement command behavior.\n\n## Tests required\n\n- integration: real CLI regression\n",
    ];

    for (index, body) in bodies.iter().enumerate() {
        let issue = write_issue_body(
            &format!("autospec-lint-implementation-contract-missing-outline-{index}"),
            body,
        );
        let diff = write_implementation_diff(
            &format!("autospec-lint-implementation-contract-missing-outline-{index}"),
            &new_diff(path, &["export const value = 1;"]),
        );

        let output = autospec()
            .args([
                "lint",
                "implementation-contract",
                "--issue-body-file",
                issue.to_str().unwrap(),
                "--diff-file",
                diff.to_str().unwrap(),
            ])
            .output()
            .expect("autospec lint implementation-contract runs");

        assert_eq!(output.status.code(), Some(1), "body fixture {index}");
        assert!(output.stderr.is_empty(), "body fixture {index}");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            "OUT_OF_SCOPE:scripts/test-autonomous-status-panel.mjs:-: file not listed in ## Implementation outline\n",
            "body fixture {index}"
        );
    }
}

#[test]
fn lint_implementation_contract_reports_usage_errors_at_exit_two() {
    let cases: &[(&[&str], &str)] = &[
        (
            &[],
            "autospec lint implementation-contract: requires --issue-body-file <PATH> and --diff-file <PATH>\n",
        ),
        (
            &["--issue-body-file", "--diff-file", "change.diff", "extra"],
            "autospec lint implementation-contract: --issue-body-file requires an argument\n",
        ),
        (
            &["--issue-body-file", "issue.md", "--unknown", "change.diff"],
            "autospec lint implementation-contract: unknown option: --unknown\n",
        ),
        (
            &["--diff-file", "first.diff", "--diff-file", "second.diff"],
            "autospec lint implementation-contract: --diff-file accepts exactly one path\n",
        ),
    ];

    for (args, expected_stderr) in cases {
        let output = autospec()
            .args(["lint", "implementation-contract"])
            .args(args.iter().copied())
            .output()
            .expect("autospec lint implementation-contract starts");

        assert_eq!(output.status.code(), Some(2), "args: {args:?}");
        assert!(output.stdout.is_empty(), "args: {args:?}");
        assert_eq!(
            String::from_utf8_lossy(&output.stderr),
            *expected_stderr,
            "args: {args:?}"
        );
    }
}

#[test]
fn lint_implementation_contract_reports_unreadable_and_malformed_inputs() {
    let fixture = temp_dir("autospec-lint-implementation-contract-input-errors");
    let missing_issue = fixture.join("missing-issue.md");
    let valid_diff = write_implementation_diff(
        "autospec-lint-implementation-contract-unreadable",
        &new_diff("src/allowed.rs", &["pub const VALUE: usize = 1;"]),
    );
    let unreadable = autospec()
        .args([
            "lint",
            "implementation-contract",
            "--issue-body-file",
            missing_issue.to_str().unwrap(),
            "--diff-file",
            valid_diff.to_str().unwrap(),
        ])
        .output()
        .expect("autospec lint implementation-contract starts");

    assert_eq!(unreadable.status.code(), Some(1));
    assert!(unreadable.stdout.is_empty());
    assert!(String::from_utf8_lossy(&unreadable.stderr).contains("could not read issue body file"));

    let issue = write_issue_body(
        "autospec-lint-implementation-contract-malformed-diff",
        "## Implementation outline\n\n- `src/allowed.rs`\n",
    );
    let malformed_diff = write_implementation_diff(
        "autospec-lint-implementation-contract-malformed-diff",
        "diff --git a/src/allowed.rs b/src/allowed.rs\n--- a/src/allowed.rs\n+++ b/src/allowed.rs\n@@ broken @@\n+value\n",
    );
    let malformed = autospec()
        .args([
            "lint",
            "implementation-contract",
            "--issue-body-file",
            issue.to_str().unwrap(),
            "--diff-file",
            malformed_diff.to_str().unwrap(),
        ])
        .output()
        .expect("autospec lint implementation-contract starts");

    assert_eq!(malformed.status.code(), Some(1));
    assert!(malformed.stdout.is_empty());
    assert!(String::from_utf8_lossy(&malformed.stderr).contains("could not parse unified diff"));
}

#[test]
fn lint_implementation_pre_commit_reads_only_staged_diff_and_enables_both_test_gates() {
    let fixture = temp_dir("autospec-lint-implementation-pre-commit");
    let log = fixture.join("git.log");
    let staged_diff = format!(
        "{}{}",
        new_diff(
            "tests/unit/test_vacuous.bats",
            &["grep -qv 'bad' result.txt || true"],
        ),
        new_diff(
            "tests/unit/test_density.bats",
            &["@test \"density\" {", "  echo no assertion", "}"],
        )
    );
    let bin = fake_bin(
        &fixture,
        Some(&format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$AUTOSPEC_LINT_COMMAND_LOG\"\nprintf '%s' {}\n",
            shell_single_quote(&staged_diff)
        )),
        None,
    );

    let output = autospec()
        .args(["lint", "implementation", "--pre-commit", "--staged"])
        .env("PATH", path_with(&bin))
        .env("AUTOSPEC_LINT_COMMAND_LOG", &log)
        .output()
        .expect("pre-commit lint runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert_eq!(output.status.code(), Some(3));
    assert!(output.stderr.is_empty());
    assert_eq!(std::fs::read_to_string(log).unwrap(), "diff\n--cached\n");
    let codes = lint_finding_codes(&stdout);
    assert!(codes.contains("VACUOUS_GREP_INVERSE_OR_TRUE"));
    assert!(!codes.contains("VACUOUS_OR_TRUE"));
    assert!(codes.contains("VACUOUS_NO_ASSERT"));
    assert!(codes.contains("ASSERTION_DENSITY"));
}

#[test]
fn lint_implementation_fetches_pr_then_issue_with_direct_gh_arguments() {
    let fixture = temp_dir("autospec-lint-implementation-pr-issue");
    let log = fixture.join("gh.log");
    let diff = new_diff("scripts/policy.sh", &["eval(input)"]);
    let bin = fake_bin(
        &fixture,
        None,
        Some(&format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" >> \"$AUTOSPEC_LINT_COMMAND_LOG\"\nif [ \"$1\" = pr ]; then\n  printf '%s' {}\nelif [ \"$1\" = issue ]; then\n  printf '%s\\n' 'Guardian: skip-SECURITY # reviewed fixture'\nelse\n  exit 19\nfi\n",
            shell_single_quote(&diff)
        )),
    );

    let output = autospec()
        .args(["lint", "implementation", "17", "--issue", "29"])
        .env("PATH", path_with(&bin))
        .env("AUTOSPEC_LINT_COMMAND_LOG", &log)
        .output()
        .expect("PR lint runs");

    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    assert_eq!(
        std::fs::read_to_string(log).unwrap(),
        "pr\ndiff\n17\nissue\nview\n29\n--json\nbody\n--jq\n.body\n"
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "INFO:SECURITY:scripts/policy.sh:2: eval() usage — potential code injection\n"
    );
}

#[test]
fn lint_implementation_directives_reformat_errors_without_changing_exit_status() {
    let diff = write_implementation_diff(
        "autospec-lint-implementation-directives",
        &new_diff("scripts/policy.sh", &["eval(input)"]),
    );

    let output = autospec()
        .args([
            "lint",
            "implementation",
            "--directives",
            "--diff-file",
            diff.to_str().unwrap(),
        ])
        .output()
        .expect("directive lint runs");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "Fix SECURITY: Remove the flagged pattern: never hardcode secrets, never bypass git hooks or use destructive resets, validate all inputs.\n"
    );
}

#[test]
fn lint_implementation_opt_in_test_gates_do_not_require_pre_commit_mode() {
    let vacuous = write_implementation_diff(
        "autospec-lint-implementation-vacuous",
        &new_diff(
            "tests/unit/test_vacuous.bats",
            &["grep -qv 'bad' result.txt || true"],
        ),
    );
    let density = write_implementation_diff(
        "autospec-lint-implementation-density",
        &new_diff(
            "tests/unit/test_density.bats",
            &["@test \"density\" {", "  echo no assertion", "}"],
        ),
    );

    let vacuous_output = autospec()
        .args([
            "lint",
            "implementation",
            "--vacuous-assertions",
            "--diff-file",
            vacuous.to_str().unwrap(),
        ])
        .output()
        .expect("vacuous assertion lint runs");
    let density_output = autospec()
        .args([
            "lint",
            "implementation",
            "--assertion-density",
            "--diff-file",
            density.to_str().unwrap(),
        ])
        .output()
        .expect("assertion density lint runs");

    assert_eq!(vacuous_output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&vacuous_output.stdout).contains("VACUOUS_GREP_INVERSE_OR_TRUE:")
    );
    assert!(!String::from_utf8_lossy(&vacuous_output.stdout).contains("VACUOUS_OR_TRUE:"));
    assert_eq!(density_output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&density_output.stdout).contains("ASSERTION_DENSITY:"));
}

#[test]
fn lint_implementation_rejects_mutually_exclusive_diff_sources() {
    let diff = write_implementation_diff(
        "autospec-lint-implementation-exclusive",
        &new_diff("scripts/policy.sh", &["eval(input)"]),
    );

    let output = autospec()
        .args([
            "lint",
            "implementation",
            "17",
            "--diff-file",
            diff.to_str().unwrap(),
        ])
        .output()
        .expect("mutually-exclusive inputs are rejected");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "autospec lint implementation: --diff-file and <PR> are mutually exclusive\n"
    );
}

#[test]
fn lint_implementation_surfaces_a_nonzero_gh_diff_response() {
    let fixture = temp_dir("autospec-lint-implementation-gh-error");
    let bin = fake_bin(
        &fixture,
        None,
        Some("#!/bin/sh\nprintf '%s\\n' 'authentication required' >&2\nexit 17\n"),
    );

    let output = autospec()
        .args(["lint", "implementation", "17"])
        .env("PATH", path_with(&bin))
        .output()
        .expect("remote lint exits");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "autospec lint implementation: gh pr diff 17 failed: authentication required\n"
    );
}

#[test]
fn lint_implementation_reports_no_staged_changes_without_loading_an_issue() {
    let fixture = temp_dir("autospec-lint-implementation-empty-staged");
    let log = fixture.join("commands.log");
    let bin = fake_bin(
        &fixture,
        Some("#!/bin/sh\nprintf '%s\\n' \"git:$*\" >> \"$AUTOSPEC_LINT_COMMAND_LOG\"\n"),
        Some("#!/bin/sh\nprintf '%s\\n' \"gh:$*\" >> \"$AUTOSPEC_LINT_COMMAND_LOG\"\nexit 99\n"),
    );

    let output = autospec()
        .args(["lint", "implementation", "--staged", "--issue", "29"])
        .env("PATH", path_with(&bin))
        .env("AUTOSPEC_LINT_COMMAND_LOG", &log)
        .output()
        .expect("staged lint runs");

    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "autospec lint implementation: no staged changes found\n"
    );
    assert_eq!(std::fs::read_to_string(log).unwrap(), "git:diff --cached\n");
}

#[test]
fn lint_implementation_caps_a_large_finding_set_at_64() {
    let diff = large_finding_diff();
    let path = write_implementation_diff("autospec-lint-implementation-cap", &diff);

    let output = autospec()
        .args([
            "lint",
            "implementation",
            "--vacuous-assertions",
            "--diff-file",
            path.to_str().unwrap(),
        ])
        .output()
        .expect("large implementation lint runs");

    assert_eq!(output.status.code(), Some(64));
    assert!(output.stderr.is_empty());
    assert!(String::from_utf8_lossy(&output.stdout).lines().count() > 64);
}

#[test]
fn lint_implementation_preserves_the_scope_explosion_exit_from_the_core() {
    let diff = write_implementation_diff(
        "autospec-lint-implementation-scope-explosion",
        &new_diff(
            "scripts/policy.sh",
            &[
                "eval(input)",
                "# TODO aggregate cap",
                "command --new-policy enabled",
            ],
        ),
    );

    let output = autospec()
        .args([
            "lint",
            "implementation",
            "--diff-file",
            diff.to_str().unwrap(),
        ])
        .env("AUTOSPEC_LINT_IMPLEMENTATION_TEST_HARD_CAP", "2")
        .output()
        .expect("scope explosion lint runs");

    assert_eq!(output.status.code(), Some(200));
    assert!(output.stderr.is_empty());
    assert!(String::from_utf8_lossy(&output.stdout)
        .contains("OUT_OF_SCOPE:-:-: too many findings — likely scope explosion"));
}

#[test]
fn lint_implementation_does_not_follow_symlinked_diff_paths_outside_the_repository() {
    let root = temp_dir("autospec-lint-implementation-symlink-root");
    let outside = temp_dir("autospec-lint-implementation-symlink-outside");
    std::fs::write(outside.join("secret.py"), "line\n".repeat(401)).expect("outside snapshot");
    symlink(&outside, root.join("linked")).expect("in-repository link");
    let diff = root.join("change.diff");
    std::fs::write(&diff, new_diff("linked/secret.py", &["pass"])).expect("diff fixture");

    let output = autospec()
        .args([
            "lint",
            "implementation",
            "--diff-file",
            diff.to_str().unwrap(),
        ])
        .current_dir(&root)
        .output()
        .expect("symlinked diff lint runs");

    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
}

#[test]
fn lint_implementation_reuse_lens_ignores_hidden_ignored_and_non_shell_evidence() {
    let root = temp_dir("autospec-lint-implementation-reuse-evidence");
    make_git_repo(&root, None);
    std::fs::create_dir_all(root.join("scripts")).expect("scripts directory");
    std::fs::create_dir_all(root.join("ignored")).expect("ignored directory");
    std::fs::create_dir_all(root.join(".hidden")).expect("hidden directory");
    std::fs::create_dir_all(root.join("nested/generated")).expect("nested ignored directory");
    std::fs::create_dir_all(root.join("ignored-by-ignore")).expect("ignore directory");
    std::fs::create_dir_all(root.join("ignored-by-rgignore")).expect("rgignore directory");
    std::fs::write(root.join(".gitignore"), "ignored/\n*.py[cod]\n").expect("gitignore");
    std::fs::write(root.join(".ignore"), "ignored-by-ignore/\n").expect("ignore");
    std::fs::write(root.join(".rgignore"), "ignored-by-rgignore/\n").expect("rgignore");
    std::fs::write(root.join("nested/.gitignore"), "generated/\n").expect("nested gitignore");
    std::fs::write(root.join("scripts/helper.txt"), "parse_config() { :; }\n")
        .expect("non-shell helper");
    std::fs::write(
        root.join("ignored/helper.sh"),
        "parse_config() { :; }\nsession-manager\n",
    )
    .expect("ignored shell helper");
    std::fs::write(
        root.join(".hidden/helper.sh"),
        "parse_config() { :; }\nsession-manager\n",
    )
    .expect("hidden shell helper");
    std::fs::write(
        root.join("nested/generated/helper.sh"),
        "parse_config() { :; }\nsession-manager\n",
    )
    .expect("nested ignored shell helper");
    std::fs::write(
        root.join("ignored-by-ignore/helper.sh"),
        "parse_config() { :; }\nsession-manager\n",
    )
    .expect("ignore shell helper");
    std::fs::write(
        root.join("ignored-by-rgignore/helper.sh"),
        "parse_config() { :; }\nsession-manager\n",
    )
    .expect("rgignore shell helper");
    std::fs::write(root.join("reference.pyc"), "session-manager\n")
        .expect("character-class ignored reference");
    let diff = root.join("change.diff");
    std::fs::write(
        &diff,
        format!(
            "{}{}",
            new_diff("scripts/new-loader.sh", &["parse_config() { :; }"]),
            new_diff("src/session-manager.rs", &["pub struct SessionManager;"]),
        ),
    )
    .expect("reuse diff fixture");

    let output = autospec()
        .args([
            "lint",
            "implementation",
            "--diff-file",
            diff.to_str().unwrap(),
        ])
        .current_dir(&root)
        .env("AUTOSPEC_REUSE_LENS", "1")
        .output()
        .expect("reuse lint runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    assert!(!stdout.contains("REINVENT_REPO_UTIL:"));
    assert!(stdout.contains("NEW_ABSTRACTION_SINGLE_CALLER:src/session-manager.rs:-:"));
    assert!(stdout.contains("has 0 external caller(s)"));
}

#[test]
fn lint_implementation_reuse_lens_excludes_an_in_repo_diff_from_abstraction_callers() {
    let root = temp_dir("autospec-lint-implementation-in-repo-diff");
    make_git_repo(&root, None);
    std::fs::create_dir_all(root.join("src")).expect("source directory");
    std::fs::write(root.join("src/consumer.rs"), "// session-manager\n").expect("real caller");
    let diff = root.join("change.diff");
    std::fs::write(
        &diff,
        new_diff("src/session-manager.rs", &["pub struct SessionManager;"]),
    )
    .expect("diff fixture");

    let output = autospec()
        .args([
            "lint",
            "implementation",
            "--diff-file",
            diff.to_str().unwrap(),
        ])
        .current_dir(&root)
        .env("AUTOSPEC_REUSE_LENS", "1")
        .output()
        .expect("reuse lint runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    assert!(stdout.contains("NEW_ABSTRACTION_SINGLE_CALLER:src/session-manager.rs:-:"));
    assert!(stdout.contains("has 1 external caller(s)"));
}

#[test]
fn status_json_reports_persisted_lifecycle_counts() {
    const EXPECTED_STATUS_JSON_FIELDS: &[&str] = &[
        "\"command\":\"status\"",
        "\"specs\":{",
        "\"planned\":",
        "\"superseded\":",
    ];

    let output = autospec()
        .args(["status", "--json"])
        .output()
        .expect("status command runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    for expected in EXPECTED_STATUS_JSON_FIELDS {
        assert!(stdout.contains(expected), "missing {expected} in {stdout}");
    }
}

#[test]
fn report_json_renders_a_release_summary_from_persisted_state() {
    let output = autospec()
        .args(["report", "--json"])
        .output()
        .expect("report command runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("\"version\":\"current\""));
    assert!(stdout.contains("\"passed\":"));
    assert!(stdout.contains("\"superseded\":"));
}

#[test]
fn plan_json_inspects_an_input_package_in_stable_path_order() {
    let package = temp_dir("autospec-plan");
    let specs = package.join("specs");
    std::fs::create_dir_all(&specs).expect("spec package directory");
    std::fs::write(
        specs.join("a-first.md"),
        "# Alpha\n\n## Version\n\nV9\n\n## Objective\n\nInspect alpha.\n",
    )
    .expect("first spec");
    std::fs::write(
        specs.join("z-second.md"),
        "# Beta\n\n## Version\n\nV8\n\n## Objective\n\nInspect beta.\n",
    )
    .expect("second spec");

    let output = autospec()
        .args(["plan", "--input", package.to_str().unwrap(), "--json"])
        .output()
        .expect("plan command runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("\"command\":\"plan\""));
    assert!(stdout.contains("\"spec_count\":2"));
    let alpha = stdout.find("\"id\":\"v9-alpha\"").expect("alpha spec");
    let beta = stdout.find("\"id\":\"v8-beta\"").expect("beta spec");
    assert!(alpha < beta, "specs should retain sorted source-path order");
}

#[test]
fn plan_text_lists_specs_from_an_input_package() {
    let package = temp_dir("autospec-plan-text");
    let specs = package.join("specs");
    std::fs::create_dir_all(&specs).expect("spec package directory");
    std::fs::write(
        specs.join("one.md"),
        "# Text Spec\n\n## Version\n\nV9\n\n## Objective\n\nRender text.\n",
    )
    .expect("spec");

    let output = autospec()
        .args(["plan", "--input", package.to_str().unwrap()])
        .output()
        .expect("plan command runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("AutoSpec plan: 1 generated spec(s)"));
    assert!(stdout.contains("v9-text-spec"));
}

#[test]
fn plan_rejects_a_missing_input_package() {
    let missing_package = temp_dir("autospec-plan-missing").join("missing-package");
    let output = autospec()
        .args(["plan", "--input", missing_package.to_str().unwrap()])
        .output()
        .expect("plan command starts");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("package directory"));
}

#[test]
fn plan_rejects_an_input_without_specs() {
    let package = temp_dir("autospec-plan-empty");
    let output = autospec()
        .args(["plan", "--input", package.to_str().unwrap()])
        .output()
        .expect("plan command starts");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("specs directory"));
}

#[test]
fn plan_rejects_an_input_with_no_markdown_specs() {
    let package = temp_dir("autospec-plan-no-specs");
    std::fs::create_dir_all(package.join("specs")).expect("specs directory");

    let output = autospec()
        .args(["plan", "--input", package.to_str().unwrap()])
        .output()
        .expect("plan command starts");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("no generated specs"));
}

#[test]
fn plan_rejects_a_malformed_generated_spec() {
    let package = temp_dir("autospec-plan-malformed");
    let specs = package.join("specs");
    std::fs::create_dir_all(&specs).expect("specs directory");
    std::fs::write(
        specs.join("missing-objective.md"),
        "# Incomplete\n\n## Version\n\nV9\n",
    )
    .expect("malformed spec");

    let output = autospec()
        .args(["plan", "--input", package.to_str().unwrap()])
        .output()
        .expect("plan command starts");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("could not parse"));
}

#[test]
fn plan_defaults_to_the_canonical_v62_package() {
    let output = autospec()
        .args(["plan", "--json"])
        .current_dir(workspace_root())
        .output()
        .expect("plan command runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("\"command\":\"plan\""));
    assert!(stdout.contains("v62-final-platform"));
    assert!(stdout.contains("\"spec_count\":14"));
    assert!(stdout.contains("\"id\":\"v62-rust-core-workspace\""));
}

#[test]
fn plan_rejects_an_input_flag_without_a_package_path() {
    let output = autospec()
        .args(["plan", "--input"])
        .output()
        .expect("plan command starts");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--input requires a path"));
}

#[test]
fn plan_rejects_an_option_as_an_input_path() {
    let output = autospec()
        .args(["plan", "--input", "--json"])
        .output()
        .expect("plan command starts");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--input requires a path"));
}

#[test]
fn plan_rejects_an_unknown_option() {
    let output = autospec()
        .args(["plan", "--not-an-option"])
        .output()
        .expect("plan command starts");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown autospec plan option"));
}

#[test]
fn validate_json_plans_rust_checks_for_changed_rust_sources() {
    let output = autospec()
        .args([
            "validate",
            "--path",
            "crates/autospec-core/src/state/mod.rs",
            "--json",
        ])
        .output()
        .expect("validate command runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("\"command\":\"validate\""));
    assert!(stdout.contains("\"mode\":\"planning\""));
    assert!(stdout.contains("\"changed_paths\":[\"crates/autospec-core/src/state/mod.rs\"]"));
    assert!(stdout.contains("\"name\":\"rust:lint\""));
    assert!(!stdout.contains("\"status\":\"ok\""));
}

#[test]
fn validate_rejects_a_path_flag_without_a_path() {
    let output = autospec()
        .args(["validate", "--path"])
        .output()
        .expect("validate command starts");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--path requires a path"));
}

#[test]
fn validate_rejects_an_empty_path() {
    let output = autospec()
        .args(["validate", "--path", ""])
        .output()
        .expect("validate command starts");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--path requires a path"));
}

#[test]
fn validate_executes_the_direct_plan_without_a_legacy_shell_handoff() {
    let root = temp_dir("autospec-validate-shell");
    let scripts = root.join("scripts");
    let log = root.join("handoff.log");
    std::fs::create_dir_all(&scripts).expect("scripts directory");
    std::fs::write(
        scripts.join("unexpected-handoff.sh"),
        "#!/usr/bin/env bash\nexit 1\n",
    )
    .expect("validation fixture");

    let output = autospec()
        .args(["validate", "--fast", "--json"])
        .current_dir(&root)
        .env("AUTOSPEC_VALIDATE_TEST_LOG", &log)
        .output()
        .expect("validate command runs");

    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        !output.status.success(),
        "an incomplete repository must fail"
    );
    assert!(stdout.contains("\"schema\":2"));
    assert!(stdout.contains("\"results\":["));
    assert!(String::from_utf8_lossy(&output.stderr).contains("direct Rust validation failed"));
    assert!(!log.exists(), "validate must not execute a shell handoff");
}

#[test]
fn validate_rejects_a_scoped_run_when_git_cannot_resolve_its_base() {
    let output = autospec()
        .args([
            "validate",
            "--fast",
            "--changed=refs/heads/autospec-missing-validation-base",
        ])
        .output()
        .expect("validate command starts");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("could not resolve changed paths from git base"));
}

#[test]
fn validate_shadow_results_aggregates_captured_shell_outcomes_without_execution() {
    let fixture = workspace_root()
        .join("crates/autospec-cli/tests/fixtures/validation-results/legacy-fast-passed.json");
    let expected = std::fs::read_to_string(workspace_root().join(
        "crates/autospec-cli/tests/fixtures/validation-results/legacy-fast-passed.aggregate.json",
    ))
    .expect("expected aggregate fixture is readable")
    .trim()
    .to_string();
    let output = autospec()
        .args([
            "validate",
            "--shadow-results",
            fixture.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("shadow validation starts");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("\"mode\":\"shadow-results\""));
    assert!(stdout.contains(&expected));
}

#[test]
fn validate_shadow_results_preserves_a_required_failure_exit() {
    let fixture = workspace_root()
        .join("crates/autospec-cli/tests/fixtures/validation-results/legacy-required-failed.json");
    let output = autospec()
        .args([
            "validate",
            "--shadow-results",
            fixture.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("failing shadow validation starts");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(!output.status.success());
    assert!(stdout.contains("\"status\":\"failed\""));
    assert!(stdout.contains("\"required_failed\":1"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("captured validation results failed"));
}

#[test]
fn validate_shadow_results_rejects_an_empty_capture() {
    let fixture = workspace_root()
        .join("crates/autospec-cli/tests/fixtures/validation-results/legacy-empty.json");
    let output = autospec()
        .args([
            "validate",
            "--shadow-results",
            fixture.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("validate command starts");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("at least one observation"));
}

#[test]
fn validate_shadow_results_never_executes_a_local_validate_script() {
    let root = temp_dir("validate-shadow-no-legacy-shell");
    let scripts = root.join("scripts");
    let log = root.join("legacy-shell.log");
    std::fs::create_dir_all(&scripts).expect("scripts directory");
    std::fs::write(
        scripts.join("validate.sh"),
        "#!/usr/bin/env bash\nset -eu\nprintf 'legacy shell ran\\n' > \"$AUTOSPEC_VALIDATE_TEST_LOG\"\n",
    )
    .expect("legacy shell fixture");
    let fixture = workspace_root()
        .join("crates/autospec-cli/tests/fixtures/validation-results/legacy-fast-passed.json");

    let output = autospec()
        .args([
            "validate",
            "--shadow-results",
            fixture.to_str().expect("fixture path"),
            "--json",
        ])
        .current_dir(&root)
        .env("AUTOSPEC_VALIDATE_TEST_LOG", &log)
        .output()
        .expect("shadow aggregation runs");

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"mode\":\"shadow-results\""));
    assert!(
        !log.exists(),
        "shadow aggregation must not execute validate.sh"
    );
}

#[test]
fn validate_rejects_untracked_root_helper_without_canonical_source() {
    let root = temp_dir("validate-root-helper-wrapper-policy");
    make_git_repo(&root, None);
    let scripts = root.join("scripts");
    std::fs::create_dir_all(&scripts).expect("scripts directory");
    std::fs::write(
        scripts.join("restored-stale-wrapper.sh"),
        "#!/usr/bin/env bash\nprintf 'stale wrapper\\n'\n",
    )
    .expect("stale wrapper fixture");

    let output = autospec()
        .args(["validate", "--fast", "--json"])
        .current_dir(&root)
        .output()
        .expect("validate command runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        !output.status.success(),
        "stale root helper wrapper must fail validation"
    );
    assert!(
        stdout.contains("\"id\":\"check_root_helper_wrapper_policy\""),
        "validation output must include the root helper wrapper policy check: {stdout}"
    );
    assert!(
        stdout.contains("\"exit_code\":1"),
        "root helper wrapper policy check must reject the fixture: {stdout}"
    );
}

#[test]
fn init_persists_explicit_spec_state_without_running_work() {
    let root = temp_dir("autospec-init");
    let output = autospec()
        .args(["init", "--spec", "v62-rust-core-workspace", "--json"])
        .current_dir(&root)
        .output()
        .expect("init command runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("\"command\":\"init\""));
    assert!(stdout.contains("\"status\":\"initialized\""));
    assert!(stdout.contains("\"spec_count\":1"));
    assert!(root.join(".autospec/state/specs.json").exists());
}

#[test]
fn init_requires_at_least_one_explicit_spec() {
    let output = autospec()
        .arg("init")
        .output()
        .expect("init command starts");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("at least one --spec"));
}

#[test]
fn init_refuses_to_overwrite_existing_spec_state() {
    let root = temp_dir("autospec-init-existing");
    let first = autospec()
        .args(["init", "--spec", "v62-rust-core-workspace"])
        .current_dir(&root)
        .output()
        .expect("first init runs");
    assert!(first.status.success());

    let second = autospec()
        .args(["init", "--spec", "v63-spec-metadata-parser"])
        .current_dir(&root)
        .output()
        .expect("second init starts");

    assert!(!second.status.success());
    assert!(String::from_utf8_lossy(&second.stderr).contains("refuses to overwrite"));
}

#[test]
fn init_refuses_an_existing_empty_primary_or_recovery_state_file() {
    for name in ["specs.json", "specs.json.tmp"] {
        let root = temp_dir(&format!("autospec-init-{name}"));
        let state = root.join(".autospec/state");
        std::fs::create_dir_all(&state).expect("state directory");
        std::fs::write(state.join(name), "{\"schema\":1,\"specs\":[]}")
            .expect("existing state document");

        let output = autospec()
            .args(["init", "--spec", "v62-rust-core-workspace"])
            .current_dir(&root)
            .output()
            .expect("init starts");

        assert!(!output.status.success(), "{name} must block initialization");
        assert!(String::from_utf8_lossy(&output.stderr).contains("refuses to overwrite"));
    }
}

#[test]
fn run_creates_a_local_queue_without_executing_work_and_refuses_existing_runs() {
    let root = temp_dir("autospec-run-create");
    let first = autospec()
        .args([
            "run",
            "--run",
            "run-cli-create",
            "--spec",
            "v67-agent-integration-contracts",
            "--json",
        ])
        .current_dir(&root)
        .output()
        .expect("run command starts");
    let stdout = String::from_utf8_lossy(&first.stdout);

    assert!(
        first.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(stdout.contains("\"command\":\"run\""));
    assert!(stdout.contains("\"mode\":\"create\""));
    assert!(stdout.contains("\"status\":\"created\""));
    assert!(root
        .join(".autospec/runs/run-cli-create/queue.json")
        .exists());

    let repeated = autospec()
        .args([
            "run",
            "--run",
            "run-cli-create",
            "--spec",
            "v67-agent-integration-contracts",
        ])
        .current_dir(&root)
        .output()
        .expect("repeated run starts");

    assert!(!repeated.status.success());
    assert!(String::from_utf8_lossy(&repeated.stderr).contains("already exists"));
}

#[test]
fn run_ingests_an_explicit_agent_result_without_launching_an_agent_or_validation() {
    let root = temp_dir("autospec-run-ingest");
    let created = autospec()
        .args([
            "run",
            "--run",
            "run-cli-ingest",
            "--spec",
            "v67-agent-integration-contracts",
        ])
        .current_dir(&root)
        .output()
        .expect("queue creation starts");
    assert!(created.status.success());
    let input = root.join("agent-result.json");
    std::fs::write(
        &input,
        "{\"result\":\"implemented\",\"files_changed\":[],\"validation\":\"cargo test --workspace: exit 0\",\"blockers\":[],\"handoff\":\"ready\"}",
    )
    .expect("agent result fixture is written");

    let output = autospec()
        .args([
            "run",
            "--ingest",
            input.to_str().unwrap(),
            "--run",
            "run-cli-ingest",
            "--spec",
            "v67-agent-integration-contracts",
            "--result-id",
            "result-1",
            "--outcome",
            "passed",
            "--json",
        ])
        .current_dir(&root)
        .output()
        .expect("result ingestion starts");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("\"mode\":\"ingest\""));
    assert!(stdout.contains("\"outcome\":\"passed\""));
    assert!(root
        .join(".autospec/runs/run-cli-ingest/agent-results/v67-agent-integration-contracts/result-1.json")
        .exists());
    let queue = std::fs::read_to_string(root.join(".autospec/runs/run-cli-ingest/queue.json"))
        .expect("updated queue is readable");
    assert!(queue.contains("\"status\":\"passed\""));
    assert!(queue.contains("\"agent_result_ids\":[\"result-1\"]"));
}

#[test]
fn run_persists_literal_resume_command_without_shell_expansion_under_set_u() {
    let root = temp_dir("autospec-run-literal-resume-command");
    let script = concat!(
        "AUTOSPEC_RESUME_COMMAND='omx exec --cd /repo $autospec-run' ",
        "\"$AUTOSPEC_BIN\" run --run run-cli-literal-resume ",
        "--spec v67-agent-integration-contracts --json"
    );

    let output = Command::new("sh")
        .args(["-eu", "-c", script])
        .env("AUTOSPEC_BIN", env!("CARGO_BIN_EXE_autospec"))
        .current_dir(&root)
        .output()
        .expect("run command starts under set -u");

    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let resume_command = std::fs::read_to_string(
        root.join(".autospec/runs/run-cli-literal-resume/resume-command.json"),
    )
    .expect("resume command metadata is persisted");
    let parsed: serde_json::Value =
        serde_json::from_str(&resume_command).expect("resume command metadata is JSON");

    assert_eq!(parsed["schema"], 1);
    assert_eq!(parsed["status"], "ready");
    assert_eq!(parsed["resume_command"]["kind"], "literal");
    assert_eq!(
        parsed["resume_command"]["value"],
        "omx exec --cd /repo $autospec-run"
    );
}

#[test]
fn run_startup_failure_records_retry_metadata_for_resume() {
    let root = temp_dir("autospec-run-startup-failure");
    let output = autospec()
        .args([
            "run",
            "--run",
            "run-cli-startup-failure",
            "--spec",
            "v67-agent-integration-contracts",
            "--json",
        ])
        .env(
            "AUTOSPEC_RESUME_COMMAND",
            std::ffi::OsString::from_vec(vec![0xff]),
        )
        .current_dir(&root)
        .output()
        .expect("run command starts");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(
        output.status.code(),
        Some(2),
        "stdout={stdout} stderr={stderr}"
    );
    assert!(stdout.contains("\"status\":\"startup_failed\""));
    assert!(stdout.contains("\"recovery\":\"retry\""));
    assert!(stdout.contains("\"lock\":\"released\""));
    assert!(root
        .join(".autospec/runs/run-cli-startup-failure/queue.json")
        .exists());
    let startup_failure = std::fs::read_to_string(
        root.join(".autospec/runs/run-cli-startup-failure/startup-failed.json"),
    )
    .expect("startup failure metadata is persisted");
    let parsed: serde_json::Value =
        serde_json::from_str(&startup_failure).expect("startup failure metadata is JSON");
    assert_eq!(parsed["status"], "startup_failed");
    assert_eq!(parsed["recovery"], "retry");
    assert_eq!(parsed["lock"], "released");

    let resumed = autospec()
        .args(["resume", "--json"])
        .current_dir(&root)
        .output()
        .expect("resume command starts");
    let resume_stdout = String::from_utf8_lossy(&resumed.stdout);
    assert!(
        resumed.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    assert!(resume_stdout.contains("\"startup_recovery\":\"retry\""));
}

#[test]
fn resume_reports_the_latest_incomplete_queue_and_errors_when_none_exists() {
    let root = temp_dir("autospec-resume");
    let missing = autospec()
        .args(["resume", "--json"])
        .current_dir(&root)
        .output()
        .expect("empty resume starts");
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("no incomplete run"));

    let created = autospec()
        .args([
            "run",
            "--run",
            "run-cli-resume",
            "--spec",
            "v67-agent-integration-contracts",
        ])
        .current_dir(&root)
        .output()
        .expect("queue creation starts");
    assert!(created.status.success());

    let resumed = autospec()
        .args(["resume", "--json"])
        .current_dir(&root)
        .output()
        .expect("resume command starts");
    let stdout = String::from_utf8_lossy(&resumed.stdout);

    assert!(
        resumed.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    assert!(stdout.contains("\"command\":\"resume\""));
    assert!(stdout.contains("\"run_id\":\"run-cli-resume\""));
    assert!(stdout.contains("\"spec_id\":\"v67-agent-integration-contracts\""));
    assert!(stdout.contains("\"entry_status\":\"pending\""));
}

#[test]
fn autonomous_start_dry_run_includes_monitor_and_supervisor_companions() {
    let output = autospec()
        .args([
            "autonomous",
            "start",
            "--repo",
            "berlinguyinca/autospec",
            "--repo-dir",
            "/tmp/autospec",
            "--dry-run",
            "--json",
        ])
        .output()
        .expect("autospec autonomous start runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("\"command\":\"autonomous\""));
    assert!(stdout.contains("\"subcommand\":\"start\""));
    assert!(stdout.contains("\"repo\":\"berlinguyinca/autospec\""));
    assert!(stdout.contains("\"repo_dir\":\"/tmp/autospec\""));
    assert!(stdout.contains("\"conductor\""));
    assert!(stdout.contains("autospec autonomous run-foreground"));
    assert!(stdout.contains("\"companions\""));
    assert!(stdout.contains("\"monitor\""));
    assert!(stdout.contains("\"supervisor\""));
    assert!(stdout.contains("autospec autonomous monitor"));
    assert!(stdout.contains("autospec autonomous supervise"));
}

#[test]
fn autonomous_main_health_reports_branch_not_found_without_waiting() {
    let temp = temp_dir("autospec-main-health-missing-branch");
    let state_dir = temp.join("state");
    let log = temp.join("gh.log");
    let bin = fake_bin(
        &temp,
        None,
        Some(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" >> \"$AUTOSPEC_GH_LOG\"\ncase \"$*\" in\n  'api repos/berlinguyinca/autospec/branches/missing-health') exit 1 ;;\n  *) exit 19 ;;\nesac\n",
        ),
    );

    let output = autospec()
        .args([
            "autonomous",
            "main-health",
            "--repo",
            "berlinguyinca/autospec",
            "--branch",
            "missing-health",
        ])
        .env("PATH", path_with(&bin))
        .env("AUTOSPEC_GH_LOG", &log)
        .env("AUTOSPEC_AUTONOMOUS_STATE_DIR", &state_dir)
        .output()
        .expect("autospec autonomous main-health runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert_eq!(output.status.code(), Some(2));
    assert!(stdout.contains("DECISION:halt"));
    assert!(stdout.contains("REASON:branch-not-found"));
    assert!(!stdout.contains("DECISION:wait"));
    let persisted = std::fs::read_to_string(
        state_dir.join("berlinguyinca_autospec/main-health-observations.jsonl"),
    )
    .expect("health observation persisted");
    assert!(persisted.contains("\"branch\":\"missing-health\""));
    assert!(persisted.contains("\"outcome\":\"halt\""));
    assert!(persisted.contains("\"diagnostic\":\"branch-not-found\""));
}

#[test]
fn autonomous_main_health_resolves_explicit_branch_before_default_branch() {
    let temp = temp_dir("autospec-main-health-explicit-first");
    let log = temp.join("gh.log");
    let bin = fake_bin(
        &temp,
        None,
        Some(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" >> \"$AUTOSPEC_GH_LOG\"\ncase \"$*\" in\n  'api repos/berlinguyinca/autospec/branches/health') printf '{\"name\":\"health\"}' ;;\n  'api repos/berlinguyinca/autospec/commits/health/status') printf '{\"state\":\"success\",\"total_count\":1,\"statuses\":[{\"context\":\"ci\",\"state\":\"success\"}]}' ;;\n  'repo view berlinguyinca/autospec --json defaultBranchRef --jq .defaultBranchRef.name') exit 18 ;;\n  *) exit 19 ;;\nesac\n",
        ),
    );

    let output = autospec()
        .args([
            "autonomous",
            "main-health",
            "--repo",
            "berlinguyinca/autospec",
            "--branch",
            "health",
        ])
        .env("PATH", path_with(&bin))
        .env("AUTOSPEC_GH_LOG", &log)
        .env("AUTOSPEC_AUTONOMOUS_STATE_DIR", temp.join("state"))
        .output()
        .expect("autospec autonomous main-health runs");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let gh_log = std::fs::read_to_string(log).expect("gh log");

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("DECISION:continue"));
    assert!(
        !gh_log.contains("repo\nview"),
        "default branch must not be queried"
    );
}

#[test]
fn autonomous_main_health_blocks_required_check_failure() {
    let temp = temp_dir("autospec-main-health-failed-check");
    let bin = fake_bin(
        &temp,
        None,
        Some(
            "#!/bin/sh\ncase \"$*\" in\n  'api repos/berlinguyinca/autospec/branches/trunk') printf '{\"name\":\"trunk\"}' ;;\n  'api repos/berlinguyinca/autospec/commits/trunk/status') printf '{\"state\":\"failure\",\"total_count\":1,\"statuses\":[{\"context\":\"ci\",\"state\":\"failure\"}]}' ;;\n  *) exit 19 ;;\nesac\n",
        ),
    );

    let output = autospec()
        .args([
            "autonomous",
            "main-health",
            "--repo",
            "berlinguyinca/autospec",
            "--branch",
            "trunk",
        ])
        .env("PATH", path_with(&bin))
        .env("AUTOSPEC_AUTONOMOUS_STATE_DIR", temp.join("state"))
        .output()
        .expect("autospec autonomous main-health runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert_eq!(output.status.code(), Some(2));
    assert!(stdout.contains("DECISION:halt"));
    assert!(stdout.contains("REASON:required-check-failed"));
    assert!(stdout.contains("CHECK:ci:completed:failure:required"));
}

#[test]
fn autonomous_run_foreground_stops_before_rust_executor_when_health_branch_is_missing() {
    let temp = temp_dir("autospec-foreground-health-block");
    let repo_dir = temp.join("repo");
    make_git_repo(
        &repo_dir,
        Some("https://github.com/berlinguyinca/autospec.git"),
    );
    let bin = fake_bin(
        &temp,
        None,
        Some(
            "#!/bin/sh\ncase \"$*\" in\n  'api repos/berlinguyinca/autospec/branches/missing-health') exit 1 ;;\n  *) exit 19 ;;\nesac\n",
        ),
    );

    let output = autospec()
        .args([
            "autonomous",
            "run-foreground",
            "--repo",
            "berlinguyinca/autospec",
            "--repo-dir",
            repo_dir.to_str().unwrap(),
            "--branch",
            "missing-health",
        ])
        .env("PATH", path_with(&bin))
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", temp.join("operator"))
        .env("AUTOSPEC_STATE_DIR", temp.join("state"))
        .env("AUTOSPEC_AUTONOMOUS_SPEND_DIR", temp.join("spend"))
        .env("AUTOSPEC_AUTONOMOUS_STATE_DIR", temp.join("state"))
        .output()
        .expect("autospec autonomous run-foreground runs");

    assert_eq!(output.status.code(), Some(20));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "{\"decision\":\"park\",\"reason\":\"health_halt\"}\n"
    );
    let lifecycle = temp.join("operator/berlinguyinca_autospec/lifecycle.json");
    assert!(lifecycle.exists(), "missing {}", lifecycle.display());
    assert!(std::fs::read_to_string(lifecycle)
        .expect("lifecycle record")
        .contains("\"reason\":\"health_halt\""));
}

#[test]
fn autonomous_foreground_stop_precedes_health_and_dispatch() {
    let temp = temp_dir("autospec-foreground-stop-precedence");
    let repo_dir = temp.join("repo");
    make_git_repo(
        &repo_dir,
        Some("https://github.com/berlinguyinca/autospec.git"),
    );
    let stop_flag = temp.join("stop.flag");
    let gh_log = temp.join("gh.log");
    std::fs::write(&stop_flag, "graceful\noperator\n").expect("stop flag");
    let bin = fake_bin(
        &temp,
        None,
        Some("#!/bin/sh\nprintf '%s\\n' \"$@\" >> \"$AUTOSPEC_GH_LOG\"\nexit 19\n"),
    );

    let output = autospec()
        .args([
            "autonomous",
            "run-foreground",
            "--repo",
            "berlinguyinca/autospec",
            "--repo-dir",
            repo_dir.to_str().unwrap(),
        ])
        .env("PATH", path_with(&bin))
        .env("AUTOSPEC_GH_LOG", &gh_log)
        .env("AUTOSPEC_STOP_FLAG_FILE", &stop_flag)
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", temp.join("operator"))
        .env("AUTOSPEC_STATE_DIR", temp.join("state"))
        .env("AUTOSPEC_AUTONOMOUS_SPEND_DIR", temp.join("spend"))
        .output()
        .expect("autospec autonomous run-foreground runs");

    assert_eq!(output.status.code(), Some(20));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "{\"decision\":\"stop\",\"mode\":\"graceful\"}\n"
    );
    assert!(!gh_log.exists(), "stop must preempt health and dispatch");
}

#[test]
fn autonomous_bare_command_defaults_to_start() {
    let output = autospec()
        .args([
            "autonomous",
            "--repo",
            "berlinguyinca/autospec",
            "--repo-dir",
            "/tmp/autospec",
            "--dry-run",
            "--json",
        ])
        .output()
        .expect("autospec autonomous runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("\"subcommand\":\"start\""));
    assert!(stdout.contains("\"status\":\"dry-run\""));
}

#[test]
fn autonomous_supervise_once_json_reports_observer_status() {
    let output = autospec()
        .args([
            "autonomous",
            "supervise",
            "--repo",
            "berlinguyinca/autospec",
            "--pid",
            "999999",
            "--once",
            "--json",
        ])
        .output()
        .expect("autospec autonomous supervise runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("\"command\":\"autonomous\""));
    assert!(stdout.contains("\"subcommand\":\"supervise\""));
    assert!(stdout.contains("\"repo\":\"berlinguyinca/autospec\""));
    assert!(stdout.contains("\"conductor\":\"stopped\""));
    assert!(stdout.contains("\"pid\":\"999999\""));
    assert!(stdout.contains("\"action\":\"conductor-not-running\""));
}

#[test]
fn autonomous_start_live_writes_repo_scoped_pid_and_log_metadata() {
    let temp = temp_dir("autospec-autonomous-live");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let repo_dir = temp.join("repo");
    make_git_repo(&repo_dir, None);

    let output = autospec()
        .args([
            "autonomous",
            "start",
            "--repo",
            "berlinguyinca/autospec",
            "--repo-dir",
            repo_dir.to_str().unwrap(),
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_STATE_DIR", temp.join("state"))
        .env("AUTOSPEC_AUTONOMOUS_SPEND_DIR", temp.join("spend"))
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .env("PATH", hermetic_autonomous_path(&temp))
        .output()
        .expect("autospec autonomous start runs");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let scope = operator_dir.join("berlinguyinca_autospec");

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("\"status\":\"started\""));
    assert!(stdout.contains("\"conductor\""));
    assert!(stdout.contains("\"monitor\""));
    assert!(stdout.contains("\"supervisor\""));
    for name in ["conductor", "monitor", "supervisor"] {
        let pid_file = scope.join(format!("{name}.pid"));
        let logpath_file = scope.join(format!("{name}.logpath"));
        assert!(pid_file.exists(), "missing {}", pid_file.display());
        assert!(logpath_file.exists(), "missing {}", logpath_file.display());
        assert!(!std::fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .is_empty());
    }

    cleanup_pids(&scope);
}

#[test]
fn autonomous_status_json_reports_companion_processes() {
    let temp = temp_dir("autospec-autonomous-status");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let repo_dir = temp.join("repo");
    make_git_repo(&repo_dir, None);

    let start = autospec()
        .args([
            "autonomous",
            "start",
            "--repo",
            "berlinguyinca/autospec",
            "--repo-dir",
            repo_dir.to_str().unwrap(),
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_STATE_DIR", temp.join("state"))
        .env("AUTOSPEC_AUTONOMOUS_SPEND_DIR", temp.join("spend"))
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .env("PATH", hermetic_autonomous_path(&temp))
        .output()
        .expect("autospec autonomous start runs");
    assert!(start.status.success());

    let status = autospec()
        .args([
            "autonomous",
            "status",
            "--repo",
            "berlinguyinca/autospec",
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_STATE_DIR", temp.join("state"))
        .env("AUTOSPEC_AUTONOMOUS_SPEND_DIR", temp.join("spend"))
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .output()
        .expect("autospec autonomous status runs");
    let stdout = String::from_utf8_lossy(&status.stdout);

    assert!(status.status.success());
    assert!(stdout.contains("\"conductor\":{\"running\":true"));
    assert!(stdout.contains("\"monitor\":{\"running\":true"));
    assert!(stdout.contains("\"supervisor\":{\"running\":true"));
    assert!(stdout.contains("\"pid_file\""));
    assert!(stdout.contains("\"logpath_file\""));

    cleanup_pids(&operator_dir.join("berlinguyinca_autospec"));
}

#[test]
fn autonomous_status_json_reports_state_heartbeat_and_spend_metadata() {
    let temp = temp_dir("autospec-autonomous-rich-status");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let home = temp.join("home");
    let scope = operator_dir.join("berlinguyinca_autospec");
    let state_scope = home
        .join(".autospec")
        .join("autonomous")
        .join("berlinguyinca_autospec");
    std::fs::create_dir_all(&scope).expect("scope dir");
    std::fs::create_dir_all(&state_scope).expect("state scope");
    std::fs::create_dir_all(
        home.join(".autospec")
            .join("autonomous-spend")
            .join("berlinguyinca__autospec"),
    )
    .expect("scoped spend dir");
    std::fs::write(scope.join("conductor.pid"), "999999\n").expect("pid");
    std::fs::write(
        state_scope.join("state.json"),
        "{\"repo\":\"berlinguyinca/autospec\",\"status\":\"parked:usage-limit\",\"heartbeat_at\":1783526400,\"cycle\":42}\n",
    )
    .expect("state");
    std::fs::write(
        home.join(".autospec")
            .join("autonomous-spend")
            .join("berlinguyinca__autospec")
            .join("spend.json"),
        "{\"schema\":1,\"tokens\":1234,\"issues\":5}\n",
    )
    .expect("spend");

    let output = autospec()
        .args([
            "autonomous",
            "status",
            "--repo",
            "berlinguyinca/autospec",
            "--json",
        ])
        .env("HOME", &home)
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .output()
        .expect("autospec autonomous status runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("\"state_status\":\"parked:usage-limit\""));
    assert!(stdout.contains("\"heartbeat_at\":1783526400"));
    assert!(stdout.contains("\"last_cycle\":\"42\""));
    assert!(stdout.contains("\"spend\""));
    assert!(stdout.contains("\"tokens\":1234"));
}

#[test]
fn autonomous_status_reports_the_repo_scoped_resilience_spend_ledger() {
    let temp = temp_dir("autospec-autonomous-scoped-spend-status");
    let scoped_spend = temp
        .join("spend")
        .join("berlinguyinca__autospec")
        .join("spend.json");
    let legacy_spend = temp.join("legacy-spend.json");
    std::fs::create_dir_all(scoped_spend.parent().expect("scoped spend parent"))
        .expect("create scoped spend parent");
    std::fs::write(
        &scoped_spend,
        "{\"schema\":1,\"tokens\":9,\"filed_issues\":7,\"budget_issues\":2}\n",
    )
    .expect("scoped spend");
    std::fs::write(&legacy_spend, "{\"tokens\":1234,\"issues\":5}\n").expect("legacy spend");

    let output = autospec()
        .args([
            "autonomous",
            "status",
            "--repo",
            "berlinguyinca/autospec",
            "--json",
        ])
        .env("AUTOSPEC_STATE_DIR", temp.join("state"))
        .env("AUTOSPEC_AUTONOMOUS_SPEND_DIR", temp.join("spend"))
        .env("AUTOSPEC_AUTONOMOUS_SPEND_FILE", &legacy_spend)
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", temp.join("operator"))
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", temp.join("logs"))
        .output()
        .expect("autospec autonomous status runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout
        .contains("\"spend\":{\"tokens\":9,\"issues\":2,\"filed_issues\":7,\"budget_issues\":2}"));
    assert!(
        !stdout.contains("1234"),
        "status must not display the legacy global spend file when policy uses a scoped ledger"
    );
}

#[test]
fn autonomous_status_all_json_aliases_list_with_conductors_key() {
    let temp = temp_dir("autospec-autonomous-status-all");
    let operator_dir = temp.join("operator");
    let scope = operator_dir.join("berlinguyinca_autospec");
    std::fs::create_dir_all(&scope).expect("scope dir");
    std::fs::write(scope.join("conductor.pid"), "999999\n").expect("pid");

    let output = autospec()
        .args(["autonomous", "status", "--all", "--json"])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_STATE_DIR", temp.join("state"))
        .env("AUTOSPEC_AUTONOMOUS_SPEND_DIR", temp.join("spend"))
        .output()
        .expect("autospec autonomous status all runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("\"conductors\":["));
    assert!(stdout.contains("\"slug\":\"berlinguyinca_autospec\""));
}

#[test]
fn autonomous_immediate_stop_drains_only_the_target_repo_conductor() {
    let temp = temp_dir("autospec-autonomous-stop");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let stop_flag = temp.join("stop.flag");
    let target_repo_dir = temp.join("target-repo");
    let other_repo_dir = temp.join("other-repo");
    std::fs::create_dir_all(&target_repo_dir).expect("target repo dir");
    std::fs::create_dir_all(&other_repo_dir).expect("other repo dir");

    start_sleeping_autonomous(
        &operator_dir,
        &log_dir,
        &target_repo_dir,
        "berlinguyinca/autospec",
    );
    start_sleeping_autonomous(
        &operator_dir,
        &log_dir,
        &other_repo_dir,
        "metabolomics-us/go-modules",
    );

    let output = autospec()
        .args([
            "autonomous",
            "stop",
            "--immediate",
            "--repo",
            "berlinguyinca/autospec",
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_STATE_DIR", temp.join("state"))
        .env("AUTOSPEC_AUTONOMOUS_SPEND_DIR", temp.join("spend"))
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .env("AUTOSPEC_STOP_FLAG_FILE", &stop_flag)
        .output()
        .expect("autospec autonomous stop runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("\"subcommand\":\"stop\""));
    assert!(stdout.contains("\"repo\":\"berlinguyinca/autospec\""));
    assert!(stdout.contains("\"stopped\":2"));
    assert!(stdout.contains("\"draining\":true"));

    let target_status = autonomous_status(&operator_dir, &log_dir, "berlinguyinca/autospec");
    assert!(target_status.contains("\"conductor\":{\"running\":true"));
    assert!(target_status.contains("\"monitor\":{\"running\":false"));
    assert!(target_status.contains("\"supervisor\":{\"running\":false"));
    let other_status = autonomous_status(&operator_dir, &log_dir, "metabolomics-us/go-modules");
    assert!(other_status.contains("\"conductor\":{\"running\":true"));
    assert!(other_status.contains("\"monitor\":{\"running\":true"));
    assert!(other_status.contains("\"supervisor\":{\"running\":true"));

    cleanup_pids(&operator_dir.join("berlinguyinca_autospec"));
    cleanup_pids(&operator_dir.join("metabolomics-us_go-modules"));
}

#[test]
fn autonomous_stop_graceful_writes_sentinel_and_leaves_conductor_running() {
    let temp = temp_dir("autospec-autonomous-graceful-stop");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let stop_flag = temp.join("stop.flag");
    let repo_dir = temp.join("repo");
    std::fs::create_dir_all(&repo_dir).expect("repo dir");

    start_sleeping_autonomous(&operator_dir, &log_dir, &repo_dir, "berlinguyinca/autospec");
    let scope = operator_dir.join("berlinguyinca_autospec");
    let original_conductor = read_pid(&scope, "conductor");
    // The foreground conductor is intentionally one-shot and may finish its
    // empty fixture scan before the stop command runs. Replace only the
    // persisted conductor record with a long-lived fixture process so this
    // test observes the lifecycle contract (graceful stop does not target the
    // conductor) instead of racing the conductor's normal completion.
    let conductor = seed_unleased_legacy_conductor(&scope);
    std::fs::write(
        scope.join("conductor.pid"),
        format!(
            "{{\"pid\":{},\"repo\":\"berlinguyinca/autospec\",\"scope\":\"berlinguyinca_autospec\"}}\n",
            conductor
        ),
    )
    .expect("running conductor metadata");

    let output = autospec()
        .args([
            "autonomous",
            "stop",
            "--repo",
            "berlinguyinca/autospec",
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_STATE_DIR", temp.join("state"))
        .env("AUTOSPEC_AUTONOMOUS_SPEND_DIR", temp.join("spend"))
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .env("AUTOSPEC_STOP_FLAG_FILE", &stop_flag)
        .output()
        .expect("autospec autonomous stop runs");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let flag = std::fs::read_to_string(&stop_flag).expect("stop flag written");

    assert!(output.status.success());
    assert!(stdout.contains("\"mode\":\"graceful\""));
    assert!(stdout.contains("\"stop_flag\""));
    assert!(flag.starts_with("graceful\n"));
    assert!(process_is_alive(&conductor));

    let target_status = autonomous_status(&operator_dir, &log_dir, "berlinguyinca/autospec");
    assert!(target_status.contains("\"conductor\":{\"running\":true"));
    assert!(target_status.contains("\"monitor\":{\"running\":false"));
    assert!(target_status.contains("\"supervisor\":{\"running\":false"));

    terminate_fixture_process(&original_conductor);
    cleanup_pids(&scope);
    assert!(!process_is_alive(&original_conductor));
}

#[test]
fn autonomous_list_json_reports_each_repo_scope_with_companions() {
    let temp = temp_dir("autospec-autonomous-list");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let repo_a = temp.join("repo-a");
    let repo_b = temp.join("repo-b");
    std::fs::create_dir_all(&repo_a).expect("repo a");
    std::fs::create_dir_all(&repo_b).expect("repo b");

    start_sleeping_autonomous(&operator_dir, &log_dir, &repo_a, "berlinguyinca/autospec");
    start_sleeping_autonomous(
        &operator_dir,
        &log_dir,
        &repo_b,
        "metabolomics-us/go-modules",
    );

    let output = autospec()
        .args(["autonomous", "list", "--json"])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_STATE_DIR", temp.join("state"))
        .env("AUTOSPEC_AUTONOMOUS_SPEND_DIR", temp.join("spend"))
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .output()
        .expect("autospec autonomous list runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("\"subcommand\":\"list\""));
    assert!(stdout.contains("\"scope\":\"berlinguyinca_autospec\""));
    assert!(stdout.contains("\"scope\":\"metabolomics-us_go-modules\""));
    assert!(stdout.contains("\"conductor\":{\"running\":true"));
    assert!(stdout.contains("\"monitor\":{\"running\":true"));
    assert!(stdout.contains("\"supervisor\":{\"running\":true"));

    cleanup_pids(&operator_dir.join("berlinguyinca_autospec"));
    cleanup_pids(&operator_dir.join("metabolomics-us_go-modules"));
}

#[test]
fn autonomous_status_marks_stale_metadata_for_dead_pids() {
    let temp = temp_dir("autospec-autonomous-stale");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let scope = operator_dir.join("berlinguyinca_autospec");
    std::fs::create_dir_all(&scope).expect("scope dir");
    std::fs::write(scope.join("conductor.pid"), "999999\n").expect("pid");
    std::fs::write(
        scope.join("conductor.logpath"),
        "/tmp/missing-autospec.log\n",
    )
    .expect("logpath");

    let output = autospec()
        .args([
            "autonomous",
            "status",
            "--repo",
            "berlinguyinca/autospec",
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_STATE_DIR", temp.join("state"))
        .env("AUTOSPEC_AUTONOMOUS_SPEND_DIR", temp.join("spend"))
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .output()
        .expect("autospec autonomous status runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("\"conductor\":{\"running\":false"));
    assert!(stdout.contains("\"stale_pid\":true"));
    assert!(stdout.contains("\"metadata_only\":true"));
}

#[test]
fn autonomous_logs_json_reads_recorded_conductor_log_tail() {
    let temp = temp_dir("autospec-autonomous-logs");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let scope = operator_dir.join("berlinguyinca_autospec");
    let log = temp.join("conductor.log");
    std::fs::create_dir_all(&scope).expect("scope dir");
    std::fs::write(&log, "first-line\nimplemented feature x\ndone\n").expect("log");
    std::fs::write(
        scope.join("conductor.logpath"),
        format!("{}\n", log.display()),
    )
    .expect("logpath");

    let output = autospec()
        .args([
            "autonomous",
            "logs",
            "--repo",
            "berlinguyinca/autospec",
            "--lines",
            "2",
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_STATE_DIR", temp.join("state"))
        .env("AUTOSPEC_AUTONOMOUS_SPEND_DIR", temp.join("spend"))
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .output()
        .expect("autospec autonomous logs runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("\"subcommand\":\"logs\""));
    assert!(stdout.contains("implemented feature x"));
    assert!(stdout.contains("done"));
    assert!(!stdout.contains("first-line"));
}

#[test]
fn autonomous_watch_once_reads_recorded_conductor_log_tail() {
    let temp = temp_dir("autospec-autonomous-watch");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let scope = operator_dir.join("berlinguyinca_autospec");
    let log = temp.join("conductor.log");
    std::fs::create_dir_all(&scope).expect("scope dir");
    std::fs::write(&log, "alpha\nbeta\n").expect("log");
    std::fs::write(
        scope.join("conductor.logpath"),
        format!("{}\n", log.display()),
    )
    .expect("logpath");

    let output = autospec()
        .args([
            "autonomous",
            "watch",
            "--repo",
            "berlinguyinca/autospec",
            "--once",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_STATE_DIR", temp.join("state"))
        .env("AUTOSPEC_AUTONOMOUS_SPEND_DIR", temp.join("spend"))
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .output()
        .expect("autospec autonomous watch runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("alpha"));
    assert!(stdout.contains("beta"));
}

#[test]
fn autonomous_timeline_summarizes_recorded_conductor_log() {
    let temp = temp_dir("autospec-autonomous-timeline");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let home = temp.join("home");
    let scope = operator_dir.join("berlinguyinca_autospec");
    let log = temp.join("conductor.log");
    std::fs::create_dir_all(&scope).expect("scope dir");
    std::fs::write(
        &log,
        "2026-07-11T04:30:00Z implemented feature x\n2026-07-11T04:45:00Z started research on y\n",
    )
    .expect("log");
    std::fs::write(
        scope.join("conductor.logpath"),
        format!("{}\n", log.display()),
    )
    .expect("logpath");

    let output = autospec()
        .args([
            "autonomous",
            "timeline",
            "--repo",
            "berlinguyinca/autospec",
            "--lines",
            "5",
        ])
        .env("HOME", &home)
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .output()
        .expect("autospec autonomous timeline runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("implemented feature x"));
    assert!(stdout.contains("started research on y"));
}

#[test]
fn autonomous_timeline_reports_forecast_and_planned_steps() {
    let temp = temp_dir("autospec-autonomous-timeline-forecast");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let home = temp.join("home");
    let log = write_conductor_log(
        &operator_dir,
        "berlinguyinca_autospec",
        "{
  \"ready\": [
    {\"number\": 1538, \"title\": \"feat: autonomous UX/UI optimization tier\"},
    {\"number\": 1539, \"title\": \"feat: autonomous accessibility standards tier\"}
  ],
  \"blocked\": [
    {\"number\": 1540, \"title\": \"feat: documentation freshness tier\"}
  ],
  \"claimed\": [
    {\"number\": 1537, \"title\": \"feat: proactive security scanning workstream\"}
  ],
  \"batch\": [
    {\"number\": 1538, \"title\": \"feat: autonomous UX/UI optimization tier\"}
  ]
}
",
    );

    let output = autospec()
        .args([
            "autonomous",
            "timeline",
            "--repo",
            "berlinguyinca/autospec",
            "--lines",
            "80",
        ])
        .env("HOME", &home)
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .output()
        .expect("autospec autonomous timeline runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success(), "log={}", log.display());
    assert!(stdout.contains("autospec-autonomous forecast"));
    assert!(stdout.contains("things left: 4 total (2 ready, 1 in progress, 1 blocked)"));
    assert!(stdout.contains("rough ETA: about 3-6 hours"));
    assert!(
        stdout.contains("planned next: finish #1537 feat: proactive security scanning workstream")
    );
    assert!(stdout.contains("then start #1538 feat: autonomous UX/UI optimization tier"));
    assert!(stdout.contains("blocked later: #1540 feat: documentation freshness tier"));
}

#[test]
fn autonomous_timeline_rate_limits_leader_nudges_and_reports_helper_recovery() {
    let temp = temp_dir("autospec-autonomous-helper-recovery");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let home = temp.join("home");
    write_conductor_log(
        &operator_dir,
        "berlinguyinca_autospec",
        "2026-07-19T01:00:00Z SpawnAgent full-history helper failed: unavailable
2026-07-19T01:00:01Z collab: Wait
2026-07-19T01:00:02Z leader_nudge_tick issue=1847
2026-07-19T01:00:03Z leader_nudge_tick issue=1847
2026-07-19T01:00:04Z leader_nudge_tick issue=1847
2026-07-19T01:00:05Z continuing without helper after SpawnAgent failure
",
    );

    let output = autospec()
        .args([
            "autonomous",
            "timeline",
            "--repo",
            "berlinguyinca/autospec",
            "--lines",
            "80",
        ])
        .env("HOME", &home)
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .output()
        .expect("autospec autonomous timeline runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("retrying-helper"));
    assert!(stdout.contains("continuing-without-helper"));
    assert_eq!(stdout.matches("leader_nudge_tick").count(), 1, "{stdout}");
    assert!(stdout.contains("suppressed 2 repeated leader nudge tick"));
}

#[test]
fn autonomous_timeline_requires_an_explicit_event_before_reporting_failed_startup() {
    let temp = temp_dir("autospec-autonomous-session-start-timeout");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let home = temp.join("home");
    write_conductor_log(
        &operator_dir,
        "berlinguyinca_autospec",
        "2026-07-19T01:00:00Z session_start child=child-1850 session_turns=0 issue_claim=none\n\
2026-07-19T01:00:01Z session_start child=child-1851 session_turns=0 issue_claim=none\n\
2026-07-19T01:06:00Z session_start_reconciled child=child-1851 session_turns=1 issue_claim=1851\n\
2026-07-19T01:06:00Z leader_nudge_tick issue=1850\n",
    );

    let output = autospec()
        .args([
            "autonomous",
            "timeline",
            "--repo",
            "berlinguyinca/autospec",
            "--lines",
            "80",
        ])
        .env("HOME", &home)
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .output()
        .expect("autospec autonomous timeline runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("session child-1850 entered starting"));
    assert!(stdout.contains("session child-1851 entered claimed"));
    assert!(!stdout.contains("failed-startup"));
    assert!(!stdout.contains("terminated and queued for retry"));
}

#[test]
fn autonomous_timeline_reports_explicit_startup_timeout_and_retry_events() {
    let temp = temp_dir("autospec-autonomous-explicit-session-start-timeout");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let home = temp.join("home");
    let drain_state = operator_dir.join("o13_berlinguyinca_r8_autospec");
    std::fs::create_dir_all(&drain_state).expect("create drain state directory");
    std::fs::write(
        drain_state.join("drain-session-events.jsonl"),
        "{\"event\":\"session_start_observed\",\"session_id\":\"child-1850\",\"state\":\"starting\"}\n\
{\"event\":\"session_start_timeout\",\"session_id\":\"child-1850\",\"state\":\"failed-startup\",\"termination\":\"process_group\",\"retry\":\"scheduled\",\"attempt\":1}\n",
    )
    .expect("write persisted drain events");

    let output = autospec()
        .args([
            "autonomous",
            "timeline",
            "--repo",
            "berlinguyinca/autospec",
            "--lines",
            "80",
        ])
        .env("HOME", &home)
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .output()
        .expect("autospec autonomous timeline runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("session child-1850 entered failed-startup"));
    assert!(stdout.contains("terminated and queued for retry"));
}

#[test]
fn autonomous_timeline_reports_item_timing() {
    let temp = temp_dir("autospec-autonomous-timeline-timing");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let home = temp.join("home");
    write_conductor_log(
        &operator_dir,
        "berlinguyinca_autospec",
        "{\"issue\":\"1539\",\"branch\":\"feat/issue-1539-accessibility\",\"step\":\"claimed\",\"ts\":1783440000,\"repo\":\"berlinguyinca/autospec\"}
{\"issue\":\"1539\",\"branch\":\"feat/issue-1539-accessibility\",\"step\":\"tests_started\",\"ts\":1783441800,\"repo\":\"berlinguyinca/autospec\"}
{\"issue\":\"1538\",\"branch\":\"feat/issue-1538-ux\",\"step\":\"claimed\",\"ts\":1783430000,\"repo\":\"berlinguyinca/autospec\"}
{\"issue\":\"1538\",\"branch\":\"feat/issue-1538-ux\",\"step\":\"merged\",\"ts\":1783437200,\"repo\":\"berlinguyinca/autospec\"}
",
    );

    let output = autospec()
        .args([
            "autonomous",
            "timeline",
            "--repo",
            "berlinguyinca/autospec",
            "--lines",
            "80",
        ])
        .env("HOME", &home)
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .output()
        .expect("autospec autonomous timeline runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("item timing"));
    assert!(stdout.contains("#1539 current step tests started after 30 minutes"));
    assert!(stdout.contains("#1538 completed in 2 hours"));
}

#[test]
fn autonomous_timeline_reconciles_heartbeat_active_issue() {
    let temp = temp_dir("autospec-autonomous-timeline-heartbeat");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let home = temp.join("home");
    write_conductor_log(
        &operator_dir,
        "berlinguyinca_autospec",
        "{
  \"ready\": [
    {\"number\": 1543, \"title\": \"feat: autonomy guardrails\"},
    {\"number\": 1544, \"title\": \"feat: immutable verifier\"}
  ],
  \"blocked\": [],
  \"claimed\": [],
  \"batch\": [
    {\"number\": 1543, \"title\": \"feat: autonomy guardrails\"}
  ]
}
",
    );
    let heartbeat_dir = home
        .join(".autospec")
        .join("process-heartbeats")
        .join("berlinguyinca__autospec");
    std::fs::create_dir_all(&heartbeat_dir).expect("heartbeat dir");
    std::fs::write(
        heartbeat_dir.join("1543.json"),
        "{\"issue\":\"1543\",\"branch\":\"feat/issue-1543-autonomy-guardrails\",\"step\":\"claimed\",\"ts\":1783466193,\"repo\":\"berlinguyinca/autospec\"}\n",
    )
    .expect("heartbeat");

    let output = autospec()
        .args([
            "autonomous",
            "timeline",
            "--repo",
            "berlinguyinca/autospec",
            "--lines",
            "80",
        ])
        .env("HOME", &home)
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .output()
        .expect("autospec autonomous timeline runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("things left: 2 total (1 ready, 1 in progress, 0 blocked)"));
    assert!(stdout.contains("planned next: finish #1543 feat: autonomy guardrails"));
    assert!(stdout.contains("then start #1544 feat: immutable verifier"));
}

#[test]
fn autonomous_start_without_repo_uses_git_remote_scope() {
    let temp = temp_dir("autospec-autonomous-git-scope");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let repo_dir = temp.join("repo");
    make_git_repo(&repo_dir, Some("https://github.com/example/rust-scope.git"));

    let output = autospec()
        .args([
            "autonomous",
            "start",
            "--repo-dir",
            repo_dir.to_str().unwrap(),
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_STATE_DIR", temp.join("state"))
        .env("AUTOSPEC_AUTONOMOUS_SPEND_DIR", temp.join("spend"))
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .env("PATH", hermetic_autonomous_path(&temp))
        .output()
        .expect("autospec autonomous start runs");

    assert!(output.status.success());
    assert!(operator_dir.join("example_rust-scope").exists());
    cleanup_pids(&operator_dir.join("example_rust-scope"));
}

#[test]
fn autonomous_start_non_git_repo_dir_fails_loud() {
    let temp = temp_dir("autospec-autonomous-nongit");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let repo_dir = temp.join("nongit");
    std::fs::create_dir_all(&repo_dir).expect("repo dir");

    let output = autospec()
        .args([
            "autonomous",
            "start",
            "--repo-dir",
            repo_dir.to_str().unwrap(),
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_STATE_DIR", temp.join("state"))
        .env("AUTOSPEC_AUTONOMOUS_SPEND_DIR", temp.join("spend"))
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .output()
        .expect("autospec autonomous start runs");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success());
    assert!(stderr.contains("not a git checkout"));
    assert!(stderr.contains("--repo-dir"));
    assert!(!operator_dir.exists());
}

#[test]
fn autonomous_start_mismatched_repo_warns_but_launches() {
    let temp = temp_dir("autospec-autonomous-mismatch");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let repo_dir = temp.join("repo");
    make_git_repo(&repo_dir, Some("https://github.com/acme/widget.git"));

    let output = autospec()
        .args([
            "autonomous",
            "start",
            "--repo",
            "other-owner/other-repo",
            "--repo-dir",
            repo_dir.to_str().unwrap(),
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_STATE_DIR", temp.join("state"))
        .env("AUTOSPEC_AUTONOMOUS_SPEND_DIR", temp.join("spend"))
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .env("PATH", hermetic_autonomous_path(&temp))
        .output()
        .expect("autospec autonomous start runs");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(output.status.success());
    assert!(stderr.contains("warning"));
    assert!(stderr.contains("other-owner/other-repo"));
    assert!(operator_dir.join("other-owner_other-repo").exists());
    cleanup_pids(&operator_dir.join("other-owner_other-repo"));
}

#[test]
fn autonomous_start_writes_launch_provenance_and_list_reports_it() {
    let temp = temp_dir("autospec-autonomous-launch");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let repo_dir = temp.join("repo");
    make_git_repo(&repo_dir, None);

    start_sleeping_autonomous(&operator_dir, &log_dir, &repo_dir, "berlinguyinca/autospec");
    let scope = operator_dir.join("berlinguyinca_autospec");
    let launch = scope.join("launch.json");
    assert!(launch.exists(), "missing {}", launch.display());
    assert!(std::fs::read_to_string(&launch)
        .expect("launch json")
        .contains("\"repo\":\"berlinguyinca/autospec\""));
    let lifecycle = scope.join("lifecycle.json");
    assert!(lifecycle.exists(), "missing {}", lifecycle.display());
    assert!(std::fs::read_to_string(&lifecycle)
        .expect("lifecycle json")
        .contains("\"decision\":\"run\""));

    let output = autospec()
        .args(["autonomous", "list", "--json"])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_STATE_DIR", temp.join("state"))
        .env("AUTOSPEC_AUTONOMOUS_SPEND_DIR", temp.join("spend"))
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .output()
        .expect("autospec autonomous list runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("\"launch\""));
    assert!(stdout.contains("\"repo\":\"berlinguyinca/autospec\""));
    cleanup_pids(&scope);
}

#[test]
fn autonomous_list_reports_empty_object_for_malformed_launch_json() {
    let temp = temp_dir("autospec-autonomous-launch-malformed");
    let operator_dir = temp.join("operator");
    let scope = operator_dir.join("berlinguyinca_autospec");
    std::fs::create_dir_all(&scope).expect("scope dir");
    std::fs::write(scope.join("launch.json"), "not-json").expect("launch json");

    let output = autospec()
        .args(["autonomous", "list", "--json"])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_STATE_DIR", temp.join("state"))
        .env("AUTOSPEC_AUTONOMOUS_SPEND_DIR", temp.join("spend"))
        .output()
        .expect("autospec autonomous list runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("\"scope\":\"berlinguyinca_autospec\""));
    assert!(stdout.contains("\"launch\":{}"));
}

#[test]
fn autonomous_start_records_argv_and_passthrough_options_in_launch_provenance() {
    let temp = temp_dir("autospec-autonomous-launch-argv");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let repo_dir = temp.join("repo");
    make_git_repo(&repo_dir, None);

    let output = autospec()
        .args([
            "autonomous",
            "start",
            "--repo",
            "berlinguyinca/autospec",
            "--repo-dir",
            repo_dir.to_str().unwrap(),
            "--max-cycles",
            "3",
            "--budget-tokens",
            "1000",
            "--budget-issues",
            "4",
            "--no-digest",
            "--poll-interval-sec",
            "15",
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_STATE_DIR", temp.join("state"))
        .env("AUTOSPEC_AUTONOMOUS_SPEND_DIR", temp.join("spend"))
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .env("PATH", hermetic_autonomous_path(&temp))
        .output()
        .expect("autospec autonomous start runs");
    let launch = std::fs::read_to_string(
        operator_dir
            .join("berlinguyinca_autospec")
            .join("launch.json"),
    )
    .expect("launch json");

    assert!(output.status.success());
    assert!(launch.contains("\"argv\":["));
    assert!(launch.contains("\"--max-cycles\""));
    assert!(launch.contains("\"max_cycles\":\"3\""));
    assert!(launch.contains("\"budget_tokens\":\"1000\""));
    assert!(launch.contains("\"budget_issues\":\"4\""));
    assert!(launch.contains("\"no_digest\":true"));
    assert!(launch.contains("\"poll_interval_sec\":\"15\""));
    let launch: serde_json::Value = serde_json::from_str(&launch).expect("parse launch json");
    let conductor = launch["conductor_argv"].as_array().expect("conductor argv");
    let supervisor = launch["supervisor_argv"]
        .as_array()
        .expect("supervisor argv");
    assert!(conductor.windows(2).any(|values| {
        values[0].as_str() == Some("--poll-interval-sec") && values[1].as_str() == Some("15")
    }));
    assert!(supervisor.windows(2).any(|values| {
        values[0].as_str() == Some("--repair-interval-sec") && values[1].as_str() == Some("5")
    }));
    cleanup_pids(&operator_dir.join("berlinguyinca_autospec"));
}

#[test]
fn autonomous_start_default_conductor_forwards_enforced_backend_options() {
    let output = autospec()
        .args([
            "autonomous",
            "start",
            "--repo",
            "berlinguyinca/autospec",
            "--repo-dir",
            "/tmp/autospec",
            "--max-cycles",
            "3",
            "--budget-tokens",
            "1000",
            "--budget-issues",
            "4",
            "--no-digest",
            "--poll-interval-sec",
            "15",
            "--dry-run",
            "--json",
        ])
        .output()
        .expect("autospec autonomous start runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("autospec autonomous run-foreground"));
    assert!(stdout.contains("--max-cycles 3"));
    assert!(stdout.contains("--budget-tokens 1000"));
    assert!(stdout.contains("--budget-issues 4"));
    assert!(stdout.contains("--no-digest"));
    assert!(stdout.contains("--poll-interval-sec 15"));
    assert!(stdout.contains("--dry-run"));
}

#[test]
fn autonomous_start_companion_opt_out_skips_monitor_and_supervisor_processes() {
    let temp = temp_dir("autospec-autonomous-no-companions");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let repo_dir = temp.join("repo");
    make_git_repo(&repo_dir, None);

    let output = autospec()
        .args([
            "autonomous",
            "start",
            "--repo",
            "berlinguyinca/autospec",
            "--repo-dir",
            repo_dir.to_str().unwrap(),
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_STATE_DIR", temp.join("state"))
        .env("AUTOSPEC_AUTONOMOUS_SPEND_DIR", temp.join("spend"))
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .env("PATH", hermetic_autonomous_path(&temp))
        .env("AUTOSPEC_AUTONOMOUS_COMPANIONS", "0")
        .output()
        .expect("autospec autonomous start runs");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let scope = operator_dir.join("berlinguyinca_autospec");

    assert!(output.status.success());
    assert!(stdout.contains("\"conductor\":{\"pid\":\""));
    assert!(stdout.contains("\"monitor\":{\"pid\":\"\""));
    assert!(stdout.contains("\"supervisor\":{\"pid\":\"\""));
    assert!(!scope.join("monitor.pid").exists());
    assert!(!scope.join("supervisor.pid").exists());
    cleanup_pids(&scope);
}

#[test]
fn autonomous_stop_defaults_to_repo_scoped_stop_flag() {
    let temp = temp_dir("autospec-autonomous-scoped-stop");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");

    let output = autospec()
        .args([
            "autonomous",
            "stop",
            "--repo",
            "berlinguyinca/autospec",
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_STATE_DIR", temp.join("state"))
        .env("AUTOSPEC_AUTONOMOUS_SPEND_DIR", temp.join("spend"))
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .output()
        .expect("autospec autonomous stop runs");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let scoped_flag = operator_dir
        .join("berlinguyinca_autospec")
        .join("stop.flag");

    assert!(output.status.success());
    assert!(stdout.contains(&json_path(&scoped_flag)));
    assert_eq!(
        std::fs::read_to_string(&scoped_flag)
            .expect("scoped stop flag")
            .lines()
            .next(),
        Some("graceful")
    );
}

#[test]
fn autonomous_monitor_and_supervise_accept_shell_compatibility_options() {
    let monitor = autospec()
        .args([
            "autonomous",
            "monitor",
            "--iterations",
            "1",
            "--log",
            "/tmp/ignored.log",
        ])
        .output()
        .expect("autospec autonomous monitor runs");
    let supervisor = autospec()
        .args([
            "autonomous",
            "supervise",
            "--iterations",
            "1",
            "--force",
            "--log",
            "/tmp/ignored.log",
        ])
        .output()
        .expect("autospec autonomous supervise runs");

    assert!(monitor.status.success());
    assert!(supervisor.status.success());
}

#[test]
fn autonomous_start_rejects_unsupported_budget_hours() {
    let output = autospec()
        .args([
            "autonomous",
            "start",
            "--repo",
            "berlinguyinca/autospec",
            "--repo-dir",
            "/tmp/autospec",
            "--budget-hours",
            "2",
            "--dry-run",
            "--json",
        ])
        .output()
        .expect("autospec autonomous start runs");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success());
    assert!(stderr.contains("--budget-hours is not supported"));
}

#[test]
fn autonomous_start_uses_explicit_conductor_log_path() {
    let temp = temp_dir("autospec-autonomous-explicit-log");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let repo_dir = temp.join("repo");
    let explicit_log = temp.join("custom").join("conductor.log");
    make_git_repo(&repo_dir, None);

    let output = autospec()
        .args([
            "autonomous",
            "start",
            "--repo",
            "berlinguyinca/autospec",
            "--repo-dir",
            repo_dir.to_str().unwrap(),
            "--log",
            explicit_log.to_str().unwrap(),
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_STATE_DIR", temp.join("state"))
        .env("AUTOSPEC_AUTONOMOUS_SPEND_DIR", temp.join("spend"))
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .env("PATH", hermetic_autonomous_path(&temp))
        .output()
        .expect("autospec autonomous start runs");
    let scope = operator_dir.join("berlinguyinca_autospec");

    assert!(output.status.success());
    assert_eq!(
        std::fs::read_to_string(scope.join("conductor.logpath"))
            .expect("logpath")
            .trim(),
        explicit_log.to_str().unwrap()
    );
    assert!(explicit_log.exists());
    cleanup_pids(&scope);
}

#[test]
fn autonomous_start_rejects_a_fresh_existing_lease_without_force() {
    let temp = temp_dir("autospec-autonomous-duplicate");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let repo_dir = temp.join("repo");
    let state_dir = temp.join("state");
    make_git_repo(&repo_dir, None);
    start_sleeping_autonomous_with_state(
        &operator_dir,
        &log_dir,
        &repo_dir,
        "berlinguyinca/autospec",
        &state_dir,
    );
    let scope = operator_dir.join("berlinguyinca_autospec");
    let original = read_pid(&scope, "conductor");

    let output = autospec()
        .args([
            "autonomous",
            "start",
            "--repo",
            "berlinguyinca/autospec",
            "--repo-dir",
            repo_dir.to_str().unwrap(),
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_STATE_DIR", &state_dir)
        .env("AUTOSPEC_AUTONOMOUS_SPEND_DIR", temp.join("spend"))
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .env("PATH", hermetic_autonomous_path(&temp))
        .output()
        .expect("autospec autonomous start runs");

    assert_lease_held(&output);
    assert_eq!(read_pid(&scope, "conductor"), original);
    assert!(process_is_alive(&original));
    cleanup_pids(&scope);
}

#[test]
fn autonomous_start_force_rejects_a_fresh_existing_lease_without_killing_conductor() {
    let temp = temp_dir("autospec-autonomous-force");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let repo_dir = temp.join("repo");
    let state_dir = temp.join("state");
    make_git_repo(&repo_dir, None);
    start_sleeping_autonomous_with_state(
        &operator_dir,
        &log_dir,
        &repo_dir,
        "berlinguyinca/autospec",
        &state_dir,
    );
    let scope = operator_dir.join("berlinguyinca_autospec");
    let original = read_pid(&scope, "conductor");

    let output = autospec()
        .args([
            "autonomous",
            "start",
            "--repo",
            "berlinguyinca/autospec",
            "--repo-dir",
            repo_dir.to_str().unwrap(),
            "--force",
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_STATE_DIR", &state_dir)
        .env("AUTOSPEC_AUTONOMOUS_SPEND_DIR", temp.join("spend"))
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .env("PATH", hermetic_autonomous_path(&temp))
        .output()
        .expect("autospec autonomous start runs");

    assert_lease_held(&output);
    assert_eq!(read_pid(&scope, "conductor"), original);
    assert!(process_is_alive(&original));
    cleanup_pids(&scope);
}

#[test]
fn autonomous_start_force_replaces_unleased_legacy_conductor_metadata() {
    let temp = temp_dir("autospec-autonomous-force-legacy-metadata");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let repo_dir = temp.join("repo");
    let state_dir = temp.join("state");
    make_git_repo(&repo_dir, None);
    let scope = operator_dir.join("berlinguyinca_autospec");
    let original = seed_unleased_legacy_conductor(&scope);
    assert!(!fresh_lease_state_path(&state_dir).exists());

    let output = autospec()
        .args([
            "autonomous",
            "start",
            "--repo",
            "berlinguyinca/autospec",
            "--repo-dir",
            repo_dir.to_str().unwrap(),
            "--force",
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_STATE_DIR", &state_dir)
        .env("AUTOSPEC_AUTONOMOUS_SPEND_DIR", temp.join("spend"))
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .env("PATH", hermetic_autonomous_path(&temp))
        .output()
        .expect("autospec autonomous start runs");
    let replacement = read_pid(&scope, "conductor");

    assert!(output.status.success());
    assert_ne!(original, replacement);
    assert!(!process_is_alive(&original));
    assert!(process_is_alive(&replacement));
    cleanup_pids(&scope);
}

#[test]
fn autonomous_logs_falls_back_to_newest_legacy_flat_log() {
    let temp = temp_dir("autospec-autonomous-legacy-log");
    let operator_dir = temp.join("operator");
    let home = temp.join("home");
    let log_root = home.join(".autospec").join("logs");
    let scope = operator_dir.join("berlinguyinca_autospec");
    std::fs::create_dir_all(&scope).expect("scope dir");
    std::fs::create_dir_all(&log_root).expect("log root");
    std::fs::write(scope.join("conductor.pid"), "999999\n").expect("pid");
    std::fs::write(
        log_root.join("autospec-autonomous-20260708T100000Z.log"),
        "old legacy\n",
    )
    .expect("old log");
    std::fs::write(
        log_root.join("autospec-autonomous-20260708T110000Z.log"),
        "new legacy\n",
    )
    .expect("new log");

    let output = autospec()
        .args([
            "autonomous",
            "logs",
            "--repo",
            "berlinguyinca/autospec",
            "--lines",
            "1",
        ])
        .env("HOME", &home)
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .output()
        .expect("autospec autonomous logs runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert_eq!(stdout.trim(), "new legacy");
}

#[test]
fn autonomous_cleanup_removes_dead_metadata_without_killing_live_units() {
    let temp = temp_dir("autospec-autonomous-cleanup");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let dead_scope = operator_dir.join("berlinguyinca_autospec");
    let live_repo = temp.join("live-repo");
    std::fs::create_dir_all(&dead_scope).expect("dead scope");
    std::fs::create_dir_all(&live_repo).expect("live repo");
    std::fs::write(dead_scope.join("conductor.pid"), "999999\n").expect("dead pid");
    std::fs::write(
        dead_scope.join("conductor.logpath"),
        "/tmp/dead-autospec.log\n",
    )
    .expect("dead logpath");
    start_sleeping_autonomous(
        &operator_dir,
        &log_dir,
        &live_repo,
        "metabolomics-us/go-modules",
    );

    let output = autospec()
        .args([
            "autonomous",
            "cleanup",
            "--repo",
            "berlinguyinca/autospec",
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_STATE_DIR", temp.join("state"))
        .env("AUTOSPEC_AUTONOMOUS_SPEND_DIR", temp.join("spend"))
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .output()
        .expect("autospec autonomous cleanup runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("\"removed\":2"));
    assert!(!dead_scope.join("conductor.pid").exists());
    let other_status = autonomous_status(&operator_dir, &log_dir, "metabolomics-us/go-modules");
    assert!(other_status.contains("\"conductor\":{\"running\":true"));
    cleanup_pids(&operator_dir.join("metabolomics-us_go-modules"));
}

#[test]
fn autonomous_supervise_reports_stale_metadata_action_for_dead_recorded_pid() {
    let temp = temp_dir("autospec-autonomous-supervise-stale");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let scope = operator_dir.join("berlinguyinca_autospec");
    std::fs::create_dir_all(&scope).expect("scope dir");
    std::fs::write(scope.join("conductor.pid"), "999999\n").expect("pid");

    let output = autospec()
        .args([
            "autonomous",
            "supervise",
            "--repo",
            "berlinguyinca/autospec",
            "--once",
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_STATE_DIR", temp.join("state"))
        .env("AUTOSPEC_AUTONOMOUS_SPEND_DIR", temp.join("spend"))
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .output()
        .expect("autospec autonomous supervise runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("\"action\":\"stale-metadata\""));
}

#[test]
fn autonomous_supervise_relaunches_a_dead_conductor_with_a_fresh_lease() {
    let temp = temp_dir("autospec-autonomous-supervise-relaunch");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let state_dir = temp.join("state");
    let repo_dir = temp.join("repo");
    make_git_repo(&repo_dir, None);
    let scope = operator_dir.join("berlinguyinca_autospec");

    let start = autospec()
        .args([
            "autonomous",
            "start",
            "--repo",
            "berlinguyinca/autospec",
            "--repo-dir",
            repo_dir.to_str().unwrap(),
            "--max-cycles",
            "7",
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_STATE_DIR", &state_dir)
        .env("AUTOSPEC_AUTONOMOUS_SPEND_DIR", temp.join("spend"))
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .env("AUTOSPEC_AUTONOMOUS_COMPANIONS", "0")
        .env("PATH", hermetic_autonomous_path(&temp))
        .output()
        .expect("autospec autonomous start runs");
    assert!(start.status.success());
    let old_pid = cleanup_pid(&read_pid(&scope, "conductor")).expect("conductor pid");
    let observe_live = autospec()
        .args([
            "autonomous",
            "supervise",
            "--repo",
            "berlinguyinca/autospec",
            "--repo-dir",
            repo_dir.to_str().unwrap(),
            "--max-cycles",
            "7",
            "--once",
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_STATE_DIR", &state_dir)
        .env("AUTOSPEC_AUTONOMOUS_SPEND_DIR", temp.join("spend"))
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .env("PATH", hermetic_autonomous_path(&temp))
        .output()
        .expect("autospec autonomous supervise observes live conductor");
    assert!(String::from_utf8_lossy(&observe_live.stdout).contains("\"action\":\"none\""));
    assert_eq!(
        cleanup_pid(&read_pid(&scope, "conductor")).as_deref(),
        Some(old_pid.as_str())
    );
    let launch = std::fs::read_to_string(scope.join("launch.json")).expect("launch manifest");
    assert!(launch.contains("\"supervisor_argv\""));
    assert!(launch.contains("\"--max-cycles\",\"7\""));
    terminate_fixture_process(&old_pid);

    let repair = autospec()
        .args([
            "autonomous",
            "supervise",
            "--repo",
            "berlinguyinca/autospec",
            "--repo-dir",
            repo_dir.to_str().unwrap(),
            "--max-cycles",
            "7",
            "--once",
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_STATE_DIR", &state_dir)
        .env("AUTOSPEC_AUTONOMOUS_SPEND_DIR", temp.join("spend"))
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .env("PATH", hermetic_autonomous_path(&temp))
        .output()
        .expect("autospec autonomous supervise runs");
    let stdout = String::from_utf8_lossy(&repair.stdout);
    let new_pid = cleanup_pid(&read_pid(&scope, "conductor")).expect("replacement pid");

    assert!(
        repair.status.success(),
        "{}",
        String::from_utf8_lossy(&repair.stderr)
    );
    assert!(
        stdout.contains("\"action\":\"restarted-conductor\""),
        "stdout={stdout} stderr={}",
        String::from_utf8_lossy(&repair.stderr)
    );
    assert_ne!(old_pid, new_pid);
    assert!(process_is_alive(&new_pid));
    cleanup_pids(&scope);
}

#[test]
fn autonomous_supervise_releases_a_fresh_lease_when_relaunch_preparation_fails() {
    let temp = temp_dir("autospec-autonomous-supervise-retry");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let state_dir = temp.join("state");
    let repo_dir = temp.join("repo");
    make_git_repo(&repo_dir, None);
    let scope = operator_dir.join("berlinguyinca_autospec");

    let start = autospec()
        .args([
            "autonomous",
            "start",
            "--repo",
            "berlinguyinca/autospec",
            "--repo-dir",
            repo_dir.to_str().unwrap(),
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_STATE_DIR", &state_dir)
        .env("AUTOSPEC_AUTONOMOUS_SPEND_DIR", temp.join("spend"))
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .env("AUTOSPEC_AUTONOMOUS_COMPANIONS", "0")
        .env("PATH", hermetic_autonomous_path(&temp))
        .output()
        .expect("autospec autonomous start runs");
    assert!(start.status.success());
    let old_pid = cleanup_pid(&read_pid(&scope, "conductor")).expect("conductor pid");
    terminate_fixture_process(&old_pid);
    std::fs::remove_dir_all(&log_dir).expect("remove log directory");
    std::fs::write(&log_dir, "blocks log directory creation").expect("obstruct log directory");

    let failed_repair = autospec()
        .args([
            "autonomous",
            "supervise",
            "--repo",
            "berlinguyinca/autospec",
            "--repo-dir",
            repo_dir.to_str().unwrap(),
            "--once",
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_STATE_DIR", &state_dir)
        .env("AUTOSPEC_AUTONOMOUS_SPEND_DIR", temp.join("spend"))
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .env("PATH", hermetic_autonomous_path(&temp))
        .output()
        .expect("autospec autonomous supervise reports deferred repair");
    assert!(failed_repair.status.success());
    assert!(
        String::from_utf8_lossy(&failed_repair.stdout).contains("\"action\":\"repair-deferred\"")
    );

    std::fs::remove_file(&log_dir).expect("remove log obstruction");
    std::fs::create_dir_all(&log_dir).expect("restore log directory");
    let retry = autospec()
        .args([
            "autonomous",
            "supervise",
            "--repo",
            "berlinguyinca/autospec",
            "--repo-dir",
            repo_dir.to_str().unwrap(),
            "--once",
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_STATE_DIR", &state_dir)
        .env("AUTOSPEC_AUTONOMOUS_SPEND_DIR", temp.join("spend"))
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .env("PATH", hermetic_autonomous_path(&temp))
        .output()
        .expect("autospec autonomous supervise retries");
    let stdout = String::from_utf8_lossy(&retry.stdout);
    let new_pid = cleanup_pid(&read_pid(&scope, "conductor")).expect("replacement pid");

    assert!(
        retry.status.success(),
        "{}",
        String::from_utf8_lossy(&retry.stderr)
    );
    assert!(
        stdout.contains("\"action\":\"restarted-conductor\""),
        "stdout={stdout} stderr={}",
        String::from_utf8_lossy(&retry.stderr)
    );
    assert!(process_is_alive(&new_pid));
    cleanup_pids(&scope);
}

#[test]
fn autonomous_supervise_contention_tracks_exactly_one_replacement_conductor() {
    let temp = temp_dir("autospec-autonomous-supervise-contention");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let state_dir = temp.join("state");
    let repo_dir = temp.join("repo");
    make_git_repo(&repo_dir, None);
    let scope = operator_dir.join("berlinguyinca_autospec");
    let path = hermetic_autonomous_path(&temp);

    let start = autospec()
        .args([
            "autonomous",
            "start",
            "--repo",
            "berlinguyinca/autospec",
            "--repo-dir",
            repo_dir.to_str().unwrap(),
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_STATE_DIR", &state_dir)
        .env("AUTOSPEC_AUTONOMOUS_SPEND_DIR", temp.join("spend"))
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .env("AUTOSPEC_AUTONOMOUS_COMPANIONS", "0")
        .env("PATH", &path)
        .output()
        .expect("autospec autonomous start runs");
    assert!(start.status.success());
    let old_pid = cleanup_pid(&read_pid(&scope, "conductor")).expect("conductor pid");
    terminate_fixture_process(&old_pid);

    let supervisor = || {
        let mut command = autospec();
        command
            .args([
                "autonomous",
                "supervise",
                "--repo",
                "berlinguyinca/autospec",
                "--repo-dir",
                repo_dir.to_str().unwrap(),
                "--once",
                "--json",
            ])
            .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
            .env("AUTOSPEC_STATE_DIR", &state_dir)
            .env("AUTOSPEC_AUTONOMOUS_SPEND_DIR", temp.join("spend"))
            .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
            .env("PATH", &path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    };
    let supervisors = (0..4)
        .map(|_| supervisor().spawn().expect("contending supervisor starts"))
        .collect::<Vec<_>>();
    let outputs = supervisors
        .into_iter()
        .map(|child| child.wait_with_output().expect("supervisor exits"))
        .collect::<Vec<_>>();
    let restarted = outputs
        .iter()
        .filter(|output| {
            String::from_utf8_lossy(&output.stdout).contains("\"action\":\"restarted-conductor\"")
        })
        .count();
    let tracked_pid = cleanup_pid(&read_pid(&scope, "conductor")).expect("tracked replacement pid");

    assert!(outputs.iter().all(|output| output.status.success()));
    assert_eq!(restarted, 1, "outputs={outputs:?}");
    assert!(process_is_alive(&tracked_pid));
    let lock = std::fs::symlink_metadata(scope.join("supervisor-repair.lock"))
        .expect("supervisor repair lock metadata");
    assert!(lock.is_file());
    #[cfg(unix)]
    assert_eq!(lock.permissions().mode() & 0o777, 0o600);
    cleanup_pids(&scope);
}

#[test]
fn autonomous_supervise_repair_lock_keeps_ambiguous_metadata_fail_closed() {
    let temp = temp_dir("autospec-autonomous-supervise-ambiguous");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let repo_dir = temp.join("repo");
    make_git_repo(&repo_dir, None);
    let scope = operator_dir.join("berlinguyinca_autospec");
    std::fs::create_dir_all(&scope).expect("create operator scope");
    std::fs::write(scope.join("launch.json"), "{}\n").expect("write launch evidence");
    std::fs::write(scope.join("conductor.pid"), "not-process-metadata\n")
        .expect("write ambiguous conductor metadata");

    let output = autospec()
        .args([
            "autonomous",
            "supervise",
            "--repo",
            "berlinguyinca/autospec",
            "--repo-dir",
            repo_dir.to_str().unwrap(),
            "--once",
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_STATE_DIR", temp.join("state"))
        .env("AUTOSPEC_AUTONOMOUS_SPEND_DIR", temp.join("spend"))
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .env("PATH", hermetic_autonomous_path(&temp))
        .output()
        .expect("supervise ambiguous metadata");

    assert!(output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("\"action\":\"repair-deferred\""),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(scope.join("conductor.pid")).expect("ambiguous metadata remains"),
        "not-process-metadata\n"
    );
    assert!(scope.join("supervisor-repair.lock").is_file());
}

#[test]
fn autonomous_supervise_rechecks_stop_after_waiting_for_repair_lock() {
    let temp = temp_dir("autospec-autonomous-supervise-stop-race");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let state_dir = temp.join("state");
    let repo_dir = temp.join("repo");
    make_git_repo(&repo_dir, None);
    let scope = operator_dir.join("berlinguyinca_autospec");
    let path = hermetic_autonomous_path(&temp);
    let start = autospec()
        .args([
            "autonomous",
            "start",
            "--repo",
            "berlinguyinca/autospec",
            "--repo-dir",
            repo_dir.to_str().unwrap(),
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_STATE_DIR", &state_dir)
        .env("AUTOSPEC_AUTONOMOUS_SPEND_DIR", temp.join("spend"))
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .env("AUTOSPEC_AUTONOMOUS_COMPANIONS", "0")
        .env("PATH", &path)
        .output()
        .expect("start conductor");
    assert!(start.status.success());
    let old_pid = cleanup_pid(&read_pid(&scope, "conductor")).expect("conductor pid");
    terminate_fixture_process(&old_pid);
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(scope.join("supervisor-repair.lock"))
        .expect("open repair lock");
    lock.lock().expect("hold repair lock");

    let child = autospec()
        .args([
            "autonomous",
            "supervise",
            "--repo",
            "berlinguyinca/autospec",
            "--repo-dir",
            repo_dir.to_str().unwrap(),
            "--once",
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_STATE_DIR", &state_dir)
        .env("AUTOSPEC_AUTONOMOUS_SPEND_DIR", temp.join("spend"))
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .env("PATH", &path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start waiting supervisor");
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert!(process_is_alive(&child.id().to_string()));
    std::fs::write(scope.join("stop.flag"), "immediate\noperator@test\n").expect("request stop");
    drop(lock);
    let output = child.wait_with_output().expect("waiting supervisor exits");

    assert!(output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("\"action\":\"stop-requested\""),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(read_pid(&scope, "conductor"), old_pid);
}

#[test]
fn autonomous_supervise_reaps_and_relaunches_two_finite_conductors() {
    let temp = temp_dir("autospec-autonomous-supervise-reap");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let state_dir = temp.join("state");
    let repo_dir = temp.join("repo");
    make_git_repo(&repo_dir, None);
    let scope = operator_dir.join("berlinguyinca_autospec");
    let bin = fake_bin(&temp, None, Some("#!/bin/sh\nexit 1\n"));
    let path = path_with(&bin);

    let start = autospec()
        .args([
            "autonomous",
            "start",
            "--repo",
            "berlinguyinca/autospec",
            "--repo-dir",
            repo_dir.to_str().unwrap(),
            "--max-cycles",
            "1",
            "--interval-sec",
            "1",
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_STATE_DIR", &state_dir)
        .env("AUTOSPEC_AUTONOMOUS_SPEND_DIR", temp.join("spend"))
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .env("PATH", path)
        .output()
        .expect("autospec autonomous start runs");
    assert!(
        start.status.success(),
        "{}",
        String::from_utf8_lossy(&start.stderr)
    );
    let supervisor_log =
        std::fs::read_to_string(scope.join("supervisor.logpath")).expect("supervisor logpath");
    let supervisor_log = std::path::PathBuf::from(supervisor_log.trim());
    let mut restarted = 0;
    for _ in 0..80 {
        restarted = std::fs::read_to_string(&supervisor_log)
            .unwrap_or_default()
            .matches("action=restarted-conductor")
            .count();
        if restarted >= 2 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    assert!(
        restarted >= 2,
        "supervisor log={}",
        std::fs::read_to_string(&supervisor_log).unwrap_or_default()
    );
    let tracked_pid = cleanup_pid(&read_pid(&scope, "conductor")).expect("tracked conductor pid");
    assert!(
        process_is_alive(&tracked_pid) || process_is_zombie(&tracked_pid),
        "latest replacement must still be tracked until the next supervisor tick"
    );
    cleanup_pids(&scope);
}

#[test]
fn autonomous_supervise_does_not_relaunch_after_a_stop_request() {
    let temp = temp_dir("autospec-autonomous-supervise-stop");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let state_dir = temp.join("state");
    let repo_dir = temp.join("repo");
    make_git_repo(&repo_dir, None);
    let scope = operator_dir.join("berlinguyinca_autospec");

    let start = autospec()
        .args([
            "autonomous",
            "start",
            "--repo",
            "berlinguyinca/autospec",
            "--repo-dir",
            repo_dir.to_str().unwrap(),
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_STATE_DIR", &state_dir)
        .env("AUTOSPEC_AUTONOMOUS_SPEND_DIR", temp.join("spend"))
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .env("AUTOSPEC_AUTONOMOUS_COMPANIONS", "0")
        .env("PATH", hermetic_autonomous_path(&temp))
        .output()
        .expect("autospec autonomous start runs");
    assert!(start.status.success());
    let old_pid = cleanup_pid(&read_pid(&scope, "conductor")).expect("conductor pid");
    terminate_fixture_process(&old_pid);
    std::fs::write(scope.join("stop.flag"), "graceful\noperator@test\n").expect("stop flag");

    let repair = autospec()
        .args([
            "autonomous",
            "supervise",
            "--repo",
            "berlinguyinca/autospec",
            "--repo-dir",
            repo_dir.to_str().unwrap(),
            "--once",
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_STATE_DIR", &state_dir)
        .env("AUTOSPEC_AUTONOMOUS_SPEND_DIR", temp.join("spend"))
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .env("PATH", hermetic_autonomous_path(&temp))
        .output()
        .expect("autospec autonomous supervise runs");
    let stdout = String::from_utf8_lossy(&repair.stdout);

    assert!(repair.status.success());
    assert!(
        stdout.contains("\"action\":\"stop-requested\""),
        "stdout={stdout} stderr={}",
        String::from_utf8_lossy(&repair.stderr)
    );
    assert!(!process_is_alive(&old_pid));
    assert_eq!(
        cleanup_pid(&read_pid(&scope, "conductor")).as_deref(),
        Some(old_pid.as_str())
    );
}

#[test]
fn autonomous_restart_rejects_a_fresh_existing_lease_without_killing_conductor_or_clearing_stop() {
    let temp = temp_dir("autospec-autonomous-restart");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let stop_flag = temp.join("stop.flag");
    let repo_dir = temp.join("repo");
    let state_dir = temp.join("state");
    make_git_repo(&repo_dir, None);

    start_sleeping_autonomous_with_state(
        &operator_dir,
        &log_dir,
        &repo_dir,
        "berlinguyinca/autospec",
        &state_dir,
    );
    let scope = operator_dir.join("berlinguyinca_autospec");
    let old_conductor = read_pid(&scope, "conductor");
    std::fs::write(&stop_flag, "graceful\nexisting\n").expect("stop flag");

    let output = autospec()
        .args([
            "autonomous",
            "restart",
            "--repo",
            "berlinguyinca/autospec",
            "--repo-dir",
            repo_dir.to_str().unwrap(),
            "--max-cycles",
            "7",
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_STATE_DIR", &state_dir)
        .env("AUTOSPEC_AUTONOMOUS_SPEND_DIR", temp.join("spend"))
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .env("PATH", hermetic_autonomous_path(&temp))
        .env("AUTOSPEC_STOP_FLAG_FILE", &stop_flag)
        .output()
        .expect("autospec autonomous restart runs");

    assert_lease_held(&output);
    assert_eq!(read_pid(&scope, "conductor"), old_conductor);
    assert!(process_starts(&old_conductor));
    assert_eq!(
        std::fs::read_to_string(&stop_flag).expect("preserved stop flag"),
        "graceful\nexisting\n"
    );

    cleanup_pids(&scope);
}

#[test]
fn autonomous_restart_replaces_unleased_legacy_conductor_metadata() {
    let temp = temp_dir("autospec-autonomous-restart-legacy-metadata");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let repo_dir = temp.join("repo");
    let state_dir = temp.join("state");
    make_git_repo(&repo_dir, None);
    let scope = operator_dir.join("berlinguyinca_autospec");
    let old_conductor = seed_unleased_legacy_conductor(&scope);
    assert!(!fresh_lease_state_path(&state_dir).exists());

    let output = autospec()
        .args([
            "autonomous",
            "restart",
            "--repo",
            "berlinguyinca/autospec",
            "--repo-dir",
            repo_dir.to_str().unwrap(),
            "--max-cycles",
            "7",
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_STATE_DIR", &state_dir)
        .env("AUTOSPEC_AUTONOMOUS_SPEND_DIR", temp.join("spend"))
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .env("PATH", hermetic_autonomous_path(&temp))
        .output()
        .expect("autospec autonomous restart runs");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let new_conductor = read_pid(&scope, "conductor");

    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        stdout,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("\"subcommand\":\"restart\""));
    assert_ne!(old_conductor, new_conductor);
    assert!(process_stops(&old_conductor));
    assert!(
        process_starts(&new_conductor),
        "stdout={} stderr={} conductor_log={}",
        stdout,
        String::from_utf8_lossy(&output.stderr),
        std::fs::read_to_string(
            std::fs::read_to_string(scope.join("conductor.logpath"))
                .unwrap_or_default()
                .trim()
        )
        .unwrap_or_default()
    );
    let launch = std::fs::read_to_string(scope.join("launch.json")).expect("launch json");
    assert!(launch.contains("\"max_cycles\":\"7\""));

    cleanup_pids(&scope);
}

#[test]
fn autonomous_restart_clears_existing_stop_flag_before_launch() {
    let temp = temp_dir("autospec-autonomous-restart-stop-flag");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let stop_flag = temp.join("stop.flag");
    let repo_dir = temp.join("repo");
    make_git_repo(&repo_dir, None);
    std::fs::write(&stop_flag, "graceful\nold\n").expect("stop flag");

    let output = autospec()
        .args([
            "autonomous",
            "restart",
            "--repo",
            "berlinguyinca/autospec",
            "--repo-dir",
            repo_dir.to_str().unwrap(),
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_STATE_DIR", temp.join("state"))
        .env("AUTOSPEC_AUTONOMOUS_SPEND_DIR", temp.join("spend"))
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .env("PATH", hermetic_autonomous_path(&temp))
        .env("AUTOSPEC_STOP_FLAG_FILE", &stop_flag)
        .output()
        .expect("autospec autonomous restart runs");

    assert!(output.status.success());
    assert!(!stop_flag.exists());
    cleanup_pids(&operator_dir.join("berlinguyinca_autospec"));
}

#[test]
fn cli_commands_json_modes_emit_json() {
    for command in [
        "doctor",
        "status",
        "plan",
        "report",
        "showcase",
        "growth-report",
    ] {
        let output = autospec()
            .args([command, "--json"])
            .current_dir(workspace_root())
            .output()
            .unwrap_or_else(|error| panic!("{command} runs: {error}"));
        let stdout = String::from_utf8_lossy(&output.stdout);

        assert!(output.status.success(), "{command} failed");
        assert!(
            stdout.trim_start().starts_with('{'),
            "{command} did not emit JSON"
        );
        assert!(stdout.contains(&format!("\"command\":\"{command}\"")));
    }
}

fn autonomous_test_state_dir(operator_dir: &std::path::Path) -> std::path::PathBuf {
    operator_dir
        .parent()
        .expect("operator directory belongs to isolated test fixture")
        .join("state")
}

fn autonomous_test_spend_dir(operator_dir: &std::path::Path) -> std::path::PathBuf {
    operator_dir
        .parent()
        .expect("operator directory belongs to isolated test fixture")
        .join("spend")
}

fn start_sleeping_autonomous(
    operator_dir: &std::path::Path,
    log_dir: &std::path::Path,
    repo_dir: &std::path::Path,
    repo: &str,
) {
    let state_dir = autonomous_test_state_dir(operator_dir);
    start_sleeping_autonomous_with_state(operator_dir, log_dir, repo_dir, repo, &state_dir);
}

fn start_sleeping_autonomous_with_state(
    operator_dir: &std::path::Path,
    log_dir: &std::path::Path,
    repo_dir: &std::path::Path,
    repo: &str,
    state_dir: &std::path::Path,
) {
    if !repo_dir.join(".git").exists() {
        make_git_repo(repo_dir, None);
    }
    let output = autospec()
        .args([
            "autonomous",
            "start",
            "--repo",
            repo,
            "--repo-dir",
            repo_dir.to_str().unwrap(),
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", operator_dir)
        .env("AUTOSPEC_STATE_DIR", state_dir)
        .env(
            "AUTOSPEC_AUTONOMOUS_SPEND_DIR",
            state_dir
                .parent()
                .expect("state directory belongs to isolated test fixture")
                .join("spend"),
        )
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", log_dir)
        .env("PATH", hermetic_autonomous_path(operator_dir))
        .output()
        .expect("autospec autonomous start runs");
    assert!(output.status.success());
}

fn make_git_repo(repo_dir: &std::path::Path, remote: Option<&str>) {
    std::fs::create_dir_all(repo_dir).expect("repo dir");
    if !repo_dir.join(".git").exists() {
        assert!(Command::new("git")
            .args(["init"])
            .current_dir(repo_dir)
            .output()
            .expect("git init")
            .status
            .success());
    }
    if let Some(remote) = remote {
        let _ = Command::new("git")
            .args(["remote", "remove", "origin"])
            .current_dir(repo_dir)
            .output();
        assert!(Command::new("git")
            .args(["remote", "add", "origin", remote])
            .current_dir(repo_dir)
            .output()
            .expect("git remote add")
            .status
            .success());
    }
}

fn autonomous_status(
    operator_dir: &std::path::Path,
    log_dir: &std::path::Path,
    repo: &str,
) -> String {
    let output = autospec()
        .args(["autonomous", "status", "--repo", repo, "--json"])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", operator_dir)
        .env(
            "AUTOSPEC_STATE_DIR",
            autonomous_test_state_dir(operator_dir),
        )
        .env(
            "AUTOSPEC_AUTONOMOUS_SPEND_DIR",
            autonomous_test_spend_dir(operator_dir),
        )
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", log_dir)
        .output()
        .expect("autospec autonomous status runs");
    assert!(output.status.success());
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn read_pid(scope: &std::path::Path, name: &str) -> String {
    let metadata = std::fs::read_to_string(scope.join(format!("{name}.pid")))
        .unwrap()
        .trim()
        .to_string();
    serde_json::from_str::<serde_json::Value>(&metadata)
        .ok()
        .and_then(|value| value.get("pid").and_then(serde_json::Value::as_u64))
        .map_or(metadata, |pid| pid.to_string())
}

fn process_is_alive(pid: &str) -> bool {
    Command::new("kill")
        .args(["-0", pid])
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn process_starts(pid: &str) -> bool {
    for _ in 0..20 {
        if process_is_alive(pid) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    false
}

fn process_is_zombie(pid: &str) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|stat| stat.rsplit_once(") ").map(|(_, fields)| fields.to_string()))
        .and_then(|fields| fields.split_whitespace().next().map(str::to_string))
        .as_deref()
        == Some("Z")
}

fn terminate_fixture_process(pid: &str) {
    let _ = Command::new("kill")
        .args(["-KILL", "--", &format!("-{pid}")])
        .stderr(Stdio::null())
        .status();
    let _ = Command::new("kill")
        .args(["-KILL", "--", pid])
        .stderr(Stdio::null())
        .status();
    for _ in 0..50 {
        if !process_is_alive(pid) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("fixture process {pid} did not stop");
}

fn assert_lease_held(output: &Output) {
    assert_eq!(output.status.code(), Some(20));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "{\"decision\":\"park\",\"reason\":\"conductor_lease_held\"}\n"
    );
}

fn fresh_lease_state_path(state_dir: &std::path::Path) -> std::path::PathBuf {
    state_dir
        .join("autonomous")
        .join("berlinguyinca__autospec")
        .join("state.json")
}

fn seed_unleased_legacy_conductor(scope: &std::path::Path) -> String {
    std::fs::create_dir_all(scope).expect("legacy conductor scope");
    let legacy_launcher = scope.join("autospec-autonomous.sh");
    write_executable(&legacy_launcher, "#!/bin/sh\nsleep 20\n");
    let mut command = Command::new(&legacy_launcher);
    command
        .arg("run-foreground")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
    let mut child = command.spawn().expect("legacy conductor process");
    let pid = child.id().to_string();
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    std::fs::write(scope.join("conductor.pid"), format!("{pid}\n"))
        .expect("legacy conductor pid metadata");
    std::fs::write(scope.join("conductor.logpath"), "legacy-conductor.log\n")
        .expect("legacy conductor log metadata");
    pid
}

fn issue_body(goal: &str, acceptance_criteria: &str, smoke: &str) -> String {
    format!(
        "## Goal\n{goal}\n\n## Files to read first\n- crates/autospec-core/src/lib.rs\n\n## Implementation outline\n1. Implement the policy.\n\n## Tests required\n- `cargo test -p autospec-core --test issue_lint`\n\n## Dependencies\nnone\n\n## Files touched\n- crates/autospec-core/src/lint/mod.rs\n\n## Acceptance criteria\n{acceptance_criteria}\n\n## Verification\n\n### Primary smoke test (inner loop)\n\n```bash\n{smoke}\n```\n"
    )
}

fn write_issue_body(prefix: &str, body: &str) -> std::path::PathBuf {
    let path = temp_dir(prefix).join("issue.md");
    std::fs::write(&path, body).expect("issue fixture");
    path
}

fn new_diff(path: &str, lines: &[&str]) -> String {
    let additions = lines
        .iter()
        .map(|line| format!("+{line}\n"))
        .collect::<String>();
    format!(
        "diff --git a/{path} b/{path}\nnew file mode 100644\n--- /dev/null\n+++ b/{path}\n@@ -0,0 +1,{} @@\n{additions}",
        lines.len()
    )
}

fn write_implementation_diff(prefix: &str, diff: &str) -> std::path::PathBuf {
    let path = temp_dir(prefix).join("change.diff");
    std::fs::write(&path, diff).expect("implementation diff fixture");
    path
}

fn large_finding_diff() -> String {
    let security = vec!["eval(input)"; 15];
    let todos = vec!["# TODO deferred"; 15];
    let nested = vec!["                    if nested {"; 15];
    let vacuous = vec!["grep -qv 'bad' result.txt || true"; 15];
    let tautologies = vec!["expect(true).toBe(true);"; 15];
    let mocks = vec!["let database = mock(database);"; 15];
    format!(
        "{}{}{}{}{}{}",
        new_diff("scripts/security.sh", &security),
        new_diff("scripts/todos.sh", &todos),
        new_diff("scripts/deeply_nested.sh", &nested),
        new_diff("tests/unit/test_vacuous.bats", &vacuous),
        new_diff("tests/unit/test_tautology.js", &tautologies),
        new_diff("tests/unit/db_test.rs", &mocks),
    )
}

fn fake_bin(
    fixture: &std::path::Path,
    git_program: Option<&str>,
    gh_program: Option<&str>,
) -> std::path::PathBuf {
    let bin = fixture.join("bin");
    std::fs::create_dir_all(&bin).expect("fake bin directory");
    if let Some(program) = git_program {
        write_executable(&bin.join("git"), program);
    }
    if let Some(program) = gh_program {
        write_executable(&bin.join("gh"), program);
    }
    bin
}

#[test]
fn generated_executable_replacement_preserves_active_inode() {
    let fixture = temp_dir("autospec-generated-executable-replacement");
    let sleep = ["/usr/bin/sleep", "/bin/sleep"]
        .into_iter()
        .find(|path| std::path::Path::new(path).is_file())
        .expect("sleep executable");
    let sleep_program = std::fs::read(sleep).expect("sleep program");

    for iteration in 0..20 {
        let executable = fixture.join(format!("active-command-{iteration}"));
        publish_executable(&executable, &sleep_program);
        let child = Command::new(&executable)
            .arg("60")
            .spawn()
            .expect("generated executable starts");
        let mut child = FixtureChild::new(child);
        assert!(
            child.is_running(),
            "iteration {iteration} exited before replacement"
        );

        write_executable(&executable, "#!/bin/sh\nexit 0\n");
        child.finish();
        assert!(Command::new(&executable)
            .status()
            .expect("replacement executable starts")
            .success());
    }
}

struct FixtureChild {
    child: Option<Child>,
}

impl FixtureChild {
    fn new(child: Child) -> Self {
        Self { child: Some(child) }
    }

    fn is_running(&mut self) -> bool {
        self.child
            .as_mut()
            .expect("fixture child")
            .try_wait()
            .expect("fixture child status")
            .is_none()
    }

    fn finish(mut self) {
        let child = self.child.as_mut().expect("fixture child");
        let kill = child.kill();
        let wait = child.wait();
        kill.expect("fixture child stops");
        wait.expect("fixture child exits");
        self.child.take();
    }
}

impl Drop for FixtureChild {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn hermetic_autonomous_path(fixture: &std::path::Path) -> String {
    let bin = fixture.join("bin");
    if !bin.join("gh").is_file() {
        fake_bin(
            fixture,
            None,
            Some("#!/bin/sh\nwhile kill -0 \"$PPID\" 2>/dev/null; do sleep 0.1; done\nexit 1\n"),
        );
    }
    path_with(&bin)
}

fn write_executable(path: &std::path::Path, contents: &str) {
    publish_executable(path, contents.as_bytes());
}

fn publish_executable(path: &std::path::Path, contents: &[u8]) {
    let sequence = EXECUTABLE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary = path.with_extension(format!("tmp-{}-{sequence}", std::process::id()));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .expect("fake command temporary");
    file.write_all(contents).expect("fake command");
    drop(file);
    let mut permissions = std::fs::metadata(&temporary)
        .expect("fake command metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&temporary, permissions).expect("fake command permissions");
    std::fs::rename(temporary, path).expect("fake command publish");
    // Atomic rename prevents inode clashes; 30-run stress showed this ZFS host
    // still needs 10 ms after close before exec stops returning ETXTBSY.
    std::thread::sleep(std::time::Duration::from_millis(10));
}

fn path_with(bin: &std::path::Path) -> String {
    format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").expect("test PATH is set")
    )
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\\"'\\\"'"))
}

fn autospec_with_stdin<const N: usize>(args: [&str; N], input: &str) -> Output {
    let mut child = autospec()
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("autospec lint issue starts");
    child
        .stdin
        .as_mut()
        .expect("autospec stdin is piped")
        .write_all(input.as_bytes())
        .expect("issue body writes to stdin");
    child.wait_with_output().expect("autospec lint issue exits")
}

fn temp_dir(prefix: &str) -> std::path::PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("{prefix}-{}-{suffix}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn workspace_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
}

#[test]
fn cleanup_pids_terminates_scoped_json_and_legacy_pid_records() {
    let scope_path = {
        let scope = TempDirFixture::new("autospec-cleanup-pids");
        let (mut scoped, descendant_pid) = grouped_fixture_process(scope.path());
        let legacy = Command::new("sleep")
            .arg("20")
            .spawn()
            .expect("legacy fixture process");
        let scoped_pid = scoped.pid().to_string();
        let legacy_pid = legacy.id().to_string();
        let mut legacy = FixtureChild::new(legacy);
        std::fs::write(
            scope.path().join("conductor.pid"),
            format!("{{\"pid\":{scoped_pid},\"repo\":\"test/repo\",\"scope\":\"test_repo\"}}\n"),
        )
        .expect("scoped pid metadata");
        std::fs::write(scope.path().join("monitor.pid"), format!("{legacy_pid}\n"))
            .expect("legacy pid metadata");

        cleanup_pids(scope.path());
        assert!(process_group_stops(&mut scoped));
        assert!(fixture_child_stops(&mut legacy));
        assert!(process_stops(&descendant_pid));
        scope.path().to_path_buf()
    };
    assert!(!scope_path.exists(), "cleanup fixture directory leaked");
}

fn grouped_fixture_process(scope: &std::path::Path) -> (ProcessGroupFixture, String) {
    let descendant_file = scope.join("descendant.pid");
    let mut command = Command::new("sh");
    command
        .args([
            "-c",
            "sleep 20 & printf '%s\\n' \"$!\" > \"$DESCENDANT_PID\"; wait",
        ])
        .env("DESCENDANT_PID", &descendant_file);
    command.process_group(0);
    let fixture = ProcessGroupFixture::new(command.spawn().expect("scoped fixture process"));
    for _ in 0..50 {
        if let Ok(pid) = std::fs::read_to_string(&descendant_file) {
            return (fixture, pid.trim().to_string());
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("scoped fixture descendant PID was not recorded");
}

fn process_stops(pid: &str) -> bool {
    for _ in 0..20 {
        if !process_is_alive(pid) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    false
}

fn process_group_stops(child: &mut ProcessGroupFixture) -> bool {
    for _ in 0..20 {
        if !child.is_running() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    false
}

fn fixture_child_stops(child: &mut FixtureChild) -> bool {
    for _ in 0..20 {
        if !child.is_running() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    false
}

struct TempDirFixture {
    path: std::path::PathBuf,
}

impl TempDirFixture {
    fn new(prefix: &str) -> Self {
        Self {
            path: temp_dir(prefix),
        }
    }

    fn path(&self) -> &std::path::Path {
        &self.path
    }
}

impl Drop for TempDirFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

struct ProcessGroupFixture {
    child: Option<Child>,
    pid: u32,
}

impl ProcessGroupFixture {
    fn new(child: Child) -> Self {
        let pid = child.id();
        Self {
            child: Some(child),
            pid,
        }
    }

    fn pid(&self) -> u32 {
        self.pid
    }

    fn is_running(&mut self) -> bool {
        self.child
            .as_mut()
            .expect("process-group fixture child")
            .try_wait()
            .expect("process-group fixture child status")
            .is_none()
    }
}

impl Drop for ProcessGroupFixture {
    fn drop(&mut self) {
        let _ = Command::new("kill")
            .args(["-KILL", "--", &format!("-{}", self.pid)])
            .stderr(Stdio::null())
            .status();
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn cleanup_pids(scope: &std::path::Path) {
    for name in ["conductor", "monitor", "supervisor"] {
        let pid_file = scope.join(format!("{name}.pid"));
        if let Ok(metadata) = std::fs::read_to_string(pid_file) {
            let Some(pid) = cleanup_pid(&metadata) else {
                continue;
            };
            let _ = Command::new("kill")
                .args(["-KILL", "--", &format!("-{pid}")])
                .stderr(Stdio::null())
                .status();
            let _ = Command::new("kill")
                .args(["-KILL", "--", &pid])
                .stderr(Stdio::null())
                .status();
        }
    }
}

fn cleanup_pid(metadata: &str) -> Option<String> {
    let metadata = metadata.trim();
    if metadata.parse::<u32>().is_ok_and(|candidate| candidate > 0) {
        return Some(metadata.to_string());
    }
    serde_json::from_str::<serde_json::Value>(metadata)
        .ok()?
        .get("pid")?
        .as_u64()
        .filter(|pid| *pid > 0)
        .map(|pid| pid.to_string())
}

fn json_path(path: &std::path::Path) -> String {
    path.display().to_string().replace('\\', "\\\\")
}

fn write_conductor_log(
    operator_dir: &std::path::Path,
    scope: &str,
    contents: &str,
) -> std::path::PathBuf {
    let scope_dir = operator_dir.join(scope);
    let log = operator_dir.join(format!("{scope}.log"));
    std::fs::create_dir_all(&scope_dir).expect("scope dir");
    std::fs::write(&log, contents).expect("log");
    std::fs::write(
        scope_dir.join("conductor.logpath"),
        format!("{}\n", log.display()),
    )
    .expect("logpath");
    log
}

#[test]
fn doctor_readiness_json_reports_workflow_safety() {
    let output = autospec()
        .args(["doctor", "--readiness", "--json"])
        .output()
        .expect("autospec doctor runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("\"command\":\"doctor\""));
    assert!(stdout.contains("\"mode\":\"readiness\""));
    assert!(stdout.contains("\"workflow_recommendations\""));
    assert!(stdout.contains("\"define\""));
    assert!(stdout.contains("\"run\""));
    assert!(stdout.contains("\"autonomous\""));
}

#[test]
fn cli_commands_require_explicit_input_or_report_the_remaining_stub() {
    for command in ["run", "resume"] {
        let output = autospec().arg(command).output().expect("autospec runs");

        assert!(
            !output.status.success(),
            "{command} should not silently succeed"
        );
    }

    let benchmark = autospec().arg("benchmark").output().expect("autospec runs");
    assert!(!benchmark.status.success());
    assert!(String::from_utf8_lossy(&benchmark.stderr).contains("not yet implemented"));
}

#[test]
fn autonomous_implementer_wait_failed_requires_actual_session_id() {
    let output = autospec()
        .args([
            "autonomous",
            "implementer-wait-failed",
            "--repo",
            "testorg/testrepo",
            "--issue",
            "42",
            "--worker-id",
            "worker-a",
            "--branch",
            "feat/test",
            "--claim-id",
            "claim-generation-a",
            "--diagnostic",
            "write_stdin failed: stdin is closed",
        ])
        .output()
        .expect("autospec recovery command runs");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--session-id is required"));
}

#[test]
fn autonomous_implementer_wait_failed_requires_expected_claim_id() {
    let output = autospec()
        .args([
            "autonomous",
            "implementer-wait-failed",
            "--repo",
            "testorg/testrepo",
            "--issue",
            "42",
            "--worker-id",
            "worker-a",
            "--branch",
            "feat/test",
            "--session-id",
            "session-real-7",
            "--diagnostic",
            "write_stdin failed: stdin is closed",
        ])
        .output()
        .expect("autospec recovery command runs");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--claim-id is required"));
}

#[test]
fn autonomous_implementer_wait_failure_requeues_exact_owner_with_bounded_comment() {
    let root = temp_dir("autospec-wait-failed");
    let bin = root.join("bin");
    let state = root.join("comments.json");
    let log = root.join("gh.log");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(
        &state,
        wait_failure_comments("worker-a", "feat/test", "claimed"),
    )
    .unwrap();
    seed_wait_claim_ref(
        &root,
        "worker-a",
        "feat/test",
        "claimed",
        "claim-generation-a",
    );
    write_executable(&bin.join("gh"), WAIT_FAILURE_GH);
    let github_pat = ["github", "_pat_11AA22BB33CC44DD55EE66FF77GG88HH99"].concat();
    let github_oauth = ["gh", "o_123456789012345678901234567890123456"].concat();
    let secrets = [
        "super-secret",
        github_pat.as_str(),
        github_oauth.as_str(),
        "bearer-value",
        "key-value",
        "token-value",
        "hunter2",
        "private-material",
    ];
    let diagnostic = format!(
        "write_stdin failed: stdin is closed token={} {} {} Authorization: Bearer {} api_key={} token:{} user:{} -----BEGIN PRIVATE KEY----- {} -----END PRIVATE KEY----- {}",
        secrets[0],
        secrets[1],
        secrets[2],
        secrets[3],
        secrets[4],
        secrets[5],
        secrets[6],
        secrets[7],
        "x".repeat(800)
    );

    let output = autospec()
        .args([
            "autonomous",
            "implementer-wait-failed",
            "--repo",
            "testorg/testrepo",
            "--issue",
            "42",
            "--worker-id",
            "worker-a",
            "--branch",
            "feat/test",
            "--claim-id",
            "claim-generation-a",
            "--session-id",
            "session-real-7",
            "--diagnostic",
            &diagnostic,
        ])
        .env(
            "PATH",
            format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
        )
        .env("AUTOSPEC_WAIT_STATE", &state)
        .env("AUTOSPEC_WAIT_LOG", &log)
        .env("AUTOSPEC_CLAIM_GIT_REMOTE", root.join("claim-remote.git"))
        .env("AUTOSPEC_CLAIM_GIT_STATE_DIR", root.join("claim-state"))
        .env("AUTOSPEC_CLAIM_RETRY_SLEEP_MS", "0")
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("\"event\":\"implementer_wait_failed\"")
    );
    let remote = std::fs::read_to_string(&state).unwrap();
    assert!(remote.contains("\\\"outcome\\\":\\\"failed\\\""));
    assert!(remote.contains("\\\"state\\\":\\\"claimed\\\""));
    assert!(remote.contains("autospec-implementer-wait-failed"));
    assert!(remote.contains("session-real-7"));
    assert!(remote.contains("[REDACTED]"));
    for secret in secrets {
        assert!(!remote.contains(secret), "leaked {secret}: {remote}");
    }
    assert!(!remote.contains(&"x".repeat(513)));
    let calls = std::fs::read_to_string(log).unwrap_or_default();
    assert!(calls.contains("--remove-label\nin-progress-by-bot\n--add-label\nauto-implement"));
}

#[test]
fn autonomous_implementer_wait_failed_requeue_allows_a_new_worker_to_acquire() {
    let root = temp_dir("autospec-wait-reacquire");
    let bin = root.join("bin");
    let state = root.join("comments.json");
    let labels = root.join("labels.mode");
    let log = root.join("gh.log");
    let heartbeats = root.join("heartbeats");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(
        &state,
        wait_failure_comments("worker-a", "feat/test", "claimed"),
    )
    .unwrap();
    std::fs::write(&labels, "active\n").unwrap();
    seed_wait_claim_ref(
        &root,
        "worker-a",
        "feat/test",
        "claimed",
        "claim-generation-a",
    );
    write_executable(&bin.join("gh"), WAIT_FAILURE_GH);

    let recovered = wait_failure_command(&bin, &state, &log)
        .env("AUTOSPEC_WAIT_LABELS", &labels)
        .output()
        .unwrap();
    assert!(recovered.status.success());
    assert_eq!(std::fs::read_to_string(&labels).unwrap().trim(), "ready");

    let issue_body = "## Safety review\n\n<!-- autospec-safety:begin -->\n- **decision:** `SAFETY_PASS`\n<!-- autospec-safety:end -->\n\n## Goal\nRecover the stranded issue.";
    let acquired = autospec()
        .args([
            "claim",
            "acquire",
            "--issue",
            "42",
            "--repo",
            "testorg/testrepo",
            "--worker-id",
            "worker-b",
            "--branch",
            "feat/next",
        ])
        .env(
            "PATH",
            format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
        )
        .env("AUTOSPEC_WAIT_STATE", &state)
        .env("AUTOSPEC_WAIT_LABELS", &labels)
        .env("AUTOSPEC_WAIT_LOG", &log)
        .env("AUTOSPEC_WAIT_ISSUE_BODY", issue_body)
        .env("AUTOSPEC_CLAIM_GIT_REMOTE", root.join("claim-remote.git"))
        .env("AUTOSPEC_CLAIM_GIT_STATE_DIR", root.join("claim-state"))
        .env("AUTOSPEC_HEARTBEAT_DIR", &heartbeats)
        .env("AUTOSPEC_CLAIM_CONFIRM_READS", "1")
        .env("AUTOSPEC_CLAIM_SETTLE_MILLIS", "0")
        .env("AUTOSPEC_CLAIM_RETRY_SLEEP_MS", "0")
        .output()
        .unwrap();

    assert!(
        acquired.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&acquired.stdout),
        String::from_utf8_lossy(&acquired.stderr)
    );
    assert!(String::from_utf8_lossy(&acquired.stdout).contains("\"claimed\":true"));
    let remote = std::fs::read_to_string(&state).unwrap();
    assert!(remote.contains("\\\"worker_id\\\":\\\"worker-b\\\""));
    assert!(remote.contains("\\\"branch\\\":\\\"feat/next\\\""));
    assert_eq!(std::fs::read_to_string(&labels).unwrap().trim(), "active");
}

#[test]
fn wait_failure_evidence_cannot_release_same_second_successor_generation() {
    let root = temp_dir("autospec-wait-same-second-generation");
    let bin = root.join("bin");
    let state = root.join("comments.json");
    let labels = root.join("labels.mode");
    let log = root.join("gh.log");
    let heartbeats = root.join("heartbeats");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(
        &state,
        wait_failure_comments("worker-a", "feat/test", "claimed"),
    )
    .unwrap();
    std::fs::write(&labels, "active\n").unwrap();
    seed_wait_claim_ref(
        &root,
        "worker-a",
        "feat/test",
        "claimed",
        "claim-generation-a",
    );
    write_executable(&bin.join("gh"), WAIT_FAILURE_GH);

    let recovered = wait_failure_command(&bin, &state, &log)
        .env("AUTOSPEC_WAIT_LABELS", &labels)
        .output()
        .unwrap();
    assert!(recovered.status.success());

    let reacquired = wait_claim_acquire(
        &bin,
        &state,
        &labels,
        &log,
        &heartbeats,
        "worker-a",
        "feat/test",
    )
    .output()
    .unwrap();
    assert!(reacquired.status.success());
    let mut remote = std::fs::read_to_string(&state).unwrap();
    force_first_wait_claimed_at(&mut remote, "2099-07-19T00:00:00Z");
    std::fs::write(&state, remote).unwrap();

    std::fs::write(&labels, "ready\n").unwrap();
    let successor_oid = wait_claim_ref_oid(&root);
    let calls_before = std::fs::read_to_string(&log).unwrap().len();
    let third = wait_claim_acquire(
        &bin,
        &state,
        &labels,
        &log,
        &heartbeats,
        "worker-c",
        "feat/third",
    )
    .output()
    .unwrap();

    assert_eq!(third.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&third.stdout).contains("\"reason\":\"claim_lost\""));
    let remote = std::fs::read_to_string(&state).unwrap();
    assert!(remote.contains("\\\"worker_id\\\":\\\"worker-a\\\""));
    assert!(!remote.contains("\\\"worker_id\\\":\\\"worker-c\\\""));
    assert_eq!(wait_claim_ref_oid(&root), successor_oid);
    assert_eq!(std::fs::read_to_string(&labels).unwrap().trim(), "ready");
    let calls = std::fs::read_to_string(&log).unwrap();
    let rejected_calls = &calls[calls_before..];
    assert!(!rejected_calls.contains("issue\nedit"));
    assert!(!rejected_calls.contains("issue\ncomment"));
}

#[test]
fn autonomous_implementer_wait_failed_does_not_touch_successor_claim() {
    let root = temp_dir("autospec-wait-successor");
    let bin = root.join("bin");
    let state = root.join("comments.json");
    let log = root.join("gh.log");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(
        &state,
        wait_failure_comments("worker-b", "feat/next", "claimed"),
    )
    .unwrap();
    seed_wait_claim_ref(
        &root,
        "worker-b",
        "feat/next",
        "claimed",
        "claim-generation-a",
    );
    write_executable(&bin.join("gh"), WAIT_FAILURE_GH);
    let output = wait_failure_command(&bin, &state, &log).output().unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"outcome\":\"ownership_lost\""));
    let calls = std::fs::read_to_string(log).unwrap_or_default();
    assert!(!calls.contains("issue\nedit"));
    assert!(!calls.contains("issue\ncomment"));
    assert!(!calls.contains("-X\nPATCH"));
}

#[test]
fn autonomous_implementer_wait_failed_rejects_delayed_same_owner_command_for_new_generation() {
    let root = temp_dir("autospec-wait-delayed-generation");
    let bin = root.join("bin");
    let state = root.join("comments.json");
    let log = root.join("gh.log");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(
        &state,
        wait_failure_comments_with_claim_id(
            "worker-a",
            "feat/test",
            "claimed",
            "claim-generation-new",
        ),
    )
    .unwrap();
    seed_wait_claim_ref(
        &root,
        "worker-a",
        "feat/test",
        "claimed",
        "claim-generation-new",
    );
    write_executable(&bin.join("gh"), WAIT_FAILURE_GH);

    let output = wait_failure_command_with_claim_id(&bin, &state, &log, "claim-generation-old")
        .output()
        .unwrap();

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"outcome\":\"ownership_lost\""));
    let calls = std::fs::read_to_string(log).unwrap_or_default();
    assert!(!calls.contains("issue\nedit"));
    assert!(!calls.contains("issue\ncomment"));
    assert!(!calls.contains("-X\nPATCH"));
}

#[test]
fn autonomous_implementer_wait_failed_does_not_requeue_successor_won_during_recovery() {
    let root = temp_dir("autospec-wait-race");
    let bin = root.join("bin");
    let state = root.join("comments.json");
    let log = root.join("gh.log");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(
        &state,
        wait_failure_comments("worker-a", "feat/test", "claimed"),
    )
    .unwrap();
    seed_wait_claim_ref(
        &root,
        "worker-a",
        "feat/test",
        "claimed",
        "claim-generation-a",
    );
    seed_wait_claim_ref(
        &root,
        "worker-b",
        "feat/next",
        "claimed",
        "claim-generation-b",
    );
    write_executable(&bin.join("gh"), WAIT_FAILURE_GH);

    let output = wait_failure_command(&bin, &state, &log).output().unwrap();

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"outcome\":\"ownership_lost\""));
    let remote = wait_claim_ref_message(&root);
    assert!(remote.contains("worker-b"));
    assert!(remote.contains("feat/next"));
    let calls = std::fs::read_to_string(log).unwrap_or_default();
    assert!(!calls.contains("issue\nedit"));
    assert!(!calls.contains("issues/comments/100"));
}

#[test]
fn autonomous_implementer_wait_failed_rejects_stale_and_terminal_claims() {
    for (name, comments, claim_state, updated_at) in [
        (
            "stale",
            wait_failure_comments("worker-a", "feat/test", "claimed")
                .replace("2099-07-19T00:00:00Z", "2000-01-01T00:00:00Z"),
            "claimed",
            "2000-01-01T00:00:00Z",
        ),
        (
            "failed-state",
            wait_failure_comments("worker-a", "feat/test", "failed"),
            "failed",
            "2099-07-19T00:00:00Z",
        ),
        (
            "terminal",
            wait_failure_terminal_comments("worker-a", "feat/test"),
            "merged",
            "2099-07-19T00:00:00Z",
        ),
    ] {
        let root = temp_dir(&format!("autospec-wait-{name}"));
        let bin = root.join("bin");
        let state = root.join("comments.json");
        let log = root.join("gh.log");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(&state, comments).unwrap();
        seed_wait_claim_ref_at(
            &root,
            "worker-a",
            "feat/test",
            claim_state,
            "claim-generation-a",
            updated_at,
        );
        write_executable(&bin.join("gh"), WAIT_FAILURE_GH);

        let output = wait_failure_command(&bin, &state, &log).output().unwrap();

        assert!(output.status.success(), "{name}");
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("\"outcome\":\"ownership_lost\""),
            "{name}: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        let calls = std::fs::read_to_string(log).unwrap_or_default();
        assert!(!calls.contains("issue\nedit"), "{name}");
        assert!(!calls.contains("issue\ncomment"), "{name}");
        assert!(!calls.contains("-X\nPATCH"), "{name}");
    }
}

#[test]
fn autonomous_implementer_wait_failed_is_idempotent_and_surfaces_comment_failure() {
    let root = temp_dir("autospec-wait-idempotent");
    let bin = root.join("bin");
    let state = root.join("comments.json");
    let log = root.join("gh.log");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(
        &state,
        wait_failure_comments("worker-a", "feat/test", "claimed"),
    )
    .unwrap();
    seed_wait_claim_ref(
        &root,
        "worker-a",
        "feat/test",
        "claimed",
        "claim-generation-a",
    );
    write_executable(&bin.join("gh"), WAIT_FAILURE_GH);
    assert!(wait_failure_command(&bin, &state, &log)
        .output()
        .unwrap()
        .status
        .success());
    assert!(wait_failure_command(&bin, &state, &log)
        .output()
        .unwrap()
        .status
        .success());
    assert_eq!(
        std::fs::read_to_string(&state)
            .unwrap()
            .matches("autospec-implementer-wait-failed")
            .count(),
        1
    );

    std::fs::write(
        &state,
        wait_failure_comments("worker-a", "feat/test", "claimed"),
    )
    .unwrap();
    seed_wait_claim_ref(
        &root,
        "worker-a",
        "feat/test",
        "claimed",
        "claim-generation-a",
    );
    let failed = wait_failure_command(&bin, &state, &log)
        .env("AUTOSPEC_WAIT_COMMENT_FAIL", "1")
        .env("AUTOSPEC_GH_API_RETRIES", "1")
        .output()
        .unwrap();
    assert!(!failed.status.success());
    assert!(
        String::from_utf8_lossy(&failed.stdout).contains("\"event\":\"implementer_wait_failed\"")
    );
    assert!(String::from_utf8_lossy(&failed.stdout).contains("\"outcome\":\"recovery_failed\""));
    assert!(String::from_utf8_lossy(&failed.stderr).contains("create run-state comment"));
    assert!(std::fs::read_to_string(&state)
        .unwrap()
        .contains("\\\"outcome\\\":\\\"failed\\\""));
}

#[test]
fn autonomous_implementer_wait_failed_retries_converge_after_each_projection_boundary() {
    for failpoint in [
        "after_claim_cas",
        "after_ref_projection",
        "after_evidence_projection",
        "after_label_requeue",
    ] {
        let root = temp_dir(&format!("autospec-wait-crash-{failpoint}"));
        let bin = root.join("bin");
        let state = root.join("comments.json");
        let labels = root.join("labels.mode");
        let log = root.join("gh.log");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(
            &state,
            wait_failure_comments("worker-a", "feat/test", "claimed"),
        )
        .unwrap();
        std::fs::write(&labels, "active\n").unwrap();
        seed_wait_claim_ref(
            &root,
            "worker-a",
            "feat/test",
            "claimed",
            "claim-generation-a",
        );
        write_executable(&bin.join("gh"), WAIT_FAILURE_GH);
        let original_oid = wait_claim_ref_oid(&root);

        let crashed = wait_failure_command(&bin, &state, &log)
            .env("AUTOSPEC_WAIT_LABELS", &labels)
            .env("AUTOSPEC_WAIT_RECOVERY_FAILPOINT", failpoint)
            .output()
            .unwrap();
        assert!(!crashed.status.success(), "{failpoint}");
        let transitioned_oid = wait_claim_ref_oid(&root);
        assert_ne!(transitioned_oid, original_oid, "{failpoint}");

        let retried = wait_failure_command(&bin, &state, &log)
            .env("AUTOSPEC_WAIT_LABELS", &labels)
            .output()
            .unwrap();
        assert!(
            retried.status.success(),
            "{failpoint}: stderr={}",
            String::from_utf8_lossy(&retried.stderr)
        );
        assert_eq!(wait_claim_ref_oid(&root), transitioned_oid, "{failpoint}");
        assert_eq!(
            std::fs::read_to_string(&labels).unwrap().trim(),
            "ready",
            "{failpoint}"
        );
        let remote = std::fs::read_to_string(&state).unwrap();
        assert_eq!(
            remote.matches("autospec-run-state-link").count(),
            1,
            "{failpoint}"
        );
        assert_eq!(
            remote.matches("autospec-executor-result:begin").count(),
            1,
            "{failpoint}"
        );
        assert_eq!(
            remote.matches("autospec-implementer-wait-failed").count(),
            1,
            "{failpoint}"
        );
    }
}

#[test]
fn autonomous_implementer_wait_failed_repairs_forged_matching_generation_projection() {
    let root = temp_dir("autospec-wait-forged-projection");
    let bin = root.join("bin");
    let state = root.join("comments.json");
    let labels = root.join("labels.mode");
    let log = root.join("gh.log");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(
        &state,
        wait_failure_comments("worker-a", "feat/test", "claimed"),
    )
    .unwrap();
    std::fs::write(&labels, "active\n").unwrap();
    seed_wait_claim_ref(
        &root,
        "worker-a",
        "feat/test",
        "claimed",
        "claim-generation-a",
    );
    write_executable(&bin.join("gh"), WAIT_FAILURE_GH);

    let crashed = wait_failure_command(&bin, &state, &log)
        .env("AUTOSPEC_WAIT_LABELS", &labels)
        .env("AUTOSPEC_WAIT_RECOVERY_FAILPOINT", "after_claim_cas")
        .output()
        .unwrap();
    assert!(!crashed.status.success());
    let transitioned_oid = wait_claim_ref_oid(&root);
    let generation = wait_claim_ref_generation(&root);
    let forged_record = RunStateRecord::new(
        "testorg/testrepo",
        42,
        "worker-b",
        "claimed",
        "feat/forged",
        "",
        "claimed",
        Vec::new(),
        "2099-07-19T00:00:00Z",
        "2099-07-19T00:00:00Z",
        10_800,
    )
    .with_claim_id("claim-generation-b");
    let forged_body = format!(
        "<!-- autospec-run-state-link parent=100 parent_generation=legacy generation={generation} -->\n{}",
        forged_record.to_marked_comment()
    );
    let forged_json = format!(
        r#"[{{"id":100,"updated_at":"2099-07-19T00:00:00Z","body":{}}}]"#,
        serde_json::to_string(&forged_body).unwrap()
    );
    std::fs::write(&state, forged_json).unwrap();

    let retried = wait_failure_command(&bin, &state, &log)
        .env("AUTOSPEC_WAIT_LABELS", &labels)
        .output()
        .unwrap();
    assert!(
        retried.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&retried.stderr)
    );
    assert_eq!(wait_claim_ref_oid(&root), transitioned_oid);
    let projected = std::fs::read_to_string(&state).unwrap();
    let comments: serde_json::Value = serde_json::from_str(&projected).unwrap();
    let available = comments
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|comment| comment.get("body").and_then(serde_json::Value::as_str))
        .filter(|body| body.contains(r#""state":"available""#))
        .collect::<Vec<_>>();
    assert_eq!(available.len(), 1);
    assert!(available[0].contains(r#""worker_id":"worker-a""#));
    assert!(available[0].contains(r#""branch":"feat/test""#));
    assert!(available[0].contains(r#""claim_id":"claim-generation-a""#));
}

#[test]
fn autonomous_implementer_wait_failed_does_not_requeue_until_failure_evidence_is_confirmed() {
    let root = temp_dir("autospec-wait-unconfirmed");
    let bin = root.join("bin");
    let state = root.join("comments.json");
    let log = root.join("gh.log");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(
        &state,
        wait_failure_comments("worker-a", "feat/test", "claimed"),
    )
    .unwrap();
    seed_wait_claim_ref(
        &root,
        "worker-a",
        "feat/test",
        "claimed",
        "claim-generation-a",
    );
    write_executable(&bin.join("gh"), WAIT_FAILURE_GH);
    let output = wait_failure_command(&bin, &state, &log)
        .env("AUTOSPEC_WAIT_EVIDENCE_NOOP", "1")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("implementer wait-failure evidence was not persisted"));
    let calls = std::fs::read_to_string(log).unwrap();
    assert!(!calls.contains("issue\nedit"));
    assert_eq!(calls.matches("autospec-executor-result:begin").count(), 1);
    assert!(!calls.contains("autospec-implementer-wait-failed"));
}

fn wait_failure_command(
    bin: &std::path::Path,
    state: &std::path::Path,
    log: &std::path::Path,
) -> Command {
    wait_failure_command_with_claim_id(bin, state, log, "claim-generation-a")
}

fn wait_failure_command_with_claim_id(
    bin: &std::path::Path,
    state: &std::path::Path,
    log: &std::path::Path,
    claim_id: &str,
) -> Command {
    let mut command = autospec();
    command
        .args([
            "autonomous",
            "implementer-wait-failed",
            "--repo",
            "testorg/testrepo",
            "--issue",
            "42",
            "--worker-id",
            "worker-a",
            "--branch",
            "feat/test",
            "--claim-id",
            claim_id,
            "--session-id",
            "session-real-7",
            "--diagnostic",
            "write_stdin failed: stdin is closed",
        ])
        .env(
            "PATH",
            format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
        )
        .env("AUTOSPEC_WAIT_STATE", state)
        .env("AUTOSPEC_WAIT_LOG", log)
        .env(
            "AUTOSPEC_CLAIM_GIT_REMOTE",
            state.parent().unwrap().join("claim-remote.git"),
        )
        .env(
            "AUTOSPEC_CLAIM_GIT_STATE_DIR",
            state.parent().unwrap().join("claim-state"),
        )
        .env("AUTOSPEC_CLAIM_RETRY_SLEEP_MS", "0");
    command
}

fn wait_git(directory: &std::path::Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(directory)
        .output()
        .expect("run wait-failure git fixture command");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn seed_wait_claim_ref(
    root: &std::path::Path,
    worker: &str,
    branch: &str,
    state: &str,
    claim_id: &str,
) -> std::path::PathBuf {
    seed_wait_claim_ref_at(
        root,
        worker,
        branch,
        state,
        claim_id,
        "2099-07-19T00:00:00Z",
    )
}

fn seed_wait_claim_ref_at(
    root: &std::path::Path,
    worker: &str,
    branch: &str,
    state: &str,
    claim_id: &str,
    updated_at: &str,
) -> std::path::PathBuf {
    let remote = root.join("claim-remote.git");
    let repo = root.join("claim-repo");
    if !remote.exists() {
        wait_git(root, &["init", "--bare", remote.to_str().unwrap()]);
    }
    if !repo.exists() {
        wait_git(root, &["init", repo.to_str().unwrap()]);
    }
    let reference = "refs/autospec/claims/issue-42";
    let current = Command::new("git")
        .args([
            "--git-dir",
            remote.to_str().unwrap(),
            "rev-parse",
            "--verify",
            reference,
        ])
        .output()
        .expect("inspect wait-failure claim ref");
    let parent = current
        .status
        .success()
        .then(|| String::from_utf8_lossy(&current.stdout).trim().to_string());
    if parent.is_some() {
        wait_git(
            &repo,
            &["fetch", "--no-tags", remote.to_str().unwrap(), reference],
        );
    }
    let tree = wait_git(&repo, &["mktree"]);
    let record = RunStateRecord::new(
        "testorg/testrepo",
        42,
        worker,
        state,
        branch,
        "",
        state,
        Vec::new(),
        updated_at,
        updated_at,
        10_800,
    )
    .with_claim_id(claim_id);
    let mut command = Command::new("git");
    command
        .arg("commit-tree")
        .arg(tree)
        .current_dir(&repo)
        .env("GIT_AUTHOR_NAME", "Autospec Wait Test")
        .env("GIT_AUTHOR_EMAIL", "autospec-wait-test@localhost")
        .env("GIT_COMMITTER_NAME", "Autospec Wait Test")
        .env("GIT_COMMITTER_EMAIL", "autospec-wait-test@localhost")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped());
    if let Some(parent) = parent.as_deref() {
        command.args(["-p", parent]);
    }
    let mut child = command.spawn().expect("create wait-failure claim commit");
    write!(
        child.stdin.take().expect("claim commit stdin"),
        "autospec-claim-ledger-v1\ngeneration=wait-fixture\n\n{}\n",
        record.to_marked_comment()
    )
    .expect("write wait-failure claim commit");
    let output = child.wait_with_output().expect("claim commit output");
    assert!(output.status.success());
    let oid = String::from_utf8_lossy(&output.stdout).trim().to_string();
    wait_git(
        &repo,
        &[
            "push",
            remote.to_str().unwrap(),
            &format!("{oid}:{reference}"),
        ],
    );
    remote
}

fn wait_claim_ref_oid(root: &std::path::Path) -> String {
    wait_git(
        root,
        &[
            "--git-dir",
            root.join("claim-remote.git").to_str().unwrap(),
            "rev-parse",
            "refs/autospec/claims/issue-42",
        ],
    )
}

fn wait_claim_ref_message(root: &std::path::Path) -> String {
    wait_git(
        root,
        &[
            "--git-dir",
            root.join("claim-remote.git").to_str().unwrap(),
            "show",
            "-s",
            "--format=%B",
            "refs/autospec/claims/issue-42",
        ],
    )
}

fn wait_claim_ref_generation(root: &std::path::Path) -> String {
    wait_claim_ref_message(root)
        .lines()
        .find_map(|line| line.strip_prefix("generation="))
        .expect("claim ref generation")
        .to_string()
}

fn wait_claim_acquire(
    bin: &std::path::Path,
    state: &std::path::Path,
    labels: &std::path::Path,
    log: &std::path::Path,
    heartbeats: &std::path::Path,
    worker: &str,
    branch: &str,
) -> Command {
    let issue_body = "## Safety review\n\n<!-- autospec-safety:begin -->\n- **decision:** `SAFETY_PASS`\n<!-- autospec-safety:end -->\n\n## Goal\nRecover the stranded issue.";
    let mut command = autospec();
    command
        .args([
            "claim",
            "acquire",
            "--issue",
            "42",
            "--repo",
            "testorg/testrepo",
            "--worker-id",
            worker,
            "--branch",
            branch,
        ])
        .env(
            "PATH",
            format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
        )
        .env("AUTOSPEC_WAIT_STATE", state)
        .env("AUTOSPEC_WAIT_LABELS", labels)
        .env("AUTOSPEC_WAIT_LOG", log)
        .env("AUTOSPEC_WAIT_ISSUE_BODY", issue_body)
        .env(
            "AUTOSPEC_CLAIM_GIT_REMOTE",
            state.parent().unwrap().join("claim-remote.git"),
        )
        .env(
            "AUTOSPEC_CLAIM_GIT_STATE_DIR",
            state.parent().unwrap().join("claim-state"),
        )
        .env("AUTOSPEC_HEARTBEAT_DIR", heartbeats)
        .env("AUTOSPEC_CLAIM_CONFIRM_READS", "1")
        .env("AUTOSPEC_CLAIM_SETTLE_MILLIS", "0")
        .env("AUTOSPEC_CLAIM_RETRY_SLEEP_MS", "0");
    command
}

fn force_first_wait_claimed_at(remote: &mut String, claimed_at: &str) {
    let marker = "\\\"claimed_at\\\":\\\"";
    let start = remote.find(marker).unwrap() + marker.len();
    let end = start + remote[start..].find("\\\"").unwrap();
    remote.replace_range(start..end, claimed_at);
}

fn wait_failure_comments(worker: &str, branch: &str, state: &str) -> String {
    wait_failure_comments_with_claim_id(worker, branch, state, "claim-generation-a")
}

fn wait_failure_comments_with_claim_id(
    worker: &str,
    branch: &str,
    state: &str,
    claim_id: &str,
) -> String {
    format!(
        r#"[{{"id":100,"updated_at":"2099-07-19T00:00:00Z","body":"<!-- autospec-run-state:begin -->\n{{\"schema\":1,\"repo\":\"testorg/testrepo\",\"issue\":42,\"worker_id\":\"{worker}\",\"state\":\"{state}\",\"branch\":\"{branch}\",\"pr\":\"\",\"step\":\"{state}\",\"paths\":[],\"claimed_at\":\"2099-07-19T00:00:00Z\",\"updated_at\":\"2099-07-19T00:00:00Z\",\"ttl_seconds\":10800,\"claim_id\":\"{claim_id}\"}}\n<!-- autospec-run-state:end -->"}}]"#
    )
}

fn wait_failure_terminal_comments(worker: &str, branch: &str) -> String {
    let active = wait_failure_comments(worker, branch, "claimed");
    let terminal = r#"{"id":99,"updated_at":"2099-07-19T00:00:01Z","body":"<!-- autospec-run-terminal:begin -->\n{\"schema\":1,\"repo\":\"testorg/testrepo\",\"issue\":42,\"worker_id\":\"worker-a\",\"state\":\"merged\",\"branch\":\"feat/test\",\"pr\":\"2273\",\"finalized_at\":\"2099-07-19T00:00:01Z\"}\n<!-- autospec-run-terminal:end -->"}"#;
    format!("[{},{}]", &active[1..active.len() - 1], terminal)
}

const WAIT_FAILURE_GH: &str = r#"#!/bin/sh
set -eu
printf '%s\n' "$@" >> "$AUTOSPEC_WAIT_LOG"
if [ "$1" = issue ] && [ "$2" = view ]; then
  if [ "$(cat "$AUTOSPEC_WAIT_LABELS")" = active ]; then labels='["in-progress-by-bot","safety:reviewed"]'; else labels='["auto-implement","safety:reviewed"]'; fi
  jq -n --argjson labels "$labels" --arg body "$AUTOSPEC_WAIT_ISSUE_BODY" '{labels:$labels,title:"Recover wait failure",body:$body,author:"agent"}'; exit 0
fi
if [ "$1" = label ] && [ "$2" = create ]; then exit 0; fi
if [ "$1" = api ] && [ "$2" = repos/testorg/testrepo/issues/42/comments ]; then cat "$AUTOSPEC_WAIT_STATE"; exit 0; fi
if [ "$1" = api ] && [ "$2" = repos/testorg/testrepo/issues/comments/100 ]; then
  body=''; shift 2; while [ "$#" -gt 0 ]; do case "$1" in -f) body="${2#body=}"; shift 2;; *) shift;; esac; done
  jq --arg body "$body" '.[0].body=$body' "$AUTOSPEC_WAIT_STATE" > "$AUTOSPEC_WAIT_STATE.tmp" && mv "$AUTOSPEC_WAIT_STATE.tmp" "$AUTOSPEC_WAIT_STATE"; exit 0
fi
if [ "$1" = issue ] && [ "$2" = edit ]; then
  if [ -n "${AUTOSPEC_WAIT_LABELS:-}" ]; then
    add=''; shift 2; while [ "$#" -gt 0 ]; do case "$1" in --add-label) add="$2"; shift 2;; *) shift;; esac; done
    if [ "$add" = auto-implement ]; then printf ready > "$AUTOSPEC_WAIT_LABELS"; else printf active > "$AUTOSPEC_WAIT_LABELS"; fi
  fi
  exit 0
fi
if [ "$1" = issue ] && [ "$2" = comment ]; then
  body=''; shift 2; while [ "$#" -gt 0 ]; do case "$1" in --body) body="$2"; shift 2;; *) shift;; esac; done
  if [ "${AUTOSPEC_WAIT_EVIDENCE_NOOP:-0}" = 1 ] && printf '%s' "$body" | grep -Fq 'autospec-executor-result:begin'; then exit 0; fi
  if printf '%s' "$body" | grep -Fq 'autospec-implementer-wait-failed'; then
    [ "${AUTOSPEC_WAIT_COMMENT_FAIL:-0}" != 1 ] || exit 44
  fi
  if [ "${AUTOSPEC_WAIT_RACE_SUCCESSOR:-0}" = 1 ] && printf '%s' "$body" | grep -Fq 'autospec-executor-result:begin'; then
    successor='<!-- autospec-run-state:begin -->
{"schema":1,"repo":"testorg/testrepo","issue":42,"worker_id":"worker-b","state":"claimed","branch":"feat/next","pr":"","step":"claimed","paths":[],"claimed_at":"2099-07-19T00:00:01Z","updated_at":"2099-07-19T00:00:01Z","ttl_seconds":10800,"claim_id":"claim-generation-b"}
<!-- autospec-run-state:end -->'
    jq --arg body "$body" --arg successor "$successor" '.[0].body=$successor | . + [{id:101,updated_at:"2099-07-19T00:00:01Z",body:$body}]' "$AUTOSPEC_WAIT_STATE" > "$AUTOSPEC_WAIT_STATE.tmp" && mv "$AUTOSPEC_WAIT_STATE.tmp" "$AUTOSPEC_WAIT_STATE"; exit 0
  fi
  jq --arg body "$body" '. + [{id:(([.[].id] | max) + 1),updated_at:"2099-07-19T00:00:01Z",body:$body}]' "$AUTOSPEC_WAIT_STATE" > "$AUTOSPEC_WAIT_STATE.tmp" && mv "$AUTOSPEC_WAIT_STATE.tmp" "$AUTOSPEC_WAIT_STATE"; exit 0
fi
exit 0
"#;
