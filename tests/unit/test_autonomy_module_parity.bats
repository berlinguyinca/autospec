#!/usr/bin/env bats
# tests/unit/test_autonomy_module_parity.bats — drift guard for the in-progress
# autospec-autonomy-v2-lib.py split.
#
# The extracted modules currently duplicate functions that still exist in the lib,
# because the commit that deletes the lib's copies and imports the modules cannot
# land until the gate fixes in
# docs/superpowers/specs/2026-08-05-lint-gate-satisfiability-design.md are in place.
#
# Until then these tests fail if the two copies diverge — otherwise a recipe added
# to the lib would silently not exist in the module. DELETE THIS FILE once the lib
# imports the modules; comparing a module to itself proves nothing.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    SCRIPTS="$REPO_ROOT/scripts"
    WORK="$(mktemp -d)"
}

teardown() {
    rm -rf "${WORK:?}"
}

# Loads the hyphenated lib under a module name, plus the extracted module.
parity_py() {
    run python3 -c "
import sys, importlib.util, filecmp, json
from pathlib import Path
sys.path.insert(0, '$SCRIPTS')
spec = importlib.util.spec_from_file_location('libmod', '$SCRIPTS/autospec-autonomy-v2-lib.py')
lib = importlib.util.module_from_spec(spec); spec.loader.exec_module(lib)

def tree_differs(a, b):
    def walk(dc, pre=''):
        out = [pre + f for f in dc.diff_files] + [pre + f for f in dc.left_only] + [pre + f for f in dc.right_only]
        for k, sub in dc.subdirs.items():
            out += walk(sub, pre + k + '/')
        return out
    return walk(filecmp.dircmp(str(a), str(b)))

work = Path('$WORK')
$1
"
}

@test "parity: capability defaults and YAML match the lib" {
    parity_py "
import autospec_autonomy_capabilities as caps
rows_lib, rows_mod = lib.default_capabilities(), caps.default_capabilities()
print(rows_lib == rows_mod, lib.capability_yaml(rows_lib) == caps.capability_yaml(rows_mod))
"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -Fqx 'True True'
}

@test "parity: the recipe set and every generated recipe artifact match the lib" {
    parity_py "
import autospec_autonomy_recipes as rec
print(lib.built_in_recipes() == rec.built_in_recipes())
a, b = work / 'lib', work / 'mod'
lib.recipe_index(a); rec.recipe_index(b)
print(tree_differs(a, b) or 'NONE')
"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -Fqx 'True'
    printf '%s\n' "$output" | grep -Fqx 'NONE'
}

@test "parity: stack detection artifacts and confidence match the lib" {
    parity_py "
import autospec_autonomy_stack as st
a, b = work / 'lib', work / 'mod'
for d in (a, b):
    d.mkdir(parents=True, exist_ok=True)
    (d / 'package.json').write_text('{\"dependencies\":{\"react\":\"1\",\"vite\":\"1\",\"typescript\":\"1\"}}')
    (d / 'App.tsx').write_text('x')
lib.detect_stack(a); st.detect_stack(b)
print(tree_differs(a, b) or 'NONE', lib.stack_confidence(a) == st.stack_confidence(b))
"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -Fqx 'NONE True'
}

@test "parity: rule-to-recipe plan artifacts match the lib" {
    parity_py "
import autospec_autonomy_rulemap as rm
rules = {'schema': 1, 'results': [
    {'rule_id': 'testing.playwright.viewport', 'status': 'fail', 'check_type': 'required_playwright_viewport_matrix'},
    {'rule_id': 'required_token_usage_tracking', 'status': 'unknown', 'check_type': 'required_token_usage_tracking'},
    {'rule_id': 'nothing.matches', 'status': 'fail', 'check_type': 'no_such'},
    {'rule_id': 'already.passing', 'status': 'pass', 'check_type': 'required_health_check'}]}
a, b = work / 'lib', work / 'mod'
for d in (a, b):
    (d / '.autospec/state').mkdir(parents=True, exist_ok=True)
    (d / '.autospec/state/rule-check-results.json').write_text(json.dumps(rules))
lib.rule_to_recipe(a); rm.rule_to_recipe(b)
print(tree_differs(a, b) or 'NONE')
"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -Fqx 'NONE'
}

@test "parity: shared IO helpers behave identically to the lib copies" {
    parity_py "
import autospec_autonomy_io as io
checks = [
    lib.yaml_scalar('docs/**') == io.yaml_scalar('docs/**'),
    lib.yaml_scalar('true') == io.yaml_scalar('true'),
    lib.slug('Hello, World!') == io.slug('Hello, World!'),
    lib.load_json(work / 'missing.json', {'d': 1}) == io.load_json(work / 'missing.json', {'d': 1}),
    lib.CAPABILITY_IDS == io.CAPABILITY_IDS,
    lib.FORBIDDEN_PATHS == io.FORBIDDEN_PATHS,
]
print(all(checks), len(checks))
"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -Fqx 'True 6'
}
