# AGENTS.md

## Engineering standards

- **Conventional commits** (`feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `chore:`).
- **Branch-per-issue**: `feat/<slug>`. Never push to `main`.
- **Never bypass hooks** (`--no-verify`) or signing flags.
- **Never amend** committed PRs; create a new commit instead.
- **Lock-step rule** (per `CONTRIBUTING.md`): every multi-harness skill keeps `SKILL.md` / `opencode/agent.md` / `codex/prompt.md` bodies identical; only frontmatters differ.
- **Validation in lieu of code tests**: this repo has no language-level test runner. Validation is via shell scripts that check lock-step diffs, frontmatter parsing, `bash -n` on install scripts, and file presence. Each PR adds or extends a validation script that passes after the change.

## Subagent model selection (cost-aware)

When the workflow dispatches a subagent (research / decomposition / Phase 3.5 reviewer / self-review / implementer / monitor), prefer the **cheaper available tier** in the active harness, **NOT** the orchestrator's own primary model. The orchestrator stays on whatever model the user invoked the skill with.

**Default tier per harness:**

| Harness     | Preferred subagent model | Fallback chain (use the next tier UP if the preferred is at capacity, deprecated, or unavailable) |
|-------------|--------------------------|----------------------------------------------------------------------------------------------------|
| Claude Code | `sonnet` (current Claude Sonnet — e.g. `claude-sonnet-4-6` or whatever the current Sonnet ID is) | `opus` → `<latest available>` |
| Codex CLI   | `gpt-5.1-codex-spark` (or the current "spark"/cost-optimized variant) | next-larger Codex variant → `gpt-5.1` → `<latest available>` |
| OpenCode    | smallest production tier configured for `task` agents (provider-dependent) | next-larger configured tier |

**Thinking / reasoning:** always set the **medium** thinking budget when the harness exposes the knob (Claude Code thinking levels, Codex `reasoning_effort`, OpenCode equivalent). Do not request `ultrathink` / `high` reasoning unless a specific issue body explicitly demands it.

**Flexibility rule:** if the preferred model name is rejected (deprecated, capacity, unauthorized), retry with the next tier UP — never silently downgrade to a smaller model. Never hard-code an exact version string in dispatch code; resolve "current Sonnet" / "current spark" at call time so the skill survives model-family churn.

This is a global preference; every phase that dispatches a subagent honors it.

## Auto-merge authority for auto-implement PRs

Admin-merge `auto-implement` PRs (`gh pr merge <#> --admin --squash --delete-branch`) when:
- All required CI checks pass (slow optional checks pending is acceptable).
- The self-review subagent returned `LGTM`.
- PR closes an `auto-implement` issue from a `feat/*` branch.

## Small-LLM target

Generated child issues are sized for 32B-class local LLMs. Pre-staged context, sectional spec anchors, checkbox AC, one Primary smoke test per inner loop.
