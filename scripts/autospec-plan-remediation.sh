#!/usr/bin/env bash
# scripts/autospec-plan-remediation.sh — turn verifier findings into a plan.
set -eu
usage(){ echo "Usage: autospec-plan-remediation.sh [--repo-root DIR] [--dry-run|--confirm] [--pr N|--verification PATH]"; }
die(){ printf 'autospec-plan-remediation: %s\n' "$*" >&2; exit 2; }
REPO_ROOT="$(pwd)"; CONFIRM=0; PR=""; VERIFICATION=""
while [ "$#" -gt 0 ]; do case "$1" in --repo-root) REPO_ROOT="$2"; shift 2;; --dry-run) CONFIRM=0; shift;; --confirm) CONFIRM=1; shift;; --pr) PR="$2"; shift 2;; --verification) VERIFICATION="$2"; shift 2;; -h|--help) usage; exit 0;; *) die "unknown arg: $1";; esac; done
REPO_ROOT="$(cd "$REPO_ROOT" && pwd -P)"
python3 - "$REPO_ROOT" "$CONFIRM" "$PR" "$VERIFICATION" <<'PY'
import json, os, sys
root, confirm, pr, verification = os.path.realpath(sys.argv[1]), sys.argv[2]=="1", sys.argv[3], sys.argv[4]
reports=os.path.join(root,'.autospec','reports'); state=os.path.join(root,'.autospec','state')
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
v=load(verification or os.path.join(reports,'verifier-report.json'),{})
groups={k:[] for k in ['blocking','required_before_human_review','recommended','follow_up','needs_guidance']}
safe=True
for i,d in enumerate(v.get('dimensions',[]),1):
    if d.get('status')=='pass': continue
    action=d.get('required_action') or d.get('summary','Review finding.')
    item={'finding_id':f'F{i:03d}','source_dimension':d.get('dimension'),'summary':d.get('summary',''),'required_action':action,'suggested_files':[],'risk':'low','estimated_scope':'small','worker_v1_may_address':True,'human_guidance_needed':False}
    if d.get('dimension') in {'forbidden_paths','risk_classification'} or v.get('verdict')=='blocked':
        item.update({'risk':'high','worker_v1_may_address':False,'human_guidance_needed':True}); groups['blocking'].append(item); safe=False
    elif d.get('dimension')=='issue_alignment':
        item.update({'worker_v1_may_address':False,'human_guidance_needed':True}); groups['needs_guidance'].append(item); safe=False
    elif d.get('status')=='fail':
        groups['required_before_human_review'].append(item)
    elif d.get('status')=='warn':
        groups['recommended'].append(item)
    else:
        groups['follow_up'].append(item)
report={'version':1,'mode':'confirm' if confirm else 'dry_run','pr':pr,'safe_for_worker_remediation':safe,'groups':groups,'stuck_guidance_recommended':not safe}
write_json(os.path.join(reports,'remediation-plan.json'),report)
work=os.path.join(state,'work-items','1'); write_json(os.path.join(work,'remediation-plan.json'),report)
md=['# Remediation Plan','',f"Safe for worker remediation: **{str(safe).lower()}**",'']
for name,items in groups.items():
    md += [f"## {name}", '', "| Finding | Dimension | Action | Worker v1 |", "| --- | --- | --- | --- |"]
    md += [f"| {it['finding_id']} | {it['source_dimension']} | {it['required_action']} | {str(it['worker_v1_may_address']).lower()} |" for it in items] or ["| none | none | none | n/a |"]
write_text(os.path.join(reports,'remediation-plan.md'),'\n'.join(md)); write_text(os.path.join(work,'remediation-plan.md'),'\n'.join(md))
print('remediation plan: PASS')
PY
