#!/usr/bin/env node
// ui-liveregion-evidence.mjs — live-region announcements (design spec L4c, second phase).
//
// Drives the app into each declared state through a test hook it exposes, and asserts that
// a screen-reader user is actually told what happened:
//
//   TEST_HOOK_MISSING               the manifest declares a hook the page never exposes
//   TEST_HOOK_FAILED                the hook threw when driven
//   TEST_HOOK_NO_EFFECT             the hook ran and nothing in the page changed
//   LIVE_REGION_ABSENT              the page changed and nothing was announced
//   LIVE_REGION_INSERTED_WITH_CONTENT  a region entered the DOM already carrying its text
//   LIVE_REGION_HIDDEN              an announcement was written into a display:none region
//   LIVE_REGION_STUCK_BUSY          aria-busy was left true after the state settled
//   LIVE_REGION_WRONG_POLITENESS    announced against the politeness the state declares
//
// This is the first tier that needs the app under test to cooperate, because the thing
// worth checking is a *transition*: content present at page load is never announced, so a
// state reached by reloading with a query parameter can only be inspected statically. The
// manifest at `.autospec/ui-test-hooks.json` is that cooperation, and its absence is a
// skip, never a pass — the QA cluster records the gap so unadopted repos do not read as
// covered.
//
// Measured before it was written. Two readings changed the design:
//
//   * MutationObserver records are read at delivery, not at mutation, so every attribute
//     on an event is end state. `aria-busy` already read false on the content event that
//     preceded the attribute event — which is why stuck-busy is judged from the settle-time
//     snapshot instead of from the event stream.
//   * A region appended empty and filled on a later task also emits `region-inserted`, so
//     "any inserted region is a bug" false-positives on the correct dynamic pattern. The
//     discriminator is whether the region carried text at delivery, and that lands exactly
//     on the frame boundary a screen reader uses: a region created and filled within one
//     task is not announced either.
//
// Usage:
//   ui-liveregion-evidence.mjs --base-url http://localhost:3000 [--manifest PATH]
//
// Exit: 0 clean or skipped, 1 findings, 3 Playwright unavailable.
//
// Env:
//   PLAYWRIGHT_CHROMIUM_PATH  launch this chromium binary instead of the bundled one.

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));

export const DEFAULT_MANIFEST = '.autospec/ui-test-hooks.json';
export const DEFAULT_HOOK = '__autospec.setState';

// A fixed sleep flakes in both directions, so a state is settled by quiescence: this long
// with no mutation at all, capped so a page that never goes quiet still terminates.
export const QUIET_MS = 150;
export const MAX_SETTLE_MS = 3000;
// The hook may be registered by a deferred bundle, so its absence is only concluded after
// a bounded wait.
export const HOOK_TIMEOUT_MS = 5000;

const LIVE_SELECTOR =
  '[aria-live], [role="status"], [role="alert"], [role="log"], output';

// ── manifest ──────────────────────────────────────────────────────────────────

/**
 * Normalise a declared manifest. States may be bare strings or `{name, kind}`; `kind` is
 * declared rather than inferred, because classifying a state *name* is a regex pretending
 * to be a judgement — "error" is obvious, "timeout" and "partial" are not.
 */
export function parseManifest(raw) {
  if (!raw || typeof raw !== 'object') throw new Error('manifest must be an object');
  if (!Array.isArray(raw.routes) || raw.routes.length === 0) {
    throw new Error('manifest must declare a non-empty routes array');
  }
  const routes = raw.routes.map((entry) => {
    if (!entry || typeof entry.route !== 'string') {
      throw new Error('each route entry must have a route string');
    }
    if (!Array.isArray(entry.states) || entry.states.length === 0) {
      throw new Error(`route '${entry.route}' must declare a non-empty states array`);
    }
    const states = entry.states.map((state) => {
      const value = typeof state === 'string' ? { name: state } : state;
      if (!value || typeof value.name !== 'string') {
        throw new Error(`route '${entry.route}' has a state with no name`);
      }
      const kind = value.kind || 'status';
      if (kind !== 'status' && kind !== 'alert') {
        throw new Error(`state '${value.name}': kind must be 'status' or 'alert'`);
      }
      // A loading state is *supposed* to leave aria-busy set, and within a single driven
      // state that is indistinguishable from a busy flag left stuck. So it is declared.
      return { name: value.name, kind, busy: Boolean(value.busy) };
    });
    return { route: entry.route, states };
  });
  return { hook: raw.hook || DEFAULT_HOOK, routes };
}

/** Read a manifest, or null when the repo has not opted in. */
export function loadManifest(file) {
  if (!fs.existsSync(file)) return null;
  return parseManifest(JSON.parse(fs.readFileSync(file, 'utf8')));
}

// ── assertions ────────────────────────────────────────────────────────────────

/**
 * The events a screen reader would actually announce.
 *
 * A region that already carried text when the observer's records were delivered was not
 * announced, and neither was anything else written into it in that same batch — hence the
 * region id, without which the same-task case would be masked by its own content event.
 */
export function announced(events) {
  const bornFull = new Set(
    events.filter((e) => e.kind === 'region-inserted' && e.text).map((e) => e.rid),
  );
  return events.filter(
    (e) =>
      (e.kind === 'content-added' || e.kind === 'text-changed') &&
      e.text &&
      !bornFull.has(e.rid),
  );
}

const uniqueBy = (items, key) => {
  const seen = new Set();
  return items.filter((item) => {
    if (seen.has(item[key])) return false;
    seen.add(item[key]);
    return true;
  });
};

/**
 * Judge one driven state. Pure: takes the recorded observation, returns findings, touches
 * no browser.
 */
export function judgeState(route, state, observation) {
  const { hookFound, hookError, mutations, events, regions } = observation;
  const at = { route, state: state.name };

  // A hook that was never exposed and a hook that did nothing are different bugs, and
  // reporting either as a silent live region sends the author after the wrong one.
  if (!hookFound) {
    return [{
      ...at,
      rule: 'TEST_HOOK_MISSING',
      detail: `the manifest declares a test hook that '${route}' never exposes`,
    }];
  }
  if (hookError) {
    return [{ ...at, rule: 'TEST_HOOK_FAILED', detail: `driving '${state.name}' threw: ${hookError}` }];
  }
  if (!mutations && events.length === 0) {
    return [{
      ...at,
      rule: 'TEST_HOOK_NO_EFFECT',
      detail: `driving '${state.name}' changed nothing in the page`,
    }];
  }

  const findings = [];
  const all = announced(events);
  const heard = all.filter((e) => e.display !== 'none');
  const unheard = all.filter((e) => e.display === 'none');
  const bornFull = events.filter((e) => e.kind === 'region-inserted' && e.text);

  for (const event of uniqueBy(bornFull, 'rid')) {
    findings.push({
      ...at,
      rule: 'LIVE_REGION_INSERTED_WITH_CONTENT',
      detail:
        `the region carrying '${event.text}' entered the page already holding that text, ` +
        'in the same task — a region created and filled together is not announced; add ' +
        'the region to the markup and fill it when the state changes',
    });
  }
  for (const event of uniqueBy(unheard, 'rid')) {
    findings.push({
      ...at,
      rule: 'LIVE_REGION_HIDDEN',
      detail: `'${event.text}' was written into a display:none region, which no screen reader announces`,
    });
  }
  // Only when nothing else explains the silence: the two rules above are the diagnosis, and
  // repeating them as a bare absence would bury it.
  if (heard.length === 0 && bornFull.length === 0 && unheard.length === 0) {
    findings.push({
      ...at,
      rule: 'LIVE_REGION_ABSENT',
      detail: `driving '${state.name}' changed the page and announced nothing`,
    });
  }

  for (const region of state.busy ? [] : regions.filter((r) => r.busy)) {
    findings.push({
      ...at,
      rule: 'LIVE_REGION_STUCK_BUSY',
      detail:
        `a live region is still aria-busy="true" after '${state.name}' settled` +
        (region.text ? `, holding '${region.text}'` : ''),
    });
  }

  const want = state.kind === 'alert' ? 'assertive' : 'polite';
  for (const event of uniqueBy(heard.filter((e) => e.politeness !== want), 'rid')) {
    findings.push({
      ...at,
      rule: 'LIVE_REGION_WRONG_POLITENESS',
      detail:
        `'${state.name}' is declared as a ${state.kind} but announced ${event.politeness}; ` +
        `expected ${want}`,
    });
  }

  return findings;
}

// ── browser collection ────────────────────────────────────────────────────────

// Region identity comes from a WeakMap rather than a DOM attribute, so observing the page
// never mutates it.
const OBSERVER = `(() => {
  const LIVE = ${JSON.stringify(LIVE_SELECTOR)};
  const rids = new WeakMap();
  let n = 0;
  const rid = (el) => {
    if (!rids.has(el)) rids.set(el, 'r' + (n += 1));
    return rids.get(el);
  };
  const politeness = (el) => {
    const explicit = el.getAttribute('aria-live');
    if (explicit) return explicit;
    const role = el.getAttribute('role');
    if (role === 'alert') return 'assertive';
    if (role === 'status' || role === 'log' || el.tagName === 'OUTPUT') return 'polite';
    return '';
  };
  const host = (node) => {
    let el = node.nodeType === 1 ? node : node.parentElement;
    while (el) {
      if (el.matches && el.matches(LIVE)) return el;
      el = el.parentElement;
    }
    return null;
  };
  const at = (el, kind, text) => ({
    kind,
    rid: rid(el),
    politeness: politeness(el),
    busy: el.getAttribute('aria-busy') === 'true',
    display: getComputedStyle(el).display,
    text: String(text || '').replace(/\\s+/g, ' ').trim().slice(0, 60),
  });

  window.__autospecEvents = [];
  window.__autospecMutations = 0;
  window.__autospecLastMutation = 0;

  new MutationObserver((records) => {
    // Counted apart from the live-region events: a state that legitimately updates only a
    // live region still did something, and must not read as a hook that did nothing.
    window.__autospecMutations += records.length;
    window.__autospecLastMutation = performance.now();
    for (const rec of records) {
      for (const node of rec.addedNodes) {
        if (node.nodeType === 1 && node.matches && node.matches(LIVE)) {
          window.__autospecEvents.push(at(node, 'region-inserted', node.textContent));
          continue;
        }
        const h = host(node);
        if (h) window.__autospecEvents.push(at(h, 'content-added', node.textContent));
      }
      if (rec.type === 'characterData') {
        const h = host(rec.target);
        if (h) window.__autospecEvents.push(at(h, 'text-changed', rec.target.data));
      }
      if (rec.type === 'attributes' && rec.target.matches && rec.target.matches(LIVE)) {
        window.__autospecEvents.push(at(rec.target, 'attr-' + rec.attributeName, ''));
      }
    }
  }).observe(document.body, {
    childList: true,
    subtree: true,
    characterData: true,
    attributes: true,
    attributeFilter: ['aria-live', 'aria-busy', 'role', 'hidden'],
  });
  return true;
})()`;

// Live regions as they stand once the state has settled, for the judgements that are about
// end state rather than about the transition.
const REGIONS_PROBE = `(() => {
  const LIVE = ${JSON.stringify(LIVE_SELECTOR)};
  return Array.from(document.querySelectorAll(LIVE)).map((el) => ({
    politeness: el.getAttribute('aria-live')
      || (el.getAttribute('role') === 'alert' ? 'assertive'
        : (el.getAttribute('role') === 'status' || el.getAttribute('role') === 'log'
          || el.tagName === 'OUTPUT' ? 'polite' : '')),
    busy: el.getAttribute('aria-busy') === 'true',
    display: getComputedStyle(el).display,
    text: (el.textContent || '').replace(/\\s+/g, ' ').trim().slice(0, 60),
  }));
})()`;

const resolveHook = (hook) =>
  `${JSON.stringify(hook)}.split('.').reduce((o, k) => (o ? o[k] : undefined), window)`;

export async function loadPlaywright() {
  const { findPlaywrightPath } = await import(path.resolve(__dirname, 'gen-screenshots.mjs'));
  const found = findPlaywrightPath();
  if (!found) return null;
  return import(found);
}

/**
 * Drive one state and record what happened.
 *
 * A fresh context per state, not per route: driving loading → error → success on one page
 * leaves state 2 inheriting state 1's DOM, so regions already present never re-fire
 * `region-inserted` and the bug this tier exists to find goes silently undetected on every
 * state after the first.
 */
export async function driveState(browser, url, hook, state) {
  const context = await browser.newContext();
  const page = await context.newPage();
  try {
    await page.goto(url, { waitUntil: 'load' });
    await page.evaluate(OBSERVER);

    const hookFound = await page
      .waitForFunction(`typeof (${resolveHook(hook)}) === 'function'`, null, {
        timeout: HOOK_TIMEOUT_MS,
      })
      .then(() => true)
      .catch(() => false);
    if (!hookFound) {
      return { hookFound: false, hookError: '', mutations: 0, events: [], regions: [], settled: true };
    }

    // Quiescence is measured from the moment the hook is driven, so a page that was already
    // idle does not read as instantly settled.
    await page.evaluate('window.__autospecLastMutation = performance.now()');
    let hookError = '';
    try {
      await page.evaluate(`(async () => { await (${resolveHook(hook)})(${JSON.stringify(state.name)}); })()`);
    } catch (error) {
      hookError = String(error.message || error).split('\n')[0].slice(0, 200);
    }

    const settled = await page
      .waitForFunction(
        `performance.now() - window.__autospecLastMutation > ${QUIET_MS}`,
        null,
        { timeout: MAX_SETTLE_MS },
      )
      .then(() => true)
      .catch(() => false);

    return {
      hookFound: true,
      hookError,
      mutations: await page.evaluate('window.__autospecMutations'),
      events: await page.evaluate('window.__autospecEvents'),
      regions: await page.evaluate(REGIONS_PROBE),
      settled,
    };
  } finally {
    await context.close();
  }
}

export async function collectEvidence(baseUrl, manifest) {
  const playwright = await loadPlaywright();
  if (!playwright) {
    return {
      schema: 1,
      status: 'blocked_missing_playwright',
      detail: 'Playwright is not installed; live-region evidence was not collected',
      states: [],
      findings: [],
    };
  }

  const launch = {};
  if (process.env.PLAYWRIGHT_CHROMIUM_PATH) {
    launch.executablePath = process.env.PLAYWRIGHT_CHROMIUM_PATH;
  }
  const browser = await playwright.chromium.launch(launch);
  const states = [];
  const findings = [];

  try {
    for (const entry of manifest.routes) {
      const url = new URL(entry.route, baseUrl).toString();
      for (const state of entry.states) {
        const observation = await driveState(browser, url, manifest.hook, state);
        findings.push(...judgeState(entry.route, state, observation));
        states.push({
          route: entry.route,
          state: state.name,
          kind: state.kind,
          mutations: observation.mutations,
          announcements: announced(observation.events).length,
          // Recorded so a report read later says whether the page went quiet or hit the cap.
          settled: observation.settled ? 'quiet' : `capped at ${MAX_SETTLE_MS}ms`,
        });
      }
    }
  } finally {
    await browser.close();
  }

  return { schema: 1, status: 'ok', states, findings };
}

// ── CLI ───────────────────────────────────────────────────────────────────────

function parseArgs(argv) {
  const opts = { baseUrl: '', manifest: DEFAULT_MANIFEST, json: '' };
  for (let i = 0; i < argv.length; i += 1) {
    if (argv[i] === '--base-url') opts.baseUrl = argv[++i];
    else if (argv[i] === '--manifest') opts.manifest = argv[++i];
    else if (argv[i] === '--json') opts.json = argv[++i];
  }
  return opts;
}

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  if (!opts.baseUrl) {
    process.stderr.write('Usage: ui-liveregion-evidence.mjs --base-url URL [--manifest PATH]\n');
    process.exit(3);
  }

  const writeReport = (report) => {
    if (!opts.json) return;
    fs.mkdirSync(path.dirname(path.resolve(opts.json)), { recursive: true });
    fs.writeFileSync(opts.json, `${JSON.stringify(report, null, 2)}\n`);
  };

  const manifest = loadManifest(path.resolve(opts.manifest));
  if (!manifest) {
    // A skip writes its report too. Leaving the file absent makes "this repo has not
    // adopted the hook" indistinguishable from "the step never ran" to anything reading
    // the report, which is exactly the gap the skip is supposed to make visible.
    writeReport({
      schema: 1,
      status: 'skipped',
      detail: `no ${opts.manifest}; no states are declared to drive`,
      states: [],
      findings: [],
    });
    process.stdout.write(
      `ui-liveregion-evidence: SKIPPED (no ${opts.manifest}; no states are declared to drive)\n`,
    );
    process.exit(0);
  }

  const report = await collectEvidence(opts.baseUrl, manifest);
  writeReport(report);

  if (report.status === 'blocked_missing_playwright') {
    process.stderr.write('ui-liveregion-evidence: Playwright unavailable; no evidence collected\n');
    process.exit(3);
  }

  for (const finding of report.findings) {
    process.stdout.write(`${finding.rule}:${finding.route}[${finding.state}]: ${finding.detail}\n`);
  }
  for (const row of report.states) {
    if (!report.findings.some((f) => f.route === row.route && f.state === row.state)) {
      process.stdout.write(
        `ok ${row.route}[${row.state}]: ${row.announcements} announcement(s), settled ${row.settled}\n`,
      );
    }
  }
  process.exit(report.findings.length > 0 ? 1 : 0);
}

if (process.argv[1] && process.argv[1].endsWith('ui-liveregion-evidence.mjs')) {
  await main();
}
