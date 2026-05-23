#!/usr/bin/env node
// skills/autospec-e2e-clone/scripts/scale-down.mjs
//
// Scale-down with foreign-key reachability (C5).
//
// Reads FK constraints from the snapshot's schema.sql (or a fk-meta.json file),
// samples N rows per table per `tables_sample` contract declaration, then performs
// BFS reachability closure: any row referenced via FK by a sampled row is pulled in
// from its own snapshot CSV.  Rewrites the CSV files to include only the closure set.
// Emits <snapshot-dir>/manifest.json with per-table included-row counts.
//
// Usage:
//   node scale-down.mjs <snapshot-dir> [--contract <path>] [--repo-root <path>]
//
// Exit codes:
//   0  success (or nothing-to-do)
//   1  fatal (missing deps, bad files)
//   2  refuse-to-run (contract invalid)

import {
  existsSync,
  mkdirSync,
  readFileSync,
  writeFileSync,
  readdirSync,
} from 'node:fs';
import { join, resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';
import { createInterface } from 'node:readline';
import { createReadStream } from 'node:fs';
import { parseArgs } from 'node:util';

// ---------------------------------------------------------------------------
// Arg parsing
// ---------------------------------------------------------------------------

const { values: args, positionals } = parseArgs({
  args: process.argv.slice(2),
  options: {
    contract: { type: 'string', short: 'c' },
    'repo-root': { type: 'string', short: 'r' },
    help: { type: 'boolean', short: 'h' },
  },
  allowPositionals: true,
  strict: true,
});

if (args.help) {
  console.log(
    'Usage: node scale-down.mjs <snapshot-dir> [--contract <clone.yml>] [--repo-root <root>]',
  );
  process.exit(0);
}

const snapshotDir = positionals[0];
if (!snapshotDir) {
  console.error('scale-down: fatal: <snapshot-dir> is required');
  process.exit(1);
}

const resolvedSnapshot = resolve(snapshotDir);
if (!existsSync(resolvedSnapshot)) {
  console.error(`scale-down: fatal: snapshot-dir not found: ${resolvedSnapshot}`);
  process.exit(1);
}

// ---------------------------------------------------------------------------
// Locate repo root and contract
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
    'scale-down: fatal: could not locate repo root (no .autospec/ directory found)',
  );
  process.exit(1);
}

const contractPath = args.contract
  ? resolve(args.contract)
  : join(repoRoot, '.autospec', 'clone.yml');

if (!existsSync(contractPath)) {
  console.error(`scale-down: refuse-to-run: contract not found: ${contractPath}`);
  process.exit(2);
}

// ---------------------------------------------------------------------------
// Load contract via yq
// ---------------------------------------------------------------------------

function requireTool(tool) {
  try {
    execFileSync('which', [tool], { stdio: 'ignore' });
  } catch {
    console.error(
      `scale-down: fatal: ${tool} not found. Install with: brew install ${tool}`,
    );
    process.exit(1);
  }
}

requireTool('yq');

let contractJson;
try {
  contractJson = JSON.parse(
    execFileSync('yq', ['-o=json', '.', contractPath], { encoding: 'utf8' }),
  );
} catch (err) {
  console.error(`scale-down: fatal: failed to parse contract: ${err.message}`);
  process.exit(1);
}

// ---------------------------------------------------------------------------
// Extract scale_down config and tables_sample from all sources
// ---------------------------------------------------------------------------

const scaleDownConfig = contractJson?.scale_down ?? {};
const foreignKeyAware = scaleDownConfig.foreign_key_aware !== false; // default true
const maxDepth = Number(scaleDownConfig.max_depth ?? 8);

/** @type {Record<string, number>} table -> sample limit */
const tablesSample = {};

const sources = Array.isArray(contractJson?.sources) ? contractJson.sources : [];
for (const src of sources) {
  if (src.tables_sample && typeof src.tables_sample === 'object') {
    for (const [tbl, limit] of Object.entries(src.tables_sample)) {
      tablesSample[tbl] = Number(limit);
    }
  }
}

if (Object.keys(tablesSample).length === 0) {
  console.log('scale-down: no tables_sample declared — nothing to do');
  writeManifest({});
  process.exit(0);
}

// ---------------------------------------------------------------------------
// CSV helpers
// ---------------------------------------------------------------------------

/**
 * Parse a CSV file into array of row-objects.  Handles quoted fields naively
 * (no embedded newlines, no escaped quotes within quotes).
 * @param {string} filePath
 * @returns {{ headers: string[], rows: Record<string, string>[] }}
 */
function readCsv(filePath) {
  if (!existsSync(filePath)) return { headers: [], rows: [] };
  const raw = readFileSync(filePath, 'utf8');
  const lines = raw.split('\n').filter((l) => l.trim() !== '');
  if (lines.length === 0) return { headers: [], rows: [] };

  const headers = parseCsvLine(lines[0]);
  const rows = [];
  for (let i = 1; i < lines.length; i++) {
    const values = parseCsvLine(lines[i]);
    const row = {};
    for (let j = 0; j < headers.length; j++) {
      row[headers[j]] = values[j] ?? '';
    }
    rows.push(row);
  }
  return { headers, rows };
}

/**
 * Split a single CSV line respecting double-quoted fields.
 * @param {string} line
 * @returns {string[]}
 */
function parseCsvLine(line) {
  const fields = [];
  let field = '';
  let inQuote = false;
  for (let i = 0; i < line.length; i++) {
    const ch = line[i];
    if (inQuote) {
      if (ch === '"') {
        if (line[i + 1] === '"') {
          field += '"';
          i++;
        } else {
          inQuote = false;
        }
      } else {
        field += ch;
      }
    } else if (ch === '"') {
      inQuote = true;
    } else if (ch === ',') {
      fields.push(field);
      field = '';
    } else {
      field += ch;
    }
  }
  fields.push(field);
  return fields;
}

/**
 * Serialize rows back to CSV.
 * @param {string[]} headers
 * @param {Record<string, string>[]} rows
 * @returns {string}
 */
function writeCsvString(headers, rows) {
  const lines = [headers.join(',')];
  for (const row of rows) {
    const values = headers.map((h) => {
      const v = row[h] ?? '';
      // Quote if contains comma, quote, or newline
      if (v.includes(',') || v.includes('"') || v.includes('\n')) {
        return '"' + v.replace(/"/g, '""') + '"';
      }
      return v;
    });
    lines.push(values.join(','));
  }
  return lines.join('\n') + '\n';
}

// ---------------------------------------------------------------------------
// FK metadata: parse schema.sql or load fk-meta.json
// ---------------------------------------------------------------------------

/**
 * A FK edge: child table has a column that references parent table's column.
 * @typedef {{ childTable: string, childCol: string, parentTable: string, parentCol: string }} FkEdge
 */

/**
 * Parse FK constraints from schema.sql (PostgreSQL DDL format).
 * Also handles SQLite PRAGMA FK list format (written as JSON by sqlite.sh if present).
 * @param {string} snapshotDir
 * @returns {FkEdge[]}
 */
function loadFkEdges(snapshotDir) {
  // Prefer explicit fk-meta.json if written by snapshot driver
  const fkMetaPath = join(snapshotDir, 'fk-meta.json');
  if (existsSync(fkMetaPath)) {
    try {
      const raw = JSON.parse(readFileSync(fkMetaPath, 'utf8'));
      if (Array.isArray(raw)) return raw;
    } catch {
      // fall through to schema.sql
    }
  }

  const schemaPath = join(snapshotDir, 'schema.sql');
  if (!existsSync(schemaPath)) return [];

  const sql = readFileSync(schemaPath, 'utf8');
  return parseFkEdgesFromSql(sql);
}

/**
 * Extract FOREIGN KEY ... REFERENCES ... from SQL DDL.
 * Handles both inline column constraints and table-level CONSTRAINT clauses.
 *
 * Patterns matched:
 *   FOREIGN KEY (child_col) REFERENCES parent_table (parent_col)
 *   REFERENCES parent_table (parent_col)  [inline column reference]
 * @param {string} sql
 * @returns {FkEdge[]}
 */
function parseFkEdgesFromSql(sql) {
  const edges = [];

  // Match table-level FOREIGN KEY constraints
  // FOREIGN KEY (col[, col]) REFERENCES table (col[, col])
  const fkPattern =
    /FOREIGN\s+KEY\s*\(([^)]+)\)\s+REFERENCES\s+"?(\w+)"?\s*\(([^)]+)\)/gi;
  let match;

  // We also need to know which table we're currently parsing.
  // Split by CREATE TABLE blocks.
  const createTablePattern =
    /CREATE\s+TABLE\s+(?:IF\s+NOT\s+EXISTS\s+)?(?:"?(\w+)"?\.)?["']?(\w+)["']?\s*\(/gi;

  // Build a map of table -> raw DDL body
  const tableBlocks = [];
  const tableMatches = [...sql.matchAll(createTablePattern)];
  for (let i = 0; i < tableMatches.length; i++) {
    const tm = tableMatches[i];
    const tableName = tm[2] || tm[1];
    const start = tm.index + tm[0].length;
    const end =
      i + 1 < tableMatches.length ? tableMatches[i + 1].index : sql.length;
    tableBlocks.push({ tableName, body: sql.slice(start, end) });
  }

  for (const block of tableBlocks) {
    const bodyFkPattern =
      /FOREIGN\s+KEY\s*\(([^)]+)\)\s+REFERENCES\s+"?(\w+)"?\s*\(([^)]+)\)/gi;
    let m;
    while ((m = bodyFkPattern.exec(block.body)) !== null) {
      const childCols = m[1].split(',').map((c) => c.trim().replace(/"/g, ''));
      const parentTable = m[2];
      const parentCols = m[3].split(',').map((c) => c.trim().replace(/"/g, ''));
      for (let k = 0; k < childCols.length; k++) {
        edges.push({
          childTable: block.tableName,
          childCol: childCols[k],
          parentTable,
          parentCol: parentCols[k] ?? parentCols[0],
        });
      }
    }
  }

  // Also match standalone REFERENCES outside CREATE TABLE (e.g., ALTER TABLE ADD CONSTRAINT)
  const alterPattern =
    /ALTER\s+TABLE\s+(?:ONLY\s+)?["']?(\w+)["']?\s+ADD\s+(?:CONSTRAINT\s+\w+\s+)?FOREIGN\s+KEY\s*\(([^)]+)\)\s+REFERENCES\s+"?(\w+)"?\s*\(([^)]+)\)/gi;
  let am;
  while ((am = alterPattern.exec(sql)) !== null) {
    const childTable = am[1];
    const childCols = am[2].split(',').map((c) => c.trim().replace(/"/g, ''));
    const parentTable = am[3];
    const parentCols = am[4].split(',').map((c) => c.trim().replace(/"/g, ''));
    for (let k = 0; k < childCols.length; k++) {
      edges.push({
        childTable,
        childCol: childCols[k],
        parentTable,
        parentCol: parentCols[k] ?? parentCols[0],
      });
    }
  }

  return edges;
}

// ---------------------------------------------------------------------------
// Main scale-down logic
// ---------------------------------------------------------------------------

/**
 * Detect all CSV tables in the snapshot directory.
 * @returns {string[]} table names (without .csv)
 */
function detectSnapshotTables(snapshotDir) {
  return readdirSync(snapshotDir)
    .filter((f) => f.endsWith('.csv'))
    .map((f) => f.slice(0, -4));
}

/**
 * Write manifest.json
 * @param {Record<string, number>} counts
 */
function writeManifest(counts) {
  const manifestPath = join(resolvedSnapshot, 'manifest.json');
  writeFileSync(manifestPath, JSON.stringify(counts, null, 2) + '\n', 'utf8');
  console.log(`scale-down: wrote manifest → ${manifestPath}`);
}

async function main() {
  const allTables = detectSnapshotTables(resolvedSnapshot);
  console.log(`scale-down: detected snapshot tables: ${allTables.join(', ') || '(none)'}`);
  console.log(`scale-down: tables_sample: ${JSON.stringify(tablesSample)}`);
  console.log(`scale-down: foreign_key_aware=${foreignKeyAware}, max_depth=${maxDepth}`);

  // Load all table CSVs into memory
  /** @type {Record<string, { headers: string[], rows: Record<string, string>[] }>} */
  const tableData = {};
  for (const tbl of allTables) {
    tableData[tbl] = readCsv(join(resolvedSnapshot, `${tbl}.csv`));
  }

  // Also load any tables listed in tables_sample that might not be in allTables yet
  for (const tbl of Object.keys(tablesSample)) {
    if (!tableData[tbl]) {
      const p = join(resolvedSnapshot, `${tbl}.csv`);
      tableData[tbl] = readCsv(p);
    }
  }

  if (!foreignKeyAware) {
    // Simple sampling only — no reachability closure
    console.log('scale-down: foreign_key_aware=false — performing naive sampling only');
    const counts = {};
    for (const [tbl, limit] of Object.entries(tablesSample)) {
      const data = tableData[tbl];
      if (!data || data.rows.length === 0) {
        counts[tbl] = 0;
        continue;
      }
      const sampled = data.rows.slice(0, limit);
      writeFileSync(
        join(resolvedSnapshot, `${tbl}.csv`),
        writeCsvString(data.headers, sampled),
        'utf8',
      );
      counts[tbl] = sampled.length;
      console.log(`scale-down: naive sample ${tbl}: ${sampled.length} rows`);
    }
    // Tables not in tables_sample remain untouched
    for (const tbl of allTables) {
      if (!(tbl in counts)) {
        counts[tbl] = tableData[tbl]?.rows.length ?? 0;
      }
    }
    writeManifest(counts);
    return;
  }

  // ---------------------------------------------------------------------------
  // FK-aware reachability closure
  // ---------------------------------------------------------------------------

  const fkEdges = loadFkEdges(resolvedSnapshot);
  console.log(`scale-down: loaded ${fkEdges.length} FK edge(s) from schema`);

  // Build index: childTable -> list of edges
  /** @type {Record<string, FkEdge[]>} */
  const childEdges = {};
  for (const edge of fkEdges) {
    if (!childEdges[edge.childTable]) childEdges[edge.childTable] = [];
    childEdges[edge.childTable].push(edge);
  }

  // Build lookup: parentTable -> parentCol -> Set of parentCol values (for fast FK lookup)
  // We'll build this lazily per table.

  /**
   * For each table, track which row indices (0-based in original rows array) are included.
   * @type {Record<string, Set<number>>}
   */
  const includedIndices = {};
  for (const tbl of Object.keys(tableData)) {
    includedIndices[tbl] = new Set();
  }

  // Step 1: seed with sampled rows from tables_sample
  for (const [tbl, limit] of Object.entries(tablesSample)) {
    const data = tableData[tbl];
    if (!data || data.rows.length === 0) continue;
    const seedCount = Math.min(limit, data.rows.length);
    for (let i = 0; i < seedCount; i++) {
      includedIndices[tbl] = includedIndices[tbl] || new Set();
      includedIndices[tbl].add(i);
    }
    console.log(`scale-down: seeded ${seedCount} rows from ${tbl}`);
  }

  // Step 2: BFS reachability closure
  // A "work item" is { table, rowIndex } — we need to pull in all FK-referenced parent rows.
  // We do BFS level by level up to maxDepth.

  /**
   * Build a column-value index for fast lookup.
   * @param {string} tbl
   * @param {string} col
   * @returns {Map<string, number[]>}  value -> array of row indices
   */
  function buildColIndex(tbl, col) {
    const data = tableData[tbl];
    if (!data) return new Map();
    const idx = new Map();
    for (let i = 0; i < data.rows.length; i++) {
      const v = data.rows[i][col];
      if (v === undefined || v === '' || v === null) continue;
      if (!idx.has(v)) idx.set(v, []);
      idx.get(v).push(i);
    }
    return idx;
  }

  /** Cache of col indices: tbl:col -> Map */
  const colIndexCache = new Map();
  function getColIndex(tbl, col) {
    const key = `${tbl}:${col}`;
    if (!colIndexCache.has(key)) {
      colIndexCache.set(key, buildColIndex(tbl, col));
    }
    return colIndexCache.get(key);
  }

  // Queue of newly included (tbl, rowIndex) pairs to process
  let queue = [];
  for (const [tbl, set] of Object.entries(includedIndices)) {
    for (const idx of set) {
      queue.push({ tbl, idx });
    }
  }

  let depth = 0;
  while (queue.length > 0 && depth < maxDepth) {
    depth++;
    const nextQueue = [];

    for (const { tbl, idx } of queue) {
      const edges = childEdges[tbl] || [];
      const row = tableData[tbl]?.rows[idx];
      if (!row) continue;

      for (const edge of edges) {
        // This row (child) references parent table via edge.childCol -> edge.parentTable.parentCol
        const childVal = row[edge.childCol];
        if (childVal === undefined || childVal === '' || childVal === null) continue;

        // Find matching rows in parent table
        const parentIndex = getColIndex(edge.parentTable, edge.parentCol);
        const matchingParentRows = parentIndex.get(String(childVal)) || [];

        const parentIncluded = includedIndices[edge.parentTable];
        if (!parentIncluded) {
          includedIndices[edge.parentTable] = new Set();
        }

        for (const parentRowIdx of matchingParentRows) {
          if (!includedIndices[edge.parentTable].has(parentRowIdx)) {
            includedIndices[edge.parentTable].add(parentRowIdx);
            nextQueue.push({ tbl: edge.parentTable, idx: parentRowIdx });
          }
        }
      }
    }

    queue = nextQueue;
    if (nextQueue.length > 0) {
      console.log(`scale-down: BFS depth ${depth}: pulled in ${nextQueue.length} additional row(s)`);
    }
  }

  // Step 3: For tables NOT in tables_sample, keep ALL rows (they were not seeded for sampling,
  // but reachability may have added some; tables_full tables keep all rows).
  // Only tables that were explicitly sampled get pruned.
  // Tables only referenced via FK get all rows kept unless they were themselves seeded.
  const sampledTables = new Set(Object.keys(tablesSample));

  // Step 4: Rewrite CSV files
  const manifest = {};

  for (const tbl of Object.keys(tableData)) {
    const data = tableData[tbl];
    if (!data || data.headers.length === 0) continue;

    let finalRows;
    if (sampledTables.has(tbl)) {
      // Pruned: only keep included rows (sampled + closure-pulled)
      const included = includedIndices[tbl] || new Set();
      finalRows = data.rows.filter((_, i) => included.has(i));
    } else {
      // Not a sampled table: keep all rows (it's either tables_full or a parent table)
      // But if reachability pulled in a subset and the table had rows not referenced,
      // we still keep all — conservative approach to not break non-FK data.
      finalRows = data.rows;
    }

    const csvPath = join(resolvedSnapshot, `${tbl}.csv`);
    writeFileSync(csvPath, writeCsvString(data.headers, finalRows), 'utf8');
    manifest[tbl] = finalRows.length;
    console.log(`scale-down: ${tbl}: ${finalRows.length} rows kept (of ${data.rows.length})`);
  }

  writeManifest(manifest);
}

main().catch((err) => {
  console.error(`scale-down: fatal: ${err.message}`);
  process.exit(1);
});
