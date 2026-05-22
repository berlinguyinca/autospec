#!/usr/bin/env node
// ai-review-doc.mjs — AI reviewer pass on auto-generated doc sections.
//
// Exports:
//   review(input, opts?)  → Promise<{ confidence: 'high'|'medium'|'low', concerns: string[] }>
//   summarize(text, opts?) → Promise<string>   (module summary, ≤200 chars)
//
// Input shape for review():
//   { section_heading, section_body, scope_globs, source_files_text }
//
// Options:
//   opts.mode       'review' (default) | 'summarize'
//   opts.stub       'high' | 'medium' | 'low'  — skip LLM, return fixture (Phase 8 tests)
//   opts.cacheDir   override cache directory (default ~/.autospec/ai-review-cache)
//   opts.maxTokens  input token budget (default 2000)
//   opts.maxRetries max parse retries (default 5)
//
// Spec §7a–§7d: deterministic prompt, adaptive-retry on parse fail, SHA-256 cache,
// cost ceiling ≤2000 input tokens with source-file truncation.

import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import crypto from 'node:crypto';
import { execSync } from 'node:child_process';

// ── Constants ─────────────────────────────────────────────────────────────────

const DEFAULT_CACHE_DIR = path.join(os.homedir(), '.autospec', 'ai-review-cache');
const DEFAULT_MAX_TOKENS = 2000;
const DEFAULT_MAX_RETRIES = 5;
const TOKENS_PER_CHAR = 0.25; // conservative: 4 chars ≈ 1 token

// Regex: exactly one line matching ai_reviewed: { confidence: high|medium|low, concerns: [...] }
const VERDICT_RE = /^\s*ai_reviewed:\s*\{\s*confidence:\s*(high|medium|low)\s*,\s*concerns:\s*(\[.*?\])\s*\}\s*$/m;

// ── Cache helpers ─────────────────────────────────────────────────────────────

function cacheKey(input) {
  const body = JSON.stringify({
    heading: input.section_heading || '',
    body: input.section_body || '',
    globs: (input.scope_globs || []).slice().sort(),
    sources: input.source_files_text || '',
  });
  return crypto.createHash('sha256').update(body).digest('hex');
}

function readCache(cacheDir, key) {
  const file = path.join(cacheDir, `${key}.json`);
  try {
    const raw = fs.readFileSync(file, 'utf8');
    return JSON.parse(raw);
  } catch {
    return null;
  }
}

function writeCache(cacheDir, key, result) {
  try {
    fs.mkdirSync(cacheDir, { recursive: true, mode: 0o700 });
    fs.writeFileSync(
      path.join(cacheDir, `${key}.json`),
      JSON.stringify(result),
      { encoding: 'utf8', mode: 0o600 }
    );
  } catch {
    // Cache write failure is non-fatal
  }
}

// ── Token budget + truncation ─────────────────────────────────────────────────

/**
 * Estimate token count from character length.
 */
function estimateTokens(str) {
  return Math.ceil((str || '').length * TOKENS_PER_CHAR);
}

/**
 * Build prompt body; if source_files_text exceeds budget, truncate at public_export
 * boundaries (or at character limit) and record truncation in a prefix.
 */
function buildPrompt(input, maxTokens) {
  const heading = input.section_heading || '';
  const body = input.section_body || '';
  const globs = (input.scope_globs || []).join(', ');
  const concerns = [];

  // Static portion (heading, body, globs, template)
  const staticPart = `You are reviewing auto-generated documentation for accuracy against source code.

Section: ${heading}
Generated content: ${body}
Declared scope (autospec-doc-scope): ${globs}
Source files in scope (full text): `;

  const staticTokens = estimateTokens(staticPart);
  const budget = maxTokens - staticTokens - 80; // reserve 80 for suffix + answer format

  let sourcesText = input.source_files_text || '';
  if (estimateTokens(sourcesText) > budget) {
    // Truncate: try to cut at last complete export signature within budget
    const charBudget = Math.floor(budget / TOKENS_PER_CHAR);
    const truncated = sourcesText.slice(0, charBudget);
    // Find last clean line boundary
    const lastNewline = truncated.lastIndexOf('\n');
    sourcesText = lastNewline > 0 ? truncated.slice(0, lastNewline) : truncated;
    concerns.push('source_files_text truncated to fit ≤2000 token budget');
  }

  const suffix = `

Verdict (exactly one line, machine-parseable):
  ai_reviewed: { confidence: high|medium|low, concerns: [str, ...] }

Rules:
- high: content accurately reflects source; no concerns
- medium: minor inaccuracies (phrasing, missed nuance); list in concerns
- low: significant mismatch or unverifiable claim; PR flagged for human review

ANSWER FORMAT: single line matching ai_reviewed: { confidence: high|medium|low, concerns: [...] }`;

  return { prompt: staticPart + sourcesText + suffix, truncationConcerns: concerns };
}

function buildSummarizePrompt(text) {
  return `Summarize the following module or code section in ≤200 characters. Return only the summary string, no prefix.

${text.slice(0, 4000)}`;
}

// ── LLM caller ────────────────────────────────────────────────────────────────

/**
 * Invoke the system LLM via `claude` CLI or fallback to a simple heuristic.
 * Returns the raw stdout string.
 * In stub mode (opts.stub) returns deterministic fixture output.
 */
function callLLM(prompt, opts) {
  if (opts && opts.stub) {
    const conf = opts.stub === 'high' ? 'high' : opts.stub === 'medium' ? 'medium' : 'low';
    return `ai_reviewed: { confidence: ${conf}, concerns: [] }`;
  }
  // Try `claude` CLI (Claude Code / autospec environment)
  try {
    return execSync(`claude -p ${JSON.stringify(prompt)}`, {
      encoding: 'utf8',
      timeout: 60000,
      stdio: ['pipe', 'pipe', 'pipe'],
    }).trim();
  } catch {
    // Fallback: treat as low-confidence (fail safe)
    return 'ai_reviewed: { confidence: low, concerns: ["LLM unavailable"] }';
  }
}

function callSummarizeLLM(prompt, opts) {
  if (opts && opts.stub) {
    return 'Stub module summary for testing.';
  }
  try {
    return execSync(`claude -p ${JSON.stringify(prompt)}`, {
      encoding: 'utf8',
      timeout: 60000,
      stdio: ['pipe', 'pipe', 'pipe'],
    }).trim().slice(0, 200);
  } catch {
    return '';
  }
}

// ── Verdict parser ────────────────────────────────────────────────────────────

/**
 * Parse a single-line verdict. Returns { confidence, concerns } or null on failure.
 */
function parseVerdict(raw) {
  const m = raw.match(VERDICT_RE);
  if (!m) return null;
  const confidence = m[1]; // 'high' | 'medium' | 'low'
  let concerns = [];
  try {
    // m[2] is the JSON array literal
    concerns = JSON.parse(m[2]);
    if (!Array.isArray(concerns)) concerns = [String(concerns)];
  } catch {
    concerns = [m[2]];
  }
  return { confidence, concerns };
}

// ── Main exports ──────────────────────────────────────────────────────────────

/**
 * Review a single generated doc section.
 *
 * @param {object} input  { section_heading, section_body, scope_globs, source_files_text }
 * @param {object} [opts] { stub, cacheDir, maxTokens, maxRetries, mode }
 * @returns {Promise<{ confidence: string, concerns: string[] }>}
 */
export async function review(input, opts = {}) {
  const cacheDir = opts.cacheDir || DEFAULT_CACHE_DIR;
  const maxTokens = opts.maxTokens || DEFAULT_MAX_TOKENS;
  const maxRetries = opts.maxRetries || DEFAULT_MAX_RETRIES;

  // Cache lookup
  const key = cacheKey(input);
  const cached = readCache(cacheDir, key);
  if (cached) return cached;

  const { prompt, truncationConcerns } = buildPrompt(input, maxTokens);

  // Adaptive-retry loop (mirrors feedback_llm_validator_adaptive_retry)
  let attempt = 1;
  let directiveContext = '';
  let lastRaw = '';
  while (attempt <= maxRetries) {
    const fullPrompt = directiveContext
      ? `${prompt}\n\n## Retry attempt ${attempt} directive\n${directiveContext}`
      : prompt;
    lastRaw = callLLM(fullPrompt, opts);
    const parsed = parseVerdict(lastRaw);
    if (parsed) {
      const result = {
        confidence: parsed.confidence,
        concerns: [...truncationConcerns, ...parsed.concerns],
      };
      writeCache(cacheDir, key, result);
      return result;
    }
    // Parse failed — accumulate directive for next attempt
    directiveContext += `\nFix PARSE_FAIL: response did not match ai_reviewed: { confidence: high|medium|low, concerns: [...] }. Last response: ${lastRaw.slice(0, 200)}`;
    attempt++;
  }

  // Exhausted retries — surface last error as low-confidence
  const result = {
    confidence: 'low',
    concerns: [
      ...truncationConcerns,
      `AI reviewer parse failed after ${maxRetries} attempts. Last response: ${lastRaw.slice(0, 200)}`,
    ],
  };
  writeCache(cacheDir, key, result);
  return result;
}

/**
 * Summarize a module or text block (mode: 'summarize').
 *
 * @param {string} text
 * @param {object} [opts] { stub, maxRetries }
 * @returns {Promise<string>}  ≤200 chars
 */
export async function summarize(text, opts = {}) {
  const maxRetries = opts.maxRetries || DEFAULT_MAX_RETRIES;
  const prompt = buildSummarizePrompt(text);

  let attempt = 1;
  while (attempt <= maxRetries) {
    const raw = callSummarizeLLM(prompt, opts);
    if (raw && raw.length > 0) {
      return raw.slice(0, 200);
    }
    attempt++;
  }
  return '';
}
