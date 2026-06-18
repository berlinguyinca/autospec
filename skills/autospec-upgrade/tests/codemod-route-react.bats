#!/usr/bin/env bats
# codemod-route-react.bats — TDD suite for codemod-route.sh React routing (issue #1179)
# No network access, no real installs. All npx calls mocked via $TMP/bin PATH shim.

SCRIPT="${BATS_TEST_DIRNAME}/../scripts/codemod-route.sh"

# ── Setup / teardown ──────────────────────────────────────────────────────────

setup() {
  MOCK_BIN="$(mktemp -d /tmp/cr-react-mock-bin.XXXXXX)"
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

# ── React 19 hop via CLI ──────────────────────────────────────────────────────

@test "react hop: CLI invokes 'npx codemod react/19/migration-recipe'" {
  run env PATH="$MOCK_BIN:$PATH" RECORDER="$RECORDER" \
    "$SCRIPT" react 19
  [ "$status" -eq 0 ]
  grep -F "codemod react/19/migration-recipe" "$RECORDER"
}

@test "react hop: exactly one npx call (no extra invocations)" {
  env PATH="$MOCK_BIN:$PATH" RECORDER="$RECORDER" \
    "$SCRIPT" react 19
  count="$(wc -l < "$RECORDER" | tr -d ' ')"
  [ "$count" -eq 1 ]
}

# ── React 19 hop via route_codemod function (sourced) ────────────────────────

@test "route_codemod react 19: invokes npx codemod react/19/migration-recipe" {
  run env PATH="$MOCK_BIN:$PATH" RECORDER="$RECORDER" \
    bash -c ". '$SCRIPT' && route_codemod react 19"
  [ "$status" -eq 0 ]
  grep -F "codemod react/19/migration-recipe" "$RECORDER"
}

@test "route_react_codemod 19: invokes npx codemod react/19/migration-recipe" {
  run env PATH="$MOCK_BIN:$PATH" RECORDER="$RECORDER" \
    bash -c ". '$SCRIPT' && route_react_codemod 19"
  [ "$status" -eq 0 ]
  grep -F "codemod react/19/migration-recipe" "$RECORDER"
}

# ── Types mode ────────────────────────────────────────────────────────────────

@test "react --types: CLI invokes 'npx types-react-codemod preset-19'" {
  run env PATH="$MOCK_BIN:$PATH" RECORDER="$RECORDER" \
    "$SCRIPT" react 19 --types
  [ "$status" -eq 0 ]
  grep -F "types-react-codemod preset-19" "$RECORDER"
}

@test "react --types: exactly one npx call" {
  env PATH="$MOCK_BIN:$PATH" RECORDER="$RECORDER" \
    "$SCRIPT" react 19 --types
  count="$(wc -l < "$RECORDER" | tr -d ' ')"
  [ "$count" -eq 1 ]
}

@test "route_codemod react 19 --types: invokes types-react-codemod preset-19" {
  run env PATH="$MOCK_BIN:$PATH" RECORDER="$RECORDER" \
    bash -c ". '$SCRIPT' && route_codemod react 19 --types"
  [ "$status" -eq 0 ]
  grep -F "types-react-codemod preset-19" "$RECORDER"
}

# ── No hand-rolled migrations ─────────────────────────────────────────────────

@test "react hop: npx is invoked with the official codemod package" {
  env PATH="$MOCK_BIN:$PATH" RECORDER="$RECORDER" \
    "$SCRIPT" react 19

  # The recorded call must use the official codemod package
  grep -F "codemod react/19/migration-recipe" "$RECORDER"
}
