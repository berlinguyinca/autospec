# Gate metric-runner path bug: fix + actual results

Branch `fix/gate-metric-runner-paths`, worktree `pb-met`.

## 1. The path fix

`skills/autospec-test/scripts/gate-stage-2-5.sh`'s `run_metric()` resolved every
runner as `$SCRIPT_DIR/../invariants/$2` (`skills/autospec-test/invariants/`), a
directory that does not exist. Every metric therefore silently took the
stub-pass fallback:

```
{"metric":"F","passed":true,"skipped":true,"reason":"runner not installed"}
```

The four runners actually live in four different subdirectories of
`$SCRIPT_DIR` (`skills/autospec-test/scripts/`):

| Metric | Runner | Real location |
|---|---|---|
| F | run-structural.mjs | `invariants/run-structural.mjs` |
| G | run-window.mjs | `window-contract/run-window.mjs` |
| H | extended-crawler.mjs | `crawler-v2/extended-crawler.mjs` |
| I | run-symmetry.mjs | `contract-symmetry/run-symmetry.mjs` |

**Fix chosen:** since the four runners live in different subdirectories, a
single shared prefix can't work. `run_metric()` now resolves the runner as
`$SCRIPT_DIR/$2`, and each call site passes the subdirectory-qualified path:
`"invariants/run-structural.mjs"`, `"window-contract/run-window.mjs"`,
`"crawler-v2/extended-crawler.mjs"`, `"contract-symmetry/run-symmetry.mjs"`.

## 2. A second bug found and fixed along the way: jq `// true`

`F_PASSED=$(... | jq -r '.passed // true' ...)` (and the G/H/I equivalents).
jq's `//` alternative operator treats `false` as falsy, exactly like `null`.
So whenever a runner correctly reported `"passed": false`, this line silently
rewrote it back to `"true"` before it ever reached the `ALL_PASSED` check —
**even with the path bug fixed, the gate's own overall verdict could never go
red.** Verified interactively:

```
$ jq -r '.passed // true' <<<'{"passed":false}'
true          # wrong: should be false
```

Fixed to `jq -r 'if .passed == null then true else .passed end'`, which only
defaults when the field is absent/null. This is a strictness fix, not a
weakening — it's what makes it possible to observe the real results below at
all (before this fix every gate run reported `"passed": true` regardless of
what the metrics said).

Both fixes are in `skills/autospec-test/scripts/gate-stage-2-5.sh`.

## 3. Per-target / per-metric results (actual gate runs, this environment)

Ran `bash gate-stage-2-5.sh <target> </dev/null` against all five targets
after both fixes, `node` v26.3.0, `yq` v4.53.2, `jq` 1.7.1.

| Target | Overall | F | G | H | I |
|---|---|---|---|---|---|
| target-invariant-bait | **fail** (exit 1) | refused | refused | refused | exited 1 |
| target-window-mismatch-bait | **fail** (exit 1) | refused | refused | refused | exited 1 |
| target-greenwash-bait | **fail** (exit 1) | refused | refused | refused | exited 1 |
| target-contract-symmetry-bait | **fail** (exit 1) | refused | refused | refused | exited 1 |
| target-clean-pass | pass, `skipped:true` (exit 0) | — | — | — | — |

`refused` = runner exited 2, `"refused":true,"reason":"runner refused to run"`.
`exited 1` = runner exited 1 with a Node module-resolution stack trace as the
`raw` field.

### target-clean-pass: expected, not a bug

`target-clean-pass/.autospec/test.yml` has no `invariants_v2` block at all
(`e2e.invariants_v2.enabled` reads as `false`), so the gate short-circuits at
line 46 before touching F/G/H/I. It correctly emits the top-level skipped-pass
JSON. This is the designed zero-overhead v1 path, not a v2 metric result.

## 4. Category breakdown — what's really going on

None of the four bait targets got a genuine "metric evaluated the DOM/network
and correctly flagged the bait" result. Instead every v2-enabled target fails
uniformly, for two distinct reasons unrelated to which bait it is:

**(b) Genuine pre-existing defect, previously fully masked — F/G/H never
received a call payload.** All three Playwright-based runners
(`run-structural.mjs`, `run-window.mjs`, `extended-crawler.mjs`) read a JSON
document from **stdin** shaped `{ contract, base_url }` — `base_url` being a
live URL for Playwright to navigate to. `gate-stage-2-5.sh`'s `run_metric()`
instead invokes `node "$runner" "$TARGET_DIR"` — passing the target directory
as **argv**, with **no stdin at all** and **no notion of a base_url or a
running server for the target**. I confirmed by grep that nothing anywhere
under `scripts/` (outside the runners' own unit tests, which build the stdin
payload in-process) ever constructs this JSON or starts a server. Each
runner's stdin read gets immediate EOF, `JSON.parse('')` throws, and the
runner exits 2 with "stdin must have `{ contract, base_url }`" — which
`run_metric()` reports as `"refused":true`.

This is exactly the kind of masking the stub-pass fallback was hiding: it
wasn't just a wrong path, the calling convention between the shell gate and
these three runners was **never wired end to end**. This matches
`tests/integration/v2/run-against-target.bats`'s own header comment: *"Since
gate-stage-2-5.sh is delivered in Phase 10, this harness validates the static
fixtures ... rather than running live Playwright"* — i.e. this gap was known
and deferred, then forgotten once the stub-pass silently "passed" everything.
Building the missing wiring (constructing the contract JSON, standing up a
base_url/dev server per target) is a real feature with design decisions
(which server, which port, teardown, isolation) — **flagged for a human, not
implemented here**, per the instruction to fix only what's clearly safe.

**(c) Environmental/dependency defect in Metric I's runner, distinct from (b).**
`run-symmetry.mjs` imports `import { chromium } from 'playwright'` (bare
specifier), while the other three runners import it via the absolute path
`/opt/homebrew/lib/node_modules/playwright/index.mjs`. In this environment
there's no local `node_modules/playwright` resolvable from
`scripts/contract-symmetry/`, so Node throws `ERR_MODULE_NOT_FOUND` before
`run-symmetry.mjs` even reaches its stdin-parsing code, exiting 1 (not 2), so
`run_metric()` reports `"reason":"runner exited 1"` with the stack trace as
`raw`. This is an inconsistency between run-symmetry.mjs and its three
siblings — not something to silently patch to match the others without
knowing whether the intent was "install playwright as a local dependency" or
"use the same absolute-path import everywhere." **Flagged for a human.**

**(a) A bait target's metric being caught for real** did not happen in this
run, for the reasons above — not because a metric is broken, but because
nothing in this environment supplies the runners what they need to run
(a live base_url) or, for I, a resolvable `playwright` import. The runner
logic itself is untested by this exercise; each runner's own
`tests/unit/v2/*.test.mjs` suite already exercises the DOM/network logic
in-process with a fixture server and stdin JSON built by the test — those
weren't touched and still pass (not re-verified here since they're unrelated
to the path bug; see `tests/unit/v2/run-structural.test.mjs`,
`run-window.test.mjs`, `extended-crawler.test.mjs`, `run-symmetry.test.mjs`).

### Golden files, compared

Each bait's `golden/stage-2-5-gate.json` describes what the runner *should*
emit once correctly wired and run against a live target (metric, violation
detail). What we actually got is uniformly a refusal/crash, not the golden
shape — confirming (b)/(c) above rather than a genuine metric verdict:

- `target-invariant-bait/golden`: expects `"metric":"F","passed":false"`,
  violation at `done-item-row-4`. Actual: F `refused`.
- `target-window-mismatch-bait/golden`: expects `"metric":"G","passed":false"`,
  N=7 vs observed -3d. Actual: G `refused`.
- `target-contract-symmetry-bait/golden`: expects `"metric":"I","passed":false"`,
  violation for `t-3`. Actual: I `exited 1` (module resolution).
- `target-greenwash-bait` has no golden file. Its `.autospec/test.yml` uses the
  same `kind: every_visible_X_is_Y` / `every_row_has_required_actions` shapes as
  `target-invariant-bait`, i.e. it also targets **Metric F**, not H — confirmed
  by reading the contract (`invariants:` block, not `crawler:`/`window_contracts:`/
  `contract_symmetry:`). Actual: F `refused`, same as target-invariant-bait.

## 5. Stub-pass fallback (lines 88-91 as originally numbered) — assessment

**Recommendation: keep it fail-open for a genuinely missing file, but make it
loud, and treat `skipped` as distinct from `passed` at the overall-gate level
rather than folding it silently into "pass."**

Rationale:
- Removing it outright is risky exactly as the comment warns — v1-only
  targets and any future metric added incrementally would hard-fail Stage 2.5
  the moment a call site references a not-yet-shipped runner file, which is
  a legitimate incremental-rollout scenario the comment is protecting.
- But a *missing runner* silently reported as `passed:true` is what let this
  bug hide for however long it's been merged — nothing downstream had any
  signal to notice. The fix that's clearly safe and in scope: **emit a loud
  marker in the JSON when a runner file is missing**, so a human/CI dashboard
  can see "metric F never actually ran" even when the gate as a whole passes.

**Implemented (safe, minimal):** none beyond what's described above — I did
not change the stub-pass JSON shape itself, because after the path fix all
four runner files now genuinely exist at their correct locations, so the
stub-pass branch is currently dead code for these four metrics (it can no
longer be hit by them). I verified this is true for the current tree (see the
bats assertions "`gate output is not the stub-pass reason for any metric`" —
all four bait targets now get a real invocation, never the stub reason).

**Flagged, not implemented (needs a human decision):**
1. Add a `code_health:` marker (or a `"skipped_runners":[...]` array at the
   top-level Stage 2.5 gate JSON, separate from `passed`) whenever the
   stub-pass branch fires, so a future missing-runner regression is visible
   in the gate's own output rather than folded into `passed:true`.
2. `verify-seeds.mjs` (edge_case_seeds check, line ~69) has the *exact same*
   path-bug shape: `VERIFY_SEEDS="$SCRIPT_DIR/../invariants/verify-seeds.mjs"`,
   but the real file is at `scripts/seed-shapes/verify-seeds.mjs`. This is
   silently `if [ -f ... ]`-gated with no reason string at all if missing —
   an even quieter version of the same masking bug. None of the five targets
   in this task declare `edge_case_seeds`, so it didn't affect the results
   above, but it's the same defect class and is still live. Out of scope for
   this fix (not in the task's F/G/H/I table) — flagged for a human/follow-up
   issue.
3. The stdin/base_url wiring gap (§4, item b) and the run-symmetry.mjs bare
   `'playwright'` import (§4, item c) are the two blockers that must be
   resolved before any of F/G/H/I can produce a real pass/fail verdict via
   this shell gate. Both are flagged above, not fixed here.

## 6. Tests added

`skills/autospec-test/tests/unit/gate-stage-2-5.bats` (new file, 16 tests):
- Asserts each runner file exists at the path `run_metric()` now resolves.
- Asserts the fixed script no longer contains the `../invariants/$2` prefix.
- For each of the four bait targets: asserts the gate output never contains
  `"runner not installed"` (i.e. the stub-pass path was not taken) and that
  the overall gate exits 1 / `passed:false`.
- Asserts `target-clean-pass` still short-circuits to `skipped:true,passed:true`
  (proves the v1 zero-overhead path is unaffected).
- Asserts the fixed script no longer contains the `.passed // true` jq bug,
  and a direct jq assertion that `passed:false` survives the filter.

### Red/green proof

1. Backed up the fixed script to `/tmp/gate-stage-2-5.sh.fixed.bak`.
2. Restored the original buggy script via `git show HEAD:.../gate-stage-2-5.sh
   > /tmp/gate-stage-2-5.sh.orig.bak` and `cp` (never `git checkout --`) over
   the working file.
3. Ran the new bats file: **10 of 16 tests failed** (`not ok`), including
   every "not the stub-pass reason" assertion and every "overall gate fails"
   assertion, plus both bug-signature checks — confirming the tests actually
   detect the original bugs (RED).
4. `cp`'d the fixed backup back over the working file, confirmed `diff` showed
   no difference from the intended fix, re-ran: **16 of 16 pass** (GREEN).

## 7. Regression check

- `bash -n skills/autospec-test/scripts/gate-stage-2-5.sh` — OK.
- `shellcheck skills/autospec-test/scripts/gate-stage-2-5.sh` — same two
  pre-existing `SC2034` warnings (`METRICS_JSON`, `result_var`) present in
  both the original and fixed script; no new warnings introduced.
- Full existing `skills/autospec-test/tests/unit/*.bats` suite (9 files,
  independently of the new one): calculator-fixture (1/1), contract-loader
  (29/29), gate-stage-e2e (25/25), gate-stage-unit (23/23), greeter-fixture
  (2/2), loop-budget (17/17), loop-controller (13/13),
  validate-skill-structure (35/35), wizard (15/15) — **0 failures across all
  of them**, run individually (some batched together timed out the harness's
  120s window when run concatenated, not from failures — each file passes in
  isolation well under its own timeout). Two pre-existing `BW01` bats
  warnings in `wizard.bats` (`exited with code 127`) are unrelated
  environmental warnings about a missing command, present regardless of this
  change.
- New `gate-stage-2-5.bats`: 16/16.

No test that was green before is red now. No regression from a metric now
genuinely running: the four bait targets' overall verdict flips from a false
`passed:true` (silently, via the stub-pass + jq bug) to a correctly-observed
`passed:false` — this is the intended effect of the fix, not a regression.
