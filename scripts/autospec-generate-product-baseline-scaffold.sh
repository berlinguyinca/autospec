#!/usr/bin/env bash
# scripts/autospec-generate-product-baseline-scaffold.sh — generate common product baseline specs and issues.

set -eu

usage() {
    cat <<'EOF'
Usage:
  autospec-generate-product-baseline-scaffold.sh [--repo-root DIR] [--dry-run|--confirm] [--capability NAME]

Dry-run is default. Confirm writes local specs and issue drafts only.
EOF
}

die() {
    printf 'autospec-generate-product-baseline-scaffold: %s\n' "$*" >&2
    exit 2
}

REPO_ROOT="$(pwd)"
CONFIRM=0
CAPABILITY="all"
while [ "$#" -gt 0 ]; do
    case "$1" in
        --repo-root) [ "$#" -ge 2 ] || die "--repo-root requires a value"; REPO_ROOT="$2"; shift 2 ;;
        --dry-run) CONFIRM=0; shift ;;
        --confirm) CONFIRM=1; shift ;;
        --capability) [ "$#" -ge 2 ] || die "--capability requires a value"; CAPABILITY="$2"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown arg: $1" ;;
    esac
done

[ -d "$REPO_ROOT" ] || die "--repo-root does not exist: $REPO_ROOT"
REPO_ROOT="$(cd "$REPO_ROOT" && pwd -P)"

python3 - "$REPO_ROOT" "$CONFIRM" "$CAPABILITY" <<'PY'
import json
import os
import sys
from datetime import date

root = os.path.realpath(sys.argv[1])
confirm = sys.argv[2] == "1"
capability = sys.argv[3]
reports = os.path.join(root, ".autospec", "reports")
state = os.path.join(root, ".autospec", "state")
issues_dir = os.path.join(root, ".autospec", "backlog", "issues-v3")
specs_dir = os.path.join(root, "docs", "specs")
today = date.today().isoformat()


def load(path, default):
    try:
        with open(path, "r", encoding="utf-8") as fh:
            data = json.load(fh)
        return data if isinstance(data, dict) else default
    except Exception:
        return default


def write_json(path, data):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as fh:
        json.dump(data, fh, indent=2, sort_keys=True)
        fh.write("\n")


def write_text(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as fh:
        fh.write(text.rstrip() + "\n")


rules = load(os.path.join(state, "rule-check-results.json"), load(os.path.join(reports, "rule-check-results.json"), {"results": []}))
failures = [r for r in rules.get("results", []) if r.get("status") in {"fail", "partial", "unknown"}]
items = [
    ("docs-center", "In-app documentation center", "In-app documentation section and RAG-ready docs are specified."),
    ("settings-area", "Settings area", "Application settings page and validation behavior are specified."),
    ("onboarding-tutorials", "Onboarding tutorials", "Tutorial workflow and screenshot artifacts are specified."),
    ("reporting-dashboard", "Reporting dashboard", "Reporting/PDF/CSV output expectations are specified."),
    ("analytics-metrics", "Analytics metrics", "Purposeful metrics and definitions are specified."),
    ("feedback-support", "Feedback support flow", "Support/feedback workflow is specified."),
    ("diagnostics-status", "Diagnostics status page", "Diagnostics and white-screen failure support are specified."),
    ("visual-design-system", "Responsive design system", "Responsive design tokens and accessibility expectations are specified."),
]
selected = items
spec = "\n".join([
    "# Product Baseline Scaffold",
    "",
    "## Failed structured rules considered",
    "",
    "\n".join(f"- `{item.get('rule_id', 'unknown')}` ({item.get('category', 'unknown')})" for item in failures[:12]) or "- None.",
    "",
    "## Baseline features",
    "",
    "\n".join(f"- {title}" for _, title, _ in selected),
])
outputs = [f"docs/specs/{today}-product-baseline-scaffold.md"] + [f".autospec/backlog/issues-v3/product-{idx:03d}-{slug}.md" for idx, (slug, _, _) in enumerate(selected, start=1)]
plan = {"schema": 1, "mode": "confirm" if confirm else "dry_run", "capability": capability, "outputs": outputs, "failed_rules_considered": len(failures), "github_writes": False}
write_json(os.path.join(reports, "product-baseline-scaffold-plan.json"), plan)
write_text(os.path.join(reports, "product-baseline-scaffold-plan.md"), "\n".join(["# Product Baseline Scaffold Plan", "", f"Mode: `{'confirm' if confirm else 'dry_run'}`", "", "## Outputs", "", "\n".join(f"- `{item}`" for item in outputs)]))
if confirm:
    write_text(os.path.join(specs_dir, f"{today}-product-baseline-scaffold.md"), spec)
    for idx, (slug, title, ac) in enumerate(selected, start=1):
        write_text(os.path.join(issues_dir, f"product-{idx:03d}-{slug}.md"), "\n".join([
            f"# feat: scaffold {title}",
            "",
            "<!-- autospec-plan-version: v3 -->",
            f"<!-- autospec-local-issue-id: product-{slug} -->",
            "",
            "## Structured rule evidence",
            "",
            "\n".join(f"- `{item.get('rule_id', 'unknown')}`: {item.get('status', 'unknown')}" for item in failures[:8]) or "- No direct failed rule found; generated from baseline capability.",
            "",
            "## Risk classification",
            "",
            "low-risk planning/spec issue; implementation requires separate worker classification.",
            "",
            "## Acceptance criteria",
            "",
            f"- [ ] {ac}",
        ]))
status = "written" if confirm else "planned"
write_json(os.path.join(reports, "product-baseline-scaffold-result.json"), {"schema": 1, "status": status, "outputs": outputs, "issues": [item[0] for item in selected]})
write_text(os.path.join(reports, "product-baseline-scaffold-result.md"), "\n".join(["# Product Baseline Scaffold Result", "", f"Status: **{status}**", "", spec]))
print(f"product baseline scaffold: {status}")
PY
