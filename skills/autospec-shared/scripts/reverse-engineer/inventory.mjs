#!/usr/bin/env node
// inventory.mjs — walk a repo root, respect .gitignore, emit source file list.
//
// Exports:
//   inventory(repoRoot: string, opts?: InventoryOptions): Promise<InventoryEntry[]>
//
// InventoryEntry:
//   { filePath: string, language: string, relPath: string }
//
// Options:
//   { skipDirs?: string[] }  — additional dirs to skip (from .autospec/init.yml)

import fs from 'node:fs';
import path from 'node:path';
import { execSync } from 'node:child_process';

const EXT_TO_LANG = {
  '.ts':   'typescript',
  '.tsx':  'typescript',
  '.mts':  'typescript',
  '.js':   'javascript',
  '.mjs':  'javascript',
  '.cjs':  'javascript',
  '.jsx':  'javascript',
  '.py':   'python',
  '.go':   'go',
  '.rs':   'rust',
  '.java': 'java',
};

// Dirs always skipped regardless of init.yml
const ALWAYS_SKIP = new Set([
  'docs', 'vendor', 'node_modules', '.git', 'generated',
  '.turbo', 'dist', 'build', 'out', '__pycache__', '.venv',
  'coverage', '.nyc_output', 'target',
]);

/**
 * Load skip_dirs from .autospec/init.yml (YAML parsed minimally — just the skip_dirs array).
 * Returns an empty array if the file is absent or the key is missing.
 * @param {string} repoRoot
 * @returns {string[]}
 */
function loadSkipDirs(repoRoot) {
  const initPath = path.join(repoRoot, '.autospec', 'init.yml');
  if (!fs.existsSync(initPath)) return [];
  try {
    const content = fs.readFileSync(initPath, 'utf8');
    // Minimal YAML: find 'skip_dirs:' block and parse list items
    const lines = content.split('\n');
    let inBlock = false;
    const dirs = [];
    for (const line of lines) {
      if (/^skip_dirs\s*:/.test(line)) { inBlock = true; continue; }
      if (inBlock) {
        const m = line.match(/^\s+-\s+(.+)/);
        if (m) {
          dirs.push(m[1].trim().replace(/^['"]|['"]$/g, ''));
        } else if (line.match(/^\S/) && !line.match(/^\s*#/)) {
          // new top-level key — end of block
          break;
        }
      }
    }
    return dirs;
  } catch {
    return [];
  }
}

/**
 * Try to get the list of tracked files from git (respects .gitignore automatically).
 * Falls back to filesystem walk if git is unavailable.
 * @param {string} repoRoot
 * @returns {string[] | null}  absolute paths, or null on failure
 */
function gitTrackedFiles(repoRoot) {
  try {
    // Use git ls-files to list all tracked and untracked-but-not-ignored files
    const out = execSync('git ls-files --cached --others --exclude-standard', {
      cwd: repoRoot,
      encoding: 'utf8',
      stdio: ['pipe', 'pipe', 'pipe'],
    });
    return out.split('\n')
      .map(l => l.trim())
      .filter(Boolean)
      .map(rel => path.resolve(repoRoot, rel));
  } catch {
    return null;
  }
}

/**
 * Filesystem walk fallback — does NOT respect .gitignore, but skips ALWAYS_SKIP + extraSkip.
 * @param {string} dir
 * @param {Set<string>} skipSet
 * @param {string[]} acc
 */
function walkFs(dir, skipSet, acc) {
  let entries;
  try { entries = fs.readdirSync(dir, { withFileTypes: true }); } catch { return; }
  for (const ent of entries) {
    if (ent.name.startsWith('.') && ent.name !== '.autospec') continue;
    if (skipSet.has(ent.name)) continue;
    const full = path.join(dir, ent.name);
    if (ent.isDirectory()) {
      walkFs(full, skipSet, acc);
    } else if (ent.isFile()) {
      acc.push(full);
    }
  }
}

/**
 * Enumerate source files in a repo root.
 *
 * @param {string} repoRoot
 * @param {{ skipDirs?: string[] }} [opts]
 * @returns {Promise<Array<{ filePath: string, language: string, relPath: string }>>}
 */
export async function inventory(repoRoot, opts = {}) {
  const absRoot = path.resolve(repoRoot);
  const extraSkip = new Set([...(opts.skipDirs || []), ...loadSkipDirs(absRoot)]);
  const skipSet = new Set([...ALWAYS_SKIP, ...extraSkip]);

  let candidates;
  const gitFiles = gitTrackedFiles(absRoot);
  if (gitFiles !== null) {
    candidates = gitFiles;
  } else {
    const acc = [];
    walkFs(absRoot, skipSet, acc);
    candidates = acc;
  }

  const result = [];
  for (const filePath of candidates) {
    const relPath = path.relative(absRoot, filePath);

    // Skip files inside always-skip or extra-skip directories
    const parts = relPath.split(path.sep);
    if (parts.some(p => skipSet.has(p))) continue;

    const ext = path.extname(filePath).toLowerCase();
    const language = EXT_TO_LANG[ext];
    if (!language) continue;

    result.push({ filePath, language, relPath });
  }

  return result;
}

// ── CLI usage ─────────────────────────────────────────────────────────────────

if (process.argv[1] && path.resolve(process.argv[1]) === path.resolve(import.meta.url.replace('file://', ''))) {
  const repoRoot = process.argv[2] || process.cwd();
  inventory(repoRoot).then(entries => {
    process.stdout.write(`${JSON.stringify(entries, null, 2)}\n`);
  }).catch(err => {
    process.stderr.write(`${err}\n`);
    process.exit(1);
  });
}
