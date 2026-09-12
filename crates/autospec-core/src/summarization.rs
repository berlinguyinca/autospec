//! Summarization against a reasoning model (issue #4350).
//!
//! Measured against the live edge gateway (`qwen3.8-flash-next`, one-sentence
//! summary prompt): reasoning overhead is roughly fixed (~60-90 completion
//! tokens) and is drawn from the same `max_tokens` budget as the answer. A
//! caller who sizes the cap for the summary — the natural thing to do, since
//! that is what it wants back — has it consumed by chain-of-thought and
//! receives a truncated or empty `content` with `finish_reason: length`.
//!
//! Suppressing reasoning produces the same answer for a fraction of the
//! budget (9 completion tokens instead of 41, 0 reasoning instead of 122
//! characters), and the truncation failure mode disappears because nothing
//! competes with the summary for the cap. Summarization is the clearest case
//! for it: compaction wants a faithful condensation, not deliberation, and
//! the deliberation is exactly what breaks it.
//!
//! Four invariants, each a checkable primitive here:
//!
//! 1. **A request whose output is a transformation of provided text should
//!    disable reasoning.** [`RequestKind::reasoning_allowed`] is false for
//!    summarization, compaction, extraction and reformatting — the input
//!    already contains the content, so chain-of-thought only competes for
//!    the budget — and [`reasoning_control`] names the wire field that turns
//!    it off.
//! 2. **`max_tokens` must be budgeted as reasoning + answer whenever
//!    reasoning is on.** [`budget_max_tokens`] sizes the cap accordingly.
//!    Sizing it for the answer alone guarantees truncation on a reasoning
//!    model, and the failure is silent — a 200 response with empty
//!    `content`.
//! 3. **Treat `finish_reason: length` with empty `content` as an error, not
//!    a response.** [`classify_response`] keeps the three outcomes a caller
//!    must not collapse: a caller that checks only HTTP status and `content`
//!    cannot distinguish "the model said nothing" from "the model was cut
//!    off mid-thought".
//! 4. **Record reasoning tokens separately in telemetry.**
//!    [`UsageSplit::from_usage`] carries the reasoning/content split when
//!    the server reports it and says so explicitly when it does not — a
//!    record without the split must never read as "no reasoning happened".
//!
//! Everything here is pure in-memory state — no I/O, no clock, no
//! subprocess.

use serde_json::{json, Value};

/// A kind of work whose output is a transformation of provided text.
///
/// The input already contains the content; chain-of-thought only competes
/// for the budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransformationKind {
    Summarization,
    Compaction,
    Extraction,
    Reformatting,
}

/// One generation request, classified by what its output is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestKind {
    /// Output is a transformation of provided text.
    Transformation(TransformationKind),
    /// Output goes beyond the provided text (planning, generation, ...).
    Generation,
}

impl RequestKind {
    /// Invariant 1: reasoning is allowed only when the output is not a
    /// transformation of provided text.
    pub fn reasoning_allowed(self) -> bool {
        matches!(self, RequestKind::Generation)
    }
}

/// The wire control that disables a reasoning model's chain-of-thought.
///
/// Both were measured equivalent against the edge gateway: the same answer
/// came back in 9 completion tokens with 0 reasoning instead of 41 with 122
/// reasoning characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReasoningControl {
    /// `"chat_template_kwargs": {"enable_thinking": false}`
    EnableThinkingOff,
    /// `"reasoning_effort": "none"`
    EffortNone,
}

impl ReasoningControl {
    /// The request fields this control adds, ready to merge into a
    /// chat-completions body.
    pub fn to_json(self) -> Value {
        match self {
            Self::EnableThinkingOff => json!({
                "chat_template_kwargs": {"enable_thinking": false}
            }),
            Self::EffortNone => json!({"reasoning_effort": "none"}),
        }
    }
}

/// Invariant 1: the control a request of `kind` must carry. `None` means
/// "reasoning allowed, carry no control"; transformation requests force
/// reasoning off, and the `enable_thinking` control is the one the serving
/// path measures against.
pub fn reasoning_control(kind: RequestKind) -> Option<ReasoningControl> {
    (!kind.reasoning_allowed()).then_some(ReasoningControl::EnableThinkingOff)
}

/// Invariant 2: budget `max_tokens` for one request.
///
/// With reasoning on the cap must cover reasoning + answer; with reasoning
/// off it is the answer. Sizing it for the answer alone while reasoning is
/// on guarantees truncation on a reasoning model, and the failure is silent
/// — a 200 response with empty `content`.
pub fn budget_max_tokens(kind: RequestKind, answer_tokens: u32, reasoning_reserve: u32) -> u32 {
    if kind.reasoning_allowed() {
        answer_tokens.saturating_add(reasoning_reserve)
    } else {
        answer_tokens
    }
}

/// The cap to send and the control to carry for one request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestPlan {
    pub max_tokens: u32,
    pub reasoning: Option<ReasoningControl>,
}

/// The full fix for one request: a transformation gets a cap sized for the
/// answer alone and the control that removes the competition; a generation
/// gets the reasoning + answer cap and no control.
pub fn plan_request(kind: RequestKind, answer_tokens: u32, reasoning_reserve: u32) -> RequestPlan {
    RequestPlan {
        max_tokens: budget_max_tokens(kind, answer_tokens, reasoning_reserve),
        reasoning: reasoning_control(kind),
    }
}

/// The `finish_reason` a generation ends with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinishReason {
    /// The model finished on its own.
    Stop,
    /// The generation hit `max_tokens`.
    Length,
}

impl FinishReason {
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "stop" => Self::Stop,
            "length" => Self::Length,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stop => "stop",
            Self::Length => "length",
        }
    }
}

/// What a caller must do with one finished generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseVerdict {
    /// Finished on its own; the content is the answer.
    Complete,
    /// Hit the cap with a partial answer: usable only with the truncation on
    /// record, never as a success.
    Truncated,
    /// Hit the cap with no content at all: an error, not a response. A
    /// caller that checks only `content` sees "a successful response with
    /// nothing in it"; this verdict is what separates it.
    TruncatedEmpty,
}

impl ResponseVerdict {
    /// True when the caller must treat the response as a failure.
    pub fn is_error(self) -> bool {
        matches!(self, Self::TruncatedEmpty)
    }
}

/// Invariant 3: classify one finished generation.
///
/// `content` is the answer field. A reasoning model may return empty
/// `content` with populated reasoning: with `finish_reason: length` that
/// means the answer never started — the whole budget was deliberation — and
/// the caller must fail, not accept an empty answer.
pub fn classify_response(finish: FinishReason, content: &str) -> ResponseVerdict {
    match finish {
        FinishReason::Stop => ResponseVerdict::Complete,
        FinishReason::Length if content.trim().is_empty() => ResponseVerdict::TruncatedEmpty,
        FinishReason::Length => ResponseVerdict::Truncated,
    }
}

/// One usage record with the reasoning/content split (invariant 4).
///
/// `completion_tokens` alone was invisible: a cap sized for the answer
/// looked like an expensive answer and was really chain-of-thought, so the
/// failure had to be measured by hand against a live worker. When the server
/// does not report the split, `split_reported` is false and both split
/// fields are zero — the zeros must never be read as "no reasoning
/// happened".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub struct UsageSplit {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    /// Reasoning tokens; 0 when `split_reported` is false.
    pub reasoning_tokens: u64,
    /// Answer tokens; 0 when `split_reported` is false.
    pub content_tokens: u64,
    /// False when the server did not report the reasoning/content split.
    pub split_reported: bool,
}

impl UsageSplit {
    /// Build the record from wire usage. `reported_reasoning_tokens` is
    /// `usage.completion_tokens_details.reasoning_tokens` when the server
    /// carries it.
    pub fn from_usage(
        prompt_tokens: u64,
        completion_tokens: u64,
        reported_reasoning_tokens: Option<u64>,
    ) -> Result<Self, String> {
        let (reasoning_tokens, content_tokens, split_reported) = match reported_reasoning_tokens {
            Some(reasoning) => {
                if reasoning > completion_tokens {
                    return Err(format!(
                        "reported reasoning_tokens {reasoning} exceed \
                             completion_tokens {completion_tokens}"
                    ));
                }
                (reasoning, completion_tokens - reasoning, true)
            }
            None => (0, 0, false),
        };
        Ok(Self {
            prompt_tokens,
            completion_tokens,
            reasoning_tokens,
            content_tokens,
            split_reported,
        })
    }

    /// One telemetry line: the split when reported, an explicit "unreported"
    /// otherwise — so a missing split is visible instead of silent.
    pub fn line(&self) -> String {
        if self.split_reported {
            format!(
                "usage: prompt={} completion={} (reasoning={} content={})",
                self.prompt_tokens,
                self.completion_tokens,
                self.reasoning_tokens,
                self.content_tokens
            )
        } else {
            format!(
                "usage: prompt={} completion={} (reasoning/content split unreported)",
                self.prompt_tokens, self.completion_tokens
            )
        }
    }
}
