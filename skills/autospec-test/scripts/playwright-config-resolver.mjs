#!/usr/bin/env node
// playwright-config-resolver.mjs — resolve Playwright config from a target repo.
//
// Usage: node playwright-config-resolver.mjs <repo_root>
//
// Output JSON (stdout):
//   {
//     "configPath": "playwright.config.ts",   // relative path found
//     "baseURL": "http://localhost:3000",
//     "useBaseURL": "http://localhost:3000",
//     "webServerURL": "http://localhost:3000",
//     "testDir": "./tests/e2e",
//     "projects": [{ "name": "chromium" }]
//   }
//
// Probes for playwright.config.{ts,js,mjs,cjs} in repo root.
// Falls back to env vars: E2E_BASE_URL, PLAYWRIGHT_BASE_URL, BASE_URL.
//
// Exit codes: 0=ok, 1=fatal

import { existsSync, readFileSync } from 'fs';
import { join, resolve } from 'path';
import { createRequire } from 'module';

const require = createRequire(import.meta.url);

const [,, repoRoot] = process.argv;

if (!repoRoot) {
  process.stderr.write('Usage: playwright-config-resolver.mjs <repo_root>\n');
  process.exit(1);
}

const absRoot = resolve(repoRoot);
if (!existsSync(absRoot)) {
  process.stderr.write(`playwright-config-resolver: fatal: repo_root not found: ${repoRoot}\n`);
  process.exit(1);
}

// ── Config file probe ─────────────────────────────────────────────────────────
const CONFIG_EXTS = ['ts', 'js', 'mjs', 'cjs'];
let configPath = null;

for (const ext of CONFIG_EXTS) {
  const candidate = join(absRoot, `playwright.config.${ext}`);
  if (existsSync(candidate)) {
    configPath = candidate;
    break;
  }
}

// ── Parse config ──────────────────────────────────────────────────────────────
let baseURL = null;
let useBaseURL = null;
let webServerURL = null;
let testDir = null;
let projects = [];

if (configPath) {
  const content = readFileSync(configPath, 'utf8');

  // Extract baseURL from use.baseURL — regex-based (avoids dynamic import issues)
  const baseURLMatch = content.match(/use\s*:\s*\{[^}]*baseURL\s*:\s*['"`]([^'"`]+)['"`]/s);
  if (baseURLMatch) {
    useBaseURL = baseURLMatch[1];
    baseURL = useBaseURL;
  }

  // Also check top-level baseURL
  const topBaseURLMatch = content.match(/(?<!use\s*:\s*\{[^}]*)baseURL\s*:\s*['"`]([^'"`]+)['"`]/);
  if (topBaseURLMatch && !baseURL) {
    baseURL = topBaseURLMatch[1];
  }

  // webServer.url
  const webServerMatch = content.match(/webServer\s*:\s*\{[^}]*url\s*:\s*['"`]([^'"`]+)['"`]/s);
  if (webServerMatch) {
    webServerURL = webServerMatch[1];
    if (!baseURL) baseURL = webServerURL;
  }

  // testDir
  const testDirMatch = content.match(/testDir\s*:\s*['"`]([^'"`]+)['"`]/);
  if (testDirMatch) {
    testDir = testDirMatch[1];
  }

  // projects — extract project names
  const projectMatches = [...content.matchAll(/name\s*:\s*['"`]([^'"`]+)['"`]/g)];
  projects = projectMatches.map(m => ({ name: m[1] }));
}

// ── Env var fallback ──────────────────────────────────────────────────────────
if (!baseURL) {
  baseURL = process.env.E2E_BASE_URL
    || process.env.PLAYWRIGHT_BASE_URL
    || process.env.BASE_URL
    || null;
}

// ── Output ────────────────────────────────────────────────────────────────────
const result = {
  configPath: configPath ? configPath.replace(absRoot + '/', '') : null,
  baseURL,
  useBaseURL,
  webServerURL,
  testDir,
  projects,
};

process.stdout.write(JSON.stringify(result, null, 2) + '\n');
process.exit(0);
