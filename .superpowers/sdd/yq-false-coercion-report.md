# yq `//` false-coercion fix

Branch: `fix/yq-enabled-false-coercion`

## Bug

`yq`'s `//` alternative operator treats a literal `false` as absent, exactly
like `jq`. Any `<path> // true` expression therefore silently rewrites an
operator's explicit `enabled: false` (or any other `false` toggle) back to
`true`. `fleet-run.sh` now actually spawns a real `autospec-autonomous`
conductor on that gate, so this was a live control-loss bug, not a cosmetic
one.

## Canonical fix chosen

`<path> != false` — evaluates to `true` for `true`, for any non-`false`
scalar, and for an absent/null path (yq's comparison against a missing node
yields `null`, and `null != false` is `true`), and to `false` only for a
literal `false`. It degrades safely through missing parent objects
(verified: `.execution.deployment.deploy_if_tests_require != false` against
a config that has no `execution` key at all still returns `true`), which
matches the behavior the parallel jq fix in this repo uses
(`if .X == false then false else true end`) — `!= false` is the equivalent
in `yq`'s expression language and is shorter, so it was preferred over
porting the `if/then/else` form verbatim. For the two `@tsv` row sites the
same substitution works unchanged inside the array constructor:
`(.require_scope != false)`.

Verified directly against `yq` v4.53.2 before and after:
```
# absent key
printf 'repos:\n  - url: y\n' | yq -r '.repos[0].enabled != false'   -> true
# explicit false
printf 'repos:\n  - url: x\n    enabled: false\n' | yq -r '.repos[0].enabled != false'  -> false
# true
printf 'repos:\n  - url: x\n    enabled: true\n' | yq -r '.repos[0].enabled != false'   -> true
```

## Independently derived site list (19 sites, matches the brief)

| # | File | Line | Expression |
|---|------|------|------------|
| 1 | `skills/autospec-fleet/scripts/fleet-run.sh` | 127 | `.repos[$idx].enabled` (launch gate) |
| 2 | `skills/autospec-fleet/scripts/fleet-status.sh` | 57 | `.repos[$idx].enabled` |
| 3 | `skills/autospec-fleet/scripts/fleet-stop.sh` | 54 | `.repos[$idx].enabled` |
| 4 | `skills/autospec-sweep/scripts/run.sh` | 128 | `.steps.review.enabled` |
| 5 | `skills/autospec-sweep/scripts/run.sh` | 129 | `.steps.run.enabled` |
| 6 | `skills/autospec-sweep/scripts/run.sh` | 130 | `.sweep.spec_sync.enabled` |
| 7 | `skills/autospec-sweep/scripts/run.sh` | 131 | `.continuous_improvement.loop.file_issues` |
| 8 | `skills/autospec-sweep/scripts/run.sh` | 132 | `.continuous_improvement.loop.route_fixes_via_autospec_run` |
| 9 | `skills/autospec-sweep/scripts/run.sh` | 134 | `.continuous_improvement.tests.enabled` |
| 10 | `skills/autospec-sweep/scripts/run.sh` | 135 | `.execution.tests.run_all_every_sweep` |
| 11 | `skills/autospec-sweep/scripts/run.sh` | 136 | `.execution.deployment.deploy_if_tests_require` |
| 12 | `skills/autospec-sweep/scripts/review.sh` | 154 | `.sweep.spec_sync.enabled` |
| 13 | `skills/autospec-sweep/scripts/review.sh` | 155 | `.continuous_improvement.docs.enabled` |
| 14 | `skills/autospec-sweep/scripts/review.sh` | 156 | `.documentation.enabled` |
| 15 | `skills/autospec-sweep/scripts/review.sh` | 157 | `.continuous_improvement.tests.enabled` |
| 16 | `skills/autospec-sweep/scripts/review.sh` | 158 | `.continuous_improvement.code.enabled` |
| 17 | `skills/autospec-sweep/scripts/review.sh` | 160 | `.execution.deployment.deploy_if_tests_require` |
| 18 | `skills/autospec-sweep/scripts/review.sh` | 253 | `(.require_scope)` inside `@tsv` (`documentation.audiences[]`) |
| 19 | `skills/autospec-sweep/scripts/review.sh` | 258 | `(.require_scope)` inside `@tsv` (`documentation.scopes[]`) |

`skills/autospec-sweep/scripts/run.sh:133` (`max_issues_per_sweep // 5`) and
`skills/autospec-fleet/scripts/fleet-run.sh:102`
(`.parallel_repos // 1`) were left untouched: they default a *numeric*
value, and `0` is not a value this class of bug affects for these two
particular fields (a `0` cap/count is already handled correctly by
`positive_int`/normal integer comparisons downstream, and no literal
`false` can appear there).

## Mutation-proof table

For every site above: mutated the site's `!= false` back to `// true`
(or `!= false` → `// true` inside the tsv row), ran the associated new
test, confirmed RED, restored from an untouched backup copy
(`cp` to a scratchpad dir, never `git checkout --`), and `diff`'d the
restored file against the backup to confirm byte-identical.

| Site | Mutation | Test file | Test | Result |
|---|---|---|---|---|
| fleet-run.sh:127 | `// true` | test_autospec_fleet_enabled_false.bats | "fleet-run does not spawn a conductor for a repo with enabled: false" | RED → restored OK |
| fleet-status.sh:57 | `// true` | test_autospec_fleet_enabled_false.bats | "fleet-status reports a repo with enabled: false without probing its queue" | RED → restored OK |
| fleet-stop.sh:54 | `// true` | test_autospec_fleet_enabled_false.bats | "fleet-stop does not call autospec-stop for a repo with enabled: false" | RED → restored OK |
| run.sh:128 (review) | `// true` | test_autospec_sweep_enabled_false.bats | "sweep run --dry-run reports every explicitly-disabled toggle as false" | RED → restored OK |
| run.sh:129 (run) | `// true` | same | same | RED → restored OK |
| run.sh:130 (spec_sync) | `// true` | same | same | RED → restored OK |
| run.sh:131 (file_issues) | `// true` | same | "sweep run does not file gaps when ... file_issues is false" | RED → restored OK |
| run.sh:132 (route_run) | `// true` | same | "sweep run does not recommend /autospec-run when route_fixes_via_autospec_run is false" | RED → restored OK |
| run.sh:134 (tests) | `// true` | same | "sweep run --dry-run reports every explicitly-disabled toggle as false" | RED → restored OK |
| run.sh:135 (run_all_tests) | `// true` | same | same | RED → restored OK |
| run.sh:136 (deploy_if_tests_require) | `// true` | same | same | RED → restored OK |
| review.sh:154 (spec_sync) | `// true` | test_autospec_sweep_enabled_false.bats | "review.sh does not emit spec_sync gap..." | RED → restored OK |
| review.sh:155 (docs) | `// true` | same | "review.sh does not emit the README gap..." | RED → restored OK |
| review.sh:156 (documentation) | `// true` | same | "review.sh does not emit documentation target gaps..." | RED → restored OK |
| review.sh:157 (tests) | `// true` | same | "review.sh does not emit the test-command gap..." | RED → restored OK |
| review.sh:158 (code) | `// true` | same | "review.sh does not emit the TODO-marker gap..." | RED → restored OK |
| review.sh:160 (deploy_if_tests_require) | `// true` | same | "review.sh does not emit the deploy-command gap..." | RED → restored OK |
| review.sh:253 (require_scope, audiences tsv) | `// true` | same | "review.sh does not emit the doc-scope gap when require_scope is false" | RED → restored OK |
| review.sh:258 (require_scope, scopes tsv) | `// true` | same | "review.sh does not emit the scope doc-scope gap when require_scope is false (scopes list)" | RED → restored OK |

All 19 sites: mutation reverted, file diffed byte-identical to the backup
after restore.

## Test results

- `bats tests/unit/test_autospec_fleet_enabled_false.bats` — 4/4 pass.
- `bats tests/unit/test_autospec_sweep_enabled_false.bats` — 14/14 pass
  (also covers the "absent key still defaults to enabled" side for both
  fleet and sweep).
- Full regression sweep — `bats tests/fleet/ tests/sweep/
  tests/unit/test_autospec_fleet_scheduler.bats
  tests/unit/test_autospec_fleet_status_stop.bats
  tests/unit/test_autospec_fleet_config.bats
  tests/unit/test_autospec_fleet_url.bats
  tests/unit/test_autospec_sweep_run.bats
  tests/unit/test_autospec_sweep_config.bats
  tests/unit/test_autospec_fleet_enabled_false.bats
  tests/unit/test_autospec_sweep_enabled_false.bats`: 84 ok, 2 not ok
  ("fleet-run dry-run emits autospec-run commands for eligible repos" and
  "fleet-run caps output at parallel_repos 2" in
  `test_autospec_fleet_scheduler.bats`) — both confirmed **pre-existing**
  on a clean `git stash` of this branch's changes (they assert
  `autospec-autonomous start --detach`, a flag `fleet_worker_command`
  deliberately no longer emits per an existing code comment; unrelated to
  this fix). No new regressions.
- `bash -n` on all 5 touched shell scripts: clean.
- `shellcheck` on all 5 touched shell scripts: only pre-existing
  info/warning-level findings unrelated to the touched lines
  (`SC1091` on `source ...fleet-lib.sh`, `SC2015` in an existing
  `A && B || C` guard, `SC2086`/`SC2034` in unrelated lines of
  `run.sh`/`review.sh`). No new shellcheck findings introduced by this
  change.

## Wider sweep: other `// true` / `// 1` sites found, not fixed

Ran `grep -rn '// true\|// "true"\|// 1\b\|// "1"' --include='*.sh'
scripts/ skills/` across the whole repo. Beyond the 19 sites above, found:

- `scripts/gen-pr-report.sh:170` — `jq -r '.passed // true' "$DRIFT_FILE"`
- `scripts/autonomous-guardrails.sh:156-157` — `.killed // 0`,
  `.total // 1` (both denominators/counts, not the `false`-coercion class —
  `0`/`1` here are legitimate numeric defaults, not a boolean the operator
  could set to `false`)
- `scripts/lib/autospec-loop.sh:2060` — `jq -r '.tier // 1'`
- `skills/autospec-test/scripts/gate-stage-unit.sh:87` —
  `.unit.function_presence_check // true`
- `skills/autospec-test/scripts/run-gate.sh:116,188` —
  `.passed // true`, `.stage2.metrics.restore_succeeded // true`
- `skills/autospec-test/scripts/pr-report.sh:89,91,93,95,97` — five
  `.stage2_5.metrics.{F,G,H,I}.passed // true` / `.seeds_ok // true`
- `skills/autospec-test/scripts/gate-stage-2-5.sh:110,114,118,122` — four
  `.passed // true`

**Same bug class, not fixed here**: `.passed // true`,
`.function_presence_check // true`, `.restore_succeeded // true`, and
`.seeds_ok // true` are all `jq` (not `yq`) but `jq`'s `//` has the
identical false-as-absent behavior — if a gate stage genuinely reports
`"passed": false`, these read back as `true` and a failed gate is silently
treated as passed. This is a real defect of the same shape, but it is a
gate-*result*-interpretation bug in `autospec-test`'s own scoring pipeline,
not an operator-facing config toggle like the 19 sites in this brief
(fleet/sweep `enabled`/`deploy_if_tests_require`/`file_issues`/etc., which
an operator sets directly in `autospec.yml`/`autospec-fleet.yml`). Fixing
it correctly requires understanding each gate stage's actual JSON producer
to confirm `false` is a real, intentionally-distinguishable value there
(vs. always produced with `passed` present-and-boolean, in which case the
practical risk is lower but the same footgun exists) and to build the same
mutation-proof regression coverage for each of the 8 sites — that is a
separate, same-shape fix that deserves its own scoped pass (and its own
brief) rather than being folded into this one under time pressure; flagging
it here as the highest-priority follow-up. `scripts/qa-heal-loop.sh:329`
(`.confidence // 1.0`) and the `// 1`/`// 5` numeric-default sites above are
not part of this bug class (no boolean `false` can round-trip through them)
and don't need follow-up.

No other `false`/`0`/empty-string-coercion patterns matching this class
were found beyond what's listed above.
