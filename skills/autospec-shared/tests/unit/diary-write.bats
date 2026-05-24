#!/usr/bin/env bats
# diary-write.bats — Unit tests for diary-write.sh
#
# Tests: append-on-day, frontmatter fields, idempotency, opt-out, mempalace fallback.
#
# Run: bats skills/autospec-shared/tests/unit/diary-write.bats

SCRIPT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)/scripts/diary-write.sh"

setup() {
  TMP_DIR="$(mktemp -d /tmp/autospec-diarytest-XXXXXX)"
  export HOME="$TMP_DIR/home"
  mkdir -p "$HOME"

  MEMORY_DIR="$TMP_DIR/memory"
  mkdir -p "$MEMORY_DIR/diary"
  export MEMORY_DIR

  # Disable mempalace for all tests unless overridden
  export AUTOSPEC_SKIP_DIARY_MEMPALACE=1
}

teardown() {
  rm -rf "$TMP_DIR"
}

run_diary() {
  run bash "$SCRIPT" "$@"
}

# ── Basic write ───────────────────────────────────────────────────────────────

@test "diary-write creates a dated file in MEMORY_DIR/diary/" {
  run_diary --phase 1 --event brainstorm-locked --body "Test body" --memory-dir "$MEMORY_DIR"
  [ "$status" -eq 0 ]
  local today
  today=$(date -u +%Y-%m-%d)
  [ -f "$MEMORY_DIR/diary/$today.md" ]
}

@test "diary-write includes correct frontmatter fields" {
  run_diary --phase 1 --event brainstorm-locked --body "Decision made" --memory-dir "$MEMORY_DIR"
  [ "$status" -eq 0 ]
  local today
  today=$(date -u +%Y-%m-%d)
  local f="$MEMORY_DIR/diary/$today.md"
  grep -q "^wing: episodic" "$f"
  grep -q "^drawer_class: diary" "$f"
  grep -q "^date:" "$f"
  grep -q "^phase:" "$f"
  grep -q "^event:" "$f"
}

@test "diary-write records correct phase and event values" {
  run_diary --phase 4 --event monitor-exit --body "Monitor exited" --memory-dir "$MEMORY_DIR"
  [ "$status" -eq 0 ]
  local today
  today=$(date -u +%Y-%m-%d)
  local f="$MEMORY_DIR/diary/$today.md"
  grep -q "^phase: 4" "$f"
  grep -q "^event: monitor-exit" "$f"
}

@test "diary-write records body content in entry" {
  run_diary --phase 1 --event brainstorm-locked --body "Spec approved by user" --memory-dir "$MEMORY_DIR"
  [ "$status" -eq 0 ]
  local today
  today=$(date -u +%Y-%m-%d)
  grep -q "Spec approved by user" "$MEMORY_DIR/diary/$today.md"
}

# ── Append-on-day (multiple events same day) ──────────────────────────────────

@test "diary-write appends second entry to same day file" {
  run_diary --phase 1 --event brainstorm-locked --body "First entry" --memory-dir "$MEMORY_DIR"
  [ "$status" -eq 0 ]
  run_diary --phase 4 --event monitor-exit --body "Second entry" --memory-dir "$MEMORY_DIR"
  [ "$status" -eq 0 ]

  local today
  today=$(date -u +%Y-%m-%d)
  local f="$MEMORY_DIR/diary/$today.md"
  grep -q "First entry" "$f"
  grep -q "Second entry" "$f"
}

@test "diary-write same-day file has multiple entry separators" {
  run_diary --phase 1 --event brainstorm-locked --body "Entry A" --memory-dir "$MEMORY_DIR"
  run_diary --phase 4 --event monitor-exit --body "Entry B" --memory-dir "$MEMORY_DIR"

  local today
  today=$(date -u +%Y-%m-%d)
  local sep_count
  sep_count=$(grep -c "^---$" "$MEMORY_DIR/diary/$today.md" || true)
  [ "$sep_count" -ge 2 ]
}

# ── Auto-create memory dir ────────────────────────────────────────────────────

@test "diary-write creates diary dir if missing" {
  rm -rf "$MEMORY_DIR/diary"
  run_diary --phase 1 --event brainstorm-locked --body "Auto-create test" --memory-dir "$MEMORY_DIR"
  [ "$status" -eq 0 ]
  local today
  today=$(date -u +%Y-%m-%d)
  [ -f "$MEMORY_DIR/diary/$today.md" ]
}

# ── Default memory dir resolution ─────────────────────────────────────────────

@test "diary-write uses docs/memory when no --memory-dir and cwd is a git repo" {
  local fake_repo="$TMP_DIR/repo"
  mkdir -p "$fake_repo/docs/memory/diary"
  git -C "$fake_repo" init -q
  git -C "$fake_repo" config user.email "test@example.com"
  git -C "$fake_repo" config user.name "Test"
  touch "$fake_repo/README.md"
  git -C "$fake_repo" add README.md
  git -C "$fake_repo" commit -q -m "init"
  local fake_repo_real
  fake_repo_real=$(cd "$fake_repo" && git rev-parse --show-toplevel)
  # Run from fake repo root without --memory-dir
  run bash -c "cd '$fake_repo_real' && AUTOSPEC_SKIP_DIARY_MEMPALACE=1 bash '$SCRIPT' --phase 1 --event test-event --body 'no-flag test'"
  [ "$status" -eq 0 ]
  local today
  today=$(date -u +%Y-%m-%d)
  [ -f "$fake_repo_real/docs/memory/diary/$today.md" ]
}

# ── Opt-out env var ───────────────────────────────────────────────────────────

@test "AUTOSPEC_SKIP_DIARY=1 is a no-op exit 0" {
  AUTOSPEC_SKIP_DIARY=1 run bash "$SCRIPT" --phase 1 --event brainstorm-locked --body "Skipped" --memory-dir "$MEMORY_DIR"
  [ "$status" -eq 0 ]
  local today
  today=$(date -u +%Y-%m-%d)
  [ ! -f "$MEMORY_DIR/diary/$today.md" ]
}

# ── Missing required args ─────────────────────────────────────────────────────

@test "diary-write exits 0 (best-effort) on missing --phase" {
  run_diary --event brainstorm-locked --body "No phase" --memory-dir "$MEMORY_DIR"
  [ "$status" -eq 0 ]
}

@test "diary-write exits 0 (best-effort) on missing --event" {
  run_diary --phase 1 --body "No event" --memory-dir "$MEMORY_DIR"
  [ "$status" -eq 0 ]
}
