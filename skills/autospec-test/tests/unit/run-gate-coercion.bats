#!/usr/bin/env bats
# skills/autospec-test/tests/unit/run-gate-coercion.bats
#
# jq alternative-operator ("//") coercion guard for run-gate.sh.
#
# jq's "//" treats a literal `false` the same as an absent key, so
# `.passed // true` silently coerces a real failure back to a pass. This
# covers the two run-gate.sh sites that used that shape:
#   - S25_PASSED  (stage 2.5 overall pass/fail feeding into OVERALL)
#   - RESTORE_SUCCEEDED (stage2.metrics.restore_succeeded feeding e2e:* labels)

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../../../.." && pwd)"
    REAL_SCRIPTS_DIR="$REPO_ROOT/skills/autospec-test/scripts"

    TEST_TMPDIR="$(mktemp -d /tmp/autospec-run-gate-bats-XXXXXX)"
}

teardown() {
    rm -rf "$TEST_TMPDIR"
}

# ── S25_PASSED: stage 2.5 "passed":false must block OVERALL ─────────────────
# Stub gate-stage-unit.sh and gate-stage-e2e.sh to always pass, and
# gate-stage-2-5.sh to explicitly emit passed:false, so the only way OVERALL
# can go true is via the "// true" coercion bug on S25_PASSED.

@test "run-gate: stage2.5 literal passed:false blocks overall_passed" {
    local stub_scripts="$TEST_TMPDIR/scripts"
    cp -R "$REAL_SCRIPTS_DIR" "$stub_scripts"

    cat > "$stub_scripts/gate-stage-unit.sh" <<'EOF'
#!/usr/bin/env bash
cat >/dev/null
printf '{"passed":true,"stage":"unit"}\n'
exit 0
EOF
    cat > "$stub_scripts/gate-stage-e2e.sh" <<'EOF'
#!/usr/bin/env bash
printf '{"passed":true,"stage":"e2e"}\n'
exit 0
EOF
    cat > "$stub_scripts/gate-stage-2-5.sh" <<'EOF'
#!/usr/bin/env bash
printf '{"passed":false,"reason":"forced-fail-for-coercion-test"}\n'
exit 0
EOF
    chmod +x "$stub_scripts/gate-stage-unit.sh" "$stub_scripts/gate-stage-e2e.sh" "$stub_scripts/gate-stage-2-5.sh"

    local target_dir="$TEST_TMPDIR/target"
    mkdir -p "$target_dir/.autospec"
    printf 'mode: strict_isolation\n' > "$target_dir/.autospec/test.yml"

    run bash "$stub_scripts/run-gate.sh" "$target_dir"
    # overall_passed must be false -> exit 1 (gate blocks the PR)
    [ "$status" -eq 1 ]
    overall="$(printf '%s' "$output" | jq -r '.overall_passed')"
    [ "$overall" = "false" ]
}

# ── RESTORE_SUCCEEDED: literal false must select the CRITICAL label path ────
# Uses the .autospec/stub-gate.json mechanism run-gate.sh already supports for
# golden-diff tests, so no real gate stage or GitHub API call is needed. `gh`
# is stubbed to log its argv instead of touching the network.

@test "run-gate: stage2.metrics.restore_succeeded:false selects restore-failed label" {
    local target_dir="$TEST_TMPDIR/target"
    mkdir -p "$target_dir/.autospec"
    printf 'mode: strict_isolation\n' > "$target_dir/.autospec/test.yml"

    jq -n '{
        "target": "stub",
        "overall_passed": false,
        "stage2": {
            "reason": "scope-violation",
            "metrics": {
                "scope_violation": true,
                "restore_succeeded": false
            }
        }
    }' > "$target_dir/.autospec/stub-gate.json"

    local fake_bin="$TEST_TMPDIR/bin"
    mkdir -p "$fake_bin"
    local gh_log="$TEST_TMPDIR/gh.log"
    : > "$gh_log"
    cat > "$fake_bin/gh" <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$*" >> "$gh_log"
exit 0
EOF
    chmod +x "$fake_bin/gh"

    local old_path="$PATH"
    export PATH="$fake_bin:$PATH"
    run bash "$REAL_SCRIPTS_DIR/run-gate.sh" "$target_dir" --pr 999
    export PATH="$old_path"

    # Only inspect the actual `pr edit --add-label` calls, not the unrelated
    # `bootstrap-labels` step (which creates every known label name up front).
    local restore_calls
    restore_calls="$(grep -c '^pr edit .*e2e:restored' "$gh_log" || true)"
    [ "${restore_calls:-0}" -eq 0 ]
    local failed_calls
    failed_calls="$(grep -c '^pr edit .*e2e:restore-failed,CRITICAL' "$gh_log" || true)"
    [ "${failed_calls:-0}" -eq 1 ]
}
