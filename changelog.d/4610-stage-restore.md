# The stage restore now undoes a staged `git apply --3way` (issue #4610)

When a gate stage fails, `classify_stage_failure` decides whose failure it
is by restoring the worktree to the base and re-running the stage there.
The restore was `git checkout -- .` + `git clean -fd` — but the patch had
been applied with `git apply --3way`, which stages its result: modified
files land in the index and new files are added to it. `checkout -- .` with
no tree-ish copies the worktree *from the index*, i.e. from the patched
state (a no-op), and `clean -fd` does not remove a new file the index
tracks. The "base" re-run ran on the patch.

So every patch that failed a gate stage failed again at its own
"base" and was attributed to `StageOrigin::Base`: a false "the base is
broken" alarm naming a green SHA, and a patch that was skipped, never
held, and re-offered on the next pass where it failed identically — a
permanently stuck state for every defective patch.

The restore is now a hard `git reset` to `HEAD` (the conversion branch
sits at the base commit; the patch is staged, never committed, so the
reset is exactly the undo the apply deserves) followed by the same
`clean -fd` for untracked leftovers.

Found while writing the first test that drives a patch through a failing
gate via the real apply path (issue #4572's progress test): the test's
patch deliberately fails the test stage, and the pass skipped it with a
false base alarm instead of holding it. The test asserts the fixed
behavior end to end: `held=1 skipped=0`, with the failing test named in
the HELD line.
