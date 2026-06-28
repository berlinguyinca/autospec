# autospec-guide

Use this skill when the operator asks what Autospec is working on, what is stuck,
why an issue is stuck, which PRs need human review, what command to run next, or
how to pause/resume a repo-local autonomy run.

Autospec guide is an operator control surface. It explains and invokes existing
local commands; it must not invent new flows.

## Operating Rules

- Prefer dry-run first.
- Explain current state in human-readable form.
- never merge PRs.
- never approve PRs.
- Never bypass verifier.
- Never remove stuck/blocked labels without evidence.
- Never resume work without explicit operator guidance.
- When posting guidance, include exact resume criteria.
- Do not add GitHub Actions, cron, scheduled CI, or background automation.
- GitHub issue/PR writes require explicit operator confirmation.

## Commands To Use

- `bash scripts/autospec-autonomy-status.sh`
- `bash scripts/autospec-sync-guidance.sh --dry-run`
- `bash scripts/autospec-publish-stuck.sh --dry-run --work-item <id>`
- `bash scripts/autospec-supervisor-cycle.sh --dry-run --issue <number>`
- `bash scripts/autospec-supervisor-loop.sh --dry-run --max-cycles 3`
- `bash scripts/autospec-stop.sh --status`
- `bash scripts/autospec-stop.sh --graceful`
- `bash scripts/autospec-resume.sh`
- `bash scripts/autospec-guide-issue.sh --dry-run --stuck <issue-number> --message-file <file>`

## Intent Mapping

| Operator asks | Do this |
| --- | --- |
| What is autospec working on? | Run `scripts/autospec-autonomy-status.sh` and summarize active work. |
| What is stuck? | Run status, then inspect `.autospec/state/stuck-handovers.json` and `guidance-sync.md` if present. |
| Why is issue 123 stuck? | Read the matching stuck handoff and stuck publish report, then summarize blockers and exact guidance needed. |
| Tell the bot to use option 2 and continue. | Write a guidance message file, dry-run `autospec-guide-issue.sh`, then ask for explicit confirmation before `--confirm --resume`. |
| Pause all autospec work on this repo. | Run `scripts/autospec-stop.sh --graceful` and report the stop flag status. |
| Resume the bot for issue 123. | Run `scripts/autospec-resume.sh`, then recommend the next dry-run loop or supervisor command. Do not start it automatically. |
| Show me the current autonomy status. | Run `scripts/autospec-autonomy-status.sh`. |
| What PRs need human review? | Use the status report PR review queue and promotion reports. |
| What command should I run next? | Use the status report next recommended command unless a stop flag, lock, budget, or guidance blocker is present. |

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

After posting guidance, run `bash scripts/autospec-sync-guidance.sh --dry-run` and
tell the operator that sync does not resume work automatically.
