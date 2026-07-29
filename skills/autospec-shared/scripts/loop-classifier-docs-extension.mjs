#!/usr/bin/env node
// loop-classifier-docs-extension.mjs
// Docs-amendment extension for the v1 self-heal loop classifier.
//
// Exports:
//   classify(gateJson, lastIterations) → ClassifyResult | null
//   DOCS_CATEGORY_PRIORITY: string[]   — ordered list (highest priority first)
//   LOOSENING_PATTERNS: RegExp[]       — anti-rubber-stamp detector patterns
//   detectBucket(diffText) → 'LOOSENING' | 'STRENGTHENING' | 'SHIFTING' | null
//
// ClassifyResult schema (superset of v1 schema):
//   {
//     classification: string,
//     target_files: string[],       // doc files or source files to fix
//     suggested_action: string,     // short imperative instruction
//     estimated_minutes: number,
//     priority: number,
//     bucket?: 'LOOSENING' | 'STRENGTHENING' | 'SHIFTING',
//   }
//
// Returns null if the gate JSON does not represent a docs-gate failure.
// The v1 classifier should call this before its own fallback logic.

// ── Priority ordering per spec §3c ────────────────────────────────────────────

export const DOCS_CATEGORY_PRIORITY = [
    'missing_doc_scope',       // 12 — highest: no scope declared at all
    'failing_doc_drift',       // 11 — doc not updated when source changed
    'failing_visual_stale',    // 10 — screenshot older than source
    'failing_ai_review_stale', //  9 — AI review stale
    'failing_manifest_stale',  //  8 — LLM manifest stale
];

const PRIORITY_BASE = 8; // lowest docs priority; missing_doc_scope = PRIORITY_BASE + 4

function categoryPriority(category) {
    // `failing_example_stale` (a `regenerate` signal added by spec §D6) slots
    // between `failing_doc_drift` and `failing_visual_stale` without changing the
    // 5-entry DOCS_CATEGORY_PRIORITY contract: it sits just above visual_stale so
    // a stale verified example self-heals before a stale screenshot, while drift
    // and missing_scope still outrank it. The half-step keeps every category at a
    // distinct priority (no sort ties).
    if (category === 'failing_example_stale') {
        return categoryPriority('failing_visual_stale') + 0.5;
    }
    const idx = DOCS_CATEGORY_PRIORITY.indexOf(category);
    if (idx === -1) return 0;
    return PRIORITY_BASE + (DOCS_CATEGORY_PRIORITY.length - 1 - idx);
}

// ── Self-heal `regenerate` action (spec §D6 row 1) ────────────────────────────
//
// The autospec-run Phase 4 per-PR docs-drift gate self-heals a subset of
// findings by regenerating the affected doc scopes IN THE SAME PR: invoke
// `/autospec-doc` (via doc-orchestrator.mjs) scoped to those scopes, then run the
// regenerated pages through verify-examples.mjs and commit them onto the PR
// branch. Three drift-JSON signals are regenerable: a doc section drifted from
// its source (`drift`), changed source has no covering scope (`missing_scope`),
// or a verified-example marker went stale (`example_stale`). The action name is
// PINNED to `regenerate` — the gate block and tests key off this literal.
export const REGENERATE_ACTION = 'regenerate';

// The exact drift-JSON signal keys that route to the `regenerate` action.
export const REGENERATE_SIGNALS = ['drift', 'missing_scope', 'example_stale'];

// Classifications that self-heal via `regenerate` (one per REGENERATE_SIGNALS
// entry, in the same order): missing_scope → missing_doc_scope,
// drift → failing_doc_drift, example_stale → failing_example_stale.
const REGENERATE_CLASSIFICATIONS = new Set([
    'missing_doc_scope',
    'failing_doc_drift',
    'failing_example_stale',
]);

// ── Anti-rubber-stamp guardrail per spec §3d ─────────────────────────────────

/**
 * Detect the edit bucket from a unified diff of doc changes.
 * Returns 'LOOSENING', 'STRENGTHENING', 'SHIFTING', or null (no doc changes).
 *
 * LOOSENING (❌ blocks auto-merge):
 *   - Remove autospec-doc-scope comment
 *   - Narrow src: globs (remove a glob from the array)
 *   - Remove a section that had non-empty scope
 *
 * STRENGTHENING (✅ auto-merge allowed):
 *   - Add new autospec-doc-scope block
 *   - Widen src: globs (add a glob)
 *
 * SHIFTING (conditional):
 *   - Edit doc section body (content change)
 */
export function detectBucket(diffText) {
    if (!diffText || typeof diffText !== 'string') return null;

    const lines = diffText.split('\n');
    const removedLines = lines.filter(l => l.startsWith('-') && !l.startsWith('---'));
    const addedLines   = lines.filter(l => l.startsWith('+') && !l.startsWith('+++'));

    const removedText = removedLines.map(l => l.slice(1)).join('\n');
    const addedText   = addedLines.map(l => l.slice(1)).join('\n');

    // LOOSENING: scope comment removed
    const scopeCommentRe = /autospec-doc-scope/;
    if (scopeCommentRe.test(removedText) && !scopeCommentRe.test(addedText)) {
        return 'LOOSENING';
    }

    // LOOSENING: src globs narrowed (fewer quoted glob strings in removed vs added)
    // Count individual quoted strings in src: context lines across removed/added
    function countGlobs(lines) {
        let count = 0;
        for (const l of lines) {
            const content = l.slice(1); // strip +/-
            // Count quoted strings that look like globs (contain / or * or end in extension)
            const matches = content.match(/"[^"]+"/g) || [];
            count += matches.filter(m => /[/*.]/.test(m)).length;
        }
        return count;
    }
    const removedGlobCount = countGlobs(removedLines);
    const addedGlobCount   = countGlobs(addedLines);
    if (removedGlobCount > addedGlobCount && removedGlobCount > 0) {
        return 'LOOSENING';
    }

    // STRENGTHENING: new scope comment added
    if (scopeCommentRe.test(addedText) && !scopeCommentRe.test(removedText)) {
        return 'STRENGTHENING';
    }

    // STRENGTHENING: globs widened (more globs added than removed)
    if (addedGlobCount > removedGlobCount && addedGlobCount > 0) {
        return 'STRENGTHENING';
    }

    // SHIFTING: doc content changed (any +/- lines in doc files)
    if (removedLines.length > 0 || addedLines.length > 0) {
        return 'SHIFTING';
    }

    return null;
}

// ── Main classify function ────────────────────────────────────────────────────

/**
 * Classify a docs-gate failure.
 *
 * @param {object} gateJson - parsed gate JSON from check-doc-drift.sh
 * @param {object[]} [lastIterations] - last N loop iterations (unused for docs)
 * @returns {object|null} ClassifyResult or null if not a docs gate failure
 */
export function classify(gateJson, lastIterations = []) {
    // Only handle doc-gate JSON (has drift/missing_scope/visual_stale fields)
    if (!gateJson || typeof gateJson !== 'object') return null;
    const hasDocFields = 'drift' in gateJson || 'missing_scope' in gateJson ||
                         'visual_stale' in gateJson || 'ai_review_stale' in gateJson ||
                         'example_stale' in gateJson;
    if (!hasDocFields) return null;

    // If gate passed, no action needed
    if (gateJson.passed === true) return null;

    const candidates = [];

    // missing_doc_scope — changed source not covered by any scope
    const missingScope = gateJson.missing_scope || [];
    if (missingScope.length > 0) {
        const targetFiles = missingScope.map(e => e.source_file || e).filter(Boolean);
        candidates.push({
            classification: 'missing_doc_scope',
            target_files: targetFiles,
            suggested_action: [
                'Add an <!-- autospec-doc-scope: src: ["<glob>"] --> block to the',
                'appropriate doc section (default: docs/ARCHITECTURE.md) covering:',
                targetFiles.slice(0, 3).join(', '),
            ].join(' '),
            estimated_minutes: 5,
            priority: categoryPriority('missing_doc_scope'),
        });
    }

    // failing_doc_drift — doc section not updated when matching source changed
    const drift = gateJson.drift || [];
    if (drift.length > 0) {
        const targetFiles = drift.map(e => e.doc_file).filter(Boolean);
        const reasons = drift.map(e => {
            const src = (e.matching_source_files || []).join(', ');
            return `${e.doc_file} §${e.heading}: source changed (${src})`;
        });
        candidates.push({
            classification: 'failing_doc_drift',
            target_files: targetFiles,
            suggested_action: [
                'Update the following doc sections to reflect source changes:',
                reasons.slice(0, 2).join('; '),
            ].join(' '),
            estimated_minutes: 10,
            priority: categoryPriority('failing_doc_drift'),
        });
    }

    // failing_visual_stale — screenshot older than source
    const visualStale = gateJson.visual_stale || [];
    if (visualStale.length > 0) {
        const targetFiles = visualStale.map(e => e.screenshot || e.doc_file).filter(Boolean);
        candidates.push({
            classification: 'failing_visual_stale',
            target_files: targetFiles,
            suggested_action: 'Regenerate screenshots via gen-screenshots.mjs for stale visual sections.',
            estimated_minutes: 15,
            priority: categoryPriority('failing_visual_stale'),
        });
    }

    // failing_example_stale — verified-example marker older than source (#922,
    // spec §D3/§D6). Same shape as visual_stale; routes to `regenerate` so the
    // affected pages are re-generated and their examples re-verified in-PR.
    const exampleStale = gateJson.example_stale || [];
    if (exampleStale.length > 0) {
        const targetFiles = exampleStale.map(e => e.doc_file || e.page || e).filter(Boolean);
        candidates.push({
            classification: 'failing_example_stale',
            target_files: targetFiles,
            suggested_action: [
                'Regenerate and re-verify the following doc pages whose verified',
                'example markers went stale:',
                targetFiles.slice(0, 3).join(', '),
            ].join(' '),
            estimated_minutes: 12,
            priority: categoryPriority('failing_example_stale'),
        });
    }

    // failing_ai_review_stale — AI review stale
    const aiStale = gateJson.ai_review_stale || [];
    if (aiStale.length > 0) {
        const targetFiles = aiStale.map(e => e.doc_file).filter(Boolean);
        candidates.push({
            classification: 'failing_ai_review_stale',
            target_files: targetFiles,
            suggested_action: 'Rerun ai-review-doc.mjs for the stale sections.',
            estimated_minutes: 10,
            priority: categoryPriority('failing_ai_review_stale'),
        });
    }

    // failing_manifest_stale — manifest stale (inferred when gate fails but no drift/scope)
    if (gateJson.manifest_stale === true) {
        candidates.push({
            classification: 'failing_manifest_stale',
            target_files: ['docs/.llm-manifest.json'],
            suggested_action: 'Rerun gen-llm-manifest.mjs to update the LLM manifest.',
            estimated_minutes: 5,
            priority: categoryPriority('failing_manifest_stale'),
        });
    }

    if (candidates.length === 0) return null;

    // Return highest-priority candidate
    candidates.sort((a, b) => b.priority - a.priority);
    const winner = candidates[0];

    // Self-heal wiring (spec §D6 row 1): regenerable findings carry the pinned
    // `regenerate` action and the affected scope list so the autospec-run gate
    // can invoke `/autospec-doc` scoped to ONLY those scopes and verify the
    // regenerated pages in-PR. Non-regenerable findings (visual/ai/manifest) are
    // returned unchanged for the generic self-heal loop.
    if (REGENERATE_CLASSIFICATIONS.has(winner.classification)) {
        winner.action = REGENERATE_ACTION;
        winner.scopes = [...new Set(winner.target_files)];
    }

    return winner;
}

// ── CLI ───────────────────────────────────────────────────────────────────────
//
// Invoked by the autospec-run Phase 4 docs-drift gate as:
//   printf '%s' "$DRIFT_JSON" | node loop-classifier-docs-extension.mjs \
//     --drift-json - --issue <ISSUE> --pr <PR>
// Reads the drift JSON from the `--drift-json` arg (a path, or `-` for stdin),
// classifies it, and prints the verdict object (incl. the `action`/`scopes`
// fields the gate keys off) to stdout. Prints nothing and exits 0 when the gate
// produced no actionable candidate, so the caller's `|| true` stays a no-op.

async function readDriftSource(src) {
    if (!src || src === '-') {
        const { readFileSync } = await import('node:fs');
        return readFileSync(0, 'utf8'); // fd 0 = stdin
    }
    const { readFileSync } = await import('node:fs');
    return readFileSync(src, 'utf8');
}

async function cliMain(argv) {
    let driftSrc = '-';
    for (let i = 0; i < argv.length; i++) {
        if (argv[i] === '--drift-json') { driftSrc = argv[i + 1]; i++; }
    }
    let gateJson;
    try {
        const raw = await readDriftSource(driftSrc);
        gateJson = JSON.parse(raw);
    } catch {
        return 0; // unreadable/unparseable drift JSON — emit nothing, never fail
    }
    const verdict = classify(gateJson);
    if (verdict) process.stdout.write(JSON.stringify(verdict));
    return 0;
}

if (import.meta.url === `file://${process.argv[1]}`) {
    cliMain(process.argv.slice(2)).then((code) => process.exit(code));
}
