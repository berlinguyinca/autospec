
# autospec-run workflow (harness-neutral)

Take the populated `auto-implement` queue on the current GitHub repo and run the implementation half of the autospec pipeline:
**autonomous monitor → admin-squash-merge each PR → periodic status updates → final report.**

Manage your own context — never exceed 60%. Delegate to subagents whenever your harness supports it; do not investigate, write code, or design directly in the main conversation when a subagent can do it.

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

If the feature-request argument matches the regex `^\s*update\s*$` (case-insensitive, whitespace-padded), this skill enters self-update mode and does not run the normal pipeline:

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

## Invocation

```
/autospec-run [--profile <name>]
```

- `--profile <name>` — filter the candidate queue against `~/.autospec/model-profiles.yml` so only issues whose `ctx:*` and `reasoning:*` labels fit the named profile are picked up. Issues that exceed the profile on either axis are appended to a `deferred[]` list and printed in the run-end summary.
- (no flag) — load `~/.autospec/model-profiles.yml`'s `default:` profile and run with it. If the file is missing, run auto-init (below) then exit so the user can review/edit before re-running.
- `--profile <unknown>` — exit non-zero and print the list of available profile names from `~/.autospec/model-profiles.yml`.

### Auto-init `~/.autospec/model-profiles.yml`

If the file is missing on run start:

1. Probe `ollama list 2>/dev/null` and grep for known local-model name patterns
   (`qwen3`, `llama3`, `qwen2`, `mistral`, `phi3`, `gemma`). For each match, write
   a profile keyed on the model name with conservative defaults:
   `ctx: 64k`, `reasoning: medium`. (`qwen3:32b` → `qwen3-32b-laptop`.)
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
>     if open_count == 0 AND latest_close > 1h ago: HARD SHUTDOWN — emit final report (incl. deferred summary, see Phase 6)
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
>   ISSUE = ready[0]
>   gh label create in-progress-by-bot --color ededed --force
>   gh issue edit ISSUE --remove-label auto-implement --add-label in-progress-by-bot
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
>    - Run the **Primary smoke test** from the issue body. If it fails, fix and recommit before review.
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
