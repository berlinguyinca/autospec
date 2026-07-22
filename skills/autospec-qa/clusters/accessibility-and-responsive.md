# Cluster: accessibility-and-responsive

Scope: a11y, viewport, browser variance.

Inputs:
- Deployed app URL.
- Headless browser harness (Playwright / Puppeteer).
- A11y rules (WCAG AA defaults).

Responsibilities:
- Run axe-core (or equivalent) on key routes; collect violations.
- Walk viewport matrix (mobile 375, tablet 768, desktop 1280, wide 1920);
  detect content overflow + horizontal-scroll regressions.
- Validate browser variance across Chromium, WebKit, Firefox where applicable.
- Defer functional-coverage gaps to `functional-coverage`.

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
   z-index, banned fonts) and emits `file:line` findings:
   ```bash
   bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/lint-ui.sh" $(git diff --name-only main...HEAD -- '*.css' '*.scss' '*.tsx' '*.jsx' '*.vue' '*.html')
   ```
   Map each finding to a `category:"visual_fidelity"` qa-verdict entry (these are
   the deterministic half of the implementer's `DESIGN_DRIFT` directive). The
   vision judge below then handles the *subjective* fidelity the linter can't.

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
