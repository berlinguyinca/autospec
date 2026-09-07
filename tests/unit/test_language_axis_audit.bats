#!/usr/bin/env bats
# tests/unit/test_language_axis_audit.bats — Phase 5.5 language-axis audit
# (epic #3104, child #3112). language-axis-audit.sh checks that every issue
# body carries a ## Language fit block naming exactly one closed-set label,
# and that every declared cross-language boundary row names an existing
# schemas/ file and an existing golden fixture. Findings are `GAP` lines;
# exit 0 when inputs are valid.

setup() {
  AUDIT="$(cd "${BATS_TEST_DIRNAME}/../.." && pwd)/scripts/language-axis-audit.sh"

  REPO="$BATS_TEST_TMPDIR/repo"
  BODIES="$BATS_TEST_TMPDIR/bodies"
  mkdir -p "$REPO/schemas" "$REPO/tests/fixtures/boundary" "$BODIES"
  printf '{}\n' > "$REPO/schemas/autospec-x.schema.json"
  printf '{}\n' > "$REPO/tests/fixtures/boundary/x.json"

  cat > "$BODIES/clean.md" <<'EOF'
# Clean child
<!-- autospec-language:begin -->
## Language fit

- **Language:** `lang:rust`
- **Source:** inherited
- **Rationale:** every touched file is Rust
<!-- autospec-language:end -->
## Cross-language boundaries

| Boundary | Transport | Schema (source of truth) | Owner | Golden fixture |
|---|---|---|---|---|
| cli→worker | subprocess + JSON stdout | schemas/autospec-x.schema.json | lang:rust | tests/fixtures/boundary/x.json |
EOF

  cat > "$BODIES/nofit.md" <<'EOF'
# No language fit
## Goal
Do the thing.
EOF

  cat > "$BODIES/twolabels.md" <<'EOF'
# Two labels
<!-- autospec-language:begin -->
## Language fit

- **Language:** `lang:rust`
- **Language:** `lang:python`
<!-- autospec-language:end -->
EOF

  cat > "$BODIES/badschema.md" <<'EOF'
# Bad schema
<!-- autospec-language:begin -->
## Language fit

- **Language:** `lang:rust`
- **Source:** inherited
<!-- autospec-language:end -->
## Cross-language boundaries

| Boundary | Transport | Schema (source of truth) | Owner | Golden fixture |
|---|---|---|---|---|
| cli→worker | subprocess + JSON stdout | schemas/autospec-missing.schema.json | lang:rust | tests/fixtures/boundary/x.json |
EOF

  cat > "$BODIES/badfixture.md" <<'EOF'
# Bad fixture
<!-- autospec-language:begin -->
## Language fit

- **Language:** `lang:rust`
- **Source:** inherited
<!-- autospec-language:end -->
## Cross-language boundaries

| Boundary | Transport | Schema (source of truth) | Owner | Golden fixture |
|---|---|---|---|---|
| cli→worker | subprocess + JSON stdout | schemas/autospec-x.schema.json | lang:rust | tests/fixtures/boundary/never-landed.json |
EOF
}

@test "a clean body with landed schema + fixture emits no GAP lines" {
  run bash "$AUDIT" "$BODIES/clean.md" --repo-root "$REPO"
  [ "$status" -eq 0 ]
  [ -z "$output" ]
}

@test "a body without a Language fit block emits one GAP" {
  run bash "$AUDIT" "$BODIES/nofit.md" --repo-root "$REPO"
  [ "$status" -eq 0 ]
  [[ "$output" == *"GAP $BODIES/nofit.md: no ## Language fit block"* ]]
  [ "$(printf '%s\n' "$output" | grep -c '^GAP ')" -eq 1 ]
}

@test "a body with two distinct labels emits one GAP" {
  run bash "$AUDIT" "$BODIES/twolabels.md" --repo-root "$REPO"
  [ "$status" -eq 0 ]
  [[ "$output" == *"2 distinct lang:* labels"* ]]
  [ "$(printf '%s\n' "$output" | grep -c '^GAP ')" -eq 1 ]
}

@test "a boundary row with a missing schema emits a GAP naming the path" {
  run bash "$AUDIT" "$BODIES/badschema.md" --repo-root "$REPO"
  [ "$status" -eq 0 ]
  [[ "$output" == *"schema missing on disk: schemas/autospec-missing.schema.json"* ]]
  [ "$(printf '%s\n' "$output" | grep -c '^GAP ')" -eq 1 ]
}

@test "a boundary row with a missing golden fixture emits a GAP naming the path" {
  run bash "$AUDIT" "$BODIES/badfixture.md" --repo-root "$REPO"
  [ "$status" -eq 0 ]
  [[ "$output" == *"golden fixture missing on disk: tests/fixtures/boundary/never-landed.json"* ]]
  [ "$(printf '%s\n' "$output" | grep -c '^GAP ')" -eq 1 ]
}

@test "--bodies-dir audits every body and findings are deterministic" {
  local d="$BATS_TEST_TMPDIR/dir" f1="$BATS_TEST_TMPDIR/o1" f2="$BATS_TEST_TMPDIR/o2"
  mkdir -p "$d"
  cp "$BODIES/nofit.md" "$BODIES/badschema.md" "$d/"
  bash "$AUDIT" --bodies-dir "$d" --repo-root "$REPO" > "$f1"
  bash "$AUDIT" --bodies-dir "$d" --repo-root "$REPO" > "$f2"
  cmp -s "$f1" "$f2"
  [ "$(grep -c '^GAP ' "$f1")" -eq 2 ]
}

@test "usage errors exit 2" {
  run bash "$AUDIT"
  [ "$status" -eq 2 ]
  run bash "$AUDIT" "$BODIES/does-not-exist.md" --repo-root "$REPO"
  [ "$status" -eq 2 ]
}

@test "install.sh: copy_repo_scripts ships language-axis-audit.sh executable" {
  local root
  root="$(cd "${BATS_TEST_DIRNAME}/../.." && pwd)"
  DEST="$(mktemp -d)/.autospec/scripts"
  info() { :; }
  warn() { :; }
  eval "$(sed -n '/^copy_repo_scripts() {/,/^}/p' "$root/install.sh")"
  REPO_ROOT="$root" DRY_RUN=0 AUTOSPEC_SCRIPTS_DIR="$DEST" copy_repo_scripts
  [ -x "$DEST/language-axis-audit.sh" ]
}
