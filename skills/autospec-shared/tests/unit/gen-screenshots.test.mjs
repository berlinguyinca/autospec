// gen-screenshots.test.mjs — unit tests for Phase 7 screenshot + transcript capture.
//
// Tests:
//   1. routeToSlug: "/" → "root", "/about/team" → "about-team"
//   2. cmdToSlug: command string → filename-safe slug
//   3. checkForbiddenUrl: clean URL → no violations
//   4. checkForbiddenUrl: forbidden URL → violations returned
//   5. captureScreenshots: Mode II forbidden URL → aborts with exit violations (no Playwright invoked)
//   6. captureScreenshots: real Playwright + fixture HTML → 2 PNGs per route (desktop + mobile)
//   7. captureScreenshots: writes files to correct paths (<slug>__desktop.png, <slug>__mobile.png)
//   8. captureTranscripts: asciinema absent → fallback to script -c invoked
//   9. serveFixture: serves HTML file on localhost, returns URL + close fn

import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const SCRIPTS_DIR = path.resolve(__dirname, '../../scripts');
const FIXTURES_DIR = path.resolve(__dirname, '../fixtures');

const {
  routeToSlug,
  cmdToSlug,
  checkForbiddenUrl,
  captureScreenshots,
  captureTranscripts,
  serveFixture,
  findPlaywrightPath,
  hasAsciinema,
  hasScript,
  VIEWPORTS,
} = await import(path.join(SCRIPTS_DIR, 'gen-screenshots.mjs'));

// ── routeToSlug tests ─────────────────────────────────────────────────────────

test('routeToSlug: "/" → "root"', () => {
  assert.strictEqual(routeToSlug('/'), 'root');
});

test('routeToSlug: "/about" → "about"', () => {
  assert.strictEqual(routeToSlug('/about'), 'about');
});

test('routeToSlug: "/about/team" → "about-team"', () => {
  assert.strictEqual(routeToSlug('/about/team'), 'about-team');
});

test('routeToSlug: empty string → "root"', () => {
  assert.strictEqual(routeToSlug(''), 'root');
});

// ── cmdToSlug tests ───────────────────────────────────────────────────────────

test('cmdToSlug: simple command → slug', () => {
  const slug = cmdToSlug('autospec-run --profile foo');
  assert.ok(typeof slug === 'string', 'must return string');
  assert.ok(slug.length > 0, 'must be non-empty');
  assert.ok(!/\s/.test(slug), 'must have no whitespace');
});

test('cmdToSlug: long command → truncated to 80 chars', () => {
  const long = 'a'.repeat(200);
  assert.ok(cmdToSlug(long).length <= 80, 'must truncate to 80 chars');
});

// ── checkForbiddenUrl tests ───────────────────────────────────────────────────

test('checkForbiddenUrl: empty patterns → no violations', async () => {
  const result = await checkForbiddenUrl('http://localhost:3000', []);
  assert.deepStrictEqual(result.violations, []);
});

test('checkForbiddenUrl: non-matching URL → no violations', async () => {
  const result = await checkForbiddenUrl('http://localhost:3000', ['production\\.example\\.com']);
  assert.deepStrictEqual(result.violations, []);
});

test('checkForbiddenUrl: matching URL → violation returned', async () => {
  const result = await checkForbiddenUrl('http://production.example.com/app', ['production\\.example\\.com']);
  assert.ok(result.violations.length > 0, 'must return at least one violation');
  assert.ok(result.violations[0].pattern === 'production\\.example\\.com', 'must report matching pattern');
});

// ── Mode II abort test (no Playwright) ───────────────────────────────────────

test('captureScreenshots: forbidden URL → violations returned, no PNG created', async () => {
  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'autospec-ss-test-'));
  try {
    const result = await captureScreenshots({
      baseUrl: 'http://production.example.com',
      routes: ['/'],
      forbiddenPatterns: ['production\\.example\\.com'],
      outputDir: tmpDir,
    });
    assert.ok(result.violations.length > 0, 'must return violations');
    assert.strictEqual(result.captured.length, 0, 'must not capture any screenshots');
    // No PNG files written
    const files = fs.readdirSync(tmpDir);
    assert.strictEqual(files.length, 0, 'no PNG files must be written when forbidden URL detected');
  } finally {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  }
});

// ── serveFixture test ─────────────────────────────────────────────────────────

test('serveFixture: serves HTML file on localhost, returns URL', async () => {
  const fixturePath = path.join(FIXTURES_DIR, 'route-sample.html');
  const { url, close } = await serveFixture(fixturePath);
  try {
    assert.ok(url.startsWith('http://127.0.0.1:'), 'must return localhost URL');
    // Fetch the served content
    const { default: http } = await import('node:http');
    await new Promise((resolve, reject) => {
      http.get(url, (res) => {
        let data = '';
        res.on('data', (chunk) => { data += chunk; });
        res.on('end', () => {
          assert.ok(data.includes('Autospec Route Sample'), 'must serve fixture HTML content');
          resolve();
        });
      }).on('error', reject);
    });
  } finally {
    close();
  }
});

// ── Real Playwright screenshot test ──────────────────────────────────────────

test('captureScreenshots: real Playwright + fixture → 2 PNGs per route (desktop + mobile)', async () => {
  const playwrightPath = findPlaywrightPath();
  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'autospec-ss-real-'));
  if (!playwrightPath) {
    await assert.rejects(
      () => captureScreenshots({
        baseUrl: 'http://127.0.0.1:1',
        routes: ['/'],
        forbiddenPatterns: [],
        outputDir: tmpDir,
      }),
      /Playwright not found/,
      'missing Playwright must produce the documented installation error'
    );
    fs.rmSync(tmpDir, { recursive: true, force: true });
    return;
  }

  const fixturePath = path.join(FIXTURES_DIR, 'route-sample.html');
  let fixtureServer = null;

  try {
    fixtureServer = await serveFixture(fixturePath);
    const result = await captureScreenshots({
      baseUrl: fixtureServer.url,
      routes: ['/'],
      forbiddenPatterns: [],
      outputDir: tmpDir,
      viewports: VIEWPORTS,
    });

    assert.strictEqual(result.violations.length, 0, 'must have no violations for fixture URL');
    assert.strictEqual(result.captured.length, 2, 'must capture 2 screenshots (desktop + mobile)');

    // Check filenames: root__desktop.png and root__mobile.png
    const basenames = result.captured.map(f => path.basename(f)).sort();
    assert.deepStrictEqual(basenames, ['root__desktop.png', 'root__mobile.png']);

    // Check files exist and are non-empty PNGs
    for (const file of result.captured) {
      assert.ok(fs.existsSync(file), `screenshot file must exist: ${file}`);
      const stat = fs.statSync(file);
      assert.ok(stat.size > 100, `screenshot must be non-trivial size: ${file}`);
      // PNG magic bytes: 89 50 4E 47
      const buf = Buffer.alloc(4);
      const fd = fs.openSync(file, 'r');
      fs.readSync(fd, buf, 0, 4, 0);
      fs.closeSync(fd);
      assert.strictEqual(buf[0], 0x89, 'must be PNG (magic byte 0)');
      assert.strictEqual(buf[1], 0x50, 'must be PNG (magic byte 1)');
      assert.strictEqual(buf[2], 0x4E, 'must be PNG (magic byte 2)');
      assert.strictEqual(buf[3], 0x47, 'must be PNG (magic byte 3)');
    }
  } finally {
    if (fixtureServer) fixtureServer.close();
    fs.rmSync(tmpDir, { recursive: true, force: true });
  }
});

// ── CLI transcript fallback test ──────────────────────────────────────────────

test('captureTranscripts: records a transcript or reports unavailable tools', () => {
  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'autospec-transcript-'));
  try {
    // Use a simple, fast command that always succeeds
    const cmds = ['echo autospec-transcript-test'];
    const result = captureTranscripts(cmds, tmpDir);

    if (!hasAsciinema() && !hasScript()) {
      assert.deepStrictEqual(result, { recorded: [], tool: 'none' });
      return;
    }

    assert.ok(['asciinema', 'script'].includes(result.tool), 'must report the selected recorder');
    assert.strictEqual(result.recorded.length, 1, 'one command must produce one transcript');
    assert.ok(fs.existsSync(result.recorded[0]), 'recorded transcript must exist');
    assert.ok(fs.statSync(result.recorded[0]).size > 0, 'recorded transcript must be non-empty');
  } finally {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  }
});
