---
description: Use when the user has already run /autospec-define (or otherwise has a populated set of auto-implement GitHub issues) and wants the implementation half — Phases 4-6 — to run autonomously with admin auto-merge. Supports --profile <name> filtering against ~/.autospec/model-profiles.yml.
mode: primary
---

# autospec-run workflow (harness-neutral)

Take the populated `auto-implement` queue on the current GitHub repo and run the implementation half of the autospec pipeline:
**autonomous monitor → admin-squash-merge each PR → periodic status updates → final report.**

Manage your own context — never exceed 60%. Delegate to subagents whenever your harness supports it; do not investigate, write code, or design directly in the main conversation when a subagent can do it.

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

- `--profile <name>` — filter the candidate queue against `~/.autospec/model-profiles.yml` so only issues whose `ctx:*` and `reasoning:*` labels fit the named profile are picked up. Issues that exceed the profile on either axis are appended to a `Deferred` list and reported at run-end.
- (no flag) — run with the file's `default:` profile if present; otherwise print `Profile filter inactive — running all auto-implement issues.` and continue without filtering.
- If `~/.autospec/model-profiles.yml` is missing, the skill prints `Profile filter inactive — running all auto-implement issues.` and continues. (Auto-init of the file lands in PR B2; this skill plumbs the flag with no filter behaviour yet.)
- `--profile <unknown>` — exit non-zero and print the available profile names.

The actual filter logic, deferred-summary accumulation, and `model-profiles.yml` auto-init arrive in PR B2 (issue #15). This issue ships the skill structure with the flag plumbed but no filter yet.


## Required capabilities & harness adapter

This workflow assumes five capabilities. Map each one to your harness's actual tool. If a capability is missing, use the listed fallback.

| Capability                  | Claude Code                          | OpenCode                                 | Codex CLI                                | Fallback if missing                                |
|-----------------------------|--------------------------------------|------------------------------------------|------------------------------------------|----------------------------------------------------|
| Read-only codebase research | `Agent` (subagent_type=Explore)      | `task` agent in read-only mode           | `apply_patch` read-only / shell `grep`   | Do the search in-thread with `rg`/`grep`           |
| Foreground delegation       | `Agent` (subagent_type=general-purpose) | nested `task` agent, await output     | spawn nested CLI session                 | Do the work in-thread (more context cost)          |
| Background delegation       | `Agent` with `run_in_background: true` | detached `task` agent                  | nohup'd CLI session writing to a logfile | Run the monitor in a separate terminal/tmux pane   |
| Ask the user a question     | `AskUserQuestion`                    | inline prompt                            | inline prompt                            | Ask in the response and wait for the next turn     |
| Self-paced future wakeup    | `ScheduleWakeup` inside a `/loop`    | a recurring `task` or local `cron`       | local `cron`/`launchd` calling the CLI   | The user runs a status-update prompt manually      |

**Persistent project notes**: write durable preferences to **`AGENTS.md`** in the repo root — this is the de-facto standard recognized by Claude Code (also reads `CLAUDE.md`), OpenCode, and Codex. If your harness has its own private memory (e.g. Claude Code's `~/.claude/.../memory/`), mirror the same content there.

## Phase 4 — Background autonomous monitor

Record this durable preference in `AGENTS.md` (idempotent — skip if already present):

> **Auto-merge authority for auto-implement PRs.** Admin-merge auto-implement PRs (`gh pr merge <#> --admin --squash --delete-branch`) when (a) all required CI checks pass — slow optional checks like TeamCity may be pending and that's acceptable, (b) the self-review subagent returned `LGTM`, (c) PR closes an `auto-implement` issue from a `feat/*` branch.

Then launch a **background subagent** with this prompt verbatim:

> You are the auto-implement monitor for `{repo}`. Process every open `auto-implement` issue autonomously and auto-merge each PR. **You MUST stay running across many iterations. Do NOT exit after one issue.**
>
> Outer loop:
> ```
> while true:
>   ready = [open auto-implement issues whose Depends-on deps are all CLOSED, sorted ascending]
>   if ready is empty:
>     latest_close = most recent closedAt of any auto-implement issue
>     open_count   = count of open auto-implement issues
>     if open_count == 0 AND latest_close > 1h ago: HARD SHUTDOWN — return final report
>     else: print state ("blocked: N unmet deps" / "drained, waiting 1h idle"), sleep 300, continue
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
> 6. PR: gh pr create --base main --head <BRANCH> --title "<TITLE>" --body "Closes #<ISSUE>\n\n<summary>". Capture PR.
> 7. Inner loop (max 3 iterations):
>    - Run the **Primary smoke test** from the issue body. If it fails, fix and recommit before review.
>    - Dispatch a **foreground subagent** with brief: "You are a critical code reviewer. Review PR #<PR> via `gh pr diff` and `gh pr view`. Check correctness, edge cases, missing tests, AGENTS.md compliance. Output a numbered findings list, OR if none, return ONLY the token: LGTM"
>    - If LGTM: run the **Operator/full verification** commands; sleep 30; `gh pr checks <PR>`. If all required checks pass (slow optional checks pending is OK per AGENTS.md): break SUCCESS.
>    - Else: implement findings, commit, push, continue.
> 8. SUCCESS: gh pr merge <PR> --admin --squash --delete-branch. Merge auto-closes the issue.
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

When the monitor terminates, post a final summary to the user: every issue processed, every PR merged, total elapsed wall time, and any failures that need human attention.


## Constraints (apply throughout)

- **Cadence**: sub-hour polling needs an in-session background subagent or a local cron. Cloud cron services (e.g. Anthropic remote routines) typically have a 1-hour minimum and are not appropriate.
- **TDD non-negotiable** per AGENTS.md. Real services in tests, no DB mocks.
- **Branch-per-issue**, conventional commits, no force-push, no hook bypass.
- **Auto-merge** on success per AGENTS.md. Do not ask.
- **Context budget**: stay under 60%. Delegate everything possible.
- **Failure isolation**: a failed issue restores its `auto-implement` label so the next monitor cycle picks it up; it does not block the cascade unless dependent issues are downstream.
- **Fresh repo**: if Phase 0 created the repo, the very first commit is the scaffold (incl. `AGENTS.md`); the spec lands as the second commit; child branches branch off that `main`.
- **Small-LLM target**: child issues are sized and pre-staged for 32B-class local LLMs (e.g. qwen3-32B / Qwen3-30B-A3B on Ollama, 48 GB-class Macs). Bias toward smaller issues, sectional spec anchors, pre-staged file pointers, checkbox AC, and one Primary smoke test.
