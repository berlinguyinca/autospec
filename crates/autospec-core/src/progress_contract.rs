//! Progress contracts: what a long-running task emits **while it runs**, and
//! how often (issue #4259).
//!
//! The incident: 22 agent runs sat wedged on a fleet for hours and every
//! signal available said fine. The Slurm jobs were `RUNNING`, the wrapper's
//! `.out` file existed (264 bytes, byte-identical across all 22), the
//! agent's own `agent.out` was 0 bytes for the wedged runs *and* for the
//! healthy ones, the crash-record query returned nothing, and the
//! utilisation counter read 22/22 — at cap, which is what a busy fleet is
//! supposed to read. Nothing was missing from the instrumentation except
//! the one thing that would have told: an artifact that a *working* run
//! produces and a *wedged* run does not.
//!
//! The failure was not a missing threshold (that is #4276) and not a missing
//! heartbeat file (that is #3995). It was that no component had ever been
//! asked what it emits while running. A completion contract — "it writes a
//! result at the end" — says nothing about the hours before the end, and
//! buffered-until-exit output makes the two states, working and wedged,
//! byte-identical until one of them stops.
//!
//! The invariants this module enforces, in the order the incident produced
//! them:
//!
//! 1. **A long-running task declares a progress contract before it runs**:
//!    which artifact it emits while running and the maximum silence it may
//!    show. [`ProgressContract::assess`] turns an observation into
//!    [`Progress`], and only [`Progress::Overdue`] means "look now".
//!    [`Progress::Unobserved`] — no artifact has ever been seen — is never
//!    reported as working, because nothing has been observed.
//! 2. **Buffered-until-exit output is not instrumentation.**
//!    [`ProgressSource::BufferedUntilExit`] is refused by
//!    [`ProgressContract::new`] ([`ContractError::UnobservableSource`]): a
//!    source that fills only when the process exits has no "while running"
//!    to observe, and any signal read from it is
//!    [`ProgressSource::confoundable_by_buffering`].
//! 3. **Liveness is a positive artifact, never the absence of a crash
//!    record.** [`Observation`] carries the process state and the crash
//!    record so the evidence record is complete, and [`ProgressContract::assess`]
//!    ignores both by design: "no failures recorded" was true for all 22
//!    wedged runs, and "the job is RUNNING" was true for all 22.
//! 4. **Absence of a known-next-step artifact beats presence of a process.**
//!    [`step_verdict`] reports [`StepVerdict::Wedged`] with
//!    `process_running: true` when the artifact of the step the run is
//!    supposed to be in is missing past its due time — the process state
//!    cannot overturn it, it is recorded alongside.
//! 5. **A signal that is constant across known-healthy and known-wedged
//!    instances is not evidence.** [`discriminate`] measures a candidate
//!    signal against instances whose state is known: a constant value
//!    ([`Discrimination::Constant`]) or a value shared by both states
//!    ([`Discrimination::Ambiguous`]) is not evidence, and neither is a
//!    signal tested on a single-state population
//!    ([`Discrimination::Untestable`]). The 264-byte wrapper log is the
//!    named instance: it was measured, and all 22 runs had it.
//! 6. **Occupancy is not work.** A utilisation counter counts slots held;
//!    [`saturation`] reads it beside a throughput baseline and returns
//!    [`Saturation::OccupiedNotWorking`] when the fleet is at cap while
//!    completions have collapsed, and [`companion_verdict`] says whether a
//!    utilisation-only series could see the collapse at all (it could not:
//!    every degraded window in the incident read 100%).
//! 7. **A spec that commissions a long-running worker must state the
//!    contract.** [`spec_progress_findings`] requires a
//!    `## Progress contract` section naming an emission and an interval for
//!    any spec declaring a `Longest expected run:` above
//!    [`LONG_RUN_THRESHOLD`], and rejects one whose named source is buffered
//!    output.
//!
//! The module is pure: the caller observes the artifacts and the clock, this
//! code decides what the observations mean.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::time::Duration;

/// Default maximum silence for a contract whose interval is not stated:
/// 5 minutes. Chosen short relative to the work (runs measured in tens of
/// minutes to hours) and long relative to a step boundary (seconds), so a
/// contract firing on healthy work stays unlikely while a wedged run is
/// visible in minutes rather than hours.
pub const DEFAULT_MAX_SILENCE: Duration = Duration::from_secs(300);

/// A spec declaring a longest expected run above this commissions a
/// long-running worker and must carry a progress contract (invariant 7).
/// Ten minutes: below it a run is short enough that "it exited, or it did
/// not" is a usable signal, and above it the incident's shape — hours of
/// silence read as normal operation — becomes reachable.
pub const LONG_RUN_THRESHOLD: Duration = Duration::from_secs(600);

/// Utilisation at or above this percentage counts as "at cap" for
/// invariant 6.
pub const SATURATION_PERCENT: u32 = 90;

/// Completions below this percentage of the healthy baseline count as a
/// throughput collapse for invariant 6.
pub const COLLAPSE_PERCENT: u64 = 50;

/// The section name a spec must carry when it commissions a long-running
/// worker (invariant 7).
pub const PROGRESS_CONTRACT_SECTION: &str = "## Progress contract";

/// The line a spec uses to declare how long its worker may run.
pub const EXPECTED_RUN_KEY: &str = "Longest expected run:";

/// Where a run's progress signal comes from (invariants 2 and 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgressSource {
    /// The task's stdout/stderr redirected to a file the runtime flushes
    /// when the process exits. Emits **nothing** while the task runs, so it
    /// cannot carry a progress signal: a working run and a wedged run show
    /// the same 0 bytes. Not instrumentation.
    BufferedUntilExit,
    /// A line the task appends to a log on a bounded interval while it
    /// works. Requires the task to cooperate — a wedged task stops writing,
    /// which is exactly the discrimination wanted.
    PeriodicEmission,
    /// An artifact the **wrapper** writes on the task's behalf (the
    /// per-step marker file). The task cannot suppress it and buffering
    /// cannot hide it, which is why it is the fleet's preferred source.
    WrapperArtifact,
}

impl ProgressSource {
    /// Whether anything can be observed from this source while the task is
    /// still running. `BufferedUntilExit` is the only `false` (invariant 2).
    pub fn observable_while_running(self) -> bool {
        !matches!(self, ProgressSource::BufferedUntilExit)
    }

    /// Whether normal operation can make this source look like a failure.
    /// Buffered output can: a healthy run shows an empty file until it
    /// exits, so "the log is empty" is confoundable by construction.
    pub fn confoundable_by_buffering(self) -> bool {
        matches!(self, ProgressSource::BufferedUntilExit)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ProgressSource::BufferedUntilExit => "buffered-until-exit",
            ProgressSource::PeriodicEmission => "periodic-emission",
            ProgressSource::WrapperArtifact => "wrapper-artifact",
        }
    }
}

/// Why [`ProgressContract::new`] refused the pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContractError {
    /// A zero interval makes every observation overdue: the contract fires
    /// on healthy work and gets switched off, which is how instrumentation
    /// that is too loud becomes the absence of instrumentation.
    ZeroInterval,
    /// The source emits nothing until the process exits, so there is no
    /// "while running" to observe. Buffering cannot be fixed by a shorter
    /// interval; it needs a different source (invariant 2).
    UnobservableSource,
}

impl ContractError {
    pub fn as_str(self) -> &'static str {
        match self {
            ContractError::ZeroInterval => "zero-interval",
            ContractError::UnobservableSource => "unobservable-source",
        }
    }

    /// The rejection names the fix, not just the fault: a shorter interval
    /// does not help a buffered source.
    pub fn hint(self) -> &'static str {
        match self {
            ContractError::ZeroInterval => {
                "state an interval a working run can actually meet (>= 1s); a contract \
                 that fires on healthy work is switched off"
            }
            ContractError::UnobservableSource => {
                "name a source that emits while the task runs (a wrapper-written step \
                 artifact or a periodic emission line); buffering is not fixed by a \
                 shorter interval"
            }
        }
    }
}

/// What a long-running task promises to emit while it runs, and how often
/// (invariant 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgressContract {
    pub source: ProgressSource,
    /// The most silence the contract tolerates between positive artifacts.
    pub max_silence: Duration,
}

impl ProgressContract {
    /// Refuses a zero interval and any source that cannot be observed while
    /// the task runs (invariants 1 and 2).
    pub fn new(source: ProgressSource, max_silence: Duration) -> Result<Self, ContractError> {
        if max_silence.is_zero() {
            return Err(ContractError::ZeroInterval);
        }
        if !source.observable_while_running() {
            return Err(ContractError::UnobservableSource);
        }
        Ok(ProgressContract {
            source,
            max_silence,
        })
    }

    /// The instant the next positive artifact is due, after `last_signal`.
    pub fn deadline_after(&self, last_signal: Duration) -> Duration {
        last_signal + self.max_silence
    }

    /// Assess one observation against the contract. `obs.artifact` is the
    /// wall-clock time of the last positive artifact the source produced
    /// (`None` = none seen since the run started).
    ///
    /// `obs.process` and `obs.crash` are read for the evidence record and
    /// deliberately ignored in the decision (invariant 3).
    pub fn assess(&self, obs: &Observation, now: Duration) -> Progress {
        let _ = (obs.process, obs.crash); // neither is liveness evidence
        match obs.artifact {
            Some(at) if now >= at => {
                let silent = now - at;
                if silent > self.max_silence {
                    Progress::Overdue {
                        silent,
                        overdue_by: silent - self.max_silence,
                    }
                } else {
                    Progress::OnSchedule { silent }
                }
            }
            // A timestamp from the future is a broken clock, not progress.
            Some(at) => Progress::Unobserved {
                elapsed: at.saturating_sub(now),
            },
            None => Progress::Unobserved { elapsed: now },
        }
    }

    /// The report line: `progress contract: wrapper-artifact every 300s`.
    pub fn line(&self) -> String {
        format!(
            "progress contract: {} every {}s",
            self.source.as_str(),
            self.max_silence.as_secs()
        )
    }
}

/// The Slurm/OS state of the process as observed. Accepted by
/// [`ProgressContract::assess`] and ignored by it (invariant 3): every wedged
/// run in the incident reported `Running`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessState {
    Running,
    Exited,
}

/// Whether a crash record exists for the run. Accepted and ignored by
/// [`ProgressContract::assess`]: the absence of a crash record is not
/// liveness, it is the absence of a record (invariant 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CrashRecord {
    Absent,
    Present,
}

/// One observation of a running task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Observation {
    /// Wall-clock time of the last positive artifact from the contract's
    /// source; `None` when none has ever been seen.
    pub artifact: Option<Duration>,
    pub process: ProcessState,
    pub crash: CrashRecord,
}

impl Observation {
    /// An observation where the process is running, no crash is recorded,
    /// and **nothing has been emitted** — the incident's reading, which is
    /// not "working" (invariants 1 and 3). The clock enters at
    /// [`ProgressContract::assess`], not here.
    pub fn silent_running() -> Observation {
        Observation {
            artifact: None,
            process: ProcessState::Running,
            crash: CrashRecord::Absent,
        }
    }
}

/// What the contract's observation means (invariant 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Progress {
    /// A positive artifact arrived within `max_silence`.
    OnSchedule { silent: Duration },
    /// The interval elapsed with no positive artifact. The only verdict that
    /// means "look now".
    Overdue {
        silent: Duration,
        overdue_by: Duration,
    },
    /// No positive artifact has ever been seen. **Not** "working": nothing
    /// has been observed, and an unobserved run is the state the incident
    /// was in for hours.
    Unobserved { elapsed: Duration },
}

impl Progress {
    /// Whether this verdict is an alarm. `Unobserved` is not an alarm but
    /// must never be rendered as healthy (invariant 1).
    pub fn is_alarm(&self) -> bool {
        matches!(self, Progress::Overdue { .. })
    }

    /// Whether this verdict may be rendered as "working". Only a positive
    /// artifact within the interval may (invariant 3).
    pub fn is_working(&self) -> bool {
        matches!(self, Progress::OnSchedule { .. })
    }

    pub fn line(&self) -> String {
        match self {
            Progress::OnSchedule { silent } => {
                format!(
                    "working: artifact {}s ago (within contract)",
                    silent.as_secs()
                )
            }
            Progress::Overdue { silent, overdue_by } => format!(
                "OVERDUE: no artifact for {}s ({}s past the contract)",
                silent.as_secs(),
                overdue_by.as_secs()
            ),
            Progress::Unobserved { elapsed } => format!(
                "unobserved: no artifact ever seen ({}s since start) — not working",
                elapsed.as_secs()
            ),
        }
    }
}

/// The step a run is known to be in, and when its artifact is due
/// (invariant 4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnownStep {
    pub name: String,
    pub due_at: Duration,
}

/// Where the known-next-step artifact is (invariant 4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepVerdict {
    /// The artifact arrived: the step is done.
    StepDone { name: String, at: Duration },
    /// The artifact is missing past its due time. Reported as wedged **with
    /// `process_running: true`**: presence of a process cannot overturn the
    /// absence of the artifact the step is supposed to produce.
    Wedged {
        name: String,
        overdue_by: Duration,
        process_running: bool,
    },
    /// Still inside the step's window: nothing to conclude.
    InFlight { name: String, remaining: Duration },
}

impl StepVerdict {
    pub fn is_wedged(&self) -> bool {
        matches!(self, StepVerdict::Wedged { .. })
    }

    /// The report line. The wedged line keeps the process state in it
    /// because the reader will otherwise ask: `wedged: 'build' artifact
    /// missing 4800s past due (job RUNNING)`.
    pub fn line(&self) -> String {
        match self {
            StepVerdict::StepDone { name, at } => {
                format!("step '{name}' done (artifact at {}s)", at.as_secs())
            }
            StepVerdict::Wedged {
                name,
                overdue_by,
                process_running,
            } => format!(
                "WEDGED: step '{name}' artifact missing {}s past due (job {})",
                overdue_by.as_secs(),
                if *process_running {
                    "RUNNING"
                } else {
                    "EXITED"
                }
            ),
            StepVerdict::InFlight { name, remaining } => {
                format!(
                    "step '{name}' in flight ({}s until due)",
                    remaining.as_secs()
                )
            }
        }
    }
}

/// Invariant 4: judge the known next step. `artifact_at` is when the step's
/// artifact was seen (`None` = not seen). `process_running` is recorded in
/// the verdict and never overrides it.
pub fn step_verdict(
    step: &KnownStep,
    artifact_at: Option<Duration>,
    process_running: bool,
    now: Duration,
) -> StepVerdict {
    if let Some(at) = artifact_at {
        return StepVerdict::StepDone {
            name: step.name.clone(),
            at,
        };
    }
    if now > step.due_at {
        return StepVerdict::Wedged {
            name: step.name.clone(),
            overdue_by: now - step.due_at,
            process_running,
        };
    }
    StepVerdict::InFlight {
        name: step.name.clone(),
        remaining: step.due_at - now,
    }
}

/// The known ground truth of an instance, used to test whether a candidate
/// signal carries information (invariant 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstanceState {
    Healthy,
    Wedged,
}

impl InstanceState {
    pub fn as_str(self) -> &'static str {
        match self {
            InstanceState::Healthy => "healthy",
            InstanceState::Wedged => "wedged",
        }
    }
}

/// One instance's value for a candidate signal, alongside the state that
/// instance is later found to have been in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignalSample {
    pub instance: String,
    pub truth: InstanceState,
    /// The signal's rendered value: `"264"`, `"0"`, `"RUNNING"`, `"22/22"`.
    pub value: String,
}

/// Whether a candidate signal carries information (invariant 5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Discrimination {
    /// Nothing to test.
    NoSamples,
    /// The population holds one state only: the signal was never presented
    /// with the condition it is supposed to detect, so its value here
    /// proves nothing (fail-closed: not evidence).
    Untestable { state: &'static str, samples: usize },
    /// Every sample carries the same value while the population contains
    /// both states: the signal cannot distinguish them. The 264-byte
    /// wrapper log — present, identical, in all 22 runs.
    Constant { value: String, samples: usize },
    /// Some value occurs in both a healthy and a wedged instance: a normal
    /// run produces the alarm condition, so the signal is not a classifier.
    Ambiguous { shared: Vec<String>, samples: usize },
    /// Healthy and wedged instances share no value: the signal separates
    /// them.
    Discriminates { samples: usize },
}

impl Discrimination {
    /// Whether this may be called evidence. Only a separating signal may
    /// (invariant 5).
    pub fn is_evidence(&self) -> bool {
        matches!(self, Discrimination::Discriminates { .. })
    }

    pub fn line(&self) -> String {
        match self {
            Discrimination::NoSamples => "signal untested: no samples".to_string(),
            Discrimination::Untestable { state, samples } => format!(
                "signal untestable: all {samples} samples are {state} — never presented \
                 with the condition it claims to detect"
            ),
            Discrimination::Constant { value, samples } => format!(
                "NOT EVIDENCE: signal constant ({value}) across {samples} samples of both \
                 states — it cannot tell working from wedged"
            ),
            Discrimination::Ambiguous { shared, samples } => format!(
                "NOT EVIDENCE: {} of {samples} values occur in healthy and wedged runs alike \
                 ({}) — normal operation produces the signal",
                shared.len(),
                shared.join(", ")
            ),
            Discrimination::Discriminates { samples } => {
                format!("signal separates states on {samples} samples")
            }
        }
    }
}

/// Invariant 5: measure a candidate signal against instances whose state is
/// known. A signal is evidence only when the healthy and wedged value sets
/// are disjoint; a constant value, or one shared across states, is not.
pub fn discriminate(samples: &[SignalSample]) -> Discrimination {
    if samples.is_empty() {
        return Discrimination::NoSamples;
    }
    let healthy: BTreeSet<&str> = samples
        .iter()
        .filter(|s| s.truth == InstanceState::Healthy)
        .map(|s| s.value.as_str())
        .collect();
    let wedged: BTreeSet<&str> = samples
        .iter()
        .filter(|s| s.truth == InstanceState::Wedged)
        .map(|s| s.value.as_str())
        .collect();
    if healthy.is_empty() {
        return Discrimination::Untestable {
            state: InstanceState::Wedged.as_str(),
            samples: samples.len(),
        };
    }
    if wedged.is_empty() {
        return Discrimination::Untestable {
            state: InstanceState::Healthy.as_str(),
            samples: samples.len(),
        };
    }
    let all: BTreeSet<&str> = samples.iter().map(|s| s.value.as_str()).collect();
    if all.len() == 1 {
        return Discrimination::Constant {
            value: all.first().copied().unwrap_or("").to_string(),
            samples: samples.len(),
        };
    }
    let shared: Vec<String> = healthy
        .intersection(&wedged)
        .map(|v| v.to_string())
        .collect();
    if !shared.is_empty() {
        return Discrimination::Ambiguous {
            shared,
            samples: samples.len(),
        };
    }
    Discrimination::Discriminates {
        samples: samples.len(),
    }
}

/// One window of fleet activity: how many slots were held, and how much work
/// completed while they were (invariant 6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FleetWindow {
    pub label: String,
    pub occupied: u32,
    pub capacity: u32,
    /// Units completed in the window — the companion metric the utilisation
    /// counter does not have.
    pub completed: u64,
}

impl FleetWindow {
    /// Utilisation percentage, the number the incident reported as 100.
    pub fn occupancy_percent(&self) -> u32 {
        if self.capacity == 0 {
            return 0;
        }
        (self.occupied as u64).saturating_mul(100) as u32 / self.capacity
    }

    pub fn at_cap(&self) -> bool {
        self.capacity > 0 && self.occupancy_percent() >= SATURATION_PERCENT
    }
}

/// Completions per window measured while the fleet was known to be working
/// (invariant 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThroughputBaseline {
    pub per_window: u64,
}

impl ThroughputBaseline {
    /// The collapse line: completions below this in a window are a
    /// degradation.
    pub fn collapse_floor(&self) -> u64 {
        self.per_window.saturating_mul(COLLAPSE_PERCENT) / 100
    }

    pub fn is_collapsed(&self, completed: u64) -> bool {
        completed < self.collapse_floor()
    }
}

/// Occupancy read beside throughput (invariant 6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Saturation {
    /// At cap and producing.
    OccupiedAndWorking {
        occupancy_percent: u32,
        completed: u64,
        baseline: u64,
    },
    /// At cap while completions collapsed: the fleet is occupied, not
    /// working. The utilisation counter on its own reports a healthy 100%.
    OccupiedNotWorking {
        occupancy_percent: u32,
        completed: u64,
        baseline: u64,
    },
    /// Not at cap: occupancy is not the constraint being reported.
    NotSaturated {
        occupancy_percent: u32,
        completed: u64,
        baseline: u64,
    },
}

impl Saturation {
    pub fn is_blind(&self) -> bool {
        matches!(self, Saturation::OccupiedNotWorking { .. })
    }

    /// One line, with both numbers on it, so "at cap" is never stated
    /// without the work count beside it.
    pub fn line(&self) -> String {
        match self {
            Saturation::OccupiedAndWorking {
                occupancy_percent,
                completed,
                baseline,
            } => format!(
                "{occupancy_percent}% occupied, throughput {completed} vs baseline {baseline} — working"
            ),
            Saturation::OccupiedNotWorking {
                occupancy_percent,
                completed,
                baseline,
            } => format!(
                "{occupancy_percent}% occupied, throughput {completed} vs baseline {baseline} — OCCUPIED, NOT WORKING"
            ),
            Saturation::NotSaturated {
                occupancy_percent,
                completed,
                baseline,
            } => format!(
                "{occupancy_percent}% occupied, throughput {completed} vs baseline {baseline}"
            ),
        }
    }
}

/// Invariant 6: read a utilisation counter beside a throughput baseline.
pub fn saturation(window: &FleetWindow, baseline: &ThroughputBaseline) -> Saturation {
    let (occ, done, base) = (
        window.occupancy_percent(),
        window.completed,
        baseline.per_window,
    );
    if !window.at_cap() {
        return Saturation::NotSaturated {
            occupancy_percent: occ,
            completed: done,
            baseline: base,
        };
    }
    if baseline.is_collapsed(done) {
        Saturation::OccupiedNotWorking {
            occupancy_percent: occ,
            completed: done,
            baseline: base,
        }
    } else {
        Saturation::OccupiedAndWorking {
            occupancy_percent: occ,
            completed: done,
            baseline: base,
        }
    }
}

/// What a **utilisation-only** series could see about a degradation the
/// throughput numbers show (invariant 6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompanionVerdict {
    /// No window fell below the collapse floor.
    NoDegradation { windows: usize },
    /// Every degraded window also moved off cap, so the utilisation counter
    /// alone would have shown something.
    VisibleInOccupancy {
        degraded_windows: usize,
        min_occupancy_percent: u32,
    },
    /// Degraded windows while occupancy stayed at cap in all of them: the
    /// utilisation counter is blind, and a companion throughput metric is
    /// mandatory.
    BlindAtCapacity {
        degraded_windows: usize,
        occupancy_percent: u32,
    },
}

impl CompanionVerdict {
    pub fn needs_companion_metric(&self) -> bool {
        matches!(self, CompanionVerdict::BlindAtCapacity { .. })
    }

    pub fn line(&self) -> String {
        match self {
            CompanionVerdict::NoDegradation { windows } => {
                format!("{windows} windows, throughput at or above baseline floor")
            }
            CompanionVerdict::VisibleInOccupancy {
                degraded_windows,
                min_occupancy_percent,
            } => format!(
                "{degraded_windows} degraded window(s), occupancy fell to {min_occupancy_percent}% — \
                 utilisation alone would have seen it"
            ),
            CompanionVerdict::BlindAtCapacity {
                degraded_windows,
                occupancy_percent,
            } => format!(
                "{degraded_windows} degraded window(s) with occupancy at {occupancy_percent}% — \
                 utilisation blind, throughput metric required"
            ),
        }
    }
}

/// Invariant 6: could the utilisation counter, on its own, have detected the
/// collapse the throughput series shows?
pub fn companion_verdict(
    windows: &[FleetWindow],
    baseline: &ThroughputBaseline,
) -> CompanionVerdict {
    let degraded: Vec<&FleetWindow> = windows
        .iter()
        .filter(|w| baseline.is_collapsed(w.completed))
        .collect();
    if degraded.is_empty() {
        return CompanionVerdict::NoDegradation {
            windows: windows.len(),
        };
    }
    let min_occ = degraded
        .iter()
        .map(|w| w.occupancy_percent())
        .min()
        .unwrap_or(0);
    if degraded.iter().all(|w| w.at_cap()) {
        CompanionVerdict::BlindAtCapacity {
            degraded_windows: degraded.len(),
            occupancy_percent: min_occ,
        }
    } else {
        CompanionVerdict::VisibleInOccupancy {
            degraded_windows: degraded.len(),
            min_occupancy_percent: min_occ,
        }
    }
}

/// Findings for the spec rule (invariant 7). Empty means the spec is
/// compliant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpecFinding {
    /// The declared run exceeds [`LONG_RUN_THRESHOLD`] and the spec has no
    /// `## Progress contract` section.
    MissingContract {
        expected_run_secs: u64,
        threshold_secs: u64,
    },
    /// The section exists but names no emission — no artifact, log file, or
    /// line the worker writes while running.
    NoEmission,
    /// The section exists but states no interval ("every 60s",
    /// "interval: 2m").
    NoInterval,
    /// The section names buffered-until-exit output as the progress source:
    /// not instrumentation (invariant 2).
    BufferedSource { phrase: &'static str },
}

impl SpecFinding {
    pub fn rule_id(&self) -> &'static str {
        match self {
            SpecFinding::MissingContract { .. } => "PROGRESS_CONTRACT_MISSING",
            SpecFinding::NoEmission => "PROGRESS_CONTRACT_NO_EMISSION",
            SpecFinding::NoInterval => "PROGRESS_CONTRACT_NO_INTERVAL",
            SpecFinding::BufferedSource { .. } => "PROGRESS_CONTRACT_BUFFERED_SOURCE",
        }
    }

    pub fn line(&self) -> String {
        match self {
            SpecFinding::MissingContract {
                expected_run_secs,
                threshold_secs,
            } => format!(
                "{}: spec declares a longest expected run of {}s (over {}s) with no \
                 '{PROGRESS_CONTRACT_SECTION}' section — state what the worker emits while \
                 running and how often",
                self.rule_id(),
                expected_run_secs,
                threshold_secs
            ),
            SpecFinding::NoEmission => format!(
                "{}: '{PROGRESS_CONTRACT_SECTION}' names no artifact or emission the worker \
                 produces while running",
                self.rule_id()
            ),
            SpecFinding::NoInterval => format!(
                "{}: '{PROGRESS_CONTRACT_SECTION}' states no emission interval \
                 (\"every <n><s|m|h>\")",
                self.rule_id()
            ),
            SpecFinding::BufferedSource { phrase } => format!(
                "{}: '{PROGRESS_CONTRACT_SECTION}' names a buffered source ({phrase}) — \
                 output flushed at exit emits nothing while the worker runs",
                self.rule_id()
            ),
        }
    }
}

/// Phrases that name a buffered-until-exit source in a spec (invariant 2).
const BUFFERED_PHRASES: &[&str] = &[
    "stdout at exit",
    "stdout on exit",
    "captured at exit",
    "captured on exit",
    "buffered until exit",
    "buffered on exit",
    "flushed at exit",
    "flushed on exit",
    "log written at exit",
    "log written on exit",
];

/// Tokens that name an emission in a progress-contract section.
const EMISSION_TOKENS: &[&str] = &[
    "artifact",
    "heartbeat",
    "progress",
    "emit",
    "write",
    "append",
    "log",
    "checkpoint",
    "marker",
];

/// Invariant 7: parse the declared longest expected run from a spec body.
/// Recognises `Longest expected run: 3h` / `2700s` / `45m` / bare seconds,
/// case-insensitively on the key.
pub fn declared_expected_run(body: &str) -> Option<Duration> {
    for line in body.lines() {
        let trimmed = line.trim().trim_start_matches('#').trim();
        let Some(rest) = strip_key(trimmed, EXPECTED_RUN_KEY) else {
            continue;
        };
        let value = rest.split_whitespace().next()?;
        if let Some(d) = parse_duration(value) {
            return Some(d);
        }
    }
    None
}

/// Case-insensitive prefix strip for a spec key.
fn strip_key<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    if line.len() < key.len() {
        return None;
    }
    let (head, tail) = line.split_at(key.len());
    if head.eq_ignore_ascii_case(key) {
        Some(tail.trim_start())
    } else {
        None
    }
}

/// A GNU-style duration with an optional `s`/`m`/`h`/`d` suffix.
fn parse_duration(token: &str) -> Option<Duration> {
    let (num, mult): (&str, u64) = if let Some(v) = token.strip_suffix('s') {
        (v, 1)
    } else if let Some(v) = token.strip_suffix('m') {
        (v, 60)
    } else if let Some(v) = token.strip_suffix('h') {
        (v, 3600)
    } else if let Some(v) = token.strip_suffix('d') {
        (v, 86400)
    } else {
        (token, 1)
    };
    let n: u64 = num.trim().parse().ok()?;
    n.checked_mul(mult).map(Duration::from_secs)
}

/// Extract a markdown section's body by heading (exact, case-insensitive
/// match on the heading text; body runs to the next heading of any level).
fn section_body(body: &str, heading: &str) -> Option<String> {
    let mut collected = String::new();
    let mut inside = false;
    for line in body.lines() {
        let trimmed = line.trim_end();
        if trimmed.trim_start().starts_with('#') {
            if inside {
                break;
            }
            if trimmed
                .trim_start()
                .trim_start_matches('#')
                .trim()
                .eq_ignore_ascii_case(heading.trim_start_matches('#').trim())
            {
                inside = true;
            }
            continue;
        }
        if inside {
            collected.push_str(trimmed);
            collected.push('\n');
        }
    }
    if inside {
        Some(collected)
    } else {
        None
    }
}

/// Does a progress-contract section state an emission interval? Looks for
/// `every <n><unit>` or `interval: <n><unit>` (case-insensitive).
fn states_interval(section: &str) -> bool {
    let lower = section.to_lowercase();
    ["every", "interval:"].iter().any(|key| {
        let mut search = lower.as_str();
        while let Some(idx) = search.find(key) {
            let rest = &search[idx + key.len()..];
            let token = rest.trim_start().split_whitespace().next().unwrap_or("");
            if parse_duration(token.trim_end_matches('.')).is_some() {
                return true;
            }
            search = rest;
        }
        false
    })
}

/// Does a progress-contract section name something the worker emits?
fn states_emission(section: &str) -> bool {
    let lower = section.to_lowercase();
    EMISSION_TOKENS.iter().any(|t| lower.contains(t))
        // a quoted path or file name also names the artifact
        || lower
            .split_whitespace()
            .any(|w| {
                (w.contains('.') && w.contains('/')) || w.ends_with(".log") || w.ends_with(".tsv")
            })
}

/// Findings for the spec rule (invariant 7): a spec declaring a
/// `Longest expected run:` above [`LONG_RUN_THRESHOLD`] must carry a
/// `## Progress contract` section that names an emission and an interval,
/// and whose source is not buffered output.
pub fn spec_progress_findings(spec_body: &str) -> Vec<SpecFinding> {
    let Some(run) = declared_expected_run(spec_body) else {
        return Vec::new(); // nothing claims a long-running worker
    };
    if run <= LONG_RUN_THRESHOLD {
        return Vec::new();
    }
    let Some(section) = section_body(spec_body, PROGRESS_CONTRACT_SECTION) else {
        return vec![SpecFinding::MissingContract {
            expected_run_secs: run.as_secs(),
            threshold_secs: LONG_RUN_THRESHOLD.as_secs(),
        }];
    };
    let mut findings = Vec::new();
    for phrase in BUFFERED_PHRASES {
        if section.to_lowercase().contains(phrase) {
            findings.push(SpecFinding::BufferedSource { phrase });
            break;
        }
    }
    if !states_emission(&section) {
        findings.push(SpecFinding::NoEmission);
    }
    if !states_interval(&section) {
        findings.push(SpecFinding::NoInterval);
    }
    findings
}
