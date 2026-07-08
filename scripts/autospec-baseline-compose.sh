#!/usr/bin/env bash
# scripts/autospec-baseline-compose.sh — compose local Baseline profiles.
set -eu

usage() {
  cat <<'EOF'
Usage:
  autospec-baseline-compose.sh [--repo-root <dir>] [--config <path>]

Writes:
  .autospec/reports/baseline-composition.json
  .autospec/reports/baseline-composition.md
  .autospec/state/baseline-composition.json
EOF
}

die() { printf 'autospec-baseline-compose: %s\n' "$*" >&2; exit 2; }

REPO_ROOT="$(pwd)"
CONFIG_PATH=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --repo-root) [ "$#" -ge 2 ] || die "--repo-root requires a value"; REPO_ROOT="$2"; shift 2 ;;
    --config) [ "$#" -ge 2 ] || die "--config requires a value"; CONFIG_PATH="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown arg: $1" ;;
  esac
done

[ -d "$REPO_ROOT" ] || die "--repo-root does not exist: $REPO_ROOT"
REPO_ROOT="$(cd "$REPO_ROOT" && pwd -P)"
[ -n "$CONFIG_PATH" ] || CONFIG_PATH="$REPO_ROOT/.autospec/autospec.yml"
[ "${CONFIG_PATH#/}" != "$CONFIG_PATH" ] || CONFIG_PATH="$REPO_ROOT/$CONFIG_PATH"

python3 - "$REPO_ROOT" "$CONFIG_PATH" <<'PY'
import datetime, json, os, re, sys
try:
    import yaml
except Exception as exc:
    print(f"autospec-baseline-compose: python yaml module is required: {exc}")
    sys.exit(2)

repo_root = os.path.realpath(sys.argv[1]); config_path = os.path.realpath(sys.argv[2])
reports_dir = os.path.join(repo_root, ".autospec", "reports")
state_dir = os.path.join(repo_root, ".autospec", "state")
os.makedirs(reports_dir, exist_ok=True); os.makedirs(state_dir, exist_ok=True)

def now_iso(): return datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
def load_yaml(path):
    with open(path, encoding="utf-8") as fh:
        data = yaml.safe_load(fh)
    return data if isinstance(data, dict) else {}
def resolve(path):
    return os.path.realpath(path if os.path.isabs(path) else os.path.join(repo_root, path))
def finding(code, message, action, severity="error", **extra):
    item = {"code": code, "severity": severity, "message": message, "action": action}; item.update(extra); return item
def as_list(v): return v if isinstance(v, list) else []
def item_id(item): return str(item.get("id", "")) if isinstance(item, dict) else str(item)
def item_value(item): return item.get("value", "") if isinstance(item, dict) else ""
def write(report):
    for path in [os.path.join(reports_dir, "baseline-composition.json"), os.path.join(state_dir, "baseline-composition.json")]:
        with open(path, "w", encoding="utf-8") as fh: json.dump(report, fh, indent=2, sort_keys=True); fh.write("\n")
    lines = ["# Baseline Composition", "", f"Status: **{report['status'].upper()}**", "", f"- Structured: {report.get('structured', False)}", f"- Requested profiles: {', '.join(report.get('baselines', {}).get('requested_profiles', []))}", "", "## Included Packs"]
    lines += [f"- `{p['id']}` from `{p.get('path','')}`" for p in report.get("included_packs", [])] or ["- None."]
    lines += ["", "## Effective Capabilities"]
    lines += [f"- `{c['id']}` from `{c.get('pack') or c.get('profile')}`" for c in report.get("composed", {}).get("capabilities", [])] or ["- None."]
    lines += ["", "## Effective Rules"]
    lines += [f"- `{r['rule_id']}` from `{r.get('pack','')}`" for r in report.get("composed", {}).get("rules", [])] or ["- None."]
    lines += ["", "## Quality Gates"]
    lines += [f"- {g}" for g in report.get("composed", {}).get("quality_gates", [])] or ["- None."]
    lines += ["", "## Findings"]
    lines += [f"- `{f['code']}`: {f['message']}. Action: {f['action']}" for f in report.get("findings", [])] or ["- None."]
    with open(os.path.join(reports_dir, "baseline-composition.md"), "w", encoding="utf-8") as fh: fh.write("\n".join(lines) + "\n")
def finish(report):
    write(report); print(f"baseline composition: {report['status'].upper()}"); [print(f"- {f['message']}\n  action: {f['action']}") for f in report.get("findings", [])]; sys.exit(1 if report["status"] == "fail" else 0)

if not os.path.exists(config_path):
    finish({"version": 1, "generated_at": now_iso(), "status": "fail", "findings": [finding("CONFIG_MISSING", "missing .autospec/autospec.yml", "Create .autospec/autospec.yml.")]})
try:
    cfg = load_yaml(config_path)
except Exception as exc:
    finish({"version": 1, "generated_at": now_iso(), "status": "fail", "findings": [finding("CONFIG_PARSE_ERROR", "failed to parse .autospec/autospec.yml", f"Fix YAML syntax: {exc}")]})

bcfg = cfg.get("baselines") or {}
source = bcfg.get("source")
path = resolve(bcfg.get("path", ""))
requested = [str(p) for p in as_list(bcfg.get("profiles"))]
baselines_report = {"source": source, "path": path, "requested_profiles": requested}
findings = []
if source != "local":
    findings.append(finding("BASELINES_SOURCE_UNSUPPORTED", "baselines.source must be local", "Set baselines.source: local."))
if not os.path.isdir(path):
    findings.append(finding("BASELINES_PATH_MISSING", "baselines path does not exist", "Fix baselines.path."))
if findings:
    finish({"version": 1, "generated_at": now_iso(), "status": "fail", "baselines": baselines_report, "findings": findings, "composed": {"capabilities": [], "requirements": [], "dependencies": [], "rules": [], "quality_gates": [], "issue_templates": []}})

profile_name_re = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]*$")
for profile in requested:
    if not profile_name_re.match(profile):
        findings.append(finding("UNSUPPORTED_PROFILE_NAME", f"unsupported baseline profile name: {profile}", "Use only letters, numbers, dot, underscore, and hyphen.", profile=profile))

structured = os.path.exists(os.path.join(path, "manifests", "baselines.yml")) and os.path.exists(os.path.join(path, "manifests", "profiles.yml"))
composed = {"capabilities": [], "requirements": [], "dependencies": [], "rules": [], "quality_gates": [], "issue_templates": [], "metadata_required": []}
loaded_profiles = []; included_packs = []

if structured and not findings:
    manifest = load_yaml(os.path.join(path, "manifests", "baselines.yml"))
    profiles_manifest = load_yaml(os.path.join(path, "manifests", "profiles.yml"))
    pack_files = {p.get("id"): p.get("file") for p in as_list(manifest.get("packs")) if isinstance(p, dict)}
    profile_packs = {p.get("id"): as_list(p.get("packs")) for p in as_list(profiles_manifest.get("profiles")) if isinstance(p, dict)}
    ordered = []
    def add_pack(pid, reason="selected"):
        if pid and pid not in ordered: ordered.append(pid)
    for profile in requested:
        if profile not in profile_packs:
            findings.append(finding("BASELINE_PROFILE_MISSING", f"requested baseline profile is missing: {profile}", "Add the profile or remove it from config.", profile=profile)); continue
        loaded_profiles.append({"id": profile, "path": "manifests/profiles.yml", "declared_id": profile})
        for pid in profile_packs[profile]: add_pack(pid)
    pack_data = {}
    changed = True
    while changed:
        changed = False
        for pid in list(ordered):
            rel = pack_files.get(pid)
            if not rel or not os.path.exists(os.path.join(path, rel)):
                findings.append(finding("BASELINE_PACK_MISSING", f"baseline pack is missing: {pid}", "Add the pack file or fix manifests/baselines.yml.", pack=pid)); continue
            if pid not in pack_data: pack_data[pid] = load_yaml(os.path.join(path, rel)); pack_data[pid]["_path"] = rel
            for dep in as_list(pack_data[pid].get("inherits")) + as_list(pack_data[pid].get("requires")):
                composed["dependencies"].append({"profile": pid, "dependency": dep})
                if dep not in pack_files:
                    findings.append(finding("MISSING_PROFILE_DEPENDENCY", f"profile dependency is missing: {pid} depends on {dep}", "Add the required pack to manifests/baselines.yml.", profile=pid, dependency=dep))
                elif dep not in ordered:
                    ordered.append(dep); changed = True
    for pid in sorted(ordered):
        data = pack_data.get(pid) or {}
        included_packs.append({"id": pid, "path": data.get("_path", "")})
        caps = data.get("capabilities") if isinstance(data.get("capabilities"), dict) else {}
        for kind in ["required", "recommended"]:
            for cap in as_list(caps.get(kind)):
                composed["capabilities"].append({"id": re.sub(r"[^a-z0-9]+", "-", str(cap).lower()).strip("-"), "title": str(cap), "pack": pid, "kind": kind})
        for rule in as_list(data.get("rules")):
            if isinstance(rule, dict) and (rule.get("rule_id") or rule.get("id")):
                composed["rules"].append({"rule_id": rule.get("rule_id") or rule.get("id"), "pack": pid})
        composed["quality_gates"].extend([str(g) for g in as_list(data.get("quality_gates"))])
        composed["issue_templates"].extend([{"pack": pid, **t} for t in as_list(data.get("issue_templates")) if isinstance(t, dict)])
        composed["metadata_required"].extend([str(m) for m in as_list(data.get("metadata_required"))])
else:
    def candidates(profile):
        return [os.path.join(path, "profiles", profile, "baseline.yml"), os.path.join(path, "profiles", profile, "profile.yml"), os.path.join(path, "profiles", f"{profile}.yml"), os.path.join(path, f"{profile}.yml")]
    requested_index = {p: i for i, p in enumerate(requested)}
    for profile in requested:
        doc = next((c for c in candidates(profile) if os.path.isfile(c)), "")
        if not doc:
            findings.append(finding("BASELINE_PROFILE_MISSING", f"requested baseline profile is missing: {profile}", f"Add profiles/{profile}/baseline.yml or remove {profile}.", profile=profile)); continue
        data = load_yaml(doc); loaded_profiles.append({"id": profile, "path": doc, "declared_id": str(data.get("id", ""))})
        for dep in as_list(data.get("depends_on")):
            composed["dependencies"].append({"profile": profile, "dependency": str(dep)})
            if dep not in requested_index: findings.append(finding("MISSING_PROFILE_DEPENDENCY", f"profile dependency is missing: {profile} depends on {dep}", f"Add {dep} before {profile}.", profile=profile, dependency=dep))
            elif requested_index[dep] > requested_index[profile]: findings.append(finding("PROFILE_ORDER_PROBLEM", f"profile dependency is requested after dependent: {profile} depends on {dep}", f"Move {dep} before {profile}.", profile=profile, dependency=dep))
        for cap in as_list(data.get("capabilities")):
            cid = item_id(cap); composed["capabilities"].append({"id": cid, "profile": profile, "description": cap.get("description", "") if isinstance(cap, dict) else ""}) if cid else None
        for req in as_list(data.get("requirements")):
            rid = item_id(req); composed["requirements"].append({"id": rid, "profile": profile, "value": item_value(req)}) if rid else None
    cap_sources = {}
    for cap in composed["capabilities"]: cap_sources.setdefault(cap["id"], []).append(cap.get("profile") or cap.get("pack"))
    for cid, sources in sorted(cap_sources.items()):
        if len(sources) > 1: findings.append(finding("DUPLICATE_CAPABILITY", f"duplicate capability id: {cid}", "Keep capability ID in one profile or rename one.", capability=cid, profiles=sources))
    req_values = {}
    for req in composed["requirements"]: req_values.setdefault(req["id"], {}).setdefault(str(req.get("value", "")), []).append(req.get("profile"))
    for rid, values in sorted(req_values.items()):
        if len(values) > 1: findings.append(finding("REQUIREMENT_CONFLICT", f"conflicting requirement value: {rid}", "Align requirement values.", requirement=rid, values=values))

status = "fail" if findings else "pass"
finish({"version": 2, "generated_at": now_iso(), "status": status, "structured": structured, "config_path": config_path, "baselines": baselines_report, "profiles": loaded_profiles, "included_packs": included_packs, "composed": composed, "findings": findings})
PY
