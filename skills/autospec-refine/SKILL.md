---
name: autospec-refine
description: Use when the user wants to refine a prompt or feature request over N rounds of repo-grounded lenses before handing off to /autospec for implementation. Supports --rounds, --lenses, --autonomous, --interactive, --dry-run, and --continue (continuous iteration mode).
---

# autospec-refine workflow (harness-neutral)

Take an operator-supplied prompt (or feature request) and iteratively refine it
over N rounds, grounding each round in current repo knowledge (AGENTS.md,
recent specs, recent commits, tagged memory). After operator approval, hand off
the final refined prompt to `/autospec --autonomous` (default) or
`/autospec` (interactive) for end-to-end implementation.

Goal: better prompts → better code, fewer mid-implementation course
corrections, fewer rounds of operator feedback. This skill closes the gap
between under-specified raw requests and hand-crafted long prompts by
automating the prompt-improvement loop operators currently run in their heads.

Manage your own context — never exceed 60%. Delegate to subagents whenever your
harness supports it; do not investigate or write the refined prompt directly in
the main conversation when a subagent can do it.

## Startup self-update

```bash
#!/usr/bin/env bash
# autospec-startup-self-update — see docs/specs/2026-05-01-autospec-startup-self-update-design.md
set +e
SKILL_NAME=autospec-refine   # per-skill: autospec-define / autospec-run / autospec-listen / autospec-classify / autospec-refine
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
   - Claude Code: `~/.claude/skills/autospec-refine/SKILL.md`
   - OpenCode:    `~/.config/opencode/agent/autospec-refine.md`
   - Codex CLI:   `~/.codex/prompts/autospec-refine.md`
2. **Re-install the full autospec suite from `main`** by piping the canonical installer:
   ```bash
   curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh | bash -s -- --skill all --harness all --update
   ```
   Run this one-liner once; it refreshes all autospec skills across all harnesses.
3. **Show the diff** between the prior installed file(s) and the freshly fetched copy.
4. **Stop.** Do not enter the refinement pipeline. Print the upgrade summary and return to the user.

If no install path is detected, print `Self-update: no installed copy of autospec-refine found; run install.sh first.` and exit.

## Stop mode

If the feature-request argument matches the regex `^\s*stop(\s+--\w+)*\s*$` (case-insensitive), this skill enters stop mode and does not run the normal pipeline:

1. Delegate to the shared stop helper:
   ```bash
   bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-stop.sh" "$@"
   ```
2. Honor `--graceful` (write `~/.autospec/stop.flag`; running iteration finishes) and `--immediate` (write `~/.autospec/refine-loop-stop.flag` plus stop.flag; abort at next iteration boundary).
3. Print the stop summary and exit. Do not enter the refinement pipeline.

## Required capabilities & harness adapter

This workflow assumes five capabilities. Map each one to your harness's actual tool. If a capability is missing, use the listed fallback.

| Capability                  | Claude Code                          | OpenCode                                 | Codex CLI                                | Fallback if missing                                |
|-----------------------------|--------------------------------------|------------------------------------------|------------------------------------------|----------------------------------------------------|
| Read-only codebase research | `Agent` (subagent_type=Explore)      | `task` agent in read-only mode           | `apply_patch` read-only / shell `grep`   | Do the search in-thread with `rg`/`grep`           |
| Foreground delegation       | `Agent` (subagent_type=general-purpose) | nested `task` agent, await output     | spawn nested CLI session                 | Do the work in-thread (more context cost)          |
| Background delegation       | `Agent` with `run_in_background: true` | detached `task` agent                  | nohup'd CLI session writing to a logfile | Run the monitor in a separate terminal/tmux pane   |
| Ask the user a question     | `AskUserQuestion`                    | inline prompt                            | inline prompt                            | Ask in the response and wait for the next turn     |
| Self-paced future wakeup    | `ScheduleWakeup` inside a `/loop`    | a recurring `task` or local `cron`       | local `cron`/`launchd` calling the CLI   | The user runs a status-update prompt manually      |
| Subagent model tier         | Tier A: `opus` + `ultrathink`; Tier B: `sonnet` + medium thinking | Tier A: top `task` model + high reasoning; Tier B: smaller-tier `task` + medium reasoning | Tier A: top GPT + `reasoning_effort=high`; Tier B: `gpt-5.1-codex-spark` + `reasoning_effort=medium` | Honor the per-phase tier mapping in AGENTS.md; retry the same subagent UP on unavailability |

**Persistent project notes**: write durable preferences to **`AGENTS.md`** in the repo root — recognized by Claude Code, OpenCode, and Codex. Per AGENTS.md, subagent dispatches use the two-tier policy: Tier A for spec/refinement work (each lens applies fresh-eyes reasoning to the prompt), Tier B for implementation. This skill is refinement-only and dispatches at Tier A. Fall back UP the tier on quota/capacity failure.

## Harness detection (run once at skill start, before refinement begins)

Detect your harness by checking available tools before any lens runs:

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

`autospec-refine` is a new top-level skill that mirrors the existing
`autospec-define` and `autospec-qa` skill-family shape:

- `skills/autospec-refine/SKILL.md` — Claude Code adapter (authoritative).
- `skills/autospec-refine/codex/prompt.md` — Codex CLI mirror (lockstep).
- `skills/autospec-refine/opencode/agent.md` — OpenCode mirror (lockstep).
- `skills/autospec-refine/install.sh` — installer; mirrors existing skills.
- `skills/autospec-refine/uninstall.sh` — uninstaller; mirrors existing skills.
- `skills/autospec-refine/README.md` — usage doc.
- `scripts/refine-prompt.sh` — orchestrator: parses args, dispatches lenses,
  writes artifacts, performs handoff. **Stubbed by this scaffold; implemented
  in the orchestrator child issue.**
- `scripts/refine-render-overview.sh` — produces dual markdown + JSON artifact.
  **Stubbed by this scaffold; implemented in the renderer child issue.**

This SKILL.md is the scaffold contract. Subsequent child issues fill in
`scripts/refine-prompt.sh`, the lens helpers, the renderer, the handoff
plumbing, and the continuous-iteration loop.

## Invocation

```
/autospec-refine "<initial prompt>" [--rounds N] [--from-file <path>] \
                 [--interactive | --autonomous (default) | --dry-run] \
                 [--lenses <comma-list>] [--output <path>] \
                 [--continue [--max-iterations N]]
```

> **Model tier:** `TIER_A` (spec work) — each refinement round dispatches a fresh-eyes lens subagent at top model with extended thinking; resolved at startup.

- `--rounds N` — number of refinement passes. Default `3`. Max `10`. Rounds
  apply the named lenses in order; if `N` < number of lenses, the trailing
  lenses are skipped. If `N` > number of lenses, the adversarial lens repeats.
- `--from-file <path>` — read the initial prompt from a file instead of
  inline string. Mutually exclusive with positional prompt.
- `--autonomous` (default) — on operator approval, hand off to
  `/autospec --autonomous "<refined>"` (no-confirmation autospec).
- `--interactive` — hand off to plain `/autospec "<refined>"` (full Phase 2
  brainstorm gate).
- `--dry-run` — produce the artifact only, do not hand off.
- `--lenses repo,clarity,sizing,adversarial` — override the default lens
  order. Comma-separated, drawn from the registered lens names.
- `--output <path>` — write the final refined prompt to a file (in addition
  to the `.autospec/refinements/` artifact).
- `--continue` — enter continuous-iteration mode: refine → handoff → execute
  → harvest-report → re-refine. See **Continuous-iteration mode** below.

## Refinement lenses

Four lenses, applied in order one per round:

1. **`repo-grounding`** — read `AGENTS.md`, recent `docs/specs/**` (last 30
   days), `git log --since=7d`, and tag-matched
   `~/.autospec/projects/*/memory/feedback_*.md`. Replace generic verbs with
   project-specific file paths, conventions, and constraints (lockstep, TDD,
   no-mock, conventional commits, sizing caps).
2. **`clarity-ac`** — extract implicit acceptance criteria and convert prose
   into `- [ ]` checkbox list. Disambiguate hedging language (`should
   probably`, `might`, `could try`). Name unambiguous test commands.
3. **`sizing`** — enforce small-LLM execution caps from the autospec
   decomposer rule (body ≤400 words, ≤3 files per child issue, ≤30 lines of
   implementation outline). If the refined prompt would produce a child that
   exceeds caps, split into a parent + child sequence and emit linked prompt
   fragments.
4. **`adversarial`** — critical-question pass. Apply the 20-question
   checklist from `autospec-qa`. For each high-risk answer that's actionable
   inside the scope, add a test requirement or scope clarification.

Operators may override the default set with `--lenses`. Custom lens registry
is out of scope for v1.

## Convergence and degradation

- **Convergence** — if round N's refined prompt is byte-identical to round
  N-1's, exit early with status `converged` and final round = N-1.
- **Degradation** — if round N's prompt is shorter than round N-1's by more
  than 25% by word count, flag `degraded:<lens>` and surface to operator.
  This catches lens implementations that over-aggressively prune scope.
- **Round cap** — hard limit `AUTOSPEC_REFINE_MAX_ROUNDS=10`. Beyond this,
  exit with `round_cap_reached`.

## Repo context scope

| Source | Read pattern |
|---|---|
| `AGENTS.md` | Always read in full. Highest-signal source. |
| `docs/specs/**/*.md` | Last 30 days by file mtime + commit date. Cap at 5 specs to avoid prompt bloat. |
| `git log --since=7d --oneline` + per-commit `git show --stat` | Last 7 days. Use to surface recently-changed files the prompt might intersect. |
| `~/.autospec/projects/*/memory/feedback_*.md` | Keyword match against the prompt. Cap at 5 most-relevant entries. |

The orchestrator MUST NOT read `.env`, `.git/`, files under `node_modules/`,
or any path matching `*credential*`, `*secret*`, `*.pem`, `*.key`. Path
allowlist is enforced in `refine-prompt.sh`.

## Data model

`.autospec/refinements/<slug>-<ISO-timestamp>.json`:

```json
{
  "original_prompt": "...",
  "rounds": [
    {
      "round_number": 1,
      "lens": "repo-grounding",
      "sources_used": ["AGENTS.md", "docs/specs/2026-05-28-foo.md"],
      "refined_prompt": "...",
      "diff_summary": "added paths/conventions/lockstep refs",
      "word_count_delta": 87,
      "reasoning": "..."
    }
  ],
  "final_prompt": "...",
  "status": "approved|converged|degraded|round_cap_reached|aborted",
  "metadata": {
    "head_sha": "...",
    "timestamp": "...",
    "rounds_requested": 3,
    "rounds_executed": 3,
    "converged_early": false,
    "degraded_rounds": [],
    "handoff_target": "/autospec --autonomous",
    "handoff_executed": true
  }
}
```

`.autospec/refinements/<slug>-<ISO-timestamp>.md` — human-readable: original
prompt → per-round headings with diff blocks → final prompt → handoff record.

A JSON schema lives at `schemas/autospec-refinement.schema.json` and is
validated by `scripts/validate.sh`.

## Error handling

- Empty prompt → usage error, exit 2.
- LLM call fails (rate limit, auth, network) → retry once with backoff, then
  surface to operator and write partial artifact.
- Repo context insufficient (no `AGENTS.md`, no specs) → log warning, use
  generic refinements, mark `metadata.context_sparse: true`.
- Forbidden path access attempt (the allowlist enforced) → fail loudly with
  `code_health:refine_path_violation`; do not silently strip.
- Handoff target unavailable (e.g. `/autospec --autonomous` missing) → write
  artifact, print the refined prompt to stdout, exit with clear message.

## Continuous-iteration mode (`--continue`)

`/autospec-refine --continue "<initial prompt>" [--max-iterations N]` runs the
refine → handoff → execute → harvest-report cycle in a loop. After each
`/autospec` run completes, the orchestrator reads the run's final report
(from `.autospec/run-summary.md` or the equivalent), extracts the "next
steps" / "blockers" / "remaining work" section, and uses THAT content as the
input prompt for the next refinement round.

### Termination (any one)

- **Convergence — no next-steps content.** The harvested report contains no
  actionable "next steps" section, OR the section is empty / says "done".
- **Oscillation.** Iteration N+1's harvested prompt equals iteration N's
  (hashed by content). Loop exits with `oscillation_detected`.
- **Round cap.** `AUTOSPEC_REFINE_LOOP_MAX_ITERATIONS` (default 5).
- **Budget cap.** Tokens > `AUTOSPEC_REFINE_LOOP_TOKEN_CAP` (default 2M) or
  wall time > `AUTOSPEC_REFINE_LOOP_TIME_CAP` (default 6h).
- **Evidence-based stop.** The harvested report contains an explicit
  `STOP: <reason>` marker.
- **Operator escape.** `~/.autospec/stop.flag` (graceful) or
  `~/.autospec/refine-loop-stop.flag` terminates at the next iteration
  boundary.

### Per-iteration record

Each loop iteration appends to `.autospec/refinements/<slug>-loop.json`:

```json
{
  "iteration": 2,
  "harvested_from_report": ".autospec/runs/2026-05-28T14:00Z/report.md",
  "harvested_prompt": "Implement a deterministic ChemOnt-aligned ontology classifier...",
  "refinement_artifact": ".autospec/refinements/<slug>-iter2-<ts>.json",
  "handoff_pr_count": 3,
  "handoff_pr_numbers": [672, 673, 674],
  "stop_reason": null
}
```

### Report harvest contract

The orchestrator reads the LAST autospec run's report and looks for, in order:

1. A section header matching `## Next steps`, `## What to do next`,
   `## Remaining work`, or `## Open blockers` (case-insensitive).
2. If absent, fenced code blocks tagged with ` ```autospec-next` or
   ` ```next-prompt`.
3. If absent, the report's `Out-of-sample`, `Stop condition`, or
   `Evidence-backed stop` sections are inspected. If they say "stop", the
   loop terminates with `evidence_based_stop`.
4. If none of the above are present → convergence (loop exits cleanly).

### Safety guardrails

- The `--autonomous` mode safety guardrails (`scripts/autospec-autonomy-gate.sh`)
  still apply on every loop iteration.
- The continuous loop inherits the autospec autonomy scope rules; it does
  NOT escalate privileges.
- Every iteration's per-PR merge still goes through the rebase-and-retest
  gate.

## Testing

- `tests/refine/test_refine_orchestrator.bats`:
  - happy path: 3-round refinement with all 4 lenses → artifact written, JSON
    schema-valid, rounds populated.
  - convergence: round 2 == round 1 → early exit, status `converged`.
  - degradation: synthetic round that drops 30% of words → flag emitted.
  - round cap: `--rounds 15` → capped at 10, status `round_cap_reached`.
  - context sparse: no `AGENTS.md` + no specs → completes with warning.
  - forbidden path: lens tries to read `.env` → exits with
    `code_health:refine_path_violation`.
- `tests/refine/test_refine_lenses.bats`: per-lens isolation tests, one
  fixture each (4 tests).
- `tests/refine/test_refine_overview.bats`: dual-artifact renderer produces
  valid markdown + JSON.
- `tests/refine/test_refine_handoff.bats`: `--autonomous`, `--interactive`,
  `--dry-run` each route correctly (mocked `gh` / `claude` invocations).
- `tests/refine/test_refine_loop.bats`: continuous-iteration mode termination
  paths.

## Handoff

After all rounds complete (or convergence/degradation/cap exit), present the
final refined prompt to the operator with the per-round diff summary. On
approval, hand off according to the invocation flag:

- `--autonomous` (default) → `/autospec --autonomous "<refined>"`.
- `--interactive` → `/autospec "<refined>"`.
- `--dry-run` → print artifact paths, stop.

For `--continue` mode, the handoff runs autonomously every iteration; the
loop summary table is printed at loop end.
