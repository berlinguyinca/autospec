//! Not yet is not not ever (#3973).
//!
//! Six diagnoses were asserted, acted on and corrected in one session. Each
//! one read a system *in motion* as a system *at rest*: a conversion log
//! caught mid-write was "stalled", a `topup.log` tail read eight minutes
//! before the dispatch lines was "not dispatching", `pgrep -c` returning the
//! matcher's own wrapper shells was "still running", and a loop that had
//! opened 86 pull requests was killed as "low yield" on the evidence of two
//! log lines. Five corrections were harmless. One terminated a productive
//! 21-hour job, which had to be restarted.
//!
//! The observations were all real and correctly read. What was missing is a
//! step no observation carries by itself: the distinction between *absence at
//! time T* and *absence*. Because it cannot be fixed by intending to be
//! careful, it is closed mechanically here:
//!
//! 1. **A single sample of an append-only source says nothing**
//!    ([`Series`], [`Motion`]). Motion needs two samples separated by a gap;
//!    two samples at the same instant are one sample, and two samples from
//!    the same source are one source ([`Observations`]).
//! 2. **Status comes from markers, never from recency** ([`classify`],
//!    [`StepStatus`]). A step that emits no heartbeat is
//!    [`StepStatus::Unannotated`] — the reader may not call it stalled, and
//!    may not call it running either.
//! 3. **Destructive work needs a second, independent observation**
//!    ([`Observations::authorize`], [`Authorization`]): a terminal marker, or two
//!    observations from different sources.
//! 4. **Characterisations carry counts** ([`characterise`], [`Tail`]). A
//!    phrase like "low yield" without a measurement behind it is refused,
//!    and a count taken over a tail of the source is refused as evidence.
//! 5. **Observation and inference are reported apart** ([`Report`],
//!    [`Statement`]), so the gap between them is visible to the reader and
//!    to the author.
//!
//! Everything here is pure — no I/O, no clock, no subprocess. The caller
//! samples, reads the log and passes `now` in; the judgement is computed from
//! what was recorded.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// The gap two samples need before they describe a state rather than two
/// moments. Sampled twice within a second is sampled once.
pub const MIN_SAMPLE_GAP_SECS: u64 = 2;

/// A heartbeat is overdue once three intervals have passed without one. One
/// missing beat is a slow line, not a stopped step.
pub const STALL_HEARTBEAT_MULTIPLIER: u64 = 3;

/// Whether a source is moving.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Motion {
    /// Fewer than two samples, or two samples inside the minimum gap: the
    /// source has not been observed for long enough to be described.
    Unknown,
    /// Two consecutive samples differ: whatever is missing in either one may
    /// simply not have happened yet.
    Moving,
    /// Two consecutive samples agree across the gap: nothing changed.
    Settled,
}

impl Motion {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Moving => "moving",
            Self::Settled => "settled",
        }
    }

    /// `false` for [`Motion::Unknown`]: a conclusion needs more than one
    /// look, and this is the check that says so.
    pub fn is_conclusive(self) -> bool {
        !matches!(self, Self::Unknown)
    }
}

/// One look at a monotone signal — lines or bytes appended to a log, entries
/// in a process table, PRs opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sample {
    /// Second of the observation (caller's clock, monotone within a series).
    pub at: u64,
    /// The signal's value at that second.
    pub signal: u64,
}

/// Consecutive samples of one source, oldest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Series {
    samples: Vec<Sample>,
}

impl Series {
    pub fn new(samples: impl IntoIterator<Item = Sample>) -> Self {
        Self {
            samples: samples.into_iter().collect(),
        }
    }

    pub fn push(&mut self, sample: Sample) {
        self.samples.push(sample);
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// The most recent sample, the one a reader is tempted to conclude from.
    pub fn latest(&self) -> Option<Sample> {
        self.samples.last().copied()
    }

    /// Motion of the last two samples, requiring a gap of at least
    /// [`MIN_SAMPLE_GAP_SECS`] between them.
    pub fn motion(&self) -> Motion {
        self.motion_after(MIN_SAMPLE_GAP_SECS)
    }

    /// [`Series::motion`] with a caller-chosen gap, for sources that append
    /// slower than the default.
    pub fn motion_after(&self, min_gap_secs: u64) -> Motion {
        let (first, second) = match self.samples.len() {
            0 | 1 => return Motion::Unknown,
            n => (self.samples[n - 2], self.samples[n - 1]),
        };
        // Two looks at the same instant are one look: no time has passed in
        // which a pending line could have appeared, so nothing is settled.
        if second.at.saturating_sub(first.at) < min_gap_secs {
            return Motion::Unknown;
        }
        if first.signal == second.signal {
            Motion::Settled
        } else {
            Motion::Moving
        }
    }

    /// The observation line: what was seen, without the conclusion.
    pub fn line(&self) -> String {
        let Some(last) = self.latest() else {
            return "observed: no samples".to_string();
        };
        if self.samples.len() == 1 {
            return format!(
                "observed: 1 sample at {} (signal {}); motion unknown — sample again after a gap",
                last.at, last.signal
            );
        }
        format!(
            "observed: {} samples, latest sample at {} (signal {}); motion {}",
            self.samples.len(),
            last.at,
            last.signal,
            self.motion().as_str()
        )
    }
}

/// What a step's own output says about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    /// The terminal marker was written. The step is finished; nothing pending
    /// is being waited for.
    Complete,
    /// A heartbeat landed inside the stall threshold.
    Running,
    /// The step declares heartbeats and has missed three intervals.
    Stalled,
    /// No terminal marker and no heartbeat has ever been seen: the step does
    /// not annotate its own progress, so its state is unknown.
    Unannotated,
}

impl StepStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Running => "running",
            Self::Stalled => "stalled",
            Self::Unannotated => "unknown",
        }
    }

    /// Whether the status may be read as a state rather than a moment. Only
    /// [`StepStatus::Complete`] is terminal; the rest describe one look.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Complete)
    }
}

/// The markers a step writes, and when it last wrote them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct StepSignals {
    /// The terminal completion marker appeared in the output.
    pub terminal_seen: bool,
    /// Seconds since the last heartbeat marker, `None` when the step emits
    /// none (or none yet).
    pub heartbeat_age: Option<u64>,
    /// Seconds since the last byte of *any* output, `None` when there is
    /// none. Kept for the report line only: recency is never a status.
    pub output_age: Option<u64>,
}

/// A status plus the evidence it was derived from, so a reader can see the
/// difference between a measurement and the absence of one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepVerdict {
    pub status: StepStatus,
    /// Why this status and not another, in one clause.
    pub basis: String,
}

impl StepVerdict {
    /// `status=<status> basis=<basis>`.
    pub fn line(&self) -> String {
        format!("status={} basis={}", self.status.as_str(), self.basis)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_string())
    }
}

/// The status of a long-running step, from its own markers.
///
/// Order is the whole rule: a terminal marker wins (a finished step is not
/// stalled by its own silence), then the heartbeat, and in the absence of
/// both the answer is "unknown" — never "stalled", which is the inference
/// that killed a mid-write pass.
///
/// `heartbeat_interval` is the step's declared beat period; the stall
/// threshold is [`STALL_HEARTBEAT_MULTIPLIER`] times that period.
pub fn classify(signals: &StepSignals, heartbeat_interval: u64) -> StepVerdict {
    if signals.terminal_seen {
        return StepVerdict {
            status: StepStatus::Complete,
            basis: "terminal marker written".to_string(),
        };
    }
    let threshold = heartbeat_interval
        .max(1)
        .saturating_mul(STALL_HEARTBEAT_MULTIPLIER);
    match signals.heartbeat_age {
        Some(age) if age <= threshold => StepVerdict {
            status: StepStatus::Running,
            basis: format!(
                "heartbeat {age}s ago, threshold {threshold}s (interval {heartbeat_interval}s)"
            ),
        },
        Some(age) => StepVerdict {
            status: StepStatus::Stalled,
            basis: format!(
                "heartbeat {age}s stale, threshold {threshold}s (interval {heartbeat_interval}s)"
            ),
        },
        None => StepVerdict {
            status: StepStatus::Unannotated,
            basis: match signals.output_age {
                // The one place log recency appears: as a reason the status
                // is unknown, never as a reason it is stalled or running.
                Some(age) => format!(
                    "no heartbeat marker and no terminal marker; last output {age}s ago is \
                     recency, not evidence of state"
                ),
                None => "no heartbeat marker, no terminal marker, no output".to_string(),
            },
        },
    }
}

/// The observations available before a destructive action (stop,
/// re-dispatch, archive).
///
/// Sources are identified by name, and the set is what makes independence
/// machine-checkable: three `pgrep` samples are one source, not three
/// observations, and a `pgrep` sample plus the job's own completion marker
/// are two.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Observations {
    sources: BTreeSet<String>,
    terminal_marker: bool,
}

impl Observations {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a look taken through `source` (`"log:topup"`, `"pgrep"`,
    /// `"queue:live"`). Repeat looks from the same source add nothing.
    pub fn observed_from(mut self, source: impl Into<String>) -> Self {
        let source = source.into();
        if !source.trim().is_empty() {
            self.sources.insert(source.trim().to_string());
        }
        self
    }

    /// Record that the step's terminal marker was seen.
    pub fn with_terminal_marker(mut self) -> Self {
        self.terminal_marker = true;
        self
    }

    pub fn independent_observations(&self) -> usize {
        self.sources.len()
    }

    /// Authorize `action` against this evidence.
    ///
    /// Allowed only on a terminal marker (the state is settled, so acting on
    /// it cannot interrupt productive work) or on two independent sources.
    /// One glance at a log is neither.
    pub fn authorize(&self, action: &str) -> Authorization {
        let action = action.trim();
        if action.is_empty() {
            return Authorization::Held {
                code: HoldCode::UnnamedAction,
                message: "no action named: an unnameable action cannot be audited".to_string(),
            };
        }
        if self.terminal_marker {
            return Authorization::Allowed {
                basis: format!("{action}: authorized on terminal marker; state is settled"),
            };
        }
        if self.sources.len() >= 2 {
            return Authorization::Allowed {
                basis: format!(
                    "{action}: authorized on {} independent sources ({})",
                    self.sources.len(),
                    self.sources.iter().cloned().collect::<Vec<_>>().join(", ")
                ),
            };
        }
        Authorization::Held {
            code: HoldCode::SingleObservation,
            message: format!(
                "{action}: held on {} independent source(s); needs the terminal marker or a \
                 second independent source — absence at time T is not absence",
                self.sources.len()
            ),
        }
    }
}

/// The outcome of [`Observations::authorize`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Authorization {
    Allowed { basis: String },
    Held { code: HoldCode, message: String },
}

impl Authorization {
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allowed { .. })
    }

    pub fn line(&self) -> String {
        match self {
            Self::Allowed { basis } => format!("authorized: {basis}"),
            Self::Held { message, .. } => format!("held: {message}"),
        }
    }
}

/// Why a destructive action was held.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HoldCode {
    /// Neither a terminal marker nor two independent sources.
    SingleObservation,
    /// The caller asked to authorize nothing in particular.
    UnnamedAction,
}

/// A count taken over a *whole* source, the only kind a characterisation may
/// rest on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Measurement {
    pub label: String,
    pub value: u64,
    pub unit: String,
}

impl Measurement {
    pub fn counted(label: impl Into<String>, value: u64, unit: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            value,
            unit: unit.into(),
        }
    }

    pub fn line(&self) -> String {
        format!("{}={}{}", self.label, self.value, self.unit)
    }
}

/// A view of an append-only source: how much was looked at, and how much
/// exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tail {
    /// Lines the reader saw.
    pub visible: u64,
    /// Lines the source has.
    pub total: u64,
}

impl Tail {
    pub fn complete(&self) -> bool {
        self.visible >= self.total
    }

    /// A count over this view, refused when the view is a tail: the same
    /// `grep -c` over the whole log said 86, the tail said "low yield".
    pub fn count(
        &self,
        label: impl Into<String>,
        value: u64,
        unit: impl Into<String>,
    ) -> Result<Measurement, String> {
        if !self.complete() {
            return Err(format!(
                "{} counted over {} of {} lines: count the whole source, a tail is a sample",
                label.into(),
                self.visible,
                self.total
            ));
        }
        Ok(Measurement::counted(label, value, unit))
    }
}

/// A ratio rendered as a percentage measurement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ratio {
    pub numerator: u64,
    pub denominator: u64,
}

impl Ratio {
    /// Rejected on a zero denominator: a yield over nothing is not zero
    /// percent, it is not a yield.
    pub fn new(numerator: u64, denominator: u64) -> Result<Self, String> {
        if denominator == 0 {
            return Err(
                "ratio over zero denominator: nothing was counted to yield anything".into(),
            );
        }
        Ok(Self {
            numerator,
            denominator,
        })
    }

    /// Whole percentage, rounded to nearest.
    pub fn percent(&self) -> u64 {
        let scaled = self
            .numerator
            .saturating_mul(100)
            .saturating_add(self.denominator / 2);
        scaled / self.denominator
    }

    pub fn measurement(&self, label: impl Into<String>) -> Measurement {
        Measurement::counted(label, self.percent(), "%")
    }
}

/// Turn a phrase ("low yield") into a report line that carries its numbers.
///
/// Rejected: an empty phrase, and any phrase with no measurement behind it.
/// A judgement expressible as a count must be computed, and this is the
/// boundary where the uncomputed one is refused.
pub fn characterise(phrase: &str, basis: &[Measurement]) -> Result<String, String> {
    let phrase = phrase.trim();
    if phrase.is_empty() {
        return Err("a characterisation needs a phrase".to_string());
    }
    if basis.is_empty() {
        return Err(format!(
            "{phrase:?} carries no counts: compute the number before characterising"
        ));
    }
    let counts = basis
        .iter()
        .map(Measurement::line)
        .collect::<Vec<_>>()
        .join(", ");
    Ok(format!("{phrase} — {counts}"))
}

/// One line of a report: either something that was seen, or something the
/// author concluded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Statement {
    /// A fact, quotable from the source it came from.
    Observed(String),
    /// A claim built on the observations, and only as strong as them.
    Inferred(String),
}

impl Statement {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Observed(_) => "observed",
            Self::Inferred(_) => "inferred",
        }
    }

    pub fn text(&self) -> &str {
        match self {
            Self::Observed(text) | Self::Inferred(text) => text,
        }
    }

    pub fn line(&self) -> String {
        match self {
            Self::Observed(text) => format!("OBSERVED {text}"),
            Self::Inferred(text) => format!("INFERRED {text}"),
        }
    }
}

/// A report whose observations and inferences cannot be conflated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Report {
    statements: Vec<Statement>,
}

impl Report {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a fact: what the source showed, in terms the source supports.
    pub fn observe(&mut self, text: impl Into<String>) -> &mut Self {
        self.statements.push(Statement::Observed(text.into()));
        self
    }

    /// Record a conclusion.
    ///
    /// Rejected when nothing has been observed yet: an inference with no
    /// observation under it is a guess wearing a report line.
    pub fn infer(&mut self, text: impl Into<String>) -> Result<&mut Self, String> {
        if !self
            .statements
            .iter()
            .any(|s| matches!(s, Statement::Observed(_)))
        {
            return Err(
                "inference has no observation to stand on: state what was seen first".to_string(),
            );
        }
        self.statements.push(Statement::Inferred(text.into()));
        Ok(self)
    }

    pub fn statements(&self) -> &[Statement] {
        &self.statements
    }

    pub fn lines(&self) -> Vec<String> {
        self.statements.iter().map(Statement::line).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(at: u64, signal: u64) -> Sample {
        Sample { at, signal }
    }

    #[test]
    fn one_sample_of_an_appending_log_is_no_conclusion() {
        let series = Series::new([sample(100, 4_000)]);
        assert_eq!(series.motion(), Motion::Unknown);
        assert!(!series.motion().is_conclusive());
        assert!(series.line().contains("motion unknown"));
    }

    #[test]
    fn two_samples_apart_that_differ_are_moving() {
        let series = Series::new([sample(100, 4_000), sample(110, 4_001)]);
        assert_eq!(series.motion(), Motion::Moving);
    }

    #[test]
    fn two_samples_at_the_same_instant_are_one_sample() {
        let series = Series::new([sample(100, 4_000), sample(101, 4_001)]);
        assert_eq!(
            series.motion(),
            Motion::Unknown,
            "a 1s gap is inside MIN_SAMPLE_GAP_SECS: pending output had no time to appear"
        );
    }

    #[test]
    fn two_agreeing_samples_across_a_gap_are_settled() {
        let series = Series::new([sample(100, 4_001), sample(130, 4_001)]);
        assert_eq!(series.motion(), Motion::Settled);
    }

    #[test]
    fn a_mid_write_log_with_a_fresh_heartbeat_is_running_not_stalled() {
        // The populated case from #3793: the header line is written, the
        // result line has not landed yet, the heartbeat beat 3s ago.
        let signals = StepSignals {
            terminal_seen: false,
            heartbeat_age: Some(3),
            output_age: Some(1),
        };
        let verdict = classify(&signals, 60);
        assert_eq!(verdict.status, StepStatus::Running);
        assert!(!verdict.status.is_terminal());
        assert!(verdict.line().starts_with("status=running"));
    }

    #[test]
    fn a_terminal_marker_overrides_silence() {
        let signals = StepSignals {
            terminal_seen: true,
            heartbeat_age: Some(100_000),
            output_age: Some(100_000),
        };
        assert_eq!(classify(&signals, 60).status, StepStatus::Complete);
    }

    #[test]
    fn one_missed_beat_is_not_a_stall() {
        let signals = StepSignals {
            terminal_seen: false,
            heartbeat_age: Some(70),
            output_age: Some(70),
        };
        assert_eq!(classify(&signals, 60).status, StepStatus::Running);
    }

    #[test]
    fn three_missed_beats_are_a_stall() {
        let signals = StepSignals {
            terminal_seen: false,
            heartbeat_age: Some(240),
            output_age: Some(240),
        };
        assert_eq!(classify(&signals, 60).status, StepStatus::Stalled);
    }

    #[test]
    fn an_unannotated_log_is_unknown_and_never_stalled() {
        // Recency alone: output 5s ago, but no heartbeat marker and no
        // terminal marker. The reader gets "unknown", not "stalled".
        let signals = StepSignals {
            terminal_seen: false,
            heartbeat_age: None,
            output_age: Some(5),
        };
        let verdict = classify(&signals, 60);
        assert_eq!(verdict.status, StepStatus::Unannotated);
        assert_eq!(verdict.status.as_str(), "unknown");
        assert!(verdict.basis.contains("recency, not evidence"));
    }

    #[test]
    fn one_source_cannot_authorize_a_stop() {
        let evidence = Observations::new().observed_from("log:topup");
        let decision = evidence.authorize("stop convert3.sh");
        assert!(!decision.is_allowed());
        assert!(decision.line().contains("absence at time T is not absence"));
    }

    #[test]
    fn three_pgrep_samples_are_one_source() {
        let evidence = Observations::new()
            .observed_from("pgrep")
            .observed_from("pgrep")
            .observed_from("pgrep");
        assert_eq!(evidence.independent_observations(), 1);
        assert!(!evidence.authorize("re-dispatch issue 3973").is_allowed());
    }

    #[test]
    fn two_independent_sources_authorize_the_action() {
        let evidence = Observations::new()
            .observed_from("log:topup")
            .observed_from("queue:live");
        let decision = evidence.authorize("archive run output");
        assert!(decision.is_allowed());
        assert!(decision.line().contains("2 independent sources"));
    }

    #[test]
    fn a_terminal_marker_authorizes_the_action_alone() {
        let evidence = Observations::new()
            .observed_from("log:convert")
            .with_terminal_marker();
        assert!(evidence.authorize("archive run output").is_allowed());
    }

    #[test]
    fn an_unnamed_action_is_held() {
        let evidence = Observations::new()
            .observed_from("log:a")
            .observed_from("log:b");
        assert!(
            matches!(
                evidence.authorize("  "),
                Authorization::Held {
                    code: HoldCode::UnnamedAction,
                    ..
                }
            ),
            "an action nobody can name cannot be audited"
        );
    }

    #[test]
    fn a_tail_count_is_refused_as_evidence() {
        let tail = Tail {
            visible: 2,
            total: 4_000,
        };
        let error = tail
            .count("holds", 2, "lines")
            .expect_err("2 of 4000 lines is a sample");
        assert!(error.contains("2 of 4000 lines"));
    }

    #[test]
    fn a_whole_source_count_is_accepted() {
        let tail = Tail {
            visible: 4_000,
            total: 4_000,
        };
        let opened = tail.count("prs_opened", 86, "prs").expect("whole log");
        assert_eq!(opened.line(), "prs_opened=86prs");
    }

    #[test]
    fn a_characterisation_without_counts_is_refused() {
        let error = characterise("low yield", &[]).expect_err("no counts");
        assert!(error.contains("carries no counts"));
    }

    #[test]
    fn a_characterisation_renders_with_its_counts() {
        let basis = vec![
            Measurement::counted("prs_opened", 86, "prs"),
            Ratio::new(86, 430).expect("non-zero").measurement("yield"),
        ];
        // 86 of 430 attempts is 20%, not "low yield".
        assert_eq!(
            characterise("loop yield", &basis).expect("counted"),
            "loop yield — prs_opened=86prs, yield=20%"
        );
    }

    #[test]
    fn a_ratio_over_nothing_is_not_a_yield() {
        assert!(Ratio::new(0, 0).is_err());
    }

    #[test]
    fn an_inference_before_any_observation_is_refused() {
        let mut report = Report::new();
        assert!(report.infer("the dispatcher is broken").is_err());
        assert!(report.statements().is_empty());
    }

    #[test]
    fn observation_and_inference_render_apart() {
        let mut report = Report::new();
        report.observe("tail shows no dispatch line since 13:10");
        report.infer("the dispatcher is broken").expect("grounded");
        assert_eq!(
            report.lines(),
            vec![
                "OBSERVED tail shows no dispatch line since 13:10".to_string(),
                "INFERRED the dispatcher is broken".to_string(),
            ]
        );
    }

    #[test]
    fn a_deserialized_series_reports_motion() {
        let json = r#"[{"at":10,"signal":1},{"at":20,"signal":1}]"#;
        let samples: Vec<Sample> = serde_json::from_str(json).expect("fixture");
        assert_eq!(Series::new(samples).motion(), Motion::Settled);
    }
}
