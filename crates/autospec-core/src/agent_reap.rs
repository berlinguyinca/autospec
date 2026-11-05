//! Liveness and reaping of a fleet of long-running agents (issue #4579).
//!
//! The incident: while the base copy was broken, a triage rule was built: an
//! agent past a 20-minute budget with **zero bytes in `agent.out`** is stuck
//! and gets reaped. It worked — the agents it identified were genuinely
//! wedged, verified independently (both `tar` processes at 0% CPU, scratch
//! directory growing 0 KB per 60 s). It worked for the wrong reason.
//! `agent.out` is written when the agent *finishes*, not as it works. It was
//! zero for stalled agents and zero for working agents alike; the stall just
//! made that indistinguishable.
//!
//! Now that the copy is fixed, the same rule is actively dangerous. Sampled
//! just after the fix, eight agents past the copy:
//!
//! ```text
//! as-4534  age=8:27  slurm=267B  agent.out=0B
//! as-4535  age=6:02  slurm=267B  agent.out=0B
//! ...all eight identical
//! ```
//!
//! Every one is healthy and doing real work — `as-4534` is running `pi`
//! against the LLM, which has issued `cargo build --workspace`, with `rustc`
//! compiling at 110% CPU. An agent legitimately spends its whole multi-hour
//! run with `agent.out` at zero. Applied today, the rule would reap the
//! entire fleet at the 20-minute mark.
//!
//! The invariants this module enforces, in the order the incident produced
//! them:
//!
//! 1. **A liveness signal must be something the subject writes *while*
//!    working, not when it finishes.** A completion artifact answers
//!    "is it done", never "is it alive", and the two are only confusable
//!    when nothing is alive — which is exactly the situation in which the
//!    signal gets chosen. [`LivenessSignal`] carries the write timing, and
//!    a completion artifact is not usable for liveness (invariant 1).
//! 2. **Prefer a signal that advances.** The Slurm log is already better:
//!    it moved 36B → 267B as the agent cleared the copy. Best is an
//!    explicit heartbeat with a phase name and a timestamp, so both
//!    "alive" and "where" are answerable without inspecting the process
//!    tree on a compute node. [`SignalQuality`] orders the three and
//!    [`SignalQuality::rank`] makes the preference checkable.
//! 3. **A reaping rule must be validated against a known-healthy subject
//!    before it is allowed to act.** The incident's rule was validated
//!    only against stuck agents, where every candidate signal looks
//!    identical ([`ValidationState::StuckOnly`]) — and a rule that fires
//!    on a known-healthy subject is defective by definition
//!    ([`ValidationState::FiresOnHealthy`]): had the incident's rule been
//!    applied a day later, this is what its validation would have shown.
//! 4. **When the underlying fault is fixed, re-validate the triage built
//!    during it.** Rules written under an outage encode the outage's
//!    conditions; the incident's rule survived the fix and silently
//!    inverted from useful to destructive. [`RuleProvenance`] carries the
//!    three facts and flags the un-revalidated survivor.
//!
//! Concretely, the zero-bytes test is replaced by: the agent's Slurm log
//! has not grown in N minutes **and** the agent is past its phase budget.
//! The phase is published ([`Phase`] — `copy`, `agent`, `gate`) so the
//! budget can differ per phase: the copy is minutes, the agent run is
//! hours, and one number cannot serve both. [`zero_byte_rule_fires`] is
//! kept — named, and never called by the decision — as the rule the
//! incident was, so the contrast stays in the code.
//!
//! Every function here is pure. The caller observes the files and the
//! clock; this code decides what the observations mean.

use std::time::Duration;

// ── Invariant 1: a liveness signal is written while working ────────────────

/// When the subject writes an artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteTiming {
    /// The artifact is written when the subject **finishes**. It is zero
    /// for a working subject and zero for a wedged one alike; the stall
    /// just makes that indistinguishable (`agent.out`).
    OnCompletion,
    /// The artifact is written or appended **as the subject works**, so a
    /// frozen artifact means the work stopped (the Slurm log, a heartbeat).
    WhileWorking,
}

/// How much information a signal carries while the subject works
/// (invariant 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalQuality {
    /// Written only at completion: answers "is it done", never "is it
    /// alive". Not a liveness signal.
    Completion,
    /// Grows while the subject works (the Slurm log, which moved 36B →
    /// 267B as the agent cleared the copy): "alive" is answerable.
    Advancing,
    /// An explicit heartbeat with a phase name and a timestamp (`phase=copy`,
    /// `phase=agent`, `phase=gate`): both "alive" and "where" are
    /// answerable without inspecting the process tree on a compute node.
    Heartbeat,
}

impl SignalQuality {
    /// The preference order: a heartbeat beats an advancing log, which
    /// beats a completion artifact. A higher-ranked signal is what a triage
    /// rule should be built on.
    pub fn rank(self) -> u8 {
        match self {
            SignalQuality::Completion => 0,
            SignalQuality::Advancing => 1,
            SignalQuality::Heartbeat => 2,
        }
    }
}

/// A candidate liveness signal for a reaping rule (invariants 1 and 2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LivenessSignal {
    /// The signal's name (`"agent.out"`, `"slurm.log"`, `"heartbeat"`).
    pub name: String,
    /// When the subject writes it.
    pub timing: WriteTiming,
    /// How much it carries while the subject works.
    pub quality: SignalQuality,
}

impl LivenessSignal {
    /// Whether this signal may be used to judge liveness.
    ///
    /// Only a signal written while the subject works may: a completion
    /// artifact is zero for a working subject and a wedged one alike, so a
    /// rule built on it fires on the healthy fleet the moment the fault
    /// that made the two states confusable is fixed (invariant 1).
    pub fn usable_for_liveness(&self) -> bool {
        self.timing == WriteTiming::WhileWorking
    }

    /// A finding when the signal is a completion artifact. `None` when the
    /// signal may be used.
    pub fn finding(&self) -> Option<String> {
        if self.usable_for_liveness() {
            return None;
        }
        Some(format!(
            "NOT A LIVENESS SIGNAL: '{name}' is written when the subject finishes — it \
             answers \"is it done\", never \"is it alive\", and the two are only \
             confusable when nothing is alive (the situation in which it was chosen)",
            name = self.name
        ))
    }
}

// ── The phase is published, so the budget can differ per phase ─────────────

/// The phase an agent is in. Published by the runner so the reaping budget
/// can differ per phase: the copy is minutes, the agent run is hours, and
/// one number cannot serve both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Copy,
    Agent,
    Gate,
}

impl Phase {
    /// Parse a published phase (`phase=copy`). `None` for an unknown phase —
    /// fail-closed: an agent whose phase cannot be read gets no budget, so
    /// it cannot be reaped on a budget.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "copy" => Some(Phase::Copy),
            "agent" => Some(Phase::Agent),
            "gate" => Some(Phase::Gate),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Phase::Copy => "copy",
            Phase::Agent => "agent",
            Phase::Gate => "gate",
        }
    }
}

/// Default budgets per phase. The copy budget is the 20 minutes the
/// incident's rule used for the whole run — a number that served the copy
/// and starved the agent phase; the agent budget is hours, because an agent
/// legitimately spends its whole multi-hour run with `agent.out` at zero.
pub const DEFAULT_COPY_BUDGET: Duration = Duration::from_secs(20 * 60);
pub const DEFAULT_AGENT_BUDGET: Duration = Duration::from_secs(12 * 3600);
pub const DEFAULT_GATE_BUDGET: Duration = Duration::from_secs(30 * 60);

/// Default quiet bound for the Slurm log: how long it may go without
/// growing before the silence starts counting.
pub const DEFAULT_LOG_QUIET: Duration = Duration::from_secs(15 * 60);

/// The per-phase budget table (the "concretely" of the issue: the budget
/// differs per phase because the phase is published).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhaseBudgets {
    pub copy: Duration,
    pub agent: Duration,
    pub gate: Duration,
}

impl Default for PhaseBudgets {
    fn default() -> Self {
        Self {
            copy: DEFAULT_COPY_BUDGET,
            agent: DEFAULT_AGENT_BUDGET,
            gate: DEFAULT_GATE_BUDGET,
        }
    }
}

impl PhaseBudgets {
    /// The budget for one phase.
    pub fn budget(&self, phase: Phase) -> Duration {
        match phase {
            Phase::Copy => self.copy,
            Phase::Agent => self.agent,
            Phase::Gate => self.gate,
        }
    }
}

// ── The decision: Slurm log quiet AND past the phase budget ────────────────

/// One observation of a running agent, as sampled by the triage loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentObservation {
    /// The agent's name (`"as-4534"`).
    pub name: String,
    /// The phase the runner published for the agent.
    pub phase: Phase,
    /// How long the agent has been running.
    pub age: Duration,
    /// How long it has been since the agent's Slurm log last grew.
    pub slurm_log_quiet: Duration,
    /// The size of `agent.out` — the incident's signal. Recorded for the
    /// evidence record and **never read by the decision**: it is a
    /// completion artifact (invariant 1).
    pub agent_out_bytes: u64,
}

/// The incident's rule, kept as the named contrast: an agent past a flat
/// 20-minute budget with zero bytes in `agent.out` is stuck.
///
/// It fired on the wedged agents during the outage (true) and on the entire
/// healthy fleet after the fix (also true) — the same value across both
/// states, which is what invariant 1 forbids. Nothing in the decision path
/// calls this; it exists so the regression tests can show the old rule
/// reaping the fleet and the new rule not.
pub fn zero_byte_rule_fires(obs: &AgentObservation, age_budget: Duration) -> bool {
    obs.age > age_budget && obs.agent_out_bytes == 0
}

/// Why a keep decision was reached (rendered, so "keep" is never silent).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeepReason {
    /// The Slurm log grew within the quiet bound: the subject writes while
    /// it works, so it is alive.
    LogAdvancing,
    /// The Slurm log is quiet, but the agent is still inside its phase
    /// budget: silence the phase can absorb.
    WithinPhaseBudget,
}

/// What the rule decides for one agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReapVerdict {
    /// Not both conditions hold: keep, with the reason named.
    Keep { reason: KeepReason },
    /// The Slurm log has not grown in `silent` and the agent is past its
    /// phase budget `budget`: reap.
    Reap { silent: Duration, budget: Duration },
}

impl ReapVerdict {
    pub fn is_reap(&self) -> bool {
        matches!(self, ReapVerdict::Reap { .. })
    }

    /// The operator line: the phase, the age, the silence, and the budget
    /// on one line, so a keep is never rendered without the numbers it
    /// rests on.
    pub fn line(&self, obs: &AgentObservation) -> String {
        let base = format!(
            "{} phase={} age={} slurm_log_quiet={}",
            obs.name,
            obs.phase.as_str(),
            fmt_clock(obs.age),
            fmt_clock(obs.slurm_log_quiet)
        );
        match self {
            ReapVerdict::Keep { reason } => match reason {
                KeepReason::LogAdvancing => format!("{base} — keep: log advancing"),
                KeepReason::WithinPhaseBudget => {
                    format!("{base} — keep: within {} budget", obs.phase.as_str())
                }
            },
            ReapVerdict::Reap { silent, budget } => format!(
                "{base} — REAP: log silent {} (past {}), {} budget {} elapsed",
                fmt_clock(*silent),
                fmt_clock(*budget),
                obs.phase.as_str(),
                fmt_clock(obs.age)
            ),
        }
    }
}

/// Why [`ReapRule::new`] refused the rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleError {
    /// A zero quiet bound makes every observation "quiet": the rule fires
    /// on healthy work and gets switched off.
    ZeroQuietBound,
    /// A zero phase budget makes every agent in that phase past its budget.
    ZeroPhaseBudget { phase: Phase },
}

impl RuleError {
    pub fn line(&self) -> String {
        match self {
            RuleError::ZeroQuietBound => {
                "zero quiet bound: every observation is 'quiet' — the rule fires on \
                 healthy work and gets switched off"
                    .to_string()
            }
            RuleError::ZeroPhaseBudget { phase } => format!(
                "zero {} budget: every agent in that phase is past its budget",
                phase.as_str()
            ),
        }
    }
}

/// The reaping rule: the Slurm log has not grown in N minutes **and** the
/// agent is past its phase budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReapRule {
    /// How long the Slurm log may go without growing before the silence
    /// starts counting.
    pub log_quiet: Duration,
    /// The per-phase budgets.
    pub budgets: PhaseBudgets,
}

impl ReapRule {
    /// Refuse a rule that fires on healthy work by construction (a zero
    /// quiet bound or a zero phase budget).
    pub fn new(log_quiet: Duration, budgets: PhaseBudgets) -> Result<Self, RuleError> {
        if log_quiet.is_zero() {
            return Err(RuleError::ZeroQuietBound);
        }
        for phase in [Phase::Copy, Phase::Agent, Phase::Gate] {
            if budgets.budget(phase).is_zero() {
                return Err(RuleError::ZeroPhaseBudget { phase });
            }
        }
        Ok(ReapRule { log_quiet, budgets })
    }

    /// A rule with the defaults ([`DEFAULT_LOG_QUIET`], [`PhaseBudgets::default`]).
    pub fn with_defaults() -> Self {
        Self {
            log_quiet: DEFAULT_LOG_QUIET,
            budgets: PhaseBudgets::default(),
        }
    }

    /// The report line: the quiet bound and every phase budget on one line,
    /// so "one number cannot serve both" is visible in the rule itself.
    pub fn line(&self) -> String {
        format!(
            "reap rule: slurm log silent >= {} AND past the phase budget (copy {}, agent {}, gate {})",
            fmt_clock(self.log_quiet),
            fmt_clock(self.budgets.copy),
            fmt_clock(self.budgets.agent),
            fmt_clock(self.budgets.gate)
        )
    }

    /// Decide for one agent.
    ///
    /// Both conditions must hold: the Slurm log quiet for at least
    /// `log_quiet` **and** the agent past its phase budget. A healthy agent
    /// spends its multi-hour run with `agent.out` at zero and the Slurm log
    /// quiet for stretches — either condition alone reaps the fleet, which
    /// is why the old flat rule could not survive the fix. `agent.out` is
    /// never read (invariant 1).
    pub fn decision(&self, obs: &AgentObservation) -> ReapVerdict {
        let budget = self.budgets.budget(obs.phase);
        let quiet = obs.slurm_log_quiet >= self.log_quiet;
        let past = obs.age > budget;
        match (quiet, past) {
            (true, true) => ReapVerdict::Reap {
                silent: obs.slurm_log_quiet,
                budget,
            },
            (false, _) => ReapVerdict::Keep {
                reason: KeepReason::LogAdvancing,
            },
            (true, false) => ReapVerdict::Keep {
                reason: KeepReason::WithinPhaseBudget,
            },
        }
    }
}

// ── Invariant 3: validate on a known-healthy subject before acting ─────────

/// One trial of the rule on a subject whose state was established
/// independently (the two `tar` processes at 0% CPU, the scratch directory
/// growing 0 KB per 60 s — verification the rule itself did not supply).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthyTrial {
    /// The subject's name.
    pub subject: String,
    /// Whether the rule fired (reaped) this known-healthy subject.
    pub fired: bool,
}

/// The rule's validation record (invariant 3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Validation {
    /// Trials on subjects independently verified **healthy**.
    pub healthy: Vec<HealthyTrial>,
    /// Trials on subjects independently verified **stuck**. The incident's
    /// rule had two of these and zero of the healthy kind.
    pub stuck: usize,
}

/// What the validation record means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationState {
    /// No trials at all: nothing has been established either way.
    Untested,
    /// Trials on stuck subjects only: every candidate signal looks identical
    /// there, so the record proves the rule catches stuck agents and nothing
    /// about what it does to healthy ones. The incident's rule, while the
    /// fault was live.
    StuckOnly { stuck: usize },
    /// At least one known-healthy trial, and the rule did not fire on any
    /// of them: the rule may act.
    Validated { healthy: usize, stuck: usize },
    /// The rule fired on a known-healthy subject: it reaps the healthy
    /// fleet (the incident, had the rule been applied after the fix).
    FiresOnHealthy { subject: String },
}

impl Validation {
    /// The state the record means.
    ///
    /// A fired-on-healthy trial dominates: a rule that fires on a known
    /// healthy subject is defective no matter what its other trials show.
    /// With no healthy trials the record is [`ValidationState::Untested`] or
    /// [`ValidationState::StuckOnly`], and neither may act (invariant 3).
    pub fn state(&self) -> ValidationState {
        if let Some(fired) = self.healthy.iter().find(|t| t.fired) {
            return ValidationState::FiresOnHealthy {
                subject: fired.subject.clone(),
            };
        }
        if self.healthy.is_empty() {
            return if self.stuck > 0 {
                ValidationState::StuckOnly { stuck: self.stuck }
            } else {
                ValidationState::Untested
            };
        }
        ValidationState::Validated {
            healthy: self.healthy.len(),
            stuck: self.stuck,
        }
    }

    /// Whether the rule may act on this record. Only
    /// [`ValidationState::Validated`] may.
    pub fn may_act(&self) -> bool {
        matches!(self.state(), ValidationState::Validated { .. })
    }

    /// The operator line.
    pub fn line(&self) -> String {
        match self.state() {
            ValidationState::Untested => {
                "validation: no trials — the rule has not been presented with any \
                 subject whose state was established independently"
                    .to_string()
            }
            ValidationState::StuckOnly { stuck } => format!(
                "MAY NOT ACT: validated on {stuck} stuck subject(s) only — every \
                 candidate signal looks identical there"
            ),
            ValidationState::Validated { healthy, stuck } => format!(
                "validated: {healthy} healthy trial(s) without a fire, {stuck} stuck trial(s)"
            ),
            ValidationState::FiresOnHealthy { subject } => format!(
                "DEFECTIVE: the rule fired on known-healthy subject '{subject}' — it \
                 reaps the healthy fleet"
            ),
        }
    }
}

// ── Invariant 4: re-validate the triage when the fault it was built under is fixed ──

/// Where the rule came from (invariant 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuleProvenance {
    /// The rule was written while the underlying fault was live.
    pub written_during_outage: bool,
    /// The underlying fault has since been fixed.
    pub fault_fixed: bool,
    /// The rule has been re-validated against the post-fix fleet.
    pub revalidated_after_fix: bool,
}

impl RuleProvenance {
    /// A finding when an outage-born rule outlived the outage without a
    /// re-validation.
    ///
    /// Rules written under an outage encode the outage's conditions: while
    /// the fault is live, healthy and wedged subjects are indistinguishable
    /// on the signals at hand, so a rule that was right under the fault can
    /// be destructive after it. The incident's rule survived the copy fix
    /// and silently inverted from useful to destructive, with no step
    /// anywhere prompting a re-check.
    pub fn finding(&self) -> Option<String> {
        if self.written_during_outage && self.fault_fixed && !self.revalidated_after_fix {
            Some(
                "STALE FROM OUTAGE: the rule was written while the fault was live and the \
                 fault is fixed, but the rule has not been re-validated against the \
                 post-fix fleet — rules written under an outage encode the outage's \
                 conditions"
                    .to_string(),
            )
        } else {
            None
        }
    }
}

/// Why a reaping rule may not act.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionBlocker {
    /// Invariant 1: the rule's liveness signal is a completion artifact.
    CompletionArtifactSignal { signal: String },
    /// Invariant 3: the rule was validated only on stuck subjects (or not
    /// at all).
    NotValidatedOnHealthy,
    /// Invariant 3: the rule fired on a known-healthy subject.
    FiresOnHealthy { subject: String },
    /// Invariant 4: an outage-born rule the fault fix outlived, never
    /// re-validated.
    StaleFromOutage,
}

impl ActionBlocker {
    pub fn line(&self) -> String {
        match self {
            ActionBlocker::CompletionArtifactSignal { signal } => {
                format!("BLOCKED: liveness signal '{signal}' is a completion artifact")
            }
            ActionBlocker::NotValidatedOnHealthy => {
                "BLOCKED: not validated against a known-healthy subject — a health \
                 signal's first use must not be a deletion"
                    .to_string()
            }
            ActionBlocker::FiresOnHealthy { subject } => {
                format!("BLOCKED: the rule fires on known-healthy subject '{subject}'")
            }
            ActionBlocker::StaleFromOutage => {
                "BLOCKED: written during the outage, the fault is fixed, and the rule \
                 has not been re-validated against the post-fix fleet"
                    .to_string()
            }
        }
    }
}

/// The blockers a reaping rule must clear before it is allowed to act.
///
/// Empty means the rule may act. Each invariant contributes at most one:
///
/// * invariant 1 — the rule's signal is a completion artifact;
/// * invariant 3 — the validation record is not
///   [`ValidationState::Validated`];
/// * invariant 4 — the rule is stale from the outage it was written under.
pub fn action_blockers(
    signal: &LivenessSignal,
    validation: &Validation,
    provenance: &RuleProvenance,
) -> Vec<ActionBlocker> {
    let mut out = Vec::new();
    if !signal.usable_for_liveness() {
        out.push(ActionBlocker::CompletionArtifactSignal {
            signal: signal.name.clone(),
        });
    }
    match validation.state() {
        ValidationState::Validated { .. } => {}
        ValidationState::FiresOnHealthy { subject } => {
            out.push(ActionBlocker::FiresOnHealthy { subject });
        }
        ValidationState::Untested | ValidationState::StuckOnly { .. } => {
            out.push(ActionBlocker::NotValidatedOnHealthy);
        }
    }
    if provenance.finding().is_some() {
        out.push(ActionBlocker::StaleFromOutage);
    }
    out
}

/// Whether the rule may act: no blockers.
pub fn may_act(
    signal: &LivenessSignal,
    validation: &Validation,
    provenance: &RuleProvenance,
) -> bool {
    action_blockers(signal, validation, provenance).is_empty()
}

// ── Rendering ───────────────────────────────────────────────────────────────

/// `8:27` / `45:00` — the clock format the incident's sample used.
fn fmt_clock(d: Duration) -> String {
    let s = d.as_secs();
    let (h, m, sec) = (s / 3600, (s % 3600) / 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{sec:02}")
    } else {
        format!("{m}:{sec:02}")
    }
}
