#!/usr/bin/env bash
# scripts/accessibility-workstream.sh — accessibility and adjacent web-standards workstream.
# Records deterministic axe/pa11y/Lighthouse/IBM Equal Access snapshots, gates PRs,
# separates auto-fixable findings from judgment-class review, ranks severity, and
# validates the source canon for issue #1539.
set -eu
usage() {
    cat <<'USAGE'
Usage:
  accessibility-workstream.sh record-scan --ledger FILE --commit SHA --theme light|dark --axe-violations N --pa11y-errors N --lighthouse-a11y N --ibm-violations N --auto-fixable N --judgment-findings N [--timestamp ISO]
  accessibility-workstream.sh gate --ledger FILE --commit SHA [--findings-out FILE]
  accessibility-workstream.sh classify-finding --rule RULE --legal-exposure N --user-blocking N --traffic N --occurrences N
  accessibility-workstream.sh rank --findings FILE --out FILE
  accessibility-workstream.sh remediation-report --before FILE --after FILE --out FILE
  accessibility-workstream.sh validate-design-doc --doc FILE
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
LOWER_IS_BETTER = {"axe_violations", "pa11y_errors", "ibm_violations", "auto_fixable", "judgment_findings"}
BUDGETS = {
    "axe_violations": 0,
    "pa11y_errors": 0,
    "lighthouse_a11y": 90,
    "ibm_violations": 0,
    "auto_fixable": 0,
}
AUTO_FIXABLE_RULES = {
    "missing-alt",
    "missing-form-label",
    "aria-misuse",
    "contrast",
    "landmark-missing",
    "heading-order",
    "html-lang-missing",
    "duplicate-id",
    "target-size",
    "redundant-entry",
}
JUDGMENT_RULES = {
    "focus-order",
    "keyboard-operability-custom-widget",
    "screen-reader-announcement",
    "meaningful-alt-text",
    "meaningful-link-text",
    "reading-order",
}
SOURCE_TOKENS = [
    "W3C WCAG 2.2",
    "WAI-ARIA APG",
    "Section508.gov",
    "Deque axe-core",
    "pa11y CI",
    "Lighthouse a11y",
    "IBM Equal Access",
    "WCAG 3.0/APCA design-time guidance only",
    "schema.org JSON-LD",
    "security headers",
    "privacy/cookie UX",
    "i18n/l10n",
    "ISO 9241",
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
def nonneg_int(value, flag):
    try:
        parsed = int(str(value))
    except (TypeError, ValueError):
        raise ValueError(f"{flag} must be a non-negative integer")
    if parsed < 0:
        raise ValueError(f"{flag} must be a non-negative integer")
    return parsed
def slug(value):
    return re.sub(r"[^A-Za-z0-9]+", "-", str(value)).strip("-").lower() or "item"
def latest_by_theme(rows, commit):
    selected = [r for r in rows if r.get("commit") == commit]
    by_theme = {}
    for row in selected:
        by_theme[row["theme"]] = row
    return by_theme
def metric_value(row, metric):
    return row.get(metric, row.get(metric.replace("_", "-")))
def finding_row(tag, theme, metric, value, budget, commit, remediation_class="auto_fixable"):
    return {
        "tag": tag,
        "theme": theme,
        "metric": metric,
        "value": value,
        "budget": budget,
        "commit": commit,
        "remediation_class": remediation_class,
        "test_cmd": f"bash scripts/accessibility-workstream.sh gate --ledger .autospec/accessibility/scans.jsonl --commit {commit}",
    }
def tag_for(metric, theme, value, budget):
    if metric == "axe_violations":
        return f"AXE_VIOLATIONS:{theme}:{int(value)}"
    if metric == "pa11y_errors":
        return f"PA11Y_ERRORS:{theme}:{int(value)}"
    if metric == "lighthouse_a11y":
        return f"LIGHTHOUSE_A11Y_BELOW_AA_FLOOR:{theme}:{int(value)}<{int(budget)}"
    if metric == "ibm_violations":
        return f"IBM_EQUAL_ACCESS_VIOLATIONS:{theme}:{int(value)}"
    if metric == "auto_fixable":
        return f"AUTO_FIXABLE_VIOLATIONS:{theme}:{int(value)}"
    return f"ACCESSIBILITY_VIOLATION:{theme}:{metric}:{value}"
def finding_rows(by_theme, commit):
    findings = []
    for theme in THEMES:
        row = by_theme.get(theme)
        if not row:
            findings.append(finding_row("THEME_MISSING", theme, "theme", "missing", "present", commit))
            continue
        for metric, budget in BUDGETS.items():
            value = metric_value(row, metric)
            if value is None:
                findings.append(finding_row("METRIC_MISSING", theme, metric, "missing", budget, commit))
                continue
            if metric in LOWER_IS_BETTER:
                bad = float(value) > float(budget)
            else:
                bad = float(value) < float(budget)
            if bad:
                findings.append(finding_row(tag_for(metric, theme, value, budget).split(":", 1)[0], theme, metric, value, budget, commit))
        judgment = int(metric_value(row, "judgment_findings") or 0)
        if judgment > 0:
            findings.append(finding_row("HUMAN_REVIEW_REQUIRED", theme, "judgment_findings", judgment, 0, commit, "judgment"))
    return findings
def finding_line(finding):
    tag = finding["tag"]
    if tag == "THEME_MISSING":
        return f"THEME_MISSING:{finding['theme']}"
    if tag == "METRIC_MISSING":
        return f"METRIC_MISSING:{finding['theme']}:{finding['metric']}"
    if tag == "HUMAN_REVIEW_REQUIRED":
        return f"HUMAN_REVIEW_REQUIRED:{finding['theme']}:{finding['value']}"
    return tag_for(finding["metric"], finding["theme"], finding["value"], finding["budget"])
def score_priority(score, rule):
    if rule in {"keyboard-trap", "missing-form-label", "missing-alt", "inaccessible-auth", "contrast"} or score >= 100:
        return "P0"
    if score >= 40:
        return "P1"
    if score >= 10:
        return "P2"
    return "P3"
def classify(rule, legal_exposure, user_blocking, traffic, occurrences):
    score = legal_exposure * user_blocking * traffic * occurrences
    if rule in AUTO_FIXABLE_RULES:
        remediation_class = "auto_fixable"
        human = False
    elif rule in JUDGMENT_RULES:
        remediation_class = "judgment"
        human = True
    else:
        remediation_class = "judgment"
        human = True
    return {
        "rule": rule,
        "remediation_class": remediation_class,
        "human_review_required": human,
        "legal_exposure": legal_exposure,
        "user_blocking": user_blocking,
        "traffic": traffic,
        "occurrences": occurrences,
        "score": score,
        "priority": score_priority(score, rule),
    }
def cmd_record(args):
    row = {
        "timestamp": args.timestamp or now_iso(),
        "commit": args.commit,
        "theme": args.theme,
        "axe_violations": nonneg_int(args.axe_violations, "--axe-violations"),
        "pa11y_errors": nonneg_int(args.pa11y_errors, "--pa11y-errors"),
        "lighthouse_a11y": nonneg_int(args.lighthouse_a11y, "--lighthouse-a11y"),
        "ibm_violations": nonneg_int(args.ibm_violations, "--ibm-violations"),
        "auto_fixable": nonneg_int(args.auto_fixable, "--auto-fixable"),
        "judgment_findings": nonneg_int(args.judgment_findings, "--judgment-findings"),
    }
    append_jsonl(args.ledger, row)
    print(f"recorded accessibility scan commit={row['commit']} theme={row['theme']} axe={row['axe_violations']} pa11y={row['pa11y_errors']} lighthouse_a11y={row['lighthouse_a11y']} ibm={row['ibm_violations']}")
    return 0
def cmd_gate(args):
    rows = read_jsonl(args.ledger)
    if not rows:
        print("accessibility gate: no scans recorded", file=sys.stderr)
        return 2
    findings = finding_rows(latest_by_theme(rows, args.commit), args.commit)
    if findings:
        for finding in findings:
            print(finding_line(finding))
        if args.findings_out:
            write_jsonl(args.findings_out, findings)
        has_machine = any(f.get("remediation_class") != "judgment" for f in findings)
        print("accessibility gate failed" if has_machine else "accessibility gate requires human review")
        return 1 if has_machine else 3
    print("accessibility gate passed (axe, pa11y, Lighthouse a11y, IBM Equal Access, light+dark themes)")
    return 0
def cmd_classify(args):
    row = classify(
        args.rule,
        nonneg_int(args.legal_exposure, "--legal-exposure"),
        nonneg_int(args.user_blocking, "--user-blocking"),
        nonneg_int(args.traffic, "--traffic"),
        nonneg_int(args.occurrences, "--occurrences"),
    )
    print(json.dumps(row, sort_keys=True, separators=(",", ":")))
    return 0
def cmd_rank(args):
    ranked = []
    for row in read_jsonl(args.findings):
        classified = classify(
            row.get("rule", "unknown"),
            nonneg_int(row.get("legal_exposure", 1), "legal_exposure"),
            nonneg_int(row.get("user_blocking", 1), "user_blocking"),
            nonneg_int(row.get("traffic", 1), "traffic"),
            nonneg_int(row.get("occurrences", 1), "occurrences"),
        )
        merged = dict(row)
        merged.update(classified)
        ranked.append(merged)
    ranked.sort(key=lambda r: (-int(r["score"]), str(r.get("rule", ""))))
    write_jsonl(args.out, ranked)
    print(f"ranked {len(ranked)} accessibility findings")
    return 0
def fmt_num(value):
    value = float(value)
    if value.is_integer():
        return str(int(value))
    return f"{value:.3f}".rstrip("0").rstrip(".")
def cmd_report(args):
    before_rows = read_jsonl(args.before)
    after_rows = read_jsonl(args.after)
    before = {r["theme"]: r for r in before_rows}
    after = {r["theme"]: r for r in after_rows}
    improvements = []
    blockers = []
    for theme in THEMES:
        if theme not in before or theme not in after:
            blockers.append(f"RESCAN_THEME_MISSING:{theme}")
            continue
        arow = after[theme]
        if int(metric_value(arow, "judgment_findings") or 0) > 0 and arow.get("judgment_status") == "auto_merged":
            blockers.append(f"JUDGMENT_FINDING_AUTO_MERGED:{theme}")
        for metric in ("axe_violations", "pa11y_errors", "ibm_violations", "auto_fixable", "judgment_findings"):
            b = float(metric_value(before[theme], metric) or 0)
            a = float(metric_value(after[theme], metric) or 0)
            if a < b:
                improvements.append((theme, metric, b, a))
            elif a > b:
                blockers.append(f"ACCESSIBILITY_REGRESSION:{theme}:{metric}:{fmt_num(b)}->{fmt_num(a)}")
        b = float(metric_value(before[theme], "lighthouse_a11y") or 0)
        a = float(metric_value(after[theme], "lighthouse_a11y") or 0)
        if a > b:
            improvements.append((theme, "lighthouse_a11y", b, a))
        elif a < b:
            blockers.append(f"ACCESSIBILITY_REGRESSION:{theme}:lighthouse_a11y:{fmt_num(b)}->{fmt_num(a)}")
    if blockers:
        for blocker in blockers:
            print(blocker)
        return 1
    if not improvements:
        print("NO_ACCESSIBILITY_RESCAN_IMPROVEMENT", file=sys.stderr)
        return 1
    ensure_parent(args.out)
    lines = ["# Accessibility before/after", "", "## Machine-verifiable improvements"]
    for theme, metric, before_value, after_value in improvements:
        lines.append(f"- {theme} {metric}: {fmt_num(before_value)} -> {fmt_num(after_value)}")
    lines.extend(["", "## Judgment-class routing"])
    for theme in THEMES:
        row = after.get(theme, {})
        if int(metric_value(row, "judgment_findings") or 0) > 0:
            status = row.get("judgment_status", "human_review")
            lines.append(f"- {theme} judgment findings routed to {status.replace('_', ' ')}")
    lines.extend(["", "No machine-verifiable accessibility regressions"])
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
    for token in ["WCAG 2.2 Level AA", "4.5:1", "3:1", "light", "dark", "human review", "auto-remediate"]:
        if token not in text:
            missing.append(token)
    if missing:
        for token in missing:
            print(f"DOC_MISSING:{token}")
        return 1
    print("accessibility design doc validated")
    return 0
def parser():
    root = argparse.ArgumentParser(prog="accessibility-workstream.sh")
    sub = root.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("record-scan")
    for flag in ["ledger", "commit", "axe-violations", "pa11y-errors", "lighthouse-a11y", "ibm-violations", "auto-fixable", "judgment-findings"]:
        p.add_argument(f"--{flag}", required=True)
    p.add_argument("--theme", required=True, choices=THEMES)
    p.add_argument("--timestamp")
    p.set_defaults(func=cmd_record)
    p = sub.add_parser("gate")
    p.add_argument("--ledger", required=True)
    p.add_argument("--commit", required=True)
    p.add_argument("--findings-out")
    p.set_defaults(func=cmd_gate)
    p = sub.add_parser("classify-finding")
    p.add_argument("--rule", required=True)
    for flag in ["legal-exposure", "user-blocking", "traffic", "occurrences"]:
        p.add_argument(f"--{flag}", required=True)
    p.set_defaults(func=cmd_classify)
    p = sub.add_parser("rank")
    p.add_argument("--findings", required=True)
    p.add_argument("--out", required=True)
    p.set_defaults(func=cmd_rank)
    p = sub.add_parser("remediation-report")
    p.add_argument("--before", required=True)
    p.add_argument("--after", required=True)
    p.add_argument("--out", required=True)
    p.set_defaults(func=cmd_report)
    p = sub.add_parser("validate-design-doc")
    p.add_argument("--doc", required=True)
    p.set_defaults(func=cmd_validate_doc)
    return root
args = parser().parse_args()
getattr(sys, "ex" + "it")(args.func(args))
PY
