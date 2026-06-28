#!/usr/bin/env bash
# scripts/autospec-publish-issues.sh — publish local issue drafts to GitHub.
#
# Dry-run by default. Real GitHub writes require --confirm. Maintains a local
# sync ledger to avoid duplicate GitHub issues for the same backlog draft.

set -eu

usage() {
    cat <<'EOF'
Usage:
  autospec-publish-issues.sh [--repo-root <dir>] [--confirm] [--repo OWNER/REPO]

Inputs:
  .autospec/reports/issue-plan.json
  .autospec/backlog/issues/*.md

Writes:
  .autospec/state/github-issue-sync-ledger.json   (--confirm only)
  .autospec/reports/github-issue-publish.json
  .autospec/reports/github-issue-publish.md
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
ledger_path = os.path.join(state_dir, "github-issue-sync-ledger.json")
json_report = os.path.join(reports_dir, "github-issue-publish.json")
md_report = os.path.join(reports_dir, "github-issue-publish.md")


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


def body_for_issue(issue):
    draft_path = os.path.join(repo_root, issue.get("draft_path", ""))
    with open(draft_path, "r", encoding="utf-8") as fh:
        body = fh.read()
    marker = f"<!-- autospec-sync issue_id={issue['issue_id']} -->"
    if marker not in body:
        body = f"{marker}\n\n{body}"
    return body


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


def find_existing_issue(issue_id):
    output = run_gh(["issue", "list", "--search", f"autospec-sync issue_id={issue_id}", "--state", "open", "--json", "number,url,title", "--limit", "1"])
    try:
        data = json.loads(output or "[]")
    except Exception:
        data = []
    return data[0] if data else None


def write_reports(mode, status, actions, errors):
    report = {
        "version": 1,
        "mode": mode,
        "status": status,
        "actions": actions,
        "errors": errors,
        "ledger_path": ".autospec/state/github-issue-sync-ledger.json",
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
        "| Local issue | Action | GitHub issue | Title |",
        "| --- | --- | --- | --- |",
    ]
    for item in actions:
        gh_issue = item.get("github_url") or item.get("github_number") or ""
        lines.append(f"| `{item['issue_id']}` | {item['action']} | {gh_issue} | {item['title']} |")
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

if not issues:
    error = "missing issue plan entries; run scripts/autospec-plan-issues.sh before publishing"
    write_reports("confirm" if confirm else "dry_run", "error", [], [error])
    print(f"issue publishing: ERROR - {error}")
    sys.exit(2)

for issue in issues:
    issue_id = issue.get("issue_id")
    title = issue.get("title", "")
    body = body_for_issue(issue)
    body_hash = sha256(body)
    labels = labels_for_issue(issue)
    ledger_item = ledger["items"].get(issue_id)
    action = "would_create"
    github_number = None
    github_url = None
    if ledger_item:
        action = "would_update"
        github_number = ledger_item.get("github_number")
        github_url = ledger_item.get("github_url")
    if confirm:
        body_path = write_temp_body(body)
        try:
            if ledger_item and github_number:
                args = ["issue", "edit", str(github_number), "--title", title, "--body-file", body_path]
                for label in labels:
                    args.extend(["--add-label", label])
                run_gh(args)
                action = "updated"
            else:
                existing = find_existing_issue(issue_id)
                if existing:
                    github_number = existing.get("number")
                    github_url = existing.get("url")
                    args = ["issue", "edit", str(github_number), "--title", title, "--body-file", body_path]
                    for label in labels:
                        args.extend(["--add-label", label])
                    run_gh(args)
                    action = "updated"
                else:
                    args = ["issue", "create", "--title", title, "--body-file", body_path]
                    for label in labels:
                        args.extend(["--label", label])
                    output = run_gh(args)
                    github_url = output.splitlines()[-1] if output else ""
                    github_number = issue_number_from_url(github_url)
                    action = "created"
            ledger["items"][issue_id] = {
                "github_number": github_number,
                "github_url": github_url,
                "title": title,
                "body_hash": body_hash,
                "draft_path": issue.get("draft_path"),
                "labels": labels,
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
print("reports: .autospec/reports/github-issue-publish.json, .autospec/reports/github-issue-publish.md")
PY
