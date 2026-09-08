//! Patch conversion pass policy (#3635).
//!
//! The step that turns a finished agent patch into a pull request used to be
//! the bottleneck of the pipeline: one exclusive lock on a single git
//! worktree, one patch at a time (checkout, apply, compile-test, commit,
//! push), while a growing queue of finished patches waited and the GPU hours
//! already spent producing them sat idle.
//!
//! The fix is five rules, each encoded here as a pure, testable primitive.
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
//! 5. **The run's own `status.txt` triages a patch before anything is
//!    applied** ([`triage`]): the converter's job is to supply the judgment
//!    the agent could not make — a comparison against current `main` — not
//!    to re-derive judgments the agent already made and recorded. A
//!    `TIMEOUT` is unfinished work to re-dispatch with a larger budget, a
//!    recorded `fmt_rc=1` is a fact about the patch that needs no re-run,
//!    and only an all-green or unjudgeable record costs a local gate.

use std::collections::BTreeMap;

/// A conversion outcome class, ordered cheapest first.
///
/// The four terminal classes cost seconds (no compile): the pass already
/// knows the answer. [`ConversionClass::Candidate`] costs a build.
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
    /// A genuine candidate: apply, compile-test, commit, push.
    Candidate,
}

impl ConversionClass {
    /// Every class, cheapest first.
    pub const ALL: [Self; 5] = [
        Self::ExistingPr,
        Self::ClosedIssue,
        Self::MemoizedHold,
        Self::NoNetChange,
        Self::Candidate,
    ];

    /// Terminal classes are decided without a compile and are therefore
    /// memoizable.
    pub fn is_terminal(self) -> bool {
        !matches!(self, Self::Candidate)
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
}

impl Patch {
    /// Construct a patch, rejecting empty identity or base sha: an empty key
    /// would collapse unrelated patches into one memo entry.
    pub fn new(
        identity: impl Into<String>,
        base_sha: impl Into<String>,
        class: ConversionClass,
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
        })
    }
}

/// Order a pass by cost, not by name: every terminal (cheap) patch before
/// every candidate (expensive) patch, with the original relative order kept
/// within each class. The pass still walks the set alphabetically; that
/// order only breaks ties inside a cost class. The sort is stable, so two
/// passes over the same queue agree on the order of same-class patches.
pub fn order_by_cost(patches: &[Patch]) -> Vec<&Patch> {
    let mut ordered: Vec<&Patch> = patches.iter().collect();
    ordered.sort_by_key(|patch| patch.class);
    ordered
}

/// Memoized terminal conversion decisions.
///
/// Keyed by (patch identity, base sha): patch identity plus base sha
/// already determines the outcome, so a re-walk of a memoized patch is a
/// map lookup, not a re-walk of the whole set.
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

    /// Record a terminal decision. Non-terminal classes are rejected: a
    /// [`ConversionClass::Candidate`] outcome is work still to do, not a
    /// decision worth memoizing.
    pub fn record(
        &mut self,
        identity: &str,
        base_sha: &str,
        class: ConversionClass,
    ) -> Result<(), String> {
        if !class.is_terminal() {
            return Err(format!(
                "refusing to memoize non-terminal class {:?}",
                class
            ));
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
    /// decision for this (identity, base sha) pair; the worker applies the
    /// recorded decision without a compile.
    pub memo_hit: bool,
}

/// The full plan for one pass over the queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversionSchedule<'a> {
    /// The worker pool (empty exactly when there is no work).
    pub workers: Vec<WorkerPlan>,
    /// Patches in execution order with their worker assignment.
    pub assignments: Vec<ScheduledPatch<'a>>,
}

/// Plan one pass: order the queue by cost, size the pool, assign
/// round-robin, and flag memo hits. An empty queue plans an empty pool —
/// there is nothing to convert, and zero workers is the only sane pool for
/// zero work.
pub fn plan_pass<'a>(
    patches: &'a [Patch],
    workers: usize,
    root: &str,
    memo: &ConversionMemo,
) -> Result<ConversionSchedule<'a>, String> {
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
            memo_hit: memo
                .lookup(&patch.identity, &patch.base_sha)
                .is_some_and(|class| class == patch.class),
        })
        .collect();
    Ok(ConversionSchedule {
        workers: pool,
        assignments,
    })
}

// ── Recorded run status triage (#3715) ─────────────────────────────────────

/// The status token a run records in its `status.txt`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunStatus {
    /// The run finished within its budget (`status=OK`).
    Ok,
    /// The run was cut off by the time limit (`status=TIMEOUT`).
    Timeout,
    /// The run was cut off by the time limit and produced no output at all
    /// (`status=TIMEOUT-NO-OUTPUT`).
    TimeoutNoOutput,
    /// The agent could not compare against main
    /// (`status=UNKNOWN-NO-BASELINE`).
    UnknownNoBaseline,
    /// A token this pass does not recognise; the raw token is preserved so a
    /// HELD line can surface it verbatim.
    Unrecognised(String),
}

impl RunStatus {
    /// The token exactly as it appears in `status.txt`.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Ok => "OK",
            Self::Timeout => "TIMEOUT",
            Self::TimeoutNoOutput => "TIMEOUT-NO-OUTPUT",
            Self::UnknownNoBaseline => "UNKNOWN-NO-BASELINE",
            Self::Unrecognised(token) => token,
        }
    }

    fn from_token(token: &str) -> Self {
        match token {
            "OK" => Self::Ok,
            "TIMEOUT" => Self::Timeout,
            "TIMEOUT-NO-OUTPUT" => Self::TimeoutNoOutput,
            "UNKNOWN-NO-BASELINE" => Self::UnknownNoBaseline,
            other => Self::Unrecognised(other.to_string()),
        }
    }
}

/// What a finished run recorded about itself: the `status.txt` that sits
/// next to the patch, the file count it points at, and the failing tests it
/// named. The converter reads this before applying anything and lets it
/// decide what to do with the patch ([`triage`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentRunStatus {
    /// The `status=` token; `None` when the run recorded no status line.
    pub status: Option<RunStatus>,
    /// `build_rc=` exit code of the agent's build.
    pub build_rc: i32,
    /// `test_rc=` exit code of the agent's test run.
    pub test_rc: i32,
    /// `fmt_rc=` exit code of the agent's format check.
    pub fmt_rc: i32,
    /// Entry count of `fmt-files.txt`; 0 when the agent recorded none.
    pub fmt_files: usize,
    /// Test names the agent recorded in `failing-tests.txt`.
    pub failing_tests: Vec<String>,
}

impl AgentRunStatus {
    /// Parse the key=value text of a run's `status.txt`.
    ///
    /// Recognised lines: `status=<token>`, `build_rc=<n>`, `test_rc=<n>`,
    /// `fmt_rc=<n>`, and `fmt-files.txt: <n> entries`. Anything else is
    /// ignored: the agent may grow this file, and the converter must keep
    /// working. Unparseable values are treated as unrecorded.
    pub fn parse_status_text(text: &str) -> Self {
        let mut parsed = Self::default();
        for line in text.lines() {
            let line = line.trim();
            if let Some(value) = line.strip_prefix("status=") {
                let token = value.trim();
                if !token.is_empty() {
                    parsed.status = Some(RunStatus::from_token(token));
                }
            } else if let Some(value) = line.strip_prefix("build_rc=") {
                parsed.build_rc = value.trim().parse().unwrap_or(0);
            } else if let Some(value) = line.strip_prefix("test_rc=") {
                parsed.test_rc = value.trim().parse().unwrap_or(0);
            } else if let Some(value) = line.strip_prefix("fmt_rc=") {
                parsed.fmt_rc = value.trim().parse().unwrap_or(0);
            } else if let Some(value) = line.strip_prefix("fmt-files.txt:") {
                parsed.fmt_files = value
                    .split_whitespace()
                    .next()
                    .and_then(|count| count.parse().ok())
                    .unwrap_or(0);
            }
        }
        parsed
    }
}

/// Parse the contents of `failing-tests.txt`: one test name per line; blank
/// lines and `#` comments are ignored.
pub fn parse_failing_tests(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_string)
        .collect()
}

/// Why a patch must be gated locally instead of trusted from the record or
/// held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateLocalReason {
    /// No `status.txt` was recorded; the converter is the only judge.
    NoRecord,
    /// `status=UNKNOWN-NO-BASELINE`: the agent could not compare against
    /// main — exactly the case the converter exists for.
    UnknownNoBaseline,
    /// The recorded build failed: a negative about an old base must be
    /// re-checked against current main.
    BuildFailed,
    /// `test_rc != 0` but `failing-tests.txt` is empty: the negative is
    /// incomplete, so the converter discovers the failures itself.
    FailuresUnrecorded,
    /// All green on the agent's base: still gate locally, because a
    /// recorded pass is a fact about the agent's base, which has usually
    /// moved.
    AllGreen,
}

impl GateLocalReason {
    /// One-line reason for the pass log explaining why the patch is gated
    /// locally.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoRecord => "no status.txt recorded by the agent",
            Self::UnknownNoBaseline => {
                "status=UNKNOWN-NO-BASELINE: the agent could not compare against main"
            }
            Self::BuildFailed => "agent recorded a failed build; re-check against current main",
            Self::FailuresUnrecorded => {
                "agent recorded failing tests without naming them; discover locally"
            }
            Self::AllGreen => "all green on the agent's base; confirm against current main",
        }
    }
}

/// The action the conversion pass takes on a held patch, decided from the
/// run's own recorded status **before** anything is applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Triage {
    /// `status=TIMEOUT` or `status=TIMEOUT-NO-OUTPUT`: the patch is
    /// unfinished work, not a bad patch. Re-dispatch with a larger budget;
    /// do not gate.
    Redispatch {
        /// The recorded status that cut the run off.
        status: RunStatus,
    },
    /// `fmt_rc != 0`: hold as `AGENT-REPORTED-UNFORMATTED`; no local run
    /// needed.
    HoldUnformatted { fmt_rc: i32, fmt_files: usize },
    /// `test_rc != 0` with a populated `failing-tests.txt`: hold naming the
    /// agent's own failures; re-verify only if the base has moved.
    HoldFailingTests {
        test_rc: i32,
        failing_tests: Vec<String>,
    },
    /// Gate locally to supply the judgment the agent could not make — a
    /// comparison against current main.
    GateLocal { reason: GateLocalReason },
}

impl Triage {
    /// True when the pass must apply the patch and run the local gate.
    pub fn gates_locally(&self) -> bool {
        matches!(self, Self::GateLocal { .. })
    }

    /// The `HELD:` line for holds and re-dispatches: the agent's own
    /// recorded facts in its own words, with the recorded status surfaced.
    /// `HELD: agent reported fmt_rc=1 (32 files), status=TIMEOUT` tells a
    /// reader to re-dispatch; `HELD: cargo fmt --check reports 32
    /// unformatted files` invites them to wonder whether the agent is
    /// broken. `None` for gate-local triages — the patch is not held, so the
    /// pass logs [`GateLocalReason::as_str`] instead.
    pub fn held_line(&self, recorded: &AgentRunStatus) -> Option<String> {
        let fact = match self {
            Self::Redispatch { .. } => fmt_fact(recorded.fmt_rc, recorded.fmt_files),
            Self::HoldUnformatted { fmt_rc, fmt_files } => fmt_fact(*fmt_rc, *fmt_files),
            Self::HoldFailingTests {
                test_rc,
                failing_tests,
            } => Some(format!(
                "test_rc={test_rc} (failing: {})",
                failing_tests.join(", ")
            )),
            Self::GateLocal { .. } => return None,
        };
        let mut line = "HELD: agent reported".to_string();
        if let Some(fact) = fact {
            line.push(' ');
            line.push_str(&fact);
        }
        if let Some(status) = &recorded.status {
            line.push_str(&format!(", status={}", status.as_str()));
        }
        Some(line)
    }
}

fn fmt_fact(fmt_rc: i32, fmt_files: usize) -> Option<String> {
    if fmt_rc == 0 {
        return None;
    }
    if fmt_files > 0 {
        Some(format!("fmt_rc={fmt_rc} ({fmt_files} files)"))
    } else {
        Some(format!("fmt_rc={fmt_rc}"))
    }
}

/// Triage a held patch from the run's own recorded status, before applying
/// anything.
///
/// The pass trusts the agent's negative results — a recorded `fmt_rc=1` is a
/// fact about the patch and needs no re-run, and a recorded test failure is
/// held naming the agent's own failures — and verifies its positive
/// results: a recorded pass is a fact about the agent's base, which has
/// usually moved, so it still needs local confirmation against current
/// main.
///
/// `recorded` is `None` when the run left no `status.txt`; that is the case
/// the converter exists for, so it gates locally.
pub fn triage(recorded: Option<&AgentRunStatus>) -> Triage {
    let Some(recorded) = recorded else {
        return Triage::GateLocal {
            reason: GateLocalReason::NoRecord,
        };
    };
    match &recorded.status {
        // Unfinished work, not a bad patch: re-dispatch with a larger
        // budget, do not gate.
        Some(status) if matches!(status, RunStatus::Timeout | RunStatus::TimeoutNoOutput) => {
            return Triage::Redispatch {
                status: status.clone(),
            };
        }
        // The agent could not compare against main: none of its recorded
        // judgments are trustworthy, so gate locally regardless of the rc
        // fields.
        Some(RunStatus::UnknownNoBaseline) => {
            return Triage::GateLocal {
                reason: GateLocalReason::UnknownNoBaseline,
            };
        }
        _ => {}
    }
    if recorded.fmt_rc != 0 {
        return Triage::HoldUnformatted {
            fmt_rc: recorded.fmt_rc,
            fmt_files: recorded.fmt_files,
        };
    }
    if recorded.test_rc != 0 {
        if recorded.failing_tests.is_empty() {
            return Triage::GateLocal {
                reason: GateLocalReason::FailuresUnrecorded,
            };
        }
        return Triage::HoldFailingTests {
            test_rc: recorded.test_rc,
            failing_tests: recorded.failing_tests.clone(),
        };
    }
    if recorded.build_rc != 0 {
        return Triage::GateLocal {
            reason: GateLocalReason::BuildFailed,
        };
    }
    Triage::GateLocal {
        reason: GateLocalReason::AllGreen,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patch(identity: &str, base_sha: &str, class: ConversionClass) -> Patch {
        Patch::new(identity, base_sha, class).unwrap()
    }

    #[test]
    fn order_by_cost_puts_terminal_classes_before_candidates() {
        let patches = vec![
            patch("a", "sha-a", ConversionClass::Candidate),
            patch("b", "sha-b", ConversionClass::ExistingPr),
            patch("c", "sha-c", ConversionClass::Candidate),
            patch("d", "sha-d", ConversionClass::NoNetChange),
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
    fn order_by_cost_keeps_input_order_within_a_class_and_is_stable() {
        // The queue arrives alphabetical; within a cost class that order is
        // the tie-break, and two passes over the same queue must agree.
        let patches = vec![
            patch("alpha", "s1", ConversionClass::ClosedIssue),
            patch("beta", "s2", ConversionClass::MemoizedHold),
            patch("gamma", "s3", ConversionClass::ClosedIssue),
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
    fn patch_rejects_empty_identity_and_base_sha() {
        assert!(Patch::new("", "sha", ConversionClass::Candidate).is_err());
        assert!(Patch::new("id", "  ", ConversionClass::Candidate).is_err());
        assert!(Patch::new("id", "sha", ConversionClass::Candidate).is_ok());
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

        let schedule = plan_pass(&patches, 2, "/scratch/convert", &memo).unwrap();

        assert_eq!(schedule.workers.len(), 2);
        let order: Vec<_> = schedule
            .assignments
            .iter()
            .map(|a| (a.patch.identity.as_str(), a.worker, a.memo_hit))
            .collect();
        // Terminal classes first (in class order), then candidates;
        // round-robin over the cost order; only the memoized hold is a hit.
        assert_eq!(
            order,
            vec![
                ("pr-patch", 0, false),
                ("hold-patch", 1, true),
                ("candidate-1", 0, false),
                ("candidate-2", 1, false),
            ]
        );
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
        let schedule = plan_pass(&patches, 1, "/root", &memo).unwrap();

        assert_eq!(schedule.assignments[0].memo_hit, false);
    }

    #[test]
    fn plan_pass_with_no_patches_plans_no_workers_and_propagates_pool_errors() {
        let schedule = plan_pass(&[], 4, "/root", &ConversionMemo::new()).unwrap();
        assert!(schedule.workers.is_empty());
        assert!(schedule.assignments.is_empty());

        let patches = vec![patch("p1", "base-1", ConversionClass::Candidate)];
        assert!(plan_pass(&patches, 0, "/root", &ConversionMemo::new()).is_err());
        assert!(plan_pass(&patches, 1, "", &ConversionMemo::new()).is_err());
    }

    // ── Recorded run status triage (#3715) ──

    /// The `status.txt` the issue records from the first observed patch.
    const TIMEOUT_STATUS_TXT: &str = "\
        status=TIMEOUT
        build_rc=0
        test_rc=101
        fmt_rc=1
        fmt-files.txt: 32 entries
        ";

    fn recorded(text: &str, failing: &[&str]) -> AgentRunStatus {
        let mut status = AgentRunStatus::parse_status_text(text);
        status.failing_tests = failing.iter().map(|t| t.to_string()).collect();
        status
    }

    #[test]
    fn parse_status_text_reads_all_recorded_fields() {
        let parsed = AgentRunStatus::parse_status_text(TIMEOUT_STATUS_TXT);
        assert_eq!(parsed.status, Some(RunStatus::Timeout));
        assert_eq!(parsed.build_rc, 0);
        assert_eq!(parsed.test_rc, 101);
        assert_eq!(parsed.fmt_rc, 1);
        assert_eq!(parsed.fmt_files, 32);
        assert!(parsed.failing_tests.is_empty());
    }

    #[test]
    fn parse_status_text_ignores_unknown_lines_and_bad_values() {
        let parsed = AgentRunStatus::parse_status_text(
            "notes=whatever\nfmt_rc=not-a-number\nstatus=\nbuild_rc=0\n",
        );
        assert_eq!(parsed.status, None);
        assert_eq!(parsed.build_rc, 0);
        assert_eq!(parsed.test_rc, 0);
        assert_eq!(parsed.fmt_rc, 0);
        assert_eq!(parsed.fmt_files, 0);
    }

    #[test]
    fn parse_status_text_keeps_unrecognised_status_tokens() {
        let parsed = AgentRunStatus::parse_status_text("status=PARTIAL\n");
        assert_eq!(
            parsed.status,
            Some(RunStatus::Unrecognised("PARTIAL".to_string()))
        );
        if let Some(RunStatus::Unrecognised(token)) = &parsed.status {
            assert_eq!(token.as_str(), "PARTIAL");
        }
    }

    #[test]
    fn parse_failing_tests_drops_blank_lines_and_comments() {
        let tests = parse_failing_tests("# failed\na::first\n\n  b::second  \n");
        assert_eq!(tests, vec!["a::first", "b::second"]);
        assert!(parse_failing_tests("").is_empty());
    }

    #[test]
    fn triage_timeout_is_redispatch_not_gate() {
        // The first observed patch: the run was cut off before it could
        // format, and the timeout wins over the recorded fmt failure.
        let rec = recorded(TIMEOUT_STATUS_TXT, &[]);
        let decision = triage(Some(&rec));
        assert_eq!(
            decision,
            Triage::Redispatch {
                status: RunStatus::Timeout
            }
        );
        assert!(!decision.gates_locally());
    }

    #[test]
    fn triage_timeout_no_output_is_redispatch_not_gate() {
        let rec = recorded("status=TIMEOUT-NO-OUTPUT\n", &[]);
        assert_eq!(
            triage(Some(&rec)),
            Triage::Redispatch {
                status: RunStatus::TimeoutNoOutput
            }
        );
    }

    #[test]
    fn triage_recorded_fmt_failure_holds_unformatted() {
        let rec = recorded("status=OK\nfmt_rc=1\nfmt-files.txt: 32 entries\n", &[]);
        assert_eq!(
            triage(Some(&rec)),
            Triage::HoldUnformatted {
                fmt_rc: 1,
                fmt_files: 32
            }
        );
    }

    #[test]
    fn triage_recorded_test_failures_hold_naming_them() {
        let rec = recorded(
            "status=OK\ntest_rc=101\n",
            &["alpha::keeps_order", "beta::survives_reload"],
        );
        assert_eq!(
            triage(Some(&rec)),
            Triage::HoldFailingTests {
                test_rc: 101,
                failing_tests: vec![
                    "alpha::keeps_order".to_string(),
                    "beta::survives_reload".to_string()
                ]
            }
        );
    }

    #[test]
    fn triage_test_failure_without_names_gates_locally() {
        let rec = recorded("status=OK\ntest_rc=101\n", &[]);
        assert_eq!(
            triage(Some(&rec)),
            Triage::GateLocal {
                reason: GateLocalReason::FailuresUnrecorded
            }
        );
    }

    #[test]
    fn triage_unknown_no_baseline_gates_locally_regardless_of_rc() {
        // The agent could not compare against main, so even a recorded fmt
        // failure is not trusted: the converter is exactly what this case
        // exists for.
        let rec = recorded(
            "status=UNKNOWN-NO-BASELINE\nfmt_rc=1\nfmt-files.txt: 3 entries\n",
            &[],
        );
        assert_eq!(
            triage(Some(&rec)),
            Triage::GateLocal {
                reason: GateLocalReason::UnknownNoBaseline
            }
        );
    }

    #[test]
    fn triage_recorded_build_failure_gates_locally() {
        let rec = recorded("status=OK\nbuild_rc=101\n", &[]);
        assert_eq!(
            triage(Some(&rec)),
            Triage::GateLocal {
                reason: GateLocalReason::BuildFailed
            }
        );
    }

    #[test]
    fn triage_all_green_gates_locally_to_confirm() {
        let rec = recorded("status=OK\nbuild_rc=0\ntest_rc=0\nfmt_rc=0\n", &[]);
        let decision = triage(Some(&rec));
        assert_eq!(
            decision,
            Triage::GateLocal {
                reason: GateLocalReason::AllGreen
            }
        );
        assert!(decision.gates_locally());
    }

    #[test]
    fn triage_missing_status_gates_locally() {
        assert_eq!(
            triage(None),
            Triage::GateLocal {
                reason: GateLocalReason::NoRecord
            }
        );
    }

    #[test]
    fn held_line_surfaces_the_agents_own_facts() {
        // The first observed patch: the line must read the agent's record,
        // not a local re-run's verdict.
        let rec = recorded(TIMEOUT_STATUS_TXT, &[]);
        let decision = triage(Some(&rec));
        assert_eq!(
            decision.held_line(&rec).as_deref(),
            Some("HELD: agent reported fmt_rc=1 (32 files), status=TIMEOUT")
        );
    }

    #[test]
    fn held_line_names_the_agents_own_failures() {
        let rec = recorded(
            "status=OK\ntest_rc=101\n",
            &["alpha::keeps_order", "beta::survives_reload"],
        );
        let decision = triage(Some(&rec));
        assert_eq!(
            decision.held_line(&rec).as_deref(),
            Some(
                "HELD: agent reported test_rc=101 (failing: alpha::keeps_order, \
                 beta::survives_reload), status=OK"
            )
        );
    }

    #[test]
    fn held_line_omits_file_count_when_unrecorded() {
        let rec = recorded("status=OK\nfmt_rc=1\n", &[]);
        let decision = triage(Some(&rec));
        assert_eq!(
            decision.held_line(&rec).as_deref(),
            Some("HELD: agent reported fmt_rc=1, status=OK")
        );
    }

    #[test]
    fn held_line_is_none_for_gate_local_triages() {
        let rec = recorded("status=OK\n", &[]);
        let decision = triage(Some(&rec));
        assert!(decision.held_line(&rec).is_none());
        assert_eq!(
            GateLocalReason::AllGreen.as_str(),
            "all green on the agent's base; confirm against current main"
        );
    }
}
