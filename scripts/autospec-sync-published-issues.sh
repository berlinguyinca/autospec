#!/usr/bin/env bash
# scripts/autospec-sync-published-issues.sh — read-only GitHub issue ledger sync.

set -eu

usage() {
    cat <<'EOF'
Usage:
  autospec-sync-published-issues.sh [--repo-root <dir>] [--repo OWNER/REPO]

Reads:
  .autospec/state/published-issues.json

Writes:
  .autospec/state/published-issues.json
  .autospec/reports/github-issue-sync.json
  .autospec/reports/github-issue-sync.md
EOF
}

die() {
    printf 'autospec-sync-published-issues: %s\n' "$*" >&2
    exit 2
}

REPO_ROOT="$(pwd)"
GH_REPO=""
while [ "$#" -gt 0 ]; do
    case "$1" in
        --repo-root) [ "$#" -ge 2 ] || die "--repo-root requires a value"; REPO_ROOT="$2"; shift 2 ;;
        --repo) [ "$#" -ge 2 ] || die "--repo requires OWNER/REPO"; GH_REPO="$2"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown arg: $1" ;;
    esac
done

[ -d "$REPO_ROOT" ] || die "--repo-root does not exist: $REPO_ROOT"
REPO_ROOT="$(cd "$REPO_ROOT" && pwd -P)"

python3 - "$REPO_ROOT" "$GH_REPO" <<'PY'
import datetime
import hashlib
import json
import os
import subprocess
import sys

repo_root = os.path.realpath(sys.argv[1])
gh_repo = sys.argv[2]
state_dir = os.path.join(repo_root, ".autospec", "state")
reports_dir = os.path.join(repo_root, ".autospec", "reports")
ledger_path = os.path.join(state_dir, "published-issues.json")
json_report = os.path.join(reports_dir, "github-issue-sync.json")
md_report = os.path.join(reports_dir, "github-issue-sync.md")
json_report_v3 = os.path.join(reports_dir, "github-issue-sync-v3.json")
md_report_v3 = os.path.join(reports_dir, "github-issue-sync-v3.md")


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


def parse_json_object(text):
    try:
        data = json.loads(text or "{}")
        return data if isinstance(data, dict) else {}
    except Exception:
        return {}


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


def marker_value(body, marker):
    for line in str(body or "").splitlines():
        prefix = f"<!-- {marker}: "
        if line.startswith(prefix) and line.endswith(" -->"):
            return line[len(prefix):-4].strip()
    return ""


def stable_hash(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True).encode("utf-8")).hexdigest()


ledger = load_json(ledger_path, {})
if not isinstance(ledger.get("issues"), list):
    error = "missing .autospec/state/published-issues.json schema 1 issues array; run scripts/autospec-publish-issues.sh --confirm first"
    write_json(json_report, {"version": 1, "status": "error", "errors": [error], "issues": []})
    with open(md_report, "w", encoding="utf-8") as fh:
        fh.write(f"# GitHub Issue Sync\n\nStatus: **ERROR**\n\n- {error}\n")
    print(f"issue sync: ERROR - {error}")
    sys.exit(2)

records = []
errors = []
closed = []
guidance = []
resume = []
rule_results = load_json(os.path.join(state_dir, "rule-check-results.json"), load_json(os.path.join(reports_dir, "rule-check-results.json"), {"results": []}))
rules_by_id = {r.get("rule_id"): r for r in rule_results.get("results", []) if isinstance(r, dict)}
for item in ledger.get("issues", []):
    local_id = item.get("local_issue_id")
    number = item.get("github_issue_number")
    before = {
        "title": item.get("title", ""),
        "state": item.get("state", ""),
        "labels": sorted(item.get("labels", [])),
        "url": item.get("github_issue_url", ""),
    }
    try:
        remote = parse_json_object(run_gh(["issue", "view", str(number), "--json", "number,url,title,state,labels,body"]))
        remote_labels = normalize_labels(remote.get("labels", []))
        remote_body_hash = marker_value(remote.get("body", ""), "autospec-body-hash")
        item.update({
            "title": remote.get("title", item.get("title", "")),
            "state": normalize_state(remote.get("state")),
            "labels": remote_labels,
            "github_issue_url": remote.get("url", item.get("github_issue_url", "")),
            "last_synced_at": now_iso(),
        })
        after = {
            "title": item.get("title", ""),
            "state": item.get("state", ""),
            "labels": sorted(item.get("labels", [])),
            "url": item.get("github_issue_url", ""),
        }
        drift = []
        for key in ["title", "state", "labels", "url"]:
            if before.get(key) != after.get(key):
                drift.append({"field": key, "before": before.get(key), "after": after.get(key)})
        if remote_body_hash and item.get("body_hash") and remote_body_hash != item.get("body_hash"):
            drift.append({"field": "body_hash", "before": item.get("body_hash"), "after": remote_body_hash})
        if item["state"] == "closed":
            closed.append(local_id)
        if "autospec:needs-guidance" in remote_labels:
            guidance.append(local_id)
        if "autospec:resume" in remote_labels or "autospec:guidance-provided" in remote_labels:
            resume.append(local_id)
        records.append({
            "local_issue_id": local_id,
            "github_issue_number": number,
            "github_issue_url": item.get("github_issue_url", ""),
            "state": item.get("state", ""),
            "labels": remote_labels,
            "drift": drift,
        })
    except Exception as exc:
        error = f"{local_id}: {exc}"
        errors.append(error)
        records.append({"local_issue_id": local_id, "github_issue_number": number, "state": item.get("state", ""), "errors": [error], "drift": []})

ledger["schema"] = 1
ledger["repo"] = gh_repo or ledger.get("repo", "")
ledger["issues"] = sorted(ledger.get("issues", []), key=lambda entry: entry.get("local_issue_id", ""))
write_json(ledger_path, ledger)

status = "fail" if errors else "pass"
report = {
    "version": 1,
    "status": status,
    "repo": ledger.get("repo", ""),
    "issues": records,
    "errors": errors,
    "summary": {
        "published_issues": len(records),
        "closed_completed_issues": len(closed),
        "issues_needing_guidance": len(guidance),
        "issues_ready_to_resume": len(resume),
        "drifted_issues": sum(1 for record in records if record.get("drift")),
    },
}
write_json(json_report, report)

v3_items = [item for item in ledger.get("issues", []) if item.get("plan_version") == "v3"]
rule_to_issues = {}
for item in ledger.get("issues", []):
    for rid in item.get("rule_ids", []) or []:
        rule_to_issues.setdefault(rid, []).append(item.get("local_issue_id"))
stale = []
disappeared = []
closed_still_failing = []
waived_open = []
changed_hash = []
duplicates = []
for item in v3_items:
    for rid in item.get("rule_ids", []) or []:
        rule = rules_by_id.get(rid)
        if not rule:
            disappeared.append({"local_issue_id": item.get("local_issue_id"), "rule_id": rid})
            continue
        if rule.get("status") == "pass" and item.get("state", "open") == "open":
            stale.append({"local_issue_id": item.get("local_issue_id"), "rule_id": rid})
        if rule.get("status") in {"waived", "opted_out"} and item.get("state", "open") == "open":
            waived_open.append({"local_issue_id": item.get("local_issue_id"), "rule_id": rid, "status": rule.get("status")})
        if rule.get("status") == "fail" and item.get("state") == "closed":
            closed_still_failing.append({"local_issue_id": item.get("local_issue_id"), "rule_id": rid})
        current_hash = stable_hash(rule)
        if item.get("rule_result_hash") and item.get("rule_result_hash") != current_hash:
            changed_hash.append({"local_issue_id": item.get("local_issue_id"), "rule_id": rid})
for rid, ids in sorted(rule_to_issues.items()):
    if len(ids) > 1:
        duplicates.append({"rule_id": rid, "local_issue_ids": sorted(ids)})
report_v3 = {
    "version": 1,
    "status": status,
    "published_v3_issues": v3_items,
    "stale_v3_issues": stale,
    "disappeared_rule_issues": disappeared,
    "source_rule_changed_hash": changed_hash,
    "closed_issues_still_failing": closed_still_failing,
    "waived_or_opted_out_open_issues": waived_open,
    "duplicate_issues": duplicates,
    "summary": {
        "published_v3_issues": len(v3_items),
        "stale_v3_issues": len(stale),
        "disappeared_rule_issues": len(disappeared),
        "source_rule_changed_hash": len(changed_hash),
        "closed_issues_still_failing": len(closed_still_failing),
        "waived_or_opted_out_open_issues": len(waived_open),
        "duplicate_issues": len(duplicates),
    },
}
write_json(json_report_v3, report_v3)

lines = [
    "# GitHub Issue Sync",
    "",
    f"Status: **{status.upper()}**",
    "",
    "## Summary",
    "",
    "| Metric | Count |",
    "| --- | ---: |",
    f"| Published issues | {report['summary']['published_issues']} |",
    f"| Drifted issues | {report['summary']['drifted_issues']} |",
    f"| Closed/completed issues | {report['summary']['closed_completed_issues']} |",
    f"| Issues needing guidance | {report['summary']['issues_needing_guidance']} |",
    f"| Issues ready to resume | {report['summary']['issues_ready_to_resume']} |",
    "",
    "## Issues",
    "",
    "| Local issue | GitHub issue | State | Labels | Drift |",
    "| --- | --- | --- | --- | --- |",
]
for record in records:
    drift = ", ".join(item["field"] for item in record.get("drift", [])) or "none"
    labels = ", ".join(record.get("labels", [])) or "none"
    lines.append(f"| `{record['local_issue_id']}` | {record.get('github_issue_url') or record.get('github_issue_number') or ''} | {record.get('state', '')} | {labels} | {drift} |")
if guidance:
    lines.extend(["", "## Guidance / Resume", ""])
    for local_id in guidance:
        lines.append(f"- `{local_id}` needs guidance.")
    for local_id in resume:
        lines.append(f"- `{local_id}` is ready to resume.")
if errors:
    lines.extend(["", "## Errors", ""])
    for error in errors:
        lines.append(f"- {error}")
lines.extend(["", "## Next Command", "", "```bash\nbash scripts/autospec-publish-issues.sh --dry-run\n```"])
with open(md_report, "w", encoding="utf-8") as fh:
    fh.write("\n".join(lines))
    fh.write("\n")

lines_v3 = [
    "# GitHub Issue Sync v3",
    "",
    f"Status: **{status.upper()}**",
    "",
    "## Summary",
    "",
    "| Metric | Count |",
    "| --- | ---: |",
    f"| Published v3 issues | {len(v3_items)} |",
    f"| Stale v3 issues | {len(stale)} |",
    f"| Disappeared rules | {len(disappeared)} |",
    f"| Changed rule hashes | {len(changed_hash)} |",
    f"| Closed but still failing | {len(closed_still_failing)} |",
    f"| Waived/opted-out open | {len(waived_open)} |",
    f"| Duplicate issues | {len(duplicates)} |",
    "",
    "## Findings",
    "",
]
for section, rows in [("Stale", stale), ("Disappeared", disappeared), ("Changed hash", changed_hash), ("Closed still failing", closed_still_failing), ("Waived/opted-out", waived_open), ("Duplicates", duplicates)]:
    lines_v3.append(f"### {section}")
    if rows:
        for row in rows:
            lines_v3.append(f"- `{row.get('rule_id')}` {row}")
    else:
        lines_v3.append("- None.")
with open(md_report_v3, "w", encoding="utf-8") as fh:
    fh.write("\n".join(lines_v3))
    fh.write("\n")

if errors:
    print("issue sync: FAIL")
    for error in errors:
        print(f"- {error}")
    sys.exit(1)
print("issue sync: PASS")
print("reports: .autospec/reports/github-issue-sync.json, .autospec/reports/github-issue-sync.md")
PY
