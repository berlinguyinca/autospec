# UX/UI optimization workstream runbook

`scripts/ux-ui-workstream.sh` is the deterministic signal backbone for the
continuous UX/UI optimization tier introduced by issue #1538. It records one
snapshot per commit and theme, gates Core Web Vitals, Lighthouse, design-token,
visual-regression, and interaction-health signals, files prioritized regression
issues, and produces the measured before/after report required for improvement
PRs.

## Source canon

- web.dev Web Vitals defines the user-centric Core Web Vitals thresholds used by
  this workstream: LCP <= 2.5s, INP <= 200ms, and CLS <= 0.1.
- Lighthouse CI provides the PR-gated lab signal and assertion model for the
  `lighthouse_performance` floor.
- Nielsen Norman Group heuristics are a proposal lens only; heuristic findings
  need validation by hard signal before merge.
- HEART supplies product-signal framing for happiness, engagement, adoption,
  retention, and task success when the dashboard has instrumentation.

## Record per-theme snapshots

```bash
bash scripts/ux-ui-workstream.sh record-snapshot \
  --ledger .autospec/ux-ui/snapshots.jsonl \
  --commit "$(git rev-parse --short HEAD)" \
  --theme light \
  --lcp-ms 2200 --inp-ms 150 --cls 0.05 \
  --lighthouse-performance 94 \
  --token-violations 0 --visual-diff-pct 0.04 \
  --console-errors 0 --failed-requests 0 \
  --tap-target-violations 0 --horizontal-overflow 0
```

Run the same command for `--theme dark`. Both light and dark themes are required;
a missing theme is a gate failure.

## PR gate

```bash
bash scripts/ux-ui-workstream.sh gate \
  --ledger .autospec/ux-ui/snapshots.jsonl \
  --commit "$CANDIDATE_SHA" \
  --regressions-out .autospec/ux-ui/regressions.jsonl
```

The gate blocks any of these conditions:

- Core Web Vitals over budget: LCP <= 2.5s, INP <= 200ms, CLS <= 0.1.
- Lighthouse performance score below 90.
- Design-token lint violations above 0.
- Visual regression diff above 0.1%; this is the visual regression gate.
- Console errors, failed requests, tap-target violations, or horizontal overflow.
- Missing `light` or `dark` theme rows.

## File prioritized regression issues

```bash
bash scripts/ux-ui-workstream.sh propose-regression-issue \
  --regressions .autospec/ux-ui/regressions.jsonl \
  --out .autospec/ux-ui/issues
```

Generated issue bodies include `auto-implement`, `priority:high`, the failed
CWV/Lighthouse/design-token/visual/interaction signal, and a primary smoke-test
command that reproduces the gate.

## Attach measured before/after proof

```bash
bash scripts/ux-ui-workstream.sh improvement-report \
  --before .autospec/ux-ui/before.jsonl \
  --after .autospec/ux-ui/after.jsonl \
  --out reports/ux-ui-before-after.md
```

The report is refused unless at least one metric improves and no other tracked
metric regresses in either theme. This makes improvement PRs carry a measured
before/after delta rather than a subjective visual claim.
