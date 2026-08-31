#!/usr/bin/env bats

bats_require_minimum_version 1.5.0
# skills/autospec-test/tests/unit/gate-stage-2-5-live-server.bats
#
# Coverage for gate-stage-2-5.sh's live-server orchestration: metrics G
# (window_contracts) and I (contract_symmetry) need a real HTTP origin,
# which gate-stage-2-5.sh now provides by starting the target's own
# server.mjs on a harness-chosen loopback port, polling it to readiness,
# and guaranteeing teardown on every exit path.
#
# Scope: this file proves (1) the server is actually started and used as
# base_url instead of the metric being skipped, (2) teardown happens on
# both the success and failure paths — no orphaned process, no port left
# bound, (3) readiness is a real poll, not a sleep, and (4) port allocation
# does not hardcode a port (two targets, or a developer's own process on
# the "obvious" port, do not collide).
#
# Each RED-proof test mutates a *copy* of scripts/ in a tmpdir (the
# established pattern already used by gate-stage-2-5-coercion.bats and
# gate-stage-2-5.bats) rather than the tracked gate-stage-2-5.sh itself, so
# there is nothing to restore afterward — the real file is never touched.

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
    # Belt-and-suspenders: kill anything this test file may have started
    # that a bug under test was specifically designed to leak.
    if [ -n "${LEAK_PID:-}" ]; then
        kill -9 "$LEAK_PID" 2>/dev/null || true
    fi
}

port_is_listening() {
    # $1 = port
    lsof -iTCP:"$1" -sTCP:LISTEN -P -n >/dev/null 2>&1
}

# ── G: actually invoked against a live server, not skipped ──────────────────

@test "target-window-mismatch-bait: metric G is genuinely invoked against a live server (not skipped)" {
    run bash "$GATE" "$TARGETS_DIR/target-window-mismatch-bait" < /dev/null
    printf '%s' "$output" | jq -e '.metrics.G.skipped != true' >/dev/null
    printf '%s' "$output" | jq -e '.metrics.G.reason == null or (.metrics.G.reason | test("live HTTP server") | not)' >/dev/null
}

@test "target-window-mismatch-bait: metric G catches the bait — matches the golden's headline claims" {
    run bash "$GATE" "$TARGETS_DIR/target-window-mismatch-bait" < /dev/null
    printf '%s' "$output" | jq -e '.metrics.G.passed == false' >/dev/null
    printf '%s' "$output" | jq -e '.metrics.G.contracts[0].id == "dashboard-streak-window"' >/dev/null
    printf '%s' "$output" | jq -e '.metrics.G.contracts[0].passed == false' >/dev/null
    printf '%s' "$output" | jq -e '.metrics.G.contracts[0].N == 7' >/dev/null
    printf '%s' "$output" | jq -e '.metrics.G.contracts[0].requests_seen == 1' >/dev/null
    printf '%s' "$output" | jq -e '.metrics.G.contracts[0].violations | length == 1' >/dev/null
    printf '%s' "$output" | jq -e '.metrics.G.contracts[0].violations[0].param == "from"' >/dev/null
}

@test "target-window-mismatch-bait: overall gate now genuinely fails (G caught the bait for real)" {
    run bash "$GATE" "$TARGETS_DIR/target-window-mismatch-bait" < /dev/null
    [ "$status" -eq 1 ]
    printf '%s' "$output" | jq -e '.passed == false' >/dev/null
}

# ── I: actually invoked against a live server, not skipped ──────────────────

@test "target-contract-symmetry-bait: metric I is genuinely invoked against a live server (not skipped)" {
    run bash "$GATE" "$TARGETS_DIR/target-contract-symmetry-bait" < /dev/null
    printf '%s' "$output" | jq -e '.metrics.I.skipped != true' >/dev/null
    printf '%s' "$output" | jq -e '.metrics.I != null' >/dev/null
}

# ── F must not regress: still file://, never gains a server ─────────────────

@test "target-invariant-bait: metric F still uses a file:// base_url and does not gain a live server" {
    run bash "$GATE" "$TARGETS_DIR/target-invariant-bait" < /dev/null
    [ "$status" -eq 1 ]
    printf '%s' "$output" | jq -e '.metrics.F.passed == false' >/dev/null
    printf '%s' "$output" | jq -e '.metrics.F.invariants[0].count_observed == 5' >/dev/null
}

@test "gate-stage-2-5.sh: start_live_server is never invoked when neither window_contracts nor contract_symmetry is declared" {
    # target-invariant-bait's contract has neither block, so NEEDS_WINDOW_SERVER
    # and NEEDS_SYMMETRY_SERVER must both be false and the live-server branch
    # must not run at all.
    run bash -c "AUTOSPEC_SERVER_READY_TIMEOUT_S=1 bash -x '$GATE' '$TARGETS_DIR/target-invariant-bait' 2>&1 | grep -c 'start_live_server '"
    [ "$output" = "0" ] || [ "$output" = "" ]
}

# ── Teardown: no orphaned process, no bound port left behind ────────────────

@test "teardown: no server.mjs process survives a completed gate run" {
    run bash "$GATE" "$TARGETS_DIR/target-window-mismatch-bait" < /dev/null
    run pgrep -f "node src/server.mjs"
    [ "$status" -ne 0 ]
    [ -z "$output" ]
}

@test "teardown: no server.mjs process survives on the readiness-timeout failure path" {
    TEST_TMPDIR="$(mktemp -d /tmp/autospec-gate25-live-teardown-XXXXXX)"
    mkdir -p "$TEST_TMPDIR/target/.autospec" "$TEST_TMPDIR/target/src"
    cat > "$TEST_TMPDIR/target/src/server.mjs" <<'EOF'
// Deliberately never listens: proves the readiness-timeout path still
// tears the process down instead of leaking it.
setInterval(() => {}, 60_000);
EOF
    cat > "$TEST_TMPDIR/target/.autospec/test.yml" <<'YAML'
mode: strict_isolation
e2e:
  start_cmd: "node src/server.mjs"
  invariants_v2:
    enabled: true
    window_contracts:
      - id: never-ready
        ui_display:
          route: /
          widget: '[data-testid=x]'
          window_days_attr: 'data-n'
        api_query:
          method: GET
          path_pattern: '^/api$'
          window_params:
            from: { type: iso_date, must_be: 'today - $N days' }
YAML

    AUTOSPEC_SERVER_READY_TIMEOUT_S=2 run bash "$GATE" "$TEST_TMPDIR/target" < /dev/null
    printf '%s' "$output" | jq -e '.metrics.G.passed == false' >/dev/null
    printf '%s' "$output" | jq -e '.metrics.G.reason | test("never answered")' >/dev/null

    run pgrep -f "node src/server.mjs"
    [ "$status" -ne 0 ]
    [ -z "$output" ]
}

@test "teardown: no server.mjs process survives when the server exits immediately" {
    TEST_TMPDIR="$(mktemp -d /tmp/autospec-gate25-live-diedearly-XXXXXX)"
    mkdir -p "$TEST_TMPDIR/target/.autospec" "$TEST_TMPDIR/target/src"
    cat > "$TEST_TMPDIR/target/src/server.mjs" <<'EOF'
process.exit(3);
EOF
    cat > "$TEST_TMPDIR/target/.autospec/test.yml" <<'YAML'
mode: strict_isolation
e2e:
  start_cmd: "node src/server.mjs"
  invariants_v2:
    enabled: true
    contract_symmetry:
      - id: dies-immediately
        ui_source: { route: /, extract: 'a', per_match: { id: 'data-id' } }
        api_target: { method: GET, path_template: '/api/${id}', must_contain: '$.x' }
YAML

    run bash "$GATE" "$TEST_TMPDIR/target" < /dev/null
    printf '%s' "$output" | jq -e '.metrics.I.passed == false' >/dev/null
    printf '%s' "$output" | jq -e '.metrics.I.reason | test("exited before becoming ready")' >/dev/null

    run pgrep -f "node src/server.mjs"
    [ "$status" -ne 0 ]
    [ -z "$output" ]
}

# ── No start command: loud skip, not a fabricated pass ───────────────────────

@test "no e2e.start_cmd and no matching package.json script: G is loudly skipped, never a silent pass" {
    TEST_TMPDIR="$(mktemp -d /tmp/autospec-gate25-live-nostart-XXXXXX)"
    mkdir -p "$TEST_TMPDIR/target/.autospec"
    cat > "$TEST_TMPDIR/target/.autospec/test.yml" <<'YAML'
mode: strict_isolation
e2e:
  invariants_v2:
    enabled: true
    window_contracts:
      - id: no-start-cmd
        ui_display:
          route: /
          widget: '[data-testid=x]'
          window_days_attr: 'data-n'
        api_query:
          method: GET
          path_pattern: '^/api$'
          window_params:
            from: { type: iso_date, must_be: 'today - $N days' }
YAML

    run bash "$GATE" "$TEST_TMPDIR/target" < /dev/null
    [ "$status" -eq 0 ]
    printf '%s' "$output" | jq -e '.metrics.G.skipped == true' >/dev/null
    printf '%s' "$output" | jq -e '.metrics.G.reason | test("no e2e.start_cmd declared")' >/dev/null
}

# ── Port allocation: no hardcoded port, no collision with an existing listener ──

@test "port allocation: gate succeeds even when the target's own hardcoded default port is already taken" {
    # target-window-mismatch-bait's server.mjs defaults to PORT=3002 when no
    # PORT env var is set. Occupy 3002 with an unrelated foreign listener
    # first; the harness must allocate and use a *different* free port
    # rather than colliding with (or depending on) the target's fallback.
    node -e '
      const net = require("node:net");
      const s = net.createServer(c => c.end());
      s.listen(3002, "127.0.0.1");
      setTimeout(() => {}, 30_000);
    ' &
    LEAK_PID=$!
    sleep 0.3

    run bash "$GATE" "$TARGETS_DIR/target-window-mismatch-bait" < /dev/null
    printf '%s' "$output" | jq -e '.metrics.G.skipped != true' >/dev/null
    printf '%s' "$output" | jq -e '.metrics.G.passed == false' >/dev/null
    printf '%s' "$output" | jq -e '.metrics.G.contracts[0].requests_seen == 1' >/dev/null

    kill -9 "$LEAK_PID" 2>/dev/null || true
    LEAK_PID=""
}

@test "port allocation: two gate runs against the same target in sequence do not collide" {
    run bash "$GATE" "$TARGETS_DIR/target-window-mismatch-bait" < /dev/null
    first_status="$status"
    run bash "$GATE" "$TARGETS_DIR/target-window-mismatch-bait" < /dev/null
    second_status="$status"
    [ "$first_status" -eq 1 ]
    [ "$second_status" -eq 1 ]
    printf '%s' "$output" | jq -e '.metrics.G.passed == false' >/dev/null
}

# ── RED proofs: each guarantee actually matters ──────────────────────────────
# Copy scripts/ to a tmpdir and mutate the copy (never the tracked file), run
# the mutated gate against a throwaway target, and confirm the expected
# safety property breaks. Proves these tests have power, not just green.

@test "RED: without teardown, the server process survives after the gate exits" {
    TEST_TMPDIR="$(mktemp -d /tmp/autospec-gate25-live-red-teardown-XXXXXX)"
    STUB_SCRIPTS="$TEST_TMPDIR/scripts"
    cp -R "$SCRIPTS_DIR" "$STUB_SCRIPTS"

    # Neuter cleanup_live_server into a no-op — the trap still fires, it
    # just no longer kills anything.
    perl -0pi -e 's/cleanup_live_server\(\) \{.*?\n\}\n/cleanup_live_server() {\n    :\n}\n/s' \
        "$STUB_SCRIPTS/gate-stage-2-5.sh"
    run grep -c 'cleanup_live_server() {' "$STUB_SCRIPTS/gate-stage-2-5.sh"
    [ "$output" -ge 1 ]

    TARGET_DIR="$TEST_TMPDIR/target"
    mkdir -p "$TARGET_DIR/.autospec" "$TARGET_DIR/src"
    cp "$TARGETS_DIR/target-window-mismatch-bait/src/server.mjs" "$TARGET_DIR/src/server.mjs"
    cp "$TARGETS_DIR/target-window-mismatch-bait/src/index.html" "$TARGET_DIR/src/index.html"
    cp "$TARGETS_DIR/target-window-mismatch-bait/.autospec/test.yml" "$TARGET_DIR/.autospec/test.yml"

    run bash "$STUB_SCRIPTS/gate-stage-2-5.sh" "$TARGET_DIR" < /dev/null

    run pgrep -f "node src/server.mjs"
    [ "$status" -eq 0 ]
    [ -n "$output" ]
    # Clean up the leaked process ourselves — this is the whole point of the
    # RED proof (the harness under test failed to).
    for leaked_pid in $output; do
        kill -9 "$leaked_pid" 2>/dev/null || true
    done
}

@test "RED: without a real readiness poll, a server that never listens is wrongly treated as ready" {
    TEST_TMPDIR="$(mktemp -d /tmp/autospec-gate25-live-red-ready-XXXXXX)"
    STUB_SCRIPTS="$TEST_TMPDIR/scripts"
    cp -R "$SCRIPTS_DIR" "$STUB_SCRIPTS"

    # Replace the real poll with one that always claims success instantly —
    # the shape a fixed `sleep` (guess-and-hope) degrades to.
    perl -0pi -e 's/wait_for_ready\(\) \{.*?\n\}\n/wait_for_ready() {\n    return 0\n}\n/s' \
        "$STUB_SCRIPTS/gate-stage-2-5.sh"

    TARGET_DIR="$TEST_TMPDIR/target"
    mkdir -p "$TARGET_DIR/.autospec" "$TARGET_DIR/src"
    cat > "$TARGET_DIR/src/server.mjs" <<'EOF'
// Never listens.
setInterval(() => {}, 60_000);
EOF
    cat > "$TARGET_DIR/.autospec/test.yml" <<'YAML'
mode: strict_isolation
e2e:
  start_cmd: "node src/server.mjs"
  invariants_v2:
    enabled: true
    window_contracts:
      - id: never-listens
        ui_display:
          route: /
          widget: '[data-testid=x]'
          window_days_attr: 'data-n'
        api_query:
          method: GET
          path_pattern: '^/api$'
          window_params:
            from: { type: iso_date, must_be: 'today - $N days' }
YAML

    run timeout 20 bash "$STUB_SCRIPTS/gate-stage-2-5.sh" "$TARGET_DIR" < /dev/null
    # With the poll faked out, the gate proceeds to invoke run-window.mjs
    # against a server that was never actually ready — it does NOT surface
    # the honest "never answered" failure this suite otherwise proves.
    run ! grep -q 'never answered' <<<"$output"
    [ "$status" -ne 0 ]

    for leaked_pid in $(pgrep -f "node src/server.mjs" 2>/dev/null || true); do
        kill -9 "$leaked_pid" 2>/dev/null || true
    done
}

@test "RED: with a hardcoded port instead of real allocation, a taken port causes collision" {
    TEST_TMPDIR="$(mktemp -d /tmp/autospec-gate25-live-red-port-XXXXXX)"
    STUB_SCRIPTS="$TEST_TMPDIR/scripts"
    cp -R "$SCRIPTS_DIR" "$STUB_SCRIPTS"

    # Force allocation to always return the same fixed port instead of
    # asking the OS for a free one.
    perl -0pi -e 's/find_free_port\(\) \{.*?\n\}\n/find_free_port() {\n    printf "38217"\n}\n/s' \
        "$STUB_SCRIPTS/gate-stage-2-5.sh"

    # Occupy that fixed port with an unrelated foreign listener first.
    node -e '
      const net = require("node:net");
      const s = net.createServer(c => c.end());
      s.listen(38217, "127.0.0.1");
      setTimeout(() => {}, 30_000);
    ' &
    LEAK_PID=$!
    sleep 0.3

    TARGET_DIR="$TEST_TMPDIR/target"
    mkdir -p "$TARGET_DIR/.autospec" "$TARGET_DIR/src"
    cp "$TARGETS_DIR/target-window-mismatch-bait/src/server.mjs" "$TARGET_DIR/src/server.mjs"
    cp "$TARGETS_DIR/target-window-mismatch-bait/src/index.html" "$TARGET_DIR/src/index.html"
    cp "$TARGETS_DIR/target-window-mismatch-bait/.autospec/test.yml" "$TARGET_DIR/.autospec/test.yml"

    AUTOSPEC_SERVER_READY_TIMEOUT_S=3 run timeout 20 bash "$STUB_SCRIPTS/gate-stage-2-5.sh" "$TARGET_DIR" < /dev/null
    # The target's server.mjs fails to bind (EADDRINUSE) since the port is
    # already taken by the foreign listener — readiness never succeeds.
    printf '%s' "$output" | jq -e '.metrics.G.passed == false' >/dev/null

    kill -9 "$LEAK_PID" 2>/dev/null || true
    LEAK_PID=""
}

# ── Metric I: catches its bait for real (not a vacuous zero-tuple pass) ─────
#
# History: ui-extractor.mjs's per_match loop destructured
# `for (const [attrName, tupleKey] of Object.entries(per_match))`, but the
# design spec's per_match convention is {logical_key: dom_attribute} (e.g.
# `task_id: 'data-task-id'`) — so it called `el.getAttribute("task_id")`
# (the logical key, never a real DOM attribute) instead of
# `el.getAttribute("data-task-id")`. Every tuple extraction silently
# produced 0 tuples, and run-symmetry.mjs's `contractPassed =
# violations.length === 0` trivially reported passed:true for a contract
# that had examined nothing — the fail-open shape called out below.
# Fixed by (1) swapping the destructuring order to match the documented
# convention, and (2) making zero extracted tuples an explicit violation
# instead of a silent pass.

@test "target-contract-symmetry-bait: metric I catches the bait — matches the golden's headline claims" {
    run bash "$GATE" "$TARGETS_DIR/target-contract-symmetry-bait" < /dev/null
    printf '%s' "$output" | jq -e '.metrics.I.skipped != true' >/dev/null
    printf '%s' "$output" | jq -e '.metrics.I.passed == false' >/dev/null
    printf '%s' "$output" | jq -e '.metrics.I.contracts[0].id == "streak-task-must-be-editable"' >/dev/null
    printf '%s' "$output" | jq -e '.metrics.I.contracts[0].passed == false' >/dev/null
    printf '%s' "$output" | jq -e '.metrics.I.contracts[0].tuples_checked == 3' >/dev/null
    printf '%s' "$output" | jq -e '.metrics.I.summary.total == 1 and .metrics.I.summary.passed_count == 0 and .metrics.I.summary.failed_count == 1 and .metrics.I.summary.violation_count == 1' >/dev/null
}

@test "target-contract-symmetry-bait: overall gate now genuinely fails (I caught the bait for real)" {
    run bash "$GATE" "$TARGETS_DIR/target-contract-symmetry-bait" < /dev/null
    [ "$status" -eq 1 ]
    printf '%s' "$output" | jq -e '.passed == false' >/dev/null
}

# ── Regression pin: a zero-tuple extraction must never report passed:true ───
#
# This is the systemic fix: `tuples_checked: 0` combined with
# `violations.length === 0` used to trivially report passed:true — a check
# that examined nothing was indistinguishable from a check that examined
# everything and found no problems. Same fail-open family as a metric
# skipped-but-marked-passed, a jq `// true` coercion of a real `false`, and
# an app harness reporting a process "started" that was never started.

@test "zero tuples extracted: run-symmetry.mjs reports passed:false, not a vacuous pass" {
    TEST_TMPDIR="$(mktemp -d /tmp/autospec-gate25-live-zerotuple-XXXXXX)"
    mkdir -p "$TEST_TMPDIR/target/.autospec" "$TEST_TMPDIR/target/src"
    cat > "$TEST_TMPDIR/target/src/server.mjs" <<'EOF'
import http from 'node:http';
const PORT = parseInt(process.env.PORT ?? '4001', 10);
const HOST = process.env.HOST ?? '127.0.0.1';
const server = http.createServer((req, res) => {
  res.writeHead(200, { 'Content-Type': 'text/html' });
  res.end('<!doctype html><html><body>no matching elements on this page</body></html>');
});
server.listen(PORT, HOST, () => process.stdout.write(`ready on ${HOST}:${PORT}\n`));
EOF
    cat > "$TEST_TMPDIR/target/.autospec/test.yml" <<'YAML'
mode: strict_isolation
e2e:
  start_cmd: "node src/server.mjs"
  invariants_v2:
    enabled: true
    contract_symmetry:
      - id: broken-selector
        ui_source:
          route: /
          extract: '[data-testid^="does-not-exist-"]'
          per_match: { task_id: 'data-task-id' }
        api_target:
          method: GET
          path_template: '/api/${task_id}'
          must_contain: '$.x'
YAML

    run bash "$GATE" "$TEST_TMPDIR/target" < /dev/null
    [ "$status" -eq 1 ]
    printf '%s' "$output" | jq -e '.metrics.I.passed == false' >/dev/null
    printf '%s' "$output" | jq -e '.metrics.I.contracts[0].tuples_checked == 0' >/dev/null
    printf '%s' "$output" | jq -e '.metrics.I.contracts[0].passed == false' >/dev/null
    printf '%s' "$output" | jq -e '.metrics.I.contracts[0].violations[0].phase == "ui_extract"' >/dev/null
    printf '%s' "$output" | jq -e '.metrics.I.contracts[0].violations[0].reason | test("matched 0 elements")' >/dev/null

    run pgrep -f "node src/server.mjs"
    [ "$status" -ne 0 ]
    [ -z "$output" ]
}

@test "RED: without the zero-tuple guard, a broken selector silently reports passed:true" {
    TEST_TMPDIR="$(mktemp -d /tmp/autospec-gate25-live-red-zerotuple-XXXXXX)"
    STUB_SCRIPTS="$TEST_TMPDIR/scripts"
    cp -R "$SCRIPTS_DIR" "$STUB_SCRIPTS"

    # Remove the guard: restore the pre-fix behavior where tuples.length===0
    # falls straight through to the per-tuple loop (a no-op) and
    # `contractPassed = violations.length === 0` trivially reports true.
    perl -0pi -e 's/\n      \/\/ A contract that extracted nothing.*?\n      if \(tuples\.length === 0\) \{.*?\n        continue;\n      \}\n/\n/s' \
        "$STUB_SCRIPTS/contract-symmetry/run-symmetry.mjs"
    run ! grep -q 'tuples.length === 0' "$STUB_SCRIPTS/contract-symmetry/run-symmetry.mjs"
    [ "$status" -ne 0 ]

    TARGET_DIR="$TEST_TMPDIR/target"
    mkdir -p "$TARGET_DIR/.autospec" "$TARGET_DIR/src"
    cat > "$TARGET_DIR/src/server.mjs" <<'EOF'
import http from 'node:http';
const PORT = parseInt(process.env.PORT ?? '4001', 10);
const HOST = process.env.HOST ?? '127.0.0.1';
const server = http.createServer((req, res) => {
  res.writeHead(200, { 'Content-Type': 'text/html' });
  res.end('<!doctype html><html><body>no matching elements on this page</body></html>');
});
server.listen(PORT, HOST, () => process.stdout.write(`ready on ${HOST}:${PORT}\n`));
EOF
    cat > "$TARGET_DIR/.autospec/test.yml" <<'YAML'
mode: strict_isolation
e2e:
  start_cmd: "node src/server.mjs"
  invariants_v2:
    enabled: true
    contract_symmetry:
      - id: broken-selector
        ui_source:
          route: /
          extract: '[data-testid^="does-not-exist-"]'
          per_match: { task_id: 'data-task-id' }
        api_target:
          method: GET
          path_template: '/api/${task_id}'
          must_contain: '$.x'
YAML

    run bash "$STUB_SCRIPTS/gate-stage-2-5.sh" "$TARGET_DIR" < /dev/null
    # With the guard removed, the same broken-selector scenario that must
    # fail loudly above instead reports a vacuous pass — proving the guard
    # (not incidental behavior) is what makes the pinned test above pass.
    printf '%s' "$output" | jq -e '.metrics.I.passed == true' >/dev/null
    printf '%s' "$output" | jq -e '.metrics.I.contracts[0].tuples_checked == 0' >/dev/null
}
