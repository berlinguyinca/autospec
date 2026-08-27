#!/usr/bin/env bats
# skills/autospec-test/tests/unit/gate-stage-2-5-coercion.bats
#
# jq alternative-operator ("//") coercion guard for gate-stage-2-5.sh.
#
# jq's "//" treats a literal `false` the same as an absent key, so
# `.passed // true` silently coerces a real metric failure back to a pass.
# This covers the four Stage 2.5 metric sites (F, G, H, I).
#
# gate-stage-2-5.sh resolves each metric runner at "$SCRIPT_DIR/<subdir>/<name>"
# (SCRIPT_DIR being the scripts/ dir itself; each metric's runner lives in a
# different subdirectory — invariants/, window-contract/, crawler-v2/,
# contract-symmetry/). Stub runners are written directly over the real
# runner files inside a copy of scripts/, at the exact path run_metric()
# resolves, so the stub's forced-fail output — not the real runner's own
# behavior — is what the gate actually observes.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../../../.." && pwd)"
    REAL_SCRIPTS_DIR="$REPO_ROOT/skills/autospec-test/scripts"

    TEST_TMPDIR="$(mktemp -d /tmp/autospec-gate25-bats-XXXXXX)"
    STUB_SCRIPTS="$TEST_TMPDIR/scripts"
    cp -R "$REAL_SCRIPTS_DIR" "$STUB_SCRIPTS"

    TARGET_DIR="$TEST_TMPDIR/target"
    mkdir -p "$TARGET_DIR/.autospec" "$TARGET_DIR/src"
    cat > "$TARGET_DIR/.autospec/test.yml" <<'EOF'
mode: strict_isolation
e2e:
  invariants_v2:
    enabled: true
EOF
    # gate-stage-2-5.sh's payload-building step skips a metric loudly instead
    # of invoking its runner when no static fixture is present (it never
    # stands up a dev server). A static src/index.html is required here so
    # the stub runners below are actually reached — these tests are about
    # jq's "//" coercion, not about the skip-detection wiring itself.
    printf '<!doctype html><html><body></body></html>' > "$TARGET_DIR/src/index.html"
}

teardown() {
    rm -rf "$TEST_TMPDIR"
}

# Overwrite the real runner at $STUB_SCRIPTS/<relpath> (the exact path
# run_metric() resolves for that metric) with a stub that always prints a
# literal passed:false and exits 0 (the shape a real failed
# structural/window/crawler/symmetry check would produce).
make_failing_runner() {
    local relpath="$1"
    local metric="$2"
    cat > "$STUB_SCRIPTS/$relpath" <<EOF
#!/usr/bin/env node
console.log(JSON.stringify({metric: "$metric", passed: false, reason: "stub-forced-fail"}));
process.exit(0);
EOF
    chmod +x "$STUB_SCRIPTS/$relpath"
}

@test "gate-stage-2-5: Metric F literal passed:false fails the gate" {
    make_failing_runner "invariants/run-structural.mjs" "F"
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
    make_failing_runner "window-contract/run-window.mjs" "G"
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
    make_failing_runner "crawler-v2/extended-crawler.mjs" "H"
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
    make_failing_runner "contract-symmetry/run-symmetry.mjs" "I"
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
#
# Genuinely uninstall each metric runner (delete the file at the exact path
# run_metric() resolves) rather than faking absence via a stale/wrong path —
# a directory that merely doesn't exist at the wrong location proves nothing
# once the real path is correct.

@test "gate-stage-2-5: no runners installed still defaults to passed:true (absent key)" {
    rm -f "$STUB_SCRIPTS/invariants/run-structural.mjs" \
          "$STUB_SCRIPTS/window-contract/run-window.mjs" \
          "$STUB_SCRIPTS/crawler-v2/extended-crawler.mjs" \
          "$STUB_SCRIPTS/contract-symmetry/run-symmetry.mjs"
    run bash "$STUB_SCRIPTS/gate-stage-2-5.sh" "$TARGET_DIR"
    [ "$status" -eq 0 ]
    local overall
    overall="$(printf '%s' "$output" | jq -r '.passed')"
    [ "$overall" = "true" ]
    printf '%s' "$output" | grep -q 'runner not installed'
}
