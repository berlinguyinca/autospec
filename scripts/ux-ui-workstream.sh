#!/usr/bin/env bash
# scripts/ux-ui-workstream.sh — autonomous UX/UI optimization workstream helper.
# Records deterministic CWV/Lighthouse/design-token/visual/interaction snapshots,
# gates regressions, emits auto-implement issues, and writes measured before/after
# reports for issue #1538.

set -eu

usage() {
    cat <<'USAGE'
Usage:
  ux-ui-workstream.sh record-snapshot --ledger FILE --commit SHA --theme light|dark --lcp-ms N --inp-ms N --cls N --lighthouse-performance N --token-violations N --visual-diff-pct N --console-errors N --failed-requests N --tap-target-violations N --horizontal-overflow N [--timestamp ISO]
  ux-ui-workstream.sh gate --ledger FILE --commit SHA [--regressions-out FILE]
  ux-ui-workstream.sh propose-regression-issue --regressions FILE --out DIR
  ux-ui-workstream.sh improvement-report --before FILE --after FILE --out FILE
  ux-ui-workstream.sh validate-design-doc --doc FILE
USAGE
}

if [ "${1:-}" = "--help" ] || [ $# -eq 0 ]; then
    usage
    exit 0
fi

python3 - "$@" <<'PY'
import argparse
import datetime as dt
import json
import os
import re
import sys
from pathlib import Path

THEMES = ("light", "dark")
BUDGETS = {
    "lcp_ms": 2500.0,
    "inp_ms": 200.0,
    "cls": 0.1,
    "lighthouse_performance": 90,
    "token_violations": 0,
    "visual_diff_pct": 0.1,
    "console_errors": 0,
    "failed_requests": 0,
    "tap_target_violations": 0,
    "horizontal_overflow": 0,
}
LOWER_IS_BETTER = {
    "lcp_ms", "inp_ms", "cls", "token_violations", "visual_diff_pct",
    "console_errors", "failed_requests", "tap_target_violations", "horizontal_overflow",
}
SOURCE_TOKENS = [
    "web.dev Web Vitals",
    "Lighthouse CI",
    "Nielsen Norman Group heuristics",
    "HEART",
]

def now_iso():
    return dt.datetime.now(dt.UTC).replace(microsecond=0).isoformat().replace("+00:00", "Z")

def ensure_parent(path):
    Path(path).parent.mkdir(parents=True, exist_ok=True)

def append_jsonl(path, row):
    ensure_parent(path)
    with open(path, "a", encoding="utf-8") as fh:
        fh.write(json.dumps(row, sort_keys=True, separators=(",", ":")) + "\n")

def write_jsonl(path, rows):
    ensure_parent(path)
    with open(path, "w", encoding="utf-8") as fh:
        for row in rows:
            fh.write(json.dumps(row, sort_keys=True, separators=(",", ":")) + "\n")

def read_jsonl(path):
    rows = []
    if not path or not os.path.exists(path):
        return rows
    with open(path, encoding="utf-8") as fh:
        for line in fh:
            line = line.strip()
            if line:
                rows.append(json.loads(line))
    return rows

def nonneg_float(value, flag):
    try:
        parsed = float(value)
    except (TypeError, ValueError):
        raise SystemExit(f"{flag} must be a non-negative number")
    if parsed < 0:
        raise SystemExit(f"{flag} must be a non-negative number")
    return parsed

def nonneg_int(value, flag):
    try:
        parsed = int(str(value))
    except (TypeError, ValueError):
        raise SystemExit(f"{flag} must be a non-negative integer")
    if parsed < 0:
        raise SystemExit(f"{flag} must be a non-negative integer")
    return parsed

def pct3(value):
    return f"{float(value):.3f}"

def fmt_num(value):
    if isinstance(value, int):
        return str(value)
    value = float(value)
    if value.is_integer():
        return str(int(value))
    return f"{value:.3f}".rstrip("0").rstrip(".")

def slug(value):
    return re.sub(r"[^A-Za-z0-9]+", "-", str(value)).strip("-").lower() or "item"

def latest_by_theme(rows, commit):
    selected = [r for r in rows if r.get("commit") == commit]
    by_theme = {}
    for row in selected:
        by_theme[row["theme"]] = row
    return by_theme

def tag_for(metric, theme, value, budget):
    if metric == "lcp_ms":
        return f"LCP_BUDGET_BREACH:{theme}:{int(value)}ms>{int(budget)}ms"
    if metric == "inp_ms":
        return f"INP_BUDGET_BREACH:{theme}:{int(value)}ms>{int(budget)}ms"
    if metric == "cls":
        return f"CLS_BUDGET_BREACH:{theme}:{pct3(value)}>{pct3(budget)}"
    if metric == "lighthouse_performance":
        return f"LIGHTHOUSE_PERFORMANCE_BELOW_FLOOR:{theme}:{int(value)}<{int(budget)}"
    if metric == "token_violations":
        return f"TOKEN_LINT_VIOLATIONS:{theme}:{int(value)}"
    if metric == "visual_diff_pct":
        return f"VISUAL_DIFF_ABOVE_BUDGET:{theme}:{pct3(value)}%>{pct3(budget)}%"
    if metric == "console_errors":
        return f"CONSOLE_ERRORS:{theme}:{int(value)}"
    if metric == "failed_requests":
        return f"FAILED_REQUESTS:{theme}:{int(value)}"
    if metric == "tap_target_violations":
        return f"TAP_TARGET_VIOLATIONS:{theme}:{int(value)}"
    if metric == "horizontal_overflow":
        return f"HORIZONTAL_OVERFLOW:{theme}:{int(value)}"
    return f"UX_UI_BUDGET_BREACH:{theme}:{metric}:{value}>{budget}"

def finding_rows(by_theme, commit):
    findings = []
    for theme in THEMES:
        row = by_theme.get(theme)
        if not row:
            findings.append({
                "tag": "THEME_MISSING", "theme": theme, "metric": "theme",
                "value": "missing", "budget": "present", "commit": commit,
                "test_cmd": f"bash scripts/ux-ui-workstream.sh gate --ledger .autospec/ux-ui/snapshots.jsonl --commit {commit}",
            })
            continue
        for metric, budget in BUDGETS.items():
            value = row.get(metric)
            if value is None:
                findings.append({
                    "tag": "METRIC_MISSING", "theme": theme, "metric": metric,
                    "value": "missing", "budget": budget, "commit": commit,
                    "test_cmd": f"bash scripts/ux-ui-workstream.sh gate --ledger .autospec/ux-ui/snapshots.jsonl --commit {commit}",
                })
                continue
            bad = float(value) > float(budget) if metric in LOWER_IS_BETTER else float(value) < float(budget)
            if bad:
                tag = tag_for(metric, theme, value, budget).split(":", 1)[0]
                findings.append({
                    "tag": tag, "theme": theme, "metric": metric,
                    "value": value, "budget": budget, "commit": commit,
                    "test_cmd": f"bash scripts/ux-ui-workstream.sh gate --ledger .autospec/ux-ui/snapshots.jsonl --commit {commit}",
                })
    return findings

def finding_line(finding):
    tag = finding["tag"]
    theme = finding.get("theme", "unknown")
    metric = finding.get("metric", "metric")
    value = finding.get("value")
    budget = finding.get("budget")
    if tag == "THEME_MISSING":
        return f"THEME_MISSING:{theme}"
    if tag == "METRIC_MISSING":
        return f"METRIC_MISSING:{theme}:{metric}"
    return tag_for(metric, theme, value, budget)

def cmd_record(args):
    row = {
        "timestamp": args.timestamp or now_iso(),
        "commit": args.commit,
        "theme": args.theme,
        "lcp_ms": nonneg_float(args.lcp_ms, "--lcp-ms"),
        "inp_ms": nonneg_float(args.inp_ms, "--inp-ms"),
        "cls": nonneg_float(args.cls, "--cls"),
        "lighthouse_performance": nonneg_int(args.lighthouse_performance, "--lighthouse-performance"),
        "token_violations": nonneg_int(args.token_violations, "--token-violations"),
        "visual_diff_pct": nonneg_float(args.visual_diff_pct, "--visual-diff-pct"),
        "console_errors": nonneg_int(args.console_errors, "--console-errors"),
        "failed_requests": nonneg_int(args.failed_requests, "--failed-requests"),
        "tap_target_violations": nonneg_int(args.tap_target_violations, "--tap-target-violations"),
        "horizontal_overflow": nonneg_int(args.horizontal_overflow, "--horizontal-overflow"),
    }
    append_jsonl(args.ledger, row)
    print(f"recorded UX/UI snapshot commit={row['commit']} theme={row['theme']} lcp={int(row['lcp_ms'])}ms inp={int(row['inp_ms'])}ms cls={pct3(row['cls'])} lighthouse={row['lighthouse_performance']}")
    return 0

def cmd_gate(args):
    rows = read_jsonl(args.ledger)
    if not rows:
        print("ux-ui gate: no snapshots recorded", file=sys.stderr)
        return 2
    findings = finding_rows(latest_by_theme(rows, args.commit), args.commit)
    if findings:
        for finding in findings:
            print(finding_line(finding))
        if args.regressions_out:
            write_jsonl(args.regressions_out, findings)
        print("ux-ui gate failed")
        return 1
    print("ux-ui gate passed (CWV, Lighthouse, token lint, visual diff, interactions, light+dark themes)")
    return 0

def issue_body(rows):
    commit = rows[0].get("commit", "unknown")
    bullets = "\n".join(
        f"- `{row.get('tag')}` on `{row.get('theme')}`: `{row.get('metric')}` value `{row.get('value')}` budget `{row.get('budget')}`."
        for row in rows
    )
    test_cmd = rows[0].get("test_cmd") or f"bash scripts/ux-ui-workstream.sh gate --ledger .autospec/ux-ui/snapshots.jsonl --commit {commit}"
    return f"""# Fix CWV/Lighthouse regression for `{commit}`

Labels: auto-implement, priority:high, ux-ui

## Goal

Restore CWV/Lighthouse UX budgets for `{commit}` in `scripts/ux-ui-workstream.sh` gate output.

## Files to read first

- `docs/runbooks/ux-ui-workstream.md`
- `.autospec/ux-ui/snapshots.jsonl`
- The dashboard source files changed by `{commit}`.

## Dependencies

none

## Implementation outline

- Reproduce the failing UX/UI gate and identify the dashboard change that caused the regression.
- Improve the measured metric without weakening budgets or deleting light/dark theme coverage.
- Attach a before/after report generated by `scripts/ux-ui-workstream.sh improvement-report`.

## Tests required

- [ ] `{test_cmd}` fails before the fix and passes after the fix.

## Acceptance criteria

- [ ] `scripts/ux-ui-workstream.sh gate` reports no CWV/Lighthouse findings for `{commit}`.
- [ ] `reports/ux-ui-before-after.md` records at least 1 measured before/after delta.
- [ ] Both `light` and `dark` theme rows remain present in `.autospec/ux-ui/snapshots.jsonl`.

## Regression details

{bullets}

## Smoke test

### Primary smoke test (inner loop)

```bash
{test_cmd}
```
"""

def cmd_propose(args):
    rows = read_jsonl(args.regressions)
    if not rows:
        print("propose-regression-issue: no UX/UI regressions found", file=sys.stderr)
        return 2
    commit = rows[0].get("commit", "unknown")
    Path(args.out).mkdir(parents=True, exist_ok=True)
    path = Path(args.out) / f"cwv-lighthouse-regression-{slug(commit)}.md"
    path.write_text(issue_body(rows), encoding="utf-8")
    print(f"wrote {path}")
    return 0

def compare_rows(before_rows, after_rows):
    before = {r["theme"]: r for r in before_rows}
    after = {r["theme"]: r for r in after_rows}
    improvements = []
    regressions = []
    for theme in THEMES:
        if theme not in before or theme not in after:
            regressions.append(f"COLLATERAL_REGRESSION:{theme}:theme:missing")
            continue
        for metric in BUDGETS:
            b = before[theme].get(metric)
            a = after[theme].get(metric)
            if b is None or a is None:
                regressions.append(f"COLLATERAL_REGRESSION:{theme}:{metric}:missing")
                continue
            if metric in LOWER_IS_BETTER:
                if float(a) < float(b):
                    improvements.append((theme, metric, b, a))
                elif float(a) > float(b):
                    regressions.append(f"COLLATERAL_REGRESSION:{theme}:{metric}:{fmt_num(b)}->{fmt_num(a)}")
            else:
                if float(a) > float(b):
                    improvements.append((theme, metric, b, a))
                elif float(a) < float(b):
                    regressions.append(f"COLLATERAL_REGRESSION:{theme}:{metric}:{fmt_num(b)}->{fmt_num(a)}")
    return improvements, regressions

def cmd_report(args):
    before_rows = read_jsonl(args.before)
    after_rows = read_jsonl(args.after)
    improvements, regressions = compare_rows(before_rows, after_rows)
    if regressions:
        for regression in regressions:
            print(regression)
        return 1
    if not improvements:
        print("NO_MEASURED_UX_UI_IMPROVEMENT", file=sys.stderr)
        return 1
    ensure_parent(args.out)
    lines = ["# Measured UX/UI before/after", "", "## Improvements"]
    for theme, metric, before, after in improvements:
        lines.append(f"- {theme} {metric}: {fmt_num(before)} -> {fmt_num(after)}")
    lines.extend(["", "No collateral UX/UI regressions"])
    Path(args.out).write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(f"wrote {args.out}")
    return 0

def cmd_validate_doc(args):
    path = Path(args.doc)
    if not path.exists():
        print(f"design doc missing: {args.doc}", file=sys.stderr)
        return 1
    text = path.read_text(encoding="utf-8")
    missing = [token for token in SOURCE_TOKENS if token not in text]
    for theme in THEMES:
        if re.search(rf"\b{theme}\b", text, re.IGNORECASE) is None:
            missing.append(f"theme:{theme}")
    for token in ["LCP <= 2.5s", "INP <= 200ms", "CLS <= 0.1", "visual regression", "HEART"]:
        if token not in text:
            missing.append(token)
    if missing:
        for token in missing:
            print(f"DOC_MISSING:{token}")
        return 1
    print("design doc validated")
    return 0

def parser():
    root = argparse.ArgumentParser(prog="ux-ui-workstream.sh")
    sub = root.add_subparsers(dest="cmd", required=True)

    p = sub.add_parser("record-snapshot")
    for flag in ["ledger", "commit", "lcp-ms", "inp-ms", "cls", "lighthouse-performance",
                 "token-violations", "visual-diff-pct", "console-errors", "failed-requests",
                 "tap-target-violations", "horizontal-overflow"]:
        p.add_argument(f"--{flag}", required=True)
    p.add_argument("--theme", required=True, choices=THEMES)
    p.add_argument("--timestamp")
    p.set_defaults(func=cmd_record)

    command_specs = [
        ("gate", ["ledger", "commit"], ["regressions-out"], cmd_gate),
        ("propose-regression-issue", ["regressions", "out"], [], cmd_propose),
        ("improvement-report", ["before", "after", "out"], [], cmd_report),
        ("validate-design-doc", ["doc"], [], cmd_validate_doc),
    ]
    for name, required_flags, optional_flags, func in command_specs:
        p = sub.add_parser(name)
        for flag in required_flags:
            p.add_argument(f"--{flag}", required=True)
        for flag in optional_flags:
            p.add_argument(f"--{flag}")
        p.set_defaults(func=func)

    return root

args = parser().parse_args()
raise SystemExit(args.func(args))
PY
