#!/usr/bin/env node
// ui-motion-evidence.mjs — runtime motion evidence (design spec L4b).
//
// Renders each route twice, once normally and once with prefers-reduced-motion: reduce,
// and asserts:
//
//   MOTION_ABSENT      the default run shows no motion at all
//   MOTION_NOT_REDUCED the reduce run still moves things
//   MOTION_UNBOUNDED   something animates forever with no pause control (WCAG 2.2.2)
//
// MOTION_ABSENT is the reason this file exists. Every other gate in the pipeline —
// lint, the fidelity judge, axe, the aesthetic rubric — passes a static, animation-free
// page cleanly. This is the only check that fails a UI for being inert rather than
// wrong, so it is the pipeline's sole defence against blandness.
//
// Usage:
//   ui-motion-evidence.mjs --base-url http://localhost:3000 --routes / /runs
//   ui-motion-evidence.mjs --base-url ... --routes / --json out.json
//
// Exit: 0 all routes pass, 1 one or more findings, 3 Playwright unavailable.
// A host without Playwright reports status blocked_missing_playwright and exits 3
// rather than reporting the routes clean.
//
// Env:
//   PLAYWRIGHT_CHROMIUM_PATH  launch this chromium binary instead of the bundled one,
//                             for hosts pinned to a browser build the package did not
//                             download.

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));

// ── what counts as motion ─────────────────────────────────────────────────────

// Properties whose animation moves or resizes something. Deliberately the same
// definition scripts/lint-ui.sh uses for UI_NO_REDUCED_MOTION: a colour or opacity
// fade is feedback, not motion, and must not satisfy the anti-blandness assertion.
// Measured against a real page: a keyframe animation reports
// ["composite", "opacity", "transform"], so intersecting against this set is what
// separates it from a background-color transition reporting ["background-color"].
export const MOTION_PROPERTIES = new Set([
  'transform', 'translate', 'rotate', 'scale', 'offsetDistance', 'offsetPath',
  'top', 'left', 'right', 'bottom', 'inset',
  'width', 'height', 'margin', 'marginTop', 'marginLeft', 'marginRight', 'marginBottom',
]);

/** The motion-bearing properties an animation record touches. */
export function motionProperties(anim) {
  return (anim.properties || []).filter((p) => MOTION_PROPERTIES.has(p));
}

/** Animations that actually move something. */
export function movingAnimations(probe) {
  return (probe.anims || []).filter((a) => motionProperties(a).length > 0);
}

/**
 * An animation runs indefinitely when it repeats forever, or long enough that WCAG
 * 2.2.2 wants a pause control. `iterations` comes back as null for `infinite`.
 */
export function isUnbounded(anim) {
  const iterations = anim.iterations;
  const forever = iterations === null || iterations === Infinity || iterations > 1e6;
  if (forever) return true;
  const total = (anim.duration || 0) * (iterations || 1);
  return total > 5000;
}

// ── assertions ────────────────────────────────────────────────────────────────

/**
 * Judge one route from its two probes. Pure: takes recorded probe objects, returns
 * findings, touches nothing. The browser half is separable so this can be tested
 * without one — and the fixtures used to test it are recorded from a real page rather
 * than invented, or the test would only prove the assertions agree with themselves.
 */
export function judgeRoute(route, defaultProbe, reducedProbe) {
  const findings = [];
  const moving = movingAnimations(defaultProbe);

  if (moving.length === 0) {
    findings.push({
      rule: 'MOTION_ABSENT',
      route,
      detail:
        'nothing moves on this route by default — every other gate passes a static ' +
        'page, so an inert UI reaches production unless this one objects',
    });
  }

  const stillMoving = movingAnimations(reducedProbe);
  if (stillMoving.length > 0) {
    findings.push({
      rule: 'MOTION_NOT_REDUCED',
      route,
      detail:
        `${stillMoving.length} animation(s) still move under prefers-reduced-motion: ` +
        `${stillMoving.map((a) => a.id).join(', ')} (WCAG 2.3.3)`,
    });
  }

  for (const anim of moving) {
    if (isUnbounded(anim)) {
      findings.push({
        rule: 'MOTION_UNBOUNDED',
        route,
        detail:
          `'${anim.id}' runs past five seconds with no pause control ` +
          `(duration ${anim.duration}ms x ${anim.iterations === null ? 'infinite' : anim.iterations}) — WCAG 2.2.2`,
      });
    }
  }

  return findings;
}

// ── browser collection ────────────────────────────────────────────────────────

// Collected in the page. Reports every animation with the properties its keyframes
// touch, so the judgement above can tell movement from a colour fade. Animations
// filled with `both` remain listed as `finished` after they end — verified in Chromium
// — so a probe taken after load still sees a page that animated on entry.
const PROBE_SOURCE = `(() => {
  const anims = document.getAnimations().map((a) => {
    const frames = (a.effect && a.effect.getKeyframes) ? a.effect.getKeyframes() : [];
    const props = new Set();
    for (const frame of frames) {
      for (const key of Object.keys(frame)) {
        if (key !== 'offset' && key !== 'computedOffset' && key !== 'easing' && key !== 'composite') {
          props.add(key);
        }
      }
    }
    const timing = (a.effect && a.effect.getTiming) ? a.effect.getTiming() : {};
    return {
      id: a.animationName || a.transitionProperty || a.id || '(unnamed)',
      playState: a.playState,
      properties: Array.from(props),
      duration: typeof timing.duration === 'number' ? timing.duration : 0,
      iterations: timing.iterations === Infinity ? null : (timing.iterations ?? 1),
    };
  });
  return { count: anims.length, anims };
})()`;

/** Resolve Playwright, reusing the search gen-screenshots.mjs already does. */
export async function loadPlaywright() {
  const { findPlaywrightPath } = await import(
    path.resolve(__dirname, 'gen-screenshots.mjs')
  );
  const found = findPlaywrightPath();
  if (!found) return null;
  return import(found);
}

/** Render one route under one motion preference and return its probe. */
export async function probeRoute(browser, url, reducedMotion) {
  const context = await browser.newContext(
    reducedMotion ? { reducedMotion: 'reduce' } : {},
  );
  const page = await context.newPage();
  try {
    await page.goto(url, { waitUntil: 'load' });
    return await page.evaluate(PROBE_SOURCE);
  } finally {
    await context.close();
  }
}

/** Run every route through both preferences and judge each. */
export async function collectEvidence(baseUrl, routes) {
  const playwright = await loadPlaywright();
  if (!playwright) {
    return {
      schema: 1,
      status: 'blocked_missing_playwright',
      detail: 'Playwright is not installed; motion evidence was not collected',
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
      const normal = await probeRoute(browser, url, false);
      const reduced = await probeRoute(browser, url, true);
      const routeFindings = judgeRoute(route, normal, reduced);
      results.push({
        route,
        moving: movingAnimations(normal).map((a) => a.id),
        movingUnderReduce: movingAnimations(reduced).map((a) => a.id),
        animationsDefault: normal.count,
        animationsReduced: reduced.count,
      });
      findings.push(...routeFindings);
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
    process.stderr.write('Usage: ui-motion-evidence.mjs --base-url URL --routes / [/more]\n');
    process.exit(3);
  }

  const report = await collectEvidence(opts.baseUrl, opts.routes);
  if (opts.json) {
    fs.mkdirSync(path.dirname(path.resolve(opts.json)), { recursive: true });
    fs.writeFileSync(opts.json, `${JSON.stringify(report, null, 2)}\n`);
  }

  if (report.status === 'blocked_missing_playwright') {
    process.stderr.write('ui-motion-evidence: Playwright unavailable; no evidence collected\n');
    process.exit(3);
  }

  for (const finding of report.findings) {
    process.stdout.write(`${finding.rule}:${finding.route}: ${finding.detail}\n`);
  }
  for (const row of report.routes) {
    if (!report.findings.some((f) => f.route === row.route)) {
      process.stdout.write(`ok ${row.route}: moves [${row.moving.join(', ')}], still under reduce\n`);
    }
  }
  process.exit(report.findings.length > 0 ? 1 : 0);
}

if (process.argv[1] && process.argv[1].endsWith('ui-motion-evidence.mjs')) {
  await main();
}
