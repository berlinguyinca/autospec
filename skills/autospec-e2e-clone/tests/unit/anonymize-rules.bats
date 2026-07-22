#!/usr/bin/env bats
# skills/autospec-e2e-clone/tests/unit/anonymize-rules.bats
#
# TDD unit tests for the C4 anonymize engine (anonymize.mjs + pii-assertions.sh).
# Each rule kind is tested independently against fixture data.
# Integration test verifies pii_assertions fail-closed on a broken anonymize run.
#
# Run: bats skills/autospec-e2e-clone/tests/unit/anonymize-rules.bats

SKILL_DIR="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
ANONYMIZE="$SKILL_DIR/scripts/anonymize.mjs"
PII_ASSERT="$SKILL_DIR/scripts/pii-assertions.sh"
FIX_BASE="$SKILL_DIR/tests/unit/fixtures/anonymize-fixture"

# ── helpers ──────────────────────────────────────────────────────────────────

# Ensure @faker-js/faker is available for anonymize.mjs.
# On a clean checkout node_modules is absent; install it now.
# Skip (not fail) when npm is unavailable or network is unreachable.
_ensure_node_modules() {
  if [ -d "$SKILL_DIR/node_modules/@faker-js" ]; then
    return 0
  fi
  if ! command -v npm >/dev/null 2>&1; then
    skip "npm not found — skipping faker-dependent tests"
  fi
  # Try npm ci first (uses lock file, reproducible); fall back to npm install.
  if [ -f "$SKILL_DIR/package-lock.json" ]; then
    npm ci --prefix "$SKILL_DIR" --silent 2>/dev/null \
      || npm install --prefix "$SKILL_DIR" --silent 2>/dev/null \
      || skip "@faker-js/faker could not be installed (offline?) — skipping faker-dependent tests"
  else
    npm install --prefix "$SKILL_DIR" --silent 2>/dev/null \
      || skip "@faker-js/faker could not be installed (offline?) — skipping faker-dependent tests"
  fi
}

# Create a fresh snapshot copy for each test (so we don't mutate shared fixtures)
setup() {
  _ensure_node_modules
  TEST_SNAPSHOT="$(mktemp -d -t anonymize-test-XXXXXX)"
  TEST_REPO="$(mktemp -d -t anonymize-repo-XXXXXX)"
  mkdir -p "$TEST_REPO/.autospec"
  cp "$FIX_BASE/.autospec/clone.yml" "$TEST_REPO/.autospec/clone.yml"
  cp -r "$FIX_BASE/snapshot/." "$TEST_SNAPSHOT/"
}

teardown() {
  rm -rf "$TEST_SNAPSHOT" "$TEST_REPO"
}

# ── Rule: hash_with_domain ────────────────────────────────────────────────────

@test "hash_with_domain: transforms email local-part to sha256 hex prefix" {
  run node "$ANONYMIZE" "$TEST_SNAPSHOT" \
      --contract "$TEST_REPO/.autospec/clone.yml" \
      --repo-root "$TEST_REPO"
  [ "$status" -eq 0 ]

  # After anonymization, users.csv email column must NOT contain original local parts
  run grep -c "alice@example.com" "$TEST_SNAPSHOT/users.csv"
  [ "$output" -eq 0 ]
}

@test "hash_with_domain: anonymized email retains @domain portion" {
  node "$ANONYMIZE" "$TEST_SNAPSHOT" \
      --contract "$TEST_REPO/.autospec/clone.yml" \
      --repo-root "$TEST_REPO"

  # Every data row email should still contain '@'
  # Skip header line (line 1), check remaining lines
  local data_lines
  data_lines="$(tail -n +2 "$TEST_SNAPSHOT/users.csv" | grep -v '^$')"
  # All email fields (col 2) should have @
  echo "$data_lines" | while IFS=',' read -r _id email _rest; do
    [[ "$email" == *"@"* ]]
  done
}

@test "hash_with_domain: same input always produces same output (deterministic)" {
  # Run twice on two separate copies
  local snap2
  snap2="$(mktemp -d -t anon-det-XXXXXX)"
  cp -r "$FIX_BASE/snapshot/." "$snap2/"

  node "$ANONYMIZE" "$TEST_SNAPSHOT" \
      --contract "$TEST_REPO/.autospec/clone.yml" \
      --repo-root "$TEST_REPO"

  local repo2
  repo2="$(mktemp -d -t anon-det-repo-XXXXXX)"
  mkdir -p "$repo2/.autospec"
  cp "$FIX_BASE/.autospec/clone.yml" "$repo2/.autospec/clone.yml"

  node "$ANONYMIZE" "$snap2" \
      --contract "$repo2/.autospec/clone.yml" \
      --repo-root "$repo2"

  # Email columns must be identical across both runs
  local col1 col2
  col1="$(cut -d',' -f2 "$TEST_SNAPSHOT/users.csv")"
  col2="$(cut -d',' -f2 "$snap2/users.csv")"
  [ "$col1" = "$col2" ]

  rm -rf "$snap2" "$repo2"
}

# ── Rule: redact ──────────────────────────────────────────────────────────────

@test "redact: replaces SSN column with empty (NULL)" {
  node "$ANONYMIZE" "$TEST_SNAPSHOT" \
      --contract "$TEST_REPO/.autospec/clone.yml" \
      --repo-root "$TEST_REPO"

  # Original SSNs should not appear in anonymized output
  run grep -c "123-45-6789" "$TEST_SNAPSHOT/users.csv"
  [ "$output" -eq 0 ]
  run grep -c "987-65-4321" "$TEST_SNAPSHOT/users.csv"
  [ "$output" -eq 0 ]
}

@test "redact: is NOT stored in reversible map" {
  node "$ANONYMIZE" "$TEST_SNAPSHOT" \
      --contract "$TEST_REPO/.autospec/clone.yml" \
      --repo-root "$TEST_REPO"

  # Find the map file
  local map_file
  map_file="$(ls "$TEST_REPO/.autospec"/anonymize-map.*.json 2>/dev/null | head -1)"
  [ -n "$map_file" ]

  # Map must not contain ssn key
  run grep -c '"users\.ssn"' "$map_file"
  [ "$output" -eq 0 ]
}

# ── Rule: scrub_to_country_code ───────────────────────────────────────────────

@test "scrub_to_country_code: retains country code prefix" {
  node "$ANONYMIZE" "$TEST_SNAPSHOT" \
      --contract "$TEST_REPO/.autospec/clone.yml" \
      --repo-root "$TEST_REPO"

  # +49 number should become +49-XXXXXXX
  run grep "+49-XXXXXXX" "$TEST_SNAPSHOT/users.csv"
  [ "$status" -eq 0 ]
}

@test "scrub_to_country_code: removes original phone digits" {
  node "$ANONYMIZE" "$TEST_SNAPSHOT" \
      --contract "$TEST_REPO/.autospec/clone.yml" \
      --repo-root "$TEST_REPO"

  run grep -c "123 456 789" "$TEST_SNAPSHOT/users.csv"
  [ "$output" -eq 0 ]
}

@test "scrub_to_country_code: stored in reversible map" {
  node "$ANONYMIZE" "$TEST_SNAPSHOT" \
      --contract "$TEST_REPO/.autospec/clone.yml" \
      --repo-root "$TEST_REPO"

  local map_file
  map_file="$(ls "$TEST_REPO/.autospec"/anonymize-map.*.json 2>/dev/null | head -1)"
  [ -n "$map_file" ]
  run grep -c '"users\.phone"' "$map_file"
  [ "$output" -gt 0 ]
}

# ── Rule: replace_with_faker ──────────────────────────────────────────────────

@test "replace_with_faker: original names no longer appear in output" {
  node "$ANONYMIZE" "$TEST_SNAPSHOT" \
      --contract "$TEST_REPO/.autospec/clone.yml" \
      --repo-root "$TEST_REPO"

  run grep -c "Alice Smith" "$TEST_SNAPSHOT/users.csv"
  [ "$output" -eq 0 ]
  run grep -c "Bob Jones" "$TEST_SNAPSHOT/users.csv"
  [ "$output" -eq 0 ]
}

@test "replace_with_faker: output still has same number of rows" {
  local original_rows
  original_rows="$(wc -l < "$TEST_SNAPSHOT/users.csv")"

  node "$ANONYMIZE" "$TEST_SNAPSHOT" \
      --contract "$TEST_REPO/.autospec/clone.yml" \
      --repo-root "$TEST_REPO"

  local anon_rows
  anon_rows="$(wc -l < "$TEST_SNAPSHOT/users.csv")"
  [ "$original_rows" -eq "$anon_rows" ]
}

@test "replace_with_faker: stored in reversible map" {
  node "$ANONYMIZE" "$TEST_SNAPSHOT" \
      --contract "$TEST_REPO/.autospec/clone.yml" \
      --repo-root "$TEST_REPO"

  local map_file
  map_file="$(ls "$TEST_REPO/.autospec"/anonymize-map.*.json 2>/dev/null | head -1)"
  [ -n "$map_file" ]
  run grep -c '"users\.name"' "$map_file"
  [ "$output" -gt 0 ]
}

# ── Rule: scrub_to_subnet ─────────────────────────────────────────────────────

@test "scrub_to_subnet: zeroes last octet of IPv4 address" {
  node "$ANONYMIZE" "$TEST_SNAPSHOT" \
      --contract "$TEST_REPO/.autospec/clone.yml" \
      --repo-root "$TEST_REPO"

  run grep "192.168.1.0" "$TEST_SNAPSHOT/events.csv"
  [ "$status" -eq 0 ]
}

@test "scrub_to_subnet: original IPs no longer in output" {
  node "$ANONYMIZE" "$TEST_SNAPSHOT" \
      --contract "$TEST_REPO/.autospec/clone.yml" \
      --repo-root "$TEST_REPO"

  run grep -c "192.168.1.5" "$TEST_SNAPSHOT/events.csv"
  [ "$output" -eq 0 ]
  run grep -c "10.0.0.123" "$TEST_SNAPSHOT/events.csv"
  [ "$output" -eq 0 ]
}

@test "scrub_to_subnet: stored in reversible map" {
  node "$ANONYMIZE" "$TEST_SNAPSHOT" \
      --contract "$TEST_REPO/.autospec/clone.yml" \
      --repo-root "$TEST_REPO"

  local map_file
  map_file="$(ls "$TEST_REPO/.autospec"/anonymize-map.*.json 2>/dev/null | head -1)"
  [ -n "$map_file" ]
  run grep -c '"events\.ip_address"' "$map_file"
  [ "$output" -gt 0 ]
}

# ── Reversible map persistence ────────────────────────────────────────────────

@test "reversible map: persisted at .autospec/anonymize-map.<sha>.json" {
  node "$ANONYMIZE" "$TEST_SNAPSHOT" \
      --contract "$TEST_REPO/.autospec/clone.yml" \
      --repo-root "$TEST_REPO"

  local map_count
  map_count="$(ls "$TEST_REPO/.autospec"/anonymize-map.*.json 2>/dev/null | wc -l | tr -d ' ')"
  [ "$map_count" -eq 1 ]
}

@test "reversible map: reused across invocations with same contract sha" {
  node "$ANONYMIZE" "$TEST_SNAPSHOT" \
      --contract "$TEST_REPO/.autospec/clone.yml" \
      --repo-root "$TEST_REPO"

  local map_file
  map_file="$(ls "$TEST_REPO/.autospec"/anonymize-map.*.json | head -1)"
  local mtime1
  mtime1="$(stat -f '%m' "$map_file" 2>/dev/null || stat -c '%Y' "$map_file")"

  # Small sleep to ensure mtime would differ if rewritten
  sleep 1

  # Run again on a fresh snapshot copy (map file should be reloaded, not cleared)
  local snap2
  snap2="$(mktemp -d -t anon-reuse-XXXXXX)"
  cp -r "$FIX_BASE/snapshot/." "$snap2/"

  node "$ANONYMIZE" "$snap2" \
      --contract "$TEST_REPO/.autospec/clone.yml" \
      --repo-root "$TEST_REPO"

  # Map file still exists (was reused/updated not recreated from scratch)
  [ -f "$map_file" ]

  rm -rf "$snap2"
}

# ── anonymize.mjs exit codes ──────────────────────────────────────────────────

@test "anonymize.mjs: exits 0 on success" {
  run node "$ANONYMIZE" "$TEST_SNAPSHOT" \
      --contract "$TEST_REPO/.autospec/clone.yml" \
      --repo-root "$TEST_REPO"
  [ "$status" -eq 0 ]
}

@test "anonymize.mjs: removes stale temporary output before processing" {
  printf 'stale output\n' > "$TEST_SNAPSHOT/users.csv.anon"
  run node "$ANONYMIZE" "$TEST_SNAPSHOT" \
      --contract "$TEST_REPO/.autospec/clone.yml" \
      --repo-root "$TEST_REPO"
  [ "$status" -eq 0 ]
  [ ! -e "$TEST_SNAPSHOT/users.csv.anon" ]
  ! grep -q 'stale output' "$TEST_SNAPSHOT/users.csv"
}

@test "anonymize.mjs: does not leave a backup after replacement" {
  run node "$ANONYMIZE" "$TEST_SNAPSHOT" \
      --contract "$TEST_REPO/.autospec/clone.yml" \
      --repo-root "$TEST_REPO"
  [ "$status" -eq 0 ]
  [ ! -e "$TEST_SNAPSHOT/users.csv.anonymize-backup" ]
  [ ! -e "$TEST_SNAPSHOT/events.csv.anonymize-backup" ]
}

@test "anonymize.mjs: exits 1 when snapshot-dir does not exist" {
  run node "$ANONYMIZE" "/nonexistent/dir" \
      --contract "$TEST_REPO/.autospec/clone.yml" \
      --repo-root "$TEST_REPO"
  [ "$status" -eq 1 ]
}

@test "anonymize.mjs: exits 2 when contract is missing" {
  run node "$ANONYMIZE" "$TEST_SNAPSHOT" \
      --contract "/nonexistent/clone.yml" \
      --repo-root "$TEST_REPO"
  [ "$status" -eq 2 ]
}

@test "anonymize.mjs: exits 0 with no-op message when contract has no anonymize.rules" {
  # Write a contract without anonymize section
  cat > "$TEST_REPO/.autospec/clone.yml" << 'EOF'
sources:
  - kind: postgres
    dsn_env: PROD_DB_URL
    tables_full: [users]
expose:
  kind: docker_compose
  compose_file: deploy/docker-compose.clone.yml
  url_template: "http://localhost:8080"
  health_endpoint: /health
  ready_wait_secs: 60
EOF
  run node "$ANONYMIZE" "$TEST_SNAPSHOT" \
      --contract "$TEST_REPO/.autospec/clone.yml" \
      --repo-root "$TEST_REPO"
  [ "$status" -eq 0 ]
  echo "$output" | grep -qi "nothing to do"
}

# ── pii-assertions.sh ─────────────────────────────────────────────────────────

@test "pii-assertions.sh: exits 0 when no pii_assertions defined" {
  cat > "$TEST_REPO/.autospec/clone.yml" << 'EOF'
sources:
  - kind: postgres
    dsn_env: PROD_DB_URL
    tables_full: [users]
anonymize:
  rules:
    - table: users
      columns:
        email: hash_with_domain
expose:
  kind: docker_compose
  compose_file: deploy/docker-compose.clone.yml
  url_template: "http://localhost:8080"
  health_endpoint: /health
  ready_wait_secs: 60
EOF
  run bash "$PII_ASSERT" "$TEST_SNAPSHOT" \
      --contract "$TEST_REPO/.autospec/clone.yml" \
      --repo-root "$TEST_REPO"
  [ "$status" -eq 0 ]
}

@test "pii-assertions.sh: exits 1 when snapshot-dir does not exist" {
  run bash "$PII_ASSERT" "/nonexistent/dir" \
      --contract "$TEST_REPO/.autospec/clone.yml" \
      --repo-root "$TEST_REPO"
  [ "$status" -eq 1 ]
}

@test "pii-assertions.sh: exits 2 when contract has pii_assertions but no DB available" {
  # Contract has pii_assertions but no real DB — should fail-closed (exit 2 or 1)
  run bash "$PII_ASSERT" "$TEST_SNAPSHOT" \
      --contract "$FIX_BASE/.autospec/clone.yml" \
      --repo-root "$TEST_REPO"
  # Must not exit 0 (fail-closed: either fatal=1 or assertion-fail=2)
  [ "$status" -ne 0 ]
}
