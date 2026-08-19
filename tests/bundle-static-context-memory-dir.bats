#!/usr/bin/env bats
# tests/bundle-static-context-memory-dir.bats — memory-dir resolution for
# skills/autospec-shared/scripts/bundle-static-context.sh.
#
# Split out of tests/bundle-static-context.bats to keep that file under the 600-line
# file-size ratchet.

SCRIPT="${BATS_TEST_DIRNAME}/../skills/autospec-shared/scripts/bundle-static-context.sh"
FIXTURES_DIR="${BATS_TEST_DIRNAME}/fixtures/bundle-static-context-memory-dir"

setup() {
  mkdir -p "$FIXTURES_DIR/memory" "$FIXTURES_DIR/skills/autospec-run/prompts"
  cat > "$FIXTURES_DIR/AGENTS.md" <<'EOF'
# Agents

## Implementation-quality contract

### RULE_ID table

| RULE_ID | Detector |
|---|---|
| `OUT_OF_SCOPE` | det |

## Next section

Unrelated.
EOF
  cat > "$FIXTURES_DIR/skills/autospec-run/prompts/implementer-contract.md" <<'EOF'
# Implementer contract (test fixture)

### RULE_ID table

| RULE_ID | Detector |
|---|---|
| `OUT_OF_SCOPE` | det |
EOF
  cat > "$FIXTURES_DIR/memory-tags.yml" <<'EOF'
feedback_bash_style.md:
  tags: [bash, scripting]
EOF
}

teardown() {
  rm -rf "$FIXTURES_DIR"
}

# ── memory-dir resolution (#3228 portability fix) ────────────────────────────
# The default path used to be hardcoded as
# $HOME/.claude/projects/-Users-<user>-IdeaProjects-autospec/memory, which could
# only match one macOS layout. Every existing case in this file passes
# AUTOSPEC_MEMORY_DIR explicitly, so none of them ever exercised the default and
# the breakage was invisible: a Linux dispatch injected nothing and printed the
# ordinary "no memory files matched" line.

# This case needs no knowledge of the harness directory-naming convention, which
# is the point — it is the one that cannot be fooled by reproducing the same
# wrong slug in both the fixture and the script.
@test "bundle-static-context.sh resolves memory from docs/memory when no harness store exists" {
  root="$BATS_TEST_TMPDIR/repo"
  mkdir -p "$root/skills/autospec-run/prompts" "$root/docs/memory"
  cp "$FIXTURES_DIR/AGENTS.md" "$root/AGENTS.md"
  cp "$FIXTURES_DIR/skills/autospec-run/prompts/implementer-contract.md" \
    "$root/skills/autospec-run/prompts/implementer-contract.md"
  cat > "$root/docs/memory/feedback_bash_style.md" <<'EOF'
---
tags: [bash, scripting]
---
DOCS_MEMORY_SENTINEL prefer printf over echo.
EOF
  run env -u AUTOSPEC_MEMORY_DIR HOME="$BATS_TEST_TMPDIR/emptyhome" \
    AUTOSPEC_REPO_ROOT="$root" \
    AUTOSPEC_SCRIPTS_DIR="${BATS_TEST_DIRNAME}/../scripts" \
    AUTOSPEC_MANIFEST="$FIXTURES_DIR/memory-tags.yml" \
    "$SCRIPT" --role implementer --issue-labels "bash"
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -q "DOCS_MEMORY_SENTINEL"
  ! printf '%s\n' "$output" | grep -q "No memory files matched"
}

# The harness store is the live per-user corpus, so it must win over the
# committed mirror when both are present.
@test "bundle-static-context.sh prefers the harness memory store over docs/memory" {
  root="$BATS_TEST_TMPDIR/repo2"
  fake_home="$BATS_TEST_TMPDIR/home2"
  mkdir -p "$root/skills/autospec-run/prompts" "$root/docs/memory"
  cp "$FIXTURES_DIR/AGENTS.md" "$root/AGENTS.md"
  cp "$FIXTURES_DIR/skills/autospec-run/prompts/implementer-contract.md" \
    "$root/skills/autospec-run/prompts/implementer-contract.md"
  printf -- '---\ntags: [bash]\n---\nDOCS_MEMORY_SENTINEL\n' \
    > "$root/docs/memory/feedback_bash_style.md"
  slug="$(printf '%s' "$root" | tr '/' '-')"
  mkdir -p "$fake_home/.claude/projects/$slug/memory"
  printf -- '---\ntags: [bash]\n---\nHARNESS_MEMORY_SENTINEL\n' \
    > "$fake_home/.claude/projects/$slug/memory/feedback_bash_style.md"
  run env -u AUTOSPEC_MEMORY_DIR HOME="$fake_home" \
    AUTOSPEC_REPO_ROOT="$root" \
    AUTOSPEC_SCRIPTS_DIR="${BATS_TEST_DIRNAME}/../scripts" \
    AUTOSPEC_MANIFEST="$FIXTURES_DIR/memory-tags.yml" \
    "$SCRIPT" --role implementer --issue-labels "bash"
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -q "HARNESS_MEMORY_SENTINEL"
  ! printf '%s\n' "$output" | grep -q "DOCS_MEMORY_SENTINEL"
}

# Negative control on the source, not the output: a hardcoded /Users path cannot
# be reintroduced without failing here. Comment lines are excluded so the
# explanation of the old bug may keep naming it.
@test "bundle-static-context.sh hardcodes no macOS-only memory path" {
  hits=$(grep -v '^[[:space:]]*#' "$SCRIPT" | grep -c -- '-Users-' || true)
  [ "$hits" -eq 0 ]
}
