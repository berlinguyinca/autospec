# autospec-test v2 (Invariants + Window Contracts + Contract Symmetry) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
>
> **Decomposition:** 10 sequential phases. Each phase = one GitHub issue for `/autospec-split`. Every phase `Depends on #328` (v1 Phase 10 SKILL.md) as its root prerequisite, plus linear deps within v2 (v2.N depends on v2.N-1).

**Goal:** Ship Stage 2.5 (invariants + window contracts + extended crawler + data-source contract symmetry) as an extension to the existing `autospec-test` skill, plus an `@autospec/test` npm helper library, plus edge-case seed declarations that handshake with Skill C.

**Architecture:** Bolt onto v1 — same skill, same SKILL.md (extended), same self-heal loop. New gate stage 2.5 runs after Stage 2. New contract namespace `e2e.invariants_v2:` sandboxes v2 fields. Per-metric runners written in Node (AST + Playwright APIs). Helper library published as `@autospec/test` so target repos can use the same primitives imperatively.

**Tech Stack:** Bash 4+, Node 20+, `yq`, `ajv`, `jsonpath-plus`, Playwright ≥1.40 (in target repos), `pg` / `mysql2` / `better-sqlite3` (for DB driver shim in edge-case seed verifier), npm publish toolchain.

**Spec reference:** `docs/specs/2026-05-21-autospec-test-invariants-design.md` (merged via PR #333, commit `aa93662`).

**v1 prerequisite:** All v1 phases must merge before v2 phases can land. v2 issues all carry `Depends on #328`.

---

## File Structure (locked across phases)

```
skills/autospec-test/
  SKILL.md                                       # Phase 10 (modify; v2 adds sections)
  scripts/
    invariants/
      run-structural.mjs                         # Phase 3 (Metric F runner)
      kinds/
        every-visible-x-is-y.mjs                 # Phase 2
        every-foldout-opens-all-nested.mjs       # Phase 2
        every-row-has-required-actions.mjs       # Phase 2
        every-visible-x-has-accessible-name.mjs  # Phase 2
        every-modal-returns-to-body-scroll.mjs   # Phase 2
        custom-kind-protocol.md                  # Phase 2 (doc)
    window-contract/
      run-window.mjs                             # Phase 4 (Metric G runner)
      date-math.mjs                              # Phase 4 (today - N days parser)
      request-recorder.mjs                       # Phase 4
    crawler-v2/
      extended-crawler.mjs                       # Phase 5 (Metric H)
      foldout-opener.mjs                         # Phase 5 (also reused by Metric F)
      affordance-verifier.mjs                    # Phase 5
    contract-symmetry/
      run-symmetry.mjs                           # Phase 6 (Metric I runner)
      ui-extractor.mjs                           # Phase 6
      interpolator.mjs                           # Phase 6 (${var} → value)
      jsonpath-verifier.mjs                      # Phase 6
    seed-shapes/
      catalog.yml                                # Phase 7 (shape predicate catalog)
      verify-seeds.mjs                           # Phase 7 (queries clone DB, asserts shapes)
      db-driver/
        postgres.mjs                             # Phase 7
        mysql.mjs                                # Phase 7
        sqlite.mjs                               # Phase 7
        jsonpath-store.mjs                       # Phase 7 (for NoSQL targets)
    gate-stage-2-5.sh                            # Phase 10 (top orchestrator for Stage 2.5)
    assertion-shift-v2-buckets.mjs               # Phase 10 (extends Phase 4 v1 classifier)
  lib/                                           # Phase 8 (npm package source)
    invariants.ts                                # Phase 8 (imperative helper API)
    index.ts                                     # Phase 8 (re-exports)
  package.json                                   # Phase 8 (npm @autospec/test)
  tsconfig.json                                  # Phase 8
  test-targets/                                  # Phase 9
    target-invariant-bait/
    target-window-mismatch-bait/
    target-contract-symmetry-bait/
    target-greenwash-bait/                        # Phase 9 (modify: add 2 invariants)
  tests/
    unit/v2/                                     # Per-phase tests added 1..8
    integration/v2/                              # Phase 9
  validate.sh                                    # Phase 10 (extend with v2 lockstep checks)

schemas/
  autospec-test-contract.schema.json             # Phase 1 (modify; add invariants_v2 sub-schema)
```

---

## Phase 1 (v2) — Contract extension + JSON Schema for `invariants_v2:`

**GH issue title:** `feat(autospec-test): v2 contract extension + JSON Schema (phase 1)`
**Depends on:** #328

**Files:**
- Modify: `schemas/autospec-test-contract.schema.json` (extend root with `e2e.invariants_v2` sub-schema)
- Modify: `skills/autospec-test/scripts/load-contract.sh` (route v2 fields through new validator)
- Modify: `skills/autospec-test/scripts/validate-contract.sh` (add v2-specific cross-field rules)
- Create: `skills/autospec-test/tests/fixtures/contracts/v2/` (5 fixtures: minimal-v2, all-metrics, missing-edge-seeds, scoped-prod-with-v2, invalid-shapes)
- Create: `skills/autospec-test/tests/unit/v2/contract-loader-v2.bats`

### Tasks

- [ ] **1.1** Extend `autospec-test-contract.schema.json` with `e2e.invariants_v2` sub-schema covering: `enabled` (bool), `invariants` (array of kind-tagged objects), `window_contracts`, `crawler`, `contract_symmetry`, `edge_case_seeds`, `thresholds`. Use JSON Schema discriminator on `kind` for invariants array.

- [ ] **1.2** Add cross-field rules to `validate-contract.sh`:
  - `invariants_v2.enabled=true` requires either `invariants[]` or one of {`window_contracts[]`, `crawler.enabled`, `contract_symmetry[]`} non-empty (otherwise refuse: enabled with no metrics is meaningless)
  - `edge_case_seeds.enforcement=refuse_to_run_if_missing` requires at least one entry under `<entity>.require_shapes`
  - When `invariants_v2.enabled=true` AND `mode=scoped_production`, all `apply_on_routes` must resolve to scope-allowed paths
  - All `apply_on_routes` strings must start with `/`

- [ ] **1.3** Write 5 fixture YAML files in `tests/fixtures/contracts/v2/`:
  - `minimal-v2.yml`: just `invariants_v2.enabled=true` + one Metric F invariant
  - `all-metrics.yml`: every Metric F/G/H/I block populated
  - `missing-edge-seeds.yml`: `enforcement=refuse_to_run_if_missing` but no shapes → must fail
  - `scoped-prod-with-v2.yml`: Mode II + invariants_v2 referencing scope-allowed routes only
  - `invalid-shapes.yml`: shape names not in `seed-shapes/catalog.yml` → must fail

- [ ] **1.4** Bats tests assert each fixture produces expected exit code + stderr structure. Use `jq -e` on emitted JSON to assert shape.

- [ ] **1.5** Confirm v1 contracts (`tests/fixtures/contracts/*.yml` without v2 block) still parse OK — backward-compatible.

**Acceptance criteria:**
- All bats tests pass
- `ajv validate -s schema.json -d <every-fixture>` matches expected accept/reject
- Existing v1 fixtures still pass (no regression)
- Commit: `feat(autospec-test): v2 contract extension + JSON Schema (phase 1)`

---

## Phase 2 (v2) — Built-in invariant kinds library

**GH issue title:** `feat(autospec-test): built-in invariant kinds (phase 2)`
**Depends on:** v2 Phase 1

**Files:**
- Create: `skills/autospec-test/scripts/invariants/kinds/every-visible-x-is-y.mjs`
- Create: `skills/autospec-test/scripts/invariants/kinds/every-foldout-opens-all-nested.mjs`
- Create: `skills/autospec-test/scripts/invariants/kinds/every-row-has-required-actions.mjs`
- Create: `skills/autospec-test/scripts/invariants/kinds/every-visible-x-has-accessible-name.mjs`
- Create: `skills/autospec-test/scripts/invariants/kinds/every-modal-returns-to-body-scroll.mjs`
- Create: `skills/autospec-test/scripts/invariants/kinds/custom-kind-protocol.md`
- Create: `skills/autospec-test/tests/unit/v2/kinds/*.test.mjs` (one test file per kind)

### Tasks

- [ ] **2.1** Define the kind module interface:
  ```ts
  export const id: string;
  export const signature: { params: Record<string, JsonSchema>, required: string[] };
  export async function run(page: Page, params: object, ctx: { baseUrl: string; route: string }): Promise<KindResult>;
  // KindResult = { passed: boolean, violations: Array<{ index: number; selector: string; reason: string }>, count_observed: number }
  ```

- [ ] **2.2** Implement `every-visible-x-is-y.mjs`: locate `visible` selector → assert `require_count_at_least` → for each match, find `action` within the row → click → assert `verifies_open` visible → click `verifies_close` → assert `verifies_open` not visible.

- [ ] **2.3** Implement `every-foldout-opens-all-nested.mjs`: locate all `foldout` selectors → for each `[aria-expanded=false]`, click → assert ≥1 `nested_must_be_visible_after_open` visible inside the opened region.

- [ ] **2.4** Implement `every-row-has-required-actions.mjs`: for each `row` match → for each `required_actions[]` selector → assert visible + interactive (button/link role).

- [ ] **2.5** Implement `every-visible-x-has-accessible-name.mjs`: enumerate every `[role=button], button, a, input` → assert non-empty `aria-label` or text content (Playwright accessible-name API).

- [ ] **2.6** Implement `every-modal-returns-to-body-scroll.mjs`: capture `document.body.style.overflow` before → trigger `modal_open` selector → assert modal visible → trigger `modal_close` selector → assert `document.body.style.overflow` matches captured value.

- [ ] **2.7** Write `custom-kind-protocol.md` documenting the export contract (id, signature, run), JSON Schema for params, error format, registration procedure (`.autospec/invariant-kinds/<name>.mjs` in target repo auto-loaded by run-structural).

- [ ] **2.8** Unit tests for each kind: build a minimal HTML fixture in `tests/fixtures/v2/kinds/<kind-name>/{pass,fail}.html`; launch Playwright against `file://` URL; assert `KindResult`.

**Acceptance criteria:**
- Each kind has both pass and fail fixtures; test suite covers both
- Custom-kind protocol doc includes a worked example
- Commit: `feat(autospec-test): built-in invariant kinds (phase 2)`

---

## Phase 3 (v2) — Metric F runner (structural invariants)

**GH issue title:** `feat(autospec-test): metric F structural invariants runner (phase 3)`
**Depends on:** v2 Phase 2

**Files:**
- Create: `skills/autospec-test/scripts/invariants/run-structural.mjs`
- Create: `skills/autospec-test/scripts/crawler-v2/foldout-opener.mjs` (shared utility — also used by Metric H)
- Create: `skills/autospec-test/tests/unit/v2/run-structural.test.mjs`
- Create: `skills/autospec-test/tests/fixtures/v2/structural/` (mini-app fixtures)

### Tasks

- [ ] **3.1** Write `foldout-opener.mjs` exporting `openAllFoldouts(page, options?)`. Recursively clicks every `[aria-expanded=false]` element up to a max-depth (default 5) to avoid infinite loops on circular structures. Returns count of opened foldouts.

- [ ] **3.2** Write `run-structural.mjs`:
  ```ts
  Input (stdin): { contract, base_url, route_list?: string[] }
  Steps:
    1. Resolve kind modules: built-in catalog + target-repo .autospec/invariant-kinds/*
    2. For each invariant in contract.invariants_v2.invariants:
       For each route in invariant.apply_on_routes:
         page.goto(base_url + route)
         if (contract.invariants_v2.crawler?.open_all_foldouts || invariant.open_foldouts_first) openAllFoldouts(page)
         kindModule = catalog[invariant.kind]
         result = await kindModule.run(page, invariant, { baseUrl, route })
         emit per-(invariant, route) JSON line
    3. Aggregate; emit final summary
  ```

- [ ] **3.3** Output JSON shape:
  ```json
  {
    "metric": "F",
    "passed": false,
    "invariants": [
      { "id": "dashboard-done-items-editable", "route": "/dashboard",
        "passed": false, "count_observed": 7,
        "violations": [{ "index": 6, "selector": "...", "reason": "no edit button found" }] }
    ],
    "summary": { "total": 8, "passed_count": 7 }
  }
  ```

- [ ] **3.4** Fixture mini-app under `tests/fixtures/v2/structural/`: tiny static HTML with 5 done-item rows where row 4 is a `<span>` instead of `<button>`. Static-serve via `python3 -m http.server` or built-in Node static server in test setup.

- [ ] **3.5** Unit tests: pass-case (all rows editable) → expect `passed: true`. Fail-case (row 4 plain text) → expect `passed: false` with violation referencing index 3.

- [ ] **3.6** Honors Mode I/II safety: invariant runner inherits the Layer A/B network intercept from Stage 2 (no duplication — global setup files already injected).

**Acceptance criteria:**
- Fixture pass + fail both produce expected gate JSON
- Foldout opener tested with nested-foldout fixture (3 levels deep)
- Commit: `feat(autospec-test): metric F structural invariants runner (phase 3)`

---

## Phase 4 (v2) — Metric G runner (window-contract symmetry)

**GH issue title:** `feat(autospec-test): metric G window-contract symmetry (phase 4)`
**Depends on:** v2 Phase 3

**Files:**
- Create: `skills/autospec-test/scripts/window-contract/run-window.mjs`
- Create: `skills/autospec-test/scripts/window-contract/date-math.mjs`
- Create: `skills/autospec-test/scripts/window-contract/request-recorder.mjs`
- Create: `skills/autospec-test/tests/unit/v2/date-math.test.mjs`
- Create: `skills/autospec-test/tests/unit/v2/run-window.test.mjs`

### Tasks

- [ ] **4.1** Write `date-math.mjs` exporting `resolve(expr, ctx)`:
  - Recognizes: `today`, `today - N days`, `today + N days`, ISO literal `2026-05-21`
  - `ctx` carries `{ today: Date, tz?: string }` for deterministic testing
  - Returns ISO date string in `ctx.tz` or UTC
  - Throws on unparseable expressions

- [ ] **4.2** Unit-test `date-math.mjs` with table:
  ```js
  cases = [
    { expr: 'today', ctx: { today: new Date('2026-05-21T00:00:00Z') }, expected: '2026-05-21' },
    { expr: 'today - 7 days', ctx: { today: new Date('2026-05-21T00:00:00Z') }, expected: '2026-05-14' },
    { expr: '2026-05-01', ctx: {}, expected: '2026-05-01' },
    { expr: 'today + 3 days', ctx: { today: new Date('2026-05-21T00:00:00Z') }, expected: '2026-05-24' },
    { expr: 'tomorrow', ctx: {}, expectedThrow: /unparseable/ },
  ]
  ```

- [ ] **4.3** Write `request-recorder.mjs` exporting `attachRecorder(page, pathPattern)`:
  - Returns `{ requests: Array<{ url, method, params }> }` mutable
  - Uses `page.route('**/*', route => { if (matches pathPattern) record; route.continue() })`
  - Parses query string into `params` object
  - Idempotent if attached twice on same page

- [ ] **4.4** Write `run-window.mjs`:
  ```
  For each window_contract:
    1. attachRecorder(page, api_query.path_pattern)
    2. page.goto(base_url + ui_display.route)
    3. await page.locator(ui_display.widget).waitFor({ state: 'visible' })
    4. N = parseInt(await widget.getAttribute(ui_display.window_days_attr))
    5. wait for at least one recorded request (timeout 30s, configurable)
    6. For each declared window_param:
       expected = date-math.resolve(must_be.replace('$N', N), { today, tz })
       observed = parse from first recorded request matching path_pattern
       if abs(diff_in_days(expected, observed)) > tolerance_days → violation
    7. Emit gate JSON
  ```

- [ ] **4.5** Fixture mini-app: static page with `<div data-testid="streak-widget" data-window-days="7" data-loaded="true">` plus inline `<script>fetch('/api/household/timeline?from=2026-05-18&to=2026-05-21')</script>`. The mismatch (4-day window vs declared 7) is the bug.

- [ ] **4.6** Unit test: mismatch case → violation with structured diff. Match case (from=today-7d, to=today) → pass.

- [ ] **4.7** Tolerance test: configure `tolerance_days: 1`, set `from=2026-05-15` (1 day later than expected `2026-05-14`) → pass. Set `from=2026-05-16` → fail.

**Acceptance criteria:**
- date-math handles every case in 4.2 table
- Recorder captures only matching requests
- run-window emits structured violations
- Commit: `feat(autospec-test): metric G window-contract symmetry (phase 4)`

---

## Phase 5 (v2) — Metric H extended crawler

**GH issue title:** `feat(autospec-test): metric H extended crawler (phase 5)`
**Depends on:** v2 Phase 4

**Files:**
- Create: `skills/autospec-test/scripts/crawler-v2/extended-crawler.mjs`
- Create: `skills/autospec-test/scripts/crawler-v2/affordance-verifier.mjs`
- Create: `skills/autospec-test/tests/unit/v2/extended-crawler.test.mjs`

### Tasks

- [ ] **5.1** Write `affordance-verifier.mjs` exporting `verifyAffordance(page, pattern, route)`:
  - Locate `pattern.element` matches on current page
  - For each match:
    1. Capture pre-click DOM snapshot summary (focused element, body class list)
    2. Assert element is interactive (role in {button, link, menuitem})
    3. Click
    4. Wait for `pattern.opens` visible within 5s (configurable)
    5. Click `pattern.closes_via`
    6. Assert `pattern.opens` not visible
    7. Assert post-close snapshot matches pre-click (best-effort: focused element returns, body classes restored)
  - Returns `Array<{ route, element_index, passed, failure_reason? }>`

- [ ] **5.2** Write `extended-crawler.mjs`:
  ```
  1. BFS from base_url:
       queue = [base_url]
       visited = set()
       while queue not empty and len(visited) < bfs_max_routes:
         url = queue.pop()
         page.goto(url)
         if crawler.open_all_foldouts: openAllFoldouts(page)
         routes_discovered = page.locator('a[href]').evaluateAll(... in-domain only)
         queue.push(...routes_discovered)
         for pattern in crawler.affordance_patterns:
           if pattern.element matches on this page:
             results.push(...await verifyAffordance(page, pattern, url))
  2. Aggregate; report (route, element_index, failure_reason) tuples
  3. If unaffordable_count > max_unaffordable_elements: gate fail
  ```

- [ ] **5.3** Fixture mini-app: 3-page static site. Page A has working edit button. Page B has a button that opens nothing (no dialog). Page C has a link to `/missing` returning 404. Expected: 2 unaffordable elements detected (B's broken button, C's 404 link).

- [ ] **5.4** Unit tests:
  - Working case (all affordances open + close cleanly) → `passed: true`
  - Broken case (B + C fail) → `passed: false`, 2 violations
  - Tolerance case (`max_unaffordable_elements: 2`) → 2 violations but `passed: true`

- [ ] **5.5** BFS cap test: site with 250 routes → crawler stops at 200 (logged as `routes_capped: true`).

- [ ] **5.6** Foldout integration test: same site with collapsed foldout containing the broken button → without `open_all_foldouts`, crawler misses it (no violation); with `open_all_foldouts: true`, crawler finds it.

**Acceptance criteria:**
- Every fixture case produces expected `passed` + violation list
- Foldout opening exposes hidden affordances
- BFS cap honored
- Commit: `feat(autospec-test): metric H extended crawler (phase 5)`

---

## Phase 6 (v2) — Metric I runner (data-source contract symmetry)

**GH issue title:** `feat(autospec-test): metric I contract symmetry (phase 6)`
**Depends on:** v2 Phase 5

**Files:**
- Create: `skills/autospec-test/scripts/contract-symmetry/run-symmetry.mjs`
- Create: `skills/autospec-test/scripts/contract-symmetry/ui-extractor.mjs`
- Create: `skills/autospec-test/scripts/contract-symmetry/interpolator.mjs`
- Create: `skills/autospec-test/scripts/contract-symmetry/jsonpath-verifier.mjs`
- Create: `skills/autospec-test/tests/unit/v2/run-symmetry.test.mjs`

### Tasks

- [ ] **6.1** Write `ui-extractor.mjs` exporting `extract(page, route, ui_source)`:
  - `page.goto(route)`
  - For each match of `ui_source.extract` selector:
    - Build tuple from `ui_source.per_match` map (`{ task_id: row.getAttribute('data-task-id'), date: row.getAttribute('data-date') }`)
  - Returns `Array<Record<string, string>>`

- [ ] **6.2** Write `interpolator.mjs` exporting `interpolate(template, vars)`:
  - Replace every `${key}` in `template` with `vars[key]`
  - URL-encode values when interpolated into query strings (use a marker `${url:key}` to opt in, default safe)
  - Throw on undefined vars

- [ ] **6.3** Write `jsonpath-verifier.mjs` using `jsonpath-plus`:
  - `assertContains(response, pathExpr, vars)` — interpolate `${var}` in pathExpr, evaluate, fail if result empty
  - `assertBoolean(response, pathExpr, vars)` — evaluate, fail if not `true`

- [ ] **6.4** Write `run-symmetry.mjs`:
  ```
  For each contract_symmetry entry C:
    1. tuples = await extract(page, C.ui_source.route, C.ui_source)
    2. For each tuple:
       url = interpolate(C.api_target.path_template, tuple)
       response = await page.request[C.api_target.method.toLowerCase()](url)   # reuses Playwright session
       body = await response.json()
       try {
         assertContains(body, C.api_target.must_contain, tuple)
         assertBoolean(body, C.api_target.must_be_editable, tuple)
       } catch (e) {
         violations.push({ contract_id: C.id, tuple, reason: e.message,
                           api_response_summary: JSON.stringify(body).slice(0, 500) })
       }
    3. Emit gate JSON
  ```

- [ ] **6.5** Fixture: tiny Express + static HTML. UI page shows 3 streak rows with data-task-id + data-date. Backend `/api/timeline` returns 2 of the 3 (the third is the bug). Expected: 1 violation.

- [ ] **6.6** Unit tests:
  - Happy path: all 3 task_ids present + editable → pass
  - Missing event: 1 of 3 absent → 1 violation
  - Not editable: present but `editable: false` → 1 violation
  - Both broken: 1 missing + 1 not-editable → 2 violations

- [ ] **6.7** Interpolator test: `'/api/timeline?from=${date}&to=${date}'` + `{date: '2026-05-21'}` → `'/api/timeline?from=2026-05-21&to=2026-05-21'`. URL-encoded variant: `'/api?q=${url:q}'` + `{q: 'a b'}` → `'/api?q=a%20b'`.

**Acceptance criteria:**
- Extractor handles missing attributes (logs warning, skips tuple)
- Interpolator throws on undefined vars (no silent `${undefined}`)
- JSONPath verifier returns structured violations
- Commit: `feat(autospec-test): metric I contract symmetry (phase 6)`

---

## Phase 7 (v2) — Edge-case seed verifier + DB driver shims

**GH issue title:** `feat(autospec-test): edge-case seed verifier + DB driver shims (phase 7)`
**Depends on:** v2 Phase 6

**Files:**
- Create: `skills/autospec-test/scripts/seed-shapes/catalog.yml`
- Create: `skills/autospec-test/scripts/seed-shapes/verify-seeds.mjs`
- Create: `skills/autospec-test/scripts/seed-shapes/db-driver/postgres.mjs`
- Create: `skills/autospec-test/scripts/seed-shapes/db-driver/mysql.mjs`
- Create: `skills/autospec-test/scripts/seed-shapes/db-driver/sqlite.mjs`
- Create: `skills/autospec-test/scripts/seed-shapes/db-driver/jsonpath-store.mjs`
- Create: `skills/autospec-test/tests/unit/v2/verify-seeds.test.mjs`

### Tasks

- [ ] **7.1** Write `catalog.yml` with the 7 shape predicates from the v2 spec:
  ```yaml
  task_done_today:
    description: "Task completed within today (local TZ)"
    predicate_sql: "completed_at::date = current_date"
    predicate_jsonpath: "$.tasks[?(@.completed_at_date == today())]"
  task_done_yesterday:
    predicate_sql: "completed_at::date = current_date - 1"
    predicate_jsonpath: "$.tasks[?(@.completed_at_date == yesterday())]"
  task_done_2_to_6_days_ago:
    predicate_sql: "completed_at::date BETWEEN current_date - 6 AND current_date - 2"
    predicate_jsonpath: "$.tasks[?(@.days_ago >= 2 && @.days_ago <= 6)]"
  task_done_around_midnight:
    predicate_sql: "abs(extract(epoch from (completed_at::time - '00:00:00'::time))) < 1800"
    predicate_jsonpath: "$.tasks[?(@.completed_at_time =~ /^(00:0[0-9]|23:[34][0-9])/)]"
  multiple_tasks_same_day:
    predicate_sql: "EXISTS (SELECT 1 FROM tasks t2 WHERE t2.completed_at::date = tasks.completed_at::date AND t2.id != tasks.id)"
    predicate_jsonpath: "$.tasks[?(@.same_day_siblings > 0)]"
  task_in_collapsed_foldout:
    predicate_sql: "foldout_id IS NOT NULL AND foldout_default_collapsed = true"
    predicate_jsonpath: "$.tasks[?(@.foldout_default_collapsed == true)]"
  last_item_in_long_list:
    predicate_sql: "row_number() OVER (PARTITION BY list_id ORDER BY position DESC) = 1 AND list_size > 20"
    predicate_jsonpath: "$.tasks[?(@.is_last_in_list_of_20_plus == true)]"
  ```

- [ ] **7.2** DB driver interface (all four exports):
  ```ts
  async function connect(dsn: string): Promise<Connection>;
  async function countMatching(conn, table: string, predicate: string): Promise<number>;
  async function close(conn): Promise<void>;
  ```

- [ ] **7.3** Implement postgres.mjs (uses `pg`), mysql.mjs (uses `mysql2/promise`), sqlite.mjs (uses `better-sqlite3`), jsonpath-store.mjs (HTTP GET against a declared JSON endpoint + `jsonpath-plus`).

- [ ] **7.4** Write `verify-seeds.mjs`:
  ```
  Input: { contract, clone_dsn, store_kind: 'postgres'|'mysql'|'sqlite'|'jsonpath' }
  1. Load catalog.yml + .autospec/seed-shapes.yml if present (custom shapes overlay)
  2. Resolve driver by store_kind
  3. For each entity under edge_case_seeds:
       For each require_shapes[]:
         predicate = catalog[shape.name].predicate_<kind>
         count = await driver.countMatching(conn, entity, predicate)
         if count < shape.count_min: violations.push({ entity, shape, observed: count, required: shape.count_min })
  4. If violations.length > 0 AND enforcement == 'refuse_to_run_if_missing':
       emit exit code 2 (operator-actionable) + structured violation JSON
  ```

- [ ] **7.5** Unit tests with SQLite (most portable in CI):
  - Build a fixture DB with 0 `task_done_today` → `verify-seeds` fails with that violation
  - Add 1 row matching today → passes
  - Build with all 7 shapes satisfied → passes
  - Custom shape via `.autospec/seed-shapes.yml` overlay → loads + evaluates correctly

- [ ] **7.6** Postgres/MySQL drivers tested in CI matrix (use docker-compose service containers; skip if not available locally — bats `skip` directive).

**Acceptance criteria:**
- SQLite path fully tested
- Each driver implements the same interface
- Verifier emits exit code 2 on missing shapes (operator-actionable)
- Commit: `feat(autospec-test): edge-case seed verifier + DB drivers (phase 7)`

---

## Phase 8 (v2) — `@autospec/test` npm package + helper library

**GH issue title:** `feat(autospec-test): @autospec/test npm helper library (phase 8)`
**Depends on:** v2 Phase 7

**Files:**
- Create: `skills/autospec-test/lib/invariants.ts`
- Create: `skills/autospec-test/lib/index.ts`
- Create: `skills/autospec-test/package.json`
- Create: `skills/autospec-test/tsconfig.json`
- Create: `skills/autospec-test/scripts/publish-helpers.sh`
- Create: `skills/autospec-test/tests/unit/v2/lib-import.test.mjs`

### Tasks

- [ ] **8.1** Write `package.json`:
  ```json
  {
    "name": "@autospec/test",
    "version": "1.0.0",
    "type": "module",
    "main": "./dist/index.js",
    "types": "./dist/index.d.ts",
    "files": ["dist", "README.md"],
    "exports": {
      ".": { "types": "./dist/index.d.ts", "default": "./dist/index.js" },
      "./invariants": { "types": "./dist/invariants.d.ts", "default": "./dist/invariants.js" }
    },
    "peerDependencies": { "@playwright/test": ">=1.40.0" },
    "scripts": { "build": "tsc -p tsconfig.json", "test": "node --test tests/unit/v2/lib-import.test.mjs" }
  }
  ```

- [ ] **8.2** Write `tsconfig.json` targeting ES2022, module NodeNext, outDir `./dist`, strict on.

- [ ] **8.3** Write `lib/invariants.ts` re-exporting imperative wrappers around the kind modules from Phase 2:
  ```ts
  export async function assertEveryVisibleDoneItemIsEditable(page, opts: {
    rowSelector: string;
    editButtonName: RegExp;
    dialogSelector: string;
    closeButtonName?: RegExp;
  }) { /* delegates to every-visible-x-is-y kind */ }

  export async function assertEveryFoldoutOpensAllNestedRows(page, opts: {
    foldoutSelector: string;
    nestedSelector: string;
  }) { /* delegates to every-foldout-opens-all-nested kind */ }

  export async function assertDateWindowCoverage(page, opts: {
    widgetSelector: string;
    windowDaysAttr: string;
    apiPathPattern: RegExp | string;
    fromParam: string;
    toParam: string;
    toleranceDays?: number;
  }) { /* delegates to run-window flow */ }

  export async function assertContractSymmetry(page, opts: {
    extractSelector: string;
    perMatch: Record<string, string>;
    apiPathTemplate: string;
    mustContainJsonPath: string;
    mustBeEditableJsonPath: string;
  }) { /* delegates to run-symmetry flow */ }

  export async function openAllFoldouts(page, opts?: { maxDepth?: number }) { /* foldout-opener */ }

  export async function enumerateAffordances(page, patterns: AffordancePattern[]) { /* extended-crawler/affordance-verifier */ }
  ```

- [ ] **8.4** Write `lib/index.ts` re-exporting from `invariants.ts`.

- [ ] **8.5** Build TypeScript: `npx tsc -p tsconfig.json`. Verify `dist/index.js` + `dist/index.d.ts` produced.

- [ ] **8.6** Write `tests/unit/v2/lib-import.test.mjs`:
  - Pack tarball: `npm pack`
  - Install in a temp scratch dir: `npm install <tarball>`
  - Run a fixture test importing `assertEveryVisibleDoneItemIsEditable` against the Phase 3 fixture mini-app
  - Assert pass + fail outcomes

- [ ] **8.7** Write `publish-helpers.sh`:
  - Builds package
  - Validates version bumped vs published
  - Dry-run `npm publish --dry-run`
  - Real publish gated behind `--release` flag

- [ ] **8.8** README in `skills/autospec-test/lib/README.md` (separate from skill SKILL.md) — package-level usage docs for npm consumers.

**Acceptance criteria:**
- `npm pack` succeeds; tarball installs cleanly
- All 6 helper functions covered by at least one fixture test
- TypeScript types resolve correctly when consumed
- Commit: `feat(autospec-test): @autospec/test npm helper library (phase 8)`

---

## Phase 9 (v2) — Synthetic targets for Stage 2.5

**GH issue title:** `test(autospec-test): v2 synthetic targets (phase 9)`
**Depends on:** v2 Phase 8

**Files:**
- Create: `skills/autospec-test/test-targets/target-invariant-bait/`
- Create: `skills/autospec-test/test-targets/target-window-mismatch-bait/`
- Create: `skills/autospec-test/test-targets/target-contract-symmetry-bait/`
- Modify: `skills/autospec-test/test-targets/target-greenwash-bait/` (add 2 invariants the loop is tempted to delete)
- Create: `skills/autospec-test/tests/integration/v2/run-against-target.bats`
- Golden files alongside each target

### Tasks

- [ ] **9.1** `target-invariant-bait/`:
  - Tiny Vite + React app with `<DoneItem>` component
  - Renders 5 items: indices 0–3 are `<button>` (editable), index 4 (last in collapsed foldout) is `<span>` (the bug)
  - `.autospec/test.yml` declares the `every_visible_X_is_Y` invariant
  - Golden gate JSON: `passed=false` with violation at index 4 referencing the span

- [ ] **9.2** `target-window-mismatch-bait/`:
  - Static SPA: dashboard renders `<div data-testid="streak-widget" data-window-days="7" data-loaded="true">`
  - Inline script: `fetch('/api/household/timeline?from=2026-05-18&to=2026-05-21')` (4-day window, mismatch)
  - Tiny Express mock returning empty array
  - `.autospec/test.yml` declares `dashboard-streak-window` contract
  - Golden: `passed=false` with `expected from=2026-05-14, got 2026-05-18`

- [ ] **9.3** `target-contract-symmetry-bait/`:
  - SPA + Express. UI shows 3 streak rows: `data-task-id={t-1,t-2,t-3}` with `data-date=2026-05-14`
  - Backend returns events only for `t-1` and `t-2` (t-3 missing)
  - `.autospec/test.yml` declares `streak-task-must-be-editable` contract
  - Golden: `passed=false`, violation for `{task_id: t-3, date: 2026-05-14}`

- [ ] **9.4** Modify `target-greenwash-bait/`:
  - Add `.autospec/test.yml` v2 block with 2 invariants
  - Add fixture diff under `tests/fixtures/v2/greenwash-bait/loop-attempts-to-delete-invariants.diff`
  - Golden for assertion-shift classifier: LOOSENING bucket → block

- [ ] **9.5** Integration test harness in `tests/integration/v2/run-against-target.bats`:
  - For each target, runs `gate-stage-2-5.sh` (from Phase 10 — initially a stub; integration test re-runs after Phase 10 lands)
  - Diffs actual JSON + report markdown against golden
  - Golden files versioned in repo

- [ ] **9.6** All 4 targets ship with their own `package.json` + `playwright.config` so they're runnable standalone (debugging support).

**Acceptance criteria:**
- All 4 targets produce expected golden outputs
- Goldens checked in
- Targets runnable standalone (operator can `cd target-invariant-bait/ && npm test`)
- Commit: `test(autospec-test): v2 synthetic targets (phase 9)`

---

## Phase 10 (v2) — Stage 2.5 orchestrator + autospec-run wiring + assertion-shift v2 + SKILL.md + docs

**GH issue title:** `feat(autospec-test): stage 2.5 orchestrator + assertion-shift v2 + SKILL.md (phase 10)`
**Depends on:** v2 Phase 9

**Files:**
- Create: `skills/autospec-test/scripts/gate-stage-2-5.sh`
- Create: `skills/autospec-test/scripts/assertion-shift-v2-buckets.mjs`
- Modify: `skills/autospec-test/scripts/run-gate.sh` (invoke Stage 2.5 after Stage 2)
- Modify: `skills/autospec-test/scripts/pr-report.sh` (append Stage 2.5 subsection)
- Modify: `skills/autospec-test/SKILL.md` (add v2 sections)
- Modify: `skills/autospec-test/validate.sh` (add v2 lockstep checks)
- Modify: top-level `validate.sh` if any aggregation needed

### Tasks

- [ ] **10.1** Write `gate-stage-2-5.sh`:
  ```
  Input: contract JSON (from load-contract.sh)
  Exit codes: 0=pass, 1=fail-block, 2=refuse-to-run
  1. If contract.invariants_v2.enabled != true → emit { metric: '2.5', skipped: true, passed: true } and exit 0
  2. Verify edge_case_seeds (if declared): node verify-seeds.mjs → on exit 2, propagate
  3. Run Metric F: node run-structural.mjs → collect JSON
  4. Run Metric G: node run-window.mjs → collect JSON
  5. Run Metric H: node extended-crawler.mjs → collect JSON
  6. Run Metric I: node run-symmetry.mjs → collect JSON
  7. Compose Stage 2.5 gate JSON; exit 0 if all passed, exit 1 otherwise
  ```

- [ ] **10.2** Modify `run-gate.sh` (from v1 Phase 9) to call `gate-stage-2-5.sh` after Stage 2 and before assertion-shift. Propagate exit codes per spec §6b table.

- [ ] **10.3** Write `assertion-shift-v2-buckets.mjs`:
  - Given a diff of `.autospec/test.yml` (before + after YAML strings parsed)
  - Diff `invariants_v2.*` paths
  - Apply v2 §5c table to bucket each change as LOOSENING/SHIFTING/STRENGTHENING
  - Returns `Array<Verdict>` with same shape as v1 Phase 4 classifier
  - Loaded by v1's assertion-shift-classifier.mjs as a second pass (or as a registered handler for the v2 namespace)

- [ ] **10.4** Modify v1 Phase 4 `assertion-shift-classifier.mjs` to register the v2 buckets module. Backward-compatible — if v2 module absent, no v2 verdicts emitted.

- [ ] **10.5** Unit tests for v2 buckets: fixture diffs covering every row of the spec §5c table (LOOSENING: remove invariant, narrow routes, lower count, hard_fail→warn_only, lower bfs_max_routes, remove affordance; STRENGTHENING: add entries, widen routes, raise thresholds; SHIFTING: change selector, change path_template).

- [ ] **10.6** Modify `pr-report.sh` to append the Stage 2.5 subsection per v2 spec §6a (the exact markdown skeleton from the spec).

- [ ] **10.7** Modify `SKILL.md` adding sections:
  - `## v2 — Stage 2.5 invariants` (high-level)
  - `## Contract: invariants_v2` (with YAML example)
  - `## Built-in invariant kinds` (list of 5 + protocol pointer)
  - `## Edge-case seeds handshake with Skill C`
  - `## Helper library (@autospec/test)`
  - Update adapter row (per saved memory: structural sections needed for decomposer)
  - Update Model tier section if needed

- [ ] **10.8** Extend `validate.sh` (per-skill) with v2 lockstep checks:
  - Presence of `invariants_v2:` example block in SKILL.md
  - Presence of `edge_case_seeds:` example
  - Adapter row updated

- [ ] **10.9** Run all v2 synthetic targets through the now-wired orchestrator (Phase 9 integration test re-run); assert golden diffs clean.

- [ ] **10.10** Update `docs/target-repo-setup.md` with a "How to enable autospec-test v2" section: install `@autospec/test`, add `invariants_v2:` block, declare edge_case_seeds, what labels mean.

**Acceptance criteria:**
- `gate-stage-2-5.sh` skipped when `enabled=false` (zero overhead)
- All 4 synthetic targets produce expected golden outputs through the wired orchestrator
- Assertion-shift v2 buckets pass all fixture diffs
- `validate.sh` passes
- Lockstep gotchas honored: structural sections, adapter row, no shell-out of user text, no RETURN traps, no `[ a ] && b` under set -e
- Commit: `feat(autospec-test): stage 2.5 orchestrator + assertion-shift v2 + SKILL.md (phase 10)`

---

## Cross-cutting acceptance (final gate before declaring v2 done)

- [ ] Every v2 phase merged to main via autospec-run
- [ ] `target-invariant-bait`, `target-window-mismatch-bait`, `target-contract-symmetry-bait` golden diffs all clean
- [ ] `target-greenwash-bait` LOOSENING-block test still passes (v1 invariants + v2 invariants both protected)
- [ ] `@autospec/test` npm package builds + packs + installs in clean Node sandbox
- [ ] SQLite-backed seed verifier passes; Postgres/MySQL skipped gracefully when service container unavailable
- [ ] Saved-memory lockstep rules honored across v2 PRs
- [ ] No commit edits `.autospec/test.yml` outside the wizard or the quarantine path

---

## Self-review

**Spec coverage:**
- §1 goal/non-goals — covered by phase boundaries
- §2 architecture (Stage 2.5 split, sandboxed namespace) — Phase 10 orchestrator
- §3 contract — Phase 1
- §4a Metric F runner — Phase 3
- §4b Metric G runner — Phase 4
- §4c Metric H extended crawler — Phase 5
- §4d Metric I contract symmetry — Phase 6
- §5a helper library — Phase 8
- §5b seed handshake — Phase 7
- §5c assertion-shift extension — Phase 10
- §5d self-heal classifier extension — covered by v1 Phase 5 loop (reads new categories from v2's gate JSON; no new code needed for the loop itself since classifier is data-driven)
- §6a/b PR report + failure semantics — Phase 10
- §7 dependencies + family map — Phase 10 SKILL.md updates
- §8 testing — Phases 2/3/4/5/6/7/8 unit tests + Phase 9 synthetic targets + Phase 10 integration

**Placeholder scan:** clean — every task has exact file path + acceptance criteria. No TBD/TODO.

**Type consistency:**
- `KindResult` shape consistent (Phase 2 → 3)
- Gate JSON `metric` field consistent across F/G/H/I (Phases 3–6)
- Exit code contract (0/1/2) consistent
- `Verdict` (assertion-shift) same shape as v1 Phase 4

**Open follow-ups (NOT in this plan, per spec §10):**
- Skill C clone provisioner — separate spec
- Visual regression — future v3
- Performance budgets — future v3
- Accessibility audit (axe-core) — separate skill candidate
