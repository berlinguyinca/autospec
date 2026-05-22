#!/usr/bin/env node
// scan-doc-scope.mjs — Parse autospec-doc-scope declarations from markdown files.
//
// Exports:
//   parse(markdownPath: string): Array<ScopeEntry>
//
// ScopeEntry schema (per plan §2.1):
//   {
//     heading_path: string,          // e.g. "## Installing autospec-test"
//     src_globs: string[],           // from src: field
//     visual_glob?: string,          // from visual: field
//     generated?: boolean,           // from generated: true
//     reason?: string,               // from reason: field
//     byte_range: [number, number],  // byte offsets of section body in the file
//   }
//
// Syntax:
//   <!-- autospec-doc-scope:
//     src: ["glob1", "glob2"]
//     reason: "optional note"
//     visual: "docs/assets/screenshots/*.png"
//     generated: true
//   -->
//
// One block per section; section delimited by markdown headings at same-or-higher level.
// Malformed YAML inside the comment block → throws with a diagnostic message.

import fs from 'node:fs';
import path from 'node:path';

// ── YAML mini-parser ──────────────────────────────────────────────────────────
// Parses the simple subset used in autospec-doc-scope blocks.
// Supports: string scalars, boolean, string arrays (block or flow).

function parseYaml(text, sourcePath) {
    const result = {};
    const lines = text.split('\n');
    let i = 0;

    function trim(s) { return s.trim(); }
    function stripQuotes(s) {
        s = trim(s);
        if ((s.startsWith('"') && s.endsWith('"')) ||
            (s.startsWith("'") && s.endsWith("'"))) {
            return s.slice(1, -1);
        }
        return s;
    }

    while (i < lines.length) {
        const line = lines[i].replace(/^\s+/, ''); // strip leading indent
        const keyMatch = line.match(/^(\w[\w-]*)\s*:\s*(.*)/);
        if (!keyMatch) { i++; continue; }

        const key = keyMatch[1];
        const rest = trim(keyMatch[2]);

        if (rest === '' || rest === '|' || rest === '>') {
            // Could be block sequence start or empty value — look ahead
            i++;
            const items = [];
            while (i < lines.length && lines[i].match(/^\s+-\s+/)) {
                items.push(stripQuotes(lines[i].replace(/^\s+-\s+/, '')));
                i++;
            }
            result[key] = items.length > 0 ? items : '';
        } else if (rest.startsWith('[')) {
            // Flow sequence: ["a", "b"] or ['a', 'b']
            const seqMatch = rest.match(/^\[(.*)\]$/);
            if (!seqMatch) {
                throw new Error(
                    `scan-doc-scope: malformed flow sequence for key '${key}' in ${sourcePath}: ${rest}`
                );
            }
            const items = seqMatch[1].split(',').map(s => stripQuotes(s)).filter(Boolean);
            result[key] = items;
            i++;
        } else if (rest === 'true' || rest === 'yes') {
            result[key] = true;
            i++;
        } else if (rest === 'false' || rest === 'no') {
            result[key] = false;
            i++;
        } else {
            result[key] = stripQuotes(rest);
            i++;
        }
    }

    return result;
}

// ── Heading helpers ───────────────────────────────────────────────────────────

function headingLevel(line) {
    const m = line.match(/^(#{1,6})\s/);
    return m ? m[1].length : 0;
}

// ── Public API ────────────────────────────────────────────────────────────────

const SCOPE_OPEN_RE  = /<!--\s*autospec-doc-scope\s*:/;
const SCOPE_CLOSE_RE = /-->/;

/**
 * Parse all autospec-doc-scope blocks from a markdown file.
 *
 * @param {string} markdownPath - path to the markdown file
 * @returns {Array<ScopeEntry>}
 * @throws if a scope block contains malformed YAML
 */
export function parse(markdownPath) {
    if (!markdownPath || typeof markdownPath !== 'string') {
        throw new Error('scan-doc-scope: markdownPath must be a non-empty string');
    }

    let content;
    try {
        content = fs.readFileSync(markdownPath, 'utf8');
    } catch (err) {
        throw new Error(`scan-doc-scope: cannot read file: ${markdownPath}: ${err.message}`);
    }

    const lines = content.split('\n');
    const results = [];

    // Track headings and sections
    let currentHeading = null;
    let currentHeadingLevel = 0;
    let i = 0;

    while (i < lines.length) {
        const line = lines[i];
        const lvl = headingLevel(line);

        if (lvl > 0) {
            currentHeading = line.trim();
            currentHeadingLevel = lvl;
            i++;
            continue;
        }

        // Detect scope comment start
        if (SCOPE_OPEN_RE.test(line)) {
            // Collect the YAML content between the comment markers
            const commentLines = [];
            // The opening line may have content after the "autospec-doc-scope:"
            const openMatch = line.match(/<!--\s*autospec-doc-scope\s*:(.*)/s);
            let rest = openMatch ? openMatch[1] : '';

            // Check if close is on the same line
            let closed = false;
            const closeIdx = rest.indexOf('-->');
            if (closeIdx !== -1) {
                commentLines.push(rest.slice(0, closeIdx));
                closed = true;
                i++;
            } else {
                commentLines.push(rest);
                i++;
                while (i < lines.length && !closed) {
                    const inner = lines[i];
                    const endIdx = inner.indexOf('-->');
                    if (endIdx !== -1) {
                        commentLines.push(inner.slice(0, endIdx));
                        closed = true;
                    } else {
                        commentLines.push(inner);
                    }
                    i++;
                }
            }

            if (!closed) {
                throw new Error(
                    `scan-doc-scope: unclosed autospec-doc-scope block in ${markdownPath} near line ${i}`
                );
            }

            // Parse YAML
            const yamlText = commentLines.join('\n').trim();
            let parsed;
            try {
                parsed = parseYaml(yamlText, markdownPath);
            } catch (err) {
                throw new Error(`scan-doc-scope: ${err.message}`);
            }

            // Validate src field
            const srcRaw = parsed.src;
            let src_globs;
            if (!srcRaw) {
                src_globs = [];
            } else if (Array.isArray(srcRaw)) {
                src_globs = srcRaw.map(String);
            } else if (typeof srcRaw === 'string') {
                src_globs = [srcRaw];
            } else {
                throw new Error(
                    `scan-doc-scope: 'src' field must be an array or string in ${markdownPath}`
                );
            }

            // Find the byte range of the section body: from after this comment to next same-or-higher heading
            const bodyStartLine = i; // first line after the comment close
            let bodyEndLine = lines.length;
            if (currentHeadingLevel > 0) {
                for (let j = bodyStartLine; j < lines.length; j++) {
                    const nextLvl = headingLevel(lines[j]);
                    if (nextLvl > 0 && nextLvl <= currentHeadingLevel) {
                        bodyEndLine = j;
                        break;
                    }
                }
            }

            // Compute byte offsets
            const lineOffsets = computeLineOffsets(content);
            const byteStart = bodyStartLine < lineOffsets.length ? lineOffsets[bodyStartLine] : content.length;
            const byteEnd = bodyEndLine < lineOffsets.length ? lineOffsets[bodyEndLine] : content.length;

            const entry = {
                heading_path: currentHeading || '(root)',
                src_globs,
                byte_range: [byteStart, byteEnd],
            };
            if (parsed.visual) entry.visual_glob = String(parsed.visual);
            if (parsed.generated === true) entry.generated = true;
            if (parsed.reason) entry.reason = String(parsed.reason);

            results.push(entry);
            continue;
        }

        i++;
    }

    return results;
}

/** Compute byte offset of the start of each line. */
function computeLineOffsets(content) {
    const offsets = [0];
    for (let i = 0; i < content.length; i++) {
        if (content[i] === '\n') {
            offsets.push(i + 1);
        }
    }
    return offsets;
}

// ── CLI ───────────────────────────────────────────────────────────────────────

if (process.argv[1] && (
    process.argv[1].endsWith('scan-doc-scope.mjs') ||
    process.argv[1].endsWith('scan-doc-scope')
)) {
    const filePath = process.argv[2];
    if (!filePath) {
        process.stderr.write('Usage: scan-doc-scope.mjs <markdown-file>\n');
        process.exit(1);
    }
    try {
        const entries = parse(filePath);
        process.stdout.write(JSON.stringify(entries, null, 2) + '\n');
    } catch (err) {
        process.stderr.write(`${err.message}\n`);
        process.exit(2);
    }
}
