/**
 * every-foldout-opens-all-nested.mjs — Built-in invariant kind: every_foldout_opens_all_nested
 *
 * For each element matching `foldout` that is collapsed (aria-expanded=false or absent),
 * clicks to open it and asserts that at least one element matching
 * `nested_must_be_visible_after_open` becomes visible inside the foldout.
 *
 * KindResult shape:
 *   { passed: boolean, violations: Array<{index,selector,reason}>, count_observed: number }
 */

export const id = 'every_foldout_opens_all_nested';

export const signature = {
  params: {
    foldout: { type: 'string', description: 'Selector for foldout container elements.' },
    nested_must_be_visible_after_open: {
      type: 'string',
      description: 'Selector for nested elements that must be visible after opening each foldout.',
    },
  },
  required: ['foldout', 'nested_must_be_visible_after_open'],
};

/**
 * @param {import('playwright').Page} page
 * @param {object} params
 * @param {{ baseUrl: string, route: string }} _ctx
 * @returns {Promise<{passed: boolean, violations: Array<{index: number, selector: string, reason: string}>, count_observed: number}>}
 */
export async function run(page, params, _ctx) {
  const { foldout: foldoutSel, nested_must_be_visible_after_open: nestedSel } = params;

  const violations = [];
  const foldouts = page.locator(foldoutSel);
  const count = await foldouts.count();

  for (let i = 0; i < count; i++) {
    const foldout = foldouts.nth(i);

    // Click to open if collapsed (aria-expanded=false or absent)
    const expanded = await foldout.getAttribute('aria-expanded').catch(() => null);
    if (expanded !== 'true') {
      // Find a clickable trigger inside or the foldout itself
      const trigger = foldout.locator('button, [role=button], summary').first();
      const hasTrigger = await trigger.count() > 0;
      if (hasTrigger) {
        await trigger.click().catch((e) => {
          violations.push({ index: i, selector: foldoutSel, reason: `open click failed: ${e.message}` });
        });
      } else {
        await foldout.click().catch((e) => {
          violations.push({ index: i, selector: foldoutSel, reason: `open click failed (no trigger): ${e.message}` });
        });
      }
    }

    // Assert nested elements are visible inside the foldout
    const nested = foldout.locator(nestedSel);
    const nestedCount = await nested.count();
    if (nestedCount === 0) {
      violations.push({
        index: i,
        selector: nestedSel,
        reason: `no nested elements matching "${nestedSel}" visible after opening foldout`,
      });
    }
  }

  return { passed: violations.length === 0, violations, count_observed: count };
}
