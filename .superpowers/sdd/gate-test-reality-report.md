# Gate Stage 2.5 test-reality audit — fix/gate-metric-runner-paths

## Background

`gate-stage-2-5.sh`'s `run_metric()` used to resolve runners at
`$SCRIPT_DIR/../invariants/<name>`, a directory that does not exist, so all
four metrics (F/G/H/I) silently took the stub-pass fallback for as long as
that code existed. The production fix (already landed on this branch, not
touched here) corrects the paths to the real per-metric subdirectories:
`invariants/`, `window-contract/`, `crawler-v2/`, `contract-symmetry/`, each
relative to `$SCRIPT_DIR`.

This report covers the test-side audit and fix requested for tests that were
written against the old broken-path behavior.

## File changed: `skills/autospec-test/tests/unit/gate-stage-2-5-coercion.bats`

### Test 5 — "no runners installed still defaults to passed:true (absent key)"

**Before:** `setup()` copied `scripts/` (including the real runner
subdirectories) into a stub tree, then separately created an *empty*
`$TEST_TMPDIR/invariants` directory at the old, buggy `../invariants` lookup
path. Under the old broken `run_metric()` resolution this empty directory is
what got probed, no runner was ever found there, and the stub-pass branch
fired — so the test passed for a reason that had nothing to do with genuine
runner absence. With the production path fix landed, `run_metric()` now
resolves the *real* per-metric subdirectory inside the stub copy, finds the
real (copied) runner files, executes them, and they fail (EOF on stdin /
`ERR_MODULE_NOT_FOUND` — the known, out-of-scope wiring gap), so the gate
exits non-zero and the test broke.

**After:** `setup()` no longer creates the stray `../invariants` directory.
The test itself now deletes the four real runner files from their correct,
resolved locations inside the stub copy
(`$STUB_SCRIPTS/invariants/run-structural.mjs`,
`$STUB_SCRIPTS/window-contract/run-window.mjs`,
`$STUB_SCRIPTS/crawler-v2/extended-crawler.mjs`,
`$STUB_SCRIPTS/contract-symmetry/run-symmetry.mjs`) — genuine absence at the
path the fixed script actually looks up — then asserts `passed:true` and
that the output contains the `"runner not installed"` stub-pass reason.

**Proof:** Restoring the runner files (i.e. reverting to "don't delete
them") and re-running the test in isolation goes RED:
`[ "$status" -eq 0 ]' failed` (gate now exits non-zero because the real,
un-deleted runners execute and refuse). Confirmed, then restored the file
byte-identical via `diff` against a backup (no `git checkout --` used).

### Tests 1–4 — "Metric {F,G,H,I} literal passed:false fails the gate"

**Audit finding:** These tests were also silently exploiting the same class
of bug, just less obviously. `make_failing_runner()` wrote its forced-fail
stub Node script into `$TEST_TMPDIR/invariants/<name>` — the old, wrong
sibling-of-`scripts/` path — which `run_metric()` never actually resolves
under either the old or the new code. Under the *new* (correct) path
resolution, the gate instead found and executed the **real runner** that
`cp -R` had copied into the stub tree. That real runner also fails (refuses,
exit 2, no stdin payload — the known out-of-scope wiring gap), which also
yields `passed:false` for metric F/G/H/I. So the tests still passed, but for
an accidental reason: they were incidentally exercising the "runner
refused" failure path, not the "coercion of a literal `passed:false` from a
metric's own JSON output" behavior the test names and file-header comment
describe. Had the stdin-wiring gap been fixed independently, these tests
would have started asserting on a real runner's actual pass/fail result
instead of a deliberately-planted stub — silently changing what they test
without any visible signal.

**Fix:** `make_failing_runner()` now writes the stub directly into
`$STUB_SCRIPTS/<relpath>` — the exact path `run_metric()` resolves for that
metric (e.g. `invariants/run-structural.mjs`) — overwriting the real copied
runner file with the stub. Each call site was updated to pass the correct
subdirectory-qualified relative path. This makes the test genuinely
exercise "a runner exists, is invoked, and emits a literal `passed:false` —
is that honored, not coerced to true," independent of whether the real
runner's stdin-wiring gap exists or ever gets fixed.

**Proof:** Temporarily changed the stub's emitted payload from
`passed: false` to `passed: true` and reran "Metric F literal passed:false
fails the gate" in isolation — went RED:
`` `[ "$f_passed" = "false" ]' failed ``. Confirmed, then restored the file
byte-identical via `diff` against a backup.

### Header comment

Updated the file's top-of-file comment block to describe the corrected
per-subdirectory path resolution (`$SCRIPT_DIR/<subdir>/<name>`) instead of
the old, no-longer-accurate `$SCRIPT_DIR/../invariants/<runner>.mjs`
description, and to describe the new stub placement (directly at the
resolved path, overwriting the real runner) instead of the old
sibling-directory approach.

## Sibling audit: no changes needed

### `skills/autospec-test/tests/unit/gate-stage-2-5.bats`

Already written *for* the corrected paths (it's this branch's own new
regression coverage). Verified all 16 tests pass for genuine reasons: the
bait-target tests (`target-invariant-bait`, `target-window-mismatch-bait`,
`target-greenwash-bait`, `target-contract-symmetry-bait`) explicitly assert
the gate output does **not** contain `"runner not installed"` — i.e. they
positively assert the real runners were reached — then separately assert
the gate fails overall (which happens for the correct, known,
out-of-scope reason: the runners refuse due to the stdin-wiring gap, not
because they silently stub-passed). No test here depends on or fakes the
old broken path. No changes made.

### `skills/autospec-test/tests/unit/run-gate-coercion.bats`

Both tests are self-contained and never exercise `gate-stage-2-5.sh`'s real
`run_metric()` path resolution at all:
- Test 1 (`S25_PASSED` coercion) replaces `gate-stage-2-5.sh` itself with a
  trivial stub script that unconditionally prints `passed:false` — it never
  invokes `run_metric()` or touches the runner directories.
- Test 2 (`RESTORE_SUCCEEDED` coercion) uses the `.autospec/stub-gate.json`
  short-circuit mechanism `run-gate.sh` already supports, bypassing gate
  execution entirely.

Neither test's premise depended on the runner-path bug. No changes made.
Verified both pass (2/2).

### `skills/autospec-test/tests/unit/pr-report-coercion.bats`

All 6 tests construct a synthetic gate JSON directly with `jq -n` and feed
it to `pr-report.sh --gate-json <file>` — `gate-stage-2-5.sh` and its
runners are never invoked. The `seeds_ok` field (also covered by this file)
is likewise synthesized directly, not read from a real
`verify-seeds.mjs` run. Not affected by the runner-path bug. No changes
made. Verified all 6 pass.

## Full regression run (foreground, real numbers)

`bats skills/autospec-test/tests/unit/` (all 13 `.bats` files in the
directory):

```
1..190
190 ok, 0 not ok
exit code 0
```

This includes the fixed `gate-stage-2-5-coercion.bats` (5/5), the sibling
`gate-stage-2-5.bats` (16/16), `run-gate-coercion.bats` (2/2), and
`pr-report-coercion.bats` (6/6), plus all other unit suites in that
directory (loop-controller, budget, wizard, function-presence, validate.sh
lockstep checks, etc.) — none regressed.

## Notes / things left alone (per instructions)

- `gate-stage-2-5.sh` itself was not modified.
- The stdin-payload wiring gap (F/G/H refuse — EOF on stdin) and metric I's
  `playwright` bare-specifier `ERR_MODULE_NOT_FOUND` were not touched, not
  stubbed into passing, and no gate/metric was weakened to hide them.
- Noticed but out of scope: `gate-stage-2-5.sh` line 69 still resolves
  `verify-seeds.mjs` at the old `$SCRIPT_DIR/../invariants/verify-seeds.mjs`
  path (the real file lives at `scripts/seed-shapes/verify-seeds.mjs`).
  None of the four audited test files exercise that code path (no test sets
  `edge_case_seeds` in its contract), so it caused no test-reality problem
  here — flagged for awareness only, not fixed (production script,
  out of scope).
