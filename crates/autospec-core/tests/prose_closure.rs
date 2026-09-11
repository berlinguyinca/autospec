//! Prose closure safety (issue #4305).
//!
//! The regression tests instantiate the configuration the incident
//! produced: a PR body that carries a `Refs #288 (does not close it)`
//! marker while also containing the prose `closed #288` describing an
//! earlier PR. GitHub's closing-keyword detection operates on the raw
//! markdown, so the prose closed the issue on merge while the marker
//! claimed it would not. On a body that either closes via a canonical
//! trailer or discusses closure only through escaped or URL
//! references, every gate passes — which is the invariant the incident
//! violated on all four counts at once.

use autospec_core::prose_closure::{
    escaped_reference, find_closure_directives, has_closing_trailer, lint_refs_body,
    pre_publish_lint, prose_violations, url_reference, verify_after_merge, ClosingDirective,
    ClosureDecision, IssueState, PostMergeAction, ReferenceForm,
};

/// The incident's body: the partial-fix marker on line 1, and prose
/// describing an earlier PR on line 11. GitHub's keyword detection reads
/// the prose and closed #288 on merge.
fn incident_body() -> String {
    [
        "Refs #288 (does not close it)",
        "",
        "## Summary",
        "",
        "Follow-up to the conversion work that shipped last week.",
        "",
        "The earlier attempt was reverted because the conversion gate",
        "skipped the test step; this revision re-runs the full suite.",
        "",
        "See the incident notes for the timeline.",
        "closed #288",
        "",
        "## Tests",
        "",
        "cargo test -p autospec-core",
    ]
    .join("\n")
}

// --- Invariant 1: the detector finds a closing verb adjacent to #N -----

#[test]
fn detector_flags_prose_closure_in_the_incident_body() {
    let body = incident_body();
    let directives = find_closure_directives(&body, 288);
    // The prose on line 11 is a live directive.
    assert_eq!(
        directives,
        vec![ClosingDirective {
            line: 11,
            verb: "closed",
            form: ReferenceForm::Bare,
        }]
    );
    // The `Refs #288 (does not close it)` line on line 1 is NOT a
    // directive: "refs" is not a closing verb.
    assert!(
        !directives.iter().any(|d| d.line == 1),
        "the Refs marker line must not read as a closing directive: {directives:?}"
    );
}

#[test]
fn detector_matches_every_verb_and_is_case_insensitive() {
    for verb in [
        "close", "closes", "closed", "fix", "fixes", "fixed", "resolve", "resolves", "resolved",
    ] {
        let body = format!("{verb} #288");
        assert_eq!(
            find_closure_directives(&body, 288),
            vec![ClosingDirective {
                line: 1,
                verb,
                form: ReferenceForm::Bare,
            }],
            "verb {verb:?} must be detected"
        );
        // GitHub's detection is case-insensitive on the verb.
        let upper = format!("{verb} #288").to_ascii_uppercase();
        assert!(
            !find_closure_directives(&upper, 288).is_empty(),
            "{upper:?} must be detected case-insensitively"
        );
    }
}

#[test]
fn detector_requires_word_boundaries_and_matching_number() {
    // "unclosed #288": the verb is inside a word, not a directive.
    assert!(find_closure_directives("unclosed #288", 288).is_empty());
    // "fixed #5": "fix" is not a whole word here; "fixed" is, but the
    // number does not match issue 288.
    assert!(find_closure_directives("fixed #5", 288).is_empty());
    // "closed #2883" is a reference to issue 2883, not 288.
    assert!(find_closure_directives("closed #2883", 288).is_empty());
    // "This closes the size-heuristic gap": a verb with no adjacent
    // reference is prose, not a directive.
    assert!(find_closure_directives("This closes the size-heuristic gap", 288).is_empty());
    // "closes#288" with no whitespace still closes on GitHub.
    assert_eq!(
        find_closure_directives("closes#288", 288),
        vec![ClosingDirective {
            line: 1,
            verb: "closes",
            form: ReferenceForm::Bare,
        }]
    );
}

// --- Invariant 2: escaped and URL forms are safe -------------------------

#[test]
fn escaped_and_url_references_are_not_live_directives() {
    for (body, verb, expected) in [
        ("closed &#35;288", "closed", ReferenceForm::Escaped),
        ("closed &#23;288", "closed", ReferenceForm::Escaped),
        ("closed &#x23;288", "closed", ReferenceForm::Escaped),
        ("closed &#X23;288", "closed", ReferenceForm::Escaped),
        (
            "closed https://github.com/o/r/issues/288",
            "closed",
            ReferenceForm::Url,
        ),
        (
            "fixed http://github.com/o/r/issues/288.",
            "fixed",
            ReferenceForm::Url,
        ),
    ] {
        assert_eq!(
            find_closure_directives(body, 288),
            vec![ClosingDirective {
                line: 1,
                verb,
                form: expected,
            }],
            "body {body:?}"
        );
    }
}

#[test]
fn escaped_entity_requires_strict_semicolon_and_matching_number() {
    // "&#3512" is entity 3512 (or a malformed entity), not 35 followed
    // by "12": without the semicolon after 35 it is not a reference.
    assert!(find_closure_directives("closed &#3512", 288).is_empty());
    // "&#35;352" is an escaped reference to issue 352, not 288.
    assert!(find_closure_directives("closed &#35;352", 288).is_empty());
    let directives = find_closure_directives("closed &#35;288", 288);
    assert_eq!(
        directives.first().map(|d| d.form),
        Some(ReferenceForm::Escaped)
    );
}

#[test]
fn url_reference_is_issue_specific() {
    // A URL to a different issue is not a directive for 288.
    assert!(find_closure_directives("closed https://github.com/o/r/issues/2883", 288).is_empty());
    // A PR URL is not an issue reference.
    assert!(find_closure_directives("closed https://github.com/o/r/pull/288", 288).is_empty());
}

#[test]
fn safe_reference_helpers_render_the_inert_forms() {
    assert_eq!(escaped_reference(288), "&#35;288");
    assert_eq!(
        url_reference("o", "r", 288),
        "https://github.com/o/r/issues/288"
    );
    // The rendered forms round-trip through the detector as inert.
    let body = format!(
        "The earlier attempt closed {} on merge.",
        escaped_reference(288)
    );
    let directives = find_closure_directives(&body, 288);
    assert_eq!(
        directives.first().map(|d| d.form),
        Some(ReferenceForm::Escaped)
    );
    let body = format!(
        "The earlier attempt closed {} on merge.",
        url_reference("o", "r", 288)
    );
    let directives = find_closure_directives(&body, 288);
    assert_eq!(directives.first().map(|d| d.form), Some(ReferenceForm::Url));
}

// --- Invariant 4: the pre-merge lint rejects the contradiction -----------

#[test]
fn lint_rejects_live_directive_in_a_refs_body() {
    let violation = lint_refs_body(&incident_body(), 288).expect_err("must flag the incident body");
    assert_eq!(
        violation.directives,
        vec![ClosingDirective {
            line: 11,
            verb: "closed",
            form: ReferenceForm::Bare,
        }]
    );
}

#[test]
fn lint_passes_a_clean_refs_body() {
    // A body that only carries the marker, with prose that keeps the
    // verb and the reference apart.
    let clean = "Refs #288 (does not close it)\n\nWorked on closing the gap in #288 last week.\n";
    assert!(lint_refs_body(clean, 288).is_ok());
    // A body that discusses closure through safe forms passes too.
    let escaped =
        "Refs #288 (does not close it)\n\nThe earlier attempt closed &#35;288 on merge.\n";
    assert!(lint_refs_body(escaped, 288).is_ok());
}

#[test]
fn lint_ignores_bodies_the_converter_did_not_mark_refs() {
    // A body without the Refs marker is not this check's concern: a
    // prose directive there is the Closes-side rule's problem.
    let body = "The earlier attempt closed #288 and was reverted.";
    assert!(lint_refs_body(body, 288).is_ok());
}

// --- Invariant 1 (Closes side): the canonical trailer, no prose ---------

#[test]
fn pre_publish_lint_accepts_a_canonical_closes_body() {
    let body = "Summary of the change.\n\nCloses #288";
    assert!(has_closing_trailer(body, 288));
    assert!(prose_violations(body, 288).is_empty());
    assert!(pre_publish_lint(ClosureDecision::Closes, body, 288).is_ok());
}

#[test]
fn pre_publish_lint_rejects_closes_body_missing_the_trailer() {
    let body = "Summary of the change.\n\nThis wraps up the work.";
    assert!(!has_closing_trailer(body, 288));
    assert!(
        pre_publish_lint(ClosureDecision::Closes, body, 288).is_err(),
        "a Closes decision without the trailer must fail before publication"
    );
}

#[test]
fn pre_publish_lint_rejects_closes_body_with_prose_directive() {
    // The trailer is present, but line 1 also closes: a body that
    // intends a clean single closure must not scatter directives.
    let body = "The earlier attempt closed #288 and was reverted.\n\nCloses #288";
    assert!(has_closing_trailer(body, 288));
    assert_eq!(
        prose_violations(body, 288),
        vec![ClosingDirective {
            line: 1,
            verb: "closed",
            form: ReferenceForm::Bare,
        }]
    );
    assert!(pre_publish_lint(ClosureDecision::Closes, body, 288).is_err());
}

#[test]
fn prose_violations_ignores_escaped_forms() {
    let body = "The earlier attempt closed &#35;288 on merge.\n\nCloses #288";
    assert!(prose_violations(body, 288).is_empty());
    assert!(pre_publish_lint(ClosureDecision::Closes, body, 288).is_ok());
}

#[test]
fn trailer_line_must_be_exactly_verb_and_reference() {
    assert!(has_closing_trailer("Closes #288", 288));
    assert!(has_closing_trailer("close  #288", 288)); // multiple spaces are fine
    assert!(!has_closing_trailer("close #288 extra", 288));
    assert!(!has_closing_trailer("closes#288", 288)); // no space: not the canonical form
    assert!(!has_closing_trailer("Closes #2883", 288));
    assert!(!has_closing_trailer("Closes #287", 288));
    // A trailer for a different issue does not count.
    assert!(!has_closing_trailer("Closes #287", 288));
    // Empty body has no trailer.
    assert!(!has_closing_trailer("", 288));
}

// --- Invariant 3: post-merge verification ---------------------------------

#[test]
fn refs_decision_with_issue_still_open_is_consistent() {
    let check = verify_after_merge(ClosureDecision::Refs, IssueState::Open, 288);
    assert!(check.consistent());
    assert_eq!(check.action, PostMergeAction::Consistent);
}

#[test]
fn refs_decision_with_issue_closed_is_loud() {
    // The incident: the converter chose Refs, GitHub closed the issue
    // anyway. The post-merge check must fail loudly.
    let check = verify_after_merge(ClosureDecision::Refs, IssueState::Closed, 288);
    assert_eq!(check.action, PostMergeAction::ReopenIssue);
    let line = check.line();
    assert!(line.contains("FAIL"), "must fail loudly: {line}");
    assert!(line.contains("#288"), "must name the issue: {line}");
    assert!(
        line.contains("closed"),
        "must name the observed state: {line}"
    );
}

#[test]
fn closes_decision_with_issue_open_is_loud() {
    // The mirror image: the converter intended to close, but the
    // trailer was not recognized. That is tracker lag, and it must be
    // reported, not silently accepted.
    let check = verify_after_merge(ClosureDecision::Closes, IssueState::Open, 288);
    assert_eq!(check.action, PostMergeAction::ReportTrackerLag);
    assert!(!check.consistent());
}

#[test]
fn closes_decision_with_issue_closed_is_consistent() {
    let check = verify_after_merge(ClosureDecision::Closes, IssueState::Closed, 288);
    assert!(check.consistent());
}

// --- The incident end-to-end ----------------------------------------------

#[test]
fn the_incident_body_fails_the_pre_publish_gate() {
    // The converter chose Refs (the marker is on line 1) but the body
    // also carries a live closing directive in prose. The pre-merge
    // lint is the machine-detectable contradiction: it must fire.
    assert!(
        pre_publish_lint(ClosureDecision::Refs, &incident_body(), 288).is_err(),
        "pre_publish_lint must reject the Refs body with a prose directive"
    );
    let violation = lint_refs_body(&incident_body(), 288).expect_err("pre-merge lint must fire");
    assert_eq!(violation.directives.len(), 1);
    assert_eq!(violation.directives[0].line, 11);
    // Had it shipped anyway, the post-merge check would have caught
    // the closed issue and ordered a reopen.
    assert_eq!(
        verify_after_merge(ClosureDecision::Refs, IssueState::Closed, 288).action,
        PostMergeAction::ReopenIssue
    );
}
