#!/usr/bin/env node
// ui-a11y-baseline.mjs — accessibility-tree baselines (design spec L4c, third phase).
//
// The first two phases ask questions with fixed answers: does it move, does it fit, can a
// keyboard reach it, is the change announced. This one catches the class of regression that
// has no fixed answer — a control that quietly stopped being a control.
//
//   A11Y_NAME_LOST              a node that had an accessible name no longer has one
//   A11Y_ROLE_LOST              an interactive control or landmark is gone from the tree
//   A11Y_HEADING_LEVEL_CHANGED  a heading kept its text and changed depth
//   A11Y_CONTROL_DISABLED       a control is disabled where it was not
//   A11Y_TREE_CHANGED           advisory: something else moved; review and re-record
//
// Whether this is worth having at all turns on churn, so that was measured first. A baseline
// that moves under ordinary refactoring gets ignored within a week, and an ignored gate is
// worse than none — it trains reviewers to approve diffs unread. Against a real Chromium
// `ariaSnapshot()`: renaming a class, wrapping a table in a scroll container, reordering
// attributes and renaming an id all leave the snapshot **byte-identical**, while every one of
// six semantic regressions moves it. Those cosmetic cases are checked in as tests, because
// they are the guarantee the tier rests on rather than a nicety.
//
// The diffs are also classifiable, which is what keeps this from being a wall of text. When
// `button "Apply"` becomes `text: Apply`, that is not "something changed" — it is a control
// that no longer answers to a keyboard or a screen reader, and it can be said that way.
//
// Growth is deliberately advisory. Shipping a feature adds nodes; failing a build for that is
// exactly how a baseline gate earns a reputation for noise.
//
// Usage:
//   ui-a11y-baseline.mjs --base-url http://localhost:3000 --routes / /runs [--update]
//
// Exit: 0 clean or recorded, 1 regressions, 3 Playwright unavailable.
//
// Env:
//   PLAYWRIGHT_CHROMIUM_PATH  launch this chromium binary instead of the bundled one.

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { loadPlaywright, OBSERVER, QUIET_MS, MAX_SETTLE_MS } from './ui-liveregion-core.mjs';

const __dirname = path.dirname(fileURLToPath(import.meta.url));

export const DEFAULT_BASELINE_DIR = '.autospec/a11y-baselines';

// Roles whose disappearance is a loss of function rather than a change of layout. A `generic`
// or `text` node vanishing means nothing; a button vanishing means a user cannot act.
// Controls: losing one means a user can no longer act.
export const CONTROL_ROLES = new Set([
  'button', 'link', 'textbox', 'checkbox', 'radio', 'combobox', 'listbox', 'option',
  'slider', 'spinbutton', 'switch', 'tab', 'menuitem', 'searchbox',
]);

// Structure: losing one means a user can no longer navigate or orient. Not a control, so it
// gets its own wording — a lost heading is a broken outline, not an unreachable button.
export const STRUCTURE_ROLES = new Set([
  'banner', 'navigation', 'main', 'contentinfo', 'complementary', 'region', 'search',
  'form', 'table', 'tabpanel', 'heading', 'dialog', 'alert', 'status',
]);

export const LOAD_BEARING_ROLES = new Set([...CONTROL_ROLES, ...STRUCTURE_ROLES]);

// ── parsing ───────────────────────────────────────────────────────────────────

// A snapshot line looks like:
//   - button "Apply"
//   - heading "Runs" [level=1]
//   - textbox "Branch" [disabled]: main
//   - text: Branch
//   - /url: /specs          ← a property of the node above, not a node
const NODE_LINE = /^(\s*)-\s+([^\s"[:]+)\s*(?:"((?:[^"\\]|\\.)*)")?\s*((?:\[[^\]]*\]\s*)*)(?::(.*))?$/;

/**
 * Parse an aria snapshot into nodes. Property lines (`/url:`, `/checked:`) are skipped:
 * counting them would make every link two nodes and every diff twice as loud.
 */
export function parseSnapshot(text) {
  const nodes = [];
  for (const raw of String(text || '').split('\n')) {
    if (!raw.trim()) continue;
    const match = raw.match(NODE_LINE);
    if (!match) continue;
    const [, indent, role, quoted, flagBlob, trailing] = match;
    if (role.startsWith('/')) continue;

    const flags = {};
    for (const flag of (flagBlob || '').matchAll(/\[([^\]=]+)(?:=([^\]]*))?\]/g)) {
      flags[flag[1].trim()] = flag[2] === undefined ? 'true' : flag[2].trim();
    }
    // `- text: Branch` carries its content after the colon rather than in quotes.
    const name = quoted !== undefined ? quoted : (role === 'text' ? (trailing || '').trim() : '');
    nodes.push({ depth: indent.length, role, name, flags, raw: raw.trim() });
  }
  return nodes;
}

// ── classification ────────────────────────────────────────────────────────────

const countBy = (nodes, fn) => {
  const counts = new Map();
  for (const node of nodes) {
    const key = fn(node);
    if (key === null) continue;
    counts.set(key, (counts.get(key) || 0) + 1);
  }
  return counts;
};

const decreases = (before, after) => {
  const out = [];
  for (const [key, count] of before) {
    const now = after.get(key) || 0;
    if (now < count) out.push({ key, before: count, after: now });
  }
  return out;
};

/**
 * Compare two snapshots and name what a user lost.
 *
 * Each rule is a specific accessibility loss rather than a textual difference, because a
 * gate that reports "the tree changed" gets approved without being read. Anything the rules
 * cannot explain falls through to one advisory finding.
 */
export function classifyDiff(baselineText, currentText) {
  const before = parseSnapshot(baselineText);
  const after = parseSnapshot(currentText);
  const findings = [];

  // Names, per role: a node kept its role and lost the label that identified it.
  const namedBefore = countBy(before, (n) => (n.name ? `${n.role}|${n.name}` : null));
  const namedAfter = countBy(after, (n) => (n.name ? `${n.role}|${n.name}` : null));
  const roleBefore = countBy(before, (n) => n.role);
  const roleAfter = countBy(after, (n) => n.role);

  for (const { key } of decreases(namedBefore, namedAfter)) {
    const [role, name] = key.split('|');
    if (role === 'text') continue;  // prose moving around is not a name loss
    // Only when the node is still there. A control that was deleted outright is a lost role,
    // and reporting it as a lost name as well would bill one defect twice.
    const roleStillThere = (roleAfter.get(role) || 0) >= (roleBefore.get(role) || 0);
    if (roleStillThere) {
      findings.push({
        rule: 'A11Y_NAME_LOST',
        detail:
          `the ${role} named '${name}' is still in the tree with no accessible name — ` +
          'a screen reader now announces its role and nothing else',
      });
    }
  }

  // Roles: a control or landmark left the tree entirely.
  for (const { key, before: was, after: now } of decreases(roleBefore, roleAfter)) {
    if (!LOAD_BEARING_ROLES.has(key)) continue;
    const gone = was - now;
    findings.push({
      rule: 'A11Y_ROLE_LOST',
      detail:
        `${gone} ${key}${gone === 1 ? '' : 's'} disappeared from the tree (${was} → ${now}); ` +
        (CONTROL_ROLES.has(key)
          ? 'a control that is no longer a control cannot be reached by keyboard or screen reader'
          : 'the page structure a screen-reader user navigates by is now missing it'),
    });
  }

  // Headings: same text, different depth. Document outline changes silently otherwise.
  const levelOf = (nodes) => {
    const map = new Map();
    for (const node of nodes) {
      if (node.role === 'heading' && node.name) map.set(node.name, node.flags.level || '?');
    }
    return map;
  };
  const levelsBefore = levelOf(before);
  const levelsAfter = levelOf(after);
  for (const [name, was] of levelsBefore) {
    const now = levelsAfter.get(name);
    if (now !== undefined && now !== was) {
      findings.push({
        rule: 'A11Y_HEADING_LEVEL_CHANGED',
        detail: `heading '${name}' moved from level ${was} to level ${now}, changing the document outline`,
      });
    }
  }

  // Disabled: matched on role and name, so this is the same control rather than a new one.
  const disabled = (nodes) =>
    new Set(nodes.filter((n) => n.flags.disabled).map((n) => `${n.role}|${n.name}`));
  const disabledBefore = disabled(before);
  for (const key of disabled(after)) {
    if (disabledBefore.has(key)) continue;
    const [role, name] = key.split('|');
    findings.push({
      rule: 'A11Y_CONTROL_DISABLED',
      detail: `the ${role}${name ? ` named '${name}'` : ''} is disabled where it was not; if that is intended, re-record the baseline`,
    });
  }

  // Anything left. Advisory on purpose: a feature adds nodes, and failing a build for growth
  // is how a baseline gate becomes something people click past.
  if (findings.length === 0 && baselineText.trim() !== currentText.trim()) {
    const added = after.length - before.length;
    findings.push({
      rule: 'A11Y_TREE_CHANGED',
      advisory: true,
      detail:
        `the tree changed without losing a name, role or heading level (${before.length} → ` +
        `${after.length} nodes${added > 0 ? ', a net addition' : ''}); review the diff and re-record`,
    });
  }

  return findings;
}

// ── baselines on disk ─────────────────────────────────────────────────────────

/** Where a route's baseline lives. Content-derived so a route rename is visible as one. */
export function baselineFile(dir, route) {
  const slug = route === '/' ? 'root' : route.replace(/^\//, '').replace(/\//g, '-').replace(/[^a-zA-Z0-9_-]/g, '_');
  return path.join(dir, `${slug || 'root'}.aria.txt`);
}

// ── collection ────────────────────────────────────────────────────────────────

/**
 * Snapshot every route and compare against its committed baseline. A route with no baseline
 * is recorded, not judged: the first run establishes, it does not accuse.
 */
export async function collectBaselines(browser, baseUrl, routes, dir, opts = {}) {
  const findings = [];
  const compared = [];
  const recorded = [];

  for (const route of routes) {
    const context = await browser.newContext();
    const page = await context.newPage();
    let snapshot;
    let settled = true;
    try {
      await page.goto(new URL(route, baseUrl).toString(), { waitUntil: 'load' });

      // Snapshot the settled tree, not whatever exists at `load`. A data-driven route is
      // still showing its skeleton then — /runs.html in the pilot recorded 8 nodes instead
      // of its loaded 14 — and which one you get is a race against the page's own fetch.
      // Baselining the loser of a race is how a baseline starts flapping in CI.
      //
      // The clock is seeded *after* the observer installs, because installing it zeroes the
      // clock; seeded before, the wait is satisfied the instant it is asked.
      await page.evaluate(OBSERVER);
      await page.evaluate('window.__autospecLastMutation = performance.now()');
      settled = await page
        .waitForFunction(
          `performance.now() - window.__autospecLastMutation > ${QUIET_MS}`,
          null,
          { timeout: MAX_SETTLE_MS },
        )
        .then(() => true)
        .catch(() => false);

      snapshot = await page.locator('body').ariaSnapshot();
    } finally {
      await context.close();
    }

    const file = baselineFile(dir, route);
    if (!fs.existsSync(file) || opts.update) {
      fs.mkdirSync(path.dirname(file), { recursive: true });
      fs.writeFileSync(file, `${snapshot.trimEnd()}\n`);
      recorded.push({
        route,
        file,
        nodes: parseSnapshot(snapshot).length,
        settled: settled ? 'quiet' : `capped at ${MAX_SETTLE_MS}ms`,
      });
      continue;
    }

    const baseline = fs.readFileSync(file, 'utf8');
    const routeFindings = classifyDiff(baseline, snapshot).map((f) => ({ ...f, route }));
    findings.push(...routeFindings);
    compared.push({
      route,
      file,
      nodes: parseSnapshot(snapshot).length,
      // A capped snapshot is weaker evidence: the page was still changing when it was taken.
      settled: settled ? 'quiet' : `capped at ${MAX_SETTLE_MS}ms`,
      regressions: routeFindings.filter((f) => !f.advisory).length,
    });
  }

  return { compared, recorded, findings };
}

// ── CLI ───────────────────────────────────────────────────────────────────────

function parseArgs(argv) {
  const opts = { baseUrl: '', routes: [], dir: DEFAULT_BASELINE_DIR, json: '', update: false };
  for (let i = 0; i < argv.length; i += 1) {
    if (argv[i] === '--base-url') opts.baseUrl = argv[++i];
    else if (argv[i] === '--baseline-dir') opts.dir = argv[++i];
    else if (argv[i] === '--json') opts.json = argv[++i];
    else if (argv[i] === '--update') opts.update = true;
    else if (argv[i] === '--routes') {
      while (i + 1 < argv.length && !argv[i + 1].startsWith('--')) opts.routes.push(argv[++i]);
    }
  }
  return opts;
}

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  if (!opts.baseUrl || opts.routes.length === 0) {
    process.stderr.write(
      'Usage: ui-a11y-baseline.mjs --base-url URL --routes / [/more] [--update]\n',
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
      detail: 'Playwright is not installed; no accessibility trees were captured',
      compared: [],
      recorded: [],
      findings: [],
    });
    process.stderr.write('ui-a11y-baseline: Playwright unavailable; no evidence collected\n');
    process.exit(3);
  }

  const launch = {};
  if (process.env.PLAYWRIGHT_CHROMIUM_PATH) {
    launch.executablePath = process.env.PLAYWRIGHT_CHROMIUM_PATH;
  }
  const browser = await playwright.chromium.launch(launch);
  let result;
  try {
    result = await collectBaselines(browser, opts.baseUrl, opts.routes, path.resolve(opts.dir), {
      update: opts.update,
    });
  } finally {
    await browser.close();
  }

  writeReport({ schema: 1, status: 'ok', ...result });

  for (const finding of result.findings) {
    const tag = finding.advisory ? 'advisory ' : '';
    process.stdout.write(`${tag}${finding.rule}:${finding.route}: ${finding.detail}\n`);
  }
  for (const row of result.recorded) {
    process.stdout.write(`recorded ${row.route}: ${row.nodes} nodes → ${row.file}\n`);
  }
  for (const row of result.compared) {
    if (!result.findings.some((f) => f.route === row.route)) {
      process.stdout.write(`ok ${row.route}: ${row.nodes} nodes match the baseline\n`);
    }
  }
  if (result.recorded.length > 0 && result.findings.length === 0) {
    process.stdout.write('commit the baseline files so the next run has something to compare\n');
  }

  // Advisory findings do not fail: growth is normal, and a gate that blocks on it stops
  // being read.
  process.exit(result.findings.some((f) => !f.advisory) ? 1 : 0);
}

if (process.argv[1] && process.argv[1].endsWith('ui-a11y-baseline.mjs')) {
  await main();
}
