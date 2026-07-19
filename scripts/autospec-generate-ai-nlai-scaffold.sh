#!/usr/bin/env bash
# scripts/autospec-generate-ai-nlai-scaffold.sh — generate AI/NLAI scaffold specs and v3 issue drafts.

set -eu

usage() {
    cat <<'EOF'
Usage:
  autospec-generate-ai-nlai-scaffold.sh [--repo-root DIR] [--dry-run|--confirm] [--capability NAME]

Dry-run is default. Confirm writes local specs and issue drafts only.
EOF
}

die() {
    printf 'autospec-generate-ai-nlai-scaffold: %s\n' "$*" >&2
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


twin = load(os.path.join(state, "digital-twin.json"), {})
ai = load(os.path.join(state, "ai-capabilities.json"), {"facts": []})
rules = load(os.path.join(state, "rule-check-results.json"), load(os.path.join(reports, "rule-check-results.json"), {"results": []}))
ai_failures = [r for r in rules.get("results", []) if r.get("category") in {"ai", "rag", "mcp", "nlai"} and r.get("status") in {"fail", "partial", "unknown"}]
evidence = [
    f"digital twin application type: {(twin.get('summary') or {}).get('application_type', 'unknown')}",
    f"ai facts: {len(ai.get('facts', []))}",
    f"ai/nlai rule gaps: {len(ai_failures)}",
]
spec_ai = "\n".join([
    "# AI Platform Scaffold",
    "",
    "## Evidence",
    "",
    "\n".join(f"- {item}" for item in evidence),
    "",
    "## Scope",
    "",
    "- Provider abstraction",
    "- OpenAI-compatible API support",
    "- Ollama/local model support",
    "- Model selection",
    "- AI settings/admin page",
    "- API key/secret references only",
    "- RAG source and embedding configuration",
    "- Agent registry and tool registry",
    "- Memory model",
    "- MCP registry",
    "- Token usage and cost tracking",
    "- Per-user usage dashboard",
    "- Budget/quota settings",
    "- Audit logging",
])
spec_nlai = "\n".join([
    "# NLAI Capability Interface",
    "",
    "## Scope",
    "",
    "- Expose core app capabilities through tools.",
    "- Query and visualize data for data apps.",
    "- Generate SQL with explanation and safety review.",
    "- Inspect and operate on files for file apps.",
    "- Generate reports and exports.",
    "- Pretty-render all outputs.",
    "- Avoid raw JSON unless requested.",
    "- Render JSON/YAML/XML/SQL/Markdown/code with appropriate viewers.",
    "- Show citations, sources, and evidence where relevant.",
])
issues = [
    ("ai-assistant", "feat: scaffold AI assistant", "AI assistant", "Provider abstraction and assistant workflow are specified."),
    ("ai-settings", "feat: scaffold AI settings", "AI settings", "AI settings page and secret-reference behavior are specified."),
    ("rag-indexing", "feat: scaffold RAG indexing", "RAG indexing", "RAG sources, embeddings, refresh, and citations are specified."),
    ("token-usage", "feat: scaffold token usage dashboard", "Token usage", "Token usage and cost tracking expectations are specified."),
    ("nlai-capability", "feat: scaffold NLAI capability interface", "NLAI", "NLAI tool interface and pretty rendering are specified."),
    ("mcp-diagnostics", "feat: scaffold MCP diagnostics", "MCP diagnostics", "MCP registry diagnostics are specified."),
]
selected = issues
outputs = [
    f"docs/specs/{today}-ai-platform-scaffold.md",
    f"docs/specs/{today}-nlai-capability-interface.md",
] + [f".autospec/backlog/issues-v3/{idx + 1:03d}-{item[0]}.md" for idx, item in enumerate(selected)]
plan = {"schema": 1, "mode": "confirm" if confirm else "dry_run", "capability": capability, "outputs": outputs, "digital_twin_evidence": evidence, "github_writes": False}
write_json(os.path.join(reports, "ai-nlai-scaffold-plan.json"), plan)
write_text(os.path.join(reports, "ai-nlai-scaffold-plan.md"), "\n".join(["# AI/NLAI Scaffold Plan", "", f"Mode: `{'confirm' if confirm else 'dry_run'}`", "", "## Outputs", "", "\n".join(f"- `{item}`" for item in outputs)]))
if confirm:
    write_text(os.path.join(specs_dir, f"{today}-ai-platform-scaffold.md"), spec_ai)
    write_text(os.path.join(specs_dir, f"{today}-nlai-capability-interface.md"), spec_nlai)
    for idx, (slug, title, label, ac) in enumerate(selected, start=1):
        write_text(os.path.join(issues_dir, f"{idx:03d}-{slug}.md"), "\n".join([
            f"# {title}",
            "",
            "<!-- autospec-plan-version: v3 -->",
            f"<!-- autospec-local-issue-id: ai-nlai-{slug} -->",
            "",
            "## Digital Twin evidence",
            "",
            "\n".join(f"- {item}" for item in evidence),
            "",
            "## Risk classification",
            "",
            "low-risk planning/spec issue; implementation must be separately classified.",
            "",
            "## Tests required",
            "",
            "- Focused unit/integration tests for provider or tool boundaries.",
            "- Playwright expectations for UI settings and dashboard surfaces.",
            "",
            "## Token/cost/settings/security expectations",
            "",
            "- Track token usage and cost where multi-user or persisted usage exists.",
            "- Store only secret references; never include raw API keys.",
            "",
            "## Acceptance criteria",
            "",
            f"- [ ] {ac}",
        ]))
status = "written" if confirm else "planned"
write_json(os.path.join(reports, "ai-nlai-scaffold-result.json"), {"schema": 1, "status": status, "outputs": outputs, "issues": [item[0] for item in selected]})
write_text(os.path.join(reports, "ai-nlai-scaffold-result.md"), "\n".join(["# AI/NLAI Scaffold Result", "", f"Status: **{status}**", "", spec_ai, "", spec_nlai]))
print(f"ai/nlai scaffold: {status}")
PY
