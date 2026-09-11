//! The gate runs the toolchain the repository declared (issue #4303).
//!
//! A CI step passed `toolchain: stable` to the toolchain setup action while
//! the repository pinned `1.91.0` in `rust-toolchain.toml` — the only
//! toolchain on the HPC cluster the agents run on. `stable` had moved past
//! `1.91.0` and carried lints that do not exist there
//! (`manual_checked_division`, `truncating_to_zero_length`), so the gate
//! failed on **every input** — and the standing instruction to ignore the
//! gate made a broken gate look like a quality signal.
//!
//! The common fact: a pin is a contract between the repository and every
//! place its code is built. A gate that names its own toolchain has opted
//! out of that contract *silently* — nothing in the failure output says
//! which toolchain ran, so the failure is read as "the code is bad"
//! instead of "the gate ran the wrong toolchain". And once a gate is red
//! on every input, its redness stops measuring quality and starts
//! measuring the gate itself: a check that has never passed is a defect
//! report, and a standing instruction to ignore it is a bug report.
//!
//! Five rules, each checkable:
//!
//! 1. **A gate runs the toolchain the repository declared.** If a pin file
//!    exists, no gate step may name a different channel — a pin that a
//!    gate can override is decoration ([`pin_drift`]).
//! 2. **Drift is directional, and never "stable is fine".** A step running
//!    *newer* than the pin has made an upgrade decision by accident, with
//!    the upgrade's work (new lints, formatting churn) buried in the gate
//!    noise; the finding names both channels and the pin file
//!    ([`DriftDirection`]).
//! 3. **A check that has never passed is a defect report, not a quality
//!    signal.** Red on every run of the default branch should page someone;
//!    "it always fails" is a diagnosis nobody filed ([`gate_standing`]).
//! 4. **A pin upgrade is a decision with work attached.** It is recorded —
//!    an issue or PR naming the change and the work it entails — or it is a
//!    decision made by accident ([`pin_change_verdict`]).
//! 5. **A standing instruction to ignore a signal is a bug report.** The
//!    workaround is the evidence that the gate is defective; it names the
//!    filed issue, or it is itself the diagnosis
//!    ([`workaround_verdict`]).

use std::cmp::Ordering;
use std::fmt::{self, Display};

/// A toolchain version with two or three components; the patch component
/// defaults to `0` when the channel names only `major.minor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ToolchainVersion {
    pub major: u16,
    pub minor: u16,
    pub patch: u16,
}

impl ToolchainVersion {
    pub fn new(major: u16, minor: u16, patch: u16) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }
}

impl Display for ToolchainVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// A moving channel: it changes on its own schedule, independent of the
/// repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChannelKind {
    Stable,
    Beta,
    Nightly,
}

impl ChannelKind {
    pub fn name(self) -> &'static str {
        match self {
            ChannelKind::Stable => "stable",
            ChannelKind::Beta => "beta",
            ChannelKind::Nightly => "nightly",
        }
    }
}

/// A toolchain channel as a pin file or a CI step names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolchainChannel {
    /// An exact version, e.g. `1.91.0`.
    Version(ToolchainVersion),
    /// A moving channel (`stable` / `beta` / `nightly`).
    Floating(ChannelKind),
}

impl Display for ToolchainChannel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ToolchainChannel::Version(v) => write!(f, "{v}"),
            ToolchainChannel::Floating(kind) => write!(f, "{}", kind.name()),
        }
    }
}

/// Parse a channel string: a `major.minor[.patch]` version, or one of
/// `stable` / `beta` / `nightly` (an optional `-<target-triple>` suffix is
/// ignored). Anything else is an error — a channel this cannot classify is
/// one it must not guess at (so `1.91.0-beta.2` is rejected, not read as
/// `1.91.0`).
pub fn parse_channel(s: &str) -> Result<ToolchainChannel, String> {
    if let Some((prefix, suffix)) = s.split_once('-') {
        // A `-` suffix is a target triple (`stable-x86_64-unknown-linux-gnu`),
        // which has at least two components. A one-component suffix is a
        // pre-release or qualifier this cannot classify.
        if !suffix.contains('-') {
            return Err(format!(
                "unrecognized toolchain channel '{s}': a pre-release or qualifier this \"
                 cannot classify — refusing to guess"
            ));
        }
        return parse_channel(prefix);
    }
    if let Some(version) = parse_version(s) {
        return Ok(ToolchainChannel::Version(version));
    }
    match s {
        "stable" => Ok(ToolchainChannel::Floating(ChannelKind::Stable)),
        "beta" => Ok(ToolchainChannel::Floating(ChannelKind::Beta)),
        "nightly" => Ok(ToolchainChannel::Floating(ChannelKind::Nightly)),
        _ => Err(format!(
            "unrecognized toolchain channel '{s}': expected a major.minor[.patch] version or \
             stable/beta/nightly — refusing to guess"
        )),
    }
}

fn parse_version(s: &str) -> Option<ToolchainVersion> {
    let parts: Vec<&str> = s.split('.').collect();
    if !(2..=3).contains(&parts.len()) {
        return None;
    }
    let mut nums = [0u16; 3];
    for (i, part) in parts.iter().enumerate().take(3) {
        if part.is_empty() || part.len() > 4 || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        nums[i] = part.parse().ok()?;
    }
    Some(ToolchainVersion {
        major: nums[0],
        minor: nums[1],
        patch: nums[2],
    })
}

/// The repository's toolchain pin (e.g. `rust-toolchain.toml`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinFile {
    /// The path the pin was read from — the finding must name it.
    pub path: String,
    pub channel: ToolchainChannel,
}

impl PinFile {
    /// Parse the `channel = "..."` key out of `rust-toolchain.toml`
    /// contents (top-level or under `[toolchain]`; comment and blank lines
    /// are skipped). A missing or duplicated `channel` key is an error: a
    /// pin file without exactly one classifiable channel is not a pin.
    pub fn from_toml(path: &str, contents: &str) -> Result<Self, String> {
        let mut found: Option<ToolchainChannel> = None;
        for line in contents.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
                continue;
            }
            let Some(eq) = line.find('=') else {
                continue;
            };
            if line[..eq].trim() != "channel" {
                continue;
            }
            let value = line[eq + 1..].trim();
            let unquoted = value
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .ok_or_else(|| {
                    format!("{path}: `channel` value must be a quoted string, got {value:?}")
                })?;
            if found.is_some() {
                return Err(format!(
                    "{path}: duplicate `channel` key; a pin names exactly one channel"
                ));
            }
            let channel = parse_channel(unquoted).map_err(|e| format!("{path}: {e}"))?;
            found = Some(channel);
        }
        found
            .map(|channel| Self {
                path: path.to_string(),
                channel,
            })
            .ok_or_else(|| {
                format!("{path}: no `channel` key; a pin file without a channel is not a pin")
            })
    }
}

/// What a gate step asks the toolchain setup action to install.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepToolchain {
    /// The step names no toolchain: it inherits the repository pin.
    Unspecified,
    /// The step names a channel explicitly — an override of whatever the
    /// repository declares.
    Channel(ToolchainChannel),
}

/// How a step's requested channel relates to the pin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriftDirection {
    /// The step runs a *newer* toolchain than the pin: an upgrade made by
    /// accident, with the upgrade's work (new lints, formatting churn)
    /// buried in the gate noise.
    Newer,
    /// The step runs an *older* toolchain than the pin.
    Older,
    /// At least one side is a moving channel: the step's toolchain moves
    /// on its own schedule, independent of the pin.
    MovesIndependently,
}

/// Rule 1's outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Drift {
    /// The gate runs the toolchain the repository declared.
    None,
    /// The gate runs a toolchain the repository did not declare.
    Overridden {
        pinned: ToolchainChannel,
        requested: ToolchainChannel,
        direction: DriftDirection,
    },
}

fn relate(pinned: ToolchainChannel, requested: ToolchainChannel) -> DriftDirection {
    match (pinned, requested) {
        (ToolchainChannel::Version(p), ToolchainChannel::Version(r)) => match p.cmp(&r) {
            Ordering::Less => DriftDirection::Newer,
            Ordering::Greater => DriftDirection::Older,
            Ordering::Equal => unreachable!("equal channels are no drift"),
        },
        _ => DriftDirection::MovesIndependently,
    }
}

/// Rule 1: if the pin exists, a step that names a different channel
/// drifts. A step that names the pin exactly — or names nothing — runs
/// the toolchain the repository declared.
pub fn pin_drift(pin: &PinFile, step: &StepToolchain) -> Drift {
    match step {
        StepToolchain::Unspecified => Drift::None,
        StepToolchain::Channel(requested) => {
            if *requested == pin.channel {
                Drift::None
            } else {
                Drift::Overridden {
                    pinned: pin.channel,
                    requested: *requested,
                    direction: relate(pin.channel, *requested),
                }
            }
        }
    }
}

/// The rendered finding for a drifting step: it names both channels, the
/// pin file, and the direction — so the failure can no longer be read as
/// "the code is bad".
pub fn drift_line(pin: &PinFile, step: &str, drift: &Drift) -> String {
    match drift {
        Drift::None => format!(
            "step '{step}' runs the toolchain the repository declared ('{}' via {})",
            pin.channel, pin.path
        ),
        Drift::Overridden {
            pinned,
            requested,
            direction,
        } => {
            let (explanation, remedy) = match direction {
                DriftDirection::Newer => (
                    "the step is running a newer toolchain than the pin — an upgrade made by \
                     accident: a pin bump is a decision with work attached (new lints, \
                     formatting churn), and no decision recorded it",
                    "remove the override, or bump the pin as a recorded decision",
                ),
                DriftDirection::Older => (
                    "the step is running an older toolchain than the pin — the gate is checking \
                     something the repository no longer declares",
                    "remove the override",
                ),
                DriftDirection::MovesIndependently => (
                    "the step is running a moving channel against a pinned one — its toolchain \
                     moves on its own schedule, independent of the pin",
                    "remove the override",
                ),
            };
            format!(
                "step '{step}' names toolchain '{requested}' but the repository declares \
                 '{pinned}' in {} — {explanation}. {remedy}; the pin is the single source of truth",
                pin.path
            )
        }
    }
}

/// One gate's run history on the default branch, oldest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateRecord {
    pub name: String,
    /// Pass (`true`) or fail (`false`) for each run.
    pub runs: Vec<bool>,
}

/// Rule 3's outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateStanding {
    /// No runs recorded yet.
    Unobserved,
    /// The gate has passed at least once: its verdicts measure the code.
    QualitySignal { greens: usize, runs: usize },
    /// The gate has never passed: it is a defect report, not a quality
    /// signal. Red on every run of the default branch should page
    /// someone; "it always fails" is a diagnosis nobody filed.
    DefectReport { runs: usize },
}

pub fn gate_standing(record: &GateRecord) -> GateStanding {
    let runs = record.runs.len();
    if runs == 0 {
        return GateStanding::Unobserved;
    }
    let greens = record.runs.iter().filter(|g| **g).count();
    if greens == 0 {
        GateStanding::DefectReport { runs }
    } else {
        GateStanding::QualitySignal { greens, runs }
    }
}

/// The rendered line: a defect report says what it is and what it is not.
pub fn standing_line(record: &GateRecord) -> String {
    match gate_standing(record) {
        GateStanding::Unobserved => {
            format!(
                "gate '{}' has no runs on the default branch yet",
                record.name
            )
        }
        GateStanding::QualitySignal { greens, runs } => format!(
            "gate '{}' is a quality signal: passed {greens} of {runs} runs on the default branch",
            record.name
        ),
        GateStanding::DefectReport { runs } => format!(
            "gate '{}' is red on all {runs} runs of the default branch: a check that has never \
             passed is a defect report, not a quality signal — it is describing a broken gate, \
             and \"it always fails\" is a diagnosis nobody filed",
            record.name
        ),
    }
}

/// Rule 4's input: a change to the pin channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PinChange {
    pub from: ToolchainChannel,
    pub to: ToolchainChannel,
    /// The decision is recorded: an issue or PR names the change and the
    /// work it entails.
    pub decision_recorded: bool,
}

/// Rule 4's outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinChangeVerdict {
    /// The pin did not move.
    Unchanged,
    /// A bump with its decision and work recorded.
    RecordedBump,
    /// A bump with no decision record: a toolchain upgrade made by accident.
    UnrecordedBump { direction: DriftDirection },
}

pub fn pin_change_verdict(change: &PinChange) -> PinChangeVerdict {
    if change.from == change.to {
        PinChangeVerdict::Unchanged
    } else if change.decision_recorded {
        PinChangeVerdict::RecordedBump
    } else {
        PinChangeVerdict::UnrecordedBump {
            direction: relate(change.from, change.to),
        }
    }
}

/// Rule 5's input: a standing instruction that a gate's signal be ignored
/// (an advisory classification, an `ignore` on the check, a note telling
/// people not to trust the gate).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workaround {
    pub gate: String,
    /// The instruction, as written.
    pub instruction: String,
    /// The filed bug report the workaround points at — the issue the
    /// gate's failure produced.
    pub bug_report: Option<String>,
}

/// Rule 5's outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkaroundVerdict {
    /// The workaround names the filed bug report: the signal was read as
    /// evidence and the evidence was filed.
    Filed,
    /// A standing ignore with no bug report: the workaround IS the bug
    /// report — the only diagnosis anyone has, and it is not filed.
    UnfiledBugReport,
}

pub fn workaround_verdict(workaround: &Workaround) -> WorkaroundVerdict {
    if workaround.bug_report.is_some() {
        WorkaroundVerdict::Filed
    } else {
        WorkaroundVerdict::UnfiledBugReport
    }
}

/// The rendered line: a workaround without a report says so plainly.
pub fn workaround_line(workaround: &Workaround) -> String {
    match &workaround.bug_report {
        Some(report) => format!(
            "workaround on gate '{}' points at filed bug report '{report}': the signal was read \
             as evidence and the evidence was filed",
            workaround.gate
        ),
        None => format!(
            "standing instruction to ignore gate '{}' (\"{}\") with no bug report: a standing \
             instruction to ignore a signal is a bug report — file the issue it is silently \
             standing in for",
            workaround.gate, workaround.instruction
        ),
    }
}

/// One toolchain-bearing step of a CI workflow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowStep {
    /// The step's id — the finding must name it.
    pub id: String,
    pub toolchain: StepToolchain,
}

/// A CI workflow that sets up the Rust toolchain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workflow {
    /// The workflow path — the finding must name it.
    pub path: String,
    pub steps: Vec<WorkflowStep>,
}

/// One audit finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// The rule id: `TOOLCHAIN_GATE_DRIFT`.
    pub rule: &'static str,
    /// `<workflow path>: <drift line>`.
    pub detail: String,
}

/// Audit every toolchain-bearing step of every workflow against the pin.
/// Clean workflows (steps that name the pin or name nothing) produce no
/// findings.
pub fn audit(pin: &PinFile, workflows: &[Workflow]) -> Vec<Finding> {
    let mut findings = Vec::new();
    for workflow in workflows {
        for step in &workflow.steps {
            let drift = pin_drift(pin, &step.toolchain);
            if let Drift::Overridden { .. } = drift {
                findings.push(Finding {
                    rule: "TOOLCHAIN_GATE_DRIFT",
                    detail: format!("{}: {}", workflow.path, drift_line(pin, &step.id, &drift)),
                });
            }
        }
    }
    findings
}
