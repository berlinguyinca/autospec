---
description: Use when the user wants /autospec-explore to start a perpetual autonomous research + ship loop on an isolated sandbox branch — 6 researchers propose features from spec/code gaps, prior reports, codebase signals, open issues, repo source analysis, and competitor research, then drain via /autospec-run with PRs targeting the sandbox branch (never main).
mode: primary
---

# autospec-explore workflow (harness-neutral)

Start a perpetual autonomous research + ship loop. `/autospec-explore "<initial prompt>"`
creates an isolated sandbox branch (`autospec/explore/<date>-<slug>`) off `origin/main`,
runs 6 parallel researchers each round (spec-vs-code, prior reports, codebase signals,
open issues, source analysis, internet), files 1-5 auto-implement issues per round,
drains them via `/autospec-run` with PRs targeting the sandbox, and continues until
the operator stops it. The operator inspects the sandbox when ready and either merges
into `main` or discards.

Manage your own context — never exceed 60%. Delegate to subagents whenever your
harness supports it; do not run researchers or aggregate proposals directly in the
main conversation when a subagent can do it.

## Startup self-update

```bash
#!/usr/bin/env bash
# autospec-startup-self-update — see docs/specs/2026-05-01-autospec-startup-self-update-design.md
set +e
SKILL_NAME=autospec-explore   # per-skill: autospec-define / autospec-run / autospec-listen / autospec-classify / autospec-refine / autospec-continue / autospec-explore
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
   - Claude Code: `~/.claude/skills/autospec-explore/SKILL.md`
   - OpenCode:    `~/.config/opencode/agent/autospec-explore.md`
   - Codex CLI:   `~/.codex/prompts/autospec-explore.md`
2. **Re-install the full autospec suite from `main`** by piping the canonical installer:
   ```bash
   curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh | bash -s -- --skill all --harness all --update
   ```
   Run this one-liner once; it refreshes all autospec skills across all harnesses.
3. **Show the diff** between the prior installed file(s) and the freshly fetched copy.
4. **Stop.** Do not enter the explore pipeline. Print the upgrade summary and return to the user.

If no install path is detected, print `Self-update: no installed copy of autospec-explore found; run install.sh first.` and exit.

## Stop mode

If the feature-request argument matches the regex `^\s*stop(\s+--\w+)*\s*$` (case-insensitive), this skill enters stop mode and does not run the normal pipeline:

1. Delegate to the shared stop helper:
   ```bash
   bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-stop.sh" "$@"
   ```
2. Honor `--graceful` (write `~/.autospec/stop.flag` and `~/.autospec/explore-stop.flag`; running iteration finishes) and `--immediate` (also write `~/.autospec/refine-loop-stop.flag`; abort at next iteration boundary).
3. Print the stop summary and exit. Do not enter the explore pipeline.

## Required capabilities & harness adapter

This workflow assumes six capabilities. Map each one to your harness's actual tool. If a capability is missing, use the listed fallback.

| Capability                  | Claude Code                          | OpenCode                                 | Codex CLI                                | Fallback if missing                                |
|-----------------------------|--------------------------------------|------------------------------------------|------------------------------------------|----------------------------------------------------|
| Read-only codebase research | `Agent` (subagent_type=Explore)      | `task` agent in read-only mode           | `apply_patch` read-only / shell `grep`   | Do the search in-thread with `rg`/`grep`           |
| Foreground delegation       | `Agent` (subagent_type=general-purpose) | nested `task` agent, await output     | spawn nested CLI session                 | Do the work in-thread (more context cost)          |
| Background delegation       | `Agent` with `run_in_background: true` | detached `task` agent                  | nohup'd CLI session writing to a logfile | Run the monitor in a separate terminal/tmux pane   |
| Ask the user a question     | `AskUserQuestion`                    | inline prompt                            | inline prompt                            | Ask in the response and wait for the next turn     |
| Self-paced future wakeup    | `ScheduleWakeup` inside a `/loop`    | a recurring `task` or local `cron`       | local `cron`/`launchd` calling the CLI   | The user runs a status-update prompt manually      |
| Subagent model tier         | Tier A: `opus` + `ultrathink`; Tier B: `sonnet` + medium thinking | Tier A: top `task` model + high reasoning; Tier B: smaller-tier `task` + medium reasoning | Tier A: top GPT + `reasoning_effort=high`; Tier B: `gpt-5.1-codex-spark` + `reasoning_effort=medium` | Honor the per-phase tier mapping in AGENTS.md; retry the same subagent UP on unavailability |
| Subagent dispatch policy   | per AGENTS.md decision matrix        | per AGENTS.md decision matrix            | per AGENTS.md decision matrix            | inline with main-session token cost                |

**Persistent project notes**: write durable preferences to **`AGENTS.md`** in the repo root — recognized by Claude Code, OpenCode, and Codex. Per AGENTS.md, subagent dispatches use the two-tier policy: Tier A for research aggregation and proposal ranking, Tier B for individual deterministic researchers and downstream implementer dispatches (inherited from `/autospec-run`).

## Harness detection (run once at skill start, before sandbox creation)

Detect your harness by checking available tools before any sandbox or research step runs:

1. **Claude Code** — the `Agent` tool with a `subagent_type` parameter is available.
   - `TIER_A` = `opus` + `ultrathink`  (model ID: claude-opus-4-7)
   - `TIER_B` = `sonnet`               (model ID: claude-sonnet-4-6)

2. **OpenCode** — a `task` tool with model/tier configuration is available (no `subagent_type`).
   - `TIER_A` = top-tier task model + high reasoning
   - `TIER_B` = smaller-tier task model + medium reasoning

3. **Codex CLI** — neither `Agent` nor a configurable `task` tool is available; `apply_patch` is the primary edit tool.
   - `TIER_A` = current top GPT model + `reasoning_effort=high`
   - `TIER_B` = `gpt-5.1-codex-spark` + `reasoning_effort=medium`

**Fallback rule:** If `TIER_B` is not available in your harness, silently retry the same subagent dispatch with `TIER_A`. Preserve parent context on retry. Never ask the user.

Hold `TIER_A` and `TIER_B` for the entire skill run. Every "Tier A" and "Tier B" reference below resolves to these harness-specific values.

## Architecture

```
/autospec-explore "<prompt>"
        │
        ▼
   create sandbox branch
   autospec/explore/<date>-<slug> off origin/main
        │
        ▼
   ┌──────────────────────────────────────────┐
   │  perpetual loop (single iteration shown) │
   │                                          │
   │  1. research cycle:                      │
   │     - 6 researchers run in parallel      │
   │     - aggregate proposals, dedup, rank   │
   │  2. file 1-5 auto-implement issues       │
   │     (max per round, configurable)        │
   │  3. drain via /autospec-run              │
   │     - implementer PRs target SANDBOX,    │
   │       not main                           │
   │  4. update .autospec/explore-summary.md  │
   │  5. check termination:                   │
   │     - operator stop flag                 │
   │     - round cap / time cap / token cap   │
   │     - usage-limit supervisor arms        │
   │  6. loop                                  │
   └──────────────────────────────────────────┘
        │
        ▼ (operator decides)
   git merge autospec/explore/<date>-<slug> → main
        │
   OR discard: gh branch -D
```

Skill family layout (mirrors existing autospec-refine / autospec-continue):

- `skills/autospec-explore/SKILL.md` — Claude Code adapter (authoritative).
- `skills/autospec-explore/codex/prompt.md` — Codex CLI mirror (lockstep).
- `skills/autospec-explore/opencode/agent.md` — OpenCode mirror (lockstep).
- `skills/autospec-explore/install.sh`, `uninstall.sh`, `README.md`.
- `scripts/autospec-explore.sh` — orchestrator. **Stubbed by Issue E.**
- `scripts/explore-sandbox.sh` — sandbox branch creation + `.autospec/explore-mode.json`.
- `scripts/explore-research-cycle.sh` — runs all researchers, aggregates. **Stubbed by Issue C.**
- `scripts/explore-research/` (subdir) — one researcher per source. **Stubbed by Issues C+D.**

This SKILL.md is the scaffold contract. Subsequent child issues fill in the
implementer PR-base integration (B), researchers (C+D), the orchestrator + loop
integration (E), and the `check_autospec_explore_contract` gate in `scripts/validate.sh` (E).

## Invocation

```
/autospec-explore "<initial prompt>" \
    [--max-iterations N] \
    [--max-issues-per-round N] \
    [--budget-tokens N] \
    [--budget-hours N] \
    [--sandbox-slug <slug>] \
    [--research-sources <comma-list>] \
    [--no-internet] \
    [--internet-allowlist <comma-list>]
```

> **Model tier:** `TIER_A` for the aggregator + proposal ranker; `TIER_B` for the
> deterministic researchers and downstream implementer dispatches (inherited from
> `/autospec-run`).

- `--max-iterations N` — outer loop round cap. Default unlimited.
- `--max-issues-per-round N` — research output cap. Default 5.
- `--budget-tokens N` — token budget across all iterations. Default 10M.
- `--budget-hours N` — wall-time budget. Default 24h.
- `--sandbox-slug <slug>` — override sandbox branch slug.
- `--research-sources <list>` — limit to a comma-separated subset of the
  6 researcher names. Default: all 6.
- `--no-internet` — disable internet research (the most expensive +
  highest-risk source).
- `--internet-allowlist <list>` — comma-separated domains the internet
  researcher is permitted to fetch. Default: a curated list of
  competitor-research-appropriate domains (GitHub, official product
  docs, HackerNews, etc.). Forbidden by default: paywalled content,
  social media, pastebin-class sites.

## Sandbox branch contract

1. **Worktree assert (MUST exit 0 before any sandbox commit/push)**: the
   orchestrator MUST run `bash scripts/worktree-guard.sh assert` before invoking
   `explore-sandbox.sh` or performing any sandbox commit step. A non-zero exit
   (in_primary_checkout / dirty / stale_base) is NEVER worked around — emit
   the `code_health` identifier from the guard, and stop. The primary checkout
   is read-only for agents; all sandbox work happens in a linked worktree.
   ```bash
   # MANDATORY assert gate: MUST exit 0 before any sandbox commit/push.
   if ! bash scripts/worktree-guard.sh assert; then
     echo "worktree-guard assert failed (see code_health identifier above); aborting sandbox commit" >&2
     exit 1
   fi
   ```
2. **Creation**: at run start, the orchestrator invokes
   `scripts/explore-sandbox.sh --slug <slug> --base main` which creates
   `autospec/explore/<YYYY-MM-DD>-<slug>` off `origin/main` (or the supplied
   `--base`) if not already present, pushes the branch to `origin`, and writes
   `.autospec/explore-mode.json` with `{branch, slug, base, head_sha, created_at}`.
   The branch lives until the operator merges or deletes it.
3. **Idempotency**: re-invocation with the same `--slug` reuses the existing
   sandbox branch — no error, no duplicate, no force-push. The script verifies
   the existing branch tracks the expected base via `git merge-base` and
   refreshes `.autospec/explore-mode.json` with the current head SHA.
4. **Implementer integration**: every child-issue implementer reads
   `.autospec/explore-mode.json` (written by the sandbox script) to learn the
   sandbox branch name. PRs target `--base <sandbox-branch>` instead of
   `main`. This is enforced by extending the Phase 4 implementer prompt
   template (`skills/autospec-run/prompts/phase4-implementer.md`) in Issue B.
5. **No accidental main merges**: orchestrator refuses to invoke
   `gh pr merge` against `main` while `.autospec/explore-mode.json` is
   present. The sandbox → main merge is a separate explicit operator
   action (`/autospec-explore-promote <sandbox-branch>` — out of scope
   for v1; documented as the manual path).
6. **Sandbox refresh policy**: the sandbox branch is NOT auto-rebased onto
   main. Operator does that explicitly. This is intentional — rebasing
   under autonomous shipping is unsafe.

## Research cycle contract

Each round runs the 6 researchers (or the operator-specified subset) in
parallel. Each researcher returns 0-N proposals as JSON:

```json
{
  "source": "spec-vs-code",
  "proposals": [
    {
      "title": "feat: implement <X> from spec docs/specs/<Y>.md",
      "evidence": "Acceptance criterion 3 in <Y>:42 has no implementation",
      "estimated_complexity": "small|medium|large",
      "confidence": 0.85
    }
  ]
}
```

Aggregation:

1. **Deduplication**: by normalized title (lowercased, action verb +
   subject), drop duplicates across researchers.
2. **Ranking**: weighted score = `confidence × source_weight ×
   1/estimated_complexity`. Default source weights:
   - `spec-vs-code` = 1.0 (highest — spec drift is concrete and grounded)
   - `prior-reports` = 0.9 (operator-derived priorities)
   - `codebase-signals` = 0.7
   - `open-issues` = 0.6
   - `source-analysis` = 0.5
   - `internet` = 0.4 (lowest — least grounded)
3. **Filtering**: drop proposals that match recently-filed issue titles
   (last 7 days) to prevent oscillation.
4. **Cap**: top `--max-issues-per-round` proposals become issues.

Each researcher is a separate script and can be enabled/disabled via
`--research-sources`. **Stubbed by Issue C; the JSON contract is the
authoritative interface between researchers and the aggregator.**

## Loop driver integration

The outer loop uses `scripts/lib/autospec-loop.sh` from PR #712 with
explore-specific callbacks (wired in Issue E):

- **per-iteration callback**: `scripts/explore-research-cycle.sh` (file
  issues for top N proposals).
- **drain callback**: invoke `/autospec-run` (which honors sandbox base
  branch via the Issue B integration).
- **termination conditions**: inherited from #712 + new
  `operator_stop` checks `~/.autospec/explore-stop.flag` AND
  `~/.autospec/stop.flag`. No convergence-stop (explore is meant to keep
  generating until operator says enough).

## Usage-limit recovery

Inherits `scripts/autospec-usage-limit.sh` (already wired for autospec-run
per existing skill prose). When the harness reports a deterministic
quota pause, the orchestrator arms the supervisor with the resume command
(the same `/autospec-explore` invocation + the sandbox branch context recovered
from `.autospec/explore-mode.json`) and exits. The supervisor relaunches after
reset.

## Loop summary

`.autospec/explore-summary.md` (markdown, human-readable) +
`.autospec/explore-loop.json` (machine-readable per-iteration log).
Structurally identical to the loop summaries from `/autospec --loop`,
`/autospec-continue`, `/autospec-qa --heal` (all four share the shape from
PR #712).

Markdown shape:

```
## /autospec-explore — sandbox autospec/explore/<date>-<slug>

| Round | Researchers run | Proposals | Issues filed | PRs merged | Time | Status |
|---|---|---|---|---|---|---|
| 1 | 6/6 | 17 (deduped to 12) | 5 | 5 | 28m | round_complete |
| 2 | 6/6 | 14 (deduped to 9) | 4 | 4 | 22m | round_complete |
| 3 | 6/6 | 8 (deduped to 6) | 5 | 3 + 2 in flight | 31m | operator_stop |

Final status: operator_stop after 3 rounds, 14 PRs merged on sandbox.

To merge sandbox into main:
  git checkout main && git merge autospec/explore/2026-05-29-X

To discard:
  git branch -D autospec/explore/2026-05-29-X && \
    git push origin --delete autospec/explore/2026-05-29-X
```

## Error handling

- **Researcher fails** (e.g., gh API error, LLM timeout) → that researcher
  contributes 0 proposals, loop continues with the others. Logged.
- **All researchers fail** → round produces no proposals → loop emits
  `code_health:explore_all_researchers_failed` and pauses for operator.
- **Issue-creation fails** → retry once, then skip that proposal.
- **/autospec-run fails** → record failure, continue loop with reduced
  rate (next round delayed by 5 min) to avoid hammering on a broken
  state.
- **Sandbox branch deleted out from under the loop** → orchestrator
  detects via `git rev-parse --verify` before each iteration. Missing →
  exit with `code_health:explore_sandbox_missing` and operator-recovery
  instructions.
- **Sandbox script idempotency violation** — if `explore-sandbox.sh` is
  re-invoked with the same slug but a divergent base, exit 3 with
  `code_health:explore_sandbox_base_mismatch` and refuse to overwrite.

## Testing

- `tests/explore/test_explore_sandbox.bats` — sandbox creation/management,
  idempotency, no accidental main writes, `.autospec/explore-mode.json`
  schema.
- `tests/explore/test_explore_researchers.bats` — each of the 6
  researchers produces well-formed JSON proposals from fixture inputs.
  **Stubbed by Issues C+D.**
- `tests/explore/test_explore_research_cycle.bats` — aggregation,
  dedup, ranking, capping. **Stubbed by Issue C.**
- `tests/explore/test_explore_loop.bats` — outer loop integration with
  shared driver; termination conditions reachable. **Stubbed by Issue E.**
- `tests/explore/test_explore_internet_safety.bats` — domain allowlist,
  prompt-injection guard, rate limit, content sanitization.
  **Stubbed by Issue D.**

### Primary smoke test

```
bash scripts/validate.sh
bash scripts/explore-sandbox.sh --slug smoke-test
```
