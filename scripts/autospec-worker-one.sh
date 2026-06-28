#!/usr/bin/env bash
# scripts/autospec-worker-one.sh — one-item worker wrapper/remediation gate.
set -eu
usage(){ echo "Usage: autospec-worker-one.sh [--repo-root DIR] [--dry-run|--confirm] --remediate --pr N [--branch NAME]"; }
die(){ printf 'autospec-worker-one: %s\n' "$*" >&2; exit 2; }
REPO_ROOT="$(pwd)"; CONFIRM=0; REMEDIATE=0; PR=""; BRANCH=""
while [ "$#" -gt 0 ]; do case "$1" in --repo-root) REPO_ROOT="$2"; shift 2;; --dry-run) CONFIRM=0; shift;; --confirm) CONFIRM=1; shift;; --remediate) REMEDIATE=1; shift;; --pr) PR="$2"; shift 2;; --branch) BRANCH="$2"; shift 2;; -h|--help) usage; exit 0;; *) die "unknown arg: $1";; esac; done
REPO_ROOT="$(cd "$REPO_ROOT" && pwd -P)"
python3 - "$REPO_ROOT" "$CONFIRM" "$REMEDIATE" "$PR" "$BRANCH" <<'PY'
import json, os, sys
root, confirm, remediate, pr, branch = os.path.realpath(sys.argv[1]), sys.argv[2]=='1', sys.argv[3]=='1', sys.argv[4], sys.argv[5]
reports=os.path.join(root,'.autospec','reports'); work=os.path.join(root,'.autospec','state','work-items','1')
def load(p,d):
    try:
        with open(p,'r',encoding='utf-8') as f: return json.load(f)
    except Exception: return d
def write_json(p,d):
    os.makedirs(os.path.dirname(p),exist_ok=True)
    with open(p,'w',encoding='utf-8') as f: json.dump(d,f,indent=2,sort_keys=True); f.write('\n')
def write_text(p,t):
    os.makedirs(os.path.dirname(p),exist_ok=True)
    with open(p,'w',encoding='utf-8') as f: f.write(t.rstrip()+'\n')
plan=load(os.path.join(work,'remediation-plan.json'),load(os.path.join(reports,'remediation-plan.json'),{}))
reasons=[]; ok=True
if not remediate: ok=False; reasons.append('only remediation mode is implemented in worker-one v0')
if branch and not branch.startswith('autospec/'): ok=False; reasons.append('non-autospec branch refused')
if not plan: ok=False; reasons.append('remediation plan missing')
if plan and not plan.get('safe_for_worker_remediation'): ok=False; reasons.append('remediation plan requires guidance')
result={'version':1,'mode':'confirm' if confirm else 'dry_run','pr':pr,'remediation_attempted':ok,'updated_existing_pr':bool(pr),'reasons':reasons,'next_recommended_command':'bash scripts/autospec-verify-worker-pr.sh --dry-run --pr '+(pr or '<number>')}
write_json(os.path.join(reports,'worker-remediation-result.json'),result); write_json(os.path.join(work,'remediation-result.json'),result)
md='\n'.join(['# Worker Remediation Result','',f"Result: **{'ready' if ok else 'refused'}**",'',*(f"- {r}" for r in reasons), '', '## Next', f"`{result['next_recommended_command']}`"])
write_text(os.path.join(reports,'worker-remediation-result.md'),md); write_text(os.path.join(work,'remediation-result.md'),md)
print('worker remediation: PASS' if ok else 'worker remediation: REFUSED')
sys.exit(0 if ok else 1)
PY
