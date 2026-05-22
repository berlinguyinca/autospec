/**
 * affordance-verifier.mjs — Verifies interactive affordances on a Playwright page.
 *
 * For each element matching pattern.element:
 *   1. Capture pre-click DOM snapshot (focused element tag, body class list)
 *   2. Assert element has an interactive role (button, link, menuitem)
 *   3. Click the element
 *   4. Assert pattern.opens becomes visible within timeout (default 5s)
 *   5. Click pattern.closes_via
 *   6. Assert pattern.opens is hidden
 *   7. Best-effort: assert body classes restored (not hard-fail)
 *
 * Returns Array<{ route, element_index, passed, failure_reason? }>
 */

const INTERACTIVE_ROLES = new Set(['button', 'link', 'menuitem']);
const OPEN_TIMEOUT_MS = parseInt(process.env.AUTOSPEC_AFFORDANCE_OPEN_TIMEOUT_MS ?? '5000', 10);

/**
 * @param {import('playwright').Page} page
 * @param {{ element: string, opens: string, closes_via: string }} pattern
 * @param {string} route
 * @returns {Promise<Array<{ route: string, element_index: number, passed: boolean, failure_reason?: string }>>}
 */
export async function verifyAffordance(page, pattern, route) {
  const results = [];

  const elements = page.locator(pattern.element);
  const count = await elements.count();

  for (let i = 0; i < count; i++) {
    const el = elements.nth(i);

    // Skip invisible elements
    const isVisible = await el.isVisible().catch(() => false);
    if (!isVisible) {
      results.push({
        route,
        element_index: i,
        passed: true, // invisible elements are not affordances to verify
        skipped: true,
        failure_reason: undefined,
      });
      continue;
    }

    // Capture pre-click snapshot
    const preSnapshot = await page.evaluate(() => ({
      focusedTag: document.activeElement?.tagName ?? '',
      bodyClasses: document.body.className,
    })).catch(() => ({ focusedTag: '', bodyClasses: '' }));

    // Check role
    const role = await el.evaluate(node => {
      const explicit = node.getAttribute('role');
      if (explicit) return explicit.toLowerCase();
      const tag = node.tagName.toLowerCase();
      if (tag === 'button') return 'button';
      if (tag === 'a') return 'link';
      return '';
    }).catch(() => '');

    if (!INTERACTIVE_ROLES.has(role)) {
      results.push({
        route,
        element_index: i,
        passed: false,
        failure_reason: `element at index ${i} has non-interactive role "${role}" (expected one of: ${[...INTERACTIVE_ROLES].join(', ')})`,
      });
      continue;
    }

    // Click the element
    try {
      await el.click({ timeout: 3000 });
    } catch (e) {
      results.push({
        route,
        element_index: i,
        passed: false,
        failure_reason: `click failed: ${e.message}`,
      });
      continue;
    }

    // Assert opens becomes visible
    try {
      await page.locator(pattern.opens).waitFor({ state: 'visible', timeout: OPEN_TIMEOUT_MS });
    } catch {
      results.push({
        route,
        element_index: i,
        passed: false,
        failure_reason: `"${pattern.opens}" did not become visible within ${OPEN_TIMEOUT_MS}ms after clicking element at index ${i}`,
      });
      continue;
    }

    // Click closes_via
    try {
      await page.locator(pattern.closes_via).click({ timeout: 3000 });
    } catch (e) {
      results.push({
        route,
        element_index: i,
        passed: false,
        failure_reason: `closes_via click failed: ${e.message}`,
      });
      continue;
    }

    // Assert opens is hidden
    try {
      await page.locator(pattern.opens).waitFor({ state: 'hidden', timeout: OPEN_TIMEOUT_MS });
    } catch {
      results.push({
        route,
        element_index: i,
        passed: false,
        failure_reason: `"${pattern.opens}" did not hide within ${OPEN_TIMEOUT_MS}ms after clicking closes_via`,
      });
      continue;
    }

    // Best-effort post-close snapshot check (body classes)
    const postSnapshot = await page.evaluate(() => ({
      bodyClasses: document.body.className,
    })).catch(() => ({ bodyClasses: '' }));

    const bodyRestored = preSnapshot.bodyClasses === postSnapshot.bodyClasses;
    // Body class mismatch is a warning, not a hard failure — modal libraries vary
    const snapshotNote = bodyRestored ? undefined : `body classes changed: "${preSnapshot.bodyClasses}" → "${postSnapshot.bodyClasses}"`;

    results.push({
      route,
      element_index: i,
      passed: true,
      failure_reason: undefined,
      ...(snapshotNote ? { snapshot_note: snapshotNote } : {}),
    });
  }

  return results;
}
