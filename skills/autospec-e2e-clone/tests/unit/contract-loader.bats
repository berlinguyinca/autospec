#!/usr/bin/env bats
# skills/autospec-e2e-clone/tests/unit/contract-loader.bats
# TDD: bats fixtures for load-contract.sh (C1 scaffold)
# Exit codes: 0=ok, 1=fatal, 2=refuse-to-run (contract invalid / missing required fields)

SKILL_DIR="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
LOADER="$SKILL_DIR/scripts/load-contract.sh"
FIX="$SKILL_DIR/tests/unit/fixtures"

# Case 1: minimal-valid fixture — exits 0, emits JSON with sources and expose
@test "minimal-valid: exits 0 and emits JSON with sources and expose" {
  run bash "$LOADER" "$FIX/minimal-valid"
  [ "$status" -eq 0 ]
  echo "$output" | grep -q '"sources"'
  echo "$output" | grep -q '"expose"'
}

# Case 2: missing-sources fixture — exits 2 (refuse-to-run)
@test "missing-sources: exits 2 when sources is absent" {
  run bash "$LOADER" "$FIX/missing-sources"
  [ "$status" -eq 2 ]
}

# Case 3: missing-expose fixture — exits 2 (refuse-to-run)
@test "missing-expose: exits 2 when expose is absent" {
  run bash "$LOADER" "$FIX/missing-expose"
  [ "$status" -eq 2 ]
}

# Case 4: full-shape fixture — exits 0 and all top-level keys present
@test "full-shape: exits 0 and emits JSON with all declared top-level keys" {
  run bash "$LOADER" "$FIX/full-shape"
  [ "$status" -eq 0 ]
  echo "$output" | grep -q '"sources"'
  echo "$output" | grep -q '"expose"'
  echo "$output" | grep -q '"anonymize"'
  echo "$output" | grep -q '"scale_down"'
  echo "$output" | grep -q '"edge_case_seed"'
}

# Case 5: unparseable fixture — exits 1 (fatal: parse error)
@test "unparseable: exits 1 on YAML parse error" {
  run bash "$LOADER" "$FIX/unparseable"
  [ "$status" -eq 1 ]
}

# Case 6: missing .autospec/clone.yml — exits 2 (refuse-to-run: contract missing)
@test "missing clone.yml: exits 2 when .autospec/clone.yml does not exist" {
  tmpdir=$(mktemp -d)
  mkdir -p "$tmpdir/.autospec"
  run bash "$LOADER" "$tmpdir"
  rm -rf "$tmpdir"
  [ "$status" -eq 2 ]
}

# Case 7: missing repo_root — exits 1 (fatal)
@test "missing repo_root: exits 1 when repo_root directory does not exist" {
  run bash "$LOADER" /tmp/does-not-exist-clone-fixture-xyz
  [ "$status" -eq 1 ]
}
