/**
 * every-row-has-required-actions.mjs — Built-in invariant kind: every_row_has_required_actions
 *
 * For each element matching `row`, asserts that every selector in `required_actions`
 * is visible and interactive (is a button or link role) within the row.
 *
 * KindResult shape:
 *   { passed: boolean, violations: Array<{index,selector,reason}>, count_observed: number }
 */

export const id = 'every_row_has_required_actions';

export const signature = {
  params: {
    row: { type: 'string', description: 'Selector for row container elements.' },
    required_actions: {
      type: 'array',
      items: { type: 'string' },
      description: 'List of selectors that must each be visible and interactive within every row.',
    },
  },
  required: ['row', 'required_actions'],
};

/**
 * @param {import('playwright').Page} page
 * @param {object} params
 * @param {{ baseUrl: string, route: string }} _ctx
 * @returns {Promise<{passed: boolean, violations: Array<{index: number, selector: string, reason: string}>, count_observed: number}>}
 */
export async function run(page, params, _ctx) {
  const { row: rowSel, required_actions: requiredActions = [] } = params;

  const violations = [];
  const rows = page.locator(rowSel);
  const count = await rows.count();

  for (let i = 0; i < count; i++) {
    const row = rows.nth(i);
    for (const actionSel of requiredActions) {
      const action = row.locator(actionSel);
      const isVisible = await action.isVisible().catch(() => false);
      if (!isVisible) {
        violations.push({
          index: i,
          selector: actionSel,
          reason: `required action "${actionSel}" not visible in row ${i}`,
        });
        continue;
      }

      // Assert it's interactive: enabled and has button/link role or is a native button/a
      const isEnabled = await action.isEnabled().catch(() => false);
      if (!isEnabled) {
        violations.push({
          index: i,
          selector: actionSel,
          reason: `required action "${actionSel}" is visible but disabled in row ${i}`,
        });
      }
    }
  }

  return { passed: violations.length === 0, violations, count_observed: count };
}
