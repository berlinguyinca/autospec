/**
 * every-visible-x-is-y.mjs — Built-in invariant kind: every_visible_X_is_Y
 *
 * For each element matching `visible`, asserts that `action` is present and
 * clickable, that clicking it makes `verifies_open` visible (if declared),
 * and that clicking `verifies_close` hides `verifies_open` again.
 *
 * KindResult shape:
 *   { passed: boolean, violations: Array<{index,selector,reason}>, count_observed: number }
 */

export const id = 'every_visible_X_is_Y';

export const signature = {
  params: {
    visible: { type: 'string', description: 'Selector for the visible container elements.' },
    action: { type: 'string', description: 'Selector for the action element within each container.' },
    verifies_open: { type: 'string', description: 'Selector that must be visible after clicking action (optional).' },
    verifies_close: { type: 'string', description: 'Selector to click to close the opened element (optional).' },
    require_count_at_least: { type: 'integer', minimum: 1, description: 'Minimum number of visible elements required.' },
  },
  required: ['visible', 'action'],
};

/**
 * @param {import('playwright').Page} page
 * @param {object} params - Invariant parameters per signature above.
 * @param {{ baseUrl: string, route: string }} ctx
 * @returns {Promise<{passed: boolean, violations: Array<{index: number, selector: string, reason: string}>, count_observed: number}>}
 */
export async function run(page, params, _ctx) {
  const {
    visible: visibleSel,
    action: actionSel,
    verifies_open: verifiesOpenSel,
    verifies_close: verifiesCloseSel,
    require_count_at_least: minCount = 1,
  } = params;

  const violations = [];
  const items = page.locator(visibleSel);
  const count = await items.count();

  if (count < minCount) {
    violations.push({
      index: -1,
      selector: visibleSel,
      reason: `require_count_at_least=${minCount} but only ${count} elements matched`,
    });
    return { passed: false, violations, count_observed: count };
  }

  for (let i = 0; i < count; i++) {
    const row = items.nth(i);

    // Assert action is visible within the row
    const action = row.locator(actionSel);
    const actionVisible = await action.isVisible().catch(() => false);
    if (!actionVisible) {
      violations.push({ index: i, selector: actionSel, reason: 'action not visible' });
      continue;
    }

    // Click the action
    await action.click().catch((e) => {
      violations.push({ index: i, selector: actionSel, reason: `click failed: ${e.message}` });
    });

    // Assert verifies_open becomes visible
    if (verifiesOpenSel) {
      const opened = page.locator(verifiesOpenSel);
      const isOpen = await opened.isVisible().catch(() => false);
      if (!isOpen) {
        violations.push({ index: i, selector: verifiesOpenSel, reason: 'verifies_open not visible after action click' });
      } else if (verifiesCloseSel) {
        // Click close and assert verifies_open disappears
        const closeBtn = page.locator(verifiesCloseSel);
        await closeBtn.click().catch((e) => {
          violations.push({ index: i, selector: verifiesCloseSel, reason: `close click failed: ${e.message}` });
        });
        const stillOpen = await opened.isVisible().catch(() => true);
        if (stillOpen) {
          violations.push({ index: i, selector: verifiesOpenSel, reason: 'verifies_open still visible after close click' });
        }
      }
    }
  }

  return { passed: violations.length === 0, violations, count_observed: count };
}
