/**
 * interpolator.mjs — Template variable interpolator for Metric I contract symmetry.
 *
 * Substitutes ${key} and ${url:key} placeholders in a template string using
 * values from a vars object.
 *
 * Syntax:
 *   ${key}      → vars[key] inserted as-is
 *   ${url:key}  → vars[key] inserted URL-encoded via encodeURIComponent()
 *
 * Throws if any referenced key is undefined in vars, with the missing key name
 * included in the error message.
 *
 * Examples:
 *   interpolate('/api/timeline?from=${date}&to=${date}', { date: '2026-05-14' })
 *   → '/api/timeline?from=2026-05-14&to=2026-05-14'
 *
 *   interpolate('/api?q=${url:q}', { q: 'hello world' })
 *   → '/api?q=hello%20world'
 */

/**
 * @param {string} template
 * @param {Record<string, string>} vars
 * @returns {string}
 */
export function interpolate(template, vars) {
  if (typeof template !== 'string') {
    throw new TypeError(`interpolate: template must be a string, got ${typeof template}`);
  }

  // Match ${url:key} and ${key} patterns
  return template.replace(/\$\{(url:)?([^}]+)\}/g, (match, urlPrefix, key) => {
    if (!(key in vars)) {
      throw new Error(
        `interpolate: missing variable "${key}" in template "${template}"; ` +
        `available keys: [${Object.keys(vars).join(', ')}]`,
      );
    }
    const value = vars[key];
    return urlPrefix ? encodeURIComponent(value) : value;
  });
}
