//! A fixed timeout on a size-dependent operation is a silent capability
//! filter, not a flaky check (issue #4402).
//!
//! The incident: a worker read the served context from the model server with a
//! single `curl --max-time 10`, ten seconds after launching it. `/props`
//! answers 503 until the weights are resident, and load time scales with model
//! size. Against the models that have ever registered on the fleet:
//!
//! | model              | weights | ever registered |
//! |--------------------|---------|-----------------|
//! | qwen3.8-27b        | 15.7 GB | yes             |
//! | qwen3.8-27b-vision | 16.6 GB | yes             |
//! | qwen3.8-flash-next | 107 GB  | **never**       |
//! | deepseek-v4-flash  | 149 GB  | **never**       |
//! | glm-5.3-flash      | 281 GB  | **never**       |
//!
//! A clean threshold at roughly 30 GB: every large model in the catalogue was
//! excluded from the fleet for as long as this code existed, and because the
//! small cases worked, nothing looked broken. The symptom presented as
//! *model-specific flakiness* — "flash-next keeps having problems" — so every
//! investigation started inside the model, where the evidence appeared to
//! point. The actual cause was one number in a registration path, and it
//! discriminated by size rather than by model. A bug that correlates with a
//! property of the input looks exactly like a property of the input.
//!
//! The primitives here make the three invariants checkable:
//!
//! 1. **A timeout on a size-dependent operation must be derived, not chosen**
//!    ([`audit_timeout`]). Scale it from the input or from a bound that already
//!    governs the process; a constant is defensible only when the duration
//!    genuinely does not depend on the input. The tell is a clean step in the
//!    success pattern ([`detect_size_filter`]): every input below a boundary
//!    succeeds, every input above it never does.
//! 2. **A capability that is absent must be distinguishable from one that is
//!    failing** ([`classify_capacity`]). "We have no capacity for this model"
//!    and "the capacity is unhealthy" need different responses.
//! 3. **When a failure correlates with a property of the input, suspect the
//!    harness before the subject** ([`attribute_from_inputs`]). The first
//!    question for "model X keeps failing" is "what does the pipeline do
//!    differently for X?" — here, nothing, except take longer.

/// Whether the operation's duration scales with the input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeSensitivity {
    /// Duration scales with the input: bytes to load, rows to index, ...
    ScalesWithInput,
    /// Duration genuinely does not depend on the input.
    InputIndependent,
}

/// How the timeout is determined.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeoutSource {
    /// Scaled from the input size (bytes to load divided by a throughput
    /// bound).
    FromInputSize,
    /// Taken from a bound that already governs the process (the server's own
    /// load budget, the queue's walltime).
    FromGoverningBound,
    /// A constant, chosen independent of the input.
    Constant,
}

/// The verdict for applying a timeout to an operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeoutVerdict {
    /// The timeout is defensible for this operation.
    Sound,
    /// A constant timeout on a size-dependent operation: it passes the small
    /// inputs and deterministically excludes the large ones, and because the
    /// small cases work, nothing looks broken.
    SilentCapabilityFilter,
}

impl TimeoutVerdict {
    /// The line to carry in a closeout or a review.
    pub fn line(self) -> &'static str {
        match self {
            TimeoutVerdict::Sound => {
                "OK: the timeout is defensible — derived from the input or a governing bound, or the duration does not depend on the input"
            }
            TimeoutVerdict::SilentCapabilityFilter => {
                "FAIL: a constant timeout on a size-dependent operation is a silent capability filter — it passes the small inputs and deterministically excludes the large ones; derive the timeout from the input or from a bound that governs the process"
            }
        }
    }
}

/// Invariant 1: a timeout on a size-dependent operation must be derived, not
/// chosen.
///
/// A constant is defensible only when the operation's duration genuinely does
/// not depend on its input. A constant on a size-dependent operation is the
/// filter, not a flake.
pub fn audit_timeout(sensitivity: SizeSensitivity, source: TimeoutSource) -> TimeoutVerdict {
    if sensitivity == SizeSensitivity::ScalesWithInput && source == TimeoutSource::Constant {
        TimeoutVerdict::SilentCapabilityFilter
    } else {
        TimeoutVerdict::Sound
    }
}

/// One input with the size that governs the operation's duration, and whether
/// it has ever completed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SizedInput {
    /// A name for the input (a model, a table).
    pub name: String,
    /// The size that governs the operation's duration — bytes to load, rows to
    /// index. The unit is whatever the operation scales with; only ordering
    /// matters to the detector.
    pub size: u64,
    /// Whether this input has ever completed the operation.
    pub ever_succeeded: bool,
}

/// The boundary a size filter sits at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FilterThreshold {
    /// Largest size that has ever succeeded — the filter admits up to here.
    pub admits_through: u64,
    /// Smallest size that has never succeeded — the filter excludes from here.
    pub excludes_from: u64,
}

impl FilterThreshold {
    /// The line to carry in a report: the two edges of the excluded band.
    pub fn line(&self) -> String {
        format!(
            "size filter: admits through {} and excludes from {} — every input below the first has succeeded, every input at or above the second has never done so",
            self.admits_through, self.excludes_from
        )
    }
}

/// Invariant 1, made measurable: does the success pattern show the clean step
/// of a size filter?
///
/// The tell of a constant timeout on a size-dependent operation is a step
/// function over size: every input below a boundary has succeeded, every input
/// above it never has, and both sides are non-empty. If any success sits at or
/// above a failure's size, the pattern does not separate on size and is not a
/// size filter — it correlates with something else.
///
/// Returns `None` when there is no success or no failure (no boundary to
/// form) or when the pattern is interleaved rather than a clean step.
pub fn detect_size_filter(inputs: &[SizedInput]) -> Option<FilterThreshold> {
    let max_succeeded = inputs
        .iter()
        .filter(|i| i.ever_succeeded)
        .map(|i| i.size)
        .max()?;
    let min_failed = inputs
        .iter()
        .filter(|i| !i.ever_succeeded)
        .map(|i| i.size)
        .min()?;
    if max_succeeded < min_failed {
        Some(FilterThreshold {
            admits_through: max_succeeded,
            excludes_from: min_failed,
        })
    } else {
        None
    }
}

/// What the fleet can say about capacity for a model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapacityStatus {
    /// A worker for this model is registered and serving.
    Healthy,
    /// A worker for this model has registered but is failing.
    Failing,
    /// No worker for this model has ever registered.
    Absent,
}

impl CapacityStatus {
    /// The response each status requires. They differ on purpose: "we have no
    /// capacity for this model" and "the capacity is unhealthy" need different
    /// fixes.
    pub fn response(self) -> &'static str {
        match self {
            CapacityStatus::Healthy => "none — serving",
            CapacityStatus::Failing => {
                "repair the failing worker — it has registered but is not serving"
            }
            CapacityStatus::Absent => {
                "investigate registration — no worker for this model has ever registered"
            }
        }
    }
}

/// The evidence a capacity report carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapacityEvidence {
    /// A worker for this model has registered at some point.
    pub ever_registered: bool,
    /// A worker for this model is serving right now.
    pub currently_healthy: bool,
}

/// Invariant 2: an absent capability must be distinguishable from a failing
/// one.
///
/// A worker that never got past registration is *absent* capacity; a worker
/// that registered but is now misbehaving is *failing* capacity. The incident
/// fleet could not tell the two apart, so "we have no flash-next capacity" and
/// "flash-next is unhealthy" read the same and drew the same response.
pub fn classify_capacity(e: &CapacityEvidence) -> CapacityStatus {
    if e.currently_healthy {
        CapacityStatus::Healthy
    } else if e.ever_registered {
        CapacityStatus::Failing
    } else {
        CapacityStatus::Absent
    }
}

/// Where the cause of a correlated failure sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Attribution {
    /// The failures separate on a property of the input the pipeline acts on
    /// differently (size). The pipeline does something differently for those
    /// inputs — suspect the harness.
    Harness,
    /// The failures do not separate on a measured input property. The subject
    /// is the suspect.
    Subject,
}

impl Attribution {
    /// The line to carry in a review.
    pub fn line(self) -> &'static str {
        match self {
            Attribution::Harness => {
                "the failure separates on an input property the pipeline acts on — check what the harness does differently for those inputs before blaming the subject"
            }
            Attribution::Subject => {
                "the failure does not separate on a measured input property — the subject is the suspect"
            }
        }
    }
}

/// Invariant 3: when a failure correlates with a property of the input,
/// suspect the harness before the subject.
///
/// `correlates_with_input` is true when the failures separate cleanly on a
/// measured input property (the size filter of invariant 1 is the canonical
/// case). A bug that correlates with a property of the input looks exactly
/// like a property of the input — the correlation is the reason to look at the
/// pipeline, not the subject.
pub fn attribute_failure(correlates_with_input: bool) -> Attribution {
    if correlates_with_input {
        Attribution::Harness
    } else {
        Attribution::Subject
    }
}

/// Invariant 3, composed from the data: correlate the failures on size, and
/// when they separate cleanly the discriminator is a property the pipeline
/// acts on — the harness, not the subject.
pub fn attribute_from_inputs(inputs: &[SizedInput]) -> Attribution {
    attribute_failure(detect_size_filter(inputs).is_some())
}
