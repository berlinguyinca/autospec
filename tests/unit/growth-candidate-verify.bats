#!/usr/bin/env bats

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
VF="$REPO_ROOT/skills/autospec-shared/scripts/growth-candidate-verify.sh"
LG="$REPO_ROOT/skills/autospec-shared/scripts/growth-ledger.sh"

setup() { TMP="$(mktemp -d)"; export GROWTH_LEDGER="$TMP/ledger.jsonl"; }
teardown() { rm -rf "$TMP"; unset GROWTH_LEDGER; }

cand() { cat > "$TMP/c.json" <<'JSON'
{"lens":"community","channel":"outreach","kind":"outbound","title":"post","norm_title":"post","roi":3,"effort":"small","severity":3,"confidence":0.6}
JSON
echo "$TMP/c.json"; }

@test "script exists and is bash -n clean" {
  [ -f "$VF" ]; run bash -n "$VF"; [ "$status" -eq 0 ]
}

@test "real:true emits the candidate and writes no ledger line" {
  echo '{"real":true,"reason":"ok"}' > "$TMP/v.json"
  run bash "$VF" "$(cand)" "$TMP/v.json"
  [ "$status" -eq 0 ]
  [[ "$output" == *'"norm_title":"post"'* ]]
  [ ! -f "$GROWTH_LEDGER" ] || [ "$(wc -l < "$GROWTH_LEDGER" | tr -d ' ')" -eq 0 ]
}

@test "real:false emits nothing and writes exactly one refuted line" {
  echo '{"real":false,"reason":"off-topic"}' > "$TMP/v.json"
  run bash "$VF" "$(cand)" "$TMP/v.json"
  [ "$status" -eq 0 ]
  [ -z "$output" ]
  [ "$(wc -l < "$GROWTH_LEDGER" | tr -d ' ')" -eq 1 ]
  run bash "$LG" --show --json
  [ "$(printf '%s\n' "$output" | jq -r '.[0].outcome')" = "refuted" ]
  [[ "$output" == *'off-topic'* ]]
}

@test "unparseable verdict fails closed (refuted)" {
  printf 'not json' > "$TMP/v.json"
  run bash "$VF" "$(cand)" "$TMP/v.json"
  [ "$status" -eq 0 ]
  [ -z "$output" ]
  [ "$(wc -l < "$GROWTH_LEDGER" | tr -d ' ')" -eq 1 ]
  run bash "$LG" --show --json
  [[ "$output" == *'refuted'* ]]
  [[ "$output" == *'unparseable verdict'* ]]
}

@test "non-boolean real fails closed (refuted)" {
  echo '{"real":"yes"}' > "$TMP/v.json"
  run bash "$VF" "$(cand)" "$TMP/v.json"
  [ "$status" -eq 0 ]
  [ -z "$output" ]
  [ "$(wc -l < "$GROWTH_LEDGER" | tr -d ' ')" -eq 1 ]
}
