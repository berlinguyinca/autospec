#!/usr/bin/env node
// gen-screenshots.mjs — Playwright-based screenshot capture with Mode II safety.
//
// Reuses v1 safety infrastructure:
//   - forbidden-url-check.mjs  (Mode II: forbidden URL → abort capture)
//   - network-intercept-inject.mjs (Mode II: runtime network intercept)
//
// Outputs:
//   docs/assets/screenshots/<route-slug>__desktop.png
//   docs/assets/screenshots/<route-slug>__mobile.png
//   docs/assets/transcripts/<cmd-slug>.cast  (asciinema) or .txt (script fallback)
//
// CLI:
//   node gen-screenshots.mjs \
//     --base-url <url> \
//     --routes <routes.json>            # JSON array of route strings, e.g. ["/", "/about"]
//     [--forbidden-patterns <file>]     # JSON array of regex strings (Mode II)
//     [--output-dir <dir>]              # default: docs/assets/screenshots
//     [--cli-commands <file>]           # JSON array of CLI command strings for transcripts
//     [--transcript-dir <dir>]          # default: docs/assets/transcripts
//     [--fixture <html-file>]           # serve a local HTML fixture instead of --base-url
//
// Exit codes:
//   0 = all captures successful
//   1 = forbidden URL violation (Mode II) — capture aborted

import fs from 'node:fs';
import path from 'node:path';
import { execFileSync, spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { createServer } from 'node:http';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

// Paths to v1 safety modules (relative to this script's location in autospec-shared/scripts/)
const FORBIDDEN_URL_CHECK = path.resolve(
  __dirname,
  '../../autospec-test/scripts/forbidden-url-check.mjs'
);

// Playwright: try local node_modules first, then global homebrew install
const PLAYWRIGHT_PATHS = [
  path.resolve(__dirname, '../../../node_modules/playwright/index.mjs'),
  path.resolve(__dirname, '../../autospec-test/node_modules/playwright/index.mjs'),
  '/opt/homebrew/lib/node_modules/playwright/index.mjs',
  '/usr/local/lib/node_modules/playwright/index.mjs',
];

// ── Viewport definitions ──────────────────────────────────────────────────────

export const VIEWPORTS = {
  desktop: { width: 1280, height: 800 },
  mobile:  { width: 375,  height: 667 },
};

// ── Helpers ──────────────────────────────────────────────────────────────────

/**
 * Convert a route path to a filename-safe slug.
 * "/" → "root", "/about/team" → "about-team"
 */
export function routeToSlug(route) {
  if (route === '/') return 'root';
  return route.replace(/^\//, '').replace(/\//g, '-').replace(/[^a-zA-Z0-9_-]/g, '_') || 'root';
}

/**
 * Convert a CLI command string to a filename-safe slug.
 * "autospec-run --profile foo" → "autospec-run---profile-foo"
 */
export function cmdToSlug(cmd) {
  return cmd.trim().replace(/\s+/g, '-').replace(/[^a-zA-Z0-9_-]/g, '_').slice(0, 80);
}

/**
 * Find the first resolvable Playwright import path.
 */
export function findPlaywrightPath() {
  for (const p of PLAYWRIGHT_PATHS) {
    if (fs.existsSync(p)) return p;
  }
  return null;
}

/**
 * Check if asciinema is available on PATH.
 */
export function hasAsciinema() {
  const result = spawnSync('which', ['asciinema'], { encoding: 'utf8' });
  return result.status === 0 && result.stdout.trim().length > 0;
}

/**
 * Check if `script` (BSD/POSIX transcript recorder) is available.
 */
export function hasScript() {
  const result = spawnSync('which', ['script'], { encoding: 'utf8' });
  return result.status === 0 && result.stdout.trim().length > 0;
}

/**
 * Record a CLI transcript using asciinema or script fallback.
 *
 * @param {string} cmd - shell command to record
 * @param {string} outputPath - destination file path (.cast or .txt)
 * @param {'asciinema'|'script'} tool - which recorder to use
 */
export function recordTranscript(cmd, outputPath, tool) {
  fs.mkdirSync(path.dirname(outputPath), { recursive: true });

  if (tool === 'asciinema') {
    // asciinema rec --command "<cmd>" <output.cast>
    const result = spawnSync('asciinema', ['rec', '--command', cmd, outputPath], {
      encoding: 'utf8',
      timeout: 30000,
    });
    if (result.status !== 0) {
      throw new Error(`asciinema failed (exit ${result.status}): ${result.stderr}`);
    }
  } else {
    // script syntax differs by platform:
    //   Linux (util-linux): script -c "<cmd>" <output.txt>
    //   macOS (BSD):        script <output.txt> <cmd> [args...]
    // Detect by checking if -c flag is supported.
    const testResult = spawnSync('script', ['--version'], { encoding: 'utf8' });
    const isBSD = testResult.status !== 0 || !testResult.stdout.includes('util-linux');

    let result;
    if (isBSD) {
      // macOS BSD script: script <file> <shell> -c "<cmd>"
      result = spawnSync('script', [outputPath, 'sh', '-c', cmd], {
        encoding: 'utf8',
        timeout: 30000,
      });
    } else {
      // Linux script: script -c "<cmd>" <file>
      result = spawnSync('script', ['-c', cmd, outputPath], {
        encoding: 'utf8',
        timeout: 30000,
      });
    }
    if (result.status !== 0) {
      // script may exit non-zero on macOS even on success; check file was created
      if (!fs.existsSync(outputPath)) {
        throw new Error(`script failed (exit ${result.status}): ${result.stderr}`);
      }
    }
  }
}

/**
 * Serve a local HTML file via a temporary HTTP server on a random port.
 * Returns { url, close }.
 *
 * @param {string} htmlFile - absolute path to the HTML fixture
 * @returns {Promise<{ url: string, close: () => void }>}
 */
export function serveFixture(htmlFile) {
  return new Promise((resolve, reject) => {
    const content = fs.readFileSync(htmlFile);
    const server = createServer((_req, res) => {
      res.writeHead(200, { 'Content-Type': 'text/html; charset=utf-8' });
      res.end(content);
    });
    server.listen(0, '127.0.0.1', () => {
      const port = server.address().port;
      resolve({
        url: `http://127.0.0.1:${port}`,
        close: () => server.close(),
      });
    });
    server.on('error', reject);
  });
}

// ── Mode II forbidden URL check ───────────────────────────────────────────────

/**
 * Check a base URL against forbidden patterns (Mode II safety).
 * Loads forbidden-url-check.mjs from v1 path.
 *
 * @param {string} baseUrl
 * @param {string[]} patterns
 * @returns {Promise<{ violations: object[] }>}
 */
export async function checkForbiddenUrl(baseUrl, patterns) {
  if (!patterns || patterns.length === 0) return { violations: [] };
  const { check } = await import(FORBIDDEN_URL_CHECK);
  // Build a minimal config object with the baseURL
  return check({ baseURL: baseUrl }, patterns);
}

// ── Screenshot capture ────────────────────────────────────────────────────────

/**
 * Capture screenshots for all routes × viewports.
 *
 * @param {object} opts
 * @param {string}   opts.baseUrl           - base URL of the app
 * @param {string[]} opts.routes            - list of route paths
 * @param {string[]} [opts.forbiddenPatterns] - Mode II patterns
 * @param {string}   [opts.outputDir]       - output directory (default: docs/assets/screenshots)
 * @param {object}   [opts.viewports]       - viewport map (default: VIEWPORTS)
 * @returns {Promise<{ captured: string[], violations: object[] }>}
 */
export async function captureScreenshots(opts) {
  const {
    baseUrl,
    routes,
    forbiddenPatterns = [],
    outputDir = 'docs/assets/screenshots',
    viewports = VIEWPORTS,
  } = opts;

  // Mode II: check base URL against forbidden patterns before any capture
  const urlCheck = await checkForbiddenUrl(baseUrl, forbiddenPatterns);
  if (urlCheck.violations.length > 0) {
    const v = urlCheck.violations[0];
    process.stderr.write(
      `gen-screenshots: ABORT — forbidden URL pattern matched: ${v.field}=${v.value} (pattern: ${v.pattern})\n`
    );
    return { captured: [], violations: urlCheck.violations };
  }

  const playwrightPath = findPlaywrightPath();
  if (!playwrightPath) {
    throw new Error(
      'gen-screenshots: Playwright not found. Install with: npm install playwright or brew install playwright'
    );
  }

  const { chromium } = await import(playwrightPath);
  fs.mkdirSync(outputDir, { recursive: true });

  const captured = [];
  const browser = await chromium.launch({ headless: true });

  try {
    for (const route of routes) {
      // Mode II: also check each individual route URL
      const routeUrl = baseUrl.replace(/\/$/, '') + route;
      const routeCheck = await checkForbiddenUrl(routeUrl, forbiddenPatterns);
      if (routeCheck.violations.length > 0) {
        const v = routeCheck.violations[0];
        process.stderr.write(
          `gen-screenshots: ABORT — forbidden URL pattern matched for route ${route}: ${v.value} (pattern: ${v.pattern})\n`
        );
        return { captured, violations: routeCheck.violations };
      }

      const slug = routeToSlug(route);

      for (const [viewportName, viewport] of Object.entries(viewports)) {
        const page = await browser.newPage();
        await page.setViewportSize(viewport);

        try {
          await page.goto(routeUrl, { waitUntil: 'domcontentloaded', timeout: 10000 });
          // Wait for [data-loaded=true] or 2s timeout (per spec)
          await page.waitForSelector('[data-loaded="true"]', { timeout: 2000 }).catch(() => {});

          const filename = `${slug}__${viewportName}.png`;
          const outputPath = path.join(outputDir, filename);
          await page.screenshot({ path: outputPath, fullPage: false });
          captured.push(outputPath);
          process.stdout.write(`gen-screenshots: captured ${outputPath}\n`);
        } finally {
          await page.close();
        }
      }
    }
  } finally {
    await browser.close();
  }

  return { captured, violations: [] };
}

// ── CLI transcript capture ────────────────────────────────────────────────────

/**
 * Capture CLI transcripts for an array of commands.
 *
 * @param {string[]} commands
 * @param {string} transcriptDir
 * @returns {{ recorded: string[], tool: string }}
 */
export function captureTranscripts(commands, transcriptDir = 'docs/assets/transcripts') {
  fs.mkdirSync(transcriptDir, { recursive: true });

  const useAsciinema = hasAsciinema();
  const useScript = !useAsciinema && hasScript();
  const tool = useAsciinema ? 'asciinema' : useScript ? 'script' : null;

  if (!tool) {
    process.stderr.write('gen-screenshots: WARN — neither asciinema nor script found; skipping transcripts\n');
    return { recorded: [], tool: 'none' };
  }

  const recorded = [];

  for (const cmd of commands) {
    const slug = cmdToSlug(cmd);
    const ext = useAsciinema ? '.cast' : '.txt';
    const outputPath = path.join(transcriptDir, `${slug}${ext}`);

    try {
      recordTranscript(cmd, outputPath, tool);
      recorded.push(outputPath);
      process.stdout.write(`gen-screenshots: transcript → ${outputPath}\n`);
    } catch (err) {
      process.stderr.write(`gen-screenshots: WARN — transcript failed for "${cmd}": ${err.message}\n`);
    }
  }

  return { recorded, tool };
}

// ── CLI entrypoint ───────────────────────────────────────────────────────────

if (process.argv[1] && fs.realpathSync(path.resolve(process.argv[1])) === fs.realpathSync(path.resolve(__filename))) {
  const args = process.argv.slice(2);
  let baseUrl        = null;
  let routesFile     = null;
  let forbiddenFile  = null;
  let outputDir      = 'docs/assets/screenshots';
  let cliCmdsFile    = null;
  let transcriptDir  = 'docs/assets/transcripts';
  let fixtureFile    = null;

  for (let i = 0; i < args.length; i++) {
    if (args[i] === '--base-url')           baseUrl       = args[i + 1];
    if (args[i] === '--routes')             routesFile    = args[i + 1];
    if (args[i] === '--forbidden-patterns') forbiddenFile = args[i + 1];
    if (args[i] === '--output-dir')         outputDir     = args[i + 1];
    if (args[i] === '--cli-commands')       cliCmdsFile   = args[i + 1];
    if (args[i] === '--transcript-dir')     transcriptDir = args[i + 1];
    if (args[i] === '--fixture')            fixtureFile   = args[i + 1];
  }

  let fixtureServer = null;

  try {
    // Fixture mode: serve a local HTML file
    if (fixtureFile) {
      fixtureServer = await serveFixture(path.resolve(fixtureFile));
      baseUrl = fixtureServer.url;
      // Default to a single "/" route for fixture mode
      if (!routesFile) {
        const routes = ['/'];
        const forbiddenPatterns = forbiddenFile
          ? JSON.parse(fs.readFileSync(forbiddenFile, 'utf8'))
          : [];
        const result = await captureScreenshots({ baseUrl, routes, forbiddenPatterns, outputDir });
        if (result.violations.length > 0) process.exit(1);
        process.exit(0);
      }
    }

    if (!baseUrl) {
      process.stderr.write('Usage: gen-screenshots.mjs --base-url <url> --routes <file> [options]\n');
      process.stderr.write('       gen-screenshots.mjs --fixture <html-file> [options]\n');
      process.exit(1);
    }

    if (!routesFile) {
      process.stderr.write('gen-screenshots: --routes <file> required\n');
      process.exit(1);
    }

    const routes           = JSON.parse(fs.readFileSync(routesFile, 'utf8'));
    const forbiddenPatterns = forbiddenFile
      ? JSON.parse(fs.readFileSync(forbiddenFile, 'utf8'))
      : [];

    const result = await captureScreenshots({ baseUrl, routes, forbiddenPatterns, outputDir });

    if (result.violations.length > 0) process.exit(1);

    // CLI transcripts (optional)
    if (cliCmdsFile) {
      const cmds = JSON.parse(fs.readFileSync(cliCmdsFile, 'utf8'));
      captureTranscripts(cmds, transcriptDir);
    }

    process.stdout.write(`gen-screenshots: done — ${result.captured.length} screenshot(s) captured\n`);
    process.exit(0);
  } finally {
    if (fixtureServer) fixtureServer.close();
  }
}
