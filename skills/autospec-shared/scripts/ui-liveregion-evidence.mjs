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

import {
  announced,
  judgeState,
  loadPlaywright,
  OBSERVER,
  REGIONS_PROBE,
  QUIET_MS,
  MAX_SETTLE_MS,
} from './ui-liveregion-core.mjs';

import { collectInduced } from './ui-liveregion-induce.mjs';

// Re-exported so callers keep one import site for the whole tier.
export { announced, judgeState, loadPlaywright, OBSERVER, REGIONS_PROBE, QUIET_MS, MAX_SETTLE_MS };

export const DEFAULT_MANIFEST = '.autospec/ui-test-hooks.json';
export const DEFAULT_HOOK = '__autospec.setState';

// The hook may be registered by a deferred bundle, so its absence is only concluded after
// a bounded wait.
export const HOOK_TIMEOUT_MS = 5000;

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

const resolveHook = (hook) =>
  `${JSON.stringify(hook)}.split('.').reduce((o, k) => (o ? o[k] : undefined), window)`;

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

/** Drive the states a manifest declares. The browser is the caller's. */
export async function collectDeclared(browser, baseUrl, manifest) {
  const states = [];
  const findings = [];

  {
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
  }

  return { states, findings };
}

/**
 * Back-compatible wrapper: opens its own browser and drives only the declared states.
 * Induction is the CLI's job, since it needs routes rather than a manifest.
 */
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
  try {
    const { states, findings } = await collectDeclared(browser, baseUrl, manifest);
    return { schema: 1, status: 'ok', states, findings };
  } finally {
    await browser.close();
  }
}

// ── CLI ───────────────────────────────────────────────────────────────────────

function parseArgs(argv) {
  const opts = { baseUrl: '', manifest: DEFAULT_MANIFEST, json: '', routes: [], noInduce: false };
  for (let i = 0; i < argv.length; i += 1) {
    if (argv[i] === '--base-url') opts.baseUrl = argv[++i];
    else if (argv[i] === '--manifest') opts.manifest = argv[++i];
    else if (argv[i] === '--json') opts.json = argv[++i];
    else if (argv[i] === '--no-induce') opts.noInduce = true;
    else if (argv[i] === '--routes') {
      while (i + 1 < argv.length && !argv[i + 1].startsWith('--')) opts.routes.push(argv[++i]);
    }
  }
  return opts;
}

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  if (!opts.baseUrl) {
    process.stderr.write(
      'Usage: ui-liveregion-evidence.mjs --base-url URL --routes / [/more] [--manifest PATH]\n',
    );
    process.exit(3);
  }

  const writeReport = (report) => {
    if (!opts.json) return;
    fs.mkdirSync(path.dirname(path.resolve(opts.json)), { recursive: true });
    fs.writeFileSync(opts.json, `${JSON.stringify(report, null, 2)}\n`);
  };

  const playwright = await loadPlaywright();
  if (!playwright) {
    writeReport({
      schema: 1,
      status: 'blocked_missing_playwright',
      detail: 'Playwright is not installed; live-region evidence was not collected',
      states: [],
      findings: [],
      skipped: [],
    });
    process.stderr.write('ui-liveregion-evidence: Playwright unavailable; no evidence collected\n');
    process.exit(3);
  }

  const launch = {};
  if (process.env.PLAYWRIGHT_CHROMIUM_PATH) {
    launch.executablePath = process.env.PLAYWRIGHT_CHROMIUM_PATH;
  }
  const browser = await playwright.chromium.launch(launch);

  const states = [];
  const findings = [];
  const skipped = [];

  try {
    // Induction first, and unconditionally. It needs nothing from the repo, so every route
    // given is measured whether or not anyone has adopted the hook.
    if (opts.routes.length > 0 && !opts.noInduce) {
      const induced = await collectInduced(browser, opts.baseUrl, opts.routes);
      states.push(...induced.states);
      findings.push(...induced.findings);
      skipped.push(...induced.skipped);
    }

    // The manifest is additive: states no request can produce — form validation, optimistic
    // updates, client-side route changes — are unreachable by induction, and this is where a
    // repo declares them.
    const manifest = loadManifest(path.resolve(opts.manifest));
    if (manifest) {
      const declared = await collectDeclared(browser, opts.baseUrl, manifest);
      states.push(...declared.states);
      findings.push(...declared.findings);
    }
  } finally {
    await browser.close();
  }

  const report = { schema: 1, status: 'ok', states, findings, skipped };
  writeReport(report);

  for (const finding of report.findings) {
    process.stdout.write(`${finding.rule}:${finding.route}[${finding.state}]: ${finding.detail}\n`);
  }
  for (const row of report.states) {
    if (!report.findings.some((f) => f.route === row.route && f.state === row.state)) {
      const how = row.induced ? 'induced' : 'declared';
      process.stdout.write(`ok ${row.route}[${row.state}]: ${how}, settled ${row.settled}\n`);
    }
  }
  for (const row of report.skipped) {
    process.stdout.write(`skip ${row.route}: ${row.reason}\n`);
  }
  if (report.states.length === 0 && report.skipped.length === 0) {
    process.stdout.write('ui-liveregion-evidence: no routes given and no manifest found\n');
  }
  process.exit(report.findings.length > 0 ? 1 : 0);
}

if (process.argv[1] && process.argv[1].endsWith('ui-liveregion-evidence.mjs')) {
  await main();
}
