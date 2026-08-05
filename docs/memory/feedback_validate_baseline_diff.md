---
name: feedback_validate_baseline_diff
description: "When autospec validate is red on main, the only trustworthy merge signal is diffing your branch's failure SET against a clean origin/main worktree run — per-suite spot checks lie"
metadata: 
  node_type: memory
  type: feedback
  originSessionId: e10ccb2a-958b-4bd4-b423-d8b67cdf9582
  modified: 2026-08-05T21:33:41.695Z
---

`autospec validate` is red on `main` (issue #2962), so "did I break anything" cannot
be answered by looking at pass/fail. Run the FULL validate on a detached worktree at
`origin/main`, then compare the named failure sets. Merge only when they match
exactly.

**Why:** on 2026-08-05 this caught two real regressions that CI (7 green checks) and
231 green bats tests both missed. Spot-checking individual suites gave a **false**
all-clear:

- I ran `bats tests/docs` on both sides, got 10 failures each, and concluded
  `check_phase4_tests` was pre-existing. Wrong — `run_bash_directory` executes the
  `*.sh` files in that directory, not the bats files. The real failure was
  `tests/e2e/test_autospec_fleet_dry_run.bats`, run separately by the same check.
  The whole-run counts were the tell: main failed 2, my branch failed 3.

**How to apply:**

1. `git worktree add --detach <tmp> origin/main` once; keep it for the session.
2. Run `autospec validate` in both trees, capturing the real exit to its own file:
   `autospec validate > log 2>&1; printf '%s\n' "$?" > log.exit`.
   NEVER trust the harness/task exit — `cmd; echo; tail` reports `tail`'s status, and
   `| grep -c` exits 1 on a zero count. Both bit me in one session
   ([[feedback_background_pipeline_exit_masking]]).
3. Compare `head -1` (the `passed=/failed=` summary) and `grep ': failed'`.
4. To attribute a single check, read its owner in
   `crates/autospec-core/src/validation/catalog.rs`, then follow the
   `ExternalCheck::*` arm in `external.rs` to see what it actually runs. Do not guess
   from the check's name — `check_phase4_tests` runs shell scripts in two directories
   AND a bats e2e suite AND a fleet fixture gate.

**Two coupling traps the same session, both invisible per-skill:**

- **Guardian block lockstep is a CROSS-skill contract.** The block between
  `<!-- guardian-block:begin/end -->` must be byte-identical across six files, with
  `skills/autospec/SKILL.md` canonical. Editing it in `autospec-run` and running
  `derive-trio.sh` + `gen-skill-goldens.sh` on that skill makes every per-skill check
  pass while `check_phase4_guardian_block_lockstep` fails. Trio consistency and
  guardian lockstep are different invariants.
  See [[feedback_skill_golden_derivation_workflow]].
- **`examples/model-profiles.yml` is runtime data, not an example.**
  `fleet-config-lint.sh` resolves it as the authoritative profile catalog *ahead of*
  `~/.autospec/model-profiles.yml`, so renaming a profile key breaks
  `examples/fleet.yml`, the fleet skill's documented quickstart in all three mirrors,
  and four test suites. Grep every consumer of a sample-catalog KEY before renaming
  it.

**Gate scope gotcha:** the pre-commit gate scores the **whole working tree**, not the
index — `raw_files=5` with 3 files staged. Splitting an oversized change means
`git stash push -- <other files>`, not selective staging.
See [[feedback_precommit_gate_commit_shaping]].
