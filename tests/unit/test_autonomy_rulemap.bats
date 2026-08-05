#!/usr/bin/env bats
# tests/unit/test_autonomy_rulemap.bats — recipe lookup and rule-to-recipe planning
# (scripts/autospec_autonomy_rulemap.py), extracted from autospec-autonomy-v2-lib.py.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    SCRIPTS="$REPO_ROOT/scripts"
    WORK="$(mktemp -d)"
    mkdir -p "$WORK/.autospec/state"
    cat > "$WORK/.autospec/state/rule-check-results.json" <<'JSON'
{"schema":1,"results":[
 {"rule_id":"testing.playwright.viewport","status":"fail","check_type":"required_playwright_viewport_matrix"},
 {"rule_id":"required_token_usage_tracking","status":"unknown","check_type":"required_token_usage_tracking"},
 {"rule_id":"nothing.matches","status":"fail","check_type":"no_such_check"},
 {"rule_id":"already.passing","status":"pass","check_type":"required_health_check"}]}
JSON
}

teardown() {
    rm -rf "${WORK:?}"
}

run_py() {
    run python3 -c "
import sys
sys.path.insert(0, '$SCRIPTS')
import autospec_autonomy_rulemap as rm
from pathlib import Path
root = Path('$WORK')
$1
"
}

@test "rulemap: passing rules are excluded and actionable ones get a plan" {
    run_py "
import json
rm.rule_to_recipe(root)
plans = json.loads((root / '.autospec/state/rule-to-recipe-plan.json').read_text())['plans']
ids = [p['rule_id'] for p in plans]
print(len(plans), 'already.passing' in ids)
"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -Fqx '3 False'
}

@test "rulemap: a matched scaffold recipe is implementable and worker eligible" {
    run_py "
import json
rm.rule_to_recipe(root)
plans = {p['rule_id']: p for p in json.loads((root / '.autospec/state/rule-to-recipe-plan.json').read_text())['plans']}
p = plans['testing.playwright.viewport']
print(p['status'], p['best_recipe'], p['worker_eligibility'])
"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -Fqx 'recipe_available playwright-viewport-matrix True'
}

@test "rulemap: a planning_only recipe is not worker eligible" {
    run_py "
import json
rm.rule_to_recipe(root)
plans = {p['rule_id']: p for p in json.loads((root / '.autospec/state/rule-to-recipe-plan.json').read_text())['plans']}
p = plans['required_token_usage_tracking']
print(p['status'], p['worker_eligibility'])
"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -Fqx 'planning_only False'
}

@test "rulemap: an unmatched rule is unsupported with empty recipe fields" {
    run_py "
import json
rm.rule_to_recipe(root)
plans = {p['rule_id']: p for p in json.loads((root / '.autospec/state/rule-to-recipe-plan.json').read_text())['plans']}
p = plans['nothing.matches']
print(p['status'], repr(p['best_recipe']), p['estimated_risk'], p['worker_eligibility'])
"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -Fqx "unsupported '' unknown False"
}

@test "rulemap: --rule filtering narrows the plan to one rule" {
    run_py "
import json
rm.rule_to_recipe(root, 'nothing.matches')
plans = json.loads((root / '.autospec/state/rule-to-recipe-plan.json').read_text())['plans']
print(len(plans), plans[0]['rule_id'])
"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -Fqx '1 nothing.matches'
}

@test "rulemap: a disabled capability yields recipe_available_but_disabled" {
    run_py "
best = {'capability': 'ui_scaffold', 'implementation': {'mode': 'scaffold'},
        'risk': {'requires_human_guidance': False}}
print(rm._plan_status(best, {'ui_scaffold': 'planned'}))
"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -Fqx 'recipe_available_but_disabled'
}

@test "rulemap: high-risk guidance outranks the planning_only mode check" {
    # No built-in recipe declares risk=high, so this branch is unreachable
    # through built_in_recipes and is exercised directly.
    run_py "
best = {'capability': 'ui_scaffold', 'implementation': {'mode': 'planning_only'},
        'risk': {'requires_human_guidance': True}}
print(rm._plan_status(best, {'ui_scaffold': 'enabled'}))
"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -Fqx 'requires_human_guidance'
}

@test "rulemap: the markdown report lists every section with None fallbacks" {
    run_py "
rm.rule_to_recipe(root)
text = (root / '.autospec/reports/rule-to-recipe-plan.md').read_text()
for head in ('Implementable now', 'Scaffoldable now', 'Planning-only',
             'Requires human guidance', 'Unsupported'):
    assert '## ' + head in text, head
print('- None.' in text)
"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -Fqx 'True'
}

@test "rulemap: find_recipe returns a known recipe and None for an unknown id" {
    run_py "
hit = rm.find_recipe(root, 'accessibility-smoke')
print(hit['id'] if hit else None, rm.find_recipe(root, 'no-such-recipe'))
"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -Fqx 'accessibility-smoke None'
}
