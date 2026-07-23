#!/usr/bin/env bash
# autospec-boundary-guardrails.sh — deterministic boundary-truth checks.
set -eu

usage() {
    cat <<'EOF'
Usage:
  autospec-boundary-guardrails.sh scan --repo-root DIR
EOF
}

die() {
    printf 'autospec-boundary-guardrails: %s\n' "$*" >&2
    exit 2
}

cmd="${1:-}"
[ -n "$cmd" ] || { usage; exit 2; }
shift

repo_root="."
while [ "$#" -gt 0 ]; do
    case "$1" in
        --repo-root) repo_root="${2:-}"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown arg: $1" ;;
    esac
done

[ "$cmd" = "scan" ] || die "unknown command: $cmd"
[ -d "$repo_root" ] || die "--repo-root does not exist: $repo_root"

python3 - "$repo_root" <<'PY'
import ast
import json
import os
import re
import sys
from pathlib import Path

root = Path(sys.argv[1]).resolve()
findings = []


def rel(path: Path) -> str:
    return path.relative_to(root).as_posix()


def add(rule_id, path, message, evidence):
    findings.append({
        "rule_id": rule_id,
        "path": path,
        "message": message,
        "evidence": evidence,
    })


def text_files():
    ignored = {".git", "target", "node_modules", ".venv", "__pycache__"}
    for base, dirs, files in os.walk(root):
        dirs[:] = [d for d in dirs if d not in ignored]
        for name in files:
            path = Path(base) / name
            if path.suffix.lower() in {".py", ".js", ".ts", ".go", ".rs", ".java", ".scala", ".sql", ".md", ".yml", ".yaml", ".json"}:
                try:
                    yield path, path.read_text(encoding="utf-8")
                except UnicodeDecodeError:
                    continue


allow_values = {}
for path, source in text_files():
    if path.suffix != ".py":
        continue
    try:
        tree = ast.parse(source)
    except SyntaxError:
        continue
    for node in ast.walk(tree):
        if isinstance(node, ast.Assign):
            names = [target.id for target in node.targets if isinstance(target, ast.Name)]
            if not any(name.isupper() and ("DOMAIN" in name or "ALLOW" in name or "VALID" in name) for name in names):
                continue
            if isinstance(node.value, (ast.Set, ast.List, ast.Tuple)):
                values = [elt.value for elt in node.value.elts if isinstance(elt, ast.Constant) and isinstance(elt.value, str)]
                for value in values:
                    allow_values.setdefault(value, []).append(rel(path))

sql_values = set()
sql_paths = []
for path, source in text_files():
    if path.suffix != ".sql":
        continue
    for match in re.finditer(r"CHECK\s*\([^)]*?\bIN\s*\(([^)]*)\)", source, re.IGNORECASE | re.DOTALL):
        sql_paths.append(rel(path))
        for value in re.findall(r"'([^']+)'|\"([^\"]+)\"", match.group(1)):
            sql_values.add(value[0] or value[1])

if allow_values and sql_values:
    missing = sorted(set(allow_values) - sql_values)
    if missing:
        first = allow_values[missing[0]][0]
        add("CONTRACT_DRIFT", first, "code allow-list contains values missing from persistence check", {"missing_from_sql": missing, "sql_paths": sql_paths})

for path, source in text_files():
    if path.suffix not in {".py", ".js", ".ts", ".go"}:
        continue
    patterns = [
        r"except\s+[^:\n]*:\s*\n\s*return\s+None\b",
        r"except\s+[^:\n]*:\s*\n\s*return\s+nil\b",
        r"catch\s*\([^)]*\)\s*\{\s*(return\s+null;?|return\s+undefined;?|continue;?)\s*\}",
    ]
    if any(re.search(pattern, source, re.MULTILINE) for pattern in patterns):
        if not re.search(r"\b(log|logger|warn|warning|slog|console\.warn|console\.error)\b", source, re.IGNORECASE):
            add("SILENT_FAILURE", rel(path), "error branch returns success/empty result without logging or warning", {})

decode_sources = []
for path, source in text_files():
    if path.parts and "tests" in path.parts:
        continue
    if re.search(r"\b(json\.loads|JSON\.parse|json\.NewDecoder|serde_json::from|ObjectMapper)\b", source):
        decode_sources.append(rel(path))

if decode_sources:
    test_text = "\n".join(source for path, source in text_files() if "tests" in path.parts or path.name.startswith("test_"))
    has_fake = re.search(r"\b(Fake|Mock|Stub)\w*", test_text) is not None
    has_raw_boundary = re.search(r"json\.loads|JSON\.parse|httptest|responses|requests_mock|raw_payload|payload_json|response_body|fixture", test_text, re.DOTALL) is not None
    if has_fake and not has_raw_boundary:
        add("BOUNDARY_TEST_MISSING", decode_sources[0], "external decode path has typed fake tests but no raw payload boundary test", {"decode_sources": decode_sources})

# G4: an external integration marked complete needs a replayable response
# artifact.  The marker is intentionally lightweight so repositories can use
# issue/spec markdown without adopting a new manifest format; fixture files
# remain local and offline-safe.
integration_done = any(
    re.search(r"area\s*:\s*integration", source, re.IGNORECASE)
    and re.search(r"status\s*:\s*done|integration\s+(?:is\s+)?(?:complete|done)", source, re.IGNORECASE)
    for path, source in text_files()
)
external_sources = [
    (path, source) for path, source in text_files()
    if path.suffix in {".py", ".js", ".ts", ".go", ".rs", ".java", ".scala"}
    and re.search(r"https?://|requests\.|fetch\(|axios|http\.|HttpClient|ureq|reqwest", source, re.IGNORECASE)
]
replay_artifacts = [
    rel(path) for path, source in text_files()
    if "fixtures" in path.parts or "recordings" in path.parts or path.suffix.lower() in {".har"}
    or re.search(r"real[-_ ]?(?:response|payload)|recorded[-_ ]?(?:response|payload)", path.name, re.IGNORECASE)
]
if integration_done and external_sources and not replay_artifacts:
    add(
        "REAL_RESPONSE_EVIDENCE_MISSING",
        rel(external_sources[0][0]),
        "completed external integration has no replayable real-response fixture",
        {"integration_sources": [rel(path) for path, _ in external_sources]},
    )

print(json.dumps({"schema": "autospec-boundary-guardrails/v1", "findings": findings}, indent=2, sort_keys=True))
PY
