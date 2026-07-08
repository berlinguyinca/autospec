#!/usr/bin/env bash
# scripts/autospec-constitution-validate.sh — local Constitution/Baseline validator.
#
# This first executable slice is intentionally local-filesystem only. It reads
# .autospec/autospec.yml, validates local Constitution/Baseline paths and files,
# and writes human-readable + machine-readable reports. It does not fetch GitHub
# sources, generate issues, run gap analysis, or produce metadata.

set -eu

usage() {
    cat <<'EOF'
Usage:
  autospec-constitution-validate.sh [--repo-root <dir>] [--config <path>]

Validates local Constitution/Baseline configuration from .autospec/autospec.yml
and writes:
  .autospec/reports/constitution-validation.json
  .autospec/reports/constitution-validation.md
EOF
}

die() {
    printf 'autospec-constitution-validate: %s\n' "$*" >&2
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
import sys

try:
    import yaml
except Exception as exc:
    print(f"autospec-constitution-validate: python yaml module is required: {exc}")
    sys.exit(2)

repo_root = os.path.realpath(sys.argv[1])
config_path = os.path.realpath(sys.argv[2])
reports_dir = os.path.join(repo_root, ".autospec", "reports")
json_report = os.path.join(reports_dir, "constitution-validation.json")
md_report = os.path.join(reports_dir, "constitution-validation.md")

required_constitution_files = ["README.md", "VISION.md", "CONSTITUTION.md"]


def now_iso():
    return datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def ensure_reports_dir():
    os.makedirs(reports_dir, exist_ok=True)


def rel(path):
    try:
        return os.path.relpath(path, repo_root)
    except ValueError:
        return path


def error(code, message, action, path=None, profile=None):
    entry = {"code": code, "message": message, "action": action}
    if path is not None:
        entry["path"] = path
    if profile is not None:
        entry["profile"] = profile
    return entry


def check(code, label, status, path=None, detail=None):
    entry = {"code": code, "label": label, "status": status}
    if path is not None:
        entry["path"] = path
    if detail is not None:
        entry["detail"] = detail
    return entry


def write_reports(status, config=None, constitution=None, baselines=None, checks=None, errors=None):
    ensure_reports_dir()
    checks = checks or []
    errors = errors or []
    report = {
        "version": 1,
        "generated_at": now_iso(),
        "status": status,
        "config_path": config_path,
        "constitution": constitution or {},
        "baselines": baselines or {},
        "checks": checks,
        "errors": errors,
    }
    with open(json_report, "w", encoding="utf-8") as fh:
        json.dump(report, fh, indent=2, sort_keys=True)
        fh.write("\n")

    title_status = status.upper()
    lines = [
        "# Constitution Validation",
        "",
        f"Status: **{title_status}**",
        "",
        f"- Config: `{config_path}`",
    ]
    if constitution and constitution.get("path"):
        lines.append(f"- Constitution: `{constitution['path']}`")
    if baselines and baselines.get("path"):
        lines.append(f"- Baselines: `{baselines['path']}`")
    profiles = (baselines or {}).get("profiles") or []
    if profiles:
        lines.append(f"- Profiles: {', '.join(f'`{p}`' for p in profiles)}")
    lines.extend(["", "## Checks", ""])
    if checks:
        for item in checks:
            marker = "PASS" if item["status"] == "pass" else "FAIL"
            path_part = f" (`{item['path']}`)" if item.get("path") else ""
            detail_part = f" — {item['detail']}" if item.get("detail") else ""
            lines.append(f"- **{marker}** {item['label']}{path_part}{detail_part}")
    else:
        lines.append("- No checks ran.")
    lines.extend(["", "## Errors", ""])
    if errors:
        for item in errors:
            path_part = f" Path: `{item['path']}`." if item.get("path") else ""
            profile_part = f" Profile: `{item['profile']}`." if item.get("profile") else ""
            lines.append(f"- `{item['code']}`: {item['message']}{path_part}{profile_part} Action: {item['action']}")
    else:
        lines.append("- None.")
    with open(md_report, "w", encoding="utf-8") as fh:
        fh.write("\n".join(lines))
        fh.write("\n")
    return report


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


def has_schema_file(root):
    return bool(glob.glob(os.path.join(root, "schemas", "*.schema.json")))


def profile_exists(root, profile):
    candidates = [
        os.path.join(root, "profiles", profile),
        os.path.join(root, "profiles", f"{profile}.yml"),
        os.path.join(root, "profiles", f"{profile}.yaml"),
        os.path.join(root, profile),
        os.path.join(root, f"{profile}.yml"),
        os.path.join(root, f"{profile}.yaml"),
    ]
    return any(os.path.exists(candidate) for candidate in candidates)


def print_summary(status, errors):
    print(f"constitution validation: {status.upper()}")
    if errors:
        for item in errors:
            path_part = f" ({item['path']})" if item.get("path") else ""
            print(f"- {item['message']}{path_part}")
            print(f"  action: {item['action']}")
    print(f"reports: {rel(json_report)}, {rel(md_report)}")


if not os.path.exists(config_path):
    err = error(
        "CONFIG_MISSING",
        "missing .autospec/autospec.yml",
        "Create .autospec/autospec.yml or pass --config with a readable config path.",
        config_path,
    )
    write_reports("error", errors=[err])
    print_summary("error", [err])
    sys.exit(2)

try:
    with open(config_path, "r", encoding="utf-8") as fh:
        config = yaml.safe_load(fh)
except Exception as exc:
    err = error(
        "CONFIG_PARSE_ERROR",
        "failed to parse .autospec/autospec.yml",
        f"Fix the YAML syntax error: {exc}",
        config_path,
    )
    write_reports("error", errors=[err])
    print_summary("error", [err])
    sys.exit(2)

if not isinstance(config, dict):
    err = error(
        "CONFIG_MALFORMED",
        ".autospec/autospec.yml must contain a YAML mapping",
        "Replace the config body with a YAML object containing constitution and baselines sections.",
        config_path,
    )
    write_reports("error", errors=[err])
    print_summary("error", [err])
    sys.exit(2)

checks = []
errors = []

constitution_cfg = config.get("constitution") or {}
baselines_cfg = config.get("baselines") or {}
if not isinstance(constitution_cfg, dict):
    constitution_cfg = {}
    errors.append(error("CONSTITUTION_CONFIG_MALFORMED", "constitution config must be a mapping", "Use constitution.source and constitution.path in .autospec/autospec.yml.", config_path))
if not isinstance(baselines_cfg, dict):
    baselines_cfg = {}
    errors.append(error("BASELINES_CONFIG_MALFORMED", "baselines config must be a mapping", "Use baselines.source, baselines.path, and baselines.profiles in .autospec/autospec.yml.", config_path))

constitution_source = constitution_cfg.get("source")
constitution_source_type = source_type(constitution_source)
constitution_path_text = source_path(constitution_cfg, constitution_source)
constitution_path = resolve_local(constitution_path_text)
constitution_report = {
    "source": constitution_source_type,
    "path": constitution_path,
    "version": constitution_cfg.get("version", ""),
}

if constitution_source_type != "local":
    errors.append(error(
        "CONSTITUTION_SOURCE_UNSUPPORTED",
        "constitution.source must be local",
        "GitHub sources are not supported by this validator; set constitution.source: local for this slice.",
        config_path,
    ))
else:
    if os.path.isdir(constitution_path):
        checks.append(check("CONSTITUTION_PATH_EXISTS", "constitution path exists", "pass", constitution_path))
        for filename in required_constitution_files:
            candidate = os.path.join(constitution_path, filename)
            if os.path.isfile(candidate):
                checks.append(check("CONSTITUTION_FILE_EXISTS", f"required constitution file exists: {filename}", "pass", candidate))
            else:
                checks.append(check("CONSTITUTION_FILE_EXISTS", f"required constitution file exists: {filename}", "fail", candidate))
                errors.append(error(
                    "CONSTITUTION_FILE_MISSING",
                    f"required constitution file is missing: {filename}",
                    f"Add {filename} to the Constitution repository root.",
                    candidate,
                ))
        doctrine_dir = os.path.join(constitution_path, "doctrine")
        if os.path.isdir(doctrine_dir) and glob.glob(os.path.join(doctrine_dir, "*.md")):
            checks.append(check("CONSTITUTION_DOCTRINE_EXISTS", "constitution doctrine docs exist", "pass", doctrine_dir))
        else:
            checks.append(check("CONSTITUTION_DOCTRINE_EXISTS", "constitution doctrine docs exist", "fail", doctrine_dir))
            errors.append(error(
                "CONSTITUTION_DOCTRINE_MISSING",
                "constitution required doctrine docs are missing",
                "Add at least one Markdown doctrine document under doctrine/.",
                doctrine_dir,
            ))
        schemas_dir = os.path.join(constitution_path, "schemas")
        if os.path.isdir(schemas_dir) and has_schema_file(constitution_path):
            checks.append(check("CONSTITUTION_SCHEMAS_EXIST", "constitution schemas exist", "pass", schemas_dir))
        else:
            checks.append(check("CONSTITUTION_SCHEMAS_EXIST", "constitution schemas exist", "fail", schemas_dir))
            errors.append(error(
                "CONSTITUTION_SCHEMAS_MISSING",
                "constitution schemas are missing",
                "Add at least one *.schema.json file under schemas/.",
                schemas_dir,
            ))
    else:
        checks.append(check("CONSTITUTION_PATH_EXISTS", "constitution path exists", "fail", constitution_path))
        errors.append(error(
            "CONSTITUTION_PATH_MISSING",
            "constitution path does not exist",
            "Create the directory or update constitution.path in .autospec/autospec.yml.",
            constitution_path,
        ))

baselines_source = baselines_cfg.get("source")
baselines_source_type = source_type(baselines_source)
baselines_path_text = source_path(baselines_cfg, baselines_source)
baselines_path = resolve_local(baselines_path_text)
profiles = baselines_cfg.get("profiles") or []
if not isinstance(profiles, list):
    profiles = []
    errors.append(error(
        "BASELINE_PROFILES_MALFORMED",
        "baselines.profiles must be a list",
        "Set baselines.profiles to a YAML list of profile IDs.",
        config_path,
    ))
profiles = [str(profile) for profile in profiles]
baselines_report = {
    "source": baselines_source_type,
    "path": baselines_path,
    "profiles": profiles,
}

if baselines_source_type != "local":
    errors.append(error(
        "BASELINES_SOURCE_UNSUPPORTED",
        "baselines.source must be local",
        "GitHub sources are not supported by this validator; set baselines.source: local for this slice.",
        config_path,
    ))
else:
    if os.path.isdir(baselines_path):
        checks.append(check("BASELINES_PATH_EXISTS", "baselines path exists", "pass", baselines_path))
        schemas_dir = os.path.join(baselines_path, "schemas")
        if os.path.isdir(schemas_dir) and has_schema_file(baselines_path):
            checks.append(check("BASELINE_SCHEMAS_EXIST", "baseline schemas exist", "pass", schemas_dir))
        else:
            checks.append(check("BASELINE_SCHEMAS_EXIST", "baseline schemas exist", "fail", schemas_dir))
            errors.append(error(
                "BASELINE_SCHEMAS_MISSING",
                "baseline schemas are missing",
                "Add at least one *.schema.json file under schemas/.",
                schemas_dir,
            ))
        for profile in profiles:
            if profile_exists(baselines_path, profile):
                checks.append(check("BASELINE_PROFILE_EXISTS", f"requested baseline profile exists: {profile}", "pass", baselines_path, profile))
            else:
                checks.append(check("BASELINE_PROFILE_EXISTS", f"requested baseline profile exists: {profile}", "fail", baselines_path, profile))
                errors.append(error(
                    "BASELINE_PROFILE_MISSING",
                    f"requested baseline profile is missing: {profile}",
                    f"Add profiles/{profile}/, profiles/{profile}.yml, or remove {profile} from baselines.profiles.",
                    baselines_path,
                    profile,
                ))
    else:
        checks.append(check("BASELINES_PATH_EXISTS", "baselines path exists", "fail", baselines_path))
        errors.append(error(
            "BASELINES_PATH_MISSING",
            "baselines path does not exist",
            "Create the directory or update baselines.path in .autospec/autospec.yml.",
            baselines_path,
        ))

status = "fail" if errors else "pass"
write_reports(status, config, constitution_report, baselines_report, checks, errors)
print_summary(status, errors)
sys.exit(1 if errors else 0)
PY
