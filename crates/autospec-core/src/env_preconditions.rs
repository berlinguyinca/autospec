//! Environmental preconditions and load-aware selection (issue #4224).
//!
//! A fix that rests on an environmental property — a homogeneous fleet,
//! identical context windows, a single gateway node — is only valid while
//! the property holds, and nothing enforces the "while".
//!
//! The incident: Gateway#21 made worker selection load-aware by picking the
//! worker with the most context. On a homogeneous fleet — every worker for
//! a model reporting the same window — that coincided with picking the
//! least-loaded worker, and the fix was right. A 32k-window worker joined a
//! fleet of 256k-window ones, and the same code became load-blind: it
//! stacked requests on the big-window worker and left the free 32k worker
//! idle. The fix had silently reverted; nothing warned, because nothing
//! asserted the homogeneity it depended on.
//!
//! Four invariants, each a primitive here:
//!
//! 1. **A fix that rests on an environmental property must assert that
//!    property in code.** [`window_mismatches`] reports every model whose
//!    workers report differing context windows, and its line is a `WARN:`
//!    that names the model and every window observed. The runtime half is
//!    [`evaluate`]: a precondition whose assertion comes back violated (or
//!    unrunnable — fail-closed) renders a warning on the same pass the
//!    breakage is observed.
//! 2. **Regression tests run in the configuration the bug required.** The
//!    bug required a heterogeneous fleet (mixed context windows); on a
//!    homogeneous fleet every selection rule agrees, so a homogeneous test
//!    cannot see the bug. The tests in `tests/env_preconditions.rs`
//!    instantiate the mixed-window fleet the incident produced.
//! 3. **"Eligible for this request" before "best among candidates".**
//!    [`select_worker`] filters on [`WorkerView::context_window`] ≥ the
//!    request requirement before ranking on free slots. A ranking filter
//!    over the whole candidate set silently re-asserts the assumption that
//!    all candidates are equivalent — which is exactly the property that
//!    broke. The picker is total over answering workers: a fleet with no
//!    eligible worker still names the least-loaded one, flagged
//!    not-eligible, and [`admit`] is the separate decision that holds a
//!    zero-free-slot worker rather than dispatching it. [`verdict`]
//!    combines the two for dispatchers.
//! 4. **Record the conditions under which the fix holds when the issue
//!    closes.** [`Precondition`] carries the property in human terms and
//!    the runtime check that asserts it, and renders the `Valid while:`
//!    line of a closeout. Construction rejects a precondition with no named
//!    assertion: that is a precondition with no expiry, and it is how this
//!    incident happened.
//!
//! Everything here is pure: no I/O, no clock, no subprocesses. The caller
//! supplies the worker views from the live fleet state and reports the
//! result of re-running an assertion as an [`Observation`].

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// One worker as the fleet dispatcher sees it: identity, model, the context
/// window it reports, and the free slots it reports right now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerView {
    /// Stable worker identity (hostname or job id).
    pub id: String,
    /// The model this worker serves.
    pub model: String,
    /// The context window this worker reports, in tokens.
    pub context_window: u64,
    /// The free slots this worker reports right now.
    pub free_slots: u64,
}

/// A model whose workers report more than one distinct context window
/// (invariant 1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowMismatch {
    /// The model the differing windows were reported under.
    pub model: String,
    /// One entry per worker, sorted by worker id: (worker id, reported
    /// window in tokens).
    pub reported: Vec<(String, u64)>,
    /// The distinct windows, sorted ascending. Always more than one entry.
    pub distinct: Vec<u64>,
}

impl WindowMismatch {
    /// The `WARN:` line a runtime check emits when it observes this. Names
    /// the model, the count of distinct windows, and every worker's window,
    /// so the operator sees exactly which property of the fleet broke —
    /// not that "something changed".
    pub fn warn_line(&self) -> String {
        let detail = self
            .reported
            .iter()
            .map(|(id, window)| format!("{id}: {window}"))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "WARN: model {} reports {} distinct context windows across its {} workers ({}); a fix that assumes one window per model is not valid for this fleet",
            self.model,
            self.distinct.len(),
            self.reported.len(),
            detail
        )
    }
}

/// Group the fleet by model and report every model whose workers report
/// more than one distinct context window. A homogeneous fleet returns an
/// empty vec — there is nothing to warn about, and a check that warns on a
/// healthy fleet teaches no one to read it.
pub fn window_mismatches(workers: &[WorkerView]) -> Vec<WindowMismatch> {
    let mut by_model: BTreeMap<&str, Vec<(String, u64)>> = BTreeMap::new();
    for worker in workers {
        by_model
            .entry(worker.model.as_str())
            .or_default()
            .push((worker.id.clone(), worker.context_window));
    }
    by_model
        .into_iter()
        .filter_map(|(model, mut reported)| {
            reported.sort();
            let mut distinct: Vec<u64> = reported.iter().map(|&(_, window)| window).collect();
            distinct.sort();
            distinct.dedup();
            (distinct.len() > 1).then(|| WindowMismatch {
                model: model.to_string(),
                reported,
                distinct,
            })
        })
        .collect()
}

/// The picker's answer (invariant 3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    /// A worker was named. `eligible` says whether the named worker meets
    /// the request's context requirement. When no worker does, the picker
    /// is still total: it names the least-loaded worker of the whole fleet
    /// and flags it not-eligible — the refusal is the admission decision's
    /// job (or the caller's), not the picker's.
    Selected { worker: String, eligible: bool },
    /// No worker answered at all (or every worker was excluded as a prior
    /// pick). Selection is not a guess: an empty fleet is named as such.
    NoWorkers,
}

/// The admission decision, separate from the picker (invariant 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Admission {
    /// The worker has a free slot; a request may be dispatched to it.
    Dispatch,
    /// The worker has no free slot; the request is held, not dispatched.
    Hold,
}

/// What a dispatcher should do with a selection: dispatch only when the
/// named worker both meets the request's requirement (the capability
/// filter) and has a free slot (the admission decision).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Eligible and admitted.
    Dispatch { worker: String },
    /// A worker meets the requirement but the one named has no free slot;
    /// hold and retry on the next pass. The named worker is reported so a
    /// saturated fleet still names its least-loaded member.
    HeldSaturated { worker: String },
    /// No worker meets the request's requirement. This is a capability
    /// gap, not a load problem — holding is named as such, and the named
    /// worker (the least-loaded of the fleet) is reported, never dispatched
    /// to.
    HeldIncapable { worker: String },
    /// No worker answered at all.
    NoWorkers,
}

/// A zero-free-slot worker is held, not dispatched — even (especially)
/// when the picker named it as the only eligible one.
pub fn admit(worker: &WorkerView) -> Admission {
    if worker.free_slots > 0 {
        Admission::Dispatch
    } else {
        Admission::Hold
    }
}

/// Does `a` beat `b` for the request: more free slots, or the same load
/// with the lexicographically smaller id. Strict and total, so the pick is
/// deterministic without an unstated tie-break.
fn dominates(a: &WorkerView, b: &WorkerView) -> bool {
    match a.free_slots.cmp(&b.free_slots) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Equal => a.id < b.id,
    }
}

/// The pick, as (index into `workers`, whether the named worker meets the
/// requirement). Shared by [`select_worker`] and [`verdict`] so the two
/// can never disagree about who was named.
fn pick(workers: &[WorkerView], required_window: u64, exclude: &[String]) -> Option<(usize, bool)> {
    let all: Vec<usize> = (0..workers.len())
        .filter(|&i| !exclude.iter().any(|excluded| *excluded == workers[i].id))
        .collect();
    if all.is_empty() {
        return None;
    }
    // The capability filter runs first and compares each worker against
    // the request requirement — never workers against each other. The
    // ranking filter runs second, inside the eligible set, on free slots.
    let eligible: Vec<usize> = all
        .iter()
        .copied()
        .filter(|&i| workers[i].context_window >= required_window)
        .collect();
    let (pool, is_eligible) = if eligible.is_empty() {
        // Total over answering workers: name the least-loaded of the whole
        // fleet and flag it not-eligible.
        (all.as_slice(), false)
    } else {
        (eligible.as_slice(), true)
    };
    let mut best: Option<usize> = None;
    for &index in pool {
        match best {
            None => best = Some(index),
            Some(current) if dominates(&workers[index], &workers[current]) => best = Some(index),
            Some(_) => {}
        }
    }
    best.map(|index| (index, is_eligible))
}

/// Pick one worker for a request that needs `required_window` tokens of
/// context.
///
/// The eligibility filter (context window ≥ requirement) runs before the
/// ranking filter (most free slots, ties broken by worker id), per
/// invariant 3. `exclude` names workers already picked earlier in the same
/// dispatch pass, so a pass never stacks on one worker before live state
/// has caught up (the worker-selection invariant, issue #3929).
pub fn select_worker(
    workers: &[WorkerView],
    required_window: u64,
    exclude: &[String],
) -> Selection {
    match pick(workers, required_window, exclude) {
        Some((index, eligible)) => Selection::Selected {
            worker: workers[index].id.clone(),
            eligible,
        },
        None => Selection::NoWorkers,
    }
}

/// The dispatch verdict for a request: the capability filter and the
/// admission decision combined, so a dispatcher cannot dispatch a worker
/// that is free but incapable, or hold an incapable fleet as if it were
/// merely saturated.
pub fn verdict(workers: &[WorkerView], required_window: u64, exclude: &[String]) -> Verdict {
    match pick(workers, required_window, exclude) {
        None => Verdict::NoWorkers,
        Some((index, true)) if workers[index].free_slots > 0 => Verdict::Dispatch {
            worker: workers[index].id.clone(),
        },
        Some((index, true)) => Verdict::HeldSaturated {
            worker: workers[index].id.clone(),
        },
        Some((index, false)) => Verdict::HeldIncapable {
            worker: workers[index].id.clone(),
        },
    }
}

/// A condition under which a fix holds, recorded when the issue closes
/// (invariant 4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Precondition {
    /// Stable id, e.g. `fleet.qwen3.8-27b:ctx-window-homogeneous`.
    pub id: String,
    /// The property in human terms, present tense: "all qwen3.8-27b workers
    /// report the same context window".
    pub holds: String,
    /// The runtime check that asserts the property — the command or module
    /// function a later run re-executes to learn whether the fix still
    /// holds. A precondition with no named assertion is rejected at
    /// construction: it would be a precondition with no expiry.
    pub asserted_by: String,
}

impl Precondition {
    /// Construction refuses blank fields: the id, the property, and the
    /// check that asserts it are all load-bearing, and a closeout that
    /// records "valid while: (asserted by )" has recorded nothing.
    pub fn new(
        id: impl Into<String>,
        holds: impl Into<String>,
        asserted_by: impl Into<String>,
    ) -> Result<Self, String> {
        let id: String = id.into();
        let holds: String = holds.into();
        let asserted_by: String = asserted_by.into();
        if id.trim().is_empty() {
            return Err("precondition id must not be blank".to_string());
        }
        if holds.trim().is_empty() {
            return Err(format!(
                "precondition {id} must state the property it carries"
            ));
        }
        if asserted_by.trim().is_empty() {
            return Err(format!(
                "precondition {id} names no assertion: a precondition that no check re-verifies has no expiry"
            ));
        }
        Ok(Self {
            id,
            holds,
            asserted_by,
        })
    }

    /// The closeout line: the condition under which the fix holds, and the
    /// check a future run executes to find out.
    pub fn line(&self) -> String {
        format!(
            "Valid while: {} (asserted by {})",
            self.holds, self.asserted_by
        )
    }
}

/// The preconditions of a fix, recorded at close (invariant 4).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreconditionSet {
    #[serde(default)]
    preconditions: Vec<Precondition>,
}

impl PreconditionSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a precondition. Re-adding an id supersedes the stored line
    /// rather than duplicating it: two lines carrying the same property
    /// would drift, and nothing would say which was authoritative.
    pub fn add(&mut self, precondition: Precondition) {
        if let Some(slot) = self
            .preconditions
            .iter_mut()
            .find(|stored| stored.id == precondition.id)
        {
            *slot = precondition;
        } else {
            self.preconditions.push(precondition);
        }
    }

    pub fn get(&self, id: &str) -> Option<&Precondition> {
        self.preconditions.iter().find(|p| p.id == id)
    }

    pub fn len(&self) -> usize {
        self.preconditions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.preconditions.is_empty()
    }

    /// The `Valid while:` lines for a closeout. An empty set renders
    /// nothing: a fix with no environmental preconditions is still a fix,
    /// and a closeout does not invent them.
    pub fn valid_while_lines(&self) -> Vec<String> {
        self.preconditions.iter().map(Precondition::line).collect()
    }
}

/// What a re-run of a precondition's assertion observed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Observation {
    /// The property still holds.
    Holds,
    /// The property no longer holds, with what the check actually saw.
    Violated { observed: String },
    /// The check itself could not run. Fail-closed: never read as
    /// [`Observation::Holds`] — an assertion that could not run is not an
    /// assertion that passed.
    Unrunnable { detail: String },
}

/// The warning lines a precondition evaluation produces (invariant 1, the
/// runtime half). [`Observation::Holds`] renders nothing; a violation and
/// an unrunnable assertion each render a `WARN:` line naming the
/// precondition, the property, and — for a violation — the observed value.
/// A fix whose precondition has broken is not a fix, and the fleet says so
/// on the same pass the breakage is observed, instead of silently re-
/// versioning to the pre-fix behavior.
pub fn evaluate(precondition: &Precondition, observation: &Observation) -> Vec<String> {
    match observation {
        Observation::Holds => Vec::new(),
        Observation::Violated { observed } => vec![format!(
            "WARN: precondition {} no longer holds — {}; observed: {observed}",
            precondition.id, precondition.holds
        )],
        Observation::Unrunnable { detail } => vec![format!(
            "WARN: precondition {} could not be re-asserted via {}: {detail} — treat as at risk, not as holding",
            precondition.id, precondition.asserted_by
        )],
    }
}
