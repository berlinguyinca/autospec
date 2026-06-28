#!/usr/bin/env bash
# scripts/autospec-sync-guidance.sh — sync stuck guidance without resuming.
set -eu
REPO_ROOT="$(pwd)"; CONFIRM=0; GH_REPO=""
while [ "$#" -gt 0 ]; do case "$1" in --repo-root) REPO_ROOT="$2"; shift 2;; --dry-run) CONFIRM=0; shift;; --confirm) CONFIRM=1; shift;; --repo) GH_REPO="$2"; shift 2;; *) printf 'autospec-sync-guidance: unknown arg: %s\n' "$1" >&2; exit 2;; esac; done
REPO_ROOT="$(cd "$REPO_ROOT" && pwd -P)"
python3 - "$REPO_ROOT" "$CONFIRM" "$GH_REPO" <<'PY'
import json,os,subprocess,sys
root,confirm,repo=os.path.realpath(sys.argv[1]),sys.argv[2]=='1',sys.argv[3]
reports=os.path.join(root,'.autospec','reports'); state=os.path.join(root,'.autospec','state'); ledger=os.path.join(state,'stuck-handovers.json')
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
def gh(args):
    cmd=['gh']+(['--repo',repo] if repo else [])+args
    cp=subprocess.run(cmd,cwd=root,text=True,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
    if cp.returncode!=0: raise RuntimeError(cp.stderr.strip() or cp.stdout.strip())
    return cp.stdout.strip()
data=load(ledger,{'schema':1,'handovers':[]}); updates=[]
for item in data.get('handovers',[]):
    num=item.get('stuck_issue_number')
    if not num: continue
    remote=json.loads(gh(['issue','view',str(num),'--json','number,title,state,labels,comments']))
    labels=[l.get('name') for l in remote.get('labels',[])]
    comments=remote.get('comments',[])
    guidance=bool(comments) or 'autospec:guidance-provided' in labels
    ready=guidance and 'autospec:resume' in labels
    item['guidance_detected']=guidance; item['resume_label_detected']='autospec:resume' in labels
    if ready: item['state']='ready-to-resume'
    updates.append(item)
if confirm: write_json(ledger,data)
report={'version':1,'mode':'confirm' if confirm else 'dry_run','handovers':data.get('handovers',[]),'automatic_resume':False}
write_json(os.path.join(reports,'guidance-sync.json'),report)
md='\n'.join(['# Guidance Sync','',f"Handovers: {len(data.get('handovers',[]))}",'','No automatic resume was performed.'])
write_text(os.path.join(reports,'guidance-sync.md'),md)
print('guidance sync: PASS')
PY
