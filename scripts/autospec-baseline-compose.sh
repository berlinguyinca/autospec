#!/usr/bin/env bash
# scripts/autospec-baseline-compose.sh — compose local Baseline profiles.
#
# Local filesystem only. Reads requested profiles from .autospec/autospec.yml,
# loads matching baseline profile documents from a local autospec-baselines path,
# and writes composition reports. No GitHub fetch, issue generation, gap
# analysis, or implementation planning.

set -eu

usage() {
    cat <<'EOF'
Usage:
  autospec-baseline-compose.sh [--repo-root <dir>] [--config <path>]

Writes:
  .autospec/reports/baseline-composition.json
  .autospec/reports/baseline-composition.md

Supported local profile document paths:
  <baselines>/profiles/<profile>/baseline.yml
  <baselines>/profiles/<profile>/profile.yml
  <baselines>/profiles/<profile>.yml
  <baselines>/<profile>.yml
EOF
}

die() {
    printf 'autospec-baseline-compose: %s\n' "$*" >&2
    exit 2
}

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

if [ -z "$CONFIG_PATH" ]; then
    CONFIG_PATH="$REPO_ROOT/.autospec/autospec.yml"
elif [ "${CONFIG_PATH#/}" = "$CONFIG_PATH" ]; then
    CONFIG_PATH="$REPO_ROOT/$CONFIG_PATH"
fi

python3 - "$REPO_ROOT" "$CONFIG_PATH" <<'PY'
import datetime
import glob
import json
import os
import re
import sys

try:
    import yaml
except Exception as exc:
    print(f"autospec-baseline-compose: python yaml module is required: {exc}")
    sys.exit(2)

repo_root = os.path.realpath(sys.argv[1])
config_path = os.path.realpath(sys.argv[2])
reports_dir = os.path.join(repo_root, ".autospec", "reports")
json_report = os.path.join(reports_dir, "baseline-composition.json")
md_report = os.path.join(reports_dir, "baseline-composition.md")
profile_name_re = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]*$")


def now_iso():
    return datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def ensure_reports_dir():
    os.makedirs(reports_dir, exist_ok=True)


def rel(path):
    try:
        return os.path.relpath(path, repo_root)
    except ValueError:
        return path


def source_type(value):
    if isinstance(value, str):
        return value
    if isinstance(value, dict):
        return value.get("type", "")
    return ""


def source_path(section, source_value):
    if isinstance(source_value, dict):
        return source_value.get("path") or section.get("path") or ""
    return section.get("path") or ""


def resolve_local(path_text):
    if not path_text:
        return ""
    if os.path.isabs(path_text):
        return os.path.realpath(path_text)
    return os.path.realpath(os.path.join(repo_root, path_text))


def finding(code, message, action, severity="error", profile=None, path=None, **extra):
    item = {"code": code, "severity": severity, "message": message, "action": action}
    if profile is not None:
        item["profile"] = profile
    if path is not None:
        item["path"] = path
    item.update(extra)
    return item


def write_reports(status, baselines=None, profiles=None, composed=None, findings=None):
    ensure_reports_dir()
    baselines = baselines or {}
    profiles = profiles or []
    composed = composed or {"capabilities": [], "requirements": [], "dependencies": []}
    findings = findings or []
    report = {
        "version": 1,
        "generated_at": now_iso(),
        "status": status,
        "config_path": config_path,
        "baselines": baselines,
        "profiles": profiles,
        "composed": composed,
        "findings": findings,
    }
    with open(json_report, "w", encoding="utf-8") as fh:
        json.dump(report, fh, indent=2, sort_keys=True)
        fh.write("\n")

    lines = [
        "# Baseline Composition",
        "",
        f"Status: **{status.upper()}**",
        "",
        f"- Config: `{config_path}`",
    ]
    if baselines.get("path"):
        lines.append(f"- Baselines: `{baselines['path']}`")
    requested = baselines.get("requested_profiles") or []
    if requested:
        lines.append(f"- Requested profiles: {', '.join(requested)}")
    lines.extend(["", "## Loaded Profiles", ""])
    if profiles:
        for profile in profiles:
            lines.append(f"- `{profile['id']}` from `{profile['path']}`")
    else:
        lines.append("- None.")
    lines.extend(["", "## Composed Capabilities", ""])
    if composed.get("capabilities"):
        for cap in composed["capabilities"]:
            lines.append(f"- `{cap['id']}` from `{cap['profile']}`")
    else:
        lines.append("- None.")
    lines.extend(["", "## Composed Requirements", ""])
    if composed.get("requirements"):
        for req in composed["requirements"]:
            lines.append(f"- `{req['id']}` = `{req.get('value', '')}` from `{req['profile']}`")
    else:
        lines.append("- None.")
    lines.extend(["", "## Findings", ""])
    if findings:
        for item in findings:
            profile = f" Profile: `{item['profile']}`." if item.get("profile") else ""
            detail = ""
            if item.get("capability"):
                detail += f" Capability: `{item['capability']}`."
            if item.get("requirement"):
                detail += f" Requirement: `{item['requirement']}`."
            if item.get("dependency"):
                detail += f" Dependency: `{item['dependency']}`."
            lines.append(f"- `{item['code']}`: {item['message']}.{profile}{detail} Action: {item['action']}")
    else:
        lines.append("- None.")
    with open(md_report, "w", encoding="utf-8") as fh:
        fh.write("\n".join(lines))
        fh.write("\n")
    return report


def print_summary(status, findings):
    print(f"baseline composition: {status.upper()}")
    for item in findings:
        print(f"- {item['message']}")
        print(f"  action: {item['action']}")
    print(f"reports: {rel(json_report)}, {rel(md_report)}")


def load_yaml_file(path):
    with open(path, "r", encoding="utf-8") as fh:
        data = yaml.safe_load(fh)
    return data if isinstance(data, dict) else {}


def profile_candidates(root, profile):
    return [
        os.path.join(root, "profiles", profile, "baseline.yml"),
        os.path.join(root, "profiles", profile, "baseline.yaml"),
        os.path.join(root, "profiles", profile, "profile.yml"),
        os.path.join(root, "profiles", profile, "profile.yaml"),
        os.path.join(root, "profiles", f"{profile}.yml"),
        os.path.join(root, "profiles", f"{profile}.yaml"),
        os.path.join(root, f"{profile}.yml"),
        os.path.join(root, f"{profile}.yaml"),
    ]


def find_profile_doc(root, profile):
    for candidate in profile_candidates(root, profile):
        if os.path.isfile(candidate):
            return os.path.realpath(candidate)
    return ""


def as_list(value):
    return value if isinstance(value, list) else []


def item_id(item):
    if isinstance(item, dict):
        return str(item.get("id", ""))
    return str(item)


def item_value(item):
    if isinstance(item, dict):
        return item.get("value", "")
    return ""


empty_composed = {"capabilities": [], "requirements": [], "dependencies": []}

if not os.path.exists(config_path):
    findings = [finding("CONFIG_MISSING", "missing .autospec/autospec.yml", "Create .autospec/autospec.yml or pass --config with a readable config path.", path=config_path)]
    write_reports("error", findings=findings)
    print_summary("error", findings)
    sys.exit(2)

try:
    config = load_yaml_file(config_path)
except Exception as exc:
    findings = [finding("CONFIG_PARSE_ERROR", "failed to parse .autospec/autospec.yml", f"Fix the YAML syntax error: {exc}", path=config_path)]
    write_reports("error", findings=findings)
    print_summary("error", findings)
    sys.exit(2)

baselines_cfg = config.get("baselines") or {}
if not isinstance(baselines_cfg, dict):
    findings = [finding("BASELINES_CONFIG_MALFORMED", "baselines config must be a mapping", "Use baselines.source, baselines.path, and baselines.profiles.", path=config_path)]
    write_reports("error", findings=findings)
    print_summary("error", findings)
    sys.exit(2)

baselines_source = baselines_cfg.get("source")
baselines_source_type = source_type(baselines_source)
baselines_path = resolve_local(source_path(baselines_cfg, baselines_source))
requested_profiles = baselines_cfg.get("profiles") or []
if not isinstance(requested_profiles, list):
    requested_profiles = []
    initial_findings = [finding("BASELINE_PROFILES_MALFORMED", "baselines.profiles must be a list", "Set baselines.profiles to a YAML list of profile IDs.", path=config_path)]
else:
    initial_findings = []
requested_profiles = [str(profile) for profile in requested_profiles]
baselines_report = {
    "source": baselines_source_type,
    "path": baselines_path,
    "requested_profiles": requested_profiles,
}

findings = list(initial_findings)
loaded_profiles = []
composed = {"capabilities": [], "requirements": [], "dependencies": []}

if baselines_source_type != "local":
    findings.append(finding(
        "BASELINES_SOURCE_UNSUPPORTED",
        "baselines.source must be local",
        "GitHub sources are not supported by this composer; set baselines.source: local.",
        path=config_path,
    ))
elif not os.path.isdir(baselines_path):
    findings.append(finding(
        "BASELINES_PATH_MISSING",
        "baselines path does not exist",
        "Create the directory or update baselines.path in .autospec/autospec.yml.",
        path=baselines_path,
    ))
else:
    schemas_dir = os.path.join(baselines_path, "schemas")
    if not (os.path.isdir(schemas_dir) and glob.glob(os.path.join(schemas_dir, "*.schema.json"))):
        findings.append(finding(
            "BASELINE_SCHEMAS_MISSING",
            "baseline schemas are missing",
            "Add at least one *.schema.json file under schemas/.",
            path=schemas_dir,
        ))

    requested_index = {profile: idx for idx, profile in enumerate(requested_profiles)}
    for profile in requested_profiles:
        if not profile_name_re.match(profile):
            findings.append(finding(
                "UNSUPPORTED_PROFILE_NAME",
                f"unsupported baseline profile name: {profile}",
                "Use only letters, numbers, dot, underscore, and hyphen; do not use path segments.",
                profile=profile,
            ))
            continue
        doc_path = find_profile_doc(baselines_path, profile)
        if not doc_path:
            findings.append(finding(
                "BASELINE_PROFILE_MISSING",
                f"requested baseline profile is missing: {profile}",
                f"Add profiles/{profile}/baseline.yml or remove {profile} from baselines.profiles.",
                profile=profile,
            ))
            continue
        try:
            data = load_yaml_file(doc_path)
        except Exception as exc:
            findings.append(finding(
                "BASELINE_PROFILE_PARSE_ERROR",
                f"failed to parse baseline profile: {profile}",
                f"Fix the YAML syntax error: {exc}",
                profile=profile,
                path=doc_path,
            ))
            continue
        loaded_profiles.append({"id": profile, "path": doc_path, "declared_id": str(data.get("id", ""))})
        for dep in as_list(data.get("depends_on")):
            dep = str(dep)
            composed["dependencies"].append({"profile": profile, "dependency": dep})
            if dep not in requested_index:
                findings.append(finding(
                    "MISSING_PROFILE_DEPENDENCY",
                    f"profile dependency is missing: {profile} depends on {dep}",
                    f"Add {dep} before {profile} in baselines.profiles or remove the dependency.",
                    profile=profile,
                    dependency=dep,
                ))
            elif requested_index[dep] > requested_index[profile]:
                findings.append(finding(
                    "PROFILE_ORDER_PROBLEM",
                    f"profile dependency is requested after dependent: {profile} depends on {dep}",
                    f"Move {dep} before {profile} in baselines.profiles.",
                    profile=profile,
                    dependency=dep,
                ))
        for cap in as_list(data.get("capabilities")):
            cap_id = item_id(cap)
            if not cap_id:
                continue
            description = cap.get("description", "") if isinstance(cap, dict) else ""
            composed["capabilities"].append({"id": cap_id, "profile": profile, "description": description})
        for req in as_list(data.get("requirements")):
            req_id = item_id(req)
            if not req_id:
                continue
            composed["requirements"].append({"id": req_id, "profile": profile, "value": item_value(req)})

    capability_sources = {}
    for cap in composed["capabilities"]:
        capability_sources.setdefault(cap["id"], []).append(cap["profile"])
    for cap_id, sources in sorted(capability_sources.items()):
        if len(sources) > 1:
            findings.append(finding(
                "DUPLICATE_CAPABILITY",
                f"duplicate capability id: {cap_id}",
                "Keep a capability ID in one profile or rename one of the duplicates.",
                capability=cap_id,
                profiles=sources,
            ))

    requirement_values = {}
    for req in composed["requirements"]:
        requirement_values.setdefault(req["id"], {}).setdefault(str(req.get("value", "")), []).append(req["profile"])
    for req_id, values in sorted(requirement_values.items()):
        if len(values) > 1:
            findings.append(finding(
                "REQUIREMENT_CONFLICT",
                f"conflicting requirement value: {req_id}",
                "Align the requirement values or split the profiles so they are not composed together.",
                requirement=req_id,
                values=values,
            ))

status = "fail" if findings else "pass"
write_reports(status, baselines_report, loaded_profiles, composed, findings)
print_summary(status, findings)
sys.exit(1 if findings else 0)
PY
