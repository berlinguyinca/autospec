#!/usr/bin/env bats
# skills/autospec-test/tests/unit/gate-stage-2-5.bats
#
# Regression coverage for gate-stage-2-5.sh's F/G/H/I metric wiring.
#
# History:
#   1. run_metric() originally resolved every runner as
#      SCRIPT_DIR/../invariants/$2, a directory that does not exist, so every
#      metric silently took the stub-pass fallback
#      ({"passed":true,"skipped":true,"reason":"runner not installed"})
#      instead of ever invoking its real runner. Fixed to
#      SCRIPT_DIR/$2 with each call site subdirectory-qualified.
#   2. Once the path was fixed, a second gap surfaced: F/G/H/I all read a
#      { contract, base_url, ... } JSON document from stdin, but
#      gate-stage-2-5.sh invoked `node "$runner" "$TARGET_DIR"` — a bare
#      positional argv, no stdin, no base_url. Every runner immediately hit
#      "stdin must have { contract, base_url }" and exited 2 ("refused").
#      This file now covers the fix for (2): gate-stage-2-5.sh parses
#      .autospec/test.yml to JSON via yq, works out a file:// base_url from
#      the target's static src/index.html (or index.html) fixture, and pipes
#      {contract, base_url} to each runner's stdin.
#
# Scope limit (intentional, not a bug): F and H support only static
# fixtures reachable at route "/" via a file:// URL — F must never gain a
# live server. A target whose contract needs a live HTTP server — declared
# window_contracts (metric G) or contract_symmetry (metric I) — now gets
# one: gate-stage-2-5.sh starts the target's own server.mjs on a
# harness-chosen loopback port (see resolve_start_cmd/start_live_server/
# wait_for_ready), polls it to readiness, and uses it as base_url for G/I.
# See gate-stage-2-5-live-server.bats for the live-server orchestration
# coverage (start command discovery, port allocation, readiness polling,
# teardown guarantees, and the G/I bait-catching verdicts). Only a target
# with no static index.html at all (target-greenwash-bait) or no
# discoverable start_cmd still skips loudly, rather than being invoked with
# a payload that could never produce a real verdict.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../../../.." && pwd)"
    SCRIPTS_DIR="$REPO_ROOT/skills/autospec-test/scripts"
    GATE="$SCRIPTS_DIR/gate-stage-2-5.sh"
    TARGETS_DIR="$REPO_ROOT/skills/autospec-test/test-targets"
}

teardown() {
    if [ -n "${TEST_TMPDIR:-}" ]; then
        rm -rf "$TEST_TMPDIR"
    fi
}

# ── Runner paths actually exist where the fixed script looks for them ───────

@test "invariants/run-structural.mjs exists at the path run_metric now resolves" {
    [ -f "$SCRIPTS_DIR/invariants/run-structural.mjs" ]
}

@test "window-contract/run-window.mjs exists at the path run_metric now resolves" {
    [ -f "$SCRIPTS_DIR/window-contract/run-window.mjs" ]
}

@test "crawler-v2/extended-crawler.mjs exists at the path run_metric now resolves" {
    [ -f "$SCRIPTS_DIR/crawler-v2/extended-crawler.mjs" ]
}

@test "contract-symmetry/run-symmetry.mjs exists at the path run_metric now resolves" {
    [ -f "$SCRIPTS_DIR/contract-symmetry/run-symmetry.mjs" ]
}

@test "gate-stage-2-5.sh no longer references the nonexistent ../invariants/\$2 prefix" {
    ! grep -q '\.\./invariants/\$2' "$GATE"
}

# ── target-invariant-bait: metric F is genuinely invoked with a well-formed
# {contract, base_url} stdin payload and returns a real verdict ────────────

@test "target-invariant-bait: gate output is not the stub-pass reason for any metric" {
    run bash "$GATE" "$TARGETS_DIR/target-invariant-bait" < /dev/null
    ! printf '%s' "$output" | grep -q 'runner not installed'
}

@test "target-invariant-bait: metric F is not refused and not a raw runner crash" {
    run bash "$GATE" "$TARGETS_DIR/target-invariant-bait" < /dev/null
    printf '%s' "$output" | jq -e '.metrics.F.refused == null' >/dev/null
    printf '%s' "$output" | jq -e '.metrics.F.raw == null' >/dev/null
}

@test "target-invariant-bait: metric F genuinely caught the bait — matches the golden's headline claims" {
    run bash "$GATE" "$TARGETS_DIR/target-invariant-bait" < /dev/null
    printf '%s' "$output" | jq -e '.metrics.F.passed == false' >/dev/null
    printf '%s' "$output" | jq -e '.metrics.F.summary.count_observed == null and (.metrics.F.invariants[0].count_observed == 5)' >/dev/null
    printf '%s' "$output" | jq -e '.metrics.F.invariants[0].violations[0].index == 4' >/dev/null
    printf '%s' "$output" | jq -e '.metrics.F.invariants[0].route == "/"' >/dev/null
}

@test "target-invariant-bait: overall gate fails (F caught the bait for real)" {
    run bash "$GATE" "$TARGETS_DIR/target-invariant-bait" < /dev/null
    [ "$status" -eq 1 ]
    printf '%s' "$output" | jq -e '.passed == false' >/dev/null
}

# Break payload construction (force an empty base_url even though a static
# fixture exists) and confirm the F assertions above go RED, proving they
# actually exercise the wiring rather than trivially passing.
@test "RED proof: an empty base_url makes metric F refuse instead of catching the bait" {
    TEST_TMPDIR="$(mktemp -d /tmp/autospec-gate25-redf-bats-XXXXXX)"
    payload='{"contract":{"e2e":{"invariants_v2":{"enabled":true,"invariants":[{"id":"x","kind":"every_visible_X_is_Y","visible":"body","action":"button","apply_on_routes":["/"]}]}}},"base_url":""}'
    run bash -c "printf '%s' '$payload' | node '$SCRIPTS_DIR/invariants/run-structural.mjs'"
    [ "$status" -eq 2 ]
    printf '%s' "$output" | grep -q 'fatal: stdin must have'
}

# ── target-window-mismatch-bait: metric G needs a live HTTP server
# (network-observable request). gate-stage-2-5.sh now stands one up (see
# gate-stage-2-5-live-server.bats for the full orchestration coverage), so
# G runs for real and must catch the bait, not skip. ────────────────────────

@test "target-window-mismatch-bait: gate output is not the stub-pass reason for any metric" {
    run bash "$GATE" "$TARGETS_DIR/target-window-mismatch-bait" < /dev/null
    ! printf '%s' "$output" | grep -q 'runner not installed'
}

@test "target-window-mismatch-bait: metric G is genuinely invoked, not skipped" {
    run bash "$GATE" "$TARGETS_DIR/target-window-mismatch-bait" < /dev/null
    printf '%s' "$output" | jq -e '.metrics.G.skipped != true' >/dev/null
    printf '%s' "$output" | jq -e '.metrics.G.passed == false' >/dev/null
}

@test "target-window-mismatch-bait: overall gate now fails (G caught the bait for real)" {
    run bash "$GATE" "$TARGETS_DIR/target-window-mismatch-bait" < /dev/null
    [ "$status" -eq 1 ]
    printf '%s' "$output" | jq -e '.passed == false' >/dev/null
}

# ── target-greenwash-bait: no static index.html fixture at all (its
# apply_on_routes targets /peaks on a page that was never shipped as a static
# file) — every metric needing a base_url must skip loudly. ─────────────────

@test "target-greenwash-bait: gate output is not the stub-pass reason for any metric" {
    run bash "$GATE" "$TARGETS_DIR/target-greenwash-bait" < /dev/null
    ! printf '%s' "$output" | grep -q 'runner not installed'
}

@test "target-greenwash-bait: metric F is loudly skipped as needing a static fixture, not silently" {
    run bash "$GATE" "$TARGETS_DIR/target-greenwash-bait" < /dev/null
    printf '%s' "$output" | jq -e '.metrics.F.skipped == true' >/dev/null
    printf '%s' "$output" | jq -e '.metrics.F.reason | test("no static index.html found")' >/dev/null
}

@test "target-greenwash-bait: overall gate passes (no static frontend to evaluate, honestly skipped)" {
    run bash "$GATE" "$TARGETS_DIR/target-greenwash-bait" < /dev/null
    [ "$status" -eq 0 ]
    printf '%s' "$output" | jq -e '.passed == true' >/dev/null
}

# ── target-contract-symmetry-bait: metric I needs a live HTTP server to
# fetch and compare API responses — same shape as the G case above. ─────────
#
# I is now genuinely invoked against a live server (not skipped) and matches
# the golden's structural claims (passed:false, tuples_checked:3, summary
# counts). Two real bugs were fixed to get here, both filed as findings, not
# golden-fitting hacks — see gate-stage-2-5-live-server.bats for the full
# "catches the bait" and zero-tuple-regression coverage:
#   1. contract-symmetry/ui-extractor.mjs's per_match loop destructured
#      `for (const [attrName, tupleKey] of Object.entries(per_match))`, but
#      per_match is declared as {logical_key: dom_attribute} per the design
#      spec (docs/specs/2026-05-21-autospec-test-invariants-design.md), so it
#      called `el.getAttribute("task_id")` (the logical key) instead of
#      `el.getAttribute("data-task-id")` (the actual DOM attribute) — every
#      extraction produced 0 tuples.
#   2. Once fixed, tuples.length === 0 silently reported passed:true (the
#      systemic fail-open pattern seen elsewhere in this codebase); a
#      zero-tuple contract now reports an explicit ui_extract violation and
#      passed:false instead.

@test "target-contract-symmetry-bait: gate output is not the stub-pass reason for any metric" {
    run bash "$GATE" "$TARGETS_DIR/target-contract-symmetry-bait" < /dev/null
    ! printf '%s' "$output" | grep -q 'runner not installed'
}

@test "target-contract-symmetry-bait: metric I is genuinely invoked against a live server, not skipped" {
    run bash "$GATE" "$TARGETS_DIR/target-contract-symmetry-bait" < /dev/null
    printf '%s' "$output" | jq -e '.metrics.I.skipped != true' >/dev/null
    printf '%s' "$output" | jq -e '.metrics.I.contracts[0].id == "streak-task-must-be-editable"' >/dev/null
    printf '%s' "$output" | jq -e '.metrics.I.passed == false' >/dev/null
}

# ── target-clean-pass: no invariants_v2 block, gate must skip (not run at all) ──

@test "target-clean-pass: gate short-circuits as skipped (no invariants_v2 declared)" {
    run bash "$GATE" "$TARGETS_DIR/target-clean-pass" < /dev/null
    [ "$status" -eq 0 ]
    printf '%s' "$output" | jq -e '.skipped == true and .passed == true' >/dev/null
}

# ── jq "// true" default-operator bug: false is falsy in jq, so `.passed //
# true` silently rewrote every real "passed":false into "true". Assert the
# fixed filter is used instead. ─────────────────────────────────────────────

@test "gate-stage-2-5.sh no longer uses the jq '.passed // true' false-is-falsy bug" {
    ! grep -q '\.passed // true' "$GATE"
}

@test "a metric JSON with explicit passed:false is honored, not coerced to true" {
    result=$(printf '{"metric":"F","passed":false}' | jq -r 'if .passed == null then true else .passed end')
    [ "$result" = "false" ]
}

# ── verify-seeds.mjs path-resolution bug: same shape, fifth site ────────────
# VERIFY_SEEDS was also resolved at "$SCRIPT_DIR/../invariants/verify-seeds.mjs",
# a directory that does not exist. It's [ -f ]-gated with a silent skip (no
# stub-pass JSON, no log line — the whole edge_case_seeds check just never
# ran), so seed verification silently no-opped for as long as that path was
# wrong. The real file lives at "$SCRIPT_DIR/seed-shapes/verify-seeds.mjs".

@test "seed-shapes/verify-seeds.mjs exists at the path the seeds check now resolves" {
    [ -f "$SCRIPTS_DIR/seed-shapes/verify-seeds.mjs" ]
}

@test "gate-stage-2-5.sh no longer references the nonexistent ../invariants/verify-seeds.mjs path" {
    run grep -q '\.\./invariants/verify-seeds\.mjs' "$GATE"
    [ "$status" -ne 0 ]
}

@test "gate-stage-2-5.sh invokes verify-seeds.mjs with --contract/--dsn/--store-kind, not a bare positional" {
    run grep -q -- '--contract "\$CONTRACT_YML"' "$GATE"
    [ "$status" -eq 0 ]
    run grep -q -- '--store-kind' "$GATE"
    [ "$status" -eq 0 ]
}

# Overwrite the real verify-seeds.mjs (at its correct, resolved path) with a
# stub that writes a distinctive line and exits non-zero. If the gate still
# resolved the old, nonexistent path, [ -f ] would be false, the whole seeds
# block would be silently skipped (no output, no log line at all), and this
# stub would never run. gate-stage-2-5.sh streams the runner's combined
# stdout+stderr straight through (not captured into a variable), so the
# stub's own distinctive line appearing in the gate's output is direct proof
# the runner was actually invoked — with named flags, since the stub ignores
# its own argv and this only tests that node was invoked at all, the named
# flags themselves are asserted by the grep test above.
@test "edge_case_seeds declared: verify-seeds.mjs is actually invoked, not silently skipped" {
    TEST_TMPDIR="$(mktemp -d /tmp/autospec-gate25-seeds-bats-XXXXXX)"
    STUB_SCRIPTS="$TEST_TMPDIR/scripts"
    cp -R "$SCRIPTS_DIR" "$STUB_SCRIPTS"

    cat > "$STUB_SCRIPTS/seed-shapes/verify-seeds.mjs" <<'EOF'
#!/usr/bin/env node
process.stderr.write('stub-seed-verify-refused\n');
process.exit(2);
EOF
    chmod +x "$STUB_SCRIPTS/seed-shapes/verify-seeds.mjs"

    target_dir="$TEST_TMPDIR/target"
    mkdir -p "$target_dir/.autospec"
    cat > "$target_dir/.autospec/test.yml" <<'YAML'
mode: strict_isolation
e2e:
  invariants_v2:
    enabled: true
    edge_case_seeds:
      household_test_family:
        require_shapes:
          - name: overdue_task
            count_min: 1
YAML

    run bash "$STUB_SCRIPTS/gate-stage-2-5.sh" "$target_dir" < /dev/null
    printf '%s' "$output" | grep -q 'stub-seed-verify-refused'
}

# ── SEED_EXIT bug: `if ! node ...; then SEED_EXIT=$?` captured the exit code
# of the *negated* `if !` test, which is always 0 (the negation of "node
# failed" is always true, and `$?` after `if !` reflects that test's own
# result, not node's). The `[ "$SEED_EXIT" -eq 2 ]` fatal-exit branch was
# therefore dead code no matter what verify-seeds.mjs exited with. Fixed by
# running node outside the `if`/`!` and capturing `$?` directly via
# `node ... || SEED_EXIT=$?`. This test proves the previously-dead fatal
# branch (gate exits 2, prints the fatal message) is now actually reachable.

@test "SEED_EXIT fix: verify-seeds.mjs exiting 2 makes gate-stage-2-5.sh exit 2 (fatal), not silently continue" {
    TEST_TMPDIR="$(mktemp -d /tmp/autospec-gate25-seedexit-bats-XXXXXX)"
    STUB_SCRIPTS="$TEST_TMPDIR/scripts"
    cp -R "$SCRIPTS_DIR" "$STUB_SCRIPTS"

    cat > "$STUB_SCRIPTS/seed-shapes/verify-seeds.mjs" <<'EOF'
#!/usr/bin/env node
process.stderr.write('stub-seed-verify-refused-fatal\n');
process.exit(2);
EOF
    chmod +x "$STUB_SCRIPTS/seed-shapes/verify-seeds.mjs"

    target_dir="$TEST_TMPDIR/target"
    mkdir -p "$target_dir/.autospec"
    cat > "$target_dir/.autospec/test.yml" <<'YAML'
mode: strict_isolation
e2e:
  invariants_v2:
    enabled: true
    edge_case_seeds:
      household_test_family:
        require_shapes:
          - name: overdue_task
            count_min: 1
YAML

    run bash "$STUB_SCRIPTS/gate-stage-2-5.sh" "$target_dir" < /dev/null
    [ "$status" -eq 2 ]
    printf '%s' "$output" | grep -q 'fatal: edge_case_seeds verification refused to run'
}
