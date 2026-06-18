#!/usr/bin/env bats
# codemod-route-angular.bats — TDD suite for codemod-route.sh Angular routing (issue #1177)
# No network access, no real installs. All ng calls mocked via $TMP/bin PATH shim.

SCRIPT="${BATS_TEST_DIRNAME}/../scripts/codemod-route.sh"

# ── Setup / teardown ──────────────────────────────────────────────────────────

setup() {
  MOCK_BIN="$(mktemp -d /tmp/cr-mock-bin.XXXXXX)"
  RECORDER="$MOCK_BIN/ng-calls.txt"
  export MOCK_BIN RECORDER

  # Fake ng: appends full argv (space-joined) to RECORDER, exits 0
  cat > "$MOCK_BIN/ng" <<'MOCKEOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$RECORDER"
exit 0
MOCKEOF
  chmod +x "$MOCK_BIN/ng"
}

teardown() {
  rm -rf "$MOCK_BIN"
}

# ── Existence / executability ─────────────────────────────────────────────────

@test "codemod-route.sh exists and is executable" {
  [ -x "$SCRIPT" ]
}

# ── Angular 17→18 hop via CLI ─────────────────────────────────────────────────

@test "angular 18 hop: CLI invokes 'ng update @angular/core@18 @angular/cli@18'" {
  run env PATH="$MOCK_BIN:$PATH" RECORDER="$RECORDER" \
    "$SCRIPT" angular 18
  [ "$status" -eq 0 ]
  grep -Fx "update @angular/core@18 @angular/cli@18" "$RECORDER"
}

@test "angular 18 hop: exactly one ng call (no extra invocations)" {
  env PATH="$MOCK_BIN:$PATH" RECORDER="$RECORDER" \
    "$SCRIPT" angular 18
  count="$(wc -l < "$RECORDER" | tr -d ' ')"
  [ "$count" -eq 1 ]
}

# ── Angular hop via route_codemod function (sourced) ─────────────────────────

@test "route_codemod angular 18: invokes ng update with correct packages" {
  run env PATH="$MOCK_BIN:$PATH" RECORDER="$RECORDER" \
    bash -c ". '$SCRIPT' && route_codemod angular 18"
  [ "$status" -eq 0 ]
  grep -Fx "update @angular/core@18 @angular/cli@18" "$RECORDER"
}

@test "route_codemod angular 17: invokes ng update @angular/core@17 @angular/cli@17" {
  run env PATH="$MOCK_BIN:$PATH" RECORDER="$RECORDER" \
    bash -c ". '$SCRIPT' && route_codemod angular 17"
  [ "$status" -eq 0 ]
  grep -Fx "update @angular/core@17 @angular/cli@17" "$RECORDER"
}

@test "route_angular_codemod 19: invokes ng update @angular/core@19 @angular/cli@19" {
  run env PATH="$MOCK_BIN:$PATH" RECORDER="$RECORDER" \
    bash -c ". '$SCRIPT' && route_angular_codemod 19"
  [ "$status" -eq 0 ]
  grep -Fx "update @angular/core@19 @angular/cli@19" "$RECORDER"
}

# ── Standalone mode ───────────────────────────────────────────────────────────

@test "angular 18 --standalone: first call is ng update" {
  run env PATH="$MOCK_BIN:$PATH" RECORDER="$RECORDER" \
    "$SCRIPT" angular 18 --standalone
  [ "$status" -eq 0 ]
  head -1 "$RECORDER" | grep -F "update @angular/core@18 @angular/cli@18"
}

@test "angular 18 --standalone: second call is 'ng generate @angular/core:standalone'" {
  env PATH="$MOCK_BIN:$PATH" RECORDER="$RECORDER" \
    "$SCRIPT" angular 18 --standalone
  sed -n '2p' "$RECORDER" | grep -F "generate @angular/core:standalone"
}

@test "angular 18 --standalone: exactly two ng calls total" {
  env PATH="$MOCK_BIN:$PATH" RECORDER="$RECORDER" \
    "$SCRIPT" angular 18 --standalone
  count="$(wc -l < "$RECORDER" | tr -d ' ')"
  [ "$count" -eq 2 ]
}

@test "route_codemod angular 18 --standalone: invokes standalone schematic" {
  run env PATH="$MOCK_BIN:$PATH" RECORDER="$RECORDER" \
    bash -c ". '$SCRIPT' && route_codemod angular 18 --standalone"
  [ "$status" -eq 0 ]
  grep -F "generate @angular/core:standalone" "$RECORDER"
}

# ── Unknown framework errors ──────────────────────────────────────────────────

@test "route_codemod unknown framework: exits non-zero" {
  run env PATH="$MOCK_BIN:$PATH" RECORDER="$RECORDER" \
    "$SCRIPT" vue 3
  [ "$status" -ne 0 ]
}

@test "route_codemod unknown framework: stderr mentions code_health:upgrade_unknown_framework" {
  run env PATH="$MOCK_BIN:$PATH" RECORDER="$RECORDER" \
    "$SCRIPT" vue 3
  [ "$status" -ne 0 ]
  printf '%s\n' "$output" | grep -qi "upgrade_unknown_framework"
}

@test "route_codemod svelte 4: exits non-zero" {
  run env PATH="$MOCK_BIN:$PATH" RECORDER="$RECORDER" \
    "$SCRIPT" svelte 4
  [ "$status" -ne 0 ]
}

@test "route_codemod missing framework arg: exits non-zero" {
  run env PATH="$MOCK_BIN:$PATH" RECORDER="$RECORDER" \
    "$SCRIPT"
  [ "$status" -ne 0 ]
}

# ── No hand-rolled migrations (ng calls are exactly the official schematics) ──

@test "angular hop: ng is invoked, not npm/npx/yarn for the upgrade itself" {
  env PATH="$MOCK_BIN:$PATH" RECORDER="$RECORDER" \
    "$SCRIPT" angular 18

  # The recorded call must start with 'update' (ng subcommand), not 'npm install' etc.
  first_word="$(head -1 "$RECORDER" | cut -d' ' -f1)"
  [ "$first_word" = "update" ]
}
