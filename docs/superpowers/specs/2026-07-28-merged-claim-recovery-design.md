# Merged Claim Recovery Design

## Problem

The foreground conductor can remain paused on `executor_receipt_failed` after an
operator or earlier Autospec process merges the exact linked pull request. If
the executor worktree was removed while durable bridge state still says
`draft_created`, recovery validates the missing worktree before observing the
already-merged pull request. The conductor exits, and its supervisor repeats
the same failure without reaching the next ready issue.

## Decision

Reconcile remote terminal truth inside the executor bridge immediately after
loading the private persisted invocation and before local worktree recovery.
The reconciliation is terminal retirement only; it does not retroactively
admit or review commits.

Require the invocation, acquisition request, and authoritative claim generation
to name the same repository, issue, worker, claim ID, invocation, branch, and PR.
Observe that exact PR with its number, state, draft flag, head branch, head OID,
base branch, and merge OID. Continue only when it is non-draft and `MERGED`, the
branch and base are exact, both OIDs are canonical, the local issue branch
equals the merged PR head, and the persisted implementation head is an ancestor
of the merged PR head.

Before changing durable state, write an idempotent private reconciliation record
bound to the executor cleanup identity. It preserves the originally verified
head, merged PR head, merge OID, PR, branch, and base. Then persist the invocation
as `Merged`, terminalize the exact claim through the existing compare-and-swap,
and enter normal terminal cleanup. The final receipt reports the actual merged
head while the reconciliation record preserves the evidence boundary: previous
review and closeout artifacts do not validate any externally added commits.

Terminal cleanup writes durable removal intent before acting. If the exact
worktree path is already absent but its exact branch/path registration is marked
prunable, remove only that registration with `git worktree remove --force`.
Unrelated prunable worktrees remain untouched. Worktree recovery itself stays
fail-closed for every non-terminal case.

## Alternatives Rejected

1. Recreate every missing `draft_created` worktree. This mutates Git state even
   though the linked work is already merged and can re-run obsolete gates.
2. Delete local paused state whenever the issue is closed. Issue closure alone
   does not prove the exact claim or pull request and could discard live work.
3. Wait for supervisor retries. The retry repeats the same deterministic
   invariant failure and blocks unrelated ready work.
4. Require the persisted head to equal the merged PR head. Autospec reviewers
   can add follow-up commits before merge; ancestry proves the persisted work is
   included without falsely treating the added commits as previously reviewed.

## Verification

Extend the real Git-ref foreground conductor fixture. Persist a
`draft_created` invocation at head A, advance the exact issue branch to B, mark
its exact PR merged at B, remove its worktree while leaving an exact prunable
registration, and seed `executor_receipt_failed`. Recovery must terminalize the
exact claim, preserve an A-to-B reconciliation record, remove only the exact
prunable registration, preserve an unrelated prunable registration, launch no
second harness, and return to `Scan` so the next ready issue is eligible.

Focused bridge tests reject every non-exact observation: open or draft PR,
number/branch/base mismatch, malformed OIDs, missing or divergent local branch,
non-ancestor persisted head, changed claim ownership, and changed reconciliation
record. Crash-resume coverage proves idempotence after record creation, merged
state persistence, and claim terminalization.
