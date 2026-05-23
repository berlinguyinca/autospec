#!/usr/bin/env bats
# agents-md-inventory.bats — Unit tests for _ensure_agents_md_inventory_section in auto-init-memory.sh
#
# Tests that the AGENTS.md ## Memory inventory block is auto-appended idempotently per spec §4c.
#
# Run: bats skills/autospec-shared/tests/unit/agents-md-inventory.bats

SCRIPT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)/scripts/auto-init-memory.sh"

setup() {
  TMP_DIR="$(mktemp -d /tmp/autospec-agentstest-XXXXXX)"
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

  # Use git's canonical path (resolves /tmp → /private/tmp on macOS)
  FAKE_REPO_REAL=$(cd "$FAKE_REPO" && git rev-parse --show-toplevel)

  # Disable mempalace-mine for all tests
  export AUTOSPEC_SCRIPTS_DIR="$TMP_DIR/scripts"
  mkdir -p "$AUTOSPEC_SCRIPTS_DIR"

  export FAKE_REPO FAKE_REPO_REAL
}

teardown() {
  rm -rf "$TMP_DIR"
}

# Helper: run script from canonical repo path
run_script() {
  run bash -c "cd \"$FAKE_REPO_REAL\" && HOME=\"$HOME\" AUTOSPEC_SCRIPTS_DIR=\"$AUTOSPEC_SCRIPTS_DIR\" bash \"$SCRIPT\" $*"
}

# ── Case 1: AGENTS.md lacks the section → helper appends it ──────────────────

@test "AGENTS.md lacks inventory section → helper appends it" {
  # Pre-create AGENTS.md without the inventory section
  cat > "$FAKE_REPO_REAL/AGENTS.md" <<'AGENTS'
# AGENTS

Some existing content here.

## Other section

More content.
AGENTS

  run_script
  [ "$status" -eq 0 ]

  grep -q "^## Memory inventory" "$FAKE_REPO_REAL/AGENTS.md"
}

@test "AGENTS.md inventory block contains required content" {
  run_script
  [ "$status" -eq 0 ]

  local agents_md="$FAKE_REPO_REAL/AGENTS.md"

  # Required lines from spec §4c
  grep -q "docs/memory/" "$agents_md"
  grep -q "docs/memory/MEMORY.md" "$agents_md"
  grep -q "\*\*semantic\*\*" "$agents_md"
  grep -q "\*\*episodic\*\*" "$agents_md"
  grep -q "\*\*procedural\*\*" "$agents_md"
  grep -q "\*\*synthesis\*\*" "$agents_md"
}

# ── Case 2: AGENTS.md already has section → helper is no-op ──────────────────

@test "AGENTS.md already has inventory section → helper is no-op" {
  # Pre-create AGENTS.md WITH the inventory section
  cat > "$FAKE_REPO_REAL/AGENTS.md" <<'AGENTS'
# AGENTS

## Memory inventory

Persistent cross-session memory lives at [`docs/memory/`](docs/memory/).
Index: [`docs/memory/MEMORY.md`](docs/memory/MEMORY.md).

Memory types (mempalace wings):
- **semantic** — codebase facts, architecture, conventions
- **episodic** — session diary (`docs/memory/diary/`), in-flight project status
- **procedural** — playbooks, runbooks, recipes (also see SKILL.md files)
- **synthesis** — lessons learned (feedback patterns, anti-patterns, gotchas)

Read memories relevant to your task at session start. Write new memories by adding/editing files in `docs/memory/` and updating the index. Mempalace MCP layer (`mempalace search`, `mempalace traverse`, `mempalace kg_query`) is available if your tool supports MCP.
AGENTS

  local before_size
  before_size=$(wc -c < "$FAKE_REPO_REAL/AGENTS.md")

  run_script
  [ "$status" -eq 0 ]

  # File size should be unchanged (no content added)
  local after_size
  after_size=$(wc -c < "$FAKE_REPO_REAL/AGENTS.md")
  [ "$before_size" -eq "$after_size" ]

  # Still exactly one inventory block
  local count
  count=$(grep -c "^## Memory inventory" "$FAKE_REPO_REAL/AGENTS.md")
  [ "$count" -eq 1 ]
}

# ── Case 3: AGENTS.md missing entirely → helper creates with inventory block ──

@test "AGENTS.md missing entirely → helper creates file with inventory block" {
  # Ensure no AGENTS.md exists
  rm -f "$FAKE_REPO_REAL/AGENTS.md"

  run_script
  [ "$status" -eq 0 ]

  [ -f "$FAKE_REPO_REAL/AGENTS.md" ]
  grep -q "^## Memory inventory" "$FAKE_REPO_REAL/AGENTS.md"
}

# ── Idempotency: running twice produces exactly one inventory block ────────────

@test "idempotent: running script twice produces exactly one inventory block" {
  run_script
  [ "$status" -eq 0 ]

  run_script
  [ "$status" -eq 0 ]

  local count
  count=$(grep -c "^## Memory inventory" "$FAKE_REPO_REAL/AGENTS.md")
  [ "$count" -eq 1 ]
}

# ── No trailing whitespace introduced ─────────────────────────────────────────

@test "no trailing whitespace introduced in AGENTS.md inventory block" {
  run_script
  [ "$status" -eq 0 ]

  # Check that none of the lines appended by our helper have trailing spaces/tabs
  # We check only lines after the ## Memory inventory header
  local found_section=0
  local trailing_ws_count=0
  while IFS= read -r line; do
    if echo "$line" | grep -q "^## Memory inventory"; then
      found_section=1
    fi
    if [ "$found_section" -eq 1 ]; then
      if echo "$line" | grep -qE ' $|	$'; then
        trailing_ws_count=$(( trailing_ws_count + 1 ))
      fi
    fi
  done < "$FAKE_REPO_REAL/AGENTS.md"

  [ "$trailing_ws_count" -eq 0 ]
}
