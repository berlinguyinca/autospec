#!/usr/bin/env bats
# tests/installer-harness-coverage.bats — guard that every per-skill installer
# accepts every supported harness (claude | opencode | codex | pi | all) and that
# the pi harness actually installs the skill (not just validates the argument).
#
# Regression guard: commit f1850cdc claimed all per-skill installers accepted
# --harness pi, but 15 did not — 11 rejected it ("invalid --harness: pi") and 4
# silently no-op'd (defined PI_DIR/PI_DEST but never installed to it), so a
# top-level `install.sh --harness all` left those skills missing from pi while
# reporting partial or even "OK" results.

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/.." && pwd)"

setup() {
  FAKE_HOME="$(mktemp -d)"
  export HOME="$FAKE_HOME"
}

teardown() {
  rm -rf "$FAKE_HOME"
}

@test "top-level install.sh lists pi among ALL_HARNESSES" {
  grep -qF 'ALL_HARNESSES="claude opencode codex pi"' "$REPO_ROOT/install.sh"
}

@test "every per-skill installer accepts --harness pi (dry-run exit 0)" {
  local dir skill
  for dir in "$REPO_ROOT"/skills/*/; do
    dir="${dir%/}"
    skill="$(basename "$dir")"
    [ -f "$dir/SKILL.md" ] || continue
    [ -f "$dir/install.sh" ] || { echo "missing installer for $skill"; return 1; }
    if ! HOME="$FAKE_HOME" sh "$dir/install.sh" --harness pi --dry-run >/dev/null 2>&1; then
      echo "installer rejected --harness pi: $skill"
      return 1
    fi
  done
}

@test "every per-skill installer plans a pi install of SKILL.md (no silent no-op)" {
  local dir skill out
  for dir in "$REPO_ROOT"/skills/*/; do
    dir="${dir%/}"
    skill="$(basename "$dir")"
    [ -f "$dir/SKILL.md" ] || continue
    [ -f "$dir/install.sh" ] || { echo "missing installer for $skill"; return 1; }
    out="$(HOME="$FAKE_HOME" sh "$dir/install.sh" --harness pi --dry-run 2>&1)" || {
      echo "installer failed for pi: $skill"; return 1
    }
    # The pi target is $HOME/.agents/skills/<skill>/SKILL.md (or $PI_SKILLS_DIR).
    if ! printf '%s\n' "$out" | grep -q "$FAKE_HOME/.agents/skills/$skill/SKILL.md"; then
      echo "installer accepted pi but planned no pi install (silent no-op): $skill"
      return 1
    fi
  done
}

@test "every per-skill installer accepts each harness: claude opencode codex pi all" {
  local dir skill harness
  for dir in "$REPO_ROOT"/skills/*/; do
    dir="${dir%/}"
    skill="$(basename "$dir")"
    [ -f "$dir/SKILL.md" ] || continue
    [ -f "$dir/install.sh" ] || { echo "missing installer for $skill"; return 1; }
    for harness in claude opencode codex pi all; do
      if ! HOME="$FAKE_HOME" sh "$dir/install.sh" --harness "$harness" --dry-run >/dev/null 2>&1; then
        echo "installer rejected --harness $harness: $skill"
        return 1
      fi
    done
  done
}
