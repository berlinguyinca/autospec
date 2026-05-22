/**
 * date-math.mjs — Pure date expression resolver for Metric G window contracts.
 *
 * Recognises:
 *   "today"               → ctx.today (or Date.now()) formatted as ISO date
 *   "today ± N day[s]"   → ctx.today ± N calendar days
 *   "YYYY-MM-DD"          → returned unchanged (validated format)
 *
 * Returns an ISO date string (YYYY-MM-DD) in UTC unless ctx.tz is supplied
 * (tz support is a future extension; currently UTC only).
 *
 * Throws on any unrecognised expression with message matching /unparseable/.
 */

/**
 * @param {string} expr
 * @param {{ today?: Date, tz?: string }} ctx
 * @returns {string}  ISO date string YYYY-MM-DD
 */
export function resolve(expr, ctx = {}) {
  if (typeof expr !== 'string' || expr.trim() === '') {
    throw new Error(`unparseable date expression: ${JSON.stringify(expr)}`);
  }

  const trimmed = expr.trim();

  // ── ISO literal: YYYY-MM-DD ───────────────────────────────────────────────
  if (/^\d{4}-\d{2}-\d{2}$/.test(trimmed)) {
    return trimmed;
  }

  // ── "today" variants ──────────────────────────────────────────────────────
  const base = ctx.today instanceof Date ? new Date(ctx.today) : new Date();

  // Normalise to midnight UTC
  const baseDate = new Date(Date.UTC(
    base.getUTCFullYear(),
    base.getUTCMonth(),
    base.getUTCDate(),
  ));

  if (trimmed === 'today') {
    return formatISO(baseDate);
  }

  // "today [+-] N day[s]"
  const offsetMatch = trimmed.match(
    /^today\s*([+-])\s*(\d+)\s*days?$/i,
  );
  if (offsetMatch) {
    const sign = offsetMatch[1] === '+' ? 1 : -1;
    const n = parseInt(offsetMatch[2], 10);
    const result = new Date(baseDate);
    result.setUTCDate(result.getUTCDate() + sign * n);
    return formatISO(result);
  }

  throw new Error(`unparseable date expression: ${JSON.stringify(trimmed)}`);
}

/**
 * Format a Date as YYYY-MM-DD using UTC components.
 * @param {Date} d
 * @returns {string}
 */
function formatISO(d) {
  const y = d.getUTCFullYear();
  const m = String(d.getUTCMonth() + 1).padStart(2, '0');
  const dd = String(d.getUTCDate()).padStart(2, '0');
  return `${y}-${m}-${dd}`;
}
