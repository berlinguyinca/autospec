/**
 * every-visible-x-has-accessible-name.mjs — Built-in kind: every_visible_X_has_accessible_name
 *
 * Enumerates all visible interactive elements (buttons, links, inputs) on the page
 * and asserts that each has a non-empty accessible name (aria-label, text content,
 * or associated label).
 *
 * KindResult shape:
 *   { passed: boolean, violations: Array<{index,selector,reason}>, count_observed: number }
 */

export const id = 'every_visible_X_has_accessible_name';

export const signature = {
  params: {
    scope: {
      type: 'string',
      description: 'Optional CSS selector to limit the scope of the check. Defaults to the full page.',
    },
    element_types: {
      type: 'array',
      items: { type: 'string' },
      description: 'Element selectors to check. Defaults to: button, [role=button], a[href], input, select, textarea.',
    },
  },
  required: [],
};

const DEFAULT_ELEMENT_TYPES = ['button', '[role=button]', 'a[href]', 'input', 'select', 'textarea'];

/**
 * @param {import('playwright').Page} page
 * @param {object} params
 * @param {{ baseUrl: string, route: string }} _ctx
 * @returns {Promise<{passed: boolean, violations: Array<{index: number, selector: string, reason: string}>, count_observed: number}>}
 */
export async function run(page, params, _ctx) {
  const { scope, element_types: elementTypes = DEFAULT_ELEMENT_TYPES } = params;

  const violations = [];
  let count_observed = 0;

  const root = scope ? page.locator(scope) : page;
  const selector = elementTypes.join(', ');
  const elements = (scope ? root.locator(selector) : page.locator(selector));
  const count = await elements.count();

  for (let i = 0; i < count; i++) {
    const el = elements.nth(i);
    const isVisible = await el.isVisible().catch(() => false);
    if (!isVisible) continue;

    count_observed++;

    // Get accessible name via evaluate (textContent + aria-label + aria-labelledby)
    const accessibleName = await el.evaluate((node) => {
      const ariaLabel = node.getAttribute('aria-label') || '';
      if (ariaLabel.trim()) return ariaLabel.trim();
      const labelledBy = node.getAttribute('aria-labelledby');
      if (labelledBy) {
        const labelEl = document.getElementById(labelledBy);
        if (labelEl && labelEl.textContent.trim()) return labelEl.textContent.trim();
      }
      // For inputs, check associated <label>
      if (node.id) {
        const label = document.querySelector(`label[for="${node.id}"]`);
        if (label && label.textContent.trim()) return label.textContent.trim();
      }
      // Text content for buttons/links
      const text = (node.textContent || '').trim();
      return text;
    }).catch(() => '');

    if (!accessibleName) {
      const tagName = await el.evaluate((n) => n.tagName.toLowerCase()).catch(() => 'unknown');
      violations.push({
        index: i,
        selector: tagName,
        reason: `element <${tagName}> at index ${i} has no accessible name (aria-label, text, or label)`,
      });
    }
  }

  return { passed: violations.length === 0, violations, count_observed };
}
