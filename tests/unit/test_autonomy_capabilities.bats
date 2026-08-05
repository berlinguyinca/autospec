#!/usr/bin/env bats
# tests/unit/test_autonomy_capabilities.bats — worker capability defaults and YAML
# (scripts/autospec_autonomy_capabilities.py), extracted from
# autospec-autonomy-v2-lib.py. Assertions pin the values the original produced.

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
import autospec_autonomy_capabilities as caps
$1
"
}

@test "capabilities: every declared capability id yields exactly one row" {
    run_py "
from autospec_autonomy_io import CAPABILITY_IDS
rows = caps.default_capabilities()
print(len(rows) == len(CAPABILITY_IDS), [r['id'] for r in rows] == CAPABILITY_IDS)
"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -Fqx 'True True'
}

@test "capabilities: enabled, experimental, and planned statuses are assigned correctly" {
    run_py "
rows = {r['id']: r for r in caps.default_capabilities()}
print(rows['docs']['status'], rows['ui_scaffold']['status'], rows['ui_scaffold']['risk_level'])
"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -Fqx 'enabled experimental medium'
}

@test "capabilities: only experimental capabilities may write under src/**" {
    run_py "
rows = {r['id']: r for r in caps.default_capabilities()}
print('src/**' in rows['ui_scaffold']['allowed_paths'], 'src/**' in rows['docs']['allowed_paths'])
"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -Fqx 'True False'
}

@test "capabilities: playwright_scaffold keeps its explicit title override" {
    run_py "
rows = {r['id']: r for r in caps.default_capabilities()}
print(rows['playwright_scaffold']['title'])
"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -Fqx 'Playwright scaffolds'
}

@test "capabilities: dependency, database, and auth writes are refused for every capability" {
    run_py "
rows = caps.default_capabilities()
keys = ('can_modify_dependencies', 'can_modify_database', 'can_modify_auth_security')
print(all(r[k] is False for r in rows for k in keys))
"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -Fqx 'True'
}

@test "capabilities: YAML declares schema 1 and quotes glob paths" {
    run_py "
text = caps.capability_yaml(caps.default_capabilities())
print(text.startswith('schema: 1\ncapabilities:\n'), '    - \"docs/**\"' in text)
"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -Fqx 'True True'
}

@test "capabilities: ensure_capabilities writes the yml, json, and md artifacts" {
    run_py "
from pathlib import Path
root = Path('$WORK')
caps.ensure_capabilities(root)
print((root / '.autospec/state/worker-capabilities.yml').is_file(),
      (root / '.autospec/state/worker-capabilities.json').is_file(),
      (root / '.autospec/reports/worker-capabilities.md').is_file())
"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -Fqx 'True True True'
}

@test "capabilities: statuses load from the written yml and bootstrap when absent" {
    run_py "
from pathlib import Path
root = Path('$WORK')
statuses = caps.load_capability_statuses(root)
print(statuses['docs'], statuses['ui_scaffold'], len(statuses))
"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -Fqx 'enabled experimental 17'
}
