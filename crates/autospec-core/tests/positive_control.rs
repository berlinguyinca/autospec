//! A negative search result requires a positive control (issue #4422).
//!
//! The incident: the analyst reported "zero issues in this repository declare
//! dependencies" and filed it upstream as a process defect. 133 issues declare
//! dependencies, with 287 edges. They use a `## Dependencies` heading followed
//! by a list; the analyst searched for `Depends on #N`, `Dependencies:`,
//! `Blocked by`, and `Requires` as inline text, and none of those patterns
//! matched a heading. The tool could see the corpus — the failure was the
//! query.
//!
//! The regression tests run in the configuration the bug required: a zero
//! asserted over a working population, on a guessed format, with no positive
//! control. The controls: a known-present fixture the query cannot find makes
//! the query the finding, a fixture it does find validates the query, and the
//! format and scepticism invariants are then each checked on their own.

use autospec_core::positive_control::{
    verdict, ClaimStrength, FormatBasis, NegativeClaim, PositiveControl, Verdict,
};

/// The incident claim: "0 of 183 issues declare dependencies", searched for as
/// inline text on a guessed schema, with no independent second method.
fn incident_claim() -> NegativeClaim {
    NegativeClaim::new(
        "search for `Depends on #N`, `Dependencies:`, `Blocked by`, `Requires`",
        183,
        FormatBasis::Guessed,
        ClaimStrength::Extraordinary,
        false,
    )
    .unwrap()
}

/// The known-present fixture: issue #216, which the analyst had already
/// downloaded and which declares dependencies under a `## Dependencies`
/// heading. The inline-text query does not match a heading, so it does not
/// find it.
fn fixture_not_matched() -> PositiveControl {
    PositiveControl::new(
        "issue #216, which declares dependencies under a '## Dependencies' heading",
        false,
    )
    .unwrap()
}

/// The same fixture run through a query built from reading real examples
/// first — it searches for the `## Dependencies` heading, and it finds it.
fn fixture_matched() -> PositiveControl {
    PositiveControl::new(
        "issue #216, which declares dependencies under a '## Dependencies' heading",
        true,
    )
    .unwrap()
}

#[test]
fn the_incident_zero_with_no_control_is_not_evidence_of_absence() {
    // The analyst never ran a positive control. The negative is not usable as
    // evidence of absence; the investigation must not terminate on it.
    let v = verdict(&incident_claim(), None);

    assert!(v.untrusted());
    assert!(!v.holds());
    match &v {
        Verdict::NoPositiveControl { query, population } => {
            assert_eq!(*population, 183);
            assert!(query.contains("Depends on #N"));
        }
        other => panic!("expected NoPositiveControl, got {other:?}"),
    }
    let line = v.line();
    assert!(line.starts_with("WARN:"), "{line}");
    assert!(line.contains("no positive control"), "{line}");
}

#[test]
fn a_fixture_the_query_cannot_find_makes_the_query_the_finding() {
    // The control that catches the bug: the known-present fixture the
    // inline-text query does not match. It is the query that is broken, and
    // the zero is not-matched, not absent.
    let v = verdict(&incident_claim(), Some(&fixture_not_matched()));

    assert!(v.untrusted());
    assert!(!v.holds());
    match &v {
        Verdict::QueryIsTheFinding {
            query,
            population,
            control,
        } => {
            assert_eq!(*population, 183);
            assert!(query.contains("Depends on #N"));
            assert!(control.contains("#216"));
        }
        other => panic!("expected QueryIsTheFinding, got {other:?}"),
    }
    let line = v.line();
    assert!(line.starts_with("FAIL:"), "{line}");
    assert!(line.contains("the query is the finding"), "{line}");
    assert!(line.contains("not-matched, not absent"), "{line}");
}

#[test]
fn a_fixture_the_query_finds_validates_the_query_and_exposes_the_guessed_format() {
    // The query is now trustworthy (it found the known-present fixture), so
    // the next invariant is reached: the format was guessed, not established.
    let v = verdict(&incident_claim(), Some(&fixture_matched()));

    assert!(v.untrusted());
    match &v {
        Verdict::FormatNotEstablished { .. } => {}
        other => panic!("expected FormatNotEstablished, got {other:?}"),
    }
    let line = v.line();
    assert!(line.starts_with("WARN:"), "{line}");
    assert!(line.contains("guessed format"), "{line}");
    assert!(line.contains("reports on the guess"), "{line}");
}

#[test]
fn an_established_format_on_an_ordinary_claim_holds() {
    // All three invariants satisfied: the query found its fixture, the format
    // was read from real examples, and the claim is ordinary. The zero may be
    // reported as absence.
    let claim = NegativeClaim::new(
        "search for the `## Dependencies` heading",
        183,
        FormatBasis::Established,
        ClaimStrength::Ordinary,
        false,
    )
    .unwrap();
    let v = verdict(&claim, Some(&fixture_matched()));

    assert!(v.holds());
    assert!(!v.untrusted());
    match &v {
        Verdict::Holds { query, population } => {
            assert_eq!(*population, 183);
            assert!(query.contains("## Dependencies"));
        }
        other => panic!("expected Holds, got {other:?}"),
    }
    let line = v.line();
    assert!(line.starts_with("OK:"), "{line}");
}

#[test]
fn an_extraordinary_claim_without_a_second_method_must_not_be_filed() {
    // Invariant 3: the query is validated and the format is established, but
    // "0 of 183" over a working system is extraordinary and has no second
    // method. It must not be filed yet.
    let claim = NegativeClaim::new(
        "search for the `## Dependencies` heading",
        183,
        FormatBasis::Established,
        ClaimStrength::Extraordinary,
        false,
    )
    .unwrap();
    let v = verdict(&claim, Some(&fixture_matched()));

    assert!(v.untrusted());
    match &v {
        // The variant is only reachable because the claim is extraordinary
        // and no second method was used.
        Verdict::NeedsSecondMethod { population, .. } => {
            assert_eq!(*population, 183);
        }
        other => panic!("expected NeedsSecondMethod, got {other:?}"),
    }
    let line = v.line();
    assert!(line.starts_with("WARN:"), "{line}");
    assert!(line.contains("extraordinary claim"), "{line}");
    assert!(line.contains("no second method"), "{line}");
}

#[test]
fn an_extraordinary_claim_backed_by_a_second_method_holds() {
    // The same extraordinary claim, but produced by an independent second
    // method (e.g. reading the raw issues, not just searching). Now it holds.
    let claim = NegativeClaim::new(
        "search for the `## Dependencies` heading",
        183,
        FormatBasis::Established,
        ClaimStrength::Extraordinary,
        true,
    )
    .unwrap();
    let v = verdict(&claim, Some(&fixture_matched()));
    assert!(v.holds());
}

#[test]
fn a_broken_query_dominates_every_other_invariant() {
    // Even with an established format and a second method, a query that cannot
    // find its known-present fixture is the finding: the zero is void, and the
    // other invariants are not reached. This is the incident's true cause.
    let claim = NegativeClaim::new(
        "search for `Depends on #N`, `Blocked by`, `Requires`",
        183,
        FormatBasis::Established,
        ClaimStrength::Ordinary,
        true,
    )
    .unwrap();
    let v = verdict(&claim, Some(&fixture_not_matched()));
    assert!(matches!(v, Verdict::QueryIsTheFinding { .. }));
    assert!(!v.holds());
}

#[test]
fn only_a_hold_is_reportable_as_zero() {
    // The tooling contract: a negative may be reported as absence only when it
    // holds. A parser that cannot find the example in its own test data must
    // fail rather than report zero.
    let not_matched = verdict(&incident_claim(), Some(&fixture_not_matched()));
    let no_control = verdict(&incident_claim(), None);
    assert!(
        !not_matched.holds(),
        "a broken query must not report the zero"
    );
    assert!(
        !no_control.holds(),
        "an uncontrolled negative must not report the zero"
    );

    let holds = verdict(
        &NegativeClaim::new(
            "q",
            10,
            FormatBasis::Established,
            ClaimStrength::Ordinary,
            false,
        )
        .unwrap(),
        Some(&fixture_matched()),
    );
    assert!(
        holds.holds(),
        "a validated, established, ordinary negative reports the zero"
    );
}

#[test]
fn the_control_line_says_which_way_the_fixture_went() {
    assert!(
        fixture_matched().line().starts_with("positive control OK:"),
        "{}",
        fixture_matched().line()
    );
    assert!(
        fixture_not_matched().line().starts_with("FAIL:"),
        "{}",
        fixture_not_matched().line()
    );
    assert!(
        fixture_not_matched()
            .line()
            .contains("the query is the finding"),
        "{}",
        fixture_not_matched().line()
    );
}

#[test]
fn the_constructors_reject_placeholders() {
    // A fixture that names nothing, and a claim with no named query, are
    // placeholders, not records.
    assert!(PositiveControl::new("", true).is_none());
    assert!(PositiveControl::new("   ", false).is_none());
    assert!(NegativeClaim::new(
        "",
        10,
        FormatBasis::Established,
        ClaimStrength::Ordinary,
        false
    )
    .is_none());
    assert!(NegativeClaim::new(
        "   ",
        10,
        FormatBasis::Established,
        ClaimStrength::Ordinary,
        false
    )
    .is_none());
}

#[test]
fn the_labels_name_the_basis_and_the_strength() {
    assert_eq!(FormatBasis::Established.label(), "established");
    assert_eq!(FormatBasis::Guessed.label(), "guessed");
    assert_eq!(ClaimStrength::Ordinary.label(), "ordinary");
    assert_eq!(ClaimStrength::Extraordinary.label(), "extraordinary");
}

#[test]
fn the_incident_end_to_end_from_wrong_issue_to_void_zero() {
    // Reconstructed end to end: the analyst's zero with no control, the
    // known-present fixture the query never matched, and the query that was
    // built from reading real examples and does find it.
    let before = verdict(&incident_claim(), None);
    assert!(before.untrusted());
    assert!(matches!(before, Verdict::NoPositiveControl { .. }));

    let control_run = verdict(&incident_claim(), Some(&fixture_not_matched()));
    assert!(control_run.untrusted());
    assert!(matches!(control_run, Verdict::QueryIsTheFinding { .. }));
    assert!(
        control_run.line().contains("not-matched, not absent"),
        "{}",
        control_run.line()
    );

    // The remedied analysis: the query is built from the format read in issue
    // #216, it finds the fixture, the format is established, and the claim is
    // ordinary. Only now is the zero reportable — and, for 133 real issues,
    // it is not zero at all.
    let remedied = NegativeClaim::new(
        "search for the `## Dependencies` heading",
        183,
        FormatBasis::Established,
        ClaimStrength::Ordinary,
        false,
    )
    .unwrap();
    assert!(verdict(&remedied, Some(&fixture_matched())).holds());
}
