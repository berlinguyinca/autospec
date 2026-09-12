# Silent false negatives: a rule that must be recalled at authoring time against an ordinary-looking command is not a fix (issue #4449)

Three separate recorded pitfalls fired again in one session. Each already
had a written rule. Each rule failed. They are the same defect wearing
three costumes:

| Tool | What it returned | What was true | Recurrences, all *after* the rule was written |
|---|---|---|---|
| `comm -12` on numerically-sorted files | 0 candidates | 68 candidates | twice |
| `grep 'worktree add'` | no matches | 4 call sites (`&["worktree", "add"]`) | at least twice |
| `pkill -f 'cargo.*test'` | exit 144, no output | killed the invoking shell | four times |

A tool reported an empty result, and the emptiness was manufactured by how
it was called rather than observed in the world. In all three the input
looked correct and nothing raised an error loud enough to stop the next
step from consuming the wrong answer. The `comm` case is the sharpest: it
*did* warn — `comm: file 1 is not in sorted order` on stderr — and then
wrote `0` to stdout anyway. Had the loop not been suspicious of a zero, it
would have reported "nothing to convert" on a backlog of 68, and the line
the conversion loop logs even when there is nothing to convert would have
been logged and would have been wrong.

**An empty result from a search, set operation, or process query is not
usable as evidence until the call has been shown capable of returning a
non-empty one.** The discharge is a positive control, and it is cheap in
all three cases:

- **set difference** — assert both inputs are non-empty and the
  intersection non-empty before trusting a difference of zero; or do the
  set operation in a language with real sets, where sort order cannot
  silently change the answer
- **search** — run the broadest single token first; if that is non-empty
  and the narrowed pattern is empty, the narrowing removed the hits
- **process query** — resolve to pids and count them before acting

**A rule that must be recalled at authoring time against an
ordinary-looking command is not a fix.** Each rule above was phrased as a
prohibition to remember in a hot path — "don't `comm` numerically-sorted
input", "validate a negative with a positive control", "never `pkill -f`
your own argv" — and every recurrence happened *after* the rule was written
down. Rules that depend on recall do not survive contact with a
routine-looking command; mechanisms do. The mechanism is the shape
`GateVerdict::NotMeasured` (#4434) proved: an empty set and an unanswered
question are different facts, and the type says which it is. The helper
surface is `autospec_core::false_negative` — `Measured` (set difference and
intersection, order-independent by construction, with a zero that is either
proven or `Unmeasured`), `SearchVerdict` (broad-first discharge), and
`ProcessVerdict` (resolve, count, then act) — and it is wired into the
conversion backlog (`execution::backlog`), where a zero `convertible` now
carries its zero-control and an unmeasured zero is reported on the summary
line instead of passing for a drained backlog.
