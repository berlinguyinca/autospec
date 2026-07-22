#!/usr/bin/env node
// skills/autospec-e2e-clone/scripts/anonymize.mjs
//
// Anonymize engine (C4) — reads snapshot SQL/CSV data, applies per-column
// rules from .autospec/clone.yml, writes anonymized output alongside, and
// persists a reversible mapping to .autospec/anonymize-map.<contract-sha>.json.
//
// Usage:
//   node anonymize.mjs <snapshot-dir> [--contract <path-to-clone.yml>]
//                                      [--repo-root <repo-root>]
//
// Exit codes:
//   0  success
//   1  fatal (missing deps, unreadable files, internal error)
//   2  refuse-to-run (contract invalid / no anonymize rules)

import { createHash } from 'node:crypto';
import {
  createReadStream,
  createWriteStream,
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  renameSync,
  unlinkSync,
  writeFileSync,
} from 'node:fs';
import { execFileSync } from 'node:child_process';
import { dirname, join, resolve } from 'node:path';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';
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
  console.log(`Usage: node anonymize.mjs <snapshot-dir> [--contract <clone.yml>] [--repo-root <root>]`);
  process.exit(0);
}

const snapshotDir = positionals[0];
if (!snapshotDir) {
  console.error('anonymize: fatal: <snapshot-dir> is required');
  process.exit(1);
}

const resolvedSnapshot = resolve(snapshotDir);
if (!existsSync(resolvedSnapshot)) {
  console.error(`anonymize: fatal: snapshot-dir not found: ${resolvedSnapshot}`);
  process.exit(1);
}

// Locate repo root: explicit flag > walk up from snapshot dir looking for .autospec/
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
  console.error('anonymize: fatal: could not locate repo root (no .autospec/ directory found)');
  process.exit(1);
}

const contractPath = args.contract
  ? resolve(args.contract)
  : join(repoRoot, '.autospec', 'clone.yml');

if (!existsSync(contractPath)) {
  console.error(`anonymize: refuse-to-run: contract not found: ${contractPath}`);
  process.exit(2);
}

// ---------------------------------------------------------------------------
// Load contract (requires yq)
// ---------------------------------------------------------------------------

function requireTool(tool) {
  try {
    execFileSync('which', [tool], { stdio: 'ignore' });
  } catch {
    console.error(`anonymize: fatal: ${tool} not found. Install with: brew install ${tool}`);
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
  console.error(`anonymize: fatal: failed to parse contract: ${err.message}`);
  process.exit(1);
}

const anonymizeConfig = contractJson?.anonymize;
if (!anonymizeConfig || !Array.isArray(anonymizeConfig.rules) || anonymizeConfig.rules.length === 0) {
  console.log('anonymize: no anonymize.rules defined in contract — nothing to do');
  process.exit(0);
}

// ---------------------------------------------------------------------------
// Contract SHA (for map file naming)
// ---------------------------------------------------------------------------

const contractSha = createHash('sha256')
  .update(readFileSync(contractPath))
  .digest('hex')
  .slice(0, 16);

// ---------------------------------------------------------------------------
// Reversible map persistence
// ---------------------------------------------------------------------------

const autospecDir = join(repoRoot, '.autospec');
mkdirSync(autospecDir, { recursive: true });

const mapPath = join(autospecDir, `anonymize-map.${contractSha}.json`);

/** @type {Record<string, Record<string, string>>} */
let reversibleMap = {};
if (existsSync(mapPath)) {
  try {
    reversibleMap = JSON.parse(readFileSync(mapPath, 'utf8'));
  } catch {
    console.error(`anonymize: warn: could not parse existing map ${mapPath} — starting fresh`);
    reversibleMap = {};
  }
}

function saveMap() {
  writeFileSync(mapPath, JSON.stringify(reversibleMap, null, 2) + '\n', 'utf8');
}

// ---------------------------------------------------------------------------
// Rule implementations
// ---------------------------------------------------------------------------

/** @param {string} value */
function hashWithDomain(value) {
  if (value === null || value === undefined || value === '') return value;
  const s = String(value);
  const atIdx = s.indexOf('@');
  if (atIdx === -1) {
    // No @ — hash the whole value
    return createHash('sha256').update(s).digest('hex').slice(0, 16) + '@example.com';
  }
  const local = s.slice(0, atIdx);
  const domain = s.slice(atIdx + 1);
  const hashed = createHash('sha256').update(local).digest('hex').slice(0, 16);
  return `${hashed}@${domain}`;
}

/** Redact — always returns null (one-way, not reversible) */
function redact(_value) {
  return null;
}

/** Scrub phone to country code: +49 123 456 → +49-XXXXXXX */
function scrubToCountryCode(value) {
  if (value === null || value === undefined || value === '') return value;
  const s = String(value).trim();
  // Extract leading + and digits until first space/dash/non-digit-after-plus
  const match = s.match(/^(\+\d{1,4})/);
  if (match) {
    return `${match[1]}-XXXXXXX`;
  }
  // Fallback: replace all digits after first 3 chars
  return s.slice(0, 3).replace(/\d/g, 'X') + 'XXXXXXX';
}

/** Replace with faker — deterministic seeding based on table+col+original */
async function replaceWithFaker(value, tableKey, _params) {
  // Lazy-load faker to keep startup fast when not needed
  if (!replaceWithFaker._fakerModule) {
    replaceWithFaker._fakerModule = await import('@faker-js/faker');
  }
  const { faker } = replaceWithFaker._fakerModule;
  // Seed faker deterministically so reruns produce the same output
  const seed = createHash('sha256')
    .update(`${tableKey}:${String(value)}`)
    .digest();
  faker.seed(seed.readUInt32BE(0));
  // Default to person.fullName if no specific kind specified
  const kind = _params?.kind ?? 'fullName';
  // Navigate faker namespace: "person.fullName" → faker.person.fullName()
  const parts = kind.includes('.') ? kind.split('.') : ['person', kind];
  let fn = faker;
  for (const part of parts) {
    fn = fn?.[part];
  }
  if (typeof fn === 'function') {
    return String(fn());
  }
  // Fallback
  return faker.person.fullName();
}

/** Scrub IP to /24 subnet: 192.168.1.5 → 192.168.1.0 */
function scrubToSubnet(value) {
  if (value === null || value === undefined || value === '') return value;
  const s = String(value).trim();
  // IPv4
  const v4 = s.match(/^(\d{1,3}\.\d{1,3}\.\d{1,3})\.\d{1,3}$/);
  if (v4) return `${v4[1]}.0`;
  // IPv6 — zero last 80 bits (keep first 48 bits / 3 groups)
  if (s.includes(':')) {
    const groups = s.split(':');
    if (groups.length >= 3) {
      return groups.slice(0, 3).join(':') + ':0:0:0:0:0';
    }
  }
  return s;
}

// ---------------------------------------------------------------------------
// Apply rule to a single value, maintaining reversible map
// ---------------------------------------------------------------------------

/**
 * @param {string} rule  rule kind string
 * @param {*}      value  original column value
 * @param {string} tableCol  e.g. "users.email"
 * @param {object} params  optional rule params
 * @returns {Promise<*>}  anonymized value
 */
async function applyRule(rule, value, tableCol, params) {
  if (value === null || value === undefined) return value;

  switch (rule) {
    case 'hash_with_domain': {
      const anon = hashWithDomain(value);
      // Reversible: store mapping
      if (!reversibleMap[tableCol]) reversibleMap[tableCol] = {};
      reversibleMap[tableCol][String(value)] = anon;
      return anon;
    }
    case 'redact':
      // One-way — NOT stored in reversible map
      return redact(value);

    case 'scrub_to_country_code': {
      const anon = scrubToCountryCode(value);
      if (!reversibleMap[tableCol]) reversibleMap[tableCol] = {};
      reversibleMap[tableCol][String(value)] = anon;
      return anon;
    }
    case 'replace_with_faker': {
      const anon = await replaceWithFaker(value, tableCol, params);
      if (!reversibleMap[tableCol]) reversibleMap[tableCol] = {};
      reversibleMap[tableCol][String(value)] = anon;
      return anon;
    }
    case 'scrub_to_subnet': {
      const anon = scrubToSubnet(value);
      if (!reversibleMap[tableCol]) reversibleMap[tableCol] = {};
      reversibleMap[tableCol][String(value)] = anon;
      return anon;
    }
    default:
      console.error(`anonymize: warn: unknown rule kind '${rule}' for ${tableCol} — leaving value unchanged`);
      return value;
  }
}

// ---------------------------------------------------------------------------
// CSV processing
// ---------------------------------------------------------------------------

/**
 * Parse a single CSV line respecting double-quoted fields.
 * Simple implementation sufficient for pg_dump CSV output.
 */
function parseCsvLine(line) {
  const fields = [];
  let current = '';
  let inQuotes = false;
  for (let i = 0; i < line.length; i++) {
    const ch = line[i];
    if (ch === '"') {
      if (inQuotes && line[i + 1] === '"') {
        current += '"';
        i++;
      } else {
        inQuotes = !inQuotes;
      }
    } else if (ch === ',' && !inQuotes) {
      fields.push(current);
      current = '';
    } else {
      current += ch;
    }
  }
  fields.push(current);
  return fields;
}

function formatCsvField(value) {
  if (value === null || value === undefined) return '';
  const s = String(value);
  if (s.includes(',') || s.includes('"') || s.includes('\n') || s.includes('\r')) {
    return '"' + s.replace(/"/g, '""') + '"';
  }
  return s;
}

function formatCsvLine(fields) {
  return fields.map(formatCsvField).join(',');
}

function removeIfPresent(path) {
  try {
    unlinkSync(path);
  } catch (err) {
    if (err.code !== 'ENOENT') throw err;
  }
}

// Windows refuses to rename a file over an existing path. Move the original
// aside first, then restore it if installing the anonymized output fails.
function replaceFilePortable(tmpPath, filePath) {
  const backupPath = `${filePath}.anonymize-backup`;
  // Preserve a backup when an interrupted replacement left the destination
  // missing; it may be the only remaining copy of the source data.
  if (existsSync(backupPath)) {
    if (existsSync(filePath)) removeIfPresent(backupPath);
    else renameSync(backupPath, filePath);
  }
  renameSync(filePath, backupPath);
  try {
    renameSync(tmpPath, filePath);
  } catch (err) {
    try {
      renameSync(backupPath, filePath);
    } catch (restoreErr) {
      err.message += `; could not restore original: ${restoreErr.message}`;
    }
    throw err;
  }
  removeIfPresent(backupPath);
}

/**
 * Process a CSV snapshot file for a given table's anonymize rules.
 * Reads <file>, writes <file>.anon, then replaces the original.
 */
async function processCsvFile(filePath, tableRule) {
  const columns = tableRule.columns || {};
  const colEntries = Object.entries(columns); // [[colName, ruleKind], ...]

  if (colEntries.length === 0) return;

  const tmpPath = filePath + '.anon';
  removeIfPresent(tmpPath);
  const input = createReadStream(filePath, { encoding: 'utf8' });
  const output = createWriteStream(tmpPath, { encoding: 'utf8' });
  const rl = createInterface({ input, crlfDelay: Infinity });

  let headers = [];
  let colIndexes = {}; // colName → index

  let lineCount = 0;
  let firstLine = true;
  for await (const line of rl) {
    if (firstLine) {
      firstLine = false;
      headers = parseCsvLine(line);
      for (const [colName] of colEntries) {
        const idx = headers.findIndex((h) => h.trim().toLowerCase() === colName.toLowerCase());
        if (idx === -1) {
          console.error(`anonymize: warn: column '${colName}' not found in ${filePath} headers — skipping`);
        } else {
          colIndexes[colName] = idx;
        }
      }
      output.write(line + '\n');
      continue;
    }

    if (line.trim() === '') {
      output.write('\n');
      continue;
    }

    const fields = parseCsvLine(line);
    lineCount++;
    for (const [colName, ruleKind] of colEntries) {
      const idx = colIndexes[colName];
      if (idx === undefined || idx >= fields.length) continue;
      const { kind, params } = normalizeRule(ruleKind);
      fields[idx] = await applyRule(kind, fields[idx], `${tableRule.table}.${colName}`, params);
    }
    output.write(formatCsvLine(fields) + '\n');
  }

  await new Promise((res, rej) => output.end((err) => (err ? rej(err) : res())));

  replaceFilePortable(tmpPath, filePath);

  console.log(`anonymize: processed ${lineCount} rows in ${filePath}`);
}

// ---------------------------------------------------------------------------
// SQL processing (pg_dump INSERT style)
// ---------------------------------------------------------------------------

/**
 * Very lightweight SQL INSERT parser — handles pg_dump INSERT INTO format:
 *   INSERT INTO "table" ("col1","col2",...) VALUES ('val1','val2',...);
 *
 * For full SQL parsing we'd need a real parser; this handles the common pg_dump
 * case. Falls back to pass-through on unrecognized lines.
 */
async function processSqlFile(filePath, rulesByTable) {
  const tmpPath = filePath + '.anon';
  removeIfPresent(tmpPath);
  const input = createReadStream(filePath, { encoding: 'utf8' });
  const output = createWriteStream(tmpPath, { encoding: 'utf8' });
  const rl = createInterface({ input, crlfDelay: Infinity });

  let rowCount = 0;

  // Regex to match: INSERT INTO "table" (...) VALUES (...);
  const insertRe = /^INSERT INTO\s+["']?(\w+)["']?\s*\(([^)]+)\)\s+VALUES\s*\((.+)\);?\s*$/i;

  for await (const line of rl) {
    const m = line.match(insertRe);
    if (!m) {
      output.write(line + '\n');
      continue;
    }

    const tableName = m[1];
    const tableRule = rulesByTable[tableName.toLowerCase()];
    if (!tableRule) {
      output.write(line + '\n');
      continue;
    }

    const cols = m[2].split(',').map((c) => c.trim().replace(/^["']|["']$/g, ''));
    const valStr = m[3];

    // Parse values (simplified — handles single-quoted strings, NULL, numbers)
    const values = parseSqlValues(valStr);
    if (!values) {
      output.write(line + '\n');
      continue;
    }

    const columns = tableRule.columns || {};
    for (const [colName, ruleKind] of Object.entries(columns)) {
      const idx = cols.findIndex((c) => c.toLowerCase() === colName.toLowerCase());
      if (idx === -1 || idx >= values.length) continue;
      const { kind, params } = normalizeRule(ruleKind);
      values[idx] = await applyRule(kind, values[idx], `${tableName}.${colName}`, params);
    }

    const newValStr = values.map(formatSqlValue).join(', ');
    const colList = cols.map((c) => `"${c}"`).join(', ');
    output.write(`INSERT INTO "${tableName}" (${colList}) VALUES (${newValStr});\n`);
    rowCount++;
  }

  await new Promise((res, rej) => output.end((err) => (err ? rej(err) : res())));

  replaceFilePortable(tmpPath, filePath);
  console.log(`anonymize: processed ${rowCount} INSERT rows in ${filePath}`);
}

function normalizeRule(rule) {
  if (typeof rule === 'object' && rule !== null) return { kind: rule.kind, params: rule.params };
  return { kind: rule, params: undefined };
}

/** Parse a SQL VALUES list into an array of JS values (null or string). */
function parseSqlValues(valStr) {
  const values = [];
  let i = 0;
  const s = valStr.trim();

  while (i < s.length) {
    // Skip whitespace
    while (i < s.length && /\s/.test(s[i])) i++;
    if (i >= s.length) break;

    if (s[i] === "'") {
      // Quoted string
      i++;
      let val = '';
      while (i < s.length) {
        if (s[i] === "'" && s[i + 1] === "'") {
          val += "'";
          i += 2;
        } else if (s[i] === "'") {
          i++;
          break;
        } else {
          val += s[i++];
        }
      }
      values.push(val);
    } else if (s.slice(i, i + 4).toUpperCase() === 'NULL') {
      values.push(null);
      i += 4;
    } else {
      // Number or bare word
      let val = '';
      while (i < s.length && s[i] !== ',' && s[i] !== ')') {
        val += s[i++];
      }
      values.push(val.trim());
    }

    // Skip comma
    while (i < s.length && /\s/.test(s[i])) i++;
    if (i < s.length && s[i] === ',') i++;
  }

  return values;
}

function formatSqlValue(value) {
  if (value === null || value === undefined) return 'NULL';
  const s = String(value);
  return "'" + s.replace(/'/g, "''") + "'";
}

// ---------------------------------------------------------------------------
// Main — discover snapshot files and apply rules
// ---------------------------------------------------------------------------

async function main() {
  recoverTemporaryOutputs(resolvedSnapshot);
  const rules = anonymizeConfig.rules;

  // Build lookup: tableName (lower) → rule
  const rulesByTable = {};
  for (const rule of rules) {
    rulesByTable[rule.table.toLowerCase()] = rule;
  }

  // Walk snapshot dir for CSV and SQL files
  function walkDir(dir) {
    const files = [];
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const full = join(dir, entry.name);
      if (entry.isDirectory()) {
        files.push(...walkDir(full));
      } else if (entry.isFile()) {
        files.push(full);
      }
    }
    return files;
  }

  const allFiles = walkDir(resolvedSnapshot);
  const csvFiles = allFiles.filter((f) => f.endsWith('.csv'));
  const sqlFiles = allFiles.filter((f) => f.endsWith('.sql'));

  // Process CSVs: infer table name from filename
  for (const csvFile of csvFiles) {
    const baseName = csvFile.split('/').pop().replace(/\.csv$/, '').toLowerCase();
    const tableRule = rulesByTable[baseName];
    if (!tableRule) continue;
    await processCsvFile(csvFile, tableRule);
  }

  // Process SQLs
  for (const sqlFile of sqlFiles) {
    await processSqlFile(sqlFile, rulesByTable);
  }

  // Save reversible map
  saveMap();
  console.log(`anonymize: reversible map saved to ${mapPath}`);
  console.log(`anonymize: done`);
}

function cleanupTemporaryOutputs(dir) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = join(dir, entry.name);
    if (entry.isDirectory()) cleanupTemporaryOutputs(full);
    else if (entry.name.endsWith('.anon')) removeIfPresent(full);
    else if (entry.name.endsWith('.anonymize-backup')) {
      const destination = full.slice(0, -'.anonymize-backup'.length);
      // Restore before cleanup when the backup is the only source copy.
      if (!existsSync(destination)) renameSync(full, destination);
      else removeIfPresent(full);
    }
  }
}

function recoverTemporaryOutputs(dir) {
  cleanupTemporaryOutputs(dir);
}

main().catch((err) => {
  try {
    cleanupTemporaryOutputs(resolvedSnapshot);
  } catch (cleanupErr) {
    err.message += `; temporary output cleanup failed: ${cleanupErr.message}`;
  }
  console.error(`anonymize: fatal: ${err.message}`);
  process.exit(1);
});
