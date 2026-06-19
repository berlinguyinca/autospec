#!/usr/bin/env bats
# load-fab-config.bats — TDD tests for skills/autospec-fab/scripts/load-fab-config.sh
# Real yq required; tests skip if yq is absent.

SCRIPT_DIR="$(cd "$(dirname "$BATS_TEST_FILENAME")/../scripts" && pwd)"
FIXTURE_DIR="$(cd "$(dirname "$BATS_TEST_FILENAME")/fixtures" && pwd)"
SCRIPT="$SCRIPT_DIR/load-fab-config.sh"

setup() {
    if ! command -v yq >/dev/null 2>&1; then
        skip "yq not found — install yq to run these tests"
    fi
    if ! command -v jq >/dev/null 2>&1; then
        skip "jq not found — install jq to run these tests"
    fi
    [ -f "$SCRIPT" ] || skip "load-fab-config.sh not yet created (TDD red)"
}

# ── 1. Complete fixture loads with expected keys/values ───────────────────────

@test "complete fab.yml: generator is present in normalized JSON" {
    run bash "$SCRIPT" "$FIXTURE_DIR/complete-fab.yml"
    [ "$status" -eq 0 ]
    result="$(printf '%s' "$output" | jq -r '.generator')"
    [ "$result" = "rm -rf build && .venv/bin/python src/generate.py" ]
}

@test "complete fab.yml: stl_roots has 2 entries" {
    run bash "$SCRIPT" "$FIXTURE_DIR/complete-fab.yml"
    [ "$status" -eq 0 ]
    count="$(printf '%s' "$output" | jq '.stl_roots | length')"
    [ "$count" -eq 2 ]
}

@test "complete fab.yml: printer.nozzle_width is 0.6" {
    run bash "$SCRIPT" "$FIXTURE_DIR/complete-fab.yml"
    [ "$status" -eq 0 ]
    val="$(printf '%s' "$output" | jq '.printer.nozzle_width')"
    [ "$val" = "0.6" ]
}

@test "complete fab.yml: printer.layer_height is 0.3" {
    run bash "$SCRIPT" "$FIXTURE_DIR/complete-fab.yml"
    [ "$status" -eq 0 ]
    val="$(printf '%s' "$output" | jq '.printer.layer_height')"
    [ "$val" = "0.3" ]
}

@test "complete fab.yml: printer.min_perimeters is 4" {
    run bash "$SCRIPT" "$FIXTURE_DIR/complete-fab.yml"
    [ "$status" -eq 0 ]
    val="$(printf '%s' "$output" | jq '.printer.min_perimeters')"
    [ "$val" -eq 4 ]
}

@test "complete fab.yml: printer.max_overhang_deg is 40" {
    run bash "$SCRIPT" "$FIXTURE_DIR/complete-fab.yml"
    [ "$status" -eq 0 ]
    val="$(printf '%s' "$output" | jq '.printer.max_overhang_deg')"
    [ "$val" -eq 40 ]
}

@test "complete fab.yml: models list has 2 entries" {
    run bash "$SCRIPT" "$FIXTURE_DIR/complete-fab.yml"
    [ "$status" -eq 0 ]
    count="$(printf '%s' "$output" | jq '.models | length')"
    [ "$count" -eq 2 ]
}

@test "complete fab.yml: first model has name inlet_manifold" {
    run bash "$SCRIPT" "$FIXTURE_DIR/complete-fab.yml"
    [ "$status" -eq 0 ]
    name="$(printf '%s' "$output" | jq -r '.models[0].name')"
    [ "$name" = "inlet_manifold" ]
}

@test "complete fab.yml: metadata_sidecar is present" {
    run bash "$SCRIPT" "$FIXTURE_DIR/complete-fab.yml"
    [ "$status" -eq 0 ]
    val="$(printf '%s' "$output" | jq -r '.metadata_sidecar')"
    [ -n "$val" ]
    [ "$val" != "null" ]
}

# ── 2. --validate: missing generator exits non-zero ───────────────────────────

@test "missing generator: --validate exits non-zero" {
    run bash "$SCRIPT" --validate "$FIXTURE_DIR/missing-generator-fab.yml"
    [ "$status" -ne 0 ]
}

@test "missing generator: --validate error mentions generator" {
    run bash "$SCRIPT" --validate "$FIXTURE_DIR/missing-generator-fab.yml"
    [ "$status" -ne 0 ]
    printf '%s' "$output" | grep -qi "generator"
}

# ── 3. --validate: empty/missing models exits non-zero ────────────────────────

@test "empty models list: --validate exits non-zero" {
    run bash "$SCRIPT" --validate "$FIXTURE_DIR/empty-models-fab.yml"
    [ "$status" -ne 0 ]
}

@test "empty models list: --validate error mentions models" {
    run bash "$SCRIPT" --validate "$FIXTURE_DIR/empty-models-fab.yml"
    [ "$status" -ne 0 ]
    printf '%s' "$output" | grep -qi "models"
}

# ── 4. Defaults applied when printer is absent ────────────────────────────────

@test "no-printer fab.yml: default nozzle_width is 0.4" {
    run bash "$SCRIPT" "$FIXTURE_DIR/no-printer-fab.yml"
    [ "$status" -eq 0 ]
    val="$(printf '%s' "$output" | jq '.printer.nozzle_width')"
    [ "$val" = "0.4" ]
}

@test "no-printer fab.yml: default layer_height is 0.2" {
    run bash "$SCRIPT" "$FIXTURE_DIR/no-printer-fab.yml"
    [ "$status" -eq 0 ]
    val="$(printf '%s' "$output" | jq '.printer.layer_height')"
    [ "$val" = "0.2" ]
}

@test "no-printer fab.yml: default min_perimeters is 3" {
    run bash "$SCRIPT" "$FIXTURE_DIR/no-printer-fab.yml"
    [ "$status" -eq 0 ]
    val="$(printf '%s' "$output" | jq '.printer.min_perimeters')"
    [ "$val" -eq 3 ]
}

@test "no-printer fab.yml: default max_overhang_deg is 45" {
    run bash "$SCRIPT" "$FIXTURE_DIR/no-printer-fab.yml"
    [ "$status" -eq 0 ]
    val="$(printf '%s' "$output" | jq '.printer.max_overhang_deg')"
    [ "$val" -eq 45 ]
}

@test "no-printer fab.yml: default stl_roots contains build/stls/manifolds/" {
    run bash "$SCRIPT" "$FIXTURE_DIR/no-printer-fab.yml"
    [ "$status" -eq 0 ]
    first="$(printf '%s' "$output" | jq -r '.stl_roots[0]')"
    [ "$first" = "build/stls/manifolds/" ]
}

# ── 5. Valid complete fab.yml passes --validate ───────────────────────────────

@test "complete fab.yml: --validate exits 0" {
    run bash "$SCRIPT" --validate "$FIXTURE_DIR/complete-fab.yml"
    [ "$status" -eq 0 ]
}

# ── 6. Missing file exits non-zero ────────────────────────────────────────────

@test "nonexistent file exits non-zero" {
    run bash "$SCRIPT" "/nonexistent/path/fab.yml"
    [ "$status" -ne 0 ]
}
