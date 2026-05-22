/**
 * jsonpath-store.mjs — JSONPath document store driver shim for the edge-case seed verifier.
 *
 * Implements the 3-function interface: connect, countMatching, close.
 *
 * Supports two modes:
 *   1. HTTP store: connect({ url }) — fetches JSON from an HTTP endpoint
 *   2. Inline store: connect({ inlineData }) — uses an in-memory array (for tests)
 *
 * Uses jsonpath-plus for predicate evaluation (mirrors jsonpath-verifier.mjs pattern).
 *
 * Interface:
 *   connect(config) -> conn
 *     config.url:        HTTP URL returning a JSON array (NoSQL target)
 *     config.inlineData: Array of objects (for tests — avoids HTTP dependency)
 *
 *   countMatching(conn, collection, predicate) -> number
 *     collection: Ignored for inline mode; used as URL path suffix for HTTP mode.
 *     predicate:  JSONPath filter expression (e.g. "$[?(@.done_at == '2026-05-21')]")
 *
 *   close(conn) -> void
 */

import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const require = createRequire(import.meta.url);

// Try to load jsonpath-plus (mirrors jsonpath-verifier.mjs pattern)
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
  } catch { /* fall through to built-in */ }
}

/**
 * Minimal built-in JSONPath evaluator for filter expressions.
 * Supports: $[?(@.field == "value")] and $[?(@.field > number)]
 * @param {unknown[]} data
 * @param {string} expr
 * @returns {unknown[]}
 */
function builtinEval(data, expr) {
  if (!Array.isArray(data)) return [];

  // Strip $[?( ... )]
  const filterMatch = expr.match(/^\$\[\?\((.+)\)\]$/);
  if (!filterMatch) {
    // No filter — return whole array
    return data;
  }

  const condition = filterMatch[1]; // e.g. @.done_at == "2026-05-21"

  // Parse simple @.field OP value
  const opMatch = condition.match(/^@\.(\w+)\s*(==|!=|>=|<=|>|<)\s*(.+)$/);
  if (!opMatch) return data;

  const [, field, op, rawVal] = opMatch;
  let cmpVal;
  if (rawVal === 'true') cmpVal = true;
  else if (rawVal === 'false') cmpVal = false;
  else if (rawVal.startsWith('"') && rawVal.endsWith('"')) cmpVal = rawVal.slice(1, -1);
  else if (rawVal.startsWith("'") && rawVal.endsWith("'")) cmpVal = rawVal.slice(1, -1);
  else if (!isNaN(Number(rawVal))) cmpVal = Number(rawVal);
  else cmpVal = rawVal;

  return data.filter(item => {
    if (item == null || typeof item !== 'object') return false;
    const v = item[field];
    switch (op) {
      case '==': return v === cmpVal;
      case '!=': return v !== cmpVal;
      case '>':  return v > cmpVal;
      case '<':  return v < cmpVal;
      case '>=': return v >= cmpVal;
      case '<=': return v <= cmpVal;
      default:   return false;
    }
  });
}

/**
 * Evaluate a JSONPath expression against data.
 * @param {unknown[]} data
 * @param {string} expr
 * @returns {unknown[]}
 */
function evaluate(data, expr) {
  if (JSONPath) {
    try {
      const result = JSONPath({ path: expr, json: data, resultType: 'value' });
      return Array.isArray(result) ? result : [];
    } catch {
      // Fall through to built-in
    }
  }
  return builtinEval(data, expr);
}

/**
 * @typedef {object} JsonpathConn
 * @property {'inline'|'http'} mode
 * @property {unknown[]} [data]  - inline mode
 * @property {string} [url]      - http mode
 */

/**
 * Connect to a JSONPath document store.
 * @param {string|{url?: string, inlineData?: unknown[]}} config
 * @returns {Promise<JsonpathConn>}
 */
export async function connect(config) {
  if (typeof config === 'string') {
    // Treat as HTTP URL
    return { mode: 'http', url: config };
  }
  if (config.inlineData !== undefined) {
    return { mode: 'inline', data: config.inlineData };
  }
  if (config.url) {
    return { mode: 'http', url: config.url };
  }
  throw new Error('jsonpath-store connect: provide { url } or { inlineData }');
}

/**
 * Count items in the store matching the given JSONPath predicate.
 * @param {JsonpathConn} conn
 * @param {string} collection - Ignored in inline mode; appended to URL in HTTP mode
 * @param {string} predicate  - JSONPath filter expression
 * @returns {Promise<number>}
 */
export async function countMatching(conn, collection, predicate) {
  let data;

  if (conn.mode === 'inline') {
    data = conn.data;
  } else {
    // HTTP mode: fetch JSON from the store
    const url = collection && collection !== 'root'
      ? `${conn.url}/${collection}`
      : conn.url;
    const res = await fetch(url);
    if (!res.ok) {
      throw new Error(`jsonpath-store: HTTP ${res.status} fetching ${url}`);
    }
    data = await res.json();
    if (!Array.isArray(data)) {
      // Wrap single object
      data = [data];
    }
  }

  const matched = evaluate(data, predicate);
  return matched.length;
}

/**
 * Close the store connection. No-op for in-memory and HTTP stores.
 * @param {JsonpathConn} _conn
 * @returns {Promise<void>}
 */
export async function close(_conn) {
  // No persistent connection to close
}
