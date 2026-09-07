//! The tracker's program result target (fast local coding, #3315).
//!
//! The program's headline goal is stated in prose: reduce the median
//! successful local coding issue's wall-clock time by at least 2x, without a
//! material quality regression and without crossing the configured quality or
//! node-stability gates. This module turns that goal into a pure, verifiable
//! verdict so the completion decision is a computation a test can pin rather
//! than an opinion.
//!
//! It consumes measurements; it does not produce them. The benchmark suite
//! (spec section 16) collects the per-issue wall-clock, quality, and
//! stability numbers and feeds them in here. Keeping the decision pure and
//! separate from measurement is what lets a reviewer re-derive the verdict
//! from raw numbers.

/// The program-level result target: the required median speedup plus the
/// configured quality and node-stability gates it must not cross.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResultTarget {
    /// Required median wall-clock speedup. The candidate must be at least this
    /// many times faster than the baseline. The tracker's target is 2.0.
    pub speedup_factor: f64,
    /// Minimum successful samples per arm before a median is trusted.
    pub min_samples: usize,
    /// The candidate's measured quality must stay at or above this floor.
    /// Configure it at (or just under) the baseline's measured quality to
    /// express "no material quality regression."
    pub quality_floor: f64,
    /// Node stability — the fraction of runs that completed without degrading
    /// the node (no KV oversubscription, no live-session failure) — must stay
    /// at or above this floor. 1.0 means no degraded run is tolerated.
    pub stability_floor: f64,
}

impl Default for ResultTarget {
    fn default() -> Self {
        Self {
            speedup_factor: 2.0,
            min_samples: 5,
            quality_floor: 0.0,
            stability_floor: 1.0,
        }
    }
}

/// Which gate a verdict failed on, or that the target is proven.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateStatus {
    /// Median speedup met and no gate crossed.
    Proven,
    /// Fewer than `min_samples` successful samples in at least one arm.
    InsufficientSamples,
    /// Median speedup below `speedup_factor`.
    InsufficientSpeedup,
    /// Measured quality fell below `quality_floor`.
    QualityGateCrossed,
    /// Measured node stability fell below `stability_floor`.
    StabilityGateCrossed,
}

impl GateStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            GateStatus::Proven => "proven",
            GateStatus::InsufficientSamples => "insufficient_samples",
            GateStatus::InsufficientSpeedup => "insufficient_speedup",
            GateStatus::QualityGateCrossed => "quality_gate_crossed",
            GateStatus::StabilityGateCrossed => "stability_gate_crossed",
        }
    }

    /// True only when the program's result target is met.
    pub fn is_met(&self) -> bool {
        matches!(self, GateStatus::Proven)
    }
}

/// The decision, plus the achieved speedup for the audit trail.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GateVerdict {
    pub status: GateStatus,
    /// `median(baseline) / median(candidate)`. `None` when there are not enough
    /// samples to compute a trusted median; `f64::INFINITY` when the candidate
    /// median is zero.
    pub achieved_speedup: Option<f64>,
}

impl GateVerdict {
    pub fn is_met(&self) -> bool {
        self.status.is_met()
    }
}

/// Median wall-clock time in milliseconds, as an `f64` so an even sample count
/// averages its two middle values. Empty input yields `0.0`.
pub fn median_wall_ms(wall_ms: &[u64]) -> f64 {
    if wall_ms.is_empty() {
        return 0.0;
    }
    let mut sorted = wall_ms.to_vec();
    sorted.sort_unstable();
    let mid = sorted.len() / 2;
    if sorted.len() % 2 == 1 {
        sorted[mid] as f64
    } else {
        (sorted[mid - 1] as f64 + sorted[mid] as f64) / 2.0
    }
}

/// `median(baseline) / median(candidate)`. A zero candidate median is the best
/// possible speedup and reports as `INFINITY`.
fn median_speedup(baseline_wall_ms: &[u64], candidate_wall_ms: &[u64]) -> f64 {
    let baseline = median_wall_ms(baseline_wall_ms);
    let candidate = median_wall_ms(candidate_wall_ms);
    if candidate == 0.0 {
        f64::INFINITY
    } else {
        baseline / candidate
    }
}

/// Decide whether the program meets its result target.
///
/// `baseline_wall_ms` and `candidate_wall_ms` must contain only *successful*
/// issue wall-clock times, one entry per issue. The gates are checked before
/// the speedup so a fast-but-broken candidate is reported as a gate crossing,
/// not as a win.
pub fn evaluate_result_target(
    target: &ResultTarget,
    baseline_wall_ms: &[u64],
    candidate_wall_ms: &[u64],
    candidate_quality: f64,
    candidate_stability: f64,
) -> GateVerdict {
    if baseline_wall_ms.len() < target.min_samples || candidate_wall_ms.len() < target.min_samples {
        return GateVerdict {
            status: GateStatus::InsufficientSamples,
            achieved_speedup: None,
        };
    }
    if candidate_quality < target.quality_floor {
        return GateVerdict {
            status: GateStatus::QualityGateCrossed,
            achieved_speedup: Some(median_speedup(baseline_wall_ms, candidate_wall_ms)),
        };
    }
    if candidate_stability < target.stability_floor {
        return GateVerdict {
            status: GateStatus::StabilityGateCrossed,
            achieved_speedup: Some(median_speedup(baseline_wall_ms, candidate_wall_ms)),
        };
    }
    let speedup = median_speedup(baseline_wall_ms, candidate_wall_ms);
    let status = if speedup >= target.speedup_factor {
        GateStatus::Proven
    } else {
        GateStatus::InsufficientSpeedup
    };
    GateVerdict {
        status,
        achieved_speedup: Some(speedup),
    }
}
