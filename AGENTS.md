# AGENTS.md

## Engineering standards

- **Conventional commits** (`feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `chore:`).
- **Branch-per-issue**: `feat/<slug>`. Never push to `main`.
- **Never bypass hooks** (`--no-verify`) or signing flags.
- **Never amend** committed PRs; create a new commit instead.
- **Lock-step rule** (per `CONTRIBUTING.md`): every multi-harness skill keeps `SKILL.md` / `opencode/agent.md` / `codex/prompt.md` bodies identical; only frontmatters differ.
- **Validation in lieu of code tests**: this repo has no language-level test runner. Validation is via shell scripts that check lock-step diffs, frontmatter parsing, `bash -n` on install scripts, and file presence. Each PR adds or extends a validation script that passes after the change.

## Subagent model selection (two-tier, cost-aware)

When the workflow dispatches a subagent, choose tier based on the **type of work**, not by phase number alone. Two tiers:

### Tier A — Specification work (top model + extended/maximum thinking)

Used by: research subagents (Phase 1), decomposition subagents (Phase 3 — turning a spec into linked GitHub issues), Phase 3.5 review-and-label subagents (turning issue bodies into model-fit metadata that drives all downstream filtering).

Reasoning: spec/issue quality is the bottleneck. A cheap model here costs you N cheap-implementer cycles correcting it downstream. The orchestrator/user is also typically running on a top model in Phase 2 (design + spec writing); subagents in spec-adjacent phases match that quality.

| Harness     | Preferred model | Thinking budget | Fallback (next-tier UP on unavailability) |
|-------------|-----------------|-----------------|--------------------------------------------|
| Claude Code | `opus` (current Claude Opus — e.g. `claude-opus-4-7`) | `ultrathink` (max thinking budget) | latest available top model |
| Codex CLI   | current top non-spark GPT (e.g. `gpt-5.1` or latest top-tier variant) | `reasoning_effort=high` | latest top variant |
| OpenCode    | top tier configured for `task` agents | provider-equivalent of "high" reasoning | next available |

### Tier B — Implementation work (cheaper model + medium thinking)

Used by: implementer subagents inside Phase 4's `process(ISSUE)` (the one writing code on `feat/*` branches), LGTM-review subagents (the inner-loop self-review of a PR).

Reasoning: implementation follows a well-specified contract from Tier A. The work is mechanical relative to the spec. We run this loop many times per spec, so cheaper-tier amortizes well.

| Harness     | Preferred model | Thinking budget | Fallback (UP on unavailability) |
|-------------|-----------------|-----------------|----------------------------------|
| Claude Code | `sonnet` (current Claude Sonnet — e.g. `claude-sonnet-4-6`) | medium thinking | `opus` → latest |
| Codex CLI   | `gpt-5.1-codex-spark` (or current spark/cost-optimized variant) | `reasoning_effort=medium` | next-larger Codex → latest |
| OpenCode    | smaller-tier task model | medium reasoning | next-larger configured tier |

### Flexibility rule (both tiers)

If the preferred model name is rejected (deprecated, capacity, unauthorized), retry with the next tier **UP** — never silently downgrade below the tier's intent. Never hard-code exact version strings in dispatch code; resolve "current Opus / Sonnet / spark / top GPT" at call time so the skill survives model-family churn.

### Tier assignment by phase (quick reference)

| Phase | Skill(s) | Tier |
|-------|----------|------|
| 1 — Investigate (research) | autospec, autospec-define | A |
| 2 — Brainstorm + design | autospec, autospec-define | (orchestrator only — no subagent dispatch; user invokes skill on top model) |
| 3 — Decompose into issues | autospec, autospec-define | A |
| 3.5 — Review and label | autospec, autospec-define | A |
| classify (per-issue review) | autospec-classify | A |
| 4 — Implementer (process(ISSUE) in worktree) | autospec, autospec-run | B |
| 4 — LGTM self-review | autospec, autospec-run | B |

## Auto-merge authority for auto-implement PRs

Admin-merge `auto-implement` PRs (`gh pr merge <#> --admin --squash --delete-branch`) when:
- All required CI checks pass (slow optional checks pending is acceptable).
- The self-review subagent returned `LGTM`.
- PR closes an `auto-implement` issue from a `feat/*` branch.

## Small-LLM target

Generated child issues are sized for 32B-class local LLMs. Pre-staged context, sectional spec anchors, checkbox AC, one Primary smoke test per inner loop.
