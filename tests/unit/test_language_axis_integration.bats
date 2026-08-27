#!/usr/bin/env bats

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  DETECT="$REPO_ROOT/scripts/autospec-detect-stack-profile.sh"
  CLASSIFY="$REPO_ROOT/scripts/classify-language.sh"
  DISCOVER="$REPO_ROOT/scripts/discover-quality-commands.sh"
  BASH_BIN="$(command -v bash)"
  TAB="$(printf '\t')"

  FIXTURE="$BATS_TEST_TMPDIR/repo"
  EMPTY_DIR="$BATS_TEST_TMPDIR/empty"
  mkdir -p "$FIXTURE/src" "$FIXTURE/scripts" "$EMPTY_DIR"

  # polyglot, rust-dominant tree: one marker per detected language family
  : > "$FIXTURE/Cargo.toml"
  seq 1 30 > "$FIXTURE/src/main.rs"
  : > "$FIXTURE/package.json"
  : > "$FIXTURE/scripts/tool.sh"

  # classifier bodies
  ALLRS_BODY="$BATS_TEST_TMPDIR/allrs.md"
  printf '%s\n' \
    "# Issue" \
    "## Goal" \
    "Add a parser for the input." \
    "## Files touched" \
    "- src/parser.rs" \
    "- src/tokenizer.rs" > "$ALLRS_BODY"

  NOSIG_BODY="$BATS_TEST_TMPDIR/nosig.md"
  printf '%s\n' \
    "# Issue" \
    "## Goal" \
    "Verify the abstention path produces a clean result." > "$NOSIG_BODY"

  # restricted PATH exposing only `find` (the linters are absent -> fail-closed)
  STUB_BIN="$BATS_TEST_TMPDIR/bin"
  mkdir -p "$STUB_BIN"
  ln -sf "$(command -v find)" "$STUB_BIN/find"
}

@test "axis walk: detect -> classify -> discover on a polyglot tree" {
  run bash "$DETECT" --repo-root "$FIXTURE"
  [ "$status" -eq 0 ]
  [ -f "$FIXTURE/.autospec/state/stack-profile.json" ]

  run python3 -c \
    'import json,sys; d=json.load(open(sys.argv[1])); print(d["primary_profile"]["id"], d["primary_profile"]["confidence"])' \
    "$FIXTURE/.autospec/state/stack-profile.json"
  [ "$status" -eq 0 ]
  [ "$output" = "rust 0.95" ]

  run bash "$CLASSIFY" "$ALLRS_BODY" --json
  [ "$status" -eq 0 ]
  printf '%s' "$output" > "$BATS_TEST_TMPDIR/classify.json"

  run python3 -c \
    'import json,sys; d=json.load(open(sys.argv[1])); print(sorted(d.keys())); print(d["lang"], d["source"], d["confidence"]); print("det="+str(d["deterministic"]))' \
    "$BATS_TEST_TMPDIR/classify.json"
  [ "$status" -eq 0 ]
  [ "$(printf '%s\n' "$output" | sed -n 1p)" = "['confidence', 'deterministic', 'lang', 'rationale', 'source']" ]
  [ "$(printf '%s\n' "$output" | sed -n 2p)" = "rust inherited 0.95" ]
  [ "$(printf '%s\n' "$output" | sed -n 3p)" = "det=True" ]

  run bash "$DISCOVER" --repo-root "$FIXTURE"
  [ "$status" -eq 0 ]
  [ "$(printf '%s\n' "$output" | wc -l | tr -d ' ')" = "3" ]
  printf '%s\n' "$output" | grep -Fq "Cargo.toml${TAB}cargo clippy"
  printf '%s\n' "$output" | grep -Fq "*.sh${TAB}"
  printf '%s\n' "$output" | grep -Fq "package.json${TAB}npm run lint"
}

@test "detector: a marker under tests/fixtures is excluded from candidates" {
  FIX2="$BATS_TEST_TMPDIR/fixture2"
  mkdir -p "$FIX2/src" "$FIX2/tests/fixtures/vendor-pkg"
  : > "$FIX2/Cargo.toml"
  seq 1 20 > "$FIX2/src/main.rs"
  : > "$FIX2/tests/fixtures/vendor-pkg/package.json"

  run bash "$DETECT" --repo-root "$FIX2"
  [ "$status" -eq 0 ]

  run python3 -c \
    'import json,sys; d=json.load(open(sys.argv[1])); print(",".join(p["id"] for p in d["languages"]))' \
    "$FIX2/.autospec/state/stack-profile.json"
  [ "$status" -eq 0 ]
  [ "$output" = "rust" ]
}

@test "detector: a sub-majority line share clamps confidence to 0.5" {
  FIX3="$BATS_TEST_TMPDIR/fixture3"
  mkdir -p "$FIX3/src" "$FIX3/cmd"
  : > "$FIX3/Cargo.toml"
  : > "$FIX3/pyproject.toml"
  : > "$FIX3/go.mod"
  seq 1 40 > "$FIX3/src/main.rs"
  seq 1 35 > "$FIX3/app.py"
  seq 1 25 > "$FIX3/cmd/main.go"

  run bash "$DETECT" --repo-root "$FIX3"
  [ "$status" -eq 0 ]

  run python3 -c \
    'import json,sys; d=json.load(open(sys.argv[1])); print(d["primary_profile"]["confidence"])' \
    "$FIX3/.autospec/state/stack-profile.json"
  [ "$status" -eq 0 ]
  [ "$output" = "0.5" ]
}

@test "classifier: a no-signal body abstains to unknown/0.0 with deterministic true" {
  run env AUTOSPEC_REPO_ROOT="$EMPTY_DIR" bash "$CLASSIFY" "$NOSIG_BODY" --json
  [ "$status" -eq 0 ]
  printf '%s' "$output" > "$BATS_TEST_TMPDIR/nosig.json"

  run python3 -c \
    'import json,sys; d=json.load(open(sys.argv[1])); print(d["lang"], d["source"], d["confidence"]); print("det="+str(d["deterministic"]))' \
    "$BATS_TEST_TMPDIR/nosig.json"
  [ "$status" -eq 0 ]
  [ "$(printf '%s\n' "$output" | sed -n 1p)" = "unknown unknown 0.0" ]
  [ "$(printf '%s\n' "$output" | sed -n 2p)" = "det=True" ]
}

@test "discover: --missing-tools reports the absent linters on a polyglot tree" {
  run env PATH="$STUB_BIN" "$BASH_BIN" "$DISCOVER" --repo-root "$FIXTURE" --missing-tools
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -Fq "Cargo.toml${TAB}"
  printf '%s\n' "$output" | grep -Fq "${TAB}rust${TAB}cargo"
  printf '%s\n' "$output" | grep -Fq "${TAB}javascript${TAB}npm"
}
