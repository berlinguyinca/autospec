# 4552 — prose adjacency is not enforcement

`AGENTS.md` forbade appending invariants to its own list four lines above the
list; thirteen patches were still blocked on the region, and the file appeared
in 21 of 96 conflicts. The rule was correct, specific, and read by every agent
— and it lost to the local convention it described.

The fix was already on main (#4543–#4550 relocated the invariants to
`AGENTS.d/`, #4551 deleted the hand-maintained list); this PR records the
mechanism in `AGENTS.d/4552-prose-adjacency-is-not-enforcement.md`:

- a rule that forbids a structure must remove the structure, or it is
  decoration — prose cannot outvote the local convention it sits inside;
- the available actions define the behaviour: while the forbidden action is
  the path of least resistance, it is taken at scale;
- redundant anchors are worse than none (the list duplicated the `AGENTS.d/`
  files and was incomplete, so it was a conflict magnet and a staleness trap);
- verify the effect, not the edit: the test is that the next invariant lands
  as a new file, which is now the only thing that can land.
