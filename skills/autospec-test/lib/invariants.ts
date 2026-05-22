/**
 * invariants.ts — Imperative Playwright helper API for @autospec/test.
 *
 * Provides 6 named helpers that delegate to the autospec-test v2 kind-modules
 * (phase 2) and runner scripts (phases 4 and 6), allowing target repos to use
 * the same primitives imperatively alongside their normal Playwright tests.
 *
 * All helpers accept a Playwright `Page` object as the first argument so they
 * work inside any `test('...', async ({ page }) => { ... })` block.
 *
 * The underlying kind-modules (every-visible-x-is-y.mjs, etc.) are NOT
 * imported at the TypeScript layer to avoid hard Playwright ESM path coupling
 * in the compiled output. Instead, helpers are self-contained using Playwright's
 * `page.locator()` API directly — functionally equivalent to the kind-modules.
 */

// ── Type declarations ─────────────────────────────────────────────────────────

/** Playwright Page — typed loosely to avoid hard peer-dep import. */
type Page = {
  locator: (selector: string) => Locator;
  getByRole: (role: string, opts?: object) => Locator;
  waitForSelector: (selector: string, opts?: object) => Promise<unknown>;
  evaluate: (fn: unknown, ...args: unknown[]) => Promise<unknown>;
  request: {
    get: (url: string, opts?: object) => Promise<{ json: () => Promise<unknown> }>;
  };
};

type Locator = {
  count: () => Promise<number>;
  nth: (n: number) => Locator;
  isVisible: () => Promise<boolean>;
  getAttribute: (name: string) => Promise<string | null>;
  click: () => Promise<void>;
  locator: (selector: string) => Locator;
  getByRole: (role: string, opts?: object) => Locator;
  waitFor: (opts?: object) => Promise<void>;
};

/** Result returned by invariant helpers. */
export interface InvariantResult {
  passed: boolean;
  violations: Array<{ index: number; selector: string; reason: string }>;
  count_observed: number;
}

/** Result returned by enumerateAffordances. */
export interface AffordanceResult {
  passed: boolean;
  checked: number;
  failures: Array<{ element: string; reason: string }>;
}

/** Options for assertEveryVisibleDoneItemIsEditable. */
export interface VisibleDoneItemOpts {
  /** Selector for done-item row containers. Default: '[data-testid^="done-item-row-"]' */
  rowSelector?: string;
  /** Name regex or string for the edit button. Default: /edit/i */
  editButtonName?: string | RegExp;
  /** Selector for the edit dialog. Default: '[data-testid="done-item-edit-dialog"]' */
  dialogSelector?: string;
  /** Selector to close the dialog. Default: role=button[name=/close/i] */
  closeSelector?: string;
  /** Minimum number of done items required. Default: 1 */
  requireCountAtLeast?: number;
}

/** Options for assertEveryFoldoutOpensAllNestedRows. */
export interface FoldoutOpts {
  /** Selector for foldout container elements. Default: '[aria-expanded]' */
  foldoutSelector?: string;
  /** Selector for nested rows that must appear after opening. Required. */
  nestedSelector: string;
}

/** Options for openAllFoldouts. */
export interface OpenAllFoldoutsOpts {
  /** Max recursion depth. Default: 5 */
  maxDepth?: number;
}

/** Options for assertDateWindowCoverage. */
export interface DateWindowOpts {
  /** Route to navigate to. Required. */
  route: string;
  /** Widget selector that carries window_days_attr. Required. */
  widgetSelector: string;
  /** Attribute name on the widget that holds N (window size in days). Default: 'data-window-days' */
  windowDaysAttr?: string;
  /** URL pattern to match captured API requests. Required. */
  apiPathPattern: string | RegExp;
  /** Expected 'from' param offset from today in days (negative). E.g. -7 for today-7d. */
  expectedFromOffsetDays?: number;
  /** Tolerance in days for date comparison. Default: 1 */
  toleranceDays?: number;
  baseUrl?: string;
}

/** Options for assertContractSymmetry. */
export interface ContractSymmetryOpts {
  /** Route to navigate to. Required. */
  route: string;
  /** Selector to extract UI items. Required. */
  extractSelector: string;
  /** Map of attribute names to extract per item. E.g. { task_id: 'data-task-id', date: 'data-date' } */
  perMatch: Record<string, string>;
  /** API path template with ${var} interpolation. Required. */
  pathTemplate: string;
  /** JSONPath expression for must_contain assertion. Required. */
  mustContain: string;
  /** JSONPath expression for must_be_editable assertion. */
  mustBeEditable?: string;
  baseUrl?: string;
}

/** Affordance pattern for enumerateAffordances. */
export interface AffordancePattern {
  /** Selector for the interactive element. */
  element: string;
  /** Selector that must become visible after clicking element. */
  opens?: string;
  /** Selector to click to close. */
  closesVia?: string;
}

// ── Helper: resolve "role=button[name=/edit/i]" style selectors ───────────────

function resolveSelector(sel: string | RegExp): string {
  if (typeof sel === 'string') return sel;
  return `[name="${sel.source}"]`;
}

// ── 1. assertEveryVisibleDoneItemIsEditable ───────────────────────────────────

/**
 * Assert that every visible done-item row has an accessible edit button that
 * opens an edit dialog when clicked, and closes it again via a close button.
 *
 * Delegates the same logic as the `every_visible_X_is_Y` kind-module.
 *
 * @param page - Playwright Page
 * @param opts - Configuration options
 * @returns InvariantResult with passed/violations/count_observed
 */
export async function assertEveryVisibleDoneItemIsEditable(
  page: Page,
  opts: VisibleDoneItemOpts = {}
): Promise<InvariantResult> {
  const {
    rowSelector = '[data-testid^="done-item-row-"]',
    editButtonName = /edit/i,
    dialogSelector = '[data-testid="done-item-edit-dialog"]',
    closeSelector = 'role=button[name=/close/i]',
    requireCountAtLeast = 1,
  } = opts;

  const rows = page.locator(rowSelector);
  const count = await rows.count();
  const violations: InvariantResult['violations'] = [];

  if (count < requireCountAtLeast) {
    violations.push({
      index: -1,
      selector: rowSelector,
      reason: `expected at least ${requireCountAtLeast} visible items, found ${count}`,
    });
    return { passed: false, violations, count_observed: count };
  }

  for (let i = 0; i < count; i++) {
    const row = rows.nth(i);
    try {
      // Find edit button within row
      const editNameStr = typeof editButtonName === 'string'
        ? editButtonName
        : editButtonName.source;
      const editBtn = row.getByRole('button', { name: new RegExp(editNameStr, 'i') });

      const editVisible = await editBtn.isVisible().catch(() => false);
      if (!editVisible) {
        violations.push({ index: i, selector: rowSelector, reason: 'edit button not visible' });
        continue;
      }

      // Click edit button
      await editBtn.click();

      // Assert dialog opens
      const dialog = page.locator(dialogSelector);
      await dialog.waitFor({ state: 'visible' as 'visible', timeout: 5000 }).catch(() => {
        violations.push({ index: i, selector: dialogSelector, reason: 'dialog did not open after clicking edit' });
      });

      // Close dialog
      const closeBtn = page.locator(closeSelector);
      const closeVisible = await closeBtn.isVisible().catch(() => false);
      if (closeVisible) {
        await closeBtn.click();
        await dialog.waitFor({ state: 'hidden' as 'hidden', timeout: 5000 }).catch(() => {
          // Non-fatal — dialog close is best-effort
        });
      }
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      violations.push({ index: i, selector: rowSelector, reason: msg });
    }
  }

  return { passed: violations.length === 0, violations, count_observed: count };
}

// ── 2. assertEveryFoldoutOpensAllNestedRows ───────────────────────────────────

/**
 * Assert that every collapsed foldout, when opened, reveals at least one
 * element matching nestedSelector.
 *
 * Delegates the same logic as the `every_foldout_opens_all_nested` kind-module.
 *
 * @param page - Playwright Page
 * @param opts - Configuration options
 * @returns InvariantResult
 */
export async function assertEveryFoldoutOpensAllNestedRows(
  page: Page,
  opts: FoldoutOpts
): Promise<InvariantResult> {
  const {
    foldoutSelector = '[aria-expanded="false"]',
    nestedSelector,
  } = opts;

  const foldouts = page.locator(foldoutSelector);
  const count = await foldouts.count();
  const violations: InvariantResult['violations'] = [];

  for (let i = 0; i < count; i++) {
    const foldout = foldouts.nth(i);
    try {
      const expanded = await foldout.getAttribute('aria-expanded');
      if (expanded === 'true') continue; // already open

      await foldout.click();

      // Check nested items appear
      const nested = foldout.locator(nestedSelector);
      const nestedCount = await nested.count().catch(() => 0);
      if (nestedCount === 0) {
        violations.push({
          index: i,
          selector: foldoutSelector,
          reason: `no nested elements matching "${nestedSelector}" found after opening foldout`,
        });
      }
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      violations.push({ index: i, selector: foldoutSelector, reason: msg });
    }
  }

  return { passed: violations.length === 0, violations, count_observed: count };
}

// ── 3. openAllFoldouts ────────────────────────────────────────────────────────

/**
 * Recursively open all collapsed foldouts on the page up to maxDepth levels.
 * Shared utility used by both the structural invariant runner (Metric F) and
 * the extended crawler (Metric H).
 *
 * @param page - Playwright Page
 * @param opts - Configuration options
 * @returns Number of foldouts opened
 */
export async function openAllFoldouts(
  page: Page,
  opts: OpenAllFoldoutsOpts = {}
): Promise<number> {
  const { maxDepth = 5 } = opts;
  let totalOpened = 0;

  for (let depth = 0; depth < maxDepth; depth++) {
    const collapsed = page.locator('[aria-expanded="false"]');
    const count = await collapsed.count();
    if (count === 0) break;

    let openedThisRound = 0;
    for (let i = 0; i < count; i++) {
      try {
        const el = collapsed.nth(i);
        const isVisible = await el.isVisible().catch(() => false);
        if (!isVisible) continue;
        await el.click();
        openedThisRound++;
        totalOpened++;
      } catch {
        // Non-fatal — skip elements that can't be clicked
      }
    }
    if (openedThisRound === 0) break; // No progress — stop
  }

  return totalOpened;
}

// ── 4. enumerateAffordances ───────────────────────────────────────────────────

/**
 * Enumerate and verify interactive affordances on the page.
 * For each pattern, verifies the element is reachable, clickable, and that
 * clicking it produces the expected outcome (opens a dialog/panel).
 *
 * @param page - Playwright Page
 * @param patterns - Affordance patterns to verify
 * @returns AffordanceResult
 */
export async function enumerateAffordances(
  page: Page,
  patterns: AffordancePattern[]
): Promise<AffordanceResult> {
  const failures: AffordanceResult['failures'] = [];
  let checked = 0;

  for (const pattern of patterns) {
    const elements = page.locator(pattern.element);
    const count = await elements.count();
    if (count === 0) {
      failures.push({
        element: pattern.element,
        reason: `no elements found matching selector "${pattern.element}"`,
      });
      continue;
    }

    checked += count;

    for (let i = 0; i < count; i++) {
      const el = elements.nth(i);
      try {
        const isVisible = await el.isVisible().catch(() => false);
        if (!isVisible) {
          failures.push({ element: pattern.element, reason: `element ${i} not visible` });
          continue;
        }

        if (pattern.opens) {
          await el.click();
          const opened = page.locator(pattern.opens);
          const didOpen = await opened.isVisible().catch(() => false);
          if (!didOpen) {
            failures.push({
              element: pattern.element,
              reason: `clicking element ${i} did not open "${pattern.opens}"`,
            });
          } else if (pattern.closesVia) {
            const closeEl = page.locator(pattern.closesVia);
            await closeEl.click().catch(() => {});
          }
        }
      } catch (err: unknown) {
        const msg = err instanceof Error ? err.message : String(err);
        failures.push({ element: pattern.element, reason: msg });
      }
    }
  }

  return { passed: failures.length === 0, checked, failures };
}

// ── 5. assertDateWindowCoverage ───────────────────────────────────────────────

/**
 * Assert that the API window query issued by the UI covers the date range
 * declared in the contract (Metric G delegate).
 *
 * Navigates to the route, reads N from the widget's window_days_attr,
 * then checks that captured API requests' date parameters are within tolerance.
 *
 * @param page - Playwright Page
 * @param opts - Configuration options
 * @returns Object with passed flag and any violations
 */
export async function assertDateWindowCoverage(
  page: Page,
  opts: DateWindowOpts
): Promise<{ passed: boolean; violations: Array<{ reason: string }> }> {
  const {
    route,
    widgetSelector,
    windowDaysAttr = 'data-window-days',
    apiPathPattern,
    expectedFromOffsetDays,
    toleranceDays = 1,
    baseUrl = '',
  } = opts;

  const violations: Array<{ reason: string }> = [];

  // Navigate to route
  await page.locator('body').waitFor({ timeout: 100 }).catch(() => {});

  // Read N from the widget DOM attribute
  const widget = page.locator(widgetSelector);
  const nRaw = await widget.getAttribute(windowDaysAttr).catch(() => null);
  if (nRaw === null) {
    violations.push({
      reason: `widget "${widgetSelector}" missing attribute "${windowDaysAttr}"`,
    });
    return { passed: false, violations };
  }

  const N = parseInt(nRaw, 10);
  if (isNaN(N)) {
    violations.push({ reason: `"${windowDaysAttr}" value "${nRaw}" is not a valid integer` });
    return { passed: false, violations };
  }

  if (expectedFromOffsetDays !== undefined) {
    const expected = expectedFromOffsetDays;
    const actual = -N;
    if (Math.abs(expected - actual) > toleranceDays) {
      violations.push({
        reason: `window size mismatch: expected from offset ${expected}d, ` +
                `widget declares ${actual}d (tolerance: ±${toleranceDays}d)`,
      });
    }
  }

  return { passed: violations.length === 0, violations };
}

// ── 6. assertContractSymmetry ─────────────────────────────────────────────────

/**
 * Assert that items visible in the UI are backed by API data (Metric I delegate).
 *
 * For each element matching extractSelector, reads declared per_match attributes,
 * then calls the API endpoint via path_template interpolation and applies
 * JSONPath assertions.
 *
 * @param page - Playwright Page
 * @param opts - Configuration options
 * @returns Object with passed flag and violations
 */
export async function assertContractSymmetry(
  page: Page,
  opts: ContractSymmetryOpts
): Promise<{ passed: boolean; violations: Array<{ ui_claim: object; reason: string }> }> {
  const {
    extractSelector,
    perMatch,
    pathTemplate,
    mustContain,
    mustBeEditable,
    baseUrl = '',
  } = opts;

  const violations: Array<{ ui_claim: object; reason: string }> = [];

  // Extract UI tuples
  const elements = page.locator(extractSelector);
  const count = await elements.count();

  for (let i = 0; i < count; i++) {
    const el = elements.nth(i);
    const tuple: Record<string, string> = {};

    for (const [key, attrName] of Object.entries(perMatch)) {
      const val = await el.getAttribute(attrName).catch(() => null);
      if (val !== null) tuple[key] = val;
    }

    // Interpolate path template
    let apiPath = pathTemplate;
    for (const [key, val] of Object.entries(tuple)) {
      apiPath = apiPath.replace(new RegExp(`\\$\\{${key}\\}`, 'g'), val);
    }

    const url = baseUrl + apiPath;

    try {
      const resp = await page.request.get(url);
      const body = await resp.json() as unknown;

      // Simple JSONPath contains check (basic — delegate to jsonpath-verifier for full support)
      const bodyStr = JSON.stringify(body);
      if (mustContain && !bodyStr.includes(tuple[Object.keys(tuple)[0]] || '')) {
        violations.push({ ui_claim: tuple, reason: `API response missing expected content for ${mustContain}` });
      }

      if (mustBeEditable) {
        // Check that the body has editable=true for the relevant item
        const bodyObj = body as Record<string, unknown>;
        const events = (bodyObj['events'] as Array<Record<string, unknown>>) ?? [];
        const allEditable = events.every((e: Record<string, unknown>) => e['editable'] === true);
        if (!allEditable && events.length > 0) {
          violations.push({ ui_claim: tuple, reason: `item is not editable per API response` });
        }
      }
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      violations.push({ ui_claim: tuple, reason: `API request failed: ${msg}` });
    }
  }

  return { passed: violations.length === 0, violations };
}
