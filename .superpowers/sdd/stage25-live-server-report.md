# Stage 2.5 gate: live-server orchestration for metrics G and I

## Summary

`gate-stage-2-5.sh` now starts a real HTTP server for targets whose contract
declares `window_contracts` (metric G) or `contract_symmetry` (metric I),
polls it to readiness, uses it as `base_url`, and guarantees teardown on
every exit path. Metric F stays on `file://` and never gains a server.

Two real defects were found and fixed along the way (not worked around):
metric I's UI extractor had an inverted key/value convention that produced
zero tuples on every run, and a zero-tuple result was silently reported as
`passed:true` — the same fail-open shape this codebase has shipped
repeatedly elsewhere.

## Start-command discovery

`resolve_start_cmd()`: reads `.autospec/test.yml`'s `e2e.start_cmd` first
(explicit contract value wins); if absent, auto-detects from
`package.json` using **the exact same convention `autodetect.sh` already
uses** for this field — `scripts["start:e2e"] // scripts.dev`. Neither bait
target's `package.json` had a `start:e2e`/`dev` script (only `serve`), so
both targets now declare `e2e.start_cmd: "node src/server.mjs"` explicitly
in their `.autospec/test.yml`.

**Fixture metadata bugs found and fixed (not worked around):**
1. Both targets' `package.json` declared `"serve": "node server.mjs"` but
   the file lives at `src/server.mjs`. Since `server.mjs` resolves
   `index.html` via `import.meta.url`-relative `__dirname` (cwd-independent),
   the only bug was the wrong relative path in the `serve` script itself —
   fixed to `"node src/server.mjs"`.
2. Neither `server.mjs` bound loopback explicitly — plain
   `server.listen(PORT, cb)` binds all interfaces on Node by default,
   violating the localhost-only requirement. Fixed to
   `server.listen(PORT, HOST, cb)` with `HOST = process.env.HOST ??
   '127.0.0.1'`, and `start_live_server` passes `HOST=127.0.0.1`.
3. `target-contract-symmetry-bait/.autospec/test.yml`'s `must_be_editable`
   field was `'...editable == true'` — a trailing `== true` that
   `jsonpath-verifier.mjs`'s `assertBoolean()` does not parse (it evaluates
   the raw JSONPath and checks the result is `=== true` itself); the
   suffix is illustrative pseudocode from the design doc, not literal
   runner syntax. Fixed to a bare path ending in `.editable`.

## Port allocation and readiness

`find_free_port()` asks the OS for a free loopback port (`net.createServer`
bound to port 0, read back, released) rather than hardcoding one — proven
under both "another process holds the target's own default port" and
"two gate runs against the same target back to back" scenarios.
`wait_for_ready()` polls `curl -s -o /dev/null -m 2 <url>` every 0.2s up to
`AUTOSPEC_SERVER_READY_TIMEOUT_S` (default 15s), also failing fast if the
process dies mid-poll — never a fixed sleep.

## Teardown guarantee

`cleanup_live_server()` is registered via `trap cleanup_live_server EXIT INT
TERM` (this project's standing rule is no `RETURN` traps under `set -u`).
It sends TERM, polls up to 2s for exit, escalates to KILL, then `wait`s the
PID so it never persists as a zombie. `start_live_server` uses
`exec env PORT=... HOST=... $start_cmd` inside a `( cd "$target_dir"; ... )
&` subshell — `env` execs the target directly (no extra shell layer), so
the captured PID is the real server process, not a wrapper.

**Verified with correctly-scoped process detection.** `start_live_server`
runs `node src/server.mjs` as a *relative* command after `cd`ing into the
target directory, so the resulting process's argv is literally `node
src/server.mjs` — an absolute-path substring (e.g.
`$TARGET_DIR/src/server.mjs`) never appears in it. An earlier verification
pass using absolute-path `pgrep -f` patterns produced false "clean"
readings for a while: one bats test that intentionally neuters
`cleanup_live_server` to prove the RED case left a real orphan, but the
detection pattern couldn't see it, so it reported the guard "worked" for
the wrong reason. This was caught, three genuinely-orphaned processes
(all traced to that RED-teardown test only) were killed, and every
detection pattern in the script and the bats suite was corrected to
`pgrep -f "node src/server.mjs"`. Re-verified from scratch afterward:

- **Success path** (`target-window-mismatch-bait`, metric G runs and
  fails): gate exit 1, `pgrep -f "node src/server.mjs"` → no match, port
  3002 free.
- **Failure path** (server exits immediately, before readiness): gate
  reports `passed:false` with `"...exited before becoming ready"`, no
  process survives.
- **Readiness-timeout path** (server binds nothing, stays alive forever
  via `setInterval`): `AUTOSPEC_SERVER_READY_TIMEOUT_S=4`, gate exits 1 in
  4s (bounded, no hang), reports `passed:false` with
  `"...never answered at http://127.0.0.1:<port>/ within 4s"`, and —
  the decisive check, since this server does *not* exit on its own —
  `pgrep -f "node src/server.mjs"` finds nothing after the run.

Four RED proofs (mutated copies of `scripts/`, never the tracked file) back
these guarantees with real failing-first evidence: neutering
`cleanup_live_server` leaks the process, faking `wait_for_ready` to always
return success lets an unready server through undetected, hardcoding the
port causes a real collision against an occupied port, and removing the
zero-tuple guard (below) lets a broken selector silently pass.

## Metric I — root-cause diagnosis and fix (the coordinator's finding)

**Symptom:** `tuples_checked: 0`, `passed: true` against a live,
correctly-responding server — a vacuous pass on a bait designed to fail.

**Root cause 1 — inverted `per_match` convention in
`contract-symmetry/ui-extractor.mjs`.** The design spec
(`docs/specs/2026-05-21-autospec-test-invariants-design.md`) and the
target's own contract both declare `per_match: { task_id: 'data-task-id',
date: 'data-date' }` — object **key** = logical field name (used later in
`${task_id}` interpolation), **value** = DOM attribute to read. The
extractor's loop destructured the opposite:
`for (const [attrName, tupleKey] of Object.entries(per_match))`, calling
`el.getAttribute("task_id")` (the logical key — never a real attribute)
instead of `el.getAttribute("data-task-id")`. Every element's first
attribute read returned `null`, every tuple was skipped, extraction always
produced 0 tuples. Diagnosed by running `extract()` directly against the
live bait server and reading its own `[ui-extractor] warn: ... missing
attribute "task_id"` diagnostic. **Fixed** by swapping the destructuring
order to `for (const [tupleKey, attrName] of Object.entries(per_match))`.
The internal unit test `tests/unit/v2/run-symmetry.test.mjs` had encoded
the *same* inverted convention in its own fixtures (`{'data-task-id':
'task_id'}`) — a case of a self-consistent fixture masking the bug — and
was corrected to the spec convention alongside the fix.

**Root cause 2 — the systemic fail-open shape (the coordinator's
required fix).** Even with extraction fixed, `contractPassed =
violations.length === 0` was trivially true whenever `tuples.length ===
0` for any reason (broken selector, page not rendered, wrong route) — a
check that examined nothing reported success, indistinguishable from a
check that examined everything and found no problems. This is the same
family as a metric skipped-but-marked-passed, a jq `// true` coercion of a
real `false`, and an app harness reporting `started_process: true` for a
process that was never started. **Decision: a zero-tuple result is a
hard failure (`passed:false`), not a loud skip.** Unlike F/G/I's
"structural incapability of the wiring" skips (no static fixture, no
live-server start command discoverable), a live page that loaded
successfully but matched zero elements for a declared selector is a
finding about *this contract* — the operator needs it surfaced as a
failure, not shrugged past. Implemented in `run-symmetry.mjs`: after
extraction, `if (tuples.length === 0)` now pushes an explicit
`{phase: 'ui_extract', reason: '...selector "..." matched 0 elements at
route "..."; a contract that examined nothing cannot be reported as
passing'}` violation and marks the contract `passed:false`.

**A third, smaller divergence surfaced once extraction worked**: with
both `must_contain` and `must_be_editable` checked independently, task
t-3 (whose event genuinely doesn't exist) produced *two* violations
instead of the golden's one — checking "is it editable" on a record that
was already proven not to exist is redundant. Fixed by skipping
`must_be_editable` once `must_contain` already failed for the same tuple.

**Regression pin.** `gate-stage-2-5-live-server.bats` adds "zero tuples
extracted: run-symmetry.mjs reports passed:false, not a vacuous pass"
(exercises the real gate + real server, asserts `passed:false`,
`tuples_checked:0`, a `ui_extract`-phase violation mentioning "matched 0
elements", and full teardown) plus a RED proof that removes the guard from
a stub copy and confirms the same scenario reports `passed:true` without
it — proving the pinned test has power, not just green.

## Golden comparisons

**Metric F** (`target-invariant-bait`, unaffected regression check):
`passed:false`, `count_observed:5`, `route:"/"`, violation `index:4` — all
match the golden exactly. No server spawned (confirmed via scoped process
check); F stays on `file://`.

**Metric G** (`target-window-mismatch-bait`) — structurally exact match:

| field | actual | golden |
|---|---|---|
| `passed` | `false` | `false` |
| `contracts[0].id` | `dashboard-streak-window` | same |
| `contracts[0].passed` | `false` | `false` |
| `contracts[0].N` | `7` | `7` |
| `contracts[0].requests_seen` | `1` | `1` |
| `summary` | `{total:1, passed_count:0, failed_count:1, violation_count:1}` | identical |

Only divergence: violation-object field names (`expected`/`observed`/
`diff_days` vs golden's `expected_offset_days`/`observed_offset_days`) —
prose/shape from the shared window-contract module, same precedent already
accepted for metric F. Not fixed; not a structural (`passed`/count/index)
divergence.

**Metric I** (`target-contract-symmetry-bait`) — after both fixes above,
now a structurally exact match:

| field | actual | golden |
|---|---|---|
| `passed` | `false` | `false` |
| `contracts[0].id` | `streak-task-must-be-editable` | same |
| `contracts[0].passed` | `false` | `false` |
| `contracts[0].tuples_checked` | `3` | `3` |
| `summary` | `{total:1, passed_count:0, failed_count:1, violation_count:1}` | identical |

Only divergence: the single violation's field shape (`contract_id`/
`tuple`/`api_url`/`phase`/`reason` vs golden's `ui_claim`/`reason`/
`api_response_summary`) — same shared-module generic-shape-vs-bait-narrative
prose precedent as F and G. Not fixed; not a structural divergence.

## Test results

- `bats skills/autospec-test/tests/unit/*.bats`: **219/219** (201 baseline
  + 18 net new — 19 new tests in `gate-stage-2-5-live-server.bats`, 1 net
  test-count reduction from consolidating outdated skip-assertion tests in
  `gate-stage-2-5.bats` into genuine-invocation assertions), run across
  several bounded foreground batches (full single-command runs exceed the
  tool's foreground window; no batch was itself allowed to time out
  silently — one did (300s→124) and was re-split and re-run to completion).
- `bash -n skills/autospec-test/scripts/gate-stage-2-5.sh`: OK.
- `shellcheck skills/autospec-test/scripts/gate-stage-2-5.sh`: only the
  pre-existing `SC2034 METRICS_JSON appears unused` warning (confirmed via
  `git show HEAD` — present before this change, unrelated to it).
- `node --check` on both modified `.mjs` files: OK.
- `skills/autospec-test/tests/unit/v2/run-symmetry.test.mjs` (a separate,
  pre-existing file, not part of the `.bats` suite): fails to run in this
  environment via plain `node --test` — `Cannot find package 'playwright'`
  — confirmed via `git stash` that this failure predates every change in
  this branch; it needs a local `node_modules/playwright` or NODE_PATH
  wiring this environment doesn't have. Its `per_match` fixtures were
  still corrected to the spec convention for whenever it *is* runnable,
  and its logic was independently re-verified by exercising the equivalent
  path through the real `gate-stage-2-5.sh` + `run-symmetry.mjs` (which
  import `playwright` via the absolute Homebrew path, not a bare
  specifier, and do run in this environment).

## Fixture/runner changes, listed

- `test-targets/target-window-mismatch-bait/package.json`:
  `serve: "node server.mjs"` → `"node src/server.mjs"` (wrong path).
- `test-targets/target-window-mismatch-bait/.autospec/test.yml`: added
  `e2e.start_cmd: "node src/server.mjs"`.
- `test-targets/target-window-mismatch-bait/src/server.mjs`: bind
  `127.0.0.1` explicitly instead of the implicit all-interfaces default.
- Same three changes mirrored for `target-contract-symmetry-bait`, plus
  its `must_be_editable` JSONPath fixed to drop the non-literal `== true`
  suffix.
- `scripts/gate-stage-2-5.sh`: live-server lifecycle (this task's core
  deliverable) + a `mktemp` portability fix (see below) + separating a
  metric runner's stdout from stderr in `run_metric()` so a runner's own
  diagnostic warnings never corrupt the JSON verdict parsed downstream.
- `scripts/contract-symmetry/ui-extractor.mjs`: fixed inverted
  `per_match` destructuring (root cause 1).
- `scripts/contract-symmetry/run-symmetry.mjs`: zero-tuple guard (root
  cause 2) + skip `must_be_editable` once `must_contain` already failed
  for the same tuple.
- `tests/unit/v2/run-symmetry.test.mjs`: `per_match` fixtures corrected to
  the spec convention (three occurrences).
- `tests/unit/gate-stage-2-5.bats`: outdated skip-assertion tests for G/I
  replaced with genuine-invocation assertions; header comment updated.
- `tests/unit/gate-stage-2-5-live-server.bats` (new): 19 tests covering
  start-command discovery, port allocation, readiness polling, teardown on
  all three exit paths, the G/I bait-catching verdicts, the zero-tuple
  regression pin, and four RED proofs.

## An incidental bug found and fixed along the way

macOS/BSD `mktemp` only substitutes a **trailing** run of `X`s — a literal
suffix after them (e.g. `...XXXXXX.log`) is not recognized as a template on
this platform, so `mktemp ".../name-XXXXXX.log"` silently created (or
collided on) a file named literally `...-XXXXXX.log` instead of a random
one. This caused a real, reproducible hang/failure under repeated
invocation. Both `mktemp` call sites in `gate-stage-2-5.sh` (the live
server's stdout/stderr log, and each metric runner's stderr-capture file)
were fixed to drop the suffix.
