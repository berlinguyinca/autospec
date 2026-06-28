#!/usr/bin/env bash
# scripts/autospec-spec-implementation-sweep.sh — plan safe coverage-driven engine work.

set -eu

usage() {
  cat <<'EOF'
Usage:
  autospec-spec-implementation-sweep.sh [--repo-root DIR] [--dry-run|--confirm] [--category NAME] [--priority a,b]

Dry-run is default. Confirm may write only local Autospec scripts/docs/templates/tests/reports/state/backlog artifacts.
It never publishes issues, upgrades dependencies, runs migrations, or changes target application runtime behavior.
EOF
}

die() { printf 'autospec-spec-implementation-sweep: %s\n' "$*" >&2; exit 2; }

REPO_ROOT="$(pwd)"
MODE="dry_run"
CATEGORY=""
PRIORITY=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --repo-root) [ "$#" -ge 2 ] || die "--repo-root requires a value"; REPO_ROOT="$2"; shift 2 ;;
    --dry-run) MODE="dry_run"; shift ;;
    --confirm) MODE="confirm"; shift ;;
    --category) [ "$#" -ge 2 ] || die "--category requires a value"; CATEGORY="$2"; shift 2 ;;
    --priority) [ "$#" -ge 2 ] || die "--priority requires a value"; PRIORITY="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown arg: $1" ;;
  esac
done

[ -d "$REPO_ROOT" ] || die "--repo-root does not exist: $REPO_ROOT"
REPO_ROOT="$(cd "$REPO_ROOT" && pwd -P)"

python3 - "$REPO_ROOT" "$MODE" "$CATEGORY" "$PRIORITY" <<'PY'
import json
import re
import sys
from pathlib import Path

root = Path(sys.argv[1]).resolve()
mode = sys.argv[2]
category_filter = sys.argv[3]
priority_filter = {p.strip() for p in sys.argv[4].split(",") if p.strip()}
reports = root / ".autospec/reports"
reports.mkdir(parents=True, exist_ok=True)

def load(path, default):
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
        return data if isinstance(data, dict) else default
    except Exception:
        return default

def write_json(path, data):
    path.write_text(json.dumps(data, indent=2, sort_keys=True) + "\n", encoding="utf-8")

def slug(value):
    return re.sub(r"[^a-z0-9]+", "-", str(value).lower()).strip("-") or "requirement"

coverage = load(root / ".autospec/state/master-requirements.json", {})
if not coverage.get("requirements"):
    coverage = load(root / ".autospec/reports/spec-coverage.json", {"requirements": []})
requirements = coverage.get("requirements", [])

def bucket(req):
    status = req.get("status")
    rtype = req.get("requirement_type")
    rid = req.get("id", "")
    cat = req.get("category", "")
    if status == "missing" and rtype in {"engine", "worker", "verifier"}:
        return "can_implement_now"
    if status in {"missing", "partial", "documented_only", "validated"} and rtype in {"validator", "policy"}:
        return "can_add_rule_check"
    if rtype == "target_app_scaffold" or cat in {"ai_platform", "nlai", "product_baseline", "ui_ux", "docs_tutorial_pdf", "reporting_analytics_visualization"}:
        return "can_add_scaffold"
    if status == "documented_only":
        return "can_add_template"
    if status == "partial":
        return "can_add_fixture"
    if status == "deferred":
        return "defer_beyond_mvp"
    if "security" in rid or req.get("risk") == "high":
        return "requires_human_guidance"
    return "requires_target_repo"

selected = []
for req in requirements:
    if category_filter and req.get("category") != category_filter:
        continue
    if priority_filter and req.get("priority") not in priority_filter:
        continue
    action = bucket(req)
    selected.append({**req, "implementation_bucket": action})

counts = {name: 0 for name in [
    "can_implement_now", "can_add_rule_check", "can_add_scaffold", "can_add_template",
    "can_add_runbook", "can_add_fixture", "requires_target_repo", "requires_human_guidance",
    "defer_beyond_mvp",
]}
for req in selected:
    counts[req["implementation_bucket"]] = counts.get(req["implementation_bucket"], 0) + 1
if selected:
    counts["can_add_runbook"] = max(1, counts["can_add_runbook"])

plan = {
    "schema": 1,
    "mode": mode,
    "filters": {"category": category_filter, "priority": sorted(priority_filter)},
    "classification_counts": counts,
    "requirements": selected,
    "allowed_write_scopes": ["scripts/", "docs/", ".autospec/templates/", ".autospec/state/", ".autospec/reports/", "tests/fixtures/", "tests/", "schemas/"],
    "side_effects": {"github_writes": False, "dependency_updates": False, "migrations": False, "target_app_runtime_changes": False},
}
result = {
    "schema": 1,
    "mode": mode,
    "planned_requirements": len(selected),
    "classification_counts": counts,
    "implemented_by_this_command": [],
    "side_effects": plan["side_effects"],
}
write_json(reports / "spec-implementation-sweep-plan.json", plan)
write_json(reports / "spec-implementation-sweep-result.json", result)
rows = "\n".join(f"| `{req.get('id')}` | {req.get('status')} | {req.get('priority')} | `{req['implementation_bucket']}` |" for req in selected)
(reports / "spec-implementation-sweep-plan.md").write_text("\n".join([
    "# Autospec Spec Implementation Sweep Plan",
    "",
    "## Summary",
    "",
    f"- Mode: `{mode}`",
    f"- Requirements selected: {len(selected)}",
    "- GitHub writes: false",
    "- Dependency updates: false",
    "",
    "## Classification Counts",
    "",
    "\n".join(f"- `{k}`: {v}" for k, v in sorted(counts.items())),
    "",
    "## Requirements",
    "",
    "| Requirement | Status | Priority | Bucket |",
    "| --- | --- | --- | --- |",
    rows,
]) + "\n", encoding="utf-8")
(reports / "spec-implementation-sweep-result.md").write_text("\n".join([
    "# Autospec Spec Implementation Sweep Result",
    "",
    "## Summary",
    "",
    f"- Planned requirements: {len(selected)}",
    "- GitHub writes: false",
    "- Target application runtime changes: false",
    "",
    "## Next Commands",
    "",
    "- `bash scripts/autospec-doctrine-audit.sh --dry-run --all`",
]) + "\n", encoding="utf-8")
print("spec implementation sweep: wrote plan")
PY
