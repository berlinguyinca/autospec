#!/usr/bin/env node
// ui-keyboard-evidence.mjs — keyboard robot (design spec L4c, first phase).
//
// Tab-traverses every route and asserts what a keyboard user needs:
//
//   KEYBOARD_TRAP        tabbing cycles without ever reaching some focusable content
//   FOCUS_NOT_VISIBLE    a stop with no visible focus indicator (WCAG 2.4.7)
//   FOCUS_OBSCURED       the focused control is painted over (WCAG 2.4.11)
//   FOCUS_ORDER_JUMPS    focus moves back up the page, against visual order (2.4.3)
//   NO_KEYBOARD_PATH     the route has focusable content that tabbing never reaches
//
// This tier converts checks that were previously judgement calls into machine-verifiable
// ones. What stays human-reviewed is the honest residual: whether an accessible name is
// *meaningful*, whether link text makes sense out of context, and plain language.
//
// Every assertion here was measured against a real browser before being written:
// a suppressed indicator reports outline style `none` while the browser default reports
// `auto`, so style rather than width is the signal; a control under a sticky header is
// reported by elementFromPoint as covered by that header; and a focus trap shows as a
// cycle that never includes focusable elements outside it.
//
// Usage:
//   ui-keyboard-evidence.mjs --base-url http://localhost:3000 --routes / /runs
//
// Exit: 0 clean, 1 findings, 3 Playwright unavailable.
//
// Env:
//   PLAYWRIGHT_CHROMIUM_PATH  launch this chromium binary instead of the bundled one.

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));

// Tabbing is bounded so a trap cannot spin forever. Twice the candidate count plus a
// margin is enough to complete a full cycle and prove it repeats.
export const TAB_MARGIN = 4;
export const MAX_TABS = 120;

// How far focus may move back up the page before it reads as an order violation. A row
// of controls varies by a few pixels; a jump to an earlier section does not.
export const ORDER_TOLERANCE_PX = 40;

// ── assertions ────────────────────────────────────────────────────────────────

/** A stop shows focus when it draws an outline or a shadow. */
export function hasVisibleFocus(stop) {
  if (stop.outlineStyle && stop.outlineStyle !== 'none') return true;
  return Boolean(stop.boxShadow);
}

/**
 * The distinct elements a tab cycle reaches. A trap is a cycle that never reaches
 * everything focusable, so the comparison is against the page's own candidate list
 * rather than against a count guessed from the sequence.
 */
export function reachedKeys(stops) {
  return new Set(stops.filter((s) => !s.none).map((s) => s.key));
}

/**
 * The traversal up to the point it starts repeating. Tab wraps from the last control
 * back to the first, so everything after the first repeat is the same cycle again.
 */
export function firstCycle(stops) {
  const cycle = [];
  const seen = new Set();
  for (const stop of stops) {
    if (stop.none) continue;
    if (seen.has(stop.key)) break;
    seen.add(stop.key);
    cycle.push(stop);
  }
  return cycle;
}

/**
 * Stops where focus moved back up the page, against reading order.
 *
 * Judged over the first cycle only. The wrap from the last control to the first is a
 * move back up the page by definition, and reporting it would fail every correctly
 * ordered page — measured on a page whose last button sits at y=79 and whose skip link
 * sits at y=24.
 */
export function orderJumps(stops, tolerance = ORDER_TOLERANCE_PX) {
  const jumps = [];
  let previous = null;
  for (const stop of firstCycle(stops)) {
    if (previous && stop.rect.y < previous.rect.y - tolerance) {
      jumps.push({ from: previous.key, to: stop.key, fromY: previous.rect.y, toY: stop.rect.y });
    }
    previous = stop;
  }
  return jumps;
}

/**
 * Judge one route from its recorded traversal. Pure: takes stops and the page's
 * focusable candidates, returns findings, touches no browser.
 */
export function judgeRoute(route, stops, candidateKeys) {
  const findings = [];
  const reached = reachedKeys(stops);

  const unreached = candidateKeys.filter((k) => !reached.has(k));
  if (unreached.length > 0) {
    // Distinguishing a trap from merely-unreachable content: a trap keeps handing focus
    // back, so the traversal repeats a short cycle while content sits outside it.
    const cycled = stops.length > reached.size * 2;
    findings.push({
      rule: cycled ? 'KEYBOARD_TRAP' : 'NO_KEYBOARD_PATH',
      route,
      detail: cycled
        ? `tabbing cycles through ${reached.size} control(s) and never reaches ` +
          `${unreached.length} other(s), starting with '${unreached[0]}'`
        : `${unreached.length} focusable control(s) are never reached by tabbing, ` +
          `starting with '${unreached[0]}'`,
    });
  }

  for (const stop of stops) {
    if (stop.none) continue;
    if (!hasVisibleFocus(stop)) {
      findings.push({
        rule: 'FOCUS_NOT_VISIBLE',
        route,
        detail: `'${stop.key}' takes focus with no visible indicator (WCAG 2.4.7)`,
      });
    }
    if (stop.covered) {
      findings.push({
        rule: 'FOCUS_OBSCURED',
        route,
        detail:
          `'${stop.key}' is focused but painted over by '${stop.coveredBy}' (WCAG 2.4.11)`,
      });
    }
  }

  for (const jump of orderJumps(stops)) {
    findings.push({
      rule: 'FOCUS_ORDER_JUMPS',
      route,
      detail:
        `focus moves from '${jump.from}' at y=${jump.fromY} back up to '${jump.to}' at ` +
        `y=${jump.toY}, against reading order (WCAG 2.4.3)`,
    });
  }

  return findings;
}

// ── browser collection ────────────────────────────────────────────────────────

// A stable identity for an element, plus everything the assertions need. Keys are
// content-derived because DOM paths change with every refactor and would make a baseline
// useless.
const STOP_PROBE = `(() => {
  const el = document.activeElement;
  if (!el || el === document.body || el === document.documentElement) return { none: true };
  const r = el.getBoundingClientRect();
  const cs = getComputedStyle(el);
  const label = (el.getAttribute('aria-label') || el.textContent || el.value || '').trim().slice(0, 30);
  let covered = false, coveredBy = '';
  if (r.width > 0 && r.height > 0) {
    const cx = Math.min(Math.max(r.left + r.width / 2, 1), innerWidth - 1);
    const cy = Math.min(Math.max(r.top + r.height / 2, 1), innerHeight - 1);
    const hit = document.elementFromPoint(cx, cy);
    if (hit && hit !== el && !el.contains(hit) && !hit.contains(el)) {
      covered = true;
      coveredBy = hit.tagName.toLowerCase() + (hit.className ? '.' + String(hit.className).split(' ')[0] : '');
    }
  }
  return {
    key: el.tagName.toLowerCase() + (label ? ':' + label : '') + (el.id ? '#' + el.id : ''),
    outlineStyle: cs.outlineStyle,
    boxShadow: cs.boxShadow === 'none' ? '' : 'yes',
    rect: { x: Math.round(r.x), y: Math.round(r.y + (window.scrollY || 0)) },
    covered,
    coveredBy,
  };
})()`;

// Everything the page offers to a keyboard, by the same key shape, so the two lists are
// comparable. Hidden and disabled controls are excluded: they are not expected stops.
const CANDIDATES_PROBE = `(() => {
  const sel = 'a[href], button, input, select, textarea, [tabindex]:not([tabindex="-1"]), [contenteditable="true"]';
  return Array.from(document.querySelectorAll(sel))
    .filter((el) => {
      if (el.disabled) return false;
      if (el.getAttribute('aria-hidden') === 'true') return false;
      const cs = getComputedStyle(el);
      if (cs.display === 'none' || cs.visibility === 'hidden') return false;
      const r = el.getBoundingClientRect();
      // A skip link parked off-screen is reachable and expected; a zero-sized element
      // is not a control.
      return r.width > 0 || r.height > 0;
    })
    .map((el) => {
      const label = (el.getAttribute('aria-label') || el.textContent || el.value || '').trim().slice(0, 30);
      return el.tagName.toLowerCase() + (label ? ':' + label : '') + (el.id ? '#' + el.id : '');
    });
})()`;

export async function loadPlaywright() {
  const { findPlaywrightPath } = await import(
    path.resolve(__dirname, 'gen-screenshots.mjs')
  );
  const found = findPlaywrightPath();
  if (!found) return null;
  return import(found);
}

/** Tab through one route, recording every stop. */
export async function traverse(browser, url) {
  const context = await browser.newContext();
  const page = await context.newPage();
  try {
    await page.goto(url, { waitUntil: 'load' });
    const candidates = await page.evaluate(CANDIDATES_PROBE);
    const budget = Math.min(candidates.length * 2 + TAB_MARGIN, MAX_TABS);
    const stops = [];
    for (let i = 0; i < budget; i += 1) {
      await page.keyboard.press('Tab');
      stops.push(await page.evaluate(STOP_PROBE));
    }
    return { candidates, stops };
  } finally {
    await context.close();
  }
}

export async function collectEvidence(baseUrl, routes) {
  const playwright = await loadPlaywright();
  if (!playwright) {
    return {
      schema: 1,
      status: 'blocked_missing_playwright',
      detail: 'Playwright is not installed; keyboard evidence was not collected',
      routes: [],
      findings: [],
    };
  }

  const launch = {};
  if (process.env.PLAYWRIGHT_CHROMIUM_PATH) {
    launch.executablePath = process.env.PLAYWRIGHT_CHROMIUM_PATH;
  }
  const browser = await playwright.chromium.launch(launch);
  const results = [];
  const findings = [];

  try {
    for (const route of routes) {
      const url = new URL(route, baseUrl).toString();
      const { candidates, stops } = await traverse(browser, url);
      findings.push(...judgeRoute(route, stops, candidates));
      results.push({
        route,
        candidates: candidates.length,
        reached: reachedKeys(stops).size,
        tabs: stops.length,
      });
    }
  } finally {
    await browser.close();
  }

  return { schema: 1, status: 'ok', routes: results, findings };
}

// ── CLI ───────────────────────────────────────────────────────────────────────

function parseArgs(argv) {
  const opts = { baseUrl: '', routes: [], json: '' };
  for (let i = 0; i < argv.length; i += 1) {
    if (argv[i] === '--base-url') opts.baseUrl = argv[++i];
    else if (argv[i] === '--json') opts.json = argv[++i];
    else if (argv[i] === '--routes') {
      while (i + 1 < argv.length && !argv[i + 1].startsWith('--')) opts.routes.push(argv[++i]);
    }
  }
  return opts;
}

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  if (!opts.baseUrl || opts.routes.length === 0) {
    process.stderr.write('Usage: ui-keyboard-evidence.mjs --base-url URL --routes / [/more]\n');
    process.exit(3);
  }

  const report = await collectEvidence(opts.baseUrl, opts.routes);
  if (opts.json) {
    fs.mkdirSync(path.dirname(path.resolve(opts.json)), { recursive: true });
    fs.writeFileSync(opts.json, `${JSON.stringify(report, null, 2)}\n`);
  }

  if (report.status === 'blocked_missing_playwright') {
    process.stderr.write('ui-keyboard-evidence: Playwright unavailable; no evidence collected\n');
    process.exit(3);
  }

  // Repeated rules on one route are collapsed: a page that hides every focus ring would
  // otherwise print one line per control and bury the other findings.
  const seen = new Set();
  for (const finding of report.findings) {
    const key = `${finding.rule}:${finding.route}`;
    const count = report.findings.filter((f) => `${f.rule}:${f.route}` === key).length;
    if (seen.has(key)) continue;
    seen.add(key);
    const suffix = count > 1 ? `  (+${count - 1} more on this route)` : '';
    process.stdout.write(`${finding.rule}:${finding.route}: ${finding.detail}${suffix}\n`);
  }
  for (const row of report.routes) {
    if (!report.findings.some((f) => f.route === row.route)) {
      process.stdout.write(
        `ok ${row.route}: ${row.reached}/${row.candidates} controls reached in ${row.tabs} tabs\n`,
      );
    }
  }
  process.exit(report.findings.length > 0 ? 1 : 0);
}

if (process.argv[1] && process.argv[1].endsWith('ui-keyboard-evidence.mjs')) {
  await main();
}
