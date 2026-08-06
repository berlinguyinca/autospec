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
- Drive each declared app state and check what is announced, via step 0e.
- Compare each route's accessibility tree against its committed baseline, via step 0f.
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
| When the app changes state, is a screen-reader user told? | step 0e |
| Did a control quietly stop being a control? | step 0f |

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

0a. **Run all five runtime gates** — one command, one report, one status line:
   ```bash
   bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/ui-evidence-gates.sh" \
     --base-url "$APP_URL" --routes "/ /runs /issues"
   ```
   ```
   ok device
   ok keyboard
   motion: findings
     MOTION_ABSENT:/: nothing moves on this route by default
   liveregion: UNKNOWN — the gate ran and verified nothing
   ui-evidence-gates: FAIL (4 ran, 1 with findings, 1 unknown)
   ```
   **Prefer this over invoking 0b–0f separately.** Five commands with five report shapes and
   five exit conventions get run once and then not again; the sections below stay as the
   description of what each gate decides, not as five things to type.

   Parse the final line, not the exit code alone. `PASS`/`FAIL`/`UNKNOWN` and the three
   counts are the authoritative summary, and `.autospec/reports/ui-evidence-gates.json`
   carries every gate's full output.

   Three distinctions the runner preserves, each of which is a way a gate run can lie:
   - **`UNKNOWN` is not `PASS`.** A gate that could not launch a browser (exit 3) or is not
     installed answered none of its questions. Exit code 2, never 0.
   - **A gate that ran and verified nothing is also `UNKNOWN`.** Live-region induction against
     a server-rendered app skips every route and exits 0; reporting that as `ok` is zero
     coverage wearing a pass.
   - **Findings outrank unknown.** A missing browser on one gate never masks a defect another
     gate actually found.

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

0e. **Live-region announcements** — drive each route into its loading, error and success
   states and assert a screen-reader user is told what happened:
   ```bash
   node "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/ui-liveregion-evidence.mjs" \
     --base-url "$APP_URL" --routes / /runs \
     --json .autospec/reports/liveregion-evidence.json
   ```
   Findings: `LIVE_REGION_ABSENT`, `LIVE_REGION_INSERTED_WITH_CONTENT`,
   `LIVE_REGION_HIDDEN`, `LIVE_REGION_STUCK_BUSY`, `LIVE_REGION_WRONG_POLITENESS`,
   `INDUCED_STATE_IGNORED`, plus `TEST_HOOK_MISSING` / `TEST_HOOK_FAILED` /
   `TEST_HOOK_NO_EFFECT`, which name a broken manifest rather than an accessibility defect
   and should be routed as such.

   **No setup is required and none should be requested.** The states worth checking are
   network-driven, so the gate induces them: it holds each route's own data requests to
   watch the pending state, then releases one normally and one as a 500. The app runs its
   own state machine. Nothing is clicked and nothing is mutated, so this is safe against a
   deployed app.

   `INDUCED_STATE_IGNORED` is the one to read carefully. It means the app did *nothing* when
   a request failed — no render, no message — so the user is left on stale content with no
   indication anything went wrong. It is usually a missing `catch`, and it is a worse defect
   than the missing announcement it sits beside.

   A route that makes no `fetch` or `xhr` request is reported under `skipped` with a reason.
   That is honest: a static page has no state to drive. It is not a finding.

   **A server-rendered app skips every route, and that is zero coverage rather than a pass.**
   Measured against `berlinguyinca/autospec-gui`: all six of its routes render their data on
   the server, so nothing is fetched client-side and nothing can be held. This is the norm
   for Next.js App Router and every comparable framework, so when the run reports
   `0 of N route(s) measured`, treat it as the signal to declare states in the manifest —
   not as a clean bill of health. It is the one case where induction alone leaves a real gap.

   States no request can produce — form validation, optimistic updates, client-side route
   changes, empty states — are unreachable this way. Those are declared in
   `.autospec/ui-test-hooks.json`, which the implementer writes as it builds them (see
   `ANNOUNCE_STATE_CHANGE` in the implementer contract); the manifest is read automatically
   when present and is purely additive. `berlinguyinca/autospec-ui-pilot` carries both
   references: `runs.html` for induction, `states.html` for the declared hook.

   Treat the findings as blocking, and exit 3 as unknown.

0f. **Accessibility-tree baselines** — snapshot each route's accessibility tree and compare
   it against the committed baseline:
   ```bash
   node "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/ui-a11y-baseline.mjs" \
     --base-url "$APP_URL" --routes / /runs \
     --json .autospec/reports/a11y-baseline.json
   ```
   Findings: `A11Y_NAME_LOST`, `A11Y_ROLE_LOST`, `A11Y_HEADING_LEVEL_CHANGED`,
   `A11Y_CONTROL_DISABLED`, and the advisory `A11Y_TREE_CHANGED`.

   This catches what the steps above cannot: a regression rather than a violation. Steps
   0b–0e each ask a question with a fixed answer. This one asks whether the page still
   exposes what it used to — a `<button>` refactored into a clickable `<div>` passes every
   other gate in this cluster and is reported here as `A11Y_ROLE_LOST`.

   A route with no baseline is **recorded, not judged**: the first run establishes, it does
   not accuse. Commit the files under `.autospec/a11y-baselines/`. Accepting an intentional
   change means re-running with `--update` and committing the diff, which is a reviewable
   act rather than a silent one.

   `A11Y_TREE_CHANGED` is advisory and does not block. Shipping a feature adds nodes, and a
   gate that fails on growth gets approved unread — which is worse than no gate, because it
   trains reviewers to click past accessibility findings. Only the four named losses block.

   Churn was measured before this was built, and it is why the tier is worth having: a class
   rename, an added wrapper div, reordered attributes and a renamed id all leave the tree
   byte-identical. Data changes are advisory too — the snapshot format puts an accessible
   name in quotes and mere content after a colon, so a list whose rows change is not a lost
   name.

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
