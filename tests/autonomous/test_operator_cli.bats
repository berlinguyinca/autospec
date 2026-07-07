#!/usr/bin/env bats

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  CLI="$REPO_ROOT/scripts/autospec-autonomous.sh"
  TEST_TMP="$(mktemp -d)"
  export HOME="$TEST_TMP/home"
  mkdir -p "$HOME"
  export CONDUCTOR_REPO="berlinguyinca/autospec"
  export AUTOSPEC_REPO_DIR="$REPO_ROOT"
}

teardown() {
  rm -rf "$TEST_TMP"
}

@test "operator cli: status emits machine-readable stopped state" {
  run bash "$CLI" status --json

  [ "$status" -eq 0 ]
  [[ "$output" == *'"running":false'* ]]
  [[ "$output" == *'"pid":""'* ]]
  [[ "$output" == *'autonomous/berlinguyinca_autospec/state.json'* ]]
}

@test "operator cli: logs reads recorded conductor log path" {
  mkdir -p "$HOME/.autospec/autonomous-operator" "$TEST_TMP/logs"
  printf '%s\n' "$TEST_TMP/logs/conductor.log" > "$HOME/.autospec/autonomous-operator/conductor.logpath"
  printf 'first\nsecond\n' > "$TEST_TMP/logs/conductor.log"

  run bash "$CLI" logs --lines 1

  [ "$status" -eq 0 ]
  [ "$output" = "second" ]
}

@test "operator cli: timeline summarizes conductor log in plain English" {
  mkdir -p "$HOME/.autospec/autonomous-operator" "$TEST_TMP/logs"
  printf '%s\n' "$TEST_TMP/logs/conductor.log" > "$HOME/.autospec/autonomous-operator/conductor.logpath"
  cat > "$TEST_TMP/logs/conductor.log" <<'EOF'
{
  "updated_at": "2026-07-07T09:45:04Z"
}
Hook audit addressed.
Changed:
- `scripts/check-doc-drift.sh`
- `skills/autospec-shared/scripts/check-doc-drift.sh`
Verified:
- `bash -n scripts/check-doc-drift.sh`
- `bats skills/autospec-shared/tests/unit/check-doc-drift.bats --filter 'docs: skip|partial match'`
[conductor] cycle 2 starting
HEARTBEAT_AT:1783417565
[conductor] tier=1 action=run-backlog
[conductor] main-health pending — skipping drain this cycle
{
  "updated_at": "2026-07-07T09:46:13Z"
}
[conductor] cycle 3 starting
HEARTBEAT_AT:1783417633
[conductor] tier=1 action=run-backlog
workdir: /tmp/autospec
user
$autospec-run
EOF

  run bash "$CLI" timeline --lines 80

  [ "$status" -eq 0 ]
  [[ "$output" == *"autospec-autonomous timeline"* ]]
  [[ "$output" == *"addressed a hook audit finding."* ]]
  [[ "$output" == *"updated scripts/check-doc-drift.sh and skills/autospec-shared/scripts/check-doc-drift.sh."* ]]
  [[ "$output" == *"verified 2 checks:"* ]]
  [[ "$output" == *"started autonomous cycle 2."* ]]
  [[ "$output" == *"skipped the backlog drain because main health was still pending."* ]]
  [[ "$output" == *"started autospec-run in /tmp/autospec."* ]]
}

@test "operator cli: timeline includes remaining work estimates and planned steps" {
  mkdir -p "$HOME/.autospec/autonomous-operator" "$TEST_TMP/logs"
  printf '%s\n' "$TEST_TMP/logs/conductor.log" > "$HOME/.autospec/autonomous-operator/conductor.logpath"
  cat > "$TEST_TMP/logs/conductor.log" <<'EOF'
{
  "updated_at": "2026-07-07T16:17:25Z"
}
[conductor] cycle 4 starting
{
  "ready": [
    {"number": 1538, "title": "feat: autonomous UX/UI optimization tier"},
    {"number": 1539, "title": "feat: autonomous accessibility standards tier"}
  ],
  "blocked": [
    {"number": 1540, "title": "feat: documentation freshness tier"}
  ],
  "claimed": [
    {"number": 1537, "title": "feat: proactive security scanning workstream"}
  ],
  "batch": [
    {"number": 1538, "title": "feat: autonomous UX/UI optimization tier"}
  ]
}
EOF

  run bash "$CLI" timeline --lines 80

  [ "$status" -eq 0 ]
  [[ "$output" == *"autospec-autonomous forecast"* ]]
  [[ "$output" == *"things left: 4 total (2 ready, 1 in progress, 1 blocked)"* ]]
  [[ "$output" == *"rough ETA: about 3-6 hours"* ]]
  [[ "$output" == *"planned next: finish #1537 feat: proactive security scanning workstream"* ]]
  [[ "$output" == *"then start #1538 feat: autonomous UX/UI optimization tier"* ]]
  [[ "$output" == *"blocked later: #1540 feat: documentation freshness tier"* ]]
}

@test "operator cli: status finds spend ledger with alternate .git slug" {
  mkdir -p "$HOME/.autospec/autonomous-spend/berlinguyinca_autospec.git"
  printf '{"issues":7,"tokens":0}\n' > "$HOME/.autospec/autonomous-spend/berlinguyinca_autospec.git/spend.json"

  run bash "$CLI" status --json

  [ "$status" -eq 0 ]
  [[ "$output" == *'berlinguyinca_autospec.git/spend.json'* ]]
  [[ "$output" == *'"issues":"7"'* ]]
  [[ "$output" == *'"tokens":"0"'* ]]
}

@test "operator cli: stop delegates to autospec-stop helper" {
  mkdir -p "$TEST_TMP/scripts"
  cat > "$TEST_TMP/scripts/autospec-stop.sh" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" > "$HOME/stop.args"
EOF
  chmod +x "$TEST_TMP/scripts/autospec-stop.sh"

  AUTOSPEC_SCRIPTS_DIR="$TEST_TMP/scripts" run bash "$CLI" stop --immediate

  [ "$status" -eq 0 ]
  [ "$(cat "$HOME/stop.args")" = "--immediate" ]
}
