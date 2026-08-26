#!/usr/bin/env bats
# Coverage for scripts/project-board-normalize.sh — label taxonomy → normalized attributes.

setup() {
  TMP="$(mktemp -d)"
  SCRIPT="${BATS_TEST_DIRNAME}/../../scripts/project-board-normalize.sh"
}
teardown() { rm -rf "$TMP"; }

plan() { printf '{"items":[{"number":1,"labels":%s}]}' "$1"; }

@test "fallback regex normalizes the colon taxonomy (project 1)" {
  printf '%s' "$(plan '["priority:p0","ctx:64k","reasoning:deep","area:security"]')" > "$TMP/in.json"
  run bash "$SCRIPT" < "$TMP/in.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.items[0].normalized.priority == "critical"'
  echo "$output" | jq -e '.items[0].normalized.ctx == "64k"'
  echo "$output" | jq -e '.items[0].normalized.area == "security"'
}

@test "fallback regex normalizes the slash taxonomy (project 2)" {
  printf '%s' "$(plan '["priority/critical","ctx:32k","reasoning/medium","area/security"]')" > "$TMP/in.json"
  run bash "$SCRIPT" < "$TMP/in.json"
  echo "$output" | jq -e '.items[0].normalized.priority == "critical"'
  echo "$output" | jq -e '.items[0].normalized.reasoning == "medium"'
  echo "$output" | jq -e '.items[0].normalized.area == "security"'
}

@test "both taxonomies land on the same normalized priority" {
  printf '%s' "$(plan '["priority:p0"]')" > "$TMP/a.json"
  printf '%s' "$(plan '["priority/critical"]')" > "$TMP/b.json"
  a="$(bash "$SCRIPT" < "$TMP/a.json" | jq -r '.items[0].normalized.priority')"
  b="$(bash "$SCRIPT" < "$TMP/b.json" | jq -r '.items[0].normalized.priority')"
  [ "$a" = "$b" ]
}

@test "an explicit label_map overrides the fallback" {
  printf '%s' "$(plan '["priority:weird"]')" > "$TMP/in.json"
  cat > "$TMP/map.yml" <<'YML'
priority:
  weird: low
YML
  run bash "$SCRIPT" --label-map "$TMP/map.yml" < "$TMP/in.json"
  echo "$output" | jq -e '.items[0].normalized.priority == "low"'
}

@test "an unknown label yields null, never a crash" {
  printf '%s' "$(plan '["totally-unrelated"]')" > "$TMP/in.json"
  run bash "$SCRIPT" < "$TMP/in.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.items[0].normalized.priority == null'
}

@test "a label containing regex metacharacters does not inject into matching" {
  printf '%s' "$(plan '["priority:.*"]')" > "$TMP/in.json"
  run bash "$SCRIPT" < "$TMP/in.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.items[0].normalized.priority == null'
}

@test "an unreadable label map falls back instead of failing" {
  printf '%s' "$(plan '["priority:p0"]')" > "$TMP/in.json"
  run bash "$SCRIPT" --label-map "$TMP/does-not-exist.yml" < "$TMP/in.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.items[0].normalized.priority == "critical"'
}

@test "stdin is not JSON at all: exit 0 with no output" {
  printf '%s' "this is not json at all" > "$TMP/in.txt"
  run bash "$SCRIPT" < "$TMP/in.txt"
  [ "$status" -eq 0 ]
  [ -z "$output" ]
}

@test "JSON stdin with no .items key: exit 0 and pass through" {
  printf '%s' '{"foo":"bar"}' > "$TMP/in.json"
  run bash "$SCRIPT" < "$TMP/in.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.foo == "bar"'
  echo "$output" | jq -e 'has("items") | not'
}

@test "an item with no labels key: exit 0 and add normalized with nulls" {
  printf '%s' '{"items":[{"number":1}]}' > "$TMP/in.json"
  run bash "$SCRIPT" < "$TMP/in.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.items[0].normalized.priority == null'
  echo "$output" | jq -e '.items[0].normalized.ctx == null'
}

@test "an item with labels: null: exit 0 and add normalized with nulls" {
  printf '%s' '{"items":[{"number":1,"labels":null}]}' > "$TMP/in.json"
  run bash "$SCRIPT" < "$TMP/in.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.items[0].normalized.priority == null'
  echo "$output" | jq -e '.items[0].normalized.area == null'
}

@test "an item with labels containing non-string elements: exit 0 and skip them" {
  printf '%s' '{"items":[{"number":1,"labels":["priority:p0",null,123,"area:security"]}]}' > "$TMP/in.json"
  run bash "$SCRIPT" < "$TMP/in.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.items[0].normalized.priority == "critical"'
  echo "$output" | jq -e '.items[0].normalized.area == "security"'
}

@test "label-map valid YAML but wrong shape (list): degrade to fallback" {
  printf '%s' "$(plan '["priority:p0"]')" > "$TMP/in.json"
  printf '%s' '- item1\n- item2\n' > "$TMP/map.yml"
  run bash "$SCRIPT" --label-map "$TMP/map.yml" < "$TMP/in.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.items[0].normalized.priority == "critical"'
}

@test "label-map valid YAML but wrong shape (scalar): degrade to fallback" {
  printf '%s' "$(plan '["priority:p0"]')" > "$TMP/in.json"
  printf '%s' 'just a string' > "$TMP/map.yml"
  run bash "$SCRIPT" --label-map "$TMP/map.yml" < "$TMP/in.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.items[0].normalized.priority == "critical"'
}
