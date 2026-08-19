---
name: autospec-upgrade
description: Use when the user wants to upgrade an Angular, Next.js, or React project to the latest official versions safely — by locking observable behavior before touching versions, upgrading one major at a time with official codemods, and gating completion on mutation score rather than coverage theater.
mode: primary
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

## Phase contract

The orchestrator (`upgrade-orchestrator.sh`) sequences every phase over a
single resumable state file: `.autospec/upgrade-state.json`. Each phase writes
its checkpoint before the next phase starts. On restart the orchestrator reads
the state file and skips every already-completed phase. A fully-completed run
is a no-op on re-invocation (idempotent). Every phase failure that cannot be
recovered within its bounded fix-loop stops the pipeline immediately, persists
state, and exits non-zero so the operator can inspect and retry.

State file schema (`.autospec/upgrade-state.json`):

```json
{
  "current_phase": "phase2_complete",
  "last_completed_hop": "angular-21",
  "last_green_tag": "post-upgrade-angular-21"
}
```

`current_phase` values in order: `phase0_complete`, `phase1_complete`,
`phase2_complete`, `phase3_complete`, `phase4_complete`, `phase5_complete`,
`phase6_complete`. An absent or unrecognised value means "start from Phase 0".

### Phase 0 — Detect

Run `upgrade-detect.sh --root <root> --out <autospec-dir>`. Emit detection
JSON to `<autospec-dir>/detect.json`. Detection JSON schema:
`{frameworks[], versions, package_manager, runners[], monorepo, has_tests}`.

Explicit handling required:

- **Zero tests present** — emit `has_tests: false`; Phase 1 will surface this
  as `code_health:upgrade_behavior_lock_unreachable`.
- **Non-npm package manager** — emit `package_manager` correctly; codemods
  adapt per manager.
- **Monorepo with multiple apps** — operate per-project; `monorepo: true`.
- **Private registry** — do not block on auth; surface registry info in
  detection JSON and continue.
- **Unknown framework** — emit `frameworks: []`; orchestrator exits with
  `code_health:upgrade_unknown_stack`. Never guess.

On success write checkpoint `phase0_complete` to state file.

### Phase 1 — Behavior-lock (gate before any upgrade)

Run `behavior-lock.sh --detect <detect.json> --root <root> --out
<autospec-dir>`. This phase:

1. Generates characterization tests biased to E2E/Playwright golden-master
   (refactor-robust) plus unit/integration to the 80 % **floor**.
2. Records the Stryker mutation **baseline** to
   `.autospec/mutation-proof.json`.
3. Tags `pre-upgrade-<fw>-<ver>` via `tag-upgrade.sh pre`.

**Hard rule:** no upgrade step in Phase 2 may run until this phase exits 0
and the mutation baseline is recorded. If `behavior-lock.sh` exits non-zero
the orchestrator stops with `code_health:upgrade_behavior_lock_unreachable`
and preserves state so the operator can fix the test surface and resume.

On success write checkpoint `phase1_complete` to state file.

### Phase 2 — Incremental upgrade loop

Run `upgrade-engine.sh --hops <hops.json> --root <root>`. The hops file is
produced by `compute-upgrade-steps.sh`. For each major hop the engine:

1. Checks idempotency: if `post-upgrade-<fw>-<to>` tag already exists, skip.
2. Sets `pre-upgrade-<fw>-<to>` tag (checkpoint before the hop).
3. Runs `codemod-route.sh <fw> <to>` (official tooling only — never
   hand-rolls a migration that an official codemod performs).
4. Runs: build → type-check → tests → `behavior-lock.sh` re-verify.
5. **Bounded fix-loop** (default 5 iterations): on failure, re-run the
   pipeline; if the bound is exceeded, stop at that hop, leave the last green
   tag intact, and exit non-zero so the operator can inspect the failing diff.
6. On success: sets `post-upgrade-<fw>-<to>` tag + commits.

Angular must never skip a major (20→21→22, never 20→22). Next.js and React
apply the same one-major-at-a-time invariant.

"Latest" is resolved per framework, hop by hop, from the computed steps.

If `upgrade-engine.sh` exits non-zero the orchestrator stops, persists the
current state (phase and last green tag), and surfaces the failure to the
operator (exit non-zero). Do not silently continue to Phase 3.

On success write checkpoint `phase2_complete` (with `last_completed_hop` and
`last_green_tag`) to state file.

### Phase 3 — Best-practice migration

Run `best-practice-migrate.sh --detect <detect.json> --root <root> --out
<autospec-dir> --versions-green`. This phase applies framework-idiomatic
modernisation **only after all version hops are green**:

- Angular: standalone component migration (`ng generate @angular/core:standalone`),
  surface Signals adoption and `inject()` migration as operator follow-ups.
- Next.js: async request API migration (`next-async-request-api` codemod),
  surface Pages→App Router restructure as an operator follow-up.
- React: React 19 types codemod (`types-react-codemod preset-19`), surface
  `forwardRef` → ref-as-prop and `use()` hook as operator follow-ups.

Each schematic step is gated: `behavior-lock.sh` re-verify must pass after
each schematic. Manual structural migrations that no official schematic
performs are **surfaced as operator follow-ups, never attempted silently**.

If `best-practice-migrate.sh` exits non-zero the orchestrator stops and
persists state. On success write checkpoint `phase3_complete` to state file.

### Phase 4 — Verify

Run `mutation-gate.sh --gate --out <autospec-dir>` then
`autospec-qa --no-heal --root <root>`. Both must pass:

- **Mutation score ≥ pre-upgrade baseline** (hard gate from
  `.autospec/mutation-proof.json`). If the score regressed, the orchestrator
  stops with state persisted and reports surviving mutants to the operator.
  The `post-upgrade` tag is **withheld** until this gate passes.
- **`autospec-qa --no-heal`**: no-mock smoke + console/network gate + proof
  artifacts. Coverage floor is checked but is never sufficient alone.

If either check exits non-zero the orchestrator stops and persists state
(exit non-zero). On success write checkpoint `phase4_complete` to state file.

### Phase 5 — Document

Run `migration-doc.sh --root <root> --out <root>`. Renders the human
migration log from `.autospec/upgrade-report.json`:

- Per-hop sections: framework, before/after version, codemods applied,
  manual fixes, residual risk.
- Written to `docs/migrations/<date>-upgrade.md`.
- Composes `autospec-doc --full` to finalise the log.

On success write checkpoint `phase5_complete` to state file.

### Phase 6 — Tag + report

Run `tag-upgrade.sh post` for each framework/version to push
`post-upgrade-<fw>-<ver>` tags (withheld until mutation gate passed in
Phase 4). Run `tag-upgrade.sh report` to emit/append to
`.autospec/upgrade-report.json`.

On success write checkpoint `phase6_complete` to state file. A subsequent
re-invocation of the orchestrator reads `phase6_complete` and exits 0
immediately (idempotent no-op).

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
