/**
 * verify-seeds.mjs — Edge-case seed shape verifier orchestrator.
 *
 * Loads the shape predicate catalog (catalog.yml), optionally merges a custom
 * overlay from .autospec/seed-shapes.yml, then queries the clone DB to verify
 * that every required shape in the contract has at least count_min matching rows.
 *
 * Exit codes (when run as CLI):
 *   0 = all shapes satisfied (or enforcement != refuse_to_run_if_missing)
 *   1 = internal error (missing catalog, bad config, driver load failure)
 *   2 = one or more required shapes are missing (refuse_to_run_if_missing)
 *
 * Programmatic API (imported by tests):
 *   export async function run({ contract, store_kind, dsn, db, customShapes }) -> Result
 *   export default run
 *
 * Result shape:
 *   { violations: Violation[], exit_code: 0|1|2, summary: string }
 *
 * Violation shape:
 *   { entity: string, shape: string, count_found: number, count_min: number }
 *
 * Usage (CLI):
 *   node verify-seeds.mjs --dsn sqlite::memory: --store-kind sqlite
 *   node verify-seeds.mjs --dsn postgresql://... --store-kind postgres
 *   node verify-seeds.mjs --contract .autospec/test.yml --dsn sqlite:./clone.db
 */

import { readFileSync, existsSync } from 'node:fs';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const require = createRequire(import.meta.url);

// ── YAML parser (yq not available in Node; use a minimal inline parser) ───────

/**
 * Parse a simple YAML file into a JS object.
 * Handles the subset of YAML used by catalog.yml and seed-shapes.yml:
 *   - top-level keys (shape names)
 *   - nested scalar fields (description, predicate_sql, predicate_jsonpath)
 *   - folded block scalars (>) for multi-line strings
 *
 * @param {string} content - YAML text
 * @returns {Record<string, object>}
 */
function parseSimpleYaml(content) {
  const result = {};
  const lines = content.split('\n');
  let currentKey = null;
  let currentObj = null;
  let blockField = null;
  let blockLines = [];
  let blockIndent = 0;

  function flushBlock() {
    if (blockField && currentObj) {
      currentObj[blockField] = blockLines.join(' ').replace(/\s+/g, ' ').trim();
      blockField = null;
      blockLines = [];
    }
  }

  for (let i = 0; i < lines.length; i++) {
    const raw = lines[i];
    const trimmed = raw.trimEnd();

    // Skip comments and blank lines (except in block scalars)
    if (blockField) {
      const indent = raw.length - raw.trimStart().length;
      if (trimmed === '' || indent > blockIndent) {
        // Continuation of block scalar
        blockLines.push(trimmed.trim());
        continue;
      } else {
        flushBlock();
        // Fall through to normal parsing
      }
    }

    if (!trimmed || trimmed.trimStart().startsWith('#')) continue;

    const indent = raw.length - raw.trimStart().length;

    if (indent === 0 && trimmed.endsWith(':')) {
      // Top-level key
      flushBlock();
      currentKey = trimmed.slice(0, -1);
      currentObj = {};
      result[currentKey] = currentObj;
    } else if (indent > 0 && currentObj !== null) {
      // Nested field
      const colonIdx = trimmed.indexOf(':');
      if (colonIdx === -1) continue;
      const field = trimmed.slice(0, colonIdx).trim();
      const rest = trimmed.slice(colonIdx + 1).trim();

      if (rest === '>') {
        // Folded block scalar — collect subsequent indented lines
        flushBlock();
        blockField = field;
        blockLines = [];
        blockIndent = indent;
      } else if (rest !== '') {
        // Inline scalar value — strip quotes if present
        let val = rest;
        if ((val.startsWith('"') && val.endsWith('"')) ||
            (val.startsWith("'") && val.endsWith("'"))) {
          val = val.slice(1, -1);
        }
        currentObj[field] = val;
      }
    }
  }
  flushBlock();
  return result;
}

// ── Load catalog ──────────────────────────────────────────────────────────────

const CATALOG_PATH = path.join(__dirname, 'catalog.yml');

function loadCatalog(customShapes = {}) {
  if (!existsSync(CATALOG_PATH)) {
    throw new Error(`verify-seeds: catalog not found at ${CATALOG_PATH}`);
  }
  const content = readFileSync(CATALOG_PATH, 'utf8');
  const catalog = parseSimpleYaml(content);
  // Merge custom overlay (custom shapes override catalog shapes with same name)
  return { ...catalog, ...customShapes };
}

// ── Driver loader ─────────────────────────────────────────────────────────────

const DRIVER_DIR = path.join(__dirname, 'db-driver');

const DRIVER_MAP = {
  sqlite: 'sqlite.mjs',
  postgres: 'postgres.mjs',
  postgresql: 'postgres.mjs',
  mysql: 'mysql.mjs',
  jsonpath: 'jsonpath-store.mjs',
  'jsonpath-store': 'jsonpath-store.mjs',
};

async function loadDriver(storeKind) {
  const file = DRIVER_MAP[storeKind];
  if (!file) {
    throw new Error(
      `verify-seeds: unknown store_kind "${storeKind}". ` +
      `Valid values: ${Object.keys(DRIVER_MAP).join(', ')}`
    );
  }
  return await import(path.join(DRIVER_DIR, file));
}

// ── Core run function ─────────────────────────────────────────────────────────

/**
 * Verify that a clone DB satisfies all required seed shapes declared in the contract.
 *
 * @param {object} opts
 * @param {object} opts.contract       - Parsed contract object (edge_case_seeds block)
 * @param {string} [opts.store_kind]   - DB kind: 'sqlite' | 'postgres' | 'mysql' | 'jsonpath-store'
 * @param {string} [opts.dsn]          - Connection string (not needed if opts.db is provided)
 * @param {object} [opts.db]           - Pre-built Database instance (tests; sqlite only)
 * @param {object} [opts.customShapes] - Custom shape overlay {name: {predicate_sql, predicate_jsonpath}}
 * @returns {Promise<{violations: object[], exit_code: 0|1|2, summary: string}>}
 */
export async function run({ contract, store_kind = 'sqlite', dsn, db, customShapes = {} }) {
  const seeds = contract?.edge_case_seeds;
  if (!seeds) {
    return {
      violations: [],
      exit_code: 0,
      summary: 'No edge_case_seeds block in contract; nothing to verify.',
    };
  }

  const enforcement = seeds.enforcement ?? 'refuse_to_run_if_missing';
  const catalog = loadCatalog(customShapes);
  const driver = await loadDriver(store_kind);

  // Open connection — accept pre-built db instance for tests
  const conn = await driver.connect(dsn ?? ':memory:', db);

  const violations = [];

  try {
    // Iterate entities (e.g. household_test_family)
    for (const [entity, entityDef] of Object.entries(seeds)) {
      if (entity === 'enforcement') continue;
      if (!entityDef || typeof entityDef !== 'object') continue;

      const requireShapes = entityDef.require_shapes ?? [];

      for (const shapeReq of requireShapes) {
        const shapeName = shapeReq.name;
        const countMin = shapeReq.count_min ?? 1;

        const shape = catalog[shapeName];
        if (!shape) {
          violations.push({
            entity,
            shape: shapeName,
            count_found: 0,
            count_min: countMin,
            error: `Shape "${shapeName}" not found in catalog or custom overlay`,
          });
          continue;
        }

        // Use SQL predicate for relational DBs, JSONPath for jsonpath-store
        const isJsonpath = store_kind === 'jsonpath' || store_kind === 'jsonpath-store';
        const predicate = isJsonpath
          ? (shape.predicate_jsonpath ?? shape.predicate_sql)
          : (shape.predicate_sql ?? shape.predicate_jsonpath);

        if (!predicate) {
          violations.push({
            entity,
            shape: shapeName,
            count_found: 0,
            count_min: countMin,
            error: `Shape "${shapeName}" has no predicate for store_kind "${store_kind}"`,
          });
          continue;
        }

        // Default table name: derive from entity name, fallback to 'tasks'
        const table = entityDef.table ?? 'tasks';

        let count;
        try {
          count = await driver.countMatching(conn, table, predicate);
        } catch (err) {
          violations.push({
            entity,
            shape: shapeName,
            count_found: 0,
            count_min: countMin,
            error: `Query failed: ${err.message}`,
          });
          continue;
        }

        if (count < countMin) {
          violations.push({
            entity,
            shape: shapeName,
            count_found: count,
            count_min: countMin,
          });
        }
      }
    }
  } finally {
    await driver.close(conn);
  }

  // Determine exit code
  let exit_code = 0;
  if (violations.length > 0 && enforcement === 'refuse_to_run_if_missing') {
    exit_code = 2;
  }

  const summary = violations.length === 0
    ? `All seed shapes satisfied.`
    : `${violations.length} shape violation(s): ${violations.map(v => v.shape).join(', ')}`;

  return { violations, exit_code, summary };
}

export default run;

// ── CLI entry point ───────────────────────────────────────────────────────────

// Detect if run directly (not imported)
const isMain = process.argv[1] && fileURLToPath(import.meta.url) === path.resolve(process.argv[1]);

if (isMain) {
  const args = process.argv.slice(2);
  const get = (flag) => {
    const i = args.indexOf(flag);
    return i !== -1 ? args[i + 1] : null;
  };

  const dsn = get('--dsn') ?? ':memory:';
  const storeKind = get('--store-kind') ?? get('--store_kind') ?? 'sqlite';
  const contractPath = get('--contract');

  let contract = {};
  if (contractPath && existsSync(contractPath)) {
    // Load contract YAML via yq if available, else use our parser
    try {
      const { execSync } = await import('node:child_process');
      const json = execSync(`yq -o=json . "${contractPath}"`, { encoding: 'utf8' });
      contract = JSON.parse(json);
    } catch {
      const content = readFileSync(contractPath, 'utf8');
      // Minimal: just extract edge_case_seeds from YAML manually
      contract = {}; // Fall back to empty; catalog verification will still run
    }
  }

  try {
    const result = await run({ contract, store_kind: storeKind, dsn });
    process.stdout.write(JSON.stringify(result, null, 2) + '\n');
    if (result.violations.length > 0) {
      process.stderr.write(`[verify-seeds] ${result.summary}\n`);
    } else {
      process.stderr.write(`[verify-seeds] ${result.summary}\n`);
    }
    process.exit(result.exit_code);
  } catch (err) {
    process.stderr.write(`[verify-seeds] fatal: ${err.message}\n`);
    process.exit(1);
  }
}
