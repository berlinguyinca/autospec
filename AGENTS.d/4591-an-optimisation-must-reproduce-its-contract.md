# An optimisation must reproduce the contract it replaces (issue #4591)

A performance change is uniquely dangerous because **its success criterion is
observable and its failure criterion is not.** "Selection went from 340s to
seconds" is measurable in one run. "Selection now re-offers a patch whose branch
is checked out" surfaces days later as a clobbered worktree or a duplicate
pull request, and nothing links it back. The asymmetry rewards shipping the
fast version and checking the numbers — which is exactly the wrong incentive.

## The three ways the first attempt was wrong

The first version of the #4587 fix (one remote query per pass instead of per
candidate) failed in three different ways, all found only after stopping to
read the function being replaced line by line:

1. **Dead code.** It computed `self.remote_branches.contains(branch)` and
   discarded the result — a leftover from assuming remote presence mattered to
   the verdict before checking whether it did. A computed-and-discarded value
   means the author was unsure whether it mattered; that uncertainty is the
   thing to resolve before review, not after.
2. **A dropped check.** It folded away the local-worktree test, so a branch
   held by a leftover worktree would have been judged dead and re-offered,
   overwriting work in progress.
3. **A missed short-circuit.** The original answers "no branch" *without*
   consulting pull requests at all. That ordering is the reason a mostly-absent
   backlog presents as a long run of remote calls — it was simultaneously the
   thing to preserve and the explanation of the symptom, and the first version
   had it backwards.

## The contract, stated

The corrected version (`AttemptIndex::liveness`, `convert.rs`) reproduces the
order explicitly and names it in the doc comment:

1. a local worktree holding the branch makes the attempt live;
2. otherwise a branch absent from the remote is dead — answered without asking
   about pull requests at all;
3. otherwise an open or merged pull request makes it live;
4. otherwise the attempt is abandoned, and abandoned is not live.

The batched path returns `None` when either fetch fails, so the caller keeps
the per-branch path. Treating a failed batch as "nothing exists" would
re-offer every patch and open duplicate pull requests — precisely the outcome
the liveness check exists to prevent. This is the `unwrap_or(true)` defect
class recorded in #4123/#4129, in a new place.

## Invariants

1. **Before replacing a predicate, state its existing contract in full** —
   every branch, in order, including short-circuits — and put that statement
   in the new code as the thing being reproduced. If the contract cannot be
   written down, it is not yet understood well enough to optimise.
2. **An optimisation changes the number of operations, never the verdict.**
   The test for it is not "is it faster" but "does it decide the same thing."
   Both need asserting; only the first is ever obvious.
3. **A batched path must fall back to the unbatched one on failure.**
4. **Keep cheap correct checks where they are.** The local worktree check
   costs nothing and catches a real hazard; folding it into a batch for
   symmetry would trade a correct check for no saving. Uniformity is not a
   reason.
5. **Dead code in a new function is a signal, not lint noise.**

The decomposer contract (Phase 3) requires the spec to carry all three — the
ordered contract, the verdict tests, the unavailable-fast-path statement —
before a performance task is filed, and the implementer's `PERF_CONTRACT`
directive restates them at the point of implementation.
