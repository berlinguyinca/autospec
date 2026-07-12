#!/usr/bin/env bash
# Smoke coverage for scripts/ci-status-compare.sh.
# Exercises deterministic CI rot classification without hitting GitHub.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$ROOT/scripts/ci-status-compare.sh"
TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

write_json() {
    local path="$1"
    cat > "$path"
}

assert_json_field() {
    local json="$1" field="$2" expected="$3"
    local actual
    actual="$(printf '%s' "$json" | jq -r "$field")"
    if [ "$actual" != "$expected" ]; then
        printf 'expected %s == %s, got %s\njson: %s\n' "$field" "$expected" "$actual" "$json" >&2
        exit 1
    fi
}

write_json "$TMPDIR/base-inherited.json" <<'JSON'
[
  {"name":"unit", "conclusion":"FAILURE", "detailsUrl":"https://ci.example/base/unit"},
  {"name":"lint", "conclusion":"SUCCESS", "detailsUrl":"https://ci.example/base/lint"}
]
JSON
write_json "$TMPDIR/head-inherited.json" <<'JSON'
[
  {"name":"unit", "conclusion":"FAILURE", "detailsUrl":"https://ci.example/head/unit"},
  {"name":"lint", "conclusion":"SUCCESS", "detailsUrl":"https://ci.example/head/lint"}
]
JSON

inherited="$(bash "$SCRIPT" --head "$TMPDIR/head-inherited.json" --base "$TMPDIR/base-inherited.json")"
assert_json_field "$inherited" '.classification' 'inherited'
assert_json_field "$inherited" '.target_url' 'https://ci.example/head/unit'
assert_json_field "$inherited" '.blocked_inherited[0].name' 'unit'
assert_json_field "$inherited" '.blocked_branch | length' '0'

write_json "$TMPDIR/base-clean.json" <<'JSON'
[
  {"name":"unit", "conclusion":"SUCCESS", "detailsUrl":"https://ci.example/base/unit"},
  {"name":"lint", "conclusion":"SUCCESS", "detailsUrl":"https://ci.example/base/lint"}
]
JSON
write_json "$TMPDIR/head-branch-caused.json" <<'JSON'
[
  {"name":"unit", "conclusion":"FAILURE", "detailsUrl":"https://ci.example/head/unit"},
  {"name":"lint", "conclusion":"SUCCESS", "detailsUrl":"https://ci.example/head/lint"}
]
JSON

branch_caused="$(bash "$SCRIPT" --head "$TMPDIR/head-branch-caused.json" --base "$TMPDIR/base-clean.json")"
assert_json_field "$branch_caused" '.classification' 'branch_caused'
assert_json_field "$branch_caused" '.target_url' 'https://ci.example/head/unit'
assert_json_field "$branch_caused" '.blocked_branch[0].name' 'unit'
assert_json_field "$branch_caused" '.blocked_inherited | length' '0'

write_json "$TMPDIR/head-mergeable.json" <<'JSON'
[
  {"name":"unit", "conclusion":"SUCCESS", "detailsUrl":"https://ci.example/head/unit"},
  {"name":"lint", "conclusion":"SUCCESS", "detailsUrl":"https://ci.example/head/lint"}
]
JSON

mergeable="$(bash "$SCRIPT" --head "$TMPDIR/head-mergeable.json" --base "$TMPDIR/base-clean.json")"
assert_json_field "$mergeable" '.classification' 'mergeable'
assert_json_field "$mergeable" '.target_url' 'null'
assert_json_field "$mergeable" '.blocked_branch | length' '0'
assert_json_field "$mergeable" '.blocked_inherited | length' '0'

printf 'ci-status-compare smoke: ok\n'
