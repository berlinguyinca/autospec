//! An average over non-substitutable units is a different quantity, not an
//! approximation (issue #4461).
//!
//! The fleet autoscaler decided scale-up from one number —
//! `31% busy (27/87 over 3 samples), target 75%` — and said no for four
//! consecutive runs. Throughout that window `deepseek-v4-flash` sat at 1/1
//! slots with a deferred queue of 2: fully saturated, turning requests
//! away. Its contribution to the fleet average is one slot in 87; its
//! saturation moves the fleet number by about one point. A 1-slot model at
//! 100% utilisation can raise a fleet average of 87 slots by at most
//! 1.1 percentage points, so no threshold that is meaningful for the fleet
//! can ever be crossed by that model's saturation alone — and the masking
//! is worst exactly where it matters most, because a planner gives large
//! models fewer slots, so the scarcest, most expensive-to-provision units
//! are the ones whose saturation is most thoroughly averaged away.
//!
//! The units are not interchangeable: a request for one model cannot be
//! served by another model's slot, so "27/87 busy" is not a statement
//! about capacity available to any actual request. Where the members of a
//! population are not substitutable, the aggregate is not a coarse version
//! of the truth; it is a different quantity that happens to share its
//! name, and the error grows as the scarce unit gets scarcer.
//!
//! The invariant: **a capacity or health decision must be made at the
//! granularity at which the resource is actually allocatable.** For the
//! scaler that is per-model: `requests_deferred` on a model, sustained
//! across samples, is the trigger, and the fleet-wide occupancy is
//! reported alongside for context rather than used as the decision.
//!
//! Everything here is pure: no I/O, no clock, no subprocess. The caller
//! observes the fleet and calls these with the observed values.

/// One unit (for the scaler: one model on one node) as observed in one
/// sample.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitObservation {
    /// The unit's name (the model).
    pub unit: String,
    /// How many allocatable slots the unit has.
    pub slots: u32,
    /// How many of them are busy.
    pub busy: u32,
    /// Requests deferred for this unit in this sample. This is the
    /// per-unit signal the average hides: deferral is a capacity trigger
    /// at the granularity where capacity is actually allocatable.
    pub requests_deferred: u32,
}

/// One sample of the whole fleet: every unit observed at one moment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FleetSample {
    pub units: Vec<UnitObservation>,
}

impl FleetSample {
    /// The population the average covers: `(busy, slots)` summed across
    /// every unit in the sample.
    pub fn busy_slots(&self) -> (u32, u32) {
        let busy: u32 = self.units.iter().map(|u| u.busy).sum();
        let slots: u32 = self.units.iter().map(|u| u.slots).sum();
        (busy, slots)
    }

    /// The fleet-wide busy percentage — the number the old decision was
    /// made from. It is a valid summary only when the units are fungible;
    /// see [`aggregate_threshold`].
    pub fn busy_percent(&self) -> f64 {
        let (busy, slots) = self.busy_slots();
        if slots == 0 {
            0.0
        } else {
            100.0 * f64::from(busy) / f64::from(slots)
        }
    }
}

/// Whether the members of a population can substitute for one another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fungibility {
    /// Members can serve each other's demand (homogeneous worker slots,
    /// identical GPU-hours, runs of one failure signature). The aggregate
    /// is a valid summary of the population.
    Substitutable,
    /// Members cannot serve each other's demand (models that cannot serve
    /// each other's requests, queues with different consumers). The
    /// aggregate is a different quantity, not a coarse version of the
    /// truth.
    NotSubstitutable,
}

/// The verdict of an aggregate-threshold check over a population.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AggregateCheck {
    /// Substitutable units; the aggregate is at or above the threshold.
    Triggered { percent: f64, target: f64 },
    /// Substitutable units; the aggregate is below the threshold.
    BelowTarget { percent: f64, target: f64 },
    /// Refused: the units are not substitutable, so the average is a
    /// different quantity, not an approximation. The check belongs on the
    /// individual; the aggregate is reported alongside for context only.
    /// No value of the target changes this verdict.
    RefusedNonFungible { percent: f64, target: f64 },
}

/// Apply a busy-percentage threshold to a population average.
///
/// The population the average covers is recorded with the check: when the
/// members are not substitutable the check is refused outright, and the
/// refusal is structural — no value of `target_percent` can make a
/// non-fungible average into a decision.
pub fn aggregate_threshold(
    busy: u32,
    slots: u32,
    target_percent: f64,
    units: Fungibility,
) -> AggregateCheck {
    let percent = if slots == 0 {
        0.0
    } else {
        100.0 * f64::from(busy) / f64::from(slots)
    };
    match units {
        Fungibility::Substitutable => {
            if percent >= target_percent {
                AggregateCheck::Triggered {
                    percent,
                    target: target_percent,
                }
            } else {
                AggregateCheck::BelowTarget {
                    percent,
                    target: target_percent,
                }
            }
        }
        Fungibility::NotSubstitutable => AggregateCheck::RefusedNonFungible {
            percent,
            target: target_percent,
        },
    }
}

/// How many percentage points of the fleet busy average a unit with
/// `unit_slots` of `fleet_slots` can move between the unit being idle and
/// the unit being saturated.
///
/// That width is the whole band a global threshold could use to detect the
/// unit's saturation: a 1-slot model in an 87-slot fleet moves the
/// average by at most about 1.1 points, so any threshold the saturation
/// could ever cross already sits inside ordinary noise. The error the
/// aggregate hides grows as the scarce unit gets scarcer.
pub fn max_average_shift(unit_slots: u32, fleet_slots: u32) -> f64 {
    if fleet_slots == 0 {
        return 0.0;
    }
    let effective = unit_slots.min(fleet_slots);
    100.0 * f64::from(effective) / f64::from(fleet_slots)
}

/// A scale-up trigger for one unit: sustained per-unit deferral.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScaleUp {
    /// The unit to scale (the model).
    pub unit: String,
    /// Its deferred count in the most recent sample.
    pub requests_deferred: u32,
    /// How many consecutive most-recent samples carried deferrals for
    /// this unit.
    pub sustained_samples: usize,
}

/// The fleet aggregate, carried alongside the decision for context only.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FleetContext {
    /// Busy slots across the fleet, most recent sample.
    pub busy: u32,
    /// Total slots across the fleet, most recent sample.
    pub slots: u32,
    /// How many samples the window spans.
    pub samples: usize,
    /// The busy percentage the old decision was made from.
    pub target_busy_percent: f64,
}

/// The scale-up decision for a window of samples.
#[derive(Debug, Clone, PartialEq)]
pub struct ScaleUpDecision {
    /// One entry per unit with sustained deferral. The fleet average never
    /// appears in this list: it is not a trigger.
    pub scale_ups: Vec<ScaleUp>,
    /// The fleet aggregate, reported alongside for context.
    pub context: FleetContext,
}

/// Decide scale-ups from a window of fleet samples, oldest first.
///
/// A unit triggers when it is present with `requests_deferred > 0` in each
/// of the last `sustained` samples — sustained across samples, per unit,
/// at the granularity where the resource is actually allocatable. The
/// fleet-wide busy percentage is computed and carried as context; it is
/// never the trigger, and `target_busy_percent` never changes the
/// decision.
pub fn decide_scale_up(
    samples: &[FleetSample],
    sustained: usize,
    target_busy_percent: f64,
) -> ScaleUpDecision {
    let sustained = sustained.max(1);
    let (busy, slots) = samples.last().map(|s| s.busy_slots()).unwrap_or((0, 0));
    let context = FleetContext {
        busy,
        slots,
        samples: samples.len(),
        target_busy_percent,
    };
    if samples.len() < sustained {
        return ScaleUpDecision {
            scale_ups: Vec::new(),
            context,
        };
    }
    let window_start = samples.len() - sustained;
    let trigger_window = &samples[window_start..];
    // Units in first-appearance order within the trigger window.
    let mut order: Vec<&str> = Vec::new();
    for sample in trigger_window {
        for unit in &sample.units {
            if !order.contains(&unit.unit.as_str()) {
                order.push(unit.unit.as_str());
            }
        }
    }
    let scale_ups = order
        .into_iter()
        .filter_map(|name| {
            let observations: Vec<&UnitObservation> = trigger_window
                .iter()
                .filter_map(|sample| sample.units.iter().find(|u| u.unit == name))
                .collect();
            (observations.len() == sustained
                && observations.iter().all(|o| o.requests_deferred > 0))
            .then(|| ScaleUp {
                unit: name.to_string(),
                requests_deferred: observations.last().unwrap().requests_deferred,
                sustained_samples: sustained,
            })
        })
        .collect();
    ScaleUpDecision { scale_ups, context }
}

impl ScaleUpDecision {
    /// The decision line: one clause per trigger, naming the unit and its
    /// deferred count, with the fleet aggregate alongside for context
    /// rather than as the decision.
    pub fn line(&self) -> String {
        let fleet = format!(
            "fleet {}% busy ({}/{} over {} samples), target {}%",
            self.context_busy_percent().round() as u32,
            self.context.busy,
            self.context.slots,
            self.context.samples,
            self.context.target_busy_percent.round() as u32
        );
        if self.scale_ups.is_empty() {
            return format!("no scale-up; {fleet}");
        }
        self.scale_ups
            .iter()
            .map(|s| {
                format!(
                    "scale-up model={} requests_deferred={} (sustained over {} samples); {fleet}",
                    s.unit, s.requests_deferred, s.sustained_samples
                )
            })
            .collect::<Vec<_>>()
            .join(" | ")
    }

    /// The fleet-wide busy percentage, context only.
    fn context_busy_percent(&self) -> f64 {
        if self.context.slots == 0 {
            0.0
        } else {
            100.0 * f64::from(self.context.busy) / f64::from(self.context.slots)
        }
    }
}
