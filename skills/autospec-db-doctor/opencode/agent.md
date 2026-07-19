---
description: Use when the user wants to run diagnostics on the optional autospec-db telemetry module — resolves the `autospec-db` binary (PATH, then `~/.autospec/bin/autospec-db`), runs `autospec-db doctor`, and maps each reported FAIL to a concrete operator fix. Degrades gracefully (prints the install one-liner and stops) when the binary is not installed. Never echoes a DSN.
mode: primary
---

# autospec-db-doctor (harness-neutral)

Run diagnostics on the optional `autospec-db` telemetry module and turn each
FAIL line into an actionable fix. This skill is a thin, read-only wrapper: it
never writes to the database, never prints a DSN, and never blocks a run.

Manage your own context — never exceed 60%. Delegate to subagents whenever your harness supports it.

<!-- autospec-block:startup-self-update SKILL_NAME=autospec-db-doctor -->

## Self-update mode

If the feature-request argument matches the regex `^\s*update\s*$` (case-insensitive, whitespace-padded), this skill enters self-update mode and does not run the normal pipeline:

1. **Detect harness** by checking which install path exists for this skill:
   - Claude Code: `~/.claude/skills/autospec-db-doctor/SKILL.md`
   - OpenCode:    `~/.config/opencode/agent/autospec-db-doctor.md`
   - Codex CLI:   `~/.codex/prompts/autospec-db-doctor.md`
2. **Re-install the full autospec suite from `main`** by piping the canonical installer:
   ```bash
   curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh | bash -s -- --skill all --harness all --update
   ```
   Run this one-liner once; it refreshes all autospec skills across all harnesses.
3. **Show the diff** between the prior installed file(s) and the freshly fetched copy.
4. **Stop.** Do not enter any pipeline phase. Print the upgrade summary and return to the user.

If no install path is detected, print `Self-update: no installed copy of autospec-db-doctor found; run install.sh first.` and exit.

## Invocation

```
/autospec-db-doctor
```

No arguments. Always runs the diagnostic pass below.

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
|-----------------------------|---------------------------------------|-------------------------------------------|--------------------------------------------|------------------------------------------------------|
| Run shell command            | `Bash`                               | `bash` tool                              | `shell` / `apply_patch`                  | Ask user to run manually                           |
| Ask the user a question      | `AskUserQuestion`                    | inline prompt                            | inline prompt                            | Ask in the response and wait for the next turn     |
| Subagent model tier          | Tier B: `sonnet` + medium thinking   | Tier B: smaller-tier `task` + medium reasoning | Tier B: `gpt-5.1-codex-spark` + `reasoning_effort=medium` | Fall back UP on unavailability |
<!-- autospec-block:harness-adapter-core -->

**Model tier:** Tier B (implementation work) — this skill is a thin diagnostic wrapper.

## Procedure

1. Run startup self-update block above.
2. Dispatch to the runnable backend: `bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/db-doctor.sh"`. All logic — binary resolution (PATH, then `~/.autospec/bin/autospec-db`), running `autospec-db doctor`, mapping each reported FAIL line to a concrete fix (no `db.conf` → installer; connect FAIL → DSN host/port/sslmode + pgbouncer hints; pending schema updates → re-run installer; spool nonzero → `autospec-db drain`), DSN redaction, and the final summary line — lives in that script.
3. Print the script's stdout to the user verbatim.
4. If `${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/db-doctor.sh` is not found, print:
   ```
   autospec-db-doctor: helper not found at ${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/db-doctor.sh.
   Reinstall autospec-db-doctor from the autospec repo, then retry.
   ```
   and exit non-zero.
5. This skill always exits 0 from the underlying script (binary absent, all-OK, or FAILs mapped) — it is a diagnostic report, never a blocking gate.

## Hard rules

- Never construct, print, or log a DSN in any form, including inside a mapped fix message.
- Never run any `autospec-db` subcommand other than `doctor` (and, only when explicitly instructed by a mapped fix that the user asks to apply, `drain` — this skill only prints the `drain` suggestion, it does not run it automatically).
- Never treat an absent binary as a failure; it is expected on machines that opted out of the telemetry module.
- This skill is read-only diagnostics: it never auto-fixes a reported problem.
