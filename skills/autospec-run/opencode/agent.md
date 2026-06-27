---
description: Use when the user has already run /autospec-define (or otherwise has a populated set of auto-implement GitHub issues) and wants the implementation half — Phases 4-6 — to run autonomously with admin auto-merge. Supports --profile <name> filtering against ~/.autospec/model-profiles.yml.
mode: primary
---

# autospec-run workflow (harness-neutral)

Take the populated `auto-implement` queue on the current GitHub repo and run the implementation half of the autospec pipeline:
**autonomous monitor → admin-squash-merge each PR → periodic status updates → final report.**

Manage your own context — never exceed 60%. Delegate to subagents whenever your harness supports it; do not investigate, write code, or design directly in the main conversation when a subagent can do it.

Autospec-run is an autonomous loop and should not ask operator questions for normal operations. Only surface a question if a hard blocker requires explicit manual recovery.

<!-- autospec-block:startup-self-update SKILL_NAME=autospec-run -->

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
2. **Re-install the full autospec suite from `main`** by piping the canonical installer:
   ```bash
   curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh | bash -s -- --skill all --harness all --update
   ```
   Run this one-liner once; it refreshes all autospec skills across all harnesses.
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

## Status mode

Apply the same read-and-normalize approach. If the normalized request is exactly
`status` (or `status` followed by `--<word>` flags such as `--json` / `--repo` /
`--stale-secs`), this skill enters status mode and does NOT run the pipeline — it
prints a one-glance operator view of the live run (in-flight issues with step +
age + branch + PR + STALE detection, the queue counts, and the stop-flag state)
instead of making the operator hand-parse heartbeat JSON.

1. Dispatch to `bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-run-status.sh" <args>`.
2. Print the helper's stdout to the user.
3. Stop. Do not enter Phase 0 or any pipeline phase.

## Invocation

```
/autospec-run [--profile <name>]
```

- `--profile <name>` — filter the candidate queue against `~/.autospec/model-profiles.yml` so only issues whose `ctx:*` and `reasoning:*` labels fit the named profile are picked up. Issues that exceed the profile on either axis are appended to a `deferred[]` list and printed in the run-end summary.
- (no flag) — load `~/.autospec/model-profiles.yml`'s `default:` profile and run with it. If the file is missing, run auto-init (below) then exit so the user can review/edit before re-running.
- `--profile <unknown>` — exit non-zero and print the list of available profile names from `~/.autospec/model-profiles.yml`.
- `--worker-id <id>` — override the distributed worker identity written to
  GitHub run-state comments. Otherwise derive it from host, user, harness, pid,
  and start timestamp.
- `--coordination-status` — print active workers, claimed issues, blockers,
  stale claims, conflicts, and the next safe batch, then exit without claiming.
- `--max-parallel-safe` — print the next safe parallel batch from
  `list-ready-issues.sh` and exit without claiming.
- `--claim <issue>` — attempt a deterministic claim via `claim-issue.sh` and
  exit with that helper's status (`0` claimed, `2` already claimed/skipped).
- `--release <issue>` — release a distributed claim via `release-issue.sh`.

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
<!-- autospec-block:harness-adapter-core -->

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

## Relevant memory injection (run-start, once)

Before executing the main pipeline phases, call the injector to surface relevant saved lessons:

```bash
bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/inject-relevant-memory.sh" \
  --context "<skill-relevant keywords from request/issue/spec>" \
  --top-k 5
```

Prepend the output block (if non-empty) to your working context. This surfaces lessons like
`feedback_bash_return_trap_leak.md` that prevent re-occurrence of known pitfalls.

## Run-start batch timestamp (run-start, once)

Capture the run-start instant so the end-of-run gap-remediation phase (Phase 5.5) can scope its review to work shipped during THIS run. Run exactly once, before the Phase 4 monitor launches:

```bash
bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/run-batch-start.sh" --write
```

This writes the UTC ISO 8601 timestamp to `~/.autospec/.run-batch-start` (idempotent within a run; pass `--force` only when intentionally starting a fresh batch window). Phase 5.5 reads it back via `run-batch-start.sh --read`, which yields `BATCH_START_DATE`.

## Explore-on-drain counter reset (run-start, once)

Reset the per-repo explore-on-drain cycle counter so a fresh `/autospec-run` always starts with a clean slate (spec: "counter resets when the operator clears it or starts a fresh `/autospec-run`"). Run once, immediately after the batch-start timestamp above:

```bash
# Derive the canonical per-repo slug (owner__name) for state scoping.
_eod_slug="$(gh repo view --json nameWithOwner -q .nameWithOwner 2>/dev/null \
    | sed 's#/#__#' \
    || printf '%s' "$(git rev-parse --show-toplevel 2>/dev/null || pwd)" \
         | tr '/' '_' | sed 's/^_//')"
rm -f "$HOME/.autospec/explore-on-drain/${_eod_slug}/cycles"
```

This ensures the cycle cap (`AUTOSPEC_EXPLORE_ON_DRAIN_MAX_CYCLES`, default 3) counts only chains within the current run, not across multiple historical runs. The `_eod_slug` variable is reused in the ALL_DONE block below.

## Single-instance session lock (run-start, once)

Acquire a per-session lock BEFORE launching the Phase 4 monitor so a single
harness session never runs two concurrent monitors (which collide on the queue
and on git branches). The lock is keyed by the harness session id, so separate
sessions still run independently by design.

```bash
LOCK="${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-run-session-lock.sh"
if ! bash "$LOCK" acquire --repo "{repo}"; then
    # exit 3 = a monitor is already active in THIS session.
    echo "autospec-run: refusing to start a second monitor in this session." >&2
    bash "$LOCK" status
    # STOP — do NOT launch Phase 4. Halt the active monitor with /autospec-stop,
    # or re-run the lock helper with --force to override a stale lock.
    exit 0
fi
```

Release the lock on EVERY terminal exit of this run — the Phase 6 final report,
the `ALL_DONE` shutdown, and stop mode:

```bash
bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-run-session-lock.sh" release
```

The lock is advisory and self-scoped: a crashed session's lock only blocks that
same session id (a fresh session gets a new id), and `--force` overrides it.

## Phase 4 — Background autonomous monitor

**Fresh-subagent-per-issue (canonical Phase 4 path, formerly single-agent absorbed-discipline).** Each issue is processed by a FRESH top-level subagent dispatched by the orchestrator — expand → implement → finalize → peer-review → evaluate-findings → PR → merge — in that subagent's own context. The orchestrator NEVER implements in its own context; it only claims, dispatches, and waits. Each subagent receives full tool access because it is a top-level `Agent` call launched directly by the main session orchestrator. **Constraint:** Subagents spawned by background `Agent` calls do NOT inherit the `Agent` tool, so nested monitors cannot dispatch inner implementers. Phase 4 implementers must be top-level agents launched directly by the main session orchestrator, not nested inside a monitor. The `process(ISSUE)` notation below is shorthand for "dispatch a fresh top-level subagent to do this work", NOT in-context implementation by the orchestrator. Batch>1 is an explicit operator opt-in via `AUTOSPEC_BATCH_SIZE=N`; the default is 1 (one issue per subagent). The `reasoning:deep` force-to-1 rule is retained: deep issues stay batch=1 even under an operator `AUTOSPEC_BATCH_SIZE=N` override.

Record this durable preference in `AGENTS.md` (idempotent — skip if already present):

> **Auto-merge authority for auto-implement PRs.** Admin-merge auto-implement PRs (`gh pr merge <#> --admin --squash --delete-branch`) when (a) the full target-repo validation/test suite has passed locally after the branch is current with `main`, (b) all required CI checks pass — slow optional checks like TeamCity may be pending only after the full local suite is green, (c) the self-review subagent returned `LGTM`, (d) PR closes an `auto-implement` issue from a `feat/*` branch.

**Off-peak tip:** For queues of 10+ issues (8+ hour runs), consider launching at night or on weekends. Usage limits are shared across all sessions — running long batches off-peak preserves daytime tokens for interactive work.

**Usage-limit recovery guard.** Before launching the monitor, determine the exact non-interactive command that can relaunch this same `/autospec-run` invocation in the current repo and store it in `AUTOSPEC_RESUME_COMMAND` if the harness has not already set one. This must be a real shell command, for example the same Claude Code, Codex CLI, OpenCode, tmux, or wrapper command the operator used to start the run with the same `--profile` and repo path. Do not ask the user for it during a running monitor.

**Durable run registry (crash-resume).** At monitor launch, also persist that same relaunch command to the durable run registry so it survives a host/session crash and reboot — the session env var `AUTOSPEC_RESUME_COMMAND` does not. Write it once per launch with the identical command:

```bash
REGISTRY="${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-run-registry.sh"
if [ -f "$REGISTRY" ] && [ -n "${AUTOSPEC_RESUME_COMMAND:-}" ]; then
  bash "$REGISTRY" write --repo "$(gh repo view --json nameWithOwner --jq .nameWithOwner)" \
    --repo-dir "$(pwd)" --harness "<claude|codex|opencode>" --command "$AUTOSPEC_RESUME_COMMAND"
fi
```

`/autospec-resume` reads `~/.autospec/active-runs/<repo-slug>.json` on a fresh start to relaunch this run. The registry is the only durable carrier of the relaunch command across reboot.

When a harness reports a deterministic usage-limit/quota/capacity pause with a known reset time or wait duration, do not spend tokens diagnosing the message. Immediately arm the shell supervisor and exit:

```bash
USAGE_LIMIT="${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-usage-limit.sh"
if [ -n "${AUTOSPEC_USAGE_LIMIT_RESUME_AT:-}" ]; then
  bash "$USAGE_LIMIT" arm --harness "<claude|codex|opencode>" --repo-dir "$(pwd)" \
    --command "$AUTOSPEC_RESUME_COMMAND" --resume-at "$AUTOSPEC_USAGE_LIMIT_RESUME_AT"
  exit 0
elif [ -n "${AUTOSPEC_USAGE_LIMIT_WAIT_SECONDS:-}" ]; then
  bash "$USAGE_LIMIT" arm --harness "<claude|codex|opencode>" --repo-dir "$(pwd)" \
    --command "$AUTOSPEC_RESUME_COMMAND" --wait-seconds "$AUTOSPEC_USAGE_LIMIT_WAIT_SECONDS"
  exit 0
fi
```

The helper writes `~/.autospec/usage-limits/<run-id>.json`, starts a background daemon, polls every 300 seconds by default, and relaunches the recorded command after the reset time. This is intentionally shell-only so recovery does not require an LLM turn after the usage limit has already been hit.

Then launch a **background monitor loop** — the orchestrator relaunches the monitor with fresh context after each batch of `AUTOSPEC_BATCH_SIZE` issues (default: 1). The monitor is stateless: all persistent state lives in GitHub labels and heartbeat files, so relaunches are always safe.

```
batch_num=1
while true:
  launch background subagent (pass batch_num; AUTOSPEC_BATCH_SIZE=${AUTOSPEC_BATCH_SIZE:-1})
  wait for task-notification (monitor agent completes)

  # Read and consume the batch-done signal.
  if [ -f "$HOME/.autospec/batch-done.json" ]; then
    status=$(jq -r .status "$HOME/.autospec/batch-done.json" 2>/dev/null || echo "BATCH_COMPLETE")
    rm -f "$HOME/.autospec/batch-done.json"
  else
    status="BATCH_COMPLETE"   # monitor crashed / overflowed — safe to relaunch
  fi

  if [ "$status" = "ALL_DONE" ]; then
    # Queue drained — consult explore-on-drain.sh to decide whether to
    # auto-chain into /autospec-explore or exit normally to Phase 6.
    # The helper encapsulates: flag check → autonomy gate → dry-well guard
    # → per-repo cycle-cap.  It emits "chain" or "stop"; default is "stop".
    # Pass AUTOSPEC_REPO so the helper scopes state to this repo without
    # calling gh a second time (uses the same slug as the run-start reset).
    _drain_decision=$(AUTOSPEC_REPO="${AUTOSPEC_REPO:-{repo}}" \
        bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/explore-on-drain.sh")
    if [ "$_drain_decision" = "chain" ]; then
      echo "[orchestrator] explore-on-drain: chaining into /autospec-explore on sandbox branch"
      # Record the PR merge watermark before explore starts so we can count
      # only the PRs it ships (used by the dry-well sentinel below).
      _eod_before="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
      /autospec-explore   # runs on its own sandbox branch, NEVER main
      # Count PRs merged by this explore cycle and write the dry-well sentinel.
      # explore-on-drain.sh reads this before the NEXT chain decision:
      # if the count is 0 it emits "stop" (dry-well guard).
      _eod_shipped="$(gh pr list --state merged \
          --json mergedAt,labels \
          --jq "[.[] | select(.mergedAt >= \"${_eod_before}\" and any(.labels[]; .name == \"autospec-explore\"))] | length" \
          2>/dev/null || echo 1)"
      mkdir -p "$HOME/.autospec/explore-on-drain/${_eod_slug}"
      printf '%s\n' "${_eod_shipped}" \
          > "$HOME/.autospec/explore-on-drain/${_eod_slug}/last-shipped"
    fi
    break   # proceed to Phase 6 final report
  fi

  batch_num=$((batch_num + 1))
  echo "[orchestrator] batch $((batch_num - 1)) complete — relaunching monitor with fresh context (batch ${batch_num})"
  # continue immediately, no sleep
```

**Continuation contract:** BATCH_COMPLETE is a continuation signal, not a terminal state. reasoning:deep may reduce a single monitor batch to one issue, but the orchestrator MUST relaunch automatically until ALL_DONE. Never tell the operator to rerun `/autospec-run` after BATCH_COMPLETE; the current invocation owns the relaunch loop until the queue is empty or a hard blocker stops it.

Pass the following prompt verbatim to each background subagent:

> You are the auto-implement monitor for `{repo}`. Process `auto-implement` issues one at a time. Exit after processing `AUTOSPEC_BATCH_SIZE` issues (default: 1) by writing `~/.autospec/batch-done.json` — the orchestrator will relaunch you with fresh context.

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
> **Session batching:** Exit after processing `AUTOSPEC_BATCH_SIZE` issues (default 1) by writing `~/.autospec/batch-done.json` with `status=BATCH_COMPLETE`. BATCH_COMPLETE is a continuation signal, not a terminal state; the orchestrator relaunches you with fresh context. When the queue is fully drained, write `status=ALL_DONE` instead. This keeps each monitor session short to prevent context overflow.

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
> #   BATCH_SIZE="${AUTOSPEC_BATCH_SIZE:-1}"
> #   [ "$BATCH_SIZE" -gt 0 ] 2>/dev/null || BATCH_SIZE=1   # guard against 0 or negative
> #   rm -f "$HOME/.autospec/batch-done.json"   # clear stale file from prior crash
>
> while true:
>   deferred = []   # issues skipped because they exceed the active profile
>
>   # Startup/per-scan heartbeat reconciliation — run before candidate selection.
>   # This deletes closed/merged/orphaned heartbeats, rejects old schemas like
>   # {"issue":407,"status":"in_progress"}, normalizes current schemas, and
>   # releases any `claimed` heartbeat older than AUTOSPEC_WATCHDOG_CLAIMED_TIMEOUT_SECS (default: 1800).
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

### Fab implementer routing (label → gate)

Decide each claimed issue's implementer gate from its labels with the
`fab-route.sh` helper (resolved like the other coordinator scripts):

```bash
FAB_ROUTE="${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/fab-route.sh"
[ -x "$FAB_ROUTE" ] || FAB_ROUTE="skills/autospec-run/scripts/fab-route.sh"
GATE=$(gh issue view "$ISSUE" --json labels --jq '[.labels[].name] | join(",")' \
  | bash "$FAB_ROUTE" --stdin)
```

- `GATE=fab` — the issue carries `area:fab` or `autospec:fab-flow`. Dispatch the
  **fab implementer**: its full-suite gate is clean regen
  (`rm -rf build && .venv/bin/python src/generate.py`) → `stl-release-gate.py`
  on affected models (blocking geometry stages must pass; vision is advisory) →
  unittest, and the Primary smoke is the model's focused regression test.
- `GATE=default` — every other issue keeps the standard implementer + gate.

The branch is the only difference; the fab branch's gate prose lives in
`prompts/phase4-implementer.md`. Routing is a pure label decision (no substring
match: `area:fabric` stays `default`), so it is deterministic and testable.

### Distributed coordinator selection

Before choosing `ready[0]`, prefer the distributed coordinator helpers when they
exist in `${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}` or the checked-out
repo:

```bash
COORD_LIST="${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/list-ready-issues.sh"
COORD_CLAIM="${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/claim-issue.sh"
COORD_RELEASE="${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/release-issue.sh"
[ -x "$COORD_LIST" ] || COORD_LIST="skills/autospec-run/scripts/list-ready-issues.sh"
[ -x "$COORD_CLAIM" ] || COORD_CLAIM="skills/autospec-run/scripts/claim-issue.sh"
[ -x "$COORD_RELEASE" ] || COORD_RELEASE="skills/autospec-run/scripts/release-issue.sh"
```

If `--coordination-status` is active, run `list-ready-issues.sh --repo {repo}
--batch-size "${AUTOSPEC_BATCH_SIZE:-1}"`, print the JSON, and exit. If
`--max-parallel-safe` is active, print only the `.batch` array and exit.

During the normal monitor loop:

1. Run `list-ready-issues.sh --repo {repo} --batch-size "$effective_batch_size"`
   after watchdog reconciliation and profile filtering.
2. Use `.ready[0].number` as the next issue candidate.
3. Claim it through `claim-issue.sh --issue "$ISSUE" --repo {repo}
   --worker-id "${AUTOSPEC_WORKER_ID:-<derived>}" --branch "<BRANCH>"`.
4. Treat claim exit `2` as a normal lost-race or conflict outcome: refresh the
   queue and try another candidate without failing the batch.
5. On failure, stop, or retry exhaustion, call `release-issue.sh` before
   returning `auto-implement` to the queue.

The GitHub `autospec-run-state` comment written by these helpers is the
cross-workstation source of truth. Local process heartbeat files remain useful
for same-host progress and compatibility, but they are not authoritative across
machines. If any coordinator helper is unavailable, fall back to the existing
inline label-swap path below.

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
>       bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/diary-write.sh" \
>         --phase 4 --event monitor-exit \
>         --body "Monitor ALL_DONE: batch=${batch_num:-1} processed=$batch_issue_count repo={repo}" \
>         2>/dev/null || true
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
>       bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/diary-write.sh" \
>         --phase 4 --event monitor-exit \
>         --body "Monitor stopped ($MODE): batch=${batch_num:-1} processed=$batch_issue_count repo={repo}" \
>         2>/dev/null || true
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
>   # reasoning:deep may reduce a single monitor batch to one issue for fresh context.
>   # This is not a run stop: the orchestrator MUST relaunch automatically until ALL_DONE.
>   _next_reasoning=$(gh issue view "${ready[0]}" --json labels \
>     --jq '[.labels[].name | select(startswith("reasoning:"))] | first // "reasoning:medium"' \
>     2>/dev/null || echo "reasoning:medium")
>   if [ "$_next_reasoning" = "reasoning:deep" ]; then
>     effective_batch_size=1
>   else
>     effective_batch_size="${AUTOSPEC_BATCH_SIZE:-1}"
>     [ "$effective_batch_size" -gt 0 ] 2>/dev/null || effective_batch_size=1
>   fi
>   echo "[monitor] effective_batch_size=$effective_batch_size (next issue reasoning: $_next_reasoning)"
>   ISSUE = ready[0]
>   # Atomic claim: claim-issue.sh is the SOLE claim path. It performs the
>   # check-and-swap (auto-implement -> in-progress-by-bot) atomically with a
>   # read-back verification, so the hot loop NEVER re-implements the inline
>   # label swap. Multiple monitors can race the same candidate; exactly one
>   # wins (exit 0), the rest lose (non-zero) and skip to the next candidate.
>   claim_json="$("$COORD_CLAIM" --issue "$ISSUE" --repo {repo} --worker-id "${AUTOSPEC_WORKER_ID:-$(hostname):${USER:-unknown}:monitor:$$}" --branch "${BRANCH:-}")" && claim_rc=0 || claim_rc=$?
>   # exit 1 = hard usage error (never a race signal); exit 2 = lost race.
>   # Split them: a misconfigured claim (rc 1 or any other non-2 non-zero) must
>   # surface an operator-visible WARN, not masquerade as a silently-dropped
>   # lost race (rc 2). rc 0 falls through to ownership below.
>   if [ "$claim_rc" -eq 2 ]; then
>     echo "[monitor] claim lost for #$ISSUE (rc=$claim_rc); refreshing queue"
>     READY_REMOVE=$(printf "%s\n%s" "$READY_REMOVE" "$ISSUE" | grep -v "^${ISSUE}$" || true)
>     ready=($READY_REMOVE)
>     continue
>   elif [ "$claim_rc" -ne 0 ]; then
>     echo "[monitor] WARN: claim hard error for #$ISSUE (rc=$claim_rc) — usage/config error, NOT a lost race; skipping. Check claim-issue.sh invocation." >&2
>     READY_REMOVE=$(printf "%s\n%s" "$READY_REMOVE" "$ISSUE" | grep -v "^${ISSUE}$" || true)
>     ready=($READY_REMOVE)
>     continue
>   fi
>   # exit 0 only: this monitor now owns #$ISSUE (label already swapped to
>   # in-progress-by-bot by claim-issue.sh) -> heartbeat write -> process(ISSUE).
>   _hb_slug="$(bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/repo-slug.sh" --canonical "{repo}")"
>   mkdir -p "$HOME/.autospec/process-heartbeats/$_hb_slug"
>   printf '{"issue":"%s","branch":"","step":"claimed","ts":%s,"pr":"","repo":"%s"}\n' "$ISSUE" "$(date -u +%s)" "{repo}" > "$HOME/.autospec/process-heartbeats/$_hb_slug/$ISSUE.json"
>   bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/notify.sh" "autospec #$ISSUE: claimed" "Starting implementation on {repo}" || true
>   # Issue start summary — print before dispatching process(ISSUE) so the operator
>   # knows exactly what the monitor is about to work on.
>   # Single body fetch — all later steps consume this file (D5: duplicate-read elimination).
>   _issue_body_file="/tmp/issue-${ISSUE}-body.md"
>   gh issue view ISSUE --json body --jq .body 2>/dev/null > "$_issue_body_file" || true
>   ISSUE_TITLE=$(gh issue view ISSUE --json title --jq .title 2>/dev/null || echo "")
>   ISSUE_URL=$(gh issue view ISSUE --json url --jq .url 2>/dev/null || echo "")
>   ISSUE_LABELS=$(gh issue view ISSUE --json labels --jq -r '[.labels[].name] | join(", ")' 2>/dev/null || echo "")
>   ISSUE_GOAL=$(awk '
>     BEGIN{in_goal=0}
>     /^## Goal[[:space:]]*$/ {in_goal=1; next}
>     /^## / && in_goal {exit}
>     in_goal && NF {print; exit}
>   ' "$_issue_body_file")
>   [ -n "$ISSUE_GOAL" ] || ISSUE_GOAL=$(awk 'NF && $0 !~ /^#/ {print; exit}' "$_issue_body_file")
>   ISSUE_SMOKE=$(awk '
>     /### Primary smoke test/ {seen=1; next}
>     seen && /^```/ {fence++; next}
>     seen && fence==1 && NF && $0 !~ /^[[:space:]]*#/ {print; exit}
>   ' "$_issue_body_file")
>   ISSUE_SCOPE=$(awk '
>     /^## Implementation outline[[:space:]]*$/ {in_scope=1; next}
>     /^## / && in_scope {exit}
>     in_scope && /^- / {gsub(/^- /,""); print; count++; if (count>=3) exit}
>   ' "$_issue_body_file" | paste -sd '; ' -)
>   echo "[monitor] starting #$ISSUE: ${ISSUE_TITLE:-<untitled>}"
>   echo "[monitor] url: ${ISSUE_URL:-<unknown>}"
>   echo "[monitor] labels: ${ISSUE_LABELS:-<none>}"
>   echo "[monitor] goal: ${ISSUE_GOAL:-<not provided>}"
>   echo "[monitor] smoke: ${ISSUE_SMOKE:-<not provided>}"
>   echo "[monitor] scope: ${ISSUE_SCOPE:-<not provided>}"
>   process(ISSUE)   # single-agent absorbed-discipline — the monitor IS the implementer; see template below
>   batch_issue_count=$((batch_issue_count + 1))
>   if [ "$batch_issue_count" -ge "${effective_batch_size:-$BATCH_SIZE}" ]; then
>     bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/diary-write.sh" \
>       --phase 4 --event monitor-exit \
>       --body "Monitor BATCH_COMPLETE: batch=${batch_num:-1} processed=$batch_issue_count repo={repo}" \
>       2>/dev/null || true
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
> ### Parallel implementer worktree isolation
>
> Background `Agent` implementers in the same git workdir collide on `git checkout`
> even when their file-level scopes are disjoint — first agent to call
> `git checkout -b <BRANCH>` wins; the second lands on the wrong branch and must
> recover via cherry-pick. File-level disjointness is necessary but NOT sufficient
> for parallel safety; the collision is at the git-branch level.
>
> When dispatching multiple Phase 4 implementers in parallel, the orchestrator
> MUST wrap each dispatch via `dispatch-implementer.sh` (or an inline
> equivalent that produces the same contract):
>
> 1. Create a per-issue worktree at `/tmp/wt-<BRANCH>` off `origin/main` via
>    `git worktree add -b <BRANCH> /tmp/wt-<BRANCH> origin/main`.
> 2. Pre-pend the implementer prompt with an explicit workdir directive naming
>    `/tmp/wt-<BRANCH>` and forbidding `cd`/`git checkout` into the main checkout
>    or sibling branches.
> 3. On agent completion, remove the worktree via
>    `git worktree remove --force /tmp/wt-<BRANCH>` (defer cleanup until the PR
>    has merged so the worktree stays available for retries).
>
> Sequential dispatch in the main checkout is the safe default; worktree-isolated
> parallel dispatch is the only safe parallelism. See `dispatch-implementer.sh`
> and `tests/autospec-run/test_parallel_dispatch.bats` for the canonical contract.
>
> ### Sandbox branch contract (autospec-explore PR-base integration)
>
> When `.autospec/explore-mode.json` is present (written by
> `explore-sandbox.sh`), the Phase 4 implementer MUST target the sandbox
> branch from that file's `branch` field as PR base instead of `main`, and MUST
> refuse `gh pr merge` against `main` while the file is present (refusal
> identifier: `code_health:explore_main_merge_refused`). The full contract lives
> in `skills/autospec-run/prompts/phase4-implementer.md` under "Sandbox branch
> contract" and "No accidental main merges"; the trio enforces it via lockstep.
>
> ### Implementer prompt selection (turbo-integration routing)
>
> Before dispatching, read the issue's labels:
>
> ```bash
> labels=$(gh issue view <ISSUE> --json labels --jq '[.labels[].name] | join(",")')
> ```
>
> - **If `labels` contains `autospec:v2-flow`** — load the prompt template from `skills/autospec-run/prompts/phase4-implementer.md` (relative to this skill's install location, or via `AUTOSPEC_SKILLS_DIR`/the harness's skill mount). That prompt embeds the absorbed-discipline path: expand → implement → finalize → peer-review (via `codex exec`) → evaluate-findings. **Wire the D3 cached prefix (spec Phase 2 child C):** do NOT send the `phase4-implementer.md` body alone — prepend the `gen-implementer-prompt.sh` static cached prefix so the default v2 path stops re-reading SKILL.md + AGENTS.md + the RULE_ID table uncached. Assemble the combined v2 prompt by passing the `phase4-implementer.md` body as the dynamic body that rides BELOW the cached prefix:
>   ```bash
>   # Reuse the single body fetch written at process(ISSUE) start (D5).
>   v2_combined_prompt=$(bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/gen-implementer-prompt.sh" \
>     --issue-body "/tmp/issue-<ISSUE>-body.md" \
>     --branch "<BRANCH>" \
>     --issue-labels "<ISSUE_LABELS>" \
>     --repo "<REPO>" \
>     --body-file "skills/autospec-run/prompts/phase4-implementer.md")
>   ```
>   `gen-implementer-prompt.sh` emits the static cached prefix (framed by `<!-- CACHE BOUNDARY -->` markers, containing SKILL.md + AGENTS.md + RULE_ID table + tag-filtered saved-memory) first; the `phase4-implementer.md` body and the issue assignment ride below it as the dynamic suffix. Pass the prefix block (up to and including the closing `<!-- CACHE BOUNDARY -->`) with `cache_control: { type: "ephemeral" }` so Anthropic's prompt cache reuses it across dispatches in the same monitor session. This only changes prompt **assembly/caching** — the implementer's absorbed-discipline BEHAVIOR (every step in `phase4-implementer.md`) is unchanged.
> - **Otherwise** — use the legacy inline prompt below (current behavior). Legacy path is retained until every pre-v2 issue has drained.
>
> Both paths share the same outer monitor loop (queue scan, lock-step compliance, label-based locking, heartbeat updates, post-process pickup). The selection only changes the inner subagent prompt body.
>
> `process(ISSUE)` is single-agent absorbed-discipline: the monitor agent performs the implementation work itself, in-context, using the prompt below as its working brief. There is NO nested subagent dispatch here — subagents spawned by background `Agent` calls do NOT inherit the `Agent` tool, so the monitor must own the work directly:
>
> **Prompt construction (cache-prefix + dynamic suffix):**
> Before dispatch, the orchestrator builds the subagent prompt. Two options:
>
> **Option A (recommended): `gen-implementer-prompt.sh`** — standalone assembler:
>    ```bash
>    # Reuse the single body fetch written at process(ISSUE) start (D5).
>    combined_prompt=$(bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/gen-implementer-prompt.sh" \
>      --issue-body "/tmp/issue-<ISSUE>-body.md" \
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
> ## Team personality as execution lens
>
> Read the issue body's **Team personality** section before choosing an
> approach. Let that team shape what you emphasize: a reliability/backend team
> should scrutinize operational safety and data boundaries, a frontend/product
> team should scrutinize user workflow and accessibility, a security-sensitive
> team should scrutinize trust boundaries and abuse cases, and so on. Do not
> invent extra scope; use the team personality to decide which risks deserve
> extra attention while still satisfying the issue's concrete acceptance
> criteria.
>
> Keep a progress heartbeat so the monitor can prove forward movement:
> - Create/update `~/.autospec/process-heartbeats/<repo-slug>/<ISSUE>.json` at each major step:
>   - `claimed`, `expand_start`, `worktree_ready`, `tests_started`, `tests_passed`, `pr_created`, `smoke_retry`, `reviewed`, `merged`, `failed`
> - Schema: `{"issue":"<ISSUE>","branch":"<BRANCH>","step":"<STEP>","ts":<unix_epoch>,"pr":"<PR>","repo":"{repo}"}`
> - Delete this file on terminal SUCCESS/FAILURE in both clean and failure paths.
> - Transition notifications: on `tests_passed`, `pr_created`, `merged`, and `failed` call
>   `bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/notify.sh" "<title>" "<body>" || true`
>   once per transition. Dedup with `_notify_fired=""` (reset per issue) and the pattern
>   `case "$_notify_fired" in *:<STEP>:*) ;; *) _notify_fired="${_notify_fired}:<STEP>:"; bash ... || true ;; esac`.
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
> 0. **Heartbeat refresh at expand start.** The very first action before any expand work (reading files, pattern survey, verifying paths) is to refresh the heartbeat to `expand_start`. This covers the claim→worktree_ready window: the monitor wrote `claimed` when it dispatched you; without this refresh the watchdog may falsely reclaim the issue during a long expand phase.
>    ```bash
>    _hb_slug="$(bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/repo-slug.sh" --canonical "{repo}")"
>    mkdir -p "$HOME/.autospec/process-heartbeats/$_hb_slug"
>    printf '{"issue":"%s","branch":"","step":"expand_start","ts":%s,"pr":"","repo":"%s"}\n' \
>      "<ISSUE>" "$(date -u +%s)" "{repo}" \
>      > "$HOME/.autospec/process-heartbeats/$_hb_slug/<ISSUE>.json"
>    ```
> 1. **PR-aware recovery ladder, then worktree.** Resolve the branch state FIRST, then act on the verdict. NEVER `cd`/`git checkout`/`git commit` in the primary checkout — all work happens in a linked worktree off `origin/main`.
>    <!-- worktree-ladder:begin -->
>    ```bash
>    cd {repo_root} && git fetch origin
>    LADDER=$(bash ${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/worktree-guard.sh resolve-branch --branch <BRANCH> --repo {repo})
>    STATE=$(printf '%s' "$LADDER" | sed -n 's/.*"state":"\([^"]*\)".*/\1/p')
>    PR=$(printf '%s' "$LADDER" | sed -n 's/.*"pr":\([0-9]*\).*/\1/p')
>    case "$STATE" in
>      open-pr)
>        # #886 recovery: a PR already exists. SKIP implementation entirely.
>        # Check out the existing PR in a fresh worktree, run the issue's
>        # verification (tests + validate.sh) and the standard review loop
>        # against the EXISTING PR, then merge if green. Never re-implement.
>        bash ${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/worktree-guard.sh create --branch <BRANCH> --path /tmp/wt-<BRANCH> --adopt
>        cd /tmp/wt-<BRANCH>
>        gh pr checkout "$PR"
>        echo "[ladder] open-pr #$PR — skip implementation; verify + review + merge existing PR"
>        ;;
>      branch-only)
>        # #917 recovery: the branch exists with un-PR'd work. Adopt it in a
>        # fresh worktree and CONTINUE the remaining work (do not start over).
>        bash ${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/worktree-guard.sh create --branch <BRANCH> --path /tmp/wt-<BRANCH> --adopt
>        cd /tmp/wt-<BRANCH>
>        echo "[ladder] branch-only — adopted <BRANCH>; continue remaining work"
>        ;;
>      fresh|*)
>        # No branch, no PR: create a new worktree off origin/main.
>        bash ${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/worktree-guard.sh create --branch <BRANCH> --path /tmp/wt-<BRANCH>
>        cd /tmp/wt-<BRANCH>
>        echo "[ladder] fresh — created <BRANCH>"
>        ;;
>    esac
>    # MANDATORY assert gate: MUST exit 0 before the first file edit/commit. A
>    # non-zero exit (in_primary_checkout / dirty / stale_base) is NEVER worked
>    # around — comment the emitted code_health identifier on the issue, restore
>    # the `auto-implement` label (swap `in-progress-by-bot` → `auto-implement`),
>    # remove the heartbeat, and stop this issue.
>    if ! bash ${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/worktree-guard.sh assert; then
>      gh issue comment <ISSUE> --body "worktree-guard assert failed (see code_health identifier above); restoring auto-implement"
>      gh issue edit <ISSUE> --remove-label in-progress-by-bot --add-label auto-implement
>      exit 1
>    fi
>    ```
>    <!-- worktree-ladder:end -->
>    On the `open-pr` path the verification bar EQUALS fresh work — full tests + the standard review loop, never a blind merge. Cleanup is identical for every path: after the merge is confirmed (or on terminal failure), `git worktree remove` the linked worktree and `git worktree prune`; never delete un-pushed work before merge.
> 1a. **Claim the edit surface (claim-guard), nested inside the issue claim.** After the `worktree-guard.sh assert` gate passes and BEFORE the first file edit, take a fine-grained lease on the skill(s)/paths this issue will touch so a concurrent session in another worktree cannot stomp the same trio+golden surface. This is the inner layer of the three-layer caller pattern (worktree-guard → claim-guard scan → claim-guard acquire); it composes with — and sits inside — the issue-level lease you already hold. Set `TARGETS` to the space-separated skill names and/or repo-relative paths the issue's **Files touched** lists.
>    <!-- claim-guard-acquire:begin -->
>    ```bash
>    TARGETS="<space-separated skills/paths from the issue's Files touched>"
>    bash ${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/worktree-guard.sh assert || exit $?
>    bash ${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/claim-guard.sh scan $TARGETS || true
>    if ! AUTOSPEC_CLAIM_GUARD=strict bash ${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/claim-guard.sh acquire $TARGETS; then
>      gh issue comment <ISSUE> --body "claim-guard acquire conflict (see code_health:claim_conflict identifier above); another live session owns this edit surface — restoring auto-implement"
>      gh issue edit <ISSUE> --remove-label in-progress-by-bot --add-label auto-implement
>      exit 6
>    fi
>    ```
>    <!-- claim-guard-acquire:end -->
>    `scan` is advisory (it warns on overlapping open PRs / worktrees / live claims but never blocks); `acquire` is all-or-nothing and exits `6` on `claim_conflict`. Hold this lease across the whole edit+test+PR step. **Refresh rides the existing heartbeat tick** — at each heartbeat write also run `bash ${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/claim-guard.sh refresh` (no new loop) so a slow-but-live editor is never reclaimed. **Release on PR open**: immediately after `gh pr create` succeeds, run `bash ${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/claim-guard.sh release $TARGETS` (the lease has served its purpose once the work is captured in a PR). `AUTOSPEC_CLAIM_GUARD=off` or an unwritable store degrades the whole block to a no-op and NEVER blocks the issue.
> 2. TDD per AGENTS.md: failing test first → implement → refactor → commit. NO DB/external mocks. Follow file paths and signatures from the issue body verbatim.
> 3. **Full test suite gate.** Run the target repo's full validation/test suite, not only the Primary smoke test. Command resolution order:
>    1. If `AUTOSPEC_FULL_TEST_COMMAND` is set, run `bash -lc "$AUTOSPEC_FULL_TEST_COMMAND"`.
>    2. Else run every command listed under the issue's **Operator/full verification** section.
>    3. Else run the repo-standard full suite: `bash scripts/validate.sh` when present; otherwise use the ecosystem default (`npm test`, `pytest`, `go test ./...`, `cargo test`, `mvn test`, etc.).
>    If the full suite fails, fix the failure, recommit, rerun the full suite, and repeat. Do NOT dispatch LGTM review while the full suite is failing. Do NOT run `gh pr merge` while the full suite is failing. Record the exact full-suite command and passing output summary in the PR comment or final report.
>    Once the suite first passes, fire the transition notification: `case "$_notify_fired" in *:tests_passed:*) ;; *) _notify_fired="${_notify_fired}:tests_passed:"; bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/notify.sh" "autospec #<ISSUE>: tests_passed" "Full suite green on {repo}" || true ;; esac`
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
> Run after autospec-test gate, before LGTM review. Skip if issue body contains a line matching `^docs:\s*skip\s*$` (case-insensitive). On `drift`/`missing_scope`/`example_stale` the classifier emits the pinned `regenerate` action carrying the affected scopes; the gate self-heals by invoking `/autospec-doc` (via `doc-orchestrator.mjs`) scoped to ONLY those scopes, re-verifying the regenerated pages with `verify-examples.mjs`, and committing `docs: regenerate <scopes>` onto the SAME PR branch. **Doc generation NEVER blocks the code PR** — generation/verification failure only warns, applies the `docs:failed` label, and comments; the code review loop continues. Only the regenerate commit's own example verification gates the regenerated docs (failing pages are NOT committed):
>    ```bash
>    if ! grep -qiE '^docs:[[:space:]]*skip[[:space:]]*$' "/tmp/issue-<ISSUE>-body.md" 2>/dev/null; then
>      SCRIPTS_DIR="${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}"
>      DOC_SCRIPTS_DIR="${AUTOSPEC_DOC_SCRIPTS_DIR:-$HOME/.autospec/skills/autospec-doc/scripts}"
>      DRIFT_JSON="$(bash "$SCRIPTS_DIR/check-doc-drift.sh" --pr "<PR>" 2>/tmp/drift-<PR>.err)"; drift_exit=$?
>      if [ "$drift_exit" = "0" ]; then
>        : # no drift — continue to LGTM
>      else
>        # drift_exit 1 (drift/example_stale) or 2 (missing_scope): classify, then
>        # self-heal in-PR when the classifier emits the `regenerate` action.
>        VERDICT_JSON="$(printf '%s' "$DRIFT_JSON" | node "$SCRIPTS_DIR/loop-classifier-docs-extension.mjs" --drift-json - --issue "<ISSUE>" --pr "<PR>" 2>/dev/null || true)"
>        ACTION="$(printf '%s' "$VERDICT_JSON" | node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>{try{process.stdout.write((JSON.parse(s).action)||"")}catch{}})' 2>/dev/null || true)"
>        if [ "$ACTION" = "regenerate" ]; then
>          # Scopes the classifier flagged — always extract for labelling/reporting.
>          mapfile -t SCOPES < <(printf '%s' "$VERDICT_JSON" | node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>{try{for(const x of (JSON.parse(s).scopes||[]))console.log(x)}catch{}})' 2>/dev/null || true)
>          gh issue edit <ISSUE> --add-label "docs:drift" 2>/dev/null || true
>          # Resolve the auto_regenerate switch (D2 gate conditional).
>          # Reads config from .autospec/autospec.yml, issue body from single-fetch file, and
>          # AUTOSPEC_WITH_DOCS env. Precedence: docs:skip (already handled above) >
>          # docs:generate > config auto_regenerate=true > AUTOSPEC_WITH_DOCS=1 > default-off.
>          _REGEN=$(node --input-type=module <<'__REGEN_EOF__' 2>/dev/null || echo "0"
>          import { resolveAutoRegenerate, loadConfig } from '${AUTOSPEC_DOC_SCRIPTS_DIR:-$HOME/.autospec/skills/autospec-doc/scripts}/doc-config.mjs';
>          import fs from 'node:fs';
>          const cfg = (() => { try { return loadConfig('.autospec/autospec.yml'); } catch { return {}; } })();
>          const body = (() => { try { return fs.readFileSync('/tmp/issue-<ISSUE>-body.md','utf8'); } catch { return ''; } })();
>          const flag = process.env.AUTOSPEC_WITH_DOCS === '1';
>          const { generate } = resolveAutoRegenerate({ config: cfg, issueBody: body, withDocsFlag: flag });
>          process.stdout.write(generate ? '1' : '0');
>          __REGEN_EOF__
>          )
>          if [ "${_REGEN:-0}" = "1" ]; then
>            # auto_regenerate is ON — run the regenerate self-heal path.
>            doc_ok=1
>            if node "$DOC_SCRIPTS_DIR/doc-orchestrator.mjs" 2>/tmp/docgen-<PR>.err; then
>              # Re-verify regenerated pages; only verified pages are committed.
>              if [ "${#SCOPES[@]}" -gt 0 ] && node "$DOC_SCRIPTS_DIR/verify-examples.mjs" "${SCOPES[@]}" 2>/tmp/docverify-<PR>.err; then
>                if ! git diff --quiet -- "${SCOPES[@]}" 2>/dev/null; then
>                  git add -- "${SCOPES[@]}"
>                  git commit -m "docs: regenerate ${SCOPES[*]}" || doc_ok=0
>                  git push || doc_ok=0
>                fi
>              else
>                doc_ok=0  # example verification failed — do NOT commit regenerated docs
>              fi
>            else
>              doc_ok=0    # generation failed
>            fi
>            if [ "$doc_ok" = "0" ]; then
>              gh issue edit <ISSUE> --add-label "docs:failed" 2>/dev/null || true
>              gh pr comment <PR> --body "$(printf 'docs: regenerate self-heal failed (generation or example verification) — code PR NOT blocked. Scopes: %s' "${SCOPES[*]:-<none>}")" 2>/dev/null || true
>            else
>              gh pr comment <PR> --body "$(printf 'docs: regenerated %s in-PR (examples re-verified).' "${SCOPES[*]:-<none>}")" 2>/dev/null || true
>            fi
>          else
>            # auto_regenerate is OFF — log and continue; detection/labels already applied.
>            echo "docs: regeneration skipped (auto_regenerate=false)"
>          fi
>        else
>          # No regenerate verdict — surface drift for operator review; do not block.
>          gh pr comment <PR> --body "$(printf 'docs drift detected — no self-heal action emitted:\n\n```json\n%s\n```' "$DRIFT_JSON")" 2>/dev/null || true
>          gh issue edit <ISSUE> --add-label "docs:drift" 2>/dev/null || true
>          if [ "$drift_exit" = "2" ]; then gh issue edit <ISSUE> --add-label "docs:missing-scope" 2>/dev/null || true; fi
>        fi
>      fi
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
>    # Stop-sentinel: abort if an immediate stop flag is present after this step.
>    if ! bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-stop-check.sh" "$ISSUE" "$BRANCH" "$LAST_STEP"; then
>      exit 0
>    fi
>    ```
> 6. PR: gh pr create --base main --head <BRANCH> --title "<TITLE>" --body "Closes #<ISSUE>\n\n<summary>". Capture PR. Immediately after the PR opens, release the claim-guard lease taken in step 1a: `bash ${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/claim-guard.sh release $TARGETS`.
>    Fire the transition notification: `case "$_notify_fired" in *:pr_created:*) ;; *) _notify_fired="${_notify_fired}:pr_created:"; bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/notify.sh" "autospec #<ISSUE>: pr_created" "PR #<PR> opened on {repo}" || true ;; esac`
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
>    # Stop-sentinel: abort if an immediate stop flag is present after this step.
>    if ! bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-stop-check.sh" "$ISSUE" "$BRANCH" "$LAST_STEP"; then
>      exit 0
>    fi
>    ```
>    - Run the **Primary smoke test** from the issue body. If it fails, fix and recommit before review.
>    - Run the **Full test suite gate** before fused guardian + LGTM review. If it fails, fix and recommit, rerun the full suite, and do not dispatch review until the full suite passes.
>    - **Fused guardian + LGTM review** (one subagent does both — saves one dispatch per inner-loop iteration):
>      <!-- guardian-block:begin -->
>      Run deterministic lint first (no subagent cost):
>        rm -f /tmp/guardian-<PR>.md
>        if [ "${AUTOSPEC_NO_GUARDIAN:-0}" != "1" ]; then
>          bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/lint-implementation.sh" <PR> --issue <ISSUE> >> /tmp/guardian-<PR>.md 2>&1
>        fi
>        det_exit=$?
>
>      **Model tier:** `TIER_B` for ALL issues — including `regression` and `priority:high`. The single fused reviewer carries the regression gap-check (see brief below), so no second Tier-A pass is dispatched. **Escape hatch:** `AUTOSPEC_REVIEWER_TIER` overrides the reviewer tier — unset (or any value other than `opus`) → `TIER_B` (sonnet); set `AUTOSPEC_REVIEWER_TIER=opus` to restore `TIER_A` for the reviewer. Silently fall back to `TIER_A` if `TIER_B` is unavailable.
>
>      **Assemble reviewer prompt** — call `gen-reviewer-prompt.sh` to compose the combined prompt (static cached prefix + dynamic suffix):
>      ```bash
>      # Reuse the single body fetch from process(ISSUE) start (D5).
>      # SHA-gated diff re-fetch: only re-fetch the PR diff when the branch head changed.
>      _current_head=$(gh pr view <PR> --json headRefOid --jq .headRefOid 2>/dev/null || echo "")
>      if [ -z "${_reviewer_pr_diff_file:-}" ] || [ "${_reviewer_last_head:-}" != "$_current_head" ]; then
>        _reviewer_pr_diff_file=$(mktemp -t autospec-pr-diff-XXXXXX.diff)
>        gh pr diff <PR> > "$_reviewer_pr_diff_file"
>        _reviewer_last_head="$_current_head"
>      fi
>      combined_reviewer_prompt=$(bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/gen-reviewer-prompt.sh" \
>        --pr-diff "$_reviewer_pr_diff_file" \
>        --issue-body "/tmp/issue-<ISSUE>-body.md" \
>        --prev-findings "/tmp/guardian-<PR>.md" \
>        --issue-labels "<ISSUE_LABELS>" \
>        --repo "<REPO>" \
>        ${_reuse_flags_file:+--reuse-flags "$_reuse_flags_file"})
>      ```
>      Pass `combined_reviewer_prompt` as the reviewer subagent prompt. The static cached prefix is framed by `<!-- CACHE BOUNDARY -->` markers; pass it with `cache_control: { type: "ephemeral" }` so Anthropic's prompt cache can reuse it across inner-loop iterations.
>
>      Dispatch ONE **foreground subagent** with this brief:
>        > You are the implementation reviewer for PR #<PR> on {repo}, closing issue #<ISSUE>.
>        >
>        > ## Review counter-team as review lens
>        >
>        > Read the issue body's **Review counter-team** section before reviewing.
>        > Review from that independent team's perspective and challenge likely blind spots
>        > from the implementation team's **Team personality**. Stay inside the issue
>        > scope: do not request unrelated rewrites, but do raise findings when the PR
>        > misses risks the counter-team was selected to notice.
>        >
>        > **Part 1 — Guardian (contract compliance)** — skip if `AUTOSPEC_NO_GUARDIAN=1`:
>        > 1. Read AGENTS.md `## Implementation-quality contract` for the RULE_ID table and directive map.
>        > 2. Read issue #<ISSUE> body — note `## Implementation scope`, `## Implementation outline`, `## Tests required`, and any `Guardian: skip-*` lines.
>        > 3. Read deterministic findings in /tmp/guardian-<PR>.md (populated by lint-implementation.sh; may be empty if guardian disabled).
>        > 4. Run `gh pr diff <PR>` and `gh pr view <PR> --json files,title,body`.
>        > 5. Apply LLM-tier RULE_IDs (HALLUCINATED_API, DUPLICATE_CODE, STRING_MATCH_DOMAIN_LOGIC, REPEATED_STRUCTURE_AS_CODE, DOC_OUT_OF_SYNC semantic pass, INVENTED_CONFIG). For DUPLICATE_CODE, explicitly search for existing components, helpers, validators, API clients, request wrappers, error banners, fixtures, and test utilities before accepting new generated code. For STRING_MATCH_DOMAIN_LOGIC, flag any function that encodes domain meaning via substring-on-name checks when the file already imports a proper-representation library (Python rdkit/ast/urllib.parse/datetime/ipaddress/lxml/jsonschema; JS/TS URL/Date/AST/zod/ajv/joi; Go net/url/time/go/ast/net.ParseIP; Java java.net.URI/java.time/JavaParser/javax.validation; Scala java.net.URI/java.time/scalameta/refined/circe; Rust url::Url/chrono/time/syn/std::net::IpAddr/serde). For REPEATED_STRUCTURE_AS_CODE, flag any function whose body has ≥5 branches sharing identical structural shape (Python if/elif, Java/Scala switch/match, Rust match arms, Go switch cases, JS if/else) — direct the implementer to extract into a table + single dispatcher loop. For each new component, module, function, endpoint, worker, hook, or test helper, require a spec-linked reason, reuse check, public contract, proof test, and "what breaks if wrong" answer. Collect as `RULE_ID:<path>:<line>: <desc>`. Honor `Guardian: skip-*` with `INFO:` lines.
>        >
>        > **Part 2 — LGTM (correctness review):** Using the same diff and issue body already in context:
>        > 6. Check correctness, edge cases, missing tests, AGENTS.md compliance (TDD, no mocks, conventional commits), whether every new code unit exists for a concrete issue/spec requirement rather than convenience, and whether deprecated routes, caches, buckets, stores, workers, config keys, UI paths, docs, specs, tests, or fixtures were removed instead of revived to make tests pass.
>        > 7. Collect findings as a numbered list.
>        > 8. Critical self-question before LGTM: "What else could still pass here while the real user workflow fails, and how could this be better?" Check especially mocked-vs-deployed behavior, external service assumptions, fallback paths, user-visible outcomes, and missing no-mock smoke coverage. If the answer is actionable inside the issue scope, emit it as a finding or required test.
>        > 9. **Regression gap-check (MANDATORY for `regression`/`priority:high` issues; skip otherwise):** ask "would the reviewer have caught the original gap?" If the fused review as written would NOT have caught the gap this regression closes, add the missing checklist item(s) to `reports/autospec-review/reviewer-lessons.md` (one entry per item, with parent `gap_id` and date) and apply those new checks to this diff before issuing the verdict. This folds the former second-pass regression meta-review into this single reviewer pass — the reviewer-lessons write-path is preserved here; there is no second Tier-A dispatch.
>        >
>        > **Hard limit:** max **25 tool calls total** (Parts 1 + 2 combined). If budget exhausted, append `RULE_ID:OUT_OF_SCOPE: reviewer budget exhausted; PR needs human review` and proceed to verdict.
>        >
>        > **Verdict:** If Part 1 has ZERO blocking findings (INFO lines OK) AND Part 2 has no findings: return ONLY the token: `LGTM`. Otherwise return a numbered findings list — RULE_ID findings first, then LGTM findings.
>
>      If `LGTM` && det_exit == 0:
>        gh pr comment <PR> --body "<!-- guardian-block --> Review: clean. <!-- /-->"
>        run **Full test suite gate** and record the exact full-suite command and passing output summary
>        bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/ci-wait.sh" <PR>  # fire-and-forget sentinel
>        if [ -f ".autospec/tokens-<ISSUE>-reviewer.json" ]; then
>          bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/record-telemetry.sh" \
>            --dispatch-id "<DISPATCH_ID>-reviewer" --role reviewer --issue "<ISSUE>" \
>            --tokens-json ".autospec/tokens-<ISSUE>-reviewer.json"
>        fi
>        # Reuse-lens decision ledger (issue #1442): when AUTOSPEC_REUSE_LENS=1,
>        # record the fused-review verdict so precision/demotion can be computed.
>        if [ "${AUTOSPEC_REUSE_LENS:-}" = "1" ]; then
>          bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/interrogation-ledger.sh" record \
>            --issue "<ISSUE>" --pr "<PR>" --trigger "<TRIGGER>" --verdict BLOCK --upheld true \
>          || true  # write failure is best-effort; never blocks the PR
>        fi
>        # monitor exits to parking state HERE — orchestrator relaunches when ~/.autospec/ci-state/<PR>.signal settles
>        # On relaunch: run ci-wait-poll.sh <PR>; break SUCCESS if exit 0 (pass)
>        break SUCCESS only if the full suite passed and required checks pass.
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
>    - **Regression coverage** for `regression`/`priority:high` issues is handled inside the single fused reviewer brief (Part 2 item 9 above): the reviewer self-asks "would the reviewer have caught the original gap?", writes any missing checks to `reports/autospec-review/reviewer-lessons.md`, and applies them before its verdict. No second Tier-A dispatch.
>    - If LGTM: break SUCCESS.
>    ```bash
>    # Stop-sentinel: abort if an immediate stop flag is present after this step.
>    if ! bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-stop-check.sh" "$ISSUE" "$BRANCH" "$LAST_STEP"; then
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
>    # Mandatory final local proof after the branch is current with main.
>    # Run the **Full test suite gate** here using the same command resolution order
>    # (`AUTOSPEC_FULL_TEST_COMMAND`, Operator/full verification, then `bash scripts/validate.sh` fallback).
>    # If it fails, fix the failure, recommit, push, rerun the full suite and review, and do NOT run `gh pr merge`.
>    gh pr merge <PR> --admin --squash --delete-branch
>    case "$_notify_fired" in *:merged:*) ;; *) _notify_fired="${_notify_fired}:merged:"; bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/notify.sh" "autospec #<ISSUE>: merged" "PR #<PR> merged on {repo}" || true ;; esac
>    ```
>    The block ends with the admin-merge; merge auto-closes the issue.
>    ```bash
>    # Stop-sentinel: abort if an immediate stop flag is present after this step.
>    if ! bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-stop-check.sh" "$ISSUE" "$BRANCH" "$LAST_STEP"; then
>      exit 0
>    fi
>    ```
>    <!-- token-report:begin -->
>    Post the per-issue token report (best-effort; never fails the run):
>    ```bash
>    # Orchestrator writes .autospec/tokens-<ISSUE>.json from Agent-result usage
>    # (harness-dependent, best-effort; absent fields → null, never blocking).
>    bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/post-token-report.sh" \
>      --issue "<ISSUE>" --repo "<REPO>" \
>      --tokens-json ".autospec/tokens-<ISSUE>.json" || true
>    ```
>    <!-- token-report:end -->
>    # Cleanup single-fetch body temp file on terminal success (D5).
>    rm -f "/tmp/issue-<ISSUE>-body.md" || true
> 9. FAILURE (loop exhausted): comment failure on issue, swap label `in-progress-by-bot` → `auto-implement`, `gh pr close <PR> --delete-branch`.
>    Fire the terminal failure notification: `case "$_notify_fired" in *:failed:*) ;; *) _notify_fired="${_notify_fired}:failed:"; bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/notify.sh" "autospec #<ISSUE>: failed" "Implementation failed on {repo}" || true ;; esac`
>    Cleanup single-fetch body temp file on terminal failure: `rm -f "/tmp/issue-<ISSUE>-body.md" || true`
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

### Running concurrent workers

autospec-run is designed for concurrent operation across separate harness sessions (e.g., two Claude Code sessions in different terminals, or two Codex CLI processes). Each session runs an independent monitor. Three layers enforce safety:

1. **Atomic claim** — `claim-issue.sh` check-and-swaps `auto-implement → in-progress-by-bot` plus writes an `autospec-run-state` GitHub comment. The **GitHub comment is the cross-machine source of truth**, not the local heartbeat.
2. **Worktree isolation** — each issue gets its own `/tmp/wt-<branch>` so two workers never share a worktree.
3. **Per-session lock** — the session-lock (see above) ensures a single harness session never runs two concurrent monitors; separate sessions run independently by design.

To launch a second concurrent worker:

```bash
# Terminal 1 — first worker (already running)
AUTOSPEC_WORKER_ID="$(hostname):user:shell:$$" /autospec-run

# Terminal 2 — second worker (fresh session, distinct worker id)
AUTOSPEC_WORKER_ID="$(hostname):user:shell:$$" /autospec-run
```

Each session derives its own `AUTOSPEC_WORKER_ID` if not overridden; the default form is `host:user:harness:pid`. Set it explicitly when two sessions run on the same host to guarantee uniqueness.

**Watchdog tuning for long concurrent runs.** The watchdog cross-checks the GitHub run-state comment before releasing any `claimed` heartbeat, so a live sibling's claim is never reclaimed. The tunable env vars (set in every concurrent terminal):

| Variable | Default | Purpose |
|---|---|---|
| `AUTOSPEC_WATCHDOG_CLAIMED_TIMEOUT_SECS` | `1800` | Minimum age before a `claimed` heartbeat triggers the GitHub cross-check. Set higher (e.g., `3600`) for hosts with slow worktree setup. |
| `AUTOSPEC_WATCHDOG_RECLAIM_SECS` | `10800` | Age after which any non-`claimed` stale heartbeat is reclaimed. |
| `AUTOSPEC_WATCHDOG_STALE_SECS` | `1800` | Age at which a heartbeat is considered stale for nudging. |

If a cross-check GitHub API call fails (offline / rate-limited), the watchdog treats the claim as live and skips the reclaim — fail-safe by design.

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

When the monitor terminates, challenge whether the run is really done before
posting the final summary. The **Done challenge** must explicitly re-check:

- Queue state: no ready `auto-implement`, `gap-remediation`, or
  `needs-classify` issues remain under the active profile.
- Phase 5.5 result: either 0 surviving gaps, or a named warning explaining why
  post-review could not run and what risk remains.
- Failures/deferred work: every failure is either fixed, restored to the queue,
  or listed as next work; every deferred issue is listed in the Deferred summary.
- Verification evidence: merged PRs passed the required local suite and required
  CI checks before admin-merge.

If the Done challenge finds unfinished work that can be handled safely, do not
claim completion; relaunch Phase 4/Phase 5.5 or file the missing follow-up and
drain it. Only emit the final report after the challenge says the run is really
done or after the remaining work is explicitly captured as next work.

Post a final summary to the user with:

- Work done: every issue processed, every PR merged, total elapsed wall time,
  and any failures that need human attention.
- Archived summary: stale trackers, superseded specs, closed follow-ups, or
  other archived items from this run; use `(none)` when nothing was archived.
- Done challenge: the challenge bullets and final verdict.
- Next work: what should be worked on next, or `- (none — converged)` when the
  challenge and Phase 5.5 found no remaining work.

Also write `.autospec/run-summary.md` via the canonical helper so downstream
tools can harvest the same answer:

```bash
bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-write-run-summary.sh" \
  --repo "{repo}" \
  --prs "$PRS_MERGED_FILE_OR_CSV" \
  --issues "$ISSUES_CLOSED_FILE_OR_CSV" \
  --failures "$FAILURES_FILE_OR_CSV" \
  --archived "$ARCHIVED_FILE_OR_CSV" \
  --elapsed "$ELAPSED_HHMM" \
  --done-challenge-file "$DONE_CHALLENGE_FILE" \
  --next-steps-file "$NEXT_STEPS_FILE" \
  --output ".autospec/run-summary.md"
```

Append the **Deferred summary** when `deferred[]` is non-empty:

```
Deferred (off-profile under <profile_name>: ctx=<P.ctx>, reasoning=<P.reasoning>):
- #<N1>: <reason>
- #<N2>: <reason>
...
Re-run with --profile <larger> to pick these up, or run /autospec-run on a host that fits the larger profile.
```

If `deferred[]` is empty, omit the section.

## Phase 5.5 — End-of-run gap remediation

Runs after the last issue in the batch closes/merges (queue drains, `ALL_DONE`), before the final report. Replaces the old report-only post-batch audit: it broad-reviews the shipped work, files surviving gaps as `auto-implement` issues, and re-runs the monitor to close them — bounded by a round cap.

**Skip the whole phase when:**

- `~/.autospec/no-review.flag` exists, OR
- `--no-postreview` was passed to autospec-run.

Otherwise run the bounded loop:

```bash
BATCH_START_DATE="$(bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/run-batch-start.sh" --read)"
RUN_ID="$(date -u +%Y%m%dT%H%MZ)-$(git rev-parse --short HEAD)"
GAPS_FILE="$HOME/.autospec/gaps-${RUN_ID}.json"
MAX="${AUTOSPEC_GAP_MAX_ROUNDS:-2}"
rm -f "$HOME/.autospec/gap-round-state.json"   # fresh window per run
```

Loop (`round = 1 … MAX`):

1. **Broad review** (Tier A): run the broad-review pass with a round-scoped `--since` window (spec Phase 2 child E):

   ```bash
   if [ "${round}" -ge 2 ] && [ -n "${GAP_ROUND_SINCE:-}" ]; then
     REVIEW_SINCE="${GAP_ROUND_SINCE}"   # round ≥2: only the gap PRs filed by the prior round
   else
     REVIEW_SINCE="${BATCH_START_DATE}"  # round 1: full batch window
   fi
   /autospec-review --remediation --since "${REVIEW_SINCE}" --emit-gaps "${GAPS_FILE}"
   ```

   **Round 1** keeps the full `BATCH_START_DATE` batch window — this preserves the per-PR-LGTM-misses-integration value (the broad pass catches cross-PR integration gaps the per-PR LGTMs missed). **Round ≥2** scopes `--since` to ONLY the newly-filed gap PRs from the prior round (`GAP_ROUND_SINCE`, the timestamp captured just before the prior round's gap PRs began merging — see step 4), so the re-review doesn't re-scan the entire already-reviewed batch, only the freshly-shipped gap remediations. On review failure, log a warning, treat as 0 survivors, and fall back to the final report (never block run completion).
1b. **Docs-completeness dimension** (spec §D6 row 2 — runs only on round 1, after the broad review emits `${GAPS_FILE}`): the gap-remediation review also audits documentation completeness for the work shipped during this batch window. Every feature shipped in the window (scoped by `run-batch-start.sh --read`) must have a page for every configured audience (the `documentation.audiences` from #917's doc config), and there must be no outstanding `visual_stale` / `example_stale` drift signals (`check-doc-drift.sh`). Run the deterministic helper and **append** its gaps onto the existing `${GAPS_FILE}` so they file, dedupe, and converge through the SAME gap-remediation machinery used in step 2 (do NOT build a parallel loop):

   ```bash
   DOCS_GAPS="$(bash "skills/autospec-run/scripts/docs-completeness-gaps.sh" 2>/tmp/docs-completeness.err)" || DOCS_GAPS='[]'
   # Merge the docs gaps into the broad-review gap array (re-numbered by the driver).
   if [ -s "${GAPS_FILE}" ]; then
     jq -s '.[0] + .[1]' "${GAPS_FILE}" <(printf '%s' "${DOCS_GAPS}") > "${GAPS_FILE}.merged" \
       && mv "${GAPS_FILE}.merged" "${GAPS_FILE}"
   else
     printf '%s' "${DOCS_GAPS}" > "${GAPS_FILE}"
   fi
   ```

   The emitted gaps carry `dimension: "docs-completeness"`; the gap-remediation loop labels them `gap-remediation` like every other survivor, so a later round does not re-flag freshly-fixed work. Failures of the docs check itself (missing config, drift-script error, missing `node`/`jq`) only log a WARN to `/tmp/docs-completeness.err` and emit an empty array — this dimension NEVER blocks run completion (same failure semantics as `/autospec-review` above).
1c. **Security dimension** (autospec-secaudit sweep — runs on round 1, after the docs dimension merges into `${GAPS_FILE}`): run a repo-wide security sweep (full-tree `--tree --root .`) for security, secret-leak, injection, and PII issues, and **append surviving must-fix findings** to the same `${GAPS_FILE}` so they file, dedupe, and converge through the SAME gap-remediation machinery used in step 2 (do NOT build a parallel loop). Skip the dimension when `~/.autospec/no-secaudit.flag` exists. This shares its engine with `/autospec-secaudit`.

   ```bash
   if [ ! -f "$HOME/.autospec/no-secaudit.flag" ]; then
     SECSCAN="${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/security-scan.sh"
     if [ -f "$SECSCAN" ]; then
       # Auto-file only must-fix findings, and exclude `license` (copyleft/IP
       # needs human confirmation — surfaced via a manual /autospec-secaudit
       # report, not auto-filed as an implementable issue).
       SEC_GAPS="$(bash "$SECSCAN" --tree --root . 2>/tmp/secaudit-sweep.err \
         | jq -sc '[ .[] | select(.severity=="must-fix" and .dimension != "license") ]' 2>/dev/null || printf '[]')"
       if [ -s "${GAPS_FILE}" ]; then
         jq -s '.[0] + .[1]' "${GAPS_FILE}" <(printf '%s' "${SEC_GAPS}") > "${GAPS_FILE}.secmerged" \
           && mv "${GAPS_FILE}.secmerged" "${GAPS_FILE}"
       else
         printf '%s' "${SEC_GAPS}" > "${GAPS_FILE}"
       fi
     fi
   fi
   ```

   The emitted gaps carry security dimensions (`secrets` / `vuln` / `injection` / `pii` / `cve`); the gap-remediation loop labels them `gap-remediation` like every other survivor, so a later round does not re-flag freshly-fixed work. A missing scan engine, missing scanners, or `jq` error only logs to `/tmp/secaudit-sweep.err` and emits nothing — this dimension NEVER blocks run completion (same failure semantics as the docs dimension above). For deeper coverage (LLM triage of PII / prompt-injection, plus copyleft/IP review and the `.autospec/secaudit.md` report), run `/autospec-secaudit` manually.
1d. **Fab-completeness dimension** (spec §Phase 5.5 — runs only on round 1, after the security dimension merges into `${GAPS_FILE}`; runs only for a fab run, i.e. when `.autospec/fab.yml` exists): assert every printable model shipped its proof artifacts. For each printable model, `fab-completeness.sh` asserts (a) its 16-view contact sheet exists (`.autospec/fab/renders/<model>/contact-sheet.html`), (b) its `release-gate.json` exists (`.autospec/fab/gates/<model>/release-gate.json`), is GREEN (no stage `status=fail`), and is FRESH (its `geometry_hash` equals the model's current STL sha256 — never re-run stages to learn status). Each failed assertion prints one `GAP <model>: <reason>` line. Convert surviving GAP lines into gap objects carrying `dimension: "fab-completeness"` and **append** them onto `${GAPS_FILE}` so they file, dedupe, and converge through the SAME gap-remediation machinery used in step 2 (do NOT build a parallel loop):

   ```bash
   if [ -f ".autospec/fab.yml" ]; then
     FAB_GAPS="$(bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/fab-completeness.sh" \
       --fab-dir .autospec/fab --stl-dir build/stls 2>/tmp/fab-completeness.err \
       | jq -Rsc 'split("\n") | map(select(length>0)) | to_entries | map({
           gap_id: ("F" + ((.key + 1) | tostring)),
           dimension: "fab-completeness", severity: "high",
           file: ".autospec/fab/release-gate.json", line: 1,
           title: ("fab-completeness: " + .value),
           body: ("Phase 5.5 fab-completeness found: " + .value + ". Every printable model must ship a 16-view contact sheet and a green + fresh release-gate.json. Re-run the fab gate (stl-release-gate.py) and regenerate the missing artifact."),
           dedupe_key: ("fab-completeness-" + (.value | gsub("[^a-zA-Z0-9]+"; "-")))
         })' 2>/dev/null || printf '[]')"
     if [ -s "${GAPS_FILE}" ]; then
       jq -s '.[0] + .[1]' "${GAPS_FILE}" <(printf '%s' "${FAB_GAPS}") > "${GAPS_FILE}.fabmerged" \
         && mv "${GAPS_FILE}.fabmerged" "${GAPS_FILE}"
     else
       printf '%s' "${FAB_GAPS}" > "${GAPS_FILE}"
     fi
   fi
   ```

   The emitted gaps carry `dimension: "fab-completeness"`; the gap-remediation loop labels them `gap-remediation` like every other survivor, so a later round does not re-flag freshly-fixed work. A non-fab run (no `.autospec/fab.yml`), a missing helper, or a `jq` error only logs to `/tmp/fab-completeness.err` and emits nothing — this dimension NEVER blocks run completion (same failure semantics as the docs dimension above).
2. **File survivors:**

   ```bash
   bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/gap-remediation-loop.sh" \
     --gaps "${GAPS_FILE}" --file
   ```

   The driver prints `gap-remediation: survivors=<N> filed=<N> round=<N>`. Capture `<N>` survivors.
3. **Converge:** if `survivors == 0`, break — the run is clean.
4. **Drain:** otherwise capture the gap-window watermark, then re-enter the Phase 4 background monitor (opus, `batch=1`) and run it until the freshly-filed `gap-remediation` issues drain (same monitor batch-exit discipline as the main run). `gap-remediation`-labelled issues are recognizable so a later round does not re-flag freshly-fixed work. Capture the watermark BEFORE the gap PRs merge so the next round's broad review (step 1) scopes `--since` to only these gap PRs (spec Phase 2 child E):

   ```bash
   GAP_ROUND_SINCE="$(date -u +%Y-%m-%dT%H:%M:%SZ)"   # next round scopes --since to gap PRs filed here onward
   ```
5. Increment `round`. The driver also enforces `AUTOSPEC_GAP_MAX_ROUNDS` internally (it refuses to file once the round-state hits the cap), so the loop never spins.

**Termination guarantees:** convergence (0 survivors) ends the loop immediately; the `dedupe_key` prevents re-filing the same gap across rounds; the hard cap `AUTOSPEC_GAP_MAX_ROUNDS` (default 2) stops the loop and surfaces any remainder to the operator.

**Feed Phase 6:** report (a) gaps closed this phase, (b) findings the filter suppressed, and (c) gaps still open after the cap. Failures from `/autospec-review` or the driver log a warning but do NOT fail the overall run.

## Constraints (apply throughout)

- **Cadence**: sub-hour polling needs an in-session background subagent or a local cron. Cloud cron services (e.g. Anthropic remote routines) typically have a 1-hour minimum and are not appropriate.
- **TDD non-negotiable** per AGENTS.md. Real services in tests, no DB mocks.
- **Branch-per-issue**, conventional commits, no force-push, no hook bypass.
- **Auto-merge** on success per AGENTS.md. Do not ask.
- **Context budget**: stay under 60%. Delegate everything possible.
- **Failure isolation**: a failed issue restores its `auto-implement` label so the next monitor cycle picks it up; it does not block the cascade unless dependent issues are downstream.
- **Fresh repo**: if Phase 0 created the repo, the very first commit is the scaffold (incl. `AGENTS.md`); the spec lands as the second commit; child branches branch off that `main`.
- **Small-LLM target**: child issues are sized and pre-staged for 32B-class local LLMs (e.g. qwen3-32B / Qwen3-30B-A3B on Ollama, 48 GB-class Macs). Bias toward smaller issues, sectional spec anchors, pre-staged file pointers, checkbox AC, and one Primary smoke test.

## Autonomous mode

`/autospec-run` is already autonomous (no operator gates during normal
operation). When invoked with `--autonomous`, or when
`~/.autospec/autonomous.flag` exists, the monitor:

- Tags run-state telemetry with `autonomous=true` so Phase 5.5 gap
  remediation and Phase 6 final reports can distinguish autonomous runs
  from interactive ones.
- Honors the same safety guardrails as `/autospec` and `/autospec-define`:
  destructive remote actions, out-of-scope file changes, and cost gate
  (`AUTOSPEC_AUTONOMOUS_ISSUE_CAP`, `AUTOSPEC_AUTONOMOUS_TOKEN_CAP`) still
  surface confirmations via `autospec-autonomy-gate.sh --check all`,
  exit 1 = ask anyway.
- Does NOT add any additional user-facing gates. The flag is informational
  here — the run loop's existing "only ask on hard blocker" rule already
  matches autonomous semantics.

Pass `--autonomous` from `/autospec-define`'s handoff (or set it
explicitly) to preserve cross-phase telemetry continuity. Existing
`feedback_autospec_autonomy_scope.md` rules remain in force.
