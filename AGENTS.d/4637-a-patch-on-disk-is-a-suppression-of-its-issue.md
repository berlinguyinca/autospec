# A patch on disk is a suppression of its issue, and a dead base makes the patch dead (issue #4637)

193 queue entries, 27 idle agent slots, measured:

```
patch exists  ->  topup skips the issue forever        (already produced)
              ->  conversion never examines it again    (outside newest N)
              ->  issue never closes
              ->  patch is neither regenerated nor converted
```

Two independently reasonable rules composed into a trap: the dispatch guard
reads the patch's presence as "work finished, do not re-dispatch," and the
conversion pass examined only the newest cohort. For a months-old patch
conflict-bound against a base that no longer exists, the file is the *opposite*
of evidence of finished work — and both halves of the pipeline saw it as
evidence.

## The invariant

**A produced patch is only evidence of completed work while it is still
convertible against the current base.** Once the pass has proved it is not —
every conflicted file a shape it will never merge (refused, or
regenerate-from-source) — the patch stops suppressing re-dispatch: the
disposition is recorded where the patch lived (`disposition.txt`: status,
reason, the base the verdict was made against, the time), the patch is
archived (never deleted), and the issue re-enters dispatch, where an agent
regenerates the patch against current main.

## What is and is not structural

- **Structural (invalidated):** every conflicted file is unclassifiable
  (the plain-module shape the measured deadlock carried: `shape unknown`)
  or regenerate-from-source (single-value, generated). No re-gate, no amount
  of trunk repair converts this patch against this base; the base only moves
  further from it.
- **Not structural (ordinary HELD, re-offered):** any conflict with at least
  one certified keep-both file (the gate may be what is red), a parser
  failure over a certified shape, an unenumerable conflict, an
  infrastructure fault (no test result line), or a base that cannot be
  resolved. The pass cannot claim the patch is dead when it cannot prove
  which files it is dead on — a hold the pass could not justify is a hold it
  must not act on.

## The counter is the category

The outcome line reports `invalidated=N` on its own. Folding the category
into `skipped` would make the deadlock invisible at exactly the summary level
the fleet monitors — the same presence-versus-substance failure the whole
line exists to prevent.
