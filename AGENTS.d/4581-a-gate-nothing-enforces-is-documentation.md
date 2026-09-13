# A gate nothing enforces is documentation: `main` now requires its reliable checks (issue #4581)

`main` is the input to every automated process in this repository — every
agent, every conversion pass, every gate builds from it. While it had no
branch protection, nine unformatted files landed across several merges and
`cargo fmt --all --check` went red on `main`, which made every branch cut from
`main` fail the conversion pass's first stage. The pipeline that turns
GPU-hours into PRs stopped on a defect none of the held patches introduced.
The shape of the defect: each individual merge looked fine; the missing
enforcement is what made the accumulation possible.

## What is now enforced

`main` requires these status checks before a non-admin merge:

- `build-test` (the full workspace: fmt, clippy, tests)
- `autospec file-size-ratchet (Autospec)` (TeamCity)
- `autospec python-suites (Autospec)` (TeamCity)
- `autospec security-workstream (Autospec)` (TeamCity)
- `autospec stack-guard (Autospec)` (TeamCity)
- `autospec ux-ui-workstream (Autospec)` (TeamCity)
- `autospec accessibility-workstream (Autospec)` (TeamCity)

`autospec architecture-fitness (Autospec)` is **deliberately excluded**: it is
red at 0s on every PR (73 pre-existing violations, 52 of them frozen baseline
data in `deadline_ratchet.rs`). An unreliable check must not be the reason the
reliable ones go unenforced. Re-include it once it is green on `main`.

## The recorded-exception rule

`enforce_admins` is off, so an admin merge over a red check remains possible —
that is the exception path, and it must stay an exception:

1. **The exception is written into the PR body before the merge** — which
   checks were red, why each is pre-existing on `main`, and the evidence
   (reproduction on a clean `main` worktree). An admin merge with no such
   record is a policy violation even though the API allows it.
2. **A merge over a red check is a repair, not a habit.** If the same check is
   red on two consecutive admin merges, stop and fix the check or the red;
   stacking exceptions is how a gate stops gating.
3. **Automation that merges is held to the reviewer standard, not below it.**
   Throughput that moves the cost downstream (held patches, idle GPUs) is a
   defect in the automation, not a feature.

## Invariants

- A branch that automated processes build from must require the checks those
  processes run. If the conversion gate runs `fmt --check`, clippy and tests,
  merging to `main` requires the same green.
- Where a check fails on every PR, exclude it from the required set and file
  its repair — do not let it veto the rest.
- Verify the effect, not the edit: after changing protection, confirm an open
  PR reports `mergeStateStatus: BLOCKED` while a required check is red.
