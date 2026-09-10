#!/usr/bin/env bash
# scripts/autospec-validate-state.sh — validate generated Autospec state/report artifacts.

set -eu
usage() { echo "Usage: autospec-validate-state.sh [--repo-root DIR]"; }
die() { printf 'autospec-validate-state: %s\n' "$*" >&2; exit 2; }
REPO_ROOT="$(pwd)"
while [ "$#" -gt 0 ]; do
    case "$1" in --repo-root) REPO_ROOT="$2"; shift 2 ;; -h|--help) usage; exit 0 ;; *) die "unknown arg: $1" ;; esac
done
[ -d "$REPO_ROOT" ] || die "--repo-root does not exist: $REPO_ROOT"
REPO_ROOT="$(cd "$REPO_ROOT" && pwd -P)"

python3 - "$REPO_ROOT" <<'PY'
import json, os, re, sys
root = os.path.realpath(sys.argv[1]); reports = os.path.join(root, ".autospec", "reports"); state = os.path.join(root, ".autospec", "state")
os.makedirs(reports, exist_ok=True)
findings = []

# Portfolio-first decomposition gate (spec docs/specs/2026-08-31-automatic-spec-
# projects-design.md): every issue-definition path provisions its primary
# portfolio before the first issue create. Two surfaces are checked here:
#   1. generated state artifacts — autospec.portfolio-transaction.v1 documents
#      carry the admission ordering (project.verified before the first
#      issue.create), complete parent sets (audit child included), exactly-one
#      binding per planned item, and durable cross-repository graph persistence;
#      autospec.portfolio-plan.v1 dry-run reports must stay tri-state and leave
#      no transaction behind.
#   2. the workflow bodies — the five workflow entry points below must keep
#      byte-identical lock-step bodies, and in every body the portfolio
#      provisioning command precedes the first `gh issue create`.
WORKFLOW_SKILLS = ("autospec", "autospec-define", "autospec-split", "autospec-explore", "autospec-run")
HEX64 = re.compile(r"^[0-9a-f]{64}$")
ITEM_ROLES = ("tracker", "umbrella", "implementation", "prerequisite", "audit")
CHILD_ROLES = ("implementation", "prerequisite", "audit")
TRI_STATE = ("verified", "unavailable", "unknown")
TRANSACTION_SCHEMA = "autospec.portfolio-transaction.v1"
DRY_RUN_SCHEMA = "autospec.portfolio-plan.v1"
transactions = []
dry_runs = []

def strip_frontmatter(text):
    separators, body = 0, []
    for line in text.splitlines(keepends=True):
        if line.rstrip("\r\n") == "---":
            separators += 1
            continue
        if separators >= 2:
            body.append(line)
    return "".join(body)

def add(sev, rel, summary):
    findings.append({"severity": sev, "file": rel, "summary": summary})

def check_transaction(rel, doc):
    if not isinstance(doc, dict):
        add("fail", rel, "portfolio transaction must be a JSON object"); return
    if not HEX64.match(str(doc.get("portfolio_id", ""))):
        add("fail", rel, "portfolio_id must be a sha256 hex digest")
    if not isinstance(doc.get("project_owner"), str) or not doc.get("project_owner"):
        add("fail", rel, "project_owner missing or empty")
    if not HEX64.match(str(doc.get("plan_digest", ""))):
        add("fail", rel, "plan_digest must be sha256(canonical_plan)")
    items = doc.get("items")
    if not isinstance(items, list) or not items:
        add("fail", rel, "items must be a non-empty list"); items = []
    declared, local = set(), {}
    for item in items:
        key = item.get("item_key") if isinstance(item, dict) else None
        repo = item.get("repository") if isinstance(item, dict) else None
        role = item.get("role") if isinstance(item, dict) else None
        if not isinstance(key, str) or not key:
            add("fail", rel, "item with missing item_key"); continue
        if key in declared:
            add("fail", rel, f"duplicate item_key {key}")
        declared.add(key)
        if not isinstance(repo, str) or not repo:
            add("fail", rel, f"item {key} missing repository")
        if role is not None and role not in ITEM_ROLES:
            add("fail", rel, f"item {key} has unknown role {role}")
        local.setdefault(repo or "", []).append((key, role))
    edges = doc.get("edges", [])
    if not isinstance(edges, list):
        add("fail", rel, "edges must be a list"); edges = []
    seen_edges = set()
    for edge in edges:
        a, b = (edge.get("from"), edge.get("to")) if isinstance(edge, dict) else (None, None)
        if a not in declared or b not in declared:
            add("fail", rel, f"edge {a}->{b} references an undeclared item_key")
        if (a, b) in seen_edges:
            add("fail", rel, f"duplicate edge {a}->{b}")
        seen_edges.add((a, b))
    state_name = doc.get("state", "complete")
    if state_name not in ("complete", "blocked", "provisioning"):
        add("fail", rel, f"unknown transaction state {state_name}")
    ops = doc.get("ops", [])
    if not isinstance(ops, list):
        add("fail", rel, "ops must be a list"); ops = []
    seqs, verified = [], None
    creates, parents, graph = [], [], []
    for op in ops:
        if not isinstance(op, dict):
            add("fail", rel, "op entry must be a JSON object"); continue
        seq, name, status, key = op.get("seq"), op.get("op"), op.get("status"), op.get("item_key")
        if not isinstance(seq, int) or (seqs and seq <= seqs[-1]):
            add("fail", rel, f"op seq not strictly increasing at {seq}"); continue
        seqs.append(seq)
        if status != "acknowledged":
            continue
        if name == "project.verified":
            verified = seq
        elif name == "issue.create":
            creates.append((seq, key))
        elif name == "parent.record":
            parents.append(seq)
        elif name == "graph.persisted":
            graph.append(seq)
    for seq, key in creates:
        if verified is None or seq < verified:
            add("fail", rel, "issue.create acknowledged before primary Project verification")
    bound = set()
    for seq, key in creates:
        if key in bound:
            add("fail", rel, f"duplicate issue.create for {key}")
        bound.add(key)
    last_create = max((seq for seq, _ in creates), default=None)
    if state_name == "complete":
        for key in sorted(declared):
            if key not in bound:
                add("fail", rel, f"planned item {key} never created")
        if not graph:
            add("fail", rel, "cross-repository graph not persisted (graph.persisted missing)")
    if parents and last_create is not None and min(parents) <= last_create:
        add("fail", rel, "parent.record acknowledged before the last issue.create")
    if graph and last_create is not None and graph[-1] <= last_create:
        add("fail", rel, "graph.persisted acknowledged before the last issue.create")
    if state_name == "complete":
        records = doc.get("parent_records", [])
        if not isinstance(records, list):
            add("fail", rel, "parent_records must be a list"); records = []
        by_repo = {}
        for record in records:
            repo = record.get("repository") if isinstance(record, dict) else None
            if not isinstance(repo, str) or not repo:
                add("fail", rel, "parent record missing repository"); continue
            if repo in by_repo:
                add("fail", rel, f"more than one parent record for {repo}")
            by_repo[repo] = record
        for repo, members in sorted(local.items()):
            if not repo:
                continue
            expected = sorted(key for key, role in members if role in CHILD_ROLES)
            record = by_repo.get(repo)
            if record is None:
                if expected:
                    add("fail", rel, f"missing parent record for repository {repo}")
                continue
            if not expected:
                add("fail", rel, f"parent record for {repo} but the repository has no local children"); continue
            tracker = {key for key, role in members if role in ("tracker", "umbrella")}
            if record.get("parent") not in tracker:
                add("fail", rel, f"parent record for {repo} does not name the repository tracker")
            children = record.get("children")
            if not isinstance(children, list):
                add("fail", rel, f"parent record for {repo} missing children list"); continue
            missing = sorted(set(expected) - set(children))
            extra = sorted(set(children) - set(expected))
            if missing or extra:
                parts = []
                if missing: parts.append("missing " + ", ".join(missing))
                if extra: parts.append("unexpected " + ", ".join(extra))
                add("fail", rel, f"incomplete parent set for {repo}: " + "; ".join(parts))

def check_dry_run(rel, doc, txn_ids):
    if not isinstance(doc, dict):
        add("fail", rel, "dry-run plan must be a JSON object"); return
    caps = doc.get("capabilities")
    if caps is not None:
        if not isinstance(caps, dict):
            add("fail", rel, "dry-run capabilities must be an object")
        else:
            for name, value in sorted(caps.items()):
                if value not in TRI_STATE:
                    add("fail", rel, f"dry-run capability {name} is {value!r}; must be verified, unavailable or unknown")
    pid = doc.get("portfolio_id")
    if isinstance(pid, str) and pid in txn_ids:
        add("fail", rel, f"dry run left a portfolio transaction for {pid}")

def check_workflow_bodies():
    skills_root = os.path.join(root, "skills")
    if not os.path.isdir(skills_root):
        return
    for skill in WORKFLOW_SKILLS:
        skill_md = os.path.join(skills_root, skill, "SKILL.md")
        codex_md = os.path.join(skills_root, skill, "codex", "prompt.md")
        opencode_md = os.path.join(skills_root, skill, "opencode", "agent.md")
        if not os.path.isfile(skill_md):
            continue
        rel = os.path.relpath(skill_md, root)
        body = strip_frontmatter(open(skill_md, encoding="utf-8", errors="ignore").read())
        if os.path.isfile(codex_md) and open(codex_md, encoding="utf-8", errors="ignore").read() != body:
            add("fail", rel, "lock-step body diverges from codex/prompt.md")
        if os.path.isfile(opencode_md) and strip_frontmatter(open(opencode_md, encoding="utf-8", errors="ignore").read()) != body:
            add("fail", rel, "lock-step body diverges from opencode/agent.md")
        def first_line(needle, text=body):
            for index, line in enumerate(text.splitlines()):
                if needle in line:
                    return index
            return None
        apply_line = first_line("portfolio apply")
        create_line = first_line("gh issue create")
        if apply_line is not None and create_line is not None and apply_line > create_line:
            add("fail", rel, "portfolio provisioning after first gh issue create")
        elif apply_line is None and create_line is not None:
            add("warn", rel, "definition path files issues without portfolio provisioning")

for folder in [reports, state]:
    if not os.path.isdir(folder): continue
    for dirpath, _, files in os.walk(folder):
        for name in files:
            path = os.path.join(dirpath, name); rel = os.path.relpath(path, root)
            if name.endswith(".json"):
                try:
                    data = json.load(open(path, encoding="utf-8"))
                    if isinstance(data, dict) and not any(k in data for k in ["schema", "version"]):
                        findings.append({"severity": "warn", "file": rel, "summary": "JSON lacks schema/version"})
                    elif isinstance(data, dict) and data.get("schema") == TRANSACTION_SCHEMA:
                        transactions.append((rel, data))
                    elif isinstance(data, dict) and data.get("schema") == DRY_RUN_SCHEMA and data.get("dry_run") is True:
                        dry_runs.append((rel, data))
                except Exception as exc:
                    findings.append({"severity": "fail", "file": rel, "summary": f"invalid JSON: {exc}"})
            try:
                text = open(path, encoding="utf-8", errors="ignore").read()
            except OSError:
                continue
            if re.search(r"gh[pousr]_[A-Za-z0-9_]{20,}|-----BEGIN [A-Z ]*PRIVATE KEY-----", text):
                findings.append({"severity": "fail", "file": rel, "summary": "possible secret in generated artifact"})
            if root in text:
                findings.append({"severity": "warn", "file": rel, "summary": "absolute local path appears in artifact"})
for rel, doc in transactions:
    check_transaction(rel, doc)
txn_ids = {doc.get("portfolio_id") for _, doc in transactions if isinstance(doc, dict)}
for rel, doc in dry_runs:
    check_dry_run(rel, doc, txn_ids)
check_workflow_bodies()
status = "fail" if any(f["severity"] == "fail" for f in findings) else "warn" if findings else "pass"
report = {"schema": 1, "status": status, "findings": findings, "required_state": [".autospec/state", ".autospec/reports"],
          "portfolio": {"workflow_skills": list(WORKFLOW_SKILLS), "transactions": len(transactions), "dry_run_reports": len(dry_runs)}}
json.dump(report, open(os.path.join(reports, "state-validation.json"), "w", encoding="utf-8"), indent=2, sort_keys=True); open(os.path.join(reports, "state-validation.json"), "a").write("\n")
rows = "\n".join(f"| {f['severity']} | `{f['file']}` | {f['summary']} |" for f in findings)
open(os.path.join(reports, "state-validation.md"), "w", encoding="utf-8").write("\n".join(["# Autospec State Validation", "", f"## Status\n\n**{status}**", "", "| Severity | File | Summary |", "| --- | --- | --- |", rows or "| pass | none | no findings |", ""]))
print(f"state validation: {status}")
sys.exit(0 if status in {"pass", "warn"} else 1)
PY
