#!/usr/bin/env bats

setup_file() {
  local repo_root
  repo_root="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  cargo build --quiet --manifest-path "$repo_root/Cargo.toml" -p autospec-cli --bin autospec
}

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  CLI="$REPO_ROOT/scripts/autospec-autonomous.sh"
  RUST_CLI="$REPO_ROOT/target/debug/autospec"
  TEST_TMP="$(mktemp -d)"
  export HOME="$TEST_TMP/home"
  mkdir -p "$HOME"
  export CONDUCTOR_REPO="berlinguyinca/autospec"
  export AUTOSPEC_REPO_DIR="$REPO_ROOT"
  unset AUTOSPEC_STOP_FLAG_FILE
}

teardown() {
  rm -rf "$TEST_TMP"
}

@test "operator cli: typed blast-radius route preserves the command token" {
  local changed_files="$TEST_TMP/changed-files.txt"
  printf '%s\n' 'docs/README.md' > "$changed_files"

  run "$RUST_CLI" autonomous blast-radius --changed-files "$changed_files" --json

  [ "$status" -eq 0 ]
  [[ "$output" == *'"fenced":false'* ]]
  [[ "$output" == *'"paths":["docs/README.md"]'* ]]
}

@test "operator cli: typed lifecycle route preserves the decide action" {
  run "$RUST_CLI" autonomous lifecycle decide \
    --repo berlinguyinca/autospec --ready-tier 1.5

  [ "$status" -eq 0 ]
  [ "$output" = '{"decision":"run","tier":"1.5"}' ]
}

@test "operator cli: default foreground drain dispatches the typed autonomous command" {
  local scripts_dir="$TEST_TMP/scripts"
  local bin_dir="$TEST_TMP/bin"
  local args_file="$TEST_TMP/autospec-args"
  mkdir -p "$scripts_dir/lib" "$bin_dir"
  cp "$CLI" "$scripts_dir/autospec-autonomous.sh"
  cp "$REPO_ROOT/scripts/lib/autospec-status-accountability.sh" "$scripts_dir/lib/"
  chmod +x "$scripts_dir/autospec-autonomous.sh"
  cat > "$scripts_dir/lib/autospec-loop.sh" <<'EOF'
autospec_conductor_run() {
  bash -c "$AUTOSPEC_RUN_CMD"
}
EOF
  cat > "$bin_dir/autospec" <<'EOF'
#!/bin/sh
printf '%s\n' "$@" > "$AUTOSPEC_TEST_ARGS_FILE"
EOF
  chmod +x "$bin_dir/autospec"

  unset AUTOSPEC_RUN_CMD
  AUTOSPEC_TEST_ARGS_FILE="$args_file" PATH="$bin_dir:$PATH" run \
    bash "$scripts_dir/autospec-autonomous.sh" run-foreground \
      --repo berlinguyinca/autospec --repo-dir "$REPO_ROOT"

  [ "$status" -eq 0 ]
  [ "$(sed -n '1p' "$args_file")" = "autonomous" ]
  [ "$(sed -n '2p' "$args_file")" = "drain" ]
  [ "$(sed -n '3p' "$args_file")" = "--repo" ]
  [ "$(sed -n '4p' "$args_file")" = "berlinguyinca/autospec" ]
  [ "$(sed -n '5p' "$args_file")" = "--repo-dir" ]
  [ "$(sed -n '6p' "$args_file")" = "$REPO_ROOT" ]
}

@test "operator cli: status emits machine-readable stopped state" {
  run bash "$CLI" status --json

  [ "$status" -eq 0 ]
  [[ "$output" == *'"running":false'* ]]
  [[ "$output" == *'"pid":""'* ]]
  [[ "$output" == *'autonomous/berlinguyinca_autospec/state.json'* ]]
}


@test "operator cli: status reports terminal conductor state from state.json" {
  mkdir -p "$HOME/.autospec/autonomous/berlinguyinca_autospec"
  cat > "$HOME/.autospec/autonomous/berlinguyinca_autospec/state.json" <<'EOF_STATE'
{"status":"stopped:signal:TERM:cycle-160","heartbeat_at":1783417633}
EOF_STATE

  run bash "$CLI" status --json

  [ "$status" -eq 0 ]
  [[ "$output" == *'"state_status":"stopped:signal:TERM:cycle-160"'* ]]
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

@test "operator cli: supervise prints one-line observer status" {
  mkdir -p "$HOME/.autospec/autonomous-operator/berlinguyinca_autospec"
  printf '999999\n' > "$HOME/.autospec/autonomous-operator/berlinguyinca_autospec/conductor.pid"
  touch "$HOME/.autospec/autonomous-operator/berlinguyinca_autospec/stop.flag"

  run bash "$CLI" supervise --interval-sec 0 --iterations 1 --repo berlinguyinca/autospec

  [ "$status" -eq 0 ]
  [[ "$output" == "autospec-supervise: ok repo=berlinguyinca/autospec conductor=stopped pid=999999 action=none" ]]
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


@test "operator cli: repo B start ignores repo A live conductor metadata" {
  sleep 30 &
  repo_a_pid="$!"
  trap 'kill "$repo_a_pid" 2>/dev/null || true' RETURN
  mkdir -p "$HOME/.autospec/autonomous-operator/berlinguyinca_autospec"
  printf '%s\n' "$repo_a_pid" > "$HOME/.autospec/autonomous-operator/berlinguyinca_autospec/conductor.pid"
  printf '%s\n' "$TEST_TMP/repo-a.log" > "$HOME/.autospec/autonomous-operator/berlinguyinca_autospec/conductor.logpath"

  mkdir -p "$TEST_TMP/bin"
  cat > "$TEST_TMP/bin/python3" <<'PY_FAKE'
#!/usr/bin/env bash
printf '424242\n'
PY_FAKE
  chmod +x "$TEST_TMP/bin/python3"

  PATH="$TEST_TMP/bin:$PATH" CONDUCTOR_REPO="metabolomics-us/go-modules" run bash "$CLI" start --repo metabolomics-us/go-modules --repo-dir "$REPO_ROOT"

  [ "$status" -eq 0 ]
  [[ "$output" == *"autospec-autonomous started"* ]]
  [ -f "$HOME/.autospec/autonomous-operator/metabolomics-us_go-modules/conductor.pid" ]
  [ "$(cat "$HOME/.autospec/autonomous-operator/metabolomics-us_go-modules/conductor.pid")" = "424242" ]
}

@test "operator cli: detached start also starts monitor and supervisor companions" {
  mkdir -p "$TEST_TMP/bin"
  cat > "$TEST_TMP/bin/python3" <<'PY_FAKE'
#!/usr/bin/env bash
state="$HOME/python-call-count"
count=0
[ -f "$state" ] && count="$(cat "$state")"
count=$((count + 1))
printf '%s\n' "$count" > "$state"
printf '%s\n' "$*" >> "$HOME/python-argv.log"
case "$count" in
  1) printf '111111\n' ;;
  2) printf '222222\n' ;;
  3) printf '333333\n' ;;
  *) printf '444444\n' ;;
esac
PY_FAKE
  chmod +x "$TEST_TMP/bin/python3"

  PATH="$TEST_TMP/bin:$PATH" \
    CONDUCTOR_REPO="berlinguyinca/autospec" \
    run bash "$CLI" start --repo berlinguyinca/autospec --repo-dir "$REPO_ROOT"

  [ "$status" -eq 0 ]
  [[ "$output" == *"autospec-autonomous started"* ]]
  [[ "$output" == *"monitor pid: 222222"* ]]
  [[ "$output" == *"supervisor pid: 333333"* ]]
  [ "$(cat "$HOME/.autospec/autonomous-operator/berlinguyinca_autospec/conductor.pid")" = "111111" ]
  [ "$(cat "$HOME/.autospec/autonomous-operator/berlinguyinca_autospec/monitor.pid")" = "222222" ]
  [ "$(cat "$HOME/.autospec/autonomous-operator/berlinguyinca_autospec/supervisor.pid")" = "333333" ]
  grep -q 'monitor --repo-dir' "$HOME/python-argv.log"
  grep -q 'supervise --repo-dir' "$HOME/python-argv.log"
}

@test "operator cli: status logs and stop use repo-scoped conductor paths" {
  mkdir -p "$HOME/.autospec/autonomous-operator/berlinguyinca_autospec" \
    "$HOME/.autospec/autonomous-operator/metabolomics-us_go-modules" \
    "$TEST_TMP/logs" "$TEST_TMP/scripts"
  printf '111111\n' > "$HOME/.autospec/autonomous-operator/berlinguyinca_autospec/conductor.pid"
  printf '%s\n' "$TEST_TMP/logs/autospec.log" > "$HOME/.autospec/autonomous-operator/berlinguyinca_autospec/conductor.logpath"
  printf 'autospec only\n' > "$TEST_TMP/logs/autospec.log"
  printf '222222\n' > "$HOME/.autospec/autonomous-operator/metabolomics-us_go-modules/conductor.pid"
  printf '%s\n' "$TEST_TMP/logs/go-modules.log" > "$HOME/.autospec/autonomous-operator/metabolomics-us_go-modules/conductor.logpath"
  printf 'go modules only\n' > "$TEST_TMP/logs/go-modules.log"
  cat > "$TEST_TMP/scripts/autospec-stop.sh" <<'EOF_STOP'
#!/usr/bin/env bash
printf '%s\n' "${AUTOSPEC_STOP_FLAG_FILE:-}" > "$HOME/stop.flag.path"
printf '%s\n' "$*" > "$HOME/stop.args"
EOF_STOP
  chmod +x "$TEST_TMP/scripts/autospec-stop.sh"

  CONDUCTOR_REPO="metabolomics-us/go-modules" run bash "$CLI" status --json --repo metabolomics-us/go-modules
  [ "$status" -eq 0 ]
  [[ "$output" == *'autonomous-operator/metabolomics-us_go-modules/conductor.pid'* ]]
  [[ "$output" == *'go-modules.log'* ]]
  [[ "$output" != *'autospec.log'* ]]

  CONDUCTOR_REPO="metabolomics-us/go-modules" run bash "$CLI" logs --repo metabolomics-us/go-modules --lines 1
  [ "$status" -eq 0 ]
  [ "$output" = "go modules only" ]

  AUTOSPEC_SCRIPTS_DIR="$TEST_TMP/scripts" CONDUCTOR_REPO="metabolomics-us/go-modules" run bash "$CLI" stop --repo metabolomics-us/go-modules --immediate
  [ "$status" -eq 0 ]
  [ "$(cat "$HOME/stop.args")" = "--immediate" ]
  [ "$(cat "$HOME/stop.flag.path")" = "$HOME/.autospec/autonomous-operator/metabolomics-us_go-modules/stop.flag" ]
}

@test "operator cli: detached start preserves dry-run and no-digest across re-exec" {
  local harness="$TEST_TMP/harness"
  local scripts_dir="$harness/scripts"
  local repo_dir="$harness/repo"
  local log_file="$TEST_TMP/conductor.log"
  mkdir -p "$scripts_dir/lib" "$repo_dir"
  # start_detached fails loud unless --repo-dir is a real git checkout.
  git -C "$repo_dir" init -q
  cp "$CLI" "$scripts_dir/autospec-autonomous.sh"
  cp "$REPO_ROOT/scripts/lib/autospec-status-accountability.sh" "$scripts_dir/lib/"
  chmod +x "$scripts_dir/autospec-autonomous.sh"
  cat > "$scripts_dir/lib/autospec-loop.sh" <<'LIBEOF'
autospec_conductor_run() {
  printf 'DRY=%s\n' "${CONDUCTOR_DRY_RUN:-unset}"
  printf 'NO_DIGEST=%s\n' "${CONDUCTOR_NO_DIGEST:-unset}"
}
LIBEOF

  AUTOSPEC_AUTONOMOUS_COMPANIONS=0 run bash "$scripts_dir/autospec-autonomous.sh" start \
    --repo-dir "$repo_dir" \
    --repo berlinguyinca/autospec \
    --log "$log_file" \
    --max-cycles 1 \
    --poll-interval-sec 0 \
    --dry-run \
    --no-digest \
    --force
  [ "$status" -eq 0 ]

  local tries=0
  while [ "$tries" -lt 50 ] && ! grep -q '^DRY=' "$log_file" 2>/dev/null; do
    sleep 0.1
    tries=$((tries + 1))
  done

  grep -q '^DRY=1$' "$log_file"
  grep -q '^NO_DIGEST=1$' "$log_file"
}
