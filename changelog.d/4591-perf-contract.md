# 4591 — a performance task now carries its verdict contract

The first attempt at the #4587 optimisation was wrong in three ways (dead code,
a dropped local-worktree check, a missed short-circuit) and was only caught
because the author stopped to read the function being replaced. The general
defect: a performance change's success criterion is observable (faster) and its
failure criterion is not (decides differently — days later, as a clobbered
worktree or a duplicate PR).

The prompts now pin the verdict before the speed is claimed:

- The decomposer contract (Phase 3) gained a *Performance-task decomposition*
  section: a spec that replaces behaviour with something faster must carry the
  existing behaviour as an ordered contract (every branch, in order, including
  short-circuits), verdict tests separate from any timing claim, and the
  unavailable-fast-path statement (the default is fall back to the unbatched
  path — never "failed batch = empty result").
- The implementer contract gained a `PERF_CONTRACT` directive: state the
  contract in the new code's doc comment, test the verdict per branch, fall
  back on failure, and treat a computed-and-discarded value as a signal to
  resolve before review.
- `AGENTS.d/4591-an-optimisation-must-reproduce-its-contract.md` records the
  three failure modes and the corrected contract (`AttemptIndex::liveness`).
