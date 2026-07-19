#!/usr/bin/env bats
# Tests for advisor-config.sh — resolves the advisor config from .autospec/autospec.yml.
# Precedence: env override (CI/test escape hatch) > autospec.yml > built-in default.

setup() {
  SCRIPT="${BATS_TEST_DIRNAME}/../../scripts/advisor-config.sh"
  TMP="$(mktemp -d)"
  export AUTOSPEC_CONFIG_FILE="$TMP/autospec.yml"
  # Clear any inherited overrides so tests are deterministic.
  unset AUTOSPEC_ADVISOR_POLICY AUTOSPEC_ADVISOR_MAX_USES AUTOSPEC_ADVISOR_MAX_CHARS
}

teardown() { rm -rf "$TMP"; }

@test "defaults when no config file and no overrides" {
  rm -f "$AUTOSPEC_CONFIG_FILE"
  [ "$(bash "$SCRIPT" --key policy)" = "auto" ]
  [ "$(bash "$SCRIPT" --key budget.max_calls_per_issue)" = "3" ]
  [ "$(bash "$SCRIPT" --key budget.guidance_char_cap)" = "2800" ]
}

@test "reads values from the advisor block in autospec.yml" {
  cat > "$AUTOSPEC_CONFIG_FILE" <<'EOF'
version: 1
advisor:
  policy: 'off'
  budget:
    max_calls_per_issue: 5
    guidance_char_cap: 1200
EOF
  [ "$(bash "$SCRIPT" --key policy)" = "off" ]
  [ "$(bash "$SCRIPT" --key budget.max_calls_per_issue)" = "5" ]
  [ "$(bash "$SCRIPT" --key budget.guidance_char_cap)" = "1200" ]
}

@test "env override beats the yaml value" {
  cat > "$AUTOSPEC_CONFIG_FILE" <<'EOF'
advisor:
  policy: 'on'
EOF
  export AUTOSPEC_ADVISOR_POLICY=off
  [ "$(bash "$SCRIPT" --key policy)" = "off" ]
  export AUTOSPEC_ADVISOR_MAX_USES=9
  [ "$(bash "$SCRIPT" --key budget.max_calls_per_issue)" = "9" ]
}

@test "missing advisor block falls back to defaults" {
  cat > "$AUTOSPEC_CONFIG_FILE" <<'EOF'
version: 1
project:
  profile: full
EOF
  [ "$(bash "$SCRIPT" --key policy)" = "auto" ]
  [ "$(bash "$SCRIPT" --key budget.max_calls_per_issue)" = "3" ]
}

@test "invalid policy value falls back to the default (auto)" {
  cat > "$AUTOSPEC_CONFIG_FILE" <<'EOF'
advisor:
  policy: 'sometimes'
EOF
  [ "$(bash "$SCRIPT" --key policy)" = "auto" ]
}

@test "unknown key errors" {
  run bash "$SCRIPT" --key nope.nope
  [ "$status" -ne 0 ]
}

@test "unquoted policy: off (YAML boolean) is honored as off, not auto" {
  cat > "$AUTOSPEC_CONFIG_FILE" <<'EOF'
advisor:
  policy: off
EOF
  [ "$(bash "$SCRIPT" --key policy)" = "off" ]
}

@test "unquoted policy: on (YAML boolean) is honored as on" {
  cat > "$AUTOSPEC_CONFIG_FILE" <<'EOF'
advisor:
  policy: on
EOF
  [ "$(bash "$SCRIPT" --key policy)" = "on" ]
}

@test "malformed yaml falls back to defaults (fail-open)" {
  printf 'advisor:\n  policy: [unterminated\n' > "$AUTOSPEC_CONFIG_FILE"
  [ "$(bash "$SCRIPT" --key policy)" = "auto" ]
}

@test "non-dict advisor node falls back to defaults" {
  printf 'advisor: "just a string"\n' > "$AUTOSPEC_CONFIG_FILE"
  [ "$(bash "$SCRIPT" --key policy)" = "auto" ]
  [ "$(bash "$SCRIPT" --key budget.max_calls_per_issue)" = "3" ]
}
