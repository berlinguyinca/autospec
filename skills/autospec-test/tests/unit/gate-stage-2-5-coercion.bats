#!/usr/bin/env bats
# skills/autospec-test/tests/unit/gate-stage-2-5-coercion.bats
#
# jq alternative-operator ("//") coercion guard for gate-stage-2-5.sh.
#
# jq's "//" treats a literal `false` the same as an absent key, so
# `.passed // true` silently coerces a real metric failure back to a pass.
# This covers the four Stage 2.5 metric sites (F, G, H, I).
#
# gate-stage-2-5.sh resolves each metric runner at
# "$SCRIPT_DIR/../invariants/<runner>.mjs" (SCRIPT_DIR being the scripts/ dir
# itself), so stub runners live in a sibling `invariants/` directory next to
# a copy of scripts/ — this mirrors run_metric()'s real path resolution
# without depending on whether the real runners are wired up at that path.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../../../.." && pwd)"
    REAL_SCRIPTS_DIR="$REPO_ROOT/skills/autospec-test/scripts"

    TEST_TMPDIR="$(mktemp -d /tmp/autospec-gate25-bats-XXXXXX)"
    STUB_SCRIPTS="$TEST_TMPDIR/scripts"
    cp -R "$REAL_SCRIPTS_DIR" "$STUB_SCRIPTS"
    mkdir -p "$TEST_TMPDIR/invariants"

    TARGET_DIR="$TEST_TMPDIR/target"
    mkdir -p "$TARGET_DIR/.autospec"
    cat > "$TARGET_DIR/.autospec/test.yml" <<'EOF'
mode: strict_isolation
e2e:
  invariants_v2:
    enabled: true
EOF
}

teardown() {
    rm -rf "$TEST_TMPDIR"
}

# Write a stub Node runner at $TEST_TMPDIR/invariants/<name> that always
# prints a literal passed:false and exits 0 (the shape a real failed
# structural/window/crawler/symmetry check would produce).
make_failing_runner() {
    local runner_name="$1"
    local metric="$2"
    cat > "$TEST_TMPDIR/invariants/$runner_name" <<EOF
#!/usr/bin/env node
console.log(JSON.stringify({metric: "$metric", passed: false, reason: "stub-forced-fail"}));
process.exit(0);
EOF
    chmod +x "$TEST_TMPDIR/invariants/$runner_name"
}

@test "gate-stage-2-5: Metric F literal passed:false fails the gate" {
    make_failing_runner "run-structural.mjs" "F"
    run bash "$STUB_SCRIPTS/gate-stage-2-5.sh" "$TARGET_DIR"
    [ "$status" -eq 1 ]
    local f_passed
    f_passed="$(printf '%s' "$output" | jq -r '.metrics.F.passed')"
    [ "$f_passed" = "false" ]
    local overall
    overall="$(printf '%s' "$output" | jq -r '.passed')"
    [ "$overall" = "false" ]
}

@test "gate-stage-2-5: Metric G literal passed:false fails the gate" {
    make_failing_runner "run-window.mjs" "G"
    run bash "$STUB_SCRIPTS/gate-stage-2-5.sh" "$TARGET_DIR"
    [ "$status" -eq 1 ]
    local g_passed
    g_passed="$(printf '%s' "$output" | jq -r '.metrics.G.passed')"
    [ "$g_passed" = "false" ]
    local overall
    overall="$(printf '%s' "$output" | jq -r '.passed')"
    [ "$overall" = "false" ]
}

@test "gate-stage-2-5: Metric H literal passed:false fails the gate" {
    make_failing_runner "extended-crawler.mjs" "H"
    run bash "$STUB_SCRIPTS/gate-stage-2-5.sh" "$TARGET_DIR"
    [ "$status" -eq 1 ]
    local h_passed
    h_passed="$(printf '%s' "$output" | jq -r '.metrics.H.passed')"
    [ "$h_passed" = "false" ]
    local overall
    overall="$(printf '%s' "$output" | jq -r '.passed')"
    [ "$overall" = "false" ]
}

@test "gate-stage-2-5: Metric I literal passed:false fails the gate" {
    make_failing_runner "run-symmetry.mjs" "I"
    run bash "$STUB_SCRIPTS/gate-stage-2-5.sh" "$TARGET_DIR"
    [ "$status" -eq 1 ]
    local i_passed
    i_passed="$(printf '%s' "$output" | jq -r '.metrics.I.passed')"
    [ "$i_passed" = "false" ]
    local overall
    overall="$(printf '%s' "$output" | jq -r '.passed')"
    [ "$overall" = "false" ]
}

# ── Absent-key default preserved: no runners installed => passed:true ───────

@test "gate-stage-2-5: no runners installed still defaults to passed:true (absent key)" {
    run bash "$STUB_SCRIPTS/gate-stage-2-5.sh" "$TARGET_DIR"
    [ "$status" -eq 0 ]
    local overall
    overall="$(printf '%s' "$output" | jq -r '.passed')"
    [ "$overall" = "true" ]
}
