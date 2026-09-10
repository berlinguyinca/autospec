#!/usr/bin/env bats
# tests/unit/test_cross_language_boundaries.bats — Phase 3.75
# cross-language boundary table (#3210).
#
# `scripts/extract-shared-contracts.sh --languages <csv>` must emit a
# `## Cross-language boundaries` table inside the shared-contracts marker
# region when the siblings span 2+ distinct languages (or any child carries
# `mixed`). The table is deterministic — rows come from `schemas/*.schema.json`
# paths shared by >=2 child bodies — and the script fails closed (non-zero
# exit, nothing on stdout) when a row's schema file does not exist. Re-running
# the patch replaces the marker region instead of stacking a second block.
#
# Fixtures are real files driven through the real scanner; no mocks.

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  EXTRACT="$REPO_ROOT/scripts/extract-shared-contracts.sh"
  TMP="$(mktemp -d)"
}

teardown() {
  rm -rf "$TMP"
}

# A child issue body that names one shared boundary schema.
write_body() {  # write_body <file> <schema>
  cat > "$1" <<MD
## Implementation outline

- Emit \`$2\` on stdout and assert the golden fixture.
MD
}

# Replicate the documented Phase 3.75 patch. Case A (no marker yet): insert
# the freshly generated block immediately before the first `## Dependencies`
# line. Case B (marker present): replace the entire marker region — inclusive
# of both marker lines — with the freshly generated block, so re-runs never
# stack a second heading.
patch_body() {  # patch_body <body> <block> <out>
  if grep -q '<!-- autospec-shared-contracts:begin -->' "$1"; then
    awk -v f="$2" '
      !replaced && /^<!-- autospec-shared-contracts:begin -->$/ {
        while ((getline line < f) > 0) print line
        skip = 1
        replaced = 1
        close(f)
        next
      }
      skip && /^<!-- autospec-shared-contracts:end -->$/ { skip = 0; next }
      !skip { print }
    ' "$1" > "$3"
  else
    awk -v f="$2" '
      !done && /^## Dependencies$/ {
        while ((getline line < f) > 0) print line
        print ""
        close(f)
        done = 1
      }
      { print }
      END { if (!done) while ((getline line < f) > 0) print line }
    ' "$1" > "$3"
  fi
}

# A minimal issue body with `## Dependencies` as the last section.
write_issue_body() {  # write_issue_body <file>
  cat > "$1" <<'MD'
## Goal

Bridge the Rust worker and the TypeScript CLI through one JSON boundary.

## Acceptance criteria

- [ ] The boundary schema is asserted by both sides

## Dependencies

Depends on issue #999
MD
}

@test "--languages with two distinct languages emits the boundary table" {
  mkdir -p "$TMP/schemas"
  touch "$TMP/schemas/roundtrip.schema.json"
  write_body "$TMP/child-a.md" "schemas/roundtrip.schema.json"
  write_body "$TMP/child-b.md" "schemas/roundtrip.schema.json"
  pushd "$TMP" > /dev/null
  run bash "$EXTRACT" --languages rust,typescript child-a.md child-b.md
  popd > /dev/null
  [ "$status" -eq 0 ]
  echo "$output" | grep -q '## Cross-language boundaries'
  echo "$output" | grep -q 'schemas/roundtrip.schema.json'
  # The table header must carry every column of the spec shape.
  echo "$output" | grep -q '| Boundary | Transport | Schema (source of truth) | Owner | Golden fixture |'
}

@test "single-language siblings do not emit a boundary table" {
  mkdir -p "$TMP/schemas"
  touch "$TMP/schemas/roundtrip.schema.json"
  write_body "$TMP/child-a.md" "schemas/roundtrip.schema.json"
  write_body "$TMP/child-b.md" "schemas/roundtrip.schema.json"
  pushd "$TMP" > /dev/null
  run bash "$EXTRACT" --languages rust,rust child-a.md child-b.md
  popd > /dev/null
  [ "$status" -eq 0 ]
  ! echo "$output" | grep -q '## Cross-language boundaries'
}

@test "a lang:mixed child alone triggers the boundary table" {
  write_body "$TMP/child-a.md" "schemas/roundtrip.schema.json"
  pushd "$TMP" > /dev/null
  run bash "$EXTRACT" --languages mixed child-a.md
  popd > /dev/null
  [ "$status" -eq 0 ]
  echo "$output" | grep -q '## Cross-language boundaries'
}

@test "lang: prefixes are normalized before the trigger test" {
  pushd "$TMP" > /dev/null
  run bash "$EXTRACT" --languages lang:rust,lang:typescript
  popd > /dev/null
  [ "$status" -eq 0 ]
  echo "$output" | grep -q '## Cross-language boundaries'
  pushd "$TMP" > /dev/null
  run bash "$EXTRACT" --languages lang:mixed
  popd > /dev/null
  [ "$status" -eq 0 ]
  echo "$output" | grep -q '## Cross-language boundaries'
}

@test "missing shared schema fails closed: non-zero exit, stderr reason, nothing on stdout" {
  write_body "$TMP/child-a.md" "schemas/does-not-exist.schema.json"
  write_body "$TMP/child-b.md" "schemas/does-not-exist.schema.json"
  pushd "$TMP" > /dev/null
  local rc=0
  bash "$EXTRACT" --languages rust,typescript child-a.md child-b.md \
    > "$TMP/out.txt" 2> "$TMP/err.txt" || rc=$?
  popd > /dev/null
  [ "$rc" -ne 0 ]
  [ ! -s "$TMP/out.txt" ]
  grep -q 'PHASE_3_75_FAILED rule=boundary-schema-missing' "$TMP/err.txt"
  grep -q 'schemas/does-not-exist.schema.json' "$TMP/err.txt"
}

@test "--languages alone (no bodies) emits the boundary frame" {
  pushd "$TMP" > /dev/null
  run bash "$EXTRACT" --languages rust,typescript
  popd > /dev/null
  [ "$status" -eq 0 ]
  echo "$output" | grep -q '## Cross-language boundaries'
}

@test "schema path in only one body is not a boundary row" {
  mkdir -p "$TMP/schemas"
  touch "$TMP/schemas/shared.schema.json" "$TMP/schemas/private.schema.json"
  write_body "$TMP/child-a.md" "schemas/shared.schema.json"
  cat > "$TMP/child-b.md" <<'MD'
## Implementation outline

- Emit `schemas/shared.schema.json` and the private `schemas/private.schema.json`.
MD
  pushd "$TMP" > /dev/null
  run bash "$EXTRACT" --languages rust,typescript child-a.md child-b.md
  popd > /dev/null
  [ "$status" -eq 0 ]
  echo "$output" | grep -q 'schemas/shared.schema.json'
  ! echo "$output" | grep -q 'schemas/private.schema.json'
}

@test "re-running the patch replaces the block instead of stacking" {
  mkdir -p "$TMP/schemas"
  touch "$TMP/schemas/roundtrip.schema.json"
  write_body "$TMP/child-a.md" "schemas/roundtrip.schema.json"
  write_body "$TMP/child-b.md" "schemas/roundtrip.schema.json"
  write_issue_body "$TMP/body.md"
  pushd "$TMP" > /dev/null
  bash "$EXTRACT" --languages rust,typescript child-a.md child-b.md > "$TMP/block1.md"
  popd > /dev/null
  patch_body "$TMP/body.md" "$TMP/block1.md" "$TMP/patched1.md"
  [ "$(grep -c '^## Cross-language boundaries$' "$TMP/patched1.md")" -eq 1 ]
  # Second run: the body already carries the marker region; the fresh block
  # must replace it, not append a second heading.
  pushd "$TMP" > /dev/null
  bash "$EXTRACT" --languages rust,typescript child-a.md child-b.md > "$TMP/block2.md"
  popd > /dev/null
  patch_body "$TMP/patched1.md" "$TMP/block2.md" "$TMP/patched2.md"
  [ "$(grep -c '^## Cross-language boundaries$' "$TMP/patched2.md")" -eq 1 ]
  [ "$(grep -c 'autospec-shared-contracts:begin' "$TMP/patched2.md")" -eq 1 ]
  [ "$(grep -c '^## Shared contracts$' "$TMP/patched2.md")" -eq 1 ]
  # The dependency line survives both patches, still last under ## Dependencies.
  grep -q '^Depends on issue #999$' "$TMP/patched2.md"
}

@test "existing scan output is unchanged when --languages is absent" {
  write_body "$TMP/child-a.md" "schemas/roundtrip.schema.json"
  write_body "$TMP/child-b.md" "schemas/roundtrip.schema.json"
  pushd "$TMP" > /dev/null
  run bash "$EXTRACT" child-a.md child-b.md
  popd > /dev/null
  [ "$status" -eq 0 ]
  echo "$output" | grep -q '^<!-- autospec-shared-contracts:begin -->$'
  echo "$output" | grep -q '^<!-- autospec-shared-contracts:end -->$'
  ! echo "$output" | grep -q '## Cross-language boundaries'
}
