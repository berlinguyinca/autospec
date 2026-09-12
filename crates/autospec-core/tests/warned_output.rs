//! A tool that warns on stderr and still writes a result to stdout is a
//! tool whose exit status must be checked (issue #4396).
//!
//! The incident: `comm -23 a b` computed "patches that have not been
//! attempted". Both inputs had been produced with `sort -un` — numerically
//! sorted — and `comm` requires lexically sorted input. It printed
//! `comm: file 1 is not in sorted order` to stderr and still emitted a
//! result: 588 "fresh patches" when the true answer was 164. Its exit status
//! was not checked, so the 3.6x overstatement was nearly reported as the
//! size of the backlog. The same class of error had already reported
//! "121 patches awaiting conversion" where the real number was 11.
//!
//! The regression tests run in the configuration the bug required: numeric
//! output fed to a lexical set operation, a stderr warning with no status
//! check, a reported number with no sanity bound, and a spec question with
//! no named filter.

use autospec_core::warned_output::{
    check_order, BacklogQuestion, Collation, CountBound, Filter, OrderConsumer, OrderVerdict,
    SetOp, Step, StepOutcome,
};

/// The incident: `sort -un` output fed to `comm -23`.
fn incident_op() -> SetOp {
    SetOp {
        producer: "sort -un".into(),
        produced: Collation::Numeric,
        consumer: OrderConsumer::Comm,
    }
}

#[test]
fn the_incident_numeric_output_is_not_the_order_comm_requires() {
    let verdict = check_order(&incident_op());

    match &verdict {
        OrderVerdict::Unordered {
            producer,
            produced,
            consumer,
            required,
        } => {
            assert_eq!(producer, "sort -un");
            assert_eq!(*produced, Collation::Numeric);
            assert_eq!(consumer, "comm");
            assert_eq!(*required, Collation::Lexical);
        }
        other => panic!("expected Unordered, got {other:?}"),
    }

    let line = verdict.line();
    assert!(line.starts_with("FAIL:"), "{line}");
    assert!(line.contains("sort -un"), "{line}");
    assert!(line.contains("comm"), "{line}");
    assert!(line.contains("numeric"), "{line}");
    assert!(line.contains("lexicographic"), "{line}");
    // Both remedies the invariant names.
    assert!(
        line.contains("sort defensively at the point of use"),
        "{line}"
    );
    assert!(line.contains("language with real sets"), "{line}");
}

#[test]
fn the_remedy_sorting_defensively_at_the_point_of_use_is_safe() {
    // The same consumer, but the input is re-sorted at the point of use
    // with the exact collation the consumer requires — never relying on how
    // it was produced.
    let op = SetOp {
        producer: "LC_ALL=C sort".into(),
        produced: Collation::Lexical,
        consumer: OrderConsumer::Comm,
    };
    let verdict = check_order(&op);
    match &verdict {
        OrderVerdict::Safe {
            producer,
            consumer,
            collation,
        } => {
            assert_eq!(producer, "LC_ALL=C sort");
            assert_eq!(consumer, "comm");
            assert_eq!(*collation, Collation::Lexical);
        }
        other => panic!("expected Safe, got {other:?}"),
    }
    let line = verdict.line();
    assert!(line.starts_with("OK:"), "{line}");
    assert!(!line.contains("FAIL"), "{line}");
}

#[test]
fn the_incident_a_warned_step_with_an_unchecked_status_is_a_confident_wrong_answer() {
    // `comm -23` warned on stderr and still wrote 588 "fresh patches"; the
    // stdout was piped onward into a reported number without the exit status
    // being checked.
    let step = Step {
        tool: "comm -23".into(),
        warned: true,
        status_checked: false,
    };

    assert_eq!(step.outcome(), StepOutcome::ConfidentWrongAnswer);
    let line = step.line();
    assert!(line.starts_with("FAIL:"), "{line}");
    assert!(line.contains("comm -23"), "{line}");
    assert!(line.contains("warned on stderr"), "{line}");
    assert!(line.contains("exit status was not checked"), "{line}");
    // The rule: this is strictly worse than a crash, and the line says why.
    assert!(line.contains("strictly worse than a crash"), "{line}");
    assert!(line.contains("nothing downstream can tell"), "{line}");
}

#[test]
fn the_same_warning_with_a_checked_status_is_detected_not_used() {
    let step = Step {
        tool: "comm -23".into(),
        warned: true,
        status_checked: true,
    };

    assert_eq!(step.outcome(), StepOutcome::Detected);
    let line = step.line();
    assert!(line.starts_with("OK:"), "{line}");
    assert!(line.contains("detected"), "{line}");
    assert!(line.contains("not used"), "{line}");
}

#[test]
fn a_clean_step_may_be_piped_onward() {
    let step = Step {
        tool: "wc -l".into(),
        warned: false,
        status_checked: false,
    };

    assert_eq!(step.outcome(), StepOutcome::Clean);
    let line = step.line();
    assert!(line.starts_with("OK:"), "{line}");
    assert!(line.contains("warned nothing"), "{line}");
}

#[test]
fn the_sanity_assertion_that_would_have_caught_the_incident() {
    // "fresh cannot exceed total patches": the reported 588 against a total
    // of 588. Equality is the giveaway that the filter did not run.
    let bound = CountBound {
        name: "fresh patches".into(),
        count: 588,
        total: 588,
    };

    assert!(!bound.impossible());
    assert!(bound.unfiltered());
    let line = bound.line();
    assert!(line.starts_with("WARN:"), "{line}");
    assert!(line.contains("equals the total 588"), "{line}");
    assert!(line.contains("filter may not have run"), "{line}");

    // The true answer holds: 164 fresh out of 588 total.
    let true_bound = CountBound {
        name: "fresh patches".into(),
        count: 164,
        total: 588,
    };
    assert!(!true_bound.impossible());
    assert!(!true_bound.unfiltered());
    let line = true_bound.line();
    assert!(line.starts_with("OK:"), "{line}");
}

#[test]
fn the_earlier_incident_121_awaiting_where_the_real_number_was_11() {
    // The same class of error, a month earlier: "121 patches awaiting
    // conversion" where the real number was 11. Had the bound been asserted,
    // 121 against 11 is impossible — the number was not measured.
    let bound = CountBound {
        name: "patches awaiting conversion".into(),
        count: 121,
        total: 11,
    };

    assert!(bound.impossible());
    let line = bound.line();
    assert!(line.starts_with("FAIL:"), "{line}");
    assert!(line.contains("exceeds the total 11"), "{line}");
    assert!(line.contains("not measured"), "{line}");
}

#[test]
fn three_filters_three_numbers_one_question() {
    // Over the same corpus of 588 patches, the three filters are three
    // different answers to "how much work is left": 588, 164, 75.
    assert_eq!(Filter::HasPatch.label(), "has a patch");
    assert_eq!(Filter::NoBranchOrPr.label(), "has no branch or PR");
    assert_eq!(
        Filter::NoBranchOrPrAndOpen.label(),
        "has no branch or PR and the issue is still open"
    );

    let with_filter = BacklogQuestion {
        filter: Some(Filter::NoBranchOrPr),
    };
    assert!(with_filter.finding().is_none());
    let line = with_filter.report(164);
    assert_eq!(line, "164 outstanding (has no branch or PR)");

    // The spec that asked "how many fresh patches are there?" without naming
    // the filter is the finding: 75 reported as 588 is an 8x misstatement
    // of the remaining work, not a rounding error.
    let unnamed = BacklogQuestion { filter: None };
    let finding = unnamed.finding().expect("an unnamed filter is a finding");
    assert!(finding.contains("must name its filter"), "{finding}");
    assert!(finding.contains("three different numbers"), "{finding}");
    let line = unnamed.report(588);
    assert_eq!(line, "588 outstanding (filter unnamed — not a number)");
}
