#!/usr/bin/env bats
# tests/unit/test_language_table.bats — marker-file language table (issue #3108).
#
# Closed label set from docs/specs/2026-08-12-language-selection-axis-design.md:
#   rust go python typescript javascript java bash ruby csharp markdown mixed unknown
# Every detector fixture is a real git repository with real files — no mocks.

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  TABLE="$REPO_ROOT/scripts/autospec-language-table.sh"
  WORK="$(mktemp -d)"
  FIXTURE="$WORK/fixtures"
  trap 'rm -rf "$WORK"' EXIT
}

commit_repo() {
  git -C "$1" init -q
  git -C "$1" add -A
  git -C "$1" -c user.email=test@autospec.local -c user.name=autospec commit -qm init
}

detect_primary() {
  python3 - "$REPO_ROOT" "$1" <<'PY'
import json, sys
from pathlib import Path
sys.path.insert(0, sys.argv[1] + "/scripts")
from autospec_autonomy_stack import detect_stack
root = Path(sys.argv[2])
detect_stack(root)
d = json.loads((root / ".autospec/state/stack-profile.json").read_text())
print(d["primary_profile"]["id"], d["primary_profile"]["confidence"])
PY
}

@test "marker_language resolves each supported language's marker" {
  run bash "$TABLE" marker_language Cargo.toml
  [ "$status" -eq 0 ]; [ "$output" = "rust" ]
  run bash "$TABLE" marker_language go.mod
  [ "$status" -eq 0 ]; [ "$output" = "go" ]
  run bash "$TABLE" marker_language pyproject.toml
  [ "$status" -eq 0 ]; [ "$output" = "python" ]
  run bash "$TABLE" marker_language package.json
  [ "$status" -eq 0 ]; [ "$output" = "javascript" ]
  run bash "$TABLE" marker_language pom.xml
  [ "$status" -eq 0 ]; [ "$output" = "java" ]
  run bash "$TABLE" marker_language build.gradle
  [ "$status" -eq 0 ]; [ "$output" = "java" ]
  run bash "$TABLE" marker_language Gemfile
  [ "$status" -eq 0 ]; [ "$output" = "ruby" ]
  run bash "$TABLE" marker_language Program.csproj
  [ "$status" -eq 0 ]; [ "$output" = "csharp" ]
}

@test "marker_language pairs package.json with a sibling tsconfig.json, else javascript" {
  local dir out
  dir="$(mktemp -d)"
  touch "$dir/package.json"
  out="$(cd "$dir" && bash "$TABLE" marker_language package.json)"
  [ "$out" = "javascript" ]
  touch "$dir/tsconfig.json"
  out="$(cd "$dir" && bash "$TABLE" marker_language package.json)"
  [ "$out" = "typescript" ]
}

@test "marker_language refuses unlisted markers with no output and no guess" {
  local marker
  for marker in CMakeLists.txt composer.json mix.exs setup.py build.sbt; do
    run bash "$TABLE" marker_language "$marker"
    [ "$status" -ne 0 ]
    [ -z "$output" ]
  done
}

@test "extension_language resolves the closed extension set case-insensitively" {
  run bash "$TABLE" extension_language main.rs
  [ "$status" -eq 0 ]; [ "$output" = "rust" ]
  run bash "$TABLE" extension_language main.go
  [ "$status" -eq 0 ]; [ "$output" = "go" ]
  run bash "$TABLE" extension_language main.py
  [ "$status" -eq 0 ]; [ "$output" = "python" ]
  run bash "$TABLE" extension_language main.ts
  [ "$status" -eq 0 ]; [ "$output" = "typescript" ]
  run bash "$TABLE" extension_language main.js
  [ "$status" -eq 0 ]; [ "$output" = "javascript" ]
  run bash "$TABLE" extension_language App.tsx
  [ "$status" -eq 0 ]; [ "$output" = "typescript" ]
  run bash "$TABLE" extension_language App.jsx
  [ "$status" -eq 0 ]; [ "$output" = "javascript" ]
  run bash "$TABLE" extension_language tool.mjs
  [ "$status" -eq 0 ]; [ "$output" = "javascript" ]
  run bash "$TABLE" extension_language tool.cjs
  [ "$status" -eq 0 ]; [ "$output" = "javascript" ]
  run bash "$TABLE" extension_language Main.java
  [ "$status" -eq 0 ]; [ "$output" = "java" ]
  run bash "$TABLE" extension_language build.sh
  [ "$status" -eq 0 ]; [ "$output" = "bash" ]
  run bash "$TABLE" extension_language app.rb
  [ "$status" -eq 0 ]; [ "$output" = "ruby" ]
  run bash "$TABLE" extension_language Program.cs
  [ "$status" -eq 0 ]; [ "$output" = "csharp" ]
  run bash "$TABLE" extension_language README.md
  [ "$status" -eq 0 ]; [ "$output" = "markdown" ]
  run bash "$TABLE" extension_language MAIN.PY
  [ "$status" -eq 0 ]; [ "$output" = "python" ]
  run bash "$TABLE" extension_language App.TS
  [ "$status" -eq 0 ]; [ "$output" = "typescript" ]
}

@test "extension_language refuses unlisted extensions with no output and no guess" {
  local name
  for name in main.c Main.kt index.html App.vue App.svelte; do
    run bash "$TABLE" extension_language "$name"
    [ "$status" -ne 0 ]
    [ -z "$output" ]
  done
}

@test "one fixture repo per marker language resolves that language as primary at 0.95" {
  local lang dir src
  for lang in rust go python javascript typescript java ruby csharp; do
    dir="$FIXTURE/$lang"
    mkdir -p "$dir"
    case "$lang" in
      rust) touch "$dir/Cargo.toml"; src="src/main.rs" ;;
      go) touch "$dir/go.mod"; src="main.go" ;;
      python) touch "$dir/pyproject.toml"; src="main.py" ;;
      javascript) touch "$dir/package.json"; src="app.js" ;;
      typescript) touch "$dir/package.json" "$dir/tsconfig.json"; src="src/main.ts" ;;
      java) touch "$dir/pom.xml"; src="src/Main.java" ;;
      ruby) touch "$dir/Gemfile"; src="app.rb" ;;
      csharp) touch "$dir/Program.csproj"; src="Program.cs" ;;
    esac
    mkdir -p "$dir/$(dirname "$src")"
    printf 'line\n' > "$dir/$src"
    commit_repo "$dir"
    run detect_primary "$dir"
    [ "$status" -eq 0 ]
    [ "$output" = "$lang 0.95" ]
  done
}

@test "the higher tracked line share wins a two-language polyglot repo" {
  local dir i
  dir="$FIXTURE/polyglot"
  mkdir -p "$dir/src"
  touch "$dir/Cargo.toml" "$dir/package.json"
  for i in 1 2 3 4 5 6 7 8 9 10; do printf 'fn f%s() {}\n' "$i"; done > "$dir/src/main.rs"
  for i in 1 2 3; do printf 'console.log(%s);\n' "$i"; done > "$dir/src/app.js"
  commit_repo "$dir"
  run detect_primary "$dir"
  [ "$status" -eq 0 ]
  [ "$output" = "rust 0.85" ]
}

@test "a repo with only unlisted markers is unknown at 0.1, never a guess" {
  local dir
  dir="$FIXTURE/unlisted"
  mkdir -p "$dir"
  printf 'cmake_minimum_required(VERSION 3.0)\n' > "$dir/CMakeLists.txt"
  printf 'int main(void) { return 0; }\n' > "$dir/main.c"
  commit_repo "$dir"
  run detect_primary "$dir"
  [ "$status" -eq 0 ]
  [ "$output" = "unknown 0.1" ]
}

@test "a marker nested under tests/fixtures casts zero candidate votes" {
  local dir i
  dir="$FIXTURE/fixture-marker"
  mkdir -p "$dir/tests/fixtures/fake"
  touch "$dir/go.mod"
  printf 'package main\n' > "$dir/main.go"
  touch "$dir/tests/fixtures/fake/Cargo.toml"
  for i in 1 2 3 4 5 6 7 8 9 10; do printf 'fn g%s() {}\n' "$i"; done > "$dir/tests/fixtures/fake/lib.rs"
  commit_repo "$dir"
  run detect_primary "$dir"
  [ "$status" -eq 0 ]
  [ "$output" = "go 0.5" ]
}

@test "line share counts tracked files only, so an untracked file dilutes nothing" {
  local dir i
  dir="$FIXTURE/untracked"
  mkdir -p "$dir/src"
  touch "$dir/Cargo.toml"
  for i in 1 2 3 4 5; do printf 'fn h%s() {}\n' "$i"; done > "$dir/src/main.rs"
  commit_repo "$dir"
  for i in $(seq 1 50); do printf 'console.log(%s);\n' "$i"; done > "$dir/src/junk.js"
  run detect_primary "$dir"
  [ "$status" -eq 0 ]
  [ "$output" = "rust 0.95" ]
}

@test "install ships the table script through the copy_repo_scripts glob" {
  local f shipped=0
  shopt -s nullglob
  for f in "$REPO_ROOT"/scripts/*.sh; do
    if [ "$(basename "$f")" = "autospec-language-table.sh" ]; then
      shipped=1
    fi
  done
  [ "$shipped" -eq 1 ]
  bash -n "$REPO_ROOT/scripts/autospec-language-table.sh"
  run bash "$REPO_ROOT/scripts/autospec-language-table.sh" extension_language main.rs
  [ "$status" -eq 0 ]; [ "$output" = "rust" ]
  run bash "$REPO_ROOT/scripts/autospec-language-table.sh" marker_language Cargo.toml
  [ "$status" -eq 0 ]; [ "$output" = "rust" ]
  run bash "$REPO_ROOT/scripts/autospec-language-table.sh" extension_language nope.zzz
  [ "$status" -ne 0 ]
}
