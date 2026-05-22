/**
 * mysql.mjs — MySQL DB driver shim for the edge-case seed verifier.
 *
 * Implements the 3-function interface: connect, countMatching, close.
 *
 * Uses the `mysql2/promise` package.
 * In CI without a MySQL service container, tests should call `skip()`.
 *
 * Interface:
 *   connect(dsn) -> conn
 *     dsn: MySQL connection string, e.g. 'mysql://user:pass@host:3306/db'
 *
 *   countMatching(conn, table, predicate) -> number
 *     table:     Table name (schema-qualified if needed)
 *     predicate: SQL WHERE clause fragment (no leading WHERE keyword)
 *
 *   close(conn) -> void
 */

import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);

let mysql2;
try {
  mysql2 = require('mysql2/promise');
} catch {
  mysql2 = null;
}

function requireMysql2() {
  if (!mysql2) {
    throw new Error(
      'mysql2 package not found. Install it with: npm install mysql2\n' +
      'In CI without a MySQL service, skip this test with: if (!process.env.MYSQL_URL) return;'
    );
  }
}

/**
 * @typedef {object} MysqlConn
 * @property {import('mysql2/promise').Connection} connection
 */

/**
 * Connect to a MySQL database.
 * @param {string} dsn - MySQL connection string (mysql://...)
 * @returns {Promise<MysqlConn>}
 */
export async function connect(dsn) {
  requireMysql2();
  const connection = await mysql2.createConnection(dsn);
  return { connection };
}

/**
 * Count rows in `table` matching the given SQL WHERE predicate.
 * @param {MysqlConn} conn
 * @param {string} table - Table name
 * @param {string} predicate - SQL WHERE clause fragment
 * @returns {Promise<number>}
 */
export async function countMatching(conn, table, predicate) {
  const sql = `SELECT COUNT(*) AS n FROM \`${table}\` WHERE ${predicate}`;
  const [rows] = await conn.connection.execute(sql);
  return rows[0] ? Number(rows[0].n) : 0;
}

/**
 * Close the database connection.
 * @param {MysqlConn} conn
 * @returns {Promise<void>}
 */
export async function close(conn) {
  if (conn && conn.connection) {
    await conn.connection.end();
  }
}
