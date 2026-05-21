/**
 * every-modal-returns-to-body-scroll.mjs — Built-in kind: every_modal_returns_to_body_scroll
 *
 * Captures document.body.style.overflow before opening a modal, opens it via
 * `modal_open` selector, asserts the modal becomes visible, closes it via
 * `modal_close` selector, and asserts that overflow is restored to the captured value.
 *
 * KindResult shape:
 *   { passed: boolean, violations: Array<{index,selector,reason}>, count_observed: number }
 */

export const id = 'every_modal_returns_to_body_scroll';

export const signature = {
  params: {
    modal_open: { type: 'string', description: 'Selector for the trigger that opens the modal.' },
    modal_selector: { type: 'string', description: 'Selector for the modal element that should become visible.' },
    modal_close: { type: 'string', description: 'Selector for the trigger that closes the modal.' },
  },
  required: ['modal_open', 'modal_selector', 'modal_close'],
};

/**
 * @param {import('playwright').Page} page
 * @param {object} params
 * @param {{ baseUrl: string, route: string }} _ctx
 * @returns {Promise<{passed: boolean, violations: Array<{index: number, selector: string, reason: string}>, count_observed: number}>}
 */
export async function run(page, params, _ctx) {
  const { modal_open: openSel, modal_selector: modalSel, modal_close: closeSel } = params;

  const violations = [];

  // Capture body overflow before opening
  const overflowBefore = await page.evaluate(() => document.body.style.overflow || '').catch(() => '');

  // Open the modal
  const openTrigger = page.locator(openSel);
  const openCount = await openTrigger.count();
  if (openCount === 0) {
    return {
      passed: false,
      violations: [{ index: 0, selector: openSel, reason: 'modal_open selector matched no elements' }],
      count_observed: 0,
    };
  }

  await openTrigger.first().click().catch((e) => {
    violations.push({ index: 0, selector: openSel, reason: `modal_open click failed: ${e.message}` });
  });

  // Assert modal is visible
  const modal = page.locator(modalSel);
  const modalVisible = await modal.isVisible().catch(() => false);
  if (!modalVisible) {
    violations.push({ index: 0, selector: modalSel, reason: 'modal not visible after modal_open click' });
  }

  // Close the modal
  const closeTrigger = page.locator(closeSel);
  await closeTrigger.first().click().catch((e) => {
    violations.push({ index: 0, selector: closeSel, reason: `modal_close click failed: ${e.message}` });
  });

  // Assert modal is no longer visible
  const stillVisible = await modal.isVisible().catch(() => true);
  if (stillVisible) {
    violations.push({ index: 0, selector: modalSel, reason: 'modal still visible after modal_close click' });
  }

  // Assert body overflow is restored
  const overflowAfter = await page.evaluate(() => document.body.style.overflow || '').catch(() => '');
  if (overflowAfter !== overflowBefore) {
    violations.push({
      index: 0,
      selector: 'document.body',
      reason: `body overflow not restored: was "${overflowBefore}", now "${overflowAfter}"`,
    });
  }

  return { passed: violations.length === 0, violations, count_observed: openCount };
}
