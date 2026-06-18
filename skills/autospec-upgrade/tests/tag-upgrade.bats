#!/usr/bin/env bats
# tag-upgrade.bats — TDD suite for tag-upgrade.sh (issue #1182)
# No real git. All git calls mocked via $MOCK_BIN PATH shim recording invocations.

SCRIPT="${BATS_TEST_DIRNAME}/../scripts/tag-upgrade.sh"
FIXTURE_DIR="${BATS_TEST_DIRNAME}/fixtures/tag-upgrade"

# ── Setup / teardown ──────────────────────────────────────────────────────────

setup() {
  TEST_ROOT="$(mktemp -d /tmp/tu-test-root.XXXXXX)"
  MOCK_BIN="$(mktemp -d /tmp/tu-mock-bin.XXXXXX)"
  AUTOSPEC_DIR="$TEST_ROOT/.autospec"
  TAGS_FILE="$MOCK_BIN/tags-recorded.txt"
  mkdir -p "$AUTOSPEC_DIR"
  touch "$TAGS_FILE"
  export TEST_ROOT MOCK_BIN AUTOSPEC_DIR TAGS_FILE

  # Install git mock that records tag names
  cat > "$MOCK_BIN/git" <<'GITEOF'
#!/usr/bin/env bash
# Fake git — records "git tag <name>" calls to $TAGS_FILE; silently passes anything else
if [ "$1" = "tag" ] && [ -n "$2" ]; then
  printf '%s\n' "$2" >> "$TAGS_FILE"
  exit 0
fi
exit 0
GITEOF
  chmod +x "$MOCK_BIN/git"
}

teardown() {
  rm -rf "$TEST_ROOT" "$MOCK_BIN"
}

# ── Helpers ───────────────────────────────────────────────────────────────────

tag_was_created() {
  local tag="$1"
  grep -qx "$tag" "$TAGS_FILE"
}

tag_was_not_created() {
  local tag="$1"
  ! grep -qx "$tag" "$TAGS_FILE"
}

# ── Existence / executability ─────────────────────────────────────────────────

@test "tag-upgrade.sh exists and is executable" {
  [ -x "$SCRIPT" ]
}

# ── pre mode: tag format ──────────────────────────────────────────────────────

@test "pre: creates tag named pre-upgrade-angular-17" {
  run env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" pre --framework angular --version 17 --out "$TEST_ROOT"
  [ "$status" -eq 0 ]
  tag_was_created "pre-upgrade-angular-17"
}

@test "pre: creates tag named pre-upgrade-next-15" {
  run env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" pre --framework next --version 15 --out "$TEST_ROOT"
  [ "$status" -eq 0 ]
  tag_was_created "pre-upgrade-next-15"
}

@test "pre: creates tag named pre-upgrade-react-19" {
  run env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" pre --framework react --version 19 --out "$TEST_ROOT"
  [ "$status" -eq 0 ]
  tag_was_created "pre-upgrade-react-19"
}

@test "pre: does NOT create a post tag" {
  run env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" pre --framework angular --version 17 --out "$TEST_ROOT"
  [ "$status" -eq 0 ]
  tag_was_not_created "post-upgrade-angular-17"
}

@test "pre: exits non-zero when --framework is missing" {
  run env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" pre --version 17 --out "$TEST_ROOT"
  [ "$status" -ne 0 ]
}

@test "pre: exits non-zero when --version is missing" {
  run env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" pre --framework angular --out "$TEST_ROOT"
  [ "$status" -ne 0 ]
}

# ── post mode: tag format (proof passes) ──────────────────────────────────────

@test "post: creates tag named post-upgrade-angular-17 when proof passes" {
  run env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" post --framework angular --version 17 \
      --proof "$FIXTURE_DIR/proof-passing.json" --out "$TEST_ROOT"
  [ "$status" -eq 0 ]
  tag_was_created "post-upgrade-angular-17"
}

@test "post: creates tag named post-upgrade-next-15 when proof passes" {
  run env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" post --framework next --version 15 \
      --proof "$FIXTURE_DIR/proof-passing.json" --out "$TEST_ROOT"
  [ "$status" -eq 0 ]
  tag_was_created "post-upgrade-next-15"
}

@test "post: creates tag named post-upgrade-react-19 when proof passes" {
  run env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" post --framework react --version 19 \
      --proof "$FIXTURE_DIR/proof-passing.json" --out "$TEST_ROOT"
  [ "$status" -eq 0 ]
  tag_was_created "post-upgrade-react-19"
}

@test "post: does NOT create a pre tag" {
  run env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" post --framework angular --version 17 \
      --proof "$FIXTURE_DIR/proof-passing.json" --out "$TEST_ROOT"
  [ "$status" -eq 0 ]
  tag_was_not_created "pre-upgrade-angular-17"
}

# ── post mode: WITHHOLD tag when proof fails ──────────────────────────────────

@test "post: WITHHOLDS tag when proof passed=false" {
  run env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" post --framework angular --version 17 \
      --proof "$FIXTURE_DIR/proof-failing.json" --out "$TEST_ROOT"
  [ "$status" -ne 0 ]
  tag_was_not_created "post-upgrade-angular-17"
}

@test "post: WITHHOLDS tag when post_upgrade.score < baseline.score (inline proof)" {
  local proof_file="$TEST_ROOT/proof-regressed.json"
  printf '{"baseline":{"score":80},"post_upgrade":{"score":70},"passed":true}\n' \
    > "$proof_file"
  run env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" post --framework react --version 19 \
      --proof "$proof_file" --out "$TEST_ROOT"
  [ "$status" -ne 0 ]
  tag_was_not_created "post-upgrade-react-19"
}

@test "post: WITHHOLDS tag when passed=false even if score >= baseline" {
  local proof_file="$TEST_ROOT/proof-explicit-false.json"
  printf '{"baseline":{"score":70},"post_upgrade":{"score":80},"passed":false}\n' \
    > "$proof_file"
  run env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" post --framework next --version 15 \
      --proof "$proof_file" --out "$TEST_ROOT"
  [ "$status" -ne 0 ]
  tag_was_not_created "post-upgrade-next-15"
}

@test "post: output mentions withheld when gate fails" {
  run bash -c "env PATH=\"$MOCK_BIN:\$PATH\" \
    \"$SCRIPT\" post --framework angular --version 17 \
      --proof \"$FIXTURE_DIR/proof-failing.json\" --out \"$TEST_ROOT\" 2>&1"
  [ "$status" -ne 0 ]
  printf '%s\n' "$output" | grep -qi 'withheld\|below\|failed\|not tagged'
}

@test "post: exits non-zero when proof file is missing" {
  run env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" post --framework angular --version 17 \
      --proof "$TEST_ROOT/nonexistent-proof.json" --out "$TEST_ROOT"
  [ "$status" -ne 0 ]
}

@test "post: exits non-zero when --framework is missing" {
  run env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" post --version 17 \
      --proof "$FIXTURE_DIR/proof-passing.json" --out "$TEST_ROOT"
  [ "$status" -ne 0 ]
}

@test "post: exits non-zero when --version is missing" {
  run env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" post --framework angular \
      --proof "$FIXTURE_DIR/proof-passing.json" --out "$TEST_ROOT"
  [ "$status" -ne 0 ]
}

# ── report mode: emits upgrade-report.json with documented fields ─────────────

@test "report: emits .autospec/upgrade-report.json" {
  run env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" report \
      --framework angular --from 16 --to 17 \
      --out "$TEST_ROOT"
  [ "$status" -eq 0 ]
  [ -f "$TEST_ROOT/.autospec/upgrade-report.json" ]
}

@test "report: upgrade-report.json contains framework field" {
  env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" report \
      --framework angular --from 16 --to 17 \
      --out "$TEST_ROOT"
  jq -e '.[0] | has("framework")' "$TEST_ROOT/.autospec/upgrade-report.json"
}

@test "report: upgrade-report.json contains from field" {
  env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" report \
      --framework angular --from 16 --to 17 \
      --out "$TEST_ROOT"
  jq -e '.[0] | has("from")' "$TEST_ROOT/.autospec/upgrade-report.json"
}

@test "report: upgrade-report.json contains to field" {
  env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" report \
      --framework angular --from 16 --to 17 \
      --out "$TEST_ROOT"
  jq -e '.[0] | has("to")' "$TEST_ROOT/.autospec/upgrade-report.json"
}

@test "report: upgrade-report.json contains codemods field (array)" {
  env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" report \
      --framework angular --from 16 --to 17 \
      --out "$TEST_ROOT"
  jq -e '.[0].codemods | type == "array"' "$TEST_ROOT/.autospec/upgrade-report.json"
}

@test "report: upgrade-report.json contains manual_fixes field (array)" {
  env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" report \
      --framework angular --from 16 --to 17 \
      --out "$TEST_ROOT"
  jq -e '.[0].manual_fixes | type == "array"' "$TEST_ROOT/.autospec/upgrade-report.json"
}

@test "report: upgrade-report.json contains residual_risk field" {
  env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" report \
      --framework angular --from 16 --to 17 \
      --out "$TEST_ROOT"
  jq -e '.[0] | has("residual_risk")' "$TEST_ROOT/.autospec/upgrade-report.json"
}

@test "report: upgrade-report.json framework value matches --framework arg" {
  env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" report \
      --framework next --from 14 --to 15 \
      --out "$TEST_ROOT"
  local fw
  fw="$(jq -r '.[0].framework' "$TEST_ROOT/.autospec/upgrade-report.json")"
  [ "$fw" = "next" ]
}

@test "report: upgrade-report.json from/to values match args" {
  env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" report \
      --framework react --from 18 --to 19 \
      --out "$TEST_ROOT"
  local from to
  from="$(jq -r '.[0].from' "$TEST_ROOT/.autospec/upgrade-report.json")"
  to="$(jq -r '.[0].to' "$TEST_ROOT/.autospec/upgrade-report.json")"
  [ "$from" = "18" ]
  [ "$to" = "19" ]
}

@test "report: appends a second hop when called twice" {
  env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" report --framework angular --from 15 --to 16 --out "$TEST_ROOT"
  env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" report --framework angular --from 16 --to 17 --out "$TEST_ROOT"
  local count
  count="$(jq 'length' "$TEST_ROOT/.autospec/upgrade-report.json")"
  [ "$count" -eq 2 ]
}

@test "report: accepts optional --codemods and includes in output" {
  env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" report \
      --framework angular --from 16 --to 17 \
      --codemods "ng update @angular/core" \
      --out "$TEST_ROOT"
  local cmd
  cmd="$(jq -r '.[0].codemods[0]' "$TEST_ROOT/.autospec/upgrade-report.json")"
  [ "$cmd" = "ng update @angular/core" ]
}

@test "report: accepts optional --residual-risk and includes in output" {
  env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" report \
      --framework angular --from 16 --to 17 \
      --residual-risk "Manual review needed for lazy-loaded modules" \
      --out "$TEST_ROOT"
  local risk
  risk="$(jq -r '.[0].residual_risk' "$TEST_ROOT/.autospec/upgrade-report.json")"
  [ "$risk" = "Manual review needed for lazy-loaded modules" ]
}
