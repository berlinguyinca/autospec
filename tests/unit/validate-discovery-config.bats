#!/usr/bin/env bats

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
V="$REPO_ROOT/skills/autospec-shared/scripts/validate-discovery-config.sh"

setup() { TMP="$(mktemp -d)"; }
teardown() { rm -rf "$TMP"; }

valid_yml() {
  cat > "$TMP/c.yml" <<'YML'
discovery:
  enabled: true
  seed_sources:
    - hackernews
    - github-trending
  forbidden_classes:
    - malware
  max_new_sources_per_round: 3
  userspace:
    opt_out: false
  rate_limits:
    internet-forums:
      per_hour: 10
YML
  echo "$TMP/c.yml"
}

@test "script exists and is bash -n clean" {
  [ -f "$V" ]; run bash -n "$V"; [ "$status" -eq 0 ]
}

@test "accepts a complete valid config" {
  run bash "$V" "$(valid_yml)"
  [ "$status" -eq 0 ]
}

@test "accepts a config missing the discovery block entirely" {
  printf 'product:\n  name: acme\n' > "$TMP/c.yml"
  run bash "$V" "$TMP/c.yml"
  [ "$status" -eq 0 ]
}

@test "accepts a discovery block missing optional keys" {
  printf 'discovery:\n  enabled: true\n' > "$TMP/c.yml"
  run bash "$V" "$TMP/c.yml"
  [ "$status" -eq 0 ]
}

@test "rejects non-boolean discovery.enabled" {
  printf 'discovery:\n  enabled: 3\n' > "$TMP/c.yml"
  run bash "$V" "$TMP/c.yml"
  [ "$status" -eq 1 ]
  [[ "$output" == *"enabled"* ]]
}

@test "rejects non-array discovery.seed_sources" {
  printf 'discovery:\n  seed_sources: "hackernews"\n' > "$TMP/c.yml"
  run bash "$V" "$TMP/c.yml"
  [ "$status" -eq 1 ]
  [[ "$output" == *"seed_sources"* ]]
}

@test "rejects non-array discovery.forbidden_classes" {
  printf 'discovery:\n  forbidden_classes: "malware"\n' > "$TMP/c.yml"
  run bash "$V" "$TMP/c.yml"
  [ "$status" -eq 1 ]
  [[ "$output" == *"forbidden_classes"* ]]
}

@test "rejects non-integer discovery.max_new_sources_per_round" {
  printf 'discovery:\n  max_new_sources_per_round: "three"\n' > "$TMP/c.yml"
  run bash "$V" "$TMP/c.yml"
  [ "$status" -eq 1 ]
  [[ "$output" == *"max_new_sources_per_round"* ]]
}

@test "rejects negative discovery.max_new_sources_per_round" {
  printf 'discovery:\n  max_new_sources_per_round: -1\n' > "$TMP/c.yml"
  run bash "$V" "$TMP/c.yml"
  [ "$status" -eq 1 ]
  [[ "$output" == *"max_new_sources_per_round"* ]]
}

@test "rejects non-boolean discovery.userspace.opt_out" {
  printf 'discovery:\n  userspace:\n    opt_out: "yes"\n' > "$TMP/c.yml"
  run bash "$V" "$TMP/c.yml"
  [ "$status" -eq 1 ]
  [[ "$output" == *"opt_out"* ]]
}

@test "rejects non-object discovery.rate_limits" {
  printf 'discovery:\n  rate_limits: "fast"\n' > "$TMP/c.yml"
  run bash "$V" "$TMP/c.yml"
  [ "$status" -eq 1 ]
  [[ "$output" == *"rate_limits"* ]]
}

@test "rejects an inline secret under discovery.rate_limits" {
  cat > "$TMP/c.yml" <<'YML'
discovery:
  rate_limits:
    internet-forums:
      token: sk-live-abc123
YML
  run bash "$V" "$TMP/c.yml"
  [ "$status" -eq 1 ]
  [[ "$output" == *"secret"* || "$output" == *"_env"* ]]
}

@test "accepts an env-var-name secret reference under discovery.rate_limits" {
  cat > "$TMP/c.yml" <<'YML'
discovery:
  rate_limits:
    internet-forums:
      token_env: FORUMS_API_TOKEN
YML
  run bash "$V" "$TMP/c.yml"
  [ "$status" -eq 0 ]
}

@test "rejects a missing config file" {
  run bash "$V" "$TMP/does-not-exist.yml"
  [ "$status" -eq 2 ]
}

@test "rejects unparseable YAML" {
  printf 'discovery: [unterminated\n' > "$TMP/c.yml"
  run bash "$V" "$TMP/c.yml"
  [ "$status" -eq 2 ]
}
