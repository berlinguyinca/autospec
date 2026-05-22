# Changelog

All notable changes to autospec are documented here.

The format is loosely based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
the repo uses conventional commits (`feat:`, `fix:`, `docs:`, `test:`, `refactor:`).

## [Unreleased]

### Added

#### Phase 2 roadmap — operational hardening (2026-05-22)
- `feat(model-tier): Haiku trial profile + select-model-profile routing` (#417 / #428)
  Routes `reasoning:shallow` / `reasoning:medium` issues to Claude Haiku; `reasoning:deep` stays on Sonnet. Per-profile telemetry tracks quality.
- `feat(heartbeat): repo-scoped heartbeat directory layout` (#416 / #426)
  Restructures `~/.autospec/process-heartbeats/` to `<repo-slug>/<issue>.json`. Closes cross-repo heartbeat collisions.
- `feat(validate): duo-lockstep check for SKILL.md+codex-only skills` (#415 / #425)
  `check_lockstep()` now also runs as a duo (SKILL.md ↔ codex/prompt.md) when `opencode/agent.md` is absent.
- `feat(autospec-run): orchestrator bundle-and-dispatch helper` (#418 / #429)
  Wraps `bundle-static-context.sh` with dynamic suffix → realizes the prompt-caching gains in practice.
- `feat(autospec-run): self-enforcement CI workflow` (#419 / #430)
  Contributor PRs touching `skills/**`, `scripts/**`, `docs/specs/**` go through lint-implementation + validate.sh; blocking violations fail the workflow.
- `feat(autospec-run): telemetry dashboard` (#422 / #431)
  `gen-telemetry-dashboard.sh` emits HTML with cache hit-rate trend, per-role token cost, LGTM first-pass rate, outlier table.
- `fix(autospec-docs): drift gate warn-only on autospec self` (#427)
  Temporary operational fix while over-broad scopes get a coordinated narrow+widen PR.

#### Docs amendment — universal doc generation + drift gate (2026-05-22)
- `feat(autospec-docs): dogfood + lockstep validate.sh` (#374 / #412)
- `feat(autospec-docs): language matrix` (#373 / #411)
- `feat(autospec-docs): synthetic targets + integration harness` (#372 / #410)
- `feat(autospec-docs): drift-gate installers + CI workflow` (#371 / #409)
- `feat(autospec-docs): --init mode + Phase 4 drift gate wiring` (#369 / #408)
- `feat(autospec-docs): AI-as-reviewer + confidence routing` (#368 / #399)
- `feat(autospec-docs): screenshots + mermaid diagrams` (#367 / #383)
- `feat(autospec-docs): llms.txt + manifest + assistant prompt` (#366 / #382)
- `feat(autospec-docs): initial doc generators` (#365 / #381)
- `feat(autospec-docs): reverse-engineer pipeline` (#364 / #380)
- `feat(autospec-docs): self-heal classifier extension` (#363 / #379)
- `feat(autospec-docs): scope parser + drift checker` (#362 / #378)
- `feat(autospec-docs): tree-sitter foundation + per-language queries` (#361 / #377)

#### Prompt caching (2026-05-22)
- `feat(autospec-run): reviewer prompt cache structure` (#404 / #407)
- `feat(autospec-run): telemetry capture + cache-hit summary` (#403 / #406)
- `feat(autospec-run): bundle-static-context.sh + implementer prompt cache structure` (#402 / #405)

#### Pipeline hardening (2026-05-22)
- `feat(autospec-run): adaptive-retry loop in implementer` (#392 / #398)
- `feat(autospec-run): implementer-prompt enrichment + gen-ac-tests` (#391 / #397)
- `fix(autospec-run): batch size + reasoning:deep gating` (#390 / #396)
- `feat(autospec-run): CI-wait sentinel` (#389 / #395)
- `fix(autospec-run): pre-commit lint hook + lint-implementation modes` (#388 / #394)
- `fix(autospec-run): memory-tag frontmatter bootstrap` (#387 / #393)

#### autospec-test v2 — Stage 2.5 invariants extension (2026-05-21 → 2026-05-22)
- `feat(autospec-test): stage 2.5 orchestrator + assertion-shift v2 + SKILL.md` (#351 / #376)
- `test(autospec-test): v2 synthetic targets` (#350 / #375)
- `feat(autospec-test): @autospec/test npm helper library` (#349 / #370)
- `feat(autospec-test): edge-case seed verifier + DB driver shims` (#348 / #359)
- `feat(autospec-test): metric I contract symmetry` (#347 / #357)
- `feat(autospec-test): metric H extended crawler` (#346 / #356)
- `feat(autospec-test): metric G window-contract symmetry` (#345 / #355)
- `feat(autospec-test): metric F structural invariants runner` (#344 / #354)
- `feat(autospec-test): built-in invariant kinds` (#343 / #353)
- `feat(autospec-test): v2 contract extension + JSON Schema` (#342 / #352)

#### autospec-test v1 — unit + E2E coverage gate (2026-05-21)
- `feat(autospec-test): SKILL.md + lockstep validation + docs (phase 10)` (#328 / #340)
- `feat(autospec-run): wire autospec-test into phase 4 (phase 9)` (#327 / #339)
- `test(autospec-test): synthetic targets + language matrix (phase 8)` (#326 / #338)
- `feat(autospec-test): operator wizard (phase 7)` (#325 / #337)
- `feat(autospec-test): mode-ii scoped-prod runtime (phase 6)` (#324)
- `feat(autospec-test): self-heal loop (phase 5)` (#323 / #335)
- `feat(autospec-test): assertion-shift AST classifier (phase 4)` (#322 / #334)
- `feat(autospec-test): stage 2 e2e gate + safety layers (phase 3)` (#321 / #332)
- `feat(autospec-test): stage 1 unit gate (phase 2)` (#320 / #330)
- `feat(autospec-test): contract loader + JSON schema (phase 1)` (#319 / #329)

### Design specs landed

- `docs: autospec Phase 2 roadmap (10 strategic gaps)` (#413)
- `docs: autospec prompt caching design` (#400)
- `docs: autospec pipeline hardening design` (#385)
- `docs: autospec family docs amendment design` (#358)
- `docs: autospec-test v2 design (invariants + window contracts + contract symmetry)` (#333)
- `docs: autospec-test skill design spec` (#317)

### Tracker issues (queued for follow-on `/autospec-define` cycles)

- `#420` Mutation testing (test-of-tests, vacuous-assertion detector, mutmut/Stryker gate)
- `#421` Tooling optimization (deterministic templates, batched classifier, gen-pr-report)
- `#423` Skill C — clone provisioner (autospec-e2e-clone, Mode II infrastructure)
- `#424` Distribution / install UX (npm package, npx, marketplace listings)

---

_Earlier history predates this changelog; see `git log` for full record._
