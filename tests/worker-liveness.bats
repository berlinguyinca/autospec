#!/usr/bin/env bats
# tests/worker-liveness.bats — host-match + PID-liveness decision helper
#
# External boundaries mocked:
#   WORKER_LIVENESS_HOSTNAME — injects a fake hostname so cross-host tests
#   don't require a second machine.  The real kill -0 is used for PID
#   liveness; tests use $$ (guaranteed alive) and 999999 (guaranteed dead).

SCRIPT="${BATS_TEST_DIRNAME}/../scripts/worker-liveness.sh"

setup() {
    THIS_HOST="$(hostname)"
    THIS_PID="$$"
}

# ── basic sanity ──────────────────────────────────────────────────────────────

@test "worker-liveness.sh exists and is executable" {
    [ -x "$SCRIPT" ]
}

@test "bash -n passes (no syntax errors)" {
    bash -n "$SCRIPT"
}

# ── same host, live pid ───────────────────────────────────────────────────────

@test "same-host with live pid ($$) prints alive, exit 0" {
    run bash "$SCRIPT" "${THIS_HOST}:testuser:autospec-run:${THIS_PID}"
    [ "$status" -eq 0 ]
    [ "$output" = "alive" ]
}

# ── same host, dead pid ───────────────────────────────────────────────────────

@test "same-host with dead pid (999999) prints dead, exit 0" {
    # 999999 is above the typical PID max on macOS/Linux and will not exist.
    run bash "$SCRIPT" "${THIS_HOST}:testuser:autospec-run:999999"
    [ "$status" -eq 0 ]
    [ "$output" = "dead" ]
}

# ── cross host ────────────────────────────────────────────────────────────────

@test "different host prints unknown, exit 0" {
    # Inject a hostname that differs from $(hostname) so the same-host branch
    # is not taken, even when this test runs on a machine named "other-host".
    run env WORKER_LIVENESS_HOSTNAME="__test_override_host__" \
        bash "$SCRIPT" "other-host.example.com:testuser:autospec-run:12345"
    [ "$status" -eq 0 ]
    [ "$output" = "unknown" ]
}

@test "fabricated cross-host worker_id returns unknown regardless of pid" {
    run env WORKER_LIVENESS_HOSTNAME="myhost" \
        bash "$SCRIPT" "notmyhost:bob:autospec-run:${THIS_PID}"
    [ "$status" -eq 0 ]
    [ "$output" = "unknown" ]
}

# ── malformed / empty ─────────────────────────────────────────────────────────

@test "empty worker_id prints unknown, exit 0" {
    run bash "$SCRIPT" ""
    [ "$status" -eq 0 ]
    [ "$output" = "unknown" ]
}

@test "no args prints unknown, exit 0" {
    run bash "$SCRIPT"
    [ "$status" -eq 0 ]
    [ "$output" = "unknown" ]
}

@test "too few fields (host:user) prints unknown, exit 0" {
    run bash "$SCRIPT" "somehost:someuser"
    [ "$status" -eq 0 ]
    [ "$output" = "unknown" ]
}

@test "three fields (host:user:harness) prints unknown, exit 0" {
    run bash "$SCRIPT" "somehost:someuser:autospec-run"
    [ "$status" -eq 0 ]
    [ "$output" = "unknown" ]
}

@test "non-numeric pid in same-host worker_id prints unknown, exit 0" {
    run bash "$SCRIPT" "${THIS_HOST}:testuser:autospec-run:notapid"
    [ "$status" -eq 0 ]
    [ "$output" = "unknown" ]
}
