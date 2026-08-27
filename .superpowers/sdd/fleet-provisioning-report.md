# Fleet checkout provisioning — report

## Scope

Closed the last gap in `/autospec-project ship <url>`: `fleet-init.sh` only
ever *planned* checkout paths (its own usage text said "no repositories are
cloned"), so `fleet-run.sh` always found an empty workspace and reported
"checkout not found; skipping launch" — no conductor ever launched.
`fleet-run.sh` itself was not touched, per instructions.

## Extended fleet-init.sh, did not add a sibling script

`fleet-init.sh` already owned the "prepare the workspace" responsibility per
`SKILL.md` (`init` = "prepares the managed workspace"), and its `--dry-run`
output contract ("plan clone X -> Y") was already the correct *preview* of
the action this task adds. Splitting provisioning into a second script would
have meant either two config-parsing/dry-run code paths or fleet-init.sh
calling out to a sibling — no benefit, more surface area. All new logic
lives in `fleet-lib.sh` (shared helpers, matching how `fleet-run.sh` already
consumes `repo_checkout_path`/`normalize_repo_url`/`repo_slug` from that
file) and is invoked from `fleet-init.sh`'s existing option-parsing shell.

New `fleet-lib.sh` functions:
- `fleet_path_within_workspace` — containment check (defense-in-depth; see
  below for why the primary guarantee is structural).
- `fleet_clone_checkout` — `git clone -- <url> <path>`, code_health marker
  on failure.
- `fleet_update_checkout` — dirty-check via `git status --porcelain`, then
  `git fetch` + `git merge --ff-only FETCH_HEAD`.
- `fleet_provision_repo` — orchestrates clone-vs-update per repo, always
  called from an `if/then` in the caller's loop (never `&&`) so one repo's
  failure can't abort the batch under `set -euo pipefail`.

`fleet-init.sh`'s non-dry-run path now: `mkdir -p "$workspace"`, then loops
`fleet_provision_repo` over the repo URLs, printing whatever that function
printed (clone/update/skip/code_health messages) and continuing regardless
of per-repo outcome. The whole run always exits 0, matching the convention
`fleet-run.sh` already established (a per-repo spawn failure there also
never fails the batch's exit code — only a `code_health:` marker on stderr
signals it).

## Idempotence and no-destroy semantics

- **Idempotent**: `[ -d "$checkout_path/.git" ]` decides clone vs. update.
  A repo already checked out is never re-cloned; re-running provisioning on
  a clean, up-to-date checkout only issues `git status`, `git fetch`, and a
  no-op `git merge --ff-only` — cheap and safe to run on every fleet cycle.
- **Never destroy local work**: `fleet_update_checkout` runs
  `git status --porcelain` first. Any output (staged, unstaged, *or*
  untracked) — i.e. anything short of a byte-for-byte clean tree — skips the
  update entirely with `fleet: <repo>: local changes present at <path>;
  skipping update` and never calls `git fetch` or `git merge`. If the fetch
  does happen and `git merge --ff-only` fails (history diverged, not
  fast-forwardable), the message is `... would not fast-forward; skipping`
  — no `reset`, `clean`, `checkout -f`, or any other mutating command is
  ever issued on that path. No `git reset|clean|checkout -f` string appears
  anywhere in the new provisioning code (grep-verified; also asserted by
  test "a non-fast-forwardable update is skipped, not force-updated").
- **Per-repo failure, not fatal**: clone and fetch failures each emit a
  distinct `code_health:fleet_provision_{clone,fetch}_failed repo=... path=...`
  marker on stderr and `return 1` from the helper; the caller's `if/then`
  (never a one-sided `&&`) absorbs that and moves to the next repo. Proven
  by "a clone failure for one repo does not abort provisioning of the next".

## Path containment

The scheme is the one `fleet-lib.sh` already used for `fleet-run.sh`:
`repo_checkout_path(workspace, normalized) = "$workspace/$(repo_slug
normalized)"`, and `repo_slug` replaces every `/` in `owner/repo` with `__`.
Because `normalize_repo_url` requires the URL to reduce to exactly one `/`
(regex `^([^/]+)/([^/]+)$`), and that `/` is the only one substituted away,
the resulting slug is structurally guaranteed to be a single path component
containing no `/` — so `repo_checkout_path`'s output can never contain a
`..` path segment or an absolute-path escape. `fleet_path_within_workspace`
adds a second, independent check (string-prefix match against workspace,
rejecting a `..`/`.`/empty trailing component) so a future change to the
slug scheme would still be caught, and `fleet_provision_repo` calls it
before ever touching the filesystem.

Probe table (all verified against the real filesystem, not exit codes
alone — `tests/fleet/fleet-provisioning.bats`):

| Probe | Input | Result | Verified |
|---|---|---|---|
| Multi-segment traversal | `https://github.com/../../etc.git` | `normalize_repo_url` rejects: "unsupported repo URL" (2+ slashes after strip) | no dir created anywhere under `$TMP`; `$GIT_LOG` empty |
| `a/../../b` shape | `https://github.com/a/../../b.git` | rejected, same reason | same |
| Absolute-looking (`//etc/passwd`) | `https://github.com//etc/passwd` | rejected — leading `/` can't match `[^/]+` | same |
| Leading-dash slug | `https://github.com/-rf/repo` | accepted; slug `-rf__repo`, directory created **inside** workspace, never parsed as a flag (every call uses `--` or `-C`/quoting) | `$WORKSPACE/-rf__repo/.git` exists; all dirs under `$WORKSPACE` verified via `find` to start with `$WORKSPACE/` |
| Direct unit check | `fleet_path_within_workspace /tmp/ws /tmp/other/repo` and `/tmp/ws/..` | both rejected; `/tmp/ws/org__repo` accepted | exit codes asserted directly against the sourced function |

No probe was marked "cannot verify" — all were fully isolated (stubbed
`git`, `mktemp -d` sandboxes, real `find`/`[ -e ]` filesystem checks).

## --dry-run stays inert

Unchanged code path in shape (still the first branch, still prints
`fleet: plan clone <normalized> -> <checkout_path>` and nothing else), but
now explicitly does **not** call `fleet_provision_repo` — dry-run never
sources the git-touching code at all. Verified: existing regression test
`tests/unit/test_autospec_fleet_url.bats` ("fleet-init dry-run prints
deterministic checkout paths without cloning") still passes unmodified, and
two new tests confirm `$GIT_LOG` (the stub's invocation log) stays empty in
dry-run mode both when the checkout is absent and when it already exists.

## End-to-end chain proof

`tests/fleet/fleet-provisioning.bats`, test "end-to-end: provisioning +
fleet-run produces a real conductor command instead of checkout-not-found":

1. `fleet-run.sh` against a fleet config with one repo, empty workspace →
   output contains `checkout not found`; the (stubbed) conductor spawn log
   is empty.
2. `fleet-init.sh --workspace <ws> https://github.com/o/a.git` (stubbed
   `git`) → creates `<ws>/o__a/.git`.
3. `fleet-run.sh` again, same config → `checkout not found` no longer
   appears; the stubbed `autospec-autonomous` was invoked and logged.

Exact command string produced by the (unmodified) `fleet-run.sh` against the
now-provisioned checkout:

```
start --repo-dir /var/.../ws/o__a --repo o/a
```

(full form, matching `fleet_worker_command` in `fleet-lib.sh`:
`autospec-autonomous start --repo-dir <checkout> --repo <normalized>`).

## Tests

- New: `tests/fleet/fleet-provisioning.bats` — 13 tests, all green.
- Full suite: `bats tests/fleet/ tests/unit/test_autospec_fleet_url.bats` —
  46/46 green, nothing pre-existing regressed.
- `bash -n` clean on both touched scripts.
- `shellcheck` on both: only pre-existing info-level notices (SC1091 on the
  `source` line, SC2317 on `fleet-lib.sh`'s pre-existing sourced-guard
  clause) — no new warnings introduced.
- Proved two of the new negative assertions can actually fail: removed the
  dirty-checkout guard in `fleet_update_checkout` → test 4 went red;
  restored via `cp`+`diff` (byte-identical), test 4 green again. Neutered
  `fleet_path_within_workspace` to `return 0` → test 10 went red; restored
  the same way, byte-identical, test 10 green again. Neither restore used
  `git checkout --`.

## Constraints honored

- Bash 3.2: no associative arrays, no `${x^^}`, no `RETURN` traps.
- Every git invocation in the new helpers is guarded by `if/then/fi`
  (never a one-sided `&&`), matching `fleet-run.sh`'s existing pattern
  under `set -euo pipefail`.
- No `eval` of any config- or board-derived value; every expansion reaching
  a command is quoted, and directory/URL arguments are passed with `--`
  or `-C` where the git subcommand supports it.

## Post-review fixes (round 2)

An independent review confirmed the four safety requirements (dirty guard,
idempotence, per-repo isolation, structural path containment) hold and the
chain closes live, but flagged two real issues, both fixed below.

### Finding 1 — silent dirty/non-ff skip made loud

`fleet_update_checkout` in `fleet-lib.sh` previously printed only a plain
`fleet: ...` line for both "dirty checkout, skipping" and "would not
fast-forward, skipping" — no `code_health:` marker, unlike every other
skip/failure path in the function. In an unattended multi-week fleet this
meant a repo could stall indefinitely with nothing in the signal stream to
explain why.

Both branches now emit a `code_health:` marker on stderr, in addition to
the existing informational `fleet:` line on stdout (skip *behavior* is
unchanged — still `return 0`, never touches the checkout):

- Dirty checkout: `code_health:fleet_provision_update_skipped repo=<normalized> path=<checkout_path> reason=dirty_checkout`
- Non-fast-forwardable: `code_health:fleet_provision_update_skipped repo=<normalized> path=<checkout_path> reason=not_fast_forward`

Same marker name (`fleet_provision_update_skipped`) for both, distinguished
by `reason=`, so a fleet-wide grep for `code_health:fleet_provision_update_skipped`
finds every stalled repo, and `reason=` tells the operator which response
applies — clean up the checkout (dirty) vs. resolve a diverged history
(not-fast-forward). New assertions in tests 4 and 5 (`fleet-provisioning.bats`)
check for the marker and the specific `reason=` value, and — being
`[ ]`-based per Finding 2's fix — will actually fail if the marker or
reason value regresses.

### Finding 2 — vacuous `[[ ]]` assertions rewritten

10 of 11 `[[ ]]` substring assertions in `tests/fleet/fleet-provisioning.bats`
were non-final `[[ ]]` invocations, which the reviewer proved do not fail a
bats test under this repo's bash (3.2.57) — verified live by changing the
fast-forward message text and the dry-run plan message text and observing
the suite stay green either way before the fix.

Every one was rewritten to a single-bracket form:
`[ -n "$(printf '%s' "$output" | grep -F -- "needle")" ]` for "contains",
`[ -z "$(printf '%s' "$output" | grep -F -- "needle")" ]` for "does not
contain" — safe in any position because `[ ]` (unlike `[[ ]]` on this bash
patch level) always drives the test's pass/fail status. No assertion was
merely reordered to land last; every occurrence was replaced at its
original position.

Full list of rewritten assertions (all in `tests/fleet/fleet-provisioning.bats`):

| Test | Old (vacuous) | New |
|---|---|---|
| "a checkout with uncommitted changes is skipped, not reset" | `[[ "$output" == *"local changes present"* ]] \|\| [[ "$output" == *"skipping"* ]]` | separate `[ -n "$(...)" ]` checks for `local changes present`, `skipping`, `code_health:fleet_provision_update_skipped`, and `reason=dirty_checkout` |
| "a non-fast-forwardable update is skipped, not force-updated" | `[[ "$output" == *"would not fast-forward"* ]]` | `[ -n "$(...)" ]` for `would not fast-forward`, plus new checks for `code_health:fleet_provision_update_skipped` and `reason=not_fast_forward` |
| "a clone failure for one repo does not abort provisioning of the next" | `[[ "$output" == *"code_health:fleet_provision_clone_failed"* ]] \|\| [[ "$output" == *"code_health"* ]]` | `[ -n "$(...)" ]` for `code_health:fleet_provision_clone_failed` |
| "a fetch failure emits a code_health marker..." | `[[ "$output" == *"code_health:fleet_provision_fetch_failed"* ]]` | `[ -n "$(...)" ]` |
| "path containment: traversal-shaped repo URLs..." (×3) | `[[ "$output" == *"unsupported repo URL"* ]]` | `[ -n "$(...)" ]`, once per probe |
| "dry-run creates nothing and performs no git invocation" | `[[ "$output" == *"fleet: plan clone org/repo-a -> ..."* ]]` | `[ -n "$(...)" ]` |
| "end-to-end..." (before provisioning) | `[[ "$output" == *"checkout not found"* ]]` | `[ -n "$(...)" ]` |
| "end-to-end..." (after provisioning) | `[[ "$output" != *"checkout not found"* ]]` | `[ -z "$(...)" ]` |

Confirmed zero `[[ ]]` occurrences remain in the file (`grep -n '\[\[' tests/fleet/fleet-provisioning.bats` → no matches).

### Integrity probe results (re-applied post-fix)

- **Probe A** — changed `fleet-lib.sh`'s fast-forward-skip message from
  `"would not fast-forward; skipping"` to `"would not FF; skip"`:
  test 5 ("a non-fast-forwardable update is skipped, not force-updated")
  went **RED**. Restored via `cp` + `diff` — byte-identical.
- **Probe B** — changed `fleet-init.sh`'s dry-run plan message from
  `"fleet: plan clone %s -> %s\n"` to `"fleet: planning clone %s -> %s\n"`:
  test 11 ("dry-run creates nothing and performs no git invocation") went
  **RED**. Restored via `cp` + `diff` — byte-identical.

Both probes used `git checkout --` at no point; restoration was always
`cp` of a pre-edit copy plus a `diff` confirming byte-identity.

### Minor: stale README/SKILL.md claims fixed

`skills/autospec-fleet/README.md` and `skills/autospec-fleet/SKILL.md` said
clone/sync was "not implemented yet." Updated both to describe the real,
idempotent `fleet-init.sh` provisioning now shipped, and re-ran
`scripts/derive-trio.sh skills/autospec-fleet --in-place` (the trio's
prose lives in `SKILL.md`/`codex/prompt.md`/`opencode/agent.md` in
lockstep) followed by `scripts/gen-skill-goldens.sh autospec-fleet` to
regenerate `tests/fixtures/skill-goldens/autospec-fleet.*.sha256` so the
derived copies and their goldens stay consistent with the edited source.

### Final verification (round 2)

- `bats tests/fleet/` — 41/41 pass.
- `bats tests/fleet/ tests/unit/test_autospec_fleet_url.bats` — 46/46 pass.
- `bash -n` clean on both touched scripts.
- `shellcheck` on both: only the same two pre-existing info-level notices
  (SC1091, SC2317) as round 1 — no new warnings.
- `git` stubbed on `PATH` in every test as before; stub-shadow confirmed
  with `[ "$(command -v git)" = "$TMP/bin/git" ]` in `setup()`. No test
  touches a real remote or the operator's home directory.

## Round 3 — marker name split (from independent re-review)

Round 2's fix made the dirty/non-ff skip loud, but used one shared marker
name (`fleet_provision_update_skipped`) discriminated only by a `reason=`
field. An independent re-review flagged that this is inconsistent with
every sibling marker in the same function — `fleet_provision_clone_failed`,
`_fetch_failed`, `_status_failed`, `_path_escape` — and with the analogous
set in `worktree-guard.sh` (`code_health:dirty`, `code_health:stale_base`,
`code_health:wrong_branch`), all of which carry the distinction in the
marker *name*, not a shared name plus a field. An operator following that
established convention would grep for a dirty-checkout-shaped name, get
zero hits, and conclude the condition never fired — the same fail-quiet
trap this whole fix round exists to close, one level down.

### New marker names

`fleet_update_checkout` in `fleet-lib.sh` now emits two distinct marker
names, matching the sibling naming shape in the same function:

- Dirty checkout: `code_health:fleet_provision_dirty_checkout repo=<normalized> path=<checkout_path>`
- Non-fast-forwardable: `code_health:fleet_provision_not_fast_forward repo=<normalized> path=<checkout_path>`

(The `reason=` field was dropped — the name alone now carries the
distinction, per the reviewer's requirement; `repo=` and `path=` are kept
on both, matching every other marker in the function.)

Full marker inventory for `fleet_update_checkout`/`fleet_clone_checkout`/
`fleet_provision_repo` after this round:

| Condition | Marker name |
|---|---|
| Clone fails | `fleet_provision_clone_failed` |
| `git status` fails | `fleet_provision_status_failed` |
| Dirty checkout (skip) | `fleet_provision_dirty_checkout` |
| `git fetch` fails | `fleet_provision_fetch_failed` |
| Update would not fast-forward (skip) | `fleet_provision_not_fast_forward` |
| Computed checkout path escapes workspace | `fleet_provision_path_escape` |
| Checkout path occupied by a non-git entry | `fleet_provision_path_occupied` |
| `mkdir -p` of parent fails | `fleet_provision_mkdir_failed` |

### Test updates

`tests/fleet/fleet-provisioning.bats` test 4 ("a checkout with uncommitted
changes is skipped, not reset") and test 5 ("a non-fast-forwardable update
is skipped, not force-updated") now assert the new marker names directly
(`code_health:fleet_provision_dirty_checkout` and
`code_health:fleet_provision_not_fast_forward` respectively), using the
same `[ -n "$(printf '%s' "$output" | grep -F -- '...')" ]` single-bracket
form as every other rewritten assertion from round 2 — safe in any
position, never a non-final `[[ ]]` or `!`. `grep -n '\[\[' tests/fleet/fleet-provisioning.bats`
confirms zero `[[` occurrences remain.

### Integrity probe results (round 3)

- **Probe C** — changed the dirty-checkout marker name in `fleet-lib.sh`
  from `fleet_provision_dirty_checkout` to `fleet_provision_dirty_repo`:
  test 4 went **RED**. Restored via `cp` + `diff` — byte-identical.
- **Probe D** — changed the non-ff marker name from
  `fleet_provision_not_fast_forward` to `fleet_provision_non_ff`: test 5
  went **RED**. Restored via `cp` + `diff` — byte-identical. Neither probe
  used `git checkout --`.

### No prose/trio changes needed

`grep -rn "fleet_provision_update_skipped\|reason=dirty_checkout\|reason=not_fast_forward"`
across `skills/` and `tests/` returned no hits before this round's edit —
neither the old nor new marker names are referenced in `SKILL.md`,
`README.md`, or the codex/opencode mirrors, so `derive-trio.sh --in-place`
and `gen-skill-goldens.sh` were not needed this round (goldens are
unchanged from round 2).

### Final verification (round 3)

- `bats tests/fleet/` — 41/41 pass.
- `bats tests/fleet/ tests/unit/test_autospec_fleet_url.bats` — 46/46 pass.
- `bash -n` clean on both touched scripts.
- `shellcheck` on both: only the same two pre-existing info-level notices
  (SC1091, SC2317) — no new warnings.
