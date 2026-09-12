# Work selection is part of the system, not the invocation (issue #4257)

A loop that runs every thirty minutes — "find the agent patches worth
converting" — never had its selection predicate written down, so it was
reconstructed from memory on every pass. Three reconstructions, three
distinct defects: the glob spanned four projects whose issue numbers
collide (`issue-14` is InferWeave, `issue-1` is the dispatcher — converting
by bare number would have opened InferWeave patches as autospec pull
requests); it counted issue *directories* rather than finished
`changes.patch` files, so in-flight work was offered as a candidate and a
whole pass printed `SKIP: no patch`; and it applied a filter from a
previous run that was never re-derived, reporting the backlog "drained" at
2 when it held 78. Each version was written in a hurry, looked right, and
produced a plausible number. None was reviewable, because none existed as
an artifact — they lived in shell history.

- **The predicate is an artifact, not an invocation.** A condition that
  decides what work to do is written down as a file: the scope (what the
  query may see at all) plus every exclusion, each with a short report
  label, a written condition, and a justification. Construction refuses an
  unnamed or unjustified condition — the comment block matters as much as
  the code, because every condition of the conversion selector is a bug
  that was actually shipped, and written down they stop being
  rediscoverable:

  ```
  #   1. scoped to ONE project's out/          (issue numbers collide across projects)
  #   2. a NON-EMPTY changes.patch exists      (the agent actually finished)
  #   3. no open or merged PR                  (CLOSED is an abandoned attempt, not a conversion)
  #   4. the issue is still OPEN               (a patch for a closed issue is moot)
  #   5. not already attempted                 (unless --retry-held)
  ```

- **The selector reports its denominator.** The pass prints
  `considered=423 finished_patches=423 have_pr=314 closed_issue=265
  attempted=232 -> candidates=0`: every exclusion reports how many items
  it removed, and the line always leads with `considered=`. A bare
  `candidates=0` is unfalsifiable — it cannot be told from "the query was
  never run" or "the filter is broken". With the denominator, a zero is
  *evidence the backlog is drained*, which is a different and much more
  useful statement (the #3992 rule applied to the selection step rather
  than the execution step). `considered=0` is neither: it is "the query
  never ran or the scope matched nothing", and it is never rendered as
  "drained".

- **Loop specs define the selection predicate as precisely as the
  action.** Every autonomous loop has a selection step and an execution
  step. Specs describe the execution step reliably — "convert each
  candidate patch to a PR" — and leave the selection step as an English
  phrase ("any new patches"), which is where the defects live, because the
  phrase is re-interpreted on every run. A spec for a loop must enumerate
  the conditions that decide what work to do, what is excluded and why, and
  it must require the implementation to report how many items each exclusion
  removed.

Checkable in `autospec_core::work_selection` (`SelectionSpec`, `Exclusion`,
`SelectionReport::line`, `SelectionReport::verdict`, `SelectionVerdict`).
Tests: `crates/autospec-core/tests/work_selection.rs`, including the
regression that reproduces the issue's report line verbatim and the three
shipped defects instantiated.
