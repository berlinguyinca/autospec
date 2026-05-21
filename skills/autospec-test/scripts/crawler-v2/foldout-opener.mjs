/**
 * foldout-opener.mjs — Shared utility: open all collapsed foldouts on a page.
 *
 * Recursively clicks every [aria-expanded=false] element until none remain or
 * maxDepth is reached (to avoid infinite loops on circular structures).
 *
 * Used by:
 *   - run-structural.mjs (Metric F: open_all_foldouts option)
 *   - extended-crawler.mjs (Metric H: open_all_foldouts)
 */

/**
 * @param {import('playwright').Page} page
 * @param {{ maxDepth?: number, selector?: string }} opts
 * @returns {Promise<{ opened_count: number, depth_reached: number }>}
 */
export async function openAllFoldouts(page, opts = {}) {
  const { maxDepth = 5, selector = '[aria-expanded="false"]' } = opts;

  let opened_count = 0;
  let depth_reached = 0;

  for (let depth = 0; depth < maxDepth; depth++) {
    const collapsed = page.locator(selector);
    const count = await collapsed.count();
    if (count === 0) break;

    depth_reached = depth + 1;

    for (let i = 0; i < count; i++) {
      const el = collapsed.nth(i);
      const isVisible = await el.isVisible().catch(() => false);
      if (!isVisible) continue;

      // Click the first button/summary child, or the element itself
      const trigger = el.locator('button, [role=button], summary').first();
      const hasTrigger = (await trigger.count()) > 0;
      if (hasTrigger) {
        await trigger.click().catch(() => {});
      } else {
        await el.click().catch(() => {});
      }
      opened_count++;
    }

    // Small yield to let DOM settle after clicks
    await page.waitForTimeout(50).catch(() => {});
  }

  return { opened_count, depth_reached };
}
