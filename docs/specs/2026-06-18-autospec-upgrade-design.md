# Design spec — `/autospec-upgrade`: behavior-locked, project-independent framework upgrades

- **Date:** 2026-06-18
- **Status:** design (decompose into auto-implement issues)
- **Origin:** refined via `/autospec-refine` from "automate the framework-upgrade
  workflow a departing engineer did manually (Angular/Next/React)."

## Result first

Build a new reusable autospec skill, **`/autospec-upgrade`**, that upgrades an
arbitrary Angular / Next.js / React repo to the latest official versions
**safely and project-independently** — by locking observable behavior *before*
touching versions, upgrading **one major at a time** with official codemods,
and gating completion on **mutation score**, not coverage theater.

It **composes** existing autospec machinery and builds new code only for the
genuine gaps. The non-negotiable principle: the engineer this replaces had
"coverage" and still shipped broken software, so **line coverage is a floor,
never the gate** — the gate is *behavior-lock + mutation score ≥ pre-upgrade
baseline*.

### Locked decisions (constraints, not open questions)
1. Deliverable is the reusable `/autospec-upgrade` skill, not a one-off migration.
2. Quality gate = E2E/Playwright golden-master behavior-lock **+** Stryker
   mutation score ≥ pre-upgrade baseline. 80% line/branch coverage across
   unit/integration/e2e/Playwright is a **floor**, never sufficient alone.
3. Upgrade strategy = incremental, one major version at a time (mandatory for
   Angular; applied to Next/React too), each hop delegating to **official
   tooling**, each hop a revertable checkpoint (tag + commit).

## Team personality

**Team: Migration & Test-Safety Engineering.** Roles: (1) Build/tooling
engineer (detection, package-manager/runner adapters), (2) Test-safety engineer
(characterization tests, golden-master, mutation gate), (3) Framework migration
specialist (Angular/Next/React official codemods + best-practice schematics),
(4) Release/SRE (tagging, checkpoints, resumability), (5) Docs engineer
(migration log). Fits because the work is *risk-managed change to code we
didn't write and don't fully trust* — the team's instinct is "prove it still
works before and after, and make every step revertable." Risks this team
notices: silent behavior drift, assertion-free tests inflating coverage,
skipped majors, hand-rolled migrations diverging from official codemods,
non-resumable big-bang upgrades, monorepo/workspace blind spots.

### Review counter-team
**Team: Adversarial Reliability & Portability.** Roles: (1) Chaos/edge reviewer,
(2) Cross-stack portability reviewer (pnpm/yarn/bun, Nx/Turborepo, Karma/Vitest),
(3) Determinism reviewer. Challenges: "Does the behavior-lock actually catch a
regression, or just pass vacuously?", "What happens on a repo with **zero**
tests, a private registry, or a non-npm lockfile?", "Is every phase truly
resumable and idempotent, or does a mid-hop crash brick the repo?", "Does the
mutation gate run on a realistic mutant set, or a trivial one?" Stays in scope
by attacking the skill's own contracts (detection JSON, gate thresholds,
checkpoint tags), not by expanding feature scope.

## Architecture

A new top-level skill mirroring the autospec-* family shape. Trio lockstep
(`SKILL.md` authoritative; `codex/prompt.md` + `opencode/agent.md` derived via
`derive-trio.sh --in-place`; goldens via `gen-skill-goldens.sh`), plus
`install.sh` / `uninstall.sh` / `README.md`, and `autospec-block` markers for
the shared startup-self-update + harness-adapter sections. The orchestrator and
helpers live under `skills/autospec-upgrade/scripts/` and install into
`~/.autospec/scripts/`.

### Composition map (reuse — do NOT reinvent)
| Capability | Reused skill / interface |
|---|---|
| Test authoring + coverage + self-heal | `autospec-test` (Stage 2A, `.autospec/test.yml`, `run-gate.sh`) |
| Playwright run + coverage report | `autospec-playwright` |
| No-mock smoke, console/network gate, proof artifacts | `autospec-qa --no-heal` |
| Migration documentation (docs-as-tests) | `autospec-doc --full` |
| Worktree / PR-aware ladder / CI-wait | `autospec-run` |

### New components (the genuine gaps)
1. **Detection** (`upgrade-detect.sh`) → JSON `{frameworks[], versions, package_manager, runners[], monorepo, has_tests}`.
2. **Behavior-lock orchestration** (`behavior-lock.sh`) — drive `autospec-test`/`autospec-playwright`, capture golden-master snapshots, record the Stryker baseline; refuse to proceed until locked.
3. **Mutation gate** (`mutation-gate.sh`) — Stryker adapter across jest/vitest/karma; `--baseline` and `--gate <threshold>` modes; writes `.autospec/mutation-proof.json`. The concrete consumer for tracker #420.
4. **Upgrade engine** (`upgrade-engine.sh` + `compute-upgrade-steps.sh`) — per-major hop loop: official codemod → build/type-check → tests → behavior-lock re-verify → bounded fix-loop → tag+commit.
5. **Codemod routing** (`codemod-route.sh`) — per framework: Angular `ng update` + `ng generate @angular/core:standalone` modes; Next `npx @next/codemod upgrade` + `next-async-request-api`; React `npx codemod react/19/migration-recipe` + `types-react-codemod preset-19`.
6. **Best-practice migration phase** — applied only after versions green, each behind behavior-lock + new tests.
7. **Tagging + report** (`tag-upgrade.sh`) — `pre-upgrade-<fw>-<ver>` / `post-upgrade-<fw>-<ver>`, structured `.autospec/upgrade-report.json`.
8. **Orchestrator** (`upgrade-orchestrator.sh`) — resumable state machine over `.autospec/upgrade-state.json`.

## Phase contract (each phase a checkpoint; resumable; bounded fix-loops; fail-to-operator on unbounded breakage)

- **Phase 0 — Detect.** Emit detection JSON. Next implies React. Explicitly
  handle: zero tests present, non-npm package manager, monorepo with multiple
  apps (operate per-project), private registry (do not block on auth — surface).
- **Phase 1 — Behavior-lock (gate before any upgrade).** Generate
  characterization tests biased to E2E/Playwright golden-master (refactor-robust)
  + unit/integration to the 80% **floor**; record Stryker mutation **baseline**.
  Tag `pre-upgrade-<fw>-<ver>`. Hard rule: no upgrade step runs until behavior
  is locked and the mutation baseline is recorded.
- **Phase 2 — Incremental upgrade loop.** For each major hop (Angular strictly
  one-at-a-time): official codemod/update → build + type-check → run tests →
  re-verify golden-master → bounded fix-loop → tag + commit. "Latest" resolved
  per framework, hop by hop.
- **Phase 3 — Best-practice migration.** Standalone/signals, App Router / async
  request APIs, React 19 patterns — only after versions are green; each behind
  the behavior-lock and accompanied by new tests.
- **Phase 4 — Verify.** `autospec-qa --no-heal` (no-mock smoke + console/network
  gate) + **mutation score ≥ baseline** (hard gate). Coverage floor checked but
  never sufficient alone.
- **Phase 5 — Document.** `autospec-doc` migration log: per-hop what changed,
  before/after, codemods applied, manual fixes, residual risk.
- **Phase 6 — Tag + report.** `post-upgrade-<fw>-<ver>`; push tags; emit report.

## Error handling
- Detection ambiguous / unknown framework → exit with `code_health:upgrade_unknown_stack`, surface to operator; never guess.
- Behavior-lock cannot reach the floor (e.g. app won't boot, no runnable surface) → STOP at Phase 1 with `code_health:upgrade_behavior_lock_unreachable`; do not upgrade blind.
- A hop's fix-loop exceeds its bound (default 5 iterations) → stop at that hop, leave the last green tag intact, surface the failing diff.
- Mutation gate below baseline post-upgrade → WITHHELD; report surviving mutants; do not tag `post-upgrade`.
- Mid-hop crash → resume from `.autospec/upgrade-state.json` at the last completed checkpoint (last tag); never re-run a completed hop.

## Testing
- bats per script: `upgrade-detect` (fixtures for npm/pnpm/yarn/bun × angular/next/react × jest/vitest/karma/playwright/cypress, monorepo, zero-tests), `compute-upgrade-steps` (one-major-at-a-time invariant; Angular never skips), `mutation-gate` (baseline + gate threshold with a mocked Stryker runner), `codemod-route` (correct official command per framework/version with a mocked `npx`/`ng`), `tag-upgrade` (tag format), `upgrade-orchestrator` (resume from each checkpoint; idempotent re-run).
- All subprocess-touching tests mock `npx`/`ng`/`git push`/network via a `$TMP/bin` PATH shim (no real network, no real installs in CI).
- `autospec-qa` revalidation plan applies when dogfooded on a real app (operator/full verification), not in unit CI.
- No-mock rule: the *real* upgrade run uses real tooling; only unit tests mock.

## Decomposition (proposed child issues)
1. Trio skill scaffold + install/uninstall + README + `validate.sh` gate (one trio unit).
2. `upgrade-detect.sh` + tests.
3. `behavior-lock.sh` orchestration (compose autospec-test + golden-master; zero-tests handling) + tests.
4. `mutation-gate.sh` Stryker adapter (jest/vitest/karma; baseline + gate) + tests.
5. `compute-upgrade-steps.sh` (incremental-major invariant) + tests.
6. `codemod-route.sh` — Angular routing + standalone schematics + tests.
7. `codemod-route.sh` — Next.js routing (@next/codemod, async-request-api) + tests.
8. `codemod-route.sh` — React routing (react/19 recipe, types preset) + tests.
9. `upgrade-engine.sh` per-hop loop wiring (codemod→build→test→verify→tag) + tests.
10. Best-practice migration phase wiring + tests.
11. `tag-upgrade.sh` + `upgrade-report.json` + tests.
12. Migration-doc generation (compose autospec-doc) + tests.
13. `upgrade-orchestrator.sh` resumable state machine + SKILL.md prose for the phase contract (trio + goldens) + tests.

## Acceptance criteria
- [ ] `skills/autospec-upgrade/` exists with trio (SKILL.md + codex/prompt.md + opencode/agent.md byte-identical bodies), install.sh, uninstall.sh, README.md, and a passing `validate.sh`.
- [ ] `bash scripts/validate.sh` exits 0 (trio lockstep + goldens for autospec-upgrade pass).
- [ ] `upgrade-detect.sh` emits valid JSON identifying framework, version, package manager, runners, monorepo, and `has_tests` for each fixture repo.
- [ ] `compute-upgrade-steps.sh` never emits a step that skips an Angular major version (test asserts a 20→23 plan is 21,22,23).
- [ ] `behavior-lock.sh` refuses (`exit != 0`) to proceed when behavior is not locked or the mutation baseline is absent.
- [ ] `mutation-gate.sh --gate` fails when post-upgrade mutation score < recorded baseline (test with a mocked Stryker score).
- [ ] `codemod-route.sh` invokes the correct official command per framework/version (asserted against a mocked `ng`/`npx`).
- [ ] `tag-upgrade.sh` creates `pre-upgrade-<fw>-<ver>` and `post-upgrade-<fw>-<ver>` tags in the documented format.
- [ ] `upgrade-orchestrator.sh` resumes from the last checkpoint after a simulated mid-hop crash without re-running a completed hop.
- [ ] No script hand-rolls a migration that an official codemod performs (review-enforced).

## Out of scope (v1)
- Frameworks beyond Angular/Next/React (Vue/Svelte/Solid) — detection returns `unknown` and exits cleanly.
- Auto-resolving genuinely manual structural migrations (e.g. full Pages→App restructure) — surfaced as operator follow-ups, not silently attempted.
- Shipping the broader #420 assertion-density / negative-path lints — this skill ships only the Stryker mutation gate it needs and references #420.
