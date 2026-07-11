use autospec_core::lint::{lint_issue_body, IssueQualityRule};

fn valid_issue_body(goal: &str, ac: &str, smoke: &str) -> String {
    format!(
        "## Goal\n{goal}\n\n## Acceptance criteria\n{ac}\n\n## Tests required\n- `cargo test issue_lint`\n\n### Primary smoke test (inner loop)\n```bash\n{smoke}\n```\n"
    )
}

fn rule_ids(body: &str) -> Vec<&'static str> {
    lint_issue_body(body)
        .iter()
        .map(|finding| finding.rule_id())
        .collect()
}

#[test]
fn issue_lint_valid_issue_with_single_sentence_goal_passes() {
    let body = valid_issue_body(
        "Add `scripts/lint-issue.sh` parity fixtures.",
        "- [ ] `cargo test issue_lint` passes.",
        "cargo test issue_lint",
    );

    assert!(lint_issue_body(&body).is_empty());
}

#[test]
fn issue_lint_valid_issue_with_two_short_sentences_passes() {
    let body = valid_issue_body(
        "Add `IssueQualityRule` fixtures. Keep `cargo test issue_lint` green.",
        "- [ ] `IssueQualityRule` reports 3 rule ids.",
        "cargo test issue_lint",
    );

    assert!(lint_issue_body(&body).is_empty());
}

#[test]
fn issue_lint_missing_goal_emits_goal_not_one_sentence() {
    let body = "## Acceptance criteria\n- [ ] `cargo test issue_lint` passes.\n";

    let findings = lint_issue_body(body);

    assert!(findings
        .iter()
        .any(|finding| finding.rule == IssueQualityRule::GoalNotOneSentence));
    assert!(rule_ids(body).contains(&"GOAL_NOT_ONE_SENTENCE"));
}

#[test]
fn issue_lint_mixed_non_checkbox_acceptance_criteria_emits_ac_prose() {
    let body = valid_issue_body(
        "Add `IssueQualityRule` fixtures.",
        "- [ ] `cargo test issue_lint` passes.
This criterion is prose, not a checkbox.",
        "cargo test issue_lint",
    );

    let findings = lint_issue_body(&body);

    assert!(findings
        .iter()
        .any(|finding| finding.rule == IssueQualityRule::AcProse));
}

#[test]
fn issue_lint_spaced_checkbox_plus_prose_emits_ac_prose() {
    let body = valid_issue_body(
        "Add `IssueQualityRule` fixtures.",
        "-   [ ] `cargo test issue_lint` passes.
This criterion is prose, not a checkbox.",
        "cargo test issue_lint",
    );

    let findings = lint_issue_body(&body);

    assert!(findings
        .iter()
        .any(|finding| finding.rule == IssueQualityRule::AcProse));
}

#[test]
fn issue_lint_checkbox_without_content_space_emits_ac_prose() {
    let body = valid_issue_body(
        "Add `IssueQualityRule` fixtures.",
        "- [ ]`cargo test issue_lint` passes.",
        "cargo test issue_lint",
    );

    let findings = lint_issue_body(&body);

    assert!(findings
        .iter()
        .any(|finding| finding.rule == IssueQualityRule::AcProse));
}

#[test]
fn issue_lint_prose_only_acceptance_criteria_does_not_emit_ac_prose() {
    let body = valid_issue_body(
        "Add `IssueQualityRule` fixtures.",
        "This shell-parity prose-only block is handled by `AC_EMPTY` later.",
        "cargo test issue_lint",
    );

    assert!(!lint_issue_body(&body)
        .iter()
        .any(|finding| finding.rule == IssueQualityRule::AcProse));
}

#[test]
fn issue_lint_verification_fallback_emits_smoke_multi_line() {
    let body = "## Goal\nAdd `IssueQualityRule` fixtures.\n\n## Acceptance criteria\n- [ ] `cargo test issue_lint` passes.\n\n## Verification\n```bash\ncargo test issue_lint\ncargo test --all\n```\n";

    let findings = lint_issue_body(body);

    assert!(findings
        .iter()
        .any(|finding| finding.rule == IssueQualityRule::SmokeMultiLine));
}

#[test]
fn issue_lint_multi_line_smoke_block_emits_smoke_multi_line() {
    let body = valid_issue_body(
        "Add `IssueQualityRule` fixtures.",
        "- [ ] `cargo test issue_lint` passes.",
        "cargo test issue_lint\ncargo test --all",
    );

    let findings = lint_issue_body(&body);

    assert!(findings
        .iter()
        .any(|finding| finding.rule == IssueQualityRule::SmokeMultiLine));
    assert!(rule_ids(&body).contains(&"SMOKE_MULTI_LINE"));
}
