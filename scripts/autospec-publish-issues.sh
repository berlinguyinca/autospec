#!/usr/bin/env bash
# scripts/autospec-publish-issues.sh — publish local issue drafts to GitHub.
#
# Dry-run by default. Real GitHub writes require --confirm. Maintains a local
# published-issues ledger to avoid duplicate GitHub issues for the same draft.

set -eu

usage() {
    cat <<'EOF'
Usage:
  autospec-publish-issues.sh [--repo-root <dir>] [--dry-run|--confirm] [--repo OWNER/REPO]

Inputs:
  .autospec/reports/issue-plan.json
  .autospec/backlog/issues/*.md
  .autospec/state/control-labels.yml

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
GH_REPO=""
while [ "$#" -gt 0 ]; do
    case "$1" in
        --repo-root) [ "$#" -ge 2 ] || die "--repo-root requires a value"; REPO_ROOT="$2"; shift 2 ;;
        --dry-run) CONFIRM=0; shift ;;
        --confirm) CONFIRM=1; shift ;;
        --repo) [ "$#" -ge 2 ] || die "--repo requires OWNER/REPO"; GH_REPO="$2"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown arg: $1" ;;
    esac
done

[ -d "$REPO_ROOT" ] || die "--repo-root does not exist: $REPO_ROOT"
REPO_ROOT="$(cd "$REPO_ROOT" && pwd -P)"

python3 - "$REPO_ROOT" "$CONFIRM" "$GH_REPO" <<'PY'
import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile

repo_root = os.path.realpath(sys.argv[1])
confirm = sys.argv[2] == "1"
gh_repo = sys.argv[3]
reports_dir = os.path.join(repo_root, ".autospec", "reports")
state_dir = os.path.join(repo_root, ".autospec", "state")
issue_plan_path = os.path.join(reports_dir, "issue-plan.json")
labels_path = os.path.join(state_dir, "control-labels.yml")
ledger_path = os.path.join(state_dir, "published-issues.json")
json_report = os.path.join(reports_dir, "github-issue-publish-result.json" if confirm else "github-issue-publish-plan.json")
md_report = os.path.join(reports_dir, "github-issue-publish-result.md" if confirm else "github-issue-publish-plan.md")


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


def issue_number_from_url(url):
    match = re.search(r"/issues/([0-9]+)", url)
    return int(match.group(1)) if match else None


def source_gap_hash(issue):
    return sha256(json.dumps(issue.get("source_gap", {}), sort_keys=True))


def body_for_issue(issue):
    draft_path = os.path.join(repo_root, issue.get("draft_path", ""))
    with open(draft_path, "r", encoding="utf-8") as fh:
        draft_body = fh.read()
    source_hash = source_gap_hash(issue)
    base = "\n".join([
        f"<!-- autospec-local-issue-id: {issue['issue_id']} -->",
        f"<!-- autospec-source-gap-hash: {source_hash} -->",
        "<!-- autospec-body-hash: __AUTOSPEC_BODY_HASH__ -->",
        "",
        draft_body,
    ])
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
    if issue.get("risk") == "High":
        labels.append("autospec:risk-high")
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


def find_existing_issue(issue_id):
    output = run_gh(["issue", "list", "--search", f"autospec-local-issue-id: {issue_id}", "--state", "open", "--json", "number,url,title", "--limit", "1"])
    try:
        data = json.loads(output or "[]")
    except Exception:
        data = []
    return data[0] if data else None


def apply_labels(github_number, labels):
    failures = []
    for label in labels:
        try:
            run_gh(["issue", "edit", str(github_number), "--add-label", label])
        except Exception as exc:
            failures.append(f"{label}: {exc}")
    return failures


def write_reports(mode, status, actions, errors):
    report = {
        "version": 1,
        "mode": mode,
        "status": status,
        "actions": actions,
        "errors": errors,
        "ledger_path": ".autospec/state/published-issues.json",
        "side_effects": {
            "github_api_calls": bool(confirm),
            "github_issues_created": bool(confirm and any(item["action"] == "created" for item in actions)),
            "github_issues_updated": bool(confirm and any(item["action"] == "updated" for item in actions)),
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
        "",
        "| Local issue | Action | GitHub issue | Labels failed | Title |",
        "| --- | --- | --- | --- | --- |",
    ]
    for item in actions:
        gh_issue = item.get("github_url") or item.get("github_number") or ""
        label_failures = "<br>".join(item.get("label_failures", [])) or "none"
        lines.append(f"| `{item['issue_id']}` | {item['action']} | {gh_issue} | {label_failures} | {item['title']} |")
    if errors:
        lines.extend(["", "## Errors", ""])
        for error in errors:
            lines.append(f"- {error}")
    lines.extend(["", "## Safety", "", "- Branches created: false", "- PRs created: false", "- Implementation started: false"])
    with open(md_report, "w", encoding="utf-8") as fh:
        fh.write("\n".join(lines))
        fh.write("\n")


plan = load_json(issue_plan_path, {})
issues = plan.get("issues", []) if isinstance(plan.get("issues"), list) else []
ledger = load_json(ledger_path, {"version": 1, "items": {}})
ledger.setdefault("version", 1)
ledger.setdefault("items", {})
actions = []
errors = []
known_labels = known_control_labels()

if not issues:
    error = "missing issue plan entries; run scripts/autospec-plan-issues.sh before publishing"
    write_reports("confirm" if confirm else "dry_run", "error", [], [error])
    print(f"issue publishing: ERROR - {error}")
    sys.exit(2)

for issue in issues:
    issue_id = issue.get("issue_id")
    title = issue.get("title", "")
    body, source_hash, body_hash = body_for_issue(issue)
    labels = labels_for_issue(issue)
    available_labels = [label for label in labels if not label.startswith("autospec:") or not known_labels or label in known_labels]
    unavailable_labels = [label for label in labels if label.startswith("autospec:") and known_labels and label not in known_labels]
    ledger_item = ledger["items"].get(issue_id)
    action = "would_create"
    github_number = None
    github_url = None
    label_failures = [f"{label}: missing from .autospec/state/control-labels.yml" for label in unavailable_labels]
    if ledger_item:
        action = "would_update"
        github_number = ledger_item.get("github_number")
        github_url = ledger_item.get("github_url")
    if confirm:
        body_path = write_temp_body(body)
        try:
            if ledger_item and github_number:
                args = ["issue", "edit", str(github_number), "--title", title, "--body-file", body_path]
                run_gh(args)
                label_failures.extend(apply_labels(github_number, available_labels))
                action = "updated"
            else:
                existing = find_existing_issue(issue_id)
                if existing:
                    github_number = existing.get("number")
                    github_url = existing.get("url")
                    args = ["issue", "edit", str(github_number), "--title", title, "--body-file", body_path]
                    run_gh(args)
                    label_failures.extend(apply_labels(github_number, available_labels))
                    action = "updated"
                else:
                    args = ["issue", "create", "--title", title, "--body-file", body_path]
                    output = run_gh(args)
                    github_url = output.splitlines()[-1] if output else ""
                    github_number = issue_number_from_url(github_url)
                    if github_number:
                        label_failures.extend(apply_labels(github_number, available_labels))
                    action = "created"
            ledger["items"][issue_id] = {
                "github_number": github_number,
                "github_url": github_url,
                "title": title,
                "body_hash": body_hash,
                "source_gap_hash": source_hash,
                "draft_path": issue.get("draft_path"),
                "labels": labels,
                "label_failures": label_failures,
            }
        except Exception as exc:
            action = "failed"
            errors.append(f"{issue_id}: {exc}")
        finally:
            try:
                os.unlink(body_path)
            except OSError:
                pass
    actions.append({
        "issue_id": issue_id,
        "title": title,
        "action": action,
        "github_number": github_number,
        "github_url": github_url,
        "labels": labels,
        "label_failures": label_failures,
        "source_gap_hash": source_hash,
        "body_hash": body_hash,
    })

mode = "confirm" if confirm else "dry_run"
status = "fail" if errors else "pass"
if confirm and not errors:
    write_json(ledger_path, ledger)
write_reports(mode, status, actions, errors)
if errors:
    print("issue publishing: FAIL")
    for error in errors:
        print(f"- {error}")
    sys.exit(1)
print("issue publishing: PASS" if confirm else "issue publishing: DRY-RUN")
if confirm:
    print("reports: .autospec/reports/github-issue-publish-result.json, .autospec/reports/github-issue-publish-result.md")
else:
    print("reports: .autospec/reports/github-issue-publish-plan.json, .autospec/reports/github-issue-publish-plan.md")
PY
