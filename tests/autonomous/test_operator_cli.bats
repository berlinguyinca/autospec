#!/usr/bin/env bats

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  CLI="$REPO_ROOT/scripts/autospec-autonomous.sh"
  TEST_TMP="$(mktemp -d)"
  export HOME="$TEST_TMP/home"
  mkdir -p "$HOME"
  export CONDUCTOR_REPO="berlinguyinca/autospec"
  export AUTOSPEC_REPO_DIR="$REPO_ROOT"
  # These cases exercise logs/timeline/monitor parsing, not per-repo operator
  # scoping (covered in tests/autospec/test_autonomous.bats). Pin the operator
  # dir so recorded PID/logpath live where the fixtures write them (#1577).
  export AUTOSPEC_AUTONOMOUS_OPERATOR_DIR="$HOME/.autospec/autonomous-operator"
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

@test "operator cli: timeline forecast uses latest coordinator state outside selected event window" {
  mkdir -p "$HOME/.autospec/autonomous-operator" "$TEST_TMP/logs"
  printf '%s\n' "$TEST_TMP/logs/conductor.log" > "$HOME/.autospec/autonomous-operator/conductor.logpath"
  cat > "$TEST_TMP/logs/conductor.log" <<'EOF'
{
  "ready": [
    {"number": 1538, "title": "feat: autonomous UX/UI optimization tier"}
  ],
  "blocked": [],
  "claimed": [
    {"number": 1537, "title": "feat: proactive security scanning workstream"}
  ],
  "batch": [
    {"number": 1538, "title": "feat: autonomous UX/UI optimization tier"}
  ]
}
2026-07-07T17:30:19Z
{"issue":"1538","branch":"feat/issue-1538-ux-ui-optimization-tier","step":"tests_started"}
2026-07-07T17:31:19Z
{"issue":"1538","branch":"feat/issue-1538-ux-ui-optimization-tier","step":"tests_started"}
2026-07-07T17:32:20Z
{"issue":"1538","branch":"feat/issue-1538-ux-ui-optimization-tier","step":"tests_started"}
EOF

  run bash "$CLI" timeline --lines 2

  [ "$status" -eq 0 ]
  [[ "$output" == *"autospec-autonomous forecast"* ]]
  [[ "$output" == *"things left: 2 total (1 ready, 1 in progress, 0 blocked)"* ]]
}

@test "operator cli: timeline forecast ignores scalar claimed fields after coordinator state" {
  mkdir -p "$HOME/.autospec/autonomous-operator" "$TEST_TMP/logs"
  printf '%s\n' "$TEST_TMP/logs/conductor.log" > "$HOME/.autospec/autonomous-operator/conductor.logpath"
  cat > "$TEST_TMP/logs/conductor.log" <<'EOF'
{
  "ready": [
    {"number": 1538, "title": "feat: autonomous UX/UI optimization tier"}
  ],
  "blocked": [],
  "claimed": [],
  "batch": [
    {"number": 1538, "title": "feat: autonomous UX/UI optimization tier"}
  ]
}
{"claimed": false}
EOF

  run bash "$CLI" timeline --lines 80

  [ "$status" -eq 0 ]
  [[ "$output" == *"autospec-autonomous forecast"* ]]
  [[ "$output" == *"things left: 1 total (1 ready, 0 in progress, 0 blocked)"* ]]
  [ "$(printf '%s\n' "$output" | grep -c "#1538 feat: autonomous UX/UI optimization tier")" -eq 1 ]
}

@test "operator cli: timeline reports item durations and next-start estimate" {
  mkdir -p "$HOME/.autospec/autonomous-operator" "$TEST_TMP/logs"
  printf '%s\n' "$TEST_TMP/logs/conductor.log" > "$HOME/.autospec/autonomous-operator/conductor.logpath"
  cat > "$TEST_TMP/logs/conductor.log" <<'EOF'
{
  "ready": [
    {"number": 1540, "title": "feat: documentation freshness tier"},
    {"number": 1541, "title": "feat: RAG documentation database tier"}
  ],
  "blocked": [],
  "claimed": [
    {"number": 1539, "title": "feat: accessibility standards tier"}
  ],
  "batch": [
    {"number": 1540, "title": "feat: documentation freshness tier"}
  ]
}
{"issue":"1539","branch":"feat/issue-1539-accessibility","step":"claimed","ts":1783440000,"repo":"berlinguyinca/autospec"}
{"issue":"1539","branch":"feat/issue-1539-accessibility","step":"tests_started","ts":1783441800,"repo":"berlinguyinca/autospec"}
{"issue":"1538","branch":"feat/issue-1538-ux","step":"claimed","ts":1783430000,"repo":"berlinguyinca/autospec"}
{"issue":"1538","branch":"feat/issue-1538-ux","step":"merged","ts":1783437200,"repo":"berlinguyinca/autospec"}
EOF

  run bash "$CLI" timeline --lines 80

  [ "$status" -eq 0 ]
  [[ "$output" == *"item timing"* ]]
  [[ "$output" == *"#1539 current step tests started after 30 minutes"* ]]
  [[ "$output" == *"#1538 completed in 2 hours"* ]]
  [[ "$output" == *"next item start estimate: after current item finishes, roughly 15-45 minutes of handoff overhead"* ]]
}

@test "operator cli: timeline reconciles heartbeat-active issue into forecast progress" {
  mkdir -p "$HOME/.autospec/autonomous-operator" "$TEST_TMP/logs"
  printf '%s\n' "$TEST_TMP/logs/conductor.log" > "$HOME/.autospec/autonomous-operator/conductor.logpath"
  cat > "$TEST_TMP/logs/conductor.log" <<'EOF'
{
  "ready": [
    {"number": 1543, "title": "feat: autonomy guardrails"},
    {"number": 1544, "title": "feat: immutable verifier"}
  ],
  "blocked": [],
  "claimed": [],
  "batch": [
    {"number": 1543, "title": "feat: autonomy guardrails"}
  ]
}
EOF
  mkdir -p "$HOME/.autospec/process-heartbeats/berlinguyinca__autospec"
  cat > "$HOME/.autospec/process-heartbeats/berlinguyinca__autospec/1543.json" <<'EOF'
{"issue":"1543","branch":"feat/issue-1543-autonomy-guardrails","step":"claimed","ts":1783466193,"repo":"berlinguyinca/autospec"}
EOF

  run bash "$CLI" timeline --lines 80

  [ "$status" -eq 0 ]
  [[ "$output" == *"things left: 2 total (1 ready, 1 in progress, 0 blocked)"* ]]
  [[ "$output" == *"planned next: finish #1543 feat: autonomy guardrails"* ]]
  [[ "$output" == *"then start #1544 feat: immutable verifier"* ]]
}

@test "operator cli: monitor prints timeline report at requested interval" {
  mkdir -p "$HOME/.autospec/autonomous-operator" "$TEST_TMP/logs"
  printf '%s\n' "$TEST_TMP/logs/conductor.log" > "$HOME/.autospec/autonomous-operator/conductor.logpath"
  cat > "$TEST_TMP/logs/conductor.log" <<'EOF'
{
  "ready": [
    {"number": 1540, "title": "feat: documentation freshness tier"}
  ],
  "blocked": [],
  "claimed": [],
  "batch": [
    {"number": 1540, "title": "feat: documentation freshness tier"}
  ]
}
EOF

  run bash "$CLI" monitor --interval-sec 0 --iterations 2 --lines 80

  [ "$status" -eq 0 ]
  [ "$(printf '%s\n' "$output" | grep -c "autospec-autonomous monitor")" -eq 2 ]
  [[ "$output" == *"things left: 1 total"* ]]
  [[ "$output" == *"planned next: start #1540 feat: documentation freshness tier"* ]]
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
