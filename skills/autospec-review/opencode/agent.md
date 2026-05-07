---
description: Audit design specs against open + closed issues to find gaps, file high-priority regression issues, and feed them back through autospec. Runs as `/autospec-review` (manual) or auto-fires after each autospec-run batch unless `~/.autospec/no-review.flag` exists.
mode: primary
---

<!-- BODY START -->
## Self-update mode

Body filled in Tasks 13–15 (skill scaffolding only at this stage).

## Required capabilities & harness adapter

| Capability                  | Claude Code                          | OpenCode                                 | Codex CLI                                | Fallback if missing                                |
|-----------------------------|--------------------------------------|------------------------------------------|------------------------------------------|----------------------------------------------------|
| Subagent model tier          | Tier B: `sonnet` + medium thinking   | Tier B: smaller-tier `task` + medium reasoning | Tier B: `gpt-5.1-codex-spark` + `reasoning_effort=medium` | Fall back UP on unavailability |

**Model tier:** Tier B (implementation work) — scaffolding placeholder; Tasks 13–15 fill the real Phase 0–7 body.
<!-- BODY END -->
