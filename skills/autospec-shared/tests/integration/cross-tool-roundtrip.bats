#!/usr/bin/env bats
# cross-tool-roundtrip.bats — Integration tests for cross-tool memory round-trip.
#
# Verifies the spec §6c flow:
#   - auto-init-memory.sh creates docs/memory/ + symlink + AGENTS.md inventory
#   - CC writes via CC_PATH (symlink) → visible in docs/memory/ (Codex/OpenCode read)
#   - Codex writes via docs/memory/ → visible via CC_PATH (CC reads)
#   - AGENTS.md contains the ## Memory inventory block
#
# Run: bats skills/autospec-shared/tests/integration/cross-tool-roundtrip.bats
# (from repo root)

SCRIPT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../../scripts" && pwd)/auto-init-memory.sh"

setup() {
  TMP_DIR="$(mktemp -d /tmp/autospec-roundtrip-test-XXXXXX)"
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
  REPO_PATH="$FAKE_REPO_REAL/docs/memory"
  AGENTS_MD="$FAKE_REPO_REAL/AGENTS.md"
  export CC_PATH FAKE_REPO FAKE_REPO_REAL CC_SLUG REPO_PATH AGENTS_MD

  export AUTOSPEC_SCRIPTS_DIR="$TMP_DIR/scripts"
  mkdir -p "$AUTOSPEC_SCRIPTS_DIR"
}

teardown() {
  rm -rf "$TMP_DIR"
}

run_script() {
  run bash -c "cd \"$FAKE_REPO_REAL\" && HOME=\"$HOME\" AUTOSPEC_SCRIPTS_DIR=\"$AUTOSPEC_SCRIPTS_DIR\" bash \"$SCRIPT\" $*"
}

# ── Auto-init creates symlink + docs/memory/ + AGENTS.md ─────────────────────

@test "auto-init creates docs/memory/ and symlink from CC_PATH (state 00)" {
  run_script
  [ "$status" -eq 0 ]

  # docs/memory/ created
  [ -d "$REPO_PATH" ]
  # CC_PATH is a symlink pointing to docs/memory/
  [ -L "$CC_PATH" ]
  [ "$(readlink "$CC_PATH")" = "$REPO_PATH" ]
}

@test "auto-init adds ## Memory inventory block to AGENTS.md" {
  run_script
  [ "$status" -eq 0 ]

  [ -f "$AGENTS_MD" ]
  grep -q "^## Memory inventory" "$AGENTS_MD"
}

@test "auto-init is idempotent (fast-path on second run)" {
  run_script
  [ "$status" -eq 0 ]

  # Record mtime of symlink target
  local before_count
  before_count=$(ls "$REPO_PATH" | wc -l)

  run_script
  [ "$status" -eq 0 ]

  local after_count
  after_count=$(ls "$REPO_PATH" | wc -l)
  [ "$before_count" -eq "$after_count" ]
}

# ── CC writes → Codex/OpenCode read (via docs/memory/) ───────────────────────

@test "CC write via CC_PATH symlink is visible in docs/memory/ (Codex read path)" {
  # Run auto-init to establish symlink (state 00 → symlink created)
  run_script
  [ "$status" -eq 0 ]
  [ -L "$CC_PATH" ]

  # Simulate CC writing a new memory file via CC_PATH (the symlink)
  echo "# Lesson: prefer inline cleanup" > "$CC_PATH/feedback-bash-cleanup.md"

  # Codex/OpenCode read from docs/memory/ directly
  [ -f "$REPO_PATH/feedback-bash-cleanup.md" ]
  grep -q "inline cleanup" "$REPO_PATH/feedback-bash-cleanup.md"
}

@test "CC writes multiple files; all visible via docs/memory/" {
  run_script
  [ "$status" -eq 0 ]

  echo "# Project fact A" > "$CC_PATH/project-fact-a.md"
  echo "# Project fact B" > "$CC_PATH/project-fact-b.md"

  [ -f "$REPO_PATH/project-fact-a.md" ]
  [ -f "$REPO_PATH/project-fact-b.md" ]
  grep -q "fact A" "$REPO_PATH/project-fact-a.md"
  grep -q "fact B" "$REPO_PATH/project-fact-b.md"
}

# ── Codex writes → CC reads (via CC_PATH symlink) ────────────────────────────

@test "Codex write via docs/memory/ is visible via CC_PATH (CC read path)" {
  run_script
  [ "$status" -eq 0 ]
  [ -L "$CC_PATH" ]

  # Simulate Codex writing a memory file directly to docs/memory/
  echo "# Anti-pattern: skip lockstep" > "$REPO_PATH/feedback-lockstep-gotcha.md"

  # CC reads via CC_PATH (the symlink)
  [ -f "$CC_PATH/feedback-lockstep-gotcha.md" ]
  grep -q "lockstep" "$CC_PATH/feedback-lockstep-gotcha.md"
}

@test "Codex overwrites existing file; CC sees updated content" {
  run_script
  [ "$status" -eq 0 ]

  # Initial write via CC (simulating CC session)
  echo "# v1" > "$CC_PATH/shared-note.md"
  [ -f "$REPO_PATH/shared-note.md" ]

  # Codex updates the same file
  echo "# v2 updated by Codex" > "$REPO_PATH/shared-note.md"

  # CC reads updated content via symlink
  grep -q "v2 updated by Codex" "$CC_PATH/shared-note.md"
}

# ── Migration path: CC has existing memory files ──────────────────────────────

@test "state-10: CC memory migrated to docs/memory/; both CC and Codex read same files" {
  # Simulate existing CC memory (state 10)
  mkdir -p "$CC_PATH"
  echo "# Existing lesson" > "$CC_PATH/existing-lesson.md"
  echo "# MEMORY index" > "$CC_PATH/MEMORY.md"

  run_script
  [ "$status" -eq 0 ]

  # CC_PATH is now a symlink
  [ -L "$CC_PATH" ]
  # docs/memory/ has the migrated files
  [ -f "$REPO_PATH/existing-lesson.md" ]
  [ -f "$REPO_PATH/MEMORY.md" ]
  # CC can read via symlink
  grep -q "Existing lesson" "$CC_PATH/existing-lesson.md"
  # Codex can read via docs/memory/
  grep -q "Existing lesson" "$REPO_PATH/existing-lesson.md"
}

# ── Collaborator push: in-repo memory exists, CC memory absent ───────────────

@test "state-01: in-repo memory from collaborator; CC gets symlink and reads files" {
  # Simulate collaborator having committed docs/memory/
  mkdir -p "$REPO_PATH"
  echo "# Collaborator note" > "$REPO_PATH/collab-note.md"

  run_script
  [ "$status" -eq 0 ]

  # CC_PATH is a symlink
  [ -L "$CC_PATH" ]
  [ "$(readlink "$CC_PATH")" = "$REPO_PATH" ]
  # CC can read collaborator's file
  [ -f "$CC_PATH/collab-note.md" ]
  grep -q "Collaborator note" "$CC_PATH/collab-note.md"
}
