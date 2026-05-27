---
description: Use when the user wants to adopt a vendor design language for the current repo — fetches the per-vendor DESIGN.md from the berlinguyinca/awesome-design-md catalog, scores the repo against candidate vendors, writes DESIGN.md at the project root, and optionally hands off a migration spec to /autospec-define. Subcommands suggest, apply, and migrate.
mode: primary
---

# autospec-design (harness-neutral)

Skill scaffold for the **autospec-design** workflow. The full workflow lets the
user pick a vendor design language (Linear, Stripe, Vercel, ...) from a curated
catalog, write `DESIGN.md` at the project root, and optionally hand off a
migration spec into `/autospec-define`. The catalog lives at
`berlinguyinca/awesome-design-md` and is fetched at runtime — never vendored.

This file is the **autospec-design trio body**. The `suggest` and `apply`
subcommands are implemented; `migrate` still holds a placeholder until issue
#580 lands.

Manage your own context — never exceed 60%. Delegate to subagents whenever your harness supports it.

## Startup self-update

```bash
#!/usr/bin/env bash
# autospec-startup-self-update — see docs/specs/2026-05-01-autospec-startup-self-update-design.md
set +e
SKILL_NAME=autospec-design   # per-skill: autospec-define / autospec-run / autospec-listen / autospec-classify
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
curl -fsSL --max-time 30 \
    "https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh" \
    | bash -s -- --skill all --harness all --update >/dev/null 2>&1
RC=$?
if [ "$RC" -ne 0 ]; then
    echo "WARN: self-update skipped (install rc=$RC); continuing on installed version" >&2; exit 0
fi
printf '%s\n' "$REMOTE" > "$INSTALLED.tmp" && mv "$INSTALLED.tmp" "$INSTALLED"
# Auto-init cross-tool memory (idempotent, <50ms fast-path)
bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/auto-init-memory.sh"
echo "[autospec] updated ${LOCAL:-fresh} → $REMOTE"
```

## Self-update mode

If the feature-request argument matches the regex `^\s*update\s*$` (case-insensitive, whitespace-padded), this skill enters self-update mode and does not run the normal pipeline:

1. **Detect harness** by checking which install path exists for this skill:
   - Claude Code: `~/.claude/skills/autospec-design/SKILL.md`
   - OpenCode:    `~/.config/opencode/agent/autospec-design.md`
   - Codex CLI:   `~/.codex/prompts/autospec-design.md`
2. **Re-install the full autospec suite from `main`** by piping the canonical installer:
   ```bash
   curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh | bash -s -- --skill all --harness all --update
   ```
   Run this one-liner once; it refreshes all autospec skills across all harnesses.
3. **Show the diff** between the prior installed file(s) and the freshly fetched copy (e.g. `diff <(cat <prior>) <(curl -fsSL ...SKILL.md)` or the equivalent recorded by the installer).
4. **Stop.** Do not enter any subcommand. Print the upgrade summary and return to the user.

If no install path is detected, print `Self-update: no installed copy of autospec-design found; run install.sh first.` and exit.

## Invocation

```
/autospec-design suggest
/autospec-design apply <vendor> [--force] [--branch <name>]
/autospec-design migrate <vendor> [--branch <name>]
```

- `suggest` — scan the current repo, score every catalog vendor by the
  framework / domain / brand-language rubric, present the top 3 with a
  one-line rationale each. Prints only — never modifies any files.
- `apply <vendor>` — fetch `<vendor>/DESIGN.md` from the catalog, write it
  to `<repo-root>/DESIGN.md` on a fresh feature branch (default:
  `feat/design-<vendor>`), commit, and push. Refuses to overwrite an
  existing `DESIGN.md` without `--force`. Does **not** open a PR.
- `migrate <vendor>` — generate a migration spec at
  `docs/specs/<YYYY-MM-DD>-design-migration-<vendor>.md` from the scanned
  UI inventory and hand off to `/autospec-define <spec-path>`.

The skill exits non-zero if `gh auth status` fails when issuing a network
operation that needs authentication. See spec
`docs/specs/2026-05-26-autospec-design-skill.md` for the full contract.

## Required capabilities & harness adapter

This workflow assumes a small set of capabilities. Map each one to your harness's actual tool. If a capability is missing, use the listed fallback.

| Capability                  | Claude Code                          | OpenCode                                 | Codex CLI                                | Fallback if missing                                |
|-----------------------------|--------------------------------------|------------------------------------------|------------------------------------------|----------------------------------------------------|
| Read-only codebase research | `Agent` (subagent_type=Explore)      | `task` agent in read-only mode           | `apply_patch` read-only / shell `grep`   | Do the search in-thread with `rg`/`grep`           |
| Foreground delegation       | `Agent` (subagent_type=general-purpose) | nested `task` agent, await output     | spawn nested CLI session                 | Do the work in-thread (more context cost)          |
| Ask the user a question     | `AskUserQuestion`                    | inline prompt                            | inline prompt                            | Ask in the response and wait for the next turn     |
| Subagent model tier         | Tier A: `opus` + `ultrathink`; Tier B: `sonnet` + medium thinking | Tier A: top `task` model + high reasoning; Tier B: smaller-tier `task` + medium reasoning | Tier A: top GPT + `reasoning_effort=high`; Tier B: `gpt-5.1-codex-spark` + `reasoning_effort=medium` | Honor the per-phase tier mapping in AGENTS.md; retry the same subagent UP on unavailability |

**Persistent project notes**: write durable preferences to **`AGENTS.md`** in the repo root — this is the de-facto standard recognized by Claude Code (also reads `CLAUDE.md`), OpenCode, and Codex. Per AGENTS.md, subagent dispatches use a **two-tier policy**: Tier A (top model + extended thinking) for spec work, Tier B (cheaper model + medium thinking) for implementation work. The orchestrator keeps the user's invoked model. Fall back UP the tier on quota/capacity or other unavailability by retrying the same subagent with the stronger tier while preserving parent context.

## Harness detection (run once at skill start, before any subcommand)

Detect your harness by checking available tools before any subcommand:

1. **Claude Code** — the `Agent` tool with a `subagent_type` parameter is available.
   - `TIER_A` = `opus` + `ultrathink`  (model ID: claude-opus-4-7)
   - `TIER_B` = `sonnet`               (model ID: claude-sonnet-4-6)

2. **OpenCode** — a `task` tool with model/tier configuration is available (no `subagent_type`).
   - `TIER_A` = top-tier task model + high reasoning
   - `TIER_B` = smaller-tier task model + medium reasoning

3. **Codex CLI** — neither `Agent` nor a configurable `task` tool is available; `apply_patch` is the primary edit tool.
   - `TIER_A` = current top GPT model + `reasoning_effort=high`
   - `TIER_B` = `gpt-5.1-codex-spark` + `reasoning_effort=medium`

**Fallback rule:** If `TIER_B` is not available in your harness (model unknown, quota/capacity failure, authorization failure, or tool call returns an error for that model), silently retry the same subagent dispatch with `TIER_A`. Preserve the parent context on retry; for Codex native subagents, fork/inherit the current conversation context and use the latest top GPT model instead of moving the work into the main session. Never ask the user.

Hold `TIER_A` and `TIER_B` for the entire skill run. Every "Tier A" and "Tier B" reference below resolves to these harness-specific values.

## Catalog source (single source of truth)

Fetched at runtime, never vendored:

```bash
AUTOSPEC_DESIGN_CATALOG_OWNER="${AUTOSPEC_DESIGN_CATALOG_OWNER:-berlinguyinca}"
AUTOSPEC_DESIGN_CATALOG_REPO="${AUTOSPEC_DESIGN_CATALOG_REPO:-awesome-design-md}"
AUTOSPEC_DESIGN_CATALOG_REF="${AUTOSPEC_DESIGN_CATALOG_REF:-main}"
```

Per-vendor DESIGN.md is fetched via `gh api repos/${owner}/${repo}/contents/design-md/${vendor}/DESIGN.md?ref=${ref} --jq '.content' | base64 -d`, with a fallback to `curl -fsSL https://raw.githubusercontent.com/...`. Cache lands under `~/.autospec/design-cache/<vendor>/DESIGN.md` with a 24h freshness window. The catalog fetcher script lands in issue #576 (`fetch-design-md.sh`).

## Suggest

> **Model tier:** `TIER_B` (implementation work) — scoring is mechanical; the
> rubric is a fixed table and the helper is deterministic.

`suggest` scans the current repo, scores every catalog vendor against a fixed
rubric, and presents the top 3 candidates with a one-line rationale each. It is
**read-only** — it never modifies any file in the target repo.

Run the deterministic scorer (it prints tab-separated `<score>\t<vendor>\t<rationale>`
lines, highest score first):

```bash
bash skills/autospec-design/scripts/score-suggestion.sh <repo-root>
```

The scorer:

1. Fetches the catalog vendor list once (via `gh api .../contents/design-md`,
   `curl` fallback). When you have already fetched it, pass it in via
   `AUTOSPEC_DESIGN_VENDORS` to avoid a second round-trip.
2. Detects repo signals: framework (`package.json`, `next.config.*`,
   `vite.config.*`, `angular.json`, `svelte.config.*`, Tailwind config), brand
   keywords (README + `package.json` `name`/`description`), and product-domain
   keywords (README).
3. Scores each vendor on the rubric below (cap 6):
   - **Framework match (+2)** — repo uses a web/JS UI framework AND the vendor
     is a developer / SaaS / web-product brand (e.g. Linear, Vercel, Stripe).
   - **Brand match (+1)** — the vendor's normalized name appears in the repo's
     README or `package.json`.
   - **Domain match (+1)** — a vendor-name token overlaps the repo README.

Present the top 3 to the user with their rationales and let them pick. Then run
`/autospec-design apply <vendor>` with the chosen vendor. Do **not** apply
automatically — `suggest` only recommends.

## Apply

> **Model tier:** `TIER_B` (implementation work) — write-out is a deterministic
> branch-and-commit; no novel design.

`apply <vendor> [--force] [--branch <name>]` fetches `<vendor>/DESIGN.md` from
the catalog and writes it to `<repo-root>/DESIGN.md` on a fresh feature branch,
commits, and pushes. It is **mechanical** — no subagent dispatch. The flow:

1. Parse `<vendor>` plus optional `--force` and `--branch <name>`.
2. Validate `<vendor>` exists in the catalog by delegating to
   `fetch-design-md.sh` (its exit code is the validation — a missing vendor
   exits non-zero and prints Levenshtein hints; do not duplicate that logic).
3. If `DESIGN.md` already exists at the repo root and `--force` is absent:
   refuse, print a `git diff` hint, and exit non-zero. Never silently
   overwrite.
4. Refuse to commit on `main` — when `--branch` is absent, auto-create
   `feat/design-<vendor>`; otherwise use the supplied branch name.
5. Fetch the DESIGN.md body (cache hit or network) via `fetch-design-md.sh`.
6. Write the body to `<repo-root>/DESIGN.md`.
7. `git add DESIGN.md`, commit with `feat(design): adopt <vendor> design
   language`, and push the branch (`git push -u origin <branch>`).
8. Print a status summary: branch name, DESIGN.md byte count, and the
   next-step hint (`open a PR, or run /autospec-design migrate <vendor>`).

`apply` never opens a PR — the operator does that, or `/autospec-design
migrate` takes over. It is idempotent: a second run on the same branch with the
same vendor rewrites identical bytes and exits without creating a duplicate
commit.

The deterministic flow below is the canonical script for the steps above. Run
it verbatim (it reads `<vendor>` from the invocation; the `DESIGN_*` overrides
exist only so the test harness can point it at a fixture repo and a seeded
cache). It honors `--force`/`--branch` via the `DESIGN_FORCE`/`DESIGN_BRANCH`
mappings the wrapper sets from the parsed flags.

```bash
# autospec-design-apply-flow
set -u
vendor="${DESIGN_VENDOR:?vendor required: /autospec-design apply <vendor>}"
repo_root="${DESIGN_REPO_ROOT:-$(git rev-parse --show-toplevel)}"
fetch="${DESIGN_FETCH:-skills/autospec-design/scripts/fetch-design-md.sh}"
branch="${DESIGN_BRANCH:-feat/design-$vendor}"
target="$repo_root/DESIGN.md"

# Guard: refuse overwrite without --force.
if [ -e "$target" ] && [ "${DESIGN_FORCE:-0}" != "1" ]; then
    printf 'autospec-design apply: %s already exists. Re-run with --force to overwrite (inspect with: git -C %q diff -- DESIGN.md).\n' \
        "$target" "$repo_root" >&2
    exit 1
fi

# Validate vendor + fetch body (non-zero here = unknown vendor or network).
body="$(bash "$fetch" "$vendor")" || exit $?

# Never commit on main: create/switch to the feature branch.
git -C "$repo_root" checkout -B "$branch" >/dev/null 2>&1 || {
    printf 'autospec-design apply: could not create branch %s\n' "$branch" >&2
    exit 1
}

# Write, stage, commit.
printf '%s' "$body" > "$target"
git -C "$repo_root" add DESIGN.md
if git -C "$repo_root" diff --cached --quiet -- DESIGN.md; then
    printf 'autospec-design apply: DESIGN.md already matches %s on branch %s.\n' \
        "$vendor" "$branch"
else
    git -C "$repo_root" commit -q -m "feat(design): adopt $vendor design language"
fi

# Push unless the test harness suppresses it.
if [ "${DESIGN_NO_PUSH:-0}" != "1" ]; then
    git -C "$repo_root" push -u origin "$branch"
fi

bytes="$(wc -c < "$target" | tr -d ' ')"
printf 'autospec-design apply: wrote %s (%s bytes) on branch %s.\n' \
    "$target" "$bytes" "$branch"
printf 'Next: open a PR, or run /autospec-design migrate %s.\n' "$vendor"
```

## Subcommand: migrate (placeholder)

> **Model tier:** `TIER_B` (implementation work) — emits a structured spec and
> hands off to `/autospec-define`.

Scaffold-only placeholder. Real prose lands in issue #580. Until then, the
migrate subcommand prints:

```
/autospec-design migrate: full implementation lands in issue #580.
```

## Hard rules

- Never overwrite an existing `DESIGN.md` without `--force`.
- Never open a PR automatically — the operator opens it (or `/autospec-define`
  takes over via the migration spec).
- Never modify the issue title (when running via the autospec pipeline).
- Always idempotent — running `apply` twice with the same vendor on the same
  branch is a no-op after the first commit.
- `gh` CLI only for GitHub ops; `curl` only as a fallback for catalog reads.
