### Added

- `autospec-core::loop_reset` — primitives for the invariant that a
  per-item loop must return to a known-clean state at the start of every
  iteration, and that the reset must be strong enough to undo everything
  the previous iteration could have done (issue #4395: a conversion loop
  reset with `git checkout --detach` and `git clean -fd` after iterations
  that ended in `git add -A`, and `clean` does not remove staged files —
  so iteration N+1 measured a tree containing two patches and issue 4368
  was held for a formatting failure that belonged to the previous
  patch's files). Each reset step names what it undoes: a hard reset to
  the base undoes staged AND unstaged changes, `git clean -fd` removes
  untracked files, and a `git checkout` undoes nothing, carrying staged,
  unstaged, and untracked state across (`ResetStep::undoes`); a plan that
  leaves a mutation class the loop can produce is a finding that names the
  leaked class and the step that looks like it handles it
  (`reset_coverage_findings`, `git_reset_plan`); and the loop proves the
  restore instead of assuming it by running the same item twice and
  requiring the same verdict both times (`repeat_run_findings`), because
  the leak is silent — it shows up as a wrong verdict, never an error, and
  the contaminated run and the correct run can agree by luck.
