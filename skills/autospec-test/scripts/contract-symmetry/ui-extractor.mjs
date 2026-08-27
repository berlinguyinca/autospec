/**
 * ui-extractor.mjs — UI tuple extractor for Metric I contract symmetry.
 *
 * Navigates to a route and extracts attribute tuples from DOM elements
 * matching the ui_source.extract selector. Each matched element produces
 * one tuple via the ui_source.per_match mapping (tuple key → DOM attribute
 * name), matching the contract convention documented in
 * docs/specs/2026-05-21-autospec-test-invariants-design.md:
 *   per_match: { task_id: 'data-task-id', date: 'data-date' }
 * i.e. the object KEY is the logical field name used later in
 * api_target.path_template (${task_id}, ${date}...), and the VALUE is the
 * DOM attribute to read it from.
 *
 * Elements with any null attribute value are warned and skipped.
 *
 * Returns Array<Record<string, string>> — one record per matched element.
 */

/**
 * @param {import('playwright').Page} page
 * @param {string} route  — full URL to navigate to
 * @param {{ extract: string, per_match: Record<string, string> }} ui_source
 * @returns {Promise<Array<Record<string, string>>>}
 */
export async function extract(page, route, ui_source) {
  await page.goto(route, { waitUntil: 'domcontentloaded', timeout: 15_000 });

  const { extract: selector, per_match } = ui_source;
  const elements = page.locator(selector);
  const count = await elements.count();

  const tuples = [];

  for (let i = 0; i < count; i++) {
    const el = elements.nth(i);
    const tuple = {};
    let hasNull = false;

    for (const [tupleKey, attrName] of Object.entries(per_match)) {
      const value = await el.getAttribute(attrName);
      if (value === null) {
        process.stderr.write(
          `[ui-extractor] warn: element ${i} at selector "${selector}" missing attribute "${attrName}"; skipping tuple\n`,
        );
        hasNull = true;
        break;
      }
      tuple[tupleKey] = value;
    }

    // Optional access metadata is extracted separately so existing tuple
    // contracts remain byte-compatible. A contract may name a custom guard
    // attribute; otherwise common middleware attributes are consulted.
    const guardAttr = ui_source.guard_attr || 'data-guard';
    const guard = await el.getAttribute(guardAttr)
      ?? await el.getAttribute('data-middleware')
      ?? await el.getAttribute('data-access-guard');
    if (guard !== null) tuple.guard = guard;

    if (!hasNull) {
      tuples.push(tuple);
    }
  }

  return tuples;
}
