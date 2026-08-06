// ui-liveregion-evidence.test.mjs — live-region announcements (design spec L4c, phase 2).
//
// Every event fixture below is recorded from a real Chromium run, not written by hand.
// The Q3/Q4 pair is the load-bearing one and is reproduced verbatim, because it is the
// entire justification for the non-empty-at-delivery rule:
//
//   Q3  appended empty, filled in the SAME task   region-inserted "Loaded 3 runs."
//                                                 content-added   "Loaded 3 runs."   (same rid)
//   Q4  appended empty, filled on a LATER task    region-inserted ""
//                                                 content-added   "Loaded 3 runs."   (same rid)
//
// The two differ only in whether the inserted region already carried text when the
// observer's records were delivered — and that is exactly the boundary a screen reader
// uses, since a region created and filled within one task is not announced either. Without
// the rid, Q3's content-added would mask the bug the tier exists to find.
//
// Tests:
//   1-4.   parseManifest: string shorthand, declared kind, unknown kind, missing routes
//   5-8.   announced: Q2 counts, Q1 does not, Q4 counts, Q3 does not
//   9-10.  judgeState: TEST_HOOK_MISSING, TEST_HOOK_NO_EFFECT
//   11.    judgeState: LIVE_REGION_ABSENT when the page changed and nothing announced
//   12.    judgeState: LIVE_REGION_INSERTED_WITH_CONTENT, and it suppresses ABSENT
//   13.    judgeState: LIVE_REGION_HIDDEN
//   14.    judgeState: LIVE_REGION_STUCK_BUSY is judged at settle, not per event
//   15.    judgeState: a state that declares itself busy is exempt
//   16-17. judgeState: LIVE_REGION_WRONG_POLITENESS, both directions
//   18.    judgeState: a correct announcement draws nothing
//   19-24. real browser: correct page clean; inserted-with-content, same-task, silent and
//          absent-hook caught; the append-then-fill-later pattern not a false positive
//   25.    an absent manifest loads as null so the caller can skip rather than pass

import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { createServer } from 'node:http';

import {
  parseManifest,
  loadManifest,
  announced,
  judgeState,
  collectEvidence,
  loadPlaywright,
} from '../../scripts/ui-liveregion-evidence.mjs';

const ev = (kind, over = {}) => ({
  kind, rid: 'r1', politeness: 'polite', busy: false, display: 'block', text: 'Loaded 3 runs.', ...over,
});
const region = (over = {}) => ({ politeness: 'polite', busy: false, display: 'block', text: '', ...over });

// Recorded: Q1 through Q4 of scratchpad/probe_live3.mjs.
const Q1_INSERTED_WITH_TEXT = [ev('region-inserted')];
const Q2_FILLED_LATER = [ev('content-added')];
const Q3_SAME_TASK = [ev('region-inserted'), ev('content-added')];
const Q4_LATER_TASK = [ev('region-inserted', { text: '' }), ev('content-added')];

const observation = (over = {}) => ({
  hookFound: true, mutations: 2, events: Q2_FILLED_LATER, regions: [region({ text: 'Loaded 3 runs.' })], ...over,
});
const rules = (findings) => findings.map((f) => f.rule);

// ── manifest ──────────────────────────────────────────────────────────────────

test('a state given as a bare string defaults to status politeness', () => {
  const m = parseManifest({ schema: 1, routes: [{ route: '/runs', states: ['loading'] }] });
  assert.deepEqual(m.routes[0].states, [{ name: 'loading', kind: 'status', busy: false }]);
  assert.equal(m.hook, '__autospec.setState');
});

test('a declared kind is kept, and a custom hook overrides the default', () => {
  const m = parseManifest({
    schema: 1,
    hook: '__myapp.drive',
    routes: [{ route: '/runs', states: [{ name: 'error', kind: 'alert' }] }],
  });
  assert.deepEqual(m.routes[0].states, [{ name: 'error', kind: 'alert', busy: false }]);
  assert.equal(m.hook, '__myapp.drive');
});

test('an unknown kind is rejected rather than guessed from the state name', () => {
  assert.throws(
    () => parseManifest({ schema: 1, routes: [{ route: '/r', states: [{ name: 'x', kind: 'shout' }] }] }),
    /kind must be 'status' or 'alert'/,
  );
});

test('a manifest with no routes is rejected', () => {
  assert.throws(() => parseManifest({ schema: 1 }), /routes/);
});

// ── what counts as announced ──────────────────────────────────────────────────

test('a pre-existing region filled later is announced', () => {
  assert.equal(announced(Q2_FILLED_LATER).length, 1);
});

test('a region that arrived already carrying its text is not announced', () => {
  assert.deepEqual(announced(Q1_INSERTED_WITH_TEXT), []);
});

test('a region appended empty and filled on a later task is announced', () => {
  assert.equal(announced(Q4_LATER_TASK).length, 1);
});

test('a region appended and filled in the same task is not announced', () => {
  // The content-added event carries the same rid as the region-inserted that already had
  // text, so it must not be mistaken for a genuine later update.
  assert.deepEqual(announced(Q3_SAME_TASK), []);
});

// ── judgement ─────────────────────────────────────────────────────────────────

test('a hook the page never exposes is reported as missing, not as a silent region', () => {
  const f = judgeState('/runs', { name: 'loading', kind: 'status' }, observation({ hookFound: false, events: [], mutations: 0 }));
  assert.deepEqual(rules(f), ['TEST_HOOK_MISSING']);
});

test('a hook that changes nothing at all is reported as a no-op', () => {
  const f = judgeState('/runs', { name: 'loading', kind: 'status' }, observation({ mutations: 0, events: [] }));
  assert.deepEqual(rules(f), ['TEST_HOOK_NO_EFFECT']);
});

test('a hook that throws is reported as a failed hook, not as a missing announcement', () => {
  const f = judgeState('/runs', { name: 'success', kind: 'status' }, observation({
    hookError: 'unknown state: sucess', events: [], mutations: 0,
  }));
  assert.deepEqual(rules(f), ['TEST_HOOK_FAILED']);
  assert.match(f[0].detail, /unknown state/);
});

test('an induced state the app ignored entirely is named as that, not as a dead hook', () => {
  // Measured: a page with no error path makes zero mutations when its request fails. The
  // user is left looking at stale content forever. That is a worse defect than a missing
  // announcement, and it is not the manifest problem TEST_HOOK_NO_EFFECT describes.
  const f = judgeState('/runs', { name: 'error', kind: 'alert', induced: true }, observation({
    mutations: 0, events: [],
  }));
  assert.deepEqual(rules(f), ['INDUCED_STATE_IGNORED']);
  assert.match(f[0].detail, /error/);
});

test('a page that changed with nothing announced is LIVE_REGION_ABSENT', () => {
  const f = judgeState('/runs', { name: 'success', kind: 'status' }, observation({ mutations: 1, events: [] }));
  assert.deepEqual(rules(f), ['LIVE_REGION_ABSENT']);
});

test('a region inserted carrying its text is named exactly, not reported as absent', () => {
  const f = judgeState('/runs', { name: 'success', kind: 'status' }, observation({ events: Q1_INSERTED_WITH_TEXT }));
  assert.deepEqual(rules(f), ['LIVE_REGION_INSERTED_WITH_CONTENT']);
  assert.match(f[0].detail, /same task/);
});

test('an announcement written into a display:none region is reported as never heard', () => {
  const f = judgeState('/runs', { name: 'success', kind: 'status' }, observation({
    events: [ev('content-added', { display: 'none' })],
    regions: [region({ display: 'none', text: 'Nobody hears this.' })],
  }));
  assert.deepEqual(rules(f), ['LIVE_REGION_HIDDEN']);
});

test('aria-busy is judged at settle, because event attributes read end state', () => {
  // Recorded in probe pass 2: the content-added event for the busy region already reported
  // busy=false, because MutationObserver records are read at delivery. Only the settle-time
  // snapshot can answer this.
  const f = judgeState('/runs', { name: 'success', kind: 'status' }, observation({
    regions: [region({ busy: true, text: 'Loaded 3 runs.' })],
  }));
  assert.deepEqual(rules(f), ['LIVE_REGION_STUCK_BUSY']);
});

test('a state that declares itself busy may keep aria-busy set', () => {
  // A loading state is supposed to leave aria-busy true; only an undeclared one is stuck.
  const busyRegion = observation({ regions: [region({ busy: true, text: 'Loading…' })] });
  assert.deepEqual(judgeState('/runs', { name: 'loading', kind: 'status', busy: true }, busyRegion), []);
  assert.deepEqual(
    rules(judgeState('/runs', { name: 'loading', kind: 'status' }, busyRegion)),
    ['LIVE_REGION_STUCK_BUSY'],
  );
});

test('a state declared as a status but announced assertively is reported', () => {
  const f = judgeState('/runs', { name: 'success', kind: 'status' }, observation({
    events: [ev('content-added', { politeness: 'assertive' })],
  }));
  assert.deepEqual(rules(f), ['LIVE_REGION_WRONG_POLITENESS']);
  assert.match(f[0].detail, /assertive/);
});

test('a state declared as an alert but announced politely is reported', () => {
  const f = judgeState('/runs', { name: 'error', kind: 'alert' }, observation({
    events: [ev('content-added', { politeness: 'polite' })],
  }));
  assert.deepEqual(rules(f), ['LIVE_REGION_WRONG_POLITENESS']);
});

test('a correct announcement draws nothing', () => {
  assert.deepEqual(judgeState('/runs', { name: 'success', kind: 'status' }, observation()), []);
});

// ── real browser ──────────────────────────────────────────────────────────────

const PAGE_HEAD = '<!doctype html><html><head><style>.gone{display:none}</style>'
  + '<script>window.__autospec={setState:(s)=>window.__drive(s)};</script></head><body>'
  + '<div id="live" role="status" aria-live="polite"></div><main id="host"><p id="body">nothing yet</p></main>';

const PAGES = {
  // Correct: the region exists at load and is filled when the state is driven.
  '/good': `${PAGE_HEAD}<script>window.__drive=()=>{
      document.getElementById('body').textContent='Loaded 3 runs.';
      document.getElementById('live').textContent='Loaded 3 runs.';
    };</script></body></html>`,
  // Bug: the region is created already carrying its text.
  '/inserted': `${PAGE_HEAD}<script>window.__drive=()=>{
      document.getElementById('body').textContent='Loaded 3 runs.';
      const d=document.createElement('div');
      d.setAttribute('role','status'); d.textContent='Loaded 3 runs.';
      document.getElementById('host').appendChild(d);
    };</script></body></html>`,
  // Bug: appended empty, then filled in the SAME task. Reaches the judge as a
  // region-inserted carrying text plus a content-added on the same region, which is the
  // only shape where the region id decides the verdict.
  '/sametask': `${PAGE_HEAD}<script>window.__drive=()=>{
      document.getElementById('body').textContent='Loaded 3 runs.';
      const d=document.createElement('div');
      d.setAttribute('aria-live','polite');
      document.getElementById('host').appendChild(d);
      d.textContent='Loaded 3 runs.';
    };</script></body></html>`,
  // Correct: appended empty, filled on a LATER task. Emits region-inserted too, so a rule
  // reading insertion alone would fail this page.
  '/latertask': `${PAGE_HEAD}<script>window.__drive=()=>{
      document.getElementById('body').textContent='Loaded 3 runs.';
      const d=document.createElement('div');
      d.setAttribute('aria-live','polite');
      document.getElementById('host').appendChild(d);
      setTimeout(()=>{ d.textContent='Loaded 3 runs.'; }, 20);
    };</script></body></html>`,
  // Bug: the page updates and says nothing.
  '/silent': `${PAGE_HEAD}<script>window.__drive=()=>{
      document.getElementById('body').textContent='Loaded 3 runs.';
    };</script></body></html>`,
  // Bug: no hook at all.
  '/nohook': '<!doctype html><html><body><p>nothing to drive</p></body></html>',
};

async function serve() {
  const server = createServer((req, res) => {
    const body = PAGES[req.url.split('?')[0]];
    res.writeHead(body ? 200 : 404, { 'content-type': 'text/html' });
    res.end(body || 'not found');
  });
  await new Promise((r) => server.listen(0, '127.0.0.1', r));
  return { server, base: `http://127.0.0.1:${server.address().port}` };
}

const withBrowser = async (routes, fn) => {
  if (!(await loadPlaywright())) return null;
  const { server, base } = await serve();
  try {
    const manifest = parseManifest({
      schema: 1,
      routes: routes.map((route) => ({ route, states: ['success'] })),
    });
    return await fn(await collectEvidence(base, manifest));
  } finally {
    server.close();
  }
};

test('real browser: a page that fills a region already present announces cleanly', async () => {
  const report = await withBrowser(['/good'], (r) => r);
  if (!report) return;
  assert.deepEqual(report.findings, []);
});

test('real browser: a region inserted carrying its text is caught', async () => {
  const report = await withBrowser(['/inserted'], (r) => r);
  if (!report) return;
  assert.deepEqual(rules(report.findings), ['LIVE_REGION_INSERTED_WITH_CONTENT']);
});

test('real browser: a region appended empty and filled in the same task is caught', async () => {
  const report = await withBrowser(['/sametask'], (r) => r);
  if (!report) return;
  assert.deepEqual(rules(report.findings), ['LIVE_REGION_INSERTED_WITH_CONTENT']);
});

test('real browser: a region appended empty and filled later is not a false positive', async () => {
  const report = await withBrowser(['/latertask'], (r) => r);
  if (!report) return;
  assert.deepEqual(report.findings, []);
});

test('real browser: a page that updates silently is caught', async () => {
  const report = await withBrowser(['/silent'], (r) => r);
  if (!report) return;
  assert.deepEqual(rules(report.findings), ['LIVE_REGION_ABSENT']);
});

test('real browser: a declared hook the page never exposes is caught', async () => {
  const report = await withBrowser(['/nohook'], (r) => r);
  if (!report) return;
  assert.deepEqual(rules(report.findings), ['TEST_HOOK_MISSING']);
});

test('an absent manifest loads as null, so the caller can skip rather than pass', () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'lr-'));
  try {
    assert.equal(loadManifest(path.join(dir, 'ui-test-hooks.json')), null);
    const file = path.join(dir, 'ui-test-hooks.json');
    fs.writeFileSync(file, JSON.stringify({ schema: 1, routes: [{ route: '/r', states: ['success'] }] }));
    assert.equal(loadManifest(file).routes[0].route, '/r');
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});
