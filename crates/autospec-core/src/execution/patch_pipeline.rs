//! Patch conversion pass policy (#3635).
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
}
