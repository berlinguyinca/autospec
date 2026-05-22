/**
 * sqlite.mjs — SQLite DB driver shim for the edge-case seed verifier.
 *
 * Implements the 3-function interface: connect, countMatching, close.
 *
 * Uses better-sqlite3 (synchronous SQLite bindings).
 * Works in CI without service containers — just needs the `better-sqlite3` package.
 *
 * Interface:
 *   connect(dsn, existingDb?) -> conn
 *     dsn:        Path to SQLite file, or ':memory:'.
 *     existingDb: Optional pre-built Database instance (for tests that build their own DB).
 *
 *   countMatching(conn, table, predicate) -> number
 *     table:     Table name to query.
 *     predicate: SQL WHERE clause fragment (no leading WHERE keyword).
 *
 *   close(conn) -> void
 */

import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const __dirname = path.dirname(fileURLToPath(import.meta.url));

// Resolve better-sqlite3 from the worktree's node_modules, then fallback to global
let Database;
const require = createRequire(import.meta.url);
try {
  Database = require('better-sqlite3');
} catch {
  // Try resolving from the skill root (where npm install ran)
  const skillRoot = path.resolve(__dirname, '../../../../..');
  try {
    const mod = require(path.join(skillRoot, 'node_modules/better-sqlite3'));
    Database = mod;
  } catch {
    throw new Error(
      'better-sqlite3 not found. Install it with: npm install better-sqlite3\n' +
      `Searched from: ${__dirname} and ${skillRoot}`
    );
  }
}

/**
 * @typedef {object} SqliteConn
 * @property {import('better-sqlite3').Database} db
 * @property {boolean} owned - true if we opened the DB and should close it
 */

/**
 * Connect to a SQLite database.
 * @param {string} dsn - File path or ':memory:'
 * @param {import('better-sqlite3').Database} [existingDb] - Pre-built DB (tests only)
 * @returns {Promise<SqliteConn>}
 */
export async function connect(dsn, existingDb = null) {
  if (existingDb) {
    return { db: existingDb, owned: false };
  }
  const db = new Database(dsn, { readonly: false });
  return { db, owned: true };
}

/**
 * Count rows in `table` matching the given SQL WHERE predicate.
 * @param {SqliteConn} conn
 * @param {string} table - Table name
 * @param {string} predicate - SQL WHERE clause fragment (no WHERE keyword)
 * @returns {Promise<number>}
 */
export async function countMatching(conn, table, predicate) {
  const sql = `SELECT COUNT(*) AS n FROM ${table} WHERE ${predicate}`;
  const row = conn.db.prepare(sql).get();
  return row ? Number(row.n) : 0;
}

/**
 * Close the database connection.
 * @param {SqliteConn} conn
 * @returns {Promise<void>}
 */
export async function close(conn) {
  if (conn && conn.owned && conn.db) {
    conn.db.close();
  }
}
