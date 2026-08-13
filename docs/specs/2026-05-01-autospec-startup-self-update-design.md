# Autospec Startup Self-Update — Design Spec

**Date**: 2026-05-01
**Repo**: github.com/berlinguyinca/autospec
**Status**: Approved (Phase 2 brainstorm complete; ready to decompose into issues)

## 2026-08-13 reliability amendment

The success rate-limit and failure diagnostics are now distinct. The preflight writes
`last-update-check` only after an up-to-date check or completed install. A failed installer keeps
the prior working install and does not advance that 24-hour success guard, so the next invocation
can retry.

Installer output is retained in the bounded 64 KiB `~/.autospec/self-update.log`; the previous
bounded log is rotated once to `self-update.log.1`. A non-zero installer exit also atomically
writes private `~/.autospec/last-update-failure.json` containing the attempt timestamp, remote SHA,
exit code, output tail, and log path. `~/.autospec/remote-version` records the last successfully
resolved remote SHA. A successful check removes the stale failure record.

`autospec autonomous status --json` and `autospec autonomous list --json` expose these values in a
top-level `toolchain` object. Foreground conductor entry emits a non-blocking warning naming the
failure record when it exists. The recent-success fast path remains local and silent.

## 1. Goals

When the user invokes any autospec skill (`/autospec`, `/autospec-define`,
`/autospec-run`, `/autospec-listen`, `/autospec-classify`), it should
**transparently fetch and apply the latest version from `main`** before
running any pipeline phase, so installed copies never silently drift from
upstream.

Today the only refresh path is the manual `^\s*update\s*$` mode (each
skill's `## Self-update mode` section, e.g. `skills/autospec/SKILL.md:13`).
Users have to remember to run it; most don't. This spec adds an automatic
preflight that runs at startup, behind a 24h rate-limit, with a hard
fail-open contract so it never blocks legitimate work.

Non-goals: changing the existing manual `update` mode, adding cross-version
compatibility shims, supporting non-`main` channels (stable/beta), or
introducing a runtime version-pin file.

## 2. Architecture

### 2.1 Inline preflight in every skill's trio

Every multi-harness skill body grows a single `## Startup self-update`
section, inserted **above** the existing `## Self-update mode` section.
The section contains a small bash block (≤30 lines) the harness executes
at startup. The block is byte-identical across all multi-harness skills
(only the hard-coded `SKILL_NAME` differs) and **lock-stepped** across the trio
files (`SKILL.md` / `opencode/agent.md` / `codex/prompt.md`) per the
existing repo invariant (`autospec validate:44`).

No new shared script, no on-the-fly fetched bootstrap. Inline keeps each
SKILL.md self-contained, which matches how harnesses load skills and how
small-LLM implementers reason about them (per AGENTS.md "small-LLM
target").

### 2.2 State directory

Re-uses the existing `~/.autospec/` directory (already home to
`model-profiles.yml` and `project-map.yml`). Two new files:

```
~/.autospec/installed-version    # one line: short commit SHA (e.g. "9e52dcd")
~/.autospec/last-update-check    # one line: ISO-8601 UTC timestamp
```

Atomic writes via temp-file + `mv`. Both files are user-private and
optional — missing files mean "first run, do the check now."

### 2.3 install.sh integration

`install.sh` learns no new flags. The preflight block invokes the existing
canonical one-liner verbatim:

```bash
bash <(curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/<SKILL_NAME>/install.sh) --harness <H> --update
```

Then writes the freshly-fetched commit SHA to
`~/.autospec/installed-version`. The SHA comes from a **second** curl to
`https://api.github.com/repos/berlinguyinca/autospec/commits/main`
(`jq -r .sha | cut -c1-7`) — single round trip, ~200 bytes. If the API
call fails the preflight skips silently (fail-open).

### 2.4 Side-fix: legacy URL in `skills/autospec/install.sh`

`skills/autospec/install.sh` lines 12 and 29 still reference the legacy
`berlinguyinca/codex-skills` repo. This bug only triggers in the
piped-fallback fetch path, but the new preflight relies on that path, so
it must be fixed as part of this spec. The other four skills' installers
already use `berlinguyinca/autospec` (verified 2026-05-01).

## 3. Interactivity / API

### 3.1 Default behavior — silent rate-limited check + auto-apply

```
$ /autospec brainstorm a search feature
[autospec] checking for updates...
[autospec] updated 9e52dcd → b1f2c30 (claude, opencode)
Phase 0: ...
```

Or, on no change:

```
$ /autospec brainstorm a search feature
Phase 0: ...
```

(Silent — no banner when up-to-date.)

### 3.2 Rate limit

If `~/.autospec/last-update-check` exists and its timestamp is within the
last 24h, the preflight skips entirely (no network call). The existing
manual `/<skill> update` invocation remains the escape hatch when a user
wants an immediate refresh.

### 3.3 Opt-out

Setting environment variable `AUTOSPEC_NO_SELF_UPDATE=1` skips the
preflight entirely (no network call, no banner). Documented in
`AGENTS.md` and `README.md`.

### 3.4 No re-exec

After a successful update, the **current invocation continues on the
in-memory body** (the harness has already loaded the pre-update copy).
The next invocation picks up the new body. Re-exec mid-skill is
explicitly out of scope — no harness exposes a clean primitive for it
and the round-trip cost would double user wait time on every refresh.

### 3.5 Concurrency

Cross-skill races (e.g. `/autospec-listen` and `/autospec` running at
once) are serialized via `flock` on `~/.autospec/.update.lock`. If the
lock is contended, the loser fails open with a one-line WARN.

### 3.6 Spec-PR auto-merge authority

Per user directive 2026-05-01, the autospec pipeline must run end-to-end
without surfacing PR-merge questions for *spec* artifacts the same way
AGENTS.md already grants Phase 4 auto-merge authority for
`auto-implement` PRs. Concretely:

- `AGENTS.md` `## Auto-merge authority` section grows a sibling
  paragraph granting admin-merge for **spec PRs**, identified as PRs
  whose head branch matches `feat/spec-*` *or* whose body contains a
  `Source spec` line referencing `docs/specs/`.
- `skills/autospec/SKILL.md` Phase 2 ends with the orchestrator
  admin-merging the spec PR (`gh pr merge <#> --admin --squash
  --delete-branch`) **before** dispatching the Phase 3 decomposer, so
  child issues always reference a canonical `main` URL.
- `skills/autospec-define/SKILL.md` mirrors the same Phase 2 step
  (lock-step preserved).
- The escape hatch `AUTOSPEC_NO_AUTOMERGE_SPEC=1` short-circuits the
  auto-merge and falls back to today's "open PR + ask user" flow, for
  contributors who want to keep humans in the loop.

This is NOT a runtime feature — the bash preflight in §6 is unchanged.
It's a workflow-doc + prompt-text change that piggybacks on the same
delivery train.

## 4. Data model

| Path                              | Format                       | Writer                       | Reader                        |
|-----------------------------------|------------------------------|------------------------------|-------------------------------|
| `~/.autospec/installed-version`   | `<7-char SHA>\n`             | preflight (post-update)      | preflight (compare on next)   |
| `~/.autospec/last-update-check`   | `<ISO-8601 UTC>\n`           | preflight (every check)      | preflight (rate-limit gate)   |
| `~/.autospec/.update.lock`        | flock-only, contents ignored | preflight                    | preflight                     |

All three are user-local (`$HOME`-rooted), zero-byte safe (missing →
"first run"), and survive across sessions.

## 5. Error handling

Fail-open contract — every failure mode prints exactly one WARN line and
proceeds to the existing pipeline. No retries, no fallback fetch, no
re-exec.

| Scenario                          | Behavior                                                          |
|-----------------------------------|-------------------------------------------------------------------|
| `AUTOSPEC_NO_SELF_UPDATE=1`       | silent skip (no WARN)                                             |
| Last check < 24h ago              | silent skip (no WARN)                                             |
| Network down / DNS fail           | `WARN: self-update skipped (network); continuing on installed`    |
| GitHub API 5xx / 429              | `WARN: self-update skipped (api rc=N); continuing on installed`   |
| `install.sh` non-zero exit        | `WARN: self-update skipped (install rc=N); continuing on installed` |
| flock contended                   | `WARN: self-update skipped (concurrent update in progress)`       |
| Disk full / permission on state   | `WARN: self-update skipped (state write); continuing on installed`|
| Successful no-op (already current)| no WARN, no banner                                                |
| Successful update applied         | `[autospec] updated <old> → <new> (<harnesses>)`                  |

The preflight runs with `set +e` and uses explicit `$?` checks. It must
not abort the calling shell on any failure.

## 6. Bash preflight contract (canonical reference)

The exact bash inserted into every skill's `## Startup self-update`
section is below. It is the authoritative source — the validator
(see §7.3) extracts it from `skills/autospec/SKILL.md` and asserts
the other four skills carry the byte-identical block (modulo the
`SKILL_NAME` substitution).

```bash
#!/usr/bin/env bash
# autospec-startup-self-update — see docs/specs/2026-05-01-autospec-startup-self-update-design.md
set +e
SKILL_NAME=autospec   # per-skill: autospec-define / autospec-run / autospec-listen / autospec-classify
if [ "${AUTOSPEC_NO_SELF_UPDATE:-0}" = "1" ]; then exit 0; fi
mkdir -p "$HOME/.autospec"
LOCKDIR="$HOME/.autospec/.update.lock.d"
LAST="$HOME/.autospec/last-update-check"
INSTALLED="$HOME/.autospec/installed-version"
NOW=$(date -u +%s)
if [ -f "$LAST" ]; then
    PREV=$(date -u -j -f '%Y-%m-%dT%H:%M:%SZ' "$(cat "$LAST" 2>/dev/null)" +%s 2>/dev/null \
        || date -u -d "$(cat "$LAST" 2>/dev/null)" +%s 2>/dev/null || echo 0)
    if [ "$((NOW - PREV))" -lt 86400 ]; then exit 0; fi
fi
if ! mkdir "$LOCKDIR" 2>/dev/null; then
    echo "WARN: self-update skipped (concurrent update in progress)" >&2; exit 0
fi
trap 'rmdir "$LOCKDIR" 2>/dev/null' EXIT
date -u +'%Y-%m-%dT%H:%M:%SZ' > "$LAST.tmp" && mv "$LAST.tmp" "$LAST"
REMOTE=$(curl -fsSL --max-time 5 \
    "https://api.github.com/repos/berlinguyinca/autospec/commits/main" \
    2>/dev/null | jq -r '.sha // empty' 2>/dev/null | cut -c1-7)
if [ -z "$REMOTE" ]; then
    echo "WARN: self-update skipped (network); continuing on installed version" >&2; exit 0
fi
LOCAL=$(cat "$INSTALLED" 2>/dev/null || true)
if [ "$REMOTE" = "$LOCAL" ]; then exit 0; fi
bash <(curl -fsSL --max-time 30 \
    "https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/$SKILL_NAME/install.sh") \
    --harness all --update >/dev/null 2>&1
RC=$?
if [ "$RC" -ne 0 ]; then
    echo "WARN: self-update skipped (install rc=$RC); continuing on installed version" >&2; exit 0
fi
printf '%s\n' "$REMOTE" > "$INSTALLED.tmp" && mv "$INSTALLED.tmp" "$INSTALLED"
echo "[autospec] updated ${LOCAL:-fresh} → $REMOTE"
```

Notes:

- The block runs as a child process (the harness's Bash-tool invocation),
  so `exit 0` terminates only the preflight, not the surrounding skill.
  The LLM observes stdout/stderr and proceeds to Phase 0 regardless.
- Rate-limit timestamp is written **before** the network probe so sustained
  failures (offline, GitHub outage) don't hammer upstream every invocation.
- Atomic state-file writes use temp + `mv`. Concurrency control is a POSIX
  `mkdir` lockdir (`flock` is not stock on macOS).
- Required runtime deps: `curl`, `jq`, `date`. All three are already
  in `skills/autospec/install.sh:220-222` dep-warning list. No new deps.

## 7. Testing

Per AGENTS.md: "validation in lieu of code tests" — bats-core for shell
behavior, lock-step diff in `autospec validate`, e2e against a real gh
clone for the integration path. No service mocks.

### 7.1 Unit (bats)

`tests/unit/test_self_update_preflight.bats` extracts the canonical bash
block from `skills/autospec/SKILL.md` via awk, sources it into a
sandboxed `$HOME` (using a temp dir), and exercises:

| Scenario                         | Setup                                              | Expect                            |
|----------------------------------|----------------------------------------------------|-----------------------------------|
| First run                        | empty `$HOME/.autospec/`                           | network attempted (or fail-open)  |
| Within 24h rate-limit            | `last-update-check` set to `now - 1h`              | no network, exit 0, no WARN       |
| Past 24h                         | `last-update-check` set to `now - 25h`             | network attempted                 |
| Opt-out env var                  | `AUTOSPEC_NO_SELF_UPDATE=1`                        | no network, exit 0, no WARN       |
| Network fail (curl exit 6)       | `PATH` shimmed with failing curl                   | WARN logged, exit 0               |
| install.sh fail (exit 2)         | shimmed bash exits non-zero on the install line    | WARN logged, exit 0               |
| Lock contention                  | hold `flock` from sibling shell                    | WARN logged, exit 0               |
| Up-to-date no-op                 | `installed-version` matches stub remote            | no WARN, no banner                |

### 7.2 E2E (bats, `e2e` label-gated)

`tests/e2e/test_self_update_apply.bats` runs against a throwaway gh
clone:

1. `git clone berlinguyinca/autospec` into a tmp dir.
2. `git checkout` to a parent commit (`HEAD~5`).
3. Run `install.sh --harness claude` against an isolated `CLAUDE_CONFIG_DIR`.
4. Seed `~/.autospec/installed-version` with the parent SHA, clear
   `last-update-check`.
5. Source the preflight block.
6. Assert: post-state `installed-version` matches `HEAD`, the installed
   `SKILL.md` matches `main`, `last-update-check` is fresh.

This test piggybacks on the existing e2e workflow gating
(`.github/workflows/e2e.yml`).

### 7.3 Validator extension

`autospec validate` grows one new check, `check_startup_preflight`,
that asserts:

- Every multi-harness skill's `SKILL.md` has a `## Startup self-update`
  section.
- The bash block extracted from each skill's `SKILL.md` is byte-identical
  (modulo the `SKILL_NAME=` first line) to the block in
  `skills/autospec/SKILL.md` — the canonical source.
- Lock-step rule already covers trio internal consistency (no new check
  needed there).

## 8. Documentation

Two doc updates:

- **README.md**: a new short paragraph in the "Installation" section
  explaining auto-update, the 24h cadence, and the `AUTOSPEC_NO_SELF_UPDATE`
  opt-out. One sentence in the FAQ.
- **AGENTS.md**: a new `## Startup self-update` heading codifying the
  fail-open contract and the opt-out env var, mirroring the existing
  `## Auto-merge authority` heading style.

## 9. Decomposition outline

EPIC umbrella: **Add startup self-update to autospec skills**.

Children (each ≤400 words, ≤3 files, ≤30-line outline):

| #  | Title                                                                   | Files                            | Deps        |
|----|-------------------------------------------------------------------------|----------------------------------|-------------|
| 1  | Fix legacy `codex-skills` URL in `skills/autospec/install.sh`           | 1                                | —           |
| 2  | Add canonical preflight to `autospec` trio (reference implementation)   | 3 (trio)                         | 1           |
| 3  | Mirror preflight to `autospec-define` trio                              | 3 (trio)                         | 2           |
| 4  | Mirror preflight to `autospec-run` trio                                 | 3 (trio)                         | 2           |
| 5  | Mirror preflight to `autospec-listen` trio                              | 3 (trio)                         | 2           |
| 6  | Mirror preflight to `autospec-classify` trio                            | 3 (trio)                         | 2           |
| 7  | Extend `autospec validate` with `check_startup_preflight`             | 1                                | 6           |
| 8  | Bats unit tests for preflight (rate-limit, opt-out, fail-open, lock)    | 1                                | 2           |
| 9  | Bats e2e test (parent SHA → invoke → verify update applies)             | 1                                | 6           |
| 10 | Doc updates: `README.md` + `AGENTS.md` (auto-update behavior)           | 2                                | 6           |
| 11 | Extend `AGENTS.md` `## Auto-merge authority` to cover spec PRs          | 1                                | —           |
| 12 | Wire spec-PR auto-merge into `autospec` Phase 2 trio                    | 3 (trio)                         | 11          |
| 13 | Wire spec-PR auto-merge into `autospec-define` Phase 2 trio             | 3 (trio)                         | 11          |
| 14 | Bats unit test: spec-PR auto-merge happy path + opt-out env var         | 1                                | 12          |

Total: 14 child issues + 1 umbrella.

## 10. Out of scope

- Channel selection (stable / beta / pinned commit).
- Auto-update of `~/.autospec/model-profiles.yml` or `project-map.yml`
  (those are user-owned config; out of bounds).
- A `--no-self-update` CLI flag on each skill (env var is enough).
- Telemetry on update success rate.
- Mid-skill re-exec after successful update (see §3.4).
- Version-pin file at the repo level (relies entirely on `main` HEAD).

## 11. Open questions

None at spec time. All five Phase 2 questions resolved.
