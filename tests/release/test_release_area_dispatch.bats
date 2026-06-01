#!/usr/bin/env bats
# tests/release/test_release_area_dispatch.bats — area dispatcher contract (#731).
#
# Covers:
#  - 6 areas are dispatched (mock subagent emits per-area JSON).
#  - per-area findings aggregate into release-verdict.json with the schema
#    consumed by scripts/compute-release-verdict.sh (PR #636).
#  - compute-release-verdict.sh consumes the merged verdict without regression.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    DISPATCH="$REPO_ROOT/scripts/release-area-dispatch.sh"
    COMPUTE="$REPO_ROOT/scripts/compute-release-verdict.sh"
    TMP="$(mktemp -d)"
    export AUTOSPEC_RELEASE_VERDICT="$TMP/release-verdict.json"
    export AUTOSPEC_RELEASE_REPO_ROOT="$REPO_ROOT"
    export AUTOSPEC_RELEASE_HEAD_SHA="deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"
    # Mock dispatcher: emit a PASS finding for each area, mark live_app_proof
    # only on qa-artifact-integrity.
    export AUTOSPEC_RELEASE_DISPATCH_CMD="$BATS_TEST_DIRNAME/_mock-dispatch.sh"
    cat > "$BATS_TEST_DIRNAME/_mock-dispatch.sh" <<'EOF'
#!/usr/bin/env bash
name="$1"
live="false"
[ "$name" = "qa-artifact-integrity" ] && live="true"
cat <<JSON
{
  "area": "$name",
  "status": "PASS",
  "release_blocking": false,
  "summary": "mock pass for $name",
  "live_app_proof": $live,
  "findings": [
    {"area": "$name", "status": "PASS", "release_blocking": false, "summary": "$name ok"}
  ]
}
JSON
EOF
    chmod +x "$BATS_TEST_DIRNAME/_mock-dispatch.sh"
}

teardown() {
    rm -rf "$TMP"
    rm -f "$BATS_TEST_DIRNAME/_mock-dispatch.sh"
}

@test "lists exactly 6 areas" {
    run bash "$DISPATCH" --list
    [ "$status" -eq 0 ]
    [ "$(printf '%s\n' "$output" | wc -l | tr -d ' ')" = "6" ]
    [[ "$output" == *"spec-completeness"* ]]
    [[ "$output" == *"docs-freshness"* ]]
    [[ "$output" == *"implementation-completeness"* ]]
    [[ "$output" == *"test-coverage"* ]]
    [[ "$output" == *"qa-artifact-integrity"* ]]
    [[ "$output" == *"legacy-cleanup"* ]]
}

@test "every area definition file exists" {
    for a in spec-completeness docs-freshness implementation-completeness \
             test-coverage qa-artifact-integrity legacy-cleanup; do
        run bash "$DISPATCH" --area "$a"
        [ "$status" -eq 0 ]
        [ -f "$output" ]
    done
}

@test "missing area returns exit 2" {
    run bash "$DISPATCH" --area nonexistent-area
    [ "$status" -eq 2 ]
}

@test "full dispatch aggregates 6 area findings into release-verdict.json" {
    run bash "$DISPATCH"
    [ "$status" -eq 0 ]
    [ -f "$AUTOSPEC_RELEASE_VERDICT" ]
    # 6 area rows
    count=$(jq '.areas | length' "$AUTOSPEC_RELEASE_VERDICT")
    [ "$count" = "6" ]
    # head_sha threaded
    head=$(jq -r '.head_sha' "$AUTOSPEC_RELEASE_VERDICT")
    [ "$head" = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef" ]
    # live_app_proof surfaced from qa-artifact-integrity
    live=$(jq -r '.live_app_proof' "$AUTOSPEC_RELEASE_VERDICT")
    [ "$live" = "true" ]
}

@test "compute-release-verdict.sh consumes the merged verdict (PASS)" {
    bash "$DISPATCH"
    run bash "$COMPUTE" "$AUTOSPEC_RELEASE_VERDICT" \
        "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"
    [ "$status" -eq 0 ]
    [[ "$output" == *"PASS"* ]]
}

@test "compute-release-verdict.sh consumes merged verdict (FAIL on blocking)" {
    # Dispatcher that marks one area release_blocking + FAIL.
    cat > "$BATS_TEST_DIRNAME/_mock-dispatch.sh" <<'EOF'
#!/usr/bin/env bash
name="$1"
live="false"
status="PASS"; blocking="false"; summary="ok"
[ "$name" = "qa-artifact-integrity" ] && live="true"
if [ "$name" = "legacy-cleanup" ]; then
    status="FAIL"; blocking="true"; summary="legacy residue remains"
fi
cat <<JSON
{
  "area": "$name",
  "status": "$status",
  "release_blocking": $blocking,
  "summary": "$summary",
  "live_app_proof": $live,
  "findings": [
    {"area": "$name", "status": "$status", "release_blocking": $blocking, "summary": "$summary"}
  ]
}
JSON
EOF
    chmod +x "$BATS_TEST_DIRNAME/_mock-dispatch.sh"
    bash "$DISPATCH"
    run bash "$COMPUTE" "$AUTOSPEC_RELEASE_VERDICT" \
        "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"
    [ "$status" -eq 2 ]
    [[ "$output" == *"FAIL"* ]]
}
