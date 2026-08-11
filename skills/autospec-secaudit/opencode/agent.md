---
description: Scan generated code for security vulnerabilities, secret/credential leaks, IP/copyright/license violations, PII/data leaks, SQL injection, prompt injection, and backdoors. Runs as `/autospec-secaudit` (manual sweep) or auto-fires after each autospec-run batch unless `~/.autospec/no-secaudit.flag` exists. Also powers the blocking Phase 4 per-PR gate.
mode: primary
---

<!-- BODY START -->
## Self-update mode

Decide this purely from the request text the harness handed you. Do NOT
shell out to test the user's free-form request. Read the request, normalize
it in your reasoning (collapse whitespace, trim, lowercase), and if the result is
exactly `update`, this skill enters self-update mode and does NOT run the
normal pipeline.

1. **Detect harness** by checking which install path exists for this skill:
   - Claude Code: `~/.claude/skills/autospec-secaudit/SKILL.md`
   - OpenCode:    `~/.config/opencode/agent/autospec-secaudit.md`
   - Codex CLI:   `~/.codex/prompts/autospec-secaudit.md`
2. **Re-install from `main`** by piping the canonical installer:
   ```bash
   curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh | bash -s -- --skill all --harness all --update
   ```
   Run this one-liner once; it refreshes all autospec skills across all harnesses.
3. **Show the diff** between the prior installed file(s) and the freshly fetched copy.

## Stop mode

If the normalized request is exactly `stop` (or matches `^\s*stop(\s+--\w+)*\s*$`),
do NOT run the pipeline. Write `~/.autospec/stop.flag` so any running autospec
monitor halts after the current issue, then report that the stop sentinel was
written and exit.

## Required capabilities & harness adapter

| Capability                  | Claude Code                          | OpenCode                                 | Codex CLI                                | Fallback if missing                                |
|-----------------------------|--------------------------------------|------------------------------------------|------------------------------------------|----------------------------------------------------|
| Subagent model tier          | Tier A: `opus` + ultrathink          | Tier A: top-tier `task` + max reasoning  | Tier A: `gpt-5.1-codex` + `reasoning_effort=high` | Fall back UP on unavailability |
| Subagent dispatch policy     | per AGENTS.md decision matrix         | per AGENTS.md decision matrix            | per AGENTS.md decision matrix            | inline if subagents unavailable                    |
| Deterministic scanners       | gitleaks, semgrep, trivy, license-checker via `ensure-tool.sh` | same | same | LLM-only fallback per missing scanner (loud WARN) |

Triage runs on the spec-work tier. **Model tier:** TIER_A — confirm/triage
findings with the top-tier reasoning model for the active harness (see row above).

## Harness detection (run once at skill start, before the pipeline)

Detect your harness by checking available tools before scanning:

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

Security triage runs on `TIER_A`. Hold `TIER_A` and `TIER_B` for the entire skill run.

## When to invoke

- Manually: `/autospec-secaudit` for a repo-wide sweep of the working tree.
- Automatically: auto-fires after each `/autospec-run` batch unless
  `~/.autospec/no-secaudit.flag` exists.
- As the Phase 4 gate engine (invoked by the implementer, not by a skill call).

## Pipeline

1. **Ensure scanners** — `ensure-tool.sh gitleaks semgrep trivy license-checker`
   (best-effort; a failed install degrades that concern to LLM-only with a loud
   WARN — never silently skipped).
2. **Scan** — run `security-scan.sh --tree` (sweep) or `--diff <base>` (branch).
   Both modes scan a git-gitignore-aware file list (tracked +
   untracked-not-ignored), so `--tree` never rescans its own prior
   `.autospec/security/*.jsonl` output or other ignored trees (`target/`,
   `node_modules/`) as fabricated findings.
   Findings are emitted as gap JSON objects:
   `{gap_id,dimension,severity,file,line,title,body,dedupe_key}`.
3. **LLM triage (Tier A)** — for each finding: confirm real vs false-positive;
   ADD findings scanners miss (PII in logs, prompt-injection sinks where
   untrusted input reaches an LLM call, contextual backdoors). Assign final
   `severity` (`must-fix` / `nice-to-have`).
4. **Report** — write `.autospec/secaudit.md` summarizing findings by dimension
   with file:line and remediation. Loudly list any scanner that fell back to
   LLM-only.
5. **File survivors** — must-fix survivors above threshold are filed as
   `auto-implement,security,priority:high` issues via the gap-filing path so
   they re-enter autospec. `dedupe_key` prevents re-filing across batches.

## Enforcement defaults

| Concern | dimension | Phase 4 gate | Auto-fix |
|---|---|---|---|
| Secrets / credentials | secrets | block | redact + flag rotation |
| Vulns / SQLi / cmd-injection / backdoors | vuln | block | yes, re-scan |
| Prompt injection | injection | advisory (deterministic; LLM triage blocks) | yes, re-scan |
| PII / data leaks | pii | advisory (deterministic; LLM triage blocks) | yes, re-scan |
| IP / copyright / license | license | advisory (deterministic; LLM triage blocks) | advisory fix (human confirms) |
| Dependency CVEs | cve | advisory (block only critical+fixable) | bump if trivial |

## Finalization

Print the report path and a one-line summary:
`secaudit: must-fix=<N> advisory=<N> scanners-degraded=<list>`.
