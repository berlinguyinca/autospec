use autospec_core::lint::lint_issue_body;

fn valid_issue_body(goal: &str, ac: &str, smoke: &str) -> String {
    format!(
        "## Goal\n{goal}\n\n## Files to read first\n- crates/autospec-core/src/lib.rs\n\n## Implementation outline\n1. Implement the policy.\n\n## Tests required\n- `cargo test -p autospec-core --test issue_lint`\n\n## Dependencies\nnone\n\n## Files touched\n- crates/autospec-core/src/lint/mod.rs\n\n## Acceptance criteria\n{ac}\n\n## Verification\n\n### Primary smoke test (inner loop)\n\n```bash\n{smoke}\n```\n"
    )
}

fn findings(body: &str) -> Vec<(String, String)> {
    lint_issue_body(body)
        .into_iter()
        .map(|finding| (finding.rule_id().to_string(), finding.message))
        .collect()
}

fn expected(rows: &[(&str, &str)]) -> Vec<(String, String)> {
    rows.iter()
        .map(|(rule, message)| ((*rule).to_string(), (*message).to_string()))
        .collect()
}

fn assert_findings(body: &str, rows: &[(&str, &str)]) {
    assert_eq!(findings(body), expected(rows));
}

fn replace_once(body: String, needle: &str, replacement: &str) -> String {
    assert!(body.contains(needle), "body did not contain {needle:?}");
    body.replacen(needle, replacement, 1)
}

#[test]
fn issue_lint_valid_complete_issue_passes() {
    let body = valid_issue_body(
        "Add `lint_issue_body` parity fixtures.",
        "- [ ] `cargo test -p autospec-core --test issue_lint` passes.",
        "cargo test -p autospec-core --test issue_lint",
    );

    assert!(lint_issue_body(&body).is_empty());
}

#[test]
fn issue_lint_allows_two_short_goal_sentences() {
    let body = valid_issue_body(
        "Add `lint_issue_body` fixtures. Keep `cargo test issue_lint` green.",
        "- [ ] `cargo test issue_lint` reports no findings.",
        "cargo test issue_lint",
    );

    assert!(lint_issue_body(&body).is_empty());
}

#[test]
fn issue_lint_allows_a_zero_terminal_goal() {
    let body = valid_issue_body(
        "Add lint issue parity fixtures without terminal punctuation",
        "- [ ] `cargo test issue_lint` reports no findings.",
        "cargo test issue_lint",
    );

    assert!(lint_issue_body(&body).is_empty());
}

#[test]
fn issue_lint_reports_a_goal_over_the_sentence_limit_with_the_shell_message() {
    let body = valid_issue_body(
        "Add `lint_issue_body` fixtures. Keep the tests green. Preserve shell parity.",
        "- [ ] `cargo test issue_lint` reports no findings.",
        "cargo test issue_lint",
    );

    assert_findings(
        &body,
        &[(
            "GOAL_NOT_ONE_SENTENCE",
            "Goal must be at most 2 sentences and 30 words; found 3 sentence(s) and 10 word(s)",
        )],
    );
}

#[test]
fn issue_lint_reports_goal_vague_with_the_shell_message() {
    let body = valid_issue_body(
        "improve the issue body.",
        "- [ ] `cargo test issue_lint` passes.",
        "cargo test issue_lint",
    );

    assert_findings(
        &body,
        &[(
            "GOAL_VAGUE",
            "Bare vague verb 'improve' used without a concrete object (path, backtick term, number, or UPPER_SNAKE label)",
        )],
    );
}

#[test]
fn issue_lint_uses_any_nonempty_backtick_span_as_a_concrete_goal_object() {
    let body = valid_issue_body(
        "improve `` `lint_issue_body` output.",
        "- [ ] `cargo test issue_lint` passes.",
        "cargo test issue_lint",
    );

    assert!(lint_issue_body(&body).is_empty());
}

#[test]
fn issue_lint_does_not_treat_a_number_inside_a_word_as_concrete() {
    let body = valid_issue_body(
        "improve v1 output.",
        "- [ ] `cargo test issue_lint` passes.",
        "cargo test issue_lint",
    );

    assert_findings(
        &body,
        &[ (
            "GOAL_VAGUE",
            "Bare vague verb 'improve' used without a concrete object (path, backtick term, number, or UPPER_SNAKE label)",
        )],
    );
}

#[test]
fn issue_lint_uses_unicode_aware_word_boundaries_like_the_shell() {
    let body = valid_issue_body(
        "éimprove improveé output.",
        "- [ ] Output élooks looksé.",
        "echo éTBD TBDé",
    );

    assert!(lint_issue_body(&body).is_empty());
}

#[test]
fn issue_lint_does_not_treat_unicode_adjacent_numbers_or_labels_as_concrete() {
    for goal in ["improve é1 output.", "improve éTOKEN output."] {
        let body = valid_issue_body(
            goal,
            "- [ ] `cargo test issue_lint` passes.",
            "cargo test issue_lint",
        );

        assert_findings(
            &body,
            &[ (
                "GOAL_VAGUE",
                "Bare vague verb 'improve' used without a concrete object (path, backtick term, number, or UPPER_SNAKE label)",
            )],
        );
    }
}

#[test]
fn issue_lint_reports_goal_hedge_with_the_shell_message() {
    let body = valid_issue_body(
        "The issue should return a stable result.",
        "- [ ] `cargo test issue_lint` passes.",
        "cargo test issue_lint",
    );

    assert_findings(
        &body,
        &[(
            "GOAL_HEDGE",
            "Hedging word 'should' found in Goal section; state the outcome flatly",
        )],
    );
}

#[test]
fn issue_lint_reports_missing_goal_with_the_shell_message() {
    let body = valid_issue_body(
        "",
        "- [ ] `cargo test issue_lint` passes.",
        "cargo test issue_lint",
    );

    assert_findings(
        &body,
        &[("GOAL_NOT_ONE_SENTENCE", "Goal section is empty or missing")],
    );
}

#[test]
fn issue_lint_reports_ac_prose_with_the_shell_message() {
    let body = valid_issue_body(
        "Add `lint_issue_body` parity fixtures.",
        "- [ ] `cargo test issue_lint` passes.\nThis criterion is prose, not a checkbox.",
        "cargo test issue_lint",
    );

    assert_findings(
        &body,
        &[(
            "AC_PROSE",
            "AC line 2 is not a checkbox ('- [ ]' with content required): This criterion is prose, not a checkbox.",
        )],
    );
}

#[test]
fn issue_lint_reports_ac_subjective_with_the_shell_message() {
    let body = valid_issue_body(
        "Add `lint_issue_body` parity fixtures.",
        "- [ ] Output looks clean for `autospec`.",
        "cargo test issue_lint",
    );

    assert_findings(
        &body,
        &[(
            "AC_SUBJECTIVE",
            "AC item 1 contains subjective word 'looks': - [ ] Output looks clean for `autospec`.",
        )],
    );
}

#[test]
fn issue_lint_reports_ac_too_long_with_the_shell_message() {
    let item = "x".repeat(121);
    let body = valid_issue_body(
        "Add `lint_issue_body` parity fixtures.",
        &format!("- [ ] {item}"),
        "cargo test issue_lint",
    );
    let message = format!("AC item 1 is 121 chars (max 120): {}...", "x".repeat(60));

    assert_eq!(findings(&body), vec![("AC_TOO_LONG".to_string(), message)]);
}

#[test]
fn issue_lint_reports_empty_acceptance_criteria_with_the_shell_message() {
    let body = valid_issue_body(
        "Add `lint_issue_body` parity fixtures.",
        "",
        "cargo test issue_lint",
    );

    assert_findings(
        &body,
        &[(
            "AC_EMPTY",
            "Acceptance criteria section has no checkbox items (section missing or empty)",
        )],
    );
}

#[test]
fn issue_lint_reports_acceptance_criteria_without_any_checkbox() {
    let body = valid_issue_body(
        "Add `lint_issue_body` parity fixtures.",
        "The Rust test must pass.",
        "cargo test issue_lint",
    );

    assert_findings(
        &body,
        &[(
            "AC_EMPTY",
            "Acceptance criteria section has no '- [ ]' checkbox items",
        )],
    );
}

#[test]
fn issue_lint_keeps_the_shells_non_enforced_ac_token_check_non_enforced() {
    let body = valid_issue_body(
        "Add `lint_issue_body` parity fixtures.",
        "- [ ] the criterion has no concrete token",
        "cargo test issue_lint",
    );

    assert!(lint_issue_body(&body).is_empty());
}

#[test]
fn issue_lint_reports_multiline_smoke_with_the_shell_message() {
    let body = valid_issue_body(
        "Add `lint_issue_body` parity fixtures.",
        "- [ ] `cargo test issue_lint` passes.",
        "cargo test issue_lint\ncargo test --workspace",
    );

    assert_findings(
        &body,
        &[(
            "SMOKE_MULTI_LINE",
            "Primary smoke test has 2 non-blank/non-comment lines (must be exactly 1; use '&&' to chain)",
        )],
    );
}

#[test]
fn issue_lint_reports_smoke_placeholder_with_the_shell_message() {
    let body = valid_issue_body(
        "Add `lint_issue_body` parity fixtures.",
        "- [ ] `cargo test issue_lint` passes.",
        "cargo test <TODO>",
    );

    assert_findings(
        &body,
        &[(
            "SMOKE_PLACEHOLDER",
            "Primary smoke test block contains placeholder '<TODO>'",
        )],
    );
}

#[test]
fn issue_lint_reports_unfenced_smoke_with_the_shell_message() {
    let body = replace_once(
        valid_issue_body(
            "Add `lint_issue_body` parity fixtures.",
            "- [ ] `cargo test issue_lint` passes.",
            "cargo test issue_lint",
        ),
        "```bash\ncargo test issue_lint\n```",
        "Run `cargo test issue_lint`.",
    );

    assert_findings(
        &body,
        &[(
            "SMOKE_NOT_FENCED",
            "No fenced code block found under Primary smoke test heading",
        )],
    );
}

#[test]
fn issue_lint_allows_a_missing_smoke_section() {
    let body = valid_issue_body(
        "Add `lint_issue_body` parity fixtures.",
        "- [ ] `cargo test issue_lint` passes.",
        "cargo test issue_lint",
    )
    .split("\n## Verification")
    .next()
    .expect("valid fixture has verification")
    .to_owned();

    assert!(lint_issue_body(&body).is_empty());
}

#[test]
fn issue_lint_checks_only_the_first_fenced_smoke_block() {
    let body = valid_issue_body(
        "Add `lint_issue_body` parity fixtures.",
        "- [ ] `cargo test issue_lint` passes.",
        "echo first\n```\n\n```bash\necho <TODO>\necho second",
    );

    assert!(lint_issue_body(&body).is_empty());
}

#[test]
fn issue_lint_reports_each_missing_required_section_with_the_shell_message() {
    for (section, rule, message) in [
        (
            "## Files to read first\n- crates/autospec-core/src/lib.rs\n\n",
            "MISSING_SECTION_FILES_TO_READ",
            "Body has no '## Files to read first' heading (implementer reads it)",
        ),
        (
            "## Implementation outline\n1. Implement the policy.\n\n",
            "MISSING_SECTION_IMPL_OUTLINE",
            "Body has no '## Implementation outline' heading (implementer reads it)",
        ),
        (
            "## Tests required\n- `cargo test -p autospec-core --test issue_lint`\n\n",
            "MISSING_SECTION_TESTS",
            "Body has no '## Tests required' heading (implementer reads it)",
        ),
    ] {
        let body = replace_once(
            valid_issue_body(
                "Add `lint_issue_body` parity fixtures.",
                "- [ ] `cargo test issue_lint` passes.",
                "cargo test issue_lint",
            ),
            section,
            "",
        );

        assert_findings(&body, &[(rule, message)]);
    }
}

#[test]
fn issue_lint_reports_malformed_dependencies_with_the_shell_message() {
    let body = replace_once(
        valid_issue_body(
            "Add `lint_issue_body` parity fixtures.",
            "- [ ] `cargo test issue_lint` passes.",
            "cargo test issue_lint",
        ),
        "## Dependencies\nnone",
        "## Dependencies\nblocked by #5",
    );

    assert_findings(
        &body,
        &[(
            "DEPS_MALFORMED",
            "Dependencies line must be 'Depends on issue #N' or 'none': blocked by #5",
        )],
    );
}

#[test]
fn issue_lint_reports_too_many_files_with_the_shell_message() {
    let body = replace_once(
        valid_issue_body(
            "Add `lint_issue_body` parity fixtures.",
            "- [ ] `cargo test issue_lint` passes.",
            "cargo test issue_lint",
        ),
        "- crates/autospec-core/src/lint/mod.rs",
        "- scripts/a.sh\n- scripts/b.sh\n- scripts/c.sh\n- scripts/d.sh",
    );

    assert_findings(
        &body,
        &[(
            "TOO_MANY_FILES",
            "Files touched lists 4 logical units (max 3; trio members + derived goldens count as one); split the issue to stay small-LLM-sized",
        )],
    );
}

#[test]
fn issue_lint_allows_three_logical_file_units() {
    let body = replace_once(
        valid_issue_body(
            "Add `lint_issue_body` parity fixtures.",
            "- [ ] `cargo test issue_lint` passes.",
            "cargo test issue_lint",
        ),
        "- crates/autospec-core/src/lint/mod.rs",
        "- skills/autospec/SKILL.md\n- skills/autospec/codex/prompt.md\n- skills/autospec/opencode/agent.md\n- tests/fixtures/skill-goldens/autospec.sha256\n- scripts/a.sh\n- scripts/b.sh",
    );

    assert!(lint_issue_body(&body).is_empty());
}

#[test]
fn issue_lint_rejects_malformed_files_touched_entries() {
    for entry in [
        "To be determined.",
        "/",
        ".",
        "../src/changed.rs",
        "/src/changed.rs",
        "src/changed.rs//",
        "src/changed.rs vendor/other.rs",
    ] {
        let body = replace_once(
            valid_issue_body(
                "Add `lint_issue_body` parity fixtures.",
                "- [ ] `cargo test issue_lint` passes.",
                "cargo test issue_lint",
            ),
            "- crates/autospec-core/src/lint/mod.rs",
            &format!("- {entry}"),
        );

        assert_eq!(
            findings(&body),
            vec![(
                "FILES_TOUCHED_MALFORMED".to_string(),
                format!(
                    "Files touched entry must be one safe repo-relative file or trailing-slash directory: - {entry}"
                ),
            )],
            "entry {entry:?} must fail issue quality"
        );
    }
}

#[test]
fn issue_lint_accepts_an_explicit_repo_relative_directory() {
    let body = replace_once(
        valid_issue_body(
            "Add `lint_issue_body` parity fixtures.",
            "- [ ] `cargo test issue_lint` passes.",
            "cargo test issue_lint",
        ),
        "- crates/autospec-core/src/lint/mod.rs",
        "- crates/autospec-core/src/lint/",
    );

    assert!(lint_issue_body(&body).is_empty());
}

#[test]
fn issue_lint_reports_body_too_long_with_the_shell_message() {
    let body = format!(
        "{}\n## Notes\n{}",
        valid_issue_body(
            "Add `lint_issue_body` parity fixtures.",
            "- [ ] `cargo test issue_lint` passes.",
            "cargo test issue_lint",
        ),
        "word ".repeat(401)
    );
    let word_count = body.split_whitespace().count();

    assert_eq!(
        findings(&body),
        vec![(("BODY_TOO_LONG").to_string(), format!(
            "Body is {word_count} words (max 400); a small-LLM implementer cannot hold an over-long issue"
        ))]
    );
}

#[test]
fn issue_lint_keeps_the_400_word_body_limit_inclusive() {
    let mut body = valid_issue_body(
        "Add `lint_issue_body` parity fixtures.",
        "- [ ] `cargo test issue_lint` passes.",
        "cargo test issue_lint",
    );
    let extra_words = 400 - body.split_whitespace().count();
    body.push(' ');
    body.push_str(&"word ".repeat(extra_words));
    assert_eq!(body.split_whitespace().count(), 400);

    assert!(lint_issue_body(&body).is_empty());
}

#[test]
fn issue_lint_reports_outline_too_long_with_the_shell_message() {
    let outline = (1..=31)
        .map(|index| format!("{index}. Implement step {index}."))
        .collect::<Vec<_>>()
        .join("\n");
    let body = replace_once(
        valid_issue_body(
            "Add `lint_issue_body` parity fixtures.",
            "- [ ] `cargo test issue_lint` passes.",
            "cargo test issue_lint",
        ),
        "1. Implement the policy.",
        &outline,
    );

    assert_findings(
        &body,
        &[(
            "OUTLINE_TOO_LONG",
            "Implementation outline has 31 non-blank lines (max 30); tighten or split",
        )],
    );
}

#[test]
fn issue_lint_keeps_the_30_line_outline_limit_inclusive() {
    let outline = (1..=30)
        .map(|index| format!("{index}. Implement step {index}."))
        .collect::<Vec<_>>()
        .join("\n");
    let body = replace_once(
        valid_issue_body(
            "Add `lint_issue_body` parity fixtures.",
            "- [ ] `cargo test issue_lint` passes.",
            "cargo test issue_lint",
        ),
        "1. Implement the policy.",
        &outline,
    );

    assert!(lint_issue_body(&body).is_empty());
}

#[test]
fn issue_lint_preserves_the_shell_rule_order_for_multiple_findings() {
    let outline = (1..=31)
        .map(|index| format!("{index}. Implement step {index}."))
        .collect::<Vec<_>>()
        .join("\n");
    let body = format!(
        "{}\n<!-- ui-feature -->\n\n## Notes\n{}",
        replace_once(
            replace_once(
                replace_once(
                    replace_once(
                        replace_once(
                            replace_once(
                                valid_issue_body(
                                    "improve the issue body should become better.",
                                    "- [ ] Output looks clean for `autospec`.\nThis line is prose.",
                                    "echo <TODO>\necho two",
                                ),
                                "## Files to read first\n- crates/autospec-core/src/lib.rs\n\n",
                                "",
                            ),
                            "## Tests required\n- `cargo test -p autospec-core --test issue_lint`\n\n",
                            "",
                        ),
                        "## Dependencies\nnone",
                        "## Dependencies\nblocked by #5",
                    ),
                    "- crates/autospec-core/src/lint/mod.rs",
                    "- scripts/a.sh\n- scripts/b.sh\n- scripts/c.sh\n- scripts/d.sh",
                ),
                "1. Implement the policy.",
                &outline,
            ),
            "## Verification",
            "## Verification",
        ),
        "word ".repeat(401)
    );
    let word_count = body.split_whitespace().count();

    assert_eq!(
        findings(&body),
        vec![
            (
                "GOAL_VAGUE".to_string(),
                "Bare vague verb 'improve' used without a concrete object (path, backtick term, number, or UPPER_SNAKE label)".to_string(),
            ),
            (
                "GOAL_HEDGE".to_string(),
                "Hedging word 'should' found in Goal section; state the outcome flatly".to_string(),
            ),
            (
                "AC_SUBJECTIVE".to_string(),
                "AC item 1 contains subjective word 'looks': - [ ] Output looks clean for `autospec`.".to_string(),
            ),
            (
                "AC_PROSE".to_string(),
                "AC line 2 is not a checkbox ('- [ ]' with content required): This line is prose.".to_string(),
            ),
            (
                "SMOKE_PLACEHOLDER".to_string(),
                "Primary smoke test block contains placeholder '<TODO>'".to_string(),
            ),
            (
                "SMOKE_MULTI_LINE".to_string(),
                "Primary smoke test has 2 non-blank/non-comment lines (must be exactly 1; use '&&' to chain)".to_string(),
            ),
            (
                "MISSING_SECTION_FILES_TO_READ".to_string(),
                "Body has no '## Files to read first' heading (implementer reads it)".to_string(),
            ),
            (
                "MISSING_SECTION_TESTS".to_string(),
                "Body has no '## Tests required' heading (implementer reads it)".to_string(),
            ),
            (
                "DEPS_MALFORMED".to_string(),
                "Dependencies line must be 'Depends on issue #N' or 'none': blocked by #5".to_string(),
            ),
            (
                "TOO_MANY_FILES".to_string(),
                "Files touched lists 4 logical units (max 3; trio members + derived goldens count as one); split the issue to stay small-LLM-sized".to_string(),
            ),
            (
                "BODY_TOO_LONG".to_string(),
                format!("Body is {word_count} words (max 400); a small-LLM implementer cannot hold an over-long issue"),
            ),
            (
                "OUTLINE_TOO_LONG".to_string(),
                "Implementation outline has 31 non-blank lines (max 30); tighten or split".to_string(),
            ),
            (
                "UI_SECTIONS_INCOMPLETE".to_string(),
                "UI feature detected; missing required section(s): '## Design reference' '## Interaction states' '## UX flows' '## Motion & feedback' '## Device & viewport' (UI issues need Design reference + Interaction states + UX flows + Motion & feedback + Device & viewport)".to_string(),
            ),
        ]
    );
}
