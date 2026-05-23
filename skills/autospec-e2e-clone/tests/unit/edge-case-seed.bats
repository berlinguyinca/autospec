#!/usr/bin/env bats
# skills/autospec-e2e-clone/tests/unit/edge-case-seed.bats
#
# TDD unit tests for the C6 edge-case seed consumer (edge-case-seed.mjs).
#
# Key scenario (from issue AC):
#   Fixture has 2 rows of task_done_today, count_min=5 → assert 3 synthetic rows
#   inserted with _autospec_synthetic=true flag column.
#
# Run: bats skills/autospec-e2e-clone/tests/unit/edge-case-seed.bats

SKILL_DIR="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
SEED_SCRIPT="$SKILL_DIR/scripts/edge-case-seed.mjs"
FIX_BASE="$SKILL_DIR/tests/unit/fixtures/edge-case-seed-fixture"
FIX_SURPLUS="$SKILL_DIR/tests/unit/fixtures/edge-case-seed-fixture-surplus"
# Canonical catalog lives in the autospec-test skill (sibling)
CATALOG="$(cd "$SKILL_DIR/../autospec-test/scripts/seed-shapes" 2>/dev/null && pwd)/catalog.yml"

# ── helpers ───────────────────────────────────────────────────────────────────

setup() {
  TEST_SNAPSHOT="$(mktemp -d -t edge-case-seed-snap-XXXXXX)"
  TEST_REPO="$(mktemp -d -t edge-case-seed-repo-XXXXXX)"
  mkdir -p "$TEST_REPO/.autospec"
  cp "$FIX_BASE/.autospec/test.yml" "$TEST_REPO/.autospec/test.yml"

  # Build snapshot CSV with today's UTC date so the predicate
  # "done_at >= date('now') AND done_at < date('now', '+1 day')" matches.
  local today
  today="$(date -u +%Y-%m-%d)"
  mkdir -p "$TEST_SNAPSHOT"
  printf 'id,done_at,foldout_collapsed,list_position\n' > "$TEST_SNAPSHOT/tasks.csv"
  printf '1,%sT08:00:00Z,0,1\n' "$today"                >> "$TEST_SNAPSHOT/tasks.csv"
  printf '2,%sT10:00:00Z,0,2\n' "$today"                >> "$TEST_SNAPSHOT/tasks.csv"
}

teardown() {
  rm -rf "$TEST_SNAPSHOT" "$TEST_REPO"
}

# ── Basic invocation ──────────────────────────────────────────────────────────

@test "edge-case-seed.mjs: exits 0 on valid snapshot + contract" {
  run node "$SEED_SCRIPT" "$TEST_SNAPSHOT" \
      --contract "$TEST_REPO/.autospec/test.yml" \
      --repo-root "$TEST_REPO" \
      --catalog "$CATALOG"
  [ "$status" -eq 0 ]
}

@test "edge-case-seed.mjs: exits 1 when snapshot-dir missing" {
  run node "$SEED_SCRIPT" /nonexistent/dir \
      --contract "$TEST_REPO/.autospec/test.yml" \
      --repo-root "$TEST_REPO" \
      --catalog "$CATALOG"
  [ "$status" -eq 1 ]
}

@test "edge-case-seed.mjs: exits 2 when test.yml not found" {
  run node "$SEED_SCRIPT" "$TEST_SNAPSHOT" \
      --contract /nonexistent/test.yml \
      --repo-root "$TEST_REPO" \
      --catalog "$CATALOG"
  [ "$status" -eq 2 ]
}

# ── Shortfall scenario (2 rows present, count_min=5) ─────────────────────────

@test "edge-case-seed.mjs: inserts 3 synthetic rows when 2 present and count_min=5" {
  node "$SEED_SCRIPT" "$TEST_SNAPSHOT" \
      --contract "$TEST_REPO/.autospec/test.yml" \
      --repo-root "$TEST_REPO" \
      --catalog "$CATALOG"

  # Count total rows in CSV (excluding header)
  local total_rows
  total_rows=$(tail -n +2 "$TEST_SNAPSHOT/tasks.csv" | grep -c '.' || true)
  [ "$total_rows" -eq 5 ]
}

@test "edge-case-seed.mjs: synthetic rows carry _autospec_synthetic=true" {
  node "$SEED_SCRIPT" "$TEST_SNAPSHOT" \
      --contract "$TEST_REPO/.autospec/test.yml" \
      --repo-root "$TEST_REPO" \
      --catalog "$CATALOG"

  # Count rows with _autospec_synthetic=true
  local synthetic_count
  synthetic_count=$(grep -c 'true' "$TEST_SNAPSHOT/tasks.csv" || true)
  [ "$synthetic_count" -eq 3 ]
}

@test "edge-case-seed.mjs: _autospec_synthetic column added to header when absent" {
  node "$SEED_SCRIPT" "$TEST_SNAPSHOT" \
      --contract "$TEST_REPO/.autospec/test.yml" \
      --repo-root "$TEST_REPO" \
      --catalog "$CATALOG"

  local header
  header=$(head -1 "$TEST_SNAPSHOT/tasks.csv")
  [[ "$header" == *"_autospec_synthetic"* ]]
}

@test "edge-case-seed.mjs: original 2 rows preserved after seeding" {
  node "$SEED_SCRIPT" "$TEST_SNAPSHOT" \
      --contract "$TEST_REPO/.autospec/test.yml" \
      --repo-root "$TEST_REPO" \
      --catalog "$CATALOG"

  # Original rows should still be in the CSV
  grep -q '^1,' "$TEST_SNAPSHOT/tasks.csv"
  grep -q '^2,' "$TEST_SNAPSHOT/tasks.csv"
}

# ── Surplus scenario (3 rows present, count_min=1) ────────────────────────────

@test "edge-case-seed.mjs: surplus is a no-op (no rows inserted)" {
  local SURPLUS_SNAP
  SURPLUS_SNAP="$(mktemp -d -t edge-case-seed-surplus-XXXXXX)"
  local SURPLUS_REPO
  SURPLUS_REPO="$(mktemp -d -t edge-case-seed-surplus-repo-XXXXXX)"
  mkdir -p "$SURPLUS_REPO/.autospec"
  cp "$FIX_SURPLUS/.autospec/test.yml" "$SURPLUS_REPO/.autospec/test.yml"
  local today
  today="$(date -u +%Y-%m-%d)"
  printf 'id,done_at,foldout_collapsed,list_position\n' > "$SURPLUS_SNAP/tasks.csv"
  printf '1,%sT08:00:00Z,0,1\n' "$today" >> "$SURPLUS_SNAP/tasks.csv"
  printf '2,%sT09:00:00Z,0,2\n' "$today" >> "$SURPLUS_SNAP/tasks.csv"
  printf '3,%sT10:00:00Z,0,3\n' "$today" >> "$SURPLUS_SNAP/tasks.csv"

  run node "$SEED_SCRIPT" "$SURPLUS_SNAP" \
      --contract "$SURPLUS_REPO/.autospec/test.yml" \
      --repo-root "$SURPLUS_REPO" \
      --catalog "$CATALOG"

  [ "$status" -eq 0 ]

  # Total rows should still be 3 (no inserts)
  local total_rows
  total_rows=$(tail -n +2 "$SURPLUS_SNAP/tasks.csv" | grep -c '.' || true)
  [ "$total_rows" -eq 3 ]

  rm -rf "$SURPLUS_SNAP" "$SURPLUS_REPO"
}

@test "edge-case-seed.mjs: surplus output mentions surplus" {
  local SURPLUS_SNAP
  SURPLUS_SNAP="$(mktemp -d -t edge-case-seed-surplus2-XXXXXX)"
  local SURPLUS_REPO
  SURPLUS_REPO="$(mktemp -d -t edge-case-seed-surplus2-repo-XXXXXX)"
  mkdir -p "$SURPLUS_REPO/.autospec"
  cp "$FIX_SURPLUS/.autospec/test.yml" "$SURPLUS_REPO/.autospec/test.yml"
  local today
  today="$(date -u +%Y-%m-%d)"
  printf 'id,done_at,foldout_collapsed,list_position\n' > "$SURPLUS_SNAP/tasks.csv"
  printf '1,%sT08:00:00Z,0,1\n' "$today" >> "$SURPLUS_SNAP/tasks.csv"
  printf '2,%sT09:00:00Z,0,2\n' "$today" >> "$SURPLUS_SNAP/tasks.csv"
  printf '3,%sT10:00:00Z,0,3\n' "$today" >> "$SURPLUS_SNAP/tasks.csv"

  run node "$SEED_SCRIPT" "$SURPLUS_SNAP" \
      --contract "$SURPLUS_REPO/.autospec/test.yml" \
      --repo-root "$SURPLUS_REPO" \
      --catalog "$CATALOG"

  [[ "$output" == *"surplus"* ]]

  rm -rf "$SURPLUS_SNAP" "$SURPLUS_REPO"
}

# ── Seed report ───────────────────────────────────────────────────────────────

@test "edge-case-seed.mjs: emits seed-report.json" {
  node "$SEED_SCRIPT" "$TEST_SNAPSHOT" \
      --contract "$TEST_REPO/.autospec/test.yml" \
      --repo-root "$TEST_REPO" \
      --catalog "$CATALOG"

  [ -f "$TEST_SNAPSHOT/seed-report.json" ]
}

@test "edge-case-seed.mjs: seed-report.json is valid JSON" {
  node "$SEED_SCRIPT" "$TEST_SNAPSHOT" \
      --contract "$TEST_REPO/.autospec/test.yml" \
      --repo-root "$TEST_REPO" \
      --catalog "$CATALOG"

  node -e "JSON.parse(require('fs').readFileSync('$TEST_SNAPSHOT/seed-report.json','utf8'))"
}

@test "edge-case-seed.mjs: seed-report.json records seeded status and inserted count" {
  node "$SEED_SCRIPT" "$TEST_SNAPSHOT" \
      --contract "$TEST_REPO/.autospec/test.yml" \
      --repo-root "$TEST_REPO" \
      --catalog "$CATALOG"

  local status_val inserted_val
  status_val=$(node -e "const r=JSON.parse(require('fs').readFileSync('$TEST_SNAPSHOT/seed-report.json','utf8')); console.log(r.shapes[0].status)")
  inserted_val=$(node -e "const r=JSON.parse(require('fs').readFileSync('$TEST_SNAPSHOT/seed-report.json','utf8')); console.log(r.shapes[0].inserted)")

  [ "$status_val" = "seeded" ]
  [ "$inserted_val" = "3" ]
}

# ── Catalog overlay ───────────────────────────────────────────────────────────

@test "edge-case-seed.mjs: loads overlay.yml when present" {
  # Create a temp overlay that overrides task_done_today predicate to always match 0 rows
  local OVERLAY
  OVERLAY="$(mktemp -t overlay-XXXXXX.yml)"
  printf 'task_done_today:\n  predicate_sql: "1 = 0"\n' > "$OVERLAY"

  # With predicate always false, shortfall = count_min = 5 → 5 rows inserted
  run node "$SEED_SCRIPT" "$TEST_SNAPSHOT" \
      --contract "$TEST_REPO/.autospec/test.yml" \
      --repo-root "$TEST_REPO" \
      --catalog "$CATALOG" \
      --overlay "$OVERLAY"

  [ "$status" -eq 0 ]

  # 5 synthetic rows should be inserted (predicate matches 0, count_min=5)
  local total_rows
  total_rows=$(tail -n +2 "$TEST_SNAPSHOT/tasks.csv" | grep -c '.' || true)
  [ "$total_rows" -eq 7 ]

  rm -f "$OVERLAY"
}

@test "edge-case-seed.mjs: missing overlay is a no-op (base catalog used)" {
  run node "$SEED_SCRIPT" "$TEST_SNAPSHOT" \
      --contract "$TEST_REPO/.autospec/test.yml" \
      --repo-root "$TEST_REPO" \
      --catalog "$CATALOG" \
      --overlay /nonexistent/overlay.yml

  [ "$status" -eq 0 ]
  [[ "$output" == *"no overlay found"* ]]
}

# ── No require_shapes ─────────────────────────────────────────────────────────

@test "edge-case-seed.mjs: exits 0 with nothing-to-do when no require_shapes" {
  # Write a test.yml with no edge_case_seeds
  printf 'e2e:\n  clone_url_env: CLONE_URL\n' > "$TEST_REPO/.autospec/test.yml"

  run node "$SEED_SCRIPT" "$TEST_SNAPSHOT" \
      --contract "$TEST_REPO/.autospec/test.yml" \
      --repo-root "$TEST_REPO" \
      --catalog "$CATALOG"

  [ "$status" -eq 0 ]
  [[ "$output" == *"nothing to do"* ]]
}
