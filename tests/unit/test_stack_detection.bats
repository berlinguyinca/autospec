#!/usr/bin/env bats
# tests/unit/test_stack_detection.bats — stack detection must name a LANGUAGE as
# the repository's primary profile: never a framework (playwright, next), and
# never a marker that only exists inside a fixture tree.
#
# Covers scripts/autospec_autonomy_stack.py (the detector) and
# scripts/autospec-detect-stack-profile.sh (the UI-capability walker that must
# share the detector's one exclusion list).

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

# A polyglot tree: a Python language marker, real Rust/Bash sources under
# tests/ (which must still count for line share), and a root Playwright config.
seed_polyglot_tree() {
    mkdir -p "$WORK/scripts" "$WORK/tests/unit"
    printf '[project]\nname = "demo"\n' > "$WORK/pyproject.toml"
    printf 'print(1)\n' > "$WORK/main.py"
    printf 'echo hi\n' > "$WORK/scripts/build.sh"
    printf 'fn main() {}\n' > "$WORK/tests/unit/demo.rs"
}

@test "stack: a framework never outranks the language it is written in" {
    seed_polyglot_tree
    printf 'export default {};\n' > "$WORK/playwright.config.ts"
    run_py "
import json
st.detect_stack(root)
d = json.loads((root / '.autospec/state/stack-profile.json').read_text())
langs = [p['id'] for p in d['languages']]
fws = [p['id'] for p in d['frameworks']]
print(d['primary_profile']['id'], 'playwright' in fws, 'playwright' in langs)
"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -Fqx 'python True False'
}

@test "stack: a marker nested under tests/fixtures casts zero candidate votes" {
    seed_polyglot_tree
    mkdir -p "$WORK/tests/fixtures/evidence/react-vite-with-playwright"
    printf 'export default {};\n' \
        > "$WORK/tests/fixtures/evidence/react-vite-with-playwright/playwright.config.ts"
    printf '%s\n' '{"dependencies":{"react":"1","vite":"1","typescript":"1"}}' \
        > "$WORK/tests/fixtures/evidence/react-vite-with-playwright/package.json"
    run_py "
import json
st.detect_stack(root)
d = json.loads((root / '.autospec/state/stack-profile.json').read_text())
print(d['primary_profile']['id'], [p['id'] for p in d['frameworks']])
"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -Fqx 'python []'
}

@test "stack: a tracked .rs/.sh source under tests/ still counts toward line share" {
    seed_polyglot_tree
    run_py "
files = [rel for rel, _ in st._walk(root)]
counts = st._line_counts(list(st._walk(root)))
print('tests/unit/demo.rs' in files, counts.get('.rs', 0) > 0, counts.get('.sh', 0) > 0)
"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -Fqx 'True True True'
}

@test "stack: confidence is clamped below the scaffold gate under a 50% line share" {
    mkdir -p "$WORK/src" "$WORK/tests/unit"
    printf '%s\n' '{"dependencies":{"react":"1","vite":"1","typescript":"1"}}' > "$WORK/package.json"
    printf 'export const A = 1;\n' > "$WORK/src/App.tsx"
    for i in $(seq 1 200); do printf 'fn f%s() {}\n' "$i"; done > "$WORK/tests/unit/big.rs"
    run_py "
import json
st.detect_stack(root)
d = json.loads((root / '.autospec/state/stack-profile.json').read_text())
print(d['primary_profile']['id'], d['primary_profile']['confidence'], st.stack_confidence(root) < 0.8)
"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -Fqx 'javascript 0.5 True'
}

@test "stack: build-output and worktree directories are excluded from the walk" {
    mkdir -p "$WORK/target" "$WORK/dist" "$WORK/build" "$WORK/.next" "$WORK/out" \
        "$WORK/coverage" "$WORK/.claude/worktrees/wt-a"
    for d in target dist build .next out coverage .claude/worktrees/wt-a; do
        printf 'print(1)\n' > "$WORK/$d/setup.py"
    done
    run_py "
import json
st.detect_stack(root)
print(json.loads((root / '.autospec/state/stack-profile.json').read_text())['primary_profile']['id'])
"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -Fqx 'unknown'
}

@test "stack: both walkers share one exclusion list defined in exactly one file" {
    run bash -c "grep -rln '^SKIP_DIRS = ' '$SCRIPTS' | sort"
    [ "$status" -eq 0 ]
    [ "$output" = "$SCRIPTS/autospec_autonomy_stack.py" ]
    run grep -Fq 'from autospec_autonomy_stack import' "$SCRIPTS/autospec-detect-stack-profile.sh"
    [ "$status" -eq 0 ]
}

@test "stack: the UI-capability walker honours the shared exclusion list" {
    mkdir -p "$WORK/dist"
    printf '%s\n' '{"dependencies":{"next":"1"}}' > "$WORK/package.json"
    printf '@media (prefers-reduced-motion: reduce) { * { animation: none; } }\n' \
        > "$WORK/dist/app.css"
    run bash "$SCRIPTS/autospec-detect-stack-profile.sh" --repo-root "$WORK"
    [ "$status" -eq 0 ]
    run python3 -c "
import json
d = json.load(open('$WORK/.autospec/state/stack-profile.json'))
print(d['ui_capabilities']['reduced_motion_reset']['present'])
"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -Fqx 'False'
}

@test "stack: this repository resolves to a language, not playwright" {
    run python3 -c "
import sys
sys.path.insert(0, '$SCRIPTS')
import autospec_autonomy_stack as st
from pathlib import Path
d = st._detect_profiles(Path('$REPO_ROOT'))
ids = [p['id'] for p in d['languages']]
print(max(d['languages'], key=lambda p: p['confidence'])['id'], 'playwright' in ids)
"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -Fqx 'rust False'
}
