#!/usr/bin/env node
// ui-liveregion-core.mjs — what the two live-region collectors share.
//
// Both halves of this tier judge the same recording: `ui-liveregion-evidence.mjs` drives
// the states a manifest declares, and `ui-liveregion-induce.mjs` drives states it
// manufactures by controlling the network. They must agree on what counts as an
// announcement, so the observer, the probes and the judge live here rather than in either.
//
// This module exists for a second reason, learned the hard way. The judge originally lived
// in the evidence module and the inducer imported it, while the evidence CLI imported the
// inducer back — a cycle. Under top-level await that is not merely untidy: importing a
// module that is still evaluating makes the import wait for that evaluation, which is
// waiting for the import. The CLI hung indefinitely while both halves worked perfectly when
// called from anywhere else. A shared leaf module has no cycle to deadlock on.

import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));

// A fixed sleep flakes in both directions, so a state is settled by quiescence: this long
// with no mutation at all, capped so a page that never goes quiet still terminates.
export const QUIET_MS = 150;
export const MAX_SETTLE_MS = 3000;

export const LIVE_SELECTOR =
  '[aria-live], [role="status"], [role="alert"], [role="log"], output';

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
    // An induced state that changed nothing is an app defect; a declared state that changed
    // nothing is almost always a stale name in the manifest. Same observation, different
    // bug, so they are named differently — an app that ignores a failed request leaves the
    // user looking at stale content with no indication anything went wrong.
    return state.induced
      ? [{
        ...at,
        rule: 'INDUCED_STATE_IGNORED',
        detail:
          `the app did nothing at all when driven into '${state.name}' — no render, no ` +
          'announcement; a failed request that changes nothing leaves stale content on screen',
      }]
      : [{
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
export const OBSERVER = `(() => {
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
export const REGIONS_PROBE = `(() => {
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

export async function loadPlaywright() {
  const { findPlaywrightPath } = await import(path.resolve(__dirname, 'gen-screenshots.mjs'));
  const found = findPlaywrightPath();
  if (!found) return null;
  return import(found);
}
