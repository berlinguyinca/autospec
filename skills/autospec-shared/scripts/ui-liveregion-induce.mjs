#!/usr/bin/env node
// ui-liveregion-induce.mjs — drive app states with zero app cooperation.
//
// The manifest in `ui-liveregion-evidence.mjs` makes coverage opt-in, and a gate nobody
// adopts measures nothing. This module removes the opt-in for the states that matter most:
// they are network-driven, and the network is interceptable.
//
//   hold the response   the app renders its own pending state — this is the starting
//                       condition for both runs below, not a state judged on its own,
//                       because a loading message rendered during mount is not a
//                       transition and no screen reader announces it
//   error               reply 500 — the app runs its own error path
//   success             reply normally, once we are watching
//
// Whether the pending state was cleaned up *is* judged: `LIVE_REGION_STUCK_BUSY` reads the
// settle-time snapshot of both runs, so an aria-busy left set after the response arrives is
// caught without loading needing a run of its own.
//
// The app drives itself through its own state machine. Nothing is clicked and nothing is
// mutated, so this is safe against a deployed app in a way that synthesising clicks is not.
//
// The sequence matters more than the interception:
//
//   1. register the hold BEFORE navigating
//   2. navigate — the app mounts and settles into its own pending state
//   3. install the observer NOW, at that quiet point
//   4. release the response
//   5. observe only the transition
//
// Step 3 is the whole trick, and it is why this works on a single-page app. Observing from
// document-start instead would make an initial mount indistinguishable from a state change:
// a framework inserting the entire page reads as every live region being inserted at once.
// Holding the response manufactures a stable "before" that no amount of DOM heuristics
// could infer.
//
// Exit and reporting are the caller's; this module collects and judges.

import {
  judgeState,
  OBSERVER,
  REGIONS_PROBE,
  QUIET_MS,
  MAX_SETTLE_MS,
} from './ui-liveregion-core.mjs';

// How long to let the app mount and reach its pending state before watching. Quiescence,
// not a guessed constant — but bounded, since an app that polls never goes quiet.
export const MOUNT_SETTLE_MS = 2500;
// How long to give the app to react at all once its response is released. An app that never
// reacts is the INDUCED_STATE_IGNORED finding, so this bound decides that verdict and has to
// be generous enough that a slow render is not mistaken for no render.
export const REACT_MS = 3000;
// Discovery navigation and collection bounds. `networkidle` is deliberately not used: an
// app that polls or holds a socket never reaches it, so it would hang a gate on exactly the
// applications most worth measuring. A bounded quiet window answers the same question —
// which data requests does this route make — without depending on the network ever stopping.
export const NAV_MS = 15000;
export const DISCOVERY_QUIET_MS = 600;
export const DISCOVERY_MAX_MS = 5000;

/**
 * The states induction can produce, with the politeness each implies.
 *
 * The kind is implicit here where the manifest requires it to be declared, and that is not
 * a double standard. A declared state carries a name someone else chose, and reading
 * "assertive" off an arbitrary string is a regex pretending to be a judgement. These two
 * names are chosen *by the inducer* and describe what it did: `error` is a request this
 * module made fail, so an interruption is warranted; `success` is data arriving normally,
 * so it waits its turn.
 */
export const INDUCED_STATES = [
  { name: 'success', kind: 'status', induced: true },
  { name: 'error', kind: 'alert', induced: true },
];

/** Requests worth holding. Intercepting everything would break the page's own HTML and CSS. */
export function isDataRequest(resourceType) {
  return resourceType === 'fetch' || resourceType === 'xhr';
}

/**
 * The data requests a route makes on a clean load. An empty result means there is no
 * network-driven state to induce — which is a skip with a reason, never a pass.
 */
export async function discoverEndpoints(browser, url) {
  const context = await browser.newContext();
  const page = await context.newPage();
  const seen = new Set();
  page.on('request', (request) => {
    if (isDataRequest(request.resourceType())) seen.add(request.url());
  });
  let lastSeen = Date.now();
  page.on('request', (request) => {
    if (isDataRequest(request.resourceType())) lastSeen = Date.now();
  });

  try {
    await page.goto(url, { waitUntil: 'domcontentloaded', timeout: NAV_MS });
    // Collect until no new data request has appeared for a beat, capped. A route that keeps
    // asking for things forever still yields the endpoints it asked for first.
    const deadline = Date.now() + DISCOVERY_MAX_MS;
    while (Date.now() < deadline && Date.now() - lastSeen < DISCOVERY_QUIET_MS) {
      await page.waitForTimeout(100);
    }
  } catch {
    // A route that will not load still tells us what it managed to ask for.
  } finally {
    await context.close();
  }
  return [...seen];
}

/** Wait for the page to stop mutating, or give up. Returns true when it actually went quiet. */
async function settle(page, cap) {
  return page
    .waitForFunction(
      `performance.now() - (window.__autospecLastMutation || 0) > ${QUIET_MS}`,
      null,
      { timeout: cap },
    )
    .then(() => true)
    .catch(() => false);
}

/**
 * Drive one route into one induced state and record what happened.
 *
 * `targets` are the URLs to hold; everything else loads normally, so the page's own markup,
 * styles and scripts are unaffected.
 */
export async function induceState(browser, url, targets, state) {
  const context = await browser.newContext();
  const page = await context.newPage();
  let release;
  const held = new Promise((resolve) => { release = resolve; });

  try {
    // Registered before navigating: the handler parks each data request until we release it.
    await page.route(
      (candidate) => targets.includes(candidate.href),
      async (route) => {
        await held;
        if (state.name === 'error') {
          await route.fulfill({
            status: 500,
            contentType: 'application/json',
            body: '{"error":"induced by autospec"}',
          });
        } else {
          await route.continue();
        }
      },
    );

    await page.goto(url, { waitUntil: 'domcontentloaded' });

    // Let the mount finish. __autospecLastMutation does not exist yet, so seed it and wait
    // for genuine quiet rather than trusting a fixed delay.
    await page.evaluate('window.__autospecLastMutation = performance.now()');
    await settle(page, MOUNT_SETTLE_MS);

    await page.evaluate(OBSERVER);
    // Seeded AFTER the observer, because installing it zeroes __autospecLastMutation. Seed
    // before, and the quiescence wait below is satisfied the instant it is asked — the page
    // has been alive far longer than QUIET_MS — so it returns before the released response
    // has even arrived and every app reads as one that ignored the state.
    await page.evaluate('window.__autospecLastMutation = performance.now()');
    release();

    // Wait for the app to react at all before waiting for it to stop. Quiescence alone
    // cannot tell "has not reacted yet" from "has finished reacting".
    await page
      .waitForFunction('window.__autospecMutations > 0', null, { timeout: REACT_MS })
      .catch(() => {});
    const settled = await settle(page, MAX_SETTLE_MS);
    return {
      hookFound: true,
      hookError: '',
      mutations: await page.evaluate('window.__autospecMutations'),
      events: await page.evaluate('window.__autospecEvents'),
      regions: await page.evaluate(REGIONS_PROBE),
      settled,
    };
  } finally {
    release();
    await context.close();
  }
}

/**
 * Induce every state on every route. Routes with no data requests are recorded as skipped
 * with a reason rather than silently passing — a static page genuinely has no state to
 * drive, and saying so is different from saying it was checked.
 */
export async function collectInduced(browser, baseUrl, routes) {
  const states = [];
  const findings = [];
  const skipped = [];

  for (const route of routes) {
    const url = new URL(route, baseUrl).toString();
    const targets = await discoverEndpoints(browser, url);
    if (targets.length === 0) {
      skipped.push({ route, reason: 'no fetch or xhr requests; nothing to induce' });
      continue;
    }
    for (const state of INDUCED_STATES) {
      const observation = await induceState(browser, url, targets, state);
      findings.push(...judgeState(route, state, observation));
      states.push({
        route,
        state: state.name,
        kind: state.kind,
        induced: true,
        endpoints: targets.length,
        mutations: observation.mutations,
        settled: observation.settled ? 'quiet' : `capped at ${MAX_SETTLE_MS}ms`,
      });
    }
  }

  return { states, findings, skipped };
}
