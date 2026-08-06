// ui-a11y-baseline.test.mjs — accessibility-tree baselines (design spec L4c, phase 3).
//
// The fixtures are recorded from a real Chromium `ariaSnapshot()`, and the churn tests are
// the reason this tier exists in this shape. A baseline that moves under ordinary
// refactoring gets ignored within a week, and an ignored gate is worse than none: it trains
// reviewers to approve diffs unread.
//
// So the cosmetic cases below are not politeness — they are the load-bearing guarantee.
// Measured before any of this was written: a class rename, an added wrapper div, reordered
// attributes and a renamed id all leave the snapshot byte-identical, while every semantic
// regression moves it.
//
// Tests:
//   1-3.   parseSnapshot: role, name, flags, nesting depth
//   4.     classifyDiff: an identical tree draws nothing
//   5.     A11Y_NAME_LOST when a named control loses its name
//   6.     A11Y_ROLE_LOST when a control degrades to plain text
//   7.     A11Y_ROLE_LOST when a landmark disappears
//   7b.    a deleted control is billed once, not as both a lost role and a lost name
//   7c.    a heading removed outright is reported rather than exempted
//   8.     A11Y_HEADING_LEVEL_CHANGED, matched by heading name
//   9.     A11Y_CONTROL_DISABLED when a control gains [disabled]
//   10.    a data change is advisory, never a lost name — the guarantee that makes this
//          usable on an app with live content
//   10b.   a list with fewer rows is not a lost role
//   10c.   purely additive growth is advisory, not a regression
//
// The real-browser cases arrive with the capture half.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  parseSnapshot,
  classifyDiff,
} from '../../scripts/ui-a11y-baseline.mjs';

// Recorded verbatim from scratchpad/probe_tree.mjs.
const BASELINE = `- link "Skip to content":
  - /url: "#main"
- banner:
  - navigation "Primary":
    - list:
      - listitem:
        - link "Runs":
          - /url: /
- main:
  - heading "Runs" [level=1]
  - status
  - region "Filters":
    - heading "Filters" [level=2]
    - text: Branch
    - textbox "Branch": main
    - button "Apply"
  - heading "Recent" [level=2]
  - table "Recent runs":
    - caption: Recent runs`;

const rules = (findings) => findings.map((f) => f.rule);

// ── parsing ───────────────────────────────────────────────────────────────────

test('a node yields its role and accessible name', () => {
  const nodes = parseSnapshot(BASELINE);
  const button = nodes.find((n) => n.role === 'button');
  assert.equal(button.name, 'Apply');
  const status = nodes.find((n) => n.role === 'status');
  assert.equal(status.name, '');
});

test('bracketed flags are parsed apart from the name', () => {
  const nodes = parseSnapshot('- heading "Runs" [level=1]\n- textbox "Branch" [disabled]: main');
  assert.equal(nodes[0].role, 'heading');
  assert.equal(nodes[0].name, 'Runs');
  assert.equal(nodes[0].flags.level, '1');
  assert.equal(nodes[1].flags.disabled, 'true');
});

test('property lines are not mistaken for nodes', () => {
  // `/url:` describes the link above it; counting it as a node would make every link two.
  const nodes = parseSnapshot('- link "Runs":\n  - /url: /');
  assert.deepEqual(nodes.map((n) => n.role), ['link']);
});

// ── classification ────────────────────────────────────────────────────────────

test('an identical tree draws nothing', () => {
  assert.deepEqual(classifyDiff(BASELINE, BASELINE), []);
});

test('a control that loses its accessible name is named exactly', () => {
  const after = BASELINE
    .replace('    - text: Branch\n', '')
    .replace('- textbox "Branch": main', '- textbox: main');
  const findings = classifyDiff(BASELINE, after);
  assert.ok(rules(findings).includes('A11Y_NAME_LOST'));
  const lost = findings.find((f) => f.rule === 'A11Y_NAME_LOST');
  assert.match(lost.detail, /textbox/);
  assert.match(lost.detail, /Branch/);
});

test('a control that degrades to plain text is a lost role', () => {
  const after = BASELINE.replace('- button "Apply"', '- text: Apply');
  const findings = classifyDiff(BASELINE, after);
  assert.ok(rules(findings).includes('A11Y_ROLE_LOST'));
  assert.match(findings.find((f) => f.rule === 'A11Y_ROLE_LOST').detail, /button/);
});

test('a landmark that disappears is a lost role, and only that', () => {
  const after = BASELINE.replace('- banner:\n', '');
  assert.ok(rules(classifyDiff(BASELINE, after)).includes('A11Y_ROLE_LOST'));
});

test('a deleted control is billed once, as a lost role rather than also a lost name', () => {
  // Without this, a removed `button "Apply"` reports both A11Y_ROLE_LOST and A11Y_NAME_LOST
  // for the same defect — and a reviewer chasing two findings finds one bug.
  const after = BASELINE.replace('    - button "Apply"\n', '');
  assert.deepEqual(rules(classifyDiff(BASELINE, after)), ['A11Y_ROLE_LOST']);
});

test('a heading removed outright is reported, not exempted', () => {
  // Heading was once skipped by the lost-role rule on the theory that its own rule covered
  // it. That rule only sees level changes, which leave the count untouched — so a deleted
  // heading went unreported entirely.
  const after = BASELINE.replace('  - heading "Recent" [level=2]\n', '');
  assert.deepEqual(rules(classifyDiff(BASELINE, after)), ['A11Y_ROLE_LOST']);
});

test('a heading that changes level is reported against its name', () => {
  const after = BASELINE.replace('- heading "Recent" [level=2]', '- heading "Recent" [level=3]');
  const findings = classifyDiff(BASELINE, after);
  assert.deepEqual(rules(findings), ['A11Y_HEADING_LEVEL_CHANGED']);
  assert.match(findings[0].detail, /Recent/);
  assert.match(findings[0].detail, /2.*3/);
});

test('a control that becomes disabled is reported', () => {
  const after = BASELINE.replace('- textbox "Branch": main', '- textbox "Branch" [disabled]: main');
  const findings = classifyDiff(BASELINE, after);
  assert.deepEqual(rules(findings), ['A11Y_CONTROL_DISABLED']);
  assert.match(findings[0].detail, /Branch/);
});

test('changing the data a page displays is not a lost name', () => {
  // A baseline contains content as well as labels — the pilot's /runs.html records
  // `listitem: run-104 passed` — so on an app with live data the obvious implementation
  // would report A11Y_NAME_LOST on every data change and be pure noise.
  //
  // It does not, and the reason is worth pinning down: the snapshot format puts an
  // accessible *name* in quotes and mere *content* after a colon, and only quoted strings
  // are read as names (`text` nodes carry content in that position and are excluded from
  // the name rule outright). This test exists so that distinction cannot be refactored away
  // without something failing.
  const after = BASELINE
    .replace('- caption: Recent runs', '- caption: Latest runs')
    .replace('    - text: Branch', '    - text: Branch name')
    .replace('- textbox "Branch": main', '- textbox "Branch": develop');
  const findings = classifyDiff(BASELINE, after);
  assert.deepEqual(rules(findings), ['A11Y_TREE_CHANGED']);
  assert.equal(findings[0].advisory, true);
});

test('a list whose rows change count is not a lost role', () => {
  // listitem is deliberately absent from LOAD_BEARING_ROLES: rows come and go with the data,
  // and a shorter list is not a lost control.
  const withRows = `${BASELINE}\n  - list:\n    - listitem: run-104\n    - listitem: run-103`;
  const fewer = `${BASELINE}\n  - list:\n    - listitem: run-104`;
  const findings = classifyDiff(withRows, fewer);
  assert.deepEqual(rules(findings), ['A11Y_TREE_CHANGED']);
});

test('a tree that only gained nodes is advisory, not a regression', () => {
  // Shipping a feature adds nodes. Treating that as a failure is how a baseline gate earns
  // its reputation for noise and stops being read.
  const after = `${BASELINE}\n- button "Export"`;
  const findings = classifyDiff(BASELINE, after);
  assert.deepEqual(rules(findings), ['A11Y_TREE_CHANGED']);
  assert.equal(findings[0].advisory, true);
});
