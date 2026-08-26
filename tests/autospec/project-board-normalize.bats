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
