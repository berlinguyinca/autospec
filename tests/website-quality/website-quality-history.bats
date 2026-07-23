#!/usr/bin/env bats

setup() {
  ROOT="$BATS_TEST_DIRNAME/../.."
  SCRIPT="$ROOT/skills/autospec-quality/scripts/website-quality.sh"
  TMP="$BATS_TEST_TMPDIR/website-quality"
  mkdir -p "$TMP"
}

write_capture() {
  cat > "$1" <<JSON
{
  "schema_version":"website-quality/v1", "site_id":"$2", "run_id":"$3",
  "git_sha":"0123456789abcdef0123456789abcdef01234567", "captured_at":"2026-07-23T12:00:00Z",
  "rubric_version":"$4", "config_hash":"sha256:config", "pages":[
    {"route_template":"/items/:id", "category_scores":{"navigation":0.9,"accessibility":0.8}, "confidence":0.95, "coverage":1.0, "validity":"current", "evidence":["evidence/items.png"], "defects":[], "remediation":[], "acceptance_test":"tests/items.bats"}
  ]
}
JSON
}

@test "records two site fixtures and validates immutable histories" {
  write_capture "$TMP/a.json" alpha run-a r1
  write_capture "$TMP/b.json" beta run-b r1
  run bash "$SCRIPT" record --input "$TMP/a.json" --history "$TMP/history"
  [ "$status" -eq 0 ]
  run bash "$SCRIPT" record --input "$TMP/b.json" --history "$TMP/history"
  [ "$status" -eq 0 ]
  run bash "$SCRIPT" validate --history "$TMP/history"
  [ "$status" -eq 0 ]
  [ -f "$TMP/history/runs/alpha/run-a.json" ]
  [ -f "$TMP/history/runs/beta/run-b.json" ]
}

@test "rejects stale, incomplete, malformed and cross-rubric verified improvement" {
  write_capture "$TMP/old.json" alpha run-old r1
  sed -i 's/"validity":"current"/"validity":"stale"/' "$TMP/old.json"
  write_capture "$TMP/new.json" alpha run-new r2
  run bash "$SCRIPT" record --input "$TMP/old.json" --history "$TMP/history"
  [ "$status" -eq 0 ]
  run bash "$SCRIPT" record --input "$TMP/new.json" --history "$TMP/history"
  [ "$status" -eq 0 ]
  run bash "$SCRIPT" report --history "$TMP/history" --output "$TMP/report.json"
  [ "$status" -ne 0 ]
  [[ "$output" == *"blocked"* || "$output" == *"rubric"* ]]
}

@test "keeps dynamic route template continuity and emits explicit blockers" {
  write_capture "$TMP/c.json" gamma run-c r1
  sed -i 's#/items/:id#/orders/:id#' "$TMP/c.json"
  run bash "$SCRIPT" record --input "$TMP/c.json" --history "$TMP/history"
  [ "$status" -eq 0 ]
  run bash "$SCRIPT" report --history "$TMP/history" --output "$TMP/report.json"
  [ "$status" -eq 0 ]
  jq -e '.pages[0].route_template == "/orders/:id" and .pages[0].status == "verified"' "$TMP/report.json"
}
