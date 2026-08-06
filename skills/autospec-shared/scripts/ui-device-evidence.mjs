#!/usr/bin/env node
// ui-device-evidence.mjs — runtime device evidence (design spec L4a).
//
// Renders each route across real device profiles rather than a width sweep, and adds the
// two dedicated WCAG runs the spec asks for:
//
//   DEVICE_OVERFLOW   the page scrolls sideways on a device profile
//   DEVICE_REFLOW     sideways scrolling at 320 CSS px (WCAG 1.4.10)
//   DEVICE_ZOOM_CLIP  sideways scrolling at 200% zoom (WCAG 1.4.4)
//   DEVICE_TARGET_TOO_SMALL  an interactive target under 24px on a coarse pointer (2.5.8)
//   DEVICE_HOVER_ONLY_INPUT  a coarse-pointer profile reports any-hover: none, and the
//                            route still hides an interactive control behind :hover
//
// A device profile is not a viewport. Playwright's descriptors carry user agent, device
// pixel ratio and touch, so `pointer: coarse` and `any-hover: none` resolve the way they
// do on the device — measured: iPhone 13 reports coarse=true, noHover=true, dpr=3, while
// a 390px desktop viewport reports none of those. A width sweep tests none of it.
//
// 200% zoom is emulated by halving the viewport rather than by a zoom API: at 1280 CSS px
// the two are equivalent for reflow purposes, and Playwright exposes no page zoom.
//
// Usage:
//   ui-device-evidence.mjs --base-url http://localhost:3000 --routes / /runs
//   ui-device-evidence.mjs --base-url ... --routes / --json out.json
//
// Exit: 0 all routes pass, 1 one or more findings, 3 Playwright unavailable.
//
// Env:
//   PLAYWRIGHT_CHROMIUM_PATH  launch this chromium binary instead of the bundled one.

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));

// Portrait and landscape are both listed: an orientation change is a layout change, and
// a tablet in landscape is the case a phone-plus-desktop sweep never covers.
export const DEVICE_PROFILES = [
  'iPhone 13',
  'iPad (gen 7)',
  'iPad (gen 7) landscape',
  'Pixel 7',
  'Desktop Chrome',
];

// WCAG 2.5.8 asks 24 CSS px on the shorter side. Anything smaller is hard to hit on a
// touch screen, and the rule only applies where the pointer is coarse.
export const MIN_TARGET_PX = 24;

// ── assertions ────────────────────────────────────────────────────────────────

/** Sideways scrolling, allowing a pixel of rounding slack. */
export function overflowsHorizontally(probe) {
  return (probe.scrollWidth - probe.clientWidth) > 1;
}

/** Interactive targets too small to hit, reported only for coarse pointers. */
export function undersizedTargets(probe) {
  if (!probe.coarse) return [];
  return (probe.targets || []).filter(
    (t) => Math.min(t.width, t.height) < MIN_TARGET_PX,
  );
}

/**
 * Judge one route on one profile. Pure: takes a recorded probe, returns findings. The
 * browser half is separable so this is testable without one, and the fixtures used to
 * test it are recorded from a real run rather than invented.
 */
export function judgeProfile(route, profile, probe) {
  const findings = [];

  if (overflowsHorizontally(probe)) {
    findings.push({
      rule: 'DEVICE_OVERFLOW',
      route,
      profile,
      detail:
        `content is ${probe.scrollWidth}px wide in a ${probe.clientWidth}px viewport, ` +
        'so the page scrolls sideways',
    });
  }

  const small = undersizedTargets(probe);
  if (small.length > 0) {
    const worst = small[0];
    findings.push({
      rule: 'DEVICE_TARGET_TOO_SMALL',
      route,
      profile,
      detail:
        `${small.length} interactive target(s) under ${MIN_TARGET_PX}px on a coarse ` +
        `pointer, smallest '${worst.label}' at ${Math.round(worst.width)}x${Math.round(worst.height)} (WCAG 2.5.8)`,
    });
  }

  if (probe.noHover && probe.hoverOnlyControls > 0) {
    findings.push({
      rule: 'DEVICE_HOVER_ONLY_INPUT',
      route,
      profile,
      detail:
        `${probe.hoverOnlyControls} control(s) reveal themselves only on hover, and this ` +
        'device cannot hover',
    });
  }

  return findings;
}

/** Judge the two dedicated WCAG runs. */
export function judgeWcagRuns(route, reflowProbe, zoomProbe) {
  const findings = [];
  if (overflowsHorizontally(reflowProbe)) {
    findings.push({
      rule: 'DEVICE_REFLOW',
      route,
      profile: '320px',
      detail:
        `content is ${reflowProbe.scrollWidth}px wide at 320px, forcing two-dimensional ` +
        'scrolling (WCAG 1.4.10)',
    });
  }
  if (overflowsHorizontally(zoomProbe)) {
    findings.push({
      rule: 'DEVICE_ZOOM_CLIP',
      route,
      profile: '200%',
      detail:
        `content is ${zoomProbe.scrollWidth}px wide at 200% zoom, so it is clipped or ` +
        'requires sideways scrolling (WCAG 1.4.4)',
    });
  }
  return findings;
}

// ── browser collection ────────────────────────────────────────────────────────

const PROBE_SOURCE = `(() => {
  const doc = document.documentElement;
  const interactive = Array.from(
    document.querySelectorAll('a[href], button, input, select, textarea, [role="button"], [tabindex]:not([tabindex="-1"])'),
  );
  const targets = interactive
    .map((el) => {
      const r = el.getBoundingClientRect();
      const label = (el.getAttribute('aria-label') || el.textContent || el.tagName)
        .trim().slice(0, 40) || el.tagName;
      return { label, width: r.width, height: r.height };
    })
    // Zero-sized elements are hidden, not undersized; a skip link parked off-screen
    // would otherwise be reported on every route.
    .filter((t) => t.width > 0 && t.height > 0);

  // Controls whose only visible affordance is a hover rule. Counted from the stylesheets
  // rather than guessed: a :hover selector with no :focus counterpart anywhere.
  //
  // Grouped selectors are split first. '.btn:hover, .btn:focus-visible' arrives as one
  // selectorText, and treating it whole reports the very pattern it demonstrates —
  // measured against a fixture that pairs them in one rule, which is how most people
  // write it.
  let hoverOnly = 0;
  try {
    const selectors = [];
    for (const sheet of Array.from(document.styleSheets)) {
      let rules;
      try { rules = Array.from(sheet.cssRules || []); } catch (e) { continue; }
      for (const rule of rules) {
        if (!rule.selectorText) continue;
        for (const part of rule.selectorText.split(',')) selectors.push(part.trim());
      }
    }
    const focusBases = new Set(
      selectors
        .filter((s) => s.includes(':focus'))
        .map((s) => s.replace(/:focus(-visible|-within)?/g, '')),
    );
    for (const sel of selectors) {
      if (!sel.includes(':hover')) continue;
      if (!focusBases.has(sel.replace(/:hover/g, ''))) hoverOnly += 1;
    }
  } catch (e) { /* cross-origin sheets are not readable; count what is */ }

  return {
    coarse: matchMedia('(pointer: coarse)').matches,
    noHover: matchMedia('(any-hover: none)').matches,
    dpr: window.devicePixelRatio,
    scrollWidth: doc.scrollWidth,
    clientWidth: doc.clientWidth,
    targets,
    hoverOnlyControls: hoverOnly,
  };
})()`;

export async function loadPlaywright() {
  const { findPlaywrightPath } = await import(
    path.resolve(__dirname, 'gen-screenshots.mjs')
  );
  const found = findPlaywrightPath();
  if (!found) return null;
  return import(found);
}

async function probeWith(browser, contextOptions, url) {
  const context = await browser.newContext(contextOptions);
  const page = await context.newPage();
  try {
    await page.goto(url, { waitUntil: 'load' });
    return await page.evaluate(PROBE_SOURCE);
  } finally {
    await context.close();
  }
}

export async function collectEvidence(baseUrl, routes, profileNames = DEVICE_PROFILES) {
  const playwright = await loadPlaywright();
  if (!playwright) {
    return {
      schema: 1,
      status: 'blocked_missing_playwright',
      detail: 'Playwright is not installed; device evidence was not collected',
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
      const profiles = [];

      for (const name of profileNames) {
        const descriptor = playwright.devices[name];
        if (!descriptor) {
          findings.push({
            rule: 'DEVICE_PROFILE_UNKNOWN',
            route,
            profile: name,
            detail: 'this Playwright build has no such device descriptor',
          });
          continue;
        }
        const probe = await probeWith(browser, descriptor, url);
        profiles.push({ profile: name, coarse: probe.coarse, dpr: probe.dpr, width: probe.clientWidth });
        findings.push(...judgeProfile(route, name, probe));
      }

      const reflow = await probeWith(browser, { viewport: { width: 320, height: 640 } }, url);
      // Half of a 1280px desktop viewport: at that width, halving and doubling the text
      // are equivalent for reflow, and Playwright exposes no page-zoom control.
      const zoom = await probeWith(browser, { viewport: { width: 640, height: 480 } }, url);
      findings.push(...judgeWcagRuns(route, reflow, zoom));

      results.push({ route, profiles, reflowWidth: reflow.scrollWidth, zoomWidth: zoom.scrollWidth });
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
    process.stderr.write('Usage: ui-device-evidence.mjs --base-url URL --routes / [/more]\n');
    process.exit(3);
  }

  const report = await collectEvidence(opts.baseUrl, opts.routes);
  if (opts.json) {
    fs.mkdirSync(path.dirname(path.resolve(opts.json)), { recursive: true });
    fs.writeFileSync(opts.json, `${JSON.stringify(report, null, 2)}\n`);
  }

  if (report.status === 'blocked_missing_playwright') {
    process.stderr.write('ui-device-evidence: Playwright unavailable; no evidence collected\n');
    process.exit(3);
  }

  for (const finding of report.findings) {
    process.stdout.write(`${finding.rule}:${finding.route}:${finding.profile}: ${finding.detail}\n`);
  }
  for (const row of report.routes) {
    if (!report.findings.some((f) => f.route === row.route)) {
      process.stdout.write(
        `ok ${row.route}: ${row.profiles.length} profiles, reflow ${row.reflowWidth}px, zoom ${row.zoomWidth}px\n`,
      );
    }
  }
  process.exit(report.findings.length > 0 ? 1 : 0);
}

if (process.argv[1] && process.argv[1].endsWith('ui-device-evidence.mjs')) {
  await main();
}
