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

1. Dispatch to `bash scripts/autospec-stop.sh <args>`.
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
| Subagent model tier         | Tier A: `opus` + `ultrathink`; Tier B: `sonnet` + medium thinking | Tier A: top `task` model + high reasoning; Tier B: smaller-tier `task` + medium reasoning | Tier A: top GPT + `reasoning_effort=high`; Tier B: `gpt-5.1-codex-spark` + `reasoning_effort=medium` | Honor the per-phase tier mapping in AGENTS.md; fall back UP on unavailability |

**Persistent project notes**: write durable preferences to **`AGENTS.md`** in the repo root — this is the de-facto standard recognized by Claude Code (also reads `CLAUDE.md`), OpenCode, and Codex. If your harness has its own private memory (e.g. Claude Code's `~/.claude/.../memory/`), mirror the same content there. Per AGENTS.md, subagent dispatches use a **two-tier policy**: Tier A (top model + extended thinking) for spec work (research, decompose, review/label — not used by this skill); Tier B (cheaper model + medium thinking) for implementation work (Phase 4 implementer + LGTM review). The orchestrator keeps the user's invoked model. Fall back UP the tier on unavailability.

## Phase 4 — Background autonomous monitor

Record this durable preference in `AGENTS.md` (idempotent — skip if already present):

> **Auto-merge authority for auto-implement PRs.** Admin-merge auto-implement PRs (`gh pr merge <#> --admin --squash --delete-branch`) when (a) all required CI checks pass — slow optional checks like TeamCity may be pending and that's acceptable, (b) the self-review subagent returned `LGTM`, (c) PR closes an `auto-implement` issue from a `feat/*` branch.

Then launch a **background subagent** with this prompt verbatim:

> You are the auto-implement monitor for `{repo}`. Process every open `auto-implement` issue autonomously and auto-merge each PR. **You MUST stay running across many iterations. Do NOT exit after one issue.**

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
> **Never exit after processing one issue** — the loop must persist until shutdown (idle timeout, stop.flag, or all issues resolved).

>
> **Profile load (run-start, once).** If `--profile <name>` was passed, look it up in `~/.autospec/model-profiles.yml`; if `<name>` is not a key under `profiles:`, exit non-zero and print the available names. If no flag was passed, load the file's `default:` profile. If the file is missing, run auto-init and exit (per the Invocation section).
>
> **Missing-label warning (run-start, once).** Count open `auto-implement` issues that lack either a `ctx:*` or a `reasoning:*` label. If non-zero, print `WARN: N issues lack model-fit labels (ctx:* / reasoning:*); they will be treated as ctx:64k, reasoning:medium. Run /autospec-classify to backfill.` Exactly once at run start.
>
> Outer loop:
> ```
> deferred = []   # issues skipped because they exceed the active profile
>
> while true:
>   candidates = [open auto-implement issues whose Depends-on deps are all CLOSED, sorted ascending]
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
>   if ready is empty:
>     latest_close = most recent closedAt of any auto-implement issue
>     open_count   = count of open auto-implement issues
>     if open_count == 0 AND latest_close > 2h ago: HARD SHUTDOWN — emit final report (incl. deferred summary, see Phase 6)
>     else: print state ("blocked: N unmet deps" / "deferred: M off-profile" / "drained, waiting 1h idle"), sleep 300, continue
>   # autospec-stop sentinel check — outer loop, top of each iteration
>   if [ -f "$HOME/.autospec/stop.flag" ]; then
>     MODE=$(head -1 "$HOME/.autospec/stop.flag" 2>/dev/null || echo "")
>     TIMESTAMP=$(sed -n '2p' "$HOME/.autospec/stop.flag" 2>/dev/null | awk '{print $1}')
>     AGE_SECS=$(( $(date -u +%s) - $(date -u -j -f '%Y-%m-%dT%H:%M:%SZ' "$TIMESTAMP" +%s 2>/dev/null \
>       || date -u -d "$TIMESTAMP" +%s 2>/dev/null || echo 0) ))
>     if [ "$AGE_SECS" -gt 86400 ]; then
>       echo "WARN: stale stop.flag ($AGE_SECS s old); ignoring" >&2
>     elif [ "$MODE" = "graceful" ] || [ "$MODE" = "immediate" ]; then
>       echo "[monitor] stop signal received: $MODE — exiting"
>       # HARD SHUTDOWN with final report
>       exit 0
>     fi
>   fi
>   # Service watch: check for stuck workers every 12 iterations so monitors don't hang indefinitely.
>   monitor_tick=$((monitor_tick + 1))
>   if [ "$monitor_tick" -ge 12 ]; then
>     monitor_tick=0
>     if [ -d "$HOME/.autospec/process-heartbeats" ]; then
>       # Default reclaim window: 10800s (3h). For local single-threaded workers set
>       # AUTOSPEC_WATCHDOG_RECLAIM_SECS=43200 (12h) before launch.
>       WATCHDOG_RECLAIM_SECS="${AUTOSPEC_WATCHDOG_RECLAIM_SECS:-10800}"
>       WATCHDOG_STALE_SECS="${AUTOSPEC_WATCHDOG_STALE_SECS:-1800}"
>       # Cheap service wake-up pass: use low-cost model only.
>       if command -v bash >/dev/null 2>&1; then
>         # Dispatch one background watchdog helper (cheap model) to iterate stale entries.
>         bash scripts/autospec-watchdog.sh
>       elif command -v pwsh >/dev/null 2>&1; then
>         # Windows fallback: PowerShell helper.
>         pwsh -NoProfile -ExecutionPolicy Bypass -File scripts/autospec-watchdog.ps1
>       elif command -v powershell >/dev/null 2>&1; then
>         # Windows fallback: classic PowerShell fallback.
>         powershell -NoProfile -ExecutionPolicy Bypass -File scripts/autospec-watchdog.ps1
>       else
>         echo "[watchdog] neither bash nor powershell found; skipping service-watch pass."
>       fi
>     fi
>   fi
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
>   process(ISSUE)   # foreground subagent — see template below
>   # NO SLEEP — go straight to the next iteration; the merge may have unblocked another issue
> ```
>
> `process(ISSUE)` dispatches a **foreground subagent** (wait for return) with this prompt:
>
> ```
> **Model tier:** Tier B (implementation work) — cheaper model with medium thinking per AGENTS.md. Claude Code: `sonnet`; Codex: `gpt-5.1-codex-spark`; OpenCode: smaller task tier. Fall back UP on unavailability.
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
> - Create/update `~/.autospec/process-heartbeats/<ISSUE>.json` at each major step:
>   - `claimed`, `worktree_ready`, `tests_started`, `tests_passed`, `pr_created`, `smoke_retry`, `reviewed`, `merged`, `failed`
> - Schema: `{"issue":"<ISSUE>","branch":"<BRANCH>","step":"<STEP>","ts":<unix_epoch>,"pr":"<PR>","repo":"{repo}"}`
> - Delete this file on terminal SUCCESS/FAILURE in both clean and failure paths.
> 
> 1. Worktree off origin/main:
>    cd {repo_root} && git fetch origin
>    git worktree add -b <BRANCH> /tmp/wt-<BRANCH> origin/main && cd /tmp/wt-<BRANCH>
> 2. TDD per AGENTS.md: failing test first → implement → refactor → commit. NO DB/external mocks. Follow file paths and signatures from the issue body verbatim.
> 3. Build + test green (use the project's test runner; for Go: `go build ./... && go test ./... -count=1`; for Node: `npm test`; for Python: `pytest`). 80%+ coverage on changed files.
> 4. Conventional commits (feat:/fix:/test:/docs:/refactor:). NEVER bypass hooks. NEVER amend.
> 5. Push: git push -u origin <BRANCH>
>    ```bash
>    # autospec-stop sentinel check — inside process(ISSUE), after each major step
>    if [ -f "$HOME/.autospec/stop.flag" ] && [ "$(head -1 $HOME/.autospec/stop.flag)" = "immediate" ]; then
>      bash scripts/autospec-stop.sh --abort-current-issue "$ISSUE" "$BRANCH" "$LAST_STEP"
>      exit 0
>    fi
>    ```
> 6. PR: gh pr create --base main --head <BRANCH> --title "<TITLE>" --body "Closes #<ISSUE>\n\n<summary>". Capture PR.
> 7. Inner loop (max 3 iterations):
>    ```bash
>    # autospec-stop sentinel check — inside process(ISSUE), after each major step
>    if [ -f "$HOME/.autospec/stop.flag" ] && [ "$(head -1 $HOME/.autospec/stop.flag)" = "immediate" ]; then
>      bash scripts/autospec-stop.sh --abort-current-issue "$ISSUE" "$BRANCH" "$LAST_STEP"
>      exit 0
>    fi
>    ```
>    - Run the **Primary smoke test** from the issue body. If it fails, fix and recommit before guardian.
>    - **Guardian gate**:
>      <!-- guardian-block:begin -->
>      If `AUTOSPEC_NO_GUARDIAN=1` is set: log `WARN: guardian disabled by AUTOSPEC_NO_GUARDIAN` and skip to LGTM dispatch.
>      Otherwise:
>        rm -f /tmp/guardian-<PR>.md
>        bash scripts/lint-implementation.sh <PR> --issue <ISSUE> >> /tmp/guardian-<PR>.md
>        det_exit=$?
>        Dispatch a **foreground subagent** (Tier A) with this brief verbatim:
>        > **Model tier:** Tier A (spec work) — guardian audit. Claude Code: `opus` + `ultrathink`; Codex: current top GPT + `reasoning_effort=high`; OpenCode: top task tier. Fall back UP on unavailability.
>        >
>        > You are the implementation guardian for PR #<PR> on {repo}, derived from issue #<ISSUE>.
>        >
>        > 1. Read AGENTS.md `## Implementation-quality contract` for the RULE_ID table and directive map.
>        > 2. Read issue #<ISSUE> body — note `## Implementation scope`, `## Implementation outline`, `## Tests required`, and any `Guardian: skip-*` lines.
>        > 3. Read deterministic findings already in /tmp/guardian-<PR>.md.
>        > 4. Run `gh pr diff <PR>` and `gh pr view <PR> --json files,title,body`.
>        > 5. Apply the LLM-tier RULE_IDs (HALLUCINATED_API, DUPLICATE_CODE, DOC_OUT_OF_SYNC semantic pass, INVENTED_CONFIG). Append findings to /tmp/guardian-<PR>.md as `RULE_ID:<path>:<line>: <one-line description>`. Honor `Guardian: skip-*` opt-outs by emitting `INFO:` instead of blocking.
>        > 6. Hard limits: max **20 tool calls**. If you cannot reach a verdict in 20 calls, append `RULE_ID:OUT_OF_SCOPE: guardian budget exhausted; PR needs human review` and exit.
>        > 7. If you appended ZERO blocking findings (only INFO lines OK), return ONLY the token: `GUARDIAN_PASS`. Otherwise return ONLY: `GUARDIAN_FAIL`.
>        If GUARDIAN_PASS && det_exit == 0:
>          gh pr comment <PR> --body "<!-- guardian-block --> Guardian: clean. <!-- /-->"
>          proceed to LGTM dispatch.
>        Else:
>          gh pr comment <PR> --edit-last --body "$(cat <<'GCMT'
>          <!-- guardian-block:begin -->
>          ## Guardian findings (iter <K>/3)
>          $(cat /tmp/guardian-<PR>.md | grep -v '^#' | sed 's/^/- /')
>          *Re-evaluated on every push. Last update: $(date -u +%Y-%m-%dT%H:%M:%SZ).*
>          <!-- guardian-block:end -->
>          GCMT
>          )"
>          Append findings to implementer retry context as:
>          ## Guardian directives — clear before re-push
>          $(cat /tmp/guardian-<PR>.md | grep -v '^INFO:' | grep -v '^#')
>          Continue inner loop (counts toward 3-iter cap).
>        On 3-iter exhaustion with GUARDIAN_FAIL:
>          gh label create guardian-blocked --color e11d21 --force --repo {repo}
>          gh issue edit <ISSUE> --add-label guardian-blocked
>          Append ## Guardian audit (failed) block to issue body.
>          Run existing failure cleanup (comment, swap label, close PR).
>          rm -f /tmp/guardian-<PR>.md
>      <!-- guardian-block:end -->
>    - Dispatch a **foreground subagent** with brief: "**Model tier:** Tier B (implementation work) — cheaper model with medium thinking per AGENTS.md. Claude Code: `sonnet`; Codex: `gpt-5.1-codex-spark`; OpenCode: smaller task tier. Fall back UP on unavailability. You are a critical code reviewer. Review PR #<PR> via `gh pr diff` and `gh pr view`. Check correctness, edge cases, missing tests, AGENTS.md compliance. Output a numbered findings list, OR if none, return ONLY the token: LGTM"
>    - If LGTM: run the **Operator/full verification** commands; sleep 30; `gh pr checks <PR>`. If all required checks pass (slow optional checks pending is OK per AGENTS.md): break SUCCESS.
>    - Else: implement findings, commit, push, continue.
>    ```bash
>    # autospec-stop sentinel check — inside process(ISSUE), after each major step
>    if [ -f "$HOME/.autospec/stop.flag" ] && [ "$(head -1 $HOME/.autospec/stop.flag)" = "immediate" ]; then
>      bash scripts/autospec-stop.sh --abort-current-issue "$ISSUE" "$BRANCH" "$LAST_STEP"
>      exit 0
>    fi
>    ```
> 8. SUCCESS: gh pr merge <PR> --admin --squash --delete-branch. Merge auto-closes the issue.
>    ```bash
>    # autospec-stop sentinel check — inside process(ISSUE), after each major step
>    if [ -f "$HOME/.autospec/stop.flag" ] && [ "$(head -1 $HOME/.autospec/stop.flag)" = "immediate" ]; then
>      bash scripts/autospec-stop.sh --abort-current-issue "$ISSUE" "$BRANCH" "$LAST_STEP"
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


## Constraints (apply throughout)

- **Cadence**: sub-hour polling needs an in-session background subagent or a local cron. Cloud cron services (e.g. Anthropic remote routines) typically have a 1-hour minimum and are not appropriate.
- **TDD non-negotiable** per AGENTS.md. Real services in tests, no DB mocks.
- **Branch-per-issue**, conventional commits, no force-push, no hook bypass.
- **Auto-merge** on success per AGENTS.md. Do not ask.
- **Context budget**: stay under 60%. Delegate everything possible.
- **Failure isolation**: a failed issue restores its `auto-implement` label so the next monitor cycle picks it up; it does not block the cascade unless dependent issues are downstream.
- **Fresh repo**: if Phase 0 created the repo, the very first commit is the scaffold (incl. `AGENTS.md`); the spec lands as the second commit; child branches branch off that `main`.
- **Small-LLM target**: child issues are sized and pre-staged for 32B-class local LLMs (e.g. qwen3-32B / Qwen3-30B-A3B on Ollama, 48 GB-class Macs). Bias toward smaller issues, sectional spec anchors, pre-staged file pointers, checkbox AC, and one Primary smoke test.
