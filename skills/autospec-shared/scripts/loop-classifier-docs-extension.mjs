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
    const idx = DOCS_CATEGORY_PRIORITY.indexOf(category);
    if (idx === -1) return 0;
    return PRIORITY_BASE + (DOCS_CATEGORY_PRIORITY.length - 1 - idx);
}

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
                         'visual_stale' in gateJson || 'ai_review_stale' in gateJson;
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
    return candidates[0];
}
