# autospec

A multi-harness skill that takes a single feature request and ships it end-to-end:
bootstrap the GitHub repo (if missing), investigate, brainstorm, write a design spec,
decompose into linked GitHub issues, then run an autonomous implementation loop with
admin auto-merge until every child issue is closed — pinging the user with periodic
status deltas along the way.

Works on **Claude Code**, **OpenCode**, and **Codex CLI**.

## Related skills

| Skill | Purpose |
| --- | --- |
| [`autospec-define`](../autospec-define/README.md) | Planning half (Phases 0–3.5). Stops after the review-and-label step and hands off to `autospec-run`. |
| [`autospec-run`](../autospec-run/README.md) | Implementation half (Phases 4–6). Consumes the populated `auto-implement` queue; supports `--profile <name>` filtering. |
| [`autospec-classify`](../autospec-classify/README.md) | Standalone retro-labeler. Applies the Phase 3.5 `ctx:*` / `reasoning:*` rubric to issues that pre-date Phase 3.5. |

## Self-update

Once installed, run the skill with the literal argument `update` to refresh in
place. The skill detects its harness, re-runs the canonical install one-liner
with `--update`, shows the diff, and stops without entering Phase 0:

```
/autospec update
```

Or re-run the per-skill installer with `--update`:

```bash
./install.sh --harness all --update
```

## Quick install

One-line install for all three supported agents (Claude Code, OpenCode, Codex CLI):

```bash
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/codex-skills/main/skills/autospec/install.sh | sh -s -- --harness all
```

This installs autospec for all three supported agents at once. The installer
auto-downloads the skill files from the same branch when piped, so you don't
need to clone the repo first.

> **Safety note.** Read the script before piping to `sh` if you don't trust the
> source. The audited two-step equivalent is:
>
> ```bash
> curl -fsSL https://raw.githubusercontent.com/berlinguyinca/codex-skills/main/skills/autospec/install.sh > install.sh
> less install.sh
> sh install.sh --harness all
> ```

Per-harness one-liners:

```bash
# Claude Code only
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/codex-skills/main/skills/autospec/install.sh | sh -s -- --harness claude
# OpenCode only
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/codex-skills/main/skills/autospec/install.sh | sh -s -- --harness opencode
# Codex CLI only
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/codex-skills/main/skills/autospec/install.sh | sh -s -- --harness codex
```

## What it does

The skill executes a 7-phase workflow (Phase 0 through Phase 6). The canonical body
is harness-neutral; each phase delegates to subagents whenever the host harness
supports them, and falls back to in-thread execution when it doesn't. The whole
pipeline runs from a single user invocation — the only required interactive step is
the optional repo-bootstrap question (name, visibility, owner) when no GitHub remote
is detected. Generated child issues are pre-staged for 32B-class local LLMs (Ollama qwen3-style on Mac/Linux), not just cloud agents — file pointers, section anchors, checkbox AC, and a single Primary smoke test per inner loop.

## Why use it

Reach for this skill when:

- You have a non-trivial feature that decomposes into multiple sequenced PRs.
- You want the agent to ship the whole thing autonomously, not just plan it.
- You'd rather review merged PRs than babysit a chat session.
- You want repo bootstrap (`gh repo create`, scaffold, push) handled automatically
  if you're starting from nothing.
- You want an opinionated TDD + branch-per-issue + admin-squash-merge loop, not a
  bespoke one.

## Architecture (the phases)

| Phase | Name | One-liner |
|---|---|---|
| **0** | Bootstrap repo (if missing) | Detect missing git/GitHub remote, ask name/visibility/owner once, scaffold `.gitignore` + `AGENTS.md` + `README.md`, push. |
| **1** | Investigate (delegate) | Read-only research subagent maps relevant files, schema, services. Real queries against remote systems if the feature touches them. |
| **2** | Brainstorm + design | Structured 5-section brainstorm (architecture / API / data / errors / testing) with explicit per-section approval, written to `docs/specs/YYYY-MM-DD-<topic>-design.md`. |
| **3** | Decompose into linked GitHub issues (delegate) | Foreground subagent creates labels, an EPIC umbrella, and N self-contained mini-specs sized for 32B-class local LLMs: file pointers, section anchors, checkbox AC, primary smoke test. |
| **3.5** | Review and label (delegate) | Foreground subagent applies the `ctx:*` / `reasoning:*` rubric to each child, writes a `## Model fit` block into the body (idempotent), runs sibling-normalization across split children, and validates dep edges (closed-dep / child-less-tracker warnings; circular sibling-dep hard fail). Optional board assignment via `~/.autospec/project-map.yml` (full reader lands in PR B3 / #16). |
| **4** | Background autonomous monitor | Background subagent loops: pick next ready issue → worktree → TDD → push → PR → self-review → admin-squash-merge → repeat until drained. |
| **5** | Periodic status updates | Self-paced ~25 min wakeups posting deltas (closed issues, merged PRs, failures, blockers); slows to ~50 min when quiet. |
| **6** | Final report | When the monitor terminates, summarize every issue processed, PR merged, wall time, and any human-attention failures. |

## Required capabilities & harness adapter

The canonical body assumes five capabilities. The skill maps each one to your
harness; if a capability is missing the listed fallback applies.

| Capability                  | Claude Code                          | OpenCode                                 | Codex CLI                                | Fallback if missing                                |
|-----------------------------|--------------------------------------|------------------------------------------|------------------------------------------|----------------------------------------------------|
| Read-only codebase research | `Agent` (subagent_type=Explore)      | `task` agent in read-only mode           | `apply_patch` read-only / shell `grep`   | Do the search in-thread with `rg`/`grep`           |
| Foreground delegation       | `Agent` (subagent_type=general-purpose) | nested `task` agent, await output     | spawn nested CLI session                 | Do the work in-thread (more context cost)          |
| Background delegation       | `Agent` with `run_in_background: true` | detached `task` agent                  | nohup'd CLI session writing to a logfile | Run the monitor in a separate terminal/tmux pane   |
| Ask the user a question     | `AskUserQuestion`                    | inline prompt                            | inline prompt                            | Ask in the response and wait for the next turn     |
| Self-paced future wakeup    | `ScheduleWakeup` inside a `/loop`    | a recurring `task` or local `cron`       | local `cron`/`launchd` calling the CLI   | The user runs a status-update prompt manually      |

Persistent project notes are written to `AGENTS.md` in the target repo root — this is
recognized by all three harnesses.

## Dependencies

Required:

- **`git`** ≥ 2.5 (worktree support — Phase 4 creates a worktree per child issue).
- **`gh` CLI** ([cli.github.com](https://cli.github.com/)), authenticated via
  `gh auth login`, with `repo` and `workflow` scopes. For the auto-merge step the
  authenticated user must have **admin** or **maintain** permission on the target
  repo (squash-admin-merge requires it).
- **`jq`** for JSON processing in shell snippets.
- A POSIX shell (`bash` or `sh`) for the installer.
- The host harness — one of:
  - **Claude Code** (any version with skills support).
  - **OpenCode** (any current version; agents loaded from `~/.config/opencode/agent/`).
  - **Codex CLI** (any version with the prompt library at `~/.codex/prompts/`).
- A project-level test runner appropriate to the target codebase — required by
  Phase 4's implementation subagent. Examples:
  - Go: `go test ./... -count=1`
  - Node: `npm test`
  - Python: `pytest`
  - Rust: `cargo test`
  - Scala: `sbt test`

Optional:

- A CI provider (GitHub Actions, TeamCity, GitGuardian, etc.). Slow optional checks
  are tolerated per the auto-merge rules in `AGENTS.md`.
- A pre-commit hook framework — the skill respects hooks (it never passes
  `--no-verify`).

## Installation (from a clone)

If you've already cloned the repo, you can run the bundled installer directly:

```bash
cd skills/autospec
./install.sh                    # interactive — prompts for harness
./install.sh --harness all      # install for Claude Code, OpenCode, and Codex CLI
./install.sh --harness claude   # Claude Code only
./install.sh --harness opencode # OpenCode only
./install.sh --harness codex    # Codex CLI only
./install.sh --symlink          # symlink instead of copy (updates propagate)
```

The installer:

1. Verifies `git`, `gh`, and `jq` are present (warns on missing optional deps).
2. Verifies `gh auth status` (warns if not authenticated).
3. Creates the harness-specific skill directory if needed.
4. Copies (or symlinks) the right variant into place.
5. Prints the install path and example invocation.
6. Re-running upgrades the install (idempotent).

Honors `CLAUDE_CONFIG_DIR`, `OPENCODE_CONFIG_DIR`, and `CODEX_HOME` if set.

### Manual install

#### Claude Code

```bash
mkdir -p ~/.claude/skills/autospec
cp skills/autospec/SKILL.md \
   ~/.claude/skills/autospec/SKILL.md
```

The skill becomes discoverable as `autospec` (or the user can
invoke it via the Skill tool).

#### OpenCode

```bash
mkdir -p ~/.config/opencode/agent
cp skills/autospec/opencode/agent.md \
   ~/.config/opencode/agent/autospec.md
```

OpenCode picks up agent markdown files from `~/.config/opencode/agent/`. The
frontmatter uses `description` + `mode: primary` per current OpenCode conventions;
if your OpenCode version expects a different schema, the skill body still works
when the file is loaded as a plain prompt.

#### Codex CLI

```bash
mkdir -p ~/.codex/prompts
cp skills/autospec/codex/prompt.md \
   ~/.codex/prompts/autospec.md
```

Codex CLI exposes any markdown file in `~/.codex/prompts/` as a slash command —
in this case `/autospec`.

### Uninstall

```bash
./uninstall.sh --harness all
```

Same flags as the installer.

## Usage

### Claude Code

```
/autospec Add a real-time presence indicator to the dashboard
```

Or invoke via the Skill tool with the same description.

### OpenCode

```
@autospec Add a real-time presence indicator to the dashboard
```

(Or whichever invocation syntax your OpenCode version uses for primary agents.)

### Codex CLI

```
codex
> /autospec Add a real-time presence indicator to the dashboard
```

### Full example

```
> /autospec Build a Slack bot that posts a daily standup digest
  pulled from yesterday's merged PRs and closed issues across our org

[Phase 0 — Bootstrap]
No git repo detected. Suggested name: slack-standup-digest
Visibility? [private]/public  Owner? [berlinguyinca]/<org>
> private, berlinguyinca
... gh repo create berlinguyinca/slack-standup-digest --private --source=. --push ...

[Phase 1 — Investigate]
(empty repo — skipped)

[Phase 2 — Design]
... structured brainstorm, written to docs/specs/2026-04-30-slack-standup-design.md ...

[Phase 3 — Issues]
EPIC #1, children #2 #3 #4 #5 #6 #7 with auto-implement labels and dependency metadata.

[Phase 4 — Monitor launched in background]
agent_id=bg_8a3f...

[Phase 5 — Status, every ~25 min]
... PR #8 merged (closes #2), PR #9 merged (closes #3), #4 in-progress-by-bot ...

[Phase 6 — Final report]
6/6 child issues closed in 4h 12m. PRs: #8 #9 #10 #11 #12 #13. No human attention required.
```

## Limits & known issues

- **Sub-hour cadence requires an in-session background subagent.** Cloud cron
  services (Anthropic remote routines, GitHub-hosted cron, Vercel cron, etc.)
  typically have a 1-hour minimum, which is too coarse for the 25-min Phase 5
  cadence. Use a local `cron`/`launchd` if your harness can't keep a background
  subagent alive.
- **Fresh-repo bootstrap requires interactive consent** for repo name / visibility /
  owner. The skill cannot auto-decide these — running fully unattended on a fresh
  directory will block at the Phase 0 question.
- **Auto-merge requires admin or maintain permission** on the target repo. PRs from
  forks and PRs against repos where the authenticated user lacks merge admin will
  fall back to opening the PR and stopping (the user merges manually).
- **OpenCode frontmatter** has evolved over the project's lifetime; the variant in
  this skill uses `description` + `mode: primary`. If your OpenCode version expects
  a different schema, drop the frontmatter and load the body as a plain prompt — the
  workflow content is harness-neutral.
- **Codex CLI prompt-library** files are loaded as plain markdown; the canonical
  body is therefore unchanged from `SKILL.md` minus the frontmatter.

## Contributing

The canonical body is `SKILL.md`. The OpenCode and Codex variants are direct copies
with adjusted (or stripped) frontmatter so each harness loads cleanly. **All three
files must stay in lock-step** — when you edit one, edit the other two. To keep this
manageable:

1. Edit the substantive content of `SKILL.md` first.
2. Copy the body (everything after the `---` frontmatter block) into
   `opencode/agent.md` and `codex/prompt.md`, preserving each file's frontmatter
   header.
3. Run `./install.sh --harness all` (or just diff the three files) to confirm
   the bodies are identical.

When extending the harness adapter table to a new harness, update all three files
plus this README's adapter table.

## License

MIT — see the repository [LICENSE](../../LICENSE) file.
