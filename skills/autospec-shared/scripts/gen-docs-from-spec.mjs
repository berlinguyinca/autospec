#!/usr/bin/env node
// gen-docs-from-spec.mjs — orchestrate doc generation from cluster output.
//
// Exports:
//   generateDocs({ clusters, specs, existingDocs, outputDir }): Promise<GenDocsResult>
//
// GenDocsResult:
//   {
//     files: Array<{ path: string, written: boolean, preserved_sections: number }>,
//   }
//
// Human-edit preservation rule:
//   Sections in existingDocs that do NOT have `generated: true` in their scope block
//   are preserved verbatim. Only `generated: true` sections are overwritten.

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const GEN_DOCS_DIR = path.join(__dirname, 'gen-docs');

// ── Generator imports ─────────────────────────────────────────────────────────

const { generate: generateUserManual }   = await import(path.join(GEN_DOCS_DIR, 'user-manual.mjs'));
const { generate: generateApiReference } = await import(path.join(GEN_DOCS_DIR, 'api-reference.mjs'));
const { generate: generateArchitecture } = await import(path.join(GEN_DOCS_DIR, 'architecture.mjs'));

const GENERATORS = [
  generateUserManual,
  generateApiReference,
  generateArchitecture,
];

// ── Section parsing ───────────────────────────────────────────────────────────

/**
 * Parse a markdown file into sections keyed by heading text.
 * Each section includes the heading line, any scope block, and body content.
 *
 * @param {string} content
 * @returns {Array<{ heading: string, level: number, raw: string, generated: boolean }>}
 */
function parseSections(content) {
  const lines = content.split('\n');
  const sections = [];
  let currentSection = null;
  let inScopeBlock = false;
  let isGenerated = false;
  let sectionLines = [];

  const flush = () => {
    if (currentSection !== null) {
      sections.push({
        heading: currentSection.heading,
        level: currentSection.level,
        raw: sectionLines.join('\n'),
        generated: isGenerated,
      });
    }
  };

  for (const line of lines) {
    const headingMatch = line.match(/^(#{1,6})\s+(.+)/);

    if (headingMatch) {
      flush();
      currentSection = { heading: headingMatch[2], level: headingMatch[1].length };
      sectionLines = [line];
      inScopeBlock = false;
      isGenerated = false;
      continue;
    }

    if (line.includes('<!-- autospec-doc-scope:')) {
      inScopeBlock = true;
    }
    if (inScopeBlock && line.includes('generated: true')) {
      isGenerated = true;
    }
    if (inScopeBlock && line.includes('-->')) {
      inScopeBlock = false;
    }

    if (currentSection !== null) {
      sectionLines.push(line);
    }
  }
  flush();

  // Handle content before first heading (preamble)
  const firstHeadingIdx = lines.findIndex(l => /^#{1,6}\s+/.test(l));
  if (firstHeadingIdx > 0) {
    sections.unshift({
      heading: '__preamble__',
      level: 0,
      raw: lines.slice(0, firstHeadingIdx).join('\n'),
      generated: false,
    });
  }

  return sections;
}

/**
 * Merge generated content with existing doc, preserving human-edited sections.
 *
 * Strategy:
 * 1. Parse existing doc into sections.
 * 2. Parse new doc into sections.
 * 3. For each section in new doc:
 *    - If an existing section with the same heading exists AND it's NOT generated: keep existing.
 *    - Otherwise: use new content.
 *
 * @param {string} newContent
 * @param {string} existingContent
 * @returns {{ merged: string, preserved: number }}
 */
function mergeWithExisting(newContent, existingContent) {
  if (!existingContent) return { merged: newContent, preserved: 0 };

  const existingSections = parseSections(existingContent);
  const newSections = parseSections(newContent);

  const existingByHeading = new Map(
    existingSections.map(s => [s.heading, s])
  );

  let preserved = 0;
  const mergedParts = [];

  for (const newSec of newSections) {
    const existing = existingByHeading.get(newSec.heading);
    if (existing && !existing.generated) {
      // Preserve human-edited section verbatim
      mergedParts.push(existing.raw);
      preserved++;
    } else {
      mergedParts.push(newSec.raw);
    }
  }

  return { merged: mergedParts.join('\n'), preserved };
}

// ── Public API ────────────────────────────────────────────────────────────────

/**
 * Generate all three docs from cluster output, preserving human edits.
 *
 * @param {{
 *   clusters: { significant: ClusterUnit[], trivial: any[] },
 *   specs?: object[],
 *   existingDocs?: { [relPath: string]: string },
 *   outputDir?: string,
 * }} opts
 * @returns {Promise<{ files: Array<{ path: string, written: boolean, preserved_sections: number }> }>}
 */
export async function generateDocs({ clusters, specs = [], existingDocs = {}, outputDir = null }) {
  const results = [];

  for (const generator of GENERATORS) {
    const { path: relPath, content: newContent, section_anchors } = generator({
      clusters, specs, existingDocs,
    });

    const existing = existingDocs[relPath] || existingDocs[path.basename(relPath)] || null;
    const { merged, preserved } = mergeWithExisting(newContent, existing);

    let written = false;
    if (outputDir) {
      const outPath = path.join(outputDir, relPath);
      fs.mkdirSync(path.dirname(outPath), { recursive: true });
      // Only write if content changed
      let currentContent = null;
      try { currentContent = fs.readFileSync(outPath, 'utf8'); } catch {}
      if (currentContent !== merged) {
        fs.writeFileSync(outPath, merged, 'utf8');
        written = true;
      }
    }

    results.push({ path: relPath, content: merged, written, preserved_sections: preserved, section_anchors });
  }

  return { files: results };
}

// ── CLI usage ─────────────────────────────────────────────────────────────────

// Detect CLI invocation: process.argv[1] matches this file.
// Use realpathSync to resolve /tmp → /private/tmp symlinks on macOS.
function realResolve(p) {
  try { return fs.realpathSync(path.resolve(p)); } catch { return path.resolve(p); }
}
const isMain = process.argv[1] &&
  realResolve(process.argv[1]) === realResolve(fileURLToPath(import.meta.url));

if (isMain) {
  const args = process.argv.slice(2);
  let fixtureFile = null;
  let outputDir = null;

  for (let i = 0; i < args.length; i++) {
    if (args[i] === '--fixture' && args[i + 1]) { fixtureFile = args[++i]; }
    if (args[i] === '--output-dir' && args[i + 1]) { outputDir = args[++i]; }
  }

  if (!fixtureFile) {
    // Read cluster JSON from stdin
    let input = '';
    process.stdin.on('data', d => { input += d; });
    process.stdin.on('end', async () => {
      const clusters = JSON.parse(input);
      const result = await generateDocs({ clusters, outputDir });
      console.log(JSON.stringify(result, null, 2));
    });
  } else {
    const clusters = JSON.parse(fs.readFileSync(fixtureFile, 'utf8'));
    const result = await generateDocs({ clusters, outputDir });
    console.log(JSON.stringify(result, null, 2));
  }
}
