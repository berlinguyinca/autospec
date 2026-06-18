#!/usr/bin/env bats
# codemod-route-next.bats — TDD suite for codemod-route.sh Next.js routing (issue #1178)
# No network access, no real installs. All npx calls mocked via $TMP/bin PATH shim.

SCRIPT="${BATS_TEST_DIRNAME}/../scripts/codemod-route.sh"

# ── Setup / teardown ──────────────────────────────────────────────────────────

setup() {
  MOCK_BIN="$(mktemp -d /tmp/cr-next-mock-bin.XXXXXX)"
  RECORDER="$MOCK_BIN/npx-calls.txt"
  export MOCK_BIN RECORDER

  # Fake npx: appends full argv (space-joined) to RECORDER, exits 0
  cat > "$MOCK_BIN/npx" <<'MOCKEOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$RECORDER"
exit 0
MOCKEOF
  chmod +x "$MOCK_BIN/npx"
}

teardown() {
  rm -rf "$MOCK_BIN"
}

# ── Existence / executability ─────────────────────────────────────────────────

@test "codemod-route.sh exists and is executable" {
  [ -x "$SCRIPT" ]
}

# ── Next.js hop via CLI ───────────────────────────────────────────────────────

@test "next hop: CLI invokes 'npx @next/codemod upgrade'" {
  run env PATH="$MOCK_BIN:$PATH" RECORDER="$RECORDER" \
    "$SCRIPT" next 15
  [ "$status" -eq 0 ]
  grep -F "@next/codemod upgrade" "$RECORDER"
}

@test "next hop: exactly one npx call (no extra invocations)" {
  env PATH="$MOCK_BIN:$PATH" RECORDER="$RECORDER" \
    "$SCRIPT" next 15
  count="$(wc -l < "$RECORDER" | tr -d ' ')"
  [ "$count" -eq 1 ]
}

# ── Next.js hop via route_codemod function (sourced) ─────────────────────────

@test "route_codemod next 15: invokes npx @next/codemod upgrade" {
  run env PATH="$MOCK_BIN:$PATH" RECORDER="$RECORDER" \
    bash -c ". '$SCRIPT' && route_codemod next 15"
  [ "$status" -eq 0 ]
  grep -F "@next/codemod upgrade" "$RECORDER"
}

@test "route_next_codemod 15: invokes npx @next/codemod upgrade" {
  run env PATH="$MOCK_BIN:$PATH" RECORDER="$RECORDER" \
    bash -c ". '$SCRIPT' && route_next_codemod 15"
  [ "$status" -eq 0 ]
  grep -F "@next/codemod upgrade" "$RECORDER"
}

# ── Async-request-api mode ────────────────────────────────────────────────────

@test "next --async-request-api: CLI invokes 'npx @next/codemod next-async-request-api'" {
  run env PATH="$MOCK_BIN:$PATH" RECORDER="$RECORDER" \
    "$SCRIPT" next 15 --async-request-api
  [ "$status" -eq 0 ]
  grep -F "@next/codemod next-async-request-api" "$RECORDER"
}

@test "next --async-request-api: exactly one npx call" {
  env PATH="$MOCK_BIN:$PATH" RECORDER="$RECORDER" \
    "$SCRIPT" next 15 --async-request-api
  count="$(wc -l < "$RECORDER" | tr -d ' ')"
  [ "$count" -eq 1 ]
}

@test "route_codemod next 15 --async-request-api: invokes next-async-request-api codemod" {
  run env PATH="$MOCK_BIN:$PATH" RECORDER="$RECORDER" \
    bash -c ". '$SCRIPT' && route_codemod next 15 --async-request-api"
  [ "$status" -eq 0 ]
  grep -F "@next/codemod next-async-request-api" "$RECORDER"
}

# ── No hand-rolled migrations ─────────────────────────────────────────────────

@test "next hop: npx is invoked, not ng/npm-install for the upgrade itself" {
  env PATH="$MOCK_BIN:$PATH" RECORDER="$RECORDER" \
    "$SCRIPT" next 15

  # The recorded call must include @next/codemod (official codemod package)
  grep -F "@next/codemod" "$RECORDER"
}
