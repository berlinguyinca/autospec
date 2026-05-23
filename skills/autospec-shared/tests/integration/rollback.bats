#!/usr/bin/env bats
# rollback.bats — Integration tests for auto-init-memory.sh --rollback flag.
#
# Covers:
#   - happy path: symlink removed, backup restored
#   - idempotent no-op: second --rollback when backup already gone
#   - no-op when no backup exists at all
#
# Run: bats skills/autospec-shared/tests/integration/rollback.bats
# (from repo root)

SCRIPT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../../scripts" && pwd)/auto-init-memory.sh"

setup() {
  TMP_DIR="$(mktemp -d /tmp/autospec-rollback-test-XXXXXX)"
  export HOME="$TMP_DIR/home"
  mkdir -p "$HOME"

  # Fake git repo
  FAKE_REPO="$TMP_DIR/repo"
  mkdir -p "$FAKE_REPO"
  git -C "$FAKE_REPO" init -q
  git -C "$FAKE_REPO" config user.email "test@example.com"
  git -C "$FAKE_REPO" config user.name "Test"
  touch "$FAKE_REPO/README.md"
  git -C "$FAKE_REPO" add README.md
  git -C "$FAKE_REPO" commit -q -m "init"

  # Canonical path (resolves /tmp → /private/tmp on macOS)
  FAKE_REPO_REAL=$(cd "$FAKE_REPO" && git rev-parse --show-toplevel)
  CC_SLUG=$(echo "$FAKE_REPO_REAL" | tr '/' '-')
  CC_PATH="$HOME/.claude/projects/${CC_SLUG}/memory"
  export CC_PATH FAKE_REPO FAKE_REPO_REAL CC_SLUG

  export AUTOSPEC_SCRIPTS_DIR="$TMP_DIR/scripts"
  mkdir -p "$AUTOSPEC_SCRIPTS_DIR"
}

teardown() {
  rm -rf "$TMP_DIR"
}

run_script() {
  run bash -c "cd \"$FAKE_REPO_REAL\" && HOME=\"$HOME\" AUTOSPEC_SCRIPTS_DIR=\"$AUTOSPEC_SCRIPTS_DIR\" bash \"$SCRIPT\" $*"
}

# Helper: set up state-10 (CC has files, repo dir absent) and run migration
_setup_migrated_state() {
  local repo_path="$FAKE_REPO_REAL/docs/memory"
  mkdir -p "$CC_PATH"
  echo "# test memory" > "$CC_PATH/lesson.md"

  # Run normal migration (state 10: CC has files, repo absent)
  run_script
  [ "$status" -eq 0 ]

  # After migration: CC_PATH is a symlink, backup exists
  [ -L "$CC_PATH" ]
  local backup
  backup=$(ls -dt "${CC_PATH}.pre-migration-"* 2>/dev/null | head -1)
  [ -n "$backup" ]
}

# ── Happy path ────────────────────────────────────────────────────────────────

@test "--rollback removes symlink and restores backup" {
  _setup_migrated_state

  local backup
  backup=$(ls -dt "${CC_PATH}.pre-migration-"* 2>/dev/null | head -1)

  run_script --rollback
  [ "$status" -eq 0 ]

  # Symlink gone
  [ ! -L "$CC_PATH" ]
  # Backup restored as real directory
  [ -d "$CC_PATH" ]
  [ ! -e "$backup" ]
  # Original memory file visible
  [ -f "$CC_PATH/lesson.md" ]
}

# ── Idempotent no-op: second --rollback after backup already restored ─────────

@test "--rollback is a no-op when backup already restored (no backup dir)" {
  _setup_migrated_state

  # First rollback
  run_script --rollback
  [ "$status" -eq 0 ]
  [ ! -L "$CC_PATH" ]
  [ -d "$CC_PATH" ]

  # Second rollback: no backup remaining → no-op, exit 0
  run_script --rollback
  [ "$status" -eq 0 ]
  # CC_PATH still a plain dir (not a symlink)
  [ -d "$CC_PATH" ]
  [ ! -L "$CC_PATH" ]
}

# ── No backup exists at all ───────────────────────────────────────────────────

@test "--rollback exits 0 when no backup exists and CC_PATH is a symlink" {
  # Set up symlink manually without any backup
  local repo_path="$FAKE_REPO_REAL/docs/memory"
  mkdir -p "$repo_path" "$(dirname "$CC_PATH")"
  ln -s "$repo_path" "$CC_PATH"

  # No .pre-migration-* backup exists
  run_script --rollback
  [ "$status" -eq 0 ]
  # Symlink untouched (nothing to roll back to)
  # (script logs warning to stderr; CC_PATH state is implementation-defined here)
}

@test "--rollback exits 0 when CC_PATH does not exist and no backup" {
  # Nothing set up at all
  run_script --rollback
  [ "$status" -eq 0 ]
}
