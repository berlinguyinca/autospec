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
  "category": "a11y_violation|viewport_overflow|browser_variance",
  "rule": "color-contrast",
  "route": "/dashboard",
  "evidence": "…"
}
```

Verify-first: pass each finding through `scripts/qa-verify-finding.sh`
(`--category failing_test`).

TODO: backfill from `## Console and network error gate` +
viewport/a11y prose sections of SKILL.md.
