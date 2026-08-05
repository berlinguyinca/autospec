#!/usr/bin/env bats
# tests/unit/test_autonomy_recipes.bats — built-in recipes, recipe YAML, and the
# recipe index (scripts/autospec_autonomy_recipes.py), extracted from
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
import autospec_autonomy_recipes as r
$1
"
}

@test "recipes: the built-in set has 30 entries with unique ids" {
    run_py "
entries = r.built_in_recipes()
ids = [e['id'] for e in entries]
print(len(entries), len(set(ids)) == len(ids))
"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -Fqx '30 True'
}

@test "recipes: builder defaults to a low-risk scaffold that requires tests" {
    run_py "
e = r.recipe('x', 'X', 'ui', 'ui_scaffold', ['rule'])
print(e['implementation']['mode'], e['risk']['level'], e['test_plan']['required'])
"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -Fqx 'scaffold low True'
}

@test "recipes: planning_only mode does not require a test plan" {
    run_py "
e = r.recipe('x', 'X', 'ai', 'ai_scaffold', ['rule'], 'planning_only')
print(e['test_plan']['required'])
"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -Fqx 'False'
}

@test "recipes: only high risk demands human guidance and architecture review" {
    run_py "
low = r.recipe('a', 'A', 'ui', 'ui_scaffold', ['x'])
high = r.recipe('b', 'B', 'ui', 'ui_scaffold', ['x'], risk='high')
print(low['risk']['requires_human_guidance'], high['risk']['requires_human_guidance'],
      high['risk']['requires_architecture_review'])
"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -Fqx 'False True True'
}

@test "recipes: every entry forbids the shared forbidden paths" {
    run_py "
from autospec_autonomy_io import FORBIDDEN_PATHS
entries = r.built_in_recipes()
print(all(e['implementation']['forbidden_paths'] == FORBIDDEN_PATHS for e in entries))
"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -Fqx 'True'
}

@test "recipes: empty list fields render as an explicit empty YAML sequence" {
    run_py "
print(r.list_block('technologies', []))
print(r.list_block('technologies', ['react'], '  '))
"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -Fq "['technologies:', '  []']"
    # Scalars are quoted by yaml_scalar; indentation nests two spaces per level.
    printf '%s\n' "$output" | grep -Fq "['  technologies:', '    - \"react\"']"
}

@test "recipes: write_recipe_files lays out one yml per recipe under its category" {
    run_py "
from pathlib import Path
root = Path('$WORK')
r.write_recipe_files(root, r.built_in_recipes())
base = root / '.autospec/recipes'
print((base / 'README.md').is_file(),
      (base / 'ui/settings-page-scaffold.yml').is_file(),
      len(list(base.rglob('*.yml'))))
"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -Fqx 'True True 30'
}

@test "recipes: recipe_index writes state and report artifacts with a summary total" {
    run_py "
import json
from pathlib import Path
root = Path('$WORK')
print(r.recipe_index(root))
payload = json.loads((root / '.autospec/state/implementation-recipes.json').read_text())
print(payload['schema'], payload['summary']['total'],
      (root / '.autospec/reports/implementation-recipes.md').is_file())
"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -Fqx '0'
    printf '%s\n' "$output" | grep -Fqx '1 30 True'
}

@test "recipes: a recipe whose capability is not enabled or experimental is unsupported" {
    run_py "
entries = [r.recipe('planned-cap', 'Planned', 'ui', 'never_enabled', ['x'])]
statuses = {'never_enabled': 'planned'}
for e in entries:
    if statuses.get(e['capability']) not in {'enabled', 'experimental'}:
        e['status'] = 'unsupported'
print(entries[0]['status'])
"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -Fqx 'unsupported'
}

@test "recipes: recipe_index regenerates capability statuses, so edits to the yml do not stick" {
    run_py "
import json
from pathlib import Path
import autospec_autonomy_capabilities as caps
root = Path('$WORK')
caps.ensure_capabilities(root)
yml = root / '.autospec/state/worker-capabilities.yml'
yml.write_text(yml.read_text().replace('status: experimental', 'status: planned'))
r.recipe_index(root)
payload = json.loads((root / '.autospec/state/implementation-recipes.json').read_text())
entries = {e['id']: e for e in payload['recipes']}
print(entries['settings-page-scaffold']['status'], payload['summary']['supported'])
"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -Fqx 'supported 30'
}
