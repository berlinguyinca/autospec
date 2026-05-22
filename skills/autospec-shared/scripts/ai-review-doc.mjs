#!/usr/bin/env node
// ai-review-doc.mjs — AI-as-reviewer for auto-generated documentation sections.
//
// Spec: docs/specs/2026-05-22-autospec-docs-amendment-design.md §7a-§7d
//
// Exports:
//   review(input, opts?) → Promise<{ confidence: 'high'|'medium'|'low', concerns: string[] }>
//   summarize(input, opts?) → Promise<string>  (module summary, ≤200 chars)
//
// input for review():
//   { section_heading, section_body, scope_globs, source_files_text }
//
// input for summarize():
//   { module_slug, exports, entry_points, files }
//
// opts:
//   { stub: 'high'|'medium'|'low' }  — return stubbed response (no LLM call); for tests
//   { mode: 'review'|'summarize' }   — default 'review'
//   { cacheDir: string }             — override cache directory
//   { maxTokens: number }            — override cost ceiling (default 2000)
//   { maxRetries: number }           — override retry cap (default 5)
//
// Cache: ~/.autospec/ai-review-cache/<sha256>.json
//   keyed by SHA-256 of (section_body || sorted_globs.join(',') || source_files_text)
//
// LLM backend: uses ANTHROPIC_API_KEY + claude-haiku if set; otherwise errors without stub.
//
// CLI:
//   node ai-review-doc.mjs --heading <str> --body <str> --globs <glob,...> --sources <file>
//   node ai-review-doc.mjs --stub high   # smoke test

import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';

const __filename = fileURLToPath(import.meta.url);

// ── Constants ─────────────────────────────────────────────────────────────────

const DEFAULT_CACHE_DIR  = path.join(process.env.HOME || '/tmp', '.autospec', 'ai-review-cache');
const DEFAULT_MAX_TOKENS = 2000;  // input token budget per spec §7c
const DEFAULT_MAX_RETRIES = 5;    // adaptive retry cap per feedback llm_validator_adaptive_retry
const APPROX_CHARS_PER_TOKEN = 4; // conservative estimate for truncation

// ── Prompt templates ──────────────────────────────────────────────────────────

/**
 * Build the deterministic review prompt from spec §7a.
 */
function buildReviewPrompt({ section_heading, section_body, scope_globs, source_files_text }) {
  const globsStr = Array.isArray(scope_globs) ? scope_globs.join(', ') : (scope_globs || '');
  return [
    'You are reviewing auto-generated documentation for accuracy against source code.',
    '',
    `Section: ${section_heading}`,
    `Generated content: ${section_body}`,
    `Declared scope (autospec-doc-scope): ${globsStr}`,
    `Source files in scope (full text): ${source_files_text || '(none)'}`,
    '',
    'Verdict (exactly one line, machine-parseable):',
    '  ai_reviewed: { confidence: high|medium|low, concerns: [str, ...] }',
    '',
    'Rules:',
    '- high: content accurately reflects source; no concerns',
    '- medium: minor inaccuracies (phrasing, missed nuance); list in concerns',
    '- low: significant mismatch or unverifiable claim; PR flagged for human review',
    '',
    'ANSWER FORMAT: single line matching ai_reviewed: { confidence: high|medium|low, concerns: [...] }',
  ].join('\n');
}

/**
 * Build the module summary prompt (mode: 'summarize').
 */
function buildSummarizePrompt({ module_slug, exports = [], entry_points = [], files = [] }) {
  const exportNames = exports.map(e => e.name || e).join(', ');
  const entryKinds = entry_points.map(ep => ep.kind || ep).join(', ');
  const fileNames = files.map(f => path.basename(f)).join(', ');
  return [
    'Write a one-sentence module summary (≤200 chars, no markdown) describing this module.',
    '',
    `Module: ${module_slug}`,
    `Files: ${fileNames || '(none)'}`,
    `Exports: ${exportNames || '(none)'}`,
    `Entry points: ${entryKinds || '(none)'}`,
    '',
    'Respond with ONLY the summary sentence. No preamble, no quotes, no newlines.',
  ].join('\n');
}

// ── Tokenization (approximate) ────────────────────────────────────────────────

/**
 * Approximate token count for a string.
 */
function approxTokens(str) {
  return Math.ceil((str || '').length / APPROX_CHARS_PER_TOKEN);
}

/**
 * Truncate source_files_text to fit within maxTokens budget.
 * Returns { text, truncated } where truncated is true if content was cut.
 */
function truncateSourceText(source_files_text, promptOverhead, maxTokens) {
  const budget = maxTokens - promptOverhead;
  if (budget <= 0) return { text: '', truncated: true };
  const maxChars = budget * APPROX_CHARS_PER_TOKEN;
  if ((source_files_text || '').length <= maxChars) {
    return { text: source_files_text || '', truncated: false };
  }
  return { text: source_files_text.slice(0, maxChars), truncated: true };
}

// ── Cache ─────────────────────────────────────────────────────────────────────

/**
 * Compute cache key SHA-256 for a review input.
 */
export function cacheKey({ section_body = '', scope_globs = [], source_files_text = '' }) {
  const normalized = [
    section_body,
    (Array.isArray(scope_globs) ? [...scope_globs].sort() : [scope_globs]).join(','),
    source_files_text,
  ].join('||');
  return crypto.createHash('sha256').update(normalized).digest('hex');
}

/**
 * Read cached result. Returns null if cache miss.
 */
function readCache(sha, cacheDir) {
  const file = path.join(cacheDir, `${sha}.json`);
  if (!fs.existsSync(file)) return null;
  try {
    return JSON.parse(fs.readFileSync(file, 'utf8'));
  } catch {
    return null;
  }
}

/**
 * Write result to cache.
 */
function writeCache(sha, result, cacheDir) {
  fs.mkdirSync(cacheDir, { recursive: true, mode: 0o700 });
  const file = path.join(cacheDir, `${sha}.json`);
  fs.writeFileSync(file, JSON.stringify({ ...result, cached_at: Date.now() }, null, 2), 'utf8');
}

// ── LLM call ──────────────────────────────────────────────────────────────────

/**
 * Call the Anthropic Claude API (claude-haiku) with a prompt.
 * Returns the raw text response.
 * Throws on error.
 */
async function callLLM(prompt) {
  const apiKey = process.env.ANTHROPIC_API_KEY;
  if (!apiKey) {
    throw new Error(
      'ai-review-doc: ANTHROPIC_API_KEY not set. Use opts.stub for testing or set the key.'
    );
  }

  // Dynamic import to avoid hard dependency
  let fetch;
  try {
    fetch = (await import('node:http')).request; // not what we want
    // Use native fetch (Node 18+)
    fetch = globalThis.fetch;
  } catch {
    fetch = globalThis.fetch;
  }

  if (!fetch) {
    throw new Error('ai-review-doc: globalThis.fetch not available (Node 18+ required)');
  }

  const response = await fetch('https://api.anthropic.com/v1/messages', {
    method: 'POST',
    headers: {
      'x-api-key': apiKey,
      'anthropic-version': '2023-06-01',
      'content-type': 'application/json',
    },
    body: JSON.stringify({
      model: 'claude-haiku-4-5',
      max_tokens: 256,
      messages: [{ role: 'user', content: prompt }],
    }),
  });

  if (!response.ok) {
    const body = await response.text();
    throw new Error(`ai-review-doc: LLM API error ${response.status}: ${body.slice(0, 200)}`);
  }

  const data = await response.json();
  return data.content?.[0]?.text || '';
}

// ── Response parser ───────────────────────────────────────────────────────────

/**
 * Parse the LLM's single-line verdict.
 * Returns { confidence, concerns } or null on parse failure.
 *
 * Expected format: ai_reviewed: { confidence: high|medium|low, concerns: ["..."] }
 */
export function parseVerdict(text) {
  if (!text || typeof text !== 'string') return null;

  // Find the ai_reviewed: line (case-sensitive, single-line)
  const lines = text.split('\n');
  for (const line of lines) {
    const trimmed = line.trim();
    if (!trimmed.startsWith('ai_reviewed:')) continue;

    // Extract confidence
    const confMatch = trimmed.match(/confidence:\s*(high|medium|low)/);
    if (!confMatch) continue;
    const confidence = confMatch[1];

    // Extract concerns array — parse JSON array after "concerns:"
    const concernsMatch = trimmed.match(/concerns:\s*(\[.*?\])/);
    let concerns = [];
    if (concernsMatch) {
      try {
        concerns = JSON.parse(concernsMatch[1]);
        if (!Array.isArray(concerns)) concerns = [String(concerns)];
      } catch {
        // Fallback: split by comma inside brackets
        const inner = concernsMatch[1].replace(/^\[|\]$/g, '').trim();
        concerns = inner
          ? inner.split(',').map(s => s.trim().replace(/^["']|["']$/g, ''))
          : [];
      }
    }

    return { confidence, concerns: concerns.map(String) };
  }

  return null;
}

// ── Stub responses ────────────────────────────────────────────────────────────

/**
 * Generate a stubbed LLM response string for the given confidence level.
 */
function stubResponse(level) {
  const concerns = level === 'high' ? '[]'
    : level === 'medium' ? '["Minor phrasing inconsistency in section intro"]'
    : '["Significant mismatch between generated content and source code"]';
  return `ai_reviewed: { confidence: ${level}, concerns: ${concerns} }`;
}

// ── Core review function ──────────────────────────────────────────────────────

/**
 * Review a doc section for accuracy.
 *
 * @param {{ section_heading: string, section_body: string, scope_globs: string[], source_files_text: string }} input
 * @param {{ stub?: string, mode?: string, cacheDir?: string, maxTokens?: number, maxRetries?: number }} opts
 * @returns {Promise<{ confidence: 'high'|'medium'|'low', concerns: string[] }>}
 */
export async function review(input, opts = {}) {
  const {
    stub,
    cacheDir = DEFAULT_CACHE_DIR,
    maxTokens = DEFAULT_MAX_TOKENS,
    maxRetries = DEFAULT_MAX_RETRIES,
  } = opts;

  const { section_heading = '', section_body = '', scope_globs = [], source_files_text = '' } = input;

  // Cache lookup
  const sha = cacheKey({ section_body, scope_globs, source_files_text });
  const cached = readCache(sha, cacheDir);
  if (cached && cached.confidence) {
    return { confidence: cached.confidence, concerns: cached.concerns || [] };
  }

  // Build base prompt (without source text) to estimate overhead
  const basePrompt = buildReviewPrompt({ section_heading, section_body, scope_globs, source_files_text: '' });
  const overhead = approxTokens(basePrompt);

  // Truncate source text if needed
  const { text: truncatedSource, truncated } = truncateSourceText(source_files_text, overhead, maxTokens);

  const concerns_extra = truncated
    ? ['Source files truncated to fit ≤2000 token cost ceiling']
    : [];

  const prompt = buildReviewPrompt({
    section_heading,
    section_body,
    scope_globs,
    source_files_text: truncatedSource,
  });

  // Adaptive retry loop (max maxRetries attempts)
  let lastError = null;
  for (let attempt = 0; attempt < maxRetries; attempt++) {
    let rawText;

    if (stub) {
      // Stubbed mode: return a valid response (or malformed for retry testing)
      rawText = stubResponse(stub);
    } else {
      try {
        rawText = await callLLM(prompt);
      } catch (err) {
        lastError = err;
        continue;
      }
    }

    const parsed = parseVerdict(rawText);
    if (parsed) {
      const result = {
        confidence: parsed.confidence,
        concerns: [...concerns_extra, ...parsed.concerns],
      };
      writeCache(sha, result, cacheDir);
      return result;
    }

    // Parse failure — retry with directive appended
    lastError = new Error(`Parse failure on attempt ${attempt + 1}: "${rawText.slice(0, 100)}"`);
  }

  // Exhausted retries
  throw new Error(
    `ai-review-doc: adaptive retry exhausted after ${maxRetries} attempts. Last error: ${lastError?.message}`
  );
}

/**
 * Summarize a module (mode: 'summarize').
 *
 * @param {{ module_slug: string, exports: object[], entry_points: object[], files: string[] }} input
 * @param {{ stub?: string, cacheDir?: string, maxRetries?: number }} opts
 * @returns {Promise<string>} summary string ≤200 chars
 */
export async function summarize(input, opts = {}) {
  const { stub, maxRetries = DEFAULT_MAX_RETRIES } = opts;

  if (stub) {
    const slug = input.module_slug || 'module';
    const summary = `Provides ${slug} functionality with ${(input.exports || []).length} export(s).`;
    return summary.slice(0, 200);
  }

  const prompt = buildSummarizePrompt(input);
  let lastError = null;

  for (let attempt = 0; attempt < maxRetries; attempt++) {
    try {
      const raw = await callLLM(prompt);
      const summary = raw.trim().replace(/\n.*/s, '').slice(0, 200);
      if (summary.length > 0) return summary;
      lastError = new Error(`Empty summary on attempt ${attempt + 1}`);
    } catch (err) {
      lastError = err;
    }
  }

  throw new Error(
    `ai-review-doc: summarize retry exhausted after ${maxRetries} attempts. Last: ${lastError?.message}`
  );
}

// ── CLI entrypoint ────────────────────────────────────────────────────────────

if (process.argv[1] && fs.realpathSync(path.resolve(process.argv[1])) === fs.realpathSync(path.resolve(__filename))) {
  const args = process.argv.slice(2);
  let heading = '';
  let body = '';
  let globs = [];
  let sourcesFile = null;
  let stubLevel = null;
  let mode = 'review';

  for (let i = 0; i < args.length; i++) {
    if (args[i] === '--heading')  heading     = args[i + 1];
    if (args[i] === '--body')     body        = args[i + 1];
    if (args[i] === '--globs')    globs       = (args[i + 1] || '').split(',').filter(Boolean);
    if (args[i] === '--sources')  sourcesFile = args[i + 1];
    if (args[i] === '--stub')     stubLevel   = args[i + 1];
    if (args[i] === '--mode')     mode        = args[i + 1];
  }

  const source_files_text = sourcesFile ? fs.readFileSync(sourcesFile, 'utf8') : '';

  try {
    if (mode === 'summarize') {
      const result = await summarize(
        { module_slug: heading, exports: [], entry_points: [], files: [] },
        { stub: stubLevel }
      );
      process.stdout.write(result + '\n');
    } else {
      const result = await review(
        { section_heading: heading, section_body: body, scope_globs: globs, source_files_text },
        { stub: stubLevel }
      );
      process.stdout.write(JSON.stringify(result, null, 2) + '\n');
    }
    process.exit(0);
  } catch (err) {
    process.stderr.write(`ai-review-doc: error: ${err.message}\n`);
    process.exit(1);
  }
}
