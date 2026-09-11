//! Evidence bound to the wrong entity (issue #4264).
//!
//! Seven wrong conclusions in one session, none of them a reasoning
//! error. In every case the inference from the measurement was sound;
//! the *measurement described something other than what was thought*:
//!
//! | # | measured | believed it described | actually described |
//! |---|---|---|---|
//! | 1 | `qwen3.8-27b-*` endpoint files | the 27b workers | 27b **plus vision** (prefix collision) |
//! | 2 | `$LLM/*/out/issue-*` | autospec patches | **four projects** with colliding issue numbers |
//! | 3 | hive gateway `served_503=0` | the gateway users hit | the **other** gateway |
//! | 4 | "a gateway is running" | InferWeave#272's gateway | a **different program in a different repo** |
//! | 5 | `cargo` at 0.3% CPU | the build is wedged | the **supervisor**, idle by design |
//! | 6 | agent `.out` mtime | agent progress | a 264-byte banner, **identical for healthy and hung agents** |
//! | 7 | `awk '$1>="19:45"'` on a log | today's lines | **any day's** lines — string compare, no date |
//!
//! Three of these nearly caused destructive action. And unlike a logic
//! error, this class **survives review of the reasoning**: the inference
//! is valid; only the binding between measurement and entity is wrong —
//! and that binding is exactly what does not appear in the write-up.
//!
//! The discipline: before any measurement becomes a claim — and
//! especially before a destructive action — answer four questions, in
//! writing. Each is a primitive here, and [`audit`] answers all four for
//! one claim at once.
//!
//! 1. **Which entity produced this number?** Name the instance, not the
//!    type. [`Claim::attribution`]: a claim that names no instance, or
//!    that names an instance *other than* the entity the conclusion is
//!    about, is not a fact about that entity. (The hive gateway's
//!    `served_503=0` was a true number; the users hit the edge
//!    instance.)
//! 2. **Could this number be identical if my hypothesis were false?**
//!    [`HealthCheck::discrimination`]: a check whose value is the same
//!    in both states is not evidence for either. The 264-byte banner was
//!    identical for healthy and hung agents; it could never have
//!    distinguished them.
//! 3. **Does the name I matched on uniquely identify what I mean?**
//!    [`name_selection`]: a name that selects anything other than
//!    exactly one known identifier — zero, or two — is not a
//!    measurement of what was thought. The mechanisms that make names
//!    collide (glob prefixes, cross-repo issue numbers, shared component
//!    names) are checkable in [`crate::name_scope`] (issue #4251); this
//!    question is whether the selection was checked at all.
//! 4. **Is this measurement current?** [`is_current`]: a measurement
//!    older than its window is evidence about its mtime, not about now.
//!    A log read without a date is any day's log. A measurement that
//!    carries no time of its own is undated, and undated is not current.
//!
//! And the actuation rule: **counter-evidence is cheaper than
//! confirmation.** In six of the seven cases the correction came from a
//! *second, differently-shaped* measurement — comparing two hosts,
//! dumping a raw payload, reading the issue body, checking a child
//! process. [`action_gate`]: when a measurement supports a conclusion
//! that licenses an irreversible action, one measurement of a
//! *different kind* is required before acting. Counter-evidence of the
//! same kind does not count.
//!
//! Everything here is pure: no I/O, no clock, no subprocess. The caller
//! supplies the instance names, the values, and the timestamps.

use serde::{Deserialize, Serialize};

// ── Q1: which entity produced this number ───────────────────────────────

/// How a claim binds its measurement to the entity the conclusion is
/// about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Attribution {
    /// The measurement was taken on the entity the claim is about.
    Bound,
    /// The measurement names an instance *other than* the entity the
    /// claim is about: the number is real, and it is about something
    /// else.
    WrongInstance,
    /// Nothing names the instance that produced the measurement: the
    /// type is named, the instance is not. "The gateway reports 10
    /// workers" is not a fact until it says *which* gateway.
    Unattributed,
}

/// A claim under the four-question discipline: a measurement bound to
/// the entity the conclusion is about, with what the writer had.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claim {
    /// The entity the conclusion is about.
    pub entity: String,
    /// The instance that produced the measurement. `None` = the type was
    /// named, the instance was not.
    pub instance: Option<String>,
    /// The value observed.
    pub value: String,
    /// The value the measurement would take if the hypothesis were
    /// false — the system broken. `None` = not stated, and the question
    /// is left unanswered; the design-time enforcement point for the
    /// answer is [`HealthCheck`].
    pub value_if_false: Option<String>,
    /// The name the measurement was matched on.
    pub name: String,
    /// The known identifiers that name selected.
    pub selected: Vec<String>,
    /// When the measurement was taken, unix seconds. `None` = the
    /// measurement carries no time of its own: a log read without a date
    /// is any day's log.
    pub taken_at: Option<u64>,
    /// Now, unix seconds.
    pub now: u64,
    /// The window within which a measurement is current.
    pub window_secs: u64,
}

impl Claim {
    /// Q1: does this claim bind its measurement to the entity?
    pub fn attribution(&self) -> Attribution {
        match &self.instance {
            None => Attribution::Unattributed,
            Some(instance) if instance == &self.entity => Attribution::Bound,
            Some(_) => Attribution::WrongInstance,
        }
    }
}

// ── Q2: could the number be identical if the hypothesis were false ──────

/// A health check a spec commissions: what it produces in each state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthCheck {
    /// The check's name.
    pub name: String,
    /// The value the check produces when the system is healthy.
    pub healthy_value: String,
    /// The value the check produces when the system is broken. `None` =
    /// the spec did not state it.
    pub broken_value: Option<String>,
}

/// Whether a check can be evidence at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Discrimination {
    /// The check's value differs between the two states: it can
    /// distinguish them.
    Discriminates,
    /// The check states the same value for healthy and broken: a
    /// constant is not evidence. The 264-byte banner was identical for
    /// healthy and hung agents, and `scancel`-ing on it would have
    /// killed the healthy ones.
    Constant,
    /// The spec did not state what the check produces when the system is
    /// broken: unjudgeable. Unjudgeable is the rejection — at design
    /// time, not in an incident.
    Unstated,
}

impl HealthCheck {
    /// Q2, at design time: can this check distinguish the two states?
    pub fn discrimination(&self) -> Discrimination {
        match &self.broken_value {
            None => Discrimination::Unstated,
            Some(broken) if broken == &self.healthy_value => Discrimination::Constant,
            Some(_) => Discrimination::Discriminates,
        }
    }
}

/// A spec check that cannot be trusted in an incident: it cannot
/// distinguish the two states, or it never stated the broken value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UntrustedCheck {
    /// The check's name.
    pub name: String,
    /// Why it cannot be trusted.
    pub why: Discrimination,
}

/// The spec rule, made checkable: a spec commissioning a health check
/// states what value the check produces when the system is broken, and
/// a check that cannot distinguish the two states is rejected at design
/// time rather than trusted in an incident.
pub fn untrusted_checks(spec: &[HealthCheck]) -> Vec<UntrustedCheck> {
    spec.iter()
        .filter_map(|check| {
            let why = check.discrimination();
            (why != Discrimination::Discriminates).then(|| UntrustedCheck {
                name: check.name.clone(),
                why,
            })
        })
        .collect()
}

// ── Q3: does the name uniquely identify what was meant ──────────────────

/// What a name selected, against the known identifiers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NameSelection {
    /// Exactly one known identifier: the name uniquely identifies.
    Unique,
    /// Zero known identifiers: the name selects nothing — there is no
    /// entity for the measurement to be about.
    SelectsNothing,
    /// Two or more known identifiers: the name selects more than what
    /// was meant. `qwen3.8-27b-*` selects the 27b workers *and* the
    /// vision workers.
    Ambiguous {
        /// The identifiers selected, sorted.
        selected: Vec<String>,
    },
}

/// Q3: does the name select exactly one known identifier?
pub fn name_selection(selected: &[String]) -> NameSelection {
    if selected.is_empty() {
        return NameSelection::SelectsNothing;
    }
    if selected.len() == 1 {
        return NameSelection::Unique;
    }
    let mut sorted = selected.to_vec();
    sorted.sort();
    sorted.dedup();
    if sorted.len() == 1 {
        return NameSelection::Unique;
    }
    NameSelection::Ambiguous { selected: sorted }
}

// ── Q4: is the measurement current ──────────────────────────────────────

/// A measurement taken at `taken_at` is current at `now` for a window
/// of `window_secs` only while its age is inside the window. Exactly at
/// the window edge is not *older than* the window, so it is still
/// current. A clock that rewinds (taken after `now`) is zero age, never
/// an underflow.
pub fn is_current(now: u64, taken_at: u64, window_secs: u64) -> bool {
    now.saturating_sub(taken_at) <= window_secs
}

// ── The audit: all four questions for one claim ─────────────────────────

/// What is wrong with how a claim binds its measurement to an entity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BindingFinding {
    /// Q1: nothing names the instance that produced the measurement.
    Unattributed,
    /// Q1: the measurement names an instance other than the entity the
    /// claim is about.
    WrongInstance {
        /// The instance the measurement came from.
        instance: String,
        /// The entity the claim is about.
        entity: String,
    },
    /// Q2: the measurement takes the same value under the hypothesis
    /// and under its negation. It is not evidence either way.
    Indistinguishable,
    /// Q3: the name selected no known identifier.
    SelectsNothing {
        /// The name as written.
        name: String,
    },
    /// Q3: the name selected more than one known identifier.
    AmbiguousName {
        /// The name as written.
        name: String,
        /// The identifiers it selected, sorted.
        selected: Vec<String>,
    },
    /// Q4: the measurement carries no time of its own, so its currency
    /// cannot be established.
    Undated,
    /// Q4: the measurement is older than its window.
    Stale {
        /// Its age in seconds at `now`.
        age_secs: u64,
        /// The window in seconds.
        window_secs: u64,
    },
}

impl BindingFinding {
    /// The finding, rendered: the question number first, so a report of
    /// findings reads as the discipline being answered.
    pub fn line(&self) -> String {
        match self {
            BindingFinding::Unattributed => {
                "Q1: unattributed — nothing names the instance that produced the measurement"
                    .to_string()
            }
            BindingFinding::WrongInstance { instance, entity } => {
                format!("Q1: wrong instance — measured {instance}, claimed about {entity}")
            }
            BindingFinding::Indistinguishable => {
                "Q2: indistinguishable — the value is identical under the hypothesis and its \
negation; not evidence either way"
                    .to_string()
            }
            BindingFinding::SelectsNothing { name } => {
                format!("Q3: {name:?} selected no known identifier")
            }
            BindingFinding::AmbiguousName { name, selected } => format!(
                "Q3: {name:?} selected {} known identifiers: {}",
                selected.len(),
                selected.join(", ")
            ),
            BindingFinding::Undated => {
                "Q4: undated — the measurement carries no time of its own; it is not \
current evidence"
                    .to_string()
            }
            BindingFinding::Stale {
                age_secs,
                window_secs,
            } => format!(
                "Q4: stale — measured {}s before now, outside the {}s window",
                age_secs, window_secs
            ),
        }
    }
}

/// Answer all four questions for one claim. Findings are in question
/// order (Q1, Q2, Q3, Q4); a claim with no findings is one whose
/// measurement is bound to the entity the conclusion is about.
pub fn audit(claim: &Claim) -> Vec<BindingFinding> {
    let mut findings = Vec::new();

    match claim.attribution() {
        Attribution::Bound => {}
        Attribution::Unattributed => findings.push(BindingFinding::Unattributed),
        Attribution::WrongInstance => {
            let instance = claim
                .instance
                .clone()
                .expect("WrongInstance implies a named instance");
            findings.push(BindingFinding::WrongInstance {
                instance,
                entity: claim.entity.clone(),
            });
        }
    }

    if let Some(false_value) = &claim.value_if_false {
        if false_value == &claim.value {
            findings.push(BindingFinding::Indistinguishable);
        }
    }

    match name_selection(&claim.selected) {
        NameSelection::Unique => {}
        NameSelection::SelectsNothing => findings.push(BindingFinding::SelectsNothing {
            name: claim.name.clone(),
        }),
        NameSelection::Ambiguous { selected } => {
            findings.push(BindingFinding::AmbiguousName {
                name: claim.name.clone(),
                selected,
            });
        }
    }

    match claim.taken_at {
        None => findings.push(BindingFinding::Undated),
        Some(taken_at) => {
            if !is_current(claim.now, taken_at, claim.window_secs) {
                findings.push(BindingFinding::Stale {
                    age_secs: claim.now.saturating_sub(taken_at),
                    window_secs: claim.window_secs,
                });
            }
        }
    }

    findings
}

// ── Counter-evidence is cheaper than confirmation ───────────────────────

/// An action a measurement licenses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Action {
    /// Whether the action can be undone.
    pub irreversible: bool,
    /// The kind of the measurement that licenses the action — the shape
    /// of the evidence: a log read, a counter, a process state, a host
    /// comparison, a raw payload, a source document.
    pub licensing_kind: String,
    /// The kinds of the counter-evidence taken.
    pub counter_kinds: Vec<String>,
}

/// The gate on an action a measurement licenses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionGate {
    /// The action is reversible: the licensing measurement stands
    /// alone.
    Reversible,
    /// The irreversible action has counter-evidence of a different kind
    /// than the licensing measurement: proceed.
    Cleared {
        /// The kind of the counter-evidence that cleared the gate.
        counter_kind: String,
    },
    /// The irreversible action is licensed by a single kind of
    /// measurement: take one measurement of a *different* kind before
    /// acting. Counter-evidence of the same kind does not count — in
    /// six of the seven cases the correction came from a second,
    /// differently-shaped measurement, and a third log read of the same
    /// log would have said the same wrong thing.
    NeedsCounterEvidence {
        /// The kind of the licensing measurement.
        licensing_kind: String,
    },
}

impl ActionGate {
    /// The gate, rendered.
    pub fn line(&self) -> String {
        match self {
            ActionGate::Reversible => {
                "reversible action — the licensing measurement stands alone".to_string()
            }
            ActionGate::Cleared { counter_kind } => {
                format!("cleared by counter-evidence of kind {counter_kind:?}")
            }
            ActionGate::NeedsCounterEvidence { licensing_kind } => format!(
                "hold: irreversible action licensed only by {licensing_kind:?} — \
take one measurement of a different kind before acting"
            ),
        }
    }
}

/// The actuation rule: when a measurement supports a conclusion that
/// licenses an irreversible action, one measurement of a different kind
/// is required before acting. It cost minutes each time in the incident,
/// and it prevented three bad outcomes.
pub fn action_gate(action: &Action) -> ActionGate {
    if !action.irreversible {
        return ActionGate::Reversible;
    }
    match action
        .counter_kinds
        .iter()
        .find(|kind| *kind != &action.licensing_kind)
    {
        Some(counter_kind) => ActionGate::Cleared {
            counter_kind: counter_kind.clone(),
        },
        None => ActionGate::NeedsCounterEvidence {
            licensing_kind: action.licensing_kind.clone(),
        },
    }
}
