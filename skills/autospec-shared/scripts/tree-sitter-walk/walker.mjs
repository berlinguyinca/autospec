#!/usr/bin/env node
// walker.mjs — tree-sitter walker for autospec-docs amendment pipeline.
//
// Exports:
//   walk(filePath: string): Promise<WalkOutput>
//
// WalkOutput schema (per plan §1.2):
//   {
//     language: string,           // 'typescript' | 'javascript' | 'python' | 'go' | 'rust' | 'java' | 'unknown'
//     exports: Array<{
//       name: string,
//       kind: 'function' | 'class' | 'type' | 'const',
//       signature: string,
//       line: number,
//     }>,
//     entry_points: Array<{
//       kind: 'cli_command' | 'http_route',
//       identifier: string,
//       line: number,
//     }>,
//     imports: Array<{
//       source: string,
//       names: string[],
//     }>,
//     file_path: string,          // absolute path
//   }
//
// Malformed / unsupported input returns { language: 'unknown', exports: [], entry_points: [], imports: [], file_path }

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { createRequire } from 'node:module';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const QUERIES_DIR = path.join(__dirname, 'queries');

// ── Language detection ────────────────────────────────────────────────────────

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

const LANG_TO_GRAMMAR_PKG = {
    typescript:  'tree-sitter-typescript',
    javascript:  'tree-sitter-javascript',
    python:      'tree-sitter-python',
    go:          'tree-sitter-go',
    rust:        'tree-sitter-rust',
    java:        'tree-sitter-java',
};

function detectLanguage(filePath) {
    const ext = path.extname(filePath).toLowerCase();
    return EXT_TO_LANG[ext] || 'unknown';
}

// ── Parser cache ──────────────────────────────────────────────────────────────

let Parser = null;
const grammarCache = new Map();
const queryCache   = new Map();

async function loadParser() {
    if (Parser) return Parser;
    const { default: TreeSitter } = await import('web-tree-sitter');
    await TreeSitter.init();
    Parser = TreeSitter;
    return Parser;
}

async function loadGrammar(lang) {
    if (grammarCache.has(lang)) return grammarCache.get(lang);
    const TS = await loadParser();
    const pkg = LANG_TO_GRAMMAR_PKG[lang];
    if (!pkg) return null;

    // web-tree-sitter grammars ship as .wasm files in their npm packages.
    // Resolve the .wasm path via node module resolution.
    const require = createRequire(import.meta.url);
    let wasmPath;
    try {
        // Try common locations: pkg root or /grammar.wasm, /tree-sitter-<lang>.wasm
        const pkgDir = path.dirname(require.resolve(`${pkg}/package.json`));
        const candidates = [
            path.join(pkgDir, `tree-sitter-${lang}.wasm`),
            path.join(pkgDir, 'grammar.wasm'),
            path.join(pkgDir, `tree-sitter-${lang === 'typescript' ? 'typescript' : lang}.wasm`),
        ];
        // For typescript: also try tree-sitter-tsx.wasm
        if (lang === 'typescript') {
            candidates.unshift(path.join(pkgDir, 'tree-sitter-typescript.wasm'));
        }
        wasmPath = candidates.find(p => fs.existsSync(p));
        if (!wasmPath) {
            // Fall back: search the package dir for any .wasm
            const wasmFiles = fs.readdirSync(pkgDir).filter(f => f.endsWith('.wasm'));
            if (wasmFiles.length > 0) wasmPath = path.join(pkgDir, wasmFiles[0]);
        }
    } catch {
        return null;
    }
    if (!wasmPath) return null;

    try {
        const grammar = await TS.Language.load(wasmPath);
        grammarCache.set(lang, grammar);
        return grammar;
    } catch {
        return null;
    }
}

function loadQuery(lang, grammar) {
    if (queryCache.has(lang)) return queryCache.get(lang);
    const qFile = path.join(QUERIES_DIR, `${lang}.scm`);
    if (!fs.existsSync(qFile)) return null;
    const qSrc = fs.readFileSync(qFile, 'utf8');
    try {
        const q = grammar.query(qSrc);
        queryCache.set(lang, q);
        return q;
    } catch {
        // Query compilation failure — degrade gracefully
        return null;
    }
}

// ── Result builders ───────────────────────────────────────────────────────────

/**
 * Build WalkOutput from tree-sitter query matches.
 * The .scm queries use capture names like @export.name, @export.kind (metadata),
 * @entry.kind, @import.source, @import.name.
 */
function buildOutput(filePath, lang, matches, source) {
    const exportMap  = new Map(); // name → WalkExport
    const entryMap   = new Map(); // identifier → WalkEntry
    const importMap  = new Map(); // source → Set<name>

    for (const { pattern, captures } of matches) {
        const get = (name) => captures.find(c => c.name === name);
        const getAll = (name) => captures.filter(c => c.name === name);
        const nodeText = (c) => c ? source.slice(c.node.startIndex, c.node.endIndex).replace(/['"]/g, '') : '';
        const nodeLine = (c) => c ? c.node.startPosition.row + 1 : 0;

        // ── Exports ────────────────────────────────────────────────────────
        const expName = get('export.name');
        if (expName) {
            const name = nodeText(expName);
            if (name && !exportMap.has(name)) {
                // Determine kind from set! metadata or defaults
                const kindMeta = captures.find(c => c.name === 'export.kind');
                let kind = kindMeta ? kindMeta.name.replace('export.', '') : 'const';
                // Heuristic from pattern text metadata stored as set! values
                // Since web-tree-sitter doesn't expose set! values directly, detect from pattern
                const paramsCapture = get('export.params');
                const declCapture = get('export.decl');
                if (!kind || kind === 'export.kind') {
                    if (paramsCapture) kind = 'function';
                    else if (name.match(/^[A-Z]/) && lang !== 'go') kind = 'class';
                    else kind = 'const';
                }
                // Build signature from source line
                const line = nodeLine(expName);
                const lines = source.split('\n');
                const sigLine = lines[line - 1] || '';
                const signature = sigLine.trim().replace(/\s+/g, ' ').slice(0, 120);
                exportMap.set(name, { name, kind, signature, line });
            }
        }

        // ── Entry points ───────────────────────────────────────────────────
        const entryShebang = get('entry.shebang');
        if (entryShebang) {
            const identifier = path.basename(filePath);
            if (!entryMap.has(identifier)) {
                entryMap.set(identifier, { kind: 'cli_command', identifier, line: 1 });
            }
        }
        const entryMain = get('entry.main');
        if (entryMain) {
            const identifier = nodeText(entryMain);
            if (!entryMap.has(identifier)) {
                entryMap.set(identifier, { kind: 'cli_command', identifier, line: nodeLine(entryMain) });
            }
        }
        const entryRoute = get('entry.route');
        if (entryRoute) {
            const route = nodeText(entryRoute);
            const key = `route:${route}`;
            if (!entryMap.has(key)) {
                entryMap.set(key, { kind: 'http_route', identifier: route, line: nodeLine(entryRoute) });
            }
        }

        // ── Imports ────────────────────────────────────────────────────────
        const importSource = get('import.source');
        if (importSource) {
            const src = nodeText(importSource);
            if (src) {
                if (!importMap.has(src)) importMap.set(src, new Set());
                const importNames = getAll('import.name');
                for (const imp of importNames) {
                    const n = nodeText(imp);
                    if (n) importMap.get(src).add(n);
                }
            }
        }
    }

    return {
        language: lang,
        exports: [...exportMap.values()],
        entry_points: [...entryMap.values()],
        imports: [...importMap.entries()].map(([source, names]) => ({
            source,
            names: [...names],
        })),
        file_path: path.resolve(filePath),
    };
}

// ── Public API ────────────────────────────────────────────────────────────────

const EMPTY_OUTPUT = (filePath) => ({
    language: 'unknown',
    exports: [],
    entry_points: [],
    imports: [],
    file_path: path.resolve(filePath),
});

/**
 * Walk a source file and extract exports, entry points, and imports.
 *
 * @param {string} filePath - absolute or relative path to the source file
 * @returns {Promise<WalkOutput>}
 */
export async function walk(filePath) {
    // Malformed input guard
    if (!filePath || typeof filePath !== 'string') return EMPTY_OUTPUT(filePath || '');

    let source;
    try {
        source = fs.readFileSync(filePath, 'utf8');
    } catch {
        return EMPTY_OUTPUT(filePath);
    }

    const lang = detectLanguage(filePath);
    if (lang === 'unknown') return EMPTY_OUTPUT(filePath);

    let TS;
    try {
        TS = await loadParser();
    } catch {
        return EMPTY_OUTPUT(filePath);
    }

    const grammar = await loadGrammar(lang);
    if (!grammar) return EMPTY_OUTPUT(filePath);

    const parser = new TS.Parser();
    parser.setLanguage(grammar);

    let tree;
    try {
        tree = parser.parse(source);
    } catch {
        return EMPTY_OUTPUT(filePath);
    }

    const query = loadQuery(lang, grammar);
    if (!query) return EMPTY_OUTPUT(filePath);

    let matches;
    try {
        matches = query.matches(tree.rootNode);
    } catch {
        return EMPTY_OUTPUT(filePath);
    }

    return buildOutput(filePath, lang, matches, source);
}
