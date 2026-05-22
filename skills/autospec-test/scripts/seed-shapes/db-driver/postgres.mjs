/**
 * postgres.mjs — PostgreSQL DB driver shim for the edge-case seed verifier.
 *
 * Implements the 3-function interface: connect, countMatching, close.
 *
 * Uses the `pg` package (node-postgres).
 * In CI without a postgres service container, tests should call `skip()`.
 *
 * Interface:
 *   connect(dsn) -> conn
 *     dsn: PostgreSQL connection string, e.g. 'postgresql://user:pass@host:5432/db'
 *
 *   countMatching(conn, table, predicate) -> number
 *     table:     Fully-qualified table name (e.g. 'public.tasks')
 *     predicate: SQL WHERE clause fragment (no leading WHERE keyword)
 *
 *   close(conn) -> void
 */

import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);

let pg;
try {
  pg = require('pg');
} catch {
  // pg not installed — all methods will throw with a clear message
  pg = null;
}

function requirePg() {
  if (!pg) {
    throw new Error(
      'pg package not found. Install it with: npm install pg\n' +
      'In CI without a postgres service, skip this test with: if (!process.env.PGURL) return;'
    );
  }
}

/**
 * @typedef {object} PgConn
 * @property {import('pg').Client} client
 */

/**
 * Connect to a PostgreSQL database.
 * @param {string} dsn - PostgreSQL connection string
 * @returns {Promise<PgConn>}
 */
export async function connect(dsn) {
  requirePg();
  const { Client } = pg;
  const client = new Client({ connectionString: dsn });
  await client.connect();
  return { client };
}

/**
 * Count rows in `table` matching the given SQL WHERE predicate.
 * @param {PgConn} conn
 * @param {string} table - Table name (schema-qualified if needed)
 * @param {string} predicate - SQL WHERE clause fragment
 * @returns {Promise<number>}
 */
export async function countMatching(conn, table, predicate) {
  const sql = `SELECT COUNT(*)::int AS n FROM ${table} WHERE ${predicate}`;
  const result = await conn.client.query(sql);
  return result.rows[0] ? Number(result.rows[0].n) : 0;
}

/**
 * Close the database connection.
 * @param {PgConn} conn
 * @returns {Promise<void>}
 */
export async function close(conn) {
  if (conn && conn.client) {
    await conn.client.end();
  }
}
