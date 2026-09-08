//! Pipeline stage capacity (#3785): find the constraint by measurement, and
//! throttle rather than expand.
//!
//! A pipeline is an ordered chain of stages. Each stage completes work at a
//! measured **throughput** (units per unit time). In front of every stage sits
//! a **queue** holding work-in-progress (WIP): work the stage upstream has
//! produced but the stage has not yet consumed. For stage 0 that queue is the
//! incoming work; for stage `i > 0` it is the inter-stage queue between stage
//! `i - 1` and stage `i`.
//!
//! Three capacity-planning decisions rest on measurement, not on how busy a
//! stage looks. Each is encoded here as a pure, testable primitive; callers
//! supply the measurements and act on the plans these functions return.
//!
//! 1. **The constraint is the slowest stage, measured by throughput — not the
//!    busiest one** ([`identify_constraint`]). In a series pipeline the stage
//!    with the lowest throughput sets the pace of the whole line, so only
//!    raising *that* stage's capacity raises end-to-end throughput.
//!
//! 2. **An idle stage upstream of a bottleneck is expected, not a bug.**
//!    Utilisation (how full a stage looks) is never, on its own, a reason to
//!    scale a stage ([`scale_decision`]). A stage that reads 0% because the
//!    downstream constraint starves it is correctly idle; adding capacity there
//!    buys nothing. Scaling is justified by throughput, never by a low
//!    utilisation number.
//!
//! 3. **When a queue outruns its downstream stage, throttle upstream.** When
//!    the WIP ahead of a stage exceeds what that stage can consume in a stated
//!    interval, the stage *upstream* is throttled (told to stop feeding), not
//!    the downstream stage expanded ([`throttle_decision`]).

use std::fmt;

/// A measurement that cannot form a valid pipeline snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThroughputError {
    /// A throughput or interval rate is negative or not a finite number.
    InvalidRate,
    /// A utilisation figure is outside the `[0, 1]` range.
    InvalidUtilisation,
    /// A pipeline snapshot is empty, or its queue count does not match its
    /// stage count.
    MalformedPipeline,
}

impl fmt::Display for ThroughputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRate => write!(
                f,
                "throughput/interval rate must be a finite, non-negative number"
            ),
            Self::InvalidUtilisation => write!(f, "utilisation must lie within [0, 1]"),
            Self::MalformedPipeline => write!(
                f,
                "pipeline snapshot is empty or its queue count does not match its stage count"
            ),
        }
    }
}

impl std::error::Error for ThroughputError {}

/// A span of unit time over which throughput is integrated (e.g. hours).
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Interval {
    unit_time: f64,
}

impl Interval {
    /// A non-negative, finite span of unit time.
    pub fn new(unit_time: f64) -> Result<Self, ThroughputError> {
        if !unit_time.is_finite() || unit_time < 0.0 {
            return Err(ThroughputError::InvalidRate);
        }
        Ok(Self { unit_time })
    }
    /// The span, in unit time.
    pub fn as_unit_time(self) -> f64 {
        self.unit_time
    }
}

/// A stage's measured throughput: units of work completed per unit time.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Throughput {
    rate: f64,
}

impl Throughput {
    /// A non-negative, finite rate. A zero rate is legal and meaningful: a
    /// stage that completes nothing is the strongest possible constraint.
    pub fn new(rate: f64) -> Result<Self, ThroughputError> {
        if !rate.is_finite() || rate < 0.0 {
            return Err(ThroughputError::InvalidRate);
        }
        Ok(Self { rate })
    }
    /// The raw rate, units per unit time.
    pub fn rate(self) -> f64 {
        self.rate
    }
    /// Units this stage completes over [`interval`](Interval).
    pub fn over(self, interval: Interval) -> f64 {
        self.rate * interval.as_unit_time()
    }
    /// True when the stage completes nothing — a total constraint.
    pub fn is_stalled(self) -> bool {
        self.rate == 0.0
    }
}

/// Work-in-progress: whole units sitting in a queue. Neither fractional nor
/// signed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Wip {
    count: usize,
}

impl Wip {
    /// A queue holding `count` units of work.
    pub fn new(count: usize) -> Self {
        Self { count }
    }
    /// Units of WIP in the queue.
    pub fn count(self) -> usize {
        self.count
    }
    /// True when the queue is empty.
    pub fn is_empty(self) -> bool {
        self.count == 0
    }
}

impl Default for Wip {
    fn default() -> Self {
        Self::new(0)
    }
}

/// One stage of the pipeline, with its measured throughput and (optionally)
/// its utilisation.
#[derive(Debug, Clone, PartialEq)]
pub struct Stage {
    /// Stable identifier for the stage.
    pub id: String,
    /// Measured throughput — the only field the capacity decisions read.
    pub throughput: Throughput,
    /// Optional, informational: how full the stage looks, in `[0, 1]`. Never a
    /// reason to scale — see [`scale_decision`].
    pub utilisation: Option<f64>,
}

impl Stage {
    /// A stage with no recorded utilisation.
    pub fn new(id: impl Into<String>, throughput: Throughput) -> Self {
        Self {
            id: id.into(),
            throughput,
            utilisation: None,
        }
    }
    /// Attach a utilisation figure, which must lie in `[0, 1]`.
    pub fn with_utilisation(&mut self, utilisation: f64) -> Result<(), ThroughputError> {
        if !utilisation.is_finite() || utilisation < 0.0 || utilisation > 1.0 {
            return Err(ThroughputError::InvalidUtilisation);
        }
        self.utilisation = Some(utilisation);
        Ok(())
    }
}

/// An ordered pipeline: stages upstream to downstream, with one input queue in
/// front of each stage.
#[derive(Debug, Clone, PartialEq)]
pub struct Pipeline {
    stages: Vec<Stage>,
    /// `queues[i]` is the WIP feeding stage `i`: for `i == 0` the incoming
    /// work, for `i > 0` the inter-stage queue between stage `i - 1` and stage
    /// `i`.
    queues: Vec<Wip>,
}

impl Pipeline {
    /// Assemble a snapshot. `queues.len()` must equal `stages.len()`, and the
    /// pipeline must have at least one stage.
    pub fn new(stages: Vec<Stage>, queues: Vec<Wip>) -> Result<Self, ThroughputError> {
        if stages.is_empty() || queues.len() != stages.len() {
            return Err(ThroughputError::MalformedPipeline);
        }
        Ok(Self { stages, queues })
    }
    /// The stages, upstream to downstream.
    pub fn stages(&self) -> &[Stage] {
        &self.stages
    }
    /// Number of stages.
    pub fn len(&self) -> usize {
        self.stages.len()
    }
    /// True when there are no stages.
    pub fn is_empty(&self) -> bool {
        self.stages.is_empty()
    }
    /// The WIP feeding stage `index`, if it exists.
    pub fn queue_feeding(&self, index: usize) -> Option<Wip> {
        self.queues.get(index).copied()
    }
    /// The WIP queued *between* stage `upstream` and stage `downstream`, where
    /// `downstream` is exactly one stage downstream of `upstream`.
    pub fn queue_between(&self, upstream: usize, downstream: usize) -> Option<Wip> {
        (downstream.checked_sub(upstream) == Some(1))
            .then(|| self.queues.get(downstream).copied())
            .flatten()
    }
}

/// The identified constraint: the index of the stage whose measured throughput
/// is lowest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Constraint {
    /// Index of the constraining stage.
    pub stage_index: usize,
}

/// The stage that sets the pace of the line: the one with the lowest measured
/// throughput. On a tie the *upstream-most* minimum wins, so the answer is
/// deterministic and stable across passes.
///
/// Utilisation is never consulted. A stage is the constraint because it
/// completes the fewest units per unit time, full stop.
pub fn identify_constraint(pipeline: &Pipeline) -> Option<Constraint> {
    // Rates are always finite (validated on construction), so `partial_cmp` is
    // total here. `min_by` is stable: on equal throughput it returns the first
    // (most upstream) stage, so the constraint is deterministic across passes.
    pipeline
        .stages()
        .iter()
        .enumerate()
        .min_by(|a, b| {
            a.1.throughput
                .rate()
                .partial_cmp(&b.1.throughput.rate())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(stage_index, _)| Constraint { stage_index })
}

/// Why a non-constraint stage must not be scaled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldReason {
    /// The stage sits upstream of the constraint: it is idle *because* the
    /// downstream constraint starves it. Expected and correct.
    IdleUpstreamOfConstraint,
    /// The stage is not the constraint for another reason (it is downstream of
    /// the constraint, or it is faster than the measured minimum).
    NotTheConstraint,
}

/// The verdict on "should we add capacity to a given stage?".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScaleDecision {
    /// Raising this stage's capacity raises end-to-end throughput: it is (one
    /// of) the slowest stages.
    Raise,
    /// Do not add capacity here. Raising a non-constraint stage buys nothing.
    Hold { reason: HoldReason },
}

/// Decide whether to add capacity to stage `target`, from throughput alone.
///
/// Utilisation is deliberately not read: a low utilisation number is not a
/// reason to scale. The verdict is [`ScaleDecision::Raise`] only when
/// `target` completes as few units per unit time as the measured minimum.
pub fn scale_decision(pipeline: &Pipeline, target: usize) -> Option<ScaleDecision> {
    let stages = pipeline.stages();
    let constraint = identify_constraint(pipeline)?;
    let target_stage = stages.get(target)?;
    if target_stage.throughput == stages[constraint.stage_index].throughput {
        return Some(ScaleDecision::Raise);
    }
    let reason = if target < constraint.stage_index {
        HoldReason::IdleUpstreamOfConstraint
    } else {
        HoldReason::NotTheConstraint
    };
    Some(ScaleDecision::Hold { reason })
}

/// Whether the stage upstream of a queue may keep feeding it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThrottleDecision {
    /// The queue is within the downstream stage's consumption for the interval;
    /// upstream may keep feeding.
    Feed,
    /// The queue already holds more than the downstream stage can consume in
    /// the interval; upstream must be throttled, not the downstream stage
    /// expanded.
    Throttle,
}

/// Flow control: compare the WIP queued ahead of `downstream` against what
/// `downstream` can consume in `interval`.
///
/// The comparison is against the *downstream* stage's throughput — the stage
/// that must absorb the backlog. "Exceeds" is strict: a queue exactly equal to
/// the downstream stage's consumption for the interval does not trigger a
/// throttle.
pub fn throttle_decision(wip: Wip, downstream: Throughput, interval: Interval) -> ThrottleDecision {
    let capacity = downstream.over(interval);
    if wip.count() as f64 > capacity {
        ThrottleDecision::Throttle
    } else {
        ThrottleDecision::Feed
    }
}

/// A per-stage row in the throughput report.
#[derive(Debug, Clone, PartialEq)]
pub struct StageReport {
    /// Index of the stage, upstream to downstream.
    pub stage_index: usize,
    /// The stage's identifier.
    pub id: String,
    /// Measured throughput.
    pub throughput: Throughput,
    /// Informational utilisation; `None` when unmeasured.
    pub utilisation: Option<f64>,
    /// The WIP feeding this stage (incoming for stage 0, inter-stage queue
    /// otherwise).
    pub input_wip: Wip,
    /// True when this stage is the identified constraint.
    pub is_constraint: bool,
}

/// The pipeline's per-stage throughput and queue-depth report: the constraint
/// made visible rather than inferred.
#[derive(Debug, Clone, PartialEq)]
pub struct PipelineReport {
    stages: Vec<StageReport>,
    constraint_index: Option<usize>,
}

/// Build the report: per-stage throughput, the input queue in front of each
/// stage, and the identified constraint.
pub fn build_report(pipeline: &Pipeline) -> PipelineReport {
    let constraint_index = identify_constraint(pipeline).map(|c| c.stage_index);
    let stages = pipeline
        .stages()
        .iter()
        .enumerate()
        .map(|(index, stage)| StageReport {
            stage_index: index,
            id: stage.id.clone(),
            throughput: stage.throughput,
            utilisation: stage.utilisation,
            input_wip: pipeline.queue_feeding(index).unwrap_or_default(),
            is_constraint: Some(index) == constraint_index,
        })
        .collect();
    PipelineReport {
        stages,
        constraint_index,
    }
}

impl PipelineReport {
    /// The per-stage rows, upstream to downstream.
    pub fn stages(&self) -> &[StageReport] {
        &self.stages
    }
    /// The index of the constraint stage, if any.
    pub fn constraint_index(&self) -> Option<usize> {
        self.constraint_index
    }
    /// A human-readable rendering: one line per stage showing throughput, the
    /// input queue depth, utilisation, and a marker on the constraint.
    pub fn render(&self) -> String {
        self.stages
            .iter()
            .map(render_row)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// One line of the report for a single stage.
fn render_row(row: &StageReport) -> String {
    let util = row
        .utilisation
        .map(|u| format!("{:.0}%", u * 100.0))
        .unwrap_or_else(|| "-".to_string());
    let marker = if row.is_constraint {
        "  <- constraint"
    } else {
        ""
    };
    format!(
        "stage[{}] id={} throughput={}/t input_wip={} utilisation={}{}",
        row.stage_index,
        row.id,
        row.throughput.rate(),
        row.input_wip.count(),
        util,
        marker,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stage(id: &str, rate: f64, utilisation: Option<f64>) -> Stage {
        let mut s = Stage::new(id, Throughput::new(rate).unwrap());
        if let Some(u) = utilisation {
            s.with_utilisation(u).unwrap();
        }
        s
    }

    // A three-stage line where the *middle* stage is the slowest, even though
    // it is neither the most nor the least utilised. The busiest stage (A) is
    // a trap: scaling it would look justified by utilisation and would not be.
    fn sample_pipeline() -> Pipeline {
        let stages = vec![
            stage("agent", 100.0, Some(1.0)),
            stage("convert", 10.0, Some(0.4)),
            stage("review", 50.0, Some(0.0)),
        ];
        let queues = vec![Wip::new(40), Wip::new(120), Wip::new(30)];
        Pipeline::new(stages, queues).unwrap()
    }

    #[test]
    fn identify_constraint_picks_lowest_throughput_not_highest_utilisation() {
        let pipeline = sample_pipeline();
        // The constraint is `convert` (10/t), not `agent` (100/t, 100% busy)
        // and not `review` (50/t, 0% idle).
        assert_eq!(
            identify_constraint(&pipeline),
            Some(Constraint { stage_index: 1 })
        );
    }

    #[test]
    fn identify_constraint_zero_throughput_is_the_constraint() {
        let stages = vec![
            stage("a", 50.0, None),
            stage("stalled", 0.0, None),
            stage("c", 5.0, None),
        ];
        let pipeline = Pipeline::new(stages, vec![Wip::new(0), Wip::new(0), Wip::new(0)]).unwrap();
        assert!(pipeline.stages()[1].throughput.is_stalled());
        assert_eq!(
            identify_constraint(&pipeline),
            Some(Constraint { stage_index: 1 })
        );
    }

    #[test]
    fn identify_constraint_tie_prefers_upstream_most() {
        let stages = vec![
            stage("a", 20.0, None),
            stage("b", 5.0, None),
            stage("c", 5.0, None),
            stage("d", 9.0, None),
        ];
        let pipeline = Pipeline::new(stages, vec![Wip::new(0); 4]).unwrap();
        // Two stages tie at the minimum (5/t); the upstream-most wins.
        assert_eq!(
            identify_constraint(&pipeline),
            Some(Constraint { stage_index: 1 })
        );
    }

    #[test]
    fn scale_decision_raises_only_the_constraint() {
        let pipeline = sample_pipeline();
        // Raising the constraint helps...
        assert_eq!(scale_decision(&pipeline, 1), Some(ScaleDecision::Raise));
        // ...a stage upstream of it is idle *because* of the constraint:
        // holding it is correct, and its 100% utilisation does not trigger a
        // raise.
        assert_eq!(
            scale_decision(&pipeline, 0),
            Some(ScaleDecision::Hold {
                reason: HoldReason::IdleUpstreamOfConstraint
            })
        );
        // A faster stage downstream is not the constraint.
        assert_eq!(
            scale_decision(&pipeline, 2),
            Some(ScaleDecision::Hold {
                reason: HoldReason::NotTheConstraint
            })
        );
    }

    #[test]
    fn scale_decision_never_uses_utilisation_alone() {
        // A completely idle (0%) stage that is *not* the slowest must still
        // hold: utilisation alone is not a scaling justification.
        let stages = vec![
            stage("idle-but-fast", 100.0, Some(0.0)),
            stage("constraint", 5.0, Some(0.9)),
        ];
        let pipeline = Pipeline::new(stages, vec![Wip::new(0), Wip::new(0)]).unwrap();
        assert_eq!(
            scale_decision(&pipeline, 0),
            Some(ScaleDecision::Hold {
                reason: HoldReason::IdleUpstreamOfConstraint
            })
        );
        // And a fully busy (100%) stage that is not the slowest holds too.
        assert_eq!(scale_decision(&pipeline, 1), Some(ScaleDecision::Raise));
    }

    #[test]
    fn scale_decision_out_of_range_target_is_none() {
        let pipeline = sample_pipeline();
        assert_eq!(scale_decision(&pipeline, 99), None);
    }

    #[test]
    fn throttle_decision_throttles_when_wip_exceeds_downstream_consumption() {
        let downstream = Throughput::new(10.0).unwrap(); // 10 units/t
        let interval = Interval::new(1.0).unwrap(); // one unit of time
                                                    // 10 units is exactly what downstream consumes; 11 exceeds it.
        assert_eq!(
            throttle_decision(Wip::new(10), downstream, interval),
            ThrottleDecision::Feed
        );
        assert_eq!(
            throttle_decision(Wip::new(11), downstream, interval),
            ThrottleDecision::Throttle
        );
        assert_eq!(
            throttle_decision(Wip::new(0), downstream, interval),
            ThrottleDecision::Feed
        );
    }

    #[test]
    fn throttle_decision_scales_with_the_stated_interval() {
        let downstream = Throughput::new(10.0).unwrap();
        // Over two units of time downstream consumes 20, so 15 no longer
        // exceeds the window and 21 does.
        let interval = Interval::new(2.0).unwrap();
        assert_eq!(
            throttle_decision(Wip::new(15), downstream, interval),
            ThrottleDecision::Feed
        );
        assert_eq!(
            throttle_decision(Wip::new(21), downstream, interval),
            ThrottleDecision::Throttle
        );
    }

    #[test]
    fn queue_between_reads_the_inter_stage_queue_only() {
        let pipeline = sample_pipeline();
        // The queue feeding stage 1 is the inter-stage queue between stages 0
        // and 1.
        assert_eq!(pipeline.queue_between(0, 1), Some(Wip::new(120)));
        assert_eq!(pipeline.queue_between(1, 2), Some(Wip::new(30)));
        // Not an inter-stage pair -> none.
        assert_eq!(pipeline.queue_between(0, 2), None);
        assert_eq!(pipeline.queue_between(1, 0), None);
        assert_eq!(pipeline.queue_feeding(0), Some(Wip::new(40)));
    }

    #[test]
    fn build_report_surfaces_throughput_queue_depth_and_constraint() {
        let pipeline = sample_pipeline();
        let report = build_report(&pipeline);

        assert_eq!(report.constraint_index(), Some(1));
        assert_eq!(report.stages().len(), 3);

        let rows: Vec<_> = report
            .stages()
            .iter()
            .map(|r| (r.stage_index, r.input_wip.count(), r.is_constraint))
            .collect();
        assert_eq!(rows, vec![(0, 40, false), (1, 120, true), (2, 30, false)]);

        // The render names every stage's throughput and queue and marks the
        // constraint, so it is visible rather than inferred.
        let rendered = report.render();
        assert!(rendered.contains("stage[0] id=agent throughput=100/t input_wip=40"));
        assert!(rendered.contains(
            "stage[1] id=convert throughput=10/t input_wip=120 utilisation=40%  <- constraint"
        ));
        assert!(rendered.contains("stage[2] id=review throughput=50/t input_wip=30"));
    }

    #[test]
    fn pipeline_rejects_malformed_snapshots() {
        assert_eq!(
            Pipeline::new(vec![], vec![]),
            Err(ThroughputError::MalformedPipeline)
        );
        // Queue count must match stage count.
        let stages = vec![stage("a", 5.0, None)];
        assert_eq!(
            Pipeline::new(stages, vec![]),
            Err(ThroughputError::MalformedPipeline)
        );
        let stages = vec![stage("a", 5.0, None), stage("b", 5.0, None)];
        assert_eq!(
            Pipeline::new(stages, vec![Wip::new(0)]),
            Err(ThroughputError::MalformedPipeline)
        );
    }

    #[test]
    fn rates_and_utilisation_are_validated() {
        assert!(Throughput::new(-1.0).is_err());
        assert!(Throughput::new(f64::NAN).is_err());
        assert!(Throughput::new(f64::INFINITY).is_err());
        assert!(Throughput::new(0.0).is_ok());
        assert!(Interval::new(-0.5).is_err());
        assert!(Interval::new(f64::NAN).is_err());

        let mut s = stage("a", 5.0, None);
        assert!(s.with_utilisation(1.5).is_err());
        assert!(s.with_utilisation(-0.1).is_err());
        assert!(s.with_utilisation(f64::NAN).is_err());
        assert!(s.with_utilisation(0.0).is_ok());
        assert!(s.with_utilisation(1.0).is_ok());
    }
}
