//! Attempt history and model rotation.
//!
//! Retrying a stall on the model that just stalled is close to worthless: same
//! model, same failure modes, same result. Rotation converts the retry into
//! evidence — one stall is ambiguous (model, endpoint, transient hang, or the
//! spec), but a stall that survives rotation across distinct architectures is
//! the cheapest strong signal available for "this issue cannot be implemented
//! as written". That signal goes to spec repair rather than to a fourth attempt.
//!
//! Two rules carry that: never re-use a model that already stalled on the issue,
//! and prefer a different family over a different endpoint of the same family.

use std::collections::BTreeSet;

use super::partial_work::WorkProduced;

/// One implementer model available to the pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelChoice {
    /// Model identifier as the harness takes it, e.g. `Qwen3.8-27B Q8`.
    pub id: String,
    /// Architecture family, e.g. `qwen`. Two endpoints of one family share
    /// failure modes, so family is what rotation diversity is measured in.
    pub family: String,
    /// Quantization, when the identifier or roster carries one.
    pub quantization: Option<String>,
}

impl ModelChoice {
    pub fn new(id: impl Into<String>, family: impl Into<String>) -> Self {
        let id = id.into();
        let family = family.into();
        let quantization = infer_quantization(&id);
        Self {
            id,
            family: normalise_family(&family),
            quantization,
        }
    }

    /// Build a choice from an identifier alone, inferring the family.
    pub fn from_id(id: impl Into<String>) -> Self {
        let id = id.into();
        let family = infer_family(&id);
        let quantization = infer_quantization(&id);
        Self {
            id,
            family,
            quantization,
        }
    }

    /// How the attempt is described on the issue: id plus quant when known,
    /// without repeating a quant the identifier already carries.
    pub fn label(&self) -> String {
        match &self.quantization {
            Some(quant)
                if !quant.is_empty()
                    && !self
                        .id
                        .to_ascii_lowercase()
                        .contains(&quant.to_ascii_lowercase()) =>
            {
                format!("{} {}", self.id, quant)
            }
            _ => self.id.clone(),
        }
    }
}

/// The architecture family of a model identifier: the leading alphabetic run of
/// its last path segment. `provider/DeepSeek-V4-Flash` and `DeepSeek-V4` are the
/// same family; `qwen` and `glm` are not.
pub fn infer_family(model_id: &str) -> String {
    let name = model_id.rsplit('/').next().unwrap_or(model_id);
    let family: String = name
        .chars()
        .take_while(|character| character.is_ascii_alphabetic())
        .collect();
    normalise_family(&family)
}

/// Quantization marker in a model identifier (`Q8`, `Q4_K_M`, `FP8`, `AWQ`).
///
/// A quant tag is a prefix from this set followed immediately by a digit or an
/// underscore, so a model *name* that happens to start with one of them
/// (`Qwen3.8-27B`) is not read as a quantization.
pub fn infer_quantization(model_id: &str) -> Option<String> {
    for token in model_id
        .split_whitespace()
        .flat_map(|chunk| chunk.split(['-', '/', '+']))
    {
        if let Some(quant) = quant_token(token) {
            return Some(quant);
        }
    }
    None
}

fn quant_token(token: &str) -> Option<String> {
    let lower = token.to_ascii_lowercase();
    if lower.is_empty()
        || lower.len() > 12
        || !lower
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '.'))
    {
        return None;
    }
    for prefix in ["nvfp", "gptq", "awq", "int", "fp", "bf", "q"] {
        let Some(rest) = lower.strip_prefix(prefix) else {
            continue;
        };
        if rest.is_empty() {
            continue;
        }
        let first = rest.chars().next()?;
        if !(first.is_ascii_digit() || first == '_') {
            continue;
        }
        if rest.chars().any(|character| character.is_ascii_digit()) {
            return Some(token.to_string());
        }
    }
    None
}

fn normalise_family(family: &str) -> String {
    let family = family.trim().to_ascii_lowercase();
    if family.is_empty() {
        return "unknown".to_string();
    }
    family
}

/// Which model the next attempt should use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rotation {
    /// A model that has not attempted the issue yet.
    Next {
        model: ModelChoice,
        /// Whether the new model is a different architecture from the last try.
        changed_family: bool,
    },
    /// The only model available, retried once because rotation was impossible.
    SameModelRetry { model: ModelChoice },
    /// Every model available has already stalled on this issue.
    Exhausted,
    /// No model is configured at all.
    NoModels,
}

impl Rotation {
    pub fn model(&self) -> Option<&ModelChoice> {
        match self {
            Rotation::Next { model, .. } | Rotation::SameModelRetry { model } => Some(model),
            Rotation::Exhausted | Rotation::NoModels => None,
        }
    }

    /// Whether the next attempt runs on a model that already failed here.
    pub fn repeats_a_model(&self) -> bool {
        matches!(self, Rotation::SameModelRetry { .. })
    }
}

/// The models an implementer pool may rotate across, in preference order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelRoster {
    models: Vec<ModelChoice>,
}

impl ModelRoster {
    pub fn new(models: Vec<ModelChoice>) -> Self {
        let mut seen: Vec<String> = Vec::new();
        let models = models
            .into_iter()
            .filter(|model| {
                if seen.contains(&model.id) {
                    return false;
                }
                seen.push(model.id.clone());
                true
            })
            .collect();
        Self { models }
    }

    /// Build a roster from identifiers, inferring each family.
    pub fn from_ids<I, S>(ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        Self::new(
            ids.into_iter()
                .map(|id| ModelChoice::from_id(id.as_ref()))
                .collect(),
        )
    }

    pub fn models(&self) -> &[ModelChoice] {
        &self.models
    }

    pub fn len(&self) -> usize {
        self.models.len()
    }

    /// How many distinct models the roster offers. A roster of one model can
    /// never rotate, which is a different failure from running out of models.
    pub fn distinct_model_count(&self) -> usize {
        self.models
            .iter()
            .map(|model| model.id.to_ascii_lowercase())
            .collect::<BTreeSet<String>>()
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.models.is_empty()
    }

    /// Pick the model for the next attempt given the models already tried,
    /// in order, most recent last.
    ///
    /// Never returns a model from `attempted`, except for the single-model
    /// retry: with one endpoint there is nothing to rotate to, and silently
    /// retrying it forever is exactly the behaviour this prevents. That retry
    /// is allowed once, and the second stall escalates.
    pub fn select_next(&self, attempted: &[String]) -> Rotation {
        if self.models.is_empty() {
            return Rotation::NoModels;
        }
        let fresh: Vec<&ModelChoice> = self
            .models
            .iter()
            .filter(|model| !attempted.iter().any(|tried| tried == &model.id))
            .collect();

        let Some(last) = attempted.last() else {
            let model = fresh.first().expect("non-empty roster has a first model");
            return Rotation::Next {
                model: (*model).clone(),
                changed_family: true,
            };
        };

        if fresh.is_empty() {
            if self.models.len() == 1 && attempted.len() < 2 {
                return Rotation::SameModelRetry {
                    model: self.models[0].clone(),
                };
            }
            return Rotation::Exhausted;
        }

        let last_family = self
            .models
            .iter()
            .find(|model| &model.id == last)
            .map(|model| model.family.clone())
            .unwrap_or_else(|| infer_family(last));
        let chosen = fresh
            .iter()
            .find(|model| model.family != last_family)
            .or(fresh.first())
            .expect("fresh roster has a model");
        Rotation::Next {
            model: (*chosen).clone(),
            changed_family: chosen.family != last_family,
        }
    }
}

/// How one attempt ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttemptOutcome {
    /// Ran to its own conclusion and released the lease normally.
    Completed,
    /// Killed for stalling, or abandoned when its lease expired.
    Stalled,
    /// Ended with a non-stall failure (crash, non-zero exit).
    Failed,
}

impl AttemptOutcome {
    pub fn label(self) -> &'static str {
        match self {
            AttemptOutcome::Completed => "completed",
            AttemptOutcome::Stalled => "stalled",
            AttemptOutcome::Failed => "failed",
        }
    }

    pub fn counts_against_attempts(self) -> bool {
        self != AttemptOutcome::Completed
    }
}

/// One attempt at one issue, as it will be recorded on the issue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttemptRecord {
    pub attempt: u32,
    pub worker_id: String,
    pub model: ModelChoice,
    /// Configuration the attempt ran under, e.g. `reasoning_effort=high`.
    pub configuration: String,
    pub duration_secs: u64,
    pub produced: WorkProduced,
    pub outcome: AttemptOutcome,
    /// What the agent was last observed doing.
    pub last_activity: String,
}

impl AttemptRecord {
    /// One line of the attempt history: model, quant, config, duration, output, ending.
    pub fn summary_line(&self) -> String {
        format!(
            "{attempt}. {model} ({config}) — {duration}s, {produced}, ended {outcome}; last activity: {last_activity}",
            attempt = self.attempt,
            model = self.model.label(),
            config = if self.configuration.trim().is_empty() {
                "default config"
            } else {
                self.configuration.as_str()
            },
            duration = self.duration_secs,
            produced = self.produced,
            outcome = self.outcome.label(),
            last_activity = if self.last_activity.trim().is_empty() {
                "unknown"
            } else {
                self.last_activity.as_str()
            },
        )
    }
}

/// Every attempt made at one issue, oldest first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AttemptHistory {
    records: Vec<AttemptRecord>,
}

impl AttemptHistory {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_records(records: Vec<AttemptRecord>) -> Self {
        Self { records }
    }

    /// Append an attempt, numbering it from the current length.
    pub fn push(&mut self, mut record: AttemptRecord) -> AttemptRecord {
        record.attempt = self.records.len() as u32 + 1;
        self.records.push(record.clone());
        record
    }

    pub fn records(&self) -> &[AttemptRecord] {
        &self.records
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub fn last(&self) -> Option<&AttemptRecord> {
        self.records.last()
    }

    /// Model identifiers that have already attempted this issue, in order.
    pub fn attempted_model_ids(&self) -> Vec<String> {
        self.records.iter().map(|r| r.model.id.clone()).collect()
    }

    /// How many attempts stalled rather than completed or failed.
    pub fn stall_count(&self) -> usize {
        self.records
            .iter()
            .filter(|record| record.outcome == AttemptOutcome::Stalled)
            .count()
    }

    /// Distinct architecture families that have tried this issue.
    pub fn distinct_families(&self) -> BTreeSet<String> {
        self.records
            .iter()
            .map(|record| record.model.family.clone())
            .collect()
    }

    /// Distinct models whose attempt did not complete.
    pub fn failed_model_labels(&self) -> Vec<String> {
        let mut labels: Vec<String> = Vec::new();
        for record in self
            .records
            .iter()
            .filter(|record| record.outcome.counts_against_attempts())
        {
            let label = record.model.label();
            if !labels.contains(&label) {
                labels.push(label);
            }
        }
        labels
    }

    /// Whether any attempt produced commits or working-tree changes.
    pub fn produced_anything(&self) -> bool {
        self.records.iter().any(|r| r.produced.produced())
    }

    /// The sentence that makes a spec-repair proposal credible: which models
    /// stalled, and whether any of them produced a commit.
    pub fn escalation_sentence(&self) -> String {
        let models = self.failed_model_labels();
        let count = models.len();
        let spelled = match count {
            0 => "no models".to_string(),
            1 => "one model".to_string(),
            2 => "two models".to_string(),
            3 => "three models".to_string(),
            4 => "four models".to_string(),
            5 => "five models".to_string(),
            other => format!("{other} models"),
        };
        let output = if self.produced_anything() {
            "partial work was captured"
        } else {
            "none produced a commit"
        };
        format!("stalled on {spelled} — {} — {output}", models.join(", "))
    }
}
