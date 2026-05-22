#!/usr/bin/env node
// gen-llm-manifest.mjs — generate docs/.llm-manifest.json from cluster output.
//
// Exports:
//   generateManifest({ clusters, repo, repoRoot, existingManifest }): object
//   writeManifest({ clusters, repo, repoRoot, outputPath }): Promise<{ written: boolean, path: string }>
//
// Output shape matches spec §5b and validates against schemas/llm-manifest.schema.json.
//
// Idempotent: skips write if byte-equal to existing file.

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { execSync } from 'node:child_process';

const __dirname = path.dirname(fileURLToPath(import.meta.url));

/**
 * Get short git HEAD sha.
 */
function getCommit(repoRoot) {
  try {
    return execSync('git rev-parse --short HEAD', {
      cwd: repoRoot, encoding: 'utf8', stdio: ['pipe','pipe','pipe']
    }).trim();
  } catch { return 'unknown'; }
}

/**
 * Detect repo slug from git remote or fallback to dirname.
 */
function detectRepo(repoRoot) {
  try {
    const remote = execSync('git remote get-url origin', {
      cwd: repoRoot, encoding: 'utf8', stdio: ['pipe','pipe','pipe']
    }).trim();
    // Parse owner/repo from https://github.com/owner/repo or git@github.com:owner/repo
    const m = remote.match(/[:/]([^/]+\/[^/]+?)(?:\.git)?$/);
    if (m) return m[1];
  } catch {}
  return path.basename(repoRoot);
}

/**
 * Build manifest modules array from cluster significant units.
 */
function buildModules(significant, repoRoot) {
  return (significant || []).map(unit => {
    const relPath = path.relative(repoRoot, unit.files[0]);
    const publicApi = (unit.exports || []).map(exp => ({
      name: exp.name,
      signature: exp.signature || '',
      params: [],
      returns: '',
      usage_example: '',
    }));
    const dependsOn = (unit.importedBy || []).map(f => path.relative(repoRoot, f));
    return {
      path: relPath,
      summary: `${unit.slug} — ${unit.language} module with ${publicApi.length} export(s). Significance: ${(unit.reasons || []).join(', ')}.`,
      public_api: publicApi,
      depends_on: dependsOn,
    };
  });
}

/**
 * Build cli_entry_points from cluster significant units.
 */
function buildCliEntryPoints(significant, repoRoot) {
  const entries = [];
  for (const unit of (significant || [])) {
    for (const ep of (unit.entry_points || [])) {
      if (ep.kind === 'cli_command') {
        entries.push({
          identifier: ep.identifier,
          path: path.relative(repoRoot, unit.files[0]),
          line: ep.line,
        });
      }
    }
  }
  return entries;
}

/**
 * Build http_endpoints from cluster significant units.
 */
function buildHttpEndpoints(significant, repoRoot) {
  const entries = [];
  for (const unit of (significant || [])) {
    for (const ep of (unit.entry_points || [])) {
      if (ep.kind === 'http_route') {
        entries.push({
          route: ep.identifier,
          path: path.relative(repoRoot, unit.files[0]),
          line: ep.line,
        });
      }
    }
  }
  return entries;
}

/**
 * Scan docs for <!-- autospec-concept: <name> --> markers.
 * Returns array of { name, definition, source_anchor }.
 */
function scanConcepts(repoRoot) {
  const docsDir = path.join(repoRoot, 'docs');
  const concepts = [];
  if (!fs.existsSync(docsDir)) return concepts;

  const scan = (dir) => {
    let entries;
    try { entries = fs.readdirSync(dir, { withFileTypes: true }); } catch { return; }
    for (const ent of entries) {
      const full = path.join(dir, ent.name);
      if (ent.isDirectory()) { scan(full); continue; }
      if (!ent.name.endsWith('.md')) continue;
      try {
        const lines = fs.readFileSync(full, 'utf8').split('\n');
        const relPath = path.relative(repoRoot, full);
        lines.forEach((line, idx) => {
          const m = line.match(/<!--\s*autospec-concept:\s*(.+?)\s*-->/);
          if (m) {
            concepts.push({
              name: m[1].trim(),
              definition: lines[idx + 1]?.trim() || '',
              source_anchor: `${relPath}#L${idx + 1}`,
            });
          }
        });
      } catch {}
    }
  };
  scan(docsDir);
  return concepts;
}

/**
 * Generate the manifest object from cluster output.
 *
 * @param {{
 *   clusters: { significant: ClusterUnit[], trivial: any[] },
 *   repo?: string,
 *   repoRoot: string,
 * }} opts
 * @returns {object}
 */
export function generateManifest({ clusters, repo, repoRoot }) {
  const significant = clusters?.significant || [];
  const commit = getCommit(repoRoot);
  const repoSlug = repo || detectRepo(repoRoot);

  return {
    schema_version: '1.0',
    repo: repoSlug,
    generated_at: new Date().toISOString(),
    commit,
    modules: buildModules(significant, repoRoot),
    cli_entry_points: buildCliEntryPoints(significant, repoRoot),
    http_endpoints: buildHttpEndpoints(significant, repoRoot),
    concepts: scanConcepts(repoRoot),
    faq: [],
  };
}

/**
 * Write manifest to outputPath (idempotent — skips if byte-equal).
 *
 * @param {{
 *   clusters: object,
 *   repo?: string,
 *   repoRoot: string,
 *   outputPath: string,
 * }} opts
 * @returns {Promise<{ written: boolean, path: string, manifest: object }>}
 */
export async function writeManifest({ clusters, repo, repoRoot, outputPath }) {
  const manifest = generateManifest({ clusters, repo, repoRoot });
  const content = JSON.stringify(manifest, null, 2) + '\n';

  fs.mkdirSync(path.dirname(outputPath), { recursive: true });

  let existing = null;
  try { existing = fs.readFileSync(outputPath, 'utf8'); } catch {}

  // Idempotency: compare body sans generated_at (timestamp changes on each run).
  // Strip generated_at for comparison.
  const normalize = (s) => s.replace(/"generated_at":\s*"[^"]*"/, '"generated_at": "<normalized>"');
  const written = !existing || normalize(existing) !== normalize(content);
  if (written) fs.writeFileSync(outputPath, content, 'utf8');

  return { written, path: outputPath, manifest };
}

// ── CLI ───────────────────────────────────────────────────────────────────────

function realResolve(p) {
  try { return fs.realpathSync(path.resolve(p)); } catch { return path.resolve(p); }
}
const isMain = process.argv[1] &&
  realResolve(process.argv[1]) === realResolve(fileURLToPath(import.meta.url));

if (isMain) {
  const args = process.argv.slice(2);
  let repoRoot = process.cwd();
  let outputPath = null;
  for (let i = 0; i < args.length; i++) {
    if (args[i] === '--repo-root' && args[i+1]) repoRoot = args[++i];
    if (args[i] === '--output' && args[i+1]) outputPath = args[++i];
  }
  outputPath = outputPath || path.join(repoRoot, 'docs', '.llm-manifest.json');

  // Read cluster JSON from stdin or generate empty
  let clusters = { significant: [], trivial: [] };
  if (!process.stdin.isTTY) {
    let input = '';
    process.stdin.on('data', d => { input += d; });
    await new Promise(res => process.stdin.on('end', res));
    if (input.trim()) {
      try { clusters = JSON.parse(input); } catch (e) {
        process.stderr.write('gen-llm-manifest: invalid JSON on stdin: ' + e.message + '\n');
        process.exit(1);
      }
    }
  }

  const result = await writeManifest({ clusters, repoRoot, outputPath });
  process.stderr.write(`[gen-llm-manifest] ${result.written ? 'written' : 'unchanged'}: ${result.path}\n`);
  console.log(JSON.stringify(result.manifest, null, 2));
}
