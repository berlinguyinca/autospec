
# autospec-rollover-status

Read-only diagnostic skill: shows context-window usage percentage and last
rollover event for the active autospec session monitor.

Manage your own context — never exceed 60%. Delegate to subagents whenever
your harness supports it.

<!-- autospec-block:startup-self-update SKILL_NAME=autospec-rollover-status -->

## Self-update mode

If the feature-request argument matches the regex `^\s*update\s*$` (case-insensitive, whitespace-padded), this skill enters self-update mode and does not run the normal pipeline:

1. **Detect harness** by checking which install path exists for this skill:
   - Claude Code: `~/.claude/skills/autospec-rollover-status/SKILL.md`
   - OpenCode:    `~/.config/opencode/agent/autospec-rollover-status.md`
   - Codex CLI:   `~/.codex/prompts/autospec-rollover-status.md`
2. **Re-install the full autospec suite from `main`** by piping the canonical installer:
   ```bash
   curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh | bash -s -- --skill all --harness all --update
   ```
3. **Show the diff** between the prior installed file(s) and the freshly fetched copy.
4. **Stop.** Do not enter any pipeline phase.

If no install path is detected, print `Self-update: no installed copy of autospec-rollover-status found; run install.sh first.` and exit.

## Invocation

```
/autospec-rollover-status
```

Runs `bash skills/autospec-rollover-status/show.sh` and presents the output.

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

| Capability             | Claude Code | OpenCode    | Codex CLI   | Fallback if missing              |
|------------------------|-------------|-------------|-------------|----------------------------------|
| Run shell command       | `Bash`      | `bash` tool | `shell`     | Ask user to run manually         |
| Subagent model tier    | Tier A: `haiku` — read-only, no reasoning | Tier A: smallest tier | Tier A | inline |
| Subagent dispatch policy | per AGENTS.md decision matrix        | per AGENTS.md decision matrix            | per AGENTS.md decision matrix            | inline with main-session token cost                |

**Model tier:** Tier A (haiku) — pure log read, no judgment needed.

## Procedure

1. Run startup self-update block above.
2. Execute `bash skills/autospec-rollover-status/show.sh`.
3. Present the output verbatim. If the monitors directory is missing or empty, explain that no monitor session is active and suggest running `autospec-session start`.
4. Stop.

## Hard rules

- This skill is **read-only**. Never write files, labels, or GitHub state.
- Never modify log files or PID files.
- Tolerates missing log gracefully — exit 0 with a clear message.
