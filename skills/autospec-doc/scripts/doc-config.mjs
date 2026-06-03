#!/usr/bin/env node
// doc-config.mjs — config loader for /autospec-doc (issue #917)
//
// Parses the `documentation:` block from .autospec/autospec.yml, applies
// defaults for the new optional `style` and `examples` keys, and seeds the
// four default audiences (user/developer/admin/general) when `audiences:` is
// absent or empty.
//
// Existing `audiences:` entries (with any key shape — name/id/label/path/focus/
// require_scope) are preserved verbatim; this module never mutates or replaces
// them. The issue's shared-contracts block is the single source of truth for
// default audience strings; any change there must propagate here.
//
// Exports:
//   loadConfig(configPath) → { audiences, style, examples }
//   DEFAULT_AUDIENCES — the four canonical defaults (array, read-only)
//   FOLDER_CONTRACT   — the approved folder-contract constants (object, read-only)

import fs from 'node:fs';

// ── Default audiences (spec §D2; shared-contracts block pinned 2026-06-03) ────

export const DEFAULT_AUDIENCES = Object.freeze([
  Object.freeze({
    name: 'user',
    path: 'docs/user',
    focus: 'tasks, workflows, how to use features',
    require_scope: true,
  }),
  Object.freeze({
    name: 'developer',
    path: 'docs/developer',
    focus: 'architecture, APIs, extending',
    require_scope: true,
  }),
  Object.freeze({
    name: 'admin',
    path: 'docs/admin',
    focus: 'install, configure, operate, troubleshoot',
    require_scope: true,
  }),
  Object.freeze({
    name: 'general',
    path: 'docs/general',
    focus: 'what it is, why it matters, plain language',
    require_scope: true,
  }),
]);

// ── Folder contract (spec §D2) ────────────────────────────────────────────────

export const FOLDER_CONTRACT = Object.freeze({
  baseFiles: Object.freeze(['index.md', 'getting-started.md']),
  tutorialPattern: 'tutorials/<feature>.md',
  featurePattern:  'features/<feature>.md',
  developerExtras: Object.freeze(['architecture/', 'api/']),
  adminRunbooksLink: 'docs/runbooks/',
  sharedAssets: Object.freeze([
    'docs/assets/screenshots',
    'docs/assets/diagrams',
    'docs/assets/transcripts',
  ]),
});

// ── Minimal YAML subset parser ────────────────────────────────────────────────
//
// Parses the subset of YAML used in .autospec/autospec.yml documentation blocks:
// - nested mappings (key: value or key:\n  subkey: value)
// - block sequences (  - key: value\n    key2: value2)
// - scalar values: strings (bare, single/double quoted), booleans, integers, null
//
// No external dependency — keeps the script self-contained.

// Split a flow-mapping/sequence body on top-level commas, respecting single and
// double quotes (so a quoted value containing a comma is not split).
function splitTopLevel(body) {
  const parts = [];
  let cur = '';
  let quote = null;
  for (let i = 0; i < body.length; i++) {
    const ch = body[i];
    if (quote) {
      cur += ch;
      if (ch === quote) quote = null;
    } else if (ch === '"' || ch === "'") {
      quote = ch;
      cur += ch;
    } else if (ch === ',') {
      parts.push(cur);
      cur = '';
    } else {
      cur += ch;
    }
  }
  if (cur.trim() !== '') parts.push(cur);
  return parts;
}

// Parse a flow mapping body (the text between { and }) into a plain object.
function parseFlowMapping(body) {
  const obj = {};
  for (const pair of splitTopLevel(body)) {
    const colonIdx = pair.indexOf(':');
    if (colonIdx === -1) continue;
    const k = pair.slice(0, colonIdx).trim();
    const v = pair.slice(colonIdx + 1).trim();
    if (k) obj[k] = parseYamlValue(v);
  }
  return obj;
}

function parseYamlValue(raw) {
  const s = raw.trim();
  if (s === '' || s === '~' || s === 'null') return null;
  if (s === 'true')  return true;
  if (s === 'false') return false;
  if (s.startsWith('"') && s.endsWith('"')) return s.slice(1, -1);
  if (s.startsWith("'") && s.endsWith("'")) return s.slice(1, -1);
  if (/^-?\d+$/.test(s)) return parseInt(s, 10);
  return s;
}

/**
 * parseYamlDoc — parse a YAML document (or sub-document) from an array of
 * lines.  Returns a plain JS object.
 *
 * Algorithm: recursive descent using a shared line-index pointer.
 * parseMapping(minIndent) reads key:value pairs while indentation >= minIndent.
 * parseSequence(indent) reads list items at exactly `indent`.
 */
function parseYamlDoc(lines) {
  // Filter out comment and blank lines, attach indent.
  const tokens = lines
    .map((line, i) => ({ line, i, indent: line.match(/^( *)/)[1].length, content: line.trim() }))
    .filter(t => t.content !== '' && !t.content.startsWith('#'));

  let pos = 0; // index into tokens

  function peek() { return pos < tokens.length ? tokens[pos] : null; }
  function consume() { return tokens[pos++]; }

  function parseValue(content, childIndent) {
    // Inline sequence: "[]" or "[ ... ]"
    if (content === '[]') return [];
    if (content.startsWith('[') && content.endsWith(']')) {
      const inner = content.slice(1, -1).trim();
      if (!inner) return [];
      return splitTopLevel(inner).map(s => parseYamlValue(s.trim()));
    }
    if (content === '' || content === null) {
      // Value is on subsequent lines — peek to see if it's a mapping or sequence.
      const next = peek();
      if (!next || next.indent < childIndent) return null;
      if (next.content.startsWith('- ')) {
        return parseSequence(next.indent);
      }
      return parseMapping(next.indent);
    }
    return parseYamlValue(content);
  }

  function parseMapping(minIndent) {
    const obj = {};
    while (true) {
      const t = peek();
      if (!t || t.indent < minIndent) break;
      // A sequence item at this level means we were called from the wrong place.
      if (t.content.startsWith('- ')) break;
      consume();
      const colonIdx = t.content.indexOf(':');
      if (colonIdx === -1) continue; // malformed line — skip
      const k = t.content.slice(0, colonIdx).trim();
      const rest = t.content.slice(colonIdx + 1).trim();
      obj[k] = parseValue(rest, t.indent + 1);
    }
    return obj;
  }

  function parseSequence(seqIndent) {
    const arr = [];
    while (true) {
      const t = peek();
      if (!t || t.indent !== seqIndent || !t.content.startsWith('- ')) break;
      consume();
      const itemContent = t.content.slice(2).trim();
      // Inline flow mapping item: "- {name: user, path: docs/user, focus: \"...\"}"
      if (itemContent.startsWith('{') && itemContent.endsWith('}')) {
        arr.push(parseFlowMapping(itemContent.slice(1, -1)));
        continue;
      }
      // Inline mapping item: "- key: value"
      if (/^[A-Za-z_][A-Za-z0-9_-]*\s*:/.test(itemContent)) {
        const itemObj = {};
        // Parse the first k:v pair from this line.
        const colonIdx = itemContent.indexOf(':');
        const k = itemContent.slice(0, colonIdx).trim();
        const rest = itemContent.slice(colonIdx + 1).trim();
        itemObj[k] = parseValue(rest, t.indent + 1);
        // Collect subsequent indented keys for this item.
        while (true) {
          const next = peek();
          if (!next || next.indent <= seqIndent || next.content.startsWith('- ')) break;
          consume();
          const ci = next.content.indexOf(':');
          if (ci === -1) continue;
          const ik = next.content.slice(0, ci).trim();
          const iv = next.content.slice(ci + 1).trim();
          itemObj[ik] = parseValue(iv, next.indent + 1);
        }
        arr.push(itemObj);
      } else if (itemContent === '') {
        // Multi-line item — next lines at deeper indent form a mapping.
        const next = peek();
        if (next && next.indent > seqIndent) {
          arr.push(parseMapping(next.indent));
        } else {
          arr.push(null);
        }
      } else {
        arr.push(parseYamlValue(itemContent));
      }
    }
    return arr;
  }

  return parseMapping(0);
}

// extractDocumentationBlock — return the lines of the `documentation:` sub-doc.
function extractDocumentationBlock(text) {
  const lines = text.split('\n');
  let inDoc = false;
  let baseIndent = -1;
  const block = [];

  for (const line of lines) {
    if (!inDoc) {
      if (/^documentation:\s*$/.test(line) || /^documentation:\s*\S/.test(line)) {
        inDoc = true;
        const inline = line.replace(/^documentation:\s*/, '');
        if (inline.trim()) block.push(inline);
      }
      continue;
    }
    // A non-empty line at column 0 that isn't a comment ends the block.
    if (/^\S/.test(line) && !/^\s*#/.test(line)) break;

    if (baseIndent === -1 && line.trim() !== '') {
      baseIndent = line.match(/^( *)/)[1].length;
    }
    block.push(baseIndent > 0 ? line.slice(baseIndent) : line);
  }
  return block;
}

// ── loadConfig ─────────────────────────────────────────────────────────────────

/**
 * loadConfig(configPath) → { audiences, style, examples }
 *
 * Reads the `documentation:` block from .autospec/autospec.yml at `configPath`
 * and returns a normalised config object with defaults applied.
 *
 * Rules:
 *  - audiences: existing entries are preserved verbatim. Default audiences are
 *    injected ONLY when the list is absent or empty.
 *  - style.palette: defaults to 'light-blue'.
 *  - examples.verify: defaults to true.
 *  - examples.sandbox: defaults to 'worktree'.
 *
 * @param {string} configPath  Absolute or CWD-relative path to autospec.yml.
 * @returns {{ audiences: object[], style: { palette: string }, examples: { verify: boolean, sandbox: string } }}
 */
export function loadConfig(configPath) {
  let raw = '';
  try {
    raw = fs.readFileSync(configPath, 'utf8');
  } catch {
    return {
      audiences: DEFAULT_AUDIENCES.map(a => ({ ...a })),
      style:    { palette: 'light-blue' },
      examples: { verify: true, sandbox: 'worktree' },
    };
  }

  const blockLines = extractDocumentationBlock(raw);
  const doc = parseYamlDoc(blockLines);

  // ── audiences ──────────────────────────────────────────────────────────────
  let audiences;
  if (Array.isArray(doc.audiences) && doc.audiences.length > 0) {
    audiences = doc.audiences;
  } else {
    audiences = DEFAULT_AUDIENCES.map(a => ({ ...a }));
  }

  // ── style ─────────────────────────────────────────────────────────────────
  const styleRaw = (typeof doc.style === 'object' && doc.style !== null && !Array.isArray(doc.style))
    ? doc.style : {};
  const style = {
    palette: (styleRaw.palette != null) ? String(styleRaw.palette) : 'light-blue',
  };

  // ── examples ──────────────────────────────────────────────────────────────
  const exRaw = (typeof doc.examples === 'object' && doc.examples !== null && !Array.isArray(doc.examples))
    ? doc.examples : {};
  const examples = {
    verify:  (exRaw.verify  != null) ? Boolean(exRaw.verify)  : true,
    sandbox: (exRaw.sandbox != null) ? String(exRaw.sandbox)  : 'worktree',
  };

  return { audiences, style, examples };
}
