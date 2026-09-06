//! What a stalled attempt leaves behind for a human or another agent to read.
//!
//! Two renderings, both plain markdown so they survive being pasted into any
//! tracker: a [`StallNote`] attached when the issue goes back on the queue, and
//! a [`SpecRepairReport`] handed over when attempts run out. They live apart
//! from the decision logic in `release` because rendering is presentation, and
//! a note that reads well is as much of the fix as the branch that picks the
//! next model.

use std::path::PathBuf;

use super::attempts::AttemptRecord;
use super::liveness::Liveness;
use super::partial_work::WorkProduced;
use super::release::{EscalationReason, StallReason};

/// The block written to the issue before anything is torn down.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StallNote {
    pub issue: u64,
    pub worker_id: String,
    /// Which attempt this was, 1-based.
    pub attempt: u32,
    /// The attempt limit the issue is running under.
    pub max_attempts: u32,
    pub duration_secs: u64,
    pub work: WorkProduced,
    /// What happens to the issue next: the model it is re-queued onto, or the
    /// escalation it stops at. The note is only useful if it says this.
    pub next_step: Option<String>,
    pub last_activity: String,
    pub model: String,
    pub quantization: Option<String>,
    pub configuration: String,
    pub reason: StallReason,
    pub liveness: Liveness,
}

impl StallNote {
    /// Build the note from the attempt record and the stall verdict.
    pub fn from_record(
        issue: u64,
        record: &AttemptRecord,
        max_attempts: u32,
        work: WorkProduced,
        reason: StallReason,
        liveness: Liveness,
    ) -> Self {
        Self {
            issue,
            worker_id: record.worker_id.clone(),
            attempt: record.attempt,
            max_attempts,
            duration_secs: record.duration_secs,
            work,
            next_step: None,
            last_activity: record.last_activity.clone(),
            model: record.model.id.clone(),
            quantization: record.model.quantization.clone(),
            configuration: record.configuration.clone(),
            reason,
            liveness,
        }
    }

    /// `<model> (<quantization>)`, without repeating a quant the id carries.
    pub fn model_configuration(&self) -> String {
        with_quant(&self.model, &self.quantization)
    }

    /// Markdown block appended to the issue before anything is torn down.
    pub fn markdown(&self) -> String {
        format!(
            "## Stalled attempt {attempt} of {max_attempts} \u{2014} {reason}\n\n\
             - worker: `{worker}`\n\
             - duration: {duration}s\n\
             - work produced: {work}\n\
             - last activity: {last_activity}\n\
             - model: {model}\n\
             - configuration: {configuration}\n\
             - liveness: {liveness}\n\
             - reason: {reason_line}\n\
             - next: {next}\n",
            attempt = self.attempt,
            max_attempts = self.max_attempts,
            reason = self.reason.label(),
            worker = or_unknown(&self.worker_id),
            duration = self.duration_secs,
            work = self.work,
            last_activity = or_unknown(&self.last_activity),
            model = self.model_configuration(),
            configuration = or_default(&self.configuration),
            liveness = describe_liveness(self.liveness),
            reason_line = self.reason.label(),
            next = self
                .next_step
                .as_deref()
                .map(or_unknown)
                .unwrap_or("nothing recorded"),
        )
    }
}

fn or_unknown(value: &str) -> &str {
    if value.trim().is_empty() {
        "unknown"
    } else {
        value
    }
}

fn or_default(value: &str) -> &str {
    if value.trim().is_empty() {
        "default"
    } else {
        value
    }
}

/// `<model> (<quant>)`, without repeating a quant the identifier already carries.
fn with_quant(model: &str, quantization: &Option<String>) -> String {
    match quantization {
        Some(quant)
            if !quant.is_empty()
                && !model
                    .to_ascii_lowercase()
                    .contains(&quant.to_ascii_lowercase()) =>
        {
            format!("{model} ({quant})")
        }
        _ => model.to_string(),
    }
}

fn describe_liveness(liveness: Liveness) -> &'static str {
    match liveness {
        Liveness::Deliberating => "transcript growing, no output (deliberating)",
        Liveness::Producing => "output growing",
        Liveness::Quiet => "both signals flat, inside the stall window",
        Liveness::Hung => "both signals flat past the stall window",
    }
}

/// The hand-off to the spec-repair path: what was tried, in order, with what result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecRepairReport {
    pub issue: u64,
    pub reason: EscalationReason,
    /// Every attempt in order, so the repairer sees the whole history, not just
    /// the last failure.
    pub attempts: Vec<AttemptRecord>,
    /// Where the captured partial work lives.
    pub artifact_paths: Vec<PathBuf>,
    /// One sentence naming the models and what none of them produced.
    pub escalation_sentence: String,
}

impl SpecRepairReport {
    pub fn attempts(&self) -> usize {
        self.attempts.len()
    }

    pub fn reason_sentence(&self) -> String {
        self.reason.sentence()
    }

    pub fn markdown(&self) -> String {
        let mut out = format!(
            "## Needs spec repair\n\n\
             Issue #{issue} stalled and {reason}; it is handed to the spec-repair path \
             instead of another attempt.\n\n\
             {sentence}\n\n\
             ### Attempts\n\n",
            issue = self.issue,
            reason = self.reason.sentence(),
            sentence = self.escalation_sentence,
        );
        for record in &self.attempts {
            out.push_str(&format!(
                "- attempt {attempt}: {model} ({configuration}) \u{2014} {outcome}, {produced}, \
                 {duration}s, last activity: {last_activity}\n",
                attempt = record.attempt,
                model = with_quant(&record.model.id, &record.model.quantization),
                configuration = or_default(&record.configuration),
                outcome = record.outcome.label(),
                produced = record.produced,
                duration = record.duration_secs,
                last_activity = or_unknown(&record.last_activity),
            ));
        }
        if !self.artifact_paths.is_empty() {
            out.push_str("\n### Captured work\n\n");
            for path in &self.artifact_paths {
                out.push_str(&format!("- `{}`\n", path.display()));
            }
        }
        out.push_str(
            "\nA stall that survives rotation is evidence about the spec, not the model: \
             check the acceptance criteria for anything unverifiable, missing context, or \
             larger than one attempt.\n",
        );
        out
    }
}
