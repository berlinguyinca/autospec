#!/usr/bin/env bats
# tests/record-telemetry.bats — TDD for skills/autospec-shared/scripts/record-telemetry.sh (issue #403)

SCRIPT="${BATS_TEST_DIRNAME}/../skills/autospec-shared/scripts/record-telemetry.sh"
FIXTURES_DIR="${BATS_TEST_DIRNAME}/fixtures/record-telemetry"

setup() {
  mkdir -p "$FIXTURES_DIR"
  TELEMETRY_FILE="$FIXTURES_DIR/telemetry.jsonl"
  export AUTOSPEC_TELEMETRY_FILE="$TELEMETRY_FILE"
}

teardown() {
  rm -rf "$FIXTURES_DIR"
}

_tokens_json() {
  local file="$FIXTURES_DIR/tokens-$1.json"
  printf '%s' "$2" > "$file"
  printf '%s' "$file"
}

@test "record-telemetry.sh is executable" {
  [ -x "$SCRIPT" ]
}

@test "record-telemetry.sh appends exactly one line per invocation" {
  tokens_file=$(_tokens_json "basic" '{"input_tokens":1000,"output_tokens":200,"cache_creation_input_tokens":800,"cache_read_input_tokens":0}')
  run "$SCRIPT" --dispatch-id "abc123" --role "implementer" --issue "403" --tokens-json "$tokens_file"
  [ "$status" -eq 0 ]
  [ "$(wc -l < "$TELEMETRY_FILE")" -eq 1 ]
}

@test "record-telemetry.sh appends second invocation as new line" {
  tokens_file=$(_tokens_json "two" '{"input_tokens":500,"output_tokens":100,"cache_creation_input_tokens":400,"cache_read_input_tokens":50}')
  "$SCRIPT" --dispatch-id "first"  --role "implementer" --issue "403" --tokens-json "$tokens_file"
  "$SCRIPT" --dispatch-id "second" --role "reviewer"    --issue "403" --tokens-json "$tokens_file"
  [ "$(wc -l < "$TELEMETRY_FILE")" -eq 2 ]
}

@test "record-telemetry.sh appended line is valid JSON" {
  tokens_file=$(_tokens_json "json-valid" '{"input_tokens":1000,"output_tokens":200,"cache_creation_input_tokens":800,"cache_read_input_tokens":0}')
  "$SCRIPT" --dispatch-id "abc123" --role "implementer" --issue "403" --tokens-json "$tokens_file"
  run jq -e . "$TELEMETRY_FILE"
  [ "$status" -eq 0 ]
}

@test "record-telemetry.sh schema includes required fields" {
  tokens_file=$(_tokens_json "schema" '{"input_tokens":1234,"output_tokens":56,"cache_creation_input_tokens":900,"cache_read_input_tokens":300}')
  "$SCRIPT" --dispatch-id "disp-xyz" --role "implementer" --issue "403" --tokens-json "$tokens_file"
  line=$(cat "$TELEMETRY_FILE")
  printf '%s\n' "$line" | jq -e '.ts'           >/dev/null
  printf '%s\n' "$line" | jq -e '.role'          >/dev/null
  printf '%s\n' "$line" | jq -e '.issue'         >/dev/null
  printf '%s\n' "$line" | jq -e '.input_tokens'  >/dev/null
  printf '%s\n' "$line" | jq -e '.output_tokens' >/dev/null
  printf '%s\n' "$line" | jq -e '.dispatch_id'   >/dev/null
  printf '%s\n' "$line" | jq -e '.cache_creation_input_tokens' >/dev/null
  printf '%s\n' "$line" | jq -e '.cache_read_input_tokens'     >/dev/null
}

@test "record-telemetry.sh stores correct values" {
  tokens_file=$(_tokens_json "values" '{"input_tokens":1234,"output_tokens":56,"cache_creation_input_tokens":900,"cache_read_input_tokens":300}')
  "$SCRIPT" --dispatch-id "disp-xyz" --role "implementer" --issue "403" --tokens-json "$tokens_file"
  line=$(cat "$TELEMETRY_FILE")
  [ "$(printf '%s\n' "$line" | jq -r '.dispatch_id')"                  = "disp-xyz" ]
  [ "$(printf '%s\n' "$line" | jq -r '.role')"                         = "implementer" ]
  [ "$(printf '%s\n' "$line" | jq -r '.issue')"                        = "403" ]
  [ "$(printf '%s\n' "$line" | jq    '.input_tokens')"                 = "1234" ]
  [ "$(printf '%s\n' "$line" | jq    '.cache_creation_input_tokens')"  = "900" ]
  [ "$(printf '%s\n' "$line" | jq    '.cache_read_input_tokens')"      = "300" ]
}

@test "record-telemetry.sh missing cache_read_input_tokens defaults to 0" {
  tokens_file=$(_tokens_json "missing-cache-read" '{"input_tokens":500,"output_tokens":100,"cache_creation_input_tokens":400}')
  "$SCRIPT" --dispatch-id "no-cache" --role "implementer" --issue "403" --tokens-json "$tokens_file"
  line=$(cat "$TELEMETRY_FILE")
  [ "$(printf '%s\n' "$line" | jq '.cache_read_input_tokens')" = "0" ]
}

@test "record-telemetry.sh missing cache_creation_input_tokens defaults to 0" {
  tokens_file=$(_tokens_json "missing-cache-creation" '{"input_tokens":500,"output_tokens":100,"cache_read_input_tokens":200}')
  "$SCRIPT" --dispatch-id "no-creation" --role "implementer" --issue "403" --tokens-json "$tokens_file"
  line=$(cat "$TELEMETRY_FILE")
  [ "$(printf '%s\n' "$line" | jq '.cache_creation_input_tokens')" = "0" ]
}

@test "record-telemetry.sh creates parent directory if missing" {
  deep_dir="$FIXTURES_DIR/deep/nested/dir"
  export AUTOSPEC_TELEMETRY_FILE="$deep_dir/telemetry.jsonl"
  tokens_file=$(_tokens_json "deep" '{"input_tokens":100,"output_tokens":10}')
  run "$SCRIPT" --dispatch-id "dir-create" --role "implementer" --issue "403" --tokens-json "$tokens_file"
  [ "$status" -eq 0 ]
  [ -f "$deep_dir/telemetry.jsonl" ]
}

@test "record-telemetry.sh stores profile_id when --profile-id given" {
  tokens_file=$(_tokens_json "profile" '{"input_tokens":500,"output_tokens":100,"cache_creation_input_tokens":0,"cache_read_input_tokens":0}')
  run "$SCRIPT" --dispatch-id "pid-test" --role "implementer" --issue "417" \
      --tokens-json "$tokens_file" --profile-id "claude-haiku-cloud"
  [ "$status" -eq 0 ]
  line="$(tail -1 "$TELEMETRY_FILE")"
  profile="$(printf '%s\n' "$line" | jq -r '.profile_id')"
  [ "$profile" = "claude-haiku-cloud" ]
}

@test "record-telemetry.sh profile_id defaults to empty string when omitted" {
  tokens_file=$(_tokens_json "no-profile" '{"input_tokens":100,"output_tokens":20}')
  run "$SCRIPT" --dispatch-id "no-pid" --role "implementer" --issue "417" \
      --tokens-json "$tokens_file"
  [ "$status" -eq 0 ]
  line="$(tail -1 "$TELEMETRY_FILE")"
  # profile_id key must exist and be empty string
  profile="$(printf '%s\n' "$line" | jq -r '.profile_id')"
  [ "$profile" = "" ]
}
