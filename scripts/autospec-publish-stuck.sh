#!/usr/bin/env bash
# scripts/autospec-publish-stuck.sh — publish/update stuck handoff issues.
set -eu
die(){ printf 'autospec-publish-stuck: %s\n' "$*" >&2; exit 2; }
REPO_ROOT="$(pwd)"; CONFIRM=0; WORK_ITEM=""; GH_REPO=""
while [ "$#" -gt 0 ]; do case "$1" in --repo-root) REPO_ROOT="$2"; shift 2;; --dry-run) CONFIRM=0; shift;; --confirm) CONFIRM=1; shift;; --work-item) WORK_ITEM="$2"; shift 2;; --repo) GH_REPO="$2"; shift 2;; *) die "unknown arg: $1";; esac; done
[ -n "$WORK_ITEM" ] || die "--work-item required"; REPO_ROOT="$(cd "$REPO_ROOT" && pwd -P)"
python3 - "$REPO_ROOT" "$CONFIRM" "$WORK_ITEM" "$GH_REPO" <<'PY'
import hashlib,json,os,re,subprocess,sys
root,confirm,work_id,repo=os.path.realpath(sys.argv[1]),sys.argv[2]=='1',sys.argv[3],sys.argv[4]
reports=os.path.join(root,'.autospec','reports'); state=os.path.join(root,'.autospec','state')
handoff=os.path.join(state,'work-items',work_id,'stuck-handoff.md'); ledger=os.path.join(state,'stuck-handovers.json')
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
body=open(handoff,encoding='utf-8').read() if os.path.exists(handoff) else '# bot stuck\n'
h=hashlib.sha256(body.encode()).hexdigest()
marked=f"<!-- autospec-stuck-for-issue: {work_id} -->\n<!-- autospec-work-item-id: {work_id} -->\n<!-- autospec-stuck-hash: {h} -->\n\n{body}"
number=None; url=''
data=load(ledger,{'schema':1,'handovers':[]})
for item in data.get('handovers',[]):
    if str(item.get('work_item_id'))==str(work_id): number=item.get('stuck_issue_number'); url=item.get('stuck_issue_url','')
if confirm:
    import tempfile
    fd,path=tempfile.mkstemp(prefix='autospec-stuck-',suffix='.md')
    with os.fdopen(fd,'w',encoding='utf-8') as f: f.write(marked)
    if number:
        gh(['issue','edit',str(number),'--body-file',path]); action='updated'
    else:
        out=gh(['issue','create','--title',f'bot stuck: work item {work_id}','--body-file',path])
        url=out.splitlines()[-1] if out else ''; m=re.search(r'/issues/(\d+)',url); number=int(m.group(1)) if m else None; action='created'
    for label in ['autospec:stuck','autospec:needs-guidance','autospec:managed']:
        if number: gh(['issue','edit',str(number),'--add-label',label])
else: action='would_create' if not number else 'would_update'
entry={'work_item_id':work_id,'source_issue_number':work_id,'stuck_issue_number':number,'stuck_issue_url':url,'stuck_hash':h,'state':'needs-guidance','action':action}
data['handovers']=[i for i in data.get('handovers',[]) if str(i.get('work_item_id'))!=str(work_id)]+[entry]
if confirm: write_json(ledger,data)
report={'version':1,'mode':'confirm' if confirm else 'dry_run','handovers':[entry],'body_markers':marked.splitlines()[:3]}
write_json(os.path.join(reports,'stuck-publish-result.json' if confirm else 'stuck-publish-plan.json'),report)
md='\n'.join(['# Stuck Publish', '', f"Action: **{action}**", '', *marked.splitlines()[:3]])
write_text(os.path.join(reports,'stuck-publish-result.md' if confirm else 'stuck-publish-plan.md'),md)
print('stuck publish: PASS')
PY
