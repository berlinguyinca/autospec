#!/usr/bin/env bats
# mempalace-mine.bats — Unit tests for mempalace-mine.sh wing+drawer_class inference.
#
# Run: bats skills/autospec-shared/tests/unit/mempalace-mine.bats

SCRIPT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)/scripts/mempalace-mine.sh"
FIXTURES="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)/tests/fixtures/memory-mine"

setup() {
  TMP_DIR="$(mktemp -d /tmp/autospec-minetest-XXXXXX)"
  # Copy fixtures into tmpdir so each test gets a fresh mutable copy
  cp "$FIXTURES"/*.md "$TMP_DIR"/
  export TMP_DIR
}

teardown() {
  rm -rf "$TMP_DIR"
}

# ── Inference: feedback → synthesis / lesson ──────────────────────────────────

@test "feedback type → wing: synthesis, drawer_class: lesson" {
  run bash "$SCRIPT" --root "$TMP_DIR" --quiet
  [ "$status" -eq 0 ]

  grep -q "^  wing: synthesis" "$TMP_DIR/feedback_example.md"
  grep -q "^  drawer_class: lesson" "$TMP_DIR/feedback_example.md"
}

# ── Inference: project → episodic / session-log ───────────────────────────────

@test "project type → wing: episodic, drawer_class: session-log" {
  run bash "$SCRIPT" --root "$TMP_DIR" --quiet
  [ "$status" -eq 0 ]

  grep -q "^  wing: episodic" "$TMP_DIR/project_example.md"
  grep -q "^  drawer_class: session-log" "$TMP_DIR/project_example.md"
}

# ── Inference: reference → semantic / reference ───────────────────────────────

@test "reference type → wing: semantic, drawer_class: reference" {
  run bash "$SCRIPT" --root "$TMP_DIR" --quiet
  [ "$status" -eq 0 ]

  grep -q "^  wing: semantic" "$TMP_DIR/reference_example.md"
  grep -q "^  drawer_class: reference" "$TMP_DIR/reference_example.md"
}

# ── Inference: user → semantic / reference ────────────────────────────────────

@test "user type → wing: semantic, drawer_class: reference" {
  run bash "$SCRIPT" --root "$TMP_DIR" --quiet
  [ "$status" -eq 0 ]

  grep -q "^  wing: semantic" "$TMP_DIR/user_example.md"
  grep -q "^  drawer_class: reference" "$TMP_DIR/user_example.md"
}

# ── Inference: unknown/missing type → synthesis / lesson (default) ────────────

@test "unknown type → default wing: synthesis, drawer_class: lesson" {
  # Create a file with no type field
  cat > "$TMP_DIR/unknown_example.md" <<'EOF'
---
name: unknown-example
description: No type field.
metadata:
  tags: [test]
---

Body.
EOF

  run bash "$SCRIPT" --root "$TMP_DIR" --quiet
  [ "$status" -eq 0 ]

  grep -q "^  wing: synthesis" "$TMP_DIR/unknown_example.md"
  grep -q "^  drawer_class: lesson" "$TMP_DIR/unknown_example.md"
}

# ── MEMORY.md index file is skipped ──────────────────────────────────────────

@test "MEMORY.md is skipped (not modified)" {
  cat > "$TMP_DIR/MEMORY.md" <<'EOF'
# Memory Index

- [feedback_example](feedback_example.md)
EOF
  local before
  before=$(cat "$TMP_DIR/MEMORY.md")

  run bash "$SCRIPT" --root "$TMP_DIR" --quiet
  [ "$status" -eq 0 ]

  local after
  after=$(cat "$TMP_DIR/MEMORY.md")
  [ "$before" = "$after" ]
}

# ── Idempotency: running twice leaves frontmatter unchanged ───────────────────

@test "idempotent: running miner twice produces same frontmatter" {
  run bash "$SCRIPT" --root "$TMP_DIR" --quiet
  [ "$status" -eq 0 ]

  local after_first
  after_first=$(cat "$TMP_DIR/feedback_example.md")

  run bash "$SCRIPT" --root "$TMP_DIR" --quiet
  [ "$status" -eq 0 ]

  local after_second
  after_second=$(cat "$TMP_DIR/feedback_example.md")

  [ "$after_first" = "$after_second" ]
}

@test "idempotent: wing: appears exactly once after two runs" {
  run bash "$SCRIPT" --root "$TMP_DIR" --quiet
  [ "$status" -eq 0 ]
  run bash "$SCRIPT" --root "$TMP_DIR" --quiet
  [ "$status" -eq 0 ]

  local count
  count=$(grep -c "^  wing:" "$TMP_DIR/feedback_example.md")
  [ "$count" -eq 1 ]
}

# ── Missing --root exits 0 silently ──────────────────────────────────────────

@test "missing --root → exit 0 with error to stderr" {
  run bash "$SCRIPT" --quiet
  [ "$status" -eq 0 ]
}

# ── Non-existent root dir exits 0 silently ────────────────────────────────────

@test "non-existent root dir → exit 0 with error to stderr" {
  run bash "$SCRIPT" --root /nonexistent-dir-xyz --quiet
  [ "$status" -eq 0 ]
}

# ── Frontmatter lines inserted in correct order ───────────────────────────────

@test "wing: and drawer_class: inserted after type: line" {
  run bash "$SCRIPT" --root "$TMP_DIR" --quiet
  [ "$status" -eq 0 ]

  # Find line numbers: type must come before wing, wing before drawer_class
  local type_line wing_line drawer_line
  type_line=$(grep -n "^  type:" "$TMP_DIR/feedback_example.md" | head -1 | cut -d: -f1)
  wing_line=$(grep -n "^  wing:" "$TMP_DIR/feedback_example.md" | head -1 | cut -d: -f1)
  drawer_line=$(grep -n "^  drawer_class:" "$TMP_DIR/feedback_example.md" | head -1 | cut -d: -f1)

  [ "$type_line" -lt "$wing_line" ]
  [ "$wing_line" -lt "$drawer_line" ]
}
