#!/usr/bin/env bats
# tests/unit/test_cross_language_boundaries.bats — Phase 3.75 cross-language
# boundary block (epic #3104, child #3111).
#
# extract-shared-contracts.sh must:
#   * emit a `## Cross-language boundaries` table INSIDE the shared-contract
#     marker region when the children span >=2 distinct lang:* labels (from
#     each child's `## Language fit` block) or any child is lang:mixed;
#   * emit nothing of the kind for single-language sibling sets with no
#     declared rows;
#   * fail closed (exit 3, nothing on stdout) when a declared boundary row
#     names a schema file that is not under schemas/ or does not exist.

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  SCAN="$REPO_ROOT/scripts/extract-shared-contracts.sh"

  REPO="$BATS_TEST_TMPDIR/repo"
  BODIES="$BATS_TEST_TMPDIR/bodies"
  mkdir -p "$REPO/schemas" "$REPO/tests/fixtures/boundary" "$BODIES"
  printf '{}\n' > "$REPO/schemas/autospec-x.schema.json"
  printf '{}\n' > "$REPO/tests/fixtures/boundary/x.json"

  # child A: rust, declares one boundary row (schema + fixture exist)
  cat > "$BODIES/a.md" <<'EOF'
# Child A
## Goal
Emit the worker protocol from the CLI.
<!-- autospec-language:begin -->
## Language fit

- **Language:** `lang:rust`
- **Source:** inherited
- **Rationale:** every touched file is Rust
- **Classified:** 2026-08-12
<!-- autospec-language:end -->
## Cross-language boundaries

| Boundary | Transport | Schema (source of truth) | Owner | Golden fixture |
|---|---|---|---|---|
| cli→worker | subprocess + JSON stdout | schemas/autospec-x.schema.json | lang:rust | tests/fixtures/boundary/x.json |
EOF

  # child B: typescript, declares nothing
  cat > "$BODIES/b.md" <<'EOF'
# Child B
## Goal
Parse the worker protocol in the web client.
<!-- autospec-language:begin -->
## Language fit

- **Language:** `lang:typescript`
- **Source:** inherited
- **Rationale:** every touched file is TypeScript
- **Classified:** 2026-08-12
<!-- autospec-language:end -->
EOF
}

@test "spanning rust+typescript emits the boundary block with declared rows" {
  run bash "$SCAN" --dir "$BODIES" --repo-root "$REPO"
  [ "$status" -eq 0 ]
  [[ "$output" == *"## Cross-language boundaries"* ]]
  [[ "$output" == *"| Boundary | Transport | Schema (source of truth) | Owner | Golden fixture |"* ]]
  [[ "$output" == *"cli→worker"* ]]
  [[ "$output" == *"schemas/autospec-x.schema.json"* ]]
  [[ "$output" == *"tests/fixtures/boundary/x.json"* ]]
  # the block sits strictly between the shared-contract markers
  before_block="${output%%## Cross-language boundaries*}"
  before_end="${output%%<!-- autospec-shared-contracts:end -->*}"
  [[ "$before_block" == *"<!-- autospec-shared-contracts:begin -->"* ]]
  [ "${#before_block}" -lt "${#before_end}" ]
}

@test "single-language children with no declared rows emit no boundary block" {
  local d="$BATS_TEST_TMPDIR/single"
  mkdir -p "$d"
  sed '/^## Cross-language boundaries/,$d' "$BODIES/a.md" > "$d/a.md"
  sed 's/lang:typescript/lang:rust/' "$BODIES/b.md" > "$d/b.md"
  run bash "$SCAN" --dir "$d" --repo-root "$REPO"
  [ "$status" -eq 0 ]
  [[ "$output" != *"## Cross-language boundaries"* ]]
}

@test "a declared row is a positive assertion: single language still emits the block" {
  local d="$BATS_TEST_TMPDIR/asserted"
  mkdir -p "$d"
  cp "$BODIES/a.md" "$d/a.md"   # rust child declaring a boundary row
  run bash "$SCAN" --dir "$d" --repo-root "$REPO"
  [ "$status" -eq 0 ]
  [[ "$output" == *"## Cross-language boundaries"* ]]
  [[ "$output" == *"cli→worker"* ]]
}

@test "a lone lang:mixed child triggers the boundary block" {
  local d="$BATS_TEST_TMPDIR/mixed"
  mkdir -p "$d"
  sed 's/lang:typescript/lang:mixed/' "$BODIES/b.md" > "$d/c.md"
  run bash "$SCAN" --dir "$d" --repo-root "$REPO"
  [ "$status" -eq 0 ]
  [[ "$output" == *"## Cross-language boundaries"* ]]
}

@test "a declared row naming a missing schema fails closed with exit 3" {
  local d="$BATS_TEST_TMPDIR/missing"
  local err="$BATS_TEST_TMPDIR/missing.err"
  mkdir -p "$d"
  sed 's|schemas/autospec-x.schema.json|schemas/autospec-missing.schema.json|' "$BODIES/a.md" > "$d/a.md"
  local out rc=0
  out="$(bash "$SCAN" --dir "$d" --repo-root "$REPO" 2>"$err")" || rc=$?
  [ "$rc" -eq 3 ]
  # nothing emitted on stdout — no unbacked table
  [ -z "${out// /}" ]
  [[ "$out" != *"## Cross-language boundaries"* ]]
  # the missing path is named on stderr
  grep -q 'autospec-missing' "$err"
}

@test "a declared row with a schema outside schemas/ also fails closed" {
  local d="$BATS_TEST_TMPDIR/outside"
  mkdir -p "$d"
  sed 's|schemas/autospec-x.schema.json|docs/elsewhere.json|' "$BODIES/a.md" > "$d/a.md"
  cp "$BODIES/b.md" "$d/b.md"
  run bash "$SCAN" --dir "$d" --repo-root "$REPO"
  [ "$status" -eq 3 ]
}

@test "the boundary block is deterministic across repeated runs" {
  local f1="$BATS_TEST_TMPDIR/run1.md" f2="$BATS_TEST_TMPDIR/run2.md"
  bash "$SCAN" --dir "$BODIES" --repo-root "$REPO" > "$f1"
  bash "$SCAN" --dir "$BODIES" --repo-root "$REPO" > "$f2"
  run cmp -s "$f1" "$f2"
  [ "$status" -eq 0 ]
}

@test "cross-language call tokens (generic + path-qualified) reach the shared list" {
  local d="$BATS_TEST_TMPDIR/sigs"
  mkdir -p "$d"
  cat > "$d/a.md" <<'EOF'
# Child A
## Goal
Serialize the struct over the wire.
Both sides agree on `parse<T: De>(s)` and `Foo::bar(x)`.
<!-- autospec-language:begin -->
## Language fit

- **Language:** `lang:rust`
- **Source:** inherited
<!-- autospec-language:end -->
EOF
  cat > "$d/b.md" <<'EOF'
# Child B
## Goal
Deserialize the struct in the client.
Both sides agree on `parse<T: De>(s)` and `Foo::bar(x)`.
<!-- autospec-language:begin -->
## Language fit

- **Language:** `lang:typescript`
- **Source:** inherited
<!-- autospec-language:end -->
EOF
  run bash "$SCAN" --dir "$d" --repo-root "$REPO"
  [ "$status" -eq 0 ]
  [[ "$output" == *"- \`parse<T: De>(s)\`"* ]]
  [[ "$output" == *"- \`Foo::bar(x)\`"* ]]
}

@test "boundary block is emitted on the no-shared-tokens path too" {
  # a.md and b.md share no tokens; the scanner must still emit the block
  run bash "$SCAN" --dir "$BODIES" --repo-root "$REPO"
  [ "$status" -eq 0 ]
  [[ "$output" == *"_No cross-issue contracts detected (no token appears in >=2 issues)._"* ]]
  [[ "$output" == *"## Cross-language boundaries"* ]]
}
