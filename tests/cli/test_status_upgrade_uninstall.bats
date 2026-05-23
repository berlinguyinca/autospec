#!/usr/bin/env bats
# tests/cli/test_status_upgrade_uninstall.bats — bats coverage for status, upgrade, uninstall

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
CLI_BIN="$REPO_ROOT/packages/cli/bin/autospec.js"

setup() {
  TMPDIR_WORK="$(mktemp -d)"
  # Mock ~/.claude/skills/autospec-run with a minimal SKILL.md
  MOCK_SKILLS_DIR="$TMPDIR_WORK/mock-skills"
  mkdir -p "$MOCK_SKILLS_DIR/autospec-run"
  cat > "$MOCK_SKILLS_DIR/autospec-run/SKILL.md" <<'EOF'
---
name: autospec-run
version: 1.2.3
description: Test skill
---

# autospec-run
EOF
  # Mock .autospec dir (to verify uninstall preserves it)
  MOCK_AUTOSPEC_DIR="$TMPDIR_WORK/dot-autospec"
  mkdir -p "$MOCK_AUTOSPEC_DIR"
  echo "version: 1" > "$MOCK_AUTOSPEC_DIR/test.yml"
}

teardown() {
  rm -rf "$TMPDIR_WORK"
}

@test "autospec status prints skill version lines" {
  run env AUTOSPEC_SKILLS_DIR="$MOCK_SKILLS_DIR" node "$CLI_BIN" status
  [ "$status" -eq 0 ]
  [[ "$output" =~ "autospec-run" ]]
}

@test "autospec uninstall removes skill dirs and preserves .autospec" {
  run env AUTOSPEC_SKILLS_DIR="$MOCK_SKILLS_DIR" AUTOSPEC_DOT_DIR="$MOCK_AUTOSPEC_DIR" node "$CLI_BIN" uninstall --yes
  [ "$status" -eq 0 ]
  # Skill dirs should be gone
  [ ! -d "$MOCK_SKILLS_DIR/autospec-run" ]
  # .autospec/ should be preserved
  [ -f "$MOCK_AUTOSPEC_DIR/test.yml" ]
}

@test "autospec upgrade invokes install.sh and exits 0" {
  run env AUTOSPEC_REPO_ROOT="$REPO_ROOT" node "$CLI_BIN" upgrade --dry-run
  [ "$status" -eq 0 ]
}

@test "unknown subcommand exits non-zero with usage line" {
  run node "$CLI_BIN" foobar-unknown
  [ "$status" -ne 0 ]
  [[ "$output" =~ "unknown subcommand" ]]
}
