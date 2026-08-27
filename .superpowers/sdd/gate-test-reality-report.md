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

## Addendum: the fifth instance — verify-seeds.mjs (fixed)

`gate-stage-2-5.sh:69` had the same bug shape as run_metric()'s F/G/H/I
lookup, in a fifth, separate site:

```bash
VERIFY_SEEDS="$SCRIPT_DIR/../invariants/verify-seeds.mjs"
```

The real file lives at `$SCRIPT_DIR/seed-shapes/verify-seeds.mjs`. Unlike
run_metric(), this site has no stub-pass fallback JSON — it's guarded by a
bare `if [ -f "$VERIFY_SEEDS" ]`, so when the path was wrong the whole
`edge_case_seeds` check silently no-opped: no output, no log line, nothing
in the gate JSON to indicate seed verification was ever attempted. Fixed to:

```bash
VERIFY_SEEDS="$SCRIPT_DIR/seed-shapes/verify-seeds.mjs"
```

### How verify-seeds.mjs expects to be invoked

Read `skills/autospec-test/scripts/seed-shapes/verify-seeds.mjs` in full. It
is a CLI + programmatic-API dual-purpose script:

- **CLI (as invoked by gate-stage-2-5.sh):** it parses `--dsn`,
  `--store-kind`/`--store_kind`, and `--contract <path>` as **named argv
  flags** (`get('--flag')` does `args.indexOf(flag)` + next element). It
  does **not** accept a bare positional argument at all.
- **gate-stage-2-5.sh calls it as:** `node "$VERIFY_SEEDS" "$TARGET_DIR"` —
  a single bare positional argument, no `--contract`, `--dsn`, or
  `--store-kind` flags.

**Consequence, confirmed by direct execution** (`node
seed-shapes/verify-seeds.mjs /tmp/some-target-dir`): the positional
`$TARGET_DIR` is silently ignored by the flag parser, `contractPath` stays
`null`, so `contract` stays `{}`, and `run()` short-circuits on
`if (!seeds) return { violations: [], exit_code: 0, summary: 'No
edge_case_seeds block in contract; nothing to verify.' }` — **every single
time**, regardless of what the target's real contract declares. Now that
the path resolves, `verify-seeds.mjs` genuinely executes (proven by the new
test below), but it can never actually verify anything: it's wired up with
the wrong invocation contract (missing `--contract`, and no `--dsn`/
`--store-kind` selecting a real driver either) — the same class of "missing
wiring" gap as F/G/H's stdin-payload requirement, just manifesting as an
always-vacuous pass instead of an explicit refusal.

**Per instructions: not fixed.** No wiring was built and nothing was
stubbed into passing. This is reported alongside the F/G/H stdin-payload
gap and metric I's `playwright` `ERR_MODULE_NOT_FOUND` as a fourth/fifth
known, pre-existing, out-of-scope wiring gap for a human to address.

### A second, independent pre-existing bug surfaced while proving the test

While building the RED/GREEN proof for the new test, exercising the
now-reachable seeds block for the first time surfaced an unrelated bug in
the same block, also pre-existing (not introduced by this branch, not
fixed here — out of scope, flagged only):

```bash
if ! node "$VERIFY_SEEDS" "$TARGET_DIR" 2>&1; then
    SEED_EXIT=$?
```

`$?` immediately after `if ! cmd; then` reflects the exit status of the
*negated test* (always `0` on entry to the `then` branch), not `cmd`'s
actual exit code — confirmed directly: a stub `verify-seeds.mjs` that
`process.exit(2)`s still produces `SEED_EXIT=0` inside the block. This
means the `[ "$SEED_EXIT" -eq 2 ]` fatal-exit branch (`gate-stage-2-5:
fatal: edge_case_seeds verification refused to run`) is dead code — it can
never fire, no matter what verify-seeds.mjs exits with. Any non-zero exit
from verify-seeds.mjs instead silently falls through to
`ALL_PASSED=false`, which is a real (if less specific) failure signal, so
this bug degrades diagnostics rather than causing a silent pass. Flagged
for a human; not fixed (outside the single-path-string scope of this
task), and the new test below does not depend on that dead branch.

### New test: `skills/autospec-test/tests/unit/gate-stage-2-5.bats`

Added three tests:

1. `"seed-shapes/verify-seeds.mjs exists at the path the seeds check now
   resolves"` — static existence check, mirrors the four existing
   run_metric()-path tests.
2. `"gate-stage-2-5.sh no longer references the nonexistent
   ../invariants/verify-seeds.mjs path"` — grep guard on the source, using
   `run grep -q ...; [ "$status" -ne 0 ]` (not a bare non-final `!`, to
   avoid the bash 3.2.57 trap).
3. `"edge_case_seeds declared: verify-seeds.mjs is actually invoked, not
   silently skipped"` — the functional proof. Copies `scripts/` into a
   stub tree, overwrites the real `seed-shapes/verify-seeds.mjs` at its
   correct resolved path with a stub that writes a distinctive line
   (`stub-seed-verify-refused`) to stderr and exits non-zero, builds a
   target whose contract declares a non-empty `edge_case_seeds` block, runs
   the stub gate, and asserts the distinctive line appears in the gate's
   combined output. `gate-stage-2-5.sh` streams the runner's `2>&1` output
   straight through uncaptured, so this line can only appear if the stub
   was truly executed — under the old broken path (`[ -f ]` false) the
   whole block would be skipped and the line would never appear.

**Proof (RED):** Temporarily reverted `gate-stage-2-5.sh`'s `VERIFY_SEEDS`
line back to the old `../invariants/verify-seeds.mjs` path and reran both
new functional tests in isolation:
- `"...actually invoked, not silently skipped"` → RED:
  `` printf '%s' "$output" | grep -q 'stub-seed-verify-refused'' failed ``
  (the stub never ran; nothing printed).
- `"...no longer references the nonexistent ../invariants/verify-seeds.mjs
  path"` → RED: `` [ "$status" -ne 0 ]' failed `` (grep found the
  reintroduced string).

Restored `gate-stage-2-5.sh` from a `/tmp` backup and confirmed byte-identical
via `diff` (never `git checkout --`).

### Confirmed: this was the last instance of the bug shape

Re-grepped the full `skills/autospec-test/scripts/` tree:

- `grep -n 'SCRIPT_DIR/\.\.' skills/autospec-test/scripts/gate-stage-2-5.sh`
  → **zero matches** (the fixed line no longer uses `..` at all; the
  four run_metric() call sites already passed correct subdirectory-qualified
  relative paths).
- Every other `SCRIPT_DIR/..`-style lookup in the directory
  (`clone-gate-hook.sh`, `gate-stage-e2e.sh` ×2, `gate-stage-unit.sh`,
  `load-contract.sh`, `validate-contract.sh`) resolves to a real, existing
  path/file — verified each with `[ -e ]` — these are legitimate repo-root
  / sibling-skill lookups, not instances of this bug.
- Every `$SCRIPT_DIR/<relative-path>` construction across all
  `skills/autospec-test/scripts/*.sh` files (coverage collectors,
  `function-presence.mjs`, `playwright-config-resolver.mjs`,
  `forbidden-url-check.mjs`, `network-intercept-inject.mjs`,
  `ui-crawler.mjs`, `behavior-taxonomy-check.mjs`,
  `findings-generator.mjs`, `loop-budget.sh`, `loop-classifier.mjs`, and
  the gate scripts themselves) resolves to a real, existing file —
  confirmed with `[ -e ]` on all of them.

No further instances of "silent-skip via a wrong `$SCRIPT_DIR/..` runner
lookup" remain in this skill's scripts.

## Updated full regression run (foreground, real numbers)

`bats skills/autospec-test/tests/unit/` (all 13 `.bats` files):

```
1..193
193 ok, 0 not ok
exit code 0
```

(Up from 190 — the three new `verify-seeds.mjs` tests in
`gate-stage-2-5.bats`.)
