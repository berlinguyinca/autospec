---
name: autospec-fleet
description: Use when the user wants to supervise autospec-run across multiple GitHub repositories from an empty workspace - initializes fleet config, clones or syncs repos, and runs per-repo autospec-run workers.
mode: primary
---

# autospec-fleet workflow (harness-neutral)

Autospec Fleet is the workspace-level supervisor for a cluster of autospec
workers. It starts from an empty directory, accepts GitHub repository URLs,
clones or syncs those repositories into a managed workspace, and launches the
existing `/autospec-run` monitor inside each eligible checkout.

It does **not** replace `/autospec-run`. Fleet owns repo-level orchestration;
`/autospec-run` owns issue-level claiming, implementation, review, CI, and
merge behavior.

Design source:
`docs/specs/2026-05-28-autospec-fleet-design.md`.

<!-- autospec-block:startup-self-update SKILL_NAME=autospec-fleet -->

## Self-update mode

If the feature-request argument matches the regex `^\s*update\s*$`
(case-insensitive, whitespace-padded), this skill enters self-update mode and
does not run a fleet command:

1. Detect harness by checking which install path exists for this skill:
   - Claude Code: `~/.claude/skills/autospec-fleet/SKILL.md`
   - OpenCode: `~/.config/opencode/agent/autospec-fleet.md`
   - Codex CLI: `~/.codex/prompts/autospec-fleet.md`
2. Re-install the full autospec suite from `main`:
   ```bash
   curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh | bash -s -- --skill all --harness all --update
   ```
3. Show the diff between prior installed files and the freshly fetched copy
   when the harness can expose it.
4. Stop. Do not enter any subcommand.

If no install path is detected, print
`Self-update: no installed copy of autospec-fleet found; run install.sh first.`
and exit.

## Invocation

```text
/autospec-fleet init <repo-url>...
/autospec-fleet sync [--config-repo <url>]
/autospec-fleet run [--profile <name>] [--parallel <N>] [--once]
/autospec-fleet status
/autospec-fleet stop --graceful
/autospec-fleet stop --immediate
```

- `init` creates `autospec-fleet.yml` in the current directory and prepares the
  managed workspace for the listed GitHub repository URLs.
- `sync` updates the local copy of a fleet control repository when configured.
- `run` loads fleet config plus node-local capacity and launches per-repo
  `/autospec-run` workers.
- `status` summarizes queue state, active workers, open PRs, and recent
  failures across configured repositories.
- `stop` forwards existing autospec stop semantics to active repo workers.
- `gui` launches a local one-page browser GUI on `127.0.0.1` with a random
  port and URL token; lets the operator toggle repos and edit top-level config.

## GUI mode

```text
/autospec-fleet gui
```

Runs `skills/autospec-fleet/scripts/fleet-gui.sh`, which:

1. Checks for `gh` on PATH (exits 1 with `code_health:fleet_gui_missing_gh`
   if absent).
2. Picks a random port (49152–65535) and 16-hex URL token.
3. Starts a Python stdlib HTTP server bound to `127.0.0.1`.
4. Opens the default browser at `http://127.0.0.1:<port>/?t=<token>`.
5. Serves `GET /api/repos`, `GET /api/config`, and `POST /api/config`.
6. Exits after save or after 15 min idle (`AUTOSPEC_GUI_IDLE_SECS` to
   override).

Flags: `--no-browser`, `--print-url`, `--once` (smoke-test mode).

## Required capabilities & harness adapter

| Capability                  | Claude Code                          | OpenCode                                 | Codex CLI                                | Fallback if missing                                |
|-----------------------------|--------------------------------------|------------------------------------------|------------------------------------------|----------------------------------------------------|
| Read-only codebase research | `Agent` (subagent_type=Explore)      | `task` agent in read-only mode           | `apply_patch` read-only / shell `grep`   | Do the search in-thread with `rg`/`grep`           |
| Foreground delegation       | `Agent` (subagent_type=general-purpose) | nested `task` agent, await output     | spawn nested CLI session                 | Do the work in-thread (more context cost)          |
| Background delegation       | `Agent` with `run_in_background: true` | detached `task` agent                  | nohup'd CLI session writing to a logfile | Run the monitor in a separate terminal/tmux pane   |
| Ask the user a question     | `AskUserQuestion`                    | inline prompt                            | inline prompt                            | Ask in the response and wait for the next turn     |
| Self-paced future wakeup    | `ScheduleWakeup` inside a `/loop`    | a recurring `task` or local `cron`       | local `cron`/`launchd` calling the CLI   | The user runs a status-update prompt manually      |
| Subagent model tier         | Tier A: `opus` + `ultrathink`; Tier B: `sonnet` + medium thinking | Tier A: top `task` model + high reasoning; Tier B: smaller-tier `task` + medium reasoning | Tier A: top GPT + `reasoning_effort=high`; Tier B: `gpt-5.1-codex-spark` + `reasoning_effort=medium` | Honor the per-phase tier mapping in AGENTS.md; retry the same subagent UP on unavailability |
<!-- autospec-block:harness-adapter-core -->

**Model tier:** `TIER_B` (implementation work) for deterministic fleet shell
helpers and status/reporting. Future spec/decomposition work stays in
`/autospec` and `/autospec-define`.

## Harness detection (run once at skill start)

Detect your harness by checking available tools before any subcommand:

1. Claude Code: the `Agent` tool with a `subagent_type` parameter is available.
2. OpenCode: a `task` tool with model/tier configuration is available.
3. Codex CLI: neither `Agent` nor configurable `task` is available; `apply_patch`
   is the primary edit tool.

Hold `TIER_A` and `TIER_B` for the entire skill run. Silently fall back UP from
`TIER_B` to `TIER_A` on quota, capacity, model, or authorization failure while
preserving parent context.

## Configuration

Fleet desired state lives in `autospec-fleet.yml` in the current workspace or a
Git-backed control repository:

```yaml
version: 1
workspace: .autospec-fleet/repos
default_profile: qwen3-6-35b-a3b-laptop
parallel_repos: 2
repos:
  - url: https://github.com/org/repo-a.git
    profile: qwen3-6-35b-a3b-laptop
    enabled: true
```

Node-local capacity lives outside source control:

```yaml
node_id: mac-mini-01
workspace: ~/.autospec/fleet/repos
max_parallel_repos: 2
profiles:
  - qwen3-6-35b-a3b-laptop
```

Never store tokens or secrets in either file. Use `gh` and git credential
helpers for authentication.

## Current scaffold status

This scaffold defines the skill surface and install contract. Follow-up issues
land the schemas, URL/workspace helpers, config linting, scheduler, status,
stop, docs, and dry-run smoke tests. Until those land, subcommands may report
that implementation is pending.
