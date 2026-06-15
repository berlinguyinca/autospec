
# autospec-stop (harness-neutral)

Halt a running autospec monitor gracefully or immediately. All paths route
through `bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-stop.sh" "$@"`.

Manage your own context — never exceed 60%. Delegate to subagents whenever your harness supports it.

<!-- autospec-block:startup-self-update SKILL_NAME=autospec-stop -->

## Self-update mode

If the feature-request argument matches the regex `^\s*update\s*$` (case-insensitive, whitespace-padded), this skill enters self-update mode and does not run the normal pipeline:

1. **Detect harness** by checking which install path exists for this skill:
   - Claude Code: `~/.claude/skills/autospec-stop/SKILL.md`
   - OpenCode:    `~/.config/opencode/agent/autospec-stop.md`
   - Codex CLI:   `~/.codex/prompts/autospec-stop.md`
2. **Re-install the full autospec suite from `main`** by piping the canonical installer:
   ```bash
   curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh | bash -s -- --skill all --harness all --update
   ```
   Run this one-liner once; it refreshes all autospec skills across all harnesses.
3. **Show the diff** between the prior installed file(s) and the freshly fetched copy.
4. **Stop.** Do not enter any pipeline phase. Print the upgrade summary and return to the user.

If no install path is detected, print `Self-update: no installed copy of autospec-stop found; run install.sh first.` and exit.

## Stop mode

If the feature-request argument matches the regex `^\s*stop(\s+--\w+)*\s*$`
(case-insensitive), this skill enters stop mode and does not run the normal
pipeline:

1. Dispatch to `bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-stop.sh" <args>`.
2. Print the helper's stdout to the user.
3. Stop. Do not enter Phase 0 or any pipeline phase.

## Invocation

```
/autospec-stop [--graceful|--immediate|--status|--resume|--help]
```

Dispatches directly to `bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-stop.sh" "$@"`. All logic lives
in the helper script.

| Flag | Behaviour |
|---|---|
| `--graceful` (default) | Write sentinel mode=graceful; monitor exits after current issue finishes. |
| `--immediate` | Write sentinel mode=immediate; process(ISSUE) commits+pushes+marks paused at next step boundary. |
| `--status` | Report current sentinel state, paused-by-user issue count, last monitor progress line. |
| `--resume` | Memory-aware: detect the monitor exit mode (silent-exit \| prompt-overflow \| clean \| unknown — driven by `docs/memory/feedback_monitor_silent_exit.md`), print the matching recovery guidance, then strip paused-by-user labels from all paused issues, restore auto-implement, delete stop.flag. |
| `--help` | Print usage and exit. |

## Harness detection (run once at skill start)

Detect your harness by checking available tools before dispatching work:

1. **Claude Code** — the `Agent` tool with a `subagent_type` parameter is available.
   - `TIER_A` = `opus` + `ultrathink`  (model ID: claude-opus-4-7)
   - `TIER_B` = `sonnet`               (model ID: claude-sonnet-4-6)

2. **OpenCode** — a `task` tool with model/tier configuration is available (no `subagent_type`).
   - `TIER_A` = top-tier task model + high reasoning
   - `TIER_B` = smaller-tier task model + medium reasoning

3. **Codex CLI** — neither `Agent` nor a configurable `task` tool is available; `apply_patch` is the primary edit tool.
   - `TIER_A` = current top GPT model + `reasoning_effort=high`
   - `TIER_B` = `gpt-5.1-codex-spark` + `reasoning_effort=medium`

**Fallback rule:** If `TIER_B` is not available in your harness (model unknown, quota/capacity failure, authorization failure, or tool call returns an error for that model), silently retry the same dispatch with `TIER_A`. Preserve the parent context on retry; for Codex native subagents, fork/inherit the current conversation context and use the latest top GPT model instead of moving the work into the main session. Never ask the user.

Hold `TIER_A` and `TIER_B` for the entire skill run.

## Required capabilities & harness adapter

This workflow assumes a small set of capabilities. Map each one to your harness's actual tool. If a capability is missing, use the listed fallback.

| Capability                  | Claude Code                          | OpenCode                                 | Codex CLI                                | Fallback if missing                                |
|-----------------------------|--------------------------------------|------------------------------------------|------------------------------------------|----------------------------------------------------|
| Run shell command            | `Bash`                               | `bash` tool                              | `shell` / `apply_patch`                  | Ask user to run manually                           |
| Ask the user a question      | `AskUserQuestion`                    | inline prompt                            | inline prompt                            | Ask in the response and wait for the next turn     |
| Subagent model tier          | Tier B: `sonnet` + medium thinking   | Tier B: smaller-tier `task` + medium reasoning | Tier B: `gpt-5.1-codex-spark` + `reasoning_effort=medium` | Fall back UP on unavailability |
<!-- autospec-block:harness-adapter-core -->

**Model tier:** Tier B (implementation work) — this skill is a thin dispatch wrapper.

## Procedure

1. Run startup self-update block above.
2. Parse the user's invocation arguments (default to `--graceful` if no flag given).
3. Execute: `bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-stop.sh" "$@"`
4. Print stdout to user and exit.

If `${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-stop.sh` is not found, print:
```
autospec-stop: helper not found at ${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-stop.sh.
Reinstall autospec-stop from the autospec repo, then retry.
```
and exit non-zero.

## Hard rules

- Never call `autospec-stop.sh --abort-current-issue` from user invocation. That flag is internal.
- Never modify issue bodies, labels, or PRs directly — the helper script owns all GitHub mutations.
- This skill is a thin wrapper. All behaviour is in the helper script.
