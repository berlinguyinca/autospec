#!/usr/bin/env node
// assertion-shift-v2-buckets.mjs — v2 assertion-shift classifier extension.
//
// Given before/after YAML strings for .autospec/test.yml, diffs the
// invariants_v2.* namespace and buckets each change as:
//   LOOSENING      — weakens coverage (remove entry, narrow routes, lower count,
//                    hard_fail→warn_only, lower bfs_max_routes, remove affordance)
//   STRENGTHENING  — tightens coverage (add entry, widen routes, raise thresholds)
//   SHIFTING       — neutral change (selector change, path_template change)
//
// Per spec §5c table. Returns Array<Verdict> with the same shape as
// v1 assertion-shift-classifier.mjs verdicts.
//
// Integration: loaded by assertion-shift-classifier.mjs as a second pass when
// the diff touches .autospec/test.yml (the v2 namespace).
//
// Export: classifyV2ContractDiff(options) async function
// CLI: node assertion-shift-v2-buckets.mjs --before <file> --after <file>

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __filename = fileURLToPath(import.meta.url);

/**
 * @typedef {Object} Verdict
 * @property {string} file       - always '.autospec/test.yml' for v2 verdicts
 * @property {number} line       - approximate line number (0 if unknown)
 * @property {'LOOSENING'|'SHIFTING'|'STRENGTHENING'} bucket
 * @property {string} framework  - always 'autospec-v2-contract'
 * @property {string} detail     - human-readable description of the change
 */

/**
 * Parse a flat key-value map from a YAML string, extracting all
 * invariants_v2.* paths. Returns a Map of dotPath → value.
 *
 * We use a lightweight structural approach (no full YAML parser dependency)
 * sufficient for the v2 namespace diffing.
 */
function parseInvariantsV2Paths(yamlText) {
    const paths = new Map();
    if (!yamlText) return paths;

    // Extract the invariants_v2 block by finding its indented extent
    const lines = yamlText.split('\n');
    let inInvariantsV2 = false;
    let baseIndent = -1;

    // Track array items by building a path stack
    const pathStack = [];
    let arrayCounters = new Map(); // path → index

    for (let i = 0; i < lines.length; i++) {
        const line = lines[i];
        const trimmed = line.trimStart();
        const indent = line.length - trimmed.length;

        if (!inInvariantsV2) {
            if (/^\s*invariants_v2\s*:/.test(line)) {
                inInvariantsV2 = true;
                baseIndent = indent;
            }
            continue;
        }

        // Stop when we return to base indent or less with a new key
        if (indent <= baseIndent && !/^\s*$/.test(line) && !/^\s*#/.test(line)) {
            // Check if it's a sibling key at the same level — stop
            if (indent < baseIndent || (indent === baseIndent && !/^\s*invariants_v2/.test(line))) {
                break;
            }
        }

        // Skip comments and blank lines
        if (/^\s*#/.test(line) || /^\s*$/.test(line)) continue;

        // Parse key: value or - key: value
        const listItemMatch = trimmed.match(/^-\s+(\w[\w_-]*):\s*(.*)/);
        const keyValMatch = trimmed.match(/^(\w[\w_-]*):\s*(.*)/);
        const listStartMatch = trimmed.match(/^-\s*$/);

        if (listItemMatch || keyValMatch) {
            const key = listItemMatch ? listItemMatch[1] : keyValMatch[1];
            const val = listItemMatch ? listItemMatch[2] : keyValMatch[2];
            // Simple: record key=val pairs within the invariants_v2 block
            paths.set(`invariants_v2.${key}`, val.trim());
        }
    }

    return paths;
}

/**
 * Extract structured data from invariants_v2 block using regex-based parsing.
 * Returns an object with arrays for each metric type.
 */
function extractInvariantsV2Struct(yamlText) {
    if (!yamlText) {
        return { enabled: false, invariants: [], window_contracts: [], affordance_patterns: [],
                 crawler: null, contract_symmetry: [], bfs_max_routes: null,
                 require_count_at_least: {}, mismatch_actions: {}, apply_on_routes: {} };
    }

    const result = {
        enabled: /invariants_v2:\s*\n[\s\S]*?enabled:\s*true/.test(yamlText),
        invariants: [],
        window_contracts: [],
        affordance_patterns: [],
        crawler: null,
        contract_symmetry: [],
        bfs_max_routes: null,
        require_count_at_least: {},
        mismatch_actions: {},
        apply_on_routes: {},
    };

    // Extract a top-level section block from invariants_v2: by key name.
    // Stops at the next sibling key at the same indent level (4 spaces under invariants_v2).
    // This avoids the common regex pitfall of stopping at child keys.
    function extractV2Section(key) {
        // Find `    key:` (4 spaces = under invariants_v2 which is at 4 spaces in e2e block)
        const pat = new RegExp(`^([ \\t]{0,12})${key}:\\s*\\n`, 'm');
        const startMatch = pat.exec(yamlText);
        if (!startMatch) return '';
        const indent = startMatch[1];
        const startIdx = startMatch.index + startMatch[0].length;
        // Stop at next line with same or less indent + word char + colon (a sibling key)
        const stopPat = new RegExp(`\\n${indent}\\w[\\w_-]*:`, 'g');
        stopPat.lastIndex = startIdx;
        const stopMatch = stopPat.exec(yamlText);
        return stopMatch
            ? yamlText.slice(startIdx, stopMatch.index)
            : yamlText.slice(startIdx);
    }

    // Parse list entries from a block. Returns array of objects {name, body}
    // where body is the text following the `- name:` line until the next `- name:`.
    function parseListEntries(block) {
        const entries = [];
        // Split on `- name:` or `- id:` boundary lines
        const entryPat = /^[ \t]*-[ \t]+(?:name|id):[ \t]*(\S+)/gm;
        let match;
        while ((match = entryPat.exec(block)) !== null) {
            const name = match[1];
            const bodyStart = match.index + match[0].length;
            // Find next entry start
            entryPat.lastIndex = bodyStart;
            const nextMatch = entryPat.exec(block);
            entryPat.lastIndex = nextMatch ? nextMatch.index : bodyStart;
            const body = nextMatch
                ? block.slice(bodyStart, nextMatch.index)
                : block.slice(bodyStart);
            entries.push({ name, body });
            if (!nextMatch) break;
            // Reset to continue from after the entry we just found
            entryPat.lastIndex = nextMatch.index;
        }
        return entries;
    }

    // Extract structural_invariants / invariants entries (support both `name:` and `id:`)
    // Also handles partial diffs where context starts at the list entries directly.
    const invBlock = extractV2Section('structural_invariants') || extractV2Section('invariants')
        || (/^\s*-\s+(?:name|id):/.test(yamlText) && !/structural_invariants:|window_contracts:|contract_symmetry:|affordance_patterns:/.test(yamlText) ? yamlText : '');
    if (invBlock) {
        const entries = parseListEntries(invBlock);
        for (const { name, body } of entries) {
            result.invariants.push(name);

            const countMatch = body.match(/require_count_at_least:\s*(\d+)/);
            if (countMatch) result.require_count_at_least[name] = parseInt(countMatch[1], 10);

            const actionMatch = body.match(/mismatch_action:\s*(\S+)/);
            if (actionMatch) result.mismatch_actions[name] = actionMatch[1];

            // apply_on_routes: inline array or block sequence
            const inlineRoutes = body.match(/apply_on_routes:\s*\[([^\]]+)\]/);
            if (inlineRoutes) {
                result.apply_on_routes[name] = inlineRoutes[1].split(',').map(s => s.trim().replace(/['"]/g, ''));
            } else {
                const routesBlock = body.match(/apply_on_routes:\s*\n([\s\S]*?)(?=\n[ \t]*\w[\w_-]*:|$)/);
                if (routesBlock) {
                    const items = [];
                    for (const line of routesBlock[1].split('\n')) {
                        const trimmed = line.replace(/^\s*-\s*/, '').trim();
                        if (trimmed) items.push(trimmed);
                    }
                    if (items.length > 0) result.apply_on_routes[name] = items;
                }
            }
        }
    }

    // Extract window_contracts entries (support `name:` and `id:`)
    const winBlock = extractV2Section('window_contracts');
    if (winBlock) {
        const entries = parseListEntries(winBlock);
        for (const { name, body } of entries) {
            result.window_contracts.push(name);
            const actionMatch = body.match(/mismatch_action:\s*(\S+)/);
            if (actionMatch) result.mismatch_actions[name] = actionMatch[1];
        }
    }

    // Extract affordance_patterns elements (support both `element:` key and bare list items)
    const affBlock = extractV2Section('affordance_patterns');
    if (affBlock) {
        // Bare list items: `- button[data-action=edit]`
        const bareMatches = affBlock.matchAll(/^\s*-\s+(?!element:)([^\n]+)/gm);
        for (const m of bareMatches) {
            const val = m[1].trim().replace(/^['"]|['"]$/g, '');
            if (val) result.affordance_patterns.push(val);
        }
        // Keyed items: `- element: 'selector'`
        const elemMatches = affBlock.matchAll(/- element:\s*['"]?([^'"]+)['"]?/g);
        for (const m of elemMatches) result.affordance_patterns.push(m[1].trim());
    }

    // Extract bfs_max_routes
    const bfsMatch = yamlText.match(/bfs_max_routes:\s*(\d+)/);
    if (bfsMatch) result.bfs_max_routes = parseInt(bfsMatch[1], 10);

    // Extract contract_symmetry entries (support `name:` and `id:`)
    const symBlock = extractV2Section('contract_symmetry');
    if (symBlock) {
        const entries = parseListEntries(symBlock);
        for (const { name, body } of entries) {
            result.contract_symmetry.push(name);
            // Track api_endpoint / path_template per entry for SHIFTING detection
            const epMatch = body.match(/(?:api_endpoint|path_template):\s*(\S+)/);
            if (epMatch) {
                if (!result.contract_symmetry_endpoints) result.contract_symmetry_endpoints = {};
                result.contract_symmetry_endpoints[name] = epMatch[1];
            }
        }
    }

    return result;
}

/**
 * Main entry point.
 *
 * @param {object} options
 * @param {string} options.beforeYaml  - content of .autospec/test.yml before the change
 * @param {string} options.afterYaml   - content of .autospec/test.yml after the change
 * @param {string} [options.filePath]  - file path to attribute verdicts to (default: '.autospec/test.yml')
 * @returns {Verdict[]}
 */
export function classifyV2ContractDiff(options) {
    const { beforeYaml = '', afterYaml = '', filePath = '.autospec/test.yml' } = options;

    const before = extractInvariantsV2Struct(beforeYaml);
    const after = extractInvariantsV2Struct(afterYaml);

    const verdicts = [];

    const makeVerdict = (bucket, detail) => ({
        file: filePath,
        line: 0,
        bucket,
        framework: 'autospec-v2-contract',
        detail,
    });

    // ── Invariant entries ─────────────────────────────────────────────────────

    // Removed invariant entries → LOOSENING
    for (const id of before.invariants) {
        if (!after.invariants.includes(id)) {
            verdicts.push(makeVerdict('LOOSENING', `invariants_v2.invariants[id="${id}"] removed`));
        }
    }
    // Added invariant entries → STRENGTHENING
    for (const id of after.invariants) {
        if (!before.invariants.includes(id)) {
            verdicts.push(makeVerdict('STRENGTHENING', `invariants_v2.invariants[id="${id}"] added`));
        }
    }

    // require_count_at_least changes
    for (const id of before.invariants) {
        if (!after.invariants.includes(id)) continue; // already handled as removal
        const bCount = before.require_count_at_least[id];
        const aCount = after.require_count_at_least[id];
        if (bCount !== undefined && aCount !== undefined && aCount < bCount) {
            verdicts.push(makeVerdict('LOOSENING',
                `invariants_v2.invariants[id="${id}"].require_count_at_least lowered: ${bCount} → ${aCount}`));
        } else if (bCount !== undefined && aCount !== undefined && aCount > bCount) {
            verdicts.push(makeVerdict('STRENGTHENING',
                `invariants_v2.invariants[id="${id}"].require_count_at_least raised: ${bCount} → ${aCount}`));
        }
    }

    // apply_on_routes narrowing/widening
    for (const id of before.invariants) {
        if (!after.invariants.includes(id)) continue;
        const bRoutes = before.apply_on_routes[id] || [];
        const aRoutes = after.apply_on_routes[id] || [];
        const removedRoutes = bRoutes.filter(r => !aRoutes.includes(r));
        const addedRoutes = aRoutes.filter(r => !bRoutes.includes(r));
        if (removedRoutes.length > 0) {
            verdicts.push(makeVerdict('LOOSENING',
                `invariants_v2.invariants[id="${id}"].apply_on_routes narrowed: removed [${removedRoutes.join(', ')}]`));
        }
        if (addedRoutes.length > 0) {
            verdicts.push(makeVerdict('STRENGTHENING',
                `invariants_v2.invariants[id="${id}"].apply_on_routes widened: added [${addedRoutes.join(', ')}]`));
        }
    }

    // mismatch_action: hard_fail → warn_only → LOOSENING
    // mismatch_action: warn_only → hard_fail → STRENGTHENING
    const allMismatchIds = new Set([
        ...Object.keys(before.mismatch_actions),
        ...Object.keys(after.mismatch_actions),
    ]);
    for (const id of allMismatchIds) {
        const bAction = before.mismatch_actions[id];
        const aAction = after.mismatch_actions[id];
        if (bAction === 'hard_fail' && aAction === 'warn_only') {
            verdicts.push(makeVerdict('LOOSENING',
                `invariants_v2 entry id="${id}": mismatch_action changed hard_fail → warn_only`));
        } else if (bAction === 'warn_only' && aAction === 'hard_fail') {
            verdicts.push(makeVerdict('STRENGTHENING',
                `invariants_v2 entry id="${id}": mismatch_action changed warn_only → hard_fail`));
        }
    }

    // ── Window contracts ──────────────────────────────────────────────────────

    for (const id of before.window_contracts) {
        if (!after.window_contracts.includes(id)) {
            verdicts.push(makeVerdict('LOOSENING', `invariants_v2.window_contracts[id="${id}"] removed`));
        }
    }
    for (const id of after.window_contracts) {
        if (!before.window_contracts.includes(id)) {
            verdicts.push(makeVerdict('STRENGTHENING', `invariants_v2.window_contracts[id="${id}"] added`));
        }
    }

    // ── Crawler affordance_patterns ───────────────────────────────────────────

    for (const elem of before.affordance_patterns) {
        if (!after.affordance_patterns.includes(elem)) {
            verdicts.push(makeVerdict('LOOSENING',
                `invariants_v2.crawler.affordance_patterns[element="${elem}"] removed`));
        }
    }
    for (const elem of after.affordance_patterns) {
        if (!before.affordance_patterns.includes(elem)) {
            verdicts.push(makeVerdict('STRENGTHENING',
                `invariants_v2.crawler.affordance_patterns[element="${elem}"] added`));
        }
    }

    // bfs_max_routes changes
    if (before.bfs_max_routes !== null && after.bfs_max_routes !== null) {
        if (after.bfs_max_routes < before.bfs_max_routes) {
            verdicts.push(makeVerdict('LOOSENING',
                `invariants_v2.crawler.bfs_max_routes lowered: ${before.bfs_max_routes} → ${after.bfs_max_routes}`));
        } else if (after.bfs_max_routes > before.bfs_max_routes) {
            verdicts.push(makeVerdict('STRENGTHENING',
                `invariants_v2.crawler.bfs_max_routes raised: ${before.bfs_max_routes} → ${after.bfs_max_routes}`));
        }
    }

    // ── Contract symmetry ─────────────────────────────────────────────────────

    for (const id of before.contract_symmetry) {
        if (!after.contract_symmetry.includes(id)) {
            verdicts.push(makeVerdict('LOOSENING', `invariants_v2.contract_symmetry[id="${id}"] removed`));
        }
    }
    for (const id of after.contract_symmetry) {
        if (!before.contract_symmetry.includes(id)) {
            verdicts.push(makeVerdict('STRENGTHENING', `invariants_v2.contract_symmetry[id="${id}"] added`));
        }
    }

    // ── Selector / path_template changes → SHIFTING ───────────────────────────
    // (Structural changes that are neutral — same component, different selector)
    // Detected when: same id remains in both, but the YAML block has changed
    // in a way not covered above. We use a diff of the raw YAML sections as a
    // heuristic: if the invariant block for a given id changed but no
    // LOOSENING/STRENGTHENING verdict was emitted for it, it's a SHIFTING change.

    for (const id of before.invariants) {
        if (!after.invariants.includes(id)) continue;
        // Check if any verdict was already emitted for this id
        const alreadyFlagged = verdicts.some(v => v.detail.includes(`id="${id}"`));
        if (!alreadyFlagged) {
            // Look for selector changes in the raw YAML
            const beforeSection = extractIdSection(beforeYaml, id);
            const afterSection = extractIdSection(afterYaml, id);
            if (beforeSection !== afterSection) {
                verdicts.push(makeVerdict('SHIFTING',
                    `invariants_v2.invariants[id="${id}"] selector/config changed`));
            }
        }
    }

    for (const id of before.contract_symmetry) {
        if (!after.contract_symmetry.includes(id)) continue;
        const alreadyFlagged = verdicts.some(v => v.detail.includes(`id="${id}"`));
        if (!alreadyFlagged) {
            const beforeSection = extractIdSection(beforeYaml, id);
            const afterSection = extractIdSection(afterYaml, id);
            if (beforeSection !== afterSection) {
                verdicts.push(makeVerdict('SHIFTING',
                    `invariants_v2.contract_symmetry[id="${id}"] path_template/config changed`));
            }
        }
    }

    return verdicts;
}

/**
 * Extract the YAML block for a specific name/id entry from a list.
 * Supports both `- name: <val>` and `- id: <val>` forms (per spec §5c).
 */
function extractIdSection(yamlText, id) {
    if (!yamlText) return '';
    const escapedId = id.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    // Find the start of the entry line
    const startPat = new RegExp(`(^|\\n)([ \\t]*)- (?:name|id):\\s*${escapedId}(\\n|$)`);
    const startMatch = startPat.exec(yamlText);
    if (!startMatch) return '';
    const startIdx = startMatch.index + (startMatch[1] === '\n' ? 1 : 0);
    const entryIndent = startMatch[2];
    // Find next sibling entry at same indent level
    const siblingPat = new RegExp(`\\n${entryIndent}- (?:name|id):`, 'g');
    siblingPat.lastIndex = startIdx + 1;
    const siblingMatch = siblingPat.exec(yamlText);
    return siblingMatch
        ? yamlText.slice(startIdx, siblingMatch.index)
        : yamlText.slice(startIdx);
}

/**
 * Extract before/after YAML from a unified diff of .autospec/test.yml.
 * Returns { beforeYaml, afterYaml }.
 */
export function extractYamlFromDiff(diffText) {
    if (!diffText) return { beforeYaml: '', afterYaml: '' };

    const beforeLines = [];
    const afterLines = [];

    for (const line of diffText.split('\n')) {
        if (line.startsWith('---') || line.startsWith('+++') || line.startsWith('@@')) continue;
        if (line.startsWith('-')) {
            beforeLines.push(line.slice(1));
        } else if (line.startsWith('+')) {
            afterLines.push(line.slice(1));
        } else {
            // Context line — appears in both
            const content = line.startsWith(' ') ? line.slice(1) : line;
            beforeLines.push(content);
            afterLines.push(content);
        }
    }

    return {
        beforeYaml: beforeLines.join('\n'),
        afterYaml: afterLines.join('\n'),
    };
}

// ── CLI entrypoint ────────────────────────────────────────────────────────────

if (process.argv[1] && fs.realpathSync(path.resolve(process.argv[1])) === fs.realpathSync(path.resolve(__filename))) {
    const args = process.argv.slice(2);
    let beforeFile = '';
    let afterFile = '';
    let diffFile = '';

    for (let i = 0; i < args.length; i++) {
        if (args[i] === '--before') beforeFile = args[i + 1];
        if (args[i] === '--after')  afterFile  = args[i + 1];
        if (args[i] === '--diff')   diffFile   = args[i + 1];
    }

    let beforeYaml = '';
    let afterYaml = '';

    if (diffFile) {
        const diffText = fs.readFileSync(diffFile, 'utf8');
        ({ beforeYaml, afterYaml } = extractYamlFromDiff(diffText));
    } else {
        if (beforeFile) beforeYaml = fs.readFileSync(beforeFile, 'utf8');
        if (afterFile)  afterYaml  = fs.readFileSync(afterFile,  'utf8');
    }

    const verdicts = classifyV2ContractDiff({ beforeYaml, afterYaml });
    process.stdout.write(JSON.stringify(verdicts, null, 2) + '\n');

    const hasLoosening = verdicts.some(v => v.bucket === 'LOOSENING');
    const hasShifting  = verdicts.some(v => v.bucket === 'SHIFTING');
    process.exit(hasLoosening || hasShifting ? 1 : 0);
}
