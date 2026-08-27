# board-config-coherence: two-finding fix report

Worktree: `.claude/worktrees/pb-cfg`, branch `feat/board-config-coherence`.

## Finding 1 — vacuous non-final `[[ ]]` assertions (and a neutered `|| true`)

### Root cause, verified on this machine (bash 3.2.57, and real `bats` 1.13.0)

Under bash's `set -e`, a `[[ ... ]]` or `!`-negated command that is **not the
final statement** of a function/`@test` body never triggers failure —
confirmed empirically:

```
@test { [[ a == b ]]; [ 1 -eq 1 ]; }   -> PASSES  (non-final [[ ]] is vacuous)
@test { [ 1 -eq 1 ]; [[ a == b ]]; }   -> fails   (final [[ ]] works)
! grep -q '127.0.0.1' /etc/hosts        -> never fails set -e, at ANY position
                                           unless it is the function's final
                                           statement (then its exit status IS
                                           the function's return, and bats
                                           reports that as a failure)
```

`grep -qF pattern file` (a plain simple command, no `[[`/`!`) fails `set -e`
correctly at any position — confirmed with a bats test. This is the idiom
already used elsewhere in this repo (`autonomous-promote-open-issues.bats`
uses `grep -q '...' "$GH_LOG"` throughout), so the fix reuses it rather than
inventing a new pattern.

### Full audit of `tests/autospec/project-board-config-wiring.bats`

| # | Test | Vacuous assertions found | Cause |
|---|------|---------------------------|-------|
| 4 | "operator-facing project_board settings reach the resolver's env unaltered" | 4 of 5 `[[ ]]` lines (all but the last) | non-final `[[ ]]` |
| 5 | "an already-exported env var wins over the bridged YAML value for the new fields" | 1 of 2 `[[ ]]` lines | non-final `[[ ]]` |
| 6→7 renumbered | "an unconfigured project_board block leaves the resolver seeing the legacy hardcoded defaults" | 3 of 4 `[[ ]]` lines | non-final `[[ ]]` |
| 1 | "a valid .autospec/autonomous.yml project_board block makes the promoter see a board" | 1 (`grep -q ... "$GH_CALLS" 2>/dev/null \|\| true`) | **not a position bug** — `\|\| true` unconditionally neuters the check. Investigated further: it was also checking the *wrong log*. `resolve.sh` is invoked directly with `--url "$_url"` (`autonomous-promote-open-issues.sh:305`), never through `gh`, so the URL can never appear in `$GH_CALLS` even when the bridge works correctly — the original assertion could not have passed even without `\|\| true`. |

Total: **9 assertions across those 4 tests could never fail before the fix**
(4+1+3+1), consistent with the "9 of 13" figure in the brief once the
`grep \|\| true` line is counted alongside the `[[ ]]` lines.

No other `[[` or non-final `!`-negated commands exist elsewhere in this
file (`grep -n '\[\['` returns only the 11 lines listed above, all now
fixed; the file's two `! grep`/`! -e` occurrences are `[ ! -e ... ]`, a
`test`-operator negation, not a negated command, and are safe).

### Fix

- Every `[[ "$output" == *"X"* ]]` line was rewritten as
  `grep -qF 'X' "$file"`, and reading `$output` via `run cat` was dropped
  entirely in favor of `grep` directly against the capture file — this also
  sidesteps the `$output`-overwrite hazard the brief called out, since
  nothing downstream depends on a `run`-captured `$output` for these checks.
- The `grep -q ... \|\| true` line was replaced with a purpose-built
  `resolve.sh` stub that logs its own invocation args to
  `$TMP/resolve-args.log`, asserted via `[ -f ... ]` + `grep -qF` against
  that log — checking the log that actually receives the URL.

### Mutation proof (Finding 1)

All three mutations were applied by editing
`scripts/autonomous-promote-open-issues.sh` in place, running
`bats tests/autospec/project-board-config-wiring.bats`, then restoring from
a `cp`'d copy and confirming `diff` showed **zero** difference from the
pre-mutation file (`git diff` also showed no residual change) before moving
to the next mutation.

| Behavior | Mutation | Intended test | Result |
|---|---|---|---|
| env-wins-over-YAML precedence for `AUTOSPEC_PROJECT_BOARD_STATE_FIELDS` | `if [ -z "${AUTOSPEC_PROJECT_BOARD_STATE_FIELDS:-}" ]` → `if true` (always re-derive from YAML, discarding the operator's export) | "an already-exported env var wins over the bridged YAML value for the new fields" | **RED** — `grep -qF 'STATE_FIELDS=Operator override' ...` failed |
| `dep_markers` YAML→env wiring | `if [ -z "${AUTOSPEC_PROJECT_BOARD_DEP_MARKERS:-}" ]` → `if false` (custom `dep_markers` from YAML never reaches the resolver) | "operator-facing project_board settings reach the resolver's env unaltered" | **RED** — `grep -qF 'DEP_MARKERS=Waiting on' ...` failed |
| `url`/allowlist gate wiring (the board-ingestion trigger) | `if [ -z "${AUTOSPEC_PROJECT_BOARD_URL:-}" ]` → `if false` (bridged URL never exported, so no board is ever seen) | "a valid .autospec/autonomous.yml project_board block makes the promoter see a board" (plus 3 others, cascading) | **RED** — 4 of 7 tests failed, including the primary target |

All three restores were verified byte-identical (`diff` printed nothing,
`git diff --stat` showed no change) before the next mutation.

## Finding 2 — `project_board.write_back: false` was inert (and a jq bug made the fix inert too, until caught by these tests)

### Design decision: single choke point

`write_back` is enforced **inside `scripts/project-board-writeback.sh`
itself**, not in each caller. Both current callers —
`autonomous-promote-open-issues.sh`'s `board_writeback()` wrapper, and
`scripts/lib/autospec-loop.sh`'s `_autospec_conductor_board_state` at PR
lifecycle points — already funnel every board mutation through this one
script before it ever reaches `gh project item-edit`. Putting the gate here
means a future caller cannot forget to check a flag, because there is
nothing to forget: the script always checks itself first. Checking the flag
separately in both call sites (the alternative) would mean a missed or
mis-ordered check at either site silently re-enables writes — exactly the
risk the brief called out.

### Wiring

1. **Rust bridge** (`crates/autospec-cli/src/commands/autonomous.rs`,
   `format_project_board_config`): added `"write_back":<bool>` to the JSON
   emitted by `autospec autonomous project-board-config`. `ProjectBoardConfig`
   already computed the correct default (`true` when `url` is set and
   `write_back` is absent from YAML — unchanged prior behavior; `false`
   when no board is configured at all; otherwise the operator's explicit
   value) — that logic pre-existed and was already unit-tested; it just
   was never surfaced in the shell bridge.
2. **`autonomous-promote-open-issues.sh`**: bridges `write_back` into
   `AUTOSPEC_PROJECT_BOARD_WRITE_BACK` (`0`/`1`) once per run, following
   the same env-wins-over-YAML precedence as every other bridged field.
   This is a **performance cache only**, not the enforcement — it exists so
   `project-board-writeback.sh` doesn't re-invoke the Rust binary on every
   item during a cycle with many writes.
3. **`project-board-writeback.sh`** (the actual choke point): reads
   `AUTOSPEC_PROJECT_BOARD_WRITE_BACK` if explicitly `0`/`1` (caller
   override/cache); otherwise re-derives the value itself by calling
   `autospec autonomous project-board-config` directly — this is what makes
   the conductor loop's writeback calls correct even though
   `autospec-loop.sh` never runs the promoter's bridge step. Any failure in
   that path degrades to enabled (write-back was unconditional before this
   fix, so an unreachable/missing config must not silently start
   suppressing writes).

### A second real bug found and fixed along the way: jq's `//` and literal `false`

Both the promoter's bridge parse and `project-board-writeback.sh`'s own
live-query parse originally used `jq -r '.write_back // true'`. **jq's `//`
treats a literal `false` value as falsy**, so `false // true` evaluates to
`true` — meaning `write_back: false` in YAML was silently coerced back to
"enabled" by this expression, even after the Rust bridge and the shell
plumbing were otherwise wired correctly. This was caught by the new tests
themselves failing (a mutation-testing hazard averted by actually running
the tests, not just writing them): `write_back: true` and absent-key tests
passed on the first try, but `write_back: false` kept writing anyway. Fixed
in both files by replacing `.write_back // true` with
`if .write_back == false then "false" else "true" end`, which tests the
value directly instead of falling through on any falsy value.

### `control_issue` — explicitly NOT wired

Per instructions, `project_board.control_issue` was left untouched. It feeds
`project-board-control-mirror.sh`, which has no caller yet; wiring it is
separately planned work and is **not done here**.

### New tests (`tests/autospec/project-board-config-wiring.bats`)

Three new tests drive a **full `--apply` cycle through the real**
`project-board-writeback.sh` (not a stub for it), with a `gh` stub that
handles `issue list`, `auth status` (reports the `project` scope), and
`item-edit` (success) — and a board item with a populated `project`/`fields`
shape (not the usual tests' `{}`) plus an unresolved dependency, so
`board_apply_item`'s blocked branch fires `board_writeback` unconditionally,
without needing a `GROOM_SAFETY_BIN` stub.

| Test | Assertion source |
|---|---|
| `write_back: false` suppresses every board write across a full `--apply` cycle | `run grep -c 'item-edit' "$GH_CALLS"; [ "$output" -eq 0 ]` — the stub's own argument log, not `$status` (fail-open contract means `$status` is 0 either way) |
| `write_back: true` still writes | `grep -qF 'item-edit' "$GH_CALLS"` |
| absent `write_back` key still writes (default unchanged) | `grep -qF 'item-edit' "$GH_CALLS"` |

### Mutation proof (Finding 2)

Reverted `if .write_back == false then "false" else "true" end` back to the
buggy `.write_back // true` in **both** files (`sed`, then restored from a
`cp`'d copy afterward; `git diff --stat` showed the fix files unchanged
post-restore):

```
$ bats tests/autospec/project-board-config-wiring.bats
not ok 8 write_back: false suppresses every board write across a full --apply cycle
#   `[ "$output" -eq 0 ]' failed
ok 9  write_back: true still writes across a full --apply cycle
ok 10 an absent write_back key still writes across a full --apply cycle (default unchanged)
```

RED as expected — restored, re-ran, all 10 green.

## Test results (all foreground, full output observed)

```
bats tests/autospec/project-board-config-wiring.bats tests/autospec/spend-ledger-scope.bats
  1..33, all ok (10 config-wiring + 23 spend-ledger-scope)

bats tests/autospec/project-board-*.bats tests/autospec/autonomous-promote-open-issues.bats
  1..192, all ok

cargo test -p autospec-core --test autonomous_project_board_config
  26 passed; 0 failed

cargo test -p autospec-cli --bin autospec project_board_config_tests
  8 passed; 0 failed (includes 2 new write_back-specific unit tests)
```

## Files changed

- `crates/autospec-cli/src/commands/autonomous.rs` — bridge emits
  `write_back`; doc comment updated; 2 new + 8 total unit tests updated for
  the new JSON field.
- `scripts/autonomous-promote-open-issues.sh` — bridges `write_back` →
  `AUTOSPEC_PROJECT_BOARD_WRITE_BACK` (cache only).
- `scripts/project-board-writeback.sh` — the enforcement choke point; new
  write_back gate before any `gh` call.
- `tests/autospec/project-board-config-wiring.bats` — vacuous-assertion
  fixes across all pre-existing tests, plus 3 new write_back tests.

## Constraints honored

Bash 3.2 idioms only (no `[[`, no non-final `!`); `set -eu` with no
one-sided `[ test ] && action`; no `RETURN` traps; no `eval` of
config-derived values; no config value interpolated into a `jq test()`
regex; all `gh`/safety-authority calls stubbed, no real GitHub API reached;
`HOME` was not touched by any test in this file (no ledger probes here);
`git checkout --` was never used — every mutation-proof restore used `cp`
from a temp copy, verified with `diff`/`git diff --stat` before proceeding.
