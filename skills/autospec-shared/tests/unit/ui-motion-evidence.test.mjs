// ui-motion-evidence.test.mjs — runtime motion evidence (design spec L4b).
//
// The probe fixtures below are recorded from a real Chromium run against
// berlinguyinca/autospec-ui-pilot, not composed by hand. Assertions tested only against
// invented probe shapes would prove the assertions agree with themselves; these prove
// they agree with a browser.
//
// Tests:
//   1. motionProperties: transform counts, background-color does not
//   2. movingAnimations: filters a colour fade out of a mixed list
//   3. judgeRoute: an animated page passes
//   4. judgeRoute: MOTION_ABSENT on a page with nothing moving  (the anti-blandness gate)
//   5. judgeRoute: MOTION_ABSENT on a page whose only animation is a colour fade
//   6. judgeRoute: MOTION_NOT_REDUCED when motion survives the reduce run
//   7. judgeRoute: MOTION_UNBOUNDED for an infinite animation
//   8. judgeRoute: MOTION_UNBOUNDED for a finite animation running past five seconds
//   9. isUnbounded: a short finite animation is bounded
//  10. real browser: an animated fixture passes and an inert one is flagged

import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { createServer } from 'node:http';
import { fileURLToPath } from 'node:url';

import {
  motionProperties,
  movingAnimations,
  isUnbounded,
  judgeRoute,
  collectEvidence,
  loadPlaywright,
} from '../../scripts/ui-motion-evidence.mjs';

// Recorded: pilot index.html, probed at load. Three entry animations.
const ANIMATED = {
  count: 3,
  anims: [
    { id: 'panel-enter', playState: 'running', properties: ['opacity', 'transform'], duration: 320, iterations: 1 },
    { id: 'row-enter', playState: 'running', properties: ['opacity', 'transform'], duration: 180, iterations: 1 },
    { id: 'row-enter', playState: 'running', properties: ['opacity', 'transform'], duration: 180, iterations: 1 },
  ],
};

// Recorded: the same page under reducedMotion: 'reduce'. The @media block sets
// animation-name: none, so nothing is listed at all.
const REDUCED = { count: 0, anims: [] };

// Recorded: a page whose only declaration is a background-color transition.
const INERT = { count: 0, anims: [] };

test('motionProperties keeps movement and drops colour', () => {
  assert.deepEqual(motionProperties({ properties: ['opacity', 'transform'] }), ['transform']);
  assert.deepEqual(motionProperties({ properties: ['background-color'] }), []);
  assert.deepEqual(motionProperties({ properties: ['opacity'] }), []);
  assert.deepEqual(motionProperties({}), []);
});

test('movingAnimations filters a colour fade out of a mixed list', () => {
  const probe = {
    anims: [
      { id: 'fade', properties: ['opacity'] },
      { id: 'slide', properties: ['transform'] },
      { id: 'tint', properties: ['background-color'] },
    ],
  };
  assert.deepEqual(movingAnimations(probe).map((a) => a.id), ['slide']);
});

test('an animated route that stills under reduce passes', () => {
  assert.deepEqual(judgeRoute('/', ANIMATED, REDUCED), []);
});

test('MOTION_ABSENT fires when nothing moves by default', () => {
  const findings = judgeRoute('/', INERT, INERT);
  assert.equal(findings.length, 1);
  assert.equal(findings[0].rule, 'MOTION_ABSENT');
  assert.equal(findings[0].route, '/');
});

test('MOTION_ABSENT fires when the only animation is a colour fade', () => {
  // The case that makes the rule worth having: the page is not literally static, but
  // nothing moves, and a colour crossfade is what a bland implementation produces.
  const colourOnly = {
    count: 1,
    anims: [{ id: 'tint', playState: 'running', properties: ['background-color'], duration: 200, iterations: 1 }],
  };
  const findings = judgeRoute('/', colourOnly, REDUCED);
  assert.equal(findings.length, 1);
  assert.equal(findings[0].rule, 'MOTION_ABSENT');
});

test('MOTION_NOT_REDUCED fires when motion survives the reduce run', () => {
  const findings = judgeRoute('/', ANIMATED, ANIMATED);
  assert.equal(findings.length, 1);
  assert.equal(findings[0].rule, 'MOTION_NOT_REDUCED');
  assert.match(findings[0].detail, /panel-enter/);
});

test('MOTION_UNBOUNDED fires for an infinite animation', () => {
  const spinner = {
    count: 1,
    anims: [{ id: 'spin', playState: 'running', properties: ['transform'], duration: 900, iterations: null }],
  };
  const findings = judgeRoute('/', spinner, REDUCED);
  assert.equal(findings.length, 1);
  assert.equal(findings[0].rule, 'MOTION_UNBOUNDED');
  assert.match(findings[0].detail, /infinite/);
});

test('MOTION_UNBOUNDED fires for a finite animation running past five seconds', () => {
  const slow = {
    count: 1,
    anims: [{ id: 'crawl', playState: 'running', properties: ['transform'], duration: 3000, iterations: 3 }],
  };
  const findings = judgeRoute('/', slow, REDUCED);
  assert.equal(findings.length, 1);
  assert.equal(findings[0].rule, 'MOTION_UNBOUNDED');
});

test('a short finite animation is bounded', () => {
  assert.equal(isUnbounded({ duration: 320, iterations: 1 }), false);
  assert.equal(isUnbounded({ duration: 900, iterations: null }), true);
  assert.equal(isUnbounded({ duration: 2000, iterations: 4 }), true);
});

// ── real browser ──────────────────────────────────────────────────────────────

const ANIMATED_PAGE = `<!doctype html><html><head><style>
  @keyframes rise { from { transform: translateY(8px); opacity: 0; } to { transform: none; opacity: 1; } }
  .card { animation: rise 200ms ease-out both; }
  @media (prefers-reduced-motion: reduce) { .card { animation-name: none; } }
</style></head><body><div class="card">animated</div></body></html>`;

const INERT_PAGE = `<!doctype html><html><head><style>
  .card { background: #ffffff; transition: background-color 200ms linear; }
</style></head><body><div class="card">inert</div></body></html>`;

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

test('real browser: an animated page passes and an inert page is flagged', async (t) => {
  const playwright = await loadPlaywright();
  if (!playwright) {
    t.skip('Playwright is not installed on this host');
    return;
  }

  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'ui-motion-'));
  fs.writeFileSync(path.join(dir, 'index.html'), ANIMATED_PAGE);
  fs.writeFileSync(path.join(dir, 'inert.html'), INERT_PAGE);
  const { server, port } = await serveDir(dir);

  try {
    const report = await collectEvidence(`http://127.0.0.1:${port}`, ['/', '/inert.html']);
    assert.equal(report.status, 'ok');

    const animated = report.routes.find((r) => r.route === '/');
    assert.ok(animated.moving.includes('rise'), 'the animated route should report movement');
    assert.deepEqual(animated.movingUnderReduce, [], 'reduce should still it');
    assert.equal(report.findings.filter((f) => f.route === '/').length, 0);

    const inert = report.findings.find((f) => f.route === '/inert.html');
    assert.ok(inert, 'the inert route should draw a finding');
    assert.equal(inert.rule, 'MOTION_ABSENT');
  } finally {
    server.close();
    fs.rmSync(dir, { recursive: true, force: true });
  }
});
