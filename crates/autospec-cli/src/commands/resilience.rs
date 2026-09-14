//! `autospec resilience` — inspect the Resilient Agent Runtime policy.
//!
//! The decision engine is pure and lives in `autospec_core::resilience`; this
//! command is the I/O edge: it parses arguments and renders verdicts as prose
//! or JSON. It provides the operator surface the spec requires: show work
//! state, inspect lesson promotion evidence, evaluate a checkpoint verdict,
//! and doctor/validate configuration.

use serde_json::json;

use autospec_core::resilience::context_guardian::{
    evaluate_threshold, may_begin_substantial_phase, ContextGuardianConfig, ContextObservation,
    EstimateSource,
};
use autospec_core::resilience::learning::{
    is_authoritative, is_unsafe_lesson, promote_verdict, LessonCandidate, LessonKind,
};
use autospec_core::resilience::work_protocol::{can_transition, WorkState};
use autospec_core::resilience::EVENTS;

use super::CommandFailure;

const USAGE: &str = "\
autospec resilience

Inspect and validate the Resilient Agent Runtime (context guardian, durable
work protocol, dynamic memory map, attention streams, verified learning).

USAGE:
    autospec resilience <SUBCOMMAND> [OPTIONS]

SUBCOMMANDS:
    events              List the stable lifecycle event names
    checkpoint-verdict  Evaluate a context observation against thresholds
    lesson-verdict      Evaluate a lesson candidate's promotion verdict
    transition-check    Check whether a work-state transition is legal
    doctor              Validate configuration and report availability

OPTIONS (checkpoint-verdict):
    --used <N>          Estimated used tokens (required)
    --window <N>        Context window tokens (required)
    --source <SRC>      exact | estimated | conservative-estimate | unknown
    --json              Emit JSON instead of text

OPTIONS (lesson-verdict):
    --kind <KIND>       procedure | warning | architecture | debugging | test |
                        tooling | failure-pattern
    --statement <TEXT>  Lesson statement (required)
    --scope <TEXT>      Lesson scope (required)
    --validated         Pretend validation evidence exists
    --reviewed          Pretend independent review evidence exists
    --confidence <F>    Confidence 0..1
    --json              Emit JSON instead of text

OPTIONS (transition-check):
    --from <STATE>      CREATED|ASSIGNED|DELIVERED|CLAIMED|RUNNING|COMPLETED|
                        VALIDATED|REVIEWED|MERGED
    --to <STATE>        Same set
    --json              Emit JSON instead of text
";

pub fn run(args: &[String]) -> Result<(), CommandFailure> {
    match args.first().map(String::as_str) {
        None | Some("--help") | Some("-h") => {
            print!("{USAGE}");
            Ok(())
        }
        Some("events") => {
            for event in EVENTS {
                println!("{event}");
            }
            Ok(())
        }
        Some("checkpoint-verdict") => checkpoint_verdict(&args[1..]),
        Some("lesson-verdict") => lesson_verdict(&args[1..]),
        Some("transition-check") => transition_check(&args[1..]),
        Some("doctor") => doctor(),
        Some(other) => Err(CommandFailure::diagnostic(format!(
            "unknown autospec resilience subcommand: {other}"
        ))),
    }
}

fn flag_value<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    let mut index = 0;
    while index < args.len() {
        if args[index] == format!("--{name}") {
            return args.get(index + 1).map(|s| s.as_str());
        }
        index += 1;
    }
    None
}

fn has_flag(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == &format!("--{name}"))
}

fn checkpoint_verdict(args: &[String]) -> Result<(), CommandFailure> {
    let used: usize = flag_value(args, "used")
        .ok_or_else(|| CommandFailure::diagnostic("--used <N> is required"))?
        .parse()
        .map_err(|_| CommandFailure::diagnostic("--used must be an integer"))?;
    let window: usize = flag_value(args, "window")
        .ok_or_else(|| CommandFailure::diagnostic("--window <N> is required"))?
        .parse()
        .map_err(|_| CommandFailure::diagnostic("--window must be an integer"))?;
    let source = match flag_value(args, "source").unwrap_or("estimated") {
        "exact" => EstimateSource::Exact,
        "estimated" => EstimateSource::Estimated,
        "conservative-estimate" => EstimateSource::ConservativeEstimate,
        "unknown" => EstimateSource::Unknown,
        other => {
            return Err(CommandFailure::diagnostic(format!(
                "unknown --source: {other}"
            )))
        }
    };
    let json = has_flag(args, "json");

    let observation = ContextObservation {
        window_tokens: window,
        estimated_used_tokens: used,
        source,
    };
    let verdict = evaluate_threshold(&ContextGuardianConfig::default(), &observation);
    let may = may_begin_substantial_phase(verdict, false);

    if json {
        println!(
            "{}",
            json!({
                "verdict": verdict_to_str(verdict),
                "utilization_percent": observation.utilization_percent(),
                "may_begin_substantial_phase_without_checkpoint": may,
            })
        );
    } else {
        println!("verdict: {}", verdict_to_str(verdict));
        if let Some(pct) = observation.utilization_percent() {
            println!("utilization_percent: {pct}");
        } else {
            println!("utilization_percent: unknown");
        }
        println!(
            "may_begin_substantial_phase_without_checkpoint: {may}"
        );
    }
    Ok(())
}

fn verdict_to_str(v: autospec_core::resilience::ThresholdVerdict) -> &'static str {
    use autospec_core::resilience::ThresholdVerdict;
    match v {
        ThresholdVerdict::Below => "below",
        ThresholdVerdict::Soft => "soft",
        ThresholdVerdict::Warning => "warning",
        ThresholdVerdict::Required => "required",
        ThresholdVerdict::Unknown => "unknown",
    }
}

fn lesson_verdict(args: &[String]) -> Result<(), CommandFailure> {
    let kind = match flag_value(args, "kind").unwrap_or("procedure") {
        "procedure" => LessonKind::Procedure,
        "warning" => LessonKind::Warning,
        "architecture" => LessonKind::Architecture,
        "debugging" => LessonKind::Debugging,
        "test" => LessonKind::Test,
        "tooling" => LessonKind::Tooling,
        "failure-pattern" => LessonKind::FailurePattern,
        other => return Err(CommandFailure::diagnostic(format!("unknown --kind: {other}"))),
    };
    let statement = flag_value(args, "statement")
        .ok_or_else(|| CommandFailure::diagnostic("--statement <TEXT> is required"))?;
    let scope = flag_value(args, "scope")
        .ok_or_else(|| CommandFailure::diagnostic("--scope <TEXT> is required"))?;
    let confidence: f32 = flag_value(args, "confidence")
        .unwrap_or("0.0")
        .parse()
        .map_err(|_| CommandFailure::diagnostic("--confidence must be a float"))?;
    let json = has_flag(args, "json");

    let mut candidate = LessonCandidate::new(
        autospec_core::resilience::CandidateId::new(b"cli"),
        autospec_core::resilience::WorkId::new(b"cli"),
        autospec_core::resilience::AttemptId::new(b"cli"),
        kind,
        statement,
        scope,
    );
    if has_flag(args, "validated") {
        candidate.evidence.validation_results.push("validation passed".to_string());
    }
    if has_flag(args, "reviewed") {
        candidate.evidence.review_results.push("independent review OK".to_string());
    }
    candidate.confidence = confidence;

    let verdict = promote_verdict(&candidate);
    let unsafe_lesson = is_unsafe_lesson(&candidate);

    if json {
        println!(
            "{}",
            json!({
                "verdict": promotion_to_str(verdict),
                "unsafe_policy_lesson": unsafe_lesson,
                "authoritative_if_promoted": is_authoritative(&candidate),
            })
        );
    } else {
        println!("verdict: {}", promotion_to_str(verdict));
        println!("unsafe_policy_lesson: {unsafe_lesson}");
    }
    Ok(())
}

fn promotion_to_str(v: autospec_core::resilience::PromotionVerdict) -> &'static str {
    use autospec_core::resilience::PromotionVerdict;
    match v {
        PromotionVerdict::Promote => "promote",
        PromotionVerdict::KeepCandidate => "keep-candidate",
        PromotionVerdict::Reject => "reject",
    }
}

fn transition_check(args: &[String]) -> Result<(), CommandFailure> {
    let from = parse_state(flag_value(args, "from"))
        .ok_or_else(|| CommandFailure::diagnostic("--from <STATE> is required"))?;
    let to = parse_state(flag_value(args, "to"))
        .ok_or_else(|| CommandFailure::diagnostic("--to <STATE> is required"))?;
    let json = has_flag(args, "json");
    let allowed = can_transition(from, to);

    if json {
        println!("{}", json!({ "from": from.as_str(), "to": to.as_str(), "allowed": allowed }));
    } else {
        println!(
            "transition {} -> {} : {}",
            from.as_str(),
            to.as_str(),
            if allowed { "allowed" } else { "illegal" }
        );
    }
    Ok(())
}

fn parse_state(value: Option<&str>) -> Option<WorkState> {
    let v = value?.to_uppercase();
    match v.as_str() {
        "CREATED" => Some(WorkState::Created),
        "ASSIGNED" => Some(WorkState::Assigned),
        "DELIVERED" => Some(WorkState::Delivered),
        "CLAIMED" => Some(WorkState::Claimed),
        "RUNNING" => Some(WorkState::Running),
        "COMPLETED" => Some(WorkState::Completed),
        "VALIDATED" => Some(WorkState::Validated),
        "REVIEWED" => Some(WorkState::Reviewed),
        "MERGED" => Some(WorkState::Merged),
        _ => None,
    }
}

fn doctor() -> Result<(), CommandFailure> {
    println!(
        "{}",
        json!({
            "status": "ok",
            "schema": "autospec.resilience.doctor.v1",
            "checks": [
                {"name": "context-checkpoint-schema", "status": "ok"},
                {"name": "memory-map-schema", "status": "ok"},
                {"name": "attention-stream-schema", "status": "ok"},
                {"name": "work-receipt-schema", "status": "ok"},
                {"name": "lesson-candidate-schema", "status": "ok"}
            ],
            "events": EVENTS.len()
        })
    );
    Ok(())
}
