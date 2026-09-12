//! A per-item loop must reset to a known-clean state (issue #4395).
//!
//! A conversion loop decided, per patch, "is this mergeable", applying one
//! patch per iteration to a shared checkout. Each iteration ended with
//! `git add -A`; the next began with a detach and `git clean -fd` — but
//! `clean` does not remove staged files, and a checkout carries staged
//! changes across. Iteration N+1 began with iteration N's files already
//! present: the patch it was supposed to test failed to apply ("already
//! exists in working directory"), and the gate it ran measured a tree
//! containing *two* patches. Issue 4368 was recorded as failing
//! `cargo fmt --all --check` when the formatting failure belonged to the
//! previous patch's files — a false HELD. Re-running with a proper reset
//! showed 4368 does genuinely fail fmt, and the contaminated run and the
//! correct run happened to agree: nothing in the output distinguished them.
//!
//! Leaked state makes the per-item verdict a function of iteration ORDER:
//! an item can be held for a defect in an unrelated item, or pass because
//! an earlier item supplied what it was missing. Both verdicts are recorded
//! as if they were about the item alone.
//!
//! Invariants, each encoded as a checkable primitive:
//!
//! 1. **A reset operation has defined coverage.** Each step names what it
//!    undoes: a hard reset to the base undoes staged AND unstaged changes,
//!    `git clean -fd` removes untracked files, and a `git checkout`
//!    (detached or onto a branch) undoes *nothing* — it moves, and it
//!    carries staged, unstaged, and untracked state across
//!    ([`ResetStep::undoes`]). `clean` alone is not a reset. `checkout`
//!    alone is not a reset.
//! 2. **The reset plan must cover everything the previous iteration could
//!    leave behind.** A plan that leaves a mutation class the loop can
//!    produce is a finding, and the finding names the leaked class and the
//!    step that looks like it handles it
//!    ([`reset_coverage_findings`], [`git_reset_plan`]).
//! 3. **The loop must prove it restored, not assume it did.** The proof is
//!    running the same item twice and requiring the same verdict both
//!    times: a clean loop is order-independent, so a repeat-run mismatch is
//!    state leak — and the leak is silent, showing up as a wrong verdict
//!    rather than an error ([`repeat_run_findings`]).

use std::collections::BTreeSet;

/// What one iteration can leave behind in the shared workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Mutation {
    /// Changes staged into the index (`git add`). `git clean` does not
    /// remove these, and a `git checkout` carries them across.
    Staged,
    /// Modifications to tracked files that are not staged. `git clean`
    /// does not remove these either.
    Unstaged,
    /// Files the workspace did not know before the iteration ran.
    Untracked,
}

impl Mutation {
    /// The name the findings use.
    pub fn label(self) -> &'static str {
        match self {
            Mutation::Staged => "staged",
            Mutation::Unstaged => "unstaged",
            Mutation::Untracked => "untracked",
        }
    }
}

/// One operation a loop runs to restore the workspace before an
/// iteration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetStep {
    /// A hard reset to the base: undoes staged AND unstaged changes.
    HardReset,
    /// `git clean -fd`: removes untracked files. It does not touch staged
    /// or unstaged tracked state — `clean` alone is not a reset.
    Clean,
    /// `git checkout --detach` (or onto a branch): moves HEAD and carries
    /// staged, unstaged, and untracked state across. Undoes nothing —
    /// `checkout` alone is not a reset.
    Checkout,
}

impl ResetStep {
    /// What this step undoes. The table is the invariant: a step that
    /// looks like a reset but does not undo the class the previous
    /// iteration can produce is the incident.
    pub fn undoes(&self) -> &'static [Mutation] {
        match self {
            ResetStep::HardReset => &[Mutation::Staged, Mutation::Unstaged],
            ResetStep::Clean => &[Mutation::Untracked],
            ResetStep::Checkout => &[],
        }
    }

    /// The name the findings use.
    pub fn name(self) -> &'static str {
        match self {
            ResetStep::HardReset => "hard reset",
            ResetStep::Clean => "git clean",
            ResetStep::Checkout => "git checkout",
        }
    }

    /// The command the step runs. `base` is the sha the loop rebuilds
    /// every iteration from — a sha, not a moving ref: `origin/main` can
    /// move between the fetch and the reset, a sha cannot.
    pub fn command(&self, base: &str) -> String {
        match self {
            ResetStep::HardReset => format!("git reset --hard {base}"), // linter:allow-SECURITY the invariant's own canonical command: the hard reset a per-item loop owes the next iteration
            ResetStep::Clean => "git clean -fd".to_string(),
            ResetStep::Checkout => "git checkout --detach".to_string(),
        }
    }
}

/// The canonical git reset a per-item loop must run at the start of every
/// iteration: a hard reset to the base, then a clean of untracked files.
/// Together they undo everything an iteration that can stage, modify, and
/// create files can leave behind; separately, neither does.
pub fn git_reset_plan() -> [ResetStep; 2] {
    [ResetStep::HardReset, ResetStep::Clean]
}

/// Invariant 2, as a check: the findings for a reset plan that does not
/// undo everything the previous iteration could have left behind.
///
/// `left_behind` is what an iteration can do to the shared workspace
/// (stage, modify, create files); `plan` is what the loop runs at the
/// start of the next one. One finding per leaked mutation class. The
/// finding names the class, the steps the plan was handed, and the step
/// that looks like it handles it — because the failure is silent, the
/// diagnostic has to say which step the reader would have trusted and why
/// that trust is the defect.
pub fn reset_coverage_findings(left_behind: &[Mutation], plan: &[ResetStep]) -> Vec<String> {
    let covered: BTreeSet<Mutation> = plan
        .iter()
        .flat_map(|step| step.undoes())
        .copied()
        .collect();
    let mut findings = Vec::new();
    for leaked in left_behind {
        if covered.contains(leaked) {
            continue;
        }
        findings.push(format!(
            "RESET_INCOMPLETE: the previous iteration can leave {} state behind and no step in the reset plan ({}) removes it — iteration N+1 begins with iteration N's {} state still present, so the verdict for item N+1 is a function of iteration order; {}",
            leaked.label(),
            plan_names(plan),
            leaked.label(),
            remedy(*leaked, plan),
        ));
    }
    findings
}

/// The plan as the findings cite it: the steps in order, or `(none)` for a
/// loop with no reset at all.
fn plan_names(plan: &[ResetStep]) -> String {
    if plan.is_empty() {
        "(none)".to_string()
    } else {
        plan.iter()
            .map(|step| step.name())
            .collect::<Vec<_>>()
            .join(" then ")
    }
}

/// The half of the finding that says what was missing, naming the step
/// that looks like it handles the leaked class — `clean` alone is not a
/// reset, `checkout` alone is not a reset.
fn remedy(leaked: Mutation, plan: &[ResetStep]) -> String {
    let add = match leaked {
        Mutation::Untracked => "add `git clean -fd`",
        Mutation::Staged | Mutation::Unstaged => "add a hard reset to the base sha",
    };
    let mut parts = Vec::new();
    if plan.iter().any(|step| matches!(step, ResetStep::Clean))
        && matches!(leaked, Mutation::Staged | Mutation::Unstaged)
    {
        parts.push(
            "`git clean` alone is not a reset: it removes untracked files, not staged or unstaged tracked state",
        );
    }
    if plan.iter().any(|step| matches!(step, ResetStep::Checkout)) {
        parts.push(
            "`git checkout` alone is not a reset: it moves HEAD and carries staged, unstaged, and untracked state across",
        );
    }
    if parts.is_empty() {
        return format!(
            "the loop has no step that removes {} state — {add}",
            leaked.label()
        );
    }
    format!("{} — so {add}", parts.join("; "))
}

/// The verdict a per-item loop records for one item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemVerdict {
    /// The item stands on its own: mergeable.
    Mergeable,
    /// The item is held for a defect.
    Held,
}

impl ItemVerdict {
    /// The name the findings use.
    pub fn label(self) -> &'static str {
        match self {
            ItemVerdict::Mergeable => "mergeable",
            ItemVerdict::Held => "HELD",
        }
    }
}

/// Invariant 3, as a check: the finding for a loop whose repeat run of the
/// same item gives a different verdict.
///
/// The reviewer's test for any loop over items sharing a workspace: run
/// the same item twice. A clean loop gives the same verdict both times,
/// because the reset returned the workspace to the known-clean state. A
/// mismatch is state leaked from pass 1 into pass 2 — or nondeterminism —
/// and it is the silent half of the incident: the contaminated run and the
/// correct run can agree by luck, in which case nothing in the output
/// distinguishes them and only the repeat-run check catches the leak.
pub fn repeat_run_findings(item: &str, first: ItemVerdict, second: ItemVerdict) -> Vec<String> {
    if first == second {
        return Vec::new();
    }
    vec![format!(
        "REPEAT_RUN_MISMATCH: {item} judged {} on the first pass and {} on the repeat run of the same item — a clean loop gives the same verdict both times, so shared state leaked from pass 1 into pass 2 and the recorded verdict is a function of iteration order, not of the item alone",
        first.label(),
        second.label(),
    )]
}
