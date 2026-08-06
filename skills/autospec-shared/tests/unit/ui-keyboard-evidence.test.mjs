// ui-keyboard-evidence.test.mjs — keyboard robot (design spec L4c, first phase).
//
// The probe fixtures are recorded from a real Chromium traversal of
// berlinguyinca/autospec-ui-pilot: a skip link and two links report outline style `auto`
// (the browser default ring), buttons and the input report `solid`, and a page that sets
// `outline: none` reports style `none` while keeping a 3px width — which is why the
// assertion reads style rather than width.
//
// Tests:
//   1. hasVisibleFocus: browser default ring counts, suppressed outline does not
//   2. hasVisibleFocus: a box-shadow ring counts
//   3. orderJumps: a small variation within a row is not a jump
//   4. orderJumps: moving back up the page is
//   5. judgeRoute: a clean traversal draws nothing
//   6. judgeRoute: KEYBOARD_TRAP when a short cycle leaves controls unreached
//   7. judgeRoute: NO_KEYBOARD_PATH when content is unreached without cycling
//   8. judgeRoute: FOCUS_NOT_VISIBLE per stop
//   9. judgeRoute: FOCUS_OBSCURED names what covers the control
//  10. real browser: the pilot page traverses clean
//  11. real browser: a focus trap is caught
//  12. real browser: a suppressed focus ring is caught
//  13. real browser: a control under a sticky header is caught

import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { createServer } from 'node:http';

import {
  hasVisibleFocus,
  orderJumps,
  firstCycle,
  reachedKeys,
  judgeRoute,
  collectEvidence,
  loadPlaywright,
} from '../../scripts/ui-keyboard-evidence.mjs';

const stop = (key, over = {}) => ({
  key, outlineStyle: 'solid', boxShadow: '', rect: { x: 0, y: 0 }, covered: false, coveredBy: '', ...over,
});

test('the browser default ring counts as visible focus', () => {
  assert.equal(hasVisibleFocus({ outlineStyle: 'auto', boxShadow: '' }), true);
  assert.equal(hasVisibleFocus({ outlineStyle: 'solid', boxShadow: '' }), true);
  assert.equal(hasVisibleFocus({ outlineStyle: 'none', boxShadow: '' }), false);
});

test('a box-shadow ring counts even with no outline', () => {
  assert.equal(hasVisibleFocus({ outlineStyle: 'none', boxShadow: 'yes' }), true);
});

test('variation within a row is not an order jump', () => {
  const stops = [stop('a', { rect: { x: 0, y: 100 } }), stop('b', { rect: { x: 200, y: 92 } })];
  assert.deepEqual(orderJumps(stops), []);
});

test('moving back up the page is an order jump', () => {
  const stops = [stop('footer', { rect: { x: 0, y: 900 } }), stop('header', { rect: { x: 0, y: 40 } })];
  const jumps = orderJumps(stops);
  assert.equal(jumps.length, 1);
  assert.equal(jumps[0].from, 'footer');
  assert.equal(jumps[0].to, 'header');
});

test('the tab wrap back to the top is not an order jump', () => {
  // Recorded from a real traversal: the last button sits at y=79 and Tab returns to the
  // skip link at y=24. Judged naively that is a move back up the page, and it would fail
  // every correctly ordered page there is.
  const stops = [
    stop('a:Skip', { rect: { x: 16, y: 24 } }),
    stop('button:Retry', { rect: { x: 16, y: 55 } }),
    stop('button:Apply', { rect: { x: 16, y: 79 } }),
    stop('a:Skip', { rect: { x: 16, y: 24 } }),
    stop('button:Retry', { rect: { x: 16, y: 55 } }),
  ];
  assert.deepEqual(orderJumps(stops), []);
});

test('a clean traversal draws nothing', () => {
  const stops = [
    stop('a:Skip to main content', { rect: { x: 24, y: 24 }, outlineStyle: 'auto' }),
    stop('a:Open run 1841', { rect: { x: 537, y: 284 }, outlineStyle: 'auto' }),
    stop('button:Retry run 1840', { rect: { x: 369, y: 374 } }),
  ];
  const candidates = ['a:Skip to main content', 'a:Open run 1841', 'button:Retry run 1840'];
  assert.deepEqual(judgeRoute('/', stops, candidates), []);
});

test('KEYBOARD_TRAP when a short cycle leaves controls unreached', () => {
  // Recorded shape: the trap fixture cycles m1, m2, m1, m2, m1, m2 while #a and #b exist.
  const stops = ['m1', 'm2', 'm1', 'm2', 'm1', 'm2'].map((k) => stop(`button:${k}`));
  const candidates = ['button:a', 'button:m1', 'button:m2', 'button:b'];
  const findings = judgeRoute('/', stops, candidates);
  assert.equal(findings.length, 1);
  assert.equal(findings[0].rule, 'KEYBOARD_TRAP');
  assert.match(findings[0].detail, /never reaches 2 other/);
});

test('NO_KEYBOARD_PATH when content is unreached without cycling', () => {
  const stops = [stop('button:a'), stop('button:b')];
  const candidates = ['button:a', 'button:b', 'button:hidden-from-tab'];
  const findings = judgeRoute('/', stops, candidates);
  assert.equal(findings.length, 1);
  assert.equal(findings[0].rule, 'NO_KEYBOARD_PATH');
});

test('FOCUS_NOT_VISIBLE is reported per stop', () => {
  const stops = [stop('button:one', { outlineStyle: 'none' }), stop('button:two', { outlineStyle: 'none' })];
  const findings = judgeRoute('/', stops, ['button:one', 'button:two']);
  assert.equal(findings.length, 2);
  assert.ok(findings.every((f) => f.rule === 'FOCUS_NOT_VISIBLE'));
  assert.match(findings[0].detail, /2\.4\.7/);
});

test('FOCUS_OBSCURED names what covers the control', () => {
  const stops = [stop('button:under', { covered: true, coveredBy: 'div.header' })];
  const findings = judgeRoute('/', stops, ['button:under']);
  assert.equal(findings.length, 1);
  assert.equal(findings[0].rule, 'FOCUS_OBSCURED');
  assert.match(findings[0].detail, /painted over by 'div\.header'/);
  assert.match(findings[0].detail, /2\.4\.11/);
});

test('reachedKeys ignores stops where focus left the document', () => {
  assert.deepEqual([...reachedKeys([stop('a'), { none: true }, stop('b')])], ['a', 'b']);
});

// ── real browser ──────────────────────────────────────────────────────────────

const CLEAN_PAGE = `<!doctype html><html><head><style>
  body { margin: 0; padding: 16px; }
  a, button { display: block; margin: 8px 0; }
  a:focus-visible, button:focus-visible { outline: 3px solid #0055cc; outline-offset: 2px; }
</style></head><body>
  <a href="#main">Skip to main content</a>
  <main id="main"><button>Retry the failed run</button><button>Apply the filter</button></main>
</body></html>`;

const TRAP_PAGE = `<!doctype html><html><body>
  <button id="a">outside before</button>
  <div id="modal"><button id="m1">inside one</button><button id="m2">inside two</button></div>
  <button id="b">outside after</button>
  <script>
    document.addEventListener('focusin', (e) => {
      const modal = document.getElementById('modal');
      if (!modal.contains(e.target)) document.getElementById('m1').focus();
    });
  </script>
</body></html>`;

const NO_RING_PAGE = `<!doctype html><html><head><style>
  button:focus, button:focus-visible { outline: none; box-shadow: none; }
</style></head><body><button>one</button><button>two</button></body></html>`;

const OBSCURED_PAGE = `<!doctype html><html><head><style>
  body { margin: 0; }
  .header { position: fixed; top: 0; left: 0; right: 0; height: 120px; background: #222; z-index: 100; }
  .spacer { height: 60px; }
  button { display: block; margin: 8px 16px; }
  button:focus-visible { outline: 3px solid #0055cc; }
</style></head><body>
  <div class="header"></div><div class="spacer"></div>
  <button id="under">under the header</button>
</body></html>`;

function serveDir(root) {
  const server = createServer((req, res) => {
    const rel = decodeURIComponent(req.url.split('?')[0]);
    const file = path.join(root, rel === '/' ? 'index.html' : rel);
    if (!file.startsWith(root) || !fs.existsSync(file)) { res.writeHead(404).end('nope'); return; }
    res.writeHead(200, { 'content-type': 'text/html' });
    res.end(fs.readFileSync(file));
  });
  return new Promise((resolve) => {
    server.listen(0, '127.0.0.1', () => resolve({ server, port: server.address().port }));
  });
}

async function withPages(pages, fn) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'ui-kbd-'));
  for (const [name, html] of Object.entries(pages)) fs.writeFileSync(path.join(dir, name), html);
  const { server, port } = await serveDir(dir);
  try {
    return await fn(`http://127.0.0.1:${port}`);
  } finally {
    server.close();
    fs.rmSync(dir, { recursive: true, force: true });
  }
}

test('real browser: a clean page traverses without findings', async (t) => {
  if (!(await loadPlaywright())) { t.skip('Playwright is not installed on this host'); return; }
  await withPages({ 'index.html': CLEAN_PAGE }, async (base) => {
    const report = await collectEvidence(base, ['/']);
    assert.equal(report.status, 'ok');
    assert.deepEqual(report.findings, []);
    assert.equal(report.routes[0].reached, report.routes[0].candidates);
  });
});

test('real browser: a focus trap is caught', async (t) => {
  if (!(await loadPlaywright())) { t.skip('Playwright is not installed on this host'); return; }
  await withPages({ 'index.html': TRAP_PAGE }, async (base) => {
    const report = await collectEvidence(base, ['/']);
    const trap = report.findings.find((f) => f.rule === 'KEYBOARD_TRAP');
    assert.ok(trap, `expected a trap finding, got ${JSON.stringify(report.findings)}`);
    assert.match(trap.detail, /never reaches/);
  });
});

test('real browser: a suppressed focus ring is caught', async (t) => {
  if (!(await loadPlaywright())) { t.skip('Playwright is not installed on this host'); return; }
  await withPages({ 'index.html': NO_RING_PAGE }, async (base) => {
    const report = await collectEvidence(base, ['/']);
    const invisible = report.findings.filter((f) => f.rule === 'FOCUS_NOT_VISIBLE');
    assert.ok(invisible.length >= 1, 'expected at least one invisible-focus finding');
  });
});

test('real browser: a control under a sticky header is caught', async (t) => {
  if (!(await loadPlaywright())) { t.skip('Playwright is not installed on this host'); return; }
  await withPages({ 'index.html': OBSCURED_PAGE }, async (base) => {
    const report = await collectEvidence(base, ['/']);
    const obscured = report.findings.find((f) => f.rule === 'FOCUS_OBSCURED');
    assert.ok(obscured, `expected an obscured finding, got ${JSON.stringify(report.findings)}`);
    assert.match(obscured.detail, /div\.header/);
  });
});
