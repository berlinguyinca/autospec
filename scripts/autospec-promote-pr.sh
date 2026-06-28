#!/usr/bin/env bash
# scripts/autospec-promote-pr.sh — promote verified PRs to human review.

set -eu

usage() {
    cat <<'EOF'
Usage:
  autospec-promote-pr.sh [--repo-root <dir>] [--dry-run|--confirm] [--repo OWNER/REPO] (--pr <number>|--issue <number>)
EOF
}

die() { printf 'autospec-promote-pr: %s\n' "$*" >&2; exit 2; }

REPO_ROOT="$(pwd)"; CONFIRM=0; GH_REPO=""; PR=""; ISSUE=""
while [ "$#" -gt 0 ]; do
    case "$1" in
        --repo-root) REPO_ROOT="$2"; shift 2 ;;
        --dry-run) CONFIRM=0; shift ;;
        --confirm) CONFIRM=1; shift ;;
        --repo) GH_REPO="$2"; shift 2 ;;
        --pr) PR="$2"; shift 2 ;;
        --issue) ISSUE="$2"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown arg: $1" ;;
    esac
done
[ -d "$REPO_ROOT" ] || die "--repo-root does not exist: $REPO_ROOT"
REPO_ROOT="$(cd "$REPO_ROOT" && pwd -P)"

python3 - "$REPO_ROOT" "$CONFIRM" "$GH_REPO" "$PR" "$ISSUE" <<'PY'
import json, os, subprocess, sys

repo_root, confirm, gh_repo, pr, issue = os.path.realpath(sys.argv[1]), sys.argv[2] == "1", sys.argv[3], sys.argv[4], sys.argv[5]
reports = os.path.join(repo_root, ".autospec", "reports")
state = os.path.join(repo_root, ".autospec", "state")
out_json = os.path.join(reports, "promotion-result.json" if confirm else "promotion-plan.json")
out_md = os.path.join(reports, "promotion-result.md" if confirm else "promotion-plan.md")
promotions = os.path.join(state, "promotions")

def load(path, default):
    try:
        with open(path, "r", encoding="utf-8") as fh: return json.load(fh)
    except Exception: return default

def write_json(path, data):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as fh: json.dump(data, fh, indent=2, sort_keys=True); fh.write("\n")

def write_text(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as fh: fh.write(text.rstrip()+"\n")

def gh(args):
    cmd = ["gh"] + (["--repo", gh_repo] if gh_repo else []) + args
    cp = subprocess.run(cmd, cwd=repo_root, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    if cp.returncode != 0: raise RuntimeError(cp.stderr.strip() or cp.stdout.strip())
    return cp.stdout.strip()

verifier = load(os.path.join(reports, "verifier-report.json"), {})
risk = load(os.path.join(reports, "worker-risk-classification.json"), {})
diff = load(os.path.join(reports, "worker-diff-review.json"), {})
verdict = verifier.get("verdict")
dimensions = {d.get("dimension"): d for d in verifier.get("dimensions", []) if isinstance(d, dict)}
blocked = []
if verdict not in {"pass", "pass_with_warnings"}: blocked.append(f"verifier verdict is {verdict or 'missing'}")
if dimensions.get("validation_evidence", {}).get("status") == "fail": blocked.append("missing validation for code changes")
if dimensions.get("forbidden_paths", {}).get("status") == "fail": blocked.append("forbidden path finding")
if dimensions.get("pr_body_completeness", {}).get("status") == "fail": blocked.append("PR body missing required sections")
if dimensions.get("issue_alignment", {}).get("status") in {"fail", "unknown"}: blocked.append("source issue is not linked")
if diff.get("patch_budget", {}).get("passed") is False: blocked.append("patch budget failed")
classification = risk.get("classification", "")
if "high-risk" in classification: blocked.append("high-risk promotion requires explicit human approval/config")
allowed = not blocked
labels_add = ["autospec:needs-human-review", "autospec:verified"] if allowed else []
labels_remove = ["autospec:needs-changes", "autospec:verification-failed"] if allowed else []
actions = []
if confirm and pr and allowed:
    for label in labels_add:
        gh(["issue", "edit", pr, "--add-label", label]); actions.append(f"add {label}")
    for label in labels_remove:
        gh(["issue", "edit", pr, "--remove-label", label]); actions.append(f"remove {label}")

source_id = f"pr-{pr}" if pr else f"issue-{issue or 'unknown'}"
report = {"version":1,"mode":"confirm" if confirm else "dry_run","source":{"pr":pr,"issue":issue},"verifier_verdict":verdict,"promotion_allowed":allowed,"labels_to_add":labels_add,"labels_to_remove":labels_remove,"ready_for_review_allowed":False,"blocked_reasons":blocked,"actions":actions,"side_effects":{"approved":False,"merged":False}}
write_json(out_json, report); write_json(os.path.join(promotions, f"{source_id}.json"), report)
md = "\n".join(["# Promotion Gate", "", f"Verdict: **{'allowed' if allowed else 'blocked'}**", "", "## Evidence", f"- Verifier verdict: `{verdict}`", f"- Risk classification: `{classification or 'unknown'}`", "", "## Labels", *[f"- add `{l}`" for l in labels_add], *[f"- remove `{l}`" for l in labels_remove], "", "## Blocked reasons", *(f"- {b}" for b in blocked or ["None."]), "", "## Next recommended command", "`bash scripts/autospec-autonomy-status.sh`"])
write_text(out_md, md); write_text(os.path.join(promotions, f"{source_id}.md"), md)
print("promotion: PASS" if allowed else "promotion: BLOCKED")
sys.exit(0 if allowed else 1)
PY
