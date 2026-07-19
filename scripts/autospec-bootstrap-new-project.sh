#!/usr/bin/env bash
# scripts/autospec-bootstrap-new-project.sh — metadata-first bootstrap for new projects.

set -eu

usage() {
    cat <<'EOF'
Usage:
  autospec-bootstrap-new-project.sh [--repo-root DIR] [--dry-run|--confirm] --name NAME --profiles web,ai-platform [options]

Options:
  --application-type TYPE
  --maturity-target LEVEL
  --description TEXT
  --non-interactive

Dry-run is default. Confirm writes local metadata/spec files only.
EOF
}

die() {
    printf 'autospec-bootstrap-new-project: %s\n' "$*" >&2
    exit 2
}

REPO_ROOT="$(pwd)"
CONFIRM=0
NAME=""
PROFILES=""
APP_TYPE=""
MATURITY="production"
DESCRIPTION=""
NON_INTERACTIVE=0
while [ "$#" -gt 0 ]; do
    case "$1" in
        --repo-root) [ "$#" -ge 2 ] || die "--repo-root requires a value"; REPO_ROOT="$2"; shift 2 ;;
        --dry-run) CONFIRM=0; shift ;;
        --confirm) CONFIRM=1; shift ;;
        --name) [ "$#" -ge 2 ] || die "--name requires a value"; NAME="$2"; shift 2 ;;
        --profiles) [ "$#" -ge 2 ] || die "--profiles requires a value"; PROFILES="$2"; shift 2 ;;
        --application-type) [ "$#" -ge 2 ] || die "--application-type requires a value"; APP_TYPE="$2"; shift 2 ;;
        --maturity-target) [ "$#" -ge 2 ] || die "--maturity-target requires a value"; MATURITY="$2"; shift 2 ;;
        --description) [ "$#" -ge 2 ] || die "--description requires a value"; DESCRIPTION="$2"; shift 2 ;;
        --non-interactive) NON_INTERACTIVE=1; shift ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown arg: $1" ;;
    esac
done

[ -d "$REPO_ROOT" ] || die "--repo-root does not exist: $REPO_ROOT"
REPO_ROOT="$(cd "$REPO_ROOT" && pwd -P)"

python3 - "$REPO_ROOT" "$CONFIRM" "$NAME" "$PROFILES" "$APP_TYPE" "$MATURITY" "$DESCRIPTION" "$NON_INTERACTIVE" <<'PY'
import json
import os
import sys
from datetime import date

root = os.path.realpath(sys.argv[1])
confirm = sys.argv[2] == "1"
name, profiles_raw, app_type, maturity, description = sys.argv[3:8]
profiles = [p.strip() for p in profiles_raw.split(",") if p.strip()]
autospec = os.path.join(root, ".autospec")
reports = os.path.join(autospec, "reports")
state = os.path.join(autospec, "state")
specs = os.path.join(root, "docs", "specs")
today = date.today().isoformat()


def write_json(path, data):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as fh:
        json.dump(data, fh, indent=2, sort_keys=True)
        fh.write("\n")


def write_text(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as fh:
        fh.write(text.rstrip() + "\n")


def existing_repo():
    for base, dirs, files in os.walk(root):
        rel = os.path.relpath(base, root)
        if any(part in {".git", ".autospec", "node_modules"} for part in rel.split(os.sep)):
            dirs[:] = []
            continue
        if any(name.endswith((".py", ".js", ".ts", ".tsx", ".go", ".rs", ".java")) for name in files):
            return True
    return False


missing = []
if not name:
    missing.append("--name")
if not profiles:
    missing.append("--profiles")
if not app_type:
    missing.append("--application-type")
if missing:
    questionnaire = "\n".join([
        "# Autospec New Project Bootstrap",
        "",
        "## Guided questionnaire",
        "",
        "Autospec needs explicit product context before creating project metadata.",
        "",
        "## Missing inputs",
        "",
        "\n".join(f"- `{item}`" for item in missing),
        "",
        "## Questions",
        "",
        "- What is the project name?",
        "- Which baseline profiles apply?",
        "- What application type should the Constitution evaluate?",
        "- What is the first maturity target?",
    ])
    write_json(os.path.join(reports, "bootstrap-plan.json"), {"schema": 1, "status": "needs_inputs", "missing_inputs": missing})
    write_text(os.path.join(reports, "bootstrap-plan.md"), questionnaire)
    write_json(os.path.join(reports, "bootstrap-result.json"), {"schema": 1, "status": "needs_inputs", "missing_inputs": missing})
    write_text(os.path.join(reports, "bootstrap-result.md"), questionnaire)
    print("bootstrap: needs inputs")
    sys.exit(0)

warning = "existing repository detected; consider scripts/autospec-onboard-existing-repo.sh" if existing_repo() else ""
blueprint = "\n".join([
    "# Project Blueprint",
    "",
    f"## Mission\n\n{description or f'Build {name} under Autospec Constitution guidance.'}",
    "",
    "## Target users\n\n- Primary users to be confirmed.",
    "",
    "## Personas\n\n- Operator\n- End user",
    "",
    "## Primary workflows\n\n- Discover\n- Configure\n- Execute\n- Review results",
    "",
    "## Non-goals\n\n- No production secrets in source control.\n- No unreviewed migrations or auth changes.",
    "",
    "## Baseline profiles",
    "",
    "\n".join(f"- `{p}`" for p in profiles),
    "",
    "## Required baseline capabilities\n\n- Metadata foundation\n- Rule audit readiness\n- Validation evidence",
    "",
    "## Architecture starting point\n\nMetadata-first; implementation follows issue-plan-v3.",
    "",
    "## AI/NLAI expectations\n\nProvider abstraction, settings, RAG, token/cost tracking, and pretty rendering when AI profiles are selected.",
    "",
    "## Documentation expectations\n\nREADME, runbooks, tutorials, and in-app documentation when applicable.",
    "",
    "## Testing expectations\n\nFocused tests first, Playwright for web workflows, validation evidence in PRs.",
    "",
    "## Reporting/analytics expectations\n\nPurposeful metrics and exportable reports where useful.",
    "",
    "## Operations expectations\n\nStatus/diagnostics pages and incident-ready logs for services.",
    "",
    "## Security/privacy expectations\n\nSecret references only, no raw secret values, documented permission model when multi-user.",
    "",
    "## Initial issue backlog\n\n- Generate structured policy backlog.\n- Build first low-risk capability slice.\n",
])
implementation = "\n".join([
    "# Initial Implementation Plan",
    "",
    "## Sequence",
    "",
    "1. Build Digital Twin metadata.",
    "2. Run constitution audit.",
    "3. Generate issue-plan-v3.",
    "4. Publish backlog in confirmed mode.",
    "5. Run one supervisor cycle.",
])
metadata = {
    "schema": 1,
    "project": name,
    "application_type": app_type,
    "profiles": profiles,
    "maturity_target": maturity,
    "description": description,
    "warning": warning,
}
outputs = [
    ".autospec/autospec.yml",
    ".autospec/state/product-purpose.md",
    ".autospec/state/product-roadmap.md",
    ".autospec/state/personas.md",
    ".autospec/state/domain-model.json",
    ".autospec/state/workflow-map.json",
    ".autospec/state/architecture-map.json",
    ".autospec/state/knowledge-graph.json",
    ".autospec/state/feature-ledger.json",
    ".autospec/state/technology-registry.yml",
    ".autospec/state/capability-registry.json",
    ".autospec/state/api-surface.json",
    ".autospec/state/ui-surface.json",
    ".autospec/state/settings-registry.json",
    ".autospec/state/permission-model.json",
    ".autospec/state/ai-capabilities.json",
    ".autospec/state/mcp-registry.json",
    ".autospec/state/report-registry.json",
    ".autospec/state/tutorial-registry.json",
    ".autospec/state/quality-dashboard.json",
    f"docs/specs/{today}-project-blueprint.md",
    f"docs/specs/{today}-initial-implementation-plan.md",
]
plan = {"schema": 1, "status": "planned", "mode": "confirm" if confirm else "dry_run", "outputs": outputs, "metadata": metadata, "warning": warning}
write_json(os.path.join(reports, "bootstrap-plan.json"), plan)
write_text(os.path.join(reports, "bootstrap-plan.md"), "\n".join(["# Autospec Bootstrap Plan", "", f"Mode: `{'confirm' if confirm else 'dry_run'}`", "", "## Outputs", "", "\n".join(f"- `{item}`" for item in outputs), "", f"## Warning\n\n{warning or 'None.'}"]))

if confirm:
    write_text(os.path.join(autospec, "autospec.yml"), "\n".join(["schema: 1", f"project: {name}", "application:", f"  type: {app_type}", f"  maturity_target: {maturity}", "baselines:", "  profiles:", *[f"    - {p}" for p in profiles]]))
    write_text(os.path.join(state, "product-purpose.md"), f"# Product Purpose\n\n{description or name}")
    write_text(os.path.join(state, "product-roadmap.md"), "# Product Roadmap\n\n- Establish metadata foundation.\n- Generate structured rule backlog.")
    write_text(os.path.join(state, "personas.md"), "# Personas\n\n- Operator\n- End user")
    for filename, payload in {
        "domain-model.json": {"entities": []},
        "workflow-map.json": {"workflows": []},
        "architecture-map.json": {"components": []},
        "knowledge-graph.json": {"nodes": [], "edges": []},
        "feature-ledger.json": {"features": []},
        "capability-registry.json": {"capabilities": []},
        "api-surface.json": {"facts": []},
        "ui-surface.json": {"facts": []},
        "settings-registry.json": {"facts": []},
        "permission-model.json": {"facts": []},
        "ai-capabilities.json": {"facts": []},
        "mcp-registry.json": {"facts": []},
        "report-registry.json": {"facts": []},
        "tutorial-registry.json": {"facts": []},
        "quality-dashboard.json": {"facts": []},
    }.items():
        write_json(os.path.join(state, filename), {"schema": 1, "generated_by": "autospec-bootstrap-new-project", **payload})
    write_text(os.path.join(state, "technology-registry.yml"), "schema: 1\ntechnologies: []\n")
    write_text(os.path.join(specs, f"{today}-project-blueprint.md"), blueprint)
    write_text(os.path.join(specs, f"{today}-initial-implementation-plan.md"), implementation)
status = "written" if confirm else "planned"
write_json(os.path.join(reports, "bootstrap-result.json"), {"schema": 1, "status": status, "outputs": outputs, "metadata": metadata, "warning": warning})
write_text(os.path.join(reports, "bootstrap-result.md"), "\n".join(["# Autospec Bootstrap Result", "", f"Status: **{status}**", "", blueprint]))
print(f"bootstrap: {status}")
PY
