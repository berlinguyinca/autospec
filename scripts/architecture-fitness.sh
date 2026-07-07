#!/usr/bin/env bash
# scripts/architecture-fitness.sh — declarative architecture fitness-function runner.
set -euo pipefail

usage() {
    cat <<'USAGE'
Usage:
  scripts/architecture-fitness.sh run [--registry FILE] [--repo DIR] [--json] [--emit-issues DIR] [--file-issues]

Runs architecture fitness functions declared in .autospec/architecture-fitness.yml.
A gated breach exits non-zero and can emit an auto-implement issue body with the
breached metric and exact locations.
USAGE
}

cmd="${1:-}"
if [ -z "$cmd" ] || [ "$cmd" = "--help" ] || [ "$cmd" = "-h" ]; then
    usage
    exit 0
fi
shift || true
if [ "$cmd" != "run" ]; then
    usage >&2
    exit 2
fi

repo="$(pwd)"
registry=""
json=0
emit_issues=""
file_issues=0
while [ "$#" -gt 0 ]; do
    case "$1" in
        --registry) registry="${2:?--registry requires a file}"; shift 2 ;;
        --repo) repo="${2:?--repo requires a directory}"; shift 2 ;;
        --json) json=1; shift ;;
        --emit-issues) emit_issues="${2:?--emit-issues requires a directory}"; shift 2 ;;
        --file-issues) file_issues=1; shift ;;
        --help|-h) usage; exit 0 ;;
        *) printf 'architecture-fitness: unknown argument: %s\n' "$1" >&2; usage >&2; exit 2 ;;
    esac
done

if [ -z "$registry" ]; then
    registry="$repo/.autospec/architecture-fitness.yml"
fi
[ -f "$registry" ] || { printf 'architecture-fitness: registry not found: %s\n' "$registry" >&2; exit 2; }
[ -d "$repo" ] || { printf 'architecture-fitness: repo not found: %s\n' "$repo" >&2; exit 2; }

python3 - "$repo" "$registry" "$json" "$emit_issues" "$file_issues" <<'PY'
import fnmatch
import json
import os
import re
import subprocess
import sys
import time
from pathlib import Path

repo = Path(sys.argv[1]).resolve()
registry_path = Path(sys.argv[2]).resolve()
json_mode = sys.argv[3] == "1"
emit_issues = sys.argv[4]
file_issues = sys.argv[5] == "1"


def strip_quotes(value):
    value = value.strip()
    if (value.startswith("'") and value.endswith("'")) or (value.startswith('"') and value.endswith('"')):
        return value[1:-1]
    return value


def parse_scalar(value):
    value = strip_quotes(value)
    if value == "true":
        return True
    if value == "false":
        return False
    if re.fullmatch(r"-?[0-9]+", value):
        return int(value)
    if value.startswith("[") and value.endswith("]"):
        inner = value[1:-1].strip()
        if not inner:
            return []
        return [strip_quotes(part.strip()) for part in inner.split(",")]
    return value


def parse_registry(path):
    functions = []
    current = None
    current_list_key = None
    current_map_key = None
    for raw in path.read_text(encoding="utf-8").splitlines():
        line = raw.split("#", 1)[0].rstrip()
        if not line.strip() or line.strip() == "fitness_functions:":
            continue
        stripped = line.strip()
        indent = len(line) - len(line.lstrip(" "))
        if stripped.startswith("- ") and indent == 2:
            if current:
                functions.append(current)
            current = {}
            current_list_key = None
            current_map_key = None
            item = stripped[2:]
            if item:
                key, value = item.split(":", 1)
                current[key.strip()] = parse_scalar(value.strip())
            continue
        if current is None:
            continue
        if stripped.startswith("- ") and current_list_key:
            current.setdefault(current_list_key, []).append(parse_scalar(stripped[2:].strip()))
            continue
        if ":" not in stripped:
            continue
        key, value = stripped.split(":", 1)
        key = key.strip()
        value = value.strip()
        if indent == 4 and value == "":
            # Either a list block (paths:) or a small nested map (issue:).
            if key == "issue":
                current.setdefault("issue", {})
                current_map_key = "issue"
                current_list_key = None
            else:
                current.setdefault(key, [])
                current_list_key = key
                current_map_key = None
            continue
        if indent >= 6 and current_map_key:
            current.setdefault(current_map_key, {})[key] = parse_scalar(value)
            continue
        current[key] = parse_scalar(value)
        current_list_key = None
        current_map_key = None
    if current:
        functions.append(current)
    return functions


def repo_files(patterns, exclude_patterns=None):
    exclude_patterns = exclude_patterns or []
    seen = set()
    for pattern in patterns:
        for path in repo.glob(pattern):
            if path.is_file():
                rel = path.relative_to(repo).as_posix()
                if any(fnmatch.fnmatch(rel, excluded) for excluded in exclude_patterns):
                    continue
                if rel not in seen:
                    seen.add(rel)
                    yield rel, path


def run_forbidden_pattern(ff):
    pattern = re.compile(str(ff.get("pattern", "")))
    threshold = int(ff.get("threshold", 0))
    paths = ff.get("paths") or []
    exclude_paths = ff.get("exclude_paths") or []
    locations = []
    count = 0
    for rel, path in repo_files(paths, exclude_paths):
        try:
            lines = path.read_text(encoding="utf-8", errors="ignore").splitlines()
        except OSError:
            continue
        for line_no, text in enumerate(lines, 1):
            if pattern.search(text):
                count += 1
                locations.append({"path": rel, "line": line_no, "excerpt": text.strip()[:160]})
    return count <= threshold, count, locations


def run_command_max_ms(ff):
    command = str(ff.get("command", "true"))
    threshold = int(ff.get("threshold", 0))
    started = time.perf_counter()
    completed = subprocess.run(command, cwd=repo, shell=True, text=True, capture_output=True)
    elapsed_ms = int(round((time.perf_counter() - started) * 1000))
    locations = []
    if completed.returncode != 0:
        locations.append({"path": "<command>", "line": 1, "excerpt": completed.stderr.strip()[:160]})
    return completed.returncode == 0 and elapsed_ms <= threshold, elapsed_ms, locations


def issue_body(ff, result):
    issue = ff.get("issue") or {}
    labels = issue.get("labels") or ["auto-implement", "architecture-fitness"]
    if isinstance(labels, str):
        labels = [labels]
    locations = result.get("locations") or []
    loc_lines = "\n".join(
        f"- `{loc['path']}:{loc['line']}` — {loc.get('excerpt','')}" for loc in locations[:25]
    ) or "- `<metric>` — threshold drift without a source location"
    return f"""## Goal

Restore architecture fitness function `{ff['id']}` to green.

## Breach

- metric: {ff.get('metric', ff['id'])}
- observed: {result['observed']}
- threshold: {ff.get('threshold')}
- gate: {str(ff.get('gate', True)).lower()}

## Locations

{loc_lines}

## Labels

{', '.join(labels)}

## Acceptance criteria

- [ ] `{ff['id']}` passes via `bash scripts/architecture-fitness.sh run --registry .autospec/architecture-fitness.yml`.
- [ ] The offending metric is at or below threshold `{ff.get('threshold')}`.
"""


def maybe_emit_issue(ff, result):
    if not emit_issues:
        return None
    out_dir = Path(emit_issues)
    out_dir.mkdir(parents=True, exist_ok=True)
    path = out_dir / f"{ff['id']}.md"
    body = issue_body(ff, result)
    path.write_text(body, encoding="utf-8")
    if file_issues:
        title = (ff.get("issue") or {}).get("title") or f"fix: restore architecture fitness gate {ff['id']}"
        labels = (ff.get("issue") or {}).get("labels") or ["auto-implement", "architecture-fitness"]
        if isinstance(labels, str):
            labels = [labels]
        cmd = ["gh", "issue", "create", "--title", title, "--body-file", str(path)]
        for label in labels:
            cmd.extend(["--label", label])
        subprocess.run(cmd, cwd=repo, check=False)
    return str(path)

functions = parse_registry(registry_path)
results = []
failed_gates = 0
for ff in functions:
    ff_type = ff.get("type")
    if ff_type == "forbidden_pattern":
        passed, observed, locations = run_forbidden_pattern(ff)
    elif ff_type == "command_max_ms":
        passed, observed, locations = run_command_max_ms(ff)
    else:
        passed, observed, locations = False, "unknown_type", [{"path":"<registry>", "line":1, "excerpt":str(ff_type)}]
    result = {
        "id": ff.get("id"),
        "name": ff.get("name"),
        "type": ff_type,
        "gate": bool(ff.get("gate", True)),
        "threshold": ff.get("threshold"),
        "metric": ff.get("metric", ff.get("id")),
        "observed": observed,
        "passed": bool(passed),
        "locations": locations,
    }
    if not passed and result["gate"]:
        failed_gates += 1
        issue_path = maybe_emit_issue(ff, result)
        if issue_path:
            result["issue_body"] = issue_path
    results.append(result)

summary = {
    "total": len(results),
    "passed": sum(1 for r in results if r["passed"]),
    "failed": sum(1 for r in results if not r["passed"]),
    "failed_gates": failed_gates,
}
payload = {"registry": str(registry_path), "summary": summary, "results": results}
if json_mode:
    print(json.dumps(payload, indent=2, sort_keys=True))
else:
    for result in results:
        status = "PASS" if result["passed"] else "FAIL"
        print(f"{status} {result['id']} metric={result['metric']} observed={result['observed']} threshold={result['threshold']}")
    print(f"architecture-fitness: total={summary['total']} failed_gates={summary['failed_gates']}")
sys.exit(1 if failed_gates else 0)
PY
