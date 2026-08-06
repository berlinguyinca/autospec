# Cluster: accessibility-and-responsive

Scope: a11y, viewport, browser variance.

Inputs:
- Deployed app URL.
- Headless browser harness (Playwright / Puppeteer).
- A11y rules (WCAG AA defaults).

Responsibilities:
- Run axe-core (or equivalent) on key routes; collect violations.
- Render each route across device profiles — user agent, DPR and touch, not widths —
  via step 0c, which also runs the 320px reflow and 200% zoom checks. A width sweep
  resolves neither `pointer: coarse` nor `any-hover: none`, so it never exercises the
  media queries a responsive UI is built on.
- Traverse each route by keyboard via step 0d.
- Validate browser variance across Chromium, WebKit, Firefox where applicable.
- Defer functional-coverage gaps to `functional-coverage`.

## What is machine-verifiable, and what is not

The evidence steps below decide these outright. Do not spend a judgement call on them,
and do not record a pass without their output:

| Question | Settled by |
|---|---|
| Does the UI move at all, and does it stop when asked? | step 0b |
| Does it fit at 320px and at 200% zoom? | step 0c |
| Are touch targets big enough on a coarse pointer? | step 0c |
| Can a keyboard reach everything, see where it is, and get out? | step 0d |
| Is focus painted over by a sticky header? | step 0d |

What remains a judgement call, honestly:

- whether an accessible name is *meaningful* — a robot confirms one exists, not that it
  describes the control
- whether link text makes sense read out of context
- plain language, reading level, and tone
- subjective visual fidelity, which the vision judge below handles

When one of the steps below exits 3, its question is **unknown**, not answered. Record it
that way; an absent browser is not a passing grade.

## Screenshot audit

Capture every spec route at desktop (1280), tablet (768), and mobile (375)
using `skills/autospec-shared/scripts/gen-screenshots.mjs`. For each route and
viewport, record `clientWidth`, `scrollWidth`, an `horizontal_overflow` boolean,
and bounding boxes for major sections, cards, tables, tabs, and charts. Keep
console errors and page errors beside these metrics so a visual finding is
reproducible from one artifact. A screenshot verdict may use these generic
responsive cohesion categories:

- `document-overflow` — remove page-level horizontal overflow or constrain the offending region.
- `clipped-tab` — make tab labels reachable with wrapping, scrolling, or a responsive alternative.
- `inconsistent-gutter` — align route gutters to the shared responsive spacing tokens.
- `mixed-control-style` — apply one control treatment across equivalent actions and breakpoints.
- `unresponsive-table` — provide a responsive table layout, scrolling container, or compact columns.
- `chart-squeeze` — preserve chart readability by resizing, reflowing, or offering a summary view.
- `unanchored-control` — keep controls anchored to their associated content across viewport changes.
- `density-overload` — reduce or reflow crowded content so touch targets and reading order remain usable.

Output JSON shape:
```json
{
  "cluster": "accessibility-and-responsive",
  "category": "a11y_violation|viewport_overflow|browser_variance|visual_fidelity|document-overflow|clipped-tab|inconsistent-gutter|mixed-control-style|unresponsive-table|chart-squeeze|unanchored-control|density-overload",
  "rule": "color-contrast",
  "route": "/dashboard",
  "evidence": "…"
}
```

Verify-first: pass each finding through `scripts/qa-verify-finding.sh`
(`--category failing_test`).

Tablet/mobile findings in any of the eight responsive cohesion categories are
release-blocking when their verdict status is `FAIL`, even if the desktop
viewport passes. The `--blocking-on FAIL` default in
`scripts/qa-visual-findings.sh` enforces this rule; use `--blocking-on PARTIAL`
when a stricter release gate is explicitly required.

## Visual fidelity (does the UI *look* right, not just *work*)

Behavioral + a11y checks confirm a screen functions; this loop confirms it
matches the adopted design language. It runs only when the repo has a root
`DESIGN.md` (otherwise skip — nothing to judge against).

0. **Deterministic token-drift lint (cheap pre-pass)** — before spending a vision
   call, run the objective design-token linter on the changed UI files; it catches
   what regex can prove (raw hex outside the palette, off-grid spacing, ad-hoc
   z-index, banned fonts) plus the motion and input rules (motion with no
   `prefers-reduced-motion` fallback, infinite animation with no pause control,
   a viewport blocking zoom, `:hover` with no `:focus` equivalent) and emits
   `file:line` findings:
   ```bash
   bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/lint-ui.sh" $(git diff --name-only main...HEAD -- '*.css' '*.scss' '*.tsx' '*.jsx' '*.vue' '*.html')
   ```
   Map each finding to a `category:"visual_fidelity"` qa-verdict entry (these are
   the deterministic half of the implementer's `DESIGN_DRIFT` directive). The
   vision judge below then handles the *subjective* fidelity the linter can't.

0b. **Runtime motion evidence** — render each route twice, once normally and once
   with `prefers-reduced-motion: reduce`, and assert that motion exists by default,
   stops under the preference, and does not run unbounded:
   ```bash
   node "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/ui-motion-evidence.mjs" \
     --base-url "$APP_URL" --routes / /runs --json .autospec/reports/motion-evidence.json
   ```
   `MOTION_ABSENT` is the one worth understanding. Every other gate in this
   pipeline — the lint above, the vision judge below, axe, the aesthetic rubric —
   passes a static, animation-free page cleanly. This is the only check that fails
   a UI for being *inert* rather than wrong, so it is the sole defence against a
   correct, accessible, entirely lifeless implementation. Treat it as blocking, not
   advisory.

   Colour and opacity fades do not satisfy it: the assertion requires an animation
   that moves or resizes something, matching what `lint-ui.sh` counts as motion.
   Exit 3 means Playwright is unavailable and no evidence was collected — record
   that as unknown, never as a pass.

0c. **Runtime device evidence** — render each route across real device descriptors
   and run the two dedicated WCAG checks:
   ```bash
   node "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/ui-device-evidence.mjs" \
     --base-url "$APP_URL" --routes / /runs --json .autospec/reports/device-evidence.json
   ```
   Findings: `DEVICE_OVERFLOW`, `DEVICE_REFLOW` (320px, WCAG 1.4.10),
   `DEVICE_ZOOM_CLIP` (200% zoom, 1.4.4), `DEVICE_TARGET_TOO_SMALL` (24px on a
   coarse pointer, 2.5.8), `DEVICE_HOVER_ONLY_INPUT`.

   This replaces judging responsiveness by width alone. The descriptors carry user
   agent, DPR and touch, so `pointer: coarse` and `any-hover: none` resolve as they
   do on the device — a 390px desktop viewport resolves none of them, so a width
   sweep never exercises the media queries a responsive UI is built on.

   It also catches what no stylesheet can state: a control's rendered size depends
   on the user agent, so a text input can pass every static rule and still render
   under the 24px minimum. Treat these as blocking, and exit 3 as unknown.

0d. **Keyboard traversal** — tab through each route and assert a keyboard user can
   work it:
   ```bash
   node "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/ui-keyboard-evidence.mjs" \
     --base-url "$APP_URL" --routes / /runs --json .autospec/reports/keyboard-evidence.json
   ```
   Findings: `KEYBOARD_TRAP`, `NO_KEYBOARD_PATH`, `FOCUS_NOT_VISIBLE` (WCAG 2.4.7),
   `FOCUS_OBSCURED` (2.4.11), `FOCUS_ORDER_JUMPS` (2.4.3).

   `KEYBOARD_TRAP` is the severe one: a user who tabs into a region and cannot tab
   out has no way forward without a mouse. It is distinguished from merely
   unreachable content by whether the traversal cycles — focus handed back
   repeatedly repeats a short cycle while other controls sit outside it.

   The browser's default focus ring counts as visible; only a suppressed indicator
   is reported. Treat these as blocking, and exit 3 as unknown.

1. **Capture** — screenshot each spec route at the viewport matrix using the
   existing `gen-screenshots.mjs` (Mode-II forbidden-URL safety already built in):
   ```bash
   node "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/gen-screenshots.mjs" \
     --base-url "$APP_URL" --routes routes.json --output-dir .autospec/visual
   ```
2. **Judge (TIER_A vision)** — for each screenshot, the vision model compares it
   against `DESIGN.md` tokens (color, spacing, typography, component patterns) and
   the issue's enumerated interaction states (default/hover/focus/loading/empty/
   error/disabled). Emit one verdict per route×viewport as JSON:
   `{"route","viewport","status":"PASS|PARTIAL|FAIL","issues":["spacing != 8px token", …]}`.
   Default to PARTIAL (not FAIL) when unsure — regex/vision can mis-judge; the heal
   loop should not wedge on a borderline call.
3. **Shape findings** — pipe the verdict array through
   `bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/qa-visual-findings.sh"`
   to produce `category:"visual_fidelity"` findings (FAIL → release-blocking,
   PARTIAL → advisory) and merge them into `.autospec/qa-verdict.json`.
4. **Heal** — the existing autospec-qa self-heal loop files/fixes the blocking
   `visual_fidelity` findings like any other category and re-judges until they
   clear. This makes the adopted DESIGN.md *enforced*, not decorative.

## Baseline design gates (rules.yaml — enforced when the repo opts in)

Runs only when the repo has `.autospec/design-gates.yml` (a baseline-pack
consumer mapping machine-checkable rule ids to local commands). Otherwise skip —
nothing is configured to enforce.

1. **Execute** the deterministic gates and produce the evidence report:
   ```bash
   bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-design-gates.sh" \
     --repo-root .
   ```
   The runner writes `.autospec/reports/design-gates.{json,md}` and prints a
   single authoritative status line
   (`autospec-design-gates: PASS|FAIL|SKIPPED (…)`). SKIPPED is a clean pass.
2. **Shape findings** from `.autospec/reports/design-gates.json`:
   - each `gates[]` entry with `status:"fail"` → one qa-verdict finding with
     `category:"design_gate"`, `release_blocking` = its `blocking` flag, and the
     gate's `output_tail` as evidence;
   - `status:"unmapped"` entries with `severity:"blocker"` → one advisory
     (non-blocking) finding listing the unmapped ids, so coverage gaps stay
     visible without wedging the loop.
3. **Judge the critic checklist** — for UI-touching PRs, use the report's
   `critic_checklist` (the pack's `check: vlm|review` rules) and
   `pack_quality_gates` as the rubric for the vision/critique pass above; cite
   the rule id in any finding it produces. Default to PARTIAL when unsure,
   mirroring the visual-fidelity loop.
