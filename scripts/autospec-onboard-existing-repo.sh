#!/usr/bin/env bash
# scripts/autospec-onboard-existing-repo.sh — read-first adoption flow for established repositories.

set -eu

usage() {
    cat <<'EOF'
Usage:
  autospec-onboard-existing-repo.sh [--repo-root DIR] [--dry-run|--confirm] [--profiles a,b,c]

Dry-run is default. This command writes local Autospec metadata/reports only; it
does not publish issues, create PRs, merge, approve, or call GitHub.
EOF
}

die() {
    printf 'autospec-onboard-existing-repo: %s\n' "$*" >&2
    exit 2
}

REPO_ROOT="$(pwd)"
CONFIRM=0
PROFILES=""
while [ "$#" -gt 0 ]; do
    case "$1" in
        --repo-root) [ "$#" -ge 2 ] || die "--repo-root requires a value"; REPO_ROOT="$2"; shift 2 ;;
        --dry-run) CONFIRM=0; shift ;;
        --confirm) CONFIRM=1; shift ;;
        --profiles) [ "$#" -ge 2 ] || die "--profiles requires a value"; PROFILES="$2"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown arg: $1" ;;
    esac
done

[ -d "$REPO_ROOT" ] || die "--repo-root does not exist: $REPO_ROOT"
REPO_ROOT="$(cd "$REPO_ROOT" && pwd -P)"

python3 - "$REPO_ROOT" "$CONFIRM" "$PROFILES" <<'PY'
import json
import os
import sys

root = os.path.realpath(sys.argv[1])
confirm = sys.argv[2] == "1"
profiles = [p.strip() for p in sys.argv[3].split(",") if p.strip()]
autospec = os.path.join(root, ".autospec")
reports = os.path.join(autospec, "reports")
state = os.path.join(autospec, "state")
clarifications = os.path.join(autospec, "backlog", "clarifications")


def load_json(path, default):
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


def rel_files(prefixes):
    found = []
    for base, dirs, files in os.walk(root):
        rel_base = os.path.relpath(base, root)
        if rel_base == ".":
            rel_base = ""
        if any(part in {".git", "node_modules", "__pycache__"} for part in rel_base.split(os.sep)):
            dirs[:] = []
            continue
        for name in files:
            rel = os.path.join(rel_base, name).lstrip(os.sep)
            if any(rel.startswith(prefix) for prefix in prefixes):
                found.append(rel)
    return sorted(found)


rule_checks = load_json(os.path.join(state, "rule-check-results.json"), load_json(os.path.join(reports, "rule-check-results.json"), {"results": []}))
digital_twin = load_json(os.path.join(state, "digital-twin.json"), {})
capabilities = load_json(os.path.join(state, "capability-registry.json"), {"capabilities": []})
technology_path = os.path.join(state, "technology-registry.yml")
source_files = rel_files(["src/", "lib/", "scripts/"])
test_files = rel_files(["tests/", "test/"])
doc_files = rel_files(["docs/", "README"])
app_type = (digital_twin.get("summary") or {}).get("application_type") or ("web" if any(path.startswith("src/") for path in source_files) else "unknown")
metadata_files = sorted([
    ".autospec/state/digital-twin.json",
    ".autospec/state/capability-registry.json",
    ".autospec/state/rule-check-results.json",
    ".autospec/state/onboarding.json",
])
top_gaps = [item for item in rule_checks.get("results", []) if item.get("status") in {"fail", "partial", "unknown"}][:10]
low_confidence_facts = []
if not capabilities.get("capabilities"):
    low_confidence_facts.append({"fact": "capability registry is sparse", "confidence": 0.3, "evidence": ["no capabilities found"]})
if not os.path.exists(technology_path):
    low_confidence_facts.append({"fact": "technology registry missing", "confidence": 0.2, "evidence": [".autospec/state/technology-registry.yml not found"]})
if not doc_files:
    low_confidence_facts.append({"fact": "documentation surface unclear", "confidence": 0.4, "evidence": ["no docs/ or README files found"]})
if float(digital_twin.get("confidence", 0.0) or 0.0) < 0.8:
    low_confidence_facts.append({"fact": "Digital Twin confidence below onboarding threshold", "confidence": digital_twin.get("confidence", 0.0), "evidence": ["digital twin confidence is below 0.8"]})

questions = []
for idx, fact in enumerate(low_confidence_facts, start=1):
    question = {
        "id": f"clarification-{idx:03d}",
        "question": f"Confirm: {fact['fact']}?",
        "why_uncertain": "Autospec only has low-confidence local evidence for this repository fact.",
        "evidence": fact["evidence"],
        "confidence": fact["confidence"],
        "possible_answers": ["yes", "no", "not applicable"],
        "recommended_default": "yes",
        "affected_rules_gaps": [gap.get("rule_id", "") for gap in top_gaps[:3]],
    }
    questions.append(question)
    write_text(os.path.join(clarifications, f"{question['id']}.md"), "\n".join([
        f"# {question['question']}",
        "",
        "## Why Autospec is uncertain",
        "",
        question["why_uncertain"],
        "",
        "## Evidence",
        "",
        "\n".join(f"- {item}" for item in question["evidence"]),
        "",
        f"## Confidence\n\n{question['confidence']}",
        "",
        "## Possible answers",
        "",
        "\n".join(f"- {item}" for item in question["possible_answers"]),
        "",
        f"## Recommended default\n\n{question['recommended_default']}",
        "",
        "## Affected rules/gaps",
        "",
        "\n".join(f"- `{item}`" for item in question["affected_rules_gaps"] if item) or "- None.",
    ]))

onboarding = {
    "schema": 1,
    "mode": "existing_repo",
    "repo": os.path.basename(root),
    "application_type": app_type,
    "profiles": profiles,
    "metadata_files": metadata_files,
    "confidence_summary": {
        "digital_twin": digital_twin.get("confidence", 0.0),
        "low_confidence_facts": len(low_confidence_facts),
        "capabilities_detected": len(capabilities.get("capabilities", [])),
    },
    "low_confidence_facts": low_confidence_facts,
    "clarification_questions": questions,
    "top_gaps": top_gaps,
    "recommended_issues": [gap.get("suggested_issue_title") or gap.get("title") or gap.get("rule_id") for gap in top_gaps[:8]],
    "next_commands": [
        "bash scripts/autospec-constitution-audit.sh",
        "bash scripts/autospec-audit-to-backlog.sh --dry-run",
        "bash scripts/autospec-autonomy-status.sh",
    ],
}
write_json(os.path.join(state, "onboarding.json"), onboarding)

plan = {
    "schema": 1,
    "mode": "confirm" if confirm else "dry_run",
    "profiles": profiles,
    "metadata_only": True,
    "github_writes": False,
    "pr_creation": False,
    "planned_outputs": onboarding["metadata_files"] + [".autospec/reports/onboarding-result.md"],
}
write_json(os.path.join(reports, "onboarding-plan.json"), plan)
write_json(os.path.join(reports, "onboarding-result.json"), onboarding)

gap_lines = "\n".join(f"- `{gap.get('rule_id', 'unknown')}`: {gap.get('title', gap.get('summary', 'gap'))}" for gap in top_gaps) or "- None."
question_lines = "\n".join(f"- {q['question']} (`{q['id']}`)" for q in questions) or "- None."
md = "\n".join([
    "# Autospec Existing Repository Onboarding",
    "",
    "## Executive summary",
    "",
    f"Autospec inspected `{os.path.basename(root)}` in `{'confirm' if confirm else 'dry_run'}` mode and prepared metadata-only onboarding evidence.",
    "",
    "## Detected product/application type",
    "",
    app_type,
    "",
    "## Selected profiles",
    "",
    "\n".join(f"- `{p}`" for p in profiles) or "- None selected.",
    "",
    "## Technology summary",
    "",
    f"- Technology registry: `{technology_path if os.path.exists(technology_path) else 'missing'}`",
    "",
    "## Capability summary",
    "",
    f"- Capabilities detected: {len(capabilities.get('capabilities', []))}",
    "",
    "## Digital Twin summary",
    "",
    f"- Confidence: `{digital_twin.get('confidence', 0.0)}`",
    "",
    "## Confidence summary",
    "",
    f"- Low-confidence facts: {len(low_confidence_facts)}",
    "",
    "## Low-confidence assumptions",
    "",
    "\n".join(f"- {fact['fact']} ({fact['confidence']})" for fact in low_confidence_facts) or "- None.",
    "",
    "## Clarification questions",
    "",
    question_lines,
    "",
    "## Constitutional gaps",
    "",
    gap_lines,
    "",
    "## Maturity score",
    "",
    f"- See `.autospec/reports/maturity-score.md` if present.",
    "",
    "## Recommended issue backlog",
    "",
    "\n".join(f"- {item}" for item in onboarding["recommended_issues"]) or "- None.",
    "",
    "## Safe next commands",
    "",
    "\n".join(f"- `{cmd}`" for cmd in onboarding["next_commands"]),
])
write_text(os.path.join(reports, "onboarding-plan.md"), md)
write_text(os.path.join(reports, "onboarding-result.md"), md)

print("onboarding: wrote local metadata reports")
PY
