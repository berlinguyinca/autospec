# A claim is lifted by the fact, never by the silence

Issue #4626: a backlog of held patches in which two thirds were for issues
that had already been closed. Nothing asked the tracker, so every pass
re-gated the same holds against a base that kept moving — the measured
waste of a fleet whose queue entries were blocked by residue.

## The defect

A hold is a claim that work is pending, derived against a base at a moment.
The claim can become false in two ways the pass already knew about — the
base moved (re-gate, #4512) and the patch changed (re-gate) — and in one
way it did not: the issue itself closed. Closure is a fact about the
issue, not the base, so no amount of base revalidation could see it.

## The fix

Before selection, the pass asks the tracker for the state of each
issue that carries a recorded hold — `gh api repos/R/issues/N` — and a
closed issue's candidate is routed to the `closed` bucket: named on its
own line, archived under `--apply` (the queue entry releases with it),
its hold record removed from the ledger, never gated, never offered.

## The invariants

1. **Bounded cost.** Only held-recorded candidates are asked. The ledger
   is small; the fresh backlog is not, and a fresh patch for a closed
   issue comes from a closed queue entry, which the dispatch side evicts.
   Asking the tracker for every patch on disk would turn a ledger-sized
   query into a backlog-sized one.

2. **Unknown never authorises acting.** `issue_is_closed` returns
   `Option<bool>`: `Some(true)` only on the literal state `closed`,
   `Some(false)` on `open`, `None` on everything else — no repo, no `gh`,
   a failed call, an unexpected state. `None` behaves as `false`. A hold
   is a claim, and a claim is lifted by the fact of closure, never by the
   absence of a state. The fail-open half is a test, not an assumption:
   an unreachable tracker leaves the hold in place.

3. **Ordering of facts.** Delivered beats closed beats attempt beats
   hold. The work being in the base is the stronger fact than the issue
   being over; both beat a claim about how the patch fared.

4. **Release is archival plus removal.** The patch moves to
   `superseded/` (never deleted) and the hold record is rewritten out of
   the ledger only when a record was actually removed — the same
   discipline as `--archive`: a ledger with none must not be created by
   the release.

## The general rule

When an external system holds the state your decisions depend on and the
set you must ask is bounded, ask it — before acting, not after — and
define the unknown to mean "do not act." The two failure directions are
asymmetric: acting on a closed issue opens PRs against dead work (the
measured waste), while keeping a hold when the tracker is unreachable
costs one pass of re-gating (the status quo). The default goes to the
cheaper error.
