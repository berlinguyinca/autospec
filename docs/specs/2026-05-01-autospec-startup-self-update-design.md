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
top-level `toolchain` object. Both `autonomous start` and direct foreground conductor entry emit a
non-blocking warning naming the failure record when it exists. The recent-success fast path remains
local and silent.

## 2026-08-16 harness-substitution amendment (issue #3177)

The preflight shell no longer lives inlined in the injected markdown block. It lives in
`scripts/autospec-startup-self-update.sh`, and the block only resolves and invokes it.

A harness substitutes positional parameters inside a *rendered skill body* at load time. Because
the expanded block was shell inlined into `SKILL.md` / `prompts/<skill>.md` /
`agent/<skill>.md`, `target="$1"` in `write_autonomous_operator_wrapper()` was rendered as
`target="<first argument to the slash command>"`. The function then ignored its own argument and
wrote a wrapper script — plus `chmod +x` — over that path on every iteration of
`heal_autonomous_operator_wrappers()`, which runs *before* the daily throttle. Passing a real file
path as a skill argument overwrote that file. The repository source was clean; only the installed
copies under `~/.claude/skills/`, `~/.codex/prompts/`, and `~/.config/opencode/agent/` carried the
substituted text, so a grep of this repo showed nothing.

Shell that lives in a `.sh` file is never rendered by a harness, so its positional parameters stay
positional. `StructuralValidator::validate_startup_preflight` now rejects any `="$1"` / `="$2"`
assignment in the block and asserts the behavioral contract against the extracted script.

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

### 2.1 Preflight invocation in every skill's trio

Every multi-harness skill body carries an `autospec-block:startup-self-update` marker above its
existing self-update section. At install/load time, `scripts/expand-skill-blocks.sh` expands that
marker from `templates/skill-blocks/startup-self-update.md`, substituting only `SKILL_NAME`. This
keeps one executable source while preserving byte-identical harness bodies and derived golden
proof for every marked `SKILL.md`, `opencode/agent.md`, and `codex/prompt.md`.

The expanded block contains no self-update logic of its own (see the 2026-08-16 amendment). It
resolves `autospec-startup-self-update.sh` through the standard three-way fallback —
`$SCRIPT_DIR`, then `${AUTOSPEC_REPO_ROOT:-$PWD}/scripts`, then
`${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}` — and invokes it with the skill name as an
inert provenance label. A missing script degrades to a `WARN:` line and exit 0, matching the
fail-open contract. `install.sh`'s `copy_repo_scripts()` globs `scripts/*.sh`, so the extracted
script ships on every fresh install and `--update`.

### 2.2 State directory

Re-uses the existing `~/.autospec/` directory. The preflight and autonomous status surfaces share
these files:

```
~/.autospec/installed-version         # installed short commit SHA
~/.autospec/remote-version            # last resolved remote short SHA
~/.autospec/last-update-check         # last successful check, ISO-8601 UTC
~/.autospec/last-update-failure.json  # durable failed-installer evidence
~/.autospec/self-update.log{,.1}      # current and one rotated bounded log
```

State receipts use guarded temp-file plus `mv` publication. The installed-version receipt is backed
up before publication and restored (or removed when it did not previously exist) if the paired
successful-check timestamp cannot publish. The state block uses `umask 077`, and the diagnostic log
is pre-created at mode `0600` before installer output is piped into it.
Missing files are valid and mean no successful check, no known version, or no recorded failure.

### 2.3 install.sh integration

The preflight downloads the canonical suite bootstrap to a temporary file, then invokes it for all
skills and harnesses:

```bash
bash "$BOOTSTRAP_TMP" --skill all --harness all --update
```

Installer stdout/stderr streams through `tail -c 65536` into the bounded log while Bash preserves
the installer's exit status through `PIPESTATUS`. Only a zero installer exit followed by successful
atomic publication updates `installed-version` and `last-update-check`. Any failure keeps the
working installed receipt and exits zero after a warning.

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
wants an immediate refresh. Failed network, installer, or state-publication attempts never refresh
this success timestamp.

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

Cross-skill races (e.g. `/autospec-listen` and `/autospec` running at once) are serialized by
atomically creating `~/.autospec/.update.lock.d`, which is portable to macOS. If the lock directory
already exists, the loser fails open with a one-line WARN.

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

| Path                                      | Format                    | Writer                  | Reader                  |
|-------------------------------------------|---------------------------|-------------------------|-------------------------|
| `~/.autospec/installed-version`           | `<7-char SHA>\n`          | successful install      | preflight + status      |
| `~/.autospec/remote-version`              | `<7-char SHA>\n`          | successful remote probe | preflight + status      |
| `~/.autospec/last-update-check`           | `<ISO-8601 UTC>\n`        | successful check only   | preflight rate limit    |
| `~/.autospec/last-update-failure.json`    | failure evidence JSON     | failed installer        | start + status          |
| `~/.autospec/self-update.log{,.1}`        | bounded diagnostic text   | installer attempt       | operator                |
| `~/.autospec/.update.lock.d`              | directory lock            | preflight                | preflight               |

All state is user-local (`$HOME`-rooted), zero-byte safe (missing →
"first run"), and survives across sessions.

## 5. Error handling

Fail-open contract — every failure mode prints a bounded WARN and proceeds to the existing
pipeline. A failed attempt does not replace the installed version or advance the successful-check
guard, so a later invocation can retry. There is no in-process retry or re-exec.

| Scenario                          | Behavior                                                          |
|-----------------------------------|-------------------------------------------------------------------|
| `AUTOSPEC_NO_SELF_UPDATE=1`       | silent skip (no WARN)                                             |
| Last check < 24h ago              | silent skip (no WARN)                                             |
| Network down / DNS fail           | `WARN: self-update skipped (network); continuing on installed`    |
| GitHub API 5xx / 429              | `WARN: self-update skipped (api rc=N); continuing on installed`   |
| installer non-zero exit           | persist record/log; WARN with both paths; continue installed       |
| lock directory contended          | `WARN: self-update skipped (concurrent update in progress)`       |
| State receipt publication fails  | WARN with failed path; keep old receipt; no success timestamp/banner |
| Successful no-op (already current)| no WARN, no banner                                                |
| Successful update applied         | `[autospec] updated <old> → <new> (<harnesses>)`                  |

The preflight runs with `set +e` and uses explicit `$?` checks. It must
not abort the calling shell on any failure.

## 6. Bash preflight contract (canonical reference)

The authoritative executable body is
[`scripts/autospec-startup-self-update.sh`](../../scripts/autospec-startup-self-update.sh). The
injected block
[`templates/skill-blocks/startup-self-update.md`](../../templates/skill-blocks/startup-self-update.md)
only resolves and invokes it, and must never inline shell that assigns a positional parameter.
Skill files contain `autospec-block:startup-self-update` markers, expanded by
`scripts/expand-skill-blocks.sh`; generated SHA fixtures prove every marked harness surface expands
from that one source. `StructuralValidator::validate_startup_preflight` additionally requires the
durable failure record, bounded log, remote version, and success-only guard ordering — asserted
against the extracted script.

The block runs as a child process, so fail-open `exit 0` terminates only the preflight. Atomic state
writes use temp-file plus `mv`, and concurrency uses the portable `mkdir` lock directory. Runtime
dependencies remain `curl`, `jq`, and `date`; this amendment adds no dependency.

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
| installer fails (exit 2)         | bootstrap exits non-zero                           | WARN logged, exit 0               |
| Installer diagnostics            | failing installer emits output                     | bounded log + durable JSON        |
| Failed installer retry           | run the same failing attempt twice                 | no 24h suppression; log rotates   |
| Installed receipt rename fails   | selective failing `mv`                             | old receipt; no timestamp/banner  |
| Lock contention                  | pre-create the portable lock directory             | WARN logged, exit 0               |
| Up-to-date no-op                 | `installed-version` matches stub remote            | no WARN, no banner                |

Rust CLI tests additionally require explicit `toolchain` JSON fields from `status` and `list`, a
warning on direct `run-foreground`, and a warning visible to the invoking operator on background
`start` before its child stderr is redirected to the conductor log.

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
