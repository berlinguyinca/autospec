// ui-device-evidence.test.mjs — runtime device evidence (design spec L4a).
//
// The probe fixtures are recorded from a real Chromium run: iPhone 13 reports
// coarse=true, noHover=true, dpr=3; a 320px viewport over a fixed 900px block reports
// scrollWidth 908 against clientWidth 320. Assertions tested only against invented probe
// shapes would prove the assertions agree with themselves.
//
// Tests:
//   1. overflowsHorizontally: a pixel of rounding slack is not overflow
//   2. undersizedTargets: only reported for a coarse pointer
//   3. undersizedTargets: a 44px target passes, a 16px one does not
//   4. judgeProfile: a responsive route on a phone passes
//   5. judgeProfile: DEVICE_OVERFLOW when content is wider than the viewport
//   6. judgeProfile: DEVICE_TARGET_TOO_SMALL names the smallest offender
//   7. judgeProfile: DEVICE_HOVER_ONLY_INPUT only where the device cannot hover
//   8. judgeWcagRuns: DEVICE_REFLOW at 320px
//   9. judgeWcagRuns: DEVICE_ZOOM_CLIP at 200%
//  10. judgeWcagRuns: a responsive route draws neither
//  11. real browser: a responsive page passes, a fixed-width one is flagged on both runs
//  12. real browser: a page of 12px buttons is flagged only on the touch profile

import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { createServer } from 'node:http';

import {
  overflowsHorizontally,
  undersizedTargets,
  judgeProfile,
  judgeWcagRuns,
  collectEvidence,
  loadPlaywright,
} from '../../scripts/ui-device-evidence.mjs';

// Recorded: pilot index.html under the iPhone 13 descriptor.
const PHONE_OK = {
  coarse: true, noHover: true, dpr: 3,
  scrollWidth: 390, clientWidth: 390,
  targets: [{ label: 'Retry run 1840', width: 120, height: 48 }],
  hoverOnlyControls: 0,
};

// Recorded: a fixed 900px block at 320px.
const NARROW_OVERFLOW = {
  coarse: false, noHover: false, dpr: 1,
  scrollWidth: 908, clientWidth: 320, targets: [], hoverOnlyControls: 0,
};

const RESPONSIVE_320 = {
  coarse: false, noHover: false, dpr: 1,
  scrollWidth: 320, clientWidth: 320, targets: [], hoverOnlyControls: 0,
};

const RESPONSIVE_640 = {
  coarse: false, noHover: false, dpr: 1,
  scrollWidth: 640, clientWidth: 640, targets: [], hoverOnlyControls: 0,
};

test('a pixel of rounding slack is not overflow', () => {
  assert.equal(overflowsHorizontally({ scrollWidth: 391, clientWidth: 390 }), false);
  assert.equal(overflowsHorizontally({ scrollWidth: 392, clientWidth: 390 }), true);
  assert.equal(overflowsHorizontally(RESPONSIVE_320), false);
  assert.equal(overflowsHorizontally(NARROW_OVERFLOW), true);
});

test('undersized targets are only reported for a coarse pointer', () => {
  const small = [{ label: 'x', width: 16, height: 16 }];
  assert.equal(undersizedTargets({ coarse: false, targets: small }).length, 0);
  assert.equal(undersizedTargets({ coarse: true, targets: small }).length, 1);
});

test('a 44px target passes and a 16px one does not', () => {
  const targets = [
    { label: 'big', width: 120, height: 44 },
    { label: 'tiny', width: 16, height: 16 },
    { label: 'thin', width: 120, height: 12 },
  ];
  const found = undersizedTargets({ coarse: true, targets });
  assert.deepEqual(found.map((t) => t.label), ['tiny', 'thin']);
});

test('a responsive route on a phone passes', () => {
  assert.deepEqual(judgeProfile('/', 'iPhone 13', PHONE_OK), []);
});

test('DEVICE_OVERFLOW fires when content is wider than the viewport', () => {
  const findings = judgeProfile('/', 'iPhone 13', { ...PHONE_OK, scrollWidth: 900, clientWidth: 390 });
  assert.equal(findings.length, 1);
  assert.equal(findings[0].rule, 'DEVICE_OVERFLOW');
  assert.match(findings[0].detail, /900px wide in a 390px viewport/);
});

test('DEVICE_TARGET_TOO_SMALL names the smallest offender', () => {
  const findings = judgeProfile('/', 'iPhone 13', {
    ...PHONE_OK,
    targets: [{ label: 'close', width: 16, height: 16 }, { label: 'ok', width: 48, height: 48 }],
  });
  assert.equal(findings.length, 1);
  assert.equal(findings[0].rule, 'DEVICE_TARGET_TOO_SMALL');
  assert.match(findings[0].detail, /'close' at 16x16/);
});

test('DEVICE_HOVER_ONLY_INPUT fires only where the device cannot hover', () => {
  const hoverOnly = { ...PHONE_OK, hoverOnlyControls: 2 };
  const onPhone = judgeProfile('/', 'iPhone 13', hoverOnly);
  assert.equal(onPhone.length, 1);
  assert.equal(onPhone[0].rule, 'DEVICE_HOVER_ONLY_INPUT');

  const onDesktop = judgeProfile('/', 'Desktop Chrome', { ...hoverOnly, coarse: false, noHover: false });
  assert.deepEqual(onDesktop, []);
});

test('DEVICE_REFLOW fires at 320px', () => {
  const findings = judgeWcagRuns('/', NARROW_OVERFLOW, RESPONSIVE_640);
  assert.equal(findings.length, 1);
  assert.equal(findings[0].rule, 'DEVICE_REFLOW');
  assert.match(findings[0].detail, /1\.4\.10/);
});

test('DEVICE_ZOOM_CLIP fires at 200%', () => {
  const clipped = { ...RESPONSIVE_640, scrollWidth: 908 };
  const findings = judgeWcagRuns('/', RESPONSIVE_320, clipped);
  assert.equal(findings.length, 1);
  assert.equal(findings[0].rule, 'DEVICE_ZOOM_CLIP');
  assert.match(findings[0].detail, /1\.4\.4/);
});

test('a responsive route draws neither WCAG finding', () => {
  assert.deepEqual(judgeWcagRuns('/', RESPONSIVE_320, RESPONSIVE_640), []);
});

// ── real browser ──────────────────────────────────────────────────────────────

const RESPONSIVE_PAGE = `<!doctype html><html><head>
<meta name="viewport" content="width=device-width, initial-scale=1">
<style>
  body { margin: 0; }
  .wrap { max-width: 72ch; margin: 0 auto; }
  .btn { min-width: 44px; min-height: 44px; }
  .btn:hover, .btn:focus-visible { text-decoration: underline; }
</style></head>
<body><div class="wrap"><button class="btn">Retry the failed run</button></div></body></html>`;

const FIXED_PAGE = `<!doctype html><html><head>
<meta name="viewport" content="width=device-width, initial-scale=1">
<style>body { margin: 0; } .wide { width: 900px; }</style></head>
<body><div class="wide">fixed width</div></body></html>`;

const TINY_TARGETS_PAGE = `<!doctype html><html><head>
<meta name="viewport" content="width=device-width, initial-scale=1">
<style>body { margin: 0; } .tiny { width: 12px; height: 12px; padding: 0; border: 0; }</style></head>
<body><button class="tiny">x</button></body></html>`;

function serveDir(root) {
  const types = { '.html': 'text/html', '.css': 'text/css' };
  const server = createServer((req, res) => {
    const rel = decodeURIComponent(req.url.split('?')[0]);
    const file = path.join(root, rel === '/' ? 'index.html' : rel);
    if (!file.startsWith(root) || !fs.existsSync(file)) {
      res.writeHead(404).end('not found');
      return;
    }
    res.writeHead(200, { 'content-type': types[path.extname(file)] || 'text/plain' });
    res.end(fs.readFileSync(file));
  });
  return new Promise((resolve) => {
    server.listen(0, '127.0.0.1', () => resolve({ server, port: server.address().port }));
  });
}

test('real browser: a responsive page passes and a fixed-width one is flagged', async (t) => {
  const playwright = await loadPlaywright();
  if (!playwright) {
    t.skip('Playwright is not installed on this host');
    return;
  }

  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'ui-device-'));
  fs.writeFileSync(path.join(dir, 'index.html'), RESPONSIVE_PAGE);
  fs.writeFileSync(path.join(dir, 'fixed.html'), FIXED_PAGE);
  const { server, port } = await serveDir(dir);

  try {
    const report = await collectEvidence(
      `http://127.0.0.1:${port}`, ['/', '/fixed.html'], ['iPhone 13', 'Desktop Chrome'],
    );
    assert.equal(report.status, 'ok');

    assert.deepEqual(report.findings.filter((f) => f.route === '/'), [],
      'the responsive route should draw nothing');

    const onFixed = report.findings.filter((f) => f.route === '/fixed.html').map((f) => f.rule);
    assert.ok(onFixed.includes('DEVICE_REFLOW'), 'expected a 320px reflow finding');
    assert.ok(onFixed.includes('DEVICE_ZOOM_CLIP'), 'expected a 200% zoom finding');
    assert.ok(onFixed.includes('DEVICE_OVERFLOW'), 'expected a device-profile overflow finding');
  } finally {
    server.close();
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test('real browser: a hover affordance with no focus pair is flagged on touch', async (t) => {
  // The positive control for the case above: the responsive fixture pairs :hover and
  // :focus-visible in one grouped rule and must stay clean, while this one omits the
  // focus half and must not.
  const playwright = await loadPlaywright();
  if (!playwright) {
    t.skip('Playwright is not installed on this host');
    return;
  }

  const page = `<!doctype html><html><head>
<meta name="viewport" content="width=device-width, initial-scale=1">
<style>body { margin: 0; } .btn { min-width: 44px; min-height: 44px; }
  .btn:hover { text-decoration: underline; }</style></head>
<body><button class="btn">Retry the failed run</button></body></html>`;

  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'ui-hover-'));
  fs.writeFileSync(path.join(dir, 'index.html'), page);
  const { server, port } = await serveDir(dir);

  try {
    const report = await collectEvidence(
      `http://127.0.0.1:${port}`, ['/'], ['iPhone 13', 'Desktop Chrome'],
    );
    const hover = report.findings.filter((f) => f.rule === 'DEVICE_HOVER_ONLY_INPUT');
    assert.equal(hover.length, 1, 'only the touch profile should report it');
    assert.equal(hover[0].profile, 'iPhone 13');
  } finally {
    server.close();
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test('real browser: tiny targets are flagged on touch and not on desktop', async (t) => {
  const playwright = await loadPlaywright();
  if (!playwright) {
    t.skip('Playwright is not installed on this host');
    return;
  }

  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'ui-target-'));
  fs.writeFileSync(path.join(dir, 'index.html'), TINY_TARGETS_PAGE);
  const { server, port } = await serveDir(dir);

  try {
    const report = await collectEvidence(
      `http://127.0.0.1:${port}`, ['/'], ['iPhone 13', 'Desktop Chrome'],
    );
    const small = report.findings.filter((f) => f.rule === 'DEVICE_TARGET_TOO_SMALL');
    assert.equal(small.length, 1, 'exactly one profile should report it');
    assert.equal(small[0].profile, 'iPhone 13');
  } finally {
    server.close();
    fs.rmSync(dir, { recursive: true, force: true });
  }
});
