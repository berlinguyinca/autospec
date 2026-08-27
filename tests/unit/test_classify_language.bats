#!/usr/bin/env bats
# tests/unit/test_classify_language.bats — bats coverage for classify-language.sh
# (language-selection ranks 1-5: explicit-with-path, inherited, explicit-prose,
# repo-dominant, chosen, with Tier-B tie-break)

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
  make_no_marker_root
  : > "$WORK/empty.md"
  run env AUTOSPEC_REPO_ROOT="$WORK/empty-repo" bash "$BIN" "$WORK/empty.md" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.lang == "unknown" and .confidence == 0.0 and .deterministic == true' >/dev/null
}

@test "classify-language: Files touched with only non-language files -> lang:unknown" {
  make_no_marker_root
  cat > "$WORK/nolang.md" <<'EOF'
## Goal
Tidy the config.

## Files touched
Cargo.toml
docs/config.json
README
EOF
  run env AUTOSPEC_REPO_ROOT="$WORK/empty-repo" bash "$BIN" "$WORK/nolang.md" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.lang == "unknown" and .confidence == 0.0' >/dev/null
}

# ── Fixtures for ranks 3-5 ─────────────────────────────────────────────────────

write_lines() {
  local file="$1" n="$2" i
  mkdir -p "$(dirname "$file")"
  : > "$file"
  for ((i = 0; i < n; i++)); do
    printf 'x\n' >> "$file"
  done
}

make_no_marker_root() {
  mkdir -p "$WORK/empty-repo"
}

make_rust_dominant() {
  mkdir -p "$WORK/repo-rust/src"
  printf '[package]\nname = "t"\n' > "$WORK/repo-rust/Cargo.toml"
  write_lines "$WORK/repo-rust/src/main.rs" 10
}

make_go_dominant() {
  mkdir -p "$WORK/repo-go"
  printf 'module t\n' > "$WORK/repo-go/go.mod"
  write_lines "$WORK/repo-go/main.go" 10
}

make_mixed_clamped() {
  mkdir -p "$WORK/repo-mixed"
  printf '[package]\nname = "t"\n' > "$WORK/repo-mixed/Cargo.toml"
  printf 'module t\n' > "$WORK/repo-mixed/go.mod"
  printf '[project]\nname = "t"\n' > "$WORK/repo-mixed/pyproject.toml"
  write_lines "$WORK/repo-mixed/a.rs" 3
  write_lines "$WORK/repo-mixed/b.go" 3
  write_lines "$WORK/repo-mixed/c.py" 3
}

make_rust_affinity_repo() {
  mkdir -p "$WORK/repo-rust-aff"
  printf '[package]\nname = "t"\n' > "$WORK/repo-rust-aff/Cargo.toml"
  printf '[project]\nname = "t"\n' > "$WORK/repo-rust-aff/pyproject.toml"
  printf 'plugins { id "java" }\n' > "$WORK/repo-rust-aff/build.gradle"
  write_lines "$WORK/repo-rust-aff/a.rs" 3
  write_lines "$WORK/repo-rust-aff/c.py" 3
  write_lines "$WORK/repo-rust-aff/Main.java" 4
}

make_operator_default_repo() {
  mkdir -p "$WORK/repo-op-def"
  printf '[project]\nname = "t"\n' > "$WORK/repo-op-def/pyproject.toml"
  printf 'plugins { id "java" }\n' > "$WORK/repo-op-def/build.gradle"
  printf "source 'https://rubygems.org'\n" > "$WORK/repo-op-def/Gemfile"
  write_lines "$WORK/repo-op-def/c.py" 3
  write_lines "$WORK/repo-op-def/Main.java" 3
  write_lines "$WORK/repo-op-def/lib.rb" 4
}

make_50_50_repo() {
  mkdir -p "$WORK/repo-5050"
  printf '[package]\nname = "t"\n' > "$WORK/repo-5050/Cargo.toml"
  printf 'module t\n' > "$WORK/repo-5050/go.mod"
  write_lines "$WORK/repo-5050/a.rs" 5
  write_lines "$WORK/repo-5050/b.go" 5
}

make_config() {
  local root="$1" lang="$2"
  mkdir -p "$root/.autospec"
  printf 'language: %s\n' "$lang" > "$root/.autospec/autospec.yml"
}

make_fake_omc() {
  mkdir -p "$WORK/fakebin"
  cat > "$WORK/fakebin/omc" <<'FAKE'
#!/usr/bin/env bash
cat > /dev/null
if [ -n "${FAKE_OMC_CALLS:-}" ]; then
  printf 'call\n' >> "$FAKE_OMC_CALLS"
fi
printf '%s\n' "${FAKE_OMC_RETURN:-}"
FAKE
  chmod +x "$WORK/fakebin/omc"
}

write_tie_body() {
  cat > "$WORK/tie.md" <<'EOF'
## Goal
Ship a single binary, distributed to users, with no runtime dependency.
EOF
}

# ── repo_dominant (autospec-language-table.sh) ─────────────────────────────────

@test "repo_dominant: no markers -> '- 0.0' exit 0" {
  local T="$REPO_ROOT/scripts/autospec-language-table.sh"
  make_no_marker_root
  run bash "$T" repo_dominant "$WORK/empty-repo"
  [ "$status" -eq 0 ]
  [ "$output" = "- 0.0" ]
}

@test "repo_dominant: rust dominant -> 'rust 0.95 rust'" {
  local T="$REPO_ROOT/scripts/autospec-language-table.sh"
  make_rust_dominant
  run bash "$T" repo_dominant "$WORK/repo-rust"
  [ "$status" -eq 0 ]
  [ "$output" = "rust 0.95 rust" ]
}

@test "repo_dominant: go dominant -> 'go 0.95 go'" {
  local T="$REPO_ROOT/scripts/autospec-language-table.sh"
  make_go_dominant
  run bash "$T" repo_dominant "$WORK/repo-go"
  [ "$status" -eq 0 ]
  [ "$output" = "go 0.95 go" ]
}

@test "repo_dominant: 3-way split clamps -> '- 0.5 go,python,rust'" {
  local T="$REPO_ROOT/scripts/autospec-language-table.sh"
  make_mixed_clamped
  run bash "$T" repo_dominant "$WORK/repo-mixed"
  [ "$status" -eq 0 ]
  [ "$output" = "- 0.5 go,python,rust" ]
}

@test "repo_dominant: 50/50 split stays dominant -> 'go 0.72 go,rust'" {
  local T="$REPO_ROOT/scripts/autospec-language-table.sh"
  make_50_50_repo
  run bash "$T" repo_dominant "$WORK/repo-5050"
  [ "$status" -eq 0 ]
  [ "$output" = "go 0.72 go,rust" ]
}

@test "repo_dominant: usage (no args) exits 2" {
  local T="$REPO_ROOT/scripts/autospec-language-table.sh"
  run bash "$T"
  [ "$status" -eq 2 ]
}

# ── Rank 3: explicit-prose ─────────────────────────────────────────────────────

@test "classify-language: rank 3 prose names one language -> python/explicit/0.8" {
  make_no_marker_root
  cat > "$WORK/prose3.md" <<'EOF'
## Goal
Add the new helper in Python.
EOF
  run env AUTOSPEC_REPO_ROOT="$WORK/empty-repo" bash "$BIN" "$WORK/prose3.md" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.lang == "python" and .source == "explicit" and .confidence == 0.8 and .deterministic == true' >/dev/null
}

@test "classify-language: rank 3 prose names two distinct languages -> abstain" {
  make_no_marker_root
  cat > "$WORK/prose3b.md" <<'EOF'
## Goal
Write the importer in Rust and the daemon in Bash.
EOF
  run env AUTOSPEC_REPO_ROOT="$WORK/empty-repo" bash "$BIN" "$WORK/prose3b.md" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.lang == "unknown" and .source == "unknown" and .confidence == 0.0 and .deterministic == true' >/dev/null
}

# ── Rank 4: repo-dominant ──────────────────────────────────────────────────────

@test "classify-language: rank 4 rust-dominant repo -> rust/repo-dominant/0.95" {
  make_rust_dominant
  cat > "$WORK/r4.md" <<'EOF'
## Goal
Wire the new endpoint.
EOF
  run env AUTOSPEC_REPO_ROOT="$WORK/repo-rust" bash "$BIN" "$WORK/r4.md" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.lang == "rust" and .source == "repo-dominant" and .confidence == 0.95 and .deterministic == true' >/dev/null
}

@test "classify-language: rank 4 go-dominant repo -> go/repo-dominant/0.95" {
  make_go_dominant
  cat > "$WORK/r4.md" <<'EOF'
## Goal
Wire the new endpoint.
EOF
  run env AUTOSPEC_REPO_ROOT="$WORK/repo-go" bash "$BIN" "$WORK/r4.md" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.lang == "go" and .source == "repo-dominant" and .confidence == 0.95 and .deterministic == true' >/dev/null
}

@test "classify-language: rank 4 50/50 boundary repo -> go/repo-dominant/0.72" {
  make_50_50_repo
  cat > "$WORK/r4.md" <<'EOF'
## Goal
Wire the new endpoint.
EOF
  run env AUTOSPEC_REPO_ROOT="$WORK/repo-5050" bash "$BIN" "$WORK/r4.md" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.lang == "go" and .source == "repo-dominant" and .confidence == 0.72 and .deterministic == true' >/dev/null
}

# ── Rank 5: chosen + tie-break chain ───────────────────────────────────────────

@test "classify-language: rank 5 unique row -> typescript/chosen/0.7" {
  make_mixed_clamped
  cat > "$WORK/r5.md" <<'EOF'
## Goal
Build the web UI.
EOF
  run env AUTOSPEC_REPO_ROOT="$WORK/repo-mixed" bash "$BIN" "$WORK/r5.md" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.lang == "typescript" and .source == "chosen" and .confidence == 0.7 and .deterministic == true' >/dev/null
}

@test "classify-language: tie resolved by repo affinity without an LLM call" {
  make_rust_affinity_repo
  make_fake_omc
  write_tie_body
  run env AUTOSPEC_REPO_ROOT="$WORK/repo-rust-aff" \
    PATH="$WORK/fakebin:$PATH" \
    FAKE_OMC_CALLS="$WORK/omc-calls" \
    FAKE_OMC_RETURN="go" \
    bash "$BIN" "$WORK/tie.md" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.lang == "rust" and .source == "chosen" and .confidence == 0.7 and .deterministic == true' >/dev/null
  [ ! -e "$WORK/omc-calls" ]
}

@test "classify-language: tie resolved by operator default config" {
  make_operator_default_repo
  make_config "$WORK/repo-op-def" rust
  write_tie_body
  run env AUTOSPEC_REPO_ROOT="$WORK/repo-op-def" bash "$BIN" "$WORK/tie.md" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.lang == "rust" and .source == "chosen" and .confidence == 0.7 and .deterministic == true' >/dev/null
}

@test "classify-language: genuine tie makes exactly one Tier-B call" {
  make_no_marker_root
  make_fake_omc
  write_tie_body
  run env AUTOSPEC_REPO_ROOT="$WORK/empty-repo" \
    PATH="$WORK/fakebin:$PATH" \
    FAKE_OMC_CALLS="$WORK/omc-calls" \
    FAKE_OMC_RETURN="go" \
    bash "$BIN" "$WORK/tie.md" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.lang == "go" and .source == "chosen" and .confidence == 0.6 and .deterministic == false' >/dev/null
  [ "$(wc -l < "$WORK/omc-calls")" -eq 1 ]
}

@test "classify-language: Tier-B unavailable -> exit 2 lang:unknown" {
  make_no_marker_root
  write_tie_body
  local telemetry_file="$WORK/empty-repo/.autospec/telemetry/classify-language.jsonl"
  local before=0
  [ -f "$telemetry_file" ] && before="$(wc -l < "$telemetry_file")"
  run env AUTOSPEC_REPO_ROOT="$WORK/empty-repo" PATH="/usr/bin:/bin" \
    bash "$BIN" "$WORK/tie.md" --json
  [ "$status" -eq 2 ]
  echo "$output" | jq -e '.lang == "unknown" and .source == "unknown" and .confidence == 0.0 and .deterministic == false' >/dev/null
  local after=0
  [ -f "$telemetry_file" ] && after="$(wc -l < "$telemetry_file")"
  [ "$after" -gt "$before" ]
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
