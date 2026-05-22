#!/usr/bin/env node
// gen-assistant-prompt.mjs — generate docs/ASSISTANT_PROMPT.md from spec §5c template.
//
// Exports:
//   generateAssistantPrompt({ repo, repoRoot }): string
//   writeAssistantPrompt({ repo, repoRoot, outputPath }): Promise<{ written: boolean, path: string }>
//
// Idempotent: skips write if byte-equal to existing file.

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { execSync } from 'node:child_process';

const __dirname = path.dirname(fileURLToPath(import.meta.url));

function detectRepo(repoRoot) {
  try {
    const remote = execSync('git remote get-url origin', {
      cwd: repoRoot, encoding: 'utf8', stdio: ['pipe','pipe','pipe']
    }).trim();
    const m = remote.match(/[:/]([^/]+\/[^/]+?)(?:\.git)?$/);
    if (m) return m[1];
  } catch {}
  return path.basename(repoRoot);
}

/**
 * Generate the ASSISTANT_PROMPT.md content per spec §5c template.
 *
 * @param {{ repo?: string, repoRoot: string }} opts
 * @returns {string}
 */
export function generateAssistantPrompt({ repo, repoRoot }) {
  const repoSlug = repo || detectRepo(repoRoot);

  return [
    `# Assistant Prompt for ${repoSlug}`,
    '',
    `You are an assistant for ${repoSlug}. Answer questions about installation, usage, troubleshooting, and architecture using:`,
    '',
    '- Repo manifest: docs/.llm-manifest.json',
    '- Full docs: llms-full.txt',
    '- Design specs: docs/specs/',
    '',
    'When citing, use the spec_ref field. If a question is outside the manifest, say so.',
    '',
    '## Sample Q&A pairs',
    '',
    '[LLM-generated, marked needs_review where confidence < high]',
    '',
  ].join('\n');
}

/**
 * Write ASSISTANT_PROMPT.md (idempotent).
 */
export async function writeAssistantPrompt({ repo, repoRoot, outputPath }) {
  const content = generateAssistantPrompt({ repo, repoRoot });
  fs.mkdirSync(path.dirname(outputPath), { recursive: true });

  let existing = null;
  try { existing = fs.readFileSync(outputPath, 'utf8'); } catch {}

  const written = existing !== content;
  if (written) fs.writeFileSync(outputPath, content, 'utf8');
  return { written, path: outputPath };
}

// ── CLI ───────────────────────────────────────────────────────────────────────

function realResolve(p) {
  try { return fs.realpathSync(path.resolve(p)); } catch { return path.resolve(p); }
}
const isMain = process.argv[1] &&
  realResolve(process.argv[1]) === realResolve(fileURLToPath(import.meta.url));

if (isMain) {
  const args = process.argv.slice(2);
  let repoRoot = process.cwd();
  let outputPath = null;
  for (let i = 0; i < args.length; i++) {
    if (args[i] === '--repo-root' && args[i+1]) repoRoot = args[++i];
    if (args[i] === '--output' && args[i+1]) outputPath = args[++i];
  }
  outputPath = outputPath || path.join(repoRoot, 'docs', 'ASSISTANT_PROMPT.md');
  const result = await writeAssistantPrompt({ repoRoot, outputPath });
  process.stderr.write(`[gen-assistant-prompt] ${result.written ? 'written' : 'unchanged'}: ${result.path}\n`);
}
