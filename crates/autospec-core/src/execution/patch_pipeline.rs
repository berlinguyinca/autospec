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
    ordered.sort_by(|a, b| {
        a.class
            .cmp(&b.class)
            .then(b.unblocks.cmp(&a.unblocks))
            .then(b.produced_at.cmp(&a.produced_at))
    });
    ordered
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
/// (#3698). A converter that runs for hours must see work produced during
/// those hours; the caller re-plans the pass after absorbing.
///
/// Identity is the dedup key. When an identity already exists, the entry
/// with the higher `produced_at` wins; on a tie the freshly observed entry
/// wins, because a re-scan is the newer observation of the same patch. The
/// queue's existing order is untouched — ordering is the planner's job.
pub fn absorb_new(queue: &mut Vec<Patch>, fresh: impl IntoIterator<Item = Patch>) {
    for observed in fresh {
        match queue.iter_mut().find(|p| p.identity == observed.identity) {
            Some(existing) if existing.produced_at > observed.produced_at => {}
            Some(existing) => *existing = observed,
            None => queue.push(observed),
        }
    }
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
/// hypothesis, not a decision (#3698).
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
        // Nine patches produced mid-pass must not stay invisible: a
        // re-scan merges them into the running queue, deduped by identity.
        let mut queue = vec![
            patch_at("p1", "base-1", ConversionClass::Candidate, 100),
            patch_at("p2", "base-1", ConversionClass::MemoizedHold, 200),
        ];

        absorb_new(
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
                ("p1", "base-1", ConversionClass::Candidate, 100),
                ("p2", "base-2", ConversionClass::Candidate, 250),
                ("p3", "base-2", ConversionClass::Candidate, 300),
            ]
        );
    }

    #[test]
    fn absorb_new_on_tie_prefers_the_fresh_observation() {
        // A re-scan is the newer observation of the same patch: on a tie the
        // fresh entry wins.
        let mut queue = vec![patch_at("p1", "base-1", ConversionClass::Candidate, 100)];

        absorb_new(
            &mut queue,
            vec![patch_at("p1", "base-2", ConversionClass::ExistingPr, 100)],
        );

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
}
