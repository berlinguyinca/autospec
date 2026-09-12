# A per-item loop must reset to a known-clean state, and clean is not a reset (issue #4395)

A loop that processes items one at a time reset the working tree with
`git clean -fd` at the top of each iteration — but the previous iteration
had run `git add -A`, and `clean` does not remove staged files. Nor does
`git checkout --detach`, which carries staged changes across.

So iteration N+1 began with iteration N's files already present. The patch
it was supposed to test failed to apply ("already exists in working
directory"), and the gate it then ran was measuring a tree containing
*two* patches.

The result was a **false HELD**: issue 4368 was recorded as failing
`cargo fmt --all --check` when the formatting failure belonged to the
previous patch's files. Re-running with a proper reset showed 4368 does
genuinely fail fmt — but that was luck. The contaminated run and the
correct run happened to agree, and nothing in the output distinguished
them.

A conversion pass exists to decide, per patch, "is this mergeable".
Leaked state makes that verdict a function of iteration ORDER. A patch can
be held for a defect in an unrelated patch, and — worse in the other
direction — a patch can pass because a previous patch supplied something
it was missing. Both verdicts are recorded as if they were about the patch
alone.

- **A reset operation has defined coverage; `clean` alone is not a reset,
  and `checkout` alone is not a reset.** Each step names what it undoes: a
  hard reset to the base undoes staged AND unstaged changes,
  `git clean -fd` removes untracked files, and a `git checkout` (detached
  or onto a branch) undoes nothing — it moves, and it carries staged,
  unstaged, and untracked state across (`ResetStep::undoes`). The table,
  not the name of the step, is the invariant.
- **The reset must be strong enough to undo everything the previous
  iteration could have done**, at the start of every iteration. For git
  that is a hard reset to the base sha plus `git clean -fd`
  (`git_reset_plan`). A plan that leaves a mutation class the loop can
  produce is a finding, and the finding names the leaked class and the
  step that looks like it handles it (`reset_coverage_findings`) — because
  the failure is silent, the diagnostic has to say which step the reader
  would have trusted and why that trust is the defect.
- **If an iteration can mutate shared state, the loop must prove it has
  restored it — not assume.** The proof is running the same item twice: a
  clean loop gives the same verdict both times (`repeat_run_findings`).
  The leak shows up as a wrong verdict rather than an error, and the
  contaminated run and the correct run can agree by luck — in which case
  only the repeat-run check distinguishes them.

Checkable in `autospec_core::loop_reset` (`Mutation`, `ResetStep::undoes`,
`git_reset_plan`, `reset_coverage_findings`, `ItemVerdict`,
`repeat_run_findings`). Tests: `crates/autospec-core/tests/loop_reset.rs`,
including the regression that reconstructs the incident (an iteration that
ends in `git add -A`, a next iteration that begins with a detach and a
`git clean -fd` — the plan leaves staged state behind, and the finding
names both steps that look like they handle it) and the control that the
canonical plan covers everything an iteration can leave.
