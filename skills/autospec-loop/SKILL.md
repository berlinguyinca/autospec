---
name: autospec-loop
description: Use when the user wants to run a single task repeatedly in a loop until a goal is reached — "loop until the build passes", "keep going until done", "run X in a loop until Y". Interactively refines the request into a frozen contract, then runs a goal-conditioned loop with conservative guardrails. Claims goal/completion loop phrasings; defers bare interval polling ("loop every 5m") to the native /loop.
---

# autospec-loop workflow (harness-neutral)

Take an operator request to "run something in a loop until a goal is reached"
and turn it into (1) an interactive **refine-until-"go"** gate that freezes a
testable contract, then (2) a **goal-conditioned loop** that re-dispatches the
refined prompt with carried state each iteration until a deterministic check —
or, failing that, an independent verifier — confirms the goal is met, with
conservative guardrails so it never spins unbounded.

Goal: a harness-neutral, autospec-family way to "keep doing X until Y" that
stops on the *goal* (not a clock), prefers a deterministic CHECK over LLM
self-assessment, and is safe to run on small-context local models.

Manage your own context — never exceed 60%. Delegate per-iteration work to
subagents whenever your harness supports it (Codex runs iterations inline); do
not carry raw iteration transcripts in the orchestrator context.

## Startup self-update

```bash
#!/usr/bin/env bash
# autospec-startup-self-update — see docs/specs/2026-05-01-autospec-startup-self-update-design.md
set +e
SKILL_NAME=autospec-loop   # per-skill: autospec-define / autospec-run / autospec-listen / autospec-classify / autospec-loop
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

If the argument matches the regex `^\s*update\s*$` (case-insensitive,
whitespace-padded), this skill enters self-update mode and does not run the
normal pipeline. This section is pure prose: never interpolate or shell out the
operator's argument text.

1. **Detect harness** by checking which install path exists for this skill:
   - Claude Code: `~/.claude/skills/autospec-loop/SKILL.md`
   - OpenCode:    `~/.config/opencode/agent/autospec-loop.md`
   - Codex CLI:   `~/.codex/prompts/autospec-loop.md`
2. **Re-install the full autospec suite from `main`** by piping the canonical installer:
   ```bash
   curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh | bash -s -- --skill all --harness all --update
   ```
   Run this one-liner once; it refreshes all autospec skills across all harnesses.
3. **Show the diff** between the prior installed file(s) and the freshly fetched copy.
4. **Stop.** Do not enter the loop pipeline. Print the upgrade summary and return to the user.

If no install path is detected, print `Self-update: no installed copy of autospec-loop found; run install.sh first.` and exit.

## Model tier

Tier A/B model resolution and the harness-detection protocol are inherited
verbatim from `AGENTS.md` (`## Subagent model selection (two-tier, cost-aware)`
and `## Subagent vs inline decision matrix`). Refinement-gate reasoning runs at
Tier A (spec work); per-iteration loop work and the independent verifier run at
Tier B (implementation work), escalating to Tier A for high-stakes goals.

> **Model tier:** Phase 1 refine-gate dispatches at Tier A (spec work); Phase 2
> per-iteration worker and verifier dispatch at TIER_B (implementation work),
> resolved at startup per the harness detection below.

### Required capabilities & harness adapter

This workflow assumes the following capabilities. Map each to your harness's
actual tool; if a capability is missing, use the listed fallback.

| Capability                  | Claude Code                             | OpenCode                              | Codex CLI                                | Fallback if missing                              |
|-----------------------------|-----------------------------------------|---------------------------------------|------------------------------------------|--------------------------------------------------|
| Per-iteration worker        | `Agent` (subagent_type=general-purpose) | `task` agent, await output            | run the iteration inline (no subagents)  | Run the iteration in-thread (more context cost)  |
| Independent verifier        | a **separate** `Agent` dispatch         | a **separate** `task` agent           | a separate inline judging pass           | Judge in-thread, but never reuse the worker pass |
| Ask the user a question     | `AskUserQuestion`                       | inline prompt                         | inline prompt                            | Ask in the response and wait for the next turn   |
| Subagent model tier         | Tier A: `opus` + `ultrathink`; Tier B: `sonnet` + medium thinking | Tier A: top `task` model + high reasoning; Tier B: smaller-tier `task` + medium reasoning | Tier A: top GPT + `reasoning_effort=high`; Tier B: `gpt-5.1-codex-spark` + `reasoning_effort=medium` | Honor the per-phase tier mapping in AGENTS.md; retry the same subagent UP on unavailability |
| Subagent dispatch policy    | per AGENTS.md decision matrix           | per AGENTS.md decision matrix         | per AGENTS.md decision matrix            | inline with main-session token cost              |

**Persistent project notes**: write durable preferences to **`AGENTS.md`** in
the repo root — recognized by Claude Code, OpenCode, and Codex.

## Harness detection (run once at skill start, before the gate begins)

Detect your harness by checking available tools before any dispatch:

1. **Claude Code** — the `Agent` tool with a `subagent_type` parameter is available.
   - `TIER_A` = `opus` + `ultrathink`  (model ID: claude-opus-4-7)
   - `TIER_B` = `sonnet`               (model ID: claude-sonnet-4-6)
2. **OpenCode** — a `task` tool with model/tier configuration is available (no `subagent_type`).
   - `TIER_A` = top-tier task model + high reasoning
   - `TIER_B` = smaller-tier task model + medium reasoning
3. **Codex CLI** — neither `Agent` nor a configurable `task` tool is available; iterations run inline.
   - `TIER_A` = current top GPT model + `reasoning_effort=high`
   - `TIER_B` = `gpt-5.1-codex-spark` + `reasoning_effort=medium`

**Fallback rule:** If `TIER_B` is not available in your harness, silently retry
the same subagent dispatch with `TIER_A`. Preserve parent context on retry.
Never ask the user.

Hold `TIER_A` and `TIER_B` for the entire skill run.

## Trigger disambiguation

`autospec-loop` and the native `/loop` cover different intents; route by
goal-vs-interval:

- **autospec-loop claims** goal/completion phrasings: *"loop until …"*, *"in a
  loop until …"*, *"keep going until done"*, *"do a loop"*, *"execute a loop"*,
  *"run X in a loop until Y"*, *"keep doing X until Y"*.
- **Native `/loop` keeps** bare interval phrasings: *"loop every 5m"*,
  *"/loop 5m /foo"*, *"poll every N minutes"*, *"keep running … on an interval"*.
- **Ambiguous** bare *"do a loop"* / *"execute a loop"* with no interval and no
  explicit goal → autospec-loop claims it, and the refine gate's **first
  question** is goal-or-interval. If the operator wants interval polling, the
  skill **redirects to native `/loop`** and exits without starting its own loop.

> Stub: the full trigger wording lives in the frontmatter `description` and is
> finalized in the trigger-disambiguation child issue.

## Phase 1 — refine-until-go gate

The interactive gate that turns a raw operator request into a frozen,
operator-approved contract before any loop iteration runs. It reuses
`autospec-refine --interactive` for the refinement rounds (do **not** fork the
lens logic — per the ROI-check rule, invoke upstream), extracts the
GOAL / CHECK / MODE contract, shows-and-explains what was understood, and keeps
iterating with the operator until they type the literal unlock token `go`.

The gate is the core requested behaviour, so it is **on by default**; only
`--yes` skips it (autonomous mode).

### 1. Trigger disambiguation handoff

Apply the **Trigger disambiguation** section above. If the request is bare
interval polling intent (*"loop every 5m"*, *"poll every N minutes"*), redirect
to the native `/loop` and **stop** — do not enter this gate. If the request is
ambiguous (*"do a loop"* with no interval and no goal), the gate's **first
clarifying question** is goal-or-interval; on an interval answer, redirect to
native `/loop` and stop. Otherwise (a goal/completion intent) continue.

### 2. Refinement rounds (reuse autospec-refine)

Run the repo-grounded prompt-improvement rounds by invoking
**`autospec-refine --interactive`** on the raw request. This reuses the upstream
lens pipeline (repo-grounding, clarity-ac, sizing, adversarial); do not
reimplement or fork it. Take the refined prompt that `autospec-refine` produces
as the input to contract extraction below.

If the operator passed `--goal '<criterion>'`, treat that as the authoritative
GOAL and skip GOAL inference; still run the refinement rounds to sharpen the
per-iteration prompt unless `--yes` is also set.

### 3. Contract extraction (GOAL / CHECK / MODE)

From the refined prompt, extract three contract artifacts:

- **GOAL** — an explicit, **testable** success criterion (one or more
  sentences). It must be phrased so that "done" is unambiguous. When `--goal`
  was supplied, use it verbatim.
- **CHECK** — a **deterministic verifier**: a shell command plus its expected
  exit status (e.g. a test command, a `grep`, or a `gh` query that exits `0`
  exactly when the goal holds), **when one is derivable from the refined
  prompt**. If no deterministic check can be derived, set CHECK to **`null`** —
  the loop will then rely on a **separate** verifier subagent (never the worker)
  to judge each iteration. If the operator passed `--check '<cmd>'`, use that
  command verbatim and skip CHECK inference; an explicit `--check ''` (empty) or
  the operator declining a check forces CHECK = `null`.
- **MODE** — `cumulative` (carry `progress[]` state between iterations;
  **default**) vs `polling` (stateless re-run of the same refined prompt each
  iteration, judged on fresh repo state). Default to `cumulative` unless the
  refined prompt clearly describes stateless polling.

### 4. Show & explain

Present to the operator, in plain language:

- the **refined prompt verbatim** (exactly what each iteration's worker
  receives);
- **what was understood** — the restated GOAL; the CHECK that will gate
  termination, shown as the literal command + expected exit (or, when CHECK is
  `null`, the sentence *"no deterministic check — an independent verifier
  subagent will judge each iteration against the GOAL"*); the MODE
  (`cumulative` / `polling`) and why; and the active guardrails (`max_iters`,
  `no_progress_K`, `stop.flag`, pre-loop short-circuit);
- the **per-iteration plan** — what one iteration will do (dispatch worker →
  persist outcome → evaluate goal → no-progress check → converge-or-continue).

### 5. Clarify loop (unlock on `go`)

Ask the operator to confirm or correct. If they correct or clarify anything,
**re-run steps 2–4** with the new input and show the updated contract. Repeat
until the operator types the literal unlock token **`go`** (case-insensitive;
also accept `"go ahead"` and `"start"`). Any other response is treated as
further clarification and re-enters the loop — never start the loop on an
ambiguous or partial confirmation.

`--yes` skips this entire gate (steps 4–5): the extracted contract is frozen
immediately and the loop starts without operator confirmation. It is **off by
default** because the gate is the core requested behaviour.

### 6. Freeze the contract

Initialize the loop-state file at
`~/.autospec/loop/<repo-slug>/loop-state.json` via `loop-state.sh init`, writing
the frozen `goal`, `check` (the command string or `null`), and `mode`, plus the
guardrail defaults (`max_iters`, `no_progress_K` — overridable by `--max-iters`
/ `--no-progress`) and the zeroed counters (`iter`, `no_progress_count`,
`progress`, `last_result`, `done`, `exit_reason`). Once frozen, the contract is
authoritative for the run; proceed to Phase 2.

## Phase 2 — goal-conditioned loop

State lives at `~/.autospec/loop/<repo-slug>/loop-state.json` (path-scoped slug
to avoid cross-repo collision; atomic `tmp`+`mv` writes via `loop-state.sh`).

**Pre-loop short-circuit:** if CHECK is non-null, run it once before iterating;
if already satisfied → `exit_reason="already-satisfied"`, report, stop.

**Each iteration** (stop-gates evaluated *before* doing work):

1. **Halt gate** — see Guardrails.
2. **Work** — dispatch a worker subagent (Tier B; inline on Codex) with the
   refined prompt + GOAL + carried `progress[]` (cumulative) or just the prompt
   + `last_result` (polling). Instruct it to make forward progress and report.
3. **Persist** — append the outcome to `progress[]`, set `last_result`, via
   `loop-state.sh`.
4. **Goal eval** — if CHECK non-null, run `loop-check.sh` (deterministic,
   preferred). If CHECK is `null`, dispatch a **separate** verifier subagent —
   **never the worker subagent** (no self-approval) — to judge against GOAL.
5. **No-progress detection** — measurable change this iteration? No →
   `no_progress_count++`; yes → reset to 0.
6. **Converge** — goal met → `done=true`, `exit_reason="goal-met"`, break.
   Else `iter++`, continue.

> Stub: the iteration body and exit report are filled in by the Phase 2
> goal-conditioned loop child issue; `loop-state.sh` / `loop-check.sh` ship in
> the scripts child issue.

## Guardrails

Conservative caps so the loop never spins unbounded (small-model target):

- **stop.flag** — `~/.autospec/stop.flag` exists (shared with `/autospec-stop`)
  → halt with `exit_reason="stopped-by-flag"`.
- **max-iters** — `iter >= max_iters` (default 25, `--max-iters N`) →
  `exit_reason="max-iters"`.
- **no-progress** — `no_progress_count >= no_progress_K` (default 3,
  `--no-progress K`) → `exit_reason="no-progress"`.
- **pre-loop short-circuit** — CHECK already satisfied before iterating →
  `exit_reason="already-satisfied"`, no wasted iteration.
- **no self-approval** — when CHECK is `null`, goal judgement is always done by
  a **separate** verifier subagent, never the worker that produced the result.

The halt gate is evaluated *before* each iteration's work, so a set flag or an
exhausted cap stops the loop without dispatching another worker.
