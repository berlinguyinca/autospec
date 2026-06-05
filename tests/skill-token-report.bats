#!/usr/bin/env bats
# tests/skill-token-report.bats — bats coverage for scripts/skill-token-report.sh

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/.." && pwd)"
BIN="$REPO_ROOT/scripts/skill-token-report.sh"

# ---------------------------------------------------------------------------
# Positive: basic exit and output
# ---------------------------------------------------------------------------

@test "script exits 0 with no args (table mode)" {
  run bash "$BIN"
  [ "$status" -eq 0 ]
}

@test "--json flag exits 0" {
  run bash "$BIN" --json
  [ "$status" -eq 0 ]
}

@test "--json output is valid JSON array" {
  run bash "$BIN" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '. | type == "array"'
}

@test "--json row count equals skill count" {
  # Count skills that have a SKILL.md
  expected=0
  for d in "$REPO_ROOT/skills/"/*/; do
    [ -f "${d}SKILL.md" ] && expected=$((expected + 1))
  done
  run bash "$BIN" --json
  [ "$status" -eq 0 ]
  actual=$(echo "$output" | jq '. | length')
  [ "$actual" -eq "$expected" ]
}

@test "--json first row has integer .words field" {
  run bash "$BIN" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.[0].words | type == "number"'
}

@test "--json first row has integer .tokens field" {
  run bash "$BIN" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.[0].tokens | type == "number"'
}

@test "--json tokens equals floor(words * 133 / 100)" {
  run bash "$BIN" --json
  [ "$status" -eq 0 ]
  # Verify every row satisfies: tokens == words * 133 / 100 (integer division)
  echo "$output" | jq -e '
    [ .[] | select( .tokens != ( .words * 133 / 100 | floor ) ) ] | length == 0
  '
}

@test "--json first row has .skill string field" {
  run bash "$BIN" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.[0].skill | type == "string"'
}

@test "table mode output contains markdown table header" {
  run bash "$BIN"
  [ "$status" -eq 0 ]
  echo "$output" | grep -q "| Skill"
}

@test "table mode output has a separator row" {
  run bash "$BIN"
  [ "$status" -eq 0 ]
  echo "$output" | grep -qE '^\|[-| ]+\|'
}

@test "primary smoke test: length>0 and first .tokens is number" {
  run bash "$BIN" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e 'length>0 and (.[0].tokens|type=="number")'
}

# ---------------------------------------------------------------------------
# Negative: zero-word fixture skill yields 0-token row (not error)
# ---------------------------------------------------------------------------

@test "zero-word SKILL.md yields 0-token row (not error)" {
  # Create a minimal fixture skill directory in a temp location
  local tmpdir
  tmpdir="$(mktemp -d)"
  local fixture_skill="$tmpdir/skills/fixture-zero"
  mkdir -p "$fixture_skill"
  # Empty SKILL.md
  touch "$fixture_skill/SKILL.md"

  # Run script with SKILLS_DIR overridden to tmpdir/skills
  run bash "$BIN" --json --skills-dir "$tmpdir/skills"
  [ "$status" -eq 0 ]
  # The fixture skill should appear with tokens == 0
  echo "$output" | jq -e '[.[] | select(.skill == "fixture-zero")] | length == 1'
  echo "$output" | jq -e '[.[] | select(.skill == "fixture-zero")][0].tokens == 0'

  rm -rf "$tmpdir"
}

@test "zero-word SKILL.md: tokens field is integer 0 not float" {
  local tmpdir
  tmpdir="$(mktemp -d)"
  local fixture_skill="$tmpdir/skills/fixture-zero"
  mkdir -p "$fixture_skill"
  touch "$fixture_skill/SKILL.md"

  run bash "$BIN" --json --skills-dir "$tmpdir/skills"
  [ "$status" -eq 0 ]
  # .tokens must be exactly integer 0, not 0.0 (jq type check)
  echo "$output" | jq -e '[.[] | select(.skill == "fixture-zero")][0].tokens | . == 0 and type == "number"'

  rm -rf "$tmpdir"
}

# ---------------------------------------------------------------------------
# Negative: non-integer assertion would fail if formula left as float
# Verify: words * 1.33 truncated to int, not left as float
# ---------------------------------------------------------------------------

@test "tokens is always an integer (no decimal point in any row)" {
  run bash "$BIN" --json
  [ "$status" -eq 0 ]
  # None of the .tokens values should be a float with fractional part
  echo "$output" | jq -e '[.[] | select( .tokens | . != floor )] | length == 0'
}

# ---------------------------------------------------------------------------
# --help flag
# ---------------------------------------------------------------------------

@test "--help exits 0" {
  run bash "$BIN" --help
  [ "$status" -eq 0 ]
}

# ---------------------------------------------------------------------------
# --update-baseline: splice mode (negative-path pairs)
# ---------------------------------------------------------------------------

_mk_baseline_workdir() {
  # Temp cwd with a marker-wrapped baseline + one fixture skill.
  WORK="$(mktemp -d)"
  mkdir -p "$WORK/docs/reports" "$WORK/skills/demo-skill"
  printf 'one two three four\n' > "$WORK/skills/demo-skill/SKILL.md"
  cat > "$WORK/docs/reports/skill-token-baseline.md" << 'BASE'
# Skill token baseline

Header prose that must survive the splice.

<!-- baseline:begin -->
| Skill | Words | Tokens |
|-------|------:|-------:|
| stale-row | 999 | 999 |
<!-- baseline:end -->

Footer prose that must survive the splice.
BASE
}

@test "--update-baseline splices fresh table, preserves header/footer/markers" {
  _mk_baseline_workdir
  cd "$WORK"
  run bash "$BIN" --skills-dir "$WORK/skills"
  table_first_data_row="$(echo "$output" | sed -n '3p')"
  run bash -c "cd '$WORK' && bash '$BIN' --skills-dir '$WORK/skills' --update-baseline"
  [ "$status" -eq 0 ]
  b="$WORK/docs/reports/skill-token-baseline.md"
  grep -q 'Header prose that must survive' "$b"
  grep -q 'Footer prose that must survive' "$b"
  [ "$(grep -c 'baseline:begin\|baseline:end' "$b")" -eq 2 ]
  grep -q 'demo-skill' "$b"
  if grep -q 'stale-row' "$b"; then false; fi
}

@test "--update-baseline is idempotent (second run leaves file byte-identical)" {
  _mk_baseline_workdir
  bash -c "cd '$WORK' && bash '$BIN' --skills-dir '$WORK/skills' --update-baseline" > /dev/null
  h1="$(shasum -a 256 "$WORK/docs/reports/skill-token-baseline.md")"
  bash -c "cd '$WORK' && bash '$BIN' --skills-dir '$WORK/skills' --update-baseline" > /dev/null
  h2="$(shasum -a 256 "$WORK/docs/reports/skill-token-baseline.md")"
  [ "$h1" = "$h2" ]
}

@test "--update-baseline fails loudly (exit 2) when markers are missing" {
  _mk_baseline_workdir
  printf '# No markers here\n' > "$WORK/docs/reports/skill-token-baseline.md"
  run bash -c "cd '$WORK' && bash '$BIN' --skills-dir '$WORK/skills' --update-baseline"
  [ "$status" -eq 2 ]
  echo "$output" | grep -q 'refusing to splice'
  grep -q 'No markers here' "$WORK/docs/reports/skill-token-baseline.md"
}

@test "--update-baseline fails loudly (exit 2) when baseline file is absent" {
  _mk_baseline_workdir
  rm "$WORK/docs/reports/skill-token-baseline.md"
  run bash -c "cd '$WORK' && bash '$BIN' --skills-dir '$WORK/skills' --update-baseline"
  [ "$status" -eq 2 ]
  echo "$output" | grep -q 'cannot --update-baseline'
}

@test "--json and --update-baseline are mutually exclusive (exit 1)" {
  run bash "$BIN" --json --update-baseline
  [ "$status" -eq 1 ]
  echo "$output" | grep -q 'mutually exclusive'
}
