#!/usr/bin/env bats
# Tests for advisor-escalate.sh — the deterministic advisor bookkeeping primitive.

setup() {
  SCRIPT="${BATS_TEST_DIRNAME}/../../scripts/advisor-escalate.sh"
  TMP="$(mktemp -d)"
  export AUTOSPEC_ADVISOR_STATE_DIR="$TMP/state"
  export AUTOSPEC_ADVISOR=on
  export AUTOSPEC_ADVISOR_GATES=impl-haiku,retry,reviewer,impl-decision
  export AUTOSPEC_HARNESS=claude
  printf 'How should I resolve the ambiguous contract?' > "$TMP/q.txt"
  printf 'Issue #42 context here.' > "$TMP/c.txt"
}

teardown() { rm -rf "$TMP"; }

# ── precheck: gating ──────────────────────────────────────────────────────────

@test "precheck GO on first use emits decision GO and use_count 1" {
  run bash "$SCRIPT" --phase precheck --issue 42 --repo acme/widget \
    --gate impl-haiku --question-file "$TMP/q.txt" --context-file "$TMP/c.txt" --json
  [ "$status" -eq 0 ]
  echo "$output" | grep -q '"decision":"GO"'
  echo "$output" | grep -q '"use_count":1'
}

@test "precheck disabled when AUTOSPEC_ADVISOR=off exits 8" {
  export AUTOSPEC_ADVISOR=off
  run bash "$SCRIPT" --phase precheck --issue 42 --repo acme/widget \
    --gate impl-haiku --question-file "$TMP/q.txt" --context-file "$TMP/c.txt" --json
  [ "$status" -eq 8 ]
  echo "$output" | grep -q '"decision":"DISABLED"'
}

@test "precheck disabled when gate not in AUTOSPEC_ADVISOR_GATES exits 8" {
  export AUTOSPEC_ADVISOR_GATES=impl-haiku
  run bash "$SCRIPT" --phase precheck --issue 42 --repo acme/widget \
    --gate reviewer --question-file "$TMP/q.txt" --context-file "$TMP/c.txt" --json
  [ "$status" -eq 8 ]
}

# ── precheck: cap ─────────────────────────────────────────────────────────────

@test "precheck cap-reached after MAX_USES exits 7" {
  export AUTOSPEC_ADVISOR_MAX_USES=2
  for _ in 1 2; do
    bash "$SCRIPT" --phase precheck --issue 42 --repo acme/widget \
      --gate impl-haiku --question-file "$TMP/q.txt" --context-file "$TMP/c.txt" --json
  done
  run bash "$SCRIPT" --phase precheck --issue 42 --repo acme/widget \
    --gate impl-decision --question-file "$TMP/q.txt" --context-file "$TMP/c.txt" --json
  [ "$status" -eq 7 ]
  echo "$output" | grep -q '"decision":"CAP-REACHED"'
}

@test "cap is shared across gates for one issue" {
  export AUTOSPEC_ADVISOR_MAX_USES=1
  bash "$SCRIPT" --phase precheck --issue 42 --repo acme/widget \
    --gate impl-haiku --question-file "$TMP/q.txt" --context-file "$TMP/c.txt" --json
  run bash "$SCRIPT" --phase precheck --issue 42 --repo acme/widget \
    --gate reviewer --question-file "$TMP/q.txt" --context-file "$TMP/c.txt" --json
  [ "$status" -eq 7 ]
}

@test "state is path-scoped: same issue number, two repos, no collision" {
  export AUTOSPEC_ADVISOR_MAX_USES=1
  bash "$SCRIPT" --phase precheck --issue 7 --repo acme/one \
    --gate impl-haiku --question-file "$TMP/q.txt" --context-file "$TMP/c.txt" --json
  run bash "$SCRIPT" --phase precheck --issue 7 --repo acme/two \
    --gate impl-haiku --question-file "$TMP/q.txt" --context-file "$TMP/c.txt" --json
  [ "$status" -eq 0 ]
  echo "$output" | grep -q '"use_count":1'
}

# ── precheck: backend resolution ──────────────────────────────────────────────

@test "precheck emits harness-specific cli_fallback for codex" {
  export AUTOSPEC_HARNESS=codex
  run bash "$SCRIPT" --phase precheck --issue 42 --repo acme/widget \
    --gate impl-haiku --question-file "$TMP/q.txt" --context-file "$TMP/c.txt" --json
  [ "$status" -eq 0 ]
  echo "$output" | grep -q '"harness":"codex"'
  echo "$output" | grep -q 'codex exec'
}

# ── record: validation, budget, telemetry ─────────────────────────────────────

@test "record validates a well-formed plan verdict" {
  printf '{"verdict":"plan","guidance":"Do X then Y.","confidence":"high"}' > "$TMP/r.json"
  run bash "$SCRIPT" --phase record --issue 42 --repo acme/widget \
    --gate impl-haiku --response-file "$TMP/r.json" --json
  [ "$status" -eq 0 ]
  echo "$output" | grep -q '"verdict":"plan"'
  echo "$output" | grep -q '"over_budget":false'
}

@test "record truncates over-budget guidance and flags over_budget" {
  export AUTOSPEC_ADVISOR_MAX_CHARS=20
  big="$(printf 'x%.0s' $(seq 1 100))"
  printf '{"verdict":"correction","guidance":"%s"}' "$big" > "$TMP/r.json"
  run bash "$SCRIPT" --phase record --issue 42 --repo acme/widget \
    --gate retry --response-file "$TMP/r.json" --json
  [ "$status" -eq 0 ]
  echo "$output" | grep -q '"over_budget":true'
}

@test "record fail-safes to stop on unparseable response, exit 2" {
  printf 'not json at all' > "$TMP/r.json"
  run bash "$SCRIPT" --phase record --issue 42 --repo acme/widget \
    --gate impl-haiku --response-file "$TMP/r.json" --json
  [ "$status" -eq 2 ]
  echo "$output" | grep -q '"verdict":"stop"'
}

@test "record rejects an invalid verdict value, fail-safe stop" {
  printf '{"verdict":"proceed","guidance":"go"}' > "$TMP/r.json"
  run bash "$SCRIPT" --phase record --issue 42 --repo acme/widget \
    --gate impl-haiku --response-file "$TMP/r.json" --json
  [ "$status" -eq 2 ]
  echo "$output" | grep -q '"verdict":"stop"'
}

@test "record appends one telemetry line with gate and verdict" {
  export AUTOSPEC_TELEMETRY_DIR="$TMP/telem"
  printf '{"verdict":"plan","guidance":"ok"}' > "$TMP/r.json"
  bash "$SCRIPT" --phase record --issue 42 --repo acme/widget \
    --gate reviewer --response-file "$TMP/r.json" --json
  [ -f "$TMP/telem/advisor-escalate.jsonl" ]
  grep -q '"gate":"reviewer"' "$TMP/telem/advisor-escalate.jsonl"
  grep -q '"verdict":"plan"' "$TMP/telem/advisor-escalate.jsonl"
  [ "$(wc -l < "$TMP/telem/advisor-escalate.jsonl")" -eq 1 ]
}

# ── robustness / negative paths (from final review) ───────────────────────────

@test "precheck does NOT burn a cap slot when the question file is missing" {
  run bash "$SCRIPT" --phase precheck --issue 42 --repo acme/widget \
    --gate impl-haiku --question-file "$TMP/nope.txt" --context-file "$TMP/c.txt" --json
  [ "$status" -ne 0 ]
  # No state file written → next real precheck still starts at use_count 1.
  run bash "$SCRIPT" --phase precheck --issue 42 --repo acme/widget \
    --gate impl-haiku --question-file "$TMP/q.txt" --context-file "$TMP/c.txt" --json
  [ "$status" -eq 0 ]
  echo "$output" | grep -q '"use_count":1'
}

@test "precheck self-heals a corrupted non-integer use_count instead of crashing" {
  mkdir -p "$AUTOSPEC_ADVISOR_STATE_DIR/acme_widget"
  printf '{"use_count":"abc"}' > "$AUTOSPEC_ADVISOR_STATE_DIR/acme_widget/42.json"
  run bash "$SCRIPT" --phase precheck --issue 42 --repo acme/widget \
    --gate impl-haiku --question-file "$TMP/q.txt" --context-file "$TMP/c.txt" --json
  [ "$status" -eq 0 ]
  echo "$output" | grep -q '"use_count":1'
}

@test "precheck rejects a non-numeric issue (path-traversal guard)" {
  run bash "$SCRIPT" --phase precheck --issue '../evil' --repo acme/widget \
    --gate impl-haiku --question-file "$TMP/q.txt" --context-file "$TMP/c.txt" --json
  [ "$status" -eq 1 ]
}

@test "record truncates over-budget guidance to MAX_CHARS exactly" {
  export AUTOSPEC_ADVISOR_MAX_CHARS=20
  export AUTOSPEC_TELEMETRY_DIR="$TMP/telem"
  big="$(printf 'x%.0s' $(seq 1 100))"
  printf '{"verdict":"plan","guidance":"%s"}' "$big" > "$TMP/r.json"
  run bash "$SCRIPT" --phase record --issue 42 --repo acme/widget \
    --gate impl-haiku --response-file "$TMP/r.json" --json
  [ "$status" -eq 0 ]
  glen="$(echo "$output" | jq -r '.guidance | length')"
  [ "$glen" -eq 20 ]
}

@test "record strips a function-call marker from guidance" {
  export AUTOSPEC_TELEMETRY_DIR="$TMP/telem"
  printf '{"verdict":"plan","guidance":"Do X <invoke name=\\"Bash\\"> then Y"}' > "$TMP/r.json"
  run bash "$SCRIPT" --phase record --issue 42 --repo acme/widget \
    --gate impl-haiku --response-file "$TMP/r.json" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -r '.guidance' | grep -qv 'invoke'
}

@test "record telemetry line is valid JSON even when repo contains a quote" {
  export AUTOSPEC_TELEMETRY_DIR="$TMP/telem"
  printf '{"verdict":"plan","guidance":"ok"}' > "$TMP/r.json"
  bash "$SCRIPT" --phase record --issue 42 --repo 'a/b"x' \
    --gate reviewer --response-file "$TMP/r.json" --json
  # Every telemetry line must round-trip through jq.
  run jq -e '.repo' "$TMP/telem/advisor-escalate.jsonl"
  [ "$status" -eq 0 ]
}

@test "record telemetry includes use_count from state" {
  export AUTOSPEC_TELEMETRY_DIR="$TMP/telem"
  mkdir -p "$AUTOSPEC_ADVISOR_STATE_DIR/acme_widget"
  printf '{"use_count":2}' > "$AUTOSPEC_ADVISOR_STATE_DIR/acme_widget/42.json"
  printf '{"verdict":"plan","guidance":"ok"}' > "$TMP/r.json"
  bash "$SCRIPT" --phase record --issue 42 --repo acme/widget \
    --gate reviewer --response-file "$TMP/r.json" --json
  run jq -r '.use_count' "$TMP/telem/advisor-escalate.jsonl"
  [ "$output" = "2" ]
}
