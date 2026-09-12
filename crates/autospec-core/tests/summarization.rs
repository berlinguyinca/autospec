//! Summarization against a reasoning model (issue #4350).
//!
//! The regression tests reconstruct the measurements from the issue:
//! reasoning overhead is roughly fixed (~60-90 completion tokens) and is
//! drawn from the same `max_tokens` budget as the answer, so a cap sized
//! for the summary is consumed by chain-of-thought and the answer comes
//! back truncated or empty with `finish_reason: length`. Suppressing
//! reasoning produces the same answer in 9 completion tokens instead of 41.

use autospec_core::summarization::{
    budget_max_tokens, classify_response, plan_request, reasoning_control, FinishReason,
    ReasoningControl, RequestKind, ResponseVerdict, TransformationKind, UsageSplit,
};

// ---------------------------------------------------------------------------
// Invariant 1: a transformation of provided text disables reasoning
// ---------------------------------------------------------------------------

/// The incident's request: a one-sentence summary. Its output is a
/// transformation of provided text, so chain-of-thought only competes for
/// the budget and must be disabled.
#[test]
fn transformation_requests_disable_reasoning() {
    for kind in [
        TransformationKind::Summarization,
        TransformationKind::Compaction,
        TransformationKind::Extraction,
        TransformationKind::Reformatting,
    ] {
        let request = RequestKind::Transformation(kind);
        assert!(!request.reasoning_allowed(), "{kind:?} must not reason");
        assert_eq!(
            reasoning_control(request),
            Some(ReasoningControl::EnableThinkingOff),
            "{kind:?} must carry the disable control"
        );
    }
}

/// A request whose output goes beyond the provided text keeps reasoning:
/// the invariant only forces reasoning off for transformations.
#[test]
fn generation_requests_keep_reasoning() {
    let request = RequestKind::Generation;
    assert!(request.reasoning_allowed());
    assert_eq!(reasoning_control(request), None);
}

/// The two measured controls render as exactly the wire fields the gateway
/// accepts.
#[test]
fn reasoning_controls_render_the_measured_wire_fields() {
    let value = ReasoningControl::EnableThinkingOff.to_json();
    assert_eq!(
        value["chat_template_kwargs"]["enable_thinking"],
        serde_json::Value::Bool(false)
    );
    assert_eq!(
        ReasoningControl::EffortNone.to_json()["reasoning_effort"],
        serde_json::Value::String("none".to_string())
    );
}

// ---------------------------------------------------------------------------
// Invariant 2: max_tokens is reasoning + answer whenever reasoning is on
// ---------------------------------------------------------------------------

/// The measured baseline: a generation request with reasoning on must
/// budget the cap as reasoning + answer. Sizing it for the answer alone is
/// exactly what produced `finish_reason: length` at `max_tokens` 64.
#[test]
fn max_tokens_bundles_reasoning_and_answer_when_reasoning_is_on() {
    assert_eq!(
        budget_max_tokens(RequestKind::Generation, 41, 128),
        41 + 128
    );
}

/// A transformation request carries no reasoning, so the cap is the answer
/// alone: nothing competes with the summary for it.
#[test]
fn max_tokens_is_the_answer_alone_when_reasoning_is_off() {
    assert_eq!(
        budget_max_tokens(
            RequestKind::Transformation(TransformationKind::Summarization),
            9,
            128
        ),
        9
    );
}

/// The combined fix: a summarization plan sizes the cap for the answer and
/// carries the disable control, so the truncation failure mode disappears.
#[test]
fn plan_request_pairs_the_cap_with_the_control() {
    let plan = plan_request(
        RequestKind::Transformation(TransformationKind::Summarization),
        9,
        128,
    );
    assert_eq!(plan.max_tokens, 9);
    assert_eq!(plan.reasoning, Some(ReasoningControl::EnableThinkingOff));

    let plan = plan_request(RequestKind::Generation, 41, 128);
    assert_eq!(plan.max_tokens, 41 + 128);
    assert_eq!(plan.reasoning, None);
}

// ---------------------------------------------------------------------------
// Invariant 3: finish_reason length with empty content is an error
// ---------------------------------------------------------------------------

/// The silent failure: the whole budget was deliberation, `content` came
/// back empty, and a caller checking only `content` sees a successful
/// response with nothing in it. The verdict must be an error.
#[test]
fn length_with_empty_content_is_an_error() {
    let verdict = classify_response(FinishReason::Length, "");
    assert_eq!(verdict, ResponseVerdict::TruncatedEmpty);
    assert!(verdict.is_error());
}

/// A whitespace-only answer is nothing at all: it is the same error, not a
/// partial response.
#[test]
fn length_with_whitespace_only_content_is_an_error() {
    assert_eq!(
        classify_response(FinishReason::Length, "   \n  "),
        ResponseVerdict::TruncatedEmpty
    );
    assert!(classify_response(FinishReason::Length, " \n").is_error());
}

/// A cap hit with a partial answer is a truncation, not an error: the
/// caller may use the partial answer with the truncation on record.
#[test]
fn length_with_partial_content_is_a_truncation() {
    let verdict = classify_response(FinishReason::Length, "The cat rested on the");
    assert_eq!(verdict, ResponseVerdict::Truncated);
    assert!(!verdict.is_error());
}

/// The model that finishes on its own is a complete response, even when it
/// says nothing: the invariant only turns `length` + empty into an error.
#[test]
fn stop_is_complete_regardless_of_content() {
    assert_eq!(
        classify_response(FinishReason::Stop, ""),
        ResponseVerdict::Complete
    );
    assert_eq!(
        classify_response(FinishReason::Stop, "A cat is sitting on a mat."),
        ResponseVerdict::Complete
    );
    assert!(!classify_response(FinishReason::Stop, "").is_error());
}

/// The wire values round-trip; anything else is rejected, never guessed.
#[test]
fn finish_reason_parses_the_wire_values() {
    assert_eq!(FinishReason::parse("stop"), Some(FinishReason::Stop));
    assert_eq!(FinishReason::parse("length"), Some(FinishReason::Length));
    assert_eq!(FinishReason::parse("tool_calls"), None);
    assert_eq!(FinishReason::Stop.as_str(), "stop");
    assert_eq!(FinishReason::Length.as_str(), "length");
}

// ---------------------------------------------------------------------------
// Invariant 4: telemetry records the reasoning/content split
// ---------------------------------------------------------------------------

/// The measured truncated run: 64 completion tokens of which 56 were
/// reasoning and 8 were the truncated answer. The record carries the split,
/// so the overhead is visible without measuring by hand against a worker.
#[test]
fn usage_split_records_reasoning_separately() {
    let split = UsageSplit::from_usage(120, 64, Some(56)).expect("split within the total");
    assert_eq!(split.prompt_tokens, 120);
    assert_eq!(split.completion_tokens, 64);
    assert_eq!(split.reasoning_tokens, 56);
    assert_eq!(split.content_tokens, 8);
    assert!(split.split_reported);
    let line = split.line();
    assert!(line.contains("reasoning=56"), "{line}");
    assert!(line.contains("content=8"), "{line}");
}

/// A server that does not report the split: the record says so explicitly
/// instead of reading zero reasoning as "no reasoning happened".
#[test]
fn usage_split_without_server_report_says_unreported() {
    let split = UsageSplit::from_usage(120, 64, None).expect("no split reported");
    assert_eq!(split.reasoning_tokens, 0);
    assert_eq!(split.content_tokens, 0);
    assert!(!split.split_reported);
    let line = split.line();
    assert!(line.contains("unreported"), "{line}");
}

/// A split that exceeds its total is malformed telemetry: reject it, never
/// invent a subtraction.
#[test]
fn usage_split_rejects_reasoning_above_completion() {
    assert!(UsageSplit::from_usage(120, 64, Some(65)).is_err());
    // Exactly the total is the boundary: all reasoning, no content.
    let split = UsageSplit::from_usage(120, 64, Some(64)).expect("boundary split");
    assert_eq!(split.reasoning_tokens, 64);
    assert_eq!(split.content_tokens, 0);
}
