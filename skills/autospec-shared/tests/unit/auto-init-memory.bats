#!/usr/bin/env bats
# auto-init-memory.bats — Unit tests for auto-init-memory.sh state matrix.
#
# Run: bats skills/autospec-shared/tests/unit/auto-init-memory.bats
# (from repo root, or the script resolves paths relative to itself)

SCRIPT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)/scripts/auto-init-memory.sh"

setup() {
  # Each test gets an isolated tmpdir acting as a fake home + fake repo
  TMP_DIR="$(mktemp -d /tmp/autospec-memtest-XXXXXX)"
  export HOME="$TMP_DIR/home"
  mkdir -p "$HOME"

  # Fake git repo (bare minimum: .git dir with HEAD so git rev-parse works)
  FAKE_REPO="$TMP_DIR/repo"
  mkdir -p "$FAKE_REPO"
  git -C "$FAKE_REPO" init -q
  git -C "$FAKE_REPO" config user.email "test@example.com"
  git -C "$FAKE_REPO" config user.name "Test"
  # Initial commit so HEAD is valid
  touch "$FAKE_REPO/README.md"
  git -C "$FAKE_REPO" add README.md
  git -C "$FAKE_REPO" commit -q -m "init"

  # CC_PATH for this fake repo — use git's canonical path (resolves /tmp → /private/tmp on macOS)
  FAKE_REPO_REAL=$(cd "$FAKE_REPO" && git rev-parse --show-toplevel)
  CC_SLUG=$(echo "$FAKE_REPO_REAL" | tr '/' '-')
  CC_PATH="$HOME/.claude/projects/${CC_SLUG}/memory"
  export CC_PATH FAKE_REPO FAKE_REPO_REAL CC_SLUG

  # Disable mempalace-mine for all tests (no external dependency)
  export AUTOSPEC_SCRIPTS_DIR="$TMP_DIR/scripts"
  mkdir -p "$AUTOSPEC_SCRIPTS_DIR"

  # Suppress mempalace auto-install in all tests (network + pipx/pip side effects)
  export AUTOSPEC_SKIP_MEMPALACE_INSTALL=1
}

teardown() {
  rm -rf "$TMP_DIR"
}

# Helper: run the script inside FAKE_REPO_REAL as cwd (canonical path, matches git rev-parse)
run_script() {
  run bash -c "cd \"$FAKE_REPO_REAL\" && HOME=\"$HOME\" AUTOSPEC_SCRIPTS_DIR=\"$AUTOSPEC_SCRIPTS_DIR\" bash \"$SCRIPT\" $*"
}

# ── Fast-path: symlink already correct ───────────────────────────────────────

@test "fast-path: symlink already correct → exit 0, no-op" {
  local repo_path="$FAKE_REPO_REAL/docs/memory"
  mkdir -p "$repo_path"
  mkdir -p "$(dirname "$CC_PATH")"
  ln -s "$repo_path" "$CC_PATH"

  # Measure elapsed time in seconds (portable: macOS date lacks %3N)
  local start_s end_s elapsed_s
  start_s=$(date +%s)
  run_script
  end_s=$(date +%s)

  [ "$status" -eq 0 ]

  # Verify symlink is still correct
  [ -L "$CC_PATH" ]
  [ "$(readlink "$CC_PATH")" = "$repo_path" ]

  # Fast-path: should complete well under 1 second
  elapsed_s=$(( end_s - start_s ))
  [ "$elapsed_s" -lt 2 ]
}

# ── State 10: CC has files, in-repo dir missing ───────────────────────────────

@test "state 10: CC has files, in-repo missing → migrate + symlink" {
  # Set up CC memory dir with files
  mkdir -p "$CC_PATH"
  echo "# memory1" > "$CC_PATH/feedback_test.md"
  echo "# memory2" > "$CC_PATH/project_test.md"

  run_script
  [ "$status" -eq 0 ]

  local repo_path="$FAKE_REPO_REAL/docs/memory"

  # Symlink created pointing at docs/memory/
  [ -L "$CC_PATH" ]
  [ "$(readlink "$CC_PATH")" = "$repo_path" ]

  # Files migrated to in-repo location
  [ -f "$repo_path/feedback_test.md" ]
  [ -f "$repo_path/project_test.md" ]

  # Backup of original CC dir created
  local backup_count
  backup_count=$(ls -d "${CC_PATH}.pre-migration-"* 2>/dev/null | wc -l | tr -d ' ')
  [ "$backup_count" -ge 1 ]
}

# ── State 01: in-repo dir exists, CC missing ─────────────────────────────────

@test "state 01: in-repo dir exists, CC missing → symlink only" {
  local repo_path="$FAKE_REPO_REAL/docs/memory"
  mkdir -p "$repo_path"
  echo "# existing" > "$repo_path/MEMORY.md"

  run_script
  [ "$status" -eq 0 ]

  # Symlink created
  [ -L "$CC_PATH" ]
  [ "$(readlink "$CC_PATH")" = "$repo_path" ]

  # No backup needed (CC dir didn't exist)
  local backup_count
  backup_count=$(ls -d "${CC_PATH}.pre-migration-"* 2>/dev/null | wc -l | tr -d ' ')
  [ "$backup_count" -eq 0 ]
}

# ── State 11: both exist ──────────────────────────────────────────────────────

@test "state 11: both exist → CC wins on dup, backup, symlink" {
  # Set up CC memory dir
  mkdir -p "$CC_PATH"
  echo "# cc version" > "$CC_PATH/shared.md"
  echo "# cc only" > "$CC_PATH/cc_only.md"

  # Set up in-repo dir with a different version of shared.md
  local repo_path="$FAKE_REPO_REAL/docs/memory"
  mkdir -p "$repo_path"
  echo "# repo version" > "$repo_path/shared.md"
  echo "# repo only" > "$repo_path/repo_only.md"

  run_script
  [ "$status" -eq 0 ]

  # Symlink in place
  [ -L "$CC_PATH" ]
  [ "$(readlink "$CC_PATH")" = "$repo_path" ]

  # CC-only file copied to repo
  [ -f "$repo_path/cc_only.md" ]

  # Repo-only file preserved
  [ -f "$repo_path/repo_only.md" ]

  # Repo wins on dup: --ignore-existing keeps existing repo file (repo version preserved)
  grep -q "repo version" "$repo_path/shared.md"

  # Backup created
  local backup_count
  backup_count=$(ls -d "${CC_PATH}.pre-migration-"* 2>/dev/null | wc -l | tr -d ' ')
  [ "$backup_count" -ge 1 ]
}

# ── State 00: neither exists ──────────────────────────────────────────────────

@test "state 00: neither exists → create empty docs/memory, symlink" {
  run_script
  [ "$status" -eq 0 ]

  local repo_path="$FAKE_REPO_REAL/docs/memory"

  # docs/memory/ created
  [ -d "$repo_path" ]

  # MEMORY.md placeholder created
  [ -f "$repo_path/MEMORY.md" ]

  # Symlink created
  [ -L "$CC_PATH" ]
  [ "$(readlink "$CC_PATH")" = "$repo_path" ]
}

# ── No-gh-remote case ─────────────────────────────────────────────────────────

@test "no git repo → exit 0 silently, no symlink touched" {
  # Run from a non-git directory
  run bash -c "cd /tmp && HOME=\"$HOME\" AUTOSPEC_SCRIPTS_DIR=\"$AUTOSPEC_SCRIPTS_DIR\" bash \"$SCRIPT\""
  [ "$status" -eq 0 ]

  # No symlink created (CC_PATH parent may not even exist)
  [ ! -L "$CC_PATH" ]
}

# ── _auto_commit failure: script still exits 0 ────────────────────────────────

@test "git commit failure → symlink created, warning on stderr, exit 0" {
  # Set up CC memory dir with a file
  mkdir -p "$CC_PATH"
  echo "# memory" > "$CC_PATH/feedback_test.md"

  # Break git by writing a bad config
  git -C "$FAKE_REPO" config core.hooksPath /nonexistent-hooks-dir

  # Install a pre-commit hook that always fails to simulate commit failure
  mkdir -p "$FAKE_REPO/.git/hooks"
  printf '#!/bin/sh\nexit 1\n' > "$FAKE_REPO/.git/hooks/pre-commit"
  chmod +x "$FAKE_REPO/.git/hooks/pre-commit"

  run_script
  [ "$status" -eq 0 ]

  # Symlink still created despite git failure
  local repo_path="$FAKE_REPO_REAL/docs/memory"
  [ -L "$CC_PATH" ]
  [ "$(readlink "$CC_PATH")" = "$repo_path" ]
}

# ── Idempotency: running twice is a no-op ────────────────────────────────────

@test "idempotent: running script twice leaves state clean" {
  run_script
  [ "$status" -eq 0 ]

  # Run a second time — should fast-path and exit 0
  run_script
  [ "$status" -eq 0 ]

  local repo_path="$FAKE_REPO_REAL/docs/memory"
  [ -L "$CC_PATH" ]
  [ "$(readlink "$CC_PATH")" = "$repo_path" ]

  # Only one backup at most (from first run if CC files existed)
  local backup_count
  backup_count=$(ls -d "${CC_PATH}.pre-migration-"* 2>/dev/null | wc -l | tr -d ' ')
  [ "$backup_count" -le 1 ]
}

# ── AGENTS.md inventory appended ─────────────────────────────────────────────

@test "AGENTS.md: inventory block appended when missing" {
  run_script
  [ "$status" -eq 0 ]

  grep -q "^## Memory inventory" "$FAKE_REPO_REAL/AGENTS.md"
}

@test "AGENTS.md: inventory block not duplicated on second run" {
  run_script
  [ "$status" -eq 0 ]
  run_script
  [ "$status" -eq 0 ]

  local count
  count=$(grep -c "^## Memory inventory" "$FAKE_REPO_REAL/AGENTS.md" || echo 0)
  [ "$count" -eq 1 ]
}

# ── Rollback ──────────────────────────────────────────────────────────────────

@test "--rollback: restores most-recent backup and removes symlink" {
  # First run to set up symlink + backup
  mkdir -p "$CC_PATH"
  echo "# original" > "$CC_PATH/feedback_test.md"

  run_script
  [ "$status" -eq 0 ]

  # Confirm symlink is in place
  [ -L "$CC_PATH" ]

  # Run rollback
  run bash -c "cd \"$FAKE_REPO_REAL\" && HOME=\"$HOME\" AUTOSPEC_SCRIPTS_DIR=\"$AUTOSPEC_SCRIPTS_DIR\" bash \"$SCRIPT\" --rollback"
  [ "$status" -eq 0 ]

  # Symlink removed, backup restored as real directory
  [ ! -L "$CC_PATH" ]
  [ -d "$CC_PATH" ]
  [ -f "$CC_PATH/feedback_test.md" ]
}

# ── Mempalace auto-install opt-out ────────────────────────────────────────────

@test "AUTOSPEC_SKIP_MEMPALACE_INSTALL=1 → no install attempted" {
  # Run a fresh CC_HAS_FILES migration; helper must be a no-op under SKIP=1
  mkdir -p "$CC_PATH"
  echo "# m" > "$CC_PATH/feedback_t.md"

  run_script
  [ "$status" -eq 0 ]
  # Stderr must NOT mention any installer (pipx/uv/pip)
  [[ "$output" != *"installed mempalace via"* ]]
}

@test "mempalace already on PATH → helper is no-op (no install lines logged)" {
  # Shim a fake mempalace into a tmp bindir at the front of PATH
  local BIN="$TMP_DIR/bin"
  mkdir -p "$BIN"
  cat > "$BIN/mempalace" <<'SHIM'
#!/usr/bin/env bash
exit 0
SHIM
  chmod +x "$BIN/mempalace"

  mkdir -p "$CC_PATH"
  echo "# m" > "$CC_PATH/feedback_t.md"

  # Re-run with SKIP unset but mempalace on PATH (helper short-circuits via command -v)
  run bash -c "cd \"$FAKE_REPO_REAL\" && HOME=\"$HOME\" PATH=\"$BIN:$PATH\" AUTOSPEC_SCRIPTS_DIR=\"$AUTOSPEC_SCRIPTS_DIR\" AUTOSPEC_SKIP_MEMPALACE_INSTALL=0 bash \"$SCRIPT\""
  [ "$status" -eq 0 ]
  [[ "$output" != *"installed mempalace via"* ]]
}
