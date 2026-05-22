#!/usr/bin/env bats
# skills/autospec-test/tests/integration/v2/run-against-target.bats
#
# Integration harness: validates that each v2 synthetic target has the
# expected golden file structure and that the target .autospec/test.yml
# contains the correct v2 contract declarations.
#
# Since gate-stage-2-5.sh is delivered in Phase 10, this harness validates
# the static fixtures (golden files + contract declarations) rather than
# running live Playwright. The golden-diff logic is the same pattern used
# by the Phase 1 integration harness.
#
# Usage:
#   bats skills/autospec-test/tests/integration/v2/run-against-target.bats
#
# To run a single test:
#   bats skills/autospec-test/tests/integration/v2/run-against-target.bats \
#     --filter "target-invariant-bait"

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../../../../.." && pwd)"
    TARGETS_DIR="$REPO_ROOT/skills/autospec-test/test-targets"
    FIXTURES_DIR="$REPO_ROOT/skills/autospec-test/tests/fixtures/v2"
}

# ── Helper: check YAML contains a string ─────────────────────────────────────
yaml_contains() {
    local file="$1"
    local pattern="$2"
    grep -q "$pattern" "$file"
}

# ── Helper: validate golden JSON is well-formed ───────────────────────────────
golden_is_valid_json() {
    local golden_file="$1"
    jq empty "$golden_file" 2>/dev/null
}

# ── target-invariant-bait ─────────────────────────────────────────────────────

@test "target-invariant-bait: directory exists" {
    [ -d "$TARGETS_DIR/target-invariant-bait" ]
}

@test "target-invariant-bait: has src/index.html" {
    [ -f "$TARGETS_DIR/target-invariant-bait/src/index.html" ]
}

@test "target-invariant-bait: has .autospec/test.yml with invariants_v2" {
    local yml="$TARGETS_DIR/target-invariant-bait/.autospec/test.yml"
    [ -f "$yml" ]
    yaml_contains "$yml" "invariants_v2"
    yaml_contains "$yml" "every_visible_X_is_Y"
    yaml_contains "$yml" "done-item-row"
}

@test "target-invariant-bait: has valid golden/stage-2-5-gate.json" {
    local golden="$TARGETS_DIR/target-invariant-bait/golden/stage-2-5-gate.json"
    [ -f "$golden" ]
    golden_is_valid_json "$golden"
}

@test "target-invariant-bait: golden shows passed=false (Metric F violation)" {
    local golden="$TARGETS_DIR/target-invariant-bait/golden/stage-2-5-gate.json"
    [ -f "$golden" ]
    local passed
    passed=$(jq -r '.passed' "$golden")
    [ "$passed" = "false" ]
}

@test "target-invariant-bait: golden identifies violation at index 4" {
    local golden="$TARGETS_DIR/target-invariant-bait/golden/stage-2-5-gate.json"
    [ -f "$golden" ]
    local violation_index
    violation_index=$(jq -r '.invariants[0].violations[0].index' "$golden")
    [ "$violation_index" = "4" ]
}

@test "target-invariant-bait: golden has count_observed=5" {
    local golden="$TARGETS_DIR/target-invariant-bait/golden/stage-2-5-gate.json"
    [ -f "$golden" ]
    local count
    count=$(jq -r '.invariants[0].count_observed' "$golden")
    [ "$count" = "5" ]
}

@test "target-invariant-bait: HTML has 5 done-item rows" {
    local html="$TARGETS_DIR/target-invariant-bait/src/index.html"
    [ -f "$html" ]
    local count
    count=$(grep -c 'data-testid="done-item-row-' "$html")
    [ "$count" -eq 5 ]
}

@test "target-invariant-bait: HTML row 4 has fake-edit span not a real button" {
    local html="$TARGETS_DIR/target-invariant-bait/src/index.html"
    [ -f "$html" ]
    # The bait: done-item-row-4 div should contain span.fake-edit
    grep -q 'fake-edit' "$html"
    # And the done-item--broken div comment confirms no button is present
    grep -q 'done-item--broken' "$html"
}

@test "target-invariant-bait: has package.json" {
    [ -f "$TARGETS_DIR/target-invariant-bait/package.json" ]
    jq -r '.name' "$TARGETS_DIR/target-invariant-bait/package.json" | grep -q "target-invariant-bait"
}

# ── target-window-mismatch-bait ───────────────────────────────────────────────

@test "target-window-mismatch-bait: directory exists" {
    [ -d "$TARGETS_DIR/target-window-mismatch-bait" ]
}

@test "target-window-mismatch-bait: has src/index.html with data-window-days=7" {
    local html="$TARGETS_DIR/target-window-mismatch-bait/src/index.html"
    [ -f "$html" ]
    grep -q 'data-window-days="7"' "$html"
}

@test "target-window-mismatch-bait: HTML fetches 3-day window not 7-day" {
    local html="$TARGETS_DIR/target-window-mismatch-bait/src/index.html"
    [ -f "$html" ]
    # JS should subtract 3 days, not 7
    grep -q '3 \* 86400000' "$html"
}

@test "target-window-mismatch-bait: has .autospec/test.yml with window_contracts" {
    local yml="$TARGETS_DIR/target-window-mismatch-bait/.autospec/test.yml"
    [ -f "$yml" ]
    yaml_contains "$yml" "window_contracts"
    yaml_contains "$yml" "dashboard-streak-window"
    yaml_contains "$yml" "data-window-days"
}

@test "target-window-mismatch-bait: has valid golden/stage-2-5-gate.json" {
    local golden="$TARGETS_DIR/target-window-mismatch-bait/golden/stage-2-5-gate.json"
    [ -f "$golden" ]
    golden_is_valid_json "$golden"
}

@test "target-window-mismatch-bait: golden shows passed=false (Metric G violation)" {
    local golden="$TARGETS_DIR/target-window-mismatch-bait/golden/stage-2-5-gate.json"
    [ -f "$golden" ]
    local passed
    passed=$(jq -r '.passed' "$golden")
    [ "$passed" = "false" ]
}

@test "target-window-mismatch-bait: golden captures N=7 with observed 3-day window" {
    local golden="$TARGETS_DIR/target-window-mismatch-bait/golden/stage-2-5-gate.json"
    [ -f "$golden" ]
    local N
    N=$(jq -r '.contracts[0].N' "$golden")
    [ "$N" = "7" ]
    local observed
    observed=$(jq -r '.contracts[0].violations[0].observed_offset_days' "$golden")
    [ "$observed" = "-3" ]
}

@test "target-window-mismatch-bait: has server.mjs" {
    [ -f "$TARGETS_DIR/target-window-mismatch-bait/src/server.mjs" ]
}

# ── target-contract-symmetry-bait ────────────────────────────────────────────

@test "target-contract-symmetry-bait: directory exists" {
    [ -d "$TARGETS_DIR/target-contract-symmetry-bait" ]
}

@test "target-contract-symmetry-bait: HTML has 3 streak-task elements" {
    local html="$TARGETS_DIR/target-contract-symmetry-bait/src/index.html"
    [ -f "$html" ]
    local count
    count=$(grep -c 'data-testid="streak-task-' "$html")
    [ "$count" -eq 3 ]
}

@test "target-contract-symmetry-bait: has .autospec/test.yml with contract_symmetry" {
    local yml="$TARGETS_DIR/target-contract-symmetry-bait/.autospec/test.yml"
    [ -f "$yml" ]
    yaml_contains "$yml" "contract_symmetry"
    yaml_contains "$yml" "streak-task-must-be-editable"
    yaml_contains "$yml" "task_id"
}

@test "target-contract-symmetry-bait: has valid golden/stage-2-5-gate.json" {
    local golden="$TARGETS_DIR/target-contract-symmetry-bait/golden/stage-2-5-gate.json"
    [ -f "$golden" ]
    golden_is_valid_json "$golden"
}

@test "target-contract-symmetry-bait: golden shows passed=false (Metric I violation)" {
    local golden="$TARGETS_DIR/target-contract-symmetry-bait/golden/stage-2-5-gate.json"
    [ -f "$golden" ]
    local passed
    passed=$(jq -r '.passed' "$golden")
    [ "$passed" = "false" ]
}

@test "target-contract-symmetry-bait: golden violation is for task t-3" {
    local golden="$TARGETS_DIR/target-contract-symmetry-bait/golden/stage-2-5-gate.json"
    [ -f "$golden" ]
    local task_id
    task_id=$(jq -r '.violations[0].ui_claim.task_id' "$golden")
    [ "$task_id" = "t-3" ]
}

@test "target-contract-symmetry-bait: server returns empty events for t-3" {
    local server="$TARGETS_DIR/target-contract-symmetry-bait/src/server.mjs"
    [ -f "$server" ]
    # Verify server code explicitly shows t-3 returning empty events
    grep -q "'t-3': { events: \[\] }" "$server"
}

# ── target-greenwash-bait v2 extension ───────────────────────────────────────

@test "greenwash-bait: .autospec/test.yml now has invariants_v2 block" {
    local yml="$TARGETS_DIR/target-greenwash-bait/.autospec/test.yml"
    [ -f "$yml" ]
    yaml_contains "$yml" "invariants_v2"
}

@test "greenwash-bait: .autospec/test.yml has 2 invariant entries" {
    local yml="$TARGETS_DIR/target-greenwash-bait/.autospec/test.yml"
    [ -f "$yml" ]
    local count
    count=$(grep -c '^\s*- id:' "$yml")
    [ "$count" -ge 2 ]
}

@test "greenwash-bait: LOOSENING fixture diff exists and removes invariants_v2" {
    local fixture="$FIXTURES_DIR/greenwash-bait/loop-attempts-to-delete-invariants.diff"
    [ -f "$fixture" ]
    grep -q '\-  invariants_v2:' "$fixture"
}

@test "greenwash-bait: LOOSENING diff removes both invariant entries" {
    local fixture="$FIXTURES_DIR/greenwash-bait/loop-attempts-to-delete-invariants.diff"
    [ -f "$fixture" ]
    local removed_ids
    removed_ids=$(grep -c '^\-.*- id:' "$fixture" || true)
    [ "$removed_ids" -ge 2 ]
}

@test "greenwash-bait: LOOSENING diff is valid unified diff format" {
    local fixture="$FIXTURES_DIR/greenwash-bait/loop-attempts-to-delete-invariants.diff"
    [ -f "$fixture" ]
    # Must start with --- and +++ headers
    head -2 "$fixture" | grep -q '^---'
}
