# autospec-playwright — disciplined no-mock Playwright UI-test authoring

- **Date:** 2026-06-04
- **Status:** approved for decomposition
- **Relation to prior specs:** folds in and supersedes-by-recency the unimplemented
  `docs/specs/2026-05-27-playwright-control-effect-coverage-design.md` (landed as docs
  via PR #611, never decomposed). Extends `docs/specs/2026-05-21-autospec-test-design.md`
  (v1) and `2026-05-21-autospec-test-invariants-design.md` (v2). Consumes the
  environment contract of `2026-05-22-autospec-e2e-clone-design.md`.

## 1. Goal

Add a capability that **generates** Playwright UI tests which drive the actual
application in a real browser against a real backend, asserting on rendered DOM
and persisted state — under a deterministic, lint-enforced discipline: no
mocking ever, selector-source verification, strict-mode locators, broad
surface coverage with an explicit coverage percentage, and a
test-bug-vs-app-bug triage rule that never weakens assertions.

This is the **authoring** counterpart to autospec-test's existing **gating**
machinery. The gate answers "is coverage sufficient?"; this stage answers
"write the disciplined tests that make it sufficient."

## 2. Team personality

- **Team:** Frontend/product — frontend developer, UX designer, accessibility
  reviewer, API/backend developer, QA engineer.
- **Why:** the generated artifact is UI tests; the dominant risks are
  UI-shaped — hallucinated selectors, strict-mode collisions, route coverage
  blind spots, auth bootstrap, a11y-role locator correctness.
- **Risks this team is expected to notice:** invented `data-testid`s, locators
  that pass in isolation but collide under strict mode, empty-state vs
  populated-state confusion, races before refetch, native HTML5 validation
  pre-empting JS handlers.
- **Emphasis carried into child issues:** every selector must be traceable to
  component source; every write asserted both in DOM and via the real API.

### Review counter-team

- **Counter-team:** Contract & operations review — API-contract reviewer,
  security/ops advisor, maintainer.
- **Blind spots to challenge:** (i) the generated reset endpoint must be
  unreachable in production-like deployments (env-gate + forbidden-URL rails);
  (ii) lint regexes must not be greenwashable (string-concat evasion of the
  mock-import ban); (iii) shared-helper centralization must not become a
  single mutable file every fan-out author edits; (iv) coverage-% arithmetic
  must come from the crawler manifest, not the author's self-report.
- **Scope discipline:** review stays inside each issue's stated scope while
  applying that lens.

## 3. Architecture (locked: thin skill + autospec-test stage)

Two deliverables, zero duplicated machinery:

1. **`skills/autospec-playwright/`** — a **thin operator-facing dispatcher**
   (full lockstep trio: `SKILL.md` + `codex/prompt.md` + `opencode/agent.md`,
   plus `install.sh`/`uninstall.sh`/`validate.sh`/`README.md`). It contains
   NO authoring machinery. It: detects/initializes `.autospec/test.yml`
   (`e2e.authoring` + `e2e.reset` blocks), then invokes the autospec-test
   authoring stage below, then prints the coverage report. Registered in the
   root `autospec validate` skill arrays and `install.sh` usage strings.
2. **autospec-test "Stage 2A — disciplined authoring"** — a new stage between
   Stage 1 (unit gate) and Stage 2 (E2E gate) in
   `skills/autospec-test/SKILL.md`, gated on `e2e.authoring.enabled`. All new
   scripts live under `skills/autospec-test/scripts/`. It reuses, unchanged:
   `ui-crawler.mjs` + `crawler-v2/` (surface enumeration),
   `playwright-config-resolver.mjs` (URL contract),
   `adapters/playwright.mjs`, the self-heal loop + `loop-classifier.mjs`
   (triage), `assertion-shift-classifier.mjs` (anti-greenwash), Mode I/II
   safety layers and `forbidden-url-check.mjs`, and the `@autospec/test`
   helper library (shared harness base). The 2026-05-27 `e2e.control_effects`
   schema and effect-assertion taxonomy are implemented as part of this stage
   (control inventory feeds the authoring prompt; effect assertions are the
   required assertion vocabulary).

## 4. Contract extensions (`.autospec/test.yml`)

```yaml
e2e:
  authoring:
    enabled: true
    spec_dir: e2e/specs            # one file per route-cluster
    helpers_dir: e2e/helpers       # shared harness; authors read-only
    route_clusters: auto           # auto (crawler-derived) | explicit list
    coverage_target_pct: 80        # % of crawler-enumerated surfaces with
                                   # render+empty-state+primary-action tests
    fanout_max: 4                  # max concurrent author subagents
  reset:
    endpoint: /api/test/reset      # OR cmd: "make db-reset"
    generate_if_missing: true      # locked: fix the environment
    guard_env: AUTOSPEC_TEST_STACK # endpoint 404s unless this env var is set
  control_effects:                 # from 2026-05-27 spec, implemented here
    enabled: true
```

Defaults are conservative: `authoring.enabled` defaults to `false`;
`generate_if_missing` requires Mode I (strict isolation) or an acked Mode II
backup driver before touching app code. `forbidden_url_patterns` remains
mandatory and fail-closed (v1 §safety), and the pre-flight production-hostname
check runs before any authoring or reset-generation step.

## 5. Hard rules → deterministic lint (locked)

New `skills/autospec-test/scripts/lint-playwright-author.mjs`, run on every
authored/modified spec file before it is accepted (and wired into the
worktree pre-commit lint hook from the pipeline-hardening spec). Rules, each
with a stable `RULE_ID`:

| RULE_ID | Check | Severity |
|---|---|---|
| `PW_MOCK_BANNED` | import/require/usage of `msw`, `nock`, `sinon` fake servers, `page.route(`, `route.fulfill(`, `route.abort(`, `**/mocks/**` fixtures — including string-concat/template evasion (AST-based, not regex-only) | hard fail |
| `PW_SELECTOR_UNVERIFIED` | every `data-testid`, `aria-label`, role-name string, and `getByText` literal in the spec must resolve to ≥1 occurrence in app source (component files), recorded in a selector-evidence manifest `selector → file:line`; non-obvious selectors additionally need a `// source: file:line` comment | hard fail |
| `PW_STRICT_MODE_RISK` | unscoped `page.getByText(` / `page.getByRole(` without `{exact:true}` or a container-scoped ancestor locator | hard fail |
| `PW_SHARED_FILE_EDIT` | an author's diff touches any file outside its assigned spec file (helpers, config, other specs) | hard fail |
| `PW_SEED_BYPASS` | direct DB writes (`prisma.`, `knex(`, raw SQL) inside specs — seeding goes through the real API helpers only | hard fail |
| `PW_NO_PERSISTENCE_ASSERT` | a spec performs a UI write (click+submit pattern) with no follow-up API-state assertion via the real-API helper | warn |

Lint pairs with the standard **adaptive retry loop (MAX 5)**: each failed
attempt's `RULE_ID: finding` lines are mapped to corrective directives and
appended to the author's next prompt (same pattern as the issue-lint loop).
After 5 failures the spec file is rejected and surfaced to the operator; it is
never committed.

## 6. Authoring pipeline (Stage 2A)

1. **Enumerate surfaces.** Run `ui-crawler.mjs` (sitemap-seeded BFS) against
   the live test stack to produce the route/element manifest. This manifest —
   not author self-report — is the denominator for coverage %.
2. **Cluster routes.** `route_clusters: auto` groups routes by first path
   segment (cap: `fanout_max` clusters per round); explicit lists override.
3. **Centralize helpers first (orchestrator, single-writer).** Before any
   fan-out, the orchestrator creates/updates `helpers_dir`: register/login
   bootstrap (real registration flow, correct token storage key),
   authed-navigation, real-API seed helpers, per-test reset hook calling
   `e2e.reset`. Authors consume these read-only (`PW_SHARED_FILE_EDIT`).
4. **Fan out authors.** One Tier-B subagent per route-cluster; each writes
   exactly ONE spec file in `spec_dir`. The author prompt embeds: the cluster's
   crawler manifest slice, the control inventory + effect-assertion taxonomy
   (2026-05-27 §controls), the component source paths for selector
   verification, the hard-rule table with RULE_IDs, and the minimum per-surface
   coverage bar: renders authenticated (no /login bounce, no crash) → empty
   state correct → primary action end-to-end (create→appears→persists via real
   API; toggle; delete→gone).
5. **Lint each spec** (per §5, adaptive retry ≤5).
6. **Parse gate.** `npx playwright test --list` on the full suite — a spec
   that breaks listing is rejected back to its author.
7. **Run the FULL suite** (not just new files) against the real stack so
   cross-test effects (shared rate limits, DB state, server crashes) surface.
8. **Triage every failure** with `loop-classifier.mjs` extended classes:
   - *test bug* (`selector_brittle`, wrong flow, HTML5-validation pre-empt,
     race before refetch) → fix the test; assertion meaning preserved
     (assertion-shift classifier blocks LOOSENING as in v1 §5c).
   - *app bug* (`product_bug`: data truly absent, 4xx/5xx, error banner,
     crash, blank shell) → **fix the app in the same loop** (self-heal loop
     may already edit product code per v1 Q5), evidence (page snapshot +
     server log excerpt) recorded in the report. Never relax the assertion.
   - *environment bug* (NEW class `env_blocker`: auth bootstrap, CORS,
     rate-limit, missing migration/reset) → fix the environment config;
     mocking around it is structurally impossible (§5 lint).
9. **Re-run until green or budget exhausted** (reuse v1 self-heal 60-min
   coding-time budget; test runtime excluded).
10. **Coverage report.** Emit `surfaces_total / surfaces_covered / pct` from
    the crawler manifest, per-surface checklist (render / empty / primary
    action), and the selector-evidence manifest. If
    `pct < coverage_target_pct`, loop back to step 2 with uncovered clusters;
    if budget exhausted below target, report FAIL with the uncovered list.

## 7. Reset-endpoint generation (locked: fix the environment)

When `e2e.reset` declares neither `endpoint` nor `cmd` and
`generate_if_missing: true`:

- Stage 2A generates a test-stack-only reset route in the target app
  (framework-detected: Express/Fastify/Next route handler), which truncates
  and re-migrates/seeds the test DB.
- **Safety rails (all mandatory):** handler is a no-op 404 unless
  `process.env[guard_env]` is set; pre-flight production-hostname check (v1
  Layer A) must pass; under Mode II the configured backup driver must
  snapshot before first invocation; the generated file carries an
  `AUTOSPEC-GENERATED test-only` header.
- The generated endpoint is itself covered by one authored spec: reset →
  empty states render across surfaces.
- Fallback chain: declared reset → generated reset → autospec-e2e-clone
  per-suite isolation (mutating tests serialized) — never in-memory fakes.

## 8. Data model

- **Selector-evidence manifest:** `e2e/.autospec/selector-evidence.json` —
  `{spec_file: {selector: "file:line"}}`; produced by the lint, consumed by
  the report and by re-verification on app-source changes.
- **Coverage manifest:** crawler output + per-surface checklist, persisted to
  `e2e/.autospec/coverage.json`; the PR report renders the % statement.
- Both files are loop-immutable for authors (only orchestrator/lint write).

## 9. Error handling

- Lint failures: adaptive retry ≤5, then reject + surface (never commit).
- Crawler can't reach the stack: `env_blocker` — fix stack bring-up first;
  stage fails closed, no authoring against an unreachable app.
- Author subagent death: WIP-preserve pattern (commit `wip(...)`, relaunch
  with iterate-on-branch instructions) as standardized in pipeline-hardening.
- Reset-generation on a repo where `guard_env` cannot be enforced (static
  hosting, no server): degrade to clone-per-suite fallback, report the
  degradation explicitly.

## 10. Testing (of this feature itself)

TDD per AGENTS.md; real services, no DB mocks; 80%+ coverage.

- **Lint:** unit tests (`.mjs` test files + bats wrappers) with
  negative-path pairs for every RULE_ID — each rule has a fixture that MUST
  fail and a near-miss that MUST pass (e.g. `page.route(` fails;
  `// page.route is banned` comment passes). String-concat evasion fixtures
  for `PW_MOCK_BANNED`. Assertion-density floor per mutation-testing tracker.
- **Pipeline:** extend the v1 synthetic target repos (clean-pass /
  failing-gap / greenwash-bait) with an `authoring-fixture` web app:
  3 routes, one missing testid, one strict-mode trap (text in nav + heading),
  one genuine product bug (delete 200 but row persists), no reset endpoint —
  the integration test asserts Stage 2A: clusters routes, centralizes
  helpers, authors specs that pass lint, generates the reset endpoint with
  guard-env gating, classifies the product bug as `product_bug` (not
  weakened), and reports correct coverage %.
- **autospec-qa handshake:** qa's no-mock minimum-coverage and control-intent
  ledger checks consume the coverage + selector-evidence manifests instead of
  recomputing (imported anchors, not copied prose).

## 11. Decomposition preview (9 children + epic)

1. Contract extensions (`e2e.authoring`/`e2e.reset`/`e2e.control_effects`
   schema + loader + validation) — carries structural sections (Self-update,
   Model tier, adapter row) per decomposer convention.
2. `lint-playwright-author.mjs` + RULE_ID table + adaptive retry + tests.
3. Selector-evidence manifest builder (source-grep resolver) + tests.
4. Control inventory + effect-assertion taxonomy (implements 2026-05-27 spec
   §§controls/effects) + tests.
5. Stage 2A orchestration: clustering, helper centralization, fan-out,
   parse gate, full-suite run, coverage report — wiring into SKILL.md.
6. Reset-endpoint generation + guard-env rails + fallback chain + tests.
7. `env_blocker` classifier class + triage wiring into self-heal loop.
8. Thin `skills/autospec-playwright/` dispatcher trio + registration
   (root validate.sh arrays, install.sh usage) + docs.
9. Authoring-fixture synthetic app + end-to-end integration test.

## 12. Open questions

None — architecture, reset policy, and enforcement model locked by operator
on 2026-06-04.
