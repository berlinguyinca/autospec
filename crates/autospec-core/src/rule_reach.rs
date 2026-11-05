//! A rule that routes work must state the capability it assumes (#4576).
//!
//! Two standing rules contradict each other when the defect is outside the
//! repository. "Fix any error you find first — errors corrupt everything
//! downstream" and "fix via autospec, not by hand: observed errors become
//! issues that agents fix". The second is right about this repo, but it
//! silently assumes every defect is reachable by an agent dispatched against
//! it. The dispatcher, worker launcher, reconciler and base-refresh logic
//! live as scripts on the cluster, in a path no agent checks out. For those,
//! the rule says "file an issue and an agent will fix it" — and a defect that
//! stalled agents for hours (#4571) sat unfixed across several sessions while
//! its issue was dutifully filed, well evidenced, and completely inert: no
//! agent could ever be dispatched against the file it described.
//!
//! The generalisable point: **a process rule that assumes a capability must
//! state that assumption.** When a rule routes work somewhere, verify the
//! destination can actually perform it, and make the rule say what to do when
//! it cannot. Following the rule where it has no reach is indistinguishable
//! from doing nothing, and it *feels* like diligence, which is why it
//! persisted for hours across sessions of otherwise careful work. It is the
//! same shape as a label with no consumer (#4565) and a readiness predicate
//! gated on a permanent property (InferWeave #326): the producer side
//! succeeded and nothing checked the consumer.
//!
//! Invariants, each encoded as a checkable primitive:
//!
//! 1. **A rule that routes work must state its reach and its escape hatch.**
//!    [`Rule`] carries whether it states the reach of the destination it
//!    routes to and whether it says what to do when the destination cannot
//!    perform the work; [`assumption_findings`] is the lint.
//! 2. **The no-hand-patch rule has a scope.** In-reach defects are never
//!    hand-patched — a hand fix here competes with the product code that
//!    should carry the behaviour. Out-of-reach defects get a guarded hand
//!    stopgap plus a companion issue. [`Reach`], [`Disposition`],
//!    [`handling_findings`].
//! 3. **A hand stopgap is well-formed only in a specific shape.** Minimal,
//!    guarded, reverted-on-failure, and paired with a companion issue that is
//!    labelled unreachable (so it is never queued for an agent) and stays
//!    open. [`Stopgap`], [`stopgap_findings`].
//! 4. **The stopgap is not the fix.** It is the bleeding stopped; the
//!    companion issue closes only when the defect is fixed for real, in the
//!    eventual Rust implementation — never when the stopgap lands.
//!
//! Everything here is pure: the caller observes the rule, the defect and how
//! it was handled, and acts on the findings.

/// Whether a defect lies within the reach of the destination a rule routes
/// work to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// The destination (an agent dispatched against this repo) can act on the
    /// file the defect describes.
    InReach,
    /// No agent can be dispatched against the file the defect describes:
    /// cluster scripts, cron, infrastructure — a path no agent checks out.
    OutOfReach,
}

/// A process rule that routes work to a destination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    /// The rule's name (e.g. `no hand-patching`).
    pub name: &'static str,
    /// Whether the rule states the reach of the destination it routes work
    /// to — where the destination can and cannot act.
    pub states_reach: bool,
    /// Whether the rule says what to do when the destination cannot perform
    /// the work it routes — the escape hatch for the out-of-reach case.
    pub states_escape_hatch: bool,
}

/// Invariant 1, as a lint over a rule.
///
/// - `UNSTATED_CAPABILITY`: the rule routes work to a destination without
///   stating the reach it assumes — the #4576 rule, which assumed every
///   defect is reachable and said nothing when it was not.
/// - `NO_ESCAPE_HATCH`: the rule states a reach but has nothing to do with
///   the defects outside it — routing them is indistinguishable from doing
///   nothing.
pub fn assumption_findings(rule: &Rule) -> Vec<String> {
    let mut findings = Vec::new();
    if !rule.states_reach {
        findings.push(format!(
            "UNSTATED_CAPABILITY: rule '{}' routes work to a destination without stating the \
             reach it assumes (#4576)",
            rule.name
        ));
    }
    if rule.states_reach && !rule.states_escape_hatch {
        findings.push(format!(
            "NO_ESCAPE_HATCH: rule '{}' states a reach but says nothing about the defects \
             outside it — routing them is indistinguishable from doing nothing (#4576)",
            rule.name
        ));
    }
    findings
}

/// How a defect outside a destination's reach was handled by the escape
/// hatch: the hand stopgap that stopped the bleeding.
///
/// The stopgap is not the fix — it is the bleeding stopped. Every field is a
/// requirement for the hand fix to be permitted on an out-of-reach defect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stopgap {
    /// The hand fix is minimal: the smallest change that stops the bleeding,
    /// not a redesign of an unreachable component.
    pub minimal: bool,
    /// The hand fix is guarded: it fails closed rather than opening a new
    /// failure mode in the cluster.
    pub guarded: bool,
    /// The hand fix is reverted if it does not hold — a stopgap that is not
    /// reverted on failure becomes the next permanent defect.
    pub reverted_on_failure: bool,
    /// A companion issue was filed that carries the invariant into the
    /// eventual Rust implementation.
    pub companion_issue: bool,
    /// The companion issue is labelled unreachable, so it is never queued for
    /// an agent that cannot act on it.
    pub issue_labelled_unreachable: bool,
    /// Whether the companion issue was closed when the stopgap landed. It
    /// must not be: the stopgap is not the fix.
    pub issue_closed_on_stopgap: bool,
}

/// Invariant 3, as a lint over a hand stopgap. Every finding is a shape the
/// permitted hand fix must have for an out-of-reach defect.
pub fn stopgap_findings(stopgap: &Stopgap) -> Vec<String> {
    let mut findings = Vec::new();
    if !stopgap.minimal {
        findings.push(
            "STOPGAP_NOT_MINIMAL: the hand fix is not minimal — the stopgap is the bleeding \
             stopped, not a redesign (#4576)"
                .to_string(),
        );
    }
    if !stopgap.guarded {
        findings.push(
            "STOPGAP_NOT_GUARDED: the hand fix is not guarded — it must fail closed, not open a \
             new cluster failure mode (#4576)"
                .to_string(),
        );
    }
    if !stopgap.reverted_on_failure {
        findings.push(
            "STOPGAP_NOT_REVERTED_ON_FAILURE: the hand fix is not reverted if it does not hold — \
             a stopgap that persists becomes the next permanent defect (#4576)"
                .to_string(),
        );
    }
    if !stopgap.companion_issue {
        findings.push(
            "STOPGAP_WITHOUT_ISSUE: no companion issue carries the invariant into the eventual \
             Rust implementation — the stopgap will not survive the port (#4576)"
                .to_string(),
        );
    }
    if stopgap.companion_issue && !stopgap.issue_labelled_unreachable {
        findings.push(
            "STOPGAP_ISSUE_UNLABELLED: the companion issue is not labelled unreachable, so it can \
             be queued for an agent that cannot act on it (#4576)"
                .to_string(),
        );
    }
    if stopgap.companion_issue && stopgap.issue_closed_on_stopgap {
        findings.push(
            "STOPGAP_READ_AS_FIX: the companion issue closed when the stopgap landed — the \
             stopgap is not the fix; the issue stays open until the defect is fixed for real \
             (#4576)"
                .to_string(),
        );
    }
    findings
}

/// How a defect was actually handled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Disposition {
    /// Routed to the destination: an agent was dispatched (or the issue was
    /// queued) against it.
    Dispatched,
    /// A hand stopgap was applied in place of dispatch.
    HandFix(Stopgap),
}

/// Invariant 2, as a lint over how a defect was handled given its reach.
///
/// - `HAND_PATCH_IN_REACH`: the defect is within reach and was hand-patched —
///   the no-hand-patch rule applies in full; a hand fix here competes with
///   the product code that should carry the behaviour.
/// - `UNREACHABLE_DISPATCHED`: the defect is out of reach and was routed to
///   an agent that cannot act on it — the incident. It should have been
///   labelled unreachable (not queued) and stopped with a guarded stopgap.
pub fn handling_findings(reach: &Reach, disposition: &Disposition) -> Vec<String> {
    match (reach, disposition) {
        (Reach::InReach, Disposition::HandFix(_)) => vec![
            "HAND_PATCH_IN_REACH: an in-reach defect was hand-patched — the no-hand-patch rule \
             applies in full; a hand fix competes with the product code that should carry the \
             behaviour (#4576)"
                .to_string(),
        ],
        (Reach::OutOfReach, Disposition::Dispatched) => vec![
            "UNREACHABLE_DISPATCHED: an out-of-reach defect was routed to an agent that cannot \
             act on it — label it unreachable so it is never queued, and stop it with a guarded \
             hand stopgap plus a companion issue (#4576)"
                .to_string(),
        ],
        (Reach::OutOfReach, Disposition::HandFix(stopgap)) => stopgap_findings(stopgap),
        (Reach::InReach, Disposition::Dispatched) => Vec::new(),
    }
}

/// The combined audit for a routing rule and how it handled a defect: the
/// rule's own assumptions plus how the defect was handled against its reach.
///
/// The incident is the empty-handled case: a rule that stated no reach, an
/// out-of-reach defect, and `Dispatched` — findings on both the rule and the
/// handling. A clean result is a rule that stated its reach and its escape
/// hatch and handled an out-of-reach defect with a well-formed stopgap, or an
/// in-reach defect dispatched to the agent that can act on it.
pub fn audit(rule: &Rule, reach: &Reach, disposition: &Disposition) -> Vec<String> {
    let mut findings = assumption_findings(rule);
    findings.extend(handling_findings(reach, disposition));
    findings
}
