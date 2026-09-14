# A branch without a PR is an interruption, not a success

The conversion pass pushes a branch and only then opens the PR. An
interruption in that window — a timeout, a killed session, a rate limit —
leaves a branch with no PR, and the old liveness check read that branch as
"already attempted" and retired the issue forever (#4499).

The invariant: **a step that marks work as done must not complete before the
work does.** Where two remote operations cannot be atomic, the liveness check
requires the marker that only the final step produces.

Consequences, enforced in `conversion_pass` + `convert`:

- The disqualifying fact is a branch **with** an open or merged PR (or a
  recorded HELD entry). A branch alone — or the pass's own checked-out
  worktree with no PR — is `Attempt::Interrupted`: re-offered, reported as a
  distinct category, and re-attempted.
- A redo of an interrupted attempt force-pushes over the orphan branch
  (`--force-with-lease`: it refuses if the branch moved since the fetch —
  e.g. a PR opened on it, which is exactly the state that disqualifies a
  redo).
- `--apply` prints `START  #N` / `DONE   #N: <outcome>` per issue, at the
  moment each happened: a START without a DONE is where the run stopped.
- `Unknown` (the liveness lookup itself failed) fails closed: offering a
  patch whose attempt state cannot be verified risks a duplicate PR.

Prose that says "a branch means attempted" is a regression: the evidence of
the attempt (a branch) is created *before* the thing that attempts (a PR), so
the two must be checked together, never one for the other.
