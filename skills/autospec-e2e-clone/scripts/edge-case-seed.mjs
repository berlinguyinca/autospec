#!/usr/bin/env node
import {
  existsSync,
  writeFileSync,
  readdirSync,
} from 'node:fs';
import { join, resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';
import { countMatchingRows } from './edge-case-seed/sqlite.mjs';
import {
  appendSyntheticRows,
} from './edge-case-seed/csv.mjs';
import { generateSyntheticRows } from './edge-case-seed/shapes.mjs';
const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
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
function defaultCatalogPath() {
  const fromEnv = process.env.AUTOSPEC_SCRIPTS_DIR
    ? join(process.env.AUTOSPEC_SCRIPTS_DIR, 'seed-shapes', 'catalog.yml')
    : null;
  if (fromEnv && existsSync(fromEnv)) return fromEnv;
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
/**
 * Evaluates a predicate_sql fragment against rows in a CSV using SQLite.
 * Writes the CSV to a temp SQLite DB, runs SELECT COUNT(*) WHERE <predicate>.
 * Returns the count of matching rows.
 */
function countMatchingRowsSqlite(csvPath, predicate, tableName = 'data') {
  try { return countMatchingRows(csvPath, predicate, tableName); } catch (err) {
    console.warn(
      `edge-case-seed: warn: predicate evaluation failed for "${predicate}": ${err.message.split('\n')[0]}`,
    );
    return 0;
  }
}
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
// CSV serialization and shape generation live in reusable modules so the CLI
// and direct module consumers exercise the same behavior.
const results = [];
for (const { name: shapeName, count_min: countMin, entity } of requireShapes) {
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
  const candidateTables = entity !== 'enforcement' ? [entity] : [];
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
  const syntheticRows = await generateSyntheticRows(
    shapeName,
    catalogEntry,
    shortfall,
    await getFaker(),
  );
  appendSyntheticRows(csvPath, syntheticRows);
  results.push({ shape: shapeName, status: 'seeded', inserted: shortfall, currentCount, countMin });
}
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
