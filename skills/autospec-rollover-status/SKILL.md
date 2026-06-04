---
name: autospec-rollover-status
description: Reports current context % and last rollover event for the active autospec-session monitor. Use when the user asks about context status, rollover status, how close to rollover, or whether a compaction/handoff is imminent.
trigger: "rollover status|context status|how close to rollover|am I about to roll|context percentage|monitor status"
---

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
