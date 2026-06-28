#!/usr/bin/env bash
# scripts/autospec-baseline-gap.sh — compare composed Baseline profiles to local metadata.

set -eu

usage() {
    cat <<'EOF'
Usage:
  autospec-baseline-gap.sh [--repo-root <dir>]

Inputs:
  .autospec/state/*
  .autospec/reports/baseline-composition.json

Writes:
  .autospec/reports/baseline-gap-analysis.json
  .autospec/reports/baseline-gap-analysis.md
EOF
}

die() {
    printf 'autospec-baseline-gap: %s\n' "$*" >&2
    exit 2
}

REPO_ROOT="$(pwd)"
while [ "$#" -gt 0 ]; do
    case "$1" in
        --repo-root) [ "$#" -ge 2 ] || die "--repo-root requires a value"; REPO_ROOT="$2"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown arg: $1" ;;
    esac
done

[ -d "$REPO_ROOT" ] || die "--repo-root does not exist: $REPO_ROOT"
REPO_ROOT="$(cd "$REPO_ROOT" && pwd -P)"

python3 - "$REPO_ROOT" <<'PY'
import json
import os
import sys

repo_root = os.path.realpath(sys.argv[1])
state_dir = os.path.join(repo_root, ".autospec", "state")
reports_dir = os.path.join(repo_root, ".autospec", "reports")
composition_path = os.path.join(reports_dir, "baseline-composition.json")
metadata_path = os.path.join(reports_dir, "metadata-discovery.json")
json_report = os.path.join(reports_dir, "baseline-gap-analysis.json")
md_report = os.path.join(reports_dir, "baseline-gap-analysis.md")


def load_json(path, default):
    try:
        with open(path, "r", encoding="utf-8") as fh:
            return json.load(fh)
    except Exception:
        return default


def write_json(path, data):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as fh:
        json.dump(data, fh, indent=2, sort_keys=True)
        fh.write("\n")


def row(profile, capability, status, confidence, evidence, priority, title):
    return {
        "feature_family": profile or "baseline",
        "capability": capability,
        "status": status,
        "confidence": confidence,
        "evidence": evidence,
        "priority": priority,
        "suggested_issue_title": title,
    }


def docs_status(metadata):
    coverage = metadata.get("coverage", {}).get("docs", {})
    if coverage.get("status") == "present":
        return "present", 0.9, coverage.get("evidence", [])
    return "missing", 0.8, coverage.get("evidence", ["metadata discovery found no documentation"])


def tests_status(metadata):
    coverage = metadata.get("coverage", {}).get("tests", {})
    if coverage.get("status") == "present":
        return "present", 0.9, coverage.get("evidence", [])
    return "missing", 0.8, coverage.get("evidence", ["metadata discovery found no tests"])


def api_status(metadata):
    api = metadata.get("api_surface", {})
    if api.get("openapi_documents"):
        return "present", 0.9, api.get("evidence", [])
    if api.get("present"):
        return "partial", 0.65, api.get("evidence", [])
    return "missing", 0.75, api.get("evidence", ["metadata discovery found no API indicators"])


def ui_status(metadata):
    ui = metadata.get("ui_surface", {})
    if ui.get("present"):
        return "present", 0.85, ui.get("evidence", [])
    return "missing", 0.75, ui.get("evidence", ["metadata discovery found no UI indicators"])


def indicator_status(metadata, name, missing_reason):
    indicator = metadata.get("indicators", {}).get(name, {})
    if indicator.get("value") is True:
        return "present", indicator.get("confidence", 0.8), indicator.get("evidence", [])
    return "missing", indicator.get("confidence", 0.5), indicator.get("evidence", [missing_reason])


def inventory_status(metadata, key, missing_reason):
    values = metadata.get("inventory", {}).get(key, [])
    if values:
        return "present", 0.85, values
    return "missing", 0.65, [missing_reason]


def classify(capability, metadata):
    cid = capability.lower()
    if "doc" in cid:
        return docs_status(metadata)
    if "test" in cid or "qa" in cid:
        return tests_status(metadata)
    if "api" in cid or "http" in cid or "route" in cid:
        return api_status(metadata)
    if "ui" in cid or "ux" in cid or "frontend" in cid or "web" in cid:
        return ui_status(metadata)
    if "playwright" in cid or "e2e" in cid:
        return indicator_status(metadata, "playwright_e2e", "metadata discovery found no Playwright/e2e indicators")
    if "cli" in cid:
        return indicator_status(metadata, "cli", "metadata discovery found no CLI indicators")
    if "ai" in cid or "rag" in cid or "llm" in cid:
        return indicator_status(metadata, "ai_rag", "metadata discovery found no AI/RAG indicators")
    if "ci" in cid or "operation" in cid:
        return inventory_status(metadata, "ci_workflows", "metadata discovery found no CI workflows")
    if "docker" in cid or "container" in cid:
        return inventory_status(metadata, "container_files", "metadata discovery found no Docker/container files")
    if "database" in cid or "migration" in cid:
        return inventory_status(metadata, "database_migration_indicators", "metadata discovery found no database/migration indicators")
    return "unknown", 0.2, ["no simple v0 rule matched this capability"]


composition = load_json(composition_path, {})
metadata = load_json(metadata_path, {})
capabilities = composition.get("composed", {}).get("capabilities", [])
matrix = []
findings = []

if not capabilities:
    findings.append({
        "code": "BASELINE_COMPOSITION_MISSING",
        "message": "baseline composition report is missing or has no capabilities",
        "action": "Run scripts/autospec-baseline-compose.sh before baseline gap analysis.",
    })

for item in capabilities:
    capability = str(item.get("id", ""))
    profile = str(item.get("profile", "baseline"))
    if not capability:
        continue
    if item.get("opt_out") is True:
        status, confidence, evidence = "opted_out", 1.0, ["capability has opt_out: true in baseline composition"]
    else:
        status, confidence, evidence = classify(capability, metadata)
    priority = "none" if status in {"present", "opted_out"} else ("high" if status == "missing" else "medium")
    title = "" if priority == "none" else f"feat: close {capability} baseline gap"
    matrix.append(row(profile, capability, status, confidence, evidence, priority, title))

status = "pass" if matrix and all(item["status"] in {"present", "opted_out"} for item in matrix) else "fail"
report = {
    "version": 1,
    "status": status,
    "inputs": {
        "metadata_discovery": metadata_path,
        "baseline_composition": composition_path,
    },
    "matrix": matrix,
    "findings": findings,
}
write_json(json_report, report)

lines = [
    "# Baseline Gap Analysis",
    "",
    f"Status: **{status.upper()}**",
    "",
    "| feature_family | capability | status | confidence | priority | evidence | suggested_issue_title |",
    "| --- | --- | --- | ---: | --- | --- | --- |",
]
for item in matrix:
    evidence = "; ".join(item["evidence"][:3])
    lines.append(f"| {item['feature_family']} | {item['capability']} | {item['status']} | {item['confidence']:.2f} | {item['priority']} | {evidence} | {item['suggested_issue_title']} |")
lines.extend(["", "## Remediation Suggestions", ""])
suggestions = [item for item in matrix if item["suggested_issue_title"]]
if suggestions:
    for item in suggestions:
        lines.append(f"- `{item['suggested_issue_title']}`: add evidence for `{item['capability']}` or mark it opted out in the baseline pack.")
else:
    lines.append("- None.")
with open(md_report, "w", encoding="utf-8") as fh:
    fh.write("\n".join(lines))
    fh.write("\n")

print(f"baseline gap analysis: {status.upper()}")
print("reports: .autospec/reports/baseline-gap-analysis.json, .autospec/reports/baseline-gap-analysis.md")
sys.exit(0 if status == "pass" else 1)
PY
