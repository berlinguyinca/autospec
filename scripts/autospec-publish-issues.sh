#!/usr/bin/env bash
# scripts/autospec-publish-issues.sh — publish local issue drafts to GitHub.
#
# Dry-run by default. Real GitHub writes require --confirm. Maintains
# .autospec/state/published-issues.json to avoid duplicate GitHub issues.

set -eu

usage() {
    cat <<'EOF'
Usage:
  autospec-publish-issues.sh [--repo-root <dir>] [--dry-run|--confirm] [--repo OWNER/REPO] [--reopen] [--plan v1|v2|v3]

Inputs:
  .autospec/reports/issue-plan.json
  .autospec/backlog/issues/*.md
  .autospec/state/control-labels.yml
  .autospec/autospec.yml

Writes:
  .autospec/state/published-issues.json                 (--confirm only)
  .autospec/reports/github-issue-publish-plan.json       (dry-run)
  .autospec/reports/github-issue-publish-plan.md         (dry-run)
  .autospec/reports/github-issue-publish-result.json     (--confirm)
  .autospec/reports/github-issue-publish-result.md       (--confirm)
EOF
}

die() {
    printf 'autospec-publish-issues: %s\n' "$*" >&2
    exit 2
}

REPO_ROOT="$(pwd)"
CONFIRM=0
MODE_SET=0
REOPEN=0
GH_REPO=""
PLAN_VERSION=""
while [ "$#" -gt 0 ]; do
    case "$1" in
        --repo-root) [ "$#" -ge 2 ] || die "--repo-root requires a value"; REPO_ROOT="$2"; shift 2 ;;
        --dry-run) CONFIRM=0; MODE_SET=1; shift ;;
        --confirm) CONFIRM=1; MODE_SET=1; shift ;;
        --repo) [ "$#" -ge 2 ] || die "--repo requires OWNER/REPO"; GH_REPO="$2"; shift 2 ;;
        --reopen) REOPEN=1; shift ;;
        --plan) [ "$#" -ge 2 ] || die "--plan requires v1, v2, or v3"; PLAN_VERSION="$2"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown arg: $1" ;;
    esac
done

[ -d "$REPO_ROOT" ] || die "--repo-root does not exist: $REPO_ROOT"
REPO_ROOT="$(cd "$REPO_ROOT" && pwd -P)"

python3 - "$REPO_ROOT" "$CONFIRM" "$MODE_SET" "$GH_REPO" "$REOPEN" "$PLAN_VERSION" <<'PY'
import datetime
import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile

try:
    import yaml
except Exception:
    yaml = None

repo_root = os.path.realpath(sys.argv[1])
confirm = sys.argv[2] == "1"
mode_set = sys.argv[3] == "1"
gh_repo = sys.argv[4]
reopen_closed = sys.argv[5] == "1"
requested_plan_version = sys.argv[6]
reports_dir = os.path.join(repo_root, ".autospec", "reports")
state_dir = os.path.join(repo_root, ".autospec", "state")
config_path = os.path.join(repo_root, ".autospec", "autospec.yml")
labels_path = os.path.join(state_dir, "control-labels.yml")
ledger_path = os.path.join(state_dir, "published-issues.json")


def select_plan():
    candidates = {
        "v3": (os.path.join(reports_dir, "issue-plan-v3.json"), os.path.join(repo_root, ".autospec", "backlog", "issues-v3")),
        "v2": (os.path.join(reports_dir, "issue-plan-v2.json"), os.path.join(repo_root, ".autospec", "backlog", "issues-v2")),
        "v1": (os.path.join(reports_dir, "issue-plan.json"), os.path.join(repo_root, ".autospec", "backlog", "issues")),
    }
    if requested_plan_version:
        if requested_plan_version not in candidates:
            raise SystemExit("autospec-publish-issues: --plan must be v1, v2, or v3")
        return requested_plan_version, candidates[requested_plan_version]
    for version in ["v3", "v2", "v1"]:
        if os.path.isfile(candidates[version][0]):
            return version, candidates[version]
    return "v1", candidates["v1"]


plan_version, (issue_plan_path, backlog_dir) = select_plan()
suffix = "-v3" if plan_version == "v3" else ""
json_report = os.path.join(reports_dir, f"github-issue-publish-{'result' if confirm else 'plan'}{suffix}.json")
md_report = os.path.join(reports_dir, f"github-issue-publish-{'result' if confirm else 'plan'}{suffix}.md")


def now_iso():
    return datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


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


def load_config():
    defaults = {
        "enabled": True,
        "default_mode": "dry_run",
        "require_confirm": True,
        "create_missing_labels": True,
        "reopen_closed_autospec_issues": False,
        "apply_labels": True,
    }
    if yaml is None or not os.path.isfile(config_path):
        return defaults
    try:
        with open(config_path, "r", encoding="utf-8") as fh:
            data = yaml.safe_load(fh) or {}
        section = (((data.get("github") or {}).get("issue_publishing")) or {})
        if isinstance(section, dict):
            defaults.update({key: section[key] for key in defaults if key in section})
    except Exception:
        pass
    return defaults


config = load_config()
if not mode_set:
    confirm = str(config.get("default_mode", "dry_run")).lower() == "confirm" and not config.get("require_confirm", True)
if not reopen_closed:
    reopen_closed = bool(config.get("reopen_closed_autospec_issues", False))
apply_config_labels = bool(config.get("apply_labels", True))


def sha256(text):
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def gh_base():
    base = ["gh"]
    if gh_repo:
        base.extend(["--repo", gh_repo])
    return base


def run_gh(args):
    cmd = gh_base() + args
    completed = subprocess.run(cmd, cwd=repo_root, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    if completed.returncode != 0:
        raise RuntimeError(f"{' '.join(cmd)} failed: {completed.stderr.strip() or completed.stdout.strip()}")
    return completed.stdout.strip()


def parse_json_list(text):
    try:
        data = json.loads(text or "[]")
        return data if isinstance(data, list) else []
    except Exception:
        return []


def parse_json_object(text):
    try:
        data = json.loads(text or "{}")
        return data if isinstance(data, dict) else {}
    except Exception:
        return {}


def issue_number_from_url(url):
    match = re.search(r"/issues/([0-9]+)", url or "")
    return int(match.group(1)) if match else None


def source_gap_hash(issue):
    payload = {
        "source_gap": issue.get("source_gap", {}),
        "rule_ids": issue.get("source_rule_ids") or issue.get("rule_ids") or [],
        "missing_evidence": issue.get("missing_evidence", []),
        "status": issue.get("status", ""),
    }
    return sha256(json.dumps(payload, sort_keys=True))


def rule_result_hash(issue):
    payload = {
        "rule_ids": issue.get("source_rule_ids") or issue.get("rule_ids") or [],
        "evidence": issue.get("evidence", []),
        "missing_evidence": issue.get("missing_evidence", []),
        "acceptance_criteria": issue.get("acceptance_criteria", []),
        "quality_gates": issue.get("quality_gates", []),
    }
    return sha256(json.dumps(payload, sort_keys=True))


def body_for_issue(issue):
    draft_path = os.path.join(repo_root, issue.get("draft_path", ""))
    with open(draft_path, "r", encoding="utf-8") as fh:
        draft_body = fh.read()
    source_hash = source_gap_hash(issue)
    markers = []
    if plan_version == "v3":
        rule_ids = issue.get("source_rule_ids") or issue.get("rule_ids") or []
        markers.append("<!-- autospec-plan-version: v3 -->")
        markers.append(f"<!-- autospec-local-issue-id: {issue['issue_id']} -->")
        if len(rule_ids) == 1:
            markers.append(f"<!-- autospec-rule-id: {rule_ids[0]} -->")
        if rule_ids:
            markers.append(f"<!-- autospec-rule-ids: {','.join(rule_ids)} -->")
        markers.append(f"<!-- autospec-rule-result-hash: {rule_result_hash(issue)} -->")
        markers.append(f"<!-- autospec-source-gap-hash: {source_hash} -->")
        markers.append("<!-- autospec-body-hash: __AUTOSPEC_BODY_HASH__ -->")
    else:
        markers = [
            f"<!-- autospec-local-issue-id: {issue['issue_id']} -->",
            f"<!-- autospec-source-gap-hash: {source_hash} -->",
            "<!-- autospec-body-hash: __AUTOSPEC_BODY_HASH__ -->",
        ]
    base = "\n".join(markers + ["", draft_body])
    body_hash = sha256(base.replace("<!-- autospec-body-hash: __AUTOSPEC_BODY_HASH__ -->", ""))
    body = base.replace("<!-- autospec-body-hash: __AUTOSPEC_BODY_HASH__ -->", f"<!-- autospec-body-hash: {body_hash} -->")
    return body, source_hash, body_hash


def write_temp_body(body):
    fd, path = tempfile.mkstemp(prefix="autospec-issue-", suffix=".md")
    with os.fdopen(fd, "w", encoding="utf-8") as fh:
        fh.write(body)
    return path


def labels_for_issue(issue):
    labels = list(issue.get("suggested_labels", []))
    labels.extend(["autospec:managed", "autospec:discovered"])
    risk = issue.get("risk", {})
    risk_level = risk.get("level") if isinstance(risk, dict) else issue.get("risk")
    if str(risk_level).lower() == "high":
        labels.append("autospec:risk-high")
    if str(risk_level).lower() == "high" or "architecture" in str(issue.get("feature_family", "")).lower():
        labels.append("autospec:architecture")
    return sorted(dict.fromkeys(labels))


def known_control_labels():
    if not os.path.isfile(labels_path):
        return set()
    labels = set()
    try:
        with open(labels_path, "r", encoding="utf-8") as fh:
            for line in fh:
                stripped = line.strip()
                if stripped.startswith("autospec:") and stripped.endswith(":"):
                    labels.add(stripped[:-1])
    except OSError:
        return set()
    return labels


def normalize_state(value):
    return str(value or "open").lower()


def normalize_labels(value):
    labels = []
    for item in value or []:
        if isinstance(item, dict):
            name = item.get("name")
        else:
            name = item
        if name:
            labels.append(str(name))
    return sorted(dict.fromkeys(labels))


def load_ledger():
    raw = load_json(ledger_path, {})
    issues = []
    if isinstance(raw.get("issues"), list):
        for item in raw["issues"]:
            if isinstance(item, dict) and item.get("local_issue_id"):
                issues.append(item)
    elif isinstance(raw.get("items"), dict):
        for local_id, item in raw["items"].items():
            if isinstance(item, dict):
                issues.append({
                    "local_issue_id": local_id,
                    "title": item.get("title", ""),
                    "source_gap_hash": item.get("source_gap_hash", ""),
                    "body_hash": item.get("body_hash", ""),
                    "labels": item.get("labels", []),
                    "github_issue_number": item.get("github_number"),
                    "github_issue_url": item.get("github_url", ""),
                    "state": item.get("state", "open"),
                })
    return {"schema": 1, "repo": raw.get("repo") or gh_repo, "issues": issues}


def ledger_index(ledger):
    return {item.get("local_issue_id"): item for item in ledger.get("issues", [])}


def upsert_ledger_issue(ledger, entry):
    indexed = ledger_index(ledger)
    existing = indexed.get(entry["local_issue_id"])
    if existing is None:
        ledger.setdefault("issues", []).append(entry)
    else:
        existing.update(entry)
    ledger["issues"] = sorted(ledger.get("issues", []), key=lambda item: item.get("local_issue_id", ""))


def github_issue_state(number):
    if not number:
        return {}
    output = run_gh(["issue", "view", str(number), "--json", "number,url,title,state,labels"])
    return parse_json_object(output)


def find_existing_issue_by_marker(issue_id):
    output = run_gh(["issue", "list", "--search", f"autospec-local-issue-id: {issue_id}", "--state", "open", "--json", "number,url,title,state", "--limit", "1"])
    data = parse_json_list(output)
    return data[0] if data else None


def find_existing_issue_by_rule(rule_ids):
    for rid in rule_ids:
        output = run_gh(["issue", "list", "--search", f"autospec-rule-id: {rid}", "--state", "open", "--json", "number,url,title,state", "--limit", "1"])
        data = parse_json_list(output)
        if data:
            return data[0]
    return None


def find_existing_issue_by_title(title):
    output = run_gh(["issue", "list", "--search", title, "--state", "open", "--json", "number,url,title,state", "--limit", "20"])
    for item in parse_json_list(output):
        if item.get("title") == title:
            return item
    return None


def apply_labels(github_number, labels):
    failures = []
    if not apply_config_labels:
        return failures
    for label in labels:
        try:
            run_gh(["issue", "edit", str(github_number), "--add-label", label])
        except Exception as exc:
            failures.append(f"{label}: {exc}")
    return failures


def permission_hint(text):
    lowered = text.lower()
    if any(token in lowered for token in ["permission", "403", "resource not accessible", "forbidden", "not authorized"]):
        return "Check GitHub issue permissions for the token/app/CLI, then rerun with --confirm."
    return None


def summarize(actions):
    created = sum(1 for item in actions if item["action"] == "created")
    updated = sum(1 for item in actions if item["action"] in ["updated", "reopened_updated"])
    return {
        "local_issue_drafts": len(actions),
        "already_published": sum(1 for item in actions if item["action"] in ["unchanged", "would_update", "updated", "skipped_closed", "reopened_updated"]),
        "to_create": sum(1 for item in actions if item["action"] == "would_create"),
        "to_update": sum(1 for item in actions if item["action"] == "would_update"),
        "created": created,
        "updated": updated,
        "github_issues_created": bool(confirm and created),
        "github_issues_updated": bool(confirm and updated),
        "skipped": sum(1 for item in actions if item["action"].startswith("skipped")),
        "missing_labels": sorted({failure.split(":", 1)[0] for item in actions for failure in item.get("label_failures", [])}),
        "permission_auth_problems": sum(1 for item in actions for error in item.get("errors", []) if permission_hint(error)),
        "high_risk_issues": sum(1 for item in actions if "autospec:risk-high" in item.get("labels", [])),
        "architecture_gated_issues": sum(1 for item in actions if "autospec:architecture" in item.get("labels", [])),
        "next_recommended_command": "bash scripts/autospec-sync-published-issues.sh" if confirm else "bash scripts/autospec-publish-issues.sh --confirm",
    }


def write_reports(mode, status, actions, errors, warnings):
    summary = summarize(actions)
    report = {
        "version": 1,
        "plan_version": plan_version,
        "mode": mode,
        "status": status,
        "config": config,
        "summary": summary,
        "actions": actions,
        "errors": errors,
        "warnings": warnings,
        "ledger_path": ".autospec/state/published-issues.json",
        "side_effects": {
            "github_api_calls": bool(confirm),
            "github_issues_created": bool(confirm and any(item["action"] == "created" for item in actions)),
            "github_issues_updated": bool(confirm and any(item["action"] in ["updated", "reopened_updated"] for item in actions)),
            "branches_created": False,
            "prs_created": False,
            "implementation_started": False,
        },
    }
    write_json(json_report, report)
    lines = [
        "# GitHub Issue Publishing",
        "",
        f"Status: **{status.upper()}**",
        f"Mode: `{mode}`",
        f"Plan version: `{plan_version}`",
        "",
        "## Summary",
        "",
        "| Metric | Count |",
        "| --- | ---: |",
        f"| Local issue drafts | {summary['local_issue_drafts']} |",
        f"| Already published | {summary['already_published']} |",
        f"| To create | {summary['to_create']} |",
        f"| To update | {summary['to_update']} |",
        f"| Skipped | {summary['skipped']} |",
        f"| Missing labels | {len(summary['missing_labels'])} |",
        f"| Permission/auth problems | {summary['permission_auth_problems']} |",
        f"| High-risk issues | {summary['high_risk_issues']} |",
        f"| Architecture-gated issues | {summary['architecture_gated_issues']} |",
        "",
        "## Issue Actions",
        "",
        "| Local issue | Action | GitHub issue | Labels failed | Title |",
        "| --- | --- | --- | --- | --- |",
    ]
    for item in actions:
        gh_issue = item.get("github_issue_url") or item.get("github_issue_number") or ""
        label_failures = "<br>".join(item.get("label_failures", [])) or "none"
        lines.append(f"| `{item['local_issue_id']}` | {item['action']} | {gh_issue} | {label_failures} | {item['title']} |")
    if plan_version == "v3":
        lines.extend(["", "## V3 Idempotency Marker Preview", ""])
        for item in actions:
            for marker in item.get("markers", []):
                lines.append(f"- `{item['local_issue_id']}` {marker}")
    if warnings:
        lines.extend(["", "## Warnings", ""])
        for warning in warnings:
            lines.append(f"- {warning}")
    if errors:
        lines.extend(["", "## Errors", ""])
        for error in errors:
            lines.append(f"- {error}")
            hint = permission_hint(error)
            if hint:
                lines.append(f"  - {hint}")
    lines.extend([
        "",
        "## Next Command",
        "",
        f"```bash\n{summary['next_recommended_command']}\n```",
        "",
        "## Safety",
        "",
        "- Branches created: false",
        "- PRs created: false",
        "- Implementation started: false",
    ])
    with open(md_report, "w", encoding="utf-8") as fh:
        fh.write("\n".join(lines))
        fh.write("\n")


plan = load_json(issue_plan_path, {})
issues = plan.get("issues", []) if isinstance(plan.get("issues"), list) else []
ledger = load_ledger()
ledger["repo"] = gh_repo or ledger.get("repo") or ""
actions = []
errors = []
warnings = []
known_labels = known_control_labels()

if not issues:
    error = f"missing {plan_version} issue plan entries; run the matching plan command before publishing"
    write_reports("confirm" if confirm else "dry_run", "error", [], [error], [])
    print(f"issue publishing: ERROR - {error}")
    sys.exit(2)

for issue in issues:
    issue_id = issue.get("issue_id")
    title = issue.get("title", "")
    body, source_hash, body_hash = body_for_issue(issue)
    rule_ids = issue.get("source_rule_ids") or issue.get("rule_ids") or []
    quality_gate_ids = issue.get("quality_gate_ids") or [re.sub(r"[^a-z0-9_.-]+", "_", str(g).lower()).strip("_") for g in issue.get("quality_gates", [])]
    source_policy_files = [x for x in [issue.get("source_file")] if x]
    labels = labels_for_issue(issue)
    available_labels = [label for label in labels if not label.startswith("autospec:") or not known_labels or label in known_labels]
    unavailable_labels = [label for label in labels if label.startswith("autospec:") and known_labels and label not in known_labels]
    label_failures = [f"{label}: missing from .autospec/state/control-labels.yml" for label in unavailable_labels]
    indexed = ledger_index(ledger)
    ledger_item = indexed.get(issue_id)
    github_number = ledger_item.get("github_issue_number") if ledger_item else None
    github_url = ledger_item.get("github_issue_url") if ledger_item else None
    action = "would_update" if ledger_item else "would_create"
    item_errors = []

    if confirm:
        body_path = write_temp_body(body)
        try:
            remote = github_issue_state(github_number) if github_number else {}
            if remote:
                github_url = remote.get("url") or github_url
                remote_state = normalize_state(remote.get("state"))
            else:
                remote_state = normalize_state(ledger_item.get("state")) if ledger_item else "open"

            if github_number and remote_state == "closed" and not reopen_closed:
                action = "skipped_closed"
                warning = f"{issue_id}: GitHub issue {github_number} is closed; skipped. Pass --reopen to reopen and update."
                warnings.append(warning)
            else:
                if github_number and remote_state == "closed" and reopen_closed:
                    run_gh(["issue", "reopen", str(github_number)])
                    action = "reopened_updated"
                if not github_number:
                    existing = find_existing_issue_by_marker(issue_id)
                    if not existing and plan_version == "v3":
                        existing = find_existing_issue_by_rule(rule_ids)
                    if not existing:
                        existing = find_existing_issue_by_title(title)
                        if existing:
                            warning = f"{issue_id}: linked existing open issue by exact title fallback; verify this is the intended issue before future publishes."
                            warnings.append(warning)
                    if existing:
                        github_number = existing.get("number")
                        github_url = existing.get("url")
                if github_number:
                    changed = not ledger_item or ledger_item.get("body_hash") != body_hash or ledger_item.get("title") != title
                    if changed or action == "reopened_updated":
                        run_gh(["issue", "edit", str(github_number), "--title", title, "--body-file", body_path])
                        if action != "reopened_updated":
                            action = "updated"
                    else:
                        action = "unchanged"
                    label_failures.extend(apply_labels(github_number, available_labels))
                else:
                    output = run_gh(["issue", "create", "--title", title, "--body-file", body_path])
                    github_url = output.splitlines()[-1] if output else ""
                    github_number = issue_number_from_url(github_url)
                    action = "created"
                    if github_number:
                        label_failures.extend(apply_labels(github_number, available_labels))
            if action != "skipped_closed":
                timestamp = now_iso()
                upsert_ledger_issue(ledger, {
                    "local_issue_id": issue_id,
                    "plan_version": plan_version,
                    "rule_ids": rule_ids,
                    "quality_gate_ids": quality_gate_ids,
                    "source_policy_files": source_policy_files,
                    "maturity_level": issue.get("maturity_level", ""),
                    "category": issue.get("category", ""),
                    "severity": issue.get("rule_severity") or issue.get("severity", ""),
                    "title": title,
                    "source_gap_hash": source_hash,
                    "body_hash": body_hash,
                    "labels": labels,
                    "github_issue_number": github_number,
                    "github_issue_url": github_url,
                    "state": "open",
                    "last_published_at": timestamp,
                    "last_synced_at": timestamp,
                })
        except Exception as exc:
            action = "failed"
            error = f"{issue_id}: {exc}"
            item_errors.append(error)
            errors.append(error)
        finally:
            try:
                os.unlink(body_path)
            except OSError:
                pass

    actions.append({
        "local_issue_id": issue_id,
        "issue_id": issue_id,
        "title": title,
        "action": action,
        "github_issue_number": github_number,
        "github_number": github_number,
        "github_issue_url": github_url,
        "github_url": github_url,
        "labels": labels,
        "plan_version": plan_version,
        "rule_ids": rule_ids,
        "quality_gate_ids": quality_gate_ids,
        "source_policy_files": source_policy_files,
        "maturity_level": issue.get("maturity_level", ""),
        "category": issue.get("category", ""),
        "severity": issue.get("rule_severity") or issue.get("severity", ""),
        "label_failures": label_failures,
        "source_gap_hash": source_hash,
        "rule_result_hash": rule_result_hash(issue),
        "body_hash": body_hash,
        "errors": item_errors,
        "markers": [line for line in body.splitlines() if line.startswith("<!-- autospec-")],
    })

mode = "confirm" if confirm else "dry_run"
status = "fail" if errors else "pass"
if confirm and not errors:
    write_json(ledger_path, ledger)
write_reports(mode, status, actions, errors, warnings)
if errors:
    print("issue publishing: FAIL")
    for error in errors:
        print(f"- {error}")
    sys.exit(1)
print("issue publishing: PASS" if confirm else "issue publishing: DRY-RUN")
if confirm:
    print(f"reports: .autospec/reports/github-issue-publish-result{suffix}.json, .autospec/reports/github-issue-publish-result{suffix}.md")
else:
    print(f"reports: .autospec/reports/github-issue-publish-plan{suffix}.json, .autospec/reports/github-issue-publish-plan{suffix}.md")
PY
