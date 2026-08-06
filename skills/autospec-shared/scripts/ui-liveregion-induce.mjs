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
// This commit is the discovery half: which data requests a route makes, and therefore what
// there is to hold. The holding itself follows.

// Discovery navigation and collection bounds. `networkidle` is deliberately not used: an
// app that polls or holds a socket never reaches it, so it would hang a gate on exactly the
// applications most worth measuring. A bounded quiet window answers the same question —
// which data requests does this route make — without depending on the network ever stopping.
export const NAV_MS = 15000;
export const DISCOVERY_QUIET_MS = 600;
export const DISCOVERY_MAX_MS = 5000;

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
