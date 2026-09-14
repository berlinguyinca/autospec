# A classification is a fact about a base, and expires when the base moves (issue #4512)

Three measurements of the same 162-item backlog, taken within one session:

```
against main @ b271d90b    96 conflict, 45 clean, 18 already-in-main
merge 4 PRs
against main @ c1e02b4a    73 refused, 24 resolvable, 45 clean, 20 already-in-main
merge 4 more PRs
against main @ d17965a0    #3821, #3863, #3871 no longer apply at all
```

#3821, #3863 and #3871 were classified `already-in-main` — their patches
applied and produced an empty diff. Twenty minutes and four merges later the
same patches do not apply: the merges touched the files they depend on, so
the classification flipped from "delivered" to "conflicting" without anything
about the patches or the issues changing. The same effect erased a category in
the other direction: 25 conflicts measured one pass earlier were gone the
next, because the base moved out from under them.

The natural reading of a stale count is "something is wrong with the patch."
The truth is that the count was always a measurement — against a base — and
the base moved.

## The invariant

**A derived classification records the base it was derived against, and a
consumer revalidates or recomputes when the base has moved.** The candidate
list is `(base_sha, entries)`, not `entries` — the same discipline
`hold_memo::HoldRecord` applies to holds, for the same reason: the hold and
the classification are both facts about a pair.

## Where it is enforced

- The conversion pass's selection line ends with the base it measured
  (`@ origin/<base>#<sha8>`), and the `--json` plan carries the full sha as
  `classified_base`, so "45 fresh" reads as "45 fresh against
  `origin/main#01234567`" — a measurement, not a standing fact.
- `autospec convert --apply` revalidates the classification against the
  current base immediately before acting (the only point at which the pass
  mutates). If the base moved since the plan, the liveness and delivered
  checks are re-run and each changed candidate is reported on its own
  `STALE #N: <from> -> <to>` line; the pass acts on the revalidated states,
  never on the stale ones. A base that cannot be resolved fails closed: the
  recorded states keep, with a warning — a missing ref never authorises
  acting on a reclassification the pass could not compute.
- The `base_sha` a HELD record cites is the tip of `origin/<base>` the patch
  is gated against — never the checkout's `HEAD`, which can be any branch
  and records a fact about the wrong ref.
