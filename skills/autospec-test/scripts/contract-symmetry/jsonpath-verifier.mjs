/**
 * jsonpath-verifier.mjs — JSONPath-based assertions for Metric I contract symmetry.
 *
 * Provides two assertion functions:
 *   assertContains(body, pathExpr, vars) — asserts the JSONPath expression
 *     matches at least one value in body; throws with api_response_summary (≤500 bytes)
 *     on empty result.
 *   assertBoolean(body, pathExpr, vars) — asserts the JSONPath expression
 *     evaluates to exactly true (boolean); throws on empty result or non-true value.
 *
 * Uses jsonpath-plus for evaluation when available; falls back to a built-in
 * evaluator that handles the subset of JSONPath used by Metric I contracts:
 *   $.field               — property access
 *   $.array[*].field      — array wildcard projection
 *   $.array[?(@.k=="v")]  — filter expression (equality only)
 *   $.array[?(@.k==true)] — filter with boolean literal
 */

// Try to import jsonpath-plus from common locations
let JSONPath = null;
const candidatePaths = [
  '/opt/homebrew/lib/node_modules/jsonpath-plus/dist/index-node-cjs.cjs',
];
for (const p of candidatePaths) {
  try {
    const mod = await import(p);
    JSONPath = mod.JSONPath;
    break;
  } catch { /* try next */ }
}
if (!JSONPath) {
  try {
    const mod = await import('jsonpath-plus');
    JSONPath = mod.JSONPath;
  } catch { /* use built-in fallback */ }
}

// ── Built-in evaluator ────────────────────────────────────────────────────────

/**
 * Minimal JSONPath evaluator supporting the subset used by Metric I.
 * @param {unknown} root
 * @param {string} expr
 * @returns {unknown[]}
 */
function builtinEval(root, expr) {
  // Remove leading $
  const path = expr.startsWith('$') ? expr.slice(1) : expr;

  // Tokenize: split on . but handle [?(...)] and [*] as single tokens
  const tokens = [];
  let i = 0;
  while (i < path.length) {
    if (path[i] === '.') {
      i++;
      continue;
    }
    if (path[i] === '[') {
      const end = path.indexOf(']', i);
      if (end === -1) break;
      tokens.push(path.slice(i, end + 1));
      i = end + 1;
    } else {
      // Read identifier up to next . or [
      const start = i;
      while (i < path.length && path[i] !== '.' && path[i] !== '[') i++;
      tokens.push(path.slice(start, i));
    }
  }

  let current = [root];

  for (const token of tokens) {
    const next = [];
    for (const item of current) {
      if (token === '[*]') {
        // Wildcard: expand array
        if (Array.isArray(item)) {
          next.push(...item);
        } else if (item != null && typeof item === 'object') {
          next.push(...Object.values(item));
        }
      } else if (token.startsWith('[?(') && token.endsWith(')]')) {
        // Filter expression: [?(@.key=="value")] or [?(@.key==true/false)]
        const inner = token.slice(3, -2); // @.key=="value"
        const filterMatch = inner.match(/^@\.(\w+)==(.+)$/);
        if (!filterMatch) continue;
        const [, filterKey, rawVal] = filterMatch;
        let filterVal;
        if (rawVal === 'true') filterVal = true;
        else if (rawVal === 'false') filterVal = false;
        else if (rawVal.startsWith('"') && rawVal.endsWith('"')) {
          filterVal = rawVal.slice(1, -1);
        } else if (rawVal.startsWith("'") && rawVal.endsWith("'")) {
          filterVal = rawVal.slice(1, -1);
        } else {
          filterVal = rawVal; // treat as string
        }
        const arr = Array.isArray(item) ? item : [item];
        for (const el of arr) {
          if (el != null && typeof el === 'object' && el[filterKey] === filterVal) {
            next.push(el);
          }
        }
      } else {
        // Property access
        const key = token.replace(/^\[["']?|["']?\]$/g, '');
        if (Array.isArray(item)) {
          for (const el of item) {
            if (el != null && key in el) next.push(el[key]);
          }
        } else if (item != null && typeof item === 'object' && key in item) {
          next.push(item[key]);
        }
      }
    }
    current = next;
  }

  return current;
}

// ── Core evaluator dispatch ───────────────────────────────────────────────────

function evaluate(body, pathExpr) {
  if (JSONPath) {
    try {
      const result = JSONPath({ path: pathExpr, json: body, resultType: 'value' });
      return Array.isArray(result) ? result : [];
    } catch {
      // Fall through to built-in on parse error
    }
  }
  return builtinEval(body, pathExpr);
}

// ── Summary helper ────────────────────────────────────────────────────────────

function summarize(body, maxBytes = 500) {
  const str = JSON.stringify(body);
  return str.length <= maxBytes ? str : str.slice(0, maxBytes) + '…';
}

// ── Exported assertions ───────────────────────────────────────────────────────

/**
 * Assert a JSONPath expression returns at least one result in body.
 * @param {object|Array} body   — parsed JSON response body
 * @param {string} pathExpr     — JSONPath expression (already interpolated)
 * @param {Record<string,string>} [vars]  — for error context only
 * @throws {Error} if no match found
 */
export function assertContains(body, pathExpr, vars = {}) {
  const results = evaluate(body, pathExpr);
  if (results.length === 0) {
    throw new Error(
      `assertContains: JSONPath "${pathExpr}" matched 0 results.\n` +
      `api_response_summary: ${summarize(body)}`,
    );
  }
}

/**
 * Assert a JSONPath expression evaluates to exactly true for all matched values.
 * @param {object|Array} body
 * @param {string} pathExpr
 * @param {Record<string,string>} [vars]
 * @throws {Error} if result is empty or any value is not exactly true
 */
export function assertBoolean(body, pathExpr, vars = {}) {
  const results = evaluate(body, pathExpr);
  if (results.length === 0) {
    throw new Error(
      `assertBoolean: JSONPath "${pathExpr}" matched 0 results.\n` +
      `api_response_summary: ${summarize(body)}`,
    );
  }
  const allTrue = results.every(r => r === true);
  if (!allTrue) {
    throw new Error(
      `assertBoolean: JSONPath "${pathExpr}" did not evaluate to true; ` +
      `got: ${JSON.stringify(results)}.\n` +
      `api_response_summary: ${summarize(body)}`,
    );
  }
}
