// ui-liveregion-induce.test.mjs — inducing app states without the app's cooperation.
//
// Almost every test here needs a browser, because the thing under test *is* the interaction
// between request interception, mount timing and the observer. A recorded fixture would
// only re-assert the sequence I already believe in.
//
// The pages are the smallest apps that behave like real ones: a shell renders, a fetch goes
// out, and the response decides what appears. That is the shape induction assumes, and the
// silent one is the shape it exists to catch.
//
// Tests:
//   1-2.  isDataRequest: fetch and xhr are held, documents and stylesheets are not
//   3.    INDUCED_STATES: error is an alert, success is a status
//   4.    discovery finds the data request a route makes
//   5.    discovery returns nothing for a static route
//   6.    a well-behaved app announces its success
//   7.    a well-behaved app announces its failure, assertively
//   8.    an app that renders data silently is caught
//   9.    an app with no error path at all is caught, and named for that
//   10.   a static route is skipped with a reason rather than passed
//   11.   an app that leaves aria-busy set after the response is caught

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';

import {
  isDataRequest,
  INDUCED_STATES,
  discoverEndpoints,
  collectInduced,
} from '../../scripts/ui-liveregion-induce.mjs';
import { loadPlaywright } from '../../scripts/ui-liveregion-core.mjs';

const SHELL = `<p id="status" role="status" aria-live="polite" aria-busy="true">Loading runs…</p>
  <p id="alert" role="alert"></p>
  <ul id="runs"><li>…</li></ul>`;

const app = (body) => `<!doctype html><html><body>${SHELL}<script>
  const s = document.getElementById('status'), a = document.getElementById('alert');
  const list = document.getElementById('runs');
  const render = (d) => { list.innerHTML = d.map((x) => '<li>' + x + '</li>').join(''); };
  ${body}
<\/script></body></html>`;

const PAGES = {
  // Announces both outcomes and clears its busy flag.
  '/good': app(`
    fetch('/api/runs')
      .then((r) => { if (!r.ok) throw new Error('http ' + r.status); return r.json(); })
      .then((d) => { s.setAttribute('aria-busy','false'); render(d); s.textContent = 'Loaded ' + d.length + ' runs.'; })
      .catch(() => { s.setAttribute('aria-busy','false'); s.textContent = ''; a.textContent = 'Could not load runs.'; });`),

  // Renders the data and says nothing; has no error path at all.
  '/silent': app(`
    fetch('/api/runs').then((r) => r.json()).then((d) => { render(d); });`),

  // Announces, but never clears aria-busy, so the announcement is suppressed indefinitely.
  '/busy': app(`
    fetch('/api/runs')
      .then((r) => { if (!r.ok) throw new Error('x'); return r.json(); })
      .then((d) => { render(d); s.textContent = 'Loaded ' + d.length + ' runs.'; })
      .catch(() => { a.textContent = 'Could not load runs.'; });`),

  '/static': '<!doctype html><html><body><h1>About</h1><p>No data here.</p></body></html>',
};

async function serve() {
  const server = createServer((req, res) => {
    const route = req.url.split('?')[0];
    if (route === '/api/runs') {
      res.writeHead(200, { 'content-type': 'application/json' });
      res.end(JSON.stringify(['run-104 passed', 'run-103 passed', 'run-102 failed']));
      return;
    }
    const body = PAGES[route];
    res.writeHead(body ? 200 : 404, { 'content-type': 'text/html' });
    res.end(body || 'not found');
  });
  await new Promise((r) => server.listen(0, '127.0.0.1', r));
  return { server, base: `http://127.0.0.1:${server.address().port}` };
}

async function launch() {
  const playwright = await loadPlaywright();
  if (!playwright) return null;
  const launchOpts = {};
  if (process.env.PLAYWRIGHT_CHROMIUM_PATH) {
    launchOpts.executablePath = process.env.PLAYWRIGHT_CHROMIUM_PATH;
  }
  return playwright.chromium.launch(launchOpts);
}

/** Run `fn` with a served app and a browser, or skip when Playwright is unavailable. */
async function withApp(fn) {
  const browser = await launch();
  if (!browser) return null;
  const { server, base } = await serve();
  try {
    return await fn(browser, base);
  } finally {
    await browser.close();
    server.close();
  }
}

const rules = (findings) => findings.map((f) => f.rule);

// ── pure ──────────────────────────────────────────────────────────────────────

test('data requests are held and page resources are not', () => {
  assert.equal(isDataRequest('fetch'), true);
  assert.equal(isDataRequest('xhr'), true);
});

test('documents, styles and scripts are left alone', () => {
  for (const type of ['document', 'stylesheet', 'script', 'image', 'font']) {
    assert.equal(isDataRequest(type), false, `${type} must not be intercepted`);
  }
});

test('an induced error is an alert and an induced success is a status', () => {
  assert.deepEqual(INDUCED_STATES.map((s) => [s.name, s.kind]), [
    ['success', 'status'],
    ['error', 'alert'],
  ]);
  assert.ok(INDUCED_STATES.every((s) => s.induced));
});

// ── discovery ─────────────────────────────────────────────────────────────────

test('discovery finds the data request a route makes', async () => {
  const found = await withApp((browser, base) => discoverEndpoints(browser, `${base}/good`));
  if (!found) return;
  assert.equal(found.length, 1);
  assert.match(found[0], /\/api\/runs$/);
});

test('discovery finds nothing on a route that fetches nothing', async () => {
  const found = await withApp((browser, base) => discoverEndpoints(browser, `${base}/static`));
  if (!found) return;
  assert.deepEqual(found, []);
});

// ── induction ─────────────────────────────────────────────────────────────────

test('an app that announces both outcomes induces clean', async () => {
  const report = await withApp((browser, base) => collectInduced(browser, base, ['/good']));
  if (!report) return;
  assert.deepEqual(report.findings, []);
  assert.equal(report.states.length, 2);
});

test('the induced error reaches the assertive region, and success the polite one', async () => {
  // The politeness rule holds without a manifest because the inducer named the state: a
  // request it made fail is an error, and correct code routes that to an alert.
  const report = await withApp((browser, base) => collectInduced(browser, base, ['/good']));
  if (!report) return;
  assert.deepEqual(
    report.states.map((s) => [s.state, s.kind]),
    [['success', 'status'], ['error', 'alert']],
  );
  assert.deepEqual(report.findings, []);
});

test('an app that renders data without announcing it is caught', async () => {
  const report = await withApp((browser, base) => collectInduced(browser, base, ['/silent']));
  if (!report) return;
  // Success renders the list and says nothing; the failure is not handled at all.
  assert.ok(rules(report.findings).includes('LIVE_REGION_ABSENT'));
});

test('an app with no error path is named for that, not for a missing announcement', async () => {
  const report = await withApp((browser, base) => collectInduced(browser, base, ['/silent']));
  if (!report) return;
  const errorFindings = report.findings.filter((f) => f.state === 'error');
  assert.deepEqual(rules(errorFindings), ['INDUCED_STATE_IGNORED']);
  assert.match(errorFindings[0].detail, /stale content/);
});

test('a route with nothing to induce is skipped with a reason, not passed', async () => {
  const report = await withApp((browser, base) => collectInduced(browser, base, ['/static']));
  if (!report) return;
  assert.deepEqual(report.findings, []);
  assert.deepEqual(report.states, []);
  assert.equal(report.skipped.length, 1);
  assert.match(report.skipped[0].reason, /nothing to induce/);
});

test('aria-busy left set after the response arrives is caught', async () => {
  const report = await withApp((browser, base) => collectInduced(browser, base, ['/busy']));
  if (!report) return;
  assert.ok(rules(report.findings).includes('LIVE_REGION_STUCK_BUSY'));
});
