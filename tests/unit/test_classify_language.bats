#!/usr/bin/env bats
# tests/unit/test_classify_language.bats — bats coverage for classify-language.sh
# (language-selection ranks 1-2: explicit-with-path, inherited)

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  export REPO_ROOT
  BIN="$REPO_ROOT/scripts/classify-language.sh"
  WORK="$(mktemp -d)"
  DEST=""
}

teardown() {
  rm -rf "$WORK" "${DEST:-}"
}

# ── CLI contract ───────────────────────────────────────────────────────────────

@test "classify-language: --help prints usage and exits 0" {
  run bash "$BIN" --help
  [ "$status" -eq 0 ]
  echo "$output" | grep -q "Usage:"
  echo "$output" | grep -q "classify-language.sh"
}

@test "classify-language: no args prints usage on stderr and exits 1" {
  run bash "$BIN"
  [ "$status" -eq 1 ]
  echo "$output" | grep -q "Usage:"
}

@test "classify-language: missing body file exits 1" {
  run bash "$BIN" "$WORK/does-not-exist.md"
  [ "$status" -eq 1 ]
  echo "$output" | grep -q "not found"
}

@test "classify-language: unknown option exits 1" {
  run bash "$BIN" "$WORK/body.md" --bogus
  [ "$status" -eq 1 ]
}

# ── JSON shape ─────────────────────────────────────────────────────────────────

make_rust_body() {
  cat > "$WORK/rust.md" <<'EOF'
## Goal
Tighten the workspace lints.

## Files touched
scripts/foo.rs
src/bin/bar.rs
crates/core/src/lib.rs
EOF
}

@test "classify-language: --json emits the documented five-key object" {
  make_rust_body
  run bash "$BIN" "$WORK/rust.md" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e 'keys_unsorted == ["lang","source","rationale","deterministic","confidence"]' >/dev/null
  echo "$output" | jq -e '.lang == "rust"' >/dev/null
  echo "$output" | jq -e '.source == "inherited"' >/dev/null
  echo "$output" | jq -e '.deterministic == true' >/dev/null
  echo "$output" | jq -e '.confidence == 0.95' >/dev/null
  echo "$output" | jq -e '.rationale | type == "string" and length > 0' >/dev/null
}

# ── Rank 2: inherited ──────────────────────────────────────────────────────────

@test "classify-language: Files touched all *.rs -> lang:rust source:inherited" {
  make_rust_body
  run bash "$BIN" "$WORK/rust.md" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.lang == "rust" and .source == "inherited" and .confidence == 0.95' >/dev/null
}

@test "classify-language: Python prose with only *.rs paths -> lang:rust (inheritance beats prose)" {
  cat > "$WORK/prose.md" <<'EOF'
## Goal
We should probably use Python for the new helper.

## Files touched
scripts/foo.rs
src/bin/bar.rs
EOF
  run bash "$BIN" "$WORK/prose.md" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.lang == "rust" and .source == "inherited"' >/dev/null
}

# ── Rank 1: explicit-with-path ─────────────────────────────────────────────────

@test "classify-language: naming a language beside a new-file path -> lang:python source:explicit" {
  cat > "$WORK/explicit.md" <<'EOF'
## Goal
Add `scripts/foo.py` in Python.

## Files touched
scripts/bar.rs
EOF
  run bash "$BIN" "$WORK/explicit.md" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.lang == "python" and .source == "explicit" and .confidence == 0.95' >/dev/null
}

# ── Rank 2: mixed ──────────────────────────────────────────────────────────────

@test "classify-language: Files touched spanning 2+ languages -> lang:mixed" {
  cat > "$WORK/mixed.md" <<'EOF'
## Goal
Wire the new helper through the CLI.

## Files touched
scripts/foo.rs
scripts/foo.py
EOF
  run bash "$BIN" "$WORK/mixed.md" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.lang == "mixed" and .deterministic == true' >/dev/null
}

# ── Abstention ─────────────────────────────────────────────────────────────────

@test "classify-language: empty body -> lang:unknown confidence:0.0" {
  : > "$WORK/empty.md"
  run bash "$BIN" "$WORK/empty.md" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.lang == "unknown" and .confidence == 0.0 and .deterministic == true' >/dev/null
}

@test "classify-language: Files touched with only non-language files -> lang:unknown" {
  cat > "$WORK/nolang.md" <<'EOF'
## Goal
Tidy the config.

## Files touched
Cargo.toml
docs/config.json
README
EOF
  run bash "$BIN" "$WORK/nolang.md" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.lang == "unknown" and .confidence == 0.0' >/dev/null
}

# ── Markdown block + telemetry ─────────────────────────────────────────────────

@test "classify-language: default mode emits a ## Language fit block" {
  make_rust_body
  run bash "$BIN" "$WORK/rust.md"
  [ "$status" -eq 0 ]
  echo "$output" | grep -q "## Language fit"
  echo "$output" | grep -q 'lang:rust'
  echo "$output" | grep -q "autospec-language:begin"
  echo "$output" | grep -q "autospec-language:end"
}

@test "classify-language: telemetry gains one line per invocation" {
  make_rust_body
  local telemetry_file="$REPO_ROOT/.autospec/telemetry/classify-language.jsonl"
  local before=0
  [ -f "$telemetry_file" ] && before="$(wc -l < "$telemetry_file")"
  run bash "$BIN" "$WORK/rust.md" --json
  [ "$status" -eq 0 ]
  local after=0
  [ -f "$telemetry_file" ] && after="$(wc -l < "$telemetry_file")"
  [ "$after" -gt "$before" ]
  tail -n 1 "$telemetry_file" | jq -e '.lang == "rust" and .source == "inherited"' >/dev/null
}

# ── Shipping ───────────────────────────────────────────────────────────────────

@test "install.sh: copy_repo_scripts ships classify-language.sh executable" {
  DEST="$(mktemp -d)/.autospec/scripts"
  info() { :; }
  warn() { :; }
  eval "$(sed -n '/^copy_repo_scripts() {/,/^}/p' "$REPO_ROOT/install.sh")"
  REPO_ROOT="$REPO_ROOT" DRY_RUN=0 AUTOSPEC_SCRIPTS_DIR="$DEST" copy_repo_scripts
  [ -x "$DEST/classify-language.sh" ]
}
