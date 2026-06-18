---
name: autospec-upgrade
description: Use when the user wants to upgrade an Angular, Next.js, or React project to the latest official versions safely — by locking observable behavior before touching versions, upgrading one major at a time with official codemods, and gating completion on mutation score rather than coverage theater.
---

# autospec-upgrade workflow

Run a behavior-locked, project-independent framework upgrade for Angular,
Next.js, and React repos. The skill locks observable behavior before any version
change, upgrades one major at a time via official tooling, and gates completion
on mutation score ≥ pre-upgrade baseline — not line coverage alone.

<!-- autospec-block:startup-self-update SKILL_NAME=autospec-upgrade -->

## Self-update mode

If the feature-request argument matches `update` after trimming and lowercasing,
re-install the full autospec suite from `main`, show the before/after diff if the
harness exposes it, then stop. Do not run the upgrade workflow.

## Required capabilities & harness adapter

| Capability | Claude Code | OpenCode | Codex CLI | Fallback if missing |
| --- | --- | --- | --- | --- |
| Subagent model tier | Tier A: `opus` + ultrathink | Tier A: top-tier `task` + max reasoning | Tier A: current top GPT + `reasoning_effort=high` | Run inline, but keep the same report contract |
<!-- autospec-block:harness-adapter-core -->
| Shell execution | Bash tool | shell tool | shell/apply_patch | Required for upgrade scripts |

**Model tier:** TIER_A for behavior-lock planning and mutation-gate analysis
because silent behavior drift is more expensive than extra review tokens.

## Harness detection

Detect the harness once at skill start:

1. Claude Code: `Agent` with `subagent_type` is available.
   - `TIER_A` = `opus` + ultrathink.
   - `TIER_B` = `sonnet`.
2. OpenCode: `task` tool is available.
   - `TIER_A` = top-tier task model + high reasoning.
   - `TIER_B` = smaller-tier task model + medium reasoning.
3. Codex CLI: `apply_patch` is the primary edit tool.
   - `TIER_A` = current top GPT + `reasoning_effort=high`.
   - `TIER_B` = current cost-optimized Codex model + `reasoning_effort=medium`.

Prefer a Tier A subagent for behavior-lock planning and mutation gate analysis.
If `TIER_A` is unavailable, silently fall back to the next available top-tier
model. If delegation is unavailable, run inline.

## When to use

- When a project is on an outdated major version of Angular, Next.js, or React
  and needs a safe, auditable upgrade path.
- When the team needs behavior-locked characterization tests and a mutation
  baseline before upgrading so regressions are detectable.
- When official codemods exist but the project needs orchestration, resumability,
  and a mutation-score gate rather than a manual migration.

## When not to use

- Do not use against Vue, Svelte, Solid, or other frameworks — detection will
  return `unknown` and the skill exits cleanly with
  `code_health:upgrade_unknown_stack`.
- Do not use when the app cannot be built or its tests cannot be run — the
  behavior-lock phase will surface this immediately as
  `code_health:upgrade_behavior_lock_unreachable`.
- Do not use when the goal is a one-off manual migration rather than a
  reusable, audited, mutation-gated upgrade.

## Composition map

This skill reuses existing autospec machinery. Do NOT reinvent:

| Capability | Reused skill / interface |
|---|---|
| Test authoring + coverage + self-heal | `autospec-test` (Stage 2A, `.autospec/test.yml`, `run-gate.sh`) |
| Playwright run + coverage report | `autospec-playwright` |
| No-mock smoke, console/network gate, proof artifacts | `autospec-qa --no-heal` |
| Migration documentation (docs-as-tests) | `autospec-doc --full` |
| Worktree / PR-aware ladder / CI-wait | `autospec-run` |

## New components (genuine gaps)

Scripts live under `skills/autospec-upgrade/scripts/` and install into
`~/.autospec/scripts/`:

1. **`upgrade-detect.sh`** — emit detection JSON
   `{frameworks[], versions, package_manager, runners[], monorepo, has_tests}`.
2. **`compute-upgrade-steps.sh`** — emit the incremental hop list; Angular
   must never skip a major; Next/React use same one-major-at-a-time invariant.
3. **`codemod-route.sh`** — per-framework codemod dispatch: Angular `ng update`
   + standalone schematics; Next `npx @next/codemod upgrade` + async-request-api;
   React `npx codemod react/19/migration-recipe` + types-react-codemod preset-19.
4. **`behavior-lock.sh`** — drive `autospec-test`/`autospec-playwright`, capture
   golden-master snapshots, record Stryker mutation baseline; refuse to proceed
   until locked.
5. **`mutation-gate.sh`** — Stryker adapter across jest/vitest/karma;
   `--baseline` and `--gate <threshold>` modes; writes
   `.autospec/mutation-proof.json`. Concrete consumer for tracker #420.
6. **`upgrade-engine.sh`** — per-major hop loop: official codemod → build +
   type-check → tests → behavior-lock re-verify → bounded fix-loop (default 5
   iterations) → tag + commit.
7. **`tag-upgrade.sh`** — create `pre-upgrade-<fw>-<ver>` /
   `post-upgrade-<fw>-<ver>` tags; emit structured
   `.autospec/upgrade-report.json`.
8. **`upgrade-orchestrator.sh`** — resumable state machine over
   `.autospec/upgrade-state.json`; resume from the last completed checkpoint
   after a mid-hop crash; never re-run a completed hop.

## Phase contract stub

> **Note:** Full phase prose lands in issue #1184
> (`upgrade-orchestrator.sh` resumable state machine). This stub documents the
> phase names and invariants so downstream issues can reference them correctly.

- **Phase 0 — Detect.** Run `upgrade-detect.sh`. Emit detection JSON. Surface
  unknown stacks or auth-required registries; never block on them.
- **Phase 1 — Behavior-lock.** Generate characterization tests (E2E/Playwright
  golden-master + unit/integration 80% floor); record Stryker mutation baseline.
  Tag `pre-upgrade-<fw>-<ver>`. Hard rule: no upgrade step runs until behavior
  is locked and the mutation baseline is recorded.
- **Phase 2 — Incremental upgrade loop.** For each major hop: run
  `codemod-route.sh` → build + type-check → run tests → re-verify golden-master
  → bounded fix-loop → `tag-upgrade.sh` + commit.
- **Phase 3 — Best-practice migration.** Standalone/signals, App Router/async
  request APIs, React 19 patterns — only after versions green; each behind
  behavior-lock + new tests.
- **Phase 4 — Verify.** `autospec-qa --no-heal` + mutation score ≥ baseline
  (hard gate). Coverage floor checked but never sufficient alone.
- **Phase 5 — Document.** `autospec-doc --full` migration log.
- **Phase 6 — Tag + report.** Push `post-upgrade-<fw>-<ver>` tags; emit
  `.autospec/upgrade-report.json`.

## Error handling

- Detection ambiguous / unknown framework → exit with
  `code_health:upgrade_unknown_stack`; surface to operator; never guess.
- Behavior-lock unreachable (app won't boot, no runnable surface) → STOP at
  Phase 1 with `code_health:upgrade_behavior_lock_unreachable`; do not upgrade
  blind.
- Hop fix-loop exceeds bound → stop at that hop, leave the last green tag
  intact, surface the failing diff for operator action.
- Mutation gate below baseline post-upgrade → WITHHELD; report surviving
  mutants; do not tag `post-upgrade`.
- Mid-hop crash → resume from `.autospec/upgrade-state.json` at the last
  completed checkpoint; never re-run a completed hop.

## Output contract

Return:

- Detection JSON path and detected frameworks/versions.
- Behavior-lock status and mutation baseline recorded.
- Per-hop upgrade log (codemod applied, build/test result, tag created).
- Best-practice migration log.
- Mutation gate result (score vs baseline).
- `autospec-qa --no-heal` verdict path.
- Migration doc path from `autospec-doc --full`.
- Tags pushed and `.autospec/upgrade-report.json` path.
- Remaining risks or operator-action items.

## Stop mode

If the request is exactly `stop` or `stop` plus `--<word>` flags after
normalization, dispatch to:

```bash
bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-stop.sh" <args>
```

Print the helper output and stop. Do not run the upgrade workflow.

## Harness-aware handoff

Loop dispatch uses `lib/autospec-harness-detect.sh` to resolve the active AI
harness and pick the canonical `/autospec --autonomous` invocation form:

- Claude Code → `claude "/autospec" "--autonomous" "$PROMPT"`.
- Codex CLI → `codex exec --skip-git-repo-check "/autospec --autonomous $PROMPT"`.
- OpenCode → `opencode "/autospec" "--autonomous" "$PROMPT"` (best-effort).

Detection order: `AUTOSPEC_HANDOFF_DISPATCHER_KIND` env override → skill-mount
probe → PATH probe. Missing dispatcher exits 3 with
`code_health:loop_handoff_no_dispatcher_for_harness`.
