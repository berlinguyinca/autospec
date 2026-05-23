---
name: Autospec decomposer + lockstep gotchas (new-skill creation)
description: Non-obvious failure modes when filing autospec issues that create a brand-new multi-harness skill — decomposer must scope first issue to include structural sections, codex/prompt.md needs leading blank line for lockstep
type: feedback
wing: synthesis
drawer_class: lesson
originSessionId: 7205d05b-f2fd-4cde-9ced-ecc266a9bc7b
---
When the autospec Phase 3 decomposer (or you, manually) splits a "new multi-harness skill" feature into child issues, two non-obvious traps consistently cause cascade failures. Both surfaced on 2026-05-01 during the autospec-listen rollout (issues #36, #42, #101).

## Trap 1 — `discover_skills` is auto-discovery; the first issue MUST land structurally complete

`scripts/validate.sh` discovers any subdir of `skills/` that has all 3 trio files: `SKILL.md`, `opencode/agent.md`, `codex/prompt.md`. The instant the third file exists, validate.sh starts running these checks against the skill's SKILL.md:
- `## Self-update mode` heading present
- `**Model tier:** Tier A (spec work)` or `**Model tier:** Tier B (implementation work)` directive present at least once
- A `| Subagent model tier |` row inside the harness adapter table

If the **first** file-creation issue (typically titled "skill skeleton + SKILL.md frontmatter") is scoped to only add frontmatter + a placeholder body, validate.sh will pass when that issue merges (still 2 of 3 trio files), but the **second** trio-file issue will trip the auto-discovery and fail validate.sh — with the implementer subagent unable to "expand scope" to fix the structural gap, blocking 5+ downstream issues.

How to apply: When decomposing a new-skill feature, the **first** child issue MUST be scoped to include the 3 structural elements above. Phrasing in the issue body: "Implementation scope: SKILL.md frontmatter + intro + Self-update mode + Required capabilities table including the `| Subagent model tier |` row + a `**Model tier:**` directive on at least one section." Do not split structural sections across multiple issues. Recovery if you've already filed and merged the under-scoped skeleton: file a hotfix PR (NOT a new issue) that backfills the structural sections directly, since the next dependent issue will be blocked.

## Trap 2 — `codex/prompt.md` needs a leading blank line for lockstep

`check_lockstep` in scripts/validate.sh diffs `strip_body(SKILL.md)` against `cat codex/prompt.md`. `strip_body` strips the YAML frontmatter (lines between the two `---`) but PRESERVES the blank line that follows the closing `---`. So:

- SKILL.md after frontmatter strip: `\n# autospec-listen ...` (leading blank)
- codex/prompt.md must start with: `\n# autospec-listen ...` (leading blank)

If you write codex/prompt.md starting directly with the `#` heading and no leading blank, lockstep fails with `1d0 < ` (one-line missing-blank diff).

How to apply: Always start a new codex/prompt.md with a single leading blank line before the `#` heading. Same applies if you're hand-syncing the trio (lock-step replication scripts handle this automatically; manual edits don't).

Why memorize: Both traps are silent until validate.sh runs against the now-discovered skill. The Phase 3 decomposer prompt in skills/autospec/SKILL.md does NOT currently warn about either, so the next "new skill" feature will hit them again unless the decomposer prompt is updated. Until that fix lands, surface these to the user pre-emptively when reviewing a decomposition.

## Trap 3 — process(ISSUE) prompt must explicitly teach lock-step sync for SKILL.md edits

When a child issue targets an EXISTING skill's SKILL.md (not creating a new skill), the implementer subagent still needs explicit lock-step sync instructions. validate.sh enforces:
- `SKILL.md body (after frontmatter strip) == codex/prompt.md (raw)`
- `SKILL.md body == opencode/agent.md body (after frontmatter strip)`

Without explicit instructions in the process(ISSUE) prompt, Sonnet-tier implementers discover this constraint mid-implementation (when running validate.sh for the first time), often exit with a message like `"SKILL.md body must match codex/prompt.md"` rather than just fixing it.

How to apply: In every process(ISSUE) dispatch that targets a SKILL.md file, prepend this instruction before STEP 2:
```
⚠️ LOCK-STEP RULE: After editing skills/<name>/SKILL.md, you MUST also:
  1. Write raw body (SKILL.md with frontmatter stripped) to skills/<name>/codex/prompt.md
  2. Keep skills/<name>/opencode/agent.md frontmatter intact, replace its body with the same raw body
Run `bash scripts/validate.sh` to verify before committing. Fix any diff before proceeding.
```

This applies even for simple text changes (adding a section, replacing a string pattern) — the trio must stay in sync or CI fails.

Confirmed on 2026-05-07 during harness-aware model-tier session: second monitor launch exited mid-work on 4 SKILL.md issues because the process(ISSUE) prompt lacked this instruction. Third launch with the instruction merged 3/4 cleanly.
