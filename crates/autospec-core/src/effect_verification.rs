//! Verify the effect, never the edit (issue #3755).
//!
//! Across one session the supervising agent caused nine outages, and four
//! were repairs for earlier problems. Every one of the nine was caught the
//! same way — by *observing the effect* of the change, never by re-reading
//! the edit:
//!
//! - #1 (a runner replaced while jobs executed it) was found by *grepping
//!   the logs* for what actually killed the jobs.
//! - #3 (a comment inserted inside `sbatch` line-continuations) was found
//!   by *running* the topup and seeing `sbatch` reject the wrap.
//! - #5 (the endpoint selector weighted by free slots) was found by
//!   *drawing from* the selector five times and getting nothing.
//! - #6 (a hand-run `clippy -D warnings`, stricter than CI) was found by
//!   *checking whether the lints existed on main*.
//!
//! Re-reading the change would have caught none of them. In each case the
//! edit looked correct, because it was correct in the sense the author
//! intended. Capability is not the control — every incident came from an
//! action taken confidently and for good reason — and neither is care: two
//! of the nine recurred three and four times each despite being known and
//! documented. The control is a mechanical verification step, and this
//! module encodes it as checkable primitives:
//!
//! 1. **After any change to a running system, verify the effect, not the
//!    edit.** Run the thing once and read its output
//!    ([`ProbeKind::Effect`] vs [`ProbeKind::Reread`];
//!    [`ship_verdict`], [`plan_findings`]).
//! 2. **A verification that returns nothing is a failure, not a pass.**
//!    Item 5's symptom was an empty result set, which reads as "no
//!    problems found" ([`run_probe`] -> [`ProbeOutcome::NoSignal`],
//!    [`outcome_line`]). A probe that cannot name the signal a healthy
//!    system produces cannot read its output as a pass.
//! 3. **State the blast radius before editing shared state.** Who reads
//!    this path, what do they do if it changes mid-read, what makes the
//!    change exclusive with respect to them (issue #3732)
//!    ([`BlastRadius`], [`blast_radius_findings`]).
//! 4. **Never raise a bar ad hoc.** A gate matches the repository's
//!    definition or it is a different gate (issue #3753)
//!    ([`GateApplication`], [`ad_hoc_gate_findings`]).
//! 5. **Prefer the change you can verify cheaply** over the better change
//!    you cannot ([`choose_change`]).
//!
//! The uncomfortable version, worth stating plainly: an autonomous
//! supervisor with write access to a running fleet will damage it, and the
//! damage will come disguised as maintenance. The useful question in a
//! design review is not "is this change correct" but "how will I know
//! within sixty seconds if it is not" ([`VERIFY_WINDOW_SECONDS`],
//! [`sixty_second_answer`]) — and any change that cannot answer it should
//! wait for a moment when the system is idle ([`ShipVerdict::WaitForIdle`]).

/// The window in which a change to a running system must be able to answer
/// "how will I know if it is not".
///
/// This is one command: run the thing once and read its output. A
/// verification that takes longer than the window does not answer the
/// question in time, and the change waits instead of shipping.
pub const VERIFY_WINDOW_SECONDS: u32 = 60;

/// What a verification step observes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeKind {
    /// Run the thing and read its output: grep the logs for what actually
    /// happened, draw from the selector, run the gate the way CI does,
    /// submit a job and read the scheduler's verdict.
    Effect,
    /// Re-read the change itself: the diff, the file, the configuration.
    /// A re-read is legitimate as a supplement, but it observes what was
    /// written, not what the system does — re-reading the change would
    /// have caught none of the nine incidents, because each edit was
    /// correct in the sense its author intended.
    Reread,
}

/// A single verification step for a change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Probe {
    /// What this step observes.
    pub kind: ProbeKind,
    /// The command (or observation) that runs the thing.
    pub command: String,
    /// The signal a healthy system produces.
    ///
    /// A probe that cannot name its pass signal cannot distinguish a
    /// healthy system from a broken one, and it must not read its output
    /// as a pass (invariant 2).
    pub expected_signal: String,
    /// How long the probe takes, in seconds.
    pub seconds: u32,
}

impl Probe {
    /// Invariant 1 + the 60-second question: does this probe answer "how
    /// will I know within sixty seconds if it is not"?
    ///
    /// Only an effect probe that names its pass signal and completes
    /// inside the window counts. A re-read is a different activity
    /// (invariant 1); a probe without a signal reads "nothing found" as a
    /// pass (invariant 2); a slow probe answers too late.
    pub fn answers_within_window(&self) -> bool {
        self.kind == ProbeKind::Effect
            && !self.expected_signal.is_empty()
            && self.seconds <= VERIFY_WINDOW_SECONDS
    }
}

/// Where a change lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// The system is running and its consumers are reading from it now.
    Running,
    /// The system is idle: a change there cannot break a live consumer,
    /// which is the state a change that cannot answer the 60-second
    /// question waits for.
    Idle,
}

/// A change to a system plus its verification plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// What the change is, for the rendered answer.
    pub summary: String,
    /// Where it lands.
    pub target: Target,
    /// How good the change is, higher better — the quality ranking that
    /// the verifiability filter runs *over*, and must never outrank it
    /// (invariant 5).
    pub value: u32,
    /// The verification plan: what will be observed after the change.
    pub probes: Vec<Probe>,
}

/// Invariant 1, as a decision: may this change ship against its target?
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShipVerdict {
    /// The change may ship as-is. `probes` are the indices of the probes
    /// that answer the 60-second question; it is empty when the target is
    /// idle and there is no live system to break.
    Ship { probes: Vec<usize> },
    /// The verification plan re-reads the edit. This is the insidious
    /// shape: the plan *looks* like verification, but it observes what was
    /// written, not what the system does — and re-reading would have
    /// caught none of the nine.
    RereadOnly,
    /// The target is running and no probe answers within the window: the
    /// change cannot say how it would be known to be wrong in time, so it
    /// waits for a moment when the system is idle.
    WaitForIdle,
}

/// Invariant 1 + the 60-second question, as a decision.
///
/// A change to a running system ships only when its plan contains a probe
/// that answers the question — an effect probe with a named pass signal
/// that completes inside [`VERIFY_WINDOW_SECONDS`]. A change to an idle
/// system ships regardless: the 60-second question is about breaking a
/// live system, and idleness is the state the other verdicts wait for.
pub fn ship_verdict(change: &Change) -> ShipVerdict {
    let answering: Vec<usize> = (0..change.probes.len())
        .filter(|&i| change.probes[i].answers_within_window())
        .collect();
    if !answering.is_empty() {
        return ShipVerdict::Ship { probes: answering };
    }
    match change.target {
        Target::Idle => ShipVerdict::Ship { probes: Vec::new() },
        Target::Running
            if !change.probes.is_empty()
                && change.probes.iter().all(|p| p.kind == ProbeKind::Reread) =>
        {
            ShipVerdict::RereadOnly
        }
        Target::Running => ShipVerdict::WaitForIdle,
    }
}

/// The findings for a verification plan.
///
/// Per-probe findings critique only probes that claim to observe the
/// system ([`ProbeKind::Effect`]); a re-read is already covered by the
/// plan-level `REREAD_ONLY` finding and does not carry its own expected
/// signal:
///
/// - `PROBE_NO_SIGNAL`: an effect probe that cannot name the signal a
///   healthy system produces reads "nothing found" as a pass
///   (invariant 2).
/// - `PROBE_OVER_WINDOW`: an effect probe that takes longer than
///   [`VERIFY_WINDOW_SECONDS`] does not answer the 60-second question in
///   time.
///
/// The plan-level finding states the shape of the whole plan against the
/// 60-second question: `REREAD_ONLY` (the plan looks like verification
/// and observes only the edit) or `NO_EFFECT_PROBE` (nothing in the plan
/// answers in time) — each of which means the change waits for an idle
/// system.
pub fn plan_findings(change: &Change) -> Vec<String> {
    let mut findings = Vec::new();
    for (i, probe) in change.probes.iter().enumerate() {
        if probe.kind != ProbeKind::Effect {
            continue;
        }
        if probe.expected_signal.is_empty() {
            findings.push(format!(
                "PROBE_NO_SIGNAL: probe {i} (\"{}\") names no expected signal: a verification that cannot say what healthy looks like reads an empty result as a pass — name the signal or drop the probe",
                probe.command
            ));
        }
        if probe.seconds > VERIFY_WINDOW_SECONDS {
            findings.push(format!(
                "PROBE_OVER_WINDOW: probe {i} (\"{}\") takes {}s: it cannot answer \"how will I know within sixty seconds if it is not\"",
                probe.command, probe.seconds
            ));
        }
    }
    if change.target == Target::Running {
        let verdict = ship_verdict(change);
        if matches!(verdict, ShipVerdict::RereadOnly) {
            findings.push(format!(
                "REREAD_ONLY: the verification plan for {} re-reads the edit ({} probe(s)) — re-reading the change would have caught none of the nine incidents; run the thing and read its output, or the change waits for an idle system",
                change.summary,
                change.probes.len()
            ));
        } else if !matches!(verdict, ShipVerdict::Ship { .. }) {
            findings.push(format!(
                "NO_EFFECT_PROBE: the change {} to a running system has no probe that answers within {}s — \"how will I know within sixty seconds if it is not\" is unanswered, so the change waits for an idle system",
                change.summary, VERIFY_WINDOW_SECONDS
            ));
        }
    }
    findings
}

/// The uncomfortable version, rendered: the answer to "how will I know
/// within sixty seconds if it is not" — or the statement that the change
/// waits for an idle system.
///
/// A design review that accepts a change whose answer is the wait
/// statement has moved the change to a moment when the system is idle; it
/// has not waved it through.
pub fn sixty_second_answer(change: &Change) -> String {
    match ship_verdict(change) {
        ShipVerdict::Ship { probes } => {
            match probes.first() {
                Some(&i) => format!(
                    "within {}s: run \"{}\" and expect \"{}\"",
                    VERIFY_WINDOW_SECONDS,
                    change.probes[i].command,
                    change.probes[i].expected_signal
                ),
                None => "the system is idle: a change there cannot break a live consumer".to_string(),
            }
        }
        ShipVerdict::RereadOnly => {
            "no answer: the plan re-reads the edit, which would have caught none of the nine; the change waits for an idle system".to_string()
        }
        ShipVerdict::WaitForIdle => format!(
            "no answer within {}s: the change waits for a moment when the system is idle",
            VERIFY_WINDOW_SECONDS
        ),
    }
}

/// Invariant 2: the result of running a probe against the system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeOutcome {
    /// The probe ran, and its output carries the expected signal.
    Pass,
    /// The probe ran and produced output, but the expected signal is not
    /// in it — or the probe cannot name a signal at all, in which case
    /// its output is not readable as a pass (fail-closed).
    Failed,
    /// The probe produced no output at all. This is the incident #5
    /// shape: an empty result set that reads as "no problems found". It
    /// is a failure, not a pass.
    NoSignal,
}

/// Invariant 2, as a function: read a probe's output against the probe.
///
/// - Empty output (after trimming) is [`ProbeOutcome::NoSignal`] — a
///   verification that returns nothing is a failure, not a pass, at any
///   repetition count.
/// - Non-empty output is a pass only when it carries the probe's expected
///   signal. A probe with no named signal cannot read its output as a
///   pass: it is [`ProbeOutcome::Failed`], fail-closed.
pub fn run_probe(probe: &Probe, output: &str) -> ProbeOutcome {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return ProbeOutcome::NoSignal;
    }
    if !probe.expected_signal.is_empty() && trimmed.contains(&probe.expected_signal) {
        ProbeOutcome::Pass
    } else {
        ProbeOutcome::Failed
    }
}

/// The line a probe result renders as. The `NoSignal` line is the whole of
/// invariant 2: it names the probe and states that the emptiness is a
/// failure, so the next reader cannot file it as "no problems found".
pub fn outcome_line(probe: &Probe, outcome: &ProbeOutcome) -> String {
    match outcome {
        ProbeOutcome::Pass => format!(
            "pass: {} produced the expected signal (\"{}\")",
            probe.command, probe.expected_signal
        ),
        ProbeOutcome::Failed => format!(
            "fail: {} ran but its output does not carry the expected signal (\"{}\")",
            probe.command, probe.expected_signal
        ),
        ProbeOutcome::NoSignal => format!(
            "fail: {} returned nothing — a verification that returns nothing is a failure, not a pass",
            probe.command
        ),
    }
}

/// Invariant 3: the blast radius of a change to shared state (issue
/// #3732). The three questions are asked *before* the edit, because after
/// the edit the answer is already a post-mortem:
///
/// - who reads this path,
/// - what do they do if it changes mid-read,
/// - what makes the change exclusive with respect to them.
///
/// Incident #3 (a comment inserted inside `sbatch` line-continuations)
/// took the dispatcher down for ~10 minutes because none of the three
/// questions had been asked.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BlastRadius {
    /// Who reads this path.
    pub readers: Vec<String>,
    /// What a reader does if the path changes mid-read.
    pub on_mid_read_change: String,
    /// What makes the change exclusive with respect to readers.
    pub exclusivity: String,
}

/// Invariant 3, as a check: the findings for a blast radius that has not
/// been fully stated. Each missing question is its own finding, because
/// each one was the missing question in a real incident — and "nobody
/// reads this" is a claim that needs the same evidence as any other, not
/// a default.
pub fn blast_radius_findings(radius: &BlastRadius) -> Vec<String> {
    let mut findings = Vec::new();
    if radius.readers.is_empty() {
        findings.push(
            "BLAST_NO_READERS: the change to shared state names no readers — who reads this path is the first blast-radius question, and \"nobody\" is a claim, not a default".to_string(),
        );
    }
    if radius.on_mid_read_change.trim().is_empty() {
        findings.push(
            "BLAST_NO_MID_READ: the change to shared state does not say what the readers do if the path changes mid-read — the #3732 question".to_string(),
        );
    }
    if radius.exclusivity.trim().is_empty() {
        findings.push(
            "BLAST_NO_EXCLUSIVITY: the change to shared state does not say what makes it exclusive with respect to its readers".to_string(),
        );
    }
    findings
}

/// Invariant 4: a gate as the repository defines it, and the gate actually
/// applied to a change.
///
/// Incident #6: the supervisor hand-ran `clippy -D warnings`, stricter
/// than the repository's CI, and the lints it fired do not exist as
/// failures on main — the nearly-discarded good patch was rejected by a
/// gate the repository never defined.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateApplication {
    /// The gate's name (`clippy`, `fmt`, ...).
    pub gate: String,
    /// How the repository's definition runs it (its CI).
    pub repo_flags: Vec<String>,
    /// How the gate was actually applied.
    pub applied_flags: Vec<String>,
}

/// Invariant 4, as a check: the finding for a gate that is not the
/// repository's definition.
///
/// A gate matches the repository's definition or it is a different gate
/// (issue #3753). Both directions are findings and both are named:
/// `adds` is the incident's direction — stricter than the repository's
/// gate, able to reject what the repository accepts — and `drops` is the
/// opposite — looser, able to accept what the repository rejects.
pub fn ad_hoc_gate_findings(application: &GateApplication) -> Vec<String> {
    let added: Vec<&String> = application
        .applied_flags
        .iter()
        .filter(|f| !application.repo_flags.contains(f))
        .collect();
    let dropped: Vec<&String> = application
        .repo_flags
        .iter()
        .filter(|f| !application.applied_flags.contains(f))
        .collect();
    if added.is_empty() && dropped.is_empty() {
        return Vec::new();
    }
    let mut detail = Vec::new();
    if !added.is_empty() {
        detail.push(format!(
            "adds {} (stricter than the repository's gate: it can reject what the repository accepts)",
            added.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(" ")
        ));
    }
    if !dropped.is_empty() {
        detail.push(format!(
            "drops {} (looser than the repository's gate: it can accept what the repository rejects)",
            dropped.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(" ")
        ));
    }
    vec![format!(
        "AD_HOC_GATE: the applied `{}` gate ({}) is not the repository's definition ({}): {} — a gate matches the repository's definition or it is a different gate",
        application.gate,
        application.applied_flags.join(" "),
        application.repo_flags.join(" "),
        detail.join("; ")
    )]
}

/// Invariant 5, as a decision: choose among candidate changes for a
/// system.
///
/// The verifiability filter runs *before* the quality ranking: among the
/// changes that [`ship_verdict`] lets ship, the one with the highest
/// [`Change::value`] wins (first index on ties). A change that cannot be
/// verified within the window is never chosen, no matter how much better
/// it is — incident #5 was a genuine improvement to load distribution
/// that broke a working system under load, and the ranking it replaced
/// was worse and total. Returns `None` when no candidate may ship: the
/// honest answer to "which change next" is sometimes "none, until the
/// system is idle".
pub fn choose_change(changes: &[Change]) -> Option<usize> {
    let mut best: Option<(u32, usize)> = None;
    for (i, change) in changes.iter().enumerate() {
        if matches!(ship_verdict(change), ShipVerdict::Ship { .. }) {
            let is_better = match best {
                None => true,
                Some((v, _)) => change.value > v,
            };
            if is_better {
                best = Some((change.value, i));
            }
        }
    }
    best.map(|(_, i)| i)
}
