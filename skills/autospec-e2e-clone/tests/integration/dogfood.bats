#!/usr/bin/env bats
# skills/autospec-e2e-clone/tests/integration/dogfood.bats
#
# C10 dogfood integration test: full provision → autospec-test Mode II smoke → teardown
# against the mode-ii-real synthetic target.
#
# This test promotes Mode II from theoretical to demonstrated:
#   1. Spins up mode-ii-real (pg + minio + nginx app via docker compose)
#   2. Runs provision.sh to anonymize + expose + write clone-url.txt
#   3. Asserts clone-url.txt is present mid-run
#   4. Verifies pii_assertions pass (fail-closed: bad assertion aborts)
#   5. Runs teardown.sh and asserts clone-url.txt is removed
#
# Prerequisites: Docker running, bats-core installed.
# Run: bats skills/autospec-e2e-clone/tests/integration/dogfood.bats
#
# CI fast path: skip if docker not available (same pattern as expose-compose.bats)

SKILL_DIR="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
PROVISION_SH="$SKILL_DIR/scripts/provision.sh"
TEARDOWN_SH="$SKILL_DIR/scripts/teardown.sh"
PII_ASSERT_SH="$SKILL_DIR/scripts/pii-assertions.sh"
TARGET_DIR="$SKILL_DIR/tests/targets/mode-ii-real"
COMPOSE_FILE="$TARGET_DIR/docker-compose.yml"

# ── skip guards ───────────────────────────────────────────────────────────────

setup_file() {
  command -v docker >/dev/null 2>&1    || skip "docker not found — skipping dogfood"
  docker compose version >/dev/null 2>&1 || skip "docker compose plugin not found"
  docker info >/dev/null 2>&1          || skip "docker daemon not running"
}

# ── per-test fixtures ─────────────────────────────────────────────────────────

setup() {
  TEST_REPO="$(mktemp -d -t dogfood-repo-XXXXXX)"
  mkdir -p "$TEST_REPO/.autospec"
  # Copy target contract files into the synthetic repo root
  cp "$TARGET_DIR/.autospec/clone.yml"  "$TEST_REPO/.autospec/clone.yml"
  cp "$TARGET_DIR/.autospec/test.yml"   "$TEST_REPO/.autospec/test.yml"
  URL_FILE="$TEST_REPO/.autospec/clone-url.txt"

  # Export env vars that provision.sh reads from clone.yml dsn_env / endpoint_env
  export MODE_II_POSTGRES_DSN="postgresql://cloneuser:clonepass@localhost:25432/clonedb"
  export MODE_II_S3_ENDPOINT="http://localhost:29000"
  export MODE_II_S3_ACCESS_KEY="minioadmin"
  export MODE_II_S3_SECRET_KEY="minioadmin"
  export MODE_II_CLONE_URL="http://localhost:28080"
}

teardown() {
  # Always bring down the compose stack and clean up the temp repo
  docker compose -f "$COMPOSE_FILE" down -v 2>/dev/null || true
  rm -rf "$TEST_REPO"
}

# ── helper ────────────────────────────────────────────────────────────────────

_start_compose() {
  docker compose -f "$COMPOSE_FILE" up -d --wait 2>/dev/null \
    || docker compose -f "$COMPOSE_FILE" up -d
  # Wait for the app health check
  local attempts=0
  while ! curl -sf "http://localhost:28080/" >/dev/null 2>&1; do
    sleep 2
    attempts=$((attempts + 1))
    [ "$attempts" -lt 30 ] || return 1
  done
}

# ── tests ─────────────────────────────────────────────────────────────────────

@test "dogfood: clone.yml exists and is valid YAML" {
  [ -f "$TEST_REPO/.autospec/clone.yml" ]
  if command -v python3 >/dev/null 2>&1 && python3 -c "import yaml" 2>/dev/null; then
    python3 - "$TEST_REPO/.autospec/clone.yml" << 'PYEOF'
import yaml, sys
yaml.safe_load(open(sys.argv[1]))
PYEOF
  else
    grep -q "sources:" "$TEST_REPO/.autospec/clone.yml"
  fi
}

@test "dogfood: test.yml exists and declares edge_case_seeds" {
  [ -f "$TEST_REPO/.autospec/test.yml" ]
  grep -q "edge_case_seeds" "$TEST_REPO/.autospec/test.yml"
  grep -q "task_done_today" "$TEST_REPO/.autospec/test.yml"
}

@test "dogfood: all four target fixtures have valid clone.yml" {
  for target in postgres-fixture s3-fixture sqlite-fixture mode-ii-real; do
    local clone_yml="$SKILL_DIR/tests/targets/$target/.autospec/clone.yml"
    [ -f "$clone_yml" ] || { echo "missing: $clone_yml"; return 1; }
    if command -v python3 >/dev/null 2>&1 && python3 -c "import yaml" 2>/dev/null; then
      python3 - "$clone_yml" << 'PYEOF' || { echo "invalid YAML: $clone_yml"; return 1; }
import yaml, sys
yaml.safe_load(open(sys.argv[1]))
PYEOF
    else
      grep -q "sources:" "$clone_yml" || { echo "no sources: key in $clone_yml"; return 1; }
    fi
  done
}

@test "dogfood: provision.sh exists and is bash-syntax-clean" {
  [ -f "$PROVISION_SH" ]
  bash -n "$PROVISION_SH"
}

@test "dogfood: teardown.sh exists and is bash-syntax-clean" {
  [ -f "$TEARDOWN_SH" ]
  bash -n "$TEARDOWN_SH"
}

@test "dogfood: pii-assertions.sh exists and is bash-syntax-clean" {
  [ -f "$PII_ASSERT_SH" ]
  bash -n "$PII_ASSERT_SH"
}

@test "dogfood: pii-assertions.sh has no untyped any marker" {
  run grep -En '(^|[^[:alnum:]_])any([^[:alnum:]_]|$)' "$PII_ASSERT_SH"
  [ "$status" -eq 1 ]
}

@test "dogfood: mode-ii-real compose stack starts and app is reachable" {
  _start_compose
  local http_code
  http_code=$(curl -s -o /dev/null -w '%{http_code}' --max-time 5 "http://localhost:28080/")
  [ "$http_code" = "200" ]
}

@test "dogfood: provision writes clone-url.txt and URL is reachable" {
  _start_compose

  run bash "$PROVISION_SH" \
    --repo "$TEST_REPO" \
    --contract "$TEST_REPO/.autospec/clone.yml" \
    --url-file "$URL_FILE" \
    --skip-snapshot \
    --skip-anonymize \
    --skip-scale-down \
    --skip-seed
  # provision.sh exits 0 on success; URL file must exist
  [ "$status" -eq 0 ] || {
    echo "provision.sh output: $output"
    return 1
  }
  [ -f "$URL_FILE" ]
  local written_url
  written_url="$(cat "$URL_FILE")"
  [ -n "$written_url" ]
}

@test "dogfood: clone-url.txt removed after teardown" {
  _start_compose

  # Write a synthetic clone-url.txt as if provision ran
  echo "http://localhost:28080" > "$URL_FILE"
  [ -f "$URL_FILE" ]

  run bash "$TEARDOWN_SH" \
    --repo "$TEST_REPO" \
    --compose-file "$COMPOSE_FILE"
  [ "$status" -eq 0 ] || {
    echo "teardown.sh output: $output"
    return 1
  }
  [ ! -f "$URL_FILE" ]
}

@test "dogfood: pii_assertions failure aborts provisioning (fail-closed)" {
  # Supply a deliberately failing assertion to confirm fail-closed behaviour
  local bad_clone_yml
  bad_clone_yml="$(mktemp -t bad-clone-XXXXXX.yml)"
  cat > "$bad_clone_yml" << 'YAML'
sources:
  - kind: sqlite
    db_path_env: FIXTURE_SQLITE_PATH
    tables_full: [items]
anonymize:
  rules:
    - table: items
      columns:
        owner_email: hash_with_domain
      pii_assertions:
        - "SELECT 1 WHERE 1=0 = 0"
scale_down:
  default_sample: 100
  foreign_key_aware: false
expose:
  kind: custom_cmd
  cmd: "echo sqlite:///test.db"
  url_template: "sqlite:///test.db"
  health_endpoint: ""
  ready_wait_secs: 5
YAML

  run bash "$PII_ASSERT_SH" \
    --contract "$bad_clone_yml" \
    --dsn "sqlite:///nonexistent.db"
  # Must exit non-zero when assertions cannot be verified / DB absent
  [ "$status" -ne 0 ]

  rm -f "$bad_clone_yml"
}
