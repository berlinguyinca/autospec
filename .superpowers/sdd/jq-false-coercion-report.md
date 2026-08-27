# jq `//` false-coercion gate bug — fix report

## The bug

jq's `//` is an *alternative* operator: it treats `null` **and** `false` as
"no value" and substitutes the right-hand side. `.passed // true` therefore
silently turns a real `passed: false` into `true`. Verified live:

```
$ echo '{"passed":false}' | jq -r '.passed // true'
true
```

Every `// true` site that reads a computed pass/fail (or an explicit
config toggle) from JSON/YAML is affected. `// false` sites are harmless
(false stays false either way).

## Independently derived site list

Re-derived via `grep -rn '// true\|// false\|// "true"\|// 1\b'` across
`scripts/` and `skills/`, then manually inspected each hit. 12 sites use the
dangerous `// true` shape on a computed result or an explicit toggle:

| # | File | Line (before fix) | Field |
|---|------|------|-------|
| 1 | `scripts/gen-pr-report.sh` | 170 | `.passed` (drift file) |
| 2 | `skills/autospec-test/scripts/gate-stage-unit.sh` | 87 | `.unit.function_presence_check` |
| 3 | `skills/autospec-test/scripts/run-gate.sh` | 116 | `.passed` (stage 2.5 result) |
| 4 | `skills/autospec-test/scripts/run-gate.sh` | 188 | `.stage2.metrics.restore_succeeded` |
| 5 | `skills/autospec-test/scripts/gate-stage-2-5.sh` | 110 | Metric F `.passed` |
| 6 | `skills/autospec-test/scripts/gate-stage-2-5.sh` | 114 | Metric G `.passed` |
| 7 | `skills/autospec-test/scripts/gate-stage-2-5.sh` | 118 | Metric H `.passed` |
| 8 | `skills/autospec-test/scripts/gate-stage-2-5.sh` | 122 | Metric I `.passed` |
| 9 | `skills/autospec-test/scripts/pr-report.sh` | 89 | `.stage2_5.metrics.F.passed` |
| 10 | `skills/autospec-test/scripts/pr-report.sh` | 91 | `.stage2_5.metrics.G.passed` |
| 11 | `skills/autospec-test/scripts/pr-report.sh` | 93 | `.stage2_5.metrics.H.passed` |
| 12 | `skills/autospec-test/scripts/pr-report.sh` | 95 | `.stage2_5.metrics.I.passed` |
| — | `skills/autospec-test/scripts/pr-report.sh` | 97 | `.stage2_5.seeds_ok` |

(Sites 9–13 are 5 distinct lines in one file — the task brief's "12 sites"
undercounts pr-report.sh's own 5 by one; the independently-derived total is
13 dangerous `// true` lines across 5 files. All 13 are fixed.)

This matches, and is a superset-consistent re-derivation of, the site list
given in the task brief.

## Canonical fix form chosen

```jq
if .X == false then false else true end
```

applied verbatim at each site (with the correct key path substituted).

**Why this form over `if has("passed") then .passed else true end`:** the
`has()` form breaks the moment a value is nested more than one level below a
possibly-absent parent object (e.g. `.stage2_5.metrics.F.passed` — `has()`
only tests the last key, and if `.stage2_5.metrics` itself is absent/null the
`has()` call errors instead of gracefully defaulting). `if .X == false ...`
degrades cleanly for absent, null, or any parent-missing case: jq's `==`
against a missing/null path evaluates to `null == false` → `false`, so the
`else true end` branch fires — the desired "absent defaults to true" behavior
— without any special-casing for nesting depth. It is also a strict
line-for-line, no-restructuring swap of the original `// true` expression, so
diffs stayed minimal (13 one-line changes, nothing else touched — confirmed
by `diff` against copies of the pre-edit files).

## Other `// true` forms found (same bug class), not fixed — reported here

The `grep` sweep also found `// true` on **config toggles read via `yq`**
(not gate pass/fail results):

- `skills/autospec-sweep/scripts/run.sh` — `.steps.review.enabled // true`,
  `.steps.run.enabled // true`, `.sweep.spec_sync.enabled // true`,
  `.continuous_improvement.loop.file_issues // true`,
  `.continuous_improvement.loop.route_fixes_via_autospec_run // true`,
  `.continuous_improvement.tests.enabled // true`,
  `.execution.tests.run_all_every_sweep // true`,
  `.execution.deployment.deploy_if_tests_require // true`
- `skills/autospec-sweep/scripts/review.sh` — same shapes
  (`spec_sync.enabled`, `docs.enabled`, `documentation.enabled`,
  `tests.enabled`, `code.enabled`, `deploy_if_tests_require`,
  `require_scope` x2)
- `skills/autospec-fleet/scripts/fleet-run.sh`,
  `fleet-status.sh`, `fleet-stop.sh` — `.repos[$idx].enabled // true`

**Confirmed live that `yq` (mikefarah/yq v4.53.2) has the identical bug**:

```
$ echo '{"enabled": false}' | yq -r '.enabled // true'
true
```

So a user who explicitly sets `enabled: false` in an autospec-sweep or
autospec-fleet config to *disable* a step or a repo gets silently
overridden back to enabled. This is the same alternative-operator coercion
bug, on a config toggle rather than a gate result — worth a human decision
on whether the same `if .X == false then false else true end` fix should be
applied there. **Not fixed in this change** — it is outside the 5-file scope
this task named, and the brief's "report what you find" instruction for
adjacent forms takes precedence over silently expanding scope.

Checked and found harmless (no bug): `.tier // 1` in `scripts/lib/autospec-loop.sh`
— jq's `//` only treats `null`/`false` as absent; `0` and `""` are truthy in
jq, so a numeric/string default is safe unless the underlying field is itself
boolean. No `.mjs`/`.py`/Rust equivalent of the bug was found: JS `??`
(nullish coalescing, used nowhere problematic here) does not coerce `false`;
no `|| true` / `|| false` patterns found on pass/success fields in `.mjs`/`.js`;
no `or True` in Python; Rust's `unwrap_or(true)` calls found
(`crates/autospec-cli/...`) operate on `Option<bool>`, which is not
subject to this bug — `Some(false)` stays `false` in Rust, only `None`
triggers the default.

## Previously-masked failure — separate, more severe, NOT fixed

While building a coercion test for `gate-stage-2-5.sh`'s Metric F/G/H/I
sites, discovered that **the real runner scripts are never invoked at all**,
independent of the jq bug. `run_metric()` resolves each runner at:

```
$SCRIPT_DIR/../invariants/<runner>.mjs
```

where `$SCRIPT_DIR` is `skills/autospec-test/scripts`. That resolves to
`skills/autospec-test/invariants/<runner>.mjs`, which **does not exist**.
The real runners live at:

- `skills/autospec-test/scripts/invariants/run-structural.mjs`
- `skills/autospec-test/scripts/window-contract/run-window.mjs`
- `skills/autospec-test/scripts/crawler-v2/extended-crawler.mjs`
- `skills/autospec-test/scripts/contract-symmetry/run-symmetry.mjs`

Live proof, run against `skills/autospec-test/test-targets/target-invariant-bait`
— a fixture purpose-built so Metric F should fail (its own
`golden/stage-2-5-gate.json` says `passed: false`):

```
$ bash skills/autospec-test/scripts/gate-stage-2-5.sh \
    skills/autospec-test/test-targets/target-invariant-bait
...
"G": {"metric": "G", "passed": true, "skipped": true, "reason": "runner not installed"},
"H": {"metric": "H", "passed": true, "skipped": true, "reason": "runner not installed"},
"I": {"metric": "I", "passed": true, "skipped": true, "reason": "runner not installed"}
  }
}
```

(Metric F shows the same `"runner not installed"` shape, truncated above.)

**This means Stage 2.5 metrics F/G/H/I never actually run in this repo
today, regardless of the jq coercion fix.** The existing integration test
(`skills/autospec-test/tests/integration/v2/run-against-target.bats`) only
checks static golden fixtures — it never invokes `gate-stage-2-5.sh` live
against the bait targets, so this path bug has zero test coverage and is
completely masked.

This is (b)-shaped in the terms of the brief: a genuine, previously-existing
defect that my jq fix does not cause and does not fix — reported here for a
human to judge, not patched, since it is a distinct bug (path resolution,
not `//`-coercion) and outside the stated scope ("do not change gate
semantics beyond fixing the coercion").

## Mutation table — every new test proven to fail on the original bug

Each site was mutated back to its original `// true` (or `// false` where the
old form appears) in a byte-copy of the file, the relevant test(s) run, then
the file restored from the copy and `diff`-verified byte-identical (never
`git checkout --`).

| Site | File:line | Mutation | Test | Result |
|---|---|---|---|---|
| 1 | gen-pr-report.sh:170 | `// true` | `tests/gen-pr-report.bats` #7 | RED (confirmed) |
| 2 | gate-stage-unit.sh:87 | `// true` | `gate-stage-unit.bats` #12 | RED (confirmed) |
| 3 | run-gate.sh:116 | `// true` | `run-gate-coercion.bats` #1 | RED (confirmed) |
| 4 | run-gate.sh:188 | `// true` | `run-gate-coercion.bats` #2 | RED (confirmed) |
| 5 | gate-stage-2-5.sh:110 (F) | `// true` | `gate-stage-2-5-coercion.bats` #1 | RED (confirmed) |
| 6 | gate-stage-2-5.sh:114 (G) | `// true` | `gate-stage-2-5-coercion.bats` #2 | RED (confirmed) |
| 7 | gate-stage-2-5.sh:118 (H) | `// true` | `gate-stage-2-5-coercion.bats` #3 | RED (confirmed) |
| 8 | gate-stage-2-5.sh:122 (I) | `// true` | `gate-stage-2-5-coercion.bats` #4 | RED (confirmed) |
| 9 | pr-report.sh:89 (F) | `// true` | `pr-report-coercion.bats` #1 | RED (confirmed) |
| 10 | pr-report.sh:91 (G) | `// true` | `pr-report-coercion.bats` #2 | RED (confirmed) |
| 11 | pr-report.sh:93 (H) | `// true` | `pr-report-coercion.bats` #3 | RED (confirmed) |
| 12 | pr-report.sh:95 (I) | `// true` | `pr-report-coercion.bats` #4 | RED (confirmed) |
| 13 | pr-report.sh:97 (seeds) | `// true` | `pr-report-coercion.bats` #5 | RED (confirmed) |

All 13 mutations independently flipped only their own test to RED with every
other test in the run staying green (spot-checked per file above), and every
restore diffed byte-identical against the pre-mutation copy.

## Test summary

```
bats tests/gen-pr-report.bats                                        8/8 pass
bats skills/autospec-test/tests/unit/gate-stage-unit.bats           24/24 pass
bats skills/autospec-test/tests/unit/gate-stage-2-5-coercion.bats     5/5 pass  (new file)
bats skills/autospec-test/tests/unit/run-gate-coercion.bats           2/2 pass  (new file)
bats skills/autospec-test/tests/unit/pr-report-coercion.bats          6/6 pass  (new file)
```
Total: 45/45 across all runs together (no cross-file interference).

`bash -n` and `shellcheck` clean on all 5 edited scripts (pre-existing
SC2034/SC2086 warnings only, unrelated to this change — spot-checked that
none appear on the lines I touched).

## New test locations chosen

- `tests/gen-pr-report.bats` — extended in place (existing idiom, existing
  fixtures dir `tests/fixtures/gen-pr-report/`); added 3 new fixture files
  (`gate-plain-pass.json`, `drift-explicit-false.json`,
  `drift-absent-key.json`) and 2 new `@test` cases.
- `skills/autospec-test/tests/unit/gate-stage-unit.bats` — extended in place
  (existing idiom); added 1 new `@test` reusing the existing
  multiply/farewell JS fixture.
- `skills/autospec-test/tests/unit/gate-stage-2-5-coercion.bats` — **new
  file**. No prior test invoked `gate-stage-2-5.sh` live (the only existing
  v2 integration test only checks static golden JSON, see the masked-failure
  section above), so there was no existing idiom to extend; the file copies
  `scripts/` into a temp dir and drops stub Node runners at the sibling
  `invariants/` path `run_metric()` actually resolves, to isolate the jq
  coercion from the separate path-resolution bug.
- `skills/autospec-test/tests/unit/run-gate-coercion.bats` — **new file**.
  `run-gate.sh` has no prior dedicated test file. Test 1 stubs all three
  sub-gate scripts in a copied `scripts/` dir; test 2 uses `run-gate.sh`'s
  own `.autospec/stub-gate.json` short-circuit (already built for
  golden-diff testing) plus a stubbed `gh` binary that logs argv instead of
  touching the network.
- `skills/autospec-test/tests/unit/pr-report-coercion.bats` — **new file**.
  `pr-report.sh` has no prior dedicated test file; feeds crafted
  `--gate-json` fixtures directly, no network or `gh` calls needed for the
  markdown-render path.

## Safety

No test reaches the real GitHub API (`gh` is either never invoked, or
stubbed with a fake `gh` binary on `PATH` that only appends to a local log
file), no installer runs against `$HOME`, and nothing outside
`/tmp/autospec-*-bats-XXXXXX` scratch dirs and the repo working tree was
touched.
