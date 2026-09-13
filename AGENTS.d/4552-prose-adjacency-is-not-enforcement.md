# A rule stated next to a live instance of what it forbids is not enforcement (issue #4552)

`AGENTS.md` carried this, verbatim:

> Add a new invariant as a NEW FILE under `AGENTS.d/`, never by appending
> here.

Four lines below it sat a hand-maintained list of invariants, appended to by
every agent that added one. Thirteen open patches were blocked on that region;
a backlog measurement put `AGENTS.md` in 21 of 96 conflicts.

The rule was correct, specific, stated its own consequence, cited its own
incident, and was printed immediately above the thing that violated it — in a
file every agent reads before working. It did not work.

## Why prose adjacency loses

An agent writing an invariant sees a section of invariants and appends to it.
That is the strongest possible signal about what to do, and it is made of
**example** rather than instruction. A sentence saying "do not do this" loses
to a worked demonstration of doing it, every time, because the demonstration
is what the surrounding content *is*. The rule was not ignored through
carelessness; it was outvoted by the file's own structure.

Negative-space rules ("never append here") have no positive action to point at
while the forbidden structure still exists. The structure itself is the
gravity well: every author, following the local convention, falls into it, and
the prose is re-read after the fact as an apology, not as a constraint.

## What actually fixed it

1. **Relocate, per patch**: extract each inlined invariant into
   `AGENTS.d/NNNN-slug.md`, leaving `AGENTS.md` untouched (#4543–#4550).
   Content byte-identical, location now matching the rule, and each change a
   new file that conflicts with nothing.
2. **Remove the anchor** (#4551): delete the hand-maintained list entirely.
   Every bullet duplicated an `AGENTS.d/` file that already existed and
   several files had no bullet, so it was redundant *and* incomplete. With no
   list to append to, the only available action is the one the rule asks for.

Only the second change is durable. The first converts a patch; the second
removes the reason the patch would be written the wrong way.

## Invariants

- **A rule that forbids a structure must remove the structure, or it is
  decoration.** Prose cannot outvote the local convention it sits inside.
- **The available actions define the behaviour.** If the forbidden action is
  still the path of least resistance, it will be taken, at scale, by agents
  that read the rule first.
- **Redundant anchors are worse than none.** A second, incomplete copy of the
  canonical location is a conflict magnet and a staleness trap; delete it.
- **Verify the effect, not the edit**: after removing the anchor, the test is
  not "the list is gone" but "the next invariant lands as a new file" — which
  is now the only thing that can land, because nothing else exists to append
  to.
