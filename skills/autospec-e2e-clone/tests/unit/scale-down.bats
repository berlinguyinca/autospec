#!/usr/bin/env bats
# skills/autospec-e2e-clone/tests/unit/scale-down.bats
#
# TDD unit tests for the C5 scale-down engine (scale-down.mjs).
# Tests FK reachability closure on an orders -> order_items -> products fixture.
#
# Run: bats skills/autospec-e2e-clone/tests/unit/scale-down.bats

SKILL_DIR="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
SCALE_DOWN="$SKILL_DIR/scripts/scale-down.mjs"
FIX_BASE="$SKILL_DIR/tests/unit/fixtures/scale-down-fixture"

# ── helpers ──────────────────────────────────────────────────────────────────

setup() {
  TEST_SNAPSHOT="$(mktemp -d -t scale-down-test-XXXXXX)"
  TEST_REPO="$(mktemp -d -t scale-down-repo-XXXXXX)"
  mkdir -p "$TEST_REPO/.autospec"
  cp "$FIX_BASE/.autospec/clone.yml" "$TEST_REPO/.autospec/clone.yml"
  cp -r "$FIX_BASE/snapshot/." "$TEST_SNAPSHOT/"
}

teardown() {
  rm -rf "$TEST_SNAPSHOT" "$TEST_REPO"
}

# ── Basic invocation ──────────────────────────────────────────────────────────

@test "scale-down.mjs: exits 0 on valid snapshot + contract" {
  run node "$SCALE_DOWN" "$TEST_SNAPSHOT" \
      --contract "$TEST_REPO/.autospec/clone.yml" \
      --repo-root "$TEST_REPO"
  [ "$status" -eq 0 ]
}

@test "scale-down.mjs: exits 1 when snapshot-dir missing" {
  run node "$SCALE_DOWN" /nonexistent/dir \
      --contract "$TEST_REPO/.autospec/clone.yml" \
      --repo-root "$TEST_REPO"
  [ "$status" -eq 1 ]
}

@test "scale-down.mjs: exits 2 when contract not found" {
  run node "$SCALE_DOWN" "$TEST_SNAPSHOT" \
      --contract /nonexistent/clone.yml \
      --repo-root "$TEST_REPO"
  [ "$status" -eq 2 ]
}

# ── manifest.json emission ────────────────────────────────────────────────────

@test "scale-down.mjs: emits manifest.json" {
  node "$SCALE_DOWN" "$TEST_SNAPSHOT" \
      --contract "$TEST_REPO/.autospec/clone.yml" \
      --repo-root "$TEST_REPO"
  [ -f "$TEST_SNAPSHOT/manifest.json" ]
}

@test "scale-down.mjs: manifest.json is valid JSON" {
  node "$SCALE_DOWN" "$TEST_SNAPSHOT" \
      --contract "$TEST_REPO/.autospec/clone.yml" \
      --repo-root "$TEST_REPO"
  run node -e "JSON.parse(require('fs').readFileSync('$TEST_SNAPSHOT/manifest.json','utf8'))"
  [ "$status" -eq 0 ]
}

@test "scale-down.mjs: manifest.json contains all tables" {
  node "$SCALE_DOWN" "$TEST_SNAPSHOT" \
      --contract "$TEST_REPO/.autospec/clone.yml" \
      --repo-root "$TEST_REPO"
  local manifest
  manifest="$(cat "$TEST_SNAPSHOT/manifest.json")"
  # Must have order_items, orders, products
  echo "$manifest" | grep -q '"order_items"'
  echo "$manifest" | grep -q '"orders"'
  echo "$manifest" | grep -q '"products"'
}

# ── FK reachability closure ───────────────────────────────────────────────────

@test "scale-down.mjs: order_items sampled to 2 rows" {
  node "$SCALE_DOWN" "$TEST_SNAPSHOT" \
      --contract "$TEST_REPO/.autospec/clone.yml" \
      --repo-root "$TEST_REPO"
  # order_items.csv header + 2 data rows = 3 lines (trailing newline excluded)
  local lines
  lines="$(grep -c '.' "$TEST_SNAPSHOT/order_items.csv" || true)"
  # header + 2 rows = 3
  [ "$lines" -eq 3 ]
}

@test "scale-down.mjs: orders reachability-closed (referenced orders included)" {
  node "$SCALE_DOWN" "$TEST_SNAPSHOT" \
      --contract "$TEST_REPO/.autospec/clone.yml" \
      --repo-root "$TEST_REPO"
  # The 2 sampled order_items (rows 1 and 2, both order_id=1) reference order 1.
  # orders.csv is tables_full so all rows are kept (not pruned).
  local lines
  lines="$(grep -c '.' "$TEST_SNAPSHOT/orders.csv" || true)"
  # header + 4 original rows = 5 (tables_full keeps all)
  [ "$lines" -eq 5 ]
}

@test "scale-down.mjs: products reachability-closed (referenced products included)" {
  node "$SCALE_DOWN" "$TEST_SNAPSHOT" \
      --contract "$TEST_REPO/.autospec/clone.yml" \
      --repo-root "$TEST_REPO"
  # products.csv is tables_full so all rows are kept.
  local lines
  lines="$(grep -c '.' "$TEST_SNAPSHOT/products.csv" || true)"
  [ "$lines" -eq 5 ]
}

@test "scale-down.mjs: manifest order_items count matches sampled rows" {
  node "$SCALE_DOWN" "$TEST_SNAPSHOT" \
      --contract "$TEST_REPO/.autospec/clone.yml" \
      --repo-root "$TEST_REPO"
  local count
  count="$(node -e "const m=JSON.parse(require('fs').readFileSync('$TEST_SNAPSHOT/manifest.json','utf8')); console.log(m.order_items)")"
  [ "$count" -eq 2 ]
}

# ── foreign_key_aware: false (naive sampling) ─────────────────────────────────

@test "scale-down.mjs: foreign_key_aware=false skips reachability, just samples" {
  # Override contract to disable FK awareness
  cat > "$TEST_REPO/.autospec/clone.yml" << 'YAML'
sources:
  - kind: postgres
    dsn_env: PROD_DB_URL
    tables_sample:
      order_items: 2
scale_down:
  foreign_key_aware: false
YAML

  run node "$SCALE_DOWN" "$TEST_SNAPSHOT" \
      --contract "$TEST_REPO/.autospec/clone.yml" \
      --repo-root "$TEST_REPO"
  [ "$status" -eq 0 ]

  # manifest must exist
  [ -f "$TEST_SNAPSHOT/manifest.json" ]

  # order_items still sampled to 2
  local count
  count="$(node -e "const m=JSON.parse(require('fs').readFileSync('$TEST_SNAPSHOT/manifest.json','utf8')); console.log(m.order_items)")"
  [ "$count" -eq 2 ]
}

# ── No tables_sample (nothing to do) ─────────────────────────────────────────

@test "scale-down.mjs: exits 0 and emits empty manifest when no tables_sample" {
  cat > "$TEST_REPO/.autospec/clone.yml" << 'YAML'
sources:
  - kind: postgres
    dsn_env: PROD_DB_URL
    tables_full:
      - orders
scale_down:
  foreign_key_aware: true
YAML

  run node "$SCALE_DOWN" "$TEST_SNAPSHOT" \
      --contract "$TEST_REPO/.autospec/clone.yml" \
      --repo-root "$TEST_REPO"
  [ "$status" -eq 0 ]
  [ -f "$TEST_SNAPSHOT/manifest.json" ]
}

# ── Schema.sql FK parsing ─────────────────────────────────────────────────────

@test "scale-down.mjs: reads FK edges from schema.sql when fk-meta.json absent" {
  # Remove fk-meta.json and provide schema.sql instead
  rm -f "$TEST_SNAPSHOT/fk-meta.json"
  cat > "$TEST_SNAPSHOT/schema.sql" << 'SQL'
CREATE TABLE products (
  id INTEGER PRIMARY KEY,
  name TEXT,
  price NUMERIC
);
CREATE TABLE orders (
  id INTEGER PRIMARY KEY,
  customer_name TEXT,
  total NUMERIC
);
CREATE TABLE order_items (
  id INTEGER PRIMARY KEY,
  order_id INTEGER,
  product_id INTEGER,
  qty INTEGER,
  FOREIGN KEY (order_id) REFERENCES orders (id),
  FOREIGN KEY (product_id) REFERENCES products (id)
);
SQL

  run node "$SCALE_DOWN" "$TEST_SNAPSHOT" \
      --contract "$TEST_REPO/.autospec/clone.yml" \
      --repo-root "$TEST_REPO"
  [ "$status" -eq 0 ]

  local count
  count="$(node -e "const m=JSON.parse(require('fs').readFileSync('$TEST_SNAPSHOT/manifest.json','utf8')); console.log(m.order_items)")"
  [ "$count" -eq 2 ]
}
