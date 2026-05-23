---
description: Use when the user has already run /autospec-define (or otherwise has a populated set of auto-implement GitHub issues) and wants the implementation half — Phases 4-6 — to run autonomously with admin auto-merge. Supports --profile <name> filtering against ~/.autospec/model-profiles.yml.
mode: primary
---

# autospec-run workflow (harness-neutral)

Take the populated `auto-implement` queue on the current GitHub repo and run the implementation half of the autospec pipeline:
**autonomous monitor → admin-squash-merge each PR → periodic status updates → final report.**

Manage your own context — never exceed 60%. Delegate to subagents whenever your harness supports it; do not investigate, write code, or design directly in the main conversation when a subagent can do it.

Autospec-run is an autonomous loop and should not ask operator questions for normal operations. Only surface a question if a hard blocker requires explicit manual recovery.

## Startup self-update

```bash
#!/usr/bin/env bash
# autospec-startup-self-update — see docs/specs/2026-05-01-autospec-startup-self-update-design.md
set +e
SKILL_NAME=autospec-run   # per-skill: autospec-define / autospec-run / autospec-listen / autospec-classify
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
# Auto-init cross-tool memory (idempotent, <50ms fast-path)
bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/auto-init-memory.sh"
echo "[autospec] updated ${LOCAL:-fresh} → $REMOTE"
```

## Self-update mode

Decide this purely from the request text the harness handed you. Do NOT
shell out (no `grep`, `sed`, `[[ =~ ]]`, command substitution, etc.) to
test the user's free-form request — passing it through a shell is what
historically tripped harness permission engines (e.g. parse errors near
backtick/pipe characters in the user's prose). Read the request, normalize
it in your reasoning (collapse whitespace, trim, lowercase), and if the result is
exactly `update`, this skill enters self-update mode and does NOT run the
normal pipeline.

1. **Detect harness** by checking which install path exists for this skill:
   - Claude Code: `~/.claude/skills/autospec-run/SKILL.md`
   - OpenCode:    `~/.config/opencode/agent/autospec-run.md`
   - Codex CLI:   `~/.codex/prompts/autospec-run.md`
2. **Re-install from `main`** by piping the canonical installer:
   ```bash
   bash <(curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-run/install.sh) --harness <detected> --update
   ```
   If multiple harness paths exist, run the one-liner once per detected harness.
3. **Show the diff** between the prior installed file(s) and the freshly fetched copy (e.g. `diff <(cat <prior>) <(curl -fsSL ...SKILL.md)` or the equivalent recorded by the installer).
4. **Stop.** Do not enter Phase 0 / Phase 1 / any pipeline phase. Print the upgrade summary and return to the user.

If no install path is detected, print `Self-update: no installed copy of autospec-run found; run install.sh first.` and exit.

## Stop mode

Apply the same read-and-normalize approach used for self-update mode (do
NOT shell out the user's request). If the normalized request is exactly
`stop`, or `stop` followed by one or more `--<word>` flags (examples:
`stop`, `stop --graceful`, `stop --immediate`, `stop --status`,
`stop --resume`, `stop --help`, `stop --flag`), this skill enters stop
mode and does NOT run the normal pipeline. When dispatching, pass any
`--<flag>` tokens the user provided as separate words to the helper.

1. Dispatch to `bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-stop.sh" <args>`.
2. Print the helper's stdout to the user.
3. Stop. Do not enter Phase 0 or any pipeline phase.

## Invocation

```
/autospec-run [--profile <name>]
```

- `--profile <name>` — filter the candidate queue against `~/.autospec/model-profiles.yml` so only issues whose `ctx:*` and `reasoning:*` labels fit the named profile are picked up. Issues that exceed the profile on either axis are appended to a `deferred[]` list and printed in the run-end summary.
- (no flag) — load `~/.autospec/model-profiles.yml`'s `default:` profile and run with it. If the file is missing, run auto-init (below) then exit so the user can review/edit before re-running.
- `--profile <unknown>` — exit non-zero and print the list of available profile names from `~/.autospec/model-profiles.yml`.

### Auto-init `~/.autospec/model-profiles.yml`

If the file is missing on run start:

1. Probe local Ollama availability directly:
   - Detect the binary robustly:
     - macOS/Linux: `command -v ollama`
     - Windows: `where.exe ollama`
   - If present, run `ollama list 2>/dev/null` once and parse returned model rows
     (ignore header/blank lines, capture first column only).
   - For each discovered model, write a local profile using conservative defaults:
     `ctx: 64k`, `reasoning: medium`.
   - Normalize the profile key by lowercasing and replacing each of `:`, `/`, `.`,
     and whitespace with `-`, then append `-laptop` (e.g. `qwen3:32b` →
     `qwen3-32b-laptop`, `library/llama3:latest` → `library-llama3-latest-laptop`).
   - If `ollama list` exits non-zero (e.g. daemon not running) or returns zero
     usable model rows, treat as no local models (do not force a false-positive
     local profile).
2. If `ANTHROPIC_API_KEY` is set in the environment, append two cloud profiles:
   `claude-sonnet-cloud` and `claude-opus-cloud`, both `ctx: 120k`,
   `reasoning: deep`.
3. If neither Ollama nor `ANTHROPIC_API_KEY` is detected, write a single
   `claude-sonnet-cloud` default with `ctx: 120k, reasoning: deep` and an
   `# edit-and-rerun` comment near the top of the file.
4. Set the top-level `default:` key to whichever profile makes sense (prefer
   the largest local profile if any, otherwise `claude-sonnet-cloud`).
5. Print:
   ```
   Wrote ~/.autospec/model-profiles.yml. Edit `default:` and profile ceilings,
   then re-run /autospec-run [--profile <name>].
   ```
   Exit 0; do not enter Phase 4.

Sample auto-init output:

```yaml
# ~/.autospec/model-profiles.yml — autospec-run profile ceilings.
# Edit `default:` and individual ceilings, then re-run.
default: claude-sonnet-cloud
profiles:
  qwen3-32b-laptop:
    ctx: 64k         # one of: 32k | 64k | 120k
    reasoning: medium  # one of: shallow | medium | deep
  claude-sonnet-cloud:
    ctx: 120k
    reasoning: deep
```

### Profile-filter ordinals

- `ctx: 32k < 64k < 120k`
- `reasoning: shallow < medium < deep`

A profile P "fits" issue I when `I.ctx_label ≤ P.ctx` AND `I.reasoning_label ≤ P.reasoning` on these ordinals.


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

**Persistent project notes**: write durable preferences to **`AGENTS.md`** in the repo root — this is the de-facto standard recognized by Claude Code (also reads `CLAUDE.md`), OpenCode, and Codex. If your harness has its own private memory (e.g. Claude Code's `~/.claude/.../memory/`), mirror the same content there. Per AGENTS.md, subagent dispatches use a **two-tier policy**: Tier A (top model + extended thinking) for spec work (research, decompose, review/label — not used by this skill); Tier B (cheaper model + medium thinking) for implementation work (Phase 4 implementer + LGTM review). The orchestrator keeps the user's invoked model. Fall back UP the tier on quota/capacity or other unavailability by retrying the same subagent with the stronger tier while preserving parent context.

## Harness detection (run once at skill start, before Phase 0)

Detect your harness by checking available tools before any phase:

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

## Phase 4 — Background autonomous monitor

Record this durable preference in `AGENTS.md` (idempotent — skip if already present):

> **Auto-merge authority for auto-implement PRs.** Admin-merge auto-implement PRs (`gh pr merge <#> --admin --squash --delete-branch`) when (a) all required CI checks pass — slow optional checks like TeamCity may be pending and that's acceptable, (b) the self-review subagent returned `LGTM`, (c) PR closes an `auto-implement` issue from a `feat/*` branch.

**Off-peak tip:** For queues of 10+ issues (8+ hour runs), consider launching at night or on weekends. Usage limits are shared across all sessions — running long batches off-peak preserves daytime tokens for interactive work.

Then launch a **background monitor loop** — the orchestrator relaunches the monitor with fresh context after each batch of `AUTOSPEC_BATCH_SIZE` issues (default: 3). The monitor is stateless: all persistent state lives in GitHub labels and heartbeat files, so relaunches are always safe.

```
batch_num=1
while true:
  launch background subagent (pass batch_num; AUTOSPEC_BATCH_SIZE=${AUTOSPEC_BATCH_SIZE:-3})
  wait for task-notification (monitor agent completes)

  # Read and consume the batch-done signal.
  if [ -f "$HOME/.autospec/batch-done.json" ]; then
    status=$(jq -r .status "$HOME/.autospec/batch-done.json" 2>/dev/null || echo "BATCH_COMPLETE")
    rm -f "$HOME/.autospec/batch-done.json"
  else
    status="BATCH_COMPLETE"   # monitor crashed / overflowed — safe to relaunch
  fi

  if [ "$status" = "ALL_DONE" ]; then
    break   # proceed to Phase 6 final report
  fi

  batch_num=$((batch_num + 1))
  echo "[orchestrator] batch $((batch_num - 1)) complete — relaunching monitor with fresh context (batch ${batch_num})"
  # continue immediately, no sleep
```

Pass the following prompt verbatim to each background subagent:

> You are the auto-implement monitor for `{repo}`. Process `auto-implement` issues one at a time. Exit after processing `AUTOSPEC_BATCH_SIZE` issues (default: 3) by writing `~/.autospec/batch-done.json` — the orchestrator will relaunch you with fresh context.

> **Harness adaptation (loop persistence).** The `while true:` below is pseudocode. In Claude Code, use `/loop` or `ScheduleWakeup` to persist across turns. In Codex CLI and OpenCode, you lack a built-in loop primitive — implement persistence via one of these patterns:
> 1. **Shell wrapper (preferred):** `exec bash << 'EOF'
> while true; do
>   # ... monitor logic ...
> done
> EOF` — keeps a single bash process alive with your agent dispatching subcommands inside it.
> 2. **nohup background process:** `nohup bash -c 'while true; do ...; sleep 1; done' > ~/.autospec/monitor.log 2>&1 &`
> 3. **tmux pane:** `tmux new-window 'bash << '''HEREDOC'''
> while true; do ...; done
> HEREDOC'`
> **Session batching:** Exit after processing `AUTOSPEC_BATCH_SIZE` issues (default 3) by writing `~/.autospec/batch-done.json` with `status=BATCH_COMPLETE`. The orchestrator relaunches you with fresh context. When the queue is fully drained, write `status=ALL_DONE` instead. This keeps each monitor session short to prevent context overflow.

>
> **Profile load (run-start, once).** If `--profile <name>` was passed, look it up in `~/.autospec/model-profiles.yml`; if `<name>` is not a key under `profiles:`, exit non-zero and print the available names. If no flag was passed, load the file's `default:` profile. If the file is missing, run auto-init and exit (per the Invocation section).
>
> **Missing-label warning (run-start, once).** Count open `auto-implement` issues that lack either a `ctx:*` or a `reasoning:*` label. If non-zero, print `WARN: N issues lack model-fit labels (ctx:* / reasoning:*); they will be treated as ctx:64k, reasoning:medium. Run /autospec-classify to backfill.` Exactly once at run start.
>
> **Shared helper scripts.** Helper scripts live at `${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}` after installation. Do not assume the target repository has an autospec `scripts/` directory.
>
> Outer loop:
> ```
> # Before the loop (run-once init):
> #   batch_issue_count=0
> #   BATCH_SIZE="${AUTOSPEC_BATCH_SIZE:-3}"
> #   [ "$BATCH_SIZE" -gt 0 ] 2>/dev/null || BATCH_SIZE=3   # guard against 0 or negative
> #   rm -f "$HOME/.autospec/batch-done.json"   # clear stale file from prior crash
>
> while true:
>   deferred = []   # issues skipped because they exceed the active profile
>
>   # Startup/per-scan heartbeat reconciliation — run before candidate selection.
>   # This deletes closed/merged/orphaned heartbeats, rejects old schemas like
>   # {"issue":407,"status":"in_progress"}, normalizes current schemas, and
>   # releases any `claimed` heartbeat older than AUTOSPEC_WATCHDOG_CLAIMED_TIMEOUT_SECS (default: 300).
>   if [ -d "$HOME/.autospec/process-heartbeats" ]; then
>     if command -v bash >/dev/null 2>&1; then
>       bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-watchdog.sh"
>     elif command -v pwsh >/dev/null 2>&1; then
>       pwsh -NoProfile -ExecutionPolicy Bypass -File "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-watchdog.ps1"
>     elif command -v powershell >/dev/null 2>&1; then
>       powershell -NoProfile -ExecutionPolicy Bypass -File "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-watchdog.ps1"
>     else
>       echo "[watchdog] neither bash nor powershell found; skipping heartbeat reconciliation."
>     fi
>   fi

### Queue priority sort (autospec-review interlock)

When selecting the next `auto-implement` issue, sort:

1. First: issues with label `priority:high` (e.g. `[REGRESSION]`
   issues filed by autospec-review). Within `priority:high`, oldest
   first.
2. Then: all other `auto-implement` issues, oldest first.

`priority:high` always wins over age. This guarantees regression
issues unblock the queue before continuing with normal feature work.

>   all_open = [open auto-implement issues, sorted ascending by issue number]
>   candidates = [all_open issues whose Depends-on deps are all CLOSED, sorted ascending]
>   blocked = [all_open issues with unmet Depends-on deps]
>
>   ready = []
>   for I in candidates:
>     ctx_lbl       = I.ctx_label or "ctx:64k"           # default if unlabeled
>     reasoning_lbl = I.reasoning_label or "reasoning:medium"
>     if ctx_lbl <= profile.ctx AND reasoning_lbl <= profile.reasoning:  # ordinal compare
>       ready.append(I)
>     else:
>       reason = []
>       if ctx_lbl > profile.ctx:             reason.append(f"{ctx_lbl} > profile.ctx={profile.ctx}")
>       if reasoning_lbl > profile.reasoning: reason.append(f"{reasoning_lbl} > profile.reasoning={profile.reasoning}")
>       deferred.append({"issue": I.number, "reason": ", ".join(reason)})
>
>   claimed_issue, claimed_step = [newest valid heartbeat issue/step, or "-" / "-"]
>   print "[monitor] queue scan: open=N ready=N blocked=N deferred=N claimed=#X step=Y order=ascending(oldest-first)"
>   # GitHub may display newer/high-numbered issues first; autospec intentionally processes ready issues ascending.
>
>   if ready is empty:
>     latest_close = most recent closedAt of any auto-implement issue
>     open_count   = count of open auto-implement issues
>     if open_count == 0 AND latest_close > 2h ago:
>       printf '{"batch":%s,"processed":%s,"repo":"%s","ts":%s,"status":"ALL_DONE"}\n' \
>         "${batch_num:-1}" "$batch_issue_count" "{repo}" "$(date -u +%s)" \
>         > "$HOME/.autospec/batch-done.json"
>       echo "[monitor] all issues processed — writing ALL_DONE and exiting"
>       HARD SHUTDOWN — emit final report (incl. deferred summary, see Phase 6)
>     else: print state ("blocked: N unmet deps" / "deferred: M off-profile" / "drained, waiting 2h idle"), sleep 300, continue
>   # autospec-stop sentinel check — outer loop, top of each iteration
>   if [ -f "$HOME/.autospec/stop.flag" ]; then
>     MODE=$(head -1 "$HOME/.autospec/stop.flag" 2>/dev/null || echo "")
>     TIMESTAMP=$(sed -n '2p' "$HOME/.autospec/stop.flag" 2>/dev/null | awk '{print $1}')
>     AGE_SECS=$(( $(date -u +%s) - $(date -u -j -f '%Y-%m-%dT%H:%M:%SZ' "$TIMESTAMP" +%s 2>/dev/null \
>       || date -u -d "$TIMESTAMP" +%s 2>/dev/null || echo 0) ))
>     if [ "$AGE_SECS" -gt 86400 ]; then
>       echo "WARN: stale stop.flag ($AGE_SECS s old); ignoring" >&2
>     elif [ "$MODE" = "graceful" ] || [ "$MODE" = "immediate" ]; then
>       printf '{"batch":%s,"processed":%s,"repo":"%s","ts":%s,"status":"ALL_DONE"}\n' \
>         "${batch_num:-1}" "$batch_issue_count" "{repo}" "$(date -u +%s)" \
>         > "$HOME/.autospec/batch-done.json"
>       echo "[monitor] stop signal received: $MODE — exiting"
>       # HARD SHUTDOWN with final report
>       exit 0
>     fi
>   fi
>   # Service watch: heartbeat reconciliation already runs before each candidate scan; every 12 iterations also runs a cheap nudge/reclaim pass for long-lived workers.
>   monitor_tick=$((monitor_tick + 1))
>   if [ "$monitor_tick" -ge 12 ]; then
>     monitor_tick=0
>     if [ -d "$HOME/.autospec/process-heartbeats" ]; then
>       # Default reclaim window: 10800s (3h). For local single-threaded workers set
>       # AUTOSPEC_WATCHDOG_RECLAIM_SECS=43200 (12h) before launch.
>       export AUTOSPEC_WATCHDOG_RECLAIM_SECS="${AUTOSPEC_WATCHDOG_RECLAIM_SECS:-10800}"
>       export AUTOSPEC_WATCHDOG_STALE_SECS="${AUTOSPEC_WATCHDOG_STALE_SECS:-1800}"
>       # Cheap service wake-up pass: use low-cost model only.
>       if command -v bash >/dev/null 2>&1; then
>         # Dispatch one background watchdog helper (cheap model) to iterate stale entries.
>         bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-watchdog.sh"
>       elif command -v pwsh >/dev/null 2>&1; then
>         # Windows fallback: PowerShell helper.
>         pwsh -NoProfile -ExecutionPolicy Bypass -File "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-watchdog.ps1"
>       elif command -v powershell >/dev/null 2>&1; then
>         # Windows fallback: classic PowerShell fallback.
>         powershell -NoProfile -ExecutionPolicy Bypass -File "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-watchdog.ps1"
>       else
>         echo "[watchdog] neither bash nor powershell found; skipping service-watch pass."
>       fi
>     fi
>   fi
>   # effective_batch_size probe — recomputed each outer-loop tick (not cached).
>   # Force batch=1 when the next ready issue is reasoning:deep (high blast-radius).
>   _next_reasoning=$(gh issue view "${ready[0]}" --json labels \
>     --jq '[.labels[].name | select(startswith("reasoning:"))] | first // "reasoning:medium"' \
>     2>/dev/null || echo "reasoning:medium")
>   if [ "$_next_reasoning" = "reasoning:deep" ]; then
>     effective_batch_size=1
>   else
>     effective_batch_size="${AUTOSPEC_BATCH_SIZE:-3}"
>     [ "$effective_batch_size" -gt 0 ] 2>/dev/null || effective_batch_size=3
>   fi
>   echo "[monitor] effective_batch_size=$effective_batch_size (next issue reasoning: $_next_reasoning)"
>   ISSUE = ready[0]
>   # Claim check: verify the issue is still labeled auto-implement before processing.
>   # Multiple monitors can query the same candidate list simultaneously;
>   # the first to claim wins, others must skip to the next candidate.
>   CURRENT_LABELS=$(gh issue view ISSUE --json labels --jq -r '.labels[].name')
>   if ! echo "$CURRENT_LABELS" | grep -q "^auto-implement$"; then
>     echo "[monitor] ISSUE $ISSUE already claimed (no auto-implement label); skipping"
>     READY_REMOVE=$(printf "%s\n%s" "$READY_REMOVE" "$ISSUE" | grep -v "^${ISSUE}$" || true)
>     ready=($READY_REMOVE)
>     continue
>   fi
>   gh label create in-progress-by-bot --color ededed --force
>   if ! gh issue edit ISSUE --remove-label auto-implement --add-label in-progress-by-bot; then
>     echo "[monitor] ISSUE $ISSUE claim failed (another monitor claimed it); skipping"
>     continue
>   fi
>   _hb_slug="$(printf '%s' "{repo}" | tr '/' '_')"
>   mkdir -p "$HOME/.autospec/process-heartbeats/$_hb_slug"
>   printf '{"issue":"%s","branch":"","step":"claimed","ts":%s,"pr":"","repo":"%s"}\n' "$ISSUE" "$(date -u +%s)" "{repo}" > "$HOME/.autospec/process-heartbeats/$_hb_slug/$ISSUE.json"
>   # Issue start summary — print before dispatching process(ISSUE) so the operator
>   # knows exactly what the monitor is about to work on.
>   ISSUE_TITLE=$(gh issue view ISSUE --json title --jq .title 2>/dev/null || echo "")
>   ISSUE_URL=$(gh issue view ISSUE --json url --jq .url 2>/dev/null || echo "")
>   ISSUE_LABELS=$(gh issue view ISSUE --json labels --jq -r '[.labels[].name] | join(", ")' 2>/dev/null || echo "")
>   ISSUE_BODY=$(gh issue view ISSUE --json body --jq .body 2>/dev/null || echo "")
>   ISSUE_GOAL=$(printf '%s\n' "$ISSUE_BODY" | awk '
>     BEGIN{in_goal=0}
>     /^## Goal[[:space:]]*$/ {in_goal=1; next}
>     /^## / && in_goal {exit}
>     in_goal && NF {print; exit}
>   ')
>   [ -n "$ISSUE_GOAL" ] || ISSUE_GOAL=$(printf '%s\n' "$ISSUE_BODY" | awk 'NF && $0 !~ /^#/ {print; exit}')
>   ISSUE_SMOKE=$(printf '%s\n' "$ISSUE_BODY" | awk '
>     /### Primary smoke test/ {seen=1; next}
>     seen && /^```/ {fence++; next}
>     seen && fence==1 && NF && $0 !~ /^[[:space:]]*#/ {print; exit}
>   ')
>   ISSUE_SCOPE=$(printf '%s\n' "$ISSUE_BODY" | awk '
>     /^## Implementation outline[[:space:]]*$/ {in_scope=1; next}
>     /^## / && in_scope {exit}
>     in_scope && /^- / {gsub(/^- /,""); print; count++; if (count>=3) exit}
>   ' | paste -sd '; ' -)
>   echo "[monitor] starting #$ISSUE: ${ISSUE_TITLE:-<untitled>}"
>   echo "[monitor] url: ${ISSUE_URL:-<unknown>}"
>   echo "[monitor] labels: ${ISSUE_LABELS:-<none>}"
>   echo "[monitor] goal: ${ISSUE_GOAL:-<not provided>}"
>   echo "[monitor] smoke: ${ISSUE_SMOKE:-<not provided>}"
>   echo "[monitor] scope: ${ISSUE_SCOPE:-<not provided>}"
>   process(ISSUE)   # foreground subagent — see template below
>   batch_issue_count=$((batch_issue_count + 1))
>   if [ "$batch_issue_count" -ge "${effective_batch_size:-$BATCH_SIZE}" ]; then
>     printf '{"batch":%s,"processed":%s,"repo":"%s","ts":%s,"status":"BATCH_COMPLETE"}\n' \
>       "${batch_num:-1}" "$batch_issue_count" "{repo}" "$(date -u +%s)" \
>       > "$HOME/.autospec/batch-done.json"
>     echo "[monitor] batch ${batch_num:-1}: processed $batch_issue_count/${effective_batch_size:-$BATCH_SIZE} issues — writing batch-done.json and exiting for fresh context"
>     exit 0
>   fi
>   # Immediate next-issue pickup: NO SLEEP after process(ISSUE). Re-enter the top
>   # of this loop immediately so the fresh queue scan can pick any issue unblocked
>   # by the merge or failure cleanup that just completed.
> ```
>
> ### Implementer prompt selection (turbo-integration routing)
>
> Before dispatching, read the issue's labels:
>
> ```bash
> labels=$(gh issue view <ISSUE> --json labels --jq '[.labels[].name] | join(",")')
> ```
>
> - **If `labels` contains `autospec:v2-flow`** — load the prompt template from `skills/autospec-run/prompts/phase4-implementer.md` (relative to this skill's install location, or via `AUTOSPEC_SKILLS_DIR`/the harness's skill mount). That prompt embeds the absorbed-discipline path: expand → implement → finalize → peer-review (via `codex exec`) → evaluate-findings. Use it verbatim as the subagent prompt body.
> - **Otherwise** — use the legacy inline prompt below (current behavior). Legacy path is retained until every pre-v2 issue has drained.
>
> Both paths share the same outer monitor loop (queue scan, lock-step compliance, label-based locking, heartbeat updates, post-process pickup). The selection only changes the inner subagent prompt body.
>
> `process(ISSUE)` dispatches a **foreground subagent** (wait for return) with this prompt:
>
> **Prompt construction (cache-prefix + dynamic suffix):**
> Before dispatch, the orchestrator builds the subagent prompt. Two options:
>
> **Option A (recommended): `gen-implementer-prompt.sh`** — standalone assembler:
>    ```bash
>    _body_file=$(mktemp -t autospec-body-XXXXXX.md)
>    trap 'rm -f "$_body_file"' EXIT
>    gh issue view <ISSUE> --json body --jq '.body' > "$_body_file"
>    combined_prompt=$(bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/gen-implementer-prompt.sh" \
>      --issue-body "$_body_file" \
>      --branch "<BRANCH>" \
>      --issue-labels "<ISSUE_LABELS>" \
>      --repo "<REPO>")
>    ```
>
> **Option B (legacy): `bundle-and-dispatch.sh`** — wraps bundle-static-context internally:
>
> 1. **Static cached prefix + dynamic suffix** — call `bundle-and-dispatch.sh` to assemble the combined prompt:
>    ```bash
>    # Write dynamic suffix to a temp file first
>    _suffix_file=$(mktemp -t autospec-suffix-XXXXXX.txt)
>    trap 'rm -f "$_suffix_file"' EXIT
>    printf '%s\n' "<ISSUE_BODY_AND_DIRECTIVES>" > "$_suffix_file"
>    combined_prompt=$(bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/bundle-and-dispatch.sh" \
>      --role implementer \
>      --issue-labels "<ISSUE_LABELS>" \
>      --dynamic-suffix-file "$_suffix_file")
>    ```
>    `bundle-and-dispatch.sh` calls `bundle-static-context.sh` internally to emit the static cached
>    prefix (framed by `<!-- CACHE BOUNDARY -->` markers, containing SKILL.md + AGENTS.md + RULE_ID
>    table + tag-filtered saved-memory + lockstep rules + implementer scaffolding), then appends the
>    dynamic suffix verbatim after the closing marker.
>    Pass the prefix block (up to and including the closing `<!-- CACHE BOUNDARY -->`) with
>    `cache_control: { type: "ephemeral" }` so Anthropic's prompt cache can reuse it across
>    dispatches in the same monitor session (5-min TTL).
>
> 2. **Dynamic uncached suffix** — appended by `bundle-and-dispatch.sh` after the cached prefix:
>    the issue body, per-iteration findings (if retry > 1), branch name, and "begin coding now".
>
> The combined prompt sent to the subagent is:
>
> ```
> **Model tier:** `TIER_B` (implementation work) — cheaper model with medium thinking; resolved at startup. Silently fall back to `TIER_A` if unavailable.
>
> **Hard limits.** Max 40 tool calls per issue. Max 3 self-review iterations. If you rewrite the same file twice with no test progress, abort: comment the blocker on the issue, release the lock label, exit. No wall-clock cap.
>
> Implement GitHub issue #<ISSUE>: "<TITLE>" on {repo}. Spec is the issue body below.
>
> ===ISSUE BODY===
> <BODY>
> ===END===
>
> Keep a progress heartbeat so the monitor can prove forward movement:
> - Create/update `~/.autospec/process-heartbeats/<repo-slug>/<ISSUE>.json` at each major step:
>   - `claimed`, `worktree_ready`, `tests_started`, `tests_passed`, `pr_created`, `smoke_retry`, `reviewed`, `merged`, `failed`
> - Schema: `{"issue":"<ISSUE>","branch":"<BRANCH>","step":"<STEP>","ts":<unix_epoch>,"pr":"<PR>","repo":"{repo}"}`
> - Delete this file on terminal SUCCESS/FAILURE in both clean and failure paths.
>
> ## Project rules you MUST honor
>
> <verbatim concatenation of relevant feedback_*.md bodies — injected by bundle-static-context.sh --role implementer before dispatch>
>
> ## RULE_IDs (from AGENTS.md ## Implementation-quality contract)
>
> <verbatim RULE_ID table from AGENTS.md — injected by bundle-static-context.sh --role implementer>
>
> ## Acceptance criteria as constraints
>
> <verbatim AC checkbox list from issue body — every checkbox must be green before push>
>
> 1. Worktree off origin/main:
>    cd {repo_root} && git fetch origin
>    git worktree add -b <BRANCH> /tmp/wt-<BRANCH> origin/main && cd /tmp/wt-<BRANCH>
> 2. TDD per AGENTS.md: failing test first → implement → refactor → commit. NO DB/external mocks. Follow file paths and signatures from the issue body verbatim.
> 3. Build + test green (use the project's test runner; for Go: `go build ./... && go test ./... -count=1`; for Node: `npm test`; for Python: `pytest`). 80%+ coverage on changed files.
> 3a. **autospec-test gate** (run when `skills/autospec-test/scripts/run-gate.sh` exists in the repo): invoke the gate against the PR's target repo root. Handle exit codes per spec §7a/§7b:
>    ```bash
>    GATE_SCRIPT="skills/autospec-test/scripts/run-gate.sh"
>    if [ -f "$GATE_SCRIPT" ] && [ -f ".autospec/test.yml" ]; then
>      GATE_JSON_OUT=$(mktemp -t autospec-gate-XXXXXX.json)
>      trap 'rm -f "$GATE_JSON_OUT"' EXIT
>      bash "$GATE_SCRIPT" . --output-gate "$GATE_JSON_OUT" --pr "<PR>" --repo "{repo}" || GATE_EXIT=$?
>      case "${GATE_EXIT:-0}" in
>        0) echo "[gate] autospec-test: passed" ;;
>        1) echo "[gate] autospec-test: blocked — PR comment posted, labels applied; continuing review loop" ;;
>        2) echo "[gate] autospec-test: fatal (exit 2) — halt batch"; exit 2 ;;
>      esac
>    fi
>    ```
>    Exit 0: proceed to merge. Exit 1: block PR (post comment + labels; do NOT merge; treat as review finding). Exit 2: halt entire batch (comment on issue, label `in-progress-by-bot` → `auto-implement`, exit monitor).
> 3b. <!-- docs-drift-gate:begin -->
> ## Docs drift gate
> Run after autospec-test gate, before LGTM review. Skip if issue body contains a line matching `^docs:\s*skip\s*$` (case-insensitive):
>    ```bash
>    if ! grep -qiE '^docs:[[:space:]]*skip[[:space:]]*$' <(gh issue view <ISSUE> --json body --jq .body 2>/dev/null || true); then
>      DRIFT_JSON="$(bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/check-doc-drift.sh" --pr "<PR>" 2>/tmp/drift-<PR>.err)"; drift_exit=$?
>      case "$drift_exit" in
>        0) ;;  # no drift — continue to LGTM
>        1)
>          # Drift detected — feed self-heal loop via docs-extension classifier
>          node "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/loop-classifier-docs-extension.mjs" \
>            --drift-json "$DRIFT_JSON" --issue "<ISSUE>" --pr "<PR>" 2>/dev/null || true
>          gh pr comment <PR> --body "$(printf 'docs drift detected — self-heal queued:\n\n```json\n%s\n```' "$DRIFT_JSON")"
>          gh issue edit <ISSUE> --add-label "docs:drift" 2>/dev/null || true
>          ;;
>        2)
>          # Missing scope — needs human review
>          gh pr comment <PR> --body "$(printf 'docs: missing scope — changed files not covered by any doc scope. Operator review needed.\n\n```json\n%s\n```' "$DRIFT_JSON")"
>          gh issue edit <ISSUE> --add-label "docs:missing-scope" 2>/dev/null || true
>          gh issue edit <ISSUE> --add-label "needs-human-review" 2>/dev/null || true
>          exit 1
>          ;;
>      esac
>    else
>      gh pr comment <PR> --body "docs: drift check skipped (docs:skip in issue body)" 2>/dev/null || true
>      gh issue edit <ISSUE> --add-label "docs:skipped" 2>/dev/null || true
>    fi
>    ```
>    <!-- docs-drift-gate:end -->
> 4. <!-- RETRY-LOOP:begin --> Adaptive commit loop (MAX_IMPL_RETRIES):
>    ```bash
>    attempt=1
>    MAX_IMPL_RETRIES="${MAX_IMPL_RETRIES:-5}"
>    directive_context=""
>    while [ "$attempt" -le "$MAX_IMPL_RETRIES" ]; do
>      # Conventional commits (feat:/fix:/test:/docs:/refactor:). NEVER bypass hooks. NEVER amend.
>      if git commit -m "<conventional-commit-message>"; then
>        # pre-commit hook passed — verify AC bats tests are green
>        if bats tests/ac/issue-<ISSUE>.bats 2>/dev/null; then
>          break  # success
>        fi
>        # AC tests still failing — treat as lint failure, roll back commit
>        git reset HEAD~1
>      fi
>      # Capture lint directives for next attempt
>      findings=$(bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/lint-implementation.sh" --pre-commit --staged --directives 2>/dev/null || true)
>      if [ -n "$findings" ]; then
>        directive_context="${directive_context}
>
> ## Retry attempt ${attempt} findings
> ${findings}
>
> Fix these BEFORE the next code generation."
>      fi
>      attempt=$((attempt + 1))
>    done
>    if [ "$attempt" -gt "$MAX_IMPL_RETRIES" ]; then
>      gh issue comment <ISSUE> --body "Implementer hit max retries; manual intervention needed"
>      gh issue edit <ISSUE> --remove-label "auto-implement-active" 2>/dev/null || true
>      exit 1
>    fi
>    ```
>    <!-- RETRY-LOOP:end -->
> 5. Push: git push -u origin <BRANCH>
>    ```bash
>    # autospec-stop sentinel check — inside process(ISSUE), after each major step
>    if [ -f "$HOME/.autospec/stop.flag" ] && [ "$(head -1 $HOME/.autospec/stop.flag)" = "immediate" ]; then
>      bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-stop.sh" --abort-current-issue "$ISSUE" "$BRANCH" "$LAST_STEP"
>      exit 0
>    fi
>    ```
> 6. PR: gh pr create --base main --head <BRANCH> --title "<TITLE>" --body "Closes #<ISSUE>\n\n<summary>". Capture PR.
>    After the LLM subagent returns, record telemetry (tokens JSON written by the harness to `.autospec/tokens-<ISSUE>.json` if present):
>    ```bash
>    if [ -f ".autospec/tokens-<ISSUE>.json" ]; then
>      bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/record-telemetry.sh" \
>        --dispatch-id "<DISPATCH_ID>" --role implementer --issue "<ISSUE>" \
>        --tokens-json ".autospec/tokens-<ISSUE>.json"
>    fi
>    ```
> 7. Inner loop (max 3 iterations):
>    ```bash
>    # autospec-stop sentinel check — inside process(ISSUE), after each major step
>    if [ -f "$HOME/.autospec/stop.flag" ] && [ "$(head -1 $HOME/.autospec/stop.flag)" = "immediate" ]; then
>      bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-stop.sh" --abort-current-issue "$ISSUE" "$BRANCH" "$LAST_STEP"
>      exit 0
>    fi
>    ```
>    - Run the **Primary smoke test** from the issue body. If it fails, fix and recommit before review.
>    - **Fused guardian + LGTM review** (one subagent does both — saves one dispatch per inner-loop iteration):
>      <!-- guardian-block:begin -->
>      Run deterministic lint first (no subagent cost):
>        rm -f /tmp/guardian-<PR>.md
>        if [ "${AUTOSPEC_NO_GUARDIAN:-0}" != "1" ]; then
>          bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/lint-implementation.sh" <PR> --issue <ISSUE> >> /tmp/guardian-<PR>.md 2>&1
>        fi
>        det_exit=$?
>
>      **Model tier:** `TIER_B` for normal issues; `TIER_A` for `regression`/`priority:high` issues. Silently fall back to `TIER_A` if `TIER_B` unavailable.
>
>      **Assemble reviewer prompt** — call `gen-reviewer-prompt.sh` to compose the combined prompt (static cached prefix + dynamic suffix):
>      ```bash
>      _pr_diff_file=$(mktemp -t autospec-pr-diff-XXXXXX.diff)
>      trap 'rm -f "$_pr_diff_file"' EXIT
>      gh pr diff <PR> > "$_pr_diff_file"
>      combined_reviewer_prompt=$(bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/gen-reviewer-prompt.sh" \
>        --pr-diff "$_pr_diff_file" \
>        --prev-findings "/tmp/guardian-<PR>.md" \
>        --issue-labels "<ISSUE_LABELS>" \
>        --repo "<REPO>")
>      ```
>      Pass `combined_reviewer_prompt` as the reviewer subagent prompt. The static cached prefix is framed by `<!-- CACHE BOUNDARY -->` markers; pass it with `cache_control: { type: "ephemeral" }` so Anthropic's prompt cache can reuse it across inner-loop iterations.
>
>      Dispatch ONE **foreground subagent** with this brief:
>        > You are the implementation reviewer for PR #<PR> on {repo}, closing issue #<ISSUE>.
>        >
>        > **Part 1 — Guardian (contract compliance)** — skip if `AUTOSPEC_NO_GUARDIAN=1`:
>        > 1. Read AGENTS.md `## Implementation-quality contract` for the RULE_ID table and directive map.
>        > 2. Read issue #<ISSUE> body — note `## Implementation scope`, `## Implementation outline`, `## Tests required`, and any `Guardian: skip-*` lines.
>        > 3. Read deterministic findings in /tmp/guardian-<PR>.md (populated by lint-implementation.sh; may be empty if guardian disabled).
>        > 4. Run `gh pr diff <PR>` and `gh pr view <PR> --json files,title,body`.
>        > 5. Apply LLM-tier RULE_IDs (HALLUCINATED_API, DUPLICATE_CODE, DOC_OUT_OF_SYNC semantic pass, INVENTED_CONFIG). Collect as `RULE_ID:<path>:<line>: <desc>`. Honor `Guardian: skip-*` with `INFO:` lines.
>        >
>        > **Part 2 — LGTM (correctness review):** Using the same diff and issue body already in context:
>        > 6. Check correctness, edge cases, missing tests, AGENTS.md compliance (TDD, no mocks, conventional commits).
>        > 7. Collect findings as a numbered list.
>        >
>        > **Hard limit:** max **25 tool calls total** (Parts 1 + 2 combined). If budget exhausted, append `RULE_ID:OUT_OF_SCOPE: reviewer budget exhausted; PR needs human review` and proceed to verdict.
>        >
>        > **Verdict:** If Part 1 has ZERO blocking findings (INFO lines OK) AND Part 2 has no findings: return ONLY the token: `LGTM`. Otherwise return a numbered findings list — RULE_ID findings first, then LGTM findings.
>
>      If `LGTM` && det_exit == 0:
>        gh pr comment <PR> --body "<!-- guardian-block --> Review: clean. <!-- /-->"
>        run **Operator/full verification**
>        bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/ci-wait.sh" <PR>  # fire-and-forget sentinel
>        if [ -f ".autospec/tokens-<ISSUE>-reviewer.json" ]; then
>          bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/record-telemetry.sh" \
>            --dispatch-id "<DISPATCH_ID>-reviewer" --role reviewer --issue "<ISSUE>" \
>            --tokens-json ".autospec/tokens-<ISSUE>-reviewer.json"
>        fi
>        # monitor exits to parking state HERE — orchestrator relaunches when ~/.autospec/ci-state/<PR>.signal settles
>        # On relaunch: run ci-wait-poll.sh <PR>; break SUCCESS if exit 0 (pass)
>        break SUCCESS if required checks pass.
>      If `LGTM` but det_exit != 0:
>        Treat deterministic findings as blocking. Comment, fix, recommit, push. Continue inner loop.
>      If findings list:
>        gh pr comment <PR> --edit-last --body "<!-- guardian-block:begin -->\n## Review findings (iter <K>/3)\n<findings>\n<!-- guardian-block:end -->"
>        Append findings to implementer retry context. Continue inner loop (counts toward 3-iter cap).
>      On 3-iter exhaustion with non-LGTM:
>        gh label create guardian-blocked --color e11d21 --force --repo {repo}
>        gh issue edit <ISSUE> --add-label guardian-blocked
>        Run failure cleanup (comment, swap label, close PR).
>        rm -f /tmp/guardian-<PR>.md
>      <!-- guardian-block:end -->
>    - **Regression meta-review** (only for `regression`/`priority:high` issues, after LGTM passes): dispatch a second `TIER_A` subagent: "Would the fused reviewer have caught the original gap? If yes, add missing checklist items to `reports/autospec-review/reviewer-lessons.md` (entry per item, parent gap_id, date) and re-review. Both passes must approve before merge."
>    - If LGTM (and meta-review passes if applicable): break SUCCESS.
>    ```bash
>    # autospec-stop sentinel check — inside process(ISSUE), after each major step
>    if [ -f "$HOME/.autospec/stop.flag" ] && [ "$(head -1 $HOME/.autospec/stop.flag)" = "immediate" ]; then
>      bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-stop.sh" --abort-current-issue "$ISSUE" "$BRANCH" "$LAST_STEP"
>      exit 0
>    fi
>    ```
> 8. SUCCESS: Run the **rebase-and-retest pre-merge gate** before admin-merging. Addresses cross-session CI rot (issue #307): two PRs each green against pre-merge main can together break main, so we re-prove this PR against post-merge main. Cap is `AUTOSPEC_REBASE_MAX_ATTEMPTS` (default 3). The gate ends with the admin-merge, so do NOT issue a second merge after the block.
>    ```bash
>    max_attempts="${AUTOSPEC_REBASE_MAX_ATTEMPTS:-3}"
>    attempt=0
>    wait_for_ci_green() {
>        while :; do
>            rollup=$(gh pr view <PR> --json statusCheckRollup --jq '.statusCheckRollup // []')
>            pending=$(printf '%s' "$rollup" | jq '[.[] | select(.conclusion == null)] | length')
>            bad=$(printf '%s' "$rollup" | jq '[.[] | select(.conclusion=="FAILURE" or .conclusion=="CANCELLED" or .conclusion=="TIMED_OUT" or .conclusion=="ACTION_REQUIRED")] | length')
>            total=$(printf '%s' "$rollup" | jq 'length')
>            if [ "$bad" != "0" ]; then
>                gh issue comment <ISSUE> --body "PR #<PR>: a required check failed after rebase-and-retest. Pausing for operator review."
>                exit 1
>            fi
>            if [ "$total" != "0" ] && [ "$pending" = "0" ]; then return 0; fi
>            sleep 30
>        done
>    }
>    while [ "$attempt" -lt "$max_attempts" ]; do
>        state=$(gh pr view <PR> --json mergeStateStatus --jq .mergeStateStatus)
>        # mergeStateStatus values: CLEAN | BEHIND | BLOCKED | DIRTY | HAS_HOOKS | UNKNOWN | UNSTABLE
>        case "$state" in
>            CLEAN|HAS_HOOKS|UNSTABLE) break ;;
>            BEHIND)
>                if ! gh pr update-branch <PR>; then
>                    gh issue comment <ISSUE> --body "PR #<PR>: \`gh pr update-branch\` failed (auth/api/conflict). Pausing for operator review."
>                    exit 1
>                fi
>                wait_for_ci_green
>                ;;
>            DIRTY)
>                gh issue comment <ISSUE> --body "PR #<PR> has a merge conflict against main; needs human resolution."
>                exit 1
>                ;;
>            BLOCKED) sleep 30; wait_for_ci_green ;;
>            *) sleep 15 ;;
>        esac
>        attempt=$((attempt + 1))
>    done
>    if [ "$attempt" -ge "$max_attempts" ]; then
>        gh issue comment <ISSUE> --body "PR #<PR>: rebase-and-retest stalled after $max_attempts attempts; main is moving faster than CI completes. Pausing for operator review."
>        exit 1
>    fi
>    gh pr merge <PR> --admin --squash --delete-branch
>    ```
>    The block ends with the admin-merge; merge auto-closes the issue.
>    ```bash
>    # autospec-stop sentinel check — inside process(ISSUE), after each major step
>    if [ -f "$HOME/.autospec/stop.flag" ] && [ "$(head -1 $HOME/.autospec/stop.flag)" = "immediate" ]; then
>      bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-stop.sh" --abort-current-issue "$ISSUE" "$BRANCH" "$LAST_STEP"
>      exit 0
>    fi
>    ```
> 9. FAILURE (loop exhausted): comment failure on issue, swap label `in-progress-by-bot` → `auto-implement`, `gh pr close <PR> --delete-branch`.
> 10. Cleanup: cd / && git -C {repo_root} worktree remove /tmp/wt-<BRANCH> --force
> 11. Report: PR number, outcome, one-paragraph summary.
>
> Hard rules: NEVER push to main, force-push, bypass hooks, or touch the umbrella issue. gh CLI only.
> ```
>
> Hard rules for the monitor: ONE issue at a time, sequential. Do NOT touch the umbrella. On transient gh errors retry once. Do NOT ask the user — auto-merge authority is granted in AGENTS.md.
>
> Final output when shutdown: numbered list of every processed issue with PR # and outcome.

Capture the agent ID / log path for monitoring.

If your harness lacks background delegation: open a separate terminal/tmux pane, run the monitor prompt in a fresh session there, and have it write progress to a logfile that Phase 5 can tail.

## Phase 5 — Periodic status updates

Set up a recurring check (every ~25 min) using your harness's self-paced wakeup capability. Each tick:

```bash
gh issue list --repo {repo} --label auto-implement,in-progress-by-bot,awaiting-merge --state all --json number,state,labels
gh pr list --repo {repo} --state all --json number,state,title --limit 20
```

Post a one-paragraph delta to the user (newly closed issues, newly merged PRs, failures, blockers). If two consecutive ticks have nothing new, slow to ~50 min cadence to reduce noise. Stop the loop when:
- the monitor agent reports completion, OR
- all child issues are CLOSED, OR
- the user explicitly stops.

If your harness lacks self-paced wakeup: register a local `cron`/`launchd` job that runs the same status-check prompt at the chosen cadence, OR ask the user to invoke `status-update` manually.

## Phase 6 — Final report

When the monitor terminates, post a final summary to the user: every issue processed, every PR merged, total elapsed wall time, and any failures that need human attention. Append the **Deferred summary** when `deferred[]` is non-empty:

```
Deferred (off-profile under <profile_name>: ctx=<P.ctx>, reasoning=<P.reasoning>):
- #<N1>: <reason>
- #<N2>: <reason>
...
Re-run with --profile <larger> to pick these up, or run /autospec-run on a host that fits the larger profile.
```

If `deferred[]` is empty, omit the section.

## Post-batch audit (autospec-review interlock)

Runs after the last issue in this batch closes/merges, before printing
the final report.

Skip when:

- `~/.autospec/no-review.flag` exists, OR
- `--no-postreview` was passed to autospec-run.

Otherwise:

```bash
/autospec-review --since "${BATCH_START_DATE}"
```

On gaps found: post a comment to the autospec-run status thread
summarising gap counts by spec. Do NOT block batch completion.
Failures from `/autospec-review` log a warning but do not fail the
overall run.

## Constraints (apply throughout)

- **Cadence**: sub-hour polling needs an in-session background subagent or a local cron. Cloud cron services (e.g. Anthropic remote routines) typically have a 1-hour minimum and are not appropriate.
- **TDD non-negotiable** per AGENTS.md. Real services in tests, no DB mocks.
- **Branch-per-issue**, conventional commits, no force-push, no hook bypass.
- **Auto-merge** on success per AGENTS.md. Do not ask.
- **Context budget**: stay under 60%. Delegate everything possible.
- **Failure isolation**: a failed issue restores its `auto-implement` label so the next monitor cycle picks it up; it does not block the cascade unless dependent issues are downstream.
- **Fresh repo**: if Phase 0 created the repo, the very first commit is the scaffold (incl. `AGENTS.md`); the spec lands as the second commit; child branches branch off that `main`.
- **Small-LLM target**: child issues are sized and pre-staged for 32B-class local LLMs (e.g. qwen3-32B / Qwen3-30B-A3B on Ollama, 48 GB-class Macs). Bias toward smaller issues, sectional spec anchors, pre-staged file pointers, checkbox AC, and one Primary smoke test.
