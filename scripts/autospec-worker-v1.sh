#!/usr/bin/env bash
# scripts/autospec-worker-v1.sh — bounded low-risk worker gate.
#
# Dry-run by default. This v1 worker processes exactly one planned issue and
# produces risk, packet, validation, diff, PR-evidence, and stuck reports. It
# allows code work only when config explicitly enables low-risk code changes.

set -eu

usage() {
    cat <<'EOF'
Usage:
  autospec-worker-v1.sh [--repo-root <dir>] [--issue-id <id>|--issue <number>] [--dry-run|--confirm]
  autospec-worker-v1.sh [--repo-root <dir>] [--dry-run|--confirm] --remediate --pr <number> [--branch <name>]

Inputs:
  .autospec/reports/issue-plan.json
  .autospec/backlog/issues/*.md
  .autospec/autospec.yml

Writes:
  .autospec/reports/worker-risk-classification.json
  .autospec/reports/worker-risk-classification.md
  .autospec/reports/worker-validation-plan.json
  .autospec/reports/worker-validation-plan.md
  .autospec/reports/worker-diff-review.json
  .autospec/reports/worker-diff-review.md
  .autospec/reports/worker-pr-body.md
  .autospec/reports/worker-stuck-handoff.md
  .autospec/state/implementation-packet.md
EOF
}

die() {
    printf 'autospec-worker-v1: %s\n' "$*" >&2
    exit 2
}

REPO_ROOT="$(pwd)"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd -P)"
ISSUE_ID=""
CONFIRM=0
REMEDIATE=0
PR=""
BRANCH=""
while [ "$#" -gt 0 ]; do
    case "$1" in
        --repo-root) [ "$#" -ge 2 ] || die "--repo-root requires a value"; REPO_ROOT="$2"; shift 2 ;;
        --issue-id|--issue) [ "$#" -ge 2 ] || die "$1 requires a value"; ISSUE_ID="$2"; shift 2 ;;
        --dry-run) CONFIRM=0; shift ;;
        --confirm) CONFIRM=1; shift ;;
        --remediate) REMEDIATE=1; shift ;;
        --pr) [ "$#" -ge 2 ] || die "--pr requires a value"; PR="$2"; shift 2 ;;
        --branch) [ "$#" -ge 2 ] || die "--branch requires a value"; BRANCH="$2"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown arg: $1" ;;
    esac
done

[ -d "$REPO_ROOT" ] || die "--repo-root does not exist: $REPO_ROOT"
REPO_ROOT="$(cd "$REPO_ROOT" && pwd -P)"

if [ "$REMEDIATE" -eq 1 ]; then
    set -- --repo-root "$REPO_ROOT"
    [ "$CONFIRM" -eq 1 ] && set -- "$@" --confirm || set -- "$@" --dry-run
    set -- "$@" --remediate
    [ -n "$PR" ] && set -- "$@" --pr "$PR"
    [ -n "$BRANCH" ] && set -- "$@" --branch "$BRANCH"
    exec bash "$SCRIPT_DIR/autospec-worker-one.sh" "$@"
fi

python3 - "$REPO_ROOT" "$ISSUE_ID" "$CONFIRM" <<'PY'
import fnmatch
import json
import os
import re
import subprocess
import sys

try:
    import yaml
except Exception:
    yaml = None

repo_root = os.path.realpath(sys.argv[1])
requested_issue_id = sys.argv[2]
confirm = sys.argv[3] == "1"
autospec_dir = os.path.join(repo_root, ".autospec")
reports_dir = os.path.join(autospec_dir, "reports")
state_dir = os.path.join(autospec_dir, "state")
config_path = os.path.join(autospec_dir, "autospec.yml")
issue_plan_path = os.path.join(reports_dir, "issue-plan.json")
issue_plan_v2_path = os.path.join(reports_dir, "issue-plan-v2.json")
issue_plan_v3_path = os.path.join(reports_dir, "issue-plan-v3.json")

risk_json = os.path.join(reports_dir, "worker-risk-classification.json")
risk_md = os.path.join(reports_dir, "worker-risk-classification.md")
validation_json = os.path.join(reports_dir, "worker-validation-plan.json")
validation_md = os.path.join(reports_dir, "worker-validation-plan.md")
diff_json = os.path.join(reports_dir, "worker-diff-review.json")
diff_md = os.path.join(reports_dir, "worker-diff-review.md")
pr_body_md = os.path.join(reports_dir, "worker-pr-body.md")
stuck_md = os.path.join(reports_dir, "worker-stuck-handoff.md")
worker_result_json = os.path.join(reports_dir, "worker-result.json")
worker_result_md = os.path.join(reports_dir, "worker-result.md")
packet_md = os.path.join(state_dir, "implementation-packet.md")
packet_json = os.path.join(state_dir, "implementation-packet.json")

HIGH_RISK = [
    "auth", "authorization", "permissions", "secrets", "encryption",
    "billing", "payments", "database migration", "data deletion",
    "deployment", "infrastructure", "framework migration", "large refactor",
    "public api breaking change", "security policy", "privacy policy",
    "multi-service", "migration",
]
UNSUPPORTED = ["dependency upgrade", "major dependency", "upgrade framework", "framework upgrade"]
LOW_RISK = ["script", "validation", "report", "format", "fixture", "cli option", "parser", "helper"]
DEPENDENCY_MANIFESTS = {
    "package.json", "package-lock.json", "pnpm-lock.yaml", "yarn.lock",
    "requirements.txt", "pyproject.toml", "poetry.lock", "go.mod", "go.sum",
    "Cargo.toml", "Cargo.lock", "Gemfile", "Gemfile.lock",
}


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


def run(args):
    completed = subprocess.run(args, cwd=repo_root, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    return completed.returncode, completed.stdout, completed.stderr


def load_config():
    worker = {
        "allow_code_changes": False,
        "code_change_mode": "low_risk_only",
        "max_files_changed": 8,
        "max_code_files_changed": 4,
        "max_lines_changed": 300,
        "max_test_files_changed": 4,
        "max_new_dependencies": 0,
        "forbidden_paths": [".env", ".env.*", "**/*secret*", "**/*credential*", "**/migrations/**", ".github/workflows/**"],
        "require_tests_for_code": True,
        "require_validation": True,
    }
    project_commands = {}
    if yaml is not None and os.path.isfile(config_path):
        try:
            with open(config_path, "r", encoding="utf-8") as fh:
                data = yaml.safe_load(fh) or {}
            section = ((data.get("autonomy") or {}).get("worker") or {})
            if isinstance(section, dict):
                worker.update({key: section[key] for key in worker if key in section})
            project_commands = (((data.get("project") or {}).get("findings") or {}).get("commands") or {})
        except Exception:
            pass
    return worker, project_commands


def read_issue(issue):
    path = issue.get("draft_path", "")
    full = os.path.join(repo_root, path)
    if path and os.path.isfile(full):
        with open(full, "r", encoding="utf-8") as fh:
            return fh.read()
    return ""


def pick_issue():
    plans = [
        ("v3", issue_plan_v3_path),
        ("v2", issue_plan_v2_path),
        ("v1", issue_plan_path),
    ]
    ledger = load_json(os.path.join(state_dir, "published-issues.json"), {"issues": []})
    ledger_items = ledger.get("issues", []) if isinstance(ledger.get("issues"), list) else []
    if requested_issue_id:
        ledger_item = next((item for item in ledger_items if str(item.get("github_issue_number")) == str(requested_issue_id) or str(item.get("local_issue_id")) == str(requested_issue_id)), {})
        requested_local_id = ledger_item.get("local_issue_id", requested_issue_id)
        for version, path in plans:
            plan = load_json(path, {})
            issues = plan.get("issues", []) if isinstance(plan.get("issues"), list) else []
            for issue in issues:
                if issue.get("issue_id") == requested_local_id:
                    merged = dict(issue)
                    merged.update({key: value for key, value in ledger_item.items() if value not in (None, "", [])})
                    merged["plan_version"] = ledger_item.get("plan_version", version)
                    merged["worker_item_id"] = str(requested_issue_id)
                    return merged, len(issues)
        raise SystemExit(f"autospec-worker-v1: issue not found: {requested_issue_id}")
    for version, path in plans:
        plan = load_json(path, {})
        issues = plan.get("issues", []) if isinstance(plan.get("issues"), list) else []
        if issues:
            issue = dict(sorted(issues, key=lambda item: item.get("issue_id", ""))[0])
            issue["plan_version"] = version
            return issue, len(issues)
    raise SystemExit("autospec-worker-v1: missing issue-plan issues")


def words(issue, body):
    labels = " ".join(issue.get("suggested_labels", []))
    scope = " ".join(issue.get("implementation_scope", []))
    return f"{issue.get('title','')} {issue.get('summary','')} {labels} {scope} {body}".lower()


def base_classification(issue, body):
    structured_risk = issue.get("risk", {}) if isinstance(issue.get("risk", {}), dict) else {}
    if issue.get("plan_version") == "v3":
        if structured_risk.get("requires_architecture_review"):
            return "needs-guidance", ["structured rule requires architecture review"]
        if structured_risk.get("requires_human_review"):
            return "needs-guidance", ["structured rule requires human review"]
        if str(structured_risk.get("level", "")).lower() == "high":
            return "needs-guidance", ["structured rule risk level is high"]
        if issue.get("unsupported_check_type"):
            return "needs-guidance", ["structured rule check type is unsupported"]
    text = words(issue, body)
    title = str(issue.get("title", "")).lower()
    labels = set(issue.get("suggested_labels", []))
    if "blocked" in labels or "autospec:blocked" in labels:
        return "blocked", ["issue carries blocked label"]
    if "autospec:needs-guidance" in labels:
        return "needs-guidance", ["issue carries needs-guidance label"]
    if any(token in text for token in UNSUPPORTED):
        return "unsupported", ["dependency/framework upgrade is unsupported in worker v1"]
    if "architecture-required" in labels or "autospec:architecture" in labels and "refactor" in text:
        return "architecture-required", ["architecture label requires higher-level review"]
    if any(token in text for token in HIGH_RISK):
        return "high-risk-code", ["high-risk keyword matched"]
    if title.startswith("docs:") or "autospec:documentation" in labels:
        return "docs-only", ["documentation-only issue"]
    if title.startswith("spec:") or "spec/" in text or "docs/specs/" in text:
        return "spec-only", ["spec-only issue"]
    if "metadata" in text or "autospec:metadata" in labels:
        return "metadata-only", ["metadata issue"]
    if title.startswith("test:") or "autospec:testing" in labels:
        return "test-only", ["test-only issue"]
    if any(token in text for token in LOW_RISK):
        return "low-risk-code", ["low-risk helper keyword matched"]
    if issue.get("risk") == "High":
        return "medium-risk-code", ["issue plan risk is high but no hard blocker matched"]
    return "needs-guidance", ["worker v1 could not prove this is low-risk"]


def extract_paths(text):
    found = []
    patterns = [
        r"(?<![\w./-])(?:scripts|tests|docs|skills|schemas|src|lib)/[A-Za-z0-9_./:-]+",
        r"(?<![\w./-])\.autospec/[A-Za-z0-9_./:-]+",
    ]
    for pattern in patterns:
        for match in re.findall(pattern, text):
            found.append(match.rstrip(".,);]"))
    return sorted(dict.fromkeys(found))


def test_paths(paths, text):
    candidates = [path for path in paths if path.startswith("tests/") or path.endswith(".bats") or "/test_" in path or path.endswith(".test.js")]
    if "bash scripts/validate.sh" in text:
        candidates.append("bash scripts/validate.sh")
    return sorted(dict.fromkeys(candidates))


def git_changed_files():
    code, stdout, _ = run(["git", "status", "--porcelain"])
    if code != 0:
        return []
    files = []
    for line in stdout.splitlines():
        path = line[3:] if len(line) > 3 else ""
        if " -> " in path:
            path = path.split(" -> ", 1)[1]
        if path:
            files.append(path)
    return sorted(dict.fromkeys(files))


def git_numstat():
    entries = {}
    code, stdout, _ = run(["git", "diff", "--numstat"])
    if code == 0:
        for line in stdout.splitlines():
            parts = line.split("\t")
            if len(parts) >= 3:
                add = 0 if parts[0] == "-" else int(parts[0] or 0)
                delete = 0 if parts[1] == "-" else int(parts[1] or 0)
                entries[parts[2]] = {"added": add, "removed": delete}
    for path in git_changed_files():
        if path not in entries and os.path.isfile(os.path.join(repo_root, path)):
            try:
                with open(os.path.join(repo_root, path), "r", encoding="utf-8", errors="ignore") as fh:
                    entries[path] = {"added": sum(1 for _ in fh), "removed": 0}
            except OSError:
                entries[path] = {"added": 0, "removed": 0}
    return entries


def is_test(path):
    return path.startswith("tests/") or path.endswith(".bats") or "/test_" in path or path.endswith(".test.js")


def is_doc_or_metadata(path):
    return path.startswith("docs/") or path.startswith(".autospec/") or path.endswith(".md")


def forbidden_matches(path, patterns):
    hits = []
    for pattern in patterns:
        if fnmatch.fnmatch(path, pattern) or fnmatch.fnmatch("/" + path, pattern):
            hits.append(pattern)
    return hits


def validation_plan(test_candidates, commands):
    focused = []
    for path in test_candidates:
        if path == "bash scripts/validate.sh":
            continue
        if path.endswith(".bats"):
            focused.append(f"bats {path}")
    full = []
    if commands.get("test"):
        full.append(str(commands["test"]))
    elif os.path.isfile(os.path.join(repo_root, "scripts", "validate.sh")):
        full.append("bash scripts/validate.sh")
    skipped = []
    if not focused:
        skipped.append("No focused validation command was inferable from the issue.")
    return {"focused_validation": focused, "full_validation": full, "skipped_validation": skipped, "validation_failures": []}


def stuck_text(issue, classification, reasons, required="worker v2 or human implementation"):
    safer = "Split this into a smaller issue that changes one helper plus one focused test."
    return "\n".join([
        f"# bot stuck: {issue.get('title','issue')}",
        "",
        "## Why worker v1 refused this issue",
        "",
        "\n".join(f"- {reason}" for reason in reasons) or "- No safe reason was available.",
        "",
        "## Required capability level",
        "",
        required,
        "",
        "## Suggested split",
        "",
        "- Separate high-risk architecture/security/data work from small helper or test changes.",
        "",
        "## Safer first issue",
        "",
        safer,
        "",
        "## Human decision needed",
        "",
        "- Confirm whether this should be split, deferred, or assigned to a higher-capability worker.",
        "",
        "## Resume criteria",
        "",
        "- [ ] guidance provided in a comment or issue update",
        "- [ ] `autospec:guidance-provided` or `autospec:resume` applied",
        "- [ ] blocker resolved",
    ])


worker_config, project_commands = load_config()
issue, issue_count = pick_issue()
structured_policy = issue.get("plan_version") == "v3"
body = read_issue(issue)
text = words(issue, body)
expected_files = extract_paths(text)
test_candidates = test_paths(expected_files, text)
classification, reasons = base_classification(issue, body)
code_like = classification in {"low-risk-code", "medium-risk-code", "high-risk-code", "architecture-required", "unsupported", "needs-guidance"}
eligible = classification == "low-risk-code" and bool(worker_config.get("allow_code_changes")) and worker_config.get("code_change_mode") == "low_risk_only"
stuck_reasons = []

if classification == "low-risk-code" and not worker_config.get("allow_code_changes"):
    classification = "needs-guidance"
    eligible = False
    stuck_reasons.append("allow_code_changes is false; worker v1 code mode is disabled by default.")
elif classification == "low-risk-code" and worker_config.get("require_tests_for_code") and not test_candidates:
    classification = "needs-guidance"
    eligible = False
    stuck_reasons.append("No focused test path, validation script, golden fixture, or snapshot/report fixture was inferable.")
elif classification != "low-risk-code" and code_like and classification not in {"docs-only", "spec-only", "metadata-only", "test-only"}:
    stuck_reasons.extend(reasons)

if structured_policy:
    structured_risk = issue.get("risk", {}) if isinstance(issue.get("risk", {}), dict) else {}
    if str(structured_risk.get("level", "")).lower() == "high":
        classification = "needs-guidance"
        eligible = False
        stuck_reasons.append("Structured rule risk.level is high; worker v1 requires stuck/guidance.")
    if structured_risk.get("requires_human_review"):
        classification = "needs-guidance"
        eligible = False
        stuck_reasons.append("Structured rule requires human review.")
    if structured_risk.get("requires_architecture_review"):
        classification = "needs-guidance"
        eligible = False
        stuck_reasons.append("Structured rule requires architecture review.")

plan = validation_plan(test_candidates, project_commands)
numstat = git_numstat()
changed_files = sorted(numstat.keys())
total_added = sum(item["added"] for item in numstat.values())
total_removed = sum(item["removed"] for item in numstat.values())
code_files = [path for path in changed_files if not is_test(path) and not is_doc_or_metadata(path)]
test_files = [path for path in changed_files if is_test(path)]
dependency_files = [path for path in changed_files if os.path.basename(path) in DEPENDENCY_MANIFESTS]
forbidden = [{"path": path, "patterns": forbidden_matches(path, worker_config.get("forbidden_paths", []))} for path in changed_files]
forbidden = [item for item in forbidden if item["patterns"]]
budget_failures = []
if len(changed_files) > int(worker_config.get("max_files_changed", 8)):
    budget_failures.append("max_files_changed exceeded")
if len(code_files) > int(worker_config.get("max_code_files_changed", 4)):
    budget_failures.append("max_code_files_changed exceeded")
if total_added + total_removed > int(worker_config.get("max_lines_changed", 300)):
    budget_failures.append("max_lines_changed patch budget exceeded")
if len(test_files) > int(worker_config.get("max_test_files_changed", 4)):
    budget_failures.append("max_test_files_changed exceeded")
if dependency_files and int(worker_config.get("max_new_dependencies", 0)) == 0:
    budget_failures.append("max_new_dependencies exceeded: " + ", ".join(dependency_files))
if forbidden:
    budget_failures.append("forbidden path check failed: " + ", ".join(item["path"] for item in forbidden))
material_mismatch = bool(expected_files and changed_files and not set(changed_files).issubset(set(expected_files)))
if material_mismatch and classification == "low-risk-code":
    budget_failures.append("planned files and actual files differ materially")

pr_allowed = eligible and not budget_failures and not forbidden and (not worker_config.get("require_tests_for_code") or bool(test_candidates))
if budget_failures:
    stuck_reasons.extend(budget_failures)

risk_report = {
    "version": 1,
    "mode": "confirm" if confirm else "dry_run",
    "processed_issue_id": issue.get("issue_id"),
    "processed_issue_count": 1,
    "available_issue_count": issue_count,
    "classification": classification,
    "classification_reasons": reasons + stuck_reasons,
    "code_change_eligible": bool(eligible),
    "worker_config": worker_config,
    "structured_policy_context": {
        "plan_version": issue.get("plan_version", "legacy"),
        "rule_ids": issue.get("source_rule_ids") or issue.get("rule_ids") or [],
        "quality_gate_ids": issue.get("quality_gate_ids") or [re.sub(r"[^a-z0-9_.-]+", "_", str(g).lower()).strip("_") for g in issue.get("quality_gates", [])],
        "category": issue.get("category", ""),
        "severity": issue.get("rule_severity") or issue.get("severity", ""),
        "maturity_target": issue.get("maturity_level", ""),
        "source_doctrine": issue.get("source_doctrine", ""),
        "source_baseline_pack": issue.get("source_baseline_pack", ""),
        "source_policy_file": issue.get("source_file", ""),
        "risk": issue.get("risk", {}),
    },
}
write_json(risk_json, risk_report)
write_text(risk_md, "\n".join([
    "# Worker Risk Classification",
    "",
    f"- Issue: `{issue.get('issue_id')}`",
    f"- Title: {issue.get('title')}",
    f"- Classification: **{classification}**",
    f"- Code-change eligible: `{str(bool(eligible)).lower()}`",
    "",
    "## Reasons",
    "",
    "\n".join(f"- {reason}" for reason in risk_report["classification_reasons"]) or "- None.",
]))

write_json(validation_json, {"version": 1, **plan})
write_text(validation_md, "\n".join([
    "# Worker Validation Plan",
    "",
    "## Focused validation",
    "",
    "\n".join(f"- `{cmd}`" for cmd in plan["focused_validation"]) or "- None.",
    "",
    "## Full validation",
    "",
    "\n".join(f"- `{cmd}`" for cmd in plan["full_validation"]) or "- None.",
    "",
    "## Skipped validation",
    "",
    "\n".join(f"- {item}" for item in plan["skipped_validation"]) or "- None.",
]))

diff_report = {
    "version": 1,
    "files_changed": [{"path": path, **numstat[path]} for path in changed_files],
    "lines_added": total_added,
    "lines_removed": total_removed,
    "forbidden_path_check": {"passed": not forbidden, "matches": forbidden},
    "patch_budget": {"passed": not budget_failures, "failures": budget_failures, "config": worker_config},
    "test_docs_metadata_change_check": {"test_files": test_files, "code_files": code_files, "expected_files": expected_files},
    "risk_change": {"planned": classification, "actual": "needs-guidance" if budget_failures else classification},
    "material_file_mismatch": material_mismatch,
    "pr_creation_allowed": bool(pr_allowed),
}
write_json(diff_json, diff_report)
write_text(diff_md, "\n".join([
    "# Worker Diff Review",
    "",
    f"- Files changed: {len(changed_files)}",
    f"- Lines added: {total_added}",
    f"- Lines removed: {total_removed}",
    f"- Forbidden path check: {'pass' if not forbidden else 'fail'}",
    f"- Patch budget result: {'pass' if not budget_failures else 'fail'}",
    f"- PR creation allowed: `{str(bool(pr_allowed)).lower()}`",
    "",
    "## Files",
    "",
    "\n".join(f"- `{item['path']}` (+{item['added']} -{item['removed']})" for item in diff_report["files_changed"]) or "- No local diff detected.",
]))

structured_context = risk_report["structured_policy_context"]
packet = "\n".join([
    f"# Implementation packet: {issue.get('title')}",
    "",
    "## Structured Policy Context",
    "",
    f"- Plan version: `{structured_context['plan_version']}`",
    f"- Category: `{structured_context['category'] or 'unknown'}`",
    f"- Severity: `{structured_context['severity'] or 'unknown'}`",
    f"- Maturity target: `{structured_context['maturity_target'] or 'unknown'}`",
    "",
    "## Rule IDs",
    "",
    "\n".join(f"- `{rid}`" for rid in structured_context["rule_ids"]) or "- None.",
    "",
    "## Source Doctrine",
    "",
    structured_context["source_doctrine"] or "n/a",
    "",
    "## Source Baseline Pack",
    "",
    structured_context["source_baseline_pack"] or "n/a",
    "",
    "## Quality Gates",
    "",
    "\n".join(f"- `{gid}`" for gid in structured_context["quality_gate_ids"]) or "- None.",
    "",
    "## Rule Check Evidence",
    "",
    "\n".join(f"- {item}" for item in issue.get("evidence", [])) or "- None.",
    "",
    "## Missing Evidence",
    "",
    "\n".join(f"- {item}" for item in issue.get("missing_evidence", [])) or "- None.",
    "",
    "## Maturity Target",
    "",
    structured_context["maturity_target"] or "unknown",
    "",
    "## Structured Acceptance Criteria",
    "",
    "\n".join(f"- [ ] {item}" for item in issue.get("acceptance_criteria", [])) or "- None.",
    "",
    "## Policy-Derived Validation Expectations",
    "",
    "\n".join(f"- `{item}`" for item in issue.get("validation_expectations", [])) or "- None.",
    "",
    "## Risk classification",
    "",
    classification,
    "",
    "## Code-change eligibility",
    "",
    f"Eligible: `{str(bool(eligible)).lower()}`",
    "",
    "## Patch budget",
    "",
    f"- Max files changed: {worker_config.get('max_files_changed')}",
    f"- Max code files changed: {worker_config.get('max_code_files_changed')}",
    f"- Max lines changed: {worker_config.get('max_lines_changed')}",
    "",
    "## Test-first plan",
    "",
    "\n".join(f"- Update or add `{path}` before implementation." for path in test_candidates) or "- No focused test path inferred.",
    "",
    "## Expected files",
    "",
    "\n".join(f"- `{path}`" for path in expected_files) or "- Not specified.",
    "",
    "## Forbidden files",
    "",
    "\n".join(f"- `{path}`" for path in worker_config.get("forbidden_paths", [])),
    "",
    "## Validation plan",
    "",
    "\n".join(f"- `{cmd}`" for cmd in (plan["focused_validation"] + plan["full_validation"])) or "- None.",
    "",
    "## Rollback plan",
    "",
    "- Revert the worker branch or discard the local diff before publishing.",
    "",
    "## Stuck criteria",
    "",
    "- Risk classification is not `low-risk-code` for code work.",
    "- Patch budget or forbidden path checks fail.",
    "- No focused test or validation path is inferable.",
])
write_text(packet_md, packet)
write_json(packet_json, {
    "version": 2,
    "issue_id": issue.get("issue_id"),
    "title": issue.get("title"),
    "classification": classification,
    "structured_policy_context": {
        **structured_context,
        "rule_check_evidence": issue.get("evidence", []),
        "missing_evidence": issue.get("missing_evidence", []),
        "acceptance_criteria": issue.get("acceptance_criteria", []),
        "validation_expectations": issue.get("validation_expectations", []),
        "metadata_expectations": issue.get("metadata_expectations", []),
        "remediation_hint": issue.get("remediation_hint", ""),
    },
    "code_change_eligible": bool(eligible),
})
work_item_id = str(issue.get("worker_item_id") or issue.get("github_issue_number") or issue.get("issue_id") or "unknown")
work_item_dir = os.path.join(state_dir, "work-items", work_item_id)
write_text(os.path.join(work_item_dir, "implementation-packet.md"), packet)
write_json(os.path.join(work_item_dir, "implementation-packet.json"), load_json(packet_json, {}))

pr_body = "\n".join([
    f"# {issue.get('title')}",
    "",
    "## Risk classification",
    "",
    f"- Classification: `{classification}`",
    f"- Reason change is low risk: {'yes' if classification == 'low-risk-code' else 'no'}",
    "",
    "## Patch budget",
    "",
    f"- Result: `{'pass' if not budget_failures else 'fail'}`",
    f"- Files changed: {len(changed_files)}",
    f"- Lines changed: {total_added + total_removed}",
    "",
    "## Test-first plan",
    "",
    "\n".join(f"- `{path}`" for path in test_candidates) or "- No focused test path inferred.",
    "",
    "## Validation evidence",
    "",
    "\n".join(f"- Focused: `{cmd}`" for cmd in plan["focused_validation"]) or "- Focused: none.",
    "\n".join(f"- Full: `{cmd}`" for cmd in plan["full_validation"]) or "- Full: none.",
    "",
    "## Diff safety review",
    "",
    f"- Forbidden paths: {'none' if not forbidden else ', '.join(item['path'] for item in forbidden)}",
    f"- PR creation allowed: `{str(bool(pr_allowed)).lower()}`",
    "",
    "## Metadata/docs impact",
    "",
    "- Worker reports under `.autospec/reports` and `.autospec/state` are expected to change.",
    "",
    "## Stuck/follow-up criteria",
    "",
    "- Escalate if validation fails, diff scope widens, or review finds risk above low-risk code.",
    "",
    "## Files changed",
    "",
    "| File | Added | Removed |",
    "| --- | ---: | ---: |",
    "\n".join(f"| `{item['path']}` | {item['added']} | {item['removed']} |" for item in diff_report["files_changed"]) or "| none | 0 | 0 |",
])
write_text(pr_body_md, pr_body)

if stuck_reasons or not pr_allowed and classification in {"needs-guidance", "high-risk-code", "unsupported", "medium-risk-code", "architecture-required"}:
    write_text(stuck_md, stuck_text(issue, classification, stuck_reasons or reasons))
    write_text(os.path.join(work_item_dir, "stuck-handoff.md"), stuck_text(issue, classification, stuck_reasons or reasons))
elif budget_failures:
    write_text(stuck_md, stuck_text(issue, classification, budget_failures))
    write_text(os.path.join(work_item_dir, "stuck-handoff.md"), stuck_text(issue, classification, budget_failures))

next_command = f"bash scripts/autospec-verify-worker-pr.sh --dry-run --work-item .autospec/state"
worker_result = {
    "version": 1,
    "issue_id": issue.get("issue_id"),
    "classification": classification,
    "pr_creation_allowed": bool(pr_allowed),
    "reports": {
        "risk_classification": ".autospec/reports/worker-risk-classification.json",
        "validation_plan": ".autospec/reports/worker-validation-plan.json",
        "diff_review": ".autospec/reports/worker-diff-review.json",
        "pr_body": ".autospec/reports/worker-pr-body.md",
    },
    "next_recommended_command": next_command,
}
write_json(worker_result_json, worker_result)
write_text(worker_result_md, "\n".join([
    "# Worker Result",
    "",
    f"- Issue: `{issue.get('issue_id')}`",
    f"- Classification: `{classification}`",
    f"- PR creation allowed: `{str(bool(pr_allowed)).lower()}`",
    "",
    "## Next Recommended Command",
    "",
    f"```bash\n{next_command}\n```",
]))

print("worker v1: PASS")
print(f"issue: {issue.get('issue_id')}")
print(f"classification: {classification}")
print("mode: confirm" if confirm else "mode: dry-run")
if structured_policy and classification == "needs-guidance":
    sys.exit(1)
PY
