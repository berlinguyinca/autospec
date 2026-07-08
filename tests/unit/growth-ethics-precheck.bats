#!/usr/bin/env bats

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
PC="$REPO_ROOT/skills/autospec-shared/scripts/growth-ethics-precheck.sh"

setup() { TMP="$(mktemp -d)"; export GROWTH_NOW_EPOCH=1000000; }
teardown() { rm -rf "$TMP"; unset GROWTH_NOW_EPOCH; }

@test "script exists and is bash -n clean" {
  [ -f "$PC" ]; run bash -n "$PC"; [ "$status" -eq 0 ]
}

@test "disclosure: plain non-sponsored draft passes" {
  printf 'Check out our open-source CLI, it is fast.\n' > "$TMP/d.md"
  run bash "$PC" --disclosure "$TMP/d.md"
  [ "$status" -eq 0 ]
}

@test "disclosure: sponsored draft WITHOUT marker fails" {
  printf 'This sponsored post highlights our tool.\n' > "$TMP/d.md"
  run bash "$PC" --disclosure "$TMP/d.md"
  [ "$status" -ne 0 ]
}

@test "disclosure: sponsored draft WITH marker passes" {
  printf 'This sponsored post highlights our tool.\n#ad\n' > "$TMP/d.md"
  run bash "$PC" --disclosure "$TMP/d.md"
  [ "$status" -eq 0 ]
}

@test "cadence: under cap passes" {
  echo '{"approval":{"cadence_caps":{"default_per_platform_per_week":2}},"targets":{"communities":[]}}' > "$TMP/c.json"
  # one published line, 1 day ago (epoch 1000000 - 86400)
  echo '{"platform":"reddit","outcome":"published","ts":913600}' > "$TMP/l.jsonl"
  run bash "$PC" --cadence "$TMP/c.json" "$TMP/l.jsonl" reddit
  [ "$status" -eq 0 ]
}

@test "cadence: at cap fails" {
  echo '{"approval":{"cadence_caps":{"default_per_platform_per_week":2}},"targets":{"communities":[]}}' > "$TMP/c.json"
  printf '{"platform":"reddit","outcome":"published","ts":990000}\n{"platform":"reddit","outcome":"published","ts":990001}\n' > "$TMP/l.jsonl"
  run bash "$PC" --cadence "$TMP/c.json" "$TMP/l.jsonl" reddit
  [ "$status" -ne 0 ]
}

@test "cadence: old publishes outside 7-day window do not count" {
  echo '{"approval":{"cadence_caps":{"default_per_platform_per_week":1}},"targets":{"communities":[]}}' > "$TMP/c.json"
  # published 8 days ago: 1000000 - 8*86400 = 308800
  echo '{"platform":"reddit","outcome":"published","ts":308800}' > "$TMP/l.jsonl"
  run bash "$PC" --cadence "$TMP/c.json" "$TMP/l.jsonl" reddit
  [ "$status" -eq 0 ]
}

@test "disclosure: sponsored draft with only a substring-like #adventure marker fails" {
  printf 'This sponsored post. Check #adventure for more.\n' > "$TMP/d.md"
  run bash "$PC" --disclosure "$TMP/d.md"
  [ "$status" -ne 0 ]
}

@test "disclosure: sponsored draft with a genuine standalone #ad token passes" {
  printf 'This sponsored post.\n#ad\n' > "$TMP/d.md"
  run bash "$PC" --disclosure "$TMP/d.md"
  [ "$status" -eq 0 ]
}

@test "cadence: community-specific cap overrides higher default cap" {
  echo '{"approval":{"cadence_caps":{"default_per_platform_per_week":5}},"targets":{"communities":[{"platform":"reddit","cadence_cap_per_week":1}]}}' > "$TMP/c.json"
  echo '{"platform":"reddit","outcome":"published","ts":990000}' > "$TMP/l.jsonl"
  run bash "$PC" --cadence "$TMP/c.json" "$TMP/l.jsonl" reddit
  [ "$status" -ne 0 ]
}

@test "cadence: malformed ledger fails closed" {
  echo '{"approval":{"cadence_caps":{"default_per_platform_per_week":2}},"targets":{"communities":[]}}' > "$TMP/c.json"
  printf 'not valid json\n' > "$TMP/l.jsonl"
  run bash "$PC" --cadence "$TMP/c.json" "$TMP/l.jsonl" reddit
  [ "$status" -ne 0 ]
}

@test "cadence interop: real ledger appended via growth-ledger.sh --append trips numeric-ts cap" {
  echo '{"approval":{"cadence_caps":{"default_per_platform_per_week":1}},"targets":{"communities":[]}}' > "$TMP/c.json"
  local ledger="$TMP/l.jsonl"
  GROWTH_LEDGER="$ledger" bash "$REPO_ROOT/skills/autospec-shared/scripts/growth-ledger.sh" --append \
    '{"round":1,"source":"community","title":"t1","norm_title":"t1","channel":"reddit","kind":"outbound","issue":11,"outcome":"published","reason":"","ts":990000,"platform":"reddit"}'
  GROWTH_LEDGER="$ledger" bash "$REPO_ROOT/skills/autospec-shared/scripts/growth-ledger.sh" --append \
    '{"round":1,"source":"community","title":"t2","norm_title":"t2","channel":"reddit","kind":"outbound","issue":12,"outcome":"published","reason":"","ts":990001,"platform":"reddit"}'
  run bash "$PC" --cadence "$TMP/c.json" "$ledger" reddit
  [ "$status" -ne 0 ]
}

@test "cadence interop: real ledger with ISO-8601 ts trips cap" {
  echo '{"approval":{"cadence_caps":{"default_per_platform_per_week":1}},"targets":{"communities":[]}}' > "$TMP/c.json"
  local ledger="$TMP/l.jsonl"
  local iso
  iso="$(date -u -r 990000 +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || date -u -d @990000 +%Y-%m-%dT%H:%M:%SZ)"
  GROWTH_LEDGER="$ledger" bash "$REPO_ROOT/skills/autospec-shared/scripts/growth-ledger.sh" --append \
    "{\"round\":1,\"source\":\"community\",\"title\":\"t1\",\"norm_title\":\"t1\",\"channel\":\"reddit\",\"kind\":\"outbound\",\"issue\":11,\"outcome\":\"published\",\"reason\":\"\",\"ts\":\"$iso\",\"platform\":\"reddit\"}"
  run bash "$PC" --cadence "$TMP/c.json" "$ledger" reddit
  [ "$status" -ne 0 ]
}

@test "cadence: unparseable ledger ts fails closed" {
  echo '{"approval":{"cadence_caps":{"default_per_platform_per_week":2}},"targets":{"communities":[]}}' > "$TMP/c.json"
  echo '{"platform":"reddit","outcome":"published","ts":"garbage-not-a-date"}' > "$TMP/l.jsonl"
  run bash "$PC" --cadence "$TMP/c.json" "$TMP/l.jsonl" reddit
  [ "$status" -ne 0 ]
}
