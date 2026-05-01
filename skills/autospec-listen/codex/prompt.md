
# autospec-listen (harness-neutral)

Conversation listener that watches for issue-filing or spec-writing intent
mid-conversation and dispatches to the right downstream flow without forcing
the user to remember a slash command.

## Self-update mode

If the feature-request argument matches the regex `^\s*update\s*$` (case-insensitive, whitespace-padded), this skill enters self-update mode and does not run the normal trigger flow:

1. **Detect harness** by checking which install path exists for this skill:
   - Claude Code: `~/.claude/skills/autospec-listen/SKILL.md`
   - OpenCode:    `~/.config/opencode/agent/autospec-listen.md`
   - Codex CLI:   `~/.codex/prompts/autospec-listen.md`
2. **Re-install from `main`** by piping the canonical installer:
   ```bash
   bash <(curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-listen/install.sh) --harness <detected> --update
   ```
   If multiple harness paths exist, run the one-liner once per detected harness.
3. **Show the diff** between the prior installed file(s) and the freshly fetched copy (e.g. `diff <(cat <prior>) <(curl -fsSL ...SKILL.md)` or the equivalent recorded by the installer).
4. **Stop.** Do not enter any trigger flow. Print the upgrade summary and return to the user.

If no install path is detected, print `Self-update: no installed copy of autospec-listen found; run install.sh first.` and exit.

## Required capabilities & harness adapter

This skill assumes four capabilities. Map each one to your harness's actual tool. If a capability is missing, use the listed fallback.

| Capability                  | Claude Code                          | OpenCode                                 | Codex CLI                                | Fallback if missing                                |
|-----------------------------|--------------------------------------|------------------------------------------|------------------------------------------|----------------------------------------------------|
| Read recent conversation    | harness-native message buffer        | harness-native message buffer            | harness-native message buffer            | Ask the user to paste the relevant turns           |
| Ask the user a question     | `AskUserQuestion`                    | inline prompt                            | inline prompt                            | Ask in the response and wait for the next turn     |
| Run `gh` CLI                | shell tool                           | shell tool                               | shell tool                               | Have the user run the command and confirm          |
| Hand off to another skill   | `Skill` invocation (`/autospec-define`) | nested skill invocation                | `/autospec-define` slash command         | Print the suggested command for the user to run    |
| Subagent model tier         | Tier B: `sonnet` + medium thinking; Tier A only if escalating to /autospec-define which honors its own tier policy | Tier B: smaller-tier `task` + medium reasoning; escalation honors target skill | Tier B: `gpt-5.1-codex-spark` + `reasoning_effort=medium`; escalation honors target | Honor AGENTS.md tier mapping; fall back UP on unavailability |

**Persistent project notes**: write durable preferences to **`AGENTS.md`** in the repo root — this is the de-facto standard recognized by Claude Code (also reads `CLAUDE.md`), OpenCode, and Codex. Per AGENTS.md, this listener runs at **Tier B** (it's a router, not a designer); when it hands off to `/autospec-define` for a spec trigger, the downstream skill applies its own tier policy. Fall back UP the tier on unavailability.

## Trigger flow

> **Model tier:** Tier B (implementation work) — this listener is a router; subagent dispatches inside its trigger flows (drafting issue bodies, summarizing recent conversation) use cheaper models per AGENTS.md. Spec-trigger handoff to `/autospec-define` lets the downstream skill apply its own tier policy.

(Trigger-flow body is implemented in follow-up issues #44 and #45.)
