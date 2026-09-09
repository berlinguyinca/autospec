//! Dispatch base policy (#3731).
//!
//! The runner used to build each job's workspace by copying a shared
//! snapshot — a real git clone of `origin` that nothing ever refreshed.
//! Every patch was therefore produced against a base that silently fell
//! behind `main`: "which commit was this patch written against" became a
//! question answered by reading a directory, and the answer was always the
//! day the snapshot was taken. A patch born against a frozen base is born
//! conflicted.
//!
//! The durable fix is the producer-side mirror of the converter's
//! re-baseline rule (#3698, [`crate::execution::patch_pipeline`]): a job
//! establishes its own base — fetch `origin/main` and check out an explicit
//! sha — instead of inheriting a mutable shared directory. The base becomes
//! a **recorded input of the run**, not an ambient property of the
//! filesystem.
//!
//! The rules are encoded as pure, testable primitives; callers perform the
//! git I/O and hand the results to these functions:
//!
//! 1. **The base is a recorded input, taken from a fresh fetch**
//!    ([`Dispatch::from_fetch`]). A dispatch that has not fetched
//!    `origin/main` has no base and cannot run: an empty tip is refused, so
//!    a snapshot taken at startup must never reach the job.
//! 2. **The base must be reachable from `origin/main` at dispatch time**
//!    ([`base_reachable_from_mainline`]). A patch written against a commit
//!    on the mainline integrates by a forward rebase; a patch written
//!    against a divergent commit is the one that is "born conflicted."
//!    [`mainline_reachable`] turns git's parent links into the mainline a
//!    base must lie on.
//! 3. **Two dispatches hours apart yield two different bases** (a property
//!    of rule 1): the base tracks the fetch, so an intervening merge moves
//!    the next dispatch's base with it. The earlier dispatch is flagged
//!    [`Dispatch::is_stale`].
//! 4. **Each run gets its own private base, never a shared one**
//!    ([`plan_base_pool`], [`BaseLeasePool`]). Two concurrent jobs reading
//!    the same mutable working tree is the original bug: the pool hands out
//!    unique base dirs and the lease registry refuses to hand one to two
//!    runs.

use std::collections::{BTreeMap, BTreeSet};

/// One dispatch: the base a single job establishes for itself.
///
/// The base is a recorded input of the run ([`Dispatch::base_sha`]), taken
/// from a fresh fetch of `origin/main` at dispatch time — not an ambient
/// property of a shared directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dispatch {
    /// Stable identifier for the run/job. Must be non-empty.
    pub run_id: String,
    /// The explicit sha the job checked out as its base. Must be non-empty;
    /// it is the recorded answer to "which commit is this patch written
    /// against," and it is part of every downstream decision about that.
    pub base_sha: String,
    /// Monotonic fetch stamp (e.g. unix seconds); higher is newer. Records
    /// *when* the base was fetched, which is what lets two dispatches hours
    /// apart be told apart.
    pub fetched_at: u64,
    /// Private base checkout for this run. Unique per run: two concurrent
    /// jobs must never read the same mutable working tree.
    pub base_dir: String,
}

impl Dispatch {
    /// Establish a dispatch's base from a **fresh fetch** of `origin/main`
    /// (#3731). The base is the tip the fetch returned: the job checks out
    /// an explicit sha rather than inheriting a shared directory, so the
    /// base is a recorded input of the run, not an ambient property of the
    /// filesystem.
    ///
    /// An empty `origin_main_tip` means the fetch was skipped. A dispatch
    /// with no fetched base has no base at all and is refused, mirroring
    /// the converter's refusal to plan a pass without a re-baselined tip
    /// ([`crate::execution::patch_pipeline::plan_pass`]).
    pub fn from_fetch(
        run_id: impl Into<String>,
        origin_main_tip: impl Into<String>,
        fetched_at: u64,
        base_dir: impl Into<String>,
    ) -> Result<Self, String> {
        let run_id = run_id.into();
        let origin_main_tip = origin_main_tip.into();
        let base_dir = base_dir.into();
        if run_id.trim().is_empty() {
            return Err("dispatch run id must not be empty".to_string());
        }
        if origin_main_tip.trim().is_empty() {
            return Err(
                "origin/main tip must not be empty: the dispatch must fetch origin/main \
                 before establishing a base"
                    .to_string(),
            );
        }
        if base_dir.trim().is_empty() {
            return Err("base checkout path must not be empty".to_string());
        }
        Ok(Self {
            run_id,
            base_sha: origin_main_tip,
            fetched_at,
            base_dir,
        })
    }

    /// True while the dispatch's base is still the trunk tip: a base taken
    /// at this fetch has not been left behind.
    pub fn is_fresh(&self, current_tip: &str) -> bool {
        self.base_sha == current_tip
    }

    /// True when the trunk has advanced past the dispatch's base: work
    /// produced against `base_sha` is now born against a base that has moved
    /// on, so a successor must re-baseline against the current tip before
    /// converting.
    pub fn is_stale(&self, current_tip: &str) -> bool {
        self.base_sha != current_tip
    }
}

/// A private base checkout allocated for one run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivateBase {
    /// Position in the pool (also the lease key).
    pub index: usize,
    /// Private checkout for this base. Unique per base, so no two runs read
    /// the same mutable working tree.
    pub path: String,
}

/// Plan `count` **private** base checkouts under `root`: base `i` lives at
/// `{root}/base-{i}`. Every path is unique — that uniqueness is what keeps
/// two concurrent jobs from reading the same mutable working tree, the
/// original bug (#3731). Zero bases is a configuration error, not an empty
/// pool: it would leave a job with no base of its own and push it back onto
/// the shared snapshot.
pub fn plan_base_pool(count: usize, root: &str) -> Result<Vec<PrivateBase>, String> {
    if count == 0 {
        return Err("base pool needs at least one base".to_string());
    }
    if root.trim().is_empty() {
        return Err("base pool root must not be empty".to_string());
    }
    Ok((0..count)
        .map(|index| PrivateBase {
            index,
            path: format!("{root}/base-{index}"),
        })
        .collect())
}

/// Compute the set of commits reachable from `root` by following `parents`
/// (#3731).
///
/// The caller obtains the raw parent links from git (`git rev-list
/// --parents origin/main`); the returned closure is the mainline a base
/// must lie on. `root` is included; a commit absent from `parents` is a
/// leaf with no further ancestors. The `seen` set guards against a
/// malformed graph looping forever.
pub fn mainline_reachable(root: &str, parents: &BTreeMap<String, Vec<String>>) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    let mut stack = vec![root.to_string()];
    while let Some(commit) = stack.pop() {
        if !seen.insert(commit.clone()) {
            continue;
        }
        let Some(parents_of) = parents.get(&commit) else {
            continue;
        };
        for parent in parents_of {
            if !seen.contains(parent) {
                stack.push(parent.clone());
            }
        }
    }
    seen
}

/// Whether `base_sha` is **reachable from `origin/main`** at dispatch time
/// (#3731): the base lies on the mainline the dispatch fetched.
///
/// `mainline` is the set of commits reachable from `origin/main` — e.g.
/// [`mainline_reachable`] over the fetched parent links. A base on that set
/// is a legitimate mainline commit, so a patch written against it integrates
/// by a forward rebase. A base *off* the set (a divergent commit, a feature
/// branch never merged, a rebased-away sha) is exactly the base a patch is
/// "born conflicted" against, and is refused. An empty base is never
/// reachable: a run with no recorded base has nothing to check.
pub fn base_reachable_from_mainline(base_sha: &str, mainline: &BTreeSet<String>) -> bool {
    !base_sha.trim().is_empty() && mainline.contains(base_sha)
}

/// A lease registry over a pool of private base checkouts: a base may be
/// held by at most one running job.
///
/// The original bug was a *shared* base read by every job at once. This
/// registry makes that structurally impossible: a base dir is either free or
/// held by exactly one run, and a second lease on a held base is refused.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BaseLeasePool {
    bases: Vec<PrivateBase>,
    holders: BTreeMap<usize, String>,
}

impl BaseLeasePool {
    /// Wrap a planned base pool in a fresh (all-free) lease registry.
    pub fn new(bases: Vec<PrivateBase>) -> Self {
        Self {
            bases,
            holders: BTreeMap::new(),
        }
    }

    /// Lease base `index` to `run_id`, returning the private checkout path.
    ///
    /// Fails when the base is already held — by another run, which would
    /// share one mutable working tree between two concurrent jobs (the bug
    /// this module exists to prevent, #3731), or by the same run, which
    /// would double-lease a base it already holds.
    pub fn lease(&mut self, index: usize, run_id: impl Into<String>) -> Result<String, String> {
        let run_id = run_id.into();
        if run_id.trim().is_empty() {
            return Err("lease run id must not be empty".to_string());
        }
        let path = self
            .bases
            .get(index)
            .map(|base| base.path.clone())
            .ok_or_else(|| format!("no base at index {index}"))?;
        if let Some(held_by) = self.holders.get(&index) {
            if held_by == &run_id {
                return Err(format!(
                    "run {run_id} already holds base {index}; release before re-leasing"
                ));
            }
            return Err(format!(
                "base {index} is already held by run {held_by}; a base must not be shared"
            ));
        }
        self.holders.insert(index, run_id);
        Ok(path)
    }

    /// Release every base held by `run_id`, returning how many were freed.
    pub fn release(&mut self, run_id: &str) -> usize {
        let before = self.holders.len();
        self.holders.retain(|_, holder| holder != run_id);
        before - self.holders.len()
    }

    /// The run currently holding base `index`, if any.
    pub fn holder(&self, index: usize) -> Option<&str> {
        self.holders.get(&index).map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mainline `c` (tip) <- `b` <- `a`, plus a divergent feature commit
    /// `d` that branched off `b` and was never merged.
    fn parent_map() -> BTreeMap<String, Vec<String>> {
        let mut parents = BTreeMap::new();
        parents.insert("c".to_string(), vec!["b".to_string()]);
        parents.insert("b".to_string(), vec!["a".to_string()]);
        parents.insert("a".to_string(), vec![]);
        parents.insert("d".to_string(), vec!["b".to_string()]);
        parents
    }

    // --- Rule 1: the base is a recorded input from a fresh fetch ----------

    #[test]
    fn from_fetch_records_the_fetched_tip_as_the_base() {
        let d = Dispatch::from_fetch("r1", "f939464", 0, "/b/r1").unwrap();
        assert_eq!(d.run_id, "r1");
        assert_eq!(d.base_sha, "f939464");
        assert_eq!(d.fetched_at, 0);
        assert_eq!(d.base_dir, "/b/r1");
    }

    #[test]
    fn from_fetch_refuses_a_missing_fetch() {
        // No fetch means no base: an empty tip is the signature of the
        // "inherit a startup snapshot" bug, so it must not produce a run.
        let err = Dispatch::from_fetch("r1", "  ", 0, "/b/r1").unwrap_err();
        assert!(err.contains("fetch origin/main"));
        assert!(Dispatch::from_fetch("", "tip", 0, "/b").is_err());
        assert!(Dispatch::from_fetch("r1", "tip", 0, "  ").is_err());
    }

    // --- Rule 3: two dispatches hours apart, with an intervening merge ----

    #[test]
    fn two_dispatches_hours_apart_get_different_bases_after_a_merge() {
        // The frozen-snapshot bug gives every job the same base (the day the
        // snapshot was taken). A fresh fetch tracks the tip, so an
        // intervening merge moves the next dispatch's base with it.
        let first = Dispatch::from_fetch("r1", "f939464", 0, "/b/r1").unwrap();

        // Hours later, main has advanced past a merge.
        let second = Dispatch::from_fetch("r2", "785447c", 3600, "/b/r2").unwrap();

        assert_ne!(first.base_sha, second.base_sha);
        assert_eq!(first.base_sha, "f939464");
        assert_eq!(second.base_sha, "785447c");
        assert!(first.fetched_at < second.fetched_at);

        // The first dispatch is now stale against the current tip; the
        // second is fresh.
        assert!(first.is_stale("785447c"));
        assert!(!first.is_fresh("785447c"));
        assert!(second.is_fresh("785447c"));
        assert!(!second.is_stale("785447c"));
    }

    #[test]
    fn a_base_is_fresh_exactly_while_it_is_the_tip() {
        let d = Dispatch::from_fetch("r1", "abc123", 10, "/b/r1").unwrap();
        assert!(d.is_fresh("abc123"));
        assert!(!d.is_stale("abc123"));
        assert!(d.is_stale("def456"));
        assert!(!d.is_fresh("def456"));
    }

    // --- Rule 2: the base must be reachable from origin/main --------------

    #[test]
    fn mainline_reachable_traverses_parent_links_from_the_tip() {
        let mainline = mainline_reachable("c", &parent_map());
        // `c` (tip), `b`, and `a` are on main; the divergent commit `d` is
        // not, because rev-list from the tip never follows the feature
        // branch.
        assert!(mainline.contains("c"));
        assert!(mainline.contains("b"));
        assert!(mainline.contains("a"));
        assert!(!mainline.contains("d"));
    }

    #[test]
    fn base_on_the_mainline_is_reachable_a_divergent_base_is_not() {
        let mainline = mainline_reachable("c", &parent_map());

        // A base at the tip, and a base pinned to an older ancestor, are
        // both on the mainline: a patch written against either integrates by
        // a forward rebase.
        assert!(base_reachable_from_mainline("c", &mainline));
        assert!(base_reachable_from_mainline("a", &mainline));

        // A base on a divergent commit is the "born conflicted" case.
        assert!(!base_reachable_from_mainline("d", &mainline));

        // No recorded base is never reachable.
        assert!(!base_reachable_from_mainline("  ", &mainline));
    }

    #[test]
    fn mainline_reachable_tolerates_a_cycle_and_an_unknown_root() {
        // A malformed graph with a cycle must terminate.
        let mut cyclic = BTreeMap::new();
        cyclic.insert("x".to_string(), vec!["y".to_string()]);
        cyclic.insert("y".to_string(), vec!["x".to_string()]);
        let mainline = mainline_reachable("x", &cyclic);
        assert!(mainline.contains("x"));
        assert!(mainline.contains("y"));

        // A root with no parent links is its own (single) mainline.
        let empty: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let solo = mainline_reachable("only", &empty);
        assert_eq!(solo, BTreeSet::from(["only".to_string()]));
    }

    // --- Rule 4: private, non-shared base checkouts -----------------------

    #[test]
    fn plan_base_pool_gives_each_base_a_unique_private_path() {
        let pool = plan_base_pool(3, "/scratch/base").unwrap();
        let paths: Vec<&str> = pool.iter().map(|base| base.path.as_str()).collect();
        assert_eq!(
            paths,
            vec![
                "/scratch/base/base-0",
                "/scratch/base/base-1",
                "/scratch/base/base-2"
            ]
        );
        let unique: BTreeSet<&str> = paths.iter().copied().collect();
        assert_eq!(unique.len(), paths.len());
    }

    #[test]
    fn plan_base_pool_rejects_zero_and_empty_root() {
        assert!(plan_base_pool(0, "/root").is_err());
        assert!(plan_base_pool(2, "  ").is_err());
    }

    #[test]
    fn lease_hands_out_private_paths_and_refuses_to_share_one() {
        let pool = plan_base_pool(2, "/scratch/base").unwrap();
        let mut leases = BaseLeasePool::new(pool);

        let p0 = leases.lease(0, "r1").unwrap();
        let p1 = leases.lease(1, "r2").unwrap();
        assert_eq!(p0, "/scratch/base/base-0");
        assert_eq!(p1, "/scratch/base/base-1");
        assert_eq!(leases.holder(0), Some("r1"));

        // A second run must not get the base r1 holds: that is the shared
        // mutable working tree the whole fix exists to remove.
        let shared = leases.lease(0, "r3").unwrap_err();
        assert!(shared.contains("already held"));
        // And a run cannot double-lease its own base.
        let again = leases.lease(1, "r2").unwrap_err();
        assert!(again.contains("already holds base"));
    }

    #[test]
    fn lease_rejects_unknown_index_and_empty_run_id() {
        let pool = plan_base_pool(1, "/scratch/base").unwrap();
        let mut leases = BaseLeasePool::new(pool);
        assert!(leases.lease(5, "r1").is_err());
        assert!(leases.lease(0, "  ").is_err());
    }

    #[test]
    fn release_frees_a_run_bases_for_reuse_by_another() {
        let pool = plan_base_pool(1, "/scratch/base").unwrap();
        let mut leases = BaseLeasePool::new(pool);
        leases.lease(0, "r1").unwrap();

        assert_eq!(leases.release("r1"), 1);
        assert_eq!(leases.holder(0), None);

        // The freed base is reusable by a different run.
        assert!(leases.lease(0, "r2").is_ok());
        // Releasing a run that holds nothing frees nothing.
        assert_eq!(leases.release("nobody"), 0);
    }
}
