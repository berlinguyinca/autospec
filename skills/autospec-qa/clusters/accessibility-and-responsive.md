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

Output JSON shape:
```json
{
  "cluster": "accessibility-and-responsive",
  "category": "a11y_violation|viewport_overflow|browser_variance|visual_fidelity",
  "rule": "color-contrast",
  "route": "/dashboard",
  "evidence": "…"
}
```

Verify-first: pass each finding through `scripts/qa-verify-finding.sh`
(`--category failing_test`).

## Visual fidelity (does the UI *look* right, not just *work*)

Behavioral + a11y checks confirm a screen functions; this loop confirms it
matches the adopted design language. It runs only when the repo has a root
`DESIGN.md` (otherwise skip — nothing to judge against).

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
