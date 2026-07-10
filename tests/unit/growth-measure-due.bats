#!/usr/bin/env bats
# tests/unit/growth-measure-due.bats
# Coverage for skills/autospec-shared/scripts/growth-measure-due.sh — the
# Tier G3 (run-growth-measure) cadence gate consumed by the conductor loop.

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
SCRIPT="$REPO_ROOT/skills/autospec-shared/scripts/growth-measure-due.sh"
LEDGER_SH="$REPO_ROOT/skills/autospec-shared/scripts/growth-ledger.sh"

setup() {
  TMP="$(mktemp -d)"
  mkdir -p "$TMP/.autospec"
  export GROWTH_LEDGER="$TMP/.autospec/growth/ledger.jsonl"
}

teardown() {
  rm -rf "$TMP"
  unset GROWTH_LEDGER
  unset GROWTH_NOW_EPOCH
}

measure_line() {
  # $1 = ts (ISO8601)
  echo "{\"round\":1,\"source\":\"conductor\",\"title\":\"measure-cycle\",\"norm_title\":\"measure-cycle\",\"channel\":\"measure\",\"kind\":\"measure\",\"issue\":0,\"outcome\":\"measured\",\"reason\":\"\",\"ts\":\"$1\"}"
}

@test "script exists and is bash -n clean" {
  [ -f "$SCRIPT" ]
  run bash -n "$SCRIPT"
  [ "$status" -eq 0 ]
}

@test "no ledger at all -> not due (0)" {
  run bash "$SCRIPT" "$TMP"
  [ "$status" -eq 0 ]
  [ "$output" = "0" ]
}

@test "ledger exists but has no measure line -> due (1, never measured)" {
  # Append a non-measure (artifact) line so the ledger exists but carries no
  # "measure" kind entry.
  bash "$LEDGER_SH" --append '{"round":1,"source":"keyword-gap","title":"t","norm_title":"t","channel":"seo","kind":"artifact","issue":7,"outcome":"pending","reason":"","ts":"2026-06-01T00:00:00Z"}'
  run bash "$SCRIPT" "$TMP"
  [ "$status" -eq 0 ]
  [ "$output" = "1" ]
}

@test "measure interval elapsed -> due (1)" {
  bash "$LEDGER_SH" --append "$(measure_line '2026-06-01T00:00:00Z')"
  # Default interval is 14 days = 1209600s. now = 2026-06-20 -> 19 days later.
  export GROWTH_NOW_EPOCH=1781913600  # 2026-06-20T00:00:00Z
  run bash "$SCRIPT" "$TMP"
  [ "$status" -eq 0 ]
  [ "$output" = "1" ]
}

@test "measure interval not yet elapsed -> not due (0)" {
  bash "$LEDGER_SH" --append "$(measure_line '2026-06-01T00:00:00Z')"
  # now = 2026-06-05 -> 4 days later, interval default 14 days.
  export GROWTH_NOW_EPOCH=1780617600  # 2026-06-05T00:00:00Z
  run bash "$SCRIPT" "$TMP"
  [ "$status" -eq 0 ]
  [ "$output" = "0" ]
}

@test "honors grow.measure_interval from .autospec/growth.yml when yq is available" {
  command -v yq >/dev/null 2>&1 || skip "yq not installed"
  cat > "$TMP/.autospec/growth.yml" <<'YAML'
grow:
  measure_interval: 3
YAML
  bash "$LEDGER_SH" --append "$(measure_line '2026-06-01T00:00:00Z')"
  # now = 2026-06-05 -> 4 days later, configured interval is 3 days -> due.
  export GROWTH_NOW_EPOCH=1780617600  # 2026-06-05T00:00:00Z
  run bash "$SCRIPT" "$TMP"
  [ "$status" -eq 0 ]
  [ "$output" = "1" ]
}

# Pin against the EXACT line skills/autospec-grow-run/SKILL.md R4 step 5 tells
# the agent to append (round:0, source:"measure", channel:"measurement",
# norm_title:"measure-<ts>"). The generic measure_line() helper above is a
# hand-built fixture; if the SKILL's documented JSON drifted (wrong key, wrong
# `kind`), this end-to-end test — and only this one — would catch it: the line
# must (a) satisfy growth-ledger.sh's 10-key REQUIRED schema on --append AND
# (b) be recognized by growth-measure-due.sh so the cadence clock resets.
@test "SKILL R4 documented measure line appends cleanly and resets cadence" {
  local ts="2026-06-01T00:00:00Z"
  # Built exactly as SKILL.md R4 step 5's jq documents it.
  local line
  line="$(jq -n --arg ts "$ts" --arg reason "all adapters degraded to {}" \
    '{round:0, source:"measure", title:"measure cycle",
      norm_title:("measure-" + $ts), channel:"measurement",
      kind:"measure", issue:0, outcome:"measured",
      reason:$reason, ts:$ts}')"
  # (a) canonical appender accepts it (10-key schema enforced, exit 0).
  run bash "$LEDGER_SH" --append "$line"
  [ "$status" -eq 0 ]
  # (b) within-interval -> the just-written line resets the cadence to not-due.
  export GROWTH_NOW_EPOCH=1780617600  # 2026-06-05T00:00:00Z, 4 days later
  run bash "$SCRIPT" "$TMP"
  [ "$status" -eq 0 ]
  [ "$output" = "0" ]
}

@test "malformed ledger -> fail-closed not due (0)" {
  mkdir -p "$(dirname "$GROWTH_LEDGER")"
  printf 'not json\n' > "$GROWTH_LEDGER"
  run bash "$SCRIPT" "$TMP"
  [ "$status" -eq 0 ]
  [ "$output" = "0" ]
}

@test "unreadable ledger -> fail-closed not due (0)" {
  bash "$LEDGER_SH" --append "$(measure_line '2026-06-01T00:00:00Z')"
  chmod 000 "$GROWTH_LEDGER"
  run bash "$SCRIPT" "$TMP"
  [ "$status" -eq 0 ]
  [ "$output" = "0" ]
  chmod 644 "$GROWTH_LEDGER"
}
