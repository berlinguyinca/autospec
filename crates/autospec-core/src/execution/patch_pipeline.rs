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
//!    candidates.
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
//! 8. **Impact first, then newest.** Within each cost class the patch that
//!    unblocks the most downstream issues goes first ([`order_by_cost`]): it
//!    is the highest-leverage work and converting it promptly unblocks the
//!    most dependents (#3799). Ties on impact fall back to newest-first:
//!    the most recently produced patch sits on the youngest base and is the
//!    one most likely to apply.
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
//!    what was on disk at the end of the run, not the start (#3801).
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

use std::collections::BTreeMap;

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
}

/// Order a pass by cost, then by downstream impact, then by recency: every
/// terminal (cheap) patch before every candidate (expensive) patch, and
/// within each cost class the highest-impact patch first (most downstream
/// issues unblocked), with recency as tiebreak. A patch that unblocks many
/// issues is the highest-leverage work: converting it promptly unblocks the
/// most downstream dependents (#3799). Patches tied on impact fall back to
/// newest-first, which keeps the previous behaviour when no impact data is
/// present (all unblocks = 0). The sort is stable, so patches tied on all
/// keys keep their queue order and two passes over the same queue agree.
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
        .then(b.unblocks.cmp(&a.unblocks))
        .then(b.produced_at.cmp(&a.produced_at))
}

/// Produce a one-line ordering summary for the top candidates in the pass.
///
/// Shows the leading `top_n` patches with their unblock counts so an inert
/// or degenerate impact key is visible in the log rather than only
/// discoverable by measuring it. Also reports the number of distinct
/// unblocks values across all candidates, mirroring the "recency key has N
/// distinct values" diagnostic that caught the degenerate-mtime bug (#3791).
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
        .map(|p| format!("{}(unblocks:{})", p.identity, p.unblocks))
        .collect();
    format!(
        "phase 1 ordered: {} first, then impact desc, then newest first — {} leads {} candidates (impact key has {} distinct values)",
        match ordered[0].class {
            ConversionClass::ExistingPr => "ExistingPr",
            ConversionClass::ClosedIssue => "ClosedIssue",
            ConversionClass::MemoizedHold => "MemoizedHold",
            ConversionClass::NoNetChange => "NoNetChange",
            ConversionClass::CouldNotEvaluate => "COULD-NOT-EVALUATE",
            ConversionClass::Candidate => "Candidate",
        },
        top.join(", "),
        ordered.len(),
        distinct_unblocks,
    )
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
    deferred: Vec<Patch>,
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

    /// The patches explicitly named as deferred to the next run.
    pub fn deferred(&self) -> &[Patch] {
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
    /// converting it in this one. A patch that is still in the remaining
    /// queue is going to be converted in this run, and the same patch may
    /// only be deferred once: both are bookkeeping errors.
    pub fn defer(&mut self, patch: Patch) -> Result<(), String> {
        if self.remaining.iter().any(|p| p.identity == patch.identity) {
            return Err(format!(
                "refusing to defer {}: already in the remaining worklist",
                patch.identity
            ));
        }
        if self.deferred.iter().any(|p| p.identity == patch.identity) {
            return Err(format!(
                "refusing to defer {}: already named as deferred",
                patch.identity
            ));
        }
        self.deferred.push(patch);
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

    /// The freeze window and the moving counts as one log line, e.g. `worklist
    /// frozen at 1022; 2 patches arrived since freeze; 84 considered, 51
    /// remaining, 2 deferred to the next run [iw-49, iw-50]`.
    pub fn summary_line(&self) -> String {
        let s = self.summary();
        let mut line = format!(
            "worklist frozen at {}; {} patches arrived since freeze; {} considered, {} remaining, {} deferred to the next run",
            s.frozen_at, s.arrived, s.considered, s.remaining, s.deferred
        );
        if !self.deferred.is_empty() {
            let names: Vec<&str> = self.deferred.iter().map(|p| p.identity.as_str()).collect();
            line.push_str(&format!(" [{}]", names.join(", ")));
        }
        line
    }
}

/// Memoized conversion decisions.
///
/// Keyed by (patch identity, base sha): patch identity plus base sha
/// already determines the outcome, so a re-walk of a memoized patch is a
/// map lookup, not a re-walk of the whole set. Only decisions *about the
/// patch* are stored: [`ConversionClass::CouldNotEvaluate`] is a state of
/// the setup and is refused by [`ConversionMemo::record`], so the pass
/// re-evaluates it on every run until the setup succeeds (#3866).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConversionMemo {
    entries: BTreeMap<(String, String), ConversionClass>,
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
    /// pair, if any. A different base sha is a different patch as far as the
    /// memo is concerned.
    pub fn lookup(&self, identity: &str, base_sha: &str) -> Option<ConversionClass> {
        self.entries
            .get(&(identity.to_string(), base_sha.to_string()))
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
        self.entries
            .insert((identity.to_string(), base_sha.to_string()), class);
        Ok(())
    }
}

/// A worker in the conversion pool: one private checkout, one shared
/// compile cache.
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
    /// a decision, and is never reported as a hit.
    pub memo_hit: bool,
    /// True when the patch's base is no longer the trunk tip: the worker
    /// must fetch `origin/main` and re-test before acting, because every
    /// decision computed against the old base has decayed (#3698).
    pub stale_base: bool,
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
/// until the setup succeeds (#3866).
pub fn plan_pass<'a>(
    patches: &'a [Patch],
    workers: usize,
    root: &str,
    memo: &ConversionMemo,
    current_tip: &str,
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
                    .lookup(&patch.identity, &patch.base_sha)
                    .is_some_and(|class| class == patch.class),
            stale_base: patch.base_sha != current_tip,
        })
        .collect();
    Ok(ConversionSchedule {
        workers: pool,
        assignments,
    })
}

/// The memo-hit gate's failure report. The gate asserts two equalities —
/// the patch's base sha equals the trunk tip, and the recorded decision
/// for `(identity, base sha)` equals the class the walk landed on. On
/// failure the report names both operands of the equality that failed; a
/// gate that cannot print the values it compared did not perform the
/// comparison, and must not claim the property failed (#3866). `None` when
/// the gate passed, or when nothing is recorded for the key (not yet
/// evaluated, not failed).
pub fn memo_gate_report(memo: &ConversionMemo, patch: &Patch, current_tip: &str) -> Option<String> {
    if patch.base_sha != current_tip {
        return patch.stale_report(current_tip);
    }
    match memo.lookup(&patch.identity, &patch.base_sha) {
        None => None,
        Some(recorded) if recorded == patch.class => None,
        Some(recorded) => Some(format!(
            "memo gate: recorded {:?} != walked {:?} for {}@{}",
            recorded, patch.class, patch.identity, patch.base_sha
        )),
    }
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

        assert!(summary.contains("#46(unblocks:79)"), "got: {summary}");
        assert!(summary.contains("#47(unblocks:3)"), "got: {summary}");
        assert!(summary.contains("#99(unblocks:0)"), "got: {summary}");
        assert!(summary.contains("3 candidates"), "got: {summary}");
        assert!(
            summary.contains("impact key has 3 distinct values"),
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
            summary.contains("impact key has 1 distinct values"),
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
    fn plan_workers_rejects_zero_and_empty_root() {
        assert!(plan_workers(0, "/root").is_err());
        assert!(plan_workers(2, "  ").is_err());
    }

    #[test]
    fn memo_key_requires_both_identity_and_base_sha() {
        let mut memo = ConversionMemo::new();
        memo.record("p1", "base-1", ConversionClass::ExistingPr)
            .unwrap();

        assert_eq!(
            memo.lookup("p1", "base-1"),
            Some(ConversionClass::ExistingPr)
        );
        // Same patch rebased onto a new trunk: a different key.
        assert_eq!(memo.lookup("p1", "base-2"), None);
        // Same trunk, different patch: a different key.
        assert_eq!(memo.lookup("p2", "base-1"), None);
    }

    #[test]
    fn memo_rejects_non_terminal_and_empty_keys() {
        let mut memo = ConversionMemo::new();
        let error = memo
            .record("p1", "base-1", ConversionClass::Candidate)
            .unwrap_err();
        assert!(error.contains("non-terminal"));
        assert!(memo
            .record("", "base", ConversionClass::ExistingPr)
            .is_err());
        assert!(memo
            .record("id", "  ", ConversionClass::ExistingPr)
            .is_err());
        assert!(memo.is_empty());
    }

    #[test]
    fn plan_pass_orders_assigns_and_flags_memo_hits() {
        let mut memo = ConversionMemo::new();
        memo.record("hold-patch", "base-h", ConversionClass::MemoizedHold)
            .unwrap();

        let patches = vec![
            patch("candidate-1", "base-a", ConversionClass::Candidate),
            patch("hold-patch", "base-h", ConversionClass::MemoizedHold),
            patch("candidate-2", "base-b", ConversionClass::Candidate),
            patch("pr-patch", "base-c", ConversionClass::ExistingPr),
        ];

        let schedule = plan_pass(&patches, 2, "/scratch/convert", &memo, "base-h").unwrap();

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
        memo.record("hold-patch", "263368c7", ConversionClass::MemoizedHold)
            .unwrap();

        let patches = vec![patch(
            "hold-patch",
            "263368c7",
            ConversionClass::MemoizedHold,
        )];

        // main moved on; the verdict is stale.
        let schedule = plan_pass(&patches, 1, "/root", &memo, "785447cf").unwrap();
        assert_eq!(schedule.assignments[0].memo_hit, false);
        assert!(schedule.assignments[0].stale_base);

        // Same verdict, base still the tip: honored.
        let schedule = plan_pass(&patches, 1, "/root", &memo, "263368c7").unwrap();
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

        let schedule = plan_pass(&patches, 2, "/root", &ConversionMemo::new(), "base-b").unwrap();
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
        memo.record("p1", "base-1", ConversionClass::NoNetChange)
            .unwrap();

        let patches = vec![patch("p1", "base-1", ConversionClass::Candidate)];
        let schedule = plan_pass(&patches, 1, "/root", &memo, "base-1").unwrap();

        assert_eq!(schedule.assignments[0].memo_hit, false);
    }

    #[test]
    fn plan_pass_with_no_patches_plans_no_workers_and_propagates_pool_errors() {
        let schedule = plan_pass(&[], 4, "/root", &ConversionMemo::new(), "base-1").unwrap();
        assert!(schedule.workers.is_empty());
        assert!(schedule.assignments.is_empty());

        let patches = vec![patch("p1", "base-1", ConversionClass::Candidate)];
        assert!(plan_pass(&patches, 0, "/root", &ConversionMemo::new(), "base-1").is_err());
        assert!(plan_pass(&patches, 1, "", &ConversionMemo::new(), "base-1").is_err());
    }

    #[test]
    fn plan_pass_rejects_an_empty_tip() {
        // The tip must come from a fresh fetch of origin/main; an empty
        // value means the re-baseline step was skipped, and a pass planned
        // without one would act on a snapshot.
        let patches = vec![patch("p1", "base-1", ConversionClass::Candidate)];
        let error = plan_pass(&patches, 1, "/root", &ConversionMemo::new(), "  ").unwrap_err();
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

        // A patch the run will not convert is named, not unrepresented.
        wl.defer(patch_at("e", "s", ConversionClass::Candidate, 310))
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

        let line = wl.summary_line();
        assert!(line.starts_with("worklist frozen at 1000; "), "{}", line);
        assert!(line.contains("1 patches arrived since freeze"), "{}", line);
        assert!(
            line.contains("2 considered, 2 remaining, 1 deferred to the next run [e]"),
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
            .defer(patch_at("queued", "s", ConversionClass::Candidate, 1))
            .unwrap_err();
        assert!(err.contains("already in the remaining worklist"), "{}", err);

        wl.defer(patch_at("later", "s", ConversionClass::Candidate, 2))
            .unwrap();
        let err = wl
            .defer(patch_at("later", "s", ConversionClass::Candidate, 2))
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
            .record("p1", "base-1", ConversionClass::CouldNotEvaluate)
            .unwrap_err();
        assert!(err.contains("CouldNotEvaluate"), "{err}");
        assert!(err.contains("p1"), "{err}");
        assert!(err.contains("base-1"), "{err}");
        assert!(memo.is_empty());
        // A held patch is a decision about the patch and stays memoizable.
        memo.record("p1", "base-1", ConversionClass::MemoizedHold)
            .unwrap();
        assert_eq!(
            memo.lookup("p1", "base-1"),
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
        let schedule = plan_pass(&queue1, 1, "/root", &memo, "base-1").unwrap();
        assert!(!schedule.assignments[0].memo_hit);
        assert!(memo
            .record("p1", "base-1", ConversionClass::CouldNotEvaluate)
            .is_err());

        // Run two: the setup succeeds and the walk lands on HELD.
        blocked.class = ConversionClass::MemoizedHold;
        let queue2 = [blocked.clone()];
        let schedule = plan_pass(&queue2, 1, "/root", &memo, "base-1").unwrap();
        assert!(!schedule.assignments[0].memo_hit);
        memo.record("p1", "base-1", ConversionClass::MemoizedHold)
            .unwrap();

        // Run three: now the decision is about the patch and memoizable.
        let queue3 = [blocked];
        let schedule = plan_pass(&queue3, 1, "/root", &memo, "base-1").unwrap();
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
        let report = memo_gate_report(&ConversionMemo::new(), &patch, "785447cf").unwrap();
        assert!(report.contains("263368c7"), "{report}");
        assert!(report.contains("785447cf"), "{report}");

        // Nothing recorded for the key: not yet evaluated, not failed.
        assert_eq!(
            memo_gate_report(&ConversionMemo::new(), &patch, "263368c7"),
            None
        );

        // A recorded decision different from the walk: the class equality
        // failed and the report names both operands.
        let mut memo = ConversionMemo::new();
        memo.record("p1", "263368c7", ConversionClass::NoNetChange)
            .unwrap();
        let report = memo_gate_report(&memo, &patch, "263368c7").unwrap();
        assert!(report.contains("NoNetChange"), "{report}");
        assert!(report.contains("MemoizedHold"), "{report}");
        assert!(report.contains("p1"), "{report}");
        assert!(report.contains("263368c7"), "{report}");

        // Both equalities hold: the gate passed.
        memo.record("p1", "263368c7", ConversionClass::MemoizedHold)
            .unwrap();
        assert_eq!(memo_gate_report(&memo, &patch, "263368c7"), None);
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
}
