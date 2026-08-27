# Gate Stage 2.5 metric wiring: F/G/H/I invocation contract

Branch `feat/gate-metric-wiring`, worktree `pb-wire2`. Builds on the prior
path fix (`gate-metric-paths-report.md`) which made `run_metric()` resolve
each runner's real file location but left the calling convention (stdin
payload, base_url, seed-verify flags) unbuilt.

## 1. Each runner's expected payload, and how it's now constructed

All four runners (`invariants/run-structural.mjs`, `window-contract/run-window.mjs`,
`crawler-v2/extended-crawler.mjs`, `contract-symmetry/run-symmetry.mjs`) read
one JSON document from **stdin**: `{ contract, base_url, route_list?,
custom_kinds_dir? }`. `route_list` is documented in run-structural.mjs's
header comment but never referenced in its implementation (dead/aspirational
— not usable as a lever).

`gate-stage-2-5.sh` now:
1. Parses `.autospec/test.yml` to JSON once via `yq -o=json '.' "$CONTRACT_YML"`
   → `CONTRACT_JSON`.
2. Builds a shared `{contract: $CONTRACT_JSON, base_url: $BASE_URL}` payload
   (`build_payload()`) and pipes it to each runner's stdin:
   `printf '%s' "$payload" | node "$runner"` (previously: `node "$runner"
   "$TARGET_DIR"` — a bare positional arg, no stdin at all, which is why
   every runner used to exit 2 with "stdin must have {contract, base_url}").
3. `run_metric()` now takes a 4th arg — a non-empty skip reason means "skip
   without invoking the runner at all, with this loud reason" (used when the
   wiring genuinely cannot supply a usable payload — see §2).
4. A runner exiting 1 with well-formed JSON on stdout (each runner does
   `process.exit(passed ? 0 : 1)`) is now passed through as-is instead of
   being flattened into the generic `"reason":"runner exited 1"` fallback —
   that fallback is now reserved for genuine crashes/non-JSON output. This
   was a real bug in the previous path-fix pass: a *correct* failing verdict
   was being masked exactly like a crash.

`verify-seeds.mjs` is a separate CLI with `--contract`/`--dsn`/`--store-kind`
named flags (not a positional target dir — passing one made it silently fall
through to "No edge_case_seeds block in contract; nothing to verify."). Fixed
call: `node "$VERIFY_SEEDS" --contract "$CONTRACT_YML" --dsn
"${AUTOSPEC_SEED_DSN:-:memory:}" --store-kind
"${AUTOSPEC_SEED_STORE_KIND:-sqlite}"`. There is no established plumbing
anywhere in this codebase yet from a contract to a real Mode II clone DSN
(grepped; only verify-seeds.mjs itself references dsn/store_kind), so the
defaults are an in-memory sqlite store, overridable via env vars for a future
caller that does have a live clone. None of the five test targets declare
`edge_case_seeds`, so this path doesn't affect any golden comparison — it's
exercised only by the bats fixtures.

## 2. Route → URL mapping, and its limits

**Supported:** a target with a static `<target>/src/index.html` (checked
first) or `<target>/index.html` fixture, reachable only at route `"/"`.
`base_url = "file://$WEB_ROOT/index.html#"`. Each runner builds its
navigation URL as `baseUrl.replace(/\/$/, '') + route`; a file:// directory
URL has no automatic index.html resolution in Chromium (verified empirically
— it renders a directory listing), so pointing base_url at a bare directory
does not work. Pointing it directly at `index.html#` (no trailing slash, so
the regex doesn't strip it) makes route `"/"` land as a URL *fragment*
(`.../index.html#/`), which the browser ignores for resource resolution —
verified with a real headless Chromium navigation before wiring this into
the gate.

**Not supported, and detected + skipped loudly rather than half-implemented:**
- No static index.html found under `<target>/src` or `<target>/` → F, G, H,
  I all skip (each needs a `base_url`, and this wiring stands up no dev
  server).
- `window_contracts` declared and non-empty → **metric G always skips**. G
  needs an *observed live network request* (`request-recorder.mjs` intercepts
  real HTTP traffic); a file:// page cannot produce one against a backend
  that doesn't exist.
- `contract_symmetry` declared and non-empty → **metric I always skips**,
  same reason (`page.request[method](apiUrl)` needs a real HTTP endpoint).
- Any invariant's `apply_on_routes` references a route other than `"/"` →
  metric F skips (no per-route static fixture to map to; only root is
  supported).

Each skip emits `{"metric":X,"passed":true,"skipped":true,"reason":"..."}`
with a specific, human-readable reason — never silently folded into
`passed:true` with no signal, and never confused with the pre-existing
"runner not installed" stub-pass path (verified: no golden-target run emits
that string).

**Targets this wiring fully supports:** `target-invariant-bait` (single
static page, root route, metric F only) — genuinely wired end to end.
**Targets it does not:** `target-window-mismatch-bait` and
`target-contract-symmetry-bait` (both ship their own `src/server.mjs` — they
were built assuming live-server orchestration) and `target-greenwash-bait`
(no static frontend at all; `apply_on_routes: ['/peaks']` with nothing to
serve it). `target-clean-pass` has no `invariants_v2` block and is unaffected
either way (v1 zero-overhead skip path).

## 3. Per-target golden comparison

### target-invariant-bait — metric F, genuinely invoked, bait caught for real

Actual `.metrics.F`:
```json
{
  "metric": "F", "passed": false,
  "invariants": [{
    "id": "done-items-editable", "kind": "every_visible_X_is_Y", "route": "/",
    "passed": false,
    "violations": [{"index": 4, "selector": "role=button[name=/edit/i]", "reason": "action not visible"}],
    "count_observed": 5
  }],
  "summary": {"total":1,"passed_count":0,"failed_count":1,"violation_count":1}
}
```
Golden (`target-invariant-bait/golden/stage-2-5-gate.json`):
```json
{
  "metric":"F","passed":false,"target":"target-invariant-bait",
  "invariants":[{"id":"done-items-editable","route":"/","passed":false,
    "violations":[{"index":4,"selector":"[data-testid^=\"done-item-row-\"]",
      "reason":"edit button not visible — row renders as span not button"}],
    "count_observed":5}],
  "summary":{"total":1,"passed_count":0,"failed_count":1,"violation_count":1}
}
```
**Match:** `passed:false`, `count_observed:5`, violation at `index:4`,
`route:"/"`, `id`, `summary` — every field the task calls out as the
acceptance criterion matches exactly. The bait is genuinely caught.

**Diverges (cosmetic, not semantic):**
- Golden nests `"target"` inside the F object; the real orchestrator only
  puts `"target"` at the outer gate envelope (`.target`, not
  `.metrics.F.target`) and nests F under `.metrics.F`. The golden's flattened
  shape doesn't match either level literally.
- Actual has an extra `"kind"` field (additive, harmless).
- `violations[0].selector`: golden says `[data-testid^="done-item-row-"]`
  (the row/container selector); actual says `role=button[name=/edit/i]` (the
  action selector) — this is the shared `every_visible_X_is_Y` kind module's
  own fixed code (`invariants/kinds/every-visible-x-is-y.mjs`, line ~60):
  `violations.push({ index: i, selector: actionSel, reason: 'action not visible' })`.
- `violations[0].reason`: golden's narrative prose ("edit button not visible
  — row renders as span not button") vs. the kind's generic, reusable message
  ("action not visible").

**Verdict: the runner is correct, the golden's flavor text is illustrative,
not literal.** `every_visible_X_is_Y` is a shared kind used by other targets
too (e.g. `target-greenwash-bait`'s `peak-items-have-edit-button`); baking
bait-specific narrative prose into it would make every other invariant using
this kind report the same generic-but-wrong text, which is worse. `index: 4`
already uniquely identifies which row failed; the selector/reason fields are
supplementary diagnostics, and I did not change the kind module to chase this
one golden's prose. Flagging this text mismatch for a human decision rather
than silently rewriting either side.

### target-window-mismatch-bait — metric G

Golden expects `"metric":"G","passed":false"`, N=7 vs observed offset -3d
(a real network-observed mismatch). Actual: G is **skipped**
(`{"metric":"G","passed":true,"skipped":true,"reason":"window_contracts
require a live HTTP server to observe a real network request; this wiring
only supports static file:// fixtures and does not stand up a dev server"}`),
gate overall **passes**.

**Verdict: expected divergence, not a bug.** The target ships
`src/server.mjs` (an Express-style Node HTTP server on port 3002) — it was
built assuming a live dev server would be running. The task explicitly
scoped this wiring to "do not build dev-server orchestration... if a target
genuinely needs a live server, detect it and skip with a clear, loud reason."
That's exactly what happens. Neither the runner (`run-window.mjs`, unchanged,
correctly implements the golden's logic — verified by reading its date-math
and network-recorder code) nor the golden is wrong; the wiring's scope is
narrower than what this particular bait needs.

### target-contract-symmetry-bait — metric I

Golden expects `"metric":"I","passed":false"`, violation for `t-3` (API
returns empty events; UI claims it exists). Actual: I is **skipped**
(`"reason":"contract_symmetry requires a live HTTP server to fetch and
compare API responses; ..."`), gate overall **passes**. Same shape and same
verdict as G above — this target also ships its own `src/server.mjs`
(port 3003, deliberately omitting `t-3` from `/api/household/timeline`).
Expected divergence, not a bug in the runner, the wiring, or the golden.

### target-greenwash-bait — no golden, sanity-checked

No `src/index.html` exists at all (only `src/peak_detector.ts`, a Jest unit
under test — this target's `invariants_v2` block references `apply_on_routes:
['/peaks']`, a route that was never shipped as a static page). All four
metrics skip with `"reason":"no static index.html found under <target>/src
or <target>/; ...")`. Gate overall **passes**. Sensible: there is nothing for
a file://-only wiring to navigate to; a false "caught the bait" or false
"refused" would both have been worse than an honest skip.

### target-clean-pass — no golden, sanity-checked

No `invariants_v2` block in `.autospec/test.yml` → the gate short-circuits at
its existing top-level check before touching F/G/H/I:
`{"metric":"2.5","skipped":true,"passed":true,"reason":"invariants_v2.enabled
!= true"}`. Passes, as expected — this is the pre-existing v1 zero-overhead
path, untouched by this wiring.

## 4. The `SEED_EXIT` bug

`if ! node "$VERIFY_SEEDS" ... ; then SEED_EXIT=$?` captured the exit status
of the **negated `if !` test**, not node's own exit code — that status is
always 0 (bash re-evaluates `$?` after the `if`'s own conditional evaluates,
and the `!`-negated test's result, once captured by `if`, resets `$?`). So
`[ "$SEED_EXIT" -eq 2 ]` could never be true; the fatal-exit branch was dead
code regardless of what verify-seeds.mjs actually exited with. Fixed by
running node outside any `if`/`!` and capturing `$?` directly:
```sh
SEED_EXIT=0
node "$VERIFY_SEEDS" --contract ... || SEED_EXIT=$?
if [ "$SEED_EXIT" -eq 2 ]; then ...; exit 2; fi
```
New test `SEED_EXIT fix: verify-seeds.mjs exiting 2 makes gate-stage-2-5.sh
exit 2 (fatal), not silently continue` (`tests/unit/gate-stage-2-5.bats`)
stubs verify-seeds.mjs to always exit 2 and asserts the gate itself now exits
2 with the fatal message — this is the previously-dead branch, now reachable.

## 5. Red/green proofs

- **F genuine-invocation tests**: backed up the fixed `gate-stage-2-5.sh` and
  `run-structural.mjs`/`run-symmetry.mjs`, restored the pre-wiring versions
  via `git show`+`cp` (never `git checkout --`), reran
  `tests/unit/gate-stage-2-5.bats` and `tests/unit/gate-stage-2-5-coercion.bats`
  — the payload/verdict assertions and the SEED_EXIT test failed (RED), then
  `cp`'d the fixed files back, `diff` confirmed byte-identical restoration,
  reran — all green.
- **`RED proof: an empty base_url makes metric F refuse instead of catching
  the bait`** test directly demonstrates the wiring's own payload-construction
  failure mode (an empty `base_url` in the stdin payload) drives the exact
  same runner to exit 2/"refused" instead of a real verdict — proving the
  passing "genuinely caught the bait" tests actually depend on the base_url
  being correctly constructed, not just on the runner file existing.
- **SEED_EXIT proof**: stubbed `verify-seeds.mjs` to `process.exit(2)`; before
  the fix the gate fell through (`ALL_PASSED=false`, no fatal exit, no fatal
  message); after the fix the gate exits 2 and prints
  `"fatal: edge_case_seeds verification refused to run"`.

## 6. Test results

`skills/autospec-test/tests/unit/*.bats`: **201/201 passing** (baseline was
193/193; net +8 from the new/expanded gate-stage-2-5.bats and
gate-stage-2-5-coercion.bats cases). `bash -n
skills/autospec-test/scripts/gate-stage-2-5.sh`: clean. `shellcheck`: one
pre-existing `SC2034` (`METRICS_JSON` unused) — present before this change,
not introduced by it, left as-is (out of scope).

`node --test tests/unit/**/*.test.mjs tests/unit/*.test.mjs`: 340/344 passing.
The 4 failures are **pre-existing and unrelated to this work** (none touch
`gate-stage-2-5.sh`, `run-structural.mjs`, `run-window.mjs`,
`extended-crawler.mjs`, or the import line changed in `run-symmetry.mjs`):
- `tests/unit/v2/run-symmetry.test.mjs` and `tests/unit/v2/verify-seeds.test.mjs`
  fail with `ERR_MODULE_NOT_FOUND` for `playwright` and `better-sqlite3`
  respectively — the test files themselves import these bare specifiers, and
  neither package is installed in this worktree's `node_modules` (no
  `npm install` has been run here). Unrelated to the `run-symmetry.mjs`
  source-file import fix in §7 below — that fix only changes the *runner*'s
  own import, not the test file's.
- `tests/unit/reset-endpoint-gen.test.mjs` ("share one reset-logic
  placeholder definition") and `tests/unit/v2/run-window.test.mjs`
  ("tolerance_days=3: mismatch of 4 days fails") fail against the untouched
  pre-existing code in this worktree — confirmed by `git diff --stat`, which
  shows only `run-symmetry.mjs`, `gate-stage-2-5.sh`, and the two
  `gate-stage-2-5*.bats` files changed.

## 7. Metric I's playwright import

Fixed `contract-symmetry/run-symmetry.mjs`'s `import { chromium } from
'playwright'` (bare specifier — unresolvable in this environment, no local
`node_modules/playwright`) to the same absolute path its three siblings use:
`/opt/homebrew/lib/node_modules/playwright/index.mjs` (confirmed present on
this machine). Verified working: target-invariant-bait's gate run now gets a
real (empty, correctly so — no `contract_symmetry` declared) I verdict
instead of a module-resolution crash.
