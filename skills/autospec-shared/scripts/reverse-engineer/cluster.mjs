#!/usr/bin/env node
// cluster.mjs — group walker output into "significant units" per spec §4b step 3.
//
// Exports:
//   cluster(walkerOutputs: WalkOutput[]): ClusterResult
//
// ClusterResult:
//   {
//     significant: Array<ClusterUnit>,
//     trivial:     Array<{ filePath: string, bubbledInto: string | null }>,
//   }
//
// ClusterUnit:
//   {
//     slug:       string,           // kebab-case module identifier
//     files:      string[],         // absolute file paths
//     language:   string,           // dominant language
//     exports:    WalkExport[],
//     entry_points: WalkEntry[],
//     importedBy: string[],         // files that import any file in this unit
//     reasons:    string[],         // why it's significant: 'has_exports'|'cli_entry'|'imported_by_3+'
//   }
//
// Significance rule (spec §4b step 3):
//   ≥1 public export  OR  CLI/HTTP entry point  OR  imported by ≥3 other files

import path from 'node:path';

/**
 * Build a reverse-import index: filePath → set of files that import it.
 * We match on the import source string (relative or basename) to the inventory filePath.
 *
 * @param {WalkOutput[]} outputs
 * @returns {Map<string, Set<string>>}
 */
function buildImportedByIndex(outputs) {
  // Map from absolute path → set of importers
  const index = new Map();
  for (const out of outputs) {
    if (!index.has(out.file_path)) index.set(out.file_path, new Set());
  }

  for (const out of outputs) {
    for (const imp of (out.imports || [])) {
      // Find matching output by matching the import source string against known file paths
      const matched = resolveImport(imp.source, out.file_path, outputs);
      if (matched) {
        if (!index.has(matched)) index.set(matched, new Set());
        index.get(matched).add(out.file_path);
      }
    }
  }
  return index;
}

/**
 * Attempt to resolve an import source string to an absolute file path in the outputs list.
 * Handles:
 *   - Relative imports: './utils', '../lib/config'
 *   - Bare basename match: 'utils' → matches any output whose basename is 'utils.*'
 *
 * @param {string} source       import source string
 * @param {string} importerPath absolute path of the importing file
 * @param {WalkOutput[]} outputs
 * @returns {string|null} matched absolute file path or null
 */
function resolveImport(source, importerPath, outputs) {
  if (!source) return null;

  // Skip node built-ins and npm packages (no leading dot and no known file extension)
  if (!source.startsWith('.') && !source.startsWith('/')) {
    // Could be a bare package — skip
    return null;
  }

  const importerDir = path.dirname(importerPath);
  const resolved = path.resolve(importerDir, source);

  // Try exact match first
  for (const out of outputs) {
    if (out.file_path === resolved) return out.file_path;
  }

  // Try with known extensions appended
  const EXTS = ['.mjs', '.js', '.ts', '.tsx', '.py', '.go', '.rs', '.java'];
  for (const ext of EXTS) {
    const candidate = resolved + ext;
    for (const out of outputs) {
      if (out.file_path === candidate) return out.file_path;
    }
  }

  // Try index file (e.g. ./foo → ./foo/index.mjs)
  for (const ext of EXTS) {
    const candidate = path.join(resolved, `index${ext}`);
    for (const out of outputs) {
      if (out.file_path === candidate) return out.file_path;
    }
  }

  return null;
}

/**
 * Convert an absolute file path to a kebab-case slug suitable for spec filenames.
 * Uses the relative path from the nearest recognizable root segment.
 *
 * @param {string} filePath absolute path
 * @param {string[]} allFiles all absolute paths (for context)
 * @returns {string}
 */
function toSlug(filePath) {
  const base = path.basename(filePath, path.extname(filePath));
  const dir = path.basename(path.dirname(filePath));
  const raw = dir === '.' || dir === '' ? base : `${dir}-${base}`;
  return raw
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-|-$/g, '');
}

/**
 * Determine the dominant language in a set of file paths given walker outputs.
 * @param {string[]} files
 * @param {Map<string, WalkOutput>} byPath
 * @returns {string}
 */
function dominantLanguage(files, byPath) {
  const counts = new Map();
  for (const f of files) {
    const lang = byPath.get(f)?.language || 'unknown';
    counts.set(lang, (counts.get(lang) || 0) + 1);
  }
  let best = 'unknown', bestN = 0;
  for (const [lang, n] of counts) {
    if (n > bestN) { best = lang; bestN = n; }
  }
  return best;
}

/**
 * Cluster walker outputs into significant and trivial units.
 *
 * @param {import('../tree-sitter-walk/walker.mjs').WalkOutput[]} walkerOutputs
 * @returns {{ significant: ClusterUnit[], trivial: Array<{filePath:string,bubbledInto:string|null}> }}
 */
export function cluster(walkerOutputs) {
  if (!Array.isArray(walkerOutputs) || walkerOutputs.length === 0) {
    return { significant: [], trivial: [] };
  }

  const byPath = new Map(walkerOutputs.map(o => [o.file_path, o]));
  const importedBy = buildImportedByIndex(walkerOutputs);

  const significant = [];
  const trivial = [];

  for (const out of walkerOutputs) {
    const hasExports = Array.isArray(out.exports) && out.exports.length > 0;
    const hasCLIEntry = Array.isArray(out.entry_points) && out.entry_points.length > 0;
    const importedByCount = importedBy.get(out.file_path)?.size || 0;
    const importedByMany = importedByCount >= 3;

    const reasons = [];
    if (hasExports) reasons.push('has_exports');
    if (hasCLIEntry) reasons.push('cli_entry');
    if (importedByMany) reasons.push('imported_by_3+');

    if (reasons.length > 0) {
      significant.push({
        slug: toSlug(out.file_path),
        files: [out.file_path],
        language: out.language,
        exports: out.exports || [],
        entry_points: out.entry_points || [],
        importedBy: [...(importedBy.get(out.file_path) || new Set())],
        reasons,
      });
    } else {
      // Trivial leaf — find parent (the significant unit that imports this file most directly)
      let bubbledInto = null;
      for (const sig of walkerOutputs) {
        if (sig.file_path === out.file_path) continue;
        const sigImports = (sig.imports || []).map(i =>
          resolveImport(i.source, sig.file_path, walkerOutputs)
        );
        if (sigImports.includes(out.file_path)) {
          // Check if the importer is significant
          const isSignificant = significant.some(s => s.files.includes(sig.file_path));
          if (isSignificant) {
            bubbledInto = sig.file_path;
            // Merge exports into the significant unit
            const sigUnit = significant.find(s => s.files.includes(sig.file_path));
            if (sigUnit) {
              sigUnit.files.push(out.file_path);
              for (const exp of (out.exports || [])) {
                if (!sigUnit.exports.some(e => e.name === exp.name)) {
                  sigUnit.exports.push(exp);
                }
              }
            }
            break;
          }
        }
      }
      trivial.push({ filePath: out.file_path, bubbledInto });
    }
  }

  return { significant, trivial };
}

// ── CLI usage ─────────────────────────────────────────────────────────────────

if (process.argv[1] && path.resolve(process.argv[1]) === path.resolve(import.meta.url.replace('file://', ''))) {
  let input = '';
  process.stdin.on('data', d => { input += d; });
  process.stdin.on('end', () => {
    try {
      const walkerOutputs = JSON.parse(input);
      const result = cluster(walkerOutputs);
      console.log(JSON.stringify(result, null, 2));
    } catch (err) {
      console.error('cluster: failed to parse input:', err.message);
      process.exit(1);
    }
  });
}
