# Identical pre/post failures are a usable baseline

Issue #4644: a crate whose suite fails on the execution host because a tool
is missing (the `schema-gen` suite, no `npm`) was held as `UNKNOWN-NO-
BASELINE` — a verdict about an environment the patch never caused. When the
same stage fails before and after the patch, the failure is a property of
the base, and the patch is unmeasured, not defective.

## The defect

A pass that cannot measure the base recorded a verdict anyway. The identical
failure was read as evidence against the change rather than as evidence
about the host, so every patch in such a workspace was held for a problem
it did not cause, and re-offering them was suppressed.

## The fix

The gate already attributes a stage to whoever fails it: `stage_origin`
re-runs the failing stage at the base (resetting index and worktree to
HEAD, then cleaning untracked files — undoing exactly the `git apply` that
placed the patch, so the re-run measures the base, never the change), and
a base that also fails yields `BaseUnverifiable` — the patch is skipped,
never held, and the issue re-enters dispatch for a later pass on a host
that can measure it. The fix here is the regression test that proves the
shape: a fixture workspace with one consistently-failing crate, driven
through the real `--apply` pass.

## The invariants

1. **A failing base is not a failing patch.** A stage that fails at the
   base and again after the patch is the base's failure. The verdict is
   `BaseUnverifiable`: the change is unmeasured, not defective.

2. **The skip is a release, not a verdict.** The patch stays on disk and
   the issue re-enters dispatch. Nothing durable records the environmental
   failure against the change — `held=0`.

3. **The regression drives the real pass.** A unit test of `verdict_for`
   already pins the mapping; the integration test pins the end-to-end
   shape: `--apply` on a workspace with a consistently-failing crate names
   the patch `skipped: base unverifiable`, never `HELD`, never `converted`.

## The general rule

When a measurement is impossible for a reason that is the same before and
after your change, the result is unmeasured, not failed. Distinguish the
two errors by re-running at the base: identical failure is the base's, a
new failure is yours. The two errors are not symmetric — re-offering a good
patch costs a delay, while holding one writes a durable false claim about
someone else's change.
