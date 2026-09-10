//! AC4 for issue #4088: the three signal-semantics cases.
//!
//! a) an incomplete consumed-signal declaration is rejected, naming the
//!    missing field;
//! b) a complete declaration passes;
//! c) a rejection that cites a signal without its healthy-population value
//!    is refused, naming the signal and the missing check.

use autospec_core::spec::{
    lint_consumed_signals, validate_rejection, CitedSignal, Rejection,
    CONSUMED_SIGNAL_INCOMPLETE_RULE_ID,
};

#[test]
fn incomplete_consumed_signal_is_rejected_naming_the_missing_field() {
    let spec = "# Release gate\n\n## Consumed Signals\n\n### `ci_failures`\n- Healthy value: 0 on green main\n- Produced under: suite green at HEAD\n";
    let findings = lint_consumed_signals(spec);

    assert_eq!(findings.len(), 1, "exactly one declaration is missing");
    let finding = &findings[0];
    assert_eq!(finding.rule_id(), CONSUMED_SIGNAL_INCOMPLETE_RULE_ID);
    assert_eq!(finding.signal, "ci_failures");
    assert_eq!(finding.missing_field, "Empty/absent means:");
    assert!(
        finding.message().contains("Empty/absent means:"),
        "the rejection must name the missing check: {}",
        finding.message()
    );
}

#[test]
fn complete_consumed_signal_declaration_passes() {
    let spec = "# Release gate\n\n## Consumed Signals\n\n### `ci_failures`\n- Empty/absent means: the suite did not run\n- Healthy value: 0 on green main\n- Produced under: suite green at HEAD\n";
    assert!(
        lint_consumed_signals(spec).is_empty(),
        "a signal with all three declarations must not be flagged"
    );
}

#[test]
fn rejection_citing_unvalidated_signal_is_refused_naming_the_check() {
    let rejection = Rejection {
        target: "patch 3".into(),
        cited_signals: vec![CitedSignal {
            name: "ci_failures".into(),
            healthy_population_value: None,
        }],
    };

    let refusal = validate_rejection(&rejection)
        .expect_err("a rejection citing a signal whose base rate was never checked must not stand");
    assert_eq!(refusal.signal, "ci_failures");
    assert_eq!(refusal.missing_check, "healthy-population value");
    let message = refusal.message();
    assert!(message.contains("ci_failures"));
    assert!(message.contains("healthy-population value"));
}

#[test]
fn rejection_citing_validated_signal_stands() {
    let rejection = Rejection {
        target: "issue 4102".into(),
        cited_signals: vec![CitedSignal {
            name: "ci_failures".into(),
            healthy_population_value: Some("0 on green main".into()),
        }],
    };
    assert!(validate_rejection(&rejection).is_ok());
}
