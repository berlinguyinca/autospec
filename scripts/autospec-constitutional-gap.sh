#!/usr/bin/env bash
# scripts/autospec-constitutional-gap.sh — summarize Constitution/Baseline gaps.

set -eu

usage() {
    cat <<'EOF'
Usage:
  autospec-constitutional-gap.sh [--repo-root <dir>]

Inputs:
  local constitution docs/schemas from .autospec/autospec.yml when configured
  .autospec/reports/baseline-composition.json
  .autospec/reports/metadata-discovery.json
  .autospec/reports/baseline-gap-analysis.json

Writes:
  .autospec/reports/constitutional-gap-report.json
  .autospec/reports/constitutional-gap-report.md
EOF
}

die() {
    printf 'autospec-constitutional-gap: %s\n' "$*" >&2
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

try:
    import yaml
except Exception:
    yaml = None

repo_root = os.path.realpath(sys.argv[1])
reports_dir = os.path.join(repo_root, ".autospec", "reports")
config_path = os.path.join(repo_root, ".autospec", "autospec.yml")
metadata_path = os.path.join(reports_dir, "metadata-discovery.json")
baseline_gap_path = os.path.join(reports_dir, "baseline-gap-analysis.json")
composition_path = os.path.join(reports_dir, "baseline-composition.json")
json_report = os.path.join(reports_dir, "constitutional-gap-report.json")
md_report = os.path.join(reports_dir, "constitutional-gap-report.md")


def load_json(path, default):
    try:
        with open(path, "r", encoding="utf-8") as fh:
            return json.load(fh)
    except Exception:
        return default


def load_yaml(path):
    if yaml is None:
        return {}
    try:
        with open(path, "r", encoding="utf-8") as fh:
            data = yaml.safe_load(fh)
        return data if isinstance(data, dict) else {}
    except Exception:
        return {}


def resolve_local(path_text):
    if not path_text:
        return ""
    if os.path.isabs(path_text):
        return os.path.realpath(path_text)
    return os.path.realpath(os.path.join(repo_root, path_text))


def write_json(path, data):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as fh:
        json.dump(data, fh, indent=2, sort_keys=True)
        fh.write("\n")


def issue(title, acceptance):
    return {"title": title, "acceptance_criteria": acceptance}


def section(status, summary, evidence, suggestions):
    return {
        "status": status,
        "summary": summary,
        "evidence": evidence,
        "suggested_issues": suggestions,
    }


def gaps_for(*tokens):
    result = []
    for item in baseline_gap.get("matrix", []):
        capability = str(item.get("capability", "")).lower()
        if item.get("status") in {"present", "opted_out"}:
            continue
        if any(token in capability for token in tokens):
            result.append(item)
    return result


config = load_yaml(config_path)
constitution_cfg = config.get("constitution") if isinstance(config.get("constitution"), dict) else {}
constitution_path = resolve_local(constitution_cfg.get("path", ""))
constitution_docs = []
if constitution_path and os.path.isdir(constitution_path):
    for root, _, names in os.walk(constitution_path):
        for name in sorted(names):
            if name.endswith((".md", ".schema.json")):
                constitution_docs.append(os.path.relpath(os.path.join(root, name), constitution_path).replace(os.sep, "/"))

metadata = load_json(metadata_path, {})
baseline_gap = load_json(baseline_gap_path, {})
composition = load_json(composition_path, {})
docs_status = metadata.get("coverage", {}).get("docs", {}).get("status", "unknown")
tests_status = metadata.get("coverage", {}).get("tests", {}).get("status", "unknown")
api_present = metadata.get("indicators", {}).get("api", {}).get("value")
ui_present = metadata.get("indicators", {}).get("ui", {}).get("value")
ai_present = metadata.get("indicators", {}).get("ai_rag", {}).get("value")
ci_present = bool(metadata.get("inventory", {}).get("ci_workflows"))

sections = {}
sections["product_purpose_gaps"] = section(
    "gap" if not metadata.get("facts", {}).get("product_purpose", {}).get("evidence") else "ok",
    "Product purpose evidence is present." if metadata.get("facts", {}).get("product_purpose", {}).get("evidence") else "Product purpose was not discovered.",
    metadata.get("facts", {}).get("product_purpose", {}).get("evidence", []),
    [] if metadata.get("facts", {}).get("product_purpose", {}).get("evidence") else [issue("docs: document product purpose", ["README.md states the product purpose.", "Metadata discovery cites README.md as evidence."])],
)
arch_gaps = gaps_for("api", "database", "migration", "container")
sections["architecture_gaps"] = section(
    "gap" if arch_gaps else "ok",
    "Architecture-related baseline gaps were found." if arch_gaps else "No architecture baseline gaps found by v0 rules.",
    [g["capability"] for g in arch_gaps],
    [issue("feat: document architecture baseline evidence", ["Architecture evidence is discoverable in docs or config.", "Baseline gap analysis reports the capability as present."])] if arch_gaps else [],
)
test_gaps = gaps_for("test", "qa", "playwright", "e2e")
sections["testing_gaps"] = section(
    "gap" if test_gaps or tests_status == "missing" else "ok",
    "Testing evidence is missing or incomplete." if test_gaps or tests_status == "missing" else "Testing evidence is present.",
    [g["capability"] for g in test_gaps] or metadata.get("coverage", {}).get("tests", {}).get("evidence", []),
    [issue("test: add baseline testing evidence", ["A tests/ directory or recognized test files exist.", "Baseline gap analysis no longer reports testing gaps."])] if test_gaps or tests_status == "missing" else [],
)
doc_gaps = gaps_for("doc")
sections["documentation_gaps"] = section(
    "gap" if doc_gaps or docs_status == "missing" else "ok",
    "Documentation evidence is missing or incomplete." if doc_gaps or docs_status == "missing" else "Documentation evidence is present.",
    [g["capability"] for g in doc_gaps] or metadata.get("coverage", {}).get("docs", {}).get("evidence", []),
    [issue("docs: add baseline documentation evidence", ["README.md or docs/ explains the repository.", "Metadata discovery reports documentation coverage as present."])] if doc_gaps or docs_status == "missing" else [],
)
ui_gaps = gaps_for("ui", "ux", "frontend", "web")
sections["ui_ux_gaps"] = section(
    "gap" if ui_gaps or api_present and not ui_present else "ok",
    "UI/UX baseline evidence is missing or incomplete." if ui_gaps or api_present and not ui_present else "UI/UX evidence is present or not requested.",
    [g["capability"] for g in ui_gaps] or metadata.get("ui_surface", {}).get("evidence", []),
    [issue("feat: add UI baseline evidence", ["UI entry points or components are discoverable.", "Baseline gap analysis reports UI capability as present or intentionally opted out."])] if ui_gaps else [],
)
ai_gaps = gaps_for("ai", "rag", "llm")
sections["ai_platform_gaps"] = section(
    "gap" if ai_gaps else "ok",
    "AI platform baseline evidence is missing or incomplete." if ai_gaps else "No AI platform baseline gaps found by v0 rules.",
    [g["capability"] for g in ai_gaps] or metadata.get("indicators", {}).get("ai_rag", {}).get("evidence", []),
    [issue("feat: add AI platform baseline evidence", ["AI/RAG usage is documented or discoverable.", "Baseline gap analysis reports AI capability as present or opted out."])] if ai_gaps else [],
)
ops_gaps = gaps_for("ci", "operation", "docker", "container")
sections["operations_gaps"] = section(
    "gap" if ops_gaps or not ci_present else "ok",
    "Operations evidence is missing or incomplete." if ops_gaps or not ci_present else "Operations evidence is present.",
    [g["capability"] for g in ops_gaps] or metadata.get("inventory", {}).get("ci_workflows", []),
    [issue("chore: add operations baseline evidence", ["CI or operations files are discoverable.", "Metadata discovery reports operations evidence."])] if ops_gaps or not ci_present else [],
)
metadata_missing = [name for name in ["metadata-discovery.json", "baseline-gap-analysis.json", "baseline-composition.json"] if not os.path.isfile(os.path.join(reports_dir, name))]
sections["metadata_gaps"] = section(
    "gap" if metadata_missing else "ok",
    "Required intelligence reports are missing." if metadata_missing else "Required intelligence reports are present.",
    metadata_missing,
    [issue("chore: regenerate autospec intelligence reports", ["Metadata discovery report exists.", "Baseline gap analysis report exists.", "Baseline composition report exists."])] if metadata_missing else [],
)
unknown_gaps = [item for item in baseline_gap.get("matrix", []) if item.get("status") == "unknown"]
sections["modernization_dependency_gaps"] = section(
    "gap" if unknown_gaps else "ok",
    "Some baseline capabilities need newer rule support or explicit documentation." if unknown_gaps else "No unknown baseline capabilities found.",
    [item.get("capability", "") for item in unknown_gaps],
    [issue("feat: classify unknown baseline capabilities", ["Unknown capabilities are mapped to deterministic metadata rules or explicitly opted out.", "Baseline gap analysis no longer reports unknown statuses."])] if unknown_gaps else [],
)

next_issues = []
for key in [
    "product_purpose_gaps",
    "architecture_gaps",
    "testing_gaps",
    "documentation_gaps",
    "ui_ux_gaps",
    "ai_platform_gaps",
    "operations_gaps",
    "metadata_gaps",
    "modernization_dependency_gaps",
]:
    next_issues.extend(sections[key]["suggested_issues"])

status = "pass" if not next_issues else "fail"
report = {
    "version": 1,
    "status": status,
    "inputs": {
        "constitution_path": constitution_path,
        "constitution_docs": sorted(constitution_docs),
        "baseline_composition": composition_path,
        "metadata_discovery": metadata_path,
        "baseline_gap_analysis": baseline_gap_path,
    },
    "sections": sections,
    "next_recommended_issues": next_issues,
    "notes": ["No issues were created; recommendations are report-only."],
}
write_json(json_report, report)

lines = ["# Constitutional Gap Report", "", f"Status: **{status.upper()}**", ""]
lines.extend(["## Summary", "", "| Section | Status | Summary |", "| --- | --- | --- |"])
for name, data in sections.items():
    lines.append(f"| {name.replace('_', ' ')} | {data['status']} | {data['summary']} |")
lines.extend(["", "## Next Recommended Issues", ""])
if next_issues:
    for item in next_issues:
        lines.append(f"### {item['title']}")
        for ac in item["acceptance_criteria"]:
            lines.append(f"- [ ] {ac}")
        lines.append("")
else:
    lines.append("- None.")
lines.extend(["## Constitution Evidence", ""])
if constitution_docs:
    for doc in sorted(constitution_docs):
        lines.append(f"- `{doc}`")
else:
    lines.append("- No local constitution docs found from `.autospec/autospec.yml`.")
with open(md_report, "w", encoding="utf-8") as fh:
    fh.write("\n".join(lines).rstrip())
    fh.write("\n")

print(f"constitutional gap report: {status.upper()}")
print("reports: .autospec/reports/constitutional-gap-report.json, .autospec/reports/constitutional-gap-report.md")
sys.exit(0 if status == "pass" else 1)
PY
