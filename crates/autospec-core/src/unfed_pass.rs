//! A pass given no work and a pass with no work (issue #4296).
//!
//! `convpass.sh` took its work positionally:
//!
//! ```sh
//! for n in "$@"; do
//!   ...
//! done
//! say "######## convpass: converted=$conv held=$held skipped=$skip ########"
//! ```
//!
//! Run with no arguments, the loop body never executes and the pass prints
//! `######## convpass: converted=0 held=0 skipped=0 ########` —
//! byte-identical to a healthy pass that examined every patch and correctly
//! found nothing to do. Meanwhile `convselect.sh` — a separate tool the
//! caller must remember to wire in — reported four waiting candidates. The
//! failure is stable and quiet: a pass that reports `converted=0` every run
//! looks like a backlog that happens to be empty, and the longer it runs the
//! more normal it looks. The sibling file already had the fix — `iwconv.sh`
//! says `nothing to convert (no issues given)` — the same divergence recorded
//! in #3741.
//!
//! Invariants, each encoded as a checkable primitive:
//!
//! 1. "I was given no work" and "there is no work" print different lines:
//!    any loop whose body may execute zero times has an explicit
//!    empty-input branch before the summary (`unfed_line` vs
//!    `examined_line`; `identical_summary_finding` flags a summary pair
//!    that does not tell the two apart).
//! 2. Selection and execution are not separately invocable without a
//!    default: a bare invocation runs the selector itself or refuses
//!    (`plan_invocation`, `BareDefault`); a bare run that would execute
//!    with no candidates, or a refusal that does not name the selector,
//!    is a finding (`bare_invocation_finding`).
//! 3. An exit-trap summary reflects the exit path: a run that ended on a
//!    guard names the guard and does not re-state counters that were never
//!    populated (`trap_line`, `trap_line_findings`).
//! 4. A counter of zero is not evidence of work performed: the summary
//!    reports the size of the input the pass was handed — `examined=N`
//!    alongside `converted`/`held`/`skipped` — so an unfed run is
//!    self-evident (`PassCounters::examined`, `missing_examined_finding`).

/// The counters a pass reports after it has been through its candidate list.
///
/// [`PassCounters::examined`] is the size of the input the pass was handed;
/// the other three are what it did with it. A zero counter says nothing
/// about work performed until it is read against `examined` (invariant 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PassCounters {
    pub examined: usize,
    pub converted: usize,
    pub held: usize,
    pub skipped: usize,
}

impl PassCounters {
    /// A pass cannot have acted on more items than it examined.
    ///
    /// `converted + held + skipped > examined` is a state that cannot exist;
    /// a report whose counters do not reconcile is reporting a state that
    /// cannot exist (the same discipline as the frontier counts in
    /// `stored_output`).
    pub fn reconciles(&self) -> bool {
        self.converted + self.held + self.skipped <= self.examined
    }
}

/// Invariant 1: the line a pass prints when the caller handed it no work.
///
/// This is the sibling fix `iwconv.sh` already had:
/// `nothing to convert (no issues given -- run convpass.sh $(convselect.sh))`.
/// It names the selector, because invariant 2 makes the selector the default
/// feed — the line is both the diagnosis and the remedy. `tool` is the pass
/// name the banner carries (`convpass`); `script` is the invocation the
/// remedy names (`convpass.sh`).
pub fn unfed_line(tool: &str, script: &str, selector: &str) -> String {
    format!(
        "######## {tool}: nothing to convert (no issues given -- run {script} $({selector})) ########"
    )
}

/// Invariant 1 + 4: the line a pass prints when it was given work and
/// worked through it (possibly finding nothing to do with any of it).
///
/// `examined=N` leads the line: the size of the input is a fact about the
/// run, and `examined=0` here is distinct from an unfed run only because
/// the line that prints it is a different line (invariant 1).
pub fn examined_line(tool: &str, counters: &PassCounters) -> String {
    format!(
        "######## {tool}: examined={} converted={} held={} skipped={} ########",
        counters.examined, counters.converted, counters.held, counters.skipped
    )
}

/// Invariant 1, as a check: the finding when the line an unfed pass prints
/// is byte-identical to the line a fed-but-idle pass prints.
///
/// That identity is the incident: one line that cannot tell "I was given
/// no work" from "there is no work", which is the one line the pass's
/// operating instruction — log a line so a broken loop is distinguishable
/// from an idle one — exists to avoid.
pub fn identical_summary_finding(unfed: &str, idle: &str) -> Vec<String> {
    if unfed == idle {
        vec![
            "UNFED_SUMMARY_IDENTICAL: the line a pass given no work prints is byte-identical to the line a fed pass that found no work prints — one line cannot tell the two apart, so the pass needs its own empty-input branch before the summary".to_string(),
        ]
    } else {
        Vec::new()
    }
}

/// Invariant 2: what a pass does when it is invoked with no arguments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BareDefault {
    /// The pass runs the selector itself when given no arguments:
    /// selection is wired into execution by default, not left to the caller.
    RunSelector,
    /// The pass refuses a bare invocation, naming the selector that would
    /// feed it. Requiring a caller to remember
    /// `convpass.sh $(convselect.sh)` guarantees that one day someone runs
    /// it bare and reads the result as good news — so the bare form must
    /// not be a silent empty run.
    Refuse,
}

/// Invariant 2: the plan for an invocation of the pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvocationPlan {
    /// Run the pass on these candidates.
    Run(Vec<String>),
    /// Refuse the invocation; the string is the refusal line.
    Refuse(String),
}

/// Invariant 2, as a decision: selection and execution are not separately
/// invocable without a default.
///
/// - `given` non-empty: run on the given candidates (the normal form).
/// - `given` empty, [`BareDefault::RunSelector`]: run on the selector's
///   candidates — the selector is the default feed.
/// - `given` empty, [`BareDefault::Refuse`]: refuse, with a line that names
///   the selector, so the remedy is in the diagnostic.
pub fn plan_invocation(
    tool: &str,
    script: &str,
    selector: &str,
    given: &[&str],
    selector_candidates: &[String],
    default: BareDefault,
) -> InvocationPlan {
    if !given.is_empty() {
        return InvocationPlan::Run(given.iter().map(|s| s.to_string()).collect());
    }
    match default {
        BareDefault::RunSelector => InvocationPlan::Run(selector_candidates.to_vec()),
        BareDefault::Refuse => InvocationPlan::Refuse(refuse_line(tool, script, selector)),
    }
}

/// The refusal line for a bare invocation: it names the tool, the absence,
/// and the selector that would feed it.
pub fn refuse_line(tool: &str, script: &str, selector: &str) -> String {
    format!(
        "error: {tool} given no issues: run {script} $({selector}), or pass issue numbers positionally"
    )
}

/// Invariant 2, as a check on an invocation, its configured default, and
/// its plan.
///
/// - A bare invocation under a [`BareDefault::Refuse`] policy whose plan is
///   `Run` with zero candidates is the incident shape: the loop body never
///   executes and the summary line is the one that cannot tell unfed from
///   idle. A bare invocation under [`BareDefault::RunSelector`] with zero
///   candidates is not a finding — the selector ran and found nothing, so
///   `examined=0` is a true idle, and the pass knew it had been fed.
/// - A bare-invocation refusal that does not name the selector tells the
///   caller there is a problem without telling them the feed.
pub fn bare_invocation_finding(
    given: &[&str],
    default: BareDefault,
    plan: &InvocationPlan,
    selector: &str,
) -> Vec<String> {
    let mut findings = Vec::new();
    match plan {
        InvocationPlan::Run(candidates)
            if given.is_empty() && candidates.is_empty() && default == BareDefault::Refuse =>
        {
            findings.push(
                "BARE_RUN_UNFEDED: a bare invocation runs the pass with no candidates: the loop body never executes and the pass reports the same clean summary as an idle one — run the selector by default or refuse".to_string(),
            );
        }
        InvocationPlan::Refuse(line) if !line.contains(selector) => {
            findings.push(format!(
                "REFUSE_MISSING_SELECTOR: the bare-invocation refusal does not name the selector ({selector}) that would feed the pass — a diagnostic that does not name the remedy is half a policy"
            ));
        }
        _ => {}
    }
    findings
}

/// Invariant 3: how the run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitPath {
    /// The pass worked through its candidate list; the counters were
    /// populated and the trap may state them.
    Completed(PassCounters),
    /// The pass ended on the empty-input guard; no counters were ever
    /// populated.
    Guarded,
}

/// Invariant 3: the exit-trap line, which must reflect the exit path.
///
/// A diagnostic that a later handler overwrites with the thing it was
/// correcting is not a fix: a run that ended on a guard must not have its
/// trap line re-state `converted=`/`held=`/`skipped=`, because those
/// counters were never populated and their zeros read as an empty backlog.
pub fn trap_line(tool: &str, rc: i32, exit: &ExitPath) -> String {
    match exit {
        ExitPath::Completed(counters) => format!(
            "######## {tool}: EXIT rc={rc} (examined={} converted={} held={} skipped={}) ########",
            counters.examined, counters.converted, counters.held, counters.skipped
        ),
        ExitPath::Guarded => {
            format!("######## {tool}: TERMINATED rc={rc} on guard (no counters to report) ########")
        }
    }
}

/// The counter keys a guarded exit never populated.
const GUARDED_COUNTERS: &[&str] = &["converted=", "held=", "skipped="];

/// Invariant 3, as a check: the findings for an exit-trap line that does
/// not reflect the exit path.
///
/// - A `Guarded` exit whose trap line restates any of
///   `converted=`/`held=`/`skipped=` re-states counters that were never
///   populated — the incident's `TERMINATED rc=0 (converted=0 held=0
///   skipped=0)` printed directly after the guard line.
/// - A `Completed` exit whose trap line reports the action counters
///   without `examined=` fails invariant 4.
pub fn trap_line_findings(exit: &ExitPath, trap_output: &str) -> Vec<String> {
    match exit {
        ExitPath::Guarded => {
            let restated: Vec<&str> = GUARDED_COUNTERS
                .iter()
                .copied()
                .filter(|key| trap_output.contains(*key))
                .collect();
            if restated.is_empty() {
                Vec::new()
            } else {
                vec![format!(
                    "TRAP_RESTATES_COUNTERS: the exit-trap line restates counters the run never populated ({}) — the guard line it follows is the thing the trap is overwriting with the misleading summary; name the guard instead",
                    restated.join(", ")
                )]
            }
        }
        ExitPath::Completed(_) => missing_examined_finding(trap_output),
    }
}

/// Invariant 4, as a check: the finding for a summary line that reports
/// action counters without the size of the input.
///
/// `converted=`/`held=`/`skipped=` say what the pass did with its input;
/// `examined=` says how much input it had. A line with the first and not
/// the second cannot tell a counter of zero from no input at all.
pub fn missing_examined_finding(line: &str) -> Vec<String> {
    if line.contains("converted=") && !line.contains("examined=") {
        vec![
            "EXAMINED_MISSING: the summary reports action counters (converted=/held=/skipped=) without examined= — a counter of zero is not evidence of work performed; report the size of the input the pass was handed".to_string(),
        ]
    } else {
        Vec::new()
    }
}
