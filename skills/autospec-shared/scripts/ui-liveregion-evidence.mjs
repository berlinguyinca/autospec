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
// This commit is the pure half: the manifest and the judgements, which take a recorded
// observation and touch no browser. The collection that produces those observations, and
// the CLI, follow.

import fs from 'node:fs';

export const DEFAULT_MANIFEST = '.autospec/ui-test-hooks.json';
export const DEFAULT_HOOK = '__autospec.setState';

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
