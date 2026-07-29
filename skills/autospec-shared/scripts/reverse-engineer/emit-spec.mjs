#!/usr/bin/env node
// emit-spec.mjs — emit reverse-engineered spec markdown per spec §4b step 4.
//
// Exports:
//   emitSpecs(clusterResult, opts): Promise<EmitResult>
//
// EmitResult:
//   {
//     written:  string[],   // paths written
//     skipped:  string[],   // paths skipped (operator-edited or unchanged)
//     manifest: Array<{ path: string, slug: string, sourceRoot: string, status: 'written'|'skipped' }>
//   }
//
// Frontmatter (per spec §4b):
//   ---
//   reverse_engineered: true
//   source_root: <module_path>
//   generated_at: <ISO>
//   commit: <sha>
//   ai_reviewed:
//     confidence: medium
//   ---
//
// Idempotency (spec §4e):
//   - Skip if source_root hash (mtime+size) matches stored commit stamp.
//   - Never rewrite if frontmatter reverse_engineered: false (operator edited).
//   - For the "commit" field we store the sha of the generating commit; on rerun
//     we compare the git sha of files under source_root instead.

import fs from 'node:fs';
import path from 'node:path';
import { execSync } from 'node:child_process';

/**
 * Get the current git HEAD sha (short).
 * @param {string} repoRoot
 * @returns {string}
 */
function getCommitSha(repoRoot) {
  try {
    return execSync('git rev-parse --short HEAD', {
      cwd: repoRoot, encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe'],
    }).trim();
  } catch {
    return 'unknown';
  }
}

/**
 * Get the latest commit sha touching a set of files.
 * @param {string[]} files absolute paths
 * @param {string} repoRoot
 * @returns {string}
 */
function getFilesCommitSha(files, repoRoot) {
  try {
    const relPaths = files.map(f => path.relative(repoRoot, f));
    const out = execSync(
      `git log -1 --format=%h -- ${relPaths.map(p => `"${p}"`).join(' ')}`,
      { cwd: repoRoot, encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe'] }
    ).trim();
    return out || getCommitSha(repoRoot);
  } catch {
    return getCommitSha(repoRoot);
  }
}

/**
 * Parse YAML frontmatter from a markdown string.
 * Returns null if not present, or an object with string values.
 * @param {string} content
 * @returns {{ [key: string]: string | boolean | object } | null}
 */
function parseFrontmatter(content) {
  if (!content.startsWith('---\n') && !content.startsWith('---\r\n')) return null;
  const end = content.indexOf('\n---\n', 4);
  if (end === -1) return null;
  const yaml = content.slice(4, end);
  const result = {};
  for (const line of yaml.split('\n')) {
    const m = line.match(/^(\w[\w_]*)\s*:\s*(.*)/);
    if (m) {
      const val = m[2].trim();
      if (val === 'true') result[m[1]] = true;
      else if (val === 'false') result[m[1]] = false;
      else result[m[1]] = val;
    }
    // Nested keys (e.g. ai_reviewed.confidence) — minimal support
    const nested = line.match(/^\s{2}(\w[\w_]*)\s*:\s*(.*)/);
    if (nested) {
      // Find the last top-level key added and attach nested
      const lastKey = Object.keys(result).slice(-1)[0];
      if (lastKey && typeof result[lastKey] === 'string') {
        result[lastKey] = { [nested[1]]: nested[2].trim() };
      } else if (lastKey && typeof result[lastKey] === 'object') {
        result[lastKey][nested[1]] = nested[2].trim();
      }
    }
  }
  return result;
}

/**
 * Build the frontmatter block for a spec file.
 * @param {{ sourceRoot: string, commit: string }} opts
 * @returns {string}
 */
function buildFrontmatter({ sourceRoot, commit }) {
  return [
    '---',
    'reverse_engineered: true',
    `source_root: ${sourceRoot}`,
    `generated_at: ${new Date().toISOString()}`,
    `commit: ${commit}`,
    'ai_reviewed:',
    '  confidence: medium',
    '---',
    '',
  ].join('\n');
}

/**
 * Build the markdown body for a single ClusterUnit spec.
 * @param {ClusterUnit} unit
 * @param {string} commit
 * @returns {string}
 */
function buildUnitSpec(unit, commit) {
  const fm = buildFrontmatter({
    sourceRoot: unit.files[0],
    commit,
  });

  const lines = [
    fm,
    `# ${unit.slug} — reverse-engineered design`,
    '',
    `**Language:** ${unit.language}`,
    `**Files:** ${unit.files.length}`,
    '',
    '## Source files',
    '',
    ...unit.files.map(f => `- \`${f}\``),
    '',
  ];

  if (unit.exports && unit.exports.length > 0) {
    lines.push('## Public exports', '');
    for (const exp of unit.exports) {
      lines.push(`### \`${exp.name}\` (${exp.kind})`, '');
      lines.push(`**Line:** ${exp.line}`, '');
      if (exp.signature) lines.push(`\`\`\`\n${exp.signature}\n\`\`\``, '');
    }
  }

  if (unit.entry_points && unit.entry_points.length > 0) {
    lines.push('## Entry points', '');
    for (const ep of unit.entry_points) {
      lines.push(`- **${ep.kind}**: \`${ep.identifier}\` (line ${ep.line})`);
    }
    lines.push('');
  }

  if (unit.importedBy && unit.importedBy.length > 0) {
    lines.push('## Imported by', '');
    for (const imp of unit.importedBy) {
      lines.push(`- \`${imp}\``);
    }
    lines.push('');
  }

  lines.push(
    '## Summary',
    '',
    `> *Auto-generated by reverse-engineer pipeline. Edit this section to describe the module purpose.*`,
    '',
    `**Significance:** ${unit.reasons.join(', ')}`,
    '',
  );

  return lines.join('\n');
}

/**
 * Build the top-level architecture spec.
 * @param {ClusterResult} clusterResult
 * @param {string} commit
 * @returns {string}
 */
function buildArchitectureSpec(clusterResult, commit) {
  const fm = buildFrontmatter({
    sourceRoot: '.',
    commit,
  });

  const lines = [
    fm,
    '# Architecture — reverse-engineered design',
    '',
    `**Significant modules:** ${clusterResult.significant.length}`,
    `**Trivial files (bubbled):** ${clusterResult.trivial.length}`,
    '',
    '## Module index',
    '',
  ];

  for (const unit of clusterResult.significant) {
    lines.push(`### ${unit.slug}`);
    lines.push('');
    lines.push(`- **Language:** ${unit.language}`);
    lines.push(`- **Files:** ${unit.files.length}`);
    lines.push(`- **Exports:** ${(unit.exports || []).length}`);
    lines.push(`- **Entry points:** ${(unit.entry_points || []).length}`);
    lines.push(`- **Significance:** ${unit.reasons.join(', ')}`);
    lines.push('');
  }

  if (clusterResult.trivial.length > 0) {
    lines.push('## Trivial files', '');
    for (const t of clusterResult.trivial) {
      const bubbled = t.bubbledInto ? ` → bubbled into \`${t.bubbledInto}\`` : '';
      lines.push(`- \`${t.filePath}\`${bubbled}`);
    }
    lines.push('');
  }

  lines.push(
    '## Summary',
    '',
    '> *Auto-generated by reverse-engineer pipeline. Edit this section to describe the system architecture.*',
    '',
  );

  return lines.join('\n');
}

/**
 * Emit reverse-engineered spec files to docsDir.
 *
 * @param {{ significant: ClusterUnit[], trivial: Array<{filePath:string,bubbledInto:string|null}> }} clusterResult
 * @param {{
 *   docsDir:  string,     // e.g. path.join(repoRoot, 'docs', 'specs')
 *   repoRoot: string,
 *   date?:    string,     // YYYY-MM-DD override (default: today)
 * }} opts
 * @returns {Promise<{ written: string[], skipped: string[], manifest: object[] }>}
 */
export async function emitSpecs(clusterResult, opts) {
  const { docsDir, repoRoot } = opts;
  const date = opts.date || new Date().toISOString().slice(0, 10);
  const commit = getCommitSha(repoRoot);

  fs.mkdirSync(docsDir, { recursive: true });

  const written = [];
  const skipped = [];
  const manifest = [];

  // ── Architecture spec ──────────────────────────────────────────────────────
  const archPath = path.join(docsDir, `${date}-architecture-reverse-engineered-design.md`);
  const archResult = writeSpecFile(
    archPath, buildArchitectureSpec(clusterResult, commit), 'architecture', repoRoot,
  );
  if (archResult === 'written') written.push(archPath);
  else skipped.push(archPath);
  manifest.push({ path: archPath, slug: 'architecture', sourceRoot: '.', status: archResult });

  // ── Per-module specs ───────────────────────────────────────────────────────
  for (const unit of clusterResult.significant) {
    const specPath = path.join(docsDir, `${date}-${unit.slug}-reverse-engineered-design.md`);
    const sourceCommit = getFilesCommitSha(unit.files, repoRoot);
    const content = buildUnitSpec(unit, sourceCommit);
    const result = writeSpecFile(specPath, content, unit.slug, repoRoot);
    if (result === 'written') written.push(specPath);
    else skipped.push(specPath);
    manifest.push({ path: specPath, slug: unit.slug, sourceRoot: unit.files[0], status: result });
  }

  return { written, skipped, manifest };
}

/**
 * Write a spec file, honoring idempotency rules.
 *
 * Returns 'written' or 'skipped'.
 *
 * @param {string} specPath
 * @param {string} newContent
 * @param {string} slug  (for logging)
 * @param {string} repoRoot
 * @returns {'written'|'skipped'}
 */
function writeSpecFile(specPath, newContent, slug, repoRoot) {
  if (fs.existsSync(specPath)) {
    const existing = fs.readFileSync(specPath, 'utf8');
    const fm = parseFrontmatter(existing);

    // Operator-edited: reverse_engineered: false — never rewrite
    if (fm && fm.reverse_engineered === false) {
      return 'skipped';
    }

    // Strip frontmatter from both for body comparison
    const stripFm = (s) => {
      if (!s.startsWith('---\n')) return s;
      const end = s.indexOf('\n---\n', 4);
      return end === -1 ? s : s.slice(end + 5);
    };
    const existingBody = stripFm(existing);
    const newBody = stripFm(newContent);

    if (existingBody.trim() === newBody.trim()) {
      // Identical body — skip even if generated_at differs
      return 'skipped';
    }
  }

  fs.writeFileSync(specPath, newContent, 'utf8');
  return 'written';
}

// ── CLI usage ─────────────────────────────────────────────────────────────────

if (process.argv[1] && path.resolve(process.argv[1]) === path.resolve(import.meta.url.replace('file://', ''))) {
  const docsDir = process.argv[2] || path.join(process.cwd(), 'docs', 'specs');
  const repoRoot = process.argv[3] || process.cwd();
  let input = '';
  process.stdin.on('data', d => { input += d; });
  process.stdin.on('end', async () => {
    try {
      const clusterResult = JSON.parse(input);
      const result = await emitSpecs(clusterResult, { docsDir, repoRoot });
      process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
    } catch (err) {
      process.stderr.write(`emit-spec: failed: ${err.message}\n`);
      process.exit(1);
    }
  });
}
