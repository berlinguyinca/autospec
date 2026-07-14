# Autospec Phase 2 Roadmap — Self-Enforcement, Skill C, Mutation Testing, Tooling Optimization, Distribution

**Status:** Draft design (2026-05-22)
**Author:** berlinguyinca + strategic review
**Scope:** Comprehensive roadmap addressing 10 gaps identified in the post-Phase-1 strategic review. Decomposed into 10 sections; each becomes an /autospec-split phase.

## 1. Goal & non-goals

### Goal
Close the strategic gaps remaining after the Phase 1 build-out (autospec-test v1/v2 + pipeline hardening + prompt caching + docs amendment, 47 PRs shipped). Phase 2 covers: Skill C clone provisioner (unblocks Mode II), mutation testing (closes the vacuous-truth gap class), tooling optimization (deterministic templates for token savings), autospec self-enforcement (gates contributor PRs), telemetry dashboard, distribution UX, and several small hardening fixes (lockstep duo, heartbeat scoping, model tier trial).

### Non-goals
- Reworking the existing skill family architecture
- Multi-host coordination
- Cross-repo orchestration of features spanning multiple repos
- Windows-first support (Unix-only commitment; PowerShell variants only where critical)

## 2. Phased decomposition

10 phases, ordered by dependency + value-density. Each phase → one or two GitHub issues for `/autospec-split`. Some phases are small (≤1 PR); others are major (multi-PR sub-chains, written as their own spec via Phase 2.X → /autospec-define).

| # | Phase | Size | Type | Depends |
|---|---|---|---|---|
| 1 | `validate.sh` duo-lockstep fix | 1 PR | Small fix | none |
| 2 | Heartbeat directory repo-scoping | 1 PR | Small fix | none |
| 3 | Implementer model tier — Haiku trial | 1 PR | Config + telemetry probe | Caching telemetry (#403) merged ✓ |
| 4 | Orchestrator monitor uses `bundle-static-context.sh` | 1 PR | Wire-up of existing bundler | Caching #402 merged ✓ |
| 5 | autospec self-enforcement CI workflow | 1–2 PRs | Workflow + gate | Hardening shipped ✓ |
| 6 | Mutation testing — vacuous-assertion detector + Stryker/mutmut gate | follow-on spec | Major new feature | Lint-implementation #388 ✓ |
| 7 | Tooling optimization — gen-issue-skeleton, batched-classifier, gen-pr-report templates | follow-on spec | Major refactor | Decomposer + classifier paths ✓ |
| 8 | Telemetry dashboard — JSONL → HTML summary + historical trend | 2–3 PRs | Visualization layer | Telemetry capture #403 ✓ |
| 9 | Skill C — clone provisioner (Mode II infrastructure) | follow-on spec | **Major new skill** | edge_case_seeds contract from autospec-test v2 ✓ |
| 10 | Distribution / install UX — npm package, `npx autospec init`, marketplace listing | follow-on spec | Adoption play | install.sh ✓ |

Phases 6, 7, 9, 10 are heavy enough to warrant their own design specs (filed via `/autospec-define` after this roadmap lands).

## 3. Phase 1 — validate.sh duo-lockstep fix

**Problem:** `validate.sh check_lockstep()` guards on all 3 trio files (`SKILL.md` + `opencode/agent.md` + `codex/prompt.md`) being present. Skills shipping only 2 (e.g., `autospec-test` has no `opencode/agent.md`) fall through the check silently. Body divergence between `SKILL.md` and `codex/prompt.md` goes undetected.

**Fix:** add a duo-mode to `check_lockstep()`: when `opencode/agent.md` is absent but the other two are present, still byte-diff SKILL.md ↔ codex/prompt.md (after frontmatter stripping).

**Implementation outline:**
- `autospec validate`: extend `check_lockstep()` with the duo branch
- bats coverage: trio-pass, duo-pass, duo-divergence-fail fixtures

**Acceptance:** the divergence that PR #412 review caught manually is now caught by validate.sh.

## 4. Phase 2 — Heartbeat directory repo-scoping

**Problem:** `~/.autospec/process-heartbeats/<issue>.json` is a flat directory shared across repos. When two repos have overlapping issue numbers (which is common), heartbeats from one bleed into the other's monitor view. Confirmed bug — saw `codingsandmore/vacuum-clamping-system` heartbeats appearing in autospec's local view.

**Fix:** path-scope per repo. New layout: `~/.autospec/process-heartbeats/<repo-slug>/<issue>.json` where `<repo-slug>` is derived from `gh repo view --json nameWithOwner`. Watchdog + monitor read only their own repo's subdir.

**Migration:** any pre-existing flat-format heartbeats older than 1 hour get deleted on first watchdog tick post-update; newer ones get classified by inspecting their `repo` field and moved to the correct subdir.

## 5. Phase 3 — Implementer model tier (Haiku trial)

**Problem:** TIER_B = `sonnet` for implementer dispatches. Now that hardening shipped (pre-commit lint + AC-bats verify + adaptive retry), the implementer is much more constrained. Haiku might handle most dispatches at 30–50% of sonnet's cost.

**Fix:** add a `claude-haiku-cloud` profile to `~/.autospec/model-profiles.yml` with `ctx: 64k, reasoning: medium`. Switch monitor to use it for `reasoning:shallow` and `reasoning:medium` issues; sonnet stays for `reasoning:deep`. Telemetry tracks quality (LGTM-first-pass rate, iteration count, time-to-merge) per profile.

**Acceptance:** at least 20 issues processed under Haiku; LGTM-first-pass rate within 10% of sonnet baseline; per-issue token cost drops ≥40%.

**Rollback:** if quality drops more than 20%, revert default back to sonnet via single config line.

## 6. Phase 4 — Orchestrator wires up `bundle-static-context.sh`

**Problem:** Caching infrastructure (#402) shipped — `bundle-static-context.sh` exists, the SKILL.md amendments document the two-part prompt structure. But the orchestrator (the agent dispatching subagents via `Agent` tool) still constructs prompts inline without calling the bundler. Net: cache infrastructure is built but unused by the current launch path.

**Fix:** add an orchestrator-side helper script `bundle-and-dispatch.sh` that:
1. Calls `bundle-static-context.sh --role <role> --issue-labels <labels>` to assemble the cached prefix
2. Constructs the Anthropic API call with `cache_control: { type: "ephemeral" }` on the prefix block
3. Appends the dynamic suffix (issue body, branch name)
4. Dispatches via the Agent tool

The wrapper monitor and orchestrator both call this helper. Cache hits start materializing within the same session.

**Acceptance:** telemetry shows `cache_read_input_tokens > 5000` for every dispatch after the first within a 5-min window.

## 7. Phase 5 — autospec self-enforcement CI

**Problem:** autospec's own contributor PRs can land without going through `/autospec-run`. Two implications: human contributors bypass the gates that autospec enforces on target repos; and there's no CI proof that autospec-run *actually works* on autospec-shaped PRs.

**Fix:** add `.github/workflows/autospec-self-enforce.yml` to autospec itself that, on every PR opened against `main`:
- Detects whether the PR touches autospec-protected paths (`skills/**`, `scripts/**`, `docs/specs/**`)
- Runs the autospec-run Phase 4 implementer's QA chain (lint-implementation, build, validate, drift gate) against the PR diff
- Posts a structured comment with findings
- Fails the workflow if any blocking finding

**Bootstrap exception:** the PR introducing this workflow itself gets `docs: skip` style escape since it can't pre-satisfy gates it's creating.

**Acceptance:** open a deliberately-bad PR (e.g., introduces an EVAL_USER_INPUT violation) and confirm the self-enforce workflow catches it.

## 8. Phase 6 — Mutation testing (follow-on spec)

This phase ships as `docs/specs/<DATE>-autospec-mutation-testing-design.md` via `/autospec-define`. Scope:

- **Vacuous-assertion detector** in `lint-implementation.sh`: catches `grep -qv "X" || true`, `expect(true).toBe(true)`, `assert(1 === 1)` patterns, AC bats stubs that still `skip "auto-stub"`. Per saved memory `project_autospec_mutation_testing`.
- **Stryker/mutmut/go-mutesting gate** in Phase 4 QA. Start scoped to `area:safety` and `area:hardening` issues. Run only against changed files. Gate at ≥80% mutants caught.
- **Assertion-density floor**: minimum assertions per test, minimum tests per public function. Pre-commit lint rule.
- **Negative-path coverage**: every "should succeed" test pairs with "should fail" sibling. Per-language linter heuristic.

Defer detailed design to the follow-on spec. This phase just files the umbrella tracker.

## 9. Phase 7 — Tooling optimization (follow-on spec)

Ships as `docs/specs/<DATE>-autospec-tooling-optimization-design.md`. Scope per saved memory `project_autospec_tooling_optimization`:

- `gen-issue-skeleton.sh` — template-driven issue body generation; LLM only fills Goal + Implementation outline
- `classify-model-fit.sh` — deterministic ctx/reasoning rubric (file count + verb keywords); LLM escalation only at low confidence
- `gen-pr-report.sh` — 100% template-driven PR comment from gate JSON
- `gen-implementer-prompt.sh` — deterministic implementer prompt assembly from issue body
- Extended `lint-implementation.sh` covering more RULE_IDs

Expected token savings: 30–60% on decomposer + reviewer + report paths.

## 10. Phase 8 — Telemetry dashboard

**Problem:** `~/.autospec/telemetry.jsonl` accumulates records from caching telemetry (#403). Summary helper exists but produces text. No visualization, no historical trend, no per-feature breakdown.

**Fix:** ship a static-HTML dashboard generator at `$AUTOSPEC_SCRIPTS_DIR/gen-telemetry-dashboard.sh`:
- Reads `~/.autospec/telemetry.jsonl`
- Emits `~/.autospec/telemetry-dashboard.html` with:
  - Cache hit-rate over time (line chart, daily aggregate)
  - Per-role token-cost breakdown (implementer / reviewer / decomposer / classifier)
  - LGTM-first-pass rate (quality metric)
  - Per-issue cost outliers (top 10 expensive issues)
- Optionally publishes to `gh-pages` branch as a public dashboard

**Implementation outline:**
- `gen-telemetry-dashboard.sh` (new, ~150 lines bash + small embedded HTML/JS)
- `tests/gen-telemetry-dashboard.bats` — fixture jsonl → expected HTML structure
- README update

## 11. Phase 9 — Skill C clone provisioner (follow-on spec)

Ships as `docs/specs/<DATE>-autospec-e2e-clone-design.md`. Scope:

- **Snapshot** — DB drivers (postgres / mysql / sqlite) + filesystem (ZFS, raw `cp` for small) + S3 (`aws s3 sync`). Per-target declarable.
- **Anonymize** — declarative redaction rules (`anonymize.yml`) covering: hash-personally-identifying-fields, scrub-emails-to-domain, replace-credit-cards, randomize-timestamps-within-window. Reversible mappings stored in `.autospec/anonymize-map.<sha>.json` so test runs can still join across tables.
- **Scale-down** — for multi-TB datasets: subset sampling (every Nth row, or stratified sample by key column), foreign-key-aware reachability (include rows referenced by sampled rows).
- **Edge-case seeding** — consumes `edge_case_seeds.require_shapes` from autospec-test contract; INSERTs synthetic rows matching each shape predicate from the catalog.
- **Expose URL** — provision a routable URL for Playwright + writes `.autospec/clone-url.txt` for the gate to read. Could be: local docker-compose, ephemeral k8s namespace, dedicated staging slot.

This is a **major new skill** — `autospec-e2e-clone`. Decomposed via its own /autospec-split, likely 8–12 phases.

## 12. Phase 10 — Distribution / install UX (follow-on spec)

Ships as `docs/specs/<DATE>-autospec-distribution-design.md`. Scope:

- **npm package `@autospec/cli`** — installs all skills + shared scripts; provides `npx autospec init`, `npx autospec install`, `npx autospec status`
- **Homebrew formula** for macOS users
- **Marketplace listings**: Claude Code skill marketplace, Codex CLI prompts catalog, OpenCode agent registry
- **Quickstart documentation** — 5-minute "first contact" guide covering a single example target repo end-to-end
- **Public landing site** — github.io repo or similar with curated docs (could reuse the docs amendment's USER_MANUAL.md / API_REFERENCE.md / ARCHITECTURE.md as inputs)

## 13. Cross-cutting acceptance (final gate for Phase 2 completion)

- [ ] All 10 phases shipped (specs landed for phases 6/7/9/10 + their downstream issues)
- [ ] autospec self-enforces — contributor PRs against autospec must pass autospec-run
- [ ] Mode II usable end-to-end against at least one real target (proves Skill C works)
- [ ] Telemetry dashboard generates HTML reliably
- [ ] Distribution: `npx autospec init` works against an empty Node repo
- [ ] Mutation testing catches at least one synthetic vacuous-truth violation in a synthetic target
- [ ] Token cost per-issue drops ≥30% from current sonnet baseline (Haiku trial validated)

## 14. Decision log

| Q | Decision | Rationale |
|---|---|---|
| One mega-spec or 10 small specs? | One mega-roadmap + 4 follow-on specs for the heavy phases (6/7/9/10) | Roadmap captures dependency edges + ordering; heavy phases need their own design depth |
| Bootstrap exception for self-enforce? | Yes (`docs: skip` equivalent for the introducing PR) | Same pattern as the docs-amendment bootstrap |
| Default tier change to Haiku safe? | Conditional on telemetry validation | Adaptive-retry + pre-commit lint protect against quality drop; rollback is one config line |
| Skill C as a v1 of a new family or extension of autospec-test? | New family (`autospec-e2e-clone`) | Cleanly separate concern; autospec-test consumes its URL contract |

## 15. Open follow-ups (NOT in this roadmap)

- Multi-repo orchestration (cross-cutting features)
- Windows-first support
- Language-aware static analysis (mypy / Clippy / SpotBugs integration)
- AI-generated security review (beyond the existing `security-review` skill)
- Cross-session prompt cache layer (Anthropic ephemeral is 5-min TTL; a Redis-backed extension could be future work)
