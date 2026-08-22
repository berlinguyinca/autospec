#!/usr/bin/env bats
# tests/unit/test_autonomy_stack.bats — target stack detection and confidence
# (scripts/autospec_autonomy_stack.py), extracted from autospec-autonomy-v2-lib.py.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    SCRIPTS="$REPO_ROOT/scripts"
    WORK="$(mktemp -d)"
}

teardown() {
    rm -rf "${WORK:?}"
}

run_py() {
    run python3 -c "
import sys
sys.path.insert(0, '$SCRIPTS')
import autospec_autonomy_stack as st
from pathlib import Path
root = Path('$WORK')
$1
"
}

@test "stack: react plus vite plus tsx detects react-vite-typescript at high confidence" {
    printf '%s\n' '{"dependencies":{"react":"1","vite":"1"}}' > "$WORK/package.json"
    printf 'x\n' > "$WORK/App.tsx"
    run_py "
st.detect_stack(root)
import json
d = json.loads((root / '.autospec/state/stack-profile.json').read_text())
print(d['primary_profile']['id'], d['primary_profile']['confidence'])
"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -Fqx 'react-vite-typescript 0.95'
}

@test "stack: next detects nextjs-web-app" {
    printf '%s\n' '{"dependencies":{"next":"1"}}' > "$WORK/package.json"
    run_py "
st.detect_stack(root)
import json
print(json.loads((root / '.autospec/state/stack-profile.json').read_text())['primary_profile']['id'])
"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -Fqx 'nextjs-web-app'
}

@test "stack: a python file alone detects python-cli-tool below the scaffold threshold" {
    printf 'print(1)\n' > "$WORK/main.py"
    run_py "
print(st.detect_stack(root), st.stack_confidence(root) < 0.8)
"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -Fqx '0 True'
}

@test "stack: an empty repo is unknown at 0.1 confidence, never absent" {
    run_py "
import json
st.detect_stack(root)
d = json.loads((root / '.autospec/state/stack-profile.json').read_text())
print(d['primary_profile']['id'], d['primary_profile']['confidence'], len(d['profiles']))
"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -Fqx 'unknown 0.1 1'
}

@test "stack: playwright is reported alongside the primary framework profile" {
    printf '%s\n' '{"dependencies":{"react":"1","vite":"1","typescript":"1"},"devDependencies":{"@playwright/test":"1"}}' > "$WORK/package.json"
    run_py "
import json
st.detect_stack(root)
ids = [p['id'] for p in json.loads((root / '.autospec/state/stack-profile.json').read_text())['profiles']]
print('playwright' in ids, 'react-vite-typescript' in ids)
"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -Fqx 'True True'
}

@test "stack: node_modules and .git are excluded from file evidence" {
    mkdir -p "$WORK/node_modules/pkg" "$WORK/.git"
    printf 'print(1)\n' > "$WORK/node_modules/pkg/setup.py"
    run_py "
import json
st.detect_stack(root)
print(json.loads((root / '.autospec/state/stack-profile.json').read_text())['primary_profile']['id'])
"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -Fqx 'unknown'
}

@test "stack: stack_confidence detects on demand when no profile exists yet" {
    printf '%s\n' '{"dependencies":{"next":"1"}}' > "$WORK/package.json"
    run_py "
c = st.stack_confidence(root)
print(c, (root / '.autospec/state/stack-profile.json').is_file())
"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -Fqx '0.9 True'
}

@test "stack: the markdown report names the primary profile" {
    printf '%s\n' '{"dependencies":{"next":"1"}}' > "$WORK/package.json"
    run_py "
st.detect_stack(root)
print((root / '.autospec/reports/stack-profile.md').read_text().strip().splitlines()[-1])
"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -Fq 'nextjs-web-app'
}
