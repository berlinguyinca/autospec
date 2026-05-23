#!/usr/bin/env node
// skills/autospec-e2e-clone/scripts/edge-case-seed.mjs
//
// Edge-case seed consumer (C6).
//
// Reads .autospec/test.yml's edge_case_seeds.require_shapes (autospec-test v2 §5b).
// For each declared shape, checks the snapshot CSV has >= count_min rows matching
// the shape's predicate_sql. If short, INSERTs synthetic rows from the seed catalog
// template into the snapshot CSV. Synthetic rows are flagged _autospec_synthetic=true.
//
// Catalog merge order:
//   1. autospec-test canonical catalog  ($AUTOSPEC_SCRIPTS_DIR/seed-shapes/catalog.yml)
//   2. clone overlay                    (skills/autospec-e2e-clone/scripts/seed-shapes/overlay.yml)
//   Overlay entries win (shallow merge per shape key).
//
// Usage:
//   node edge-case-seed.mjs <snapshot-dir> [--contract <path>] [--repo-root <path>]
//                           [--catalog <path>] [--overlay <path>]
//
// Exit codes:
//   0  success (or nothing-to-do)
//   1  fatal (missing deps, bad files)
//   2  refuse-to-run (contract invalid / shape not in catalog)

import {
  existsSync,
  mkdirSync,
  readFileSync,
  writeFileSync,
  readdirSync,
  appendFileSync,
} from 'node:fs';
import { join, resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';
import { parseArgs } from 'node:util';
import { createInterface } from 'node:readline';
import { createReadStream } from 'node:fs';

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

// ---------------------------------------------------------------------------
// Arg parsing
// ---------------------------------------------------------------------------

const { values: args, positionals } = parseArgs({
  args: process.argv.slice(2),
  options: {
    contract: { type: 'string', short: 'c' },
    'repo-root': { type: 'string', short: 'r' },
    catalog: { type: 'string' },
    overlay: { type: 'string' },
    help: { type: 'boolean', short: 'h' },
  },
  allowPositionals: true,
  strict: true,
});

if (args.help) {
  console.log(
    'Usage: node edge-case-seed.mjs <snapshot-dir> [--contract <test.yml>] [--repo-root <root>] [--catalog <catalog.yml>] [--overlay <overlay.yml>]',
  );
  process.exit(0);
}

const snapshotDir = positionals[0];
if (!snapshotDir) {
  console.error('edge-case-seed: fatal: <snapshot-dir> is required');
  process.exit(1);
}

const resolvedSnapshot = resolve(snapshotDir);
if (!existsSync(resolvedSnapshot)) {
  console.error(`edge-case-seed: fatal: snapshot-dir not found: ${resolvedSnapshot}`);
  process.exit(1);
}

// ---------------------------------------------------------------------------
// Locate repo root
// ---------------------------------------------------------------------------

function findRepoRoot(startDir) {
  let dir = startDir;
  for (let i = 0; i < 20; i++) {
    if (existsSync(join(dir, '.autospec'))) return dir;
    const parent = dirname(dir);
    if (parent === dir) break;
    dir = parent;
  }
  return null;
}

const repoRoot = args['repo-root']
  ? resolve(args['repo-root'])
  : findRepoRoot(resolvedSnapshot);

if (!repoRoot) {
  console.error(
    'edge-case-seed: fatal: could not locate repo root (no .autospec/ directory found)',
  );
  process.exit(1);
}

// ---------------------------------------------------------------------------
// Tools check
// ---------------------------------------------------------------------------

function requireTool(tool) {
  try {
    execFileSync('which', [tool], { stdio: 'ignore' });
  } catch {
    console.error(
      `edge-case-seed: fatal: ${tool} not found. Install with: brew install ${tool}`,
    );
    process.exit(1);
  }
}

requireTool('yq');

// ---------------------------------------------------------------------------
// Load .autospec/test.yml contract
// ---------------------------------------------------------------------------

const contractPath = args.contract
  ? resolve(args.contract)
  : join(repoRoot, '.autospec', 'test.yml');

if (!existsSync(contractPath)) {
  console.error(`edge-case-seed: refuse-to-run: test.yml not found: ${contractPath}`);
  process.exit(2);
}

let testContract;
try {
  testContract = JSON.parse(
    execFileSync('yq', ['-o=json', '.', contractPath], { encoding: 'utf8' }),
  );
} catch (err) {
  console.error(`edge-case-seed: fatal: failed to parse test.yml: ${err.message}`);
  process.exit(1);
}

// ---------------------------------------------------------------------------
// Extract require_shapes from all entity keys
// ---------------------------------------------------------------------------

/**
 * Collects all require_shapes entries from edge_case_seeds across all entity keys.
 * Schema: edge_case_seeds.<entity>.require_shapes[{name, count_min}]
 * @returns {{ name: string, count_min: number, entity: string }[]}
 */
function extractRequireShapes(contract) {
  const edgeCaseSeeds = contract?.edge_case_seeds ?? {};
  const shapes = [];
  for (const [entityKey, entityVal] of Object.entries(edgeCaseSeeds)) {
    if (entityKey === 'enforcement') continue;
    if (typeof entityVal !== 'object' || !Array.isArray(entityVal.require_shapes)) continue;
    for (const shape of entityVal.require_shapes) {
      if (typeof shape === 'object' && shape.name && typeof shape.count_min === 'number') {
        shapes.push({ name: shape.name, count_min: shape.count_min, entity: entityKey });
      }
    }
  }
  return shapes;
}

const requireShapes = extractRequireShapes(testContract);

if (requireShapes.length === 0) {
  console.log('edge-case-seed: no require_shapes declared — nothing to do');
  process.exit(0);
}

// ---------------------------------------------------------------------------
// Load seed catalog (with overlay merge)
// ---------------------------------------------------------------------------

// Default catalog path: $AUTOSPEC_SCRIPTS_DIR/seed-shapes/catalog.yml or sibling in autospec-test
function defaultCatalogPath() {
  const fromEnv = process.env.AUTOSPEC_SCRIPTS_DIR
    ? join(process.env.AUTOSPEC_SCRIPTS_DIR, 'seed-shapes', 'catalog.yml')
    : null;
  if (fromEnv && existsSync(fromEnv)) return fromEnv;
  // Fallback: look relative to skills/autospec-test from this script's location
  const fromSkill = resolve(__dirname, '..', '..', '..', 'autospec-test', 'scripts', 'seed-shapes', 'catalog.yml');
  if (existsSync(fromSkill)) return fromSkill;
  return null;
}

const catalogPath = args.catalog ? resolve(args.catalog) : defaultCatalogPath();

if (!catalogPath || !existsSync(catalogPath)) {
  console.error(
    `edge-case-seed: fatal: seed catalog not found. ` +
    `Set AUTOSPEC_SCRIPTS_DIR or pass --catalog. Tried: ${catalogPath ?? '(not found)'}`,
  );
  process.exit(1);
}

// Default overlay path: scripts/seed-shapes/overlay.yml sibling to this script
const defaultOverlayPath = join(__dirname, 'seed-shapes', 'overlay.yml');
const overlayPath = args.overlay ? resolve(args.overlay) : defaultOverlayPath;

function loadYaml(filePath) {
  try {
    return JSON.parse(execFileSync('yq', ['-o=json', '.', filePath], { encoding: 'utf8' }));
  } catch (err) {
    console.error(`edge-case-seed: fatal: failed to parse ${filePath}: ${err.message}`);
    process.exit(1);
  }
}

const catalogRaw = loadYaml(catalogPath);

// Merge overlay (shallow per shape key — overlay wins)
let mergedCatalog = { ...catalogRaw };
if (existsSync(overlayPath)) {
  const overlayRaw = loadYaml(overlayPath);
  if (overlayRaw && typeof overlayRaw === 'object') {
    for (const [shapeKey, shapeVal] of Object.entries(overlayRaw)) {
      mergedCatalog[shapeKey] = { ...(mergedCatalog[shapeKey] ?? {}), ...shapeVal };
    }
    console.log(`edge-case-seed: loaded overlay from ${overlayPath}`);
  }
} else {
  console.log(`edge-case-seed: no overlay found at ${overlayPath} — using base catalog only`);
}

// ---------------------------------------------------------------------------
// Find snapshot CSV files
// ---------------------------------------------------------------------------

/**
 * Discovers CSV files in the snapshot dir.
 * Naming convention: <table>.csv or <source-id>/<table>.csv
 * Returns a flat map of tableName -> absolutePath (last wins for duplicates).
 */
function discoverCsvs(snapshotRoot) {
  const map = {};
  function walk(dir) {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const full = join(dir, entry.name);
      if (entry.isDirectory()) {
        walk(full);
      } else if (entry.isFile() && entry.name.endsWith('.csv')) {
        const tableName = entry.name.replace(/\.csv$/, '');
        map[tableName] = full;
      }
    }
  }
  walk(snapshotRoot);
  return map;
}

const csvMap = discoverCsvs(resolvedSnapshot);

// ---------------------------------------------------------------------------
// CSV helpers (header-aware, no external deps)
// ---------------------------------------------------------------------------

/**
 * Reads a CSV file and returns { headers: string[], rows: string[][] }.
 */
async function readCsv(filePath) {
  const headers = [];
  const rows = [];
  let firstLine = true;
  const rl = createInterface({ input: createReadStream(filePath), crlfDelay: Infinity });
  for await (const line of rl) {
    if (!line.trim()) continue;
    const cols = parseCsvLine(line);
    if (firstLine) {
      headers.push(...cols);
      firstLine = false;
    } else {
      rows.push(cols);
    }
  }
  return { headers, rows };
}

/**
 * Naive CSV line parser (handles double-quoted fields with embedded commas).
 */
function parseCsvLine(line) {
  const result = [];
  let current = '';
  let inQuotes = false;
  for (let i = 0; i < line.length; i++) {
    const ch = line[i];
    if (ch === '"') {
      if (inQuotes && line[i + 1] === '"') { current += '"'; i++; }
      else { inQuotes = !inQuotes; }
    } else if (ch === ',' && !inQuotes) {
      result.push(current);
      current = '';
    } else {
      current += ch;
    }
  }
  result.push(current);
  return result;
}

function toCsvLine(fields) {
  return fields
    .map((f) => {
      const s = String(f ?? '');
      return s.includes(',') || s.includes('"') || s.includes('\n')
        ? `"${s.replace(/"/g, '""')}"`
        : s;
    })
    .join(',');
}

// ---------------------------------------------------------------------------
// Predicate evaluation (SQLite-compatible SQL WHERE fragment)
// ---------------------------------------------------------------------------

/**
 * Evaluates a predicate_sql fragment against rows in a CSV using SQLite.
 * Writes the CSV to a temp SQLite DB, runs SELECT COUNT(*) WHERE <predicate>.
 * Returns the count of matching rows.
 */
function countMatchingRowsSqlite(csvPath, predicate, tableName = 'data') {
  try {
    execFileSync('which', ['sqlite3'], { stdio: 'ignore' });
  } catch {
    console.error('edge-case-seed: fatal: sqlite3 not found. Install with: brew install sqlite3');
    process.exit(1);
  }

  // Use sqlite3's .import to load the CSV and run the predicate
  const sql = [
    `.mode csv`,
    `.import "${csvPath.replace(/"/g, '\\"')}" ${tableName}`,
    `SELECT COUNT(*) FROM ${tableName} WHERE ${predicate};`,
  ].join('\n');

  try {
    const out = execFileSync('sqlite3', [':memory:'], {
      input: sql,
      encoding: 'utf8',
      stdio: ['pipe', 'pipe', 'pipe'],
    });
    return parseInt(out.trim(), 10) || 0;
  } catch (err) {
    // If predicate uses columns not present in CSV, treat as 0 matches
    console.warn(
      `edge-case-seed: warn: predicate evaluation failed for "${predicate}": ${err.message.split('\n')[0]}`,
    );
    return 0;
  }
}

// ---------------------------------------------------------------------------
// Synthetic row generation from catalog template
// ---------------------------------------------------------------------------

let faker;
async function getFaker() {
  if (!faker) {
    try {
      const { faker: f } = await import('@faker-js/faker');
      faker = f;
    } catch {
      console.warn('edge-case-seed: warn: @faker-js/faker not available — using fallback values');
      faker = null;
    }
  }
  return faker;
}

/**
 * Generates `count` synthetic rows for the given shape.
 * The catalog entry's `template` map provides column -> value expressions.
 * Falls back to sensible defaults per shape name if no template is provided.
 */
async function generateSyntheticRows(shapeName, catalogEntry, existingHeaders, count) {
  const f = await getFaker();
  const template = catalogEntry.template ?? {};
  const rows = [];

  for (let i = 0; i < count; i++) {
    const row = {};

    // Apply template values (support simple string | "faker:<method>" syntax)
    for (const [col, expr] of Object.entries(template)) {
      if (typeof expr === 'string' && expr.startsWith('faker:')) {
        const method = expr.slice(6); // e.g. "date.recent"
        row[col] = f ? resolveFaker(f, method) : `synthetic_${col}_${i}`;
      } else {
        row[col] = expr;
      }
    }

    // Built-in shape defaults (when no template or partial template)
    applyShapeDefaults(shapeName, row, i, f);

    // Always set the synthetic flag
    row['_autospec_synthetic'] = 'true';

    rows.push(row);
  }

  return rows;
}

function resolveFaker(f, dotPath) {
  const parts = dotPath.split('.');
  let obj = f;
  for (const part of parts) {
    if (obj == null) return `synthetic_${dotPath}`;
    obj = obj[part];
  }
  return typeof obj === 'function' ? obj() : String(obj ?? `synthetic_${dotPath}`);
}

/**
 * Applies sensible defaults per known shape name if the template didn't set the column.
 */
function applyShapeDefaults(shapeName, row, idx, f) {
  const now = new Date();
  const todayIso = now.toISOString().slice(0, 10);
  const yesterdayIso = new Date(now - 86400000).toISOString().slice(0, 10);

  switch (shapeName) {
    case 'task_done_today':
      row.done_at ??= `${todayIso}T12:00:00Z`;
      break;
    case 'task_done_yesterday':
      row.done_at ??= `${yesterdayIso}T12:00:00Z`;
      break;
    case 'task_done_2_to_6_days_ago': {
      const daysAgo = 2 + (idx % 5);
      const d = new Date(now - daysAgo * 86400000);
      row.done_at ??= d.toISOString().slice(0, 10) + 'T12:00:00Z';
      break;
    }
    case 'task_done_around_midnight':
      row.done_at ??= `${todayIso}T23:57:00Z`;
      break;
    case 'multiple_tasks_same_day':
      row.done_at ??= `${todayIso}T${String(10 + idx).padStart(2, '0')}:00:00Z`;
      break;
    case 'task_in_collapsed_foldout':
      row.foldout_collapsed ??= '1';
      break;
    case 'last_item_in_long_list':
      row.list_position ??= String(51 + idx);
      break;
    default:
      break;
  }
}

// ---------------------------------------------------------------------------
// Ensure _autospec_synthetic column exists in CSV header
// ---------------------------------------------------------------------------

/**
 * Rewrites a CSV file to add the _autospec_synthetic column if absent.
 * Returns the (possibly updated) headers array.
 */
async function ensureSyntheticColumn(csvPath) {
  const { headers, rows } = await readCsv(csvPath);
  if (headers.includes('_autospec_synthetic')) return headers;

  const newHeaders = [...headers, '_autospec_synthetic'];
  const newRows = rows.map((r) => [...r, '']);
  const lines = [
    toCsvLine(newHeaders),
    ...newRows.map(toCsvLine),
  ].join('\n') + '\n';
  writeFileSync(csvPath, lines, 'utf8');
  console.log(`edge-case-seed: added _autospec_synthetic column to ${csvPath}`);
  return newHeaders;
}

// ---------------------------------------------------------------------------
// Append synthetic rows to CSV
// ---------------------------------------------------------------------------

async function appendSyntheticRows(csvPath, syntheticRows) {
  const headers = await ensureSyntheticColumn(csvPath);

  const lines = syntheticRows.map((row) => {
    const fields = headers.map((h) => row[h] ?? '');
    return toCsvLine(fields);
  });
  appendFileSync(csvPath, lines.join('\n') + '\n', 'utf8');
  console.log(`edge-case-seed: inserted ${syntheticRows.length} synthetic row(s) into ${csvPath}`);
}

// ---------------------------------------------------------------------------
// Main processing loop
// ---------------------------------------------------------------------------

const results = [];

for (const { name: shapeName, count_min: countMin, entity } of requireShapes) {
  // Locate shape in merged catalog
  const catalogEntry = mergedCatalog[shapeName];
  if (!catalogEntry) {
    console.error(
      `edge-case-seed: refuse-to-run: shape "${shapeName}" not found in catalog. ` +
      `Register it in catalog.yml or overlay.yml.`,
    );
    process.exit(2);
  }

  const predicate = catalogEntry.predicate_sql;
  if (!predicate) {
    console.error(
      `edge-case-seed: refuse-to-run: shape "${shapeName}" has no predicate_sql in catalog.`,
    );
    process.exit(2);
  }

  // Determine which table CSV to use.
  // Entity key is used as table name hint; fall back to searching all CSVs.
  const candidateTables = entity !== 'enforcement' ? [entity] : [];
  // Also try known shape-related table names
  const additionalGuesses = ['tasks', 'items', 'records'];
  const tableCandidates = [...candidateTables, ...additionalGuesses];

  let csvPath = null;
  let tableName = null;
  for (const t of tableCandidates) {
    if (csvMap[t]) {
      csvPath = csvMap[t];
      tableName = t;
      break;
    }
  }

  if (!csvPath) {
    // If only one CSV exists, use it
    const allCsvs = Object.entries(csvMap);
    if (allCsvs.length === 1) {
      [tableName, csvPath] = allCsvs[0];
    } else {
      console.warn(
        `edge-case-seed: warn: no CSV found for entity "${entity}" (shape "${shapeName}") — skipping`,
      );
      results.push({ shape: shapeName, status: 'skipped', reason: 'no-csv' });
      continue;
    }
  }

  // Count matching rows
  const currentCount = countMatchingRowsSqlite(csvPath, predicate, tableName);
  const shortfall = countMin - currentCount;

  console.log(
    `edge-case-seed: shape "${shapeName}" → table "${tableName}": ` +
    `${currentCount} matching / ${countMin} required (shortfall: ${Math.max(0, shortfall)})`,
  );

  if (shortfall <= 0) {
    console.log(`edge-case-seed: shape "${shapeName}" — surplus, no-op`);
    results.push({ shape: shapeName, status: 'surplus', currentCount, countMin });
    continue;
  }

  // Generate and insert synthetic rows
  const syntheticRows = await generateSyntheticRows(shapeName, catalogEntry, [], shortfall);
  await appendSyntheticRows(csvPath, syntheticRows);
  results.push({ shape: shapeName, status: 'seeded', inserted: shortfall, currentCount, countMin });
}

// ---------------------------------------------------------------------------
// Emit seed-report.json
// ---------------------------------------------------------------------------

const reportPath = join(resolvedSnapshot, 'seed-report.json');
writeFileSync(reportPath, JSON.stringify({ shapes: results }, null, 2) + '\n', 'utf8');
console.log(`edge-case-seed: wrote ${reportPath}`);

const seededCount = results.filter((r) => r.status === 'seeded').length;
const skippedCount = results.filter((r) => r.status === 'skipped').length;
console.log(
  `edge-case-seed: done — ${seededCount} shape(s) seeded, ` +
  `${results.length - seededCount - skippedCount} surplus, ${skippedCount} skipped`,
);
process.exit(0);
