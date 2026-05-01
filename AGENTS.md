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

Spec PRs (head branch matches `feat/spec-*` OR body contains a `Source spec` line
referencing `docs/specs/`) carry the same admin-merge authority: orchestrators run
`gh pr merge <#> --admin --squash --delete-branch` once required CI checks pass and
the body matches one of those criteria.
Escape hatch: set `AUTOSPEC_NO_AUTOMERGE_SPEC=1` to short-circuit the auto-merge and
fall back to "open PR + ask user".

## Startup self-update

Every multi-harness skill runs a preflight at startup that updates the installed copy
from `main` at most once per 24 hours (fail-open: any network or install error logs a
`WARN:` line and continues). Set `AUTOSPEC_NO_SELF_UPDATE=1` to skip. The canonical
bash block lives in `skills/autospec/SKILL.md` (`## Startup self-update` section) and
is mirrored byte-identically (modulo `SKILL_NAME=`) across all five skill trios.
`scripts/validate.sh` (`check_startup_preflight`) enforces byte-identity.

## Small-LLM target

Generated child issues are sized for 32B-class local LLMs. Pre-staged context, sectional spec anchors, checkbox AC, one Primary smoke test per inner loop.

## Anti-loop guardrails

Per spec §5.1, both the Phase 1 research subagent and the Phase 4
implementer subagent run under hard, no-wall-clock-cap limits to keep a
runaway model from burning tokens or getting stuck rewriting the same
file forever:

- **Phase 1 research subagent.** Max **25 tool calls**. If 3 consecutive
  read/grep calls return nothing useful, stop and write a best-effort
  summary even if it is incomplete. Never retry the same query verbatim.
- **Phase 4 implementer subagent.** Max **40 tool calls** per issue. Max
  **3 self-review iterations**. If the implementer rewrites the same
  file twice with no test progress, abort: comment the blocker on the
  issue, release the `locked-by-autospec-processor` label, and exit.
- **No wall-clock cap.** Both limits are tool-call / iteration based,
  not time based, so stalled work is detected by behavior, not clock
  time.
- **Where they live.** These limits live inline in
  `skills/autospec/SKILL.md` (Phases 1 and 4) and
  `skills/autospec-run/SKILL.md` (Phase 4). The lock-step rule
  replicates the same body to `opencode/agent.md` and
  `codex/prompt.md`.

## Listener-filed issues lifecycle

Per spec §4.1 and §5.3, issues filed by `autospec-listen` follow a
distinct two-step lifecycle on the way to the `auto-implement` queue:

- **Step 1: listener creates with `needs-classify`.** When the listener
  fires on an issue trigger and the user confirms, the resulting
  `gh issue create` call carries `--label needs-classify` (color
  `#fbca04`, idempotently created via
  `gh label create needs-classify --color fbca04 --force`). The issue
  is NOT yet on the implementation queue — it is a draft awaiting
  classification.
- **Step 2: classifier transitions to `auto-implement`.**
  `/autospec-classify` walks BOTH `auto-implement` AND `needs-classify`
  issues. After applying `ctx:*` / `reasoning:*` labels and inserting
  the `## Model fit` block, on any issue carrying `needs-classify` it
  ALSO performs:
  `gh issue edit <N> --add-label auto-implement --remove-label needs-classify`.
  Issues that already carried `auto-implement` (and not
  `needs-classify`) are re-classified in place; no label transition.
- **No auto-promotion.** There is no TTL-based promotion. Stuck
  `needs-classify` issues are swept by re-running `/autospec-classify`
  manually or via the sample crontab in
  `docs/runbooks/needs-classify-sweep.md`.
