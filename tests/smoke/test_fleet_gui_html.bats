#!/usr/bin/env bats
# tests/smoke/test_fleet_gui_html.bats — smoke tests for the fleet GUI
# frontend (skills/autospec-fleet/gui/index.html).
#
# Verifies that the file exists and contains the required accessibility and
# functional attributes mandated by issue #828.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    GUI_HTML="$REPO_ROOT/skills/autospec-fleet/gui/index.html"
}

@test "fleet-gui index.html exists" {
    [ -f "$GUI_HTML" ]
}

@test "fleet-gui search box has aria-label='Filter repos by name'" {
    grep -q 'aria-label="Filter repos by name"' "$GUI_HTML"
}

@test "fleet-gui repo rows use label for='repo-N' pattern (dynamic setAttribute)" {
    grep -q "setAttribute.*'for'" "$GUI_HTML"
}

@test "fleet-gui Save button has Cmd/Ctrl+S keydown handler with preventDefault" {
    grep -q 'metaKey\|ctrlKey' "$GUI_HTML"
    grep -q "key === 's'" "$GUI_HTML"
    grep -q 'preventDefault' "$GUI_HTML"
}

@test "fleet-gui POSTs to /api/config with X-Autospec-Token header" {
    grep -q "X-Autospec-Token" "$GUI_HTML"
    grep -q '/api/config' "$GUI_HTML"
}

@test "fleet-gui has Select all visible and Clear all controls" {
    grep -q 'select-all' "$GUI_HTML"
    grep -q 'clear-all' "$GUI_HTML"
}

@test "fleet-gui two-column grid layout (35%/65%)" {
    grep -q '35% 65%' "$GUI_HTML"
}
