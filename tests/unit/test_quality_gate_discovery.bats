#!/usr/bin/env bats
# tests/unit/test_quality_gate_discovery.bats — the pre-merge quality gate must
# discover one command per language actually present, and fail closed when a
# marker is present but its linter is not installed.
#
# Real files, no mocks: each test builds a fixture repo on disk and runs the
# discovery script against it. "Linter absent" is produced by running the script
# with PATH pointing at a stub bin holding only the executables the test wants
# visible, so absence is genuine rather than simulated.

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  DISCOVER="$REPO_ROOT/scripts/discover-quality-commands.sh"
  BASH_BIN="$(command -v bash)"
  AUTOSPEC_RUN_TRIO=(
    "$REPO_ROOT/skills/autospec-run/SKILL.md"
    "$REPO_ROOT/skills/autospec-run/codex/prompt.md"
    "$REPO_ROOT/skills/autospec-run/opencode/agent.md"
  )
  FIXTURE="$BATS_TEST_TMPDIR/repo"
  mkdir -p "$FIXTURE"
  # Stub PATH: only `find` (the single external the script needs) plus whatever
  # a test explicitly stubs in. Everything else is genuinely absent.
  STUB_BIN="$BATS_TEST_TMPDIR/bin"
  mkdir -p "$STUB_BIN"
  ln -sf "$(command -v find)" "$STUB_BIN/find"
  TAB="$(printf '\t')"
  unset AUTOSPEC_FINAL_QUALITY_COMMAND
}

stub_tool() {
  printf '#!/bin/sh\nexit 0\n' > "$STUB_BIN/$1"
  chmod +x "$STUB_BIN/$1"
}

@test "a polyglot repo discovers one command per present marker" {
  : > "$FIXTURE/Cargo.toml"
  : > "$FIXTURE/package.json"
  mkdir -p "$FIXTURE/scripts"
  : > "$FIXTURE/scripts/tool.sh"

  run bash "$DISCOVER" --repo-root "$FIXTURE"

  [ "$status" -eq 0 ]
  [ "${#lines[@]}" -eq 3 ]
  printf '%s\n' "$output" | grep -Fq "Cargo.toml${TAB}cargo clippy --workspace --all-targets -- -D warnings"
  printf '%s\n' "$output" | grep -Fq "package.json${TAB}npm run lint"
  printf '%s\n' "$output" | grep -Fq "*.sh${TAB}"
  printf '%s\n' "$output" | grep -Fq "shellcheck"
}

@test "a repo with no language markers discovers nothing and exits clean" {
  run bash "$DISCOVER" --repo-root "$FIXTURE"
  [ "$status" -eq 0 ]
  [ -z "$output" ]

  run bash "$DISCOVER" --repo-root "$FIXTURE" --missing-tools
  [ "$status" -eq 0 ]
  [ -z "$output" ]
}

@test "a present marker whose linter is absent is reported for a fail-closed gate" {
  : > "$FIXTURE/pyproject.toml"

  run env PATH="$STUB_BIN" "$BASH_BIN" "$DISCOVER" --repo-root "$FIXTURE" --missing-tools

  [ "$status" -eq 0 ]
  [ "${#lines[@]}" -eq 1 ]
  [ "$output" = "pyproject.toml${TAB}ruff check${TAB}python${TAB}ruff" ]
}

@test "a present marker whose linter is installed reports no missing tool" {
  : > "$FIXTURE/pyproject.toml"
  stub_tool ruff

  run env PATH="$STUB_BIN" "$BASH_BIN" "$DISCOVER" --repo-root "$FIXTURE" --missing-tools

  [ "$status" -eq 0 ]
  [ -z "$output" ]
}

@test "every tool of a multi-tool command is checked, not just the first" {
  : > "$FIXTURE/go.mod"
  stub_tool go

  run env PATH="$STUB_BIN" "$BASH_BIN" "$DISCOVER" --repo-root "$FIXTURE" --missing-tools

  [ "$status" -eq 0 ]
  [ "${#lines[@]}" -eq 1 ]
  [ "$output" = "go.mod${TAB}go vet ./... && golangci-lint run${TAB}go${TAB}golangci-lint" ]
}

@test "AUTOSPEC_FINAL_QUALITY_COMMAND overrides discovery" {
  : > "$FIXTURE/Cargo.toml"

  run env AUTOSPEC_FINAL_QUALITY_COMMAND='make quality' bash "$DISCOVER" --repo-root "$FIXTURE"
  [ "$status" -eq 0 ]
  [ "$output" = "override${TAB}make quality" ]

  run env AUTOSPEC_FINAL_QUALITY_COMMAND='make quality' bash "$DISCOVER" --repo-root "$FIXTURE" --missing-tools
  [ "$status" -eq 0 ]
  [ -z "$output" ]
}

@test "the autospec-run trio drives the final quality gate from discovery" {
  for f in "${AUTOSPEC_RUN_TRIO[@]}"; do
    grep -Fq 'discover-quality-commands.sh' "$f"
    grep -Fq '${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/discover-quality-commands.sh' "$f"
    grep -Fq -- '--missing-tools' "$f"
    grep -Fq 'rule=${_lang}-unavailable' "$f"
    grep -Fq 'FINAL_QUALITY_GATE_FAILED' "$f"
  done
}

@test "the trio still runs the Rust marker exactly once, through the clippy path" {
  for f in "${AUTOSPEC_RUN_TRIO[@]}"; do
    grep -Fq 'if [ "$_marker" = "Cargo.toml" ]; then continue; fi' "$f"
    grep -Fq 'cargo clippy --workspace --all-targets -- -D warnings' "$f"
  done
}
