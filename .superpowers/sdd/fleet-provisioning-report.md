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
