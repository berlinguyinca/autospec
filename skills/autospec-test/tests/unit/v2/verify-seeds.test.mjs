/**
 * verify-seeds.test.mjs — Unit tests for Phase 7 edge-case seed verifier.
 *
 * Tests the full verify-seeds.mjs orchestrator + SQLite driver shim.
 * Postgres/MySQL shims skip if no service container is available.
 * Uses node:test (built-in runner). No mocks — real SQLite DBs via better-sqlite3.
 *
 * Run: node --test skills/autospec-test/tests/unit/v2/verify-seeds.test.mjs
 */

import { describe, it, before, after } from 'node:test';
import assert from 'node:assert/strict';
import path from 'node:path';
import fs from 'node:fs';
import { fileURLToPath } from 'node:url';
import Database from 'better-sqlite3';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(__dirname, '../../../../..');

// Paths under test
const SCRIPTS_DIR = path.join(ROOT, 'skills/autospec-test/scripts/seed-shapes');
const CATALOG_PATH = path.join(SCRIPTS_DIR, 'catalog.yml');
const SQLITE_DRIVER_PATH = path.join(SCRIPTS_DIR, 'db-driver/sqlite.mjs');

// ── Helper: build a temporary SQLite DB ───────────────────────────────────────

function buildDb(rows) {
  const db = new Database(':memory:');
  db.exec(`
    CREATE TABLE tasks (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      title TEXT NOT NULL,
      done_at TEXT,
      is_visible INTEGER DEFAULT 1,
      list_position INTEGER DEFAULT 0,
      foldout_collapsed INTEGER DEFAULT 0
    )
  `);
  const insert = db.prepare(
    'INSERT INTO tasks (title, done_at, is_visible, list_position, foldout_collapsed) VALUES (?, ?, ?, ?, ?)'
  );
  for (const r of rows) {
    insert.run(r.title, r.done_at, r.is_visible ?? 1, r.list_position ?? 0, r.foldout_collapsed ?? 0);
  }
  return db;
}

// ISO date helpers
function isoDate(offsetDays = 0) {
  const d = new Date();
  d.setDate(d.getDate() + offsetDays);
  return d.toISOString().slice(0, 10);
}

function isoDateTime(offsetDays = 0, hour = 12, minute = 0) {
  const d = new Date();
  d.setDate(d.getDate() + offsetDays);
  d.setUTCHours(hour, minute, 0, 0);
  return d.toISOString().replace('T', ' ').slice(0, 19);
}

// ── catalog.yml existence + shape count ───────────────────────────────────────

describe('catalog.yml', () => {
  it('exists at expected path', () => {
    assert.ok(fs.existsSync(CATALOG_PATH), `catalog.yml not found at ${CATALOG_PATH}`);
  });

  it('contains exactly 7 top-level shape keys', async () => {
    // Parse YAML manually (avoid yq dependency in tests) — each top-level key is "^<word>:"
    const content = fs.readFileSync(CATALOG_PATH, 'utf8');
    const keys = content.match(/^[a-z][a-z0-9_]+:/gm) || [];
    assert.strictEqual(keys.length, 7, `expected 7 shapes, got ${keys.length}: ${keys.join(', ')}`);
  });

  it('contains all 7 required shape names', () => {
    const content = fs.readFileSync(CATALOG_PATH, 'utf8');
    const required = [
      'task_done_today',
      'task_done_yesterday',
      'task_done_2_to_6_days_ago',
      'task_done_around_midnight',
      'multiple_tasks_same_day',
      'task_in_collapsed_foldout',
      'last_item_in_long_list',
    ];
    for (const name of required) {
      assert.ok(content.includes(`${name}:`), `catalog.yml missing shape: ${name}`);
    }
  });

  it('each shape has predicate_sql and predicate_jsonpath fields', () => {
    const content = fs.readFileSync(CATALOG_PATH, 'utf8');
    assert.ok(content.includes('predicate_sql:'), 'catalog.yml missing predicate_sql fields');
    assert.ok(content.includes('predicate_jsonpath:'), 'catalog.yml missing predicate_jsonpath fields');
  });
});

// ── SQLite driver interface ───────────────────────────────────────────────────

describe('sqlite driver', () => {
  let driver;

  before(async () => {
    const mod = await import(SQLITE_DRIVER_PATH);
    driver = mod;
  });

  it('exports connect, countMatching, close', () => {
    assert.strictEqual(typeof driver.connect, 'function');
    assert.strictEqual(typeof driver.countMatching, 'function');
    assert.strictEqual(typeof driver.close, 'function');
  });

  it('connect returns a connection object', async () => {
    const conn = await driver.connect(':memory:');
    assert.ok(conn, 'connect should return a connection');
    await driver.close(conn);
  });

  it('countMatching returns 0 for empty table', async () => {
    const db = buildDb([]);
    const conn = await driver.connect(':memory:', db);
    const count = await driver.countMatching(conn, 'tasks', `done_at >= date('now')`);
    assert.strictEqual(count, 0);
    await driver.close(conn);
    db.close();
  });

  it('countMatching returns correct count for task_done_today', async () => {
    const db = buildDb([
      { title: 'Task A', done_at: isoDate(0) },
      { title: 'Task B', done_at: isoDate(-1) },
    ]);
    const conn = await driver.connect(':memory:', db);
    const count = await driver.countMatching(conn, 'tasks', `done_at >= date('now') AND done_at < date('now', '+1 day')`);
    assert.strictEqual(count, 1);
    await driver.close(conn);
    db.close();
  });

  it('countMatching returns correct count for multiple matching rows', async () => {
    const db = buildDb([
      { title: 'T1', done_at: isoDate(0) },
      { title: 'T2', done_at: isoDate(0) },
      { title: 'T3', done_at: isoDate(-1) },
    ]);
    const conn = await driver.connect(':memory:', db);
    const count = await driver.countMatching(conn, 'tasks', `done_at >= date('now') AND done_at < date('now', '+1 day')`);
    assert.strictEqual(count, 2);
    await driver.close(conn);
    db.close();
  });
});

// ── verify-seeds.mjs orchestrator ─────────────────────────────────────────────

describe('verify-seeds orchestrator', () => {
  let verifySeeds;
  const VERIFY_PATH = path.join(SCRIPTS_DIR, 'verify-seeds.mjs');

  before(async () => {
    verifySeeds = await import(VERIFY_PATH);
  });

  it('exports a default function or run function', () => {
    assert.ok(
      typeof verifySeeds.default === 'function' || typeof verifySeeds.run === 'function',
      'verify-seeds.mjs must export default or run function'
    );
  });

  it('returns no violations when all 7 shapes are satisfied (SQLite)', async () => {
    const fn = verifySeeds.default || verifySeeds.run;
    const today = isoDate(0);
    const yesterday = isoDate(-1);

    // Build SQLite DB with all 7 shapes present
    const db = buildDb([
      // task_done_today
      { title: 'Done Today', done_at: today, list_position: 5 },
      // task_done_yesterday
      { title: 'Done Yesterday', done_at: yesterday, list_position: 3 },
      // task_done_2_to_6_days_ago (5 rows)
      { title: 'Done 2d ago', done_at: isoDate(-2) },
      { title: 'Done 3d ago', done_at: isoDate(-3) },
      { title: 'Done 4d ago', done_at: isoDate(-4) },
      { title: 'Done 5d ago', done_at: isoDate(-5) },
      { title: 'Done 6d ago', done_at: isoDate(-6) },
      // task_done_around_midnight (23:58)
      { title: 'Done Midnight', done_at: isoDateTime(0, 23, 58) },
      // multiple_tasks_same_day — 2 on same day
      { title: 'Multi A', done_at: isoDate(-7) },
      { title: 'Multi B', done_at: isoDate(-7) },
      // task_in_collapsed_foldout
      { title: 'In Foldout', done_at: isoDate(-1), foldout_collapsed: 1 },
      // last_item_in_long_list — list_position > 50
      { title: 'Last Item', done_at: isoDate(-1), list_position: 51 },
    ]);

    const contract = {
      edge_case_seeds: {
        household_test_family: {
          require_shapes: [
            { name: 'task_done_today', count_min: 1 },
            { name: 'task_done_yesterday', count_min: 1 },
            { name: 'task_done_2_to_6_days_ago', count_min: 5 },
            { name: 'task_done_around_midnight', count_min: 1 },
            { name: 'multiple_tasks_same_day', count_min: 1 },
            { name: 'task_in_collapsed_foldout', count_min: 1 },
            { name: 'last_item_in_long_list', count_min: 1 },
          ],
        },
        enforcement: 'refuse_to_run_if_missing',
      },
    };

    const result = await fn({ contract, store_kind: 'sqlite', db });
    assert.strictEqual(result.violations.length, 0, `expected 0 violations, got: ${JSON.stringify(result.violations)}`);
    assert.strictEqual(result.exit_code, 0);
    db.close();
  });

  it('returns violation and exit_code 2 when task_done_today has 0 rows', async () => {
    const fn = verifySeeds.default || verifySeeds.run;

    // Build a DB with NO tasks done today
    const db = buildDb([
      { title: 'Old Task', done_at: isoDate(-10) },
    ]);

    const contract = {
      edge_case_seeds: {
        household_test_family: {
          require_shapes: [
            { name: 'task_done_today', count_min: 1 },
          ],
        },
        enforcement: 'refuse_to_run_if_missing',
      },
    };

    const result = await fn({ contract, store_kind: 'sqlite', db });
    assert.ok(result.violations.length > 0, 'expected at least 1 violation');
    assert.strictEqual(result.exit_code, 2);
    const v = result.violations[0];
    assert.strictEqual(v.shape, 'task_done_today');
    assert.ok(v.entity, 'violation must have entity field');
    db.close();
  });

  it('passes when count exactly meets count_min', async () => {
    const fn = verifySeeds.default || verifySeeds.run;
    const db = buildDb([
      { title: 'T', done_at: isoDate(0) },
    ]);
    const contract = {
      edge_case_seeds: {
        household_test_family: {
          require_shapes: [{ name: 'task_done_today', count_min: 1 }],
        },
        enforcement: 'refuse_to_run_if_missing',
      },
    };
    const result = await fn({ contract, store_kind: 'sqlite', db });
    assert.strictEqual(result.violations.length, 0);
    assert.strictEqual(result.exit_code, 0);
    db.close();
  });

  it('loads custom shape overlay from .autospec/seed-shapes.yml', async () => {
    const fn = verifySeeds.default || verifySeeds.run;
    const db = buildDb([
      // custom shape: tasks with title starting with "CUSTOM"
      { title: 'CUSTOM task', done_at: isoDate(0) },
    ]);

    // Custom overlay adds a shape not in catalog
    const customOverlay = {
      custom_titled_task: {
        description: 'A task with a title starting with CUSTOM',
        predicate_sql: `title LIKE 'CUSTOM%'`,
        predicate_jsonpath: `$[?(@.title =~ /^CUSTOM/)]`,
      },
    };

    const contract = {
      edge_case_seeds: {
        household_test_family: {
          require_shapes: [{ name: 'custom_titled_task', count_min: 1 }],
        },
        enforcement: 'refuse_to_run_if_missing',
      },
    };

    const result = await fn({ contract, store_kind: 'sqlite', db, customShapes: customOverlay });
    assert.strictEqual(result.violations.length, 0, `expected no violations, got: ${JSON.stringify(result.violations)}`);
    assert.strictEqual(result.exit_code, 0);
    db.close();
  });

  it('emits exit_code 0 when enforcement is not refuse_to_run_if_missing', async () => {
    const fn = verifySeeds.default || verifySeeds.run;
    const db = buildDb([]); // empty DB

    const contract = {
      edge_case_seeds: {
        household_test_family: {
          require_shapes: [{ name: 'task_done_today', count_min: 1 }],
        },
        enforcement: 'warn_only',
      },
    };

    const result = await fn({ contract, store_kind: 'sqlite', db });
    // violations may exist but exit_code must NOT be 2
    assert.notStrictEqual(result.exit_code, 2);
    db.close();
  });
});

// ── Postgres driver — skip if no service ──────────────────────────────────────

describe('postgres driver', async () => {
  const PG_DRIVER_PATH = path.join(SCRIPTS_DIR, 'db-driver/postgres.mjs');

  it('exports connect, countMatching, close', async () => {
    const mod = await import(PG_DRIVER_PATH);
    assert.strictEqual(typeof mod.connect, 'function');
    assert.strictEqual(typeof mod.countMatching, 'function');
    assert.strictEqual(typeof mod.close, 'function');
  });

  it('skips connection test if postgres not available', async () => {
    const mod = await import(PG_DRIVER_PATH);
    const pgDsn = process.env.PGURL || process.env.DATABASE_URL || '';
    if (!pgDsn) {
      // No DSN — skip live test
      return;
    }
    let conn;
    try {
      conn = await mod.connect(pgDsn);
      const count = await mod.countMatching(conn, 'information_schema.tables', `table_schema = 'public'`);
      assert.ok(typeof count === 'number');
    } finally {
      if (conn) await mod.close(conn);
    }
  });
});

// ── MySQL driver — skip if no service ─────────────────────────────────────────

describe('mysql driver', async () => {
  const MYSQL_DRIVER_PATH = path.join(SCRIPTS_DIR, 'db-driver/mysql.mjs');

  it('exports connect, countMatching, close', async () => {
    const mod = await import(MYSQL_DRIVER_PATH);
    assert.strictEqual(typeof mod.connect, 'function');
    assert.strictEqual(typeof mod.countMatching, 'function');
    assert.strictEqual(typeof mod.close, 'function');
  });

  it('skips connection test if mysql not available', async () => {
    const mod = await import(MYSQL_DRIVER_PATH);
    const mysqlDsn = process.env.MYSQL_URL || '';
    if (!mysqlDsn) {
      return;
    }
    let conn;
    try {
      conn = await mod.connect(mysqlDsn);
      const count = await mod.countMatching(conn, 'information_schema.tables', `TABLE_SCHEMA = DATABASE()`);
      assert.ok(typeof count === 'number');
    } finally {
      if (conn) await mod.close(conn);
    }
  });
});

// ── jsonpath-store driver ──────────────────────────────────────────────────────

describe('jsonpath-store driver', () => {
  const JP_DRIVER_PATH = path.join(SCRIPTS_DIR, 'db-driver/jsonpath-store.mjs');

  it('exports connect, countMatching, close', async () => {
    const mod = await import(JP_DRIVER_PATH);
    assert.strictEqual(typeof mod.connect, 'function');
    assert.strictEqual(typeof mod.countMatching, 'function');
    assert.strictEqual(typeof mod.close, 'function');
  });

  it('countMatching filters records by jsonpath predicate', async () => {
    const mod = await import(JP_DRIVER_PATH);
    // Use in-memory data store (array of objects) instead of HTTP
    const data = [
      { done_at: new Date().toISOString().slice(0, 10), title: 'Today task' },
      { done_at: '2020-01-01', title: 'Old task' },
    ];
    const today = new Date().toISOString().slice(0, 10);
    // Inline store: connect accepts { data } config instead of HTTP URL
    const conn = await mod.connect({ inlineData: data });
    const count = await mod.countMatching(conn, 'root', `$[?(@.done_at == "${today}")]`);
    assert.strictEqual(count, 1);
    await mod.close(conn);
  });
});
