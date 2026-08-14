//! ui-feature contract cases for the issue lint: the five required sections, marker
//! detection, and the spec §L1a exclusion that keeps those sections out of the
//! BODY_TOO_LONG word count.
//!
//! Split out of issue_lint.rs, which had grown past the file-size limit. Rust
//! integration tests are separate crates, so the fixture helpers are duplicated rather
//! than shared through a common module.

use autospec_core::lint::lint_issue_body;

fn valid_issue_body(goal: &str, ac: &str, smoke: &str) -> String {
    format!(
        "## Goal\n{goal}\n\n## Files to read first\n- crates/autospec-core/src/lib.rs\n\n## Implementation outline\n1. Implement the policy.\n\n## Tests required\n- `cargo test -p autospec-core`\n\n## Dependencies\nnone\n\n## Files touched\n- crates/autospec-core/src/lint/mod.rs\n\n## Acceptance criteria\n{ac}\n\n## Verification\n\n### Primary smoke test (inner loop)\n\n```bash\n{smoke}\n```\n"
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

/// Spec §L1a (docs/superpowers/specs/2026-08-04-autospec-web-ui-design.md):
/// the five `ui-feature` sections are excluded from the ≤400-word body count.
/// A ~400-word non-UI body plus all five sections' own content — which alone
/// would push the raw word count past 400 — must still pass.
#[test]
fn issue_lint_excludes_ui_sections_from_the_400_word_body_count() {
    let mut body = valid_issue_body(
        "Add `lint_issue_body` parity fixtures.",
        "- [ ] `cargo test issue_lint` passes.",
        "cargo test issue_lint",
    );
    let extra_words = 400 - body.split_whitespace().count();
    body.push(' ');
    body.push_str(&"word ".repeat(extra_words));
    assert_eq!(body.split_whitespace().count(), 400);

    body.push_str(&format!(
        "\n## Design reference\n\n{a}\n\n## Interaction states\n\n{b}\n\n## UX flows\n\n{c}\n\n## Motion & feedback\n\n{d}\n\n## Device & viewport\n\n{e}\n",
        a = "uiword ".repeat(10),
        b = "uiword ".repeat(10),
        c = "uiword ".repeat(10),
        d = "uiword ".repeat(10),
        e = "uiword ".repeat(10),
    ));

    // Sanity: without the §L1a exclusion the raw word count would exceed 400.
    assert!(body.split_whitespace().count() > 400);

    assert!(lint_issue_body(&body).is_empty());
}

/// Negative pair for the exclusion above: non-UI prose that itself exceeds 400
/// words must still trip BODY_TOO_LONG even with all five UI sections present —
/// the exclusion narrows the count, it does not disable the cap.
#[test]
fn issue_lint_still_reports_body_too_long_when_non_ui_prose_exceeds_400_words() {
    let mut body = valid_issue_body(
        "Add `lint_issue_body` parity fixtures.",
        "- [ ] `cargo test issue_lint` passes.",
        "cargo test issue_lint",
    );
    let extra_words = 401 - body.split_whitespace().count();
    body.push(' ');
    body.push_str(&"word ".repeat(extra_words));
    let non_ui_word_count = body.split_whitespace().count();
    assert_eq!(non_ui_word_count, 401);

    body.push_str(
        "\n## Design reference\n\nDESIGN.md#buttons\n\n## Interaction states\n\ndefault/hover/focus\n\n## UX flows\n\nhappy: click -> submit\n\n## Motion & feedback\n\nMotion: fade-in; reduced: opacity-only\n\n## Device & viewport\n\nDevices: iPhone SE; reflow-320: no h-scroll\n",
    );

    let rows = findings(&body);
    assert!(
        rows.iter().any(|(rule, message)| rule == "BODY_TOO_LONG"
            && *message
                == format!(
                    "Body is {non_ui_word_count} words (max 400); a small-LLM implementer cannot hold an over-long issue"
                )),
        "expected BODY_TOO_LONG at {non_ui_word_count} words, got {rows:?}"
    );
    assert!(
        !rows
            .iter()
            .any(|(rule, _)| rule == "UI_SECTIONS_INCOMPLETE"),
        "all five UI sections are present; UI_SECTIONS_INCOMPLETE should not fire: {rows:?}"
    );
}

#[test]
fn issue_lint_reports_incomplete_ui_sections_with_the_shell_message() {
    let body = format!(
        "{}\n<!-- ui-feature -->\n",
        valid_issue_body(
            "Add `lint_issue_body` parity fixtures.",
            "- [ ] `cargo test issue_lint` passes.",
            "cargo test issue_lint",
        )
    );

    assert_findings(
        &body,
        &[(
            "UI_SECTIONS_INCOMPLETE",
            "UI feature detected; missing required section(s): '## Design reference' '## Interaction states' '## UX flows' '## Motion & feedback' '## Device & viewport' (UI issues need Design reference + Interaction states + UX flows + Motion & feedback + Device & viewport)",
        )],
    );
}

#[test]
fn issue_lint_detects_a_ui_marker_after_an_earlier_html_comment() {
    let body = format!(
        "{}\n<!-- note --> <!-- ui-feature -->\n",
        valid_issue_body(
            "Add `lint_issue_body` parity fixtures.",
            "- [ ] `cargo test issue_lint` passes.",
            "cargo test issue_lint",
        )
    );

    assert_findings(
        &body,
        &[ (
            "UI_SECTIONS_INCOMPLETE",
            "UI feature detected; missing required section(s): '## Design reference' '## Interaction states' '## UX flows' '## Motion & feedback' '## Device & viewport' (UI issues need Design reference + Interaction states + UX flows + Motion & feedback + Device & viewport)",
        )],
    );
}

#[test]
fn issue_lint_detects_a_ui_marker_nested_in_a_malformed_comment() {
    let body = format!(
        "{}\n<!-- malformed <!-- ui-feature -->\n",
        valid_issue_body(
            "Add `lint_issue_body` parity fixtures.",
            "- [ ] `cargo test issue_lint` passes.",
            "cargo test issue_lint",
        )
    );

    assert_findings(
        &body,
        &[ (
            "UI_SECTIONS_INCOMPLETE",
            "UI feature detected; missing required section(s): '## Design reference' '## Interaction states' '## UX flows' '## Motion & feedback' '## Device & viewport' (UI issues need Design reference + Interaction states + UX flows + Motion & feedback + Device & viewport)",
        )],
    );
}

#[test]
fn issue_lint_preserves_ui_missing_section_order() {
    let body = format!(
        "{}\n## Interaction states\n- Loading\n",
        valid_issue_body(
            "Add `lint_issue_body` parity fixtures.",
            "- [ ] `cargo test issue_lint` passes.",
            "cargo test issue_lint",
        )
    );

    assert_findings(
        &body,
        &[ (
            "UI_SECTIONS_INCOMPLETE",
            "UI feature detected; missing required section(s): '## Design reference' '## UX flows' '## Motion & feedback' '## Device & viewport' (UI issues need Design reference + Interaction states + UX flows + Motion & feedback + Device & viewport)",
        )],
    );
}

/// A UI heading carrying trailing whitespace is still excluded from the word count.
/// `has_heading` accepts trailing whitespace when deciding the section is present, so
/// the exclusion has to accept it too; if the two disagree the section counts against
/// the cap it is exempt from. Markdown's hard line break is two trailing spaces, so a
/// heading written that way is ordinary rather than pathological.
#[test]
fn issue_lint_excludes_ui_sections_whose_headings_have_trailing_whitespace() {
    let body = format!(
        "{base}\n## Design reference  \n\n{a}\n## Interaction states \n\n{b}\n## UX flows   \n\n{c}\n## Motion & feedback  \n\n{d}\n## Device & viewport  \n\n{e}\n",
        base = valid_issue_body(
            "Add `lint_issue_body` parity fixtures.",
            "- [ ] `cargo test issue_lint` passes.",
            "cargo test issue_lint",
        ),
        a = "uiword ".repeat(100),
        b = "uiword ".repeat(100),
        c = "uiword ".repeat(100),
        d = "uiword ".repeat(100),
        e = "uiword ".repeat(100),
    );

    // Sanity: counted raw, this body is far past the cap, so a pass cannot be an
    // accident of a body that was under 400 words anyway.
    assert!(body.split_whitespace().count() > 500);

    let found = findings(&body);
    assert!(
        !found.iter().any(|(rule, _)| rule == "BODY_TOO_LONG"),
        "trailing whitespace on a UI heading must not reinstate the word cap: {found:?}"
    );
    // And the headings must still register as present, or this would pass because the
    // sections went invisible to both checks rather than because they are exempt.
    assert!(
        !found
            .iter()
            .any(|(rule, _)| rule == "UI_SECTIONS_INCOMPLETE"),
        "the same headings must satisfy the section-presence check: {found:?}"
    );
}
