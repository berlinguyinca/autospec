
# autospec-fleet workflow (harness-neutral)

Autospec Fleet is the workspace-level supervisor for a cluster of autospec
workers. It starts from an empty directory, accepts GitHub repository URLs,
and (once checkouts exist in the managed workspace) `run` launches a
per-repo `autospec-autonomous` conductor for each eligible checkout — not a
single one-shot `/autospec-run` session. Each conductor is itself perpetual:
it walks the same priority waterfall `/autospec-autonomous` runs standalone,
of which draining the repo's `/autospec-run` queue is one tier among several.

It does **not** replace `/autospec-run` or `/autospec-autonomous`. Fleet owns
cross-repo scheduling (which repos get a worker, how many run in parallel,
liveness/idempotence so a repo is never double-spawned); the per-repo
conductor it launches owns everything inside that checkout — issue-level
claiming, implementation, review, CI, and merge behavior all still happen
inside `/autospec-run`, which the conductor drives.

Be precise, not optimistic, about what is wired up today: `fleet-run.sh`
launches real conductor processes, and `fleet-init.sh` provisions real
checkouts — cloning a repo missing from the workspace and
fetch+fast-forward-updating one that already exists, idempotently and
without ever touching a dirty or non-fast-forwardable checkout. A repo
`fleet-init.sh` has not yet been run for is still skipped by `fleet-run.sh`
with "checkout not found" rather than being created on the fly — provision
the workspace first. `--emit fleet-config` and `AUTOSPEC_SPEND_SCOPE` have
no production consumer yet, and the project-board control-label mirror
(`project-board-control-mirror.sh`) has no caller wired into this flow. Do
not describe fleet as an end-to-end, fully-unattended multi-repo shipping
pipeline until those land.

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
- `run` loads fleet config plus node-local capacity and launches a per-repo
  `autospec-autonomous` conductor for each eligible checkout that has ready
  queue work and is not already live (never a raw `/autospec-run` one-shot).
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

`init` (real, idempotent checkout provisioning — clone missing repos,
fetch+fast-forward-update existing ones, never touching a dirty or
non-fast-forwardable checkout), `sync`, config linting, `run` (scheduling,
liveness/idempotence, and the real per-repo `autospec-autonomous` conductor
launch), `status`, and `stop` are implemented and tested (`tests/fleet/`,
`tests/install/`). `run` still requires `init` to have populated the
workspace first — a repo missing from `workspace/<owner>__<repo>` is
skipped with a "checkout not found" message rather than being created
on the fly. `--emit fleet-config` and `AUTOSPEC_SPEND_SCOPE` have no
production consumer yet, and the project-board control-label mirror has no
caller wired into this flow.
