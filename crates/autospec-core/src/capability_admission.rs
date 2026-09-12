//! Admit workers on measured throughput, not liveness (issue #4409).
//!
//! A model ran **100x slower than it should** for an unknown length of time,
//! and no check in the fleet noticed, because every check was a *liveness*
//! check. `qwen3.8-flash-next` prefilled at 6 tok/s where `qwen3.8-27b` on the
//! same card, same image and same node class managed 918 tok/s. Measured
//! inside the allocation during an active prefill: both GPUs at **0%
//! utilisation, 74 W** (idle) while the weights sat resident in VRAM.
//! llama.cpp had loaded the model to the GPU and was executing it on CPU.
//!
//! Every signal the fleet had said healthy — process up, `/health` 200,
//! `/props` answered, the admission probe's one-token completion succeeded,
//! registration succeeded, the stuck detector saw token counters advancing.
//! All true, all useless. **The worker was alive; the capability was
//! absent.** Five workers held 10 GPUs running CPU inference.
//!
//! The invariants from the issue, each mapped to a primitive here:
//!
//! 1. **Admit on measured throughput, not on response.** A worker whose
//!    measured prefill or decode rate falls far below the floor for its card
//!    must fail admission, not register silently. The floor is knowable per
//!    (model, card) and the measurement is one request ([`admit`]).
//! 2. **GPU utilisation during generation is a first-class health signal.**
//!    Near zero while generating is unambiguous and is one call away
//!    ([`gpu_during_generation`]).
//! 3. **Record a per-(model, card) performance baseline, and alert on
//!    deviation from it** rather than on absolute thresholds — the baseline
//!    is also what tells you a regression from an image or model upgrade
//!    ([`BaselineTable`], [`deviation`]).
//!
//! And the general form: for any resource pool, health must be defined as
//! *the capability the pool exists to provide*, not the reachability of its
//! members. Those diverge exactly when something interesting is wrong
//! ([`diverges`]).
//!
//! Everything here is pure: no I/O, no clock, no subprocess. The caller
//! observes the worker (the one probe request, the GPU counter) and calls
//! these with the observed values.

use std::collections::BTreeMap;

/// The six liveness signals that were all green in the incident. Each one is
/// true of a worker that is 100x slow: slow is still responding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LivenessProfile {
    /// The server process is running.
    pub process_up: bool,
    /// `/health` returned 200.
    pub health_ok: bool,
    /// `/props` answered.
    pub props_ok: bool,
    /// The admission probe's one-token completion succeeded.
    pub probe_ok: bool,
    /// Registration with the gateway succeeded.
    pub registered: bool,
    /// The stuck detector saw token counters advancing.
    pub counters_advancing: bool,
}

impl LivenessProfile {
    /// The incident's profile: every liveness check green, the capability
    /// absent.
    pub fn all_green(&self) -> bool {
        self.process_up
            && self.health_ok
            && self.props_ok
            && self.probe_ok
            && self.registered
            && self.counters_advancing
    }
}

/// Which stage of generation a throughput figure covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// Prompt processing: tokens consumed per second.
    Prefill,
    /// Token generation: tokens produced per second.
    Decode,
}

impl Stage {
    pub fn label(&self) -> &'static str {
        match self {
            Stage::Prefill => "prefill",
            Stage::Decode => "decode",
        }
    }
}

/// The one measurement admission needs: a single request, timed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Measurement {
    /// Measured prefill rate, in tokens per second.
    pub prefill_toks_per_s: u64,
    /// Measured decode rate, in tokens per second.
    pub decode_toks_per_s: u64,
}

/// The floor a worker must meet to be admitted, knowable per (model, card).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThroughputFloor {
    /// The minimum prefill rate this card must produce for this model.
    pub prefill_toks_per_s: u64,
    /// The minimum decode rate this card must produce for this model.
    pub decode_toks_per_s: u64,
}

impl ThroughputFloor {
    /// A floor of zero in either stage names no capability, so it is not a
    /// floor.
    pub fn new(prefill_toks_per_s: u64, decode_toks_per_s: u64) -> Option<Self> {
        (prefill_toks_per_s > 0 && decode_toks_per_s > 0).then_some(Self {
            prefill_toks_per_s,
            decode_toks_per_s,
        })
    }
}

/// The outcome of admitting a worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    /// Every measured rate meets the floor: register the worker.
    Admitted,
    /// A liveness signal failed: the worker is not serving at all.
    NotServing,
    /// No floor is recorded for this (model, card). Fail closed: a worker
    /// that cannot be judged against a floor does not register silently.
    NoFloor,
    /// No measurement was taken. Liveness is a precondition, not a health
    /// definition: fail closed rather than admit on responses alone.
    Unmeasured,
    /// The worker is alive and responding, but the capability is absent:
    /// the measured rate for `stage` is below the floor. This is the
    /// incident — the state every liveness check reads as healthy.
    BelowFloor {
        /// Which stage fell below the floor.
        stage: Stage,
        /// The measured rate, in tokens per second.
        measured: u64,
        /// The floor for this (model, card), in tokens per second.
        floor: u64,
    },
}

/// Invariant 1: admit on measured throughput, not on response.
///
/// The decision order is deliberate: a non-serving worker is reported as not
/// serving (there is nothing to measure); a worker that is serving but
/// cannot be judged — no floor recorded, or no measurement taken — fails
/// closed instead of registering silently; only a measured worker with a
/// floor is admitted or rejected on its rates.
pub fn admit(
    liveness: &LivenessProfile,
    measured: Option<&Measurement>,
    floor: Option<&ThroughputFloor>,
) -> Admission {
    if !liveness.all_green() {
        return Admission::NotServing;
    }
    let floor = match floor {
        Some(floor) => floor,
        None => return Admission::NoFloor,
    };
    let measured = match measured {
        Some(measured) => measured,
        None => return Admission::Unmeasured,
    };
    if measured.prefill_toks_per_s < floor.prefill_toks_per_s {
        return Admission::BelowFloor {
            stage: Stage::Prefill,
            measured: measured.prefill_toks_per_s,
            floor: floor.prefill_toks_per_s,
        };
    }
    if measured.decode_toks_per_s < floor.decode_toks_per_s {
        return Admission::BelowFloor {
            stage: Stage::Decode,
            measured: measured.decode_toks_per_s,
            floor: floor.decode_toks_per_s,
        };
    }
    Admission::Admitted
}

/// The general form: liveness and capability diverge exactly when something
/// interesting is wrong, which is the only time it matters. True precisely
/// for the incident's shape — every liveness check green, the capability
/// the pool exists to provide absent.
pub fn diverges(liveness_green: bool, capability_meets_floor: bool) -> bool {
    liveness_green && !capability_meets_floor
}

/// GPU utilisation below this percentage, observed *while generating*, is
/// near zero and unambiguous: the model is executing somewhere other than
/// the GPU (in the incident: CPU, at 0% / 74 W with the weights resident in
/// VRAM).
pub const NEAR_ZERO_UTILIZATION: u8 = 5;

/// What one GPU-counter read during generation says about the worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenerationGpu {
    /// The GPU is doing the work.
    Busy {
        /// Observed utilisation, in percent.
        percent: u8,
    },
    /// Near-zero utilisation while generating: the worker is alive, the
    /// capability is absent. One call away, and no liveness check can see
    /// it.
    IdleDuringGeneration {
        /// Observed utilisation, in percent.
        percent: u8,
    },
}

impl GenerationGpu {
    /// True only when the GPU is doing the work it is paying for.
    pub fn healthy(&self) -> bool {
        matches!(self, GenerationGpu::Busy { .. })
    }
}

/// Invariant 2: GPU utilisation during generation is a first-class health
/// signal. A fleet of GPU workers that never looks at GPU utilisation is not
/// monitoring the thing it is paying for.
pub fn gpu_during_generation(percent: u8) -> GenerationGpu {
    if percent < NEAR_ZERO_UTILIZATION {
        GenerationGpu::IdleDuringGeneration { percent }
    } else {
        GenerationGpu::Busy { percent }
    }
}

/// The recorded performance of one (model, card) pair: the baseline
/// admission floors and regression alerts are derived from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Baseline {
    /// The model, e.g. `qwen3.8-27b`.
    pub model: String,
    /// The card, e.g. the node class's GPU.
    pub card: String,
    /// Recorded prefill rate, in tokens per second.
    pub prefill_toks_per_s: u64,
    /// Recorded decode rate, in tokens per second.
    pub decode_toks_per_s: u64,
}

/// The per-(model, card) performance baselines of a fleet.
#[derive(Debug, Clone, Default)]
pub struct BaselineTable(BTreeMap<(String, String), Baseline>);

impl BaselineTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record (or refresh) the baseline for one (model, card) pair. Re-
    /// recording is an upsert: a re-measurement replaces the entry.
    pub fn record(&mut self, model: &str, card: &str, prefill: u64, decode: u64) {
        self.0.insert(
            (model.to_string(), card.to_string()),
            Baseline {
                model: model.to_string(),
                card: card.to_string(),
                prefill_toks_per_s: prefill,
                decode_toks_per_s: decode,
            },
        );
    }

    /// The baseline recorded for one (model, card) pair, if any.
    pub fn baseline_for(&self, model: &str, card: &str) -> Option<&Baseline> {
        self.0.get(&(model.to_string(), card.to_string()))
    }
}

/// Whether a fresh measurement deviates from its baseline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Deviation {
    /// Within the baseline: no alert.
    WithinBaseline,
    /// The measured rate for `stage` is `times_slower` times below the
    /// baseline. This is the regression an image or model upgrade produces —
    /// the deviation from the baseline, not an absolute threshold, is what
    /// names it.
    Regression {
        /// Which stage regressed.
        stage: Stage,
        /// The freshly measured rate, in tokens per second.
        measured: u64,
        /// The recorded baseline rate, in tokens per second.
        baseline: u64,
        /// How many times slower: `baseline / measured` (measured of zero
        /// counts as 1, so a dead stage reports the full baseline factor).
        times_slower: u64,
    },
}

/// Invariant 3: alert on deviation from the per-(model, card) baseline
/// rather than on absolute thresholds.
///
/// A stage alerts when its measured rate is `min_times_slower` or more
/// times below the baseline — `min_times_slower` must be at least 2, since
/// a factor of 1 is no deviation. Both stages are checked and the worse
/// regression is reported; a floor of two is the smallest deviation that is
/// a regression at all.
pub fn deviation(baseline: &Baseline, measured: &Measurement, min_times_slower: u64) -> Deviation {
    let min = min_times_slower.max(2);
    let mut worst: Option<Deviation> = None;
    for (stage, base, got) in [
        (
            Stage::Prefill,
            baseline.prefill_toks_per_s,
            measured.prefill_toks_per_s,
        ),
        (
            Stage::Decode,
            baseline.decode_toks_per_s,
            measured.decode_toks_per_s,
        ),
    ] {
        if got >= base {
            continue;
        }
        let times_slower = base / got.max(1);
        if times_slower >= min {
            let d = Deviation::Regression {
                stage,
                measured: got,
                baseline: base,
                times_slower,
            };
            let worse = match worst {
                None => true,
                Some(Deviation::Regression {
                    times_slower: w, ..
                }) => times_slower > w,
                Some(Deviation::WithinBaseline) => true,
            };
            if worse {
                worst = Some(d);
            }
        }
    }
    worst.unwrap_or(Deviation::WithinBaseline)
}

impl Deviation {
    /// The regression factor, or 0 when within baseline.
    pub fn times_slower(&self) -> u64 {
        match self {
            Deviation::WithinBaseline => 0,
            Deviation::Regression { times_slower, .. } => *times_slower,
        }
    }
}
