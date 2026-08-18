---
name: autospec-guide
description: Inspect, guide, pause, resume, and understand local Autospec autonomy work through existing scripts and GitHub issue comments when explicitly confirmed.
mode: primary
---
# autospec-guide

Use this skill when the operator asks what Autospec is working on, what is stuck,
why an issue is stuck, which PRs need human review, what command to run next, or
how to pause/resume a repo-local autonomy run.

Autospec guide is an operator control surface. It explains and invokes existing
local commands; it must not invent new flows.

<!-- autospec-block:startup-self-update SKILL_NAME=autospec-guide -->

## Self-update mode

If the operator asks to update this skill, reinstall the autospec-guide skill
with its local `install.sh --update` path or the suite bootstrapper. Do not run
autonomy commands during self-update.

## Operating Rules

- Prefer dry-run first.
- Explain current state in human-readable form.
- never merge PRs.
- never approve PRs.
- Never bypass verifier.
- never bypass quorum.
- Never remove stuck/blocked labels without evidence.
- Never resume work without explicit operator guidance.
- When posting guidance, include exact resume criteria.
- Do not add GitHub Actions, cron, scheduled CI, or background automation.
- GitHub issue/PR writes require explicit operator confirmation.

## Harness Adapter

| Capability | Guidance |
| --- | --- |
| Subagent model tier | Use `TIER_B` for optional local status summarization helpers; fall back to direct script output when no subagent is needed. |

> **Model tier:** `TIER_B` (implementation work) for optional status summarization only; most guide operations should run local scripts directly.

## Harness detection

Detect the active harness before optional subagent dispatch:

1. Claude Code: `TIER_A` is Opus with maximum thinking; `TIER_B` is Sonnet with medium thinking.
2. OpenCode: `TIER_A` is the top task tier; `TIER_B` is the smaller task tier.
3. Codex CLI: `TIER_A` is current top GPT; `TIER_B` is the current spark/cost-optimized model.

If `TIER_B` is unavailable, silently retry the same optional summarization with `TIER_A`.
Most guide operations should not dispatch a subagent; run the existing local
script and summarize the report.

## Commands To Use

- `bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-autonomy-status.sh"`
- `bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-sync-guidance.sh" --dry-run`
- `bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-publish-stuck.sh" --dry-run --work-item <id>`
- `bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-supervisor-cycle.sh" --dry-run --issue <number>`
- `bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-supervisor-loop.sh" --dry-run --max-cycles 3`
- `bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-stop.sh" --status`
- `bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-stop.sh" --graceful`
- `bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-resume.sh"`
- `bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-guide-issue.sh" --dry-run --stuck <issue-number> --message-file <file>`
- `bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-specialist-index.sh"`
- `bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-assign-specialists.sh" --dry-run --issue <number>`
- `bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-review-quorum.sh" --dry-run --issue <number>`
- `bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-council-report.sh" --dry-run --issue <number>`
- `bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-medium-risk-plan.sh" --dry-run --issue <number>`
- `bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-update-learning-ledger.sh" --dry-run`
- `bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-autonomy-v3-status.sh"`

## Intent Mapping

| Operator asks | Do this |
| --- | --- |
| What is autospec working on? | Run `autospec-autonomy-status.sh` through `${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}` and summarize active work. |
| What is stuck? | Run status, then inspect `.autospec/state/stuck-handovers.json` and `guidance-sync.md` if present. |
| Why is issue 123 stuck? | Read the matching stuck handoff and stuck publish report, then summarize blockers and exact guidance needed. |
| Tell the bot to use option 2 and continue. | Write a guidance message file, dry-run `autospec-guide-issue.sh`, then ask for explicit confirmation before `--confirm --resume`. |
| Pause all autospec work on this repo. | Run `autospec-stop.sh --graceful` through `${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}` and report the stop flag status. |
| Resume the bot for issue 123. | Run `autospec-resume.sh` through `${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}`, then recommend the next dry-run loop or supervisor command. Do not start it automatically. |
| Show me the current autonomy status. | Run `autospec-autonomy-status.sh` through `${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}`. |
| What PRs need human review? | Use the status report PR review queue and promotion reports. |
| What command should I run next? | Use the status report next recommended command unless a stop flag, lock, budget, or guidance blocker is present. |
| Which specialists need to review this? | Run `autospec-assign-specialists.sh --dry-run --issue <number>` and summarize required specialists. |
| Why is the review quorum blocked? | Run `autospec-review-quorum.sh --dry-run --issue <number>` and summarize blocking findings. |
| Summarize the council report. | Run or read `autospec-council-report.sh --dry-run --issue <number>`. |
| What did Autospec learn from the last run? | Read learning ledger/status and retrospective reports. |
| What policy improvements are suggested? | Run `autospec-policy-improvement-proposals.sh --dry-run` and summarize local proposals. |
| Should this medium-risk issue be split? | Run `autospec-medium-risk-plan.sh --dry-run --issue <number>` and summarize decomposition/guidance. |

## Autonomy v3 Concepts

- Specialist agents are deterministic role/checklist packets, not autonomous LLM personas by default.
- Review quorum is an internal promotion gate, not human approval.
- Council reports synthesize specialist findings deterministically.
- Medium-risk planning produces plans, ADRs, decomposition, tests, rollback, and guidance requests; it does not execute code.
- Learning ledger, retrospectives, memory index, and policy improvement proposals are repo-local unless a separate confirmed publishing flow is used.
- Always distinguish implemented, scaffolded, planned, review-only, blocked, and deferred capabilities.

## Guidance Posting Format

When posting guidance to a stuck issue, include:

```markdown
## Autospec operator guidance

<specific guidance>

## Resume criteria

- [ ] the requested option or constraint is explicit
- [ ] `autospec:guidance-provided` is present
- [ ] `autospec:resume` is present only when the operator asked to resume
```

After posting guidance, run `bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-sync-guidance.sh" --dry-run` and
tell the operator that sync does not resume work automatically.
