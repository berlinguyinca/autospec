//! Progress contracts and liveness from positive artifacts (issue #4259).
//!
//! The incident, measured on a live cluster: 19 of 21 running agents were
//! stuck past their 45-minute limit — one by 6 h 50 m — each holding a GPU
//! worker slot while producing **zero bytes**. Every available signal reported
//! the fleet healthy:
//!
//! | signal | said | true? |
//! |---|---|---|
//! | scheduler job state | `RUNNING` | yes, and useless |
//! | agent count vs cap | 22/22 — fully utilised | yes, and actively misleading |
//! | wrapper `.out` file | exists, 264 bytes | yes — 264 bytes for every agent, healthy or hung |
//! | `agent.out` | 0 bytes | yes — 0 bytes for every agent, healthy or hung |
//!
//! Every one of those is true and none of them discriminates. A dashboard
//! assembled from any of them shows a healthy fleet at full utilisation while
//! the fleet does nothing. The work product itself (`changes.patch`) is
//! written at exit, so a nine-minute-old healthy agent and a seven-hour-old
//! hung one are byte-identical on disk.
//!
//! The invariants this module makes checkable:
//!
//! 1. **A long-running task must emit progress, or it is unobservable exactly
//!    when observation matters.** Redirecting stdout to a file is not a
//!    progress signal: the file's existence is created by the shell, and a
//!    buffer flushed at exit says nothing while the run is live.
//!    ([`instrumented`], [`observe_run`])
//! 2. **Detect liveness from a positive artifact, never from the absence of a
//!    crash.** What discriminated here was the marker written immediately
//!    after the model call returns: its *absence* long past the limit proves
//!    the call never returned. Absence of a known-next-step beats presence of
//!    a process. ([`step_liveness`], [`StepMarker`])
//! 3. **A check that could not run is never a green check.** Absence read from
//!    a directory that does not exist, cannot be listed, or was spelled wrong
//!    says nothing about the run. [`read_marker`] returns
//!    [`MarkerEvidence::CouldNotRun`] and [`step_liveness_at`] returns
//!    [`LivenessOutcome::Blocked`] naming the path, so a bad path cannot yield
//!    a verdict in either direction — neither "healthy" nor 19 false `stuck`
//!    alerts. ([`validate_marker_path`])
//! 4. **A detector must not be confoundable by buffering.** "The log is empty"
//!    fails that test — empty is the normal state of a working task. "Still
//!    inside the call after `limit + grace`" cannot be confounded, because it
//!    depends only on elapsed time and a file the *wrapper* writes.
//!    ([`Detector`], [`usable_for_liveness`])
//! 5. **Utilisation counters measure occupancy, not work.** "22/22 agents
//!    running" was the most trusted and most wrong metric on the board; any
//!    saturation metric needs a companion throughput metric — here patches
//!    produced per hour, which had collapsed to 2 while occupancy read 100%.
//!    ([`fleet_verdict`], [`FleetSignal`])
//!
//! And the spec-facing consequence: a spec that commissions a long-running
//! worker must state **what it emits while running and how often**, not only
//! what it produces at the end. "Writes `changes.patch` on success" is a
//! completion contract; without a progress contract a working instance is
//! indistinguishable from a wedged one for as long as the timeout allows —
//! seven hours per agent, across nineteen of them. ([`commission_verdict`])
//!
//! Everything here is pure: no I/O, no clock, no subprocesses. The caller
//! reads the byte counts, the marker files and the elapsed times, and this
//! module decides what those observations can and cannot support.

use serde::{Deserialize, Serialize};
use std::time::Duration;

// --- Invariant 1: buffered-until-exit output is not instrumentation -------

/// Where a task's output goes, and therefore whether anything is visible
/// while the task is still running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputSink {
    /// The task's stdout redirected to a file by the shell. Buffered until
    /// exit: 0 bytes for a working task and a wedged one alike, so the file
    /// is evidence only after the process is already gone.
    RedirectedStdout,
    /// A file the task flushes itself after each step, so bytes land on disk
    /// while the task is live.
    FlushPerStep,
    /// A file written by the wrapper rather than by the task, so it advances
    /// even when the task itself emits nothing.
    WrapperArtifact,
}

impl OutputSink {
    /// True when bytes can appear on disk before the task exits. A sink that
    /// cannot deliver bytes during the run can never carry a progress signal,
    /// whatever the contract claims about it.
    pub fn visible_while_running(&self) -> bool {
        matches!(self, Self::FlushPerStep | Self::WrapperArtifact)
    }
}

/// What a caller promises about observing a run while it is live.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgressContract {
    /// The longest the run may be silent before silence is evidence of
    /// something. Must be shorter than [`Self::run_limit`] or it never fires.
    pub max_silence: Duration,
    /// Where the progress emission actually goes.
    pub sink: OutputSink,
    /// The run's own deadline: how long it is allowed to take.
    pub run_limit: Duration,
}

/// Whether a contract makes the run observable during the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Instrumented {
    /// The sink delivers bytes while the task lives, and the interval is
    /// short enough to fire at least once inside the run.
    Observable,
    /// The sink is buffered until exit. No emission interval rescues it: the
    /// bytes exist only after the process is gone, which is exactly when no
    /// observation is needed.
    BufferedUntilExit,
    /// The sink works but the silence bound exceeds the run limit, so a
    /// healthy run is silent for its whole life and silence proves nothing.
    IntervalNeverFires {
        /// The promised maximum silence.
        max_silence: Duration,
        /// The run's own deadline.
        run_limit: Duration,
    },
}

impl Instrumented {
    /// True only for [`Instrumented::Observable`].
    pub fn is_observable(&self) -> bool {
        matches!(self, Self::Observable)
    }

    /// The one-line report, naming the defect in the contract rather than
    /// reporting a healthy-looking run.
    pub fn line(&self) -> String {
        match self {
            Self::Observable => "progress visible while running".to_string(),
            Self::BufferedUntilExit => "output is buffered until exit: 0 bytes for a working \
                 task and a wedged one alike — this is not instrumentation"
                .to_string(),
            Self::IntervalNeverFires {
                max_silence,
                run_limit,
            } => format!(
                "max silence {}s exceeds the run limit {}s: a healthy run is silent \
                 for its whole life",
                max_silence.as_secs(),
                run_limit.as_secs()
            ),
        }
    }
}

/// Decide whether a progress contract makes a run observable *during* the
/// run.
///
/// The fold this prevents: "stdout goes to a file, so we have progress". The
/// file is created by the shell at launch and filled by a buffer flushed at
/// exit — its existence and its size are both facts about the container of the
/// work, not about the work.
pub fn instrumented(contract: &ProgressContract) -> Instrumented {
    if !contract.sink.visible_while_running() {
        return Instrumented::BufferedUntilExit;
    }
    if contract.max_silence > contract.run_limit {
        return Instrumented::IntervalNeverFires {
            max_silence: contract.max_silence,
            run_limit: contract.run_limit,
        };
    }
    Instrumented::Observable
}

/// A single observation of a live run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunObservation {
    /// How long the run has been going.
    pub age: Duration,
    /// How long since the last progress emission that this contract can see.
    /// `None` means none has ever been seen.
    pub last_progress: Option<Duration>,
    /// Bytes currently in the output file. Recorded, never decisive: see
    /// [`instrumented`].
    pub output_bytes: u64,
}

/// What one observation of a live run supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunVerdict {
    /// A progress emission landed inside the contract's bound.
    Progressing,
    /// Silent longer than the contract allows, on a sink that would have
    /// shown bytes if the task were working. This is the signal the fleet
    /// never had.
    SilentBeyondContract {
        /// How long the silence has run.
        silent_for: Duration,
    },
    /// Inside the contract's bound: nothing to conclude yet.
    NotYetDue,
    /// The contract cannot support any verdict at all. A run under a
    /// buffered-until-exit sink reads this way whether it is healthy or
    /// wedged, which is the point.
    Unobservable,
}

impl RunVerdict {
    /// True when the observation actually says something.
    pub fn discriminating(&self) -> bool {
        !matches!(self, Self::Unobservable)
    }

    /// The one-line report.
    pub fn line(&self) -> String {
        match self {
            Self::Progressing => "progress emitted inside the contract bound".to_string(),
            Self::SilentBeyondContract { silent_for } => format!(
                "silent for {}s on a sink that would show work: suspect a wedged run",
                silent_for.as_secs()
            ),
            Self::NotYetDue => "inside the contract bound: no verdict yet".to_string(),
            Self::Unobservable => "no progress contract in force: a healthy run and a wedged \
                 one read identically"
                .to_string(),
        }
    }
}

/// Interpret one observation of a live run against its contract.
///
/// An unobservable contract never yields [`RunVerdict::SilentBeyondContract`],
/// however many bytes are missing: on a buffered sink, zero bytes is the
/// normal state of a working task, and a detector that fires on it fires on
/// everyone (invariant 4).
pub fn observe_run(contract: &ProgressContract, obs: &RunObservation) -> RunVerdict {
    if !instrumented(contract).is_observable() {
        return RunVerdict::Unobservable;
    }
    match obs.last_progress {
        Some(silent) if silent > contract.max_silence => {
            RunVerdict::SilentBeyondContract { silent_for: silent }
        }
        Some(_) => RunVerdict::Progressing,
        None if obs.age > contract.max_silence => RunVerdict::SilentBeyondContract {
            silent_for: obs.age,
        },
        None => RunVerdict::NotYetDue,
    }
}

// --- Invariant 2: liveness from a positive artifact -----------------------

/// A known next step whose completion writes an artifact the wrapper
/// controls. The artifact is what makes the step's completion *positive*
/// evidence; the step's beginning is not, because the process was already
/// running when it got there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepMarker {
    /// What the step is, e.g. `model-call`.
    pub name: String,
    /// The artifact written *after* the step returns, by the wrapper rather
    /// than by the task, e.g. `build.log`.
    pub artifact: String,
}

/// What the caller observed about one step of a live run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepObservation {
    /// How long the step has been running.
    pub elapsed: Duration,
    /// The step's own limit.
    pub limit: Duration,
    /// Extra slack past the limit before the step is called wedged.
    pub grace: Duration,
    /// Whether the completion artifact exists.
    pub marker_present: bool,
    /// Whether the scheduler still reports the job `RUNNING`, or the process
    /// is still alive. Recorded, never decisive.
    pub process_running: bool,
}

/// What a step's observed state means.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepLiveness {
    /// The completion artifact exists: the step returned. Positive evidence,
    /// independent of elapsed time.
    Completed,
    /// Inside the step's limit. Zero output here means nothing.
    WithinLimit,
    /// Past the limit but inside the grace window: late, not yet evidence.
    AwaitingGrace {
        /// How far past the limit the step has run.
        beyond_limit: Duration,
    },
    /// Past `limit + grace` with the completion artifact read as genuinely
    /// absent. The step never returned, whatever the job state says. The
    /// scheduler's `RUNNING` is true and useless: a wedged call holds its
    /// process forever. Absence from a directory that could not be read never
    /// reaches this variant — see [`MarkerEvidence::CouldNotRun`].
    WedgedInStep {
        /// How far past the limit the step has run.
        beyond_limit: Duration,
    },
}

impl StepLiveness {
    /// True when the verdict says something about liveness, as opposed to
    /// "too early to tell".
    pub fn discriminating(&self) -> bool {
        matches!(self, Self::Completed | Self::WedgedInStep { .. })
    }

    /// True when the step must be treated as dead: the only verdict that
    /// outranks a live process and a full GPU slot.
    pub fn wedged(&self) -> bool {
        matches!(self, Self::WedgedInStep { .. })
    }

    /// The one-line report, naming the marker whose absence carries the
    /// verdict.
    pub fn line(&self, marker: &StepMarker) -> String {
        match self {
            Self::Completed => format!("{} complete: {} exists", marker.name, marker.artifact),
            Self::WithinLimit => {
                format!("{} inside its limit: no output is normal here", marker.name)
            }
            Self::AwaitingGrace { beyond_limit } => format!(
                "{} past its limit by {}s, within grace — late, not yet evidence",
                marker.name,
                beyond_limit.as_secs()
            ),
            Self::WedgedInStep { beyond_limit } => format!(
                "{} still running {}s past its limit with no {}: the call never \
                 returned, job state notwithstanding",
                marker.name,
                beyond_limit.as_secs(),
                marker.artifact
            ),
        }
    }
}

impl StepObservation {
    fn beyond_limit(&self) -> Duration {
        self.elapsed.saturating_sub(self.limit)
    }
}

/// Decide a step's liveness from its completion artifact and elapsed time.
///
/// The marker is passed to [`StepLiveness::line`] rather than here: the
/// verdict depends only on the observation, the name is for the report.
///
/// The order matters. Presence of the artifact settles it positively; absence
/// only means something once `limit + grace` has passed, because before that
/// it is the normal state of a working step. The process state is never
/// consulted for a verdict — the incident had 19 live processes holding 19 GPU
/// slots and doing nothing.
pub fn step_liveness(obs: &StepObservation) -> StepLiveness {
    if obs.marker_present {
        return StepLiveness::Completed;
    }
    if obs.elapsed <= obs.limit {
        return StepLiveness::WithinLimit;
    }
    let beyond_limit = obs.beyond_limit();
    if beyond_limit <= obs.grace {
        return StepLiveness::AwaitingGrace { beyond_limit };
    }
    StepLiveness::WedgedInStep { beyond_limit }
}

// --- Invariant 3: a check that could not run is not a green check ---------

/// Where the marker lookup actually happened, as observed by the caller
/// rather than as intended by the config.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkerLocation {
    /// The directory was listed; `present` is the truth about the artifact.
    Scanned { present: bool },
    /// The directory does not exist: nothing was looked for.
    DirectoryMissing,
    /// The directory exists but could not be listed.
    Unreadable,
}

/// Why a marker check could not run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkerReadDefect {
    /// The configured path is empty, relative, or otherwise unusable.
    MalformedPath,
    /// The containing directory does not exist: the check was pointed at a
    /// location that is not the one holding the markers.
    DirectoryMissing,
    /// The directory exists but cannot be read.
    Unreadable,
}

impl MarkerReadDefect {
    /// The operator-facing reason.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::MalformedPath => "path is empty or relative",
            Self::DirectoryMissing => "containing directory does not exist",
            Self::Unreadable => "containing directory is unreadable",
        }
    }
}

/// The result of looking for a marker, before any claim about the run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkerEvidence {
    /// The artifact is there: the step completed.
    Present,
    /// The artifact is genuinely not there: absence is evidence.
    Absent,
    /// The check did not run. Absence here means nothing about the run, and
    /// reporting it as a verdict in either direction is the fold that turns a
    /// typo in a path into a green board — or into 19 false `stuck` alerts.
    CouldNotRun {
        /// The path that was asked for.
        path: String,
        /// Why it could not be consulted.
        reason: MarkerReadDefect,
    },
}

impl MarkerEvidence {
    /// True when the check actually consulted something.
    pub fn ran(&self) -> bool {
        !matches!(self, Self::CouldNotRun { .. })
    }

    /// The one-line report, naming the path when the check could not run.
    pub fn line(&self) -> String {
        match self {
            Self::Present => "marker present: step completed".to_string(),
            Self::Absent => "marker absent: read from a directory that exists".to_string(),
            Self::CouldNotRun { path, reason } => {
                format!("check could not run: {path} ({})", reason.as_str())
            }
        }
    }
}

/// Validate a marker path: absolute, with a usable parent.
pub fn validate_marker_path(path: &str) -> Result<(), MarkerReadDefect> {
    let trimmed = path.trim();
    if trimmed.is_empty() || !trimmed.starts_with('/') || trimmed == "/" || !trimmed.contains('/') {
        return Err(MarkerReadDefect::MalformedPath);
    }
    if trimmed.ends_with('/') {
        return Err(MarkerReadDefect::MalformedPath);
    }
    Ok(())
}

/// Turn a path plus a directory observation into evidence, failing closed.
///
/// A malformed path or a missing directory is `CouldNotRun`, never `Absent`:
/// the first real run of a check pointed at the wrong directory would
/// otherwise report every job wedged, and a check pointed at a directory that
/// is *supposed* to be empty would report everybody healthy.
pub fn read_marker(path: &str, location: MarkerLocation) -> MarkerEvidence {
    if validate_marker_path(path).is_err() {
        return MarkerEvidence::CouldNotRun {
            path: path.to_string(),
            reason: MarkerReadDefect::MalformedPath,
        };
    }
    match location {
        MarkerLocation::Scanned { present: true } => MarkerEvidence::Present,
        MarkerLocation::Scanned { present: false } => MarkerEvidence::Absent,
        MarkerLocation::DirectoryMissing => MarkerEvidence::CouldNotRun {
            path: path.to_string(),
            reason: MarkerReadDefect::DirectoryMissing,
        },
        MarkerLocation::Unreadable => MarkerEvidence::CouldNotRun {
            path: path.to_string(),
            reason: MarkerReadDefect::Unreadable,
        },
    }
}

/// A liveness outcome that cannot silently become a verdict when the check
/// could not run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LivenessOutcome {
    /// The check ran, and here is what it says.
    Verdict(StepLiveness),
    /// The check could not run: no verdict is available, in either direction.
    Blocked {
        /// The path that was asked for.
        path: String,
        /// Why it could not be consulted.
        reason: MarkerReadDefect,
    },
}

impl LivenessOutcome {
    /// The verdict, or `None` when the check never ran.
    pub fn verdict(&self) -> Option<StepLiveness> {
        match self {
            Self::Verdict(v) => Some(*v),
            Self::Blocked { .. } => None,
        }
    }

    /// True only when a verdict exists.
    pub fn ran(&self) -> bool {
        matches!(self, Self::Verdict(_))
    }

    /// The one-line report. A blocked check reads as a blocked check, not as
    /// `WithinLimit`.
    pub fn line(&self, marker: &StepMarker) -> String {
        match self {
            Self::Verdict(v) => v.line(marker),
            Self::Blocked { path, reason } => format!(
                "liveness check could not run: {} ({}) — no verdict about the run",
                path,
                reason.as_str()
            ),
        }
    }
}

/// Read a step's marker and decide liveness, failing closed when the path
/// cannot be consulted.
pub fn step_liveness_at(
    path: &str,
    location: MarkerLocation,
    obs: &StepObservation,
) -> LivenessOutcome {
    match read_marker(path, location) {
        MarkerEvidence::Present => LivenessOutcome::Verdict(StepLiveness::Completed),
        // Absence that was actually observed: the timing rules decide whether
        // it means anything yet.
        MarkerEvidence::Absent => LivenessOutcome::Verdict(step_liveness(&StepObservation {
            marker_present: false,
            ..*obs
        })),
        MarkerEvidence::CouldNotRun { path, reason } => LivenessOutcome::Blocked { path, reason },
    }
}

// --- Invariant 4: a detector must not be confoundable by buffering --------

/// A candidate liveness detector, described by the signal it reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Detector {
    /// A snapshot of the task's own output size ("the log is empty", "the log
    /// is 264 bytes"). Confoundable: empty and short are the normal states of
    /// a buffered, working task.
    OutputSnapshot {
        /// Bytes in the task's output file right now.
        bytes: u64,
    },
    /// The age of the wrapper log's mtime. Confoundable: a wrapper log that
    /// never grows has a fixed mtime for healthy and wedged runs alike, so
    /// "the log is stale" fires on everybody.
    OutputMtime {
        /// How old the wrapper log's mtime is.
        mtime_age: Duration,
    },
    /// "Still inside the call after `limit + grace`", judged from elapsed time
    /// and a marker the *wrapper* writes. Not confoundable: neither half of it
    /// depends on the task emitting anything.
    StepOverrun(StepObservation),
}

/// The class of confound behind an unusable detector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfoundedReason {
    /// Empty or short output is the normal state of a working task whose
    /// output is buffered until exit.
    Buffering,
    /// A file that never grows has a constant mtime for every run, so its age
    /// is not a staleness signal.
    FrozenMtime,
}

impl ConfoundedReason {
    /// The operator-facing explanation.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Buffering => {
                "empty output is the normal state of a working task: \
                 the buffer flushes at exit"
            }
            Self::FrozenMtime => {
                "the wrapper log never grows, so its mtime is identical \
                 for healthy and wedged runs"
            }
        }
    }
}

impl Detector {
    /// True when the signal cannot be produced by a healthy run, and therefore
    /// may be used for liveness at all.
    pub fn discriminates(&self) -> bool {
        matches!(self, Self::StepOverrun(_))
    }

    /// The confound that makes this detector unusable, if it is unusable.
    pub fn defect(&self) -> Option<ConfoundedReason> {
        match self {
            Self::OutputSnapshot { .. } => Some(ConfoundedReason::Buffering),
            Self::OutputMtime { .. } => Some(ConfoundedReason::FrozenMtime),
            Self::StepOverrun(_) => None,
        }
    }
}

/// The verdict a usable detector reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DetectionVerdict {
    /// Nothing outranks the run's own state: keep watching.
    NoEvidence,
    /// The run is wedged, from a signal a healthy run cannot produce.
    Stuck,
    /// The run finished the monitored step.
    Finished,
}

/// Run a detector, refusing the ones that cannot support the claim.
///
/// `Err` is the important half: a dashboard built on a confoundable detector
/// reports a healthy fleet at full utilisation, and the report is not false —
/// it is simply about something else.
pub fn usable_for_liveness(detector: &Detector) -> Result<DetectionVerdict, ConfoundedReason> {
    match detector {
        Detector::StepOverrun(obs) => Ok(match step_liveness(obs) {
            StepLiveness::Completed => DetectionVerdict::Finished,
            StepLiveness::WedgedInStep { .. } => DetectionVerdict::Stuck,
            _ => DetectionVerdict::NoEvidence,
        }),
        other => Err(other.defect().unwrap_or(ConfoundedReason::Buffering)),
    }
}

/// Screen a set of candidate detectors — what a dashboard builder runs before
/// wiring a panel. Returns one line per detector that must not be wired,
/// naming its confound.
pub fn reject_confoundable(detectors: &[Detector]) -> Vec<String> {
    detectors
        .iter()
        .filter(|d| !d.discriminates())
        .map(|d| {
            let reason = d
                .defect()
                .map(|r| r.as_str())
                .unwrap_or("signal cannot be produced by a working run");
            format!("{d:?} is not a liveness signal: {reason}")
        })
        .collect()
}

// --- Invariant 5: occupancy is not work -----------------------------------

/// A fleet snapshot pairing the saturation metric with its throughput
/// companion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FleetSignal {
    /// Agents currently occupying slots.
    pub running: u32,
    /// Slots available. `0` means occupancy is unknown, not 0%.
    pub capacity: u32,
    /// Work products completed in the last hour (patches produced per hour).
    pub throughput_per_hour: u32,
    /// What a full fleet at normal pace produces per hour. `0` means the
    /// companion metric does not exist.
    pub expected_per_hour: u32,
}

/// What a fleet snapshot means.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FleetVerdict {
    /// Occupancy and throughput agree that work is being done.
    Working,
    /// Slots are full and output has collapsed: the agents are occupying
    /// workers, not doing work. This is the incident — 22/22 occupied,
    /// 2 patches per hour against an expected 22.
    OccupiedNotWorking {
        /// Occupancy as a whole percentage.
        occupancy_pct: u32,
    },
    /// A saturation figure with no throughput companion, so the saturation
    /// figure must not be read as health.
    NoThroughputCompanion,
    /// Neither capacity nor a companion baseline is known: nothing to say.
    Unknown,
}

impl FleetVerdict {
    /// True when the snapshot shows work rather than occupancy.
    pub fn working(&self) -> bool {
        matches!(self, Self::Working)
    }

    /// The one-line report: occupancy and throughput on the same line, so a
    /// reader cannot take one without the other.
    pub fn line(&self, signal: &FleetSignal) -> String {
        let occupancy = signal
            .occupancy_pct()
            .map(|p| format!("{p}%"))
            .unwrap_or_else(|| "unknown".to_string());
        match self {
            Self::Working => format!(
                "{}/{} slots ({} occupied), {} work products/h — working",
                signal.running, signal.capacity, occupancy, signal.throughput_per_hour
            ),
            Self::OccupiedNotWorking { occupancy_pct } => format!(
                "{}/{} slots ({}% occupied) but only {}/h against {} expected — occupancy \
                 is not work",
                signal.running,
                signal.capacity,
                occupancy_pct,
                signal.throughput_per_hour,
                signal.expected_per_hour
            ),
            Self::NoThroughputCompanion => format!(
                "{}/{} slots ({} occupied) with no throughput baseline: this is an \
                 occupancy reading, not a health reading",
                signal.running, signal.capacity, occupancy
            ),
            Self::Unknown => "capacity or throughput unknown — no fleet verdict".to_string(),
        }
    }
}

impl FleetSignal {
    /// Occupancy as a whole percentage, or `None` when capacity is unknown.
    /// A zero capacity is never reported as 0% occupied.
    pub fn occupancy_pct(&self) -> Option<u32> {
        if self.capacity == 0 {
            return None;
        }
        Some((self.running as u64 * 100 / self.capacity as u64) as u32)
    }

    /// True when the fleet is saturated enough that occupancy alone looks
    /// healthy — the number that was most trusted and most wrong.
    pub fn saturated(&self) -> bool {
        self.occupancy_pct()
            .map(|pct| pct >= SATURATION_PCT)
            .unwrap_or(false)
    }

    /// True when throughput has collapsed relative to the companion baseline,
    /// i.e. below [`OUTPUT_COLLAPSE_NUMERATOR`] / [`OUTPUT_COLLAPSE_DENOMINATOR`] of it.
    pub fn output_collapsed(&self) -> bool {
        if self.expected_per_hour == 0 {
            return false;
        }
        self.throughput_per_hour as u64 * OUTPUT_COLLAPSE_DENOMINATOR
            < self.expected_per_hour as u64 * OUTPUT_COLLAPSE_NUMERATOR
    }
}

/// Occupancy at or above this percentage reads as "fully utilised" on a
/// dashboard.
pub const SATURATION_PCT: u32 = 90;

/// Numerator of the collapse fraction: output is collapsed when throughput
/// falls below NUMERATOR/DENOMINATOR of the companion baseline (half).
pub const OUTPUT_COLLAPSE_NUMERATOR: u64 = 1;
/// Denominator of [`OUTPUT_COLLAPSE_NUMERATOR`].
pub const OUTPUT_COLLAPSE_DENOMINATOR: u64 = 2;

/// Pair an occupancy reading with its throughput companion.
///
/// A saturation metric with no companion baseline is reported as
/// [`FleetVerdict::NoThroughputCompanion`] rather than as health: "22/22
/// running" is a true statement about slots that a wedged fleet satisfies
/// perfectly.
pub fn fleet_verdict(signal: &FleetSignal) -> FleetVerdict {
    if signal.capacity == 0 {
        return FleetVerdict::Unknown;
    }
    if signal.expected_per_hour == 0 {
        return FleetVerdict::NoThroughputCompanion;
    }
    if signal.saturated() && signal.output_collapsed() {
        return FleetVerdict::OccupiedNotWorking {
            occupancy_pct: signal.occupancy_pct().unwrap_or(0),
        };
    }
    FleetVerdict::Working
}

// --- The spec-facing half: a progress contract is commissioned too --------

/// Who writes the progress emission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Emitter {
    /// The task itself. Only observable if the task flushes: see
    /// [`OutputSink`].
    Task,
    /// The wrapper, outside the task. Observable even when the task emits
    /// nothing, which is the case that matters.
    Wrapper,
}

/// What a commissioned worker emits while it runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgressEmission {
    /// The artifact the emission lands in.
    pub artifact: String,
    /// How often it is written.
    pub every: Duration,
    /// Who writes it.
    pub emitter: Emitter,
    /// Where the bytes go.
    pub sink: OutputSink,
}

impl ProgressEmission {
    /// View the emission as a [`ProgressContract`] for the given run limit, so
    /// the run-time and spec-time checks share one implementation.
    pub fn as_contract(&self, run_limit: Duration) -> ProgressContract {
        ProgressContract {
            max_silence: self.every,
            sink: self.sink,
            run_limit,
        }
    }
}

/// A spec's contract for one long-running worker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerCommission {
    /// How long the worker is allowed to run.
    pub run_limit: Duration,
    /// The completion artifact — "writes `changes.patch` on success".
    pub completion_artifact: String,
    /// What it emits while running, if the spec said.
    pub emits_while_running: Option<ProgressEmission>,
}

/// Whether a spec commissioned a progress contract, not only a completion
/// contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommissionVerdict {
    /// The worker states what it emits while running, on an interval that
    /// fires inside its own limit, into a sink that delivers it.
    ContractComplete,
    /// A run longer than the caller's watch horizon with no progress contract
    /// at all: the worker is unobservable for the whole window in which
    /// observation would matter.
    CompletionOnly {
        /// The run limit the worker was given.
        run_limit: Duration,
    },
    /// A progress contract exists but describes a sink or interval that can
    /// never deliver a signal.
    ContractInert(Instrumented),
    /// Short enough that a completion contract is acceptable.
    ShortRun,
}

impl CommissionVerdict {
    /// True when the spec may commission the worker as written.
    pub fn complete(&self) -> bool {
        matches!(self, Self::ContractComplete | Self::ShortRun)
    }

    /// The one-line report the spec reviewer sees.
    pub fn line(&self) -> String {
        match self {
            Self::ContractComplete => "progress contract states emission and interval".into(),
            Self::CompletionOnly { run_limit } => format!(
                "completion contract only ({run_limit_secs}s run): the spec must state \
                 what the worker emits while running and how often",
                run_limit_secs = run_limit.as_secs()
            ),
            Self::ContractInert(inner) => format!("progress contract is inert: {}", inner.line()),
            Self::ShortRun => {
                "run is inside the watch horizon: completion contract is enough".into()
            }
        }
    }
}

/// Check a spec's contract for a long-running worker.
///
/// `watch_horizon` is the caller's number — how long a human or a pipeline
/// will wait without looking — not an invented one. A run expected to exceed
/// it needs a progress contract; a shorter one may ship with completion only.
pub fn commission_verdict(
    commission: &WorkerCommission,
    watch_horizon: Duration,
) -> CommissionVerdict {
    if commission.run_limit <= watch_horizon {
        return CommissionVerdict::ShortRun;
    }
    let Some(emission) = &commission.emits_while_running else {
        return CommissionVerdict::CompletionOnly {
            run_limit: commission.run_limit,
        };
    };
    match instrumented(&emission.as_contract(commission.run_limit)) {
        Instrumented::Observable => CommissionVerdict::ContractComplete,
        inert => CommissionVerdict::ContractInert(inert),
    }
}
