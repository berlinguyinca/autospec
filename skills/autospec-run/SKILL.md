---
name: autospec-run
description: Use when the user has already run /autospec-define (or otherwise has a populated set of auto-implement GitHub issues) and wants the implementation half — Phases 4-6 — to run autonomously with admin auto-merge. Supports --profile <name> filtering against ~/.autospec/model-profiles.yml.
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
  `autospec queue ready` and exit without claiming.
- `--claim <issue>` — attempt a deterministic claim via `autospec claim acquire`
  and exit with that command's status (`0` claimed, `2` already claimed/skipped).
- `--release <issue>` — release a distributed claim via `autospec claim release`.

### Auto-init `~/.autospec/model-profiles.yml`

If the file is missing on run start:

**Dispatching to a local model.** A profile resolved to a local model is executed via
`bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/local-dispatch.sh" --model <tag>
--prompt-file <path>`, never by calling a runtime directly. It refuses (exit 3) when
Codex CLI is absent or does not advertise `--oss`, when the capability probe reports the
model is not `dispatch_recommended`, or when no wall-clock bound can be applied — on any
refusal, keep the cloud tier. The local GPU is capacity-1, so the helper serializes
dispatches; never run two local dispatches concurrently.

1. **Discover local supply with the probe — never enumerate models yourself.**
   ```bash
   bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/discover-model-supply.sh" --profiles
   ```
   Paste its `profiles:` fragment into the file verbatim. The probe measures each
   model's real context length via `ollama show`, keeps only completion-capable
   models, writes the original runtime tag as `model:`, and normalizes the profile
   key (`qwen3:32b` → `qwen3-32b-laptop`).

   Do **not** list models from memory, from the model names you happen to know, or
   from an inferred parameter count. An earlier revision of this section asked the
   orchestrator to do exactly that, and on a live host it produced nine local
   profiles for four real models — including an unrunnable `qwen3-coder-480b` — and
   rated every one of them `ctx: 64k` when the measured values were 131072 and
   262144. Enumerating ground truth is the probe's job.

   Read these fields off the probe rather than guessing:
   - `accelerator.usable` / `accelerator.reason` — a present `nvidia-smi` and a
     present `/dev/nvidia0` do **not** mean a usable GPU. When `usable` is false,
     surface `reason` to the operator (e.g. `nvml_driver_library_mismatch` is a
     five-minute fix) instead of silently accepting CPU inference.
   - `dispatch_recommended` per model — false means the probe found the model but
     the host cannot run it usefully (no accelerator, or weights exceed the memory
     budget). Leave those profiles commented out; the probe already does this.
   - A model set that comes back empty (no `ollama`, daemon down, or zero
     completion models) means **no local profiles**. Never synthesize one.
2. If `ANTHROPIC_API_KEY` is set in the environment, append two cloud profiles:
   `claude-sonnet-cloud` and `claude-opus-cloud`, both `ctx: 120k`,
   `reasoning: deep`, each with its `model:` id. Also append `claude-haiku-cloud`
   (`ctx: 64k`, `reasoning: medium`) — `select-model-profile.sh` routes
   `reasoning:shallow|medium` issues to it, so omitting it silently disables the
   cheap implementer tier.
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
    model: qwen3:32b   # original runtime tag — what a dispatch sends
    ctx: 64k           # one of: 32k | 64k | 120k
    reasoning: medium  # one of: shallow | medium | deep
  claude-haiku-cloud:
    model: claude-haiku-4-5
    ctx: 64k
    reasoning: medium
  claude-sonnet-cloud:
    model: claude-sonnet-4-6
    ctx: 120k
    reasoning: deep
```

### Profile-filter ordinals

- `ctx: 32k < 64k < 120k`
- `reasoning: shallow < medium < deep`

A profile P "fits" issue I when `I.ctx_label ≤ P.ctx` AND `I.reasoning_label ≤ P.reasoning` on these ordinals.

---

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
| Local model dispatch        | not native — `Agent(model:)` takes Claude tiers only; shell out to `local-dispatch.sh` | provider pointed at `:11434/v1` (or `:8000` for vLLM) | `codex exec --oss --local-provider ollama` | cloud tier (fail closed) |
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

**Fallback rule:** If `TIER_B` is not available in your harness (model unknown, quota/capacity failure, authorization failure, or tool call returns an error for that model), silently retry the same subagent dispatch with `TIER_A`. Preserve the parent context on retry by passing a bounded handoff containing the issue number, repo path, branch/worktree plan, relevant issue body sections, last error, and current queue/claim state. Codex native subagents with explicit `agent_type`, `model`, or `reasoning_effort` MUST use a bounded handoff, not a full-history fork; do not ask Codex to inherit/fork the full parent conversation when those fields are set. Never ask the user.

**Codex SpawnAgent call-shape contract:** Codex has two valid subagent call shapes. Use the bounded handoff when setting an explicit executor/tier route: `SpawnAgent({ prompt: bounded_handoff, agent_type: "executor", model: TIER_B, reasoning_effort: "medium" })`. Use a full-history fork only when inheriting the current conversation without explicit routing: `SpawnAgent({ prompt: full_history_prompt, full_history: true })`. Never combine `full_history: true` with `agent_type`, `model`, or `reasoning_effort`. On Codex dispatch failure, retry once with the other valid shape when that still satisfies the phase's tier rule; if both valid shapes fail, release the claimed issue back to `auto-implement` with a visible blocker comment.

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

> **Auto-merge authority for auto-implement PRs.** Admin-merge auto-implement PRs (`gh pr merge <#> --admin --squash --delete-branch`) when (a) the full target-repo validation/test suite has passed locally after the branch is current with `main`, (b) all **non-advisory** required CI checks pass — checks matching `AUTOSPEC_PR_ADVISORY_CHECKS` (default `AUTOSPEC_MAIN_HEALTH_IGNORE_CHECKS`; e.g. self-hosted TeamCity) are advisory and may be pending **or failing** once the full local suite is green, (c) the self-review subagent returned `LGTM`, (d) PR closes an `auto-implement` issue from a `feat/*` branch.

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

**Codex Wait call-shape contract:** When awaiting a native Codex `wait_agent` call, omit `timeout_ms` or pass an integer greater than or equal to `10000`. Never pass `timeout_ms` below `10000`; the native router rejects it before the executor can report progress. Keep waiting through task notifications until the monitor reaches a terminal state.

```
batch_num=1
while true:
  launch background subagent paused before its first claim (pass batch_num; AUTOSPEC_BATCH_SIZE=${AUTOSPEC_BATCH_SIZE:-1})
  capture the returned ACTUAL_SESSION_ID and bind it into that child's durable session context as WAIT_TARGET_SESSION_ID
  do not allow the child to claim work until it can pass that exact WAIT_TARGET_SESSION_ID to `autospec claim acquire --session-id`
  wait for task-notification (monitor agent completes)

  if Wait returns `write_stdin failed` with `stdin is closed`:
    inspect durable agent/session state once for a live recovery path
    if the child is reported live:
      attempt one harness reattach to the exact failed Wait target
      if reattach succeeds: continue waiting for task-notification
      explicitly terminate and reap the child through the harness process API
      if termination and reap cannot be proven: stop without typed recovery or label mutation
    require durable proof that the child exited or was terminated and reaped
    use the actual session ID from the failed Wait target (never infer it from an environment variable)
    read the immutable heartbeat binding for ACTUAL_SESSION_ID before inspecting any current claim state
    BOUND_HEARTBEAT="$(bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/heartbeat-read.sh" --repo {repo} --session-id "<ACTUAL_SESSION_ID>")" or stop without typed recovery or label mutation
    ISSUE="$(printf '%s' "$BOUND_HEARTBEAT" | jq -er '.issue | select((type == "string" and length > 0) or (type == "number" and . > 0)) | tostring')" or stop
    BRANCH="$(printf '%s' "$BOUND_HEARTBEAT" | jq -er '.branch | select(type == "string")')" or stop
    WORKER_ID="$(printf '%s' "$BOUND_HEARTBEAT" | jq -er '.worker_id | select(type == "string" and length > 0)')" or stop
    CLAIM_ID="$(printf '%s' "$BOUND_HEARTBEAT" | jq -er '.claim_id | select(type == "string" and length > 0)')" or stop
    if the binding is absent, malformed, or legacy/unbound: stop without typed recovery or label mutation
    never read CLAIM_ID from the currently active claim after Wait fails; a successor may already own the same issue, worker, and branch
    run `"${AUTOSPEC_BIN:-autospec}" autonomous implementer-wait-failed --repo {repo} --issue "<ISSUE>" --worker-id "<WORKER_ID>" --branch "<BRANCH>" --claim-id "<CLAIM_ID>" --session-id "<ACTUAL_SESSION_ID>" --diagnostic "<REDACTED_WAIT_ERROR>"`
    if typed recovery exits non-zero: log the surfaced recovery failure and stop processing that issue
    never mutate labels inline or overwrite a successor claim
    continue  # start a fresh monitor batch from the restored auto-implement queue

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

>   # Backlog grooming preflight — run exactly one existing-backlog grooming
>   # cycle before each queue scan when grooming policy is auto/on. The helper
>   # invokes autonomous-promote-open-issues.sh --apply, but mutations are still
>   # protected by the orchestrator's double gate: --apply AND grooming policy
>   # in {auto,on}. This is no discovery: do not run Tier 2-4 discovery and do
>   # not file new issues from this preflight.
>   GROOM_PREFLIGHT="${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/run-groom-preflight.sh"
>   [ -x "$GROOM_PREFLIGHT" ] || GROOM_PREFLIGHT="skills/autospec-run/scripts/run-groom-preflight.sh"
>   if [ -f "$GROOM_PREFLIGHT" ]; then
>     bash "$GROOM_PREFLIGHT" \
>       --repo "{repo}" \
>       --report "${AUTOSPEC_RUN_REPORT:-$HOME/.autospec/autospec-run-report.md}" || true
>   else
>     echo "WARN: backlog grooming preflight helper missing; continuing drain"
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
- `GATE=growth` — the issue carries `growth:artifact`. Keep the **standard
  implementer**, and add one **content-quality gate** to Phase 4 before the
  standard reviewer + `growth-ethics` + `autospec-secaudit` gates: run
  `${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/growth-content-quality-precheck.sh`
  on the changed content (deterministic pre-checks: keyword-density ceiling,
  FTC-disclosure presence, citation presence), then a `TIER_A` reviewer for
  E-E-A-T / brand-voice, wrapped in the standard 5-attempt adaptive-retry loop
  that feeds findings back as directives. A failing gate blocks merge
  (fail-closed); it never ships unreviewed growth content. This makes the gate
  fire for every path that reaches a `growth:artifact` issue — the autonomous
  Tier-1 drain and `/autospec-grow-run` R1 alike.
- `GATE=default` — every other issue keeps the standard implementer + gate.

The branch is the only difference; the fab branch's gate prose lives in
`prompts/phase4-implementer.md`. Routing is a pure label decision (no substring
match: `area:fabric` stays `default`), so it is deterministic and testable.

### Distributed coordinator selection

Before choosing `ready[0]`, resolve the Rust control-plane binary:

```bash
AUTOSPEC_QUEUE_BIN="${AUTOSPEC_QUEUE_BIN:-${AUTOSPEC_BIN:-autospec}}"
```

If `--coordination-status` is active, run `autospec queue ready --repo {repo}
--batch-size "${AUTOSPEC_BATCH_SIZE:-1}"`, print the JSON, and exit. If
`--max-parallel-safe` is active, print only the `.batch` array and exit.

During the normal monitor loop:

> **Issue intent safety claim gate.** `autospec claim acquire` reads the current
> labels and issue body, validates the reviewed Safety review block, performs the
> label transition, and confirms the lowest-ID run-state comment. The monitor
> never reimplements that safety or lease transition with `gh issue edit`.

0. Reconcile parents whose children may have closed outside autospec before
   selecting work:
   ```bash
   _parent_slug=$(bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/repo-slug.sh" --canonical "{repo}")
   export AUTOSPEC_PARENT_STATE_ROOT="${AUTOSPEC_PARENT_STATE_ROOT:-$HOME/.autospec/parent-state/$_parent_slug}"
   if ! "${AUTOSPEC_BIN:-autospec}" parent sweep --repo {repo}; then
     echo "[monitor] WARN: parent sweep failed; remote parent state is unknown and will be retried" >&2
   fi
   ```
1. Run `autospec queue ready --repo {repo} --batch-size "$effective_batch_size"`
   after watchdog reconciliation and profile filtering.
2. Use `.ready[0].number` as the next issue candidate.
3. Claim it through `autospec claim acquire --issue "$ISSUE" --repo {repo}
   --worker-id "${AUTOSPEC_WORKER_ID:-<derived>}" --branch "<BRANCH>"`.
4. Treat claim exit `2` as a normal lost-race or conflict outcome: refresh the
   queue and try another candidate without failing the batch.
5. On failure, stop, or retry exhaustion, call `autospec claim release` before
   returning `auto-implement` to the queue.

### Final safety-stamp contract

`autospec queue review-safety` is the only automatic writer of issue-intent
safety outcomes. After an admission surface persists its final issue body and
adds interim `auto-implement`, it must invoke the exact target:

```bash
autospec queue review-safety --repo {repo} --limit 1 --issue <N>
```

Only the command's `pass: 1` total admits that invocation. The ready queue and
claim command consume Rust's typed passing evidence and refuse a missing,
ambiguous, or blocking result. They never reconstruct, stamp, or replace a
safety decision with `gh` commands or `lint issue safety` output.

The GitHub `autospec-run-state` comment written by the Rust control plane is the
cross-workstation source of truth. Local process heartbeat files remain useful
for same-host progress and compatibility, but they are not authoritative across
machines. If the Rust command is unavailable, fail the monitor start visibly;
do not fall back to an inline label-swap path.

>   all_open = [open auto-implement issues, sorted ascending by issue number]
>   all_open excludes every issue carrying `autospec:blocked-prerequisite`, even
>   if a stale or manual edit also left `auto-implement` attached.
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
>   # Security prerequisite pre-dispatch gate. Re-read the live body and labels
>   # before claim/dispatch so a post-classification edit cannot bypass the
>   # portfolio gate. A security child is one containing any of the generated
>   # Evidence consumed / Controls covered / Prerequisites headings. Its
>   # Prerequisites section must be absent, exactly `none`, or contain only
>   # bullet lines whose content begins `verified:`. Anything else is removed
>   # from the ready queue and marked visibly; never dispatch it.
>   _security_body="$(gh issue view "$ISSUE" --repo {repo} --json body --jq .body)"
>   if printf '%s\n' "$_security_body" | grep -qE '^## (Evidence consumed|Controls covered|Prerequisites)[[:space:]]*$'; then
>     _prerequisites="$(printf '%s\n' "$_security_body" | awk '
>       /^## Prerequisites[[:space:]]*$/ {inside=1; next}
>       /^## / && inside {exit}
>       inside && NF {sub(/^[[:space:]]*-[[:space:]]*/, ""); print}
>     ')"
>     if [ -n "$_prerequisites" ] && [ "$_prerequisites" != "none" ] \
>       && printf '%s\n' "$_prerequisites" | grep -qv '^verified:'; then
>       echo "code_health:security_prerequisite_blocked issue=$ISSUE" >&2
>       gh issue comment "$ISSUE" --repo {repo} --body \
>         "code_health:security_prerequisite_blocked — every security prerequisite must be verified before dispatch."
>       gh issue edit "$ISSUE" --repo {repo} \
>         --remove-label auto-implement --add-label autospec:blocked-prerequisite
>       ready = ready without ISSUE
>       continue
>     fi
>   fi
>   # Atomic claim: autospec claim acquire is the SOLE claim path. It performs the
>   # check-and-swap (auto-implement -> in-progress-by-bot) atomically with a
>   # read-back verification, so the hot loop NEVER re-implements the inline
>   # label swap. Multiple monitors can race the same candidate; exactly one
>   # wins (exit 0), the rest lose (non-zero) and skip to the next candidate.
>   claim_json="$("$AUTOSPEC_CLAIM_BIN" claim acquire --issue "$ISSUE" --repo {repo} --worker-id "${AUTOSPEC_WORKER_ID:-$(hostname):${USER:-unknown}:monitor:$$}" --branch "${BRANCH:-}" --session-id "${WAIT_TARGET_SESSION_ID:?missing durable Wait target session binding}")" && claim_rc=0 || claim_rc=$?
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
>     echo "[monitor] WARN: claim hard error for #$ISSUE (rc=$claim_rc) — usage/config error, NOT a lost race; skipping. Check autospec claim acquire invocation." >&2
>     READY_REMOVE=$(printf "%s\n%s" "$READY_REMOVE" "$ISSUE" | grep -v "^${ISSUE}$" || true)
>     ready=($READY_REMOVE)
>     continue
>   fi
>   # exit 0 only: this monitor now owns #$ISSUE. claim acquire has already
>   # persisted both the per-issue heartbeat and immutable session sidecar.
>   CLAIM_ID="$(printf '%s' "$claim_json" | jq -er '.claim_id | select(type == "string" and length > 0)')" || exit 1
>   CLAIM_WORKER_ID="$(printf '%s' "$claim_json" | jq -er '.worker_id | select(type == "string" and length > 0)')" || exit 1
>   bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/heartbeat-read.sh" --repo "{repo}" --session-id "$WAIT_TARGET_SESSION_ID" >/dev/null || exit 1
>   bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/notify.sh" "autospec #$ISSUE: claimed" "Starting implementation on {repo}" || true
>   # Issue start summary — print before dispatching process(ISSUE) so the operator
>   # knows exactly what the monitor is about to work on.
>   # Single API call — body+title+url+labels in ONE `gh issue view` (D5: duplicate-read elimination).
>   # issue-snapshot.sh wraps that one call and caches the JSON per issue; later steps
>   # (start summary, implementer/reviewer prompt assembly) reuse the file instead of
>   # re-fetching. --refresh: labels may have transitioned since the previous run.
>   _issue_snapshot_file="$(bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/issue-snapshot.sh" get "$ISSUE" --refresh 2>/dev/null || true)"
>   _issue_body_file="/tmp/issue-${ISSUE}-body.md"
>   if [ -n "${_issue_snapshot_file:-}" ] && [ -f "${_issue_snapshot_file:-}" ]; then
>     jq -r '.body // ""' "$_issue_snapshot_file" > "$_issue_body_file"
>     ISSUE_TITLE=$(jq -r '.title // ""' "$_issue_snapshot_file")
>     ISSUE_URL=$(jq -r '.url // ""' "$_issue_snapshot_file")
>     ISSUE_LABELS=$(jq -r '[.labels[]?.name] | join(", ")' "$_issue_snapshot_file")
>   else
>     # fetch failed — fail open with empty metadata (same end state as a gh outage)
>     : > "$_issue_body_file"; ISSUE_TITLE=""; ISSUE_URL=""; ISSUE_LABELS=""
>   fi
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
> - **If `labels` contains `autospec:v2-flow`** — load the prompt template from `skills/autospec-run/prompts/phase4-implementer.md` (relative to this skill's install location, or via `AUTOSPEC_SKILLS_DIR`/the harness's skill mount). That prompt embeds the absorbed-discipline path: expand → implement → finalize → peer-review (via `codex exec`) → evaluate-findings. **Wire the D3 cached prefix (spec Phase 2 child C):** do NOT send the `phase4-implementer.md` body alone — prepend the `gen-implementer-prompt.sh` static cached prefix so the default v2 path stops re-reading SKILL.md + AGENTS.md + the RULE_ID table uncached. Assemble the combined v2 prompt by handing `phase4-implementer.md` to `--body-file`, which forwards it to `bundle-static-context.sh --static-body` so it lands INSIDE the cached prefix. It is one fixed template, so emitting it below the boundary re-sent it uncached on every dispatch and every retry:
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
>   UI/client-interaction issues handled by that prompt must record exactly one
>   browser verification state in the PR body's `## Validation` section:
>   `browser-verified`, `fallback-smoke-only`, or `not-run`. Harness-caused
>   Browser connector skips that produce `fallback-smoke-only` or `not-run` must
>   open or link a browser remediation issue with the redacted error detail before
>   merge.
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
> 3. **Resolve the implementer model (deterministic — do this before dispatching)** — the issue's
>    `reasoning:*` label decides which profile implements it, and the profile carries the concrete
>    model id. Ask the selector rather than deciding yourself:
>    ```bash
>    _impl_model=$(bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/select-model-profile.sh" \
>      --labels "<ISSUE_LABELS>" \
>      --print-model) || _impl_model=""
>    ```
>    If `_impl_model` is **non-empty**, dispatch the implementer subagent with that model in place of
>    the harness-detected `TIER_B`. If the selector exits 3 (empty output — no `model:` key is
>    resolvable, which is the case for an auto-initialised `~/.autospec/model-profiles.yml`), keep
>    `TIER_B` exactly as detected. Never invent a model id, and never widen this override:
>    - It applies to the **implementer dispatch only**. The reviewer keeps its own tier — a reviewer
>      must never run on a weaker model than the implementer it is checking.
>    - The fall-back-**up** rule still holds: if the resolved model is unavailable at dispatch time
>      (quota, capacity, authorization), silently retry the same dispatch at `TIER_B`, then `TIER_A`.
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
> - Recovery-bound schema: `{"issue":"<ISSUE>","branch":"<BRANCH>","step":"<STEP>","ts":<unix_epoch>,"pr":"<PR>","repo":"{repo}","host":"<HOST>","worker_id":"<WORKER_ID>","claim_id":"<CLAIM_ID>","session_id":"<WAIT_TARGET_SESSION_ID>"}`. `heartbeat-write.sh` creates the exact-session sidecar once: an identical identity refresh preserves it, while any session/issue/worker/branch/claim mismatch fails closed before updating liveness. Legacy heartbeats without that sidecar remain valid only for liveness and fail closed for Wait recovery.
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
>    bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/heartbeat-write.sh" \
>      --issue "<ISSUE>" --branch "${BRANCH:-}" --step expand_start --repo "{repo}" \
>      --worker-id "$CLAIM_WORKER_ID" --claim-id "$CLAIM_ID" --session-id "$WAIT_TARGET_SESSION_ID"
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
>    # Reset repo-local state roots after entering the issue worktree. Parent
>    # autospec-run processes may export AUTOSPEC_REPO_DIR for the primary
>    # checkout; premerge/validation helpers must read mutable artifacts such as
>    # .autospec/qa-verdict.json from this active worktree instead.
>    export AUTOSPEC_REPO_DIR="$PWD"
>    # MANDATORY assert gate: MUST exit 0 before the first file edit/commit. A
>    # non-zero exit (in_primary_checkout / dirty / stale_base / wrong_branch) is NEVER worked
>    # around — comment the emitted code_health identifier on the issue, restore
>    # the `auto-implement` label (swap `in-progress-by-bot` → `auto-implement`),
>    # remove the heartbeat, and stop this issue.
>    if ! bash ${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/worktree-guard.sh assert --expected-branch <BRANCH> --branch-pattern 'feat/*'; then
>      gh issue comment <ISSUE> --body "worktree-guard assert failed (see code_health identifier above); restoring auto-implement"
>      gh issue edit <ISSUE> --remove-label in-progress-by-bot --add-label auto-implement
>      exit 1
>    fi
>    ```
>    <!-- worktree-ladder:end -->
>    On the `open-pr` path the verification bar EQUALS fresh work — full tests + the standard review loop, never a blind merge. Cleanup is identical for every path: after the merge is confirmed (or on terminal failure), `git worktree remove` the linked worktree and `git worktree prune`; never delete un-pushed work before merge.
<!-- autospec-block:runtime-resource-preflight -->
> 1b. **Claim the edit surface (claim-guard), nested inside the issue claim.** After the `worktree-guard.sh assert` gate passes and BEFORE the first file edit, take a fine-grained lease on the skill(s)/paths this issue will touch so a concurrent session in another worktree cannot stomp the same trio+golden surface. This is the inner layer of the three-layer caller pattern (worktree-guard → claim-guard scan → claim-guard acquire); it composes with — and sits inside — the issue-level lease you already hold. Set `TARGETS` to the space-separated skill names and/or repo-relative paths the issue's **Files touched** lists.
>    <!-- claim-guard-acquire:begin -->
>    ```bash
>    TARGETS="<space-separated skills/paths from the issue's Files touched>"
>    bash ${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/worktree-guard.sh assert --expected-branch <BRANCH> --branch-pattern 'feat/*' || exit $?
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
>    3. Else run the repo-standard full suite: `autospec validate` when present; otherwise use the ecosystem default (`npm test`, `pytest`, `go test ./...`, `cargo test`, `mvn test`, etc.).
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
> __REGEN_EOF__
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
> 4a. <!-- pr-size-pre-push:begin --> **Deterministic PR_SIZE pre-push gate.**
>    Define this helper in the monitor shell and run it before any remote mutation:
>    ```bash
>    # pr-size-helper:begin
>    run_pr_size_gate() {
>      PR_SIZE_PHASE="$1"
>      PR_SIZE_BASE_OID="$2"
>      PR_SIZE_HEAD_OID="$3"
>      PR_SIZE_DIFF=$(mktemp -t autospec-pr-size-XXXXXX.diff) || return 1
>      git diff --binary "$PR_SIZE_BASE_OID" "$PR_SIZE_HEAD_OID" >"$PR_SIZE_DIFF" || {
>        rm -f "$PR_SIZE_DIFF"
>        return 1
>      }
>      PR_SIZE_RC=0
>      PR_SIZE_OUTPUT=$(bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/lint-implementation.sh" \
>        --diff-file "$PR_SIZE_DIFF" --issue <ISSUE>) || PR_SIZE_RC=$?
>      rm -f "$PR_SIZE_DIFF"
>      printf '%s\n' "$PR_SIZE_OUTPUT"
>      if [ "$PR_SIZE_RC" -ne 0 ] || printf '%s\n' "$PR_SIZE_OUTPUT" | grep -q '^ERROR:PR_SIZE:'; then
>        printf 'ERROR:PR_SIZE: %s rejected exact diff %s..%s\n' \
>          "$PR_SIZE_PHASE" "$PR_SIZE_BASE_OID" "$PR_SIZE_HEAD_OID"
>        return 1
>      fi
>      printf '%s\n' 'INFO:PR_SIZE: acceptance'
>    }
>    # pr-size-helper:end
>    # pr-size-pre-push-exec:begin
>    PR_SIZE_BASE_OID=$(git merge-base "origin/${AUTOSPEC_BASE_BRANCH:-main}" HEAD) || exit 1
>    PR_SIZE_HEAD_OID=$(git rev-parse HEAD) || exit 1
>    PR_SIZE_EVIDENCE=$(run_pr_size_gate pre-push "$PR_SIZE_BASE_OID" "$PR_SIZE_HEAD_OID") || {
>      printf '%s\n' "$PR_SIZE_EVIDENCE"
>      exit 1
>    }
>    printf '%s\n' "$PR_SIZE_EVIDENCE"
>    printf '%s\n' "$PR_SIZE_EVIDENCE" | grep -qxF 'INFO:PR_SIZE: acceptance' || exit 1
>    # pr-size-pre-push-exec:end
>    ```
>    The deterministic linter rejects the first over-limit values: **401 changed lines**,
>    **9 raw files**, or **4 logical units**. On rejection, preserve the branch and do not
>    run `git push`, `gh pr create`, `gh pr ready`, or any merge command.
>    <!-- pr-size-pre-push:end -->
> 5. Push: git push -u origin <BRANCH>
>    ```bash
>    # Stop-sentinel: abort if an immediate stop flag is present after this step.
>    if ! bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-stop-check.sh" "$ISSUE" "$BRANCH" "$LAST_STEP"; then
>      exit 0
>    fi
>    ```
> 6. PR: build the body deterministically instead of writing it out — the closing
>    reference, the change list, and the verification line are all already in the
>    branch, and restating them costs output tokens to reproduce what git holds:
>    ```bash
>    # mktemp, not a fixed /tmp/...-<ISSUE> path: two implementers working the same
>    # issue NUMBER in different repos would otherwise overwrite each other's body.
>    PR_SUMMARY_FILE=$(mktemp -t autospec-pr-summary-XXXXXX) || exit 1
>    PR_BODY_FILE=$(mktemp -t autospec-pr-body-XXXXXX) || exit 1
>    # Write ONLY the summary — the part no template can derive: what this change
>    # does and which alternative it rejected. Everything else is assembled.
>    # A quoted heredoc, because the summary is MULTI-PARAGRAPH markdown: printf
>    # '%s\n' would flatten it to one line, and any literal % in it is a format
>    # string. 'EOF' is quoted so backticks and $ in the prose are not expanded.
>    cat > "$PR_SUMMARY_FILE" <<'PR_SUMMARY_EOF'
>    <summary>
>    PR_SUMMARY_EOF
>    bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/compose-pr-body.sh" \
>      --issue "<ISSUE>" --summary-file "$PR_SUMMARY_FILE" \
>      > "$PR_BODY_FILE" || exit 1
>    ```
>    Exit 3 from the composer means the commit range is empty: do NOT run
>    `gh pr create`, and investigate why the branch has no commits.
>    Then: gh pr create --base main --head <BRANCH> --title "<TITLE>" --body-file "$PR_BODY_FILE". Capture PR. Immediately after the PR opens, release the claim-guard lease taken in step 1a: `bash ${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/claim-guard.sh release $TARGETS`.
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
>      Reuse lens (issues #1439/#1440/#1442): when armed, extract the reuse-triage
>      RULE_IDs from the deterministic findings so the reviewer prompt receives the
>      build-vs-buy block. Flag-OFF or no reuse hits → `_reuse_flags_file` stays empty
>      → the reviewer prompt is byte-identical to today (the `${_reuse_flags_file:+…}`
>      expansion below adds nothing):
>        ```bash
>        _reuse_flags_file=""
>        if [ "${AUTOSPEC_REUSE_LENS:-}" = "1" ] && [ -f /tmp/guardian-<PR>.md ]; then
>          _reuse_candidate=$(mktemp -t autospec-reuse-flags-XXXXXX)
>          grep -E '^(REINVENT_REPO_UTIL|NEW_DEP_UNJUSTIFIED|NEW_ABSTRACTION_SINGLE_CALLER):' \
>            /tmp/guardian-<PR>.md > "$_reuse_candidate" 2>/dev/null || true
>          if [ -s "$_reuse_candidate" ]; then _reuse_flags_file="$_reuse_candidate"; fi
>        fi
>        ```
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
>      <!-- guardian-block:end -->
>        > 8a. **data-scope invariant lens (diagnostic/filter endpoints):** When the issue touches endpoints, dashboards, reports, or diagnostics that accept optional job/sample/filter parameters, verify filters never widen to unrelated records. empty optional filters reject unless documented as a deliberate all-records mode. Require concrete evidence for `job-only`, `sample-only`, `job+sample`, `unsupported-filter`, and `empty-filter` paths; unsupported-filter and empty-filter cases must prove rejection or a documented scoped response, not silent broadening.
>        > 9. **Regression gap-check (MANDATORY for `regression`/`priority:high` issues; skip otherwise):** ask "would the reviewer have caught the original gap?" If the fused review as written would NOT have caught the gap this regression closes, add the missing checklist item(s) to `reports/autospec-review/reviewer-lessons.md` (one entry per item, with parent `gap_id` and date) and apply those new checks to this diff before issuing the verdict. This folds the former second-pass regression meta-review into this single reviewer pass — the reviewer-lessons write-path is preserved here; there is no second Tier-A dispatch.
>        >
>        > **Hard limit:** max **25 tool calls total** (Parts 1 + 2 combined). If budget exhausted, append `RULE_ID:OUT_OF_SCOPE: reviewer budget exhausted; PR needs human review` and proceed to verdict.
>        >
>        > **Simplicity axis is ADVISE-only (anti-gold-plating):** the reuse / build-vs-buy / "how could this be better?" axis may argue only toward *less* code — reuse a named existing util (`scripts/lib/`, repo source), adopt a named library, or delete an unneeded abstraction — and only when tied to a named acceptance criterion. It may NEVER emit a `BLOCK` that demands *more* code, a new abstraction, or speculative generality; such suggestions are at most `ADVISE` and never halt the commit. Every reuse verdict must name the matched util or library (evidence-bound), never assert a match from belief.
>        >
>        > **Verdict:** If Part 1 has ZERO blocking findings (INFO lines OK) AND Part 2 has no findings: return ONLY the token: `LGTM`. Otherwise return a numbered findings list — RULE_ID findings first, then LGTM findings. A reuse `BLOCK` is provisional until it survives the refute pass below.
>
>      **Reuse-BLOCK refute pass (before consuming the verdict):** If the findings list contains a build-vs-buy / reuse `BLOCK`, do NOT halt on it yet. Dispatch a **cheap refute pass** — one short `TIER_B` second voter (≤5 tool calls) whose only job is to *kill* the BLOCK: `rg`-search the repo for the named util/library and confirm the claimed reuse target actually exists, is reachable, and fits this call site. **Majority rules:** keep the BLOCK only if the refuter also upholds it; if the refuter refutes it (the named target is absent, unreachable, or ill-fitting), demote that BLOCK to `ADVISE`, drop it from the blocking findings, and continue. If demotion leaves no remaining blocking findings, treat the verdict as `LGTM`. This keeps a hallucinated "library exists" from stalling the merge (`feedback_llm_validator_adaptive_retry`). **Record the outcome of this reuse `BLOCK` decision to the reuse-lens ledger HERE** (issue #1442) — at the decision point, so precision = upheld ÷ total is computed only over real reuse BLOCKs and never from phantom rows on clean passes. Set `_reuse_block_raised=1`, `_reuse_trigger` to the flagged RULE_ID, and `_reuse_upheld=true` when the refuter upheld the BLOCK or `_reuse_upheld=false` when it was refuted/demoted-to-ADVISE, then:
>      **Draw the refuter from a different vendor than the proposer.** Two dispatches to the same model family share failure modes and tend to be wrong together, which is the one case this second vote exists to catch — so resolve the refuter's vendor before dispatching it, passing the harness you detected in step 1 of Phase 0 as `--proposer`:
>        ```bash
>        REFUTE_VENDOR=$(bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/verify-voter-vendor.sh" \
>          --proposer "<HARNESS>") || REFUTE_VENDOR=
>        ```
>        Run the refute pass on `$REFUTE_VENDOR`'s own `TIER_B` — vendor is the independence lever, tier is the quality lever, and this changes only the former. An **empty** `REFUTE_VENDOR` (exit 3: single-harness host, or every alternative already failed over) means keep the same-harness `TIER_B` refuter: a same-vendor second vote is weaker than a cross-vendor one but still better than none, and the script refuses to name the proposer's own vendor rather than report an independence the host cannot provide.
>        **On a quota failure, re-resolve rather than give up.** If the refuter's dispatch fails with a 429 / quota / capacity error, call the script again adding `--unavailable <that vendor>` and dispatch to what it returns; a 429 is the only ground-truth quota signal available, since `usage-observe.sh` reports `observable=false` for all three harnesses. Repeat until it exits 3, then fall back to the same-harness refuter.
>        ```bash
>        if [ "${AUTOSPEC_REUSE_LENS:-}" = "1" ] && [ "${_reuse_block_raised:-0}" = "1" ]; then
>          bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/interrogation-ledger.sh" record \
>            --issue "<ISSUE>" --pr "<PR>" --trigger "${_reuse_trigger:-REINVENT_REPO_UTIL}" \
>            --verdict BLOCK --upheld "${_reuse_upheld:-true}" \
>          || true  # write failure is best-effort; never blocks the PR
>        fi
>        ```
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
>        # Reuse-lens verdict is recorded at the refute-pass decision point above
>        # (issue #1442), not here — recording in this LGTM-only branch produced
>        # phantom BLOCK rows on clean passes and never recorded upheld BLOCKs.
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
>    # Advisory checks (e.g. self-hosted TeamCity) are operator-declared via
>    # AUTOSPEC_PR_ADVISORY_CHECKS, defaulting to the same regex the conductor's
>    # main-health gate already honors (AUTOSPEC_MAIN_HEALTH_IGNORE_CHECKS) —
>    # one shared definition. Unset/empty ("^$") matches no real check name, so
>    # default behavior is unchanged: every FAILURE blocks.
>    adv="${AUTOSPEC_PR_ADVISORY_CHECKS:-${AUTOSPEC_MAIN_HEALTH_IGNORE_CHECKS:-^$}}"
>    wait_for_ci_green() {
>        while :; do
>            rollup=$(gh pr view <PR> --json statusCheckRollup --jq '.statusCheckRollup // []')
>            pending=$(printf '%s' "$rollup" | jq --arg adv "$adv" '[.[] | select((((.name // .context // "") as $n | $n != "" and ($n | test($adv)))) | not) | select(.conclusion == null)] | length')
>            bad=$(printf '%s' "$rollup" | jq --arg adv "$adv" '[.[] | select((((.name // .context // "") as $n | $n != "" and ($n | test($adv)))) | not) | select(.conclusion=="FAILURE" or .conclusion=="CANCELLED" or .conclusion=="TIMED_OUT" or .conclusion=="ACTION_REQUIRED")] | length')
>            total=$(printf '%s' "$rollup" | jq 'length')
>            if [ "$bad" != "0" ]; then
>                # Distinguish inherited base-branch CI rot from branch-caused failures
>                # before blocking. Capture the PR head rollup and the current base
>                # commit's check/status contexts as merge evidence, then let
>                # ci-status-compare.sh emit classification plus blocked_branch and
>                # blocked_inherited arrays for the PR comment/final report.
>                base_sha=$(gh pr view <PR> --json baseRefOid --jq .baseRefOid)
>                head_checks="/tmp/autospec-ci-head-<PR>.json"
>                base_checks="/tmp/autospec-ci-base-<PR>.json"
>                base_check_runs="/tmp/autospec-ci-base-check-runs-<PR>.json"
>                base_statuses="/tmp/autospec-ci-base-statuses-<PR>.json"
>                compare_json="/tmp/autospec-ci-compare-<PR>.json"
>                printf '%s\n' "$rollup" > "$head_checks"
>                gh api "repos/{repo}/commits/$base_sha/check-runs" --paginate \
>                  --jq '[.check_runs[] | {name, conclusion, status, detailsUrl: .details_url}]' \
>                  > "$base_check_runs"
>                gh api "repos/{repo}/commits/$base_sha/status" \
>                  --jq '[.statuses[] | {context, state, targetUrl: .target_url}]' \
>                  > "$base_statuses"
>                jq -s 'add' "$base_check_runs" "$base_statuses" > "$base_checks"
>                bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/ci-status-compare.sh" \
>                  --head "$head_checks" --base "$base_checks" > "$compare_json"
>                classification=$(jq -r '.classification' "$compare_json")
>                target_url=$(jq -r '.target_url // ""' "$compare_json")
>                case "$classification" in
>                  inherited)
>                    gh issue comment <ISSUE> --body "PR #<PR>: required checks are red, but ci-status-compare classified them as inherited base-branch CI rot (blocked_inherited). Merge evidence: ${target_url:-see attached rollup}; compare artifact: \`$compare_json\`. Pausing for operator review instead of marking the branch broken."
>                    exit 1
>                    ;;
>                  branch_caused)
>                    gh issue comment <ISSUE> --body "PR #<PR>: required checks failed and ci-status-compare classified them as branch-caused (blocked_branch). Merge evidence: ${target_url:-see attached rollup}; compare artifact: \`$compare_json\`."
>                    exit 1
>                    ;;
>                  *)
>                    gh issue comment <ISSUE> --body "PR #<PR>: required checks looked bad but ci-status-compare returned \`$classification\`; pausing for operator review. Compare artifact: \`$compare_json\`."
>                    exit 1
>                    ;;
>                esac
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
>    # (`AUTOSPEC_FULL_TEST_COMMAND`, Operator/full verification, then `autospec validate` fallback).
>    # If it fails, fix the failure, recommit, push, rerun the full suite and review, and do NOT run `gh pr merge`.
>    #
>    # Final quality gate (pre-merge, fail-closed): after the full suite passes and
>    # before admin-merging, discover repository-specific whole-workspace quality
>    # commands and run each one from the target repo root. Discovery is additive:
>    #   1. If AUTOSPEC_FINAL_QUALITY_COMMAND is set, run it exactly as provided.
>    #   2. If a root Cargo.toml exists and `cargo metadata --no-deps --format-version=1`
>    #      succeeds, this is a Rust workspace; run:
>    #      `cargo clippy --workspace --all-targets -- -D warnings`
>    # Treat every discovered command as required merge evidence. Do NOT run `gh pr merge` while the final quality gate is failing.
>    # On failure, post/comment a
>    # `FINAL_QUALITY_GATE_FAILED` block that includes the command plus one finding
>    # record with `crate`, `file`, `line`, and `rule` fields (use `unknown` only when
>    # the tool output genuinely omits a field), then return to the fix/recommit/retry
>    # loop and rerun the full suite plus final quality gate before merge.
>    if [ -n "${AUTOSPEC_FINAL_QUALITY_COMMAND:-}" ]; then
>      sh -lc "$AUTOSPEC_FINAL_QUALITY_COMMAND" || {
>        gh issue comment <ISSUE> --body "FINAL_QUALITY_GATE_FAILED command=AUTOSPEC_FINAL_QUALITY_COMMAND crate=unknown file=unknown line=unknown rule=unknown"
>        exit 1
>      }
>    fi
>    if [ -f Cargo.toml ]; then
>      if ! command -v cargo >/dev/null 2>&1; then
>        gh issue comment <ISSUE> --body "FINAL_QUALITY_GATE_FAILED command=cargo-metadata crate=unknown file=Cargo.toml line=1 rule=cargo-unavailable"
>        exit 1
>      fi
>      if ! cargo metadata --no-deps --format-version=1 >/tmp/autospec-cargo-metadata.json 2>/tmp/autospec-cargo-metadata.err; then
>        _metadata_err=$(tr '
' ' ' </tmp/autospec-cargo-metadata.err | sed 's/[[:space:]][[:space:]]*/ /g; s/^ //; s/ $//' | cut -c1-500)
>        gh issue comment <ISSUE> --body "FINAL_QUALITY_GATE_FAILED command=cargo-metadata crate=unknown file=Cargo.toml line=1 rule=${_metadata_err:-metadata-failed}"
>        exit 1
>      fi
>      if ! cargo clippy --workspace --all-targets -- -D warnings >/tmp/autospec-final-quality-clippy.log 2>&1; then
>        # Preserve raw clippy output and summarize the first diagnostic in a stable schema for reviewers.
>        _crate=$(jq -r '.packages[0].name // "unknown"' /tmp/autospec-cargo-metadata.json 2>/dev/null || printf 'unknown')
>        _location=$(grep -m1 -E '^[[:space:]]*-->' /tmp/autospec-final-quality-clippy.log | sed -E 's/^[[:space:]]*-->[[:space:]]*//' || true)
>        _file=$(printf '%s' "$_location" | awk -F: '{print $1}')
>        _line=$(printf '%s' "$_location" | awk -F: '{print $2}')
>        _rule=$(grep -m1 -Eo 'clippy::[A-Za-z0-9_]+' /tmp/autospec-final-quality-clippy.log || true)
>        if [ -z "$_rule" ]; then _rule=$(grep -m1 -E 'warning:|error:' /tmp/autospec-final-quality-clippy.log | sed 's/^ *//' | cut -c1-200 || true); fi
>        gh issue comment <ISSUE> --body "FINAL_QUALITY_GATE_FAILED command=cargo-clippy crate=${_crate:-unknown} file=${_file:-unknown} line=${_line:-unknown} rule=${_rule:-unknown}"
>        exit 1
>      fi
>    fi
>    <!-- pr-size-final-merge:begin -->
>    # Query GitHub after update-branch, review, and final local proof. The live PR
>    # endpoints are authoritative; stale local OIDs can never create acceptance.
>    # If the shell boundary discarded the helper, redefine it exactly as in step 4a.
>    # pr-size-final-merge-exec:begin
>    PR_SIZE_REMOTE_OIDS=$(gh pr view <PR> --json baseRefOid,headRefOid \
>      --jq '[.baseRefOid, .headRefOid] | @tsv') || exit 1
>    [ "$(printf '%s\n' "$PR_SIZE_REMOTE_OIDS" | awk -F '\t' \
>      'NF == 2 && $1 != "" && $2 != "" { print "valid" }')" = "valid" ] || exit 1
>    PR_SIZE_BASE_OID=$(printf '%s\n' "$PR_SIZE_REMOTE_OIDS" | cut -f1)
>    PR_SIZE_HEAD_OID=$(printf '%s\n' "$PR_SIZE_REMOTE_OIDS" | cut -f2)
>    git fetch --no-tags origin \
>      "+refs/heads/${AUTOSPEC_BASE_BRANCH:-main}:refs/remotes/origin/${AUTOSPEC_BASE_BRANCH:-main}" \
>      "+refs/heads/<BRANCH>:refs/remotes/origin/<BRANCH>" || exit 1
>    git cat-file -e "${PR_SIZE_BASE_OID}^{commit}" || exit 1
>    git cat-file -e "${PR_SIZE_HEAD_OID}^{commit}" || exit 1
>    [ "$(git rev-parse "origin/${AUTOSPEC_BASE_BRANCH:-main}")" = "$PR_SIZE_BASE_OID" ] || exit 1
>    [ "$(git rev-parse "origin/<BRANCH>")" = "$PR_SIZE_HEAD_OID" ] || exit 1
>    [ "$(git rev-parse HEAD)" = "$PR_SIZE_HEAD_OID" ] || exit 1
>    PR_SIZE_EVIDENCE=$(run_pr_size_gate final-pre-merge \
>      "$PR_SIZE_BASE_OID" "$PR_SIZE_HEAD_OID") || {
>      printf '%s\n' "$PR_SIZE_EVIDENCE"
>      exit 1
>    }
>    printf '%s\n' "$PR_SIZE_EVIDENCE"
>    # Reviewer evidence is accepted only as this complete line; prefixes,
>    # suffixes, summaries, and inferred approval are not acceptance.
>    printf '%s\n' "$PR_SIZE_EVIDENCE" | grep -qxF 'INFO:PR_SIZE: acceptance' || exit 1
>    # pr-size-final-merge-exec:end
>    <!-- pr-size-final-merge:end -->
>    # Blast-radius domain fence at the merge chokepoint (issue #1732). The guarded-merge
>    # wrapper classifies the PR's ACTUAL changed files against the repo's fenced_surfaces
>    # registry and refuses to merge a fenced-surface diff (the wrapper applies the
>    # human-review quarantine label and comments) unless the PR carries the
>    # `autospec:fenced-approved` override label; otherwise
>    # it performs the same admin squash-merge. Call it INSTEAD of a bare `gh pr merge --admin`
>    # so "merge without the fence check" requires deliberately bypassing the wrapper.
>    # exit 0 = merged (allowed/overridden); 1 = quarantined (NOT merged); 2 = fail-closed error.
>    # This replaces the historical bare `gh pr merge <PR> --admin --squash --delete-branch`.
>    # pr-size-guarded-merge-exec:begin
>    run_guarded_pr_size_merge() {
>      bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-guarded-merge.sh" \
>        --pr <PR> --repo {repo} \
>        --merge-args "--admin --squash --delete-branch --match-head-commit $PR_SIZE_HEAD_OID"
>    }
>    # pr-size-guarded-merge-exec:end
>    if run_guarded_pr_size_merge; then
>      :
>    else
>      _gm_rc=$?
>      if [ "$_gm_rc" -eq 1 ]; then
>        "$AUTOSPEC_CLAIM_BIN" claim release --issue "<ISSUE>" --repo {repo} --worker-id "${AUTOSPEC_WORKER_ID:-<derived>}" --state blocked --branch "<BRANCH>" --pr "<PR>" || true
>        echo "[monitor] #<ISSUE> quarantined by blast-radius fence — fenced surface, left for human review; PR NOT merged"
>      else
>        echo "[monitor] #<ISSUE> guarded-merge fail-closed (rc=$_gm_rc) — PR NOT merged; pausing for operator review"
>      fi
>      rm -f "/tmp/issue-<ISSUE>-body.md" || true
>      exit 0
>    fi
>    "$AUTOSPEC_CLAIM_BIN" claim release --issue "<ISSUE>" --repo {repo} \
>      --worker-id "${AUTOSPEC_WORKER_ID:-<derived>}" \
>      --state merged --branch "<BRANCH>" --pr "<PR>" || true
>    _parent_slug=$(bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/repo-slug.sh" --canonical "{repo}")
>    export AUTOSPEC_PARENT_STATE_ROOT="${AUTOSPEC_PARENT_STATE_ROOT:-$HOME/.autospec/parent-state/$_parent_slug}"
>    if ! "$AUTOSPEC_CLAIM_BIN" parent reconcile-child --repo {repo} --child "<ISSUE>"; then
>      gh issue comment "<ISSUE>" --repo {repo} --body "Parent reconciliation failed after merge; remote parent state is unknown and will be retried by the recurring parent sweep."
>      echo "[monitor] WARN: parent reconciliation failed for merged child #<ISSUE>" >&2
>    fi
>    case "$_notify_fired" in *:merged:*) ;; *) _notify_fired="${_notify_fired}:merged:"; bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/notify.sh" "autospec #<ISSUE>: merged" "PR #<PR> merged on {repo}" || true ;; esac
>    ```
>    The block ends with the admin-merge and merged-state claim release; merge auto-closes the issue.
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
> 10. Cleanup: run `autospec runtime env down --repo /tmp/wt-<BRANCH> --mode "${AUTOSPEC_RUNTIME_MODE:-auto}" --purge-maven`; then run `bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-runtime-worktree-cleanup.sh" /tmp/wt-<BRANCH>`; only after both succeed, run `cd / && git -C {repo_root} worktree remove /tmp/wt-<BRANCH> --force`.
> 11. Report: PR number, outcome, one-paragraph summary.
>
> Hard rules: NEVER push to main, force-push, or bypass hooks. Only `autospec parent` may update or close an umbrella issue. gh CLI only.
> ```
>
> Hard rules for the monitor: ONE issue at a time, sequential. Only `autospec parent` may update or close an umbrella issue. On transient gh errors retry once. Do NOT ask the user — auto-merge authority is granted in AGENTS.md.
>
> Final output when shutdown: numbered list of every processed issue with PR # and outcome.

Capture the agent ID / log path for monitoring.

If your harness lacks background delegation: open a separate terminal/tmux pane, run the monitor prompt in a fresh session there, and have it write progress to a logfile that Phase 5 can tail.

### Running concurrent workers

autospec-run is designed for concurrent operation across separate harness sessions (e.g., two Claude Code sessions in different terminals, or two Codex CLI processes). Each session runs an independent monitor. Three layers enforce safety:

1. **Atomic claim** — `autospec claim acquire` check-and-swaps `auto-implement → in-progress-by-bot` plus writes an `autospec-run-state` GitHub comment. The **GitHub comment is the cross-machine source of truth**, not the local heartbeat.
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
| `AUTOSPEC_MAX_CONCURRENT_REPO_WORKERS` | `0` (disabled) | Repo-wide active-worker cap. When active `in-progress-by-bot` issues meet the cap, `autospec queue ready` returns an empty `.batch` while still reporting `.ready`. Use this to throttle one workstation or a cluster. |
| `AUTOSPEC_RUN_ONLY_ISSUES` | unset (unconstrained) | Space-separated issue-number allowlist. When set and non-empty, `autospec queue ready` scopes `.ready`/`.blocked`/`.batch` to only those issue numbers — set by the autonomous conductor's dispatch-time provenance split so the operator and self-originated batches each drain their own subset. Unset or empty keeps the full-queue scan. |
| `AUTOSPEC_CLAIM_LEASE_SECONDS` | `10800` | Cross-machine claim lease TTL written into the GitHub run-state comment and used by `autospec claim acquire` stale-reclaim decisions. |
| `AUTOSPEC_CLAIM_SETTLE_SECONDS` | `0.2` | Short post-upsert readback delay so simultaneous comment creates converge before a worker reports claim success. |
| `AUTOSPEC_CLAIM_CONFIRM_READS` | `5` | Number of settled lowest-lock readbacks required before `autospec claim acquire` reports claim success. |
| `AUTOSPEC_WATCHDOG_CLAIMED_TIMEOUT_SECS` | `1800` | Minimum age before a `claimed` heartbeat triggers the GitHub cross-check. Set higher (e.g., `3600`) for hosts with slow worktree setup. |
| `AUTOSPEC_WATCHDOG_RECLAIM_SECS` | `10800` | Legacy fallback for claim lease age when `AUTOSPEC_CLAIM_LEASE_SECONDS` is unset; also used by watchdog stale cleanup. |
| `AUTOSPEC_WATCHDOG_STALE_SECS` | `1800` | Age at which a heartbeat is considered stale for nudging. |

If a cross-check GitHub API call fails (offline / rate-limited), the watchdog treats the claim as live and skips the reclaim — fail-safe by design.

Recommended starting caps:

| Deployment | `AUTOSPEC_MAX_CONCURRENT_REPO_WORKERS` | `AUTOSPEC_BATCH_SIZE` | `AUTOSPEC_CLAIM_LEASE_SECONDS` |
|---|---:|---:|---:|
| 10 workers | `6` | `1` | `10800` |
| 25 workers | `12` | `1` | `10800` |
| 50 workers | `20` | `1` | `14400` |

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

**Resolving `references/` paths.** The reference pointers below are written relative to the autospec repo root, which is where the validation gate resolves them. Inside a target repo that path does not exist: resolve against this skill's own installed directory instead — `~/.claude/skills/autospec-run/references/`, `$CODEX_HOME/skills/autospec-run/references/`, or `$AUTOSPEC_SKILLS_DIR/autospec-run/references/` — and fall back to the `~/.autospec/repo` checkout. A pointer that will not resolve is a stop-and-report condition, never a skipped step.

## Phase 6 — Final report

**MUST** read `skills/autospec-run/references/end-of-run.md` (section "Phase 6 — Final report") and follow it when the monitor terminates: it defines the Done challenge, the final summary, and the `.autospec/run-summary.md` write via `autospec-write-run-summary.sh`.

## Phase 5.5 — End-of-run gap remediation

**MUST** read `skills/autospec-run/references/end-of-run.md` (section "Phase 5.5 — End-of-run gap remediation") and follow it after the last issue in the batch closes/merges (queue drains, `ALL_DONE`), before the final report: the bounded broad-review → file-survivors → converge loop plus the docs / security / fab-completeness dimensions, and the `autospec-gap-miner.sh` closeout invocation that mines run evidence into `docs/memory/autospec-gap-ledger.md`. Skip the whole phase when `~/.autospec/no-review.flag` exists or `--no-postreview` was passed.

## Phase 5.6 — Repo quality audit

**MUST** read `skills/autospec-run/references/end-of-run.md` (section "Phase 5.6 — Repo quality audit") and run `repo-quality-audit.sh` after Phase 5.5 converges or hits its cap, before Phase 6 writes the final run summary.

## Advisor escalation

**MUST** read `skills/autospec-run/references/end-of-run.md` (section "Advisor escalation") before escalating any bounded hard decision to a TIER_A advisor (protocol, gates, and the `reviewer` gate).

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
  destructive remote actions, out-of-scope file changes, and the token cost gate
  (`AUTOSPEC_AUTONOMOUS_TOKEN_CAP`) still
  surface confirmations via `autospec-autonomy-gate.sh --check all`,
  exit 1 = ask anyway.
- Does NOT add any additional user-facing gates. The flag is informational
  here — the run loop's existing "only ask on hard blocker" rule already
  matches autonomous semantics.

Pass `--autonomous` from `/autospec-define`'s handoff (or set it
explicitly) to preserve cross-phase telemetry continuity. Existing
`feedback_autospec_autonomy_scope.md` rules remain in force.
