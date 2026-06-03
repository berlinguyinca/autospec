#!/usr/bin/env bats
# skills/autospec-run/tests/post-token-report.bats — TDD for post-token-report.sh (issue #938)
#
# Uses PATH-shadowed `gh` mock so no real GitHub API is called.

SCRIPT="${BATS_TEST_DIRNAME}/../scripts/post-token-report.sh"
FIXTURES_DIR="${BATS_TEST_DIRNAME}/fixtures/post-token-report"

setup() {
  mkdir -p "$FIXTURES_DIR/bin"

  # Write a full tokens JSON fixture
  cat > "$FIXTURES_DIR/tokens-938.json" << 'FIXTURE'
{
  "implementer": {
    "input_tokens": 121643,
    "cache_creation_input_tokens": 8000,
    "cache_read_input_tokens": 4000,
    "output_tokens": 3500,
    "model": "claude-opus-4-5"
  },
  "reviewer": {
    "input_tokens": 41200,
    "cache_creation_input_tokens": 0,
    "cache_read_input_tokens": 3000,
    "output_tokens": 1800,
    "model": "claude-sonnet-4-5"
  },
  "recovery": null,
  "pr": 930
}
FIXTURE

  # Write a minimal/partial tokens JSON (some fields missing)
  cat > "$FIXTURES_DIR/tokens-partial.json" << 'FIXTURE'
{
  "implementer": {
    "input_tokens": 50000
  },
  "pr": 930
}
FIXTURE

  # PATH-shadowed gh mock — records call args and returns canned responses
  cat > "$FIXTURES_DIR/bin/gh" << 'STUB'
#!/usr/bin/env bash
# Minimal gh stub for post-token-report tests
GH_LOG="${GH_MOCK_LOG:-/tmp/gh-mock-$$.log}"
printf '%s\n' "$*" >> "$GH_LOG"

case "$*" in
  "issue list-comments"*|"api repos"*/comments*)
    # First call: no existing comment (no marker)
    printf '[]\n'
    exit 0
    ;;
  "issue comment"*)
    # Post new comment — echo back the body arg
    printf 'comment created\n'
    exit 0
    ;;
  "issue edit-comment"*|*"--edit-last"*)
    printf 'comment updated\n'
    exit 0
    ;;
  *)
    # For any other gh call, succeed silently
    exit 0
    ;;
esac
STUB
  chmod 0755 "$FIXTURES_DIR/bin/gh"

  export PATH="$FIXTURES_DIR/bin:$PATH"
  export GH_MOCK_LOG="$FIXTURES_DIR/gh-calls.log"
  rm -f "$GH_MOCK_LOG"
}

teardown() {
  rm -rf "$FIXTURES_DIR"
}

# ── Existence / executability ─────────────────────────────────────────────────

@test "post-token-report.sh exists and is executable" {
  [ -x "$SCRIPT" ]
}

@test "post-token-report.sh exits 1 on missing required args" {
  run "$SCRIPT"
  [ "$status" -eq 1 ]
}

@test "post-token-report.sh exits 1 when --issue missing value" {
  run "$SCRIPT" --repo berlinguyinca/autospec
  [ "$status" -eq 1 ]
}

@test "post-token-report.sh exits 1 when --repo missing value" {
  run "$SCRIPT" --issue 938
  [ "$status" -eq 1 ]
}

# ── Compose comment body ──────────────────────────────────────────────────────

@test "post-token-report.sh composes comment with implementer tokens" {
  run "$SCRIPT" --issue 938 --repo berlinguyinca/autospec \
    --tokens-json "$FIXTURES_DIR/tokens-938.json"
  [ "$status" -eq 0 ]
  # implementer total = input(121643) + output(3500) = 125,143
  printf '%s\n' "$output" | grep -qiE "implementer.*125[,.]?143|125[,.]?143.*implementer"
}

@test "post-token-report.sh composes comment with reviewer tokens" {
  run "$SCRIPT" --issue 938 --repo berlinguyinca/autospec \
    --tokens-json "$FIXTURES_DIR/tokens-938.json"
  [ "$status" -eq 0 ]
  # reviewer total = input(41200) + output(1800) = 43,000
  printf '%s\n' "$output" | grep -qiE "reviewer.*43[,.]?000|43[,.]?000.*reviewer"
}

@test "post-token-report.sh composes comment with PR number" {
  run "$SCRIPT" --issue 938 --repo berlinguyinca/autospec \
    --tokens-json "$FIXTURES_DIR/tokens-938.json"
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -qE "#930|PR.*930|930.*PR"
}

@test "post-token-report.sh includes marker begin/end in output" {
  run "$SCRIPT" --issue 938 --repo berlinguyinca/autospec \
    --tokens-json "$FIXTURES_DIR/tokens-938.json"
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -qF "autospec-tokens:begin"
  printf '%s\n' "$output" | grep -qF "autospec-tokens:end"
}

@test "post-token-report.sh includes Token usage heading" {
  run "$SCRIPT" --issue 938 --repo berlinguyinca/autospec \
    --tokens-json "$FIXTURES_DIR/tokens-938.json"
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -qiE "token.usage|## Token"
}

# ── Missing / partial tokens-json fallback ────────────────────────────────────

@test "post-token-report.sh exits 0 when tokens-json is absent (fallback)" {
  run "$SCRIPT" --issue 938 --repo berlinguyinca/autospec \
    --tokens-json "$FIXTURES_DIR/nonexistent.json"
  [ "$status" -eq 0 ]
}

@test "post-token-report.sh posts unavailable message when tokens-json absent" {
  run "$SCRIPT" --issue 938 --repo berlinguyinca/autospec \
    --tokens-json "$FIXTURES_DIR/nonexistent.json"
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -qi "unavailable"
}

@test "post-token-report.sh exits 0 when --tokens-json not provided at all" {
  run "$SCRIPT" --issue 938 --repo berlinguyinca/autospec
  [ "$status" -eq 0 ]
}

@test "post-token-report.sh exits 0 on partial tokens-json (missing reviewer)" {
  run "$SCRIPT" --issue 938 --repo berlinguyinca/autospec \
    --tokens-json "$FIXTURES_DIR/tokens-partial.json"
  [ "$status" -eq 0 ]
}

# ── Idempotent edit-in-place ──────────────────────────────────────────────────

@test "post-token-report.sh is idempotent: second run edits existing comment" {
  # First run — gh returns no existing comment, should create
  run "$SCRIPT" --issue 938 --repo berlinguyinca/autospec \
    --tokens-json "$FIXTURES_DIR/tokens-938.json"
  [ "$status" -eq 0 ]

  # Create a stub that mimics an existing marker comment
  cat > "$FIXTURES_DIR/bin/gh" << 'STUB2'
#!/usr/bin/env bash
GH_MOCK_LOG="${GH_MOCK_LOG:-/tmp/gh-mock-$$.log}"
printf '%s\n' "$*" >> "$GH_MOCK_LOG"
case "$*" in
  "api repos"*"/issues/938/comments"*"--method GET"*)
    # Return a comment with the marker
    printf '[{"id":99001,"body":"<!-- autospec-tokens:begin -->old tokens<!-- autospec-tokens:end -->"}]\n'
    exit 0
    ;;
  "api repos"*"/issues/comments/99001"*"--method PATCH"*)
    printf '{"id":99001}\n'
    exit 0
    ;;
  *)
    exit 0
    ;;
esac
STUB2
  chmod 0755 "$FIXTURES_DIR/bin/gh"

  # Second run — should detect existing marker and edit in place (exit 0)
  run "$SCRIPT" --issue 938 --repo berlinguyinca/autospec \
    --tokens-json "$FIXTURES_DIR/tokens-938.json"
  [ "$status" -eq 0 ]
  # The edit-in-place call must appear in the log
  grep -qE "PATCH|edit|99001" "$GH_MOCK_LOG" || \
    grep -qE "issues/comments" "$GH_MOCK_LOG"
}

# ── Telemetry e2e: tokens-json → telemetry row golden ─────────────────────────

@test "record-telemetry.sh accepts the tokens-json implementer sub-object shape" {
  TELEMETRY_FILE="$FIXTURES_DIR/telemetry.jsonl"
  # Extract implementer object into a flat file (the recorder role='implementer')
  FLAT_JSON="$FIXTURES_DIR/flat-implementer.json"
  jq '.implementer // {}' "$FIXTURES_DIR/tokens-938.json" > "$FLAT_JSON"

  RECORD_SCRIPT="${BATS_TEST_DIRNAME}/../../autospec-shared/scripts/record-telemetry.sh"
  AUTOSPEC_TELEMETRY_FILE="$TELEMETRY_FILE" run bash "$RECORD_SCRIPT" \
    --dispatch-id "test-d1" \
    --role implementer \
    --issue 938 \
    --tokens-json "$FLAT_JSON"
  [ "$status" -eq 0 ]
  [ -f "$TELEMETRY_FILE" ]
  grep -q '"role":"implementer"' "$TELEMETRY_FILE"
  grep -q '"input_tokens":121643' "$TELEMETRY_FILE"
}

# ── Trio presence checks ──────────────────────────────────────────────────────

@test "SKILL.md includes post-token-report.sh step after admin-merge" {
  skill_md="${BATS_TEST_DIRNAME}/../SKILL.md"
  grep -qF "post-token-report.sh" "$skill_md"
}

@test "codex/prompt.md includes post-token-report.sh step (lock-step)" {
  codex_md="${BATS_TEST_DIRNAME}/../codex/prompt.md"
  grep -qF "post-token-report.sh" "$codex_md"
}

@test "opencode/agent.md includes post-token-report.sh step (lock-step)" {
  opencode_md="${BATS_TEST_DIRNAME}/../opencode/agent.md"
  grep -qF "post-token-report.sh" "$opencode_md"
}
