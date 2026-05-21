# autospec-test v2 — Structural Invariants, Window Contracts, Extended Crawler, Contract Symmetry

**Status:** Draft design (2026-05-21)
**Author:** berlinguyinca + brainstorm
**Skill name:** `autospec-test` (extends v1; v1 spec at `docs/specs/2026-05-21-autospec-test-design.md`)
**Family position:** v2 of Skill A. Builds on v1 without forking the skill.

## 1. Goal & non-goals

### Goal
Catch the regression class where Playwright suites pass green but the user-visible product still breaks — specifically: missing edit affordances on visible items, frontend-display windows wider than backend query windows, broken or unaffordable interactive elements, and UI claims that the API cannot back. v2 adds four declarative gate metrics (F/G/H/I) that bolt onto v1's Stage 2 E2E gate as a new Stage 2.5, plus a shipped Playwright helper library so target teams can write the same patterns imperatively. Edge-case seed declarations create a hard handshake with Skill C (clone provisioner).

### Non-goals
- Visual regression / screenshot baselines (different tooling)
- Performance budgets (latency, payload size — future v3)
- Accessibility audits beyond `every_visible_X_has_accessible_name` (axe-core integration is a separate concern)
- Cross-browser matrix policy (target's `playwright.config` is still authoritative)
- Clone provisioning details (Skill C owns seed creation; v2 declares the handshake)

## 2. Architecture & integration with v1

```
Phase 4 build+lint  →  Stage 1 unit (v1)  →  Stage 2 E2E (v1: A+B+D)
                                                      │
                                                      ▼
                                             Stage 2.5 invariants (v2)
                                              ├─ F: structural invariants
                                              ├─ G: window-contract symmetry
                                              ├─ H: extended crawler
                                              └─ I: data-source contract symmetry
                                                      │
                                                      ▼
                                             Assertion-shift guardrail (v1 §5c, extended)
                                                      │
                                                      ▼
                                              Auto-merge or block
```

**Why Stage 2.5 (separate from Stage 2):** v1 Stage 2 metrics (A: code cov, B: UI element touched, D: behavior taxonomy) consume the *user's* Playwright suite outputs. v2 metrics require dedicated test runs with skill-supplied fixtures. Conflating them muddies failure diagnosis.

**Shared with v1 (no rework):**
- Contract file `.autospec/test.yml` extended with new `e2e.invariants_v2:` namespace
- Safety rails: Mode I forbidden URLs + Mode II scope tokens apply identically; v2's dedicated runs go through the same network intercept
- Self-heal loop: same 60-min coding budget; classifier extended with v2 categories
- Assertion-shift classifier: extended buckets for v2 contract fields
- PR report: appends Stage 2.5 subsection to the same marker comment

**Build ordering:** v1 issues #321–#328 remain in flight under v1 spec. v2 issues (filed after this spec lands) all `Depends on #328` (v1 Phase 10 SKILL.md) as their root prerequisite so the skill scaffold exists before v2 lands.

## 3. Contract additions

All v2 fields nested under `e2e.invariants_v2:` for sandboxing.

```yaml
e2e:
  invariants_v2:
    enabled: true                       # master switch

    # Metric F — Structural invariants
    invariants:
      - id: dashboard-done-items-editable
        kind: every_visible_X_is_Y
        visible: '[data-testid^="done-item-row-"]'
        action: 'role=button[name=/edit/i]'
        verifies_open: '[data-testid="done-item-edit-dialog"]'
        verifies_close: 'role=button[name=/close/i]'
        apply_on_routes: ['/dashboard', '/family']
        require_count_at_least: 1
      - id: foldouts-all-open
        kind: every_foldout_opens_all_nested
        foldout: '[data-testid^="day-foldout-"]'
        nested_must_be_visible_after_open: '[data-testid^="day-foldout-row-"]'
        apply_on_routes: ['/dashboard']
      - id: list-items-have-actions
        kind: every_row_has_required_actions
        row: '[data-testid^="task-row-"]'
        required_actions:
          - 'role=button[name=/edit/i]'
          - 'role=button[name=/delete/i]'
        apply_on_routes: ['/tasks']

    # Metric G — Window-contract symmetry
    window_contracts:
      - id: dashboard-streak-window
        ui_display:
          route: /dashboard
          widget: '[data-testid=streak-widget]'
          window_days_attr: 'data-window-days'
        api_query:
          method: GET
          path_pattern: '^/api/household/timeline$'
          window_params:
            from: { type: iso_date, must_be: 'today - $N days', tolerance_days: 1 }
            to:   { type: iso_date, must_be: 'today', tolerance_days: 1 }
          recorded_via: network_intercept
        mismatch_action: hard_fail

    # Metric H — Extended crawler
    crawler:
      enabled: true
      bfs_max_routes: 200
      open_all_foldouts: true
      affordance_patterns:
        - element: '[data-testid^="done-item-row-"] role=button[name=/edit/i]'
          opens: '[data-testid="done-item-edit-dialog"]'
          closes_via: 'role=button[name=/close/i]'
        - element: '[data-testid^="task-row-"] role=button[name=/delete/i]'
          opens: '[data-testid="confirm-delete-dialog"]'
          closes_via: 'role=button[name=/cancel/i]'
      failure_threshold:
        max_unaffordable_elements: 0

    # Metric I — Data-source contract symmetry
    contract_symmetry:
      - id: streak-task-must-be-editable
        ui_source:
          route: /dashboard
          extract: '[data-testid^="streak-task-"]'
          per_match: { task_id: 'data-task-id', date: 'data-date' }
        api_target:
          method: GET
          path_template: '/api/household/timeline?from=${date}&to=${date}&member_id=current'
          must_contain: '$.events[?(@.task_id=="${task_id}")]'
          must_be_editable: '$.events[?(@.task_id=="${task_id}")].editable == true'
        mismatch_action: hard_fail

    # Edge-case seeds — declared here, provisioned by Skill C
    edge_case_seeds:
      household_test_family:
        require_shapes:
          - { name: 'task_done_today',           count_min: 1 }
          - { name: 'task_done_yesterday',       count_min: 1 }
          - { name: 'task_done_2_to_6_days_ago', count_min: 5 }
          - { name: 'task_done_around_midnight', count_min: 1 }
          - { name: 'multiple_tasks_same_day',   count_min: 1 }
          - { name: 'task_in_collapsed_foldout', count_min: 1 }
          - { name: 'last_item_in_long_list',    count_min: 1 }
      enforcement: refuse_to_run_if_missing

    thresholds:
      invariants_required_pass_rate: 100
      window_contracts_required_pass_rate: 100
      crawler_required_pass_rate: 100
      contract_symmetry_required_pass_rate: 100
```

**Key properties:**
- `require_count_at_least` on invariants prevents vacuous truth from a selector that matches nothing
- `window_days_attr` reads N from a DOM attribute the UI itself sets — adding pages that read the same API automatically get covered
- `recorded_via: network_intercept` observes actual API calls during the test; not a static list
- `edge_case_seeds.enforcement: refuse_to_run_if_missing` is a hard handshake with Skill C
- All `mismatch_action` values: `hard_fail | warn_only`

## 4. Runtime behavior of the four metrics

### 4a. Metric F — Structural invariants

For each declared invariant, the runner spawns one Playwright test per `apply_on_routes` entry. Generated by `skills/autospec-test/scripts/invariants/run-structural.mjs`:

```ts
test('invariant: dashboard-done-items-editable @ /dashboard', async ({ page }) => {
  await page.goto(`${BASE_URL}/dashboard`);
  await openAllFoldoutsIfDeclared(page);
  const visible = page.locator('[data-testid^="done-item-row-"]');
  const count = await visible.count();
  expect(count).toBeGreaterThanOrEqual(1);   // require_count_at_least
  for (let i = 0; i < count; i++) {
    const row = visible.nth(i);
    const action = row.getByRole('button', { name: /edit/i });
    await expect(action).toBeVisible();
    await action.click();
    await expect(page.locator('[data-testid="done-item-edit-dialog"]')).toBeVisible();
    await page.getByRole('button', { name: /close/i }).click();
    await expect(page.locator('[data-testid="done-item-edit-dialog"]')).not.toBeVisible();
  }
});
```

**Built-in invariant kinds** (`skills/autospec-test/scripts/invariants/kinds/`):
- `every_visible_X_is_Y` — A has child/sibling B; clicking B opens C; closing via D returns to prior state
- `every_foldout_opens_all_nested` — every foldout reveals ≥1 nested row matching declared selector
- `every_row_has_required_actions` — every row has each declared action as a visible interactive element
- `every_visible_X_has_accessible_name` — a11y floor
- `every_modal_returns_to_body_scroll` — modal close restores body scroll

**Custom kinds** registered in `.autospec/invariant-kinds/<name>.mjs` exporting `{ id, signature, run(page, params) }`.

### 4b. Metric G — Window contracts

1. Visit `ui_display.route`; attach `page.route('**/*', recorder)` capturing requests matching `api_query.path_pattern` into `seen_requests[]`.
2. Read `N = parseInt(widget.getAttribute(window_days_attr))`.
3. Wait for widget to render (`data-loaded=true` or per-contract custom selector).
4. Assert `seen_requests` contains ≥1 match satisfying every declared `window_param`:
   - `from.must_be: 'today - $N days'` resolved via small expression evaluator (`today`, `today ± N days`, ISO literals); compared with actual parsed `from`.
5. Mismatch → fail with structured diff: `expected from=2026-05-14 (today-7d), got from=2026-05-18 (today-3d)`.

**Tolerance:** ±1 day default (timezone slippage); per-contract overridable via `tolerance_days`.

### 4c. Metric H — Extended crawler

Extends v1's UI element crawler:

1. **Opens all foldouts before enumerating.** Recursively clicks every `[aria-expanded=false]` so collapsed-foldout content participates in coverage and affordance checks.
2. **Affordance verification, not just enumeration.** For every element matching `affordance_patterns[].element`:
   - Assert expected interactive kind
   - Click
   - Assert `opens:` selector becomes visible within timeout
   - Close via `closes_via:`
   - Assert pre-click state returns

Failure aggregation: every `(route, element, failure_reason)` tuple recorded; `max_unaffordable_elements: 0` (default) means any failure blocks. Operators can soften, but the default is hard zero.

### 4d. Metric I — Data-source contract symmetry

The most consequential metric for the regression class described in the v2 ask:

1. Visit `ui_source.route` in Playwright. For every match of `extract` selector, read declared `per_match` attributes into a tuple — `{ task_id: 't-abc', date: '2026-05-21' }`.
2. For each tuple, issue the **same backend HTTP request the UI would issue**, using `path_template` with `${var}` interpolation. Same session cookies/auth.
3. Apply `must_contain` (JSONPath) — empty result fails.
4. Apply `must_be_editable` (JSONPath boolean) — `false`/undefined fails.
5. Per-mismatch report: `{ ui_claim, api_response_summary (first 500 bytes), missing_assertion }`.

Catches: UI dashboard shows a streak entry → backend says "no event in that date range" → impossible to edit. The exact bug class the v2 ask cited.

## 5. Playwright helper library + seed handoff + assertion-shift

### 5a. Helper library

Shipped at `skills/autospec-test/lib/invariants.ts` (npm package `@autospec/test`). Target repos import for imperative usage:

```ts
import {
  assertEveryVisibleDoneItemIsEditable,
  assertEveryFoldoutOpensAllNestedRows,
  assertDateWindowCoverage,
  assertContractSymmetry,
  openAllFoldouts,
  enumerateAffordances,
} from '@autospec/test/invariants';

test('dashboard done items', async ({ page }) => {
  await page.goto('/dashboard');
  await openAllFoldouts(page);
  await assertEveryVisibleDoneItemIsEditable(page, {
    rowSelector: '[data-testid^="done-item-row-"]',
    editButtonName: /edit/i,
    dialogSelector: '[data-testid="done-item-edit-dialog"]',
  });
});
```

**Why both declarative + imperative:** declarative is the gate (runs every PR, catches regressions, no extra code in target repo); imperative is for exploratory + custom invariants team writes alongside normal tests. They share underlying kind-modules.

Published as `@autospec/test` npm; versioned 1.0.0 on first release; semver after. Wizard handles target-repo install.

### 5b. Edge-case seed handshake with Skill C

`edge_case_seeds` read by BOTH skills:
- **Skill C (clone provisioner)** reads `require_shapes` and produces synthetic rows matching each shape during clone build
- **Skill A (this spec)** verifies at gate-time that the cloned environment contains rows matching each shape (`SELECT COUNT(*) WHERE <shape_predicate> ≥ count_min`). Missing → `e2e:contract-error` with the missing shape names listed.

Shape predicate catalog ships at `skills/autospec-test/scripts/seed-shapes/catalog.yml`:

```yaml
task_done_today:
  predicate_sql: "completed_at::date = current_date"
  predicate_jsonpath: "$.tasks[?(@.completed_at_date == today())]"
task_done_around_midnight:
  predicate_sql: "abs(extract(epoch from completed_at::time - '00:00:00'::time)) < 1800"
```

Custom shapes via `.autospec/seed-shapes.yml` in target repo.

**Skill A non-functional without Skill C** for any target enabling `edge_case_seeds`. Intentional: invariants only mean something against representative data, and "representative" must be declared.

### 5c. Assertion-shift extension

V1's classifier extends to cover edits in `.autospec/test.yml` itself for v2 fields. (`forbidden_url_patterns` and Mode II scope tokens remain loop-immutable per v1 §5d.)

| Field edit | Bucket |
|---|---|
| Remove `invariants[]` entry | LOOSENING |
| Narrow `apply_on_routes` | LOOSENING |
| Lower `require_count_at_least` | LOOSENING |
| Change `mismatch_action: hard_fail → warn_only` | LOOSENING |
| Lower `bfs_max_routes` | LOOSENING |
| Remove `affordance_patterns[]` entry | LOOSENING |
| Add `invariants[]` / `affordance_patterns[]` / `window_contracts[]` / `contract_symmetry[]` entry | STRENGTHENING |
| Widen `apply_on_routes` | STRENGTHENING |
| Raise `require_count_at_least` | STRENGTHENING |
| Raise `bfs_max_routes` | STRENGTHENING |
| Change invariant selector (new component) | SHIFTING |
| Change `path_template` in contract_symmetry | SHIFTING |

SHIFTING requires (i) same-iteration product-code edit + (ii) `JUSTIFICATION:` commit line, same as v1.

Net effect: the loop **can** add invariants when it sees gaps; **cannot** delete or weaken them.

### 5d. Self-heal loop classifier extension

Priority order extended (v1 §6):

```
product_bug
> missing_unit_test
> missing_test (E2E)
> missing_invariant                    ← v2 new
> missing_window_contract              ← v2 new
> missing_contract_symmetry            ← v2 new
> selector_brittle
> failing_unit_test
> failing_invariant                    ← v2 new
> failing_window_contract              ← v2 new
> failing_contract_symmetry            ← v2 new
> failing_test (E2E)
> flaky_test
```

Detection:
- `missing_invariant`: LLM findings suggest "route X has visible elements matching pattern P but no invariant declared"
- `missing_window_contract`: trace shows date-windowed widget rendered but no `window_contracts[]` entry
- `missing_contract_symmetry`: UI extracts `task_id`/`date` pairs but no contract_symmetry entry validates them
- `failing_invariant`: invariant runner reports a violation
- `failing_window_contract`: detected mismatch
- `failing_contract_symmetry`: UI claim absent in API response

**Edge-case seed missing** → `e2e:contract-error` (refuse to run); loop **cannot** fix this — Skill A explicitly does not edit clone-provisioner concerns. Operator must re-run Skill C.

## 6. Failure semantics & reporting

### 6a. PR report — Stage 2.5 subsection

Appended after v1 Stage 2 section in the same marker-replaced comment:

```markdown
### Stage 2.5 — Invariants & contracts

**Metric F — Structural invariants:** 3/4 passed
- ❌ `dashboard-done-items-editable @ /dashboard`: row 7 (`done-item-row-task-abc`) — no `edit` button found. Last visible row in foldout; suspect plain-text rendering.

**Metric G — Window contracts:** 0/1 passed
- ❌ `dashboard-streak-window`: UI shows 7-day window (`data-window-days="7"`); `/api/household/timeline` called with `from=2026-05-18, to=2026-05-21` (4-day window).

**Metric H — Extended crawler:** 2 unaffordable elements (threshold: 0)
- ❌ `/dashboard` → `button[data-testid=delete-task-7]`: clicked, no dialog within 5s
- ❌ `/family` → `a[href=/family/edit]`: navigated to 404

**Metric I — Contract symmetry:** 1/3 passed
- ❌ `streak-task-must-be-editable`: UI claims `{task_id: t-xyz, date: 2026-05-14}` done; backend returned `events: []`
- ❌ Same contract, `{task_id: t-abc, date: 2026-05-15}`: backend returned event but `editable: false`; UI shows edit button anyway

**Edge-case seeds (Skill C handshake):** ✅ all 7 shapes present.
```

### 6b. Per-PR outcomes

| State | Labels | Auto-merge | Pipeline impact |
|---|---|---|---|
| All v1 + v2 pass | `e2e:passed` | ✅ | none |
| v1 passes, v2 invariant fail | `e2e:passed-stage2`, `e2e:blocked-stage25`, `needs-human-review` | ❌ | move on |
| Window-contract mismatch | + `e2e:window-mismatch` | ❌ | move on |
| Contract-symmetry fail | + `e2e:contract-mismatch` | ❌ | move on |
| Crawler unaffordable elements | + `e2e:unaffordable-elements` | ❌ | move on |
| Edge-case seed missing | `e2e:refused`, `e2e:seed-missing` | ❌ | operator re-runs Skill C provisioning |
| Loop healed by adding invariants | `e2e:healed`, `e2e:v2-invariants-added` | ✅ | none |

Mode II (scoped production) behavior unchanged: any scope violation during v2 still halts the entire batch.

### 6c. Artifacts (added to v1's set)

- `playwright-report-stage2.5/` (HTML report of dedicated v2 test run)
- `.autospec/invariants-result.json` (gate JSON for Stage 2.5)
- `.autospec/seed-shape-report.json` (edge-case seed handshake result)
- `traces/stage2.5/` (Playwright traces of any v2 failures)

## 7. Dependencies & scope boundaries

| Dependency | Status | Failure mode |
|---|---|---|
| v1 Skill A scaffold (#319–#328) | in flight | v2 depends on #328 (Phase 10 SKILL.md) |
| Skill C (clone provisioner) | follow-on spec | if `edge_case_seeds` declared → `e2e:contract-error` until C ships |
| `@autospec/test` npm package | new; published from this skill | wizard installs into target repo |
| Playwright ≥ 1.40 in target | per-repo | autodetect per v1; missing → `e2e:contract-error` |

### Skill family map (updated)

```
autospec-test           (Skill A — v1 spec + this v2 extension)
   ├─ depends on
autospec-e2e-clone      (Skill C — follow-on spec)            clone provisioning + edge-case seeding
   └─ shared types
autospec-test-bootstrap (Skill D — future)                     bootstraps unit + Playwright + invariants for empty repos
```

## 8. Testing the v2 surface

### 8a. New synthetic targets

| Target | Purpose |
|---|---|
| `target-invariant-bait/` | App where last-item-in-foldout deliberately renders as `<span>`. Golden: Metric F catches it; loop adds invariant + product fix; gate re-runs green. |
| `target-window-mismatch-bait/` | Dashboard renders `data-window-days="7"`; backend always queries 3 days. Golden: Metric G catches it; loop fixes backend; assertion-shift correctly allows widening `window_params.from`. |
| `target-contract-symmetry-bait/` | UI shows `streak-task-t-xyz` for `2026-05-14`; backend `/api/timeline?from=2026-05-14&to=2026-05-14` returns empty. Golden: Metric I catches it. |

`target-greenwash-bait` extended with two invariant declarations the loop is tempted to delete; golden asserts they remain LOOSENING-blocked.

### 8b. Unit tests

- Invariant kind runners — fixture trees + real Playwright against synthetic targets
- Window-contract date math (`today - $N days` parser with timezone fixtures)
- Contract symmetry JSONPath evaluator — `(ui_tuple, api_response, expected_pass)` table
- Edge-case seed shape predicates — SQL + JSONPath verifiers against fixture rows
- Assertion-shift v2 buckets — fixture diffs of `.autospec/test.yml` covering every row of §5c table

### 8c. `@autospec/test` package publish flow

Added to `skills/autospec-test/scripts/publish-helpers.sh`. Versioned 1.0.0 first release; semver after. CI step verifies tarball install + import works in a clean Node sandbox.

### 8d. Lock-step lint (per saved memory)

`validate.sh` checks must extend with: presence of `invariants_v2:` example block in SKILL.md, presence of `edge_case_seeds:` example, adapter row updated.

## 9. Decision log (incremental — v1 log entries remain valid)

| Q | Decision | Rationale |
|---|---|---|
| Same skill vs new family? | Same skill, v2 extends | Reuses safety + loop infra; no SKILL.md fork |
| Stage 2 vs Stage 2.5 | New Stage 2.5 | Different fixtures; muddy diagnosis otherwise |
| Declarative + imperative? | Both | Gate needs declarative; team-authored tests need imperative; share kind-modules |
| Where do seeds live? | Declared in Skill A, provisioned by Skill C | Single source of truth; clear handshake |
| Loop can add invariants? | Yes (STRENGTHENING); cannot remove (LOOSENING) | Same anti-greenwash as v1 |
| Loop fixes Skill-C concerns? | No, refuses-to-run | Layer separation |
| Crawler default threshold | `max_unaffordable_elements: 0` | Hard zero by default; operator opts in to slack |
| `require_count_at_least` on invariants? | Yes, ≥1 default | Prevents vacuous truth from broken selectors |
| Affordance opens require close-and-return? | Yes | Catches modals that don't dismiss, broken back navigation |
| Window-contract tolerance? | ±1 day default, per-contract overridable | Timezone slippage realism |
| Ship npm package? | Yes, `@autospec/test` | Avoid vendor copy-paste; single source of truth for helpers |

## 10. Open follow-ups (separate specs)

1. **Skill C — autospec-e2e-clone:** must consume `edge_case_seeds.require_shapes` and provision matching rows.
2. **Visual regression / pixel diff:** screenshot baselines — different concern, future v3.
3. **Performance budgets:** latency, payload size — future v3.
4. **Accessibility audit:** axe-core integration — separate skill candidate.
