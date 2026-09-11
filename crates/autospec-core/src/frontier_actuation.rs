//! Actuation gaps: a loop that decides but cannot act (issue #4268).
//!
//! The frontier loop computed "ready" — issues whose dependencies were all
//! closed — and the cluster reported the count every pass: **8 ready, 0
//! dispatched**. Both numbers were true, and together they hid the one fact
//! that mattered: dispatch was *impossible*. Dispatching an issue writes its
//! body as a staged spec and starts a worker on it, and the script that
//! staged specs (`iw-stage.sh`) was not part of the loop. It was a manual
//! step, run by whoever remembered to run it. Every dependency closure was
//! "ready" and nothing could ever be dispatched, forever, and the report was
//! accurate the whole time.
//!
//! A report that is accurate but incomplete is not a safe report: "N ready,
//! 0 dispatched" reads as an idle loop to anyone reading it, and the state
//! was a permanent stop. Three invariants, each a primitive here:
//!
//! 1. **A loop that reports a decision must also report whether it could act
//!    on it** — [`TickReport`] carries the decided count (`ready`), the done
//!    count (`dispatched`) and every item in between with the reason it is
//!    held; [`TickReport::line`] renders all three and
//!    [`line_names_gap`] refuses a line whose decided count differs from its
//!    done count without naming the gap and its reason.
//! 2. **Every precondition an actuator enforces needs a producer** —
//!    [`Precondition`] pairs a [`Guard`] with the component that satisfies it,
//!    [`Actuator::producerless`] lists the guards that have none, and a guard
//!    with no producer is a [`LoopVerdict::PermanentStop`], not a safety
//!    check: nothing in the system can ever satisfy it, so the hold has no
//!    release.
//! 3. **Test the loop end to end against a real new item, not only the
//!    steady state** — see `crates/autospec-core/tests/frontier_actuation.rs`:
//!    the incident is reconstructed (a pre-staged backlog the loop could
//!    dispatch, plus one brand-new issue nobody staged) and the fix is
//!    exercised by running the loop's own pass, producer included.
//!
//! The durable fix is the producer inside the loop's pass: [`FrontierLoop::tick`]
//! runs the producer for an unsatisfied guard *before* deciding the item is
//! blocked, so staging is not a step a person has to remember. The producer is
//! still a refusal, not a bypass: [`stage`] refuses a body too short to
//! implement from rather than handing an agent nothing to work from, and that
//! refusal is reported as a block naming its reason (`producerless: false` —
//! a real refusal is a state, a missing producer is a stop).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// The shortest body a staged spec may carry. Below this the body is a title
/// restated, not a task, and the producer refuses to stage it: dispatching an
/// agent onto an empty spec is the failure the guard exists to prevent, so
/// satisfying the guard must not be the thing that causes it.
pub const MIN_STAGED_SPEC_BYTES: usize = 64;

/// A precondition an actuator enforces before it can act on an item. The
/// guard is the *state* required; [`Precondition`] pairs it with the producer
/// of that state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Guard {
    /// Dispatch requires a staged spec for the item (its body written into
    /// the project's `specs/` directory).
    StagedSpec,
}

impl Guard {
    pub fn as_str(&self) -> &'static str {
        match self {
            Guard::StagedSpec => "staged-spec",
        }
    }

    /// Why an item is held while this guard is unsatisfied. This is the text
    /// the incident's report omitted: it existed nowhere in its output.
    pub fn block_reason(&self) -> &'static str {
        match self {
            Guard::StagedSpec => "no staged spec",
        }
    }

    /// Whether this guard holds for `issue` in `world`.
    pub fn satisfied(&self, world: &World, issue: &str) -> bool {
        match self {
            Guard::StagedSpec => world.staged.contains(issue),
        }
    }

    /// The producer's action that satisfies this guard for `issue`. Staging
    /// the body as a spec is the only producer action in this deployment; a
    /// second guard kind needs its own action added here, and a guard added
    /// without one has no way to ever hold.
    fn satisfy(&self, world: &mut World, issue: &str) {
        match self {
            Guard::StagedSpec => {
                world.staged.insert(issue.to_string());
            }
        }
    }
}

/// A guard paired with the component that produces the state it checks
/// (`iw-stage.sh` for [`Guard::StagedSpec`]). `producer: None` is not a
/// stricter guard; it is a stop with no release.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Precondition {
    pub guard: Guard,
    /// The owner that satisfies the guard. `None` — a guard with no producer
    /// — is a permanent stop, named by [`Actuator::producerless`].
    pub producer: Option<String>,
}

impl Precondition {
    /// A guard asserted with nothing that satisfies it. Legal to construct
    /// because it is the incident's shape and must be representable to be
    /// detected; [`Precondition::has_producer`] is the check.
    pub fn guarded(guard: Guard) -> Self {
        Precondition {
            guard,
            producer: None,
        }
    }

    /// A guard wired to its producer, so the loop can satisfy it in its own
    /// pass instead of waiting on a person.
    pub fn with_producer(guard: Guard, producer: impl Into<String>) -> Self {
        Precondition {
            guard,
            producer: Some(producer.into()),
        }
    }

    pub fn has_producer(&self) -> bool {
        self.producer.is_some()
    }

    /// A guard with no producer can hold forever: that is a stop, not a
    /// safety check.
    pub fn permanent_stop(&self) -> bool {
        !self.has_producer()
    }
}

/// The thing that acts on decided work (dispatch), and what it refuses to act
/// without.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Actuator {
    pub name: String,
    pub preconditions: Vec<Precondition>,
}

impl Actuator {
    pub fn new(name: impl Into<String>) -> Self {
        Actuator {
            name: name.into(),
            preconditions: Vec::new(),
        }
    }

    /// Builder: add a precondition to the actuator.
    pub fn requiring(mut self, precondition: Precondition) -> Self {
        self.preconditions.push(precondition);
        self
    }

    /// Invariant 2, made mechanical: the guards this actuator enforces that no
    /// component produces. Each one is a permanent stop waiting for an item to
    /// need it.
    pub fn producerless(&self) -> Vec<&Precondition> {
        self.preconditions
            .iter()
            .filter(|p| p.permanent_stop())
            .collect()
    }

    /// Findings as lines for the loop's own report/log: each names the
    /// actuator, the guard, and that the missing producer is a stop.
    pub fn findings(&self) -> Vec<String> {
        self.producerless()
            .iter()
            .map(|p| {
                format!(
                    "'{name}' guards '{guard}' with no producer: nothing can satisfy it, so the guard is a permanent stop, not a safety check",
                    name = self.name,
                    guard = p.guard.as_str()
                )
            })
            .collect()
    }
}

/// An item the loop observed: `ready` means its dependencies are all closed
/// (the loop's *decision* input), `body` is what the producer would stage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    pub issue: String,
    pub ready: bool,
    pub body: String,
}

impl Candidate {
    /// A dependency-closed issue: decided work, awaiting actuation.
    pub fn new(issue: impl Into<String>, body: impl Into<String>) -> Self {
        Candidate {
            issue: issue.into(),
            ready: true,
            body: body.into(),
        }
    }

    /// An issue still waiting on a dependency: observed, never ready, so it
    /// belongs in the denominator and not in the gap.
    pub fn waiting(issue: impl Into<String>, body: impl Into<String>) -> Self {
        Candidate {
            issue: issue.into(),
            ready: false,
            body: body.into(),
        }
    }
}

/// The world the loop acts on: what is staged, and what has been dispatched.
/// Passed to [`FrontierLoop::tick`] and mutated by the act stage, so a test
/// can assert what the loop itself produced rather than what was pre-seeded.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct World {
    /// Issues with a staged spec (the state [`Guard::StagedSpec`] checks).
    pub staged: BTreeSet<String>,
    /// Issues dispatched, in dispatch order.
    pub dispatched: Vec<String>,
}

impl World {
    pub fn is_staged(&self, issue: &str) -> bool {
        self.staged.contains(issue)
    }

    pub fn is_dispatched(&self, issue: &str) -> bool {
        self.dispatched.iter().any(|d| d == issue)
    }
}

/// The producer's staging step for one body: the work `iw-stage.sh` does. It
/// refuses rather than fabricates — an empty or trivially short body is not
/// staged, because the guard's purpose is to keep an agent off a spec with
/// nothing in it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StagingOutcome {
    Staged { bytes: usize },
    Refused { reason: String },
}

impl StagingOutcome {
    pub fn staged(&self) -> bool {
        matches!(self, StagingOutcome::Staged { .. })
    }
}

/// Stage one issue body as a spec. Length is measured on the trimmed body:
/// whitespace is not a specification.
pub fn stage(body: &str) -> StagingOutcome {
    let bytes = body.trim().len();
    if bytes < MIN_STAGED_SPEC_BYTES {
        return StagingOutcome::Refused {
            reason: format!(
                "body too short to implement from ({bytes} bytes, min {MIN_STAGED_SPEC_BYTES})"
            ),
        };
    }
    StagingOutcome::Staged { bytes }
}

/// One decided item the loop could not act on, with the reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockedCandidate {
    pub issue: String,
    /// The guard that held it.
    pub guard: Guard,
    /// Why it is held, as rendered in the report line.
    pub reason: String,
    /// `true` when the guard has no producer at all — a stop. `false` when a
    /// producer ran and refused this item on its own merits.
    pub producerless: bool,
}

/// The outcome of one loop pass: what was decided, what was done, and every
/// item in between with its reason. Invariant 1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TickReport {
    /// Items observed this pass (the denominator, including unready ones).
    pub observed: u32,
    /// Decided work: dependency-closed items.
    pub ready: u32,
    /// Items the loop's own producer staged this pass.
    pub staged: u32,
    /// Done work.
    pub dispatched: u32,
    /// The gap, itemized. Never empty when `ready != dispatched`.
    pub blocked: Vec<BlockedCandidate>,
}

impl TickReport {
    /// The gap between decided and done. In the incident this was 8 and
    /// reported nowhere.
    pub fn gap(&self) -> u32 {
        self.ready.saturating_sub(self.dispatched)
    }

    /// The gap must be fully explained: every ready item is either dispatched
    /// or in `blocked` with a reason. A report whose numbers do not reconcile
    /// reports a state that cannot exist.
    pub fn reconciles(&self) -> bool {
        self.ready == self.dispatched + self.blocked.len() as u32
    }

    /// Ready work with nothing dispatched: the shape the incident reported as
    /// idleness.
    pub fn stalled(&self) -> bool {
        self.ready > 0 && self.dispatched == 0 && !self.blocked.is_empty()
    }

    /// Distinct block reasons in first-seen order (the incident's single
    /// reason, or several when the gap has several causes).
    pub fn distinct_reasons(&self) -> Vec<String> {
        let mut reasons: Vec<String> = Vec::new();
        for b in &self.blocked {
            if !reasons.iter().any(|r| r == &b.reason) {
                reasons.push(b.reason.clone());
            }
        }
        reasons
    }

    /// How many items are held by `reason`.
    pub fn blocked_by(&self, reason: &str) -> u32 {
        self.blocked.iter().filter(|b| b.reason == reason).count() as u32
    }

    /// The report line: decided, staged, done, and — whenever there is a gap —
    /// the count and reason for every item in it.
    ///
    /// Incident line: `8 ready, 0 dispatched, 8 blocked: no staged spec`.
    pub fn line(&self) -> String {
        let mut line = format!("{} ready", self.ready);
        if self.staged > 0 {
            line.push_str(&format!(", {} staged", self.staged));
        }
        line.push_str(&format!(", {} dispatched", self.dispatched));
        for reason in self.distinct_reasons() {
            line.push_str(&format!(", {} blocked: {reason}", self.blocked_by(&reason)));
        }
        line
    }

    /// Whether this pass could act on what it decided. A ready loop that
    /// dispatched nothing is held, and the reason decides whether the hold has
    /// a release.
    pub fn verdict(&self) -> LoopVerdict {
        if self.ready == 0 {
            return LoopVerdict::Idle;
        }
        if self.blocked.is_empty() {
            return LoopVerdict::Acted {
                dispatched: self.dispatched,
            };
        }
        // Any producerless hold dominates: a guard nothing can satisfy outranks
        // every transient hold in the same pass.
        if let Some(stop) = self.blocked.iter().find(|b| b.producerless) {
            return LoopVerdict::PermanentStop {
                guard: stop.guard.as_str().to_string(),
                count: self.blocked.iter().filter(|b| b.producerless).count() as u32,
            };
        }
        LoopVerdict::BlockedWithProducer {
            reason: self.blocked[0].reason.clone(),
            count: self.blocked.len() as u32,
        }
    }
}

/// Invariant 1, made mechanical: a report whose decided count differs from its
/// done count must name the gap and every reason behind it. The incident's
/// line — `8 ready, 0 dispatched`, both numbers true — fails this check, and
/// that is the point: the old report had no way to express the gap, so it said
/// nothing. A gap with no held items behind it has no honest line at all: the
/// word `blocked` is required alongside the count, so an unreconciled report
/// cannot pass this check by restating its own numbers.
pub fn line_names_gap(line: &str, report: &TickReport) -> bool {
    let gap = report.gap();
    if gap == 0 {
        return true;
    }
    line.contains("blocked")
        && line.contains(&gap.to_string())
        && report
            .distinct_reasons()
            .iter()
            .all(|reason| line.contains(reason.as_str()))
}

/// What the loop's pass amounts to, once the counts are read together.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LoopVerdict {
    /// Nothing was ready. Idleness is a legitimate state; the incident was not
    /// idle, it was stopped, and the two must not render the same way.
    Idle,
    /// The loop acted on the work it decided.
    Acted { dispatched: u32 },
    /// Every held item is held by a guard that has a producer, or by a
    /// producer's own refusal on the item's merits: the hold has a release.
    BlockedWithProducer { reason: String, count: u32 },
    /// Held by a guard with no producer. Nothing in the system can satisfy it,
    /// so the loop will report this same line forever.
    PermanentStop { guard: String, count: u32 },
}

impl LoopVerdict {
    pub fn permanent_stop(&self) -> bool {
        matches!(self, LoopVerdict::PermanentStop { .. })
    }

    /// The verdict line, appended to the report line so "0 dispatched" is
    /// never read as idleness.
    pub fn line(&self) -> String {
        match self {
            LoopVerdict::Idle => "nothing ready".to_string(),
            LoopVerdict::Acted { dispatched } => {
                format!("loop acted: {dispatched} dispatched")
            }
            LoopVerdict::BlockedWithProducer { reason, count } => format!(
                "loop held: {count} ready, 0 dispatched by a guard with a producer ({reason})"
            ),
            LoopVerdict::PermanentStop { guard, count } => format!(
                "permanent stop: {count} ready, 0 dispatched, no producer satisfies guard '{guard}'",
            ),
        }
    }
}

/// Invariant 2, made mechanical on the rendered line: a permanent stop must
/// name the guard that has no producer. A stop reported without its guard is
/// as unactionable as a stop that names nothing.
pub fn stop_line_names_guard(line: &str, guard: Guard) -> bool {
    line.contains(guard.as_str())
}

/// The frontier loop: observe (read the candidates), decide (which are
/// dependency-closed), act (satisfy the actuator's guards, then dispatch).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrontierLoop {
    pub actuator: Actuator,
}

impl FrontierLoop {
    pub fn new(actuator: Actuator) -> Self {
        FrontierLoop { actuator }
    }

    /// The findings the actuator carries, surfaced every pass: a guard with no
    /// producer is reported before it holds anything, because that is when it
    /// is still a wiring mistake rather than a stopped cluster.
    pub fn preflight(&self) -> Vec<String> {
        self.actuator.findings()
    }

    /// One pass: observe → decide → act. The act stage runs the producer for
    /// an unsatisfied guard before calling the item blocked, so staging happens
    /// in the loop rather than in someone's memory.
    pub fn tick(&self, world: &mut World, candidates: &[Candidate]) -> TickReport {
        let mut report = TickReport {
            observed: candidates.len() as u32,
            ready: 0,
            staged: 0,
            dispatched: 0,
            blocked: Vec::new(),
        };

        for candidate in candidates {
            // Observe / decide: only dependency-closed items are ready work.
            if !candidate.ready {
                continue;
            }
            report.ready += 1;

            // Act: for every unsatisfied guard, run its producer if it has
            // one, and only call the item held once the producer is exhausted.
            let mut holds: Vec<(Guard, String, bool)> = Vec::new();
            for precondition in &self.actuator.preconditions {
                if precondition.guard.satisfied(world, &candidate.issue) {
                    continue;
                }
                match &precondition.producer {
                    Some(_owner) => match stage(&candidate.body) {
                        StagingOutcome::Staged { .. } => {
                            precondition.guard.satisfy(world, &candidate.issue);
                            report.staged += 1;
                        }
                        StagingOutcome::Refused { reason } => holds.push((
                            precondition.guard,
                            format!("stage refused: {reason}"),
                            false,
                        )),
                    },
                    None => holds.push((
                        precondition.guard,
                        precondition.guard.block_reason().to_string(),
                        true,
                    )),
                }
            }

            if holds.is_empty() {
                world.dispatched.push(candidate.issue.clone());
                report.dispatched += 1;
            } else {
                let reason = holds
                    .iter()
                    .map(|(_, reason, _)| reason.as_str())
                    .collect::<Vec<_>>()
                    .join("; ");
                // A producerless guard dominates the entry's classification and
                // its named guard, whatever else held the same item.
                let guard = holds
                    .iter()
                    .find(|(_, _, producerless)| *producerless)
                    .map(|(guard, _, _)| *guard)
                    .unwrap_or(holds[0].0);
                let producerless = holds.iter().any(|(_, _, p)| *p);
                report.blocked.push(BlockedCandidate {
                    issue: candidate.issue.clone(),
                    guard,
                    reason,
                    producerless,
                });
            }
        }

        report
    }
}
