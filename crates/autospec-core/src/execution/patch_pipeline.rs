//! Patch conversion pass policy (#3635, #3698).
//!
//! The step that turns a finished agent patch into a pull request used to be
//! the bottleneck of the pipeline: one exclusive lock on a single git
//! worktree, one patch at a time (checkout, apply, compile-test, commit,
//! push), while a growing queue of finished patches waited and the GPU hours
//! already spent producing them sat idle.
//!
//! The fix is four rules, each encoded here as a pure, testable primitive.
//! Callers perform the git/cargo I/O with the plans these functions return:
//!
//! 1. **N checkouts, N workers.** The lock protects the checkout, not the
//!    operation ([`plan_workers`]). N private worktrees admit N concurrent
//!    converters.
//! 2. **A shared `CARGO_TARGET_DIR` per worker, not per patch**
//!    ([`WorkerPlan::target_dir`]), so the second patch on a worker is an
//!    incremental build. Target dirs stay distinct per worker because cargo
//!    locks the target directory; one shared dir would serialise the workers
//!    again.
//! 3. **The queue is ordered by cost, not by name** ([`order_by_cost`]):
//!    terminal, cheap cases — existing PRs, closed issues, memoized holds,
//!    no-net-changes — run first, and compute is spent only on genuine
//!    candidates. Never lexicographic or filesystem order: the key is
//!    expected value, and every item records the key that ordered it
//!    ([`Patch::ordering_key`]) (#3783).
//! 4. **A memo over (patch identity, base sha) covers every terminal
//!    decision** ([`ConversionMemo`]), not just holds. Identity alone does
//!    not determine the outcome: the same patch rebased onto a new trunk may
//!    compile or not, so the base sha is part of the key.
//!
//! The inputs change continuously — patches arrive every few minutes and the
//! base moves on every merge — so a long-running pass must re-read the
//! world between candidates, not snapshot it once at startup (#3698):
//!
//! 5. **Re-baseline before each candidate, not once.** The caller fetches
//!    `origin/main` immediately before planning each patch and passes that
//!    tip to [`plan_pass`]). A patch whose base is no longer the tip is
//!    flagged [`ScheduledPatch::stale_base`]: re-testing it is cheap
//!    compared with discarding good work.
//! 6. **Absorb patches that appear mid-pass** ([`absorb_new`]). A converter
//!    that runs for hours must see work produced during those hours, or the
//!    newest and most-applicable patches wait longest — the opposite of the
//!    right order.
//! 7. **A verdict is a statement about a pair** — this patch against that
//!    base — and is honored only while its base is still the tip. A `HELD`
//!    computed against an old base is a hypothesis, not a decision:
//!    [`plan_pass`] refuses to report it as a memo hit.
//! 8. **Expected value: verified status, then impact, then newest.**
//!    Within each cost class the agent's own verdict ranks first
//!    ([`order_by_cost`]): a recorded `PASS` goes first — its
//!    verification cost is already paid — no report is neutral, and a
//!    recorded failure goes last, since it is evidence the patch carries
//!    a defect the pass has not yet fixed. Ties on status go to the patch
//!    that unblocks the most downstream issues: it is the
//!    highest-leverage work and converting it promptly unblocks the most
//!    dependents (#3799). Ties on impact fall back to newest-first: the
//!    most recently produced patch sits on the youngest base and is the
//!    one most likely to apply. Every item records its full ordering key
//!    ([`Patch::ordering_key`]) and the pass reports it per item, so the
//!    order of any pair is inspectable from the log alone (#3783).
//! 9. **The worklist is a moving quantity** ([`Worklist`]). A pass that
//!    enumerates its inputs once at startup and then works for hours
//!    presents a snapshot as a queue: the newest work — precisely the work
//!    most likely to matter — is invisible until the next run, and the
//!    summary stays "accurate" the whole time because a patch that was
//!    never enumerated cannot be reported as missing. The worklist
//!    re-scans on a candidate-count timer ([`Worklist::rescan_due`]),
//!    absorbs mid-run arrivals into the remaining queue or explicitly names
//!    them as deferred ([`Worklist::absorb`], [`Worklist::defer`]), and
//!    reports considered / arrived / deferred so the summary adds up to
//!    what was on disk at the end of the run, not the start (#3801). A
//!    defer carries a recorded reason and the summary names the deferred
//!    items with it ([`Worklist::defer`], [`DeferredPatch`]) — a defer
//!    nobody can explain is unrepresented work wearing a different label —
//!    and the freeze line reports the worklist's age, `now` minus the
//!    freeze stamp ([`Worklist::elapsed_since_freeze`]), so a worklist
//!    that stopped being re-read is visible in the log, not only in
//!    hindsight (#3783).
//! 10. **A gate that could not run is not a failed assertion.** A pass
//!     whose setup fails (the build died, the provisioning aborted) has not
//!     evaluated the patch, and must not report it as if the patch were
//!     held or otherwise defective: it records
//!     [`ConversionClass::CouldNotEvaluate`], a distinct terminal state.
//!     "The property is false" and "I could not evaluate it" demand
//!     opposite responses; collapsing them sends everyone to the wrong
//!     place (#3866). The state is terminal — the pass moves on to the next
//!     patch — but it is not a decision *about the patch*, so it is never
//!     memoized: memoizing a non-evaluation would make one run's setup
//!     failure a permanent verdict against the patch, and a gate that did
//!     not run neither satisfies nor refutes an acceptance criterion.
//! 11. **An open PR is work in progress, and the opener owns it**
//!     ([`PrLedger`], [`run_summary`]). "Opened a PR" is not a terminal
//!     state: the pass that opens a PR is responsible for landing it, so
//!     every run summary reports the PRs the pass's family previously
//!     opened that are still open, with age, alongside converted/held/
//!     skipped — a number that only goes up is the alarm. And a
//!     `Mergeable -> Conflicting` transition is surfaced in the pass
//!     that observes it: that is the moment the cheap fix expired, and
//!     decay measured by age alone is invisible until it is terminal
//!     (#3899).
//! 12. **An outcome is derived from verified steps, not from the
//!     commands the pass intended to run** ([`decide_publication`],
//!     [`PublicationLog`]). The four publish steps — commit, push, PR
//!     creation, PR merge — each record their exit status *and their
//!     captured output* ([`VerifiedStep`]); a step that fails before a
//!     PR can exist is a `HELD` that quotes the step's own output, not
//!     a `CONVERTED`; `CONVERTED` is refused without a PR number the
//!     remote confirmed; the converted counter is derived from recorded
//!     outcomes, never incremented beside a claim; and the pass
//!     reconciles its converted claims against what the remote shows
//!     before it finishes ([`reconcile_converted`]) (#3972).
//! 13. **A result is attributable to the logic that produced it**
//!     ([`CONVERSION_LOGIC_VERSION`]). A memo verdict or a publication
//!     outcome is evidence about its inputs only under the version of the
//!     decision logic that made it. When this module's logic changes,
//!     results recorded under the old version stop being decisions and
//!     become hypotheses the new logic must re-verify: every result
//!     carries the version that produced it
//!     ([`PublicationOutcome::Converted::logic_version`]), a verdict
//!     recorded under a superseded version stays queryable but is never a
//!     memo hit ([`ConversionMemo::records`],
//!     [`superseded_verdict_report`]), and the pass re-reads its own
//!     version when it finishes — a mid-run change to the logic is
//!     reported in the run summary ([`logic_change_report`],
//!     [`run_summary`]) instead of silently attributing results from two
//!     logics to one (#4041).
//! 14. **A patch's verdict, not its existence, answers "already
//!     produced"** ([`PatchVerdict`], [`ConversionClass::verdict`]). A
//!     re-dispatch gate that keyed on existence could not tell a held
//!     patch from a converted one: both sit in the queue, and a hold was
//!     therefore indistinguishable from done work — the issue stayed open,
//!     owed its work, and was acted on by nothing (#3674). A converted or
//!     resolved patch is done; a held patch is not; a candidate and a
//!     [`ConversionClass::CouldNotEvaluate`] state have no verdict at
//!     all, because the pass has not decided.
//! 15. **Held patches are retired to outside the patch glob, with the
//!     reason recorded** ([`plan_retirement`], [`HoldReason`],
//!     [`retirement_root`]) — and only where a re-run would plausibly
//!     differ, so the issue becomes eligible for re-dispatch again and a
//!     repeat hold is visible instead of silent. Retirement invalidates
//!     the memoized hold ([`ConversionMemo::invalidate`]) so the next
//!     pass re-walks the patch against the rules as they stand now
//!     instead of re-holding it from the record for the same since-fixed
//!     reason (#3674).
//! 16. **The queue reports its inert fraction** ([`queue_health`]):
//!     "120 queued, 43 held" is the line that makes the trap visible, and
//!     it is one loop to compute (#3674).
//! 17. **The pass reconciles its phase counts before it exits**
//!     ([`reconcile_phase_counts`]). Phase 1 is the cheap triage that
//!     decides the terminal classes without compute and hands the
//!     remainder to phase 2 as the candidates that need it; phase 2 is
//!     the expensive gate that applies, compile-tests, commits, and
//!     pushes each candidate. The one number the phases exchange is the
//!     candidate count, and phase 2's only obligation is to process all
//!     of them. A pass that processes fewer than it was handed and still
//!     exits `0` has *dropped, not decided* the candidates it never
//!     reached, and because the pass that dropped them also owns the
//!     summary that would have named them, the shortfall is silent. The
//!     gate asserts the two counts reconcile and, when they do not,
//!     returns an error the caller turns into a non-zero exit, so the
//!     pass fails loudly instead of printing a summary that adds up to a
//!     lie (#3742).

use std::collections::BTreeMap;

/// The identity of the conversion-pass decision logic, for the version in
/// force in this build (#4041).
///
/// A result of the pass — a memo verdict, a publication outcome, a run
/// summary — is evidence about its inputs only under the version of the
/// logic that computed it. When the logic changes, this constant must be
/// bumped in the same change, and every result the old version produced
/// remains attributable to it: stored verdicts keep the version they were
/// recorded under ([`ConversionMemo::records`]) and the pass reports a
/// mid-run change of logic instead of passing it off as one version's
/// work ([`logic_change_report`], #3866 pattern).
/// Bumped to 2 in #3674: the re-dispatch verdict ([`PatchVerdict`],
/// [`ConversionClass::verdict`]) and held-patch retirement
/// ([`plan_retirement`]) are new decision logic in this module, so
/// results recorded under version 1 are hypotheses the version-2 logic
/// re-verifies, not decisions.
/// Bumped to 3 in #3715: agent-reported hold reasons
/// ([`HoldReason::AgentReportedUnformatted`],
/// [`HoldReason::AgentReportedUnbuilt`], triaged by the `status_triage`
/// module) join the retirement decision — an agent-reported unformatted
/// hold is retired on re-dispatch (the unformatted state is a property
/// of the submission a fresh agent fixes), an agent-reported unbuilt hold
/// is not (a re-run reproduces it). Results recorded under version 2 are
/// hypotheses the version-3 logic re-verifies, not decisions.
/// Bumped to 4 in #3783: the ordering comparator gained the agent-status
/// dimension (recorded `PASS` first, no report neutral, recorded failures
/// last) and every item records its full ordering key
/// ([`Patch::ordering_key`]). Ordering decides which work a pass spends
/// its compute on; results recorded under version 3 are hypotheses the
/// version-4 logic re-verifies, not decisions.
/// Bumped to 5 in #4279: the stale-base hold reason
/// ([`HoldReason::StaleBase`], [`classify_build_failure`]) joins the
/// retirement decision — a stale-base hold is retired on re-dispatch
/// (the drift resolves when the patch is rebased onto the new tip), so
/// results recorded under version 4 are hypotheses the version-5 logic
/// re-verifies, not decisions.
pub const CONVERSION_LOGIC_VERSION: u32 = 5;

/// A conversion outcome class, ordered cheapest first.
///
/// The four decided classes cost seconds (no compile): the pass already
/// knows the answer. [`ConversionClass::CouldNotEvaluate`] costs nothing
/// but is a state of the setup, not of the patch. [`Candidate`] costs a
/// build.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ConversionClass {
    /// The patch already maps to an open pull request.
    ExistingPr,
    /// The issue the patch answers is closed.
    ClosedIssue,
    /// A prior pass already held this patch for a recorded reason.
    MemoizedHold,
    /// The patch produces no net change against the base.
    NoNetChange,
    /// The pass could not evaluate this patch: its setup (the build, the
    /// checkout provisioning) failed before any property of the patch was
    /// asserted. A terminal state of the pass, but not a decision about
    /// the patch — never memoized, never a memo hit, and re-evaluated on
    /// every pass until the setup succeeds (#3866).
    CouldNotEvaluate,
    /// A genuine candidate: apply, compile-test, commit, push.
    Candidate,
}

impl ConversionClass {
    /// Every class, cheapest first.
    pub const ALL: [Self; 6] = [
        Self::ExistingPr,
        Self::ClosedIssue,
        Self::MemoizedHold,
        Self::NoNetChange,
        Self::CouldNotEvaluate,
        Self::Candidate,
    ];

    /// Whether this class is a terminal state of the pass: the pass has
    /// reached its final state for the patch this run and the worker
    /// moves on without a successful build. Everything except
    /// [`Candidate`] is terminal — including [`CouldNotEvaluate`], which
    /// is final for the run but not a decision about the patch (see
    /// [`Self::is_memoizable`]).
    pub fn is_terminal(self) -> bool {
        !matches!(self, Self::Candidate)
    }

    /// Whether this class is a decision *about the patch* and therefore
    /// safe to memoize. The four decided classes are memoizable;
    /// [`Candidate`] is work still to do, and [`CouldNotEvaluate`] is a
    /// state of the setup, not of the patch — memoizing either would skip
    /// the evaluation that still has to happen (#3866).
    pub fn is_memoizable(self) -> bool {
        matches!(
            self,
            Self::ExistingPr | Self::ClosedIssue | Self::MemoizedHold | Self::NoNetChange
        )
    }

    /// The verdict this class carries for the re-dispatch gate (#3674).
    /// A landed or resolved terminal class is done; a memoized hold is
    /// not, because the work is produced but unlanded and the issue is
    /// still open. [`Candidate`] and [`CouldNotEvaluate`] have no verdict
    /// at all: the pass has not decided (work still to do, or a state of
    /// the setup, not of the patch), and "not yet decided" must not be
    /// reported as "done".
    pub fn verdict(self) -> Option<PatchVerdict> {
        match self {
            Self::ExistingPr => Some(PatchVerdict::Converted),
            Self::ClosedIssue | Self::NoNetChange => Some(PatchVerdict::Resolved),
            Self::MemoizedHold => Some(PatchVerdict::Held),
            Self::CouldNotEvaluate | Self::Candidate => None,
        }
    }

    /// The class's stable display name in log lines and ordering keys
    /// (#3783). [`CouldNotEvaluate`] keeps its all-caps form from #3866,
    /// distinct from the PascalCase decided classes.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ExistingPr => "ExistingPr",
            Self::ClosedIssue => "ClosedIssue",
            Self::MemoizedHold => "MemoizedHold",
            Self::NoNetChange => "NoNetChange",
            Self::CouldNotEvaluate => "COULD-NOT-EVALUATE",
            Self::Candidate => "Candidate",
        }
    }
}

/// The verdict of a terminal patch, as a re-dispatch gate must see it
/// (#3674).
///
/// Existence is not the test. A held patch exists — produced, paid for,
/// sitting in the queue — and it is not done: its issue is still open
/// and still owed the work. A gate that answers "already produced" from
/// existence makes such issues inert forever, because nothing ever
/// retires a hold that looks exactly like a completed conversion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchVerdict {
    /// The work has landed: the patch converted to a pull request.
    Converted,
    /// The work resolved to no conversion: the issue is closed, or the
    /// patch is a no-op. Terminal like converted — the issue must not be
    /// re-dispatched.
    Resolved,
    /// The pass held the patch for a recorded reason: the work is
    /// produced but unlanded, and the issue is still open.
    Held,
}

impl PatchVerdict {
    /// The "already produced" answer for the re-dispatch gate (#3674):
    /// a converted or resolved patch is done; a held patch is not.
    pub fn is_done(self) -> bool {
        !matches!(self, Self::Held)
    }
}

/// One finished agent patch waiting in the conversion queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Patch {
    /// Stable identity of the finished work (patch file name or content
    /// hash). Must be non-empty.
    pub identity: String,
    /// Trunk revision the patch was produced against. Must be non-empty; it
    /// is part of the memo key because a rebase changes the outcome.
    pub base_sha: String,
    /// The class a prior walk of this patch landed on (or the default
    /// [`ConversionClass::Candidate`] for a never-converted patch).
    pub class: ConversionClass,
    /// Monotonic production stamp (e.g. file mtime seconds); higher is
    /// newer. Drives newest-first ordering and mid-pass absorption.
    pub produced_at: u64,
    /// Number of downstream issues this patch unblocks. The frontier loop
    /// publishes this value; missing entries default to 0, which degrades
    /// ordering to recency-only (today's behaviour).
    pub unblocks: usize,
    /// The agent's own verdict on this patch, parsed from its status
    /// report (the `status_triage` module): `PASS` when the agent's build
    /// and tests were green, a failure token (`TIMEOUT`, `BUILD-FAILED`,
    /// …) when they were not, `None` when no report exists. Drives the
    /// expected-value dimension of the ordering key
    /// ([`Patch::ordering_key`]) (#3783).
    pub agent_status: Option<String>,
}

impl Patch {
    /// Construct a patch, rejecting empty identity or base sha: an empty key
    /// would collapse unrelated patches into one memo entry.
    pub fn new(
        identity: impl Into<String>,
        base_sha: impl Into<String>,
        class: ConversionClass,
        produced_at: u64,
        unblocks: usize,
    ) -> Result<Self, String> {
        let identity = identity.into();
        let base_sha = base_sha.into();
        if identity.trim().is_empty() {
            return Err("patch identity must not be empty".to_string());
        }
        if base_sha.trim().is_empty() {
            return Err("patch base sha must not be empty".to_string());
        }
        Ok(Self {
            identity,
            base_sha,
            class,
            produced_at,
            unblocks,
            agent_status: None,
        })
    }

    /// True when the patch's base is no longer the trunk tip: every decision
    /// computed against `base_sha` has decayed, so the patch must be
    /// re-tested against the current tip before acting on it.
    pub fn is_stale(&self, current_tip: &str) -> bool {
        self.base_sha != current_tip
    }

    /// The re-baseline gate's failure report: names both operands of the
    /// equality the gate asserts — this patch's base sha and the trunk
    /// tip. `None` when the gate passed. A gate that cannot print the
    /// values it compared did not perform the comparison, and must not
    /// claim the property failed (#3866).
    pub fn stale_report(&self, current_tip: &str) -> Option<String> {
        if self.base_sha == current_tip {
            return None;
        }
        Some(format!(
            "stale base: {} built against {}, trunk tip is {}",
            self.identity, self.base_sha, current_tip
        ))
    }

    /// The "already produced" check for this patch (#3674): it references
    /// the verdict, not the existence. A converted or resolved patch is
    /// done; a held patch is not, because its issue is still open; a
    /// candidate and a [`ConversionClass::CouldNotEvaluate`] state have
    /// no verdict, and the pass has not decided.
    pub fn is_done(&self) -> bool {
        self.class.verdict().is_some_and(PatchVerdict::is_done)
    }

    /// Set the agent's own verdict on this patch, when one exists. Blank
    /// input is treated as no report: an empty status line is not
    /// evidence either way.
    pub fn with_agent_status(mut self, status: impl Into<String>) -> Self {
        let status = status.into();
        self.agent_status = if status.trim().is_empty() {
            None
        } else {
            Some(status.trim().to_string())
        };
        self
    }

    /// The patch's full ordering key, recorded for post-hoc inspection
    /// (#3783): every dimension [`order_by_cost`] sorts by, in the order
    /// it sorts them. Two items whose recorded keys differ are ordered by
    /// the first dimension on which they differ, so the order of any pair
    /// is reproducible from the log lines alone.
    pub fn ordering_key(&self) -> String {
        format!(
            "class={},status={},unblocks={},produced_at={}",
            self.class.as_str(),
            self.agent_status.as_deref().unwrap_or("none"),
            self.unblocks,
            self.produced_at
        )
    }
}

/// Order a pass by expected value: cost, then verified agent status, then
/// downstream impact, then recency. Every terminal (cheap) patch goes
/// before every candidate (expensive) patch; within each cost class a
/// patch with a recorded agent `PASS` goes first (its verification cost
/// is already paid), then patches with no report, then patches with a
/// recorded failure (evidence the patch carries a defect the pass has not
/// yet fixed); within each status group the highest-impact patch goes
/// first (most downstream issues unblocked), with recency as tiebreak. A
/// patch that unblocks many issues is the highest-leverage work:
/// converting it promptly unblocks the most downstream dependents
/// (#3799). Patches tied on impact fall back to newest-first, which keeps
/// the previous behaviour when no impact data is present (all unblocks =
/// 0). The sort is by expected value, never lexicographic or filesystem
/// order (#3783). The sort is stable, so patches tied on all keys keep
/// their queue order and two passes over the same queue agree.
pub fn order_by_cost(patches: &[Patch]) -> Vec<&Patch> {
    let mut ordered: Vec<&Patch> = patches.iter().collect();
    ordered.sort_by(|a, b| cost_then_impact(a, b));
    ordered
}

/// The pass's ordering key, as a comparator.
///
/// Named rather than inlined because the worklist re-sorts the *remaining*
/// queue after absorbing mid-run arrivals (#3801), and it must use the same
/// key the initial ordering used. Two orderings of one queue that disagree
/// would make an arrival's position depend on when it showed up rather than
/// on what it is.
fn cost_then_impact(a: &Patch, b: &Patch) -> std::cmp::Ordering {
    a.class
        .cmp(&b.class)
        .then(
            agent_status_rank(a.agent_status.as_deref())
                .cmp(&agent_status_rank(b.agent_status.as_deref())),
        )
        .then(b.unblocks.cmp(&a.unblocks))
        .then(b.produced_at.cmp(&a.produced_at))
}

/// The agent-status dimension of the ordering key (#3783), ascending =
/// higher expected value. A recorded `PASS` means the verification cost
/// is already paid — first. No report is neutral. A recorded failure
/// (timeout, build failure, …) is evidence the patch carries a defect
/// the pass has not yet fixed — last.
fn agent_status_rank(status: Option<&str>) -> u8 {
    match status {
        Some("PASS") => 0,
        None => 1,
        Some(_) => 2,
    }
}

/// Produce a one-line ordering summary for the top candidates in the pass.
///
/// Shows the leading `top_n` patches with their full ordering key
/// ([`Patch::ordering_key`]) so the order of any pair is inspectable from
/// the log alone (#3783). Also reports the number of distinct unblocks
/// values across the population the key was measured on — the population
/// is stated with the number (the baseline), not assumed from an absolute
/// bar — mirroring the "recency key has N distinct values" diagnostic
/// that caught the degenerate-mtime bug (#3791). A single distinct value
/// is degenerate at any baseline: the key separates no pair.
pub fn order_summary(ordered: &[&Patch], top_n: usize) -> String {
    if ordered.is_empty() {
        return "phase 1 ordered: 0 candidates".to_string();
    }
    let distinct_unblocks = ordered
        .iter()
        .map(|p| p.unblocks)
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    let top: Vec<String> = ordered
        .iter()
        .take(top_n)
        .map(|p| format!("{}({})", p.identity, p.ordering_key()))
        .collect();
    let mut summary = format!(
        "phase 1 ordered: {} first, then status, then impact desc, then newest first — {} leads {} candidates (impact key has {} distinct values, baseline: {} items)",
        ordered[0].class.as_str(),
        top.join(", "),
        ordered.len(),
        distinct_unblocks,
        ordered.len(),
    );
    if distinct_unblocks < 2 {
        summary.push_str(" — degenerate: the key separates no pair");
    }
    summary
}

/// Read an `issue<TAB>unblocks` file produced by the frontier loop.
///
/// Each non-blank, non-comment line must be `issue_number<TAB>count`.
/// Returns a map from issue identity string to unblock count. Missing
/// entries (issues not in the file) default to 0 at the caller, which
/// degrades ordering to recency-only.
pub fn read_unblock_counts(path: &str) -> std::io::Result<BTreeMap<String, usize>> {
    let content = std::fs::read_to_string(path)?;
    Ok(parse_unblock_counts(&content))
}

/// Parse the content of an `issue<TAB>unblocks` file into a map.
///
/// Exposed for testing without filesystem I/O.
pub fn parse_unblock_counts(content: &str) -> BTreeMap<String, usize> {
    let mut map = BTreeMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.splitn(2, '\t');
        let key = parts.next().unwrap_or("").to_string();
        let value = parts
            .next()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .unwrap_or(0);
        if !key.is_empty() {
            map.insert(key, value);
        }
    }
    map
}

/// Merge patches observed by a mid-pass re-scan into the running queue
/// (#3698, #3801). A converter that runs for hours must see work produced
/// during those hours; the caller re-plans the pass after absorbing.
///
/// Identity is the dedup key. When an identity already exists, the entry
/// with the higher `produced_at` wins; on a tie the freshly observed entry
/// wins, because a re-scan is the newer observation of the same patch.
///
/// The queue is re-sorted after the merge (stable, cost-then-recency):
/// ordering is only meaningful over the current population, and a list
/// ordered once at startup degrades into arrival order for everything that
/// comes later (#3801). Returns the number of newly arrived identities — a
/// re-observation of a known patch is not an arrival.
pub fn absorb_new(queue: &mut Vec<Patch>, fresh: impl IntoIterator<Item = Patch>) -> usize {
    let mut arrived = 0;
    for observed in fresh {
        match queue.iter_mut().find(|p| p.identity == observed.identity) {
            Some(existing) if existing.produced_at > observed.produced_at => {}
            Some(existing) => *existing = observed,
            None => {
                queue.push(observed);
                arrived += 1;
            }
        }
    }
    let mut ordered = std::mem::take(queue);
    ordered.sort_by(|a, b| cost_then_impact(a, b));
    *queue = ordered;
    arrived
}

/// The run summary of a [`Worklist`]: the worklist reported as a moving
/// quantity (#3801).
///
/// "141 candidates" stated once and never revised is the defect this
/// replaces: an arrival was not merely depriorised, it was unrepresented, and
/// no log line was wrong. The counts add up to what was on disk at the
/// **end** of the run, not the start:
///
/// ```text
/// on_disk    = initial + arrived + deferred
/// considered + remaining + deferred = on_disk
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorklistSummary {
    /// The stamp at which the worklist was frozen (enumerated).
    pub frozen_at: u64,
    /// Patches on disk at the freeze: the startup enumeration.
    pub initial: usize,
    /// New identities absorbed into the running queue mid-run.
    pub arrived: usize,
    /// Patches the run has converted, or decided cheaply.
    pub considered: usize,
    /// Patches still in the remaining queue.
    pub remaining: usize,
    /// Patches explicitly named as deferred to the next run.
    pub deferred: usize,
    /// Patches on disk at the end of the run: `initial + arrived + deferred`.
    pub on_disk: usize,
}

/// A patch named as deferred to the next run, with its recorded reason
/// (#3783). A defer with no reason is unrepresented work wearing a
/// different label; the run summary names deferred items with their
/// reason, so the defer is inspectable in the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeferredPatch {
    /// The deferred patch.
    pub patch: Patch,
    /// Why the run deferred it, recorded at the defer.
    pub reason: String,
}

/// A live view of the conversion pass's worklist: the queue as a moving
/// quantity, not a startup snapshot (#3801).
///
/// A pass that enumerates its inputs once and then works through them for
/// hours presents a snapshot as a queue. Two consequences: work arriving
/// mid-run is invisible until the next run (and it is precisely the newest
/// work, which is the work most likely to matter), and every ordering
/// decision was made against a population that no longer exists. This type
/// owns the bookkeeping a live worklist needs; the caller performs the I/O:
///
/// - re-scan on a candidate-count timer, not wall-clock time
///   ([`Worklist::rescan_due`]): lengthening the per-candidate work (e.g. a
///   mandatory validate) cannot silently enlarge the set of work a run
///   cannot see, because arrivals are counted either way;
/// - absorb mid-run arrivals into the remaining queue
///   ([`Worklist::absorb`]), which re-sorts it, or name them as explicitly
///   deferred to the next run ([`Worklist::defer`]) — never unrepresented;
/// - report the freeze window and the moving counts
///   ([`Worklist::summary`], [`Worklist::summary_line`]) so the summary
///   adds up to what was on disk at the end of the run, not the start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worklist {
    remaining: Vec<Patch>,
    /// Patches explicitly named as deferred to the next run, each with
    /// its recorded reason (#3783).
    deferred: Vec<DeferredPatch>,
    frozen_at: u64,
    initial: usize,
    considered: usize,
    arrived: usize,
    considered_since_rescan: usize,
}

impl Worklist {
    /// Freeze the worklist: adopt the enumeration taken at `frozen_at` (the
    /// same monotonic stamp space as [`Patch::produced_at`]) and start it in
    /// execution order.
    pub fn new(patches: impl IntoIterator<Item = Patch>, frozen_at: u64) -> Self {
        let mut remaining: Vec<Patch> = patches.into_iter().collect();
        remaining.sort_by(|a, b| cost_then_impact(a, b));
        let initial = remaining.len();
        Self {
            remaining,
            deferred: Vec::new(),
            frozen_at,
            initial,
            considered: 0,
            arrived: 0,
            considered_since_rescan: 0,
        }
    }

    /// The stamp at which this worklist was frozen (its startup
    /// enumeration).
    pub fn frozen_at(&self) -> u64 {
        self.frozen_at
    }

    /// The remaining queue in execution order: next to convert first.
    pub fn remaining(&self) -> &[Patch] {
        &self.remaining
    }

    /// The patches explicitly named as deferred to the next run, each
    /// with its recorded reason (#3783).
    pub fn deferred(&self) -> &[DeferredPatch] {
        &self.deferred
    }

    /// Take the next patch in execution order for conversion and count it
    /// as considered.
    pub fn take_next(&mut self) -> Option<Patch> {
        if self.remaining.is_empty() {
            return None;
        }
        let next = self.remaining.remove(0);
        self.considered += 1;
        self.considered_since_rescan += 1;
        Some(next)
    }

    /// Whether a re-scan of shared storage is due: `every_n` candidates have
    /// been considered since the last re-scan. `every_n == 1` re-scans
    /// before each candidate; `every_n == 0` means always due.
    ///
    /// The timer counts candidates, not wall-clock time. Lengthening the
    /// per-candidate cost (e.g. a mandatory validate) lengthens the window
    /// in seconds, but no longer silently: whatever the window misses is
    /// still named in [`Worklist::summary_line`], and the summary adds up
    /// to what was on disk at the end of the run (#3801).
    pub fn rescan_due(&self, every_n: usize) -> bool {
        if every_n == 0 {
            return true;
        }
        self.considered_since_rescan >= every_n
    }

    /// Absorb patches observed by a mid-run re-scan: merge into the
    /// remaining queue (identity-deduped, fresher observation wins),
    /// re-sort the queue, count new arrivals, and reset the re-scan timer.
    /// Returns the number of newly arrived identities.
    pub fn absorb(&mut self, fresh: impl IntoIterator<Item = Patch>) -> usize {
        let arrived = absorb_new(&mut self.remaining, fresh);
        self.arrived += arrived;
        self.considered_since_rescan = 0;
        arrived
    }

    /// Name a patch as explicitly deferred to the next run instead of
    /// converting it in this one, with the reason recorded at the defer
    /// (#3783). A patch that is still in the remaining queue is going to
    /// be converted in this run, the same patch may only be deferred
    /// once, and a defer without a reason is unrepresented work wearing a
    /// different label: all three are bookkeeping errors.
    pub fn defer(&mut self, patch: Patch, reason: &str) -> Result<(), String> {
        if self.remaining.iter().any(|p| p.identity == patch.identity) {
            return Err(format!(
                "refusing to defer {}: already in the remaining worklist",
                patch.identity
            ));
        }
        if reason.trim().is_empty() {
            return Err(format!(
                "refusing to defer {} without a reason: a defer nobody can \
                 explain is unrepresented work wearing a different label \
                 (#3783)",
                patch.identity
            ));
        }
        if self
            .deferred
            .iter()
            .any(|p| p.patch.identity == patch.identity)
        {
            return Err(format!(
                "refusing to defer {}: already named as deferred",
                patch.identity
            ));
        }
        self.deferred.push(DeferredPatch {
            patch,
            reason: reason.trim().to_string(),
        });
        Ok(())
    }

    /// The run summary: the counts add up to what was on disk at the end of
    /// the run, not the start (see [`WorklistSummary`]).
    pub fn summary(&self) -> WorklistSummary {
        let on_disk = self.initial + self.arrived + self.deferred.len();
        WorklistSummary {
            frozen_at: self.frozen_at,
            initial: self.initial,
            arrived: self.arrived,
            considered: self.considered,
            remaining: self.remaining.len(),
            deferred: self.deferred.len(),
            on_disk,
        }
    }

    /// The worklist's age: `now` minus the freeze stamp, in the same
    /// monotonic-stamp space as [`Patch::produced_at`]. Reported so a
    /// worklist that stopped being re-read is visible in the log, not
    /// only in hindsight (#3783).
    pub fn elapsed_since_freeze(&self, now: u64) -> u64 {
        now.saturating_sub(self.frozen_at)
    }

    /// The freeze window (with the elapsed time at `now`) and the moving
    /// counts as one log line, e.g. `worklist frozen at 1022 (elapsed
    /// 1440); 2 patches arrived since freeze; 84 considered, 51 remaining,
    /// 2 deferred to the next run [iw-49: base stale beyond the run's
    /// re-baseline window, iw-50: held by the agent's report]`.
    pub fn summary_line(&self, now: u64) -> String {
        let s = self.summary();
        let mut line = format!(
            "worklist frozen at {} (elapsed {}); {} patches arrived since freeze; {} considered, {} remaining, {} deferred to the next run",
            s.frozen_at,
            now.saturating_sub(s.frozen_at),
            s.arrived,
            s.considered,
            s.remaining,
            s.deferred
        );
        if !self.deferred.is_empty() {
            let names: Vec<String> = self
                .deferred
                .iter()
                .map(|d| format!("{}: {}", d.patch.identity, d.reason))
                .collect();
            line.push_str(&format!(" [{}]", names.join(", ")));
        }
        line
    }
}

/// Memoized conversion decisions.
///
/// Keyed by (patch identity, base sha, logic version): patch identity
/// plus base sha already determines the outcome, so a re-walk of a
/// memoized patch is a map lookup, not a re-walk of the whole set. Only
/// decisions *about the patch* are stored:
/// [`ConversionClass::CouldNotEvaluate`] is a state of the setup and is
/// refused by [`ConversionMemo::record`], so the pass re-evaluates it on
/// every run until the setup succeeds (#3866).
///
/// The key carries the logic version that produced the decision
/// ([`CONVERSION_LOGIC_VERSION`]): a decision is evidence about its
/// inputs only under the version of the logic that made it, so when the
/// logic changes the old version's verdicts stay stored and queryable
/// ([`ConversionMemo::records`]) but are no longer honored by
/// [`ConversionMemo::lookup`] — the new logic must re-verify them
/// (#4041).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConversionMemo {
    entries: BTreeMap<(String, String, u32), ConversionClass>,
}

impl ConversionMemo {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// The recorded terminal decision for this exact (identity, base sha)
    /// pair under the given logic version, if any. A different base sha
    /// is a different patch as far as the memo is concerned; a decision
    /// recorded under a different logic version is not honored here — it
    /// is queryable via [`Self::records`] but the pass must re-derive the
    /// decision itself (#4041).
    pub fn lookup(
        &self,
        identity: &str,
        base_sha: &str,
        logic_version: u32,
    ) -> Option<ConversionClass> {
        self.entries
            .get(&(identity.to_string(), base_sha.to_string(), logic_version))
            .copied()
    }

    /// Record a decision about the patch. Non-memoizable classes are
    /// rejected: a [`ConversionClass::Candidate`] outcome is work still to
    /// do, and a [`ConversionClass::CouldNotEvaluate`] outcome is a state
    /// of the setup, not evidence about the patch — memoizing a
    /// non-evaluation would make one run's setup failure a permanent
    /// verdict against the patch (#3866).
    pub fn record(
        &mut self,
        identity: &str,
        base_sha: &str,
        class: ConversionClass,
        logic_version: u32,
    ) -> Result<(), String> {
        if !class.is_memoizable() {
            return Err(match class {
                ConversionClass::CouldNotEvaluate => format!(
                    "refusing to memoize CouldNotEvaluate for {identity}@{base_sha}: a gate that could not evaluate the patch is not evidence about it; the pass must re-evaluate, not remember"
                ),
                other => format!(
                    "refusing to memoize non-terminal class {other:?}: work still to do, not a decision worth memoizing"
                ),
            });
        }
        if identity.trim().is_empty() || base_sha.trim().is_empty() {
            return Err("memo keys must have a non-empty identity and base sha".to_string());
        }
        self.entries.insert(
            (identity.to_string(), base_sha.to_string(), logic_version),
            class,
        );
        Ok(())
    }

    /// Every recorded decision for this (identity, base sha) pair, across
    /// all logic versions, highest version first (#4041). This is the
    /// superseded-version query path: what an old version of the logic
    /// decided stays visible to the operator, while [`Self::lookup`]
    /// honors only the version the pass is running under.
    pub fn records(&self, identity: &str, base_sha: &str) -> Vec<MemoRecord> {
        self.entries
            .iter()
            .filter(|((id, base, _), _)| id == identity && base == base_sha)
            .map(|((_, _, version), class)| MemoRecord {
                class: *class,
                logic_version: *version,
            })
            .rev()
            .collect()
    }

    /// Invalidate the recorded decision for this exact (identity, base
    /// sha) pair under the given logic version (#3674). Retiring a held
    /// patch removes its entry so the next pass re-walks the patch
    /// against the rules as they stand now, instead of re-holding it
    /// from the record for the same since-fixed reason. Only the entry
    /// under the named version is removed: verdicts a superseded version
    /// recorded stay queryable via [`Self::records`] — the audit trail
    /// outlives the decision. Returns whether an entry was removed.
    pub fn invalidate(&mut self, identity: &str, base_sha: &str, logic_version: u32) -> bool {
        self.entries
            .remove(&(identity.to_string(), base_sha.to_string(), logic_version))
            .is_some()
    }
}

/// A memo record with the identity of the logic that produced it (#4041).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoRecord {
    /// The decision the recorded logic landed on.
    pub class: ConversionClass,
    /// The logic version that produced the decision
    /// ([`CONVERSION_LOGIC_VERSION`]).
    pub logic_version: u32,
}

/// The build gate a conversion worker must pass before a patch may be
/// reported green (#3702).
///
/// `cargo build` compiles library and binary targets only and skips test
/// targets. A patch that adds a test file referencing a stale API passes a
/// plain build gate without the file the agent just wrote ever compiling,
/// so the runner's `build_rc=0` answered a narrower question than the one
/// it appeared to answer. The gate therefore builds `--all-targets` —
/// tests, benches and examples as well as lib and bin — and carries the
/// exact command line, because a green result must record which gate
/// produced it: the difference between `cargo build --workspace` and
/// `cargo build --workspace --all-targets` decides whether the exit code
/// means anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildGate {
    /// Exact command line a worker runs; recorded verbatim with any
    /// outcome it produces.
    pub command: String,
}

impl BuildGate {
    /// The all-targets gate: compiles every target the workspace owns, so
    /// a gate that skips the patch's own tests is not checking the patch.
    pub fn all_targets() -> Self {
        Self {
            command: "cargo build --workspace --all-targets".to_string(),
        }
    }
}

impl Default for BuildGate {
    fn default() -> Self {
        Self::all_targets()
    }
}

/// What the build gate plus the test run decided about one patch.
///
/// [`GateVerdict::CompileFailure`] is a hard failure distinct from
/// [`GateVerdict::TestsFailed`]: `cargo test` exits 101 in both cases —
/// when a test target fails to compile (nothing ran) and when tests
/// compile but some fail. Because the all-targets build gate has already
/// compiled the test targets, only it can separate the two.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateVerdict {
    /// The gate command exited 0: every target the patch touched compiles.
    Green,
    /// A target the patch added or modified does not compile: a hard
    /// failure, no test ran.
    CompileFailure,
    /// The test targets compiled but at least one test failed.
    TestsFailed,
}

impl GateVerdict {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Green => "green",
            Self::CompileFailure => "compile_failure",
            Self::TestsFailed => "tests_failed",
        }
    }

    /// Anything but green blocks the commit-and-push step.
    pub fn is_failure(&self) -> bool {
        !matches!(self, Self::Green)
    }
}

/// Classify a worker's gate results into a verdict.
///
/// `build_rc` is the exit code of the all-targets build gate
/// ([`BuildGate::all_targets`]) and `test_rc` the exit code of
/// `cargo test`. A failing build rc means a target the patch touched —
/// including its own tests — did not compile, and it wins over `test_rc`
/// unconditionally: with the gate skipped, `cargo test` would fail for the
/// same reason and look identical to tests running and failing.
pub fn classify_gate(build_rc: i32, test_rc: i32) -> GateVerdict {
    if build_rc != 0 {
        GateVerdict::CompileFailure
    } else if test_rc != 0 {
        GateVerdict::TestsFailed
    } else {
        GateVerdict::Green
    }
}

/// A worker in the conversion pool: one private checkout, one shared
/// compile cache, one explicit build gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerPlan {
    /// Position in the pool (also the round-robin assignment target).
    pub index: usize,
    /// Private checkout for this worker. The worktree lock from #3608
    /// protects this path, not the operation: N checkouts admit N
    /// concurrent converters.
    pub worktree: String,
    /// `CARGO_TARGET_DIR` shared by every patch this worker processes (not
    /// per patch), making most runs incremental. Distinct per worker
    /// because cargo takes an exclusive lock on the target dir.
    pub target_dir: String,
    /// The exact build gate this worker's results must name, so a
    /// `build_rc=0` records the command that produced it (#3702).
    pub build_gate: BuildGate,
}

/// Plan `count` workers under `root`: worker `i` gets checkout
/// `{root}/worker-{i}` and compile cache `{root}/target/worker-{i}`.
/// Every path is unique, which is what makes N-way concurrency safe under
/// the per-checkout lock. Zero workers is a configuration error, not an
/// empty pass: it would silently re-serialise the pipeline.
pub fn plan_workers(count: usize, root: &str) -> Result<Vec<WorkerPlan>, String> {
    if count == 0 {
        return Err("conversion pool needs at least one worker".to_string());
    }
    if root.trim().is_empty() {
        return Err("worker pool root must not be empty".to_string());
    }
    Ok((0..count)
        .map(|index| WorkerPlan {
            index,
            worktree: format!("{root}/worker-{index}"),
            target_dir: format!("{root}/target/worker-{index}"),
            build_gate: BuildGate::default(),
        })
        .collect())
}

/// One patch placed on one worker, in execution order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScheduledPatch<'a> {
    /// The patch, in cost order (terminal classes first).
    pub patch: &'a Patch,
    /// Worker index that converts this patch (round-robin over the cost
    /// order).
    pub worker: usize,
    /// True when [`ConversionMemo`] already carries the matching terminal
    /// decision for this (identity, base sha) pair **and the base is still
    /// the trunk tip**; the worker applies the recorded decision without a
    /// compile. A verdict computed against an old base is a hypothesis, not
    /// a decision, and is never reported as a hit. A verdict recorded
    /// under a superseded logic version is the same kind of stale
    /// evidence — flagged by [`Self::superseded_verdict`] instead
    /// (#4041).
    pub memo_hit: bool,
    /// True when the patch's base is no longer the trunk tip: the worker
    /// must fetch `origin/main` and re-test before acting, because every
    /// decision computed against the old base has decayed (#3698).
    pub stale_base: bool,
    /// True when the memo holds a decision for this (identity, base sha)
    /// pair that a **superseded logic version** recorded: the old verdict
    /// is queryable ([`ConversionMemo::records`]) but is a hypothesis the
    /// current logic must re-verify, not a decision it may act on
    /// (#4041). The gate's report naming both versions is
    /// [`superseded_verdict_report`]; a decision recorded under the
    /// running version is never flagged.
    pub superseded_verdict: bool,
}

/// The full plan for one pass over the queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversionSchedule<'a> {
    /// The worker pool (empty exactly when there is no work).
    pub workers: Vec<WorkerPlan>,
    /// Patches in execution order with their worker assignment.
    pub assignments: Vec<ScheduledPatch<'a>>,
}

/// Plan one pass over the queue against the **current** trunk tip: order
/// the queue by cost, then impact, then recency; size the pool; assign
/// round-robin; and flag memo hits and stale bases. An empty queue plans
/// an empty pool — there is nothing to convert, and zero workers is the
/// only sane pool for zero work.
///
/// `current_tip` is the tip of `origin/main` fetched immediately before this
/// plan: the pass re-reads the world before each candidate, so a snapshot
/// taken at startup must never reach the workers. A memo entry is honored
/// only when its base is still the tip — a verdict about an old base is a
/// hypothesis, not a decision (#3698). A
/// [`ConversionClass::CouldNotEvaluate`] patch can never be a memo hit: the
/// memo cannot hold the state, so the pass re-evaluates the patch every run
/// until the setup succeeds (#3866). The same rule applies to the logic
/// version the decision was recorded under (`logic_version`,
/// [`CONVERSION_LOGIC_VERSION`]): a verdict a superseded version of the
/// logic recorded is never a hit, and is flagged
/// [`ScheduledPatch::superseded_verdict`] so the worker re-verifies it
/// (#4041).
pub fn plan_pass<'a>(
    patches: &'a [Patch],
    workers: usize,
    root: &str,
    memo: &ConversionMemo,
    current_tip: &str,
    logic_version: u32,
) -> Result<ConversionSchedule<'a>, String> {
    if current_tip.trim().is_empty() {
        return Err(
            "current tip must not be empty: the pass must re-baseline against origin/main"
                .to_string(),
        );
    }
    if patches.is_empty() {
        return Ok(ConversionSchedule {
            workers: Vec::new(),
            assignments: Vec::new(),
        });
    }
    let pool = plan_workers(workers, root)?;
    let assignments = order_by_cost(patches)
        .into_iter()
        .enumerate()
        .map(|(position, patch)| ScheduledPatch {
            patch,
            worker: position % pool.len(),
            memo_hit: patch.base_sha == current_tip
                && memo
                    .lookup(&patch.identity, &patch.base_sha, logic_version)
                    .is_some_and(|class| class == patch.class),
            stale_base: patch.base_sha != current_tip,
            superseded_verdict: superseded_verdict_report(memo, patch, logic_version).is_some(),
        })
        .collect();
    Ok(ConversionSchedule {
        workers: pool,
        assignments,
    })
}

/// The memo-hit gate's failure report. The gate asserts two equalities —
/// the patch's base sha equals the trunk tip, and the recorded decision
/// for `(identity, base sha)` under the running logic version equals the
/// class the walk landed on. On failure the report names both operands
/// of the equality that failed; a gate that cannot print the values it
/// compared did not perform the comparison, and must not claim the
/// property failed (#3866). The lookup is for the version the pass is
/// running under (`logic_version`, [`CONVERSION_LOGIC_VERSION`]) — a
/// verdict a superseded logic recorded is invisible to this gate and is
/// the business of [`superseded_verdict_report`] instead (#4041). `None`
/// when the gate passed, or when nothing is recorded for the key under
/// this version (not yet evaluated, not failed).
pub fn memo_gate_report(
    memo: &ConversionMemo,
    patch: &Patch,
    current_tip: &str,
    logic_version: u32,
) -> Option<String> {
    if patch.base_sha != current_tip {
        return patch.stale_report(current_tip);
    }
    match memo.lookup(&patch.identity, &patch.base_sha, logic_version) {
        None => None,
        Some(recorded) if recorded == patch.class => None,
        Some(recorded) => Some(format!(
            "memo gate: recorded {:?} != walked {:?} for {}@{}",
            recorded, patch.class, patch.identity, patch.base_sha
        )),
    }
}

/// The superseded-verdict gate's failure report (#4041, #3866 pattern).
///
/// The gate asserts one equality: the logic version that produced the
/// newest recorded decision for this patch's (identity, base sha) pair
/// equals the logic version the pass is running under (`logic_version`).
/// A verdict a superseded logic computed is queryable —
/// [`ConversionMemo::records`] keeps it — but not a decision: the current
/// logic must re-verify it, and this report is what the pass prints
/// instead of applying the old verdict. On failure the report names both
/// operands — a gate that cannot print the values it compared did not
/// perform the comparison, and must not claim the property failed
/// (#3866). `None` when nothing is recorded for the pair, or when the
/// newest record is the current logic's own — in which case the decision
/// is live and the memo-hit gate's business, not this one's.
pub fn superseded_verdict_report(
    memo: &ConversionMemo,
    patch: &Patch,
    logic_version: u32,
) -> Option<String> {
    let newest = memo
        .records(&patch.identity, &patch.base_sha)
        .into_iter()
        .next()?;
    if newest.logic_version == logic_version {
        return None;
    }
    Some(format!(
        "superseded memo verdict: {}@{} was last decided under logic v{}, the pass is running v{} — the recorded decision requires re-verification, not a memo hit",
        patch.identity, patch.base_sha, newest.logic_version, logic_version
    ))
}

/// Why the pass held a patch (#3674). Recorded with the retirement so a
/// repeat hold is visible rather than silent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldReason {
    /// A deterministic rule — a lint or policy gate — failed, and the
    /// agent prompt or the rule itself has since changed.
    RuleFixed,
    /// A genuine build or test failure the agent reproduces.
    BuildFailure,
    /// The agent's own report held the patch unformatted (`fmt_rc != 0`,
    /// #3715). A deterministic signal the pass trusts without re-running:
    /// a fresh agent formats its submission, so a re-run differs.
    AgentReportedUnformatted,
    /// The agent's own report held the patch unbuilt (`build_rc != 0` or
    /// `status=BUILD-FAILED` / `TESTS-DO-NOT-COMPILE`, #3715). A
    /// deterministic signal the pass trusts without re-running; a re-run
    /// reproduces the failure, so it is retired only like a
    /// [`BuildFailure`]: it is not.
    AgentReportedUnbuilt,
    /// The build failed because the patch's base has drifted from the
    /// trunk tip: the patch compiles at its own base but not at the
    /// current tip (#4279). A re-dispatch at a fresh base differs — the
    /// new trunk code resolves the conflict — so this hold is retirable.
    StaleBase,
}

impl HoldReason {
    /// Whether a re-run of the held patch could plausibly differ. That is
    /// the retirement question, and it is a judgement about *why* the
    /// patch was held, not about the hold itself: a deterministic rule
    /// the agent has learned since, yes — a re-run meets the fixed rule;
    /// an agent-reported unformatted hold, yes — a fresh agent formats
    /// its submission; a stale-base hold (#4279), yes — the build failed
    /// because the trunk moved past the patch's base, and a re-dispatch
    /// at the new base compiles; a genuine build failure the agent
    /// reproduces, no — a re-run reproduces it, and retiring would
    /// discard paid-for work with no chance of a different outcome (the
    /// same applies to an agent-reported unbuilt hold: the failure is a
    /// property of the submission, not of the run).
    pub fn rerun_plausibly_differs(self) -> bool {
        matches!(
            self,
            Self::RuleFixed | Self::AgentReportedUnformatted | Self::StaleBase
        )
    }
}

/// One held patch being retired out of the active queue (#3674).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetirementPlan {
    /// The identity of the held patch being retired.
    pub patch: String,
    /// The retirement destination, deliberately outside the active patch
    /// glob: once the patch is moved there it no longer counts as
    /// "already produced", so the issue becomes eligible for
    /// re-dispatch.
    pub destination: String,
    /// Why the pass held the patch, recorded next to the retired patch so
    /// a repeat hold is visible rather than silent.
    pub reason: HoldReason,
}

/// The retirement directory under `root`: `{root}/out-retired`, outside
/// the active patch glob.
pub fn retirement_root(root: &str) -> String {
    format!("{root}/out-retired")
}

/// Plan the retirement of one held patch (#3674): held patches are
/// retired rather than left in place, so their issue becomes eligible
/// for re-dispatch, and the hold reason travels with the patch so a
/// repeat is visible.
///
/// Retirement is refused when:
/// - the patch is not held: only a held patch is inert and worth
///   retiring;
/// - the hold would not differ on a re-run ([`HoldReason`]): retiring a
///   build failure discards paid-for work with no chance of a different
///   outcome;
/// - the root is empty.
///
/// The plan does not move the patch or touch the memo. The caller moves
/// the file to `destination` and calls [`ConversionMemo::invalidate`]
/// for the same (identity, base sha, logic version) triple: skip the
/// invalidation and the pass re-holds the patch from the record, for the
/// same since-fixed reason.
pub fn plan_retirement(
    patch: &Patch,
    reason: HoldReason,
    root: &str,
) -> Result<RetirementPlan, String> {
    if patch.class != ConversionClass::MemoizedHold {
        return Err(format!(
            "only held patches can be retired, not {:?}",
            patch.class
        ));
    }
    if !reason.rerun_plausibly_differs() {
        return Err(format!(
            "refusing to retire a {:?} hold: a re-run would not plausibly differ",
            reason
        ));
    }
    if root.trim().is_empty() {
        return Err("retirement root must not be empty".to_string());
    }
    Ok(RetirementPlan {
        patch: patch.identity.clone(),
        destination: format!("{}/{}", retirement_root(root), patch.identity),
        reason,
    })
}

/// Classify a build failure by drift (#4279): attribute before accusing.
///
/// When the build gate fails, the first question is not *whether the
/// patch is broken* but *whether the trunk moved past the patch's base
/// in a way that broke the build*. The caller has already measured
/// `commits_behind` (the `git rev-list --count` result) and re-run the
/// build at the patch's own base. This function is the pure decision:
///
/// - **No drift** (`commits_behind == 0`): the base is the tip, so the
///   failure is the patch's own. [`HoldReason::BuildFailure`].
/// - **Drift, base compiles** (`commits_behind > 0` and
///   `base_compiles_clean`): the patch is fine at its own base; the
///   trunk moved underneath it. [`HoldReason::StaleBase`] — retirable,
///   fed into re-dispatch.
/// - **Drift, base also fails** (`commits_behind > 0` and
///   `!base_compiles_clean`): the failure reproduces at the base, so it
///   is the patch's own, not the drift's. [`HoldReason::BuildFailure`].
#[must_use]
pub fn classify_build_failure(commits_behind: usize, base_compiles_clean: bool) -> HoldReason {
    if commits_behind > 0 && base_compiles_clean {
        HoldReason::StaleBase
    } else {
        HoldReason::BuildFailure
    }
}

/// The drift clause for a build-failure outcome line (#4279): the
/// `"(patch is N commit(s) behind base {sha})"` parenthetical that makes
/// the drift visible next to the build error. Returns an empty string
/// when `commits_behind == 0` (no drift to report), so the caller can
/// unconditionally concatenate it.
#[must_use]
pub fn drift_clause(commits_behind: usize, base_sha: &str) -> String {
    if commits_behind == 0 {
        return String::new();
    }
    let unit = if commits_behind == 1 {
        "commit"
    } else {
        "commits"
    };
    format!("(patch is {commits_behind} {unit} behind base {base_sha})")
}

/// The health of a conversion queue, as the operator should see it
/// (#3674): the total, and the inert fraction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueHealth {
    /// Every patch in the queue, held or not.
    pub queued: usize,
    /// Patches the pass held: produced, unlandable, skipped by every
    /// later pass. Their issues are open but inert.
    pub held: usize,
}

impl QueueHealth {
    /// The inert fraction: held patches as a share of the queue. An empty
    /// queue is not inert — there is nothing sitting there.
    pub fn inert_fraction(&self) -> f64 {
        if self.queued == 0 {
            return 0.0;
        }
        self.held as f64 / self.queued as f64
    }

    /// The one line that makes the inert fraction visible: "120 queued,
    /// 43 held".
    pub fn report(&self) -> String {
        format!("{} queued, {} held", self.queued, self.held)
    }
}

/// Measure the queue's inert fraction in one loop (#3674). A patch counts
/// as held when its verdict is held — the same verdict the re-dispatch
/// gate references, never the bare existence of the patch. A candidate
/// and a [`ConversionClass::CouldNotEvaluate`] state count as queued but
/// not held: the pass has not decided them, and an undecided patch is
/// not yet inert.
pub fn queue_health(patches: &[Patch]) -> QueueHealth {
    let queued = patches.len();
    let held = patches
        .iter()
        .filter(|patch| patch.class.verdict() == Some(PatchVerdict::Held))
        .count();
    QueueHealth { queued, held }
}

/// The platform's `mergeable` state for an open pull request.
///
/// GitHub reports `MERGEABLE`, `CONFLICTING`, or `UNKNOWN` while it is
/// still computing. Only the first two are verdicts; the transition
/// between them — mergeable to conflicting — is the moment the cheap fix
/// expired (#3899).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mergeable {
    /// `MERGEABLE`: the PR can still be merged as-is or rebased cheaply.
    Mergeable,
    /// `CONFLICTING`: `main` has moved under the PR; the cheap fix
    /// (merge now, rebase later) has expired.
    Conflicting,
    /// `UNKNOWN`: the platform has not computed the state yet. Not a
    /// verdict: it must neither fire an alarm nor overwrite the last
    /// decisive observation.
    Unknown,
}

impl Mergeable {
    /// The platform's spelling, for summary lines and reports.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mergeable => "MERGEABLE",
            Self::Conflicting => "CONFLICTING",
            Self::Unknown => "UNKNOWN",
        }
    }
}

/// One pull request the conversion pass's family opened, still tracked.
///
/// "Opened a PR" is not a terminal state (#3899): a PR that was mergeable
/// when opened becomes unmergeable as `main` moves under it, and the
/// cost is superlinear in the latency (day 0: merge, day 1: rebase, day 3:
/// regenerate). Tracking it across passes is what makes that decay
/// visible before it is terminal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenPr {
    /// Stable PR identity (e.g. `"#3899"` or the PR URL). Must be
    /// non-empty.
    pub identity: String,
    /// The stamp at which the pass opened the PR (the same monotonic
    /// stamp space as [`Patch::produced_at`]). Age is always measured
    /// from the first opening, not the last sighting.
    pub opened_at: u64,
    /// The last decisive observation of the PR's `mergeable` state. A
    /// freshly opened PR against the current tip starts `Mergeable`.
    pub last_mergeable: Mergeable,
    /// The stamp of the last decisive observation.
    pub observed_at: u64,
}

/// The ledger of PRs the conversion pass's family opened that are still
/// open (#3899).
///
/// Conversion produces PRs and nothing consumed them: every individual PR
/// looked fine, so the queue was invisible. The ledger restores the two
/// things that were missing —
///
/// 1. **A line item.** [`PrLedger::summary_line`] reports the
///    previously-opened PRs still open, with age, for every run summary,
///    alongside converted/held/skipped. A number that only goes up is the
///    alarm.
/// 2. **An alarm on the transition, not on age alone.**
///    [`PrLedger::observe`] returns a report when a PR goes
///    `Mergeable -> Conflicting` — the moment the cheap fix expired — so
///    the pass that observes it surfaces it, in the same pass.
///
/// The caller performs the GitHub I/O: it records a PR when the pass
/// opens it, observes the current `mergeable` state each pass, and
/// settles a PR when it merges or is closed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrLedger {
    prs: BTreeMap<String, OpenPr>,
}

impl PrLedger {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.prs.is_empty()
    }

    pub fn len(&self) -> usize {
        self.prs.len()
    }

    /// The pass opened (or took ownership of) this PR at `stamp`.
    ///
    /// A PR opens against the current tip and is therefore mergeable at
    /// the moment of opening: the initial decisive observation is
    /// `Mergeable` at the opening stamp. Re-recording a known PR keeps the
    /// **original** `opened_at` — age is measured from the first opening,
    /// not the last sighting, or a PR re-seen every pass would stay age 0
    /// forever.
    pub fn record_opened(&mut self, identity: &str, opened_at: u64) -> Result<(), String> {
        if identity.trim().is_empty() {
            return Err("PR identity must not be empty".to_string());
        }
        let identity = identity.to_string();
        if let Some(existing) = self.prs.get_mut(&identity) {
            // A re-observation of an opening, not a new one: the age clock
            // does not reset.
            existing.opened_at = existing.opened_at.min(opened_at);
            return Ok(());
        }
        self.prs.insert(
            identity.clone(),
            OpenPr {
                identity,
                opened_at,
                last_mergeable: Mergeable::Mergeable,
                observed_at: opened_at,
            },
        );
        Ok(())
    }

    /// Record a fresh observation of the PR's `mergeable` state at `stamp`.
    ///
    /// Returns `Some(report)` exactly when the observation records a
    /// `Mergeable -> Conflicting` transition — the moment the cheap fix
    /// expired — naming the PR and both operands of the comparison, so
    /// the pass surfaces it **in this pass** rather than waiting for a
    /// rebase to fail (#3899). Repeated `Conflicting` observations are
    /// not re-reported: the alarm is on the transition, not on the state,
    /// so a PR that has already been surfaced is not alarmed at every
    /// pass. An `Unknown` observation is not a verdict: it leaves the
    /// entry unchanged, so a later `Conflicting` still fires against the
    /// last decisive state.
    pub fn observe(
        &mut self,
        identity: &str,
        state: Mergeable,
        stamp: u64,
    ) -> Result<Option<String>, String> {
        if identity.trim().is_empty() {
            return Err("PR identity must not be empty".to_string());
        }
        let pr = self
            .prs
            .get_mut(identity)
            .ok_or_else(|| format!("refusing to observe {identity}: not in the ledger; a PR the pass did not open is not one it tracks"))?;
        if state == Mergeable::Unknown {
            return Ok(None);
        }
        let transitioned =
            pr.last_mergeable == Mergeable::Mergeable && state == Mergeable::Conflicting;
        let report = if transitioned {
            Some(format!(
                "mergeability transition: {identity} went {} -> {} at stamp {} (opened at {}, age {})",
                Mergeable::Mergeable.as_str(),
                Mergeable::Conflicting.as_str(),
                stamp,
                pr.opened_at,
                format_age(stamp.saturating_sub(pr.opened_at)),
            ))
        } else {
            None
        };
        pr.last_mergeable = state;
        pr.observed_at = stamp;
        Ok(report)
    }

    /// The PR has settled: merged or closed. Remove it from the ledger and
    /// return the last decisive state it was tracked in, if it was tracked
    /// at all. A settled PR is no longer work in progress and must stop
    /// being counted in the summary.
    pub fn settle(&mut self, identity: &str) -> Option<Mergeable> {
        self.prs.remove(identity).map(|pr| pr.last_mergeable)
    }

    /// The PRs still open, with their age at `now`, oldest first: the
    /// alarm is the PR that has been open the longest. Ages saturate at
    /// zero if `now` ever lands before an opening stamp.
    pub fn open_with_age(&self, now: u64) -> Vec<(String, u64, Mergeable)> {
        let mut open: Vec<(String, u64, Mergeable)> = self
            .prs
            .values()
            .map(|pr| {
                (
                    pr.identity.clone(),
                    now.saturating_sub(pr.opened_at),
                    pr.last_mergeable,
                )
            })
            .collect();
        open.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        open
    }

    /// The run-summary line: the previously-opened PRs still open, with
    /// age, for every pass — "no line item" was the reason the queue was
    /// invisible (#3899). e.g. `2 previously opened PRs still open: #31
    /// (age 2h 45m 0s, MERGEABLE), #28 (age 1d 0h 0m 0s, CONFLICTING)`.
    pub fn summary_line(&self, now: u64) -> String {
        let open = self.open_with_age(now);
        if open.is_empty() {
            return "no previously opened PRs still open".to_string();
        }
        let parts: Vec<String> = open
            .iter()
            .map(|(id, age, state)| format!("{id} (age {}, {})", format_age(*age), state.as_str()))
            .collect();
        format!(
            "{} previously opened PR{} still open: {}",
            open.len(),
            if open.len() == 1 { "" } else { "s" },
            parts.join(", ")
        )
    }
}

/// Render an age in seconds as `Nd Nh Nm Ns`, omitting leading zero units
/// (but always keeping seconds), e.g. `0s`, `90m 5s`, `2h 45m 0s`,
/// `1d 2h 3m 4s`. Deterministic, so summary lines are stable and
/// diffable across runs.
fn format_age(seconds: u64) -> String {
    let days = seconds / 86_400;
    let hours = (seconds % 86_400) / 3_600;
    let minutes = (seconds % 3_600) / 60;
    let secs = seconds % 60;
    let mut parts: Vec<String> = Vec::new();
    if days > 0 {
        parts.push(format!("{days}d"));
    }
    if days > 0 || hours > 0 {
        parts.push(format!("{hours}h"));
    }
    if days > 0 || hours > 0 || minutes > 0 {
        parts.push(format!("{minutes}m"));
    }
    parts.push(format!("{secs}s"));
    parts.join(" ")
}

// ============================================================================
// Branch hermeticity (#3915)
// ============================================================================

/// A hermetic build of one conversion branch (#3915).
///
/// Two independent silent git defects made one helper land two patches in
/// one loop: `git checkout <branch>` carries staged changes across the
/// switch (when the staged paths do not conflict with the target, git moves
/// cleanly with exit 0 and the staged patch just comes along and ends up
/// committed onto the wrong branch), and a hard reset to `origin/main`
/// rewinds the branch you are *standing on*, not the branch you are about
/// to build. The plan encodes the fix: build every branch hermetically from
/// the base — `git checkout -B <branch> <base>` first, then clean, then
/// apply — never resetting while standing on the previous branch, and never
/// switching branches with a staged or dirty index. A build step that
/// inherits state from the previous iteration is not reproducible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchBuild {
    branch: String,
    base: String,
}

impl BranchBuild {
    /// Plans the hermetic build of `branch` from `base`. Both must be
    /// named: a build that cannot state what it is building and where from
    /// does not perform the hermeticity promise.
    pub fn new(branch: &str, base: &str) -> Result<Self, String> {
        if branch.trim().is_empty() || base.trim().is_empty() {
            return Err(
                "a branch build must name the branch and the base sha it is built from".to_string(),
            );
        }
        Ok(Self {
            branch: branch.to_string(),
            base: base.to_string(),
        })
    }

    pub fn branch(&self) -> &str {
        &self.branch
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    /// The setup steps, in the order the caller must execute them, before
    /// applying the patch. The order is the contract: `checkout -B` onto
    /// the base comes first, so the reset and clean run while standing on
    /// the *new* branch — a hard reset rewinds the branch you are standing
    /// on, and resetting before the checkout would rewind the
    /// branch the previous iteration just committed (#3915). The base is a
    /// sha, not a moving ref: `origin/main` can move between the fetch and
    /// the checkout, a sha cannot.
    ///
    /// The caller must run [`assert_clean_checkout`] before executing any
    /// of these steps: a switch with a staged or dirty index would carry
    /// the previous iteration's state onto this branch with exit 0.
    pub fn setup_steps(&self) -> Vec<String> {
        vec![
            format!("git checkout -B {} {}", self.branch, self.base),
            format!("git reset --hard {}", self.base), // linter:allow-SECURITY the plan's own step string: a hermetic build must hard-reset onto the base sha, not a moving ref
            "git clean -fdx".to_string(),
        ]
    }

    /// The gate that must pass before this branch is pushed: the branch's
    /// contents asserted against the source patch's file list (see
    /// [`assert_branch_matches_patch`]). `actual` is the file list the diff
    /// between the base and the branch reports (one path per line).
    pub fn pre_push_assert(
        &self,
        patch_identity: &str,
        declared: &[String],
        actual: &[String],
    ) -> Result<(), String> {
        assert_branch_matches_patch(patch_identity, self.base(), self.branch(), declared, actual)
    }
}

/// Refuses to build a branch while the checkout has staged, dirty, or
/// untracked state (#3915).
///
/// `git checkout <branch>` moves cleanly when the staged paths do not
/// conflict with the target — exit 0 — and the staged patch just comes
/// along and ends up committed onto the wrong branch. The caller observes
/// the checkout with `git status --porcelain` and passes every entry; any
/// entry blocks the switch. An untracked file is carried across the switch
/// the way a staged path is, so "clean" means empty, not "nothing staged".
/// A build step that inherits state from the previous iteration is not
/// reproducible.
pub fn assert_clean_checkout(worktree: &str, status: &[String]) -> Result<(), String> {
    let dirty: Vec<&String> = status
        .iter()
        .filter(|line| !line.trim().is_empty())
        .collect();
    if dirty.is_empty() {
        return Ok(());
    }
    Err(format!(
        "refusing to switch branches in {worktree}: {} staged/dirty/untracked entr{} before the build (first: `{}`); `git checkout` would carry them onto the target branch with exit 0 — clean the checkout first",
        dirty.len(),
        if dirty.len() == 1 { "y" } else { "ies" },
        dirty[0].as_str()
    ))
}

/// Asserts the branch's contents against the source patch's file list,
/// before anything is pushed (#3915).
///
/// `declared` is the file list the patch declares, `actual` is the list of
/// paths the diff between the base and the branch reports — what the branch
/// actually changed. A mismatch is a hard error, never a warning: the title, body,
/// gate table, and issue reference are all assembled from the loop
/// variable, and when the contents do not match they describe a different
/// tree than the one under review — a coherent, internally consistent,
/// entirely wrong document (#3915). Comparison is order-independent (a
/// patch's file list is not a diff's ordering), but the sets must match
/// exactly: a superset is as wrong as a subset.
pub fn assert_branch_matches_patch(
    patch_identity: &str,
    base: &str,
    branch: &str,
    declared: &[String],
    actual: &[String],
) -> Result<(), String> {
    let declared: std::collections::BTreeSet<&str> = declared.iter().map(String::as_str).collect();
    let actual: std::collections::BTreeSet<&str> = actual.iter().map(String::as_str).collect();
    if declared == actual {
        return Ok(());
    }
    let missing: Vec<&str> = declared.difference(&actual).copied().collect();
    let extra: Vec<&str> = actual.difference(&declared).copied().collect();
    Err(format!(
        "branch contents mismatch: {patch_identity} declares {} file(s) but the diff between {base} and {branch} shows {} — the branch does not contain this patch; refusing to push (missing from branch: [{}]; in branch but not declared: [{}])",
        declared.len(),
        actual.len(),
        missing.join(", "),
        extra.join(", ")
    ))
}

/// A build that produced no commits failed, and the failure is reported by
/// the *builder*, not later by the PR API as an opaque "No commits between
/// main and <branch>" (#3915).
///
/// `ahead` is the commit count the build measured with `git rev-list
/// --count <base>..<branch>` after the commit step. Zero means the commit
/// never happened — typically because the branch was silently rewound —
/// and the error names the patch: a failure that does not name its input
/// reports a consequence, not a cause, and the operator cannot tell which
/// of the N patches in the queue produced it.
pub fn require_commit(
    patch_identity: &str,
    base: &str,
    branch: &str,
    ahead: u64,
) -> Result<(), String> {
    if ahead == 0 {
        return Err(format!(
            "build failed for {patch_identity}: branch {branch} is ahead of {base} by 0 commits — the patch produced no commit, and no PR is opened for an empty diff"
        ));
    }
    Ok(())
}

/// Gate evidence bound to the tree it actually ran against (#3915).
///
/// The gate's numbers were real and the evidence was true — what was
/// missing was the binding between the evidence and the artifact:
/// "2090 passed" is not evidence unless it states *what* was tested. The
/// caller records the commit sha the gate ran against, and
/// [`GateEvidence::publish`] refuses to bind evidence to a different tree
/// than the one being proposed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateEvidence {
    /// The commit sha the gate actually ran against (observed, not
    /// assumed).
    pub ran_against: String,
    /// The gate's verdict.
    pub verdict: GateVerdict,
    /// The gate's own summary line, e.g. "2090 passed".
    pub summary: String,
}

impl GateEvidence {
    /// Records evidence. The sha must be named: a result that does not name
    /// the tree it tested cannot be published.
    pub fn record(
        ran_against: &str,
        verdict: GateVerdict,
        summary: impl Into<String>,
    ) -> Result<Self, String> {
        if ran_against.trim().is_empty() {
            return Err(
                "gate evidence must name the commit sha it ran against: a result that does not name its tree is not evidence".to_string(),
            );
        }
        Ok(Self {
            ran_against: ran_against.to_string(),
            verdict,
            summary: summary.into(),
        })
    }

    /// Binds the evidence to the branch being proposed. Refuses on sha
    /// mismatch — the gate ran against a different tree than the one under
    /// review, and publishing it would render a coherent, internally
    /// consistent, entirely wrong document (#3915). On success, returns
    /// the evidence line the PR body should carry.
    pub fn publish(self, branch: &str, branch_tip: &str) -> Result<String, String> {
        if self.ran_against != branch_tip {
            return Err(format!(
                "refusing to publish gate evidence for {branch}: the gate ran against {} but the branch tip is {} — the evidence describes a different tree than the one under review",
                self.ran_against, branch_tip
            ));
        }
        Ok(format!(
            "{} (gate ran against {} on {})",
            self.summary, self.ran_against, branch
        ))
    }
}

/// The publish steps in the order the pass runs them after the hermetic
/// build has passed its gate (#3972).
///
/// Each step is a separate process with a separate failure mode: `git
/// commit` can produce nothing, `git push` can be refused by the remote,
/// `gh pr create` can fail on an API error, and `gh pr merge` can meet a
/// conflict. Running the four as one gesture — then announcing the
/// outcome — is exactly how the pass reported `CONVERTED` for a patch
/// whose PR never existed (#3972).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PublishStep {
    /// `git commit` on the conversion branch.
    Commit,
    /// `git push` of the conversion branch to the remote.
    Push,
    /// `gh pr create` for the conversion branch.
    PrCreate,
    /// `gh pr merge` of the opened PR.
    PrMerge,
}

impl PublishStep {
    /// All steps, in run order.
    pub const ALL: [Self; 4] = [Self::Commit, Self::Push, Self::PrCreate, Self::PrMerge];

    /// The command the step runs, for log lines and errors.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Commit => "git commit",
            Self::Push => "git push",
            Self::PrCreate => "gh pr create",
            Self::PrMerge => "gh pr merge",
        }
    }

    /// Whether this step's failure turns the patch into a hold.
    ///
    /// Commit, push, and PR creation: when one of them fails, the PR
    /// does not exist, and claiming it is reporting a success that was
    /// not verified. A merge failure is different: the PR exists and is
    /// open, "merge deferred" is the honest description of that state,
    /// and the ledger keeps owning the PR (rule 11).
    pub fn failure_holds(self) -> bool {
        !matches!(self, Self::PrMerge)
    }
}

/// One executed publish step with its result verified and its output
/// captured (#3972).
///
/// The two fields the old pass lost: the exit status, which was
/// discarded, and the output, which was piped away. A failure with no
/// captured output has nothing to quote — the hold line states the
/// consequence instead of the cause, and the operator has to re-run the
/// command to find out why it failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedStep {
    step: PublishStep,
    exit: i32,
    output: String,
}

impl VerifiedStep {
    /// Record one step. The output must have been captured — both
    /// streams, to a file, read back — and refusing to record an empty
    /// capture is what makes "discarded the output" a loud error
    /// instead of a silent habit (#3972).
    pub fn record(step: PublishStep, exit: i32, output: &str) -> Result<Self, String> {
        if output.trim().is_empty() {
            return Err(format!(
                "refusing to record {}: the step's output capture is empty — capture both streams to a file and read it back before recording; a failure with no captured output is a failure with no cause",
                step.as_str()
            ));
        }
        Ok(Self {
            step,
            exit,
            output: output.to_string(),
        })
    }

    /// The step this result is for.
    pub fn step(&self) -> PublishStep {
        self.step
    }

    /// The exit status the caller observed.
    pub fn exit(&self) -> i32 {
        self.exit
    }

    /// The captured output, quoted in the hold line when the step
    /// failed.
    pub fn output(&self) -> &str {
        &self.output
    }

    /// Whether the step's exit status is zero.
    pub fn ok(&self) -> bool {
        self.exit == 0
    }
}

/// What the pass may report for one patch, derived only from the
/// verified steps and the PR the remote confirmed (#3972).
///
/// Every outcome also carries the `logic_version` of the decision logic
/// that derived it ([`CONVERSION_LOGIC_VERSION`]): a result is evidence
/// about its steps only under the version that computed it, so a result
/// a superseded version derived is attributable — and re-verifiable —
/// rather than indistinguishable from a current one (rule 13, #4041).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublicationOutcome {
    /// The PR exists and its number is the one `gh pr create` reported.
    /// `merged` is true only when the merge step was recorded with exit
    /// zero.
    Converted {
        /// The PR number the remote confirmed, e.g. `#42`.
        pr: String,
        /// Whether the merge step was verified.
        merged: bool,
        /// The logic version that derived this outcome
        /// ([`CONVERSION_LOGIC_VERSION`]) (#4041).
        logic_version: u32,
    },
    /// A step failed before the PR could exist. The hold names the step
    /// and quotes its captured output: the cause, not the consequence.
    Held {
        /// The step that failed.
        step: PublishStep,
        /// The step's captured output, quoted in the claim line.
        output: String,
        /// The logic version that derived this outcome
        /// ([`CONVERSION_LOGIC_VERSION`]) (#4041).
        logic_version: u32,
    },
}

impl PublicationOutcome {
    /// The line the pass may print for this outcome. Every token in the
    /// line is derived from a verified field: the PR number is the one
    /// the remote reported, and the quoted text is the one the failed
    /// step printed. Every line also names the logic version that
    /// derived the outcome — `[logic v{version}]` — so a claim a
    /// superseded version of the logic produced is distinguishable in
    /// the run summary at a glance (rule 13, #4041).
    pub fn claim_line(&self, patch_identity: &str) -> String {
        match self {
            Self::Converted { pr, merged: true, logic_version } => format!(
                "CONVERTED (PR {pr} merged): {patch_identity} [logic v{logic_version}]"
            ),
            Self::Converted { pr, merged: false, logic_version } => format!(
                "CONVERTED (PR {pr} open, merge deferred): {patch_identity} [logic v{logic_version}]"
            ),
            Self::Held { step, output, logic_version } => format!(
                "HELD at {}: {patch_identity} [logic v{logic_version}]: {output}",
                step.as_str()
            ),
        }
    }
}

/// Derive one patch's outcome from its verified steps. This is the
/// function the pass calls before printing any outcome line: the line
/// is assembled from what the remote confirmed, not from the intent to
/// run four commands (#3972).
///
/// `steps` are the steps the pass actually ran, in run order. A step
/// that fails stops the run, so a failure may only appear as the last
/// recorded step. `verified_pr` is the PR number extracted from the
/// remote's own response (`gh pr create` prints the URL); a `Converted`
/// outcome is refused without it — a pass that cannot point at a PR
/// number cannot claim a PR.
///
/// The outcome the function derives carries `logic_version` — the logic
/// version of this decision logic ([`CONVERSION_LOGIC_VERSION`]) — so a
/// later reader can attribute the result to the version that produced
/// it and re-verify results a superseded version derived (rule 13,
/// #4041).
pub fn decide_publication(
    patch_identity: &str,
    steps: &[VerifiedStep],
    verified_pr: Option<&str>,
    logic_version: u32,
) -> Result<PublicationOutcome, String> {
    let expected = PublishStep::ALL;
    if steps.is_empty() {
        return Err(format!(
            "{patch_identity}: no publish steps recorded — a pass cannot report an outcome for steps it never ran"
        ));
    }
    for (index, step) in steps.iter().enumerate() {
        if step.step() != expected[index] {
            return Err(format!(
                "{patch_identity}: publish step {index} is {} but the pass runs {} there — steps must be recorded in run order, each exactly once",
                step.step().as_str(),
                expected[index].as_str()
            ));
        }
    }
    for step in steps.iter().take(steps.len().saturating_sub(1)) {
        if !step.ok() {
            return Err(format!(
                "{patch_identity}: {} failed but later steps were recorded — a failed step stops the run; re-run the pass instead of reporting past the failure",
                step.step().as_str()
            ));
        }
    }
    let last = steps.last().expect("checked non-empty above");
    if !last.ok() && last.step().failure_holds() {
        return Ok(PublicationOutcome::Held {
            step: last.step(),
            output: last.output().to_string(),
            logic_version,
        });
    }
    if steps.len() < expected.len() && last.ok() {
        return Err(format!(
            "{patch_identity}: the pass stopped after {} without a recorded failure — a run that stops early must record why it stopped",
            last.step().as_str()
        ));
    }
    let pr = match verified_pr {
        Some(pr) if !pr.trim().is_empty() => pr.trim().to_string(),
        _ => {
            return Err(format!(
                "{patch_identity}: refusing to report CONVERTED without a PR number the remote confirmed — the counter counts verified PRs, not commands the pass intended to run"
            ))
        }
    };
    Ok(PublicationOutcome::Converted {
        pr,
        merged: last.ok(),
        logic_version,
    })
}

/// The run's publication record: one outcome per patch, with counters
/// derived from the outcomes rather than incremented beside them
/// (#3972).
///
/// A counter the driver bumps while printing the line can disagree with
/// the line — the bump and the claim are two gestures. Deriving the
/// count from the recorded outcomes makes them one: a patch the pass
/// did not record as converted does not enter the total.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PublicationLog {
    outcomes: Vec<(String, PublicationOutcome)>,
}

impl PublicationLog {
    /// An empty run record.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one patch's outcome. Each identity is recorded once: a
    /// second record for the same patch means the pass re-ran the
    /// publish without retiring the first claim, and silently taking
    /// either one is a guess.
    pub fn record(&mut self, identity: &str, outcome: PublicationOutcome) -> Result<(), String> {
        if identity.trim().is_empty() {
            return Err("a publication outcome must name the patch it is about".to_string());
        }
        if self.outcomes.iter().any(|(id, _)| id == identity) {
            return Err(format!(
                "{identity}: already recorded — a patch has one publication outcome per pass; retire the first claim before re-recording"
            ));
        }
        self.outcomes.push((identity.to_string(), outcome));
        Ok(())
    }

    /// The number of patches with a verified PR. Derived from the
    /// outcomes; never incremented by the driver (#3972).
    pub fn converted_count(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|(_, o)| matches!(o, PublicationOutcome::Converted { .. }))
            .count()
    }

    /// The number of patches held at a failed step.
    pub fn held_count(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|(_, o)| matches!(o, PublicationOutcome::Held { .. }))
            .count()
    }

    /// The claim line for one patch, if it was recorded.
    pub fn claim_line(&self, identity: &str) -> Option<String> {
        self.outcomes
            .iter()
            .find(|(id, _)| id == identity)
            .map(|(_, o)| o.claim_line(identity))
    }

    /// Every claim line, in record order — the block the pass prints.
    pub fn claim_lines(&self) -> Vec<String> {
        self.outcomes
            .iter()
            .map(|(id, o)| o.claim_line(id))
            .collect()
    }
}

/// Reconcile the run's converted claims against what the remote
/// actually shows (#3972).
///
/// `observed` is what the pass re-reads from the remote after the run
/// (`gh pr list`): (patch identity, PR number) pairs. Every patch the
/// log claims as converted must appear there under the same PR number
/// the run recorded. The check runs at the end of the pass, not at the
/// start of the next one: a claim with no PR behind it is a lie the
/// pass is about to hand to the operator, and the handoff is where the
/// damage happens.
pub fn reconcile_converted(
    log: &PublicationLog,
    observed: &[(&str, &str)],
) -> Result<String, String> {
    let mut missing: Vec<&str> = Vec::new();
    let mut mismatched: Vec<String> = Vec::new();
    for (identity, outcome) in log.outcomes.iter() {
        if let PublicationOutcome::Converted { pr, .. } = outcome {
            match observed.iter().find(|(id, _)| *id == identity.as_str()) {
                None => missing.push(identity),
                Some((_, number)) if number.trim() != pr.trim() => {
                    mismatched.push(format!("{identity}: recorded {pr}, remote shows {number}"))
                }
                Some(_) => {}
            }
        }
    }
    if !missing.is_empty() || !mismatched.is_empty() {
        let mut parts = Vec::new();
        if !missing.is_empty() {
            parts.push(format!(
                "reported converted but no PR observed for: [{}]",
                missing.join(", ")
            ));
        }
        if !mismatched.is_empty() {
            parts.push(format!(
                "PR number disagrees with the remote: [{}]",
                mismatched.join("; ")
            ));
        }
        return Err(format!("reconciliation failed: {}", parts.join("; ")));
    }
    let count = log.converted_count();
    Ok(format!(
        "reconciled: {} converted claim{} all backed by an observed PR",
        count,
        if count == 1 { "" } else { "s" }
    ))
}

/// The mid-run logic-change alarm (#4041, #3866 pattern).
///
/// The gate asserts one equality: the logic version the pass read when
/// it started (`logic_started`) equals the one it read again when it
/// finished (`logic_finished`), both readings of
/// [`CONVERSION_LOGIC_VERSION`]. When the logic's source changed between
/// the two readings — a hot rebuild, a version bump landing mid-pass —
/// the results the pass recorded are a mix of two logics, and the
/// summary must say so instead of attributing both to one. On failure
/// the report names both operands: a gate that cannot print the values
/// it compared did not perform the comparison, and must not claim the
/// property failed (#3866). `None` when the two readings agree — the
/// results are all one logic's work and need no alarm.
pub fn logic_change_report(logic_started: u32, logic_finished: u32) -> Option<String> {
    if logic_started == logic_finished {
        return None;
    }
    Some(format!(
        "conversion logic changed mid-run: started under v{logic_started}, finished under v{logic_finished} — results recorded under either version require re-verification"
    ))
}

// ---- #3742: the phase-2 count reconciliation ----

/// The phase-2 count reconciliation of the conversion pass (#3742).
///
/// The pass has two phases. Phase 1 is the cheap triage: the terminal
/// classes — an existing PR, a closed issue, a memoized hold, a
/// no-net-change — are decided without compute, and the remainder is
/// handed to phase 2 as the candidates that do need it. Phase 2 is the
/// expensive gate: each candidate is applied, compile-tested, committed,
/// and pushed, one at a time.
///
/// The two phases exchange exactly one number — how many candidates phase
/// 1 produced — and phase 2's only obligation is to process all of them.
/// The trap this rule closes is a pass that processes fewer than it was
/// handed and still exits `0`: the candidates it never reached are
/// *dropped, not decided*. They sit in the queue looking like finished
/// work, and because the pass that dropped them also owns the summary
/// that would have named them, the shortfall is silent. The classic
/// cause is a shell loop that reads its worklist from stdin while a
/// command in the body (`gh`, `cargo`, `git`, `ssh`) consumes it: the
/// loop reads one line, the command eats the rest, and the `while` ends
/// cleanly.
///
/// The gate asserts the one equality that matters: the candidates phase 1
/// produced (`candidates_produced`) equal the candidates phase 2 actually
/// processed (`candidates_processed`). On a mismatch in either direction
/// it returns an error naming both operands and the gap — a gate that
/// cannot print the values it compared did not perform the comparison
/// (#3866 pattern) — and the caller turns that error into a non-zero
/// exit, so the pass fails loudly instead of printing a summary that
/// adds up to a lie. When the counts reconcile it returns the line the
/// run summary can print.
///
/// The counts are passed in rather than read from a shared counter: the
/// caller is the only thing that knows both the worklist it handed phase
/// 2 and the work phase 2 actually finished, and deriving the gate from
/// the caller's two readings — not from a counter a buggy loop stopped
/// incrementing — is the detection.
pub fn reconcile_phase_counts(
    candidates_produced: usize,
    candidates_processed: usize,
) -> Result<String, String> {
    if candidates_processed < candidates_produced {
        let dropped = candidates_produced - candidates_processed;
        return Err(format!(
            "phase reconciliation failed: phase 1 produced {candidates_produced} candidate(s) but phase 2 processed only {candidates_processed} — {dropped} candidate(s) were dropped, not decided; the pass must not exit 0"
        ));
    }
    if candidates_processed > candidates_produced {
        let surplus = candidates_processed - candidates_produced;
        return Err(format!(
            "phase reconciliation failed: phase 1 produced {candidates_produced} candidate(s) but phase 2 processed {candidates_processed} — {surplus} candidate(s) processed beyond the worklist; the counts disagree and the pass must not exit 0"
        ));
    }
    Ok(format!(
        "phase reconciliation: all {candidates_produced} candidate(s) from phase 1 processed by phase 2"
    ))
}

// ---- #3899: the pass run summary ----

/// The run summary of one conversion pass (#3899).
///
/// The block the pass prints at the end of *every* pass, derived only
/// from what the pass recorded — never from what it intended:
///
/// 1. The converted/held counts, derived from the recorded outcomes
///    (rule 12, #3972). Between the counts and the claim lines: the
///    mid-run logic-change alarm ([`logic_change_report`]), present only
///    when the logic version the pass started under differs from the one
///    it finished under — results recorded under either version require
///    re-verification (rule 13, #4041). Then the per-patch claim lines
///    in recorded order, each naming the logic version that produced it.
/// 2. One line per mergeability transition this pass observed, in
///    observation order. The caller collects the reports its
///    [`PrLedger::observe`] calls returned during this pass and passes
///    them in; a fresh pass passes a fresh list. The `Mergeable ->
///    Conflicting` alarm is therefore surfaced in exactly one summary —
///    the pass that observed it, which is the moment the cheap fix
///    expired (rule 11, #3899) — and re-aging the PR in later passes
///    shows up in the line item, not as a repeated alarm.
/// 3. The line item: the previously opened PRs still open, with age
///    (rule 11, #3899). A pass that converted and held nothing prints
///    it too — a number that only goes up is the alarm, and a pass
///    that printed nothing is the pass that hid it.
///
/// Deterministic, so the block is stable across runs and diff-able:
/// the same recorded outcomes, observations, and ledger state produce
/// the same summary at the same stamp.
///
/// The logic versions are passed in rather than read from
/// [`CONVERSION_LOGIC_VERSION`] at the top and bottom: the pass reads
/// the constant when it starts and again when it finishes, and a
/// mid-run change to the logic's source is exactly the case this summary
/// must report — comparing the two readings, not the one value the
/// compiler inlined, is the detection (rule 13, #4041).
pub fn run_summary(
    log: &PublicationLog,
    ledger: &PrLedger,
    transitions: &[String],
    now: u64,
    logic_started: u32,
    logic_finished: u32,
) -> String {
    let mut lines = vec![format!(
        "converted: {}, held: {}",
        log.converted_count(),
        log.held_count()
    )];
    if let Some(report) = logic_change_report(logic_started, logic_finished) {
        lines.push(report);
    }
    lines.extend(log.claim_lines());
    lines.extend(transitions.iter().cloned());
    lines.push(ledger.summary_line(now));
    lines.join("\n")
}

/// The evidence of one gate run against the unmodified base (#3793).
///
/// The cheapest test of a gate's soundness is the self-test the pipeline
/// owes itself before spending worker time on real work: run each gate
/// against the base the work was produced against and require that it
/// reports *nothing*. A gate that flags the base as regressed is broken
/// by definition — the tree it runs on and the baseline it compares
/// against are the same commit, so any report it makes is a property of
/// the gate (an unstable comparison key), not of the work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateBaseEvidence {
    /// The gate's name (e.g. `build`, `validate`, `acceptance`), carried
    /// so a broken gate can be named in the report.
    pub gate: String,
    /// Whether the gate command ran to completion. An incomplete run is
    /// not evidence of cleanness: a gate that stopped without a verdict
    /// cannot be read as "nothing reported" (#3768 — a timed-out gate
    /// printed no failure lines, and the empty set was read as "all
    /// fixed").
    pub completed: bool,
    /// What the gate reported against the base (new failures, holds,
    /// flagged keys). Empty means the gate reports nothing.
    pub findings: Vec<String>,
}

/// The self-check's verdict for the gate set (#3793).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BaseSelfCheck {
    /// Every gate ran to completion and reported nothing against the
    /// unmodified base: base → nothing. The pass may proceed to real
    /// work.
    Clean,
    /// A gate did not run to completion, so its cleanness is unknown.
    /// Unknown is not safe: the pass holds rather than proceeding on a
    /// gate it cannot see.
    NoVerdict {
        /// The gate whose run did not complete.
        gate: String,
    },
    /// A gate reported something against the unmodified base: the gate is
    /// broken by definition, and no decision it makes about real work is
    /// trustworthy until it is. The report names the gate and the finding.
    FlagsBase {
        /// The gate that flagged the base.
        gate: String,
        /// The finding the gate reported against the base, verbatim.
        finding: String,
    },
}

impl BaseSelfCheck {
    /// Whether the pass may proceed to real work.
    pub fn is_clean(&self) -> bool {
        matches!(self, Self::Clean)
    }

    /// The pass's report line for the self-check; `None` when clean — a
    /// self-check that reports nothing has passed, mirroring the gates it
    /// checks.
    pub fn line(&self) -> Option<String> {
        match self {
            Self::Clean => None,
            Self::NoVerdict { gate } => Some(format!(
                "base self-check: gate {gate} did not complete — no verdict, not clean; holding the pass"
            )),
            Self::FlagsBase { gate, finding } => Some(format!(
                "base self-check: gate {gate} reported against the unmodified base: {finding} — the gate is broken; holding the pass"
            )),
        }
    }
}

/// Run the base self-check over the gate set (#3793): every gate must
/// have completed and reported nothing against the unmodified base.
///
/// A concrete finding beats a missing verdict regardless of gate order:
/// a named defect is reported rather than a vague one, so the operator
/// fixes the broken gate instead of chasing an incomplete run that may
/// have been incomplete because of it. An empty gate set is an error, not
/// a silent pass: a self-check that checked nothing proves nothing, and
/// reading it as clean is the same fold as reading a missing outcome as a
/// passing check.
pub fn base_gate_self_check(evidence: &[GateBaseEvidence]) -> Result<BaseSelfCheck, String> {
    if evidence.is_empty() {
        return Err(
            "base self-check needs at least one gate: an empty gate set proves nothing".to_string(),
        );
    }
    let mut incomplete: Option<&GateBaseEvidence> = None;
    for ev in evidence {
        if let Some(finding) = ev.findings.iter().next() {
            return Ok(BaseSelfCheck::FlagsBase {
                gate: ev.gate.clone(),
                finding: finding.clone(),
            });
        }
        if !ev.completed && incomplete.is_none() {
            incomplete = Some(ev);
        }
    }
    Ok(match incomplete {
        Some(ev) => BaseSelfCheck::NoVerdict {
            gate: ev.gate.clone(),
        },
        None => BaseSelfCheck::Clean,
    })
}

/// The dry-run rendering of one pass plan (#3793): every decision the
/// pass would make, printed without a git or cargo side effect.
///
/// The decision logic is pure, so CI can exercise it against fixture
/// queues on every pipeline change: plan the pass, render this report,
/// assert on it — no worker, no checkout, no compile. The report is a
/// pure function of the plan, so a pipeline change whose dry-run report
/// changes without a test change is a pipeline change that ran against
/// real work ungated by its own tests.
pub fn dry_run_report(schedule: &ConversionSchedule) -> String {
    if schedule.assignments.is_empty() {
        return "dry-run: nothing to convert".to_string();
    }
    let mut lines = vec![format!(
        "dry-run: {} worker(s), {} patch(es)",
        schedule.workers.len(),
        schedule.assignments.len()
    )];
    for a in &schedule.assignments {
        lines.push(format!(
            "worker {}: {} class={} memo_hit={} stale_base={} superseded_verdict={}",
            a.worker,
            a.patch.identity,
            a.patch.class.as_str(),
            a.memo_hit,
            a.stale_base,
            a.superseded_verdict
        ));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patch(identity: &str, base_sha: &str, class: ConversionClass) -> Patch {
        patch_at(identity, base_sha, class, 1)
    }

    fn patch_at(identity: &str, base_sha: &str, class: ConversionClass, produced_at: u64) -> Patch {
        Patch::new(identity, base_sha, class, produced_at, 0).unwrap()
    }

    fn patch_unblocks(
        identity: &str,
        base_sha: &str,
        class: ConversionClass,
        produced_at: u64,
        unblocks: usize,
    ) -> Patch {
        Patch::new(identity, base_sha, class, produced_at, unblocks).unwrap()
    }

    #[test]
    fn order_by_cost_puts_terminal_classes_before_candidates() {
        let patches = vec![
            patch_at("a", "sha-a", ConversionClass::Candidate, 40),
            patch_at("b", "sha-b", ConversionClass::ExistingPr, 30),
            patch_at("c", "sha-c", ConversionClass::Candidate, 50),
            patch_at("d", "sha-d", ConversionClass::NoNetChange, 20),
        ];

        let classes: Vec<_> = order_by_cost(&patches).iter().map(|p| p.class).collect();

        assert_eq!(
            classes,
            vec![
                ConversionClass::ExistingPr,
                ConversionClass::NoNetChange,
                ConversionClass::Candidate,
                ConversionClass::Candidate,
            ]
        );
    }

    #[test]
    fn order_by_cost_prefers_newest_first_within_a_class() {
        // A patch produced ten minutes ago against a base ten minutes old is
        // far likelier to apply than one from this morning (#3698).
        let patches = vec![
            patch_at("morning", "s1", ConversionClass::Candidate, 100),
            patch_at("recent", "s2", ConversionClass::Candidate, 300),
            patch_at("just-now", "s3", ConversionClass::Candidate, 250),
        ];

        let identities: Vec<_> = order_by_cost(&patches)
            .iter()
            .map(|p| p.identity.as_str())
            .collect();
        assert_eq!(identities, vec!["recent", "just-now", "morning"]);
    }

    #[test]
    fn order_by_cost_prefers_higher_unblocks_within_a_class() {
        // A patch that unblocks 79 downstream issues must be converted
        // before an unrelated newer patch that unblocks 0 (#3799).
        let patches = vec![
            patch_unblocks("newer-no-impact", "s1", ConversionClass::Candidate, 900, 0),
            patch_unblocks(
                "older-high-impact",
                "s2",
                ConversionClass::Candidate,
                100,
                79,
            ),
            patch_unblocks("mid", "s3", ConversionClass::Candidate, 500, 5),
        ];

        let identities: Vec<_> = order_by_cost(&patches)
            .iter()
            .map(|p| p.identity.as_str())
            .collect();
        assert_eq!(
            identities,
            vec!["older-high-impact", "mid", "newer-no-impact"]
        );
    }

    #[test]
    fn order_by_cost_defaults_zero_unblocks_degrades_to_mtime() {
        // With no impact data (all unblocks = 0), ordering is unchanged
        // from today: newest first within a class.
        let patches = vec![
            patch_unblocks("old", "s1", ConversionClass::Candidate, 100, 0),
            patch_unblocks("new", "s2", ConversionClass::Candidate, 300, 0),
            patch_unblocks("mid", "s3", ConversionClass::Candidate, 200, 0),
        ];

        let identities: Vec<_> = order_by_cost(&patches)
            .iter()
            .map(|p| p.identity.as_str())
            .collect();
        assert_eq!(identities, vec!["new", "mid", "old"]);
    }

    #[test]
    fn order_by_cost_ties_on_unblocks_use_mtime() {
        // Two patches with equal impact: the newer one goes first.
        let patches = vec![
            patch_unblocks("a", "s1", ConversionClass::Candidate, 100, 10),
            patch_unblocks("b", "s2", ConversionClass::Candidate, 200, 10),
        ];

        let identities: Vec<_> = order_by_cost(&patches)
            .iter()
            .map(|p| p.identity.as_str())
            .collect();
        assert_eq!(identities, vec!["b", "a"]);
    }

    #[test]
    fn order_summary_shows_top_candidates_with_unblock_counts() {
        let patches = vec![
            patch_unblocks("#46", "s1", ConversionClass::Candidate, 100, 79),
            patch_unblocks("#47", "s2", ConversionClass::Candidate, 200, 3),
            patch_unblocks("#99", "s3", ConversionClass::Candidate, 300, 0),
        ];
        let ordered = order_by_cost(&patches);
        let summary = order_summary(&ordered, 3);

        assert!(
            summary.contains("#46(class=Candidate,status=none,unblocks=79,produced_at=100)"),
            "got: {summary}"
        );
        assert!(
            summary.contains("#47(class=Candidate,status=none,unblocks=3,produced_at=200)"),
            "got: {summary}"
        );
        assert!(
            summary.contains("#99(class=Candidate,status=none,unblocks=0,produced_at=300)"),
            "got: {summary}"
        );
        assert!(summary.contains("3 candidates"), "got: {summary}");
        assert!(
            summary.contains("impact key has 3 distinct values, baseline: 3 items"),
            "got: {summary}"
        );
    }

    #[test]
    fn order_summary_reports_single_distinct_when_all_zero() {
        let patches = vec![
            patch_at("a", "s1", ConversionClass::Candidate, 100),
            patch_at("b", "s2", ConversionClass::Candidate, 200),
        ];
        let ordered = order_by_cost(&patches);
        let summary = order_summary(&ordered, 5);

        assert!(
            summary.contains("impact key has 1 distinct values, baseline: 2 items"),
            "got: {summary}"
        );
        assert!(
            summary.contains("degenerate: the key separates no pair"),
            "got: {summary}"
        );
    }

    #[test]
    fn order_summary_empty_queue() {
        let summary = order_summary(&[], 5);
        assert_eq!(summary, "phase 1 ordered: 0 candidates");
    }

    #[test]
    fn parse_unblock_counts_parses_tab_separated_content() {
        let content = "# comment line\n46\t79\n47\t3\n\n99\t0\n";
        let counts = parse_unblock_counts(content);
        assert_eq!(counts.get("46"), Some(&79));
        assert_eq!(counts.get("47"), Some(&3));
        assert_eq!(counts.get("99"), Some(&0));
        assert_eq!(counts.len(), 3);
    }

    #[test]
    fn parse_unblock_counts_ignores_malformed_lines() {
        let content = "46\t79\nbad-line-no-tab\n50\t\n";
        let counts = parse_unblock_counts(content);
        assert_eq!(counts.get("46"), Some(&79));
        // "bad-line-no-tab" has no tab so value defaults to 0, but it is
        // still a valid key entry.
        assert_eq!(counts.get("bad-line-no-tab"), Some(&0));
        // "50" with empty value defaults to 0.
        assert_eq!(counts.get("50"), Some(&0));
    }

    #[test]
    fn order_by_cost_keeps_queue_order_on_ties_and_is_stable() {
        // Patches tied on recency keep their queue order, and two passes
        // over the same queue must agree.
        let patches = vec![
            patch_at("alpha", "s1", ConversionClass::ClosedIssue, 50),
            patch_at("beta", "s2", ConversionClass::MemoizedHold, 50),
            patch_at("gamma", "s3", ConversionClass::ClosedIssue, 50),
        ];
        let first = order_by_cost(&patches);
        let second = order_by_cost(&patches);

        let identities: Vec<_> = first.iter().map(|p| p.identity.as_str()).collect();
        assert_eq!(identities, vec!["alpha", "gamma", "beta"]);
        assert_eq!(
            first.iter().map(|p| &p.identity).collect::<Vec<_>>(),
            second.iter().map(|p| &p.identity).collect::<Vec<_>>()
        );
    }

    #[test]
    fn absorb_new_adds_new_identities_and_dedups_by_recency() {
        // Patches produced mid-pass must not stay invisible: a re-scan
        // merges them into the running queue, deduped by identity, and
        // re-sorts the remaining queue (#3801).
        let mut queue = vec![
            patch_at("p1", "base-1", ConversionClass::Candidate, 100),
            patch_at("p2", "base-1", ConversionClass::MemoizedHold, 200),
        ];

        let arrived = absorb_new(
            &mut queue,
            vec![
                // New identity: appended.
                patch_at("p3", "base-2", ConversionClass::Candidate, 300),
                // Older observation of an existing patch: ignored.
                patch_at("p1", "base-1", ConversionClass::Candidate, 50),
                // Fresher observation of an existing patch: replaces, even
                // when the re-scan re-classified it.
                patch_at("p2", "base-2", ConversionClass::Candidate, 250),
            ],
        );

        // Only p3 is a new arrival; p1 and p2 are re-observations.
        assert_eq!(arrived, 1);
        // All three are candidates now: newest first.
        assert_eq!(
            queue
                .iter()
                .map(|p| (
                    p.identity.as_str(),
                    p.base_sha.as_str(),
                    p.class,
                    p.produced_at
                ))
                .collect::<Vec<_>>(),
            vec![
                ("p3", "base-2", ConversionClass::Candidate, 300),
                ("p2", "base-2", ConversionClass::Candidate, 250),
                ("p1", "base-1", ConversionClass::Candidate, 100),
            ]
        );
    }

    #[test]
    fn absorb_new_resorts_the_remaining_queue_by_cost_then_impact() {
        // A list ordered once at startup degrades into arrival order for
        // everything that comes later (#3801): the late terminal class jumps
        // ahead of both candidates even though its stamp is older than the
        // newest candidate — cost dominates recency.
        let mut queue = vec![
            patch_at("old-candidate", "s1", ConversionClass::Candidate, 100),
            patch_at("new-candidate", "s2", ConversionClass::Candidate, 300),
        ];

        let arrived = absorb_new(
            &mut queue,
            vec![patch_at(
                "late-terminal",
                "s3",
                ConversionClass::NoNetChange,
                150,
            )],
        );

        assert_eq!(arrived, 1);
        let identities: Vec<_> = queue.iter().map(|p| p.identity.as_str()).collect();
        assert_eq!(
            identities,
            vec!["late-terminal", "new-candidate", "old-candidate"]
        );
    }

    #[test]
    fn absorb_new_on_tie_prefers_the_fresh_observation() {
        // A re-scan is the newer observation of the same patch: on a tie the
        // fresh entry wins.
        let mut queue = vec![patch_at("p1", "base-1", ConversionClass::Candidate, 100)];

        let arrived = absorb_new(
            &mut queue,
            vec![patch_at("p1", "base-2", ConversionClass::ExistingPr, 100)],
        );

        // A re-observation of a known identity is not an arrival.
        assert_eq!(arrived, 0);
        assert_eq!(queue.len(), 1);
        assert_eq!(queue[0].base_sha, "base-2");
        assert_eq!(queue[0].class, ConversionClass::ExistingPr);
    }

    #[test]
    fn patch_is_stale_only_when_the_base_is_no_longer_the_tip() {
        let patch = patch("p1", "263368c7", ConversionClass::Candidate);
        assert!(patch.is_stale("785447cf"));
        assert!(!patch.is_stale("263368c7"));
    }

    #[test]
    fn patch_rejects_empty_identity_and_base_sha() {
        assert!(Patch::new("", "sha", ConversionClass::Candidate, 1, 0).is_err());
        assert!(Patch::new("id", "  ", ConversionClass::Candidate, 1, 0).is_err());
        assert!(Patch::new("id", "sha", ConversionClass::Candidate, 1, 0).is_ok());
    }

    #[test]
    fn plan_workers_gives_each_worker_a_unique_checkout_and_target_dir() {
        let pool = plan_workers(3, "/scratch/convert").unwrap();

        let worktrees: Vec<_> = pool.iter().map(|w| w.worktree.as_str()).collect();
        let targets: Vec<_> = pool.iter().map(|w| w.target_dir.as_str()).collect();
        assert_eq!(
            worktrees,
            vec![
                "/scratch/convert/worker-0",
                "/scratch/convert/worker-1",
                "/scratch/convert/worker-2"
            ]
        );
        assert_eq!(
            targets,
            vec![
                "/scratch/convert/target/worker-0",
                "/scratch/convert/target/worker-1",
                "/scratch/convert/target/worker-2"
            ]
        );
        // Uniqueness is what makes N-way concurrency safe under the
        // per-checkout lock.
        let unique = |paths: &Vec<String>| {
            let set: std::collections::HashSet<_> = paths.iter().collect();
            set.len() == paths.len()
        };
        let worktrees: Vec<_> = pool.iter().map(|w| w.worktree.clone()).collect();
        let targets: Vec<_> = pool.iter().map(|w| w.target_dir.clone()).collect();
        assert!(unique(&worktrees) && unique(&targets));
    }

    #[test]
    fn build_gate_default_covers_all_targets_and_names_its_command() {
        // A gate that skips test targets is not checking the patch:
        // `cargo build` never compiles the test file an agent just wrote.
        let gate = BuildGate::default();
        assert_eq!(gate, BuildGate::all_targets());
        assert_eq!(gate.command, "cargo build --workspace --all-targets");
    }

    #[test]
    fn every_worker_plan_carries_the_all_targets_build_gate() {
        let pool = plan_workers(2, "/scratch/convert").unwrap();

        for worker in &pool {
            assert_eq!(worker.build_gate, BuildGate::all_targets());
        }
    }

    #[test]
    fn classify_gate_makes_a_broken_test_target_a_hard_failure() {
        // build_rc is the all-targets gate: nonzero means a target the
        // patch touched — including its own tests — did not compile.
        assert_eq!(classify_gate(0, 0), GateVerdict::Green);
        assert_eq!(classify_gate(1, 0), GateVerdict::CompileFailure);
        // The observed defect: build_rc=0 came from a gate that skipped
        // test targets. With the all-targets gate the same situation is
        // build_rc=1 — a hard failure, not green.
        assert_eq!(classify_gate(1, 1), GateVerdict::CompileFailure);
        assert!(classify_gate(1, 0).is_failure());
        assert!(!classify_gate(0, 0).is_failure());
    }

    #[test]
    fn classify_gate_separates_compile_failure_from_tests_that_ran_and_failed() {
        // Both surface as test_rc=101 today; only the green all-targets
        // build gate says "the tests compiled, then some failed".
        assert_eq!(classify_gate(0, 101), GateVerdict::TestsFailed);
        assert_eq!(GateVerdict::CompileFailure.as_str(), "compile_failure");
        assert_eq!(GateVerdict::TestsFailed.as_str(), "tests_failed");
        assert_eq!(GateVerdict::Green.as_str(), "green");
        assert!(classify_gate(0, 101).is_failure());
    }

    #[test]
    fn plan_workers_rejects_zero_and_empty_root() {
        assert!(plan_workers(0, "/root").is_err());
        assert!(plan_workers(2, "  ").is_err());
    }

    #[test]
    fn memo_key_requires_both_identity_and_base_sha() {
        let mut memo = ConversionMemo::new();
        memo.record(
            "p1",
            "base-1",
            ConversionClass::ExistingPr,
            CONVERSION_LOGIC_VERSION,
        )
        .unwrap();

        assert_eq!(
            memo.lookup("p1", "base-1", CONVERSION_LOGIC_VERSION),
            Some(ConversionClass::ExistingPr)
        );
        // Same patch rebased onto a new trunk: a different key.
        assert_eq!(memo.lookup("p1", "base-2", CONVERSION_LOGIC_VERSION), None);
        // Same trunk, different patch: a different key.
        assert_eq!(memo.lookup("p2", "base-1", CONVERSION_LOGIC_VERSION), None);
    }

    #[test]
    fn memo_rejects_non_terminal_and_empty_keys() {
        let mut memo = ConversionMemo::new();
        let error = memo
            .record(
                "p1",
                "base-1",
                ConversionClass::Candidate,
                CONVERSION_LOGIC_VERSION,
            )
            .unwrap_err();
        assert!(error.contains("non-terminal"));
        assert!(memo
            .record(
                "",
                "base",
                ConversionClass::ExistingPr,
                CONVERSION_LOGIC_VERSION
            )
            .is_err());
        assert!(memo
            .record(
                "id",
                "  ",
                ConversionClass::ExistingPr,
                CONVERSION_LOGIC_VERSION
            )
            .is_err());
        assert!(memo.is_empty());
    }

    #[test]
    fn plan_pass_orders_assigns_and_flags_memo_hits() {
        let mut memo = ConversionMemo::new();
        memo.record(
            "hold-patch",
            "base-h",
            ConversionClass::MemoizedHold,
            CONVERSION_LOGIC_VERSION,
        )
        .unwrap();

        let patches = vec![
            patch("candidate-1", "base-a", ConversionClass::Candidate),
            patch("hold-patch", "base-h", ConversionClass::MemoizedHold),
            patch("candidate-2", "base-b", ConversionClass::Candidate),
            patch("pr-patch", "base-c", ConversionClass::ExistingPr),
        ];

        let schedule = plan_pass(
            &patches,
            2,
            "/scratch/convert",
            &memo,
            "base-h",
            CONVERSION_LOGIC_VERSION,
        )
        .unwrap();

        assert_eq!(schedule.workers.len(), 2);
        let order: Vec<_> = schedule
            .assignments
            .iter()
            .map(|a| {
                (
                    a.patch.identity.as_str(),
                    a.worker,
                    a.memo_hit,
                    a.stale_base,
                )
            })
            .collect();
        // Terminal classes first (in class order), then candidates;
        // round-robin over the cost order; only the memoized hold is a hit,
        // and only because its base is still the tip. The patches on older
        // bases are flagged for re-baselining.
        assert_eq!(
            order,
            vec![
                ("pr-patch", 0, false, true),
                ("hold-patch", 1, true, false),
                ("candidate-1", 0, false, true),
                ("candidate-2", 1, false, true),
            ]
        );
    }

    #[test]
    fn plan_pass_refuses_memo_hit_when_the_verdict_base_is_no_longer_the_tip() {
        // A HELD computed against an old base is a hypothesis, not a
        // decision: the (identity, base sha) pair matches, but the verdict
        // must be re-tested, not acted on (#3698).
        let mut memo = ConversionMemo::new();
        memo.record(
            "hold-patch",
            "263368c7",
            ConversionClass::MemoizedHold,
            CONVERSION_LOGIC_VERSION,
        )
        .unwrap();

        let patches = vec![patch(
            "hold-patch",
            "263368c7",
            ConversionClass::MemoizedHold,
        )];

        // main moved on; the verdict is stale.
        let schedule = plan_pass(
            &patches,
            1,
            "/root",
            &memo,
            "785447cf",
            CONVERSION_LOGIC_VERSION,
        )
        .unwrap();
        assert_eq!(schedule.assignments[0].memo_hit, false);
        assert!(schedule.assignments[0].stale_base);

        // Same verdict, base still the tip: honored.
        let schedule = plan_pass(
            &patches,
            1,
            "/root",
            &memo,
            "263368c7",
            CONVERSION_LOGIC_VERSION,
        )
        .unwrap();
        assert!(schedule.assignments[0].memo_hit);
        assert!(!schedule.assignments[0].stale_base);
    }

    #[test]
    fn plan_pass_orders_candidates_by_impact_then_newest_first() {
        let patches = vec![
            patch_unblocks("morning", "base-a", ConversionClass::Candidate, 100, 0),
            patch_unblocks("just-now", "base-b", ConversionClass::Candidate, 300, 0),
            patch_unblocks("high-impact", "base-c", ConversionClass::Candidate, 50, 79),
        ];

        let schedule = plan_pass(
            &patches,
            2,
            "/root",
            &ConversionMemo::new(),
            "base-b",
            CONVERSION_LOGIC_VERSION,
        )
        .unwrap();
        let identities: Vec<_> = schedule
            .assignments
            .iter()
            .map(|a| a.patch.identity.as_str())
            .collect();
        // high-impact leads (79 unblocks), then just-now (newer of the zeros).
        assert_eq!(identities, vec!["high-impact", "just-now", "morning"]);
    }

    #[test]
    fn memo_hit_requires_the_class_to_match_the_record() {
        // A patch re-classified after a rebase is not a hit even when its
        // (identity, base sha) pair has an entry: the recorded outcome no
        // longer describes this patch.
        let mut memo = ConversionMemo::new();
        memo.record(
            "p1",
            "base-1",
            ConversionClass::NoNetChange,
            CONVERSION_LOGIC_VERSION,
        )
        .unwrap();

        let patches = vec![patch("p1", "base-1", ConversionClass::Candidate)];
        let schedule = plan_pass(
            &patches,
            1,
            "/root",
            &memo,
            "base-1",
            CONVERSION_LOGIC_VERSION,
        )
        .unwrap();

        assert_eq!(schedule.assignments[0].memo_hit, false);
    }

    #[test]
    fn plan_pass_with_no_patches_plans_no_workers_and_propagates_pool_errors() {
        let schedule = plan_pass(
            &[],
            4,
            "/root",
            &ConversionMemo::new(),
            "base-1",
            CONVERSION_LOGIC_VERSION,
        )
        .unwrap();
        assert!(schedule.workers.is_empty());
        assert!(schedule.assignments.is_empty());

        let patches = vec![patch("p1", "base-1", ConversionClass::Candidate)];
        assert!(plan_pass(
            &patches,
            0,
            "/root",
            &ConversionMemo::new(),
            "base-1",
            CONVERSION_LOGIC_VERSION
        )
        .is_err());
        assert!(plan_pass(
            &patches,
            1,
            "",
            &ConversionMemo::new(),
            "base-1",
            CONVERSION_LOGIC_VERSION
        )
        .is_err());
    }

    #[test]
    fn plan_pass_rejects_an_empty_tip() {
        // The tip must come from a fresh fetch of origin/main; an empty
        // value means the re-baseline step was skipped, and a pass planned
        // without one would act on a snapshot.
        let patches = vec![patch("p1", "base-1", ConversionClass::Candidate)];
        let error = plan_pass(
            &patches,
            1,
            "/root",
            &ConversionMemo::new(),
            "  ",
            CONVERSION_LOGIC_VERSION,
        )
        .unwrap_err();
        assert!(error.contains("current tip"));
    }

    #[test]
    fn worklist_mid_run_arrival_is_converted_by_the_same_run() {
        // Acceptance (#3801): a patch written to shared storage during a run
        // is converted by that run, or explicitly named as deferred.
        let mut wl = Worklist::new(
            vec![patch_at("iw-45", "base-a", ConversionClass::Candidate, 100)],
            1000,
        );

        let next = wl.take_next().unwrap();
        assert_eq!(next.identity, "iw-45");

        // iw-47 finished on the cluster mid-run; the re-scan finds it and
        // the same run absorbs and converts it.
        let arrived = wl.absorb(vec![patch_at(
            "iw-47",
            "base-b",
            ConversionClass::Candidate,
            1100,
        )]);
        assert_eq!(arrived, 1);

        let next = wl.take_next().unwrap();
        assert_eq!(next.identity, "iw-47");

        let s = wl.summary();
        assert_eq!(s.initial, 1);
        assert_eq!(s.arrived, 1);
        assert_eq!(s.considered, 2);
        assert_eq!(s.remaining, 0);
        assert_eq!(s.deferred, 0);
        assert_eq!(s.on_disk, 2);
    }

    #[test]
    fn worklist_summary_adds_up_to_end_of_run_not_start() {
        // "141 candidates" stated once and never revised: the summary's
        // counts must add up to what was on disk at the *end* of the run.
        let mut wl = Worklist::new(
            vec![
                patch_at("a", "s", ConversionClass::Candidate, 100),
                patch_at("b", "s", ConversionClass::ClosedIssue, 50),
                patch_at("c", "s", ConversionClass::Candidate, 200),
            ],
            1000,
        );
        // Execution order: b (terminal first), then c, then a.
        assert_eq!(wl.remaining()[0].identity, "b");

        wl.take_next().unwrap();
        wl.take_next().unwrap();
        assert!(wl.rescan_due(2));

        // A re-scan: one new arrival, one re-observation of a queued patch.
        let arrived = wl.absorb(vec![
            patch_at("d", "s", ConversionClass::Candidate, 300),
            patch_at("a", "s", ConversionClass::Candidate, 100),
        ]);
        assert_eq!(arrived, 1);
        assert!(!wl.rescan_due(2));

        // A patch the run will not convert is named, not unrepresented,
        // and carries the reason it was deferred (#3783).
        wl.defer(
            patch_at("e", "s", ConversionClass::Candidate, 310),
            "base is stale beyond the run's rebaseline window",
        )
        .unwrap();

        let s = wl.summary();
        assert_eq!(s.initial, 3);
        assert_eq!(s.arrived, 1);
        assert_eq!(s.considered, 2);
        assert_eq!(s.remaining, 2);
        assert_eq!(s.deferred, 1);
        assert_eq!(s.on_disk, 5);
        // The counts add up to what was on disk at the end of the run.
        assert_eq!(s.considered + s.remaining + s.deferred, s.on_disk);

        let line = wl.summary_line(2000);
        assert!(
            line.starts_with("worklist frozen at 1000 (elapsed 1000); "),
            "{}",
            line
        );
        assert!(line.contains("1 patches arrived since freeze"), "{}", line);
        assert!(
            line.contains(
                "2 considered, 2 remaining, 1 deferred to the next run [e: base is stale beyond the run's rebaseline window]"
            ),
            "{}",
            line
        );
    }

    #[test]
    fn rescan_due_counts_candidates_not_wall_clock() {
        let mut wl = Worklist::new(
            vec![
                patch_at("a", "s", ConversionClass::Candidate, 1),
                patch_at("b", "s", ConversionClass::Candidate, 2),
                patch_at("c", "s", ConversionClass::Candidate, 3),
            ],
            1000,
        );

        // Every candidate: not due before the first take, due right after.
        assert!(!wl.rescan_due(1));
        wl.take_next().unwrap();
        assert!(wl.rescan_due(1));

        // The timer resets on absorb; every two candidates, due after two.
        wl.absorb(Vec::new());
        assert!(!wl.rescan_due(2));
        wl.take_next().unwrap();
        assert!(!wl.rescan_due(2));
        wl.take_next().unwrap();
        assert!(wl.rescan_due(2));

        // Zero means always due.
        let empty = Worklist::new(Vec::<Patch>::new(), 1000);
        assert!(empty.rescan_due(0));
    }

    #[test]
    fn defer_rejects_queued_and_duplicate_patches() {
        let mut wl = Worklist::new(
            vec![patch_at("queued", "s", ConversionClass::Candidate, 1)],
            1000,
        );

        let err = wl
            .defer(patch_at("queued", "s", ConversionClass::Candidate, 1), "r")
            .unwrap_err();
        assert!(err.contains("already in the remaining worklist"), "{}", err);

        wl.defer(
            patch_at("later", "s", ConversionClass::Candidate, 2),
            "held by the agent's report",
        )
        .unwrap();
        let err = wl
            .defer(patch_at("later", "s", ConversionClass::Candidate, 2), "r")
            .unwrap_err();
        assert!(err.contains("already named as deferred"), "{}", err);
    }

    #[test]
    fn worklist_starts_in_execution_order() {
        let wl = Worklist::new(
            vec![
                patch_at("old", "s", ConversionClass::Candidate, 100),
                patch_at("terminal", "s", ConversionClass::NoNetChange, 90),
                patch_at("new", "s", ConversionClass::Candidate, 200),
            ],
            1000,
        );

        let identities: Vec<_> = wl.remaining().iter().map(|p| p.identity.as_str()).collect();
        assert_eq!(identities, vec!["terminal", "new", "old"]);
        assert_eq!(wl.frozen_at(), 1000);
    }

    #[test]
    fn ordering_key_records_each_dimension() {
        let mut p = patch_unblocks("#46", "s1", ConversionClass::Candidate, 100, 79);
        p = p.with_agent_status("PASS");
        assert_eq!(
            p.ordering_key(),
            "class=Candidate,status=PASS,unblocks=79,produced_at=100"
        );
        // A blank status is no report, like the absence of one.
        let p = patch_at("#47", "s2", ConversionClass::Candidate, 200).with_agent_status("   ");
        assert_eq!(
            p.ordering_key(),
            "class=Candidate,status=none,unblocks=0,produced_at=200"
        );
    }

    #[test]
    fn ordering_key_distinguishes_the_class_strings() {
        // CouldNotEvaluate is all-caps: it must not be confusable with a
        // PascalCase variant name in the log.
        assert_eq!(
            ConversionClass::CouldNotEvaluate.as_str(),
            "COULD-NOT-EVALUATE"
        );
        assert_eq!(ConversionClass::Candidate.as_str(), "Candidate");
    }

    #[test]
    fn verified_status_orders_before_unverified_at_equal_cost_and_impact() {
        let verified =
            patch_at("verified", "s", ConversionClass::Candidate, 100).with_agent_status("PASS");
        let unverified = patch_at("unverified", "s", ConversionClass::Candidate, 100);
        let failing = patch_at("failing", "s", ConversionClass::Candidate, 100)
            .with_agent_status("BUILD-FAILED");
        let patches = [failing.clone(), unverified.clone(), verified.clone()];
        let ordered = order_by_cost(&patches);
        assert_eq!(ordered[0].identity, "verified");
        assert_eq!(ordered[1].identity, "unverified");
        assert_eq!(ordered[2].identity, "failing");

        // The status dimension ranks before impact: a verified patch with
        // zero unblocks still leads an unverified patch with 99.
        let high = patch_unblocks("high", "s", ConversionClass::Candidate, 100, 99)
            .with_agent_status("TIMEOUT");
        let low = patch_unblocks("low", "s", ConversionClass::Candidate, 100, 0)
            .with_agent_status("PASS");
        let patches = [high.clone(), low.clone()];
        let ordered = order_by_cost(&patches);
        assert_eq!(ordered[0].identity, "low");
        assert_eq!(ordered[1].identity, "high");
    }

    #[test]
    fn order_summary_records_the_per_item_key() {
        let p = patch_unblocks("#46", "s1", ConversionClass::Candidate, 100, 79)
            .with_agent_status("PASS");
        let ordered = order_by_cost(std::slice::from_ref(&p));
        let summary = order_summary(&ordered, 5);
        assert!(
            summary.contains("#46(class=Candidate,status=PASS,unblocks=79,produced_at=100)"),
            "got: {summary}"
        );
    }

    #[test]
    fn order_summary_reports_degenerate_key_relative_to_baseline() {
        let patches = vec![
            patch_at("a", "s1", ConversionClass::Candidate, 100),
            patch_at("b", "s2", ConversionClass::Candidate, 200),
            patch_at("c", "s3", ConversionClass::Candidate, 300),
        ];
        let summary = order_summary(&order_by_cost(&patches), 3);
        assert!(
            summary.contains("impact key has 1 distinct values, baseline: 3 items"),
            "got: {summary}"
        );
        assert!(
            summary.contains("degenerate: the key separates no pair"),
            "got: {summary}"
        );
    }

    #[test]
    fn defer_records_the_reason_and_summary_line_reports_age() {
        let mut wl = Worklist::new(
            vec![
                patch_at("a", "s", ConversionClass::Candidate, 1),
                patch_at("b", "s", ConversionClass::Candidate, 2),
            ],
            1000,
        );
        wl.take_next().unwrap();
        wl.defer(
            patch_at("b", "s", ConversionClass::Candidate, 2),
            "held by the agent's report",
        )
        .unwrap();

        // The deferred entry keeps its recorded reason (#3783).
        assert_eq!(wl.deferred().len(), 1);
        assert_eq!(wl.deferred()[0].patch.identity, "b");
        assert_eq!(wl.deferred()[0].reason, "held by the agent's report");

        // A defer without a reason is refused, not defaulted.
        let err = wl
            .defer(patch_at("c", "s", ConversionClass::Candidate, 3), "   ")
            .unwrap_err();
        assert!(err.contains("without a reason"), "{}", err);

        // The age of the worklist is now minus the frozen stamp, and
        // saturates instead of underflowing.
        assert_eq!(wl.elapsed_since_freeze(2440), 1440);
        assert_eq!(wl.elapsed_since_freeze(500), 0);

        let line = wl.summary_line(2440);
        assert!(
            line.starts_with("worklist frozen at 1000 (elapsed 1440); "),
            "{}",
            line
        );
        assert!(
            line.contains("1 deferred to the next run [b: held by the agent's report]"),
            "{}",
            line
        );
    }

    #[test]
    fn could_not_evaluate_is_terminal_but_not_memoizable() {
        // A gate that could not run is a terminal state of the pass (the
        // worker moves on) but not a decision about the patch (#3866).
        assert!(ConversionClass::CouldNotEvaluate.is_terminal());
        assert!(!ConversionClass::CouldNotEvaluate.is_memoizable());
        // Memoizing it would permanently skip the re-evaluation.
        for class in ConversionClass::ALL {
            assert_eq!(
                class.is_memoizable(),
                class != ConversionClass::Candidate && class != ConversionClass::CouldNotEvaluate
            );
        }
        // The pass orders a blocked evaluation before it spends a build on a
        // candidate: retry the setup before starting new work.
        assert!(ConversionClass::CouldNotEvaluate < ConversionClass::Candidate);
        // Every class is represented exactly once in the ordering.
        assert_eq!(ConversionClass::ALL.len(), 6);
    }

    #[test]
    fn memo_refuses_could_not_evaluate_and_the_error_names_the_operands() {
        // Acceptance: "the property is false" (HELD) and "I could not
        // evaluate it" demand opposite responses. Memoizing the second
        // would make one run's setup failure a permanent verdict against
        // the patch (#3866).
        let mut memo = ConversionMemo::new();
        let err = memo
            .record(
                "p1",
                "base-1",
                ConversionClass::CouldNotEvaluate,
                CONVERSION_LOGIC_VERSION,
            )
            .unwrap_err();
        assert!(err.contains("CouldNotEvaluate"), "{err}");
        assert!(err.contains("p1"), "{err}");
        assert!(err.contains("base-1"), "{err}");
        assert!(memo.is_empty());
        // A held patch is a decision about the patch and stays memoizable.
        memo.record(
            "p1",
            "base-1",
            ConversionClass::MemoizedHold,
            CONVERSION_LOGIC_VERSION,
        )
        .unwrap();
        assert_eq!(
            memo.lookup("p1", "base-1", CONVERSION_LOGIC_VERSION),
            Some(ConversionClass::MemoizedHold)
        );
    }

    #[test]
    fn a_failed_evaluation_is_re_evaluated_on_every_pass() {
        // The same patch goes through the same classifier twice: run one's
        // setup dies, run two's succeeds. The pass must re-evaluate, not
        // remember, because the first run's failure was about the setup,
        // not the patch (#3866).
        let mut memo = ConversionMemo::new();

        // Run one: the build died before the patch was evaluated.
        let mut blocked = patch("p1", "base-1", ConversionClass::CouldNotEvaluate);
        let queue1 = [blocked.clone()];
        let schedule = plan_pass(
            &queue1,
            1,
            "/root",
            &memo,
            "base-1",
            CONVERSION_LOGIC_VERSION,
        )
        .unwrap();
        assert!(!schedule.assignments[0].memo_hit);
        assert!(memo
            .record(
                "p1",
                "base-1",
                ConversionClass::CouldNotEvaluate,
                CONVERSION_LOGIC_VERSION
            )
            .is_err());

        // Run two: the setup succeeds and the walk lands on HELD.
        blocked.class = ConversionClass::MemoizedHold;
        let queue2 = [blocked.clone()];
        let schedule = plan_pass(
            &queue2,
            1,
            "/root",
            &memo,
            "base-1",
            CONVERSION_LOGIC_VERSION,
        )
        .unwrap();
        assert!(!schedule.assignments[0].memo_hit);
        memo.record(
            "p1",
            "base-1",
            ConversionClass::MemoizedHold,
            CONVERSION_LOGIC_VERSION,
        )
        .unwrap();

        // Run three: now the decision is about the patch and memoizable.
        let queue3 = [blocked];
        let schedule = plan_pass(
            &queue3,
            1,
            "/root",
            &memo,
            "base-1",
            CONVERSION_LOGIC_VERSION,
        )
        .unwrap();
        assert!(schedule.assignments[0].memo_hit);
    }

    #[test]
    fn stale_report_names_both_operands_on_failure_and_silences_on_pass() {
        // A gate that cannot print the values it compared did not perform
        // the comparison (#3866).
        let patch = patch("p1", "263368c7", ConversionClass::Candidate);
        let report = patch.stale_report("785447cf").unwrap();
        assert!(report.contains("263368c7"), "{report}");
        assert!(report.contains("785447cf"), "{report}");
        assert!(report.contains("p1"), "{report}");
        assert_eq!(patch.stale_report("263368c7"), None);
    }

    #[test]
    fn memo_gate_report_names_both_operands_of_the_failing_equality() {
        // Base no longer the tip: the stale-base equality failed and the
        // report names both sides.
        let patch = patch("p1", "263368c7", ConversionClass::MemoizedHold);
        let report = memo_gate_report(
            &ConversionMemo::new(),
            &patch,
            "785447cf",
            CONVERSION_LOGIC_VERSION,
        )
        .unwrap();
        assert!(report.contains("263368c7"), "{report}");
        assert!(report.contains("785447cf"), "{report}");

        // Nothing recorded for the key: not yet evaluated, not failed.
        assert_eq!(
            memo_gate_report(
                &ConversionMemo::new(),
                &patch,
                "263368c7",
                CONVERSION_LOGIC_VERSION
            ),
            None
        );

        // A recorded decision different from the walk: the class equality
        // failed and the report names both operands.
        let mut memo = ConversionMemo::new();
        memo.record(
            "p1",
            "263368c7",
            ConversionClass::NoNetChange,
            CONVERSION_LOGIC_VERSION,
        )
        .unwrap();
        let report = memo_gate_report(&memo, &patch, "263368c7", CONVERSION_LOGIC_VERSION).unwrap();
        assert!(report.contains("NoNetChange"), "{report}");
        assert!(report.contains("MemoizedHold"), "{report}");
        assert!(report.contains("p1"), "{report}");
        assert!(report.contains("263368c7"), "{report}");

        // Both equalities hold: the gate passed.
        memo.record(
            "p1",
            "263368c7",
            ConversionClass::MemoizedHold,
            CONVERSION_LOGIC_VERSION,
        )
        .unwrap();
        assert_eq!(
            memo_gate_report(&memo, &patch, "263368c7", CONVERSION_LOGIC_VERSION),
            None
        );
    }

    #[test]
    fn order_summary_reports_could_not_evaluate_separately_from_held() {
        // The two states must be distinguishable in the log without reading
        // the log: a pass whose build died is not a pass that held the
        // patch, and the summary says which (#3866).
        let died_queue = vec![
            patch_at("died", "s1", ConversionClass::CouldNotEvaluate, 100),
            patch_at("cand", "s2", ConversionClass::Candidate, 50),
        ];
        let died_ordered = order_by_cost(&died_queue);
        // The blocked evaluation is retried before new builds start.
        assert_eq!(died_ordered[0].identity, "died");
        let died_summary = order_summary(&died_ordered, 2);
        assert!(
            died_summary.contains("COULD-NOT-EVALUATE"),
            "got: {died_summary}"
        );

        let held_queue = vec![
            patch_at("held", "s1", ConversionClass::MemoizedHold, 100),
            patch_at("cand", "s2", ConversionClass::Candidate, 50),
        ];
        let held_ordered = order_by_cost(&held_queue);
        let held_summary = order_summary(&held_ordered, 2);
        assert!(held_summary.contains("MemoizedHold"), "got: {held_summary}");
        assert!(
            !held_summary.contains("COULD-NOT-EVALUATE"),
            "got: {held_summary}"
        );
        assert_ne!(died_summary, held_summary);
    }

    #[test]
    fn pr_ledger_record_opened_rejects_empty_identity() {
        let mut ledger = PrLedger::new();
        assert!(ledger.record_opened("", 100).is_err());
        assert!(ledger.record_opened("   ", 100).is_err());
        assert!(ledger.is_empty());
    }

    #[test]
    fn pr_ledger_re_recording_keeps_the_original_opening_stamp() {
        // A PR re-seen every pass must not reset its age clock: age is
        // measured from the first opening, not the last sighting.
        let mut ledger = PrLedger::new();
        ledger.record_opened("#31", 100).unwrap();
        ledger.record_opened("#31", 9000).unwrap();

        let open = ledger.open_with_age(10_000);
        assert_eq!(open, vec![("#31".to_string(), 9_900, Mergeable::Mergeable)]);
        assert_eq!(ledger.len(), 1);
    }

    #[test]
    fn pr_ledger_observe_surfaces_mergeable_to_conflicting_in_one_pass() {
        // Acceptance: a PR whose mergeable transitions to CONFLICTING is
        // surfaced within one pass — the pass that observes it, not some
        // later age threshold (#3899).
        let mut ledger = PrLedger::new();
        ledger.record_opened("#28", 1_000).unwrap();

        // Same pass or a later one: still fine.
        assert_eq!(ledger.observe("#28", Mergeable::Mergeable, 2_000), Ok(None));

        // main moved under it: the transition fires, and the report names
        // the PR and both operands of the comparison.
        let report = ledger
            .observe("#28", Mergeable::Conflicting, 87_400)
            .unwrap()
            .unwrap();
        assert!(report.contains("#28"), "{report}");
        assert!(report.contains("MERGEABLE"), "{report}");
        assert!(report.contains("CONFLICTING"), "{report}");
        assert!(report.contains("87400"), "{report}");
        assert!(report.contains("1d 0h 0m 0s"), "{report}");
    }

    #[test]
    fn pr_ledger_alarm_is_on_the_transition_not_the_state() {
        // A PR already surfaced stays alarming in the summary but is not
        // re-reported at every pass: repeated Conflicting observations are
        // not transitions.
        let mut ledger = PrLedger::new();
        ledger.record_opened("#28", 1_000).unwrap();
        assert!(ledger
            .observe("#28", Mergeable::Conflicting, 2_000)
            .unwrap()
            .is_some());
        assert_eq!(
            ledger.observe("#28", Mergeable::Conflicting, 3_000),
            Ok(None)
        );
        assert_eq!(
            ledger.observe("#28", Mergeable::Conflicting, 4_000),
            Ok(None)
        );
    }

    #[test]
    fn pr_ledger_unknown_is_not_a_verdict() {
        // While the platform is still computing, the observation must
        // neither fire an alarm nor overwrite the last decisive state: a
        // later Conflicting still fires against Mergeable.
        let mut ledger = PrLedger::new();
        ledger.record_opened("#28", 1_000).unwrap();
        assert_eq!(ledger.observe("#28", Mergeable::Unknown, 2_000), Ok(None));
        assert!(ledger
            .observe("#28", Mergeable::Conflicting, 3_000)
            .unwrap()
            .is_some());
    }

    #[test]
    fn pr_ledger_rebase_back_to_mergeable_arms_the_next_transition() {
        // A rebase that restores mergeability clears the conflicting
        // state; if main moves under it again, the transition fires
        // again.
        let mut ledger = PrLedger::new();
        ledger.record_opened("#28", 1_000).unwrap();
        assert!(ledger
            .observe("#28", Mergeable::Conflicting, 2_000)
            .unwrap()
            .is_some());
        assert_eq!(ledger.observe("#28", Mergeable::Mergeable, 3_000), Ok(None));
        assert!(ledger
            .observe("#28", Mergeable::Conflicting, 4_000)
            .unwrap()
            .is_some());
    }

    #[test]
    fn pr_ledger_observe_rejects_a_pr_it_does_not_track() {
        // Observing a PR the pass did not open would make the ledger track
        // other people's queue: refuse.
        let mut ledger = PrLedger::new();
        let err = ledger.observe("#999", Mergeable::Mergeable, 1).unwrap_err();
        assert!(err.contains("#999"), "{err}");
        assert!(err.contains("not in the ledger"), "{err}");
        assert!(ledger.observe("  ", Mergeable::Mergeable, 1).is_err());
    }

    #[test]
    fn pr_ledger_settle_stops_counting_the_pr() {
        let mut ledger = PrLedger::new();
        ledger.record_opened("#28", 1_000).unwrap();
        assert_eq!(ledger.settle("#28"), Some(Mergeable::Mergeable));
        assert!(ledger.is_empty());
        assert_eq!(ledger.settle("#28"), None);
        assert_eq!(
            ledger.summary_line(10_000),
            "no previously opened PRs still open"
        );
    }

    #[test]
    fn open_pr_older_than_one_pass_appears_in_the_next_summary() {
        // Acceptance, exercised against a populated case (#3793): a PR
        // opened by an earlier pass still open now must appear in this
        // pass's summary, with age — the line item that was missing
        // (#3899).
        let mut ledger = PrLedger::new();

        // Pass one: opens a PR and (as today) moves on.
        ledger.record_opened("#28", 1_000).unwrap();

        // Pass two, later: the PR was observed mergeable in between, and
        // is still open. It must show up in the summary with its age.
        ledger.observe("#28", Mergeable::Mergeable, 10_000).unwrap();
        let line = ledger.summary_line(10_000);
        assert!(line.contains("#28"), "{line}");
        assert!(line.contains("age 2h 30m 0s"), "{line}");
        assert!(line.contains("MERGEABLE"), "{line}");
        assert!(line.starts_with("1 previously opened PR "), "{line}");
    }

    #[test]
    fn summary_line_reports_all_open_prs_oldest_first() {
        // A number that only goes up is the alarm: with several PRs open
        // the summary names each with its age, oldest first.
        let mut ledger = PrLedger::new();
        ledger.record_opened("#28", 1_000).unwrap();
        ledger.record_opened("#31", 8_000).unwrap();
        ledger
            .observe("#28", Mergeable::Conflicting, 9_000)
            .unwrap();

        let line = ledger.summary_line(10_000);
        // #28 (age 9000s) is older than #31 (age 2000s): it comes first.
        assert!(
            line.starts_with("2 previously opened PRs still open:"),
            "{line}"
        );
        assert!(
            line.find("#28").unwrap() < line.find("#31").unwrap(),
            "{line}"
        );
        assert!(line.contains("#28 (age 2h 30m 0s, CONFLICTING)"), "{line}");
        assert!(line.contains("#31 (age 33m 20s, MERGEABLE)"), "{line}");
    }

    #[test]
    fn format_age_renders_units_and_omits_leading_zeros() {
        assert_eq!(format_age(0), "0s");
        assert_eq!(format_age(59), "59s");
        assert_eq!(format_age(90), "1m 30s");
        assert_eq!(format_age(9_900), "2h 45m 0s");
        assert_eq!(format_age(93_784), "1d 2h 3m 4s");
    }

    #[test]
    fn branch_build_checks_out_onto_the_base_before_it_resets() {
        // A hard reset rewinds the branch you are standing on: the checkout
        // must come first, so the reset runs while standing on the new
        // branch, never the previous one (#3915).
        let build = BranchBuild::new("conv/issue-3888", "785447cf").unwrap();
        let steps = build.setup_steps();
        assert_eq!(
            steps,
            vec![
                "git checkout -B conv/issue-3888 785447cf".to_string(),
                "git reset --hard 785447cf".to_string(), // linter:allow-SECURITY test fixture: the plan's expected step string, not an executed command
                "git clean -fdx".to_string(),
            ]
        );
        let reset_prefix = "git reset --hard"; // linter:allow-SECURITY test fixture: the plan's expected step prefix, not an executed command
        assert!(steps[0].starts_with("git checkout -B"));
        assert!(steps[1].starts_with(reset_prefix));
        // The base is a sha, not a moving ref: origin/main can move
        // between the fetch and the checkout, a sha cannot.
        assert!(!steps.iter().any(|s| s.contains("origin/")));

        // Both operands must be named.
        assert!(BranchBuild::new("", "sha").is_err());
        assert!(BranchBuild::new("branch", "  ").is_err());
    }

    #[test]
    fn assert_clean_checkout_blocks_staged_dirty_and_untracked() {
        // Empty: hermetic, the switch is allowed.
        assert!(assert_clean_checkout("/scratch/convert/worker-0", &[]).is_ok());
        assert!(assert_clean_checkout("/scratch/convert/worker-0", &["   ".to_string()]).is_ok());

        // Staged (the observed defect): the previous iteration's patch is
        // in the index; the switch would carry it onto the target branch
        // with exit 0.
        let err = assert_clean_checkout(
            "/scratch/convert/worker-0",
            &["M  crates/autospec-core/src/lint.rs".to_string()],
        )
        .unwrap_err();
        assert!(err.contains("/scratch/convert/worker-0"), "{err}");
        assert!(err.contains("staged"), "{err}");
        assert!(err.contains("crates/autospec-core/src/lint.rs"), "{err}");

        // Dirty and untracked block just the same: an untracked file rides
        // the switch the way a staged path does.
        assert!(assert_clean_checkout("/w", &[" M file.rs".to_string()]).is_err());
        assert!(assert_clean_checkout("/w", &["?? untracked.txt".to_string()]).is_err());
    }

    #[test]
    fn assert_branch_matches_patch_passes_on_identical_file_sets() {
        // Order-independent: a patch's file list is not a diff's ordering.
        let declared = vec!["a.rs".to_string(), "b.rs".to_string()];
        let actual = vec!["b.rs".to_string(), "a.rs".to_string()];
        assert!(assert_branch_matches_patch(
            "#3870",
            "785447cf",
            "conv/issue-3870",
            &declared,
            &actual
        )
        .is_ok());
        // Both empty: no net change, which is a classification
        // (NoNetChange), not a contents mismatch.
        assert!(
            assert_branch_matches_patch("#3870", "785447cf", "conv/issue-3870", &[], &[]).is_ok()
        );
    }

    #[test]
    fn assert_branch_matches_patch_hard_fails_on_any_mismatch_and_names_the_operands() {
        let declared = vec!["a.rs".to_string(), "b.rs".to_string()];

        // A superset: the branch carries files the patch never declared.
        let extra = vec![
            "a.rs".to_string(),
            "b.rs".to_string(),
            "images/env-block.sh".to_string(),
        ];
        let err =
            assert_branch_matches_patch("#3888", "785447cf", "conv/issue-3888", &declared, &extra)
                .unwrap_err();
        assert!(err.contains("#3888"), "{err}");
        assert!(err.contains("785447cf"), "{err}");
        assert!(err.contains("conv/issue-3888"), "{err}");
        assert!(err.contains("images/env-block.sh"), "{err}");
        assert!(err.contains("refusing to push"), "{err}");

        // A subset: the branch is missing files the patch declared.
        let missing = vec!["a.rs".to_string()];
        let err = assert_branch_matches_patch(
            "#3888",
            "785447cf",
            "conv/issue-3888",
            &declared,
            &missing,
        )
        .unwrap_err();
        assert!(err.contains("b.rs"), "{err}");

        // A disjoint set: the branch is a different patch entirely.
        let other = vec!["x.rs".to_string(), "y.rs".to_string()];
        assert!(assert_branch_matches_patch(
            "#3888",
            "785447cf",
            "conv/issue-3888",
            &declared,
            &other
        )
        .is_err());
    }

    #[test]
    fn require_commit_fails_ahead_zero_and_names_the_patch() {
        // A build that produced a commit is fine.
        assert!(require_commit("#3870", "785447cf", "conv/issue-3870", 1).is_ok());
        // ahead=0 fails at the builder, with the patch named — the
        // failure cannot surface later as an opaque "No commits between
        // main and <branch>" from the PR API.
        let err = require_commit("#3870", "785447cf", "conv/issue-3870", 0).unwrap_err();
        assert!(err.contains("#3870"), "{err}");
        assert!(err.contains("0 commits"), "{err}");
        assert!(err.contains("build failed"), "{err}");
    }

    #[test]
    fn gate_evidence_is_bound_to_the_tree_it_ran_against() {
        // Recording: an unnamed tree is not evidence.
        assert!(GateEvidence::record("", GateVerdict::Green, "2090 passed").is_err());

        let evidence = GateEvidence::record("263368c7", GateVerdict::Green, "2090 passed").unwrap();
        // The evidence line the PR body carries names the tree.
        let line = evidence
            .clone()
            .publish("conv/issue-3870", "263368c7")
            .unwrap();
        assert!(line.contains("2090 passed"), "{line}");
        assert!(line.contains("263368c7"), "{line}");
        assert!(line.contains("conv/issue-3870"), "{line}");

        // Refuse on mismatch: the gate ran against one tree, the branch
        // under review is a different one (#3915).
        let err = evidence.publish("conv/issue-3870", "785447cf").unwrap_err();
        assert!(err.contains("263368c7"), "{err}");
        assert!(err.contains("785447cf"), "{err}");
        assert!(err.contains("conv/issue-3870"), "{err}");
    }

    #[test]
    fn a_two_patch_run_with_the_second_patch_pre_staged_fails_rather_than_mislabels() {
        // Acceptance (#3793, exercised against the populated case): two
        // verified patches land in one helper loop, and the second patch
        // is staged before the first is committed. The run must fail —
        // and must not open a PR whose title, body, and gate table
        // describe one patch while its contents are the other (#3915).
        let base = "785447cf";
        let files_3870: Vec<String> = (1..=13).map(|i| format!("lint-3870-{i:02}.rs")).collect();
        let files_3888: Vec<String> = vec![
            "images/env-block-a.sh".to_string(),
            "images/env-block-b.sh".to_string(),
        ];

        // The shared checkout: the staged index as porcelain entries, and
        // the branches as their file contents — what
        // `git diff --name-only <base> <branch>` would show after the
        // commit step.
        let mut staged: Vec<String> = Vec::new();
        let mut branches: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut failure: Option<String> = None;

        let patches = [("#3870", &files_3870), ("#3888", &files_3888)];

        'run: for (index, (identity, files)) in patches.iter().enumerate() {
            let branch = format!("conv/issue-{identity}");
            let build = BranchBuild::new(&branch, base).unwrap();

            // Invariant: never switch branches with a staged or dirty
            // index. On iteration two this is where the first
            // iteration's uncommitted state would be caught.
            if let Err(err) = assert_clean_checkout("/scratch/convert/worker-0", &staged) {
                failure = Some(err);
                break;
            }

            // Setup: checkout -B onto the base, then clean — simulated by
            // the state the steps leave behind.
            let _ = build.setup_steps();
            staged.clear();

            // Apply the patch: it lands in the index.
            staged.extend(files.iter().map(|f| format!("M  {f}")));

            // The defect under test (#3793): the second patch is staged
            // before the first is committed.
            if index == 0 {
                staged.extend(files_3888.iter().map(|f| format!("M  {f}")));
            }

            // Commit: everything staged lands on the branch, so the
            // branch now holds both patches' files. A commit happened,
            // so ahead=1; the rewind defect (ahead=0) is covered by
            // require_commit_fails_ahead_zero_and_names_the_patch.
            let committed: Vec<String> = staged
                .iter()
                .map(|line| line.trim_start_matches("M  ").to_string())
                .collect();
            branches.insert(branch.clone(), committed.clone());
            staged.clear();
            require_commit(identity, base, &branch, 1).unwrap();

            // Invariant: contents asserted against the patch's file list
            // before push.
            if let Err(err) = build.pre_push_assert(identity, files, &committed) {
                failure = Some(err);
                break 'run;
            }
        }

        // The run failed instead of mislabeling: patch #3870's branch
        // contains #3888's files, so the contents assert refused the push
        // and named the patch.
        let failure = failure.expect("the pre-staged second patch must fail the run");
        assert!(failure.contains("#3870"), "{failure}");
        assert!(failure.contains("images/env-block-a.sh"), "{failure}");
        let landed = &branches["conv/issue-#3870"];
        assert_eq!(landed.len(), files_3870.len() + files_3888.len());
        assert!(landed.iter().any(|f| f == "images/env-block-a.sh"));
        // The second branch was never built: the loop stopped at the
        // first failure rather than pushing a mislabeled PR.
        assert!(!branches.contains_key("conv/issue-#3888"));
    }

    // ---- #3972: verified publication outcomes ----

    fn green_steps(patch: &str, pr: &str) -> Vec<VerifiedStep> {
        vec![
            VerifiedStep::record(
                PublishStep::Commit,
                0,
                &format!("[conv/issue-{patch}] fix: the change"),
            )
            .unwrap(),
            VerifiedStep::record(
                PublishStep::Push,
                0,
                &format!("To github.com:owner/repo.git  {pr} -> conv/issue-{patch}"),
            )
            .unwrap(),
            VerifiedStep::record(
                PublishStep::PrCreate,
                0,
                &format!("https://github.com/owner/repo/pull/{pr}"),
            )
            .unwrap(),
            VerifiedStep::record(PublishStep::PrMerge, 0, &format!("Merged PR #{pr}")).unwrap(),
        ]
    }

    #[test]
    fn verified_step_refuses_an_empty_capture() {
        // A step's output must be captured, not discarded: an empty
        // capture means a failure has nothing to quote (#3972).
        let err = VerifiedStep::record(PublishStep::PrCreate, 1, "").unwrap_err();
        assert!(err.contains("gh pr create"), "{err}");
        assert!(err.contains("empty"), "{err}");
        assert!(VerifiedStep::record(PublishStep::Push, 0, "   \n").is_err());
        let step = VerifiedStep::record(PublishStep::Commit, 1, "nothing to commit").unwrap();
        assert_eq!(step.step(), PublishStep::Commit);
        assert_eq!(step.exit(), 1);
        assert!(!step.ok());
        assert_eq!(step.output(), "nothing to commit");
    }

    #[test]
    fn a_failed_pr_create_is_held_not_converted_and_the_counter_agrees() {
        // Acceptance, exercised against a populated case (#3793 / #3972):
        // a run where `gh pr create` is forced to fail must report HELD —
        // quoting the tool's own error — and must not increment the
        // converted counter.
        let mut log = PublicationLog::new();

        // Patch A: all four steps verified, the remote confirmed the
        // number, the merge landed.
        let a = green_steps("41", "42");
        let outcome_a =
            decide_publication("#41", &a, Some("#42"), CONVERSION_LOGIC_VERSION).unwrap();
        assert_eq!(
            outcome_a.claim_line("#41"),
            format!("CONVERTED (PR #42 merged): #41 [logic v{CONVERSION_LOGIC_VERSION}]")
        );
        log.record("#41", outcome_a).unwrap();

        // Patch B: the PR opened but the merge was deferred — the PR
        // exists, so this is a converted claim with the merge pending.
        let mut b = green_steps("42", "43");
        b[3] = VerifiedStep::record(
            PublishStep::PrMerge,
            1,
            "PR #43 is not mergeable: status checks are still pending",
        )
        .unwrap();
        let outcome_b =
            decide_publication("#42", &b, Some("#43"), CONVERSION_LOGIC_VERSION).unwrap();
        assert_eq!(
            outcome_b.claim_line("#42"),
            format!(
                "CONVERTED (PR #43 open, merge deferred): #42 [logic v{CONVERSION_LOGIC_VERSION}]"
            )
        );
        log.record("#42", outcome_b).unwrap();

        // Patch C: the defect under test. The old pass piped this to
        // /dev/null and printed CONVERTED anyway.
        let mut c = green_steps("43", "44");
        c[2] = VerifiedStep::record(
            PublishStep::PrCreate,
            1,
            "GraphQL: no commits between main and conv/issue-#43",
        )
        .unwrap();
        // The run stopped at the failure: the merge was never run.
        c.pop();
        let outcome_c = decide_publication("#43", &c, None, CONVERSION_LOGIC_VERSION).unwrap();
        match &outcome_c {
            PublicationOutcome::Held { step, output, .. } => {
                assert_eq!(*step, PublishStep::PrCreate);
                assert_eq!(
                    output,
                    "GraphQL: no commits between main and conv/issue-#43"
                );
            }
            other => panic!("expected Held, got {other:?}"),
        }
        let line_c = outcome_c.claim_line("#43");
        assert!(
            line_c.starts_with(&format!(
                "HELD at gh pr create: #43 [logic v{CONVERSION_LOGIC_VERSION}]:"
            )),
            "{line_c}"
        );
        assert!(
            line_c.contains("GraphQL: no commits between main and conv/issue-#43"),
            "{line_c}"
        );
        log.record("#43", outcome_c).unwrap();

        // The counter is derived from verified outcomes: two converted,
        // one held — the failed PR create did not enter the total.
        assert_eq!(log.converted_count(), 2);
        assert_eq!(log.held_count(), 1);
        assert_eq!(
            log.claim_lines(),
            vec![
                format!("CONVERTED (PR #42 merged): #41 [logic v{CONVERSION_LOGIC_VERSION}]"),
                format!("CONVERTED (PR #43 open, merge deferred): #42 [logic v{CONVERSION_LOGIC_VERSION}]"),
                format!(
                    "HELD at gh pr create: #43 [logic v{CONVERSION_LOGIC_VERSION}]: GraphQL: no commits between main and conv/issue-#43"
                ),
            ]
        );
        // A patch cannot be re-recorded over its first claim.
        let err = log
            .record(
                "#43",
                PublicationOutcome::Held {
                    step: PublishStep::PrCreate,
                    output: "x".into(),
                    logic_version: CONVERSION_LOGIC_VERSION,
                },
            )
            .unwrap_err();
        assert!(err.contains("already recorded"), "{err}");
    }

    #[test]
    fn converted_is_refused_without_a_verified_pr() {
        // A green run that cannot point at a PR number the remote
        // confirmed is not a converted run (#3972).
        let steps = green_steps("41", "42");
        let err = decide_publication("#41", &steps, None, CONVERSION_LOGIC_VERSION).unwrap_err();
        assert!(
            err.contains("without a PR number the remote confirmed"),
            "{err}"
        );
        let err =
            decide_publication("#41", &steps, Some("  "), CONVERSION_LOGIC_VERSION).unwrap_err();
        assert!(
            err.contains("without a PR number the remote confirmed"),
            "{err}"
        );
    }

    #[test]
    fn decide_refuses_missing_out_of_order_and_past_failure_steps() {
        // Outcomes are derived from the steps actually run, in run
        // order: nothing may be skipped, reordered, or reported past a
        // failure (#3972).
        let steps = green_steps("41", "42");
        assert!(decide_publication("#41", &[], Some("#42"), CONVERSION_LOGIC_VERSION).is_err());

        let mut reordered = steps.clone();
        reordered.swap(0, 1);
        let err = decide_publication("#41", &reordered, Some("#42"), CONVERSION_LOGIC_VERSION)
            .unwrap_err();
        assert!(err.contains("run order"), "{err}");

        let mut pushed_after_commit_failure = steps.clone();
        pushed_after_commit_failure[0] =
            VerifiedStep::record(PublishStep::Commit, 1, "nothing to commit").unwrap();
        let err = decide_publication(
            "#41",
            &pushed_after_commit_failure,
            None,
            CONVERSION_LOGIC_VERSION,
        )
        .unwrap_err();
        assert!(
            err.contains("failed but later steps were recorded"),
            "{err}"
        );
    }

    #[test]
    fn a_pass_that_stops_early_without_a_failure_is_refused() {
        // Stopping after a green step is not "merge deferred" — the
        // deferral has to be the recorded result of the merge step
        // itself (#3972).
        let steps = green_steps("41", "42");
        let early = &steps[..3];
        let err =
            decide_publication("#41", early, Some("#42"), CONVERSION_LOGIC_VERSION).unwrap_err();
        assert!(err.contains("stopped after gh pr create"), "{err}");
    }

    #[test]
    fn reconcile_catches_a_converted_claim_with_no_pr() {
        // The end-of-pass assertion: every converted claim must have a
        // PR behind it, under the number the run recorded (#3972).
        let mut log = PublicationLog::new();
        log.record(
            "#41",
            decide_publication(
                "#41",
                &green_steps("41", "42"),
                Some("#42"),
                CONVERSION_LOGIC_VERSION,
            )
            .unwrap(),
        )
        .unwrap();
        let mut deferred = green_steps("42", "43");
        deferred[3] = VerifiedStep::record(PublishStep::PrMerge, 1, "checks pending").unwrap();
        log.record(
            "#42",
            decide_publication("#42", &deferred, Some("#43"), CONVERSION_LOGIC_VERSION).unwrap(),
        )
        .unwrap();

        let ok = reconcile_converted(&log, &[("#41", "#42"), ("#42", "#43")]).unwrap();
        assert!(ok.contains("2 converted claims"), "{ok}");

        // The remote shows no PR for #42: the claim has nothing behind
        // it and the reconciliation names it.
        let err = reconcile_converted(&log, &[("#41", "#42")]).unwrap_err();
        assert!(err.contains("no PR observed"), "{err}");
        assert!(err.contains("#42"), "{err}");

        // The remote shows a different number: the recorded claim and
        // the remote disagree.
        let err = reconcile_converted(&log, &[("#41", "#99"), ("#42", "#43")]).unwrap_err();
        assert!(err.contains("#41: recorded #42, remote shows #99"), "{err}");
    }

    // ---- #3742: the phase-2 count reconciliation ----

    #[test]
    fn phase_reconciliation_passes_when_every_candidate_is_processed() {
        // The normal case: phase 1 hands phase 2 exactly the candidates
        // it produced, and phase 2 works through all of them.
        let ok = reconcile_phase_counts(73, 73).unwrap();
        assert!(
            ok.contains("all 73 candidate(s) from phase 1 processed"),
            "{ok}"
        );
    }

    #[test]
    fn phase_reconciliation_treats_a_zero_candidate_pass_as_complete() {
        // No candidates, no work: phase 1 decided everything cheaply, so
        // phase 2 processed 0 of 0 and the counts reconcile.
        let ok = reconcile_phase_counts(0, 0).unwrap();
        assert!(
            ok.contains("all 0 candidate(s) from phase 1 processed"),
            "{ok}"
        );
    }

    #[test]
    fn phase_reconciliation_fails_loudly_on_a_shortfall() {
        // The bug from #3742: phase 1 produced 73 candidates, the loop
        // processed 1 (a command in the body ate the worklist from
        // stdin), and the pass still exited 0. The gate must refuse:
        // the 72 never reached are dropped, not decided, and the pass
        // must not report success.
        let err = reconcile_phase_counts(73, 1).unwrap_err();
        assert!(err.contains("produced 73 candidate(s)"), "{err}");
        assert!(err.contains("processed only 1"), "{err}");
        assert!(
            err.contains("72 candidate(s) were dropped, not decided"),
            "{err}"
        );
        assert!(err.contains("the pass must not exit 0"), "{err}");
    }

    #[test]
    fn phase_reconciliation_fails_loudly_on_a_surplus() {
        // The mirror case: phase 2 processed more than phase 1 produced.
        // The two numbers do not reconcile, so the pass fails loudly
        // rather than exiting 0 with a summary that does not add up.
        let err = reconcile_phase_counts(9, 11).unwrap_err();
        assert!(err.contains("produced 9 candidate(s)"), "{err}");
        assert!(err.contains("processed 11"), "{err}");
        assert!(
            err.contains("2 candidate(s) processed beyond the worklist"),
            "{err}"
        );
        assert!(err.contains("the pass must not exit 0"), "{err}");
    }

    // ---- #3899: the pass run summary ----

    #[test]
    fn run_summary_reports_open_prs_with_age_alongside_converted_and_held() {
        // Acceptance (#3899, exercised against the populated case):
        // the previous pass opened PR #43 and deferred its merge; this
        // pass converts one patch, holds another, and observes the
        // still-open PR. The run summary must report the open PR with
        // age alongside the converted/held claims.
        let mut ledger = PrLedger::new();
        ledger.record_opened("#43", 1_000).unwrap();

        let mut log = PublicationLog::new();
        // This pass: patch #41 converts, PR #42 merged.
        log.record(
            "#41",
            decide_publication(
                "#41",
                &green_steps("41", "42"),
                Some("#42"),
                CONVERSION_LOGIC_VERSION,
            )
            .unwrap(),
        )
        .unwrap();
        // Patch #44 held at gh pr create: the old pass piped this to
        // /dev/null and printed CONVERTED anyway (#3972).
        let mut held = green_steps("44", "45");
        held[2] = VerifiedStep::record(
            PublishStep::PrCreate,
            1,
            "GraphQL: no commits between main and conv/issue-#44",
        )
        .unwrap();
        held.pop();
        log.record(
            "#44",
            decide_publication("#44", &held, None, CONVERSION_LOGIC_VERSION).unwrap(),
        )
        .unwrap();

        // The pass observed the open PR this pass: still mergeable, so
        // no transition is reported.
        assert_eq!(ledger.observe("#43", Mergeable::Mergeable, 9_000), Ok(None));

        let summary = run_summary(
            &log,
            &ledger,
            &[],
            10_000,
            CONVERSION_LOGIC_VERSION,
            CONVERSION_LOGIC_VERSION,
        );
        assert!(summary.contains("converted: 1, held: 1"), "{summary}");
        assert!(
            summary.contains(&format!(
                "CONVERTED (PR #42 merged): #41 [logic v{CONVERSION_LOGIC_VERSION}]"
            )),
            "{summary}"
        );
        assert!(
            summary.contains(&format!(
                "HELD at gh pr create: #44 [logic v{CONVERSION_LOGIC_VERSION}]:"
            )),
            "{summary}"
        );
        assert!(
            summary.contains("1 previously opened PR still open: #43 (age 2h 30m 0s, MERGEABLE)"),
            "{summary}"
        );
    }

    #[test]
    fn run_summary_surfaces_a_conflicting_transition_in_the_pass_that_observed_it() {
        // Acceptance (#3899): a PR whose mergeability transitioned to
        // CONFLICTING is surfaced in the pass that observes it — the
        // transition line is part of that pass's run summary, and the
        // next pass reports the PR as CONFLICTING without re-reporting
        // the transition.
        let mut ledger = PrLedger::new();
        ledger.record_opened("#43", 1_000).unwrap();
        assert_eq!(ledger.observe("#43", Mergeable::Mergeable, 9_000), Ok(None));

        // This pass observes the transition.
        let report = ledger
            .observe("#43", Mergeable::Conflicting, 10_000)
            .unwrap()
            .expect("the decisive transition must be reported");

        let summary = run_summary(
            &PublicationLog::new(),
            &ledger,
            std::slice::from_ref(&report),
            10_000,
            CONVERSION_LOGIC_VERSION,
            CONVERSION_LOGIC_VERSION,
        );
        assert!(summary.contains(&report), "{summary}");
        assert!(
            summary.contains("went MERGEABLE -> CONFLICTING"),
            "{summary}"
        );
        assert!(
            summary.contains("1 previously opened PR still open: #43 (age 2h 30m 0s, CONFLICTING)"),
            "{summary}"
        );

        // The next pass: a fresh transition list — the alarm is
        // surfaced once, in the pass that observed it; the line item
        // carries the state and the age keeps growing.
        let next = run_summary(
            &PublicationLog::new(),
            &ledger,
            &[],
            20_000,
            CONVERSION_LOGIC_VERSION,
            CONVERSION_LOGIC_VERSION,
        );
        assert!(!next.contains("mergeability transition"), "{next}");
        assert!(
            next.contains("1 previously opened PR still open: #43 (age 5h 16m 40s, CONFLICTING)"),
            "{next}"
        );
    }

    #[test]
    fn run_summary_prints_the_line_item_on_an_idle_pass() {
        // Acceptance (#3899): the line item appears in *every* run
        // summary — a pass that converted and held nothing still
        // reports the previously opened PRs still open, with age. The
        // block is deterministic, so it is stable across runs and
        // diff-able; pin the exact shape.
        let mut ledger = PrLedger::new();
        ledger.record_opened("#43", 1_000).unwrap();

        let summary = run_summary(
            &PublicationLog::new(),
            &ledger,
            &[],
            10_000,
            CONVERSION_LOGIC_VERSION,
            CONVERSION_LOGIC_VERSION,
        );
        assert_eq!(
            summary,
            "converted: 0, held: 0\n1 previously opened PR still open: #43 (age 2h 30m 0s, MERGEABLE)"
        );

        // An empty pass with an empty ledger prints the same shape
        // with the fallback line: the summary is printed, not omitted.
        let summary = run_summary(
            &PublicationLog::new(),
            &PrLedger::new(),
            &[],
            10_000,
            CONVERSION_LOGIC_VERSION,
            CONVERSION_LOGIC_VERSION,
        );
        assert_eq!(
            summary,
            "converted: 0, held: 0\nno previously opened PRs still open"
        );
    }

    #[test]
    fn memo_results_are_attributable_to_their_logic_version() {
        // Acceptance (#4041): a result is attributable to the logic that
        // produced it. Two versions deciding the same (identity, base sha)
        // pair leave two distinct, version-tagged records; the lookup the
        // pass honors is scoped to the version it is running under.
        let mut memo = ConversionMemo::new();
        memo.record("p1", "base-1", ConversionClass::MemoizedHold, 0)
            .unwrap();
        memo.record(
            "p1",
            "base-1",
            ConversionClass::ExistingPr,
            CONVERSION_LOGIC_VERSION,
        )
        .unwrap();

        // The records carry their versions, newest first.
        let records = memo.records("p1", "base-1");
        assert_eq!(
            records,
            vec![
                MemoRecord {
                    class: ConversionClass::ExistingPr,
                    logic_version: CONVERSION_LOGIC_VERSION,
                },
                MemoRecord {
                    class: ConversionClass::MemoizedHold,
                    logic_version: 0,
                },
            ]
        );

        // A pass running under the current logic sees its own decision.
        assert_eq!(
            memo.lookup("p1", "base-1", CONVERSION_LOGIC_VERSION),
            Some(ConversionClass::ExistingPr)
        );
        // The same pair queried under the superseded version sees the
        // old decision — the two versions' results are distinguishable.
        assert_eq!(
            memo.lookup("p1", "base-1", 0),
            Some(ConversionClass::MemoizedHold)
        );
        // A version that never decided the pair has no record.
        assert_eq!(memo.lookup("p1", "base-1", 7), None);
    }

    #[test]
    fn a_superseded_verdict_is_flagged_and_rejected_as_a_memo_hit() {
        // Acceptance (#4041): a result produced by a superseded version of
        // the logic stays queryable but is not honored as a memo hit — it
        // requires re-verification. The plan flags it and the report
        // names both versions (the #3866 gate pattern).
        let mut memo = ConversionMemo::new();
        memo.record("hold-patch", "263368c7", ConversionClass::MemoizedHold, 0)
            .unwrap();

        let patches = vec![patch(
            "hold-patch",
            "263368c7",
            ConversionClass::MemoizedHold,
        )];

        // The pass is running the current logic: the recorded decision was
        // made by an older one. Not a memo hit, flagged for re-verification.
        let schedule = plan_pass(
            &patches,
            1,
            "/root",
            &memo,
            "263368c7",
            CONVERSION_LOGIC_VERSION,
        )
        .unwrap();
        assert!(!schedule.assignments[0].memo_hit);
        assert!(schedule.assignments[0].superseded_verdict);
        let report =
            superseded_verdict_report(&memo, &patches[0], CONVERSION_LOGIC_VERSION).unwrap();
        assert!(report.contains("logic v0"), "{report}");
        assert!(
            report.contains(&format!("running v{CONVERSION_LOGIC_VERSION}")),
            "{report}"
        );

        // The same decision re-recorded under the current logic is live:
        // a memo hit, nothing superseded.
        memo.record(
            "hold-patch",
            "263368c7",
            ConversionClass::MemoizedHold,
            CONVERSION_LOGIC_VERSION,
        )
        .unwrap();
        let schedule = plan_pass(
            &patches,
            1,
            "/root",
            &memo,
            "263368c7",
            CONVERSION_LOGIC_VERSION,
        )
        .unwrap();
        assert!(schedule.assignments[0].memo_hit);
        assert!(!schedule.assignments[0].superseded_verdict);
        assert!(superseded_verdict_report(&memo, &patches[0], CONVERSION_LOGIC_VERSION).is_none());
    }

    #[test]
    fn the_run_summary_reports_a_mid_run_logic_change() {
        // Acceptance (#4041): a pass that detects its own logic changed
        // between the reading at start and the reading at finish reports
        // it in the summary — with both operands (#3866).
        let alarm = logic_change_report(0, CONVERSION_LOGIC_VERSION).unwrap();
        assert!(alarm.contains("v0"), "{alarm}");
        assert!(
            alarm.contains(&format!("v{CONVERSION_LOGIC_VERSION}")),
            "{alarm}"
        );

        let summary = run_summary(
            &PublicationLog::new(),
            &PrLedger::new(),
            &[],
            10_000,
            0,
            CONVERSION_LOGIC_VERSION,
        );
        let lines: Vec<&str> = summary.lines().collect();
        // The alarm sits between the count line and the line item: it is
        // part of the summary, printed with it, not after it.
        assert_eq!(lines[0], "converted: 0, held: 0");
        assert_eq!(lines[1], alarm);

        // Same logic at both readings: no alarm, the summary is unchanged
        // in shape.
        let summary = run_summary(
            &PublicationLog::new(),
            &PrLedger::new(),
            &[],
            10_000,
            CONVERSION_LOGIC_VERSION,
            CONVERSION_LOGIC_VERSION,
        );
        assert!(logic_change_report(CONVERSION_LOGIC_VERSION, CONVERSION_LOGIC_VERSION).is_none());
        assert_eq!(summary.lines().count(), 2);
    }

    // #3674: the inert-queue trap — verdicts, retirement, inert fraction.

    #[test]
    fn the_verdict_maps_every_decided_class_and_leaves_undecided_open() {
        assert_eq!(
            ConversionClass::ExistingPr.verdict(),
            Some(PatchVerdict::Converted)
        );
        assert_eq!(
            ConversionClass::ClosedIssue.verdict(),
            Some(PatchVerdict::Resolved)
        );
        assert_eq!(
            ConversionClass::NoNetChange.verdict(),
            Some(PatchVerdict::Resolved)
        );
        assert_eq!(
            ConversionClass::MemoizedHold.verdict(),
            Some(PatchVerdict::Held)
        );
        // The pass has not decided either of these: no verdict, and
        // "not yet decided" must not be reported as "done". A
        // CouldNotEvaluate is a state of the setup, not of the patch
        // (#3866), re-evaluated on every pass.
        assert_eq!(ConversionClass::Candidate.verdict(), None);
        assert_eq!(ConversionClass::CouldNotEvaluate.verdict(), None);
    }

    #[test]
    fn only_converted_and_resolved_verdicts_are_done() {
        assert!(PatchVerdict::Converted.is_done());
        assert!(PatchVerdict::Resolved.is_done());
        // The trap #3674 closes: a held patch exists, and it is not done.
        assert!(!PatchVerdict::Held.is_done());
    }

    #[test]
    fn the_patch_is_done_check_references_the_verdict_not_the_existence() {
        // The patch exists in every case here; only the verdict differs.
        assert!(patch("a", "s1", ConversionClass::ExistingPr).is_done());
        assert!(patch("b", "s2", ConversionClass::NoNetChange).is_done());
        // The held patch exists in the queue, and the gate must not
        // answer "already produced": the issue is still open.
        assert!(!patch("c", "s3", ConversionClass::MemoizedHold).is_done());
        // A candidate exists too, and the pass has not decided.
        assert!(!patch("d", "s4", ConversionClass::Candidate).is_done());
        assert!(!patch("e", "s5", ConversionClass::CouldNotEvaluate).is_done());
    }

    #[test]
    fn queue_health_counts_the_inert_fraction_in_one_loop() {
        let patches = vec![
            patch("a", "s1", ConversionClass::MemoizedHold),
            patch("b", "s2", ConversionClass::ExistingPr),
            patch("c", "s3", ConversionClass::MemoizedHold),
            patch("d", "s4", ConversionClass::Candidate),
            patch("e", "s5", ConversionClass::CouldNotEvaluate),
            patch("f", "s6", ConversionClass::MemoizedHold),
        ];

        let health = queue_health(&patches);
        assert_eq!(health.queued, 6);
        assert_eq!(health.held, 3);
        assert!((health.inert_fraction() - 0.5).abs() < f64::EPSILON);
        assert_eq!(health.report(), "6 queued, 3 held");
    }

    #[test]
    fn an_empty_queue_is_not_inert() {
        let health = queue_health(&[]);
        assert_eq!(health.queued, 0);
        assert_eq!(health.held, 0);
        assert_eq!(health.inert_fraction(), 0.0);
        assert_eq!(health.report(), "0 queued, 0 held");
    }

    #[test]
    fn retirable_holds_rerun_plausibly_differ() {
        // A deterministic rule the agent has learned since: a re-run
        // differs. A genuine build failure the agent reproduces: it does
        // not. An agent-reported unformatted hold (#3715): a fresh agent
        // formats its submission, so it differs. An agent-reported
        // unbuilt hold: the failure is a property of the submission, so
        // it does not. A stale-base hold (#4279): the trunk moved past
        // the patch's base, so a re-dispatch at the new base differs.
        assert!(HoldReason::RuleFixed.rerun_plausibly_differs());
        assert!(!HoldReason::BuildFailure.rerun_plausibly_differs());
        assert!(HoldReason::AgentReportedUnformatted.rerun_plausibly_differs());
        assert!(!HoldReason::AgentReportedUnbuilt.rerun_plausibly_differs());
        assert!(HoldReason::StaleBase.rerun_plausibly_differs());
    }

    #[test]
    fn retirement_is_planned_for_rule_fixed_holds_only() {
        let held = patch("issue-41", "base-1", ConversionClass::MemoizedHold);
        let converted = patch("issue-42", "base-2", ConversionClass::ExistingPr);

        let plan = plan_retirement(&held, HoldReason::RuleFixed, "/scratch/out").unwrap();
        // The destination is outside the active patch glob: once moved,
        // the patch no longer counts as "already produced".
        assert_eq!(plan.destination, "/scratch/out/out-retired/issue-41");
        assert_eq!(plan.patch, "issue-41");
        // The reason travels with the patch: a repeat hold is visible.
        assert_eq!(plan.reason, HoldReason::RuleFixed);

        // An agent-reported unformatted hold is retired like a
        // rule-fixed one: a fresh agent fixes the formatting.
        let plan =
            plan_retirement(&held, HoldReason::AgentReportedUnformatted, "/scratch/out").unwrap();
        assert_eq!(plan.reason, HoldReason::AgentReportedUnformatted);

        // A stale-base hold (#4279) is retired like a rule-fixed one:
        // the drift resolves when the patch is rebased onto the new tip.
        let plan = plan_retirement(&held, HoldReason::StaleBase, "/scratch/out").unwrap();
        assert_eq!(plan.reason, HoldReason::StaleBase);

        // A build-failure hold is not retired: a re-run would reproduce
        // it, so retiring discards paid-for work for nothing.
        let error = plan_retirement(&held, HoldReason::BuildFailure, "/scratch/out").unwrap_err();
        assert!(error.contains("BuildFailure"));
        // An agent-reported unbuilt hold is not retired either: the
        // agent already ran the build and it failed — a re-run reproduces
        // it.
        let error =
            plan_retirement(&held, HoldReason::AgentReportedUnbuilt, "/scratch/out").unwrap_err();
        assert!(error.contains("AgentReportedUnbuilt"));
        // A patch the pass did not hold is not retired.
        let error = plan_retirement(&converted, HoldReason::RuleFixed, "/scratch/out").unwrap_err();
        assert!(error.contains("only held"));
        // An empty root is a configuration error.
        assert!(plan_retirement(&held, HoldReason::RuleFixed, "  ").is_err());
    }

    #[test]
    fn classify_build_failure_attributes_stale_base() {
        // No drift: the base is the tip, so the failure is the patch's
        // own regardless of what the base build did.
        assert_eq!(classify_build_failure(0, false), HoldReason::BuildFailure);
        assert_eq!(classify_build_failure(0, true), HoldReason::BuildFailure);

        // Drift, base compiles: the trunk moved underneath the patch.
        assert_eq!(classify_build_failure(71, true), HoldReason::StaleBase);
        assert_eq!(classify_build_failure(1, true), HoldReason::StaleBase);

        // Drift, base also fails: the failure is the patch's own, not
        // the drift's.
        assert_eq!(classify_build_failure(71, false), HoldReason::BuildFailure);
        assert_eq!(classify_build_failure(1, false), HoldReason::BuildFailure);
    }

    #[test]
    fn drift_clause_formats_the_outcome_line() {
        // No drift: no clause.
        assert_eq!(drift_clause(0, "8f3a21c"), "");

        // Singular.
        assert_eq!(
            drift_clause(1, "8f3a21c"),
            "(patch is 1 commit behind base 8f3a21c)"
        );

        // Plural.
        assert_eq!(
            drift_clause(71, "8f3a21c"),
            "(patch is 71 commits behind base 8f3a21c)"
        );
    }

    #[test]
    fn retirement_invalidates_only_the_running_logic_version() {
        let mut memo = ConversionMemo::new();
        // A superseded version's hold is still on file (audit trail),
        // and the running version holds the same pair.
        memo.record("issue-41", "base-1", ConversionClass::MemoizedHold, 1)
            .unwrap();
        memo.record(
            "issue-41",
            "base-1",
            ConversionClass::MemoizedHold,
            CONVERSION_LOGIC_VERSION,
        )
        .unwrap();
        assert_eq!(
            memo.lookup("issue-41", "base-1", CONVERSION_LOGIC_VERSION),
            Some(ConversionClass::MemoizedHold)
        );

        // Without invalidation the next pass re-holds from the record,
        // for the same since-fixed reason.
        assert!(memo.invalidate("issue-41", "base-1", CONVERSION_LOGIC_VERSION));
        assert_eq!(
            memo.lookup("issue-41", "base-1", CONVERSION_LOGIC_VERSION),
            None
        );
        // The superseded version's record survives: the audit trail
        // outlives the decision.
        assert_eq!(memo.records("issue-41", "base-1").len(), 1);

        // A different (identity, base sha) pair is untouched.
        memo.record("issue-42", "base-2", ConversionClass::MemoizedHold, 1)
            .unwrap();
        assert!(memo.invalidate("issue-42", "base-2", 1));
        // Invalidating twice, or a never-recorded pair, removes nothing.
        assert!(!memo.invalidate("issue-42", "base-2", 1));
        assert!(!memo.invalidate("never-recorded", "base-3", 1));
    }

    #[test]
    fn the_logic_version_bumped_when_the_verdict_logic_landed() {
        // #3674 added new decision logic (the re-dispatch verdict and
        // retirement planning): results recorded under version 1 are
        // hypotheses the version-2 logic re-verifies, not decisions.
        // #3715 added the agent-reported hold reasons to the retirement
        // decision: results recorded under version 2 are hypotheses the
        // version-3 logic re-verifies, not decisions.
        // #3783 added the agent-status dimension to the ordering key and
        // the baseline-relative key diagnostic: results recorded under
        // version 3 are hypotheses the version-4 logic re-verifies, not
        // decisions.
        // #4279 added the stale-base hold reason (a build failure on a
        // drifted patch that compiles cleanly at its base is retirable,
        // not a genuine defect): results recorded under version 4 are
        // hypotheses the version-5 logic re-verifies, not decisions.
        assert_eq!(CONVERSION_LOGIC_VERSION, 5);
    }

    // ---- #3793: base self-check and dry-run ----

    fn gate_evidence(gate: &str, completed: bool, findings: &[&str]) -> GateBaseEvidence {
        GateBaseEvidence {
            gate: gate.to_string(),
            completed,
            findings: findings.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// base → nothing: every gate completed and reported nothing against
    /// the unmodified base, so the self-check is clean and the pass may
    /// proceed to real work.
    #[test]
    fn base_self_check_is_clean_when_every_gate_reports_nothing() {
        let check = base_gate_self_check(&[
            gate_evidence("build", true, &[]),
            gate_evidence("validate", true, &[]),
            gate_evidence("acceptance", true, &[]),
        ])
        .unwrap();
        assert_eq!(check, BaseSelfCheck::Clean);
        assert!(check.is_clean());
        // A passed self-check reports nothing, mirroring the gates it
        // checks.
        assert_eq!(check.line(), None);
    }

    /// known-bad → fail: a gate that reports something against the
    /// unmodified base is broken by definition; the self-check names the
    /// gate and the finding so the operator fixes the gate, not the work.
    #[test]
    fn base_self_check_flags_a_gate_that_reports_on_the_base() {
        let check = base_gate_self_check(&[
            gate_evidence("build", true, &[]),
            gate_evidence(
                "validate",
                true,
                &["check_x: failed (baseline had no such failure)"],
            ),
        ])
        .unwrap();
        assert_eq!(
            check,
            BaseSelfCheck::FlagsBase {
                gate: "validate".to_string(),
                finding: "check_x: failed (baseline had no such failure)".to_string(),
            }
        );
        assert!(!check.is_clean());
        let line = check.line().unwrap();
        assert!(line.contains("validate"));
        assert!(line.contains("check_x"));
    }

    /// A finding beats a missing verdict regardless of gate order: a
    /// named defect is reported rather than a vague one.
    #[test]
    fn base_self_check_prefers_a_finding_over_a_missing_verdict() {
        let check = base_gate_self_check(&[
            gate_evidence("build", false, &[]),
            gate_evidence("acceptance", true, &["unexpected hold"]),
        ])
        .unwrap();
        assert_eq!(
            check,
            BaseSelfCheck::FlagsBase {
                gate: "acceptance".to_string(),
                finding: "unexpected hold".to_string(),
            }
        );
    }

    /// Incomplete is not clean: a gate that did not run to completion
    /// cannot be read as "nothing reported". Unknown is not safe; the
    /// pass holds and names the gate (#3768 pattern — the timed-out gate
    /// whose silence was read as "all fixed").
    #[test]
    fn base_self_check_incomplete_gate_is_no_verdict_not_clean() {
        let check = base_gate_self_check(&[
            gate_evidence("build", true, &[]),
            gate_evidence("acceptance", false, &[]),
        ])
        .unwrap();
        assert_eq!(
            check,
            BaseSelfCheck::NoVerdict {
                gate: "acceptance".to_string(),
            }
        );
        assert!(!check.is_clean());
        assert!(check.line().unwrap().contains("acceptance"));
    }

    /// An empty gate set is an error, not a silent pass: a self-check
    /// that checked nothing proves nothing.
    #[test]
    fn base_self_check_refuses_an_empty_gate_set() {
        let err = base_gate_self_check(&[]).unwrap_err();
        assert!(err.contains("empty gate set"));
    }

    /// The dry run prints every decision the pass would make — order,
    /// class, memo hit, stale base, superseded verdict, worker — and
    /// nothing it would do: the report is a pure render of the plan.
    #[test]
    fn dry_run_report_renders_every_decision_of_the_pass() {
        let mut memo = ConversionMemo::new();
        // A memo record for the existing-PR patch at the tip, under the
        // running logic version: the plan flags it as a memo hit.
        memo.record(
            "pr-patch",
            "tip-1",
            ConversionClass::ExistingPr,
            CONVERSION_LOGIC_VERSION,
        )
        .unwrap();

        let patches = vec![
            patch("pr-patch", "tip-1", ConversionClass::ExistingPr),
            patch("hold-stale", "old-1", ConversionClass::MemoizedHold),
            // A patch whose newest recorded decision is a superseded
            // logic version: the plan must flag it for re-verification.
            patch("hold-superseded", "tip-1", ConversionClass::MemoizedHold),
        ];
        memo.record("hold-superseded", "tip-1", ConversionClass::MemoizedHold, 1)
            .unwrap();

        let schedule = plan_pass(
            &patches,
            2,
            "/scratch/convert",
            &memo,
            "tip-1",
            CONVERSION_LOGIC_VERSION,
        )
        .unwrap();
        let report = dry_run_report(&schedule);
        let mut lines = report.lines();
        // Header: the pool and the queue sizes, decided by the pass.
        assert_eq!(lines.next().unwrap(), "dry-run: 2 worker(s), 3 patch(es)");
        // Cost order (ExistingPr before MemoizedHold), round-robin over
        // the two workers; every decision flag the pass computes is
        // printed, so CI can assert on the plan without running it.
        assert_eq!(
            lines.next().unwrap(),
            "worker 0: pr-patch class=ExistingPr memo_hit=true stale_base=false superseded_verdict=false"
        );
        assert_eq!(
            lines.next().unwrap(),
            "worker 1: hold-stale class=MemoizedHold memo_hit=false stale_base=true superseded_verdict=false"
        );
        // The superseded patch carries its flag in the dry run: the
        // memo record exists but under a superseded logic version, so it
        // is not a memo hit and must be re-verified.
        assert_eq!(
            lines.next().unwrap(),
            "worker 0: hold-superseded class=MemoizedHold memo_hit=false stale_base=false superseded_verdict=true"
        );
        assert!(lines.next().is_none());
    }

    /// An empty queue renders as nothing-to-convert, not as an empty
    /// report: a pass with no patches says so, instead of printing
    /// nothing and looking like it never ran.
    #[test]
    fn dry_run_report_says_nothing_to_convert_for_an_empty_pass() {
        let memo = ConversionMemo::new();
        let schedule = plan_pass(
            &[],
            2,
            "/scratch/convert",
            &memo,
            "tip-1",
            CONVERSION_LOGIC_VERSION,
        )
        .unwrap();
        assert_eq!(dry_run_report(&schedule), "dry-run: nothing to convert");
    }
}
