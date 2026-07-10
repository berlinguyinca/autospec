#!/usr/bin/env bats
# grooming-config.bats — resolver for the grooming: config block.

setup() { TMP="$(mktemp -d)"; export AUTOSPEC_CONFIG_FILE="$TMP/autospec.yml"; SCRIPT="${BATS_TEST_DIRNAME}/../scripts/grooming-config.sh"; }
teardown() { rm -rf "$TMP"; }

@test "policy defaults to auto when no config" { printf 'version: 1\n' > "$AUTOSPEC_CONFIG_FILE"; run bash "$SCRIPT" --key policy; [ "$status" -eq 0 ]; [ "$output" = "auto" ]; }
@test "policy reads yaml value" { printf 'grooming:\n  policy: on\n' > "$AUTOSPEC_CONFIG_FILE"; run bash "$SCRIPT" --key policy; [ "$output" = "on" ]; }
@test "unquoted off is normalized (not PyYAML boolean False)" { printf 'grooming:\n  policy: off\n' > "$AUTOSPEC_CONFIG_FILE"; run bash "$SCRIPT" --key policy; [ "$output" = "off" ]; }
@test "env overrides yaml" { printf 'grooming:\n  policy: off\n' > "$AUTOSPEC_CONFIG_FILE"; AUTOSPEC_GROOMING_POLICY=auto run bash "$SCRIPT" --key policy; [ "$output" = "auto" ]; }
@test "budget.max_issues_per_cycle default 5" { printf 'version: 1\n' > "$AUTOSPEC_CONFIG_FILE"; run bash "$SCRIPT" --key budget.max_issues_per_cycle; [ "$output" = "5" ]; }
@test "valid grooming block resolves policy and both budgets" {
  cat > "$AUTOSPEC_CONFIG_FILE" <<'EOF'
grooming:
  policy: on
  budget:
    max_issues_per_cycle: 7
    groom_attempts_per_issue: 3
EOF
  run bash "$SCRIPT" --key policy; [ "$output" = "on" ]
  run bash "$SCRIPT" --key budget.max_issues_per_cycle; [ "$output" = "7" ]
  run bash "$SCRIPT" --key budget.groom_attempts_per_issue; [ "$output" = "3" ]
}
@test "invalid policy value falls back to the default (auto)" {
  printf 'grooming:\n  policy: bogus\n' > "$AUTOSPEC_CONFIG_FILE"
  run bash "$SCRIPT" --key policy; [ "$output" = "auto" ]
}
@test "negative max_issues_per_cycle falls back to the default (5)" {
  printf 'grooming:\n  budget:\n    max_issues_per_cycle: -1\n' > "$AUTOSPEC_CONFIG_FILE"
  run bash "$SCRIPT" --key budget.max_issues_per_cycle; [ "$output" = "5" ]
}
@test "negative groom_attempts_per_issue falls back to the default (2)" {
  printf 'grooming:\n  budget:\n    groom_attempts_per_issue: -3\n' > "$AUTOSPEC_CONFIG_FILE"
  run bash "$SCRIPT" --key budget.groom_attempts_per_issue; [ "$output" = "2" ]
}
@test "corrupt/unparseable config fails CLOSED to off (present-but-broken != absent)" {
  # A tab in indentation makes PyYAML raise; the kill switch must NOT silently
  # re-enable grooming by falling back to the auto default.
  printf 'grooming:\n\tpolicy: on\n  bogus: [unterminated\n' > "$AUTOSPEC_CONFIG_FILE"
  run bash "$SCRIPT" --key policy
  [ "$status" -eq 0 ]
  [ "$output" = "off" ]
}
@test "top-level non-mapping config fails CLOSED to off" {
  printf -- '- just\n- a\n- list\n' > "$AUTOSPEC_CONFIG_FILE"
  run bash "$SCRIPT" --key policy
  [ "$output" = "off" ]
}
@test "missing config file resolves to auto (absent != broken)" {
  rm -f "$AUTOSPEC_CONFIG_FILE"
  run bash "$SCRIPT" --key policy
  [ "$status" -eq 0 ]
  [ "$output" = "auto" ]
}

@test "canary_floor: default is 5 when unset" {
  run env -u AUTOSPEC_GROOMING_CANARY_FLOOR AUTOSPEC_CONFIG_FILE=/nonexistent \
    bash "$SCRIPT" --key budget.canary_floor
  [ "$status" -eq 0 ]
  [ "$output" = "5" ]
}

@test "canary_floor: env override wins" {
  run env AUTOSPEC_GROOMING_CANARY_FLOOR=9 bash "$SCRIPT" --key budget.canary_floor
  [ "$status" -eq 0 ]
  [ "$output" = "9" ]
}

@test "canary_floor: reads from yaml grooming.budget.canary_floor" {
  cfg="$BATS_TEST_TMPDIR/autospec.yml"
  printf 'grooming:\n  budget:\n    canary_floor: 7\n' > "$cfg"
  run env -u AUTOSPEC_GROOMING_CANARY_FLOOR AUTOSPEC_CONFIG_FILE="$cfg" \
    bash "$SCRIPT" --key budget.canary_floor
  [ "$status" -eq 0 ]
  [ "$output" = "7" ]
}

@test "canary_floor: non-numeric yaml falls back to default 5" {
  cfg="$BATS_TEST_TMPDIR/autospec.yml"
  printf 'grooming:\n  budget:\n    canary_floor: banana\n' > "$cfg"
  run env -u AUTOSPEC_GROOMING_CANARY_FLOOR AUTOSPEC_CONFIG_FILE="$cfg" \
    bash "$SCRIPT" --key budget.canary_floor
  [ "$status" -eq 0 ]
  [ "$output" = "5" ]
}
