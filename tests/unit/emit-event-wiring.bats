#!/usr/bin/env bats
# emit-event-wiring.bats — SHARED wiring scaffold (issue #1771) that each
# telemetry chokepoint issue (#1772-#1776) extends with its own @test cases.
# This foundation suite only proves the shim sources cleanly and defines
# emit_event; chokepoint issues add call-site-specific cases stubbing the
# autospec-db binary via the same PATH-shim idiom as emit-event.bats.
#
# Isolation, belt-and-braces (this repo had a live incident where an unstubbed
# test leaked telemetry into production): HOME is pinned to a per-test tmpdir so
# a real ~/.autospec/db.env cannot leak in, the autospec-db binary is stubbed on
# PATH so no test can reach a real binary, and AUTOSPEC_DB_DISABLE=1 is exported
# as a hard kill switch. Never psql; never a live database.

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
SHIM="$REPO_ROOT/skills/autospec-shared/scripts/emit-event.sh"

setup() {
  TMP="$(mktemp -d)"
  mkdir -p "$TMP/bin" "$TMP/home"
  # stub autospec-db binary: logs argv verbatim, one line per invocation
  cat > "$TMP/bin/autospec-db" <<'SH'
#!/usr/bin/env bash
echo "$@" >> "$BIN_LOG"
exit 0
SH
  chmod +x "$TMP/bin/autospec-db"
  export BIN_LOG="$TMP/bin.log"
  export HOME="$TMP/home"
  export AUTOSPEC_DB_DISABLE=1
  unset AUTOSPEC_DB_DSN
}

teardown() {
  rm -rf "$TMP"
  unset AUTOSPEC_DB_DSN BIN_LOG AUTOSPEC_DB_DISABLE
}

@test "wiring scaffold: shim exists and is bash -n clean" {
  [ -f "$SHIM" ]
  run bash -n "$SHIM"
  [ "$status" -eq 0 ]
}

@test "sourcing the shim defines emit_event" {
  export PATH="$TMP/bin:$PATH"
  run bash -c ". '$SHIM'; type emit_event >/dev/null 2>&1 && echo defined"
  [ "$status" -eq 0 ]
  [[ "$output" == *"defined"* ]]
}

# ── chokepoint: scripts/autospec-run-registry.sh (issue #1772) ──────────────
# ── chokepoint: skills/autospec-run/scripts/run-state.sh (issue #1772) ──────
#
# Both helpers resolve the shim at ${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/emit-event.sh
# (the installed-runtime path from the shared contract), so these cases stage
# a copy of the real shim under a per-test AUTOSPEC_SCRIPTS_DIR and PATH-shim
# the autospec-db binary the shim dispatches to — never a real binary, never
# a live database.

REGISTRY="$REPO_ROOT/scripts/autospec-run-registry.sh"
RUN_STATE="$REPO_ROOT/skills/autospec-run/scripts/run-state.sh"

# _enable_emit stages the shim under a fresh AUTOSPEC_SCRIPTS_DIR and puts the
# autospec-db stub on PATH + sets a DSN so the shim's guards both pass.
_enable_emit() {
  mkdir -p "$TMP/scripts"
  cp "$SHIM" "$TMP/scripts/emit-event.sh"
  export AUTOSPEC_SCRIPTS_DIR="$TMP/scripts"
  export PATH="$TMP/bin:$PATH"
  export AUTOSPEC_DB_DSN="postgresql://autospec_test:secret@127.0.0.1:5432/autospec?sslmode=require"
}

# _install_gh_stub installs a minimal `gh` stub on PATH for run-state.sh,
# mirroring the idiom in tests/unit/test_autospec_run_state.bats: an
# in-memory comments.json backs `gh issue comment` / `gh api ... -X PATCH|DELETE`.
_install_gh_stub() {
  COMMENTS="$TMP/comments.json"
  printf '[]\n' > "$COMMENTS"
  cat > "$TMP/bin/gh" <<'SH'
#!/usr/bin/env bash
set -eu
comments="${AUTOSPEC_TEST_COMMENTS:?}"

if [ "$1" = "repo" ] && [ "$2" = "view" ]; then
  printf 'o/n\n'
  exit 0
fi

if [ "$1" = "issue" ] && [ "$2" = "comment" ]; then
  body_file=""
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --body-file) body_file="$2"; shift 2 ;;
      *) shift ;;
    esac
  done
  jq --arg body "$(cat "$body_file")" '. + [{id: 1, body: $body}]' "$comments" > "$comments.tmp"
  mv "$comments.tmp" "$comments"
  exit 0
fi

if [ "$1" = "api" ]; then
  method=""
  body_file=""
  url="$2"
  shift 2
  while [ "$#" -gt 0 ]; do
    case "$1" in
      -X) method="$2"; shift 2 ;;
      -F)
        case "$2" in body=@*) body_file="${2#body=@}" ;; esac
        shift 2
        ;;
      *) shift ;;
    esac
  done
  id="${url##*/}"
  if [ "$url" = "repos/o/n/issues/42/comments" ]; then
    cat "$comments"
    exit 0
  fi
  case "$method" in
    PATCH)
      jq --argjson id "$id" --arg body "$(cat "$body_file")" \
        'map(if .id == $id then .body = $body else . end)' "$comments" > "$comments.tmp"
      mv "$comments.tmp" "$comments"
      ;;
    DELETE)
      jq --argjson id "$id" 'map(select(.id != $id))' "$comments" > "$comments.tmp"
      mv "$comments.tmp" "$comments"
      ;;
  esac
  exit 0
fi
exit 1
SH
  chmod +x "$TMP/bin/gh"
  export AUTOSPEC_TEST_COMMENTS="$COMMENTS"
  export AUTOSPEC_GH_API_RETRY_SLEEP=0
}

@test "heartbeat write emits exactly one heartbeat event" {
  _enable_emit
  export AUTOSPEC_ACTIVE_RUNS_DIR="$TMP/active-runs"

  run bash "$REGISTRY" write --repo o/n --repo-dir /abs/checkout --harness claude --command "echo hi" --host h1
  [ "$status" -eq 0 ]

  [ -f "$BIN_LOG" ]
  [ "$(wc -l < "$BIN_LOG" | tr -d ' ')" -eq 1 ]
  run cat "$BIN_LOG"
  [[ "$output" == *"emit heartbeat"* ]]
  [[ "$output" == *"repo=o/n"* ]]
}

@test "heartbeat write emit never alters the write's exit code or stdout when telemetry is disabled" {
  # Same AUTOSPEC_ACTIVE_RUNS_DIR for both runs so the printed path is
  # byte-for-byte comparable, not just similarly shaped.
  export AUTOSPEC_ACTIVE_RUNS_DIR="$TMP/active-runs"

  run bash "$REGISTRY" write --repo o/n --repo-dir /abs/checkout --harness claude --command "echo hi" --host h1
  disabled_status="$status"
  disabled_output="$output"

  _enable_emit
  export AUTOSPEC_ACTIVE_RUNS_DIR="$TMP/active-runs"
  run bash "$REGISTRY" write --repo o/n --repo-dir /abs/checkout --harness claude --command "echo hi" --host h1
  enabled_status="$status"
  enabled_output="$output"

  # The write's own observable contract (exit code + printed path) is
  # byte-identical whether telemetry is wired in and enabled, or fully
  # no-opped (disabled/absent DSN).
  [ "$disabled_status" -eq "$enabled_status" ]
  [ "$disabled_output" = "$enabled_output" ]
  [[ "$enabled_output" == *"/o__n.json" ]]
}

@test "run-state upsert with no prior state comment emits session.started" {
  _enable_emit
  _install_gh_stub

  run bash "$RUN_STATE" upsert --issue 42 --repo o/n --worker-id worker-a --state claimed --step claimed
  [ "$status" -eq 0 ]

  [ -f "$BIN_LOG" ]
  [ "$(wc -l < "$BIN_LOG" | tr -d ' ')" -eq 1 ]
  run cat "$BIN_LOG"
  [[ "$output" == *"emit session.started"* ]]
  [[ "$output" == *"repo=o/n"* ]]
  [[ "$output" == *"issue=42"* ]]
}

@test "run-state upsert over an existing state comment emits session.step" {
  _enable_emit
  _install_gh_stub

  bash "$RUN_STATE" upsert --issue 42 --repo o/n --worker-id worker-a --state claimed --step claimed >/dev/null
  : > "$BIN_LOG"   # only assert on the SECOND upsert's emit
  run bash "$RUN_STATE" upsert --issue 42 --repo o/n --worker-id worker-a --state worktree_ready --step worktree_ready
  [ "$status" -eq 0 ]

  [ "$(wc -l < "$BIN_LOG" | tr -d ' ')" -eq 1 ]
  run cat "$BIN_LOG"
  [[ "$output" == *"emit session.step"* ]]
}

@test "run-state clear emits session.terminal" {
  _enable_emit
  _install_gh_stub

  bash "$RUN_STATE" upsert --issue 42 --repo o/n --worker-id worker-a --state claimed --step claimed >/dev/null
  : > "$BIN_LOG"   # only assert on clear's emit
  run bash "$RUN_STATE" clear --issue 42 --repo o/n
  [ "$status" -eq 0 ]

  [ "$(wc -l < "$BIN_LOG" | tr -d ' ')" -eq 1 ]
  run cat "$BIN_LOG"
  [[ "$output" == *"emit session.terminal"* ]]
}

@test "unset AUTOSPEC_DB_DSN yields 0 emit-binary calls from the heartbeat write" {
  # setup() already unsets AUTOSPEC_DB_DSN; stage the shim + stub binary on
  # PATH but do NOT set a DSN, so the shim's own guard 1 no-ops before ever
  # touching the binary.
  mkdir -p "$TMP/scripts"
  cp "$SHIM" "$TMP/scripts/emit-event.sh"
  export AUTOSPEC_SCRIPTS_DIR="$TMP/scripts"
  export PATH="$TMP/bin:$PATH"
  export AUTOSPEC_ACTIVE_RUNS_DIR="$TMP/active-runs-nodsn"

  run bash "$REGISTRY" write --repo o/n --repo-dir /abs/checkout --harness claude --command "echo hi" --host h1
  [ "$status" -eq 0 ]
  [ ! -s "$BIN_LOG" ]
}

@test "unset AUTOSPEC_DB_DSN yields 0 emit-binary calls from run-state upsert" {
  mkdir -p "$TMP/scripts"
  cp "$SHIM" "$TMP/scripts/emit-event.sh"
  export AUTOSPEC_SCRIPTS_DIR="$TMP/scripts"
  export PATH="$TMP/bin:$PATH"
  _install_gh_stub

  run bash "$RUN_STATE" upsert --issue 42 --repo o/n --worker-id worker-a --state claimed --step claimed
  [ "$status" -eq 0 ]
  [ ! -s "$BIN_LOG" ]
}

@test "no emitted call-site ever carries the DSN value" {
  _enable_emit
  _install_gh_stub
  export AUTOSPEC_ACTIVE_RUNS_DIR="$TMP/active-runs-dsn-leak"

  bash "$REGISTRY" write --repo o/n --repo-dir /abs/checkout --harness claude --command "echo hi" --host h1 >/dev/null
  bash "$RUN_STATE" upsert --issue 42 --repo o/n --worker-id worker-a --state claimed --step claimed >/dev/null

  run cat "$BIN_LOG"
  [[ "$output" != *"postgresql://"* ]]
  [[ "$output" != *"secret"* ]]
}

# ── chokepoint: skills/autospec-shared/scripts/explore-ledger.sh (issue #1773) ──
# ── chokepoint: skills/autospec-shared/scripts/growth-ledger.sh (issue #1773) ───
#
# Both ledgers resolve the shim at the same installed-runtime path and stamp
# repo= via a local-only `git config --get remote.origin.url` (no network
# call), so these cases run inside a throwaway git repo under TMP.

EXPLORE_LEDGER_SH="$REPO_ROOT/skills/autospec-shared/scripts/explore-ledger.sh"
GROWTH_LEDGER_SH="$REPO_ROOT/skills/autospec-shared/scripts/growth-ledger.sh"

# _repo_cwd sets up a throwaway git repo (with an origin remote, so
# _repo_slug() has something to resolve) as the ledger's cwd.
_repo_cwd() {
  REPO_DIR="$TMP/repo"
  mkdir -p "$REPO_DIR"
  ( cd "$REPO_DIR" && git init -q && git remote add origin https://github.com/o/n.git )
}

@test "explore-ledger append emits exactly one artifact.filed event" {
  _enable_emit
  _repo_cwd
  export AUTOSPEC_EXPLORE_LEDGER="$TMP/explore-ledger.jsonl"

  run bash -c "cd '$REPO_DIR' && bash '$EXPLORE_LEDGER_SH' --append '{\"round\":1,\"source\":\"s\",\"title\":\"t\",\"norm_title\":\"t\",\"complexity\":\"small\",\"confidence\":0.9,\"issue\":7,\"pr\":null,\"outcome\":\"pending\",\"reason\":\"\",\"ts\":\"2026-01-01T00:00:00Z\"}'"
  [ "$status" -eq 0 ]

  [ -f "$BIN_LOG" ]
  [ "$(wc -l < "$BIN_LOG" | tr -d ' ')" -eq 1 ]
  # Exact-line assertion: a substring match would miss a malformed repo
  # value like repo=o/n.git (the staged remote ends in .git on purpose).
  [ "$(cat "$BIN_LOG")" = "emit artifact.filed repo=o/n issue=7 detail=explore" ]
}

@test "growth-ledger append emits exactly one artifact.filed event" {
  _enable_emit
  _repo_cwd
  export GROWTH_LEDGER="$TMP/growth-ledger.jsonl"

  run bash -c "cd '$REPO_DIR' && bash '$GROWTH_LEDGER_SH' --append '{\"round\":1,\"source\":\"s\",\"title\":\"t\",\"norm_title\":\"t\",\"channel\":\"c\",\"kind\":\"artifact\",\"issue\":9,\"outcome\":\"pending\",\"reason\":\"\",\"ts\":\"2026-01-01T00:00:00Z\"}'"
  [ "$status" -eq 0 ]

  [ -f "$BIN_LOG" ]
  [ "$(wc -l < "$BIN_LOG" | tr -d ' ')" -eq 1 ]
  # Exact-line assertion: a substring match would miss a malformed repo
  # value like repo=o/n.git (the staged remote ends in .git on purpose).
  [ "$(cat "$BIN_LOG")" = "emit artifact.filed repo=o/n issue=9 detail=growth" ]
}

@test "unset AUTOSPEC_DB_DSN yields 0 emit-binary calls from explore-ledger append" {
  mkdir -p "$TMP/scripts"
  cp "$SHIM" "$TMP/scripts/emit-event.sh"
  export AUTOSPEC_SCRIPTS_DIR="$TMP/scripts"
  export PATH="$TMP/bin:$PATH"
  _repo_cwd
  export AUTOSPEC_EXPLORE_LEDGER="$TMP/explore-ledger-nodsn.jsonl"

  run bash -c "cd '$REPO_DIR' && bash '$EXPLORE_LEDGER_SH' --append '{\"round\":1,\"source\":\"s\",\"title\":\"t\",\"norm_title\":\"t\",\"complexity\":\"small\",\"confidence\":0.9,\"issue\":7,\"pr\":null,\"outcome\":\"pending\",\"reason\":\"\",\"ts\":\"2026-01-01T00:00:00Z\"}'"
  [ "$status" -eq 0 ]
  [ ! -s "$BIN_LOG" ]
}

@test "unset AUTOSPEC_DB_DSN yields 0 emit-binary calls from growth-ledger append" {
  mkdir -p "$TMP/scripts"
  cp "$SHIM" "$TMP/scripts/emit-event.sh"
  export AUTOSPEC_SCRIPTS_DIR="$TMP/scripts"
  export PATH="$TMP/bin:$PATH"
  _repo_cwd
  export GROWTH_LEDGER="$TMP/growth-ledger-nodsn.jsonl"

  run bash -c "cd '$REPO_DIR' && bash '$GROWTH_LEDGER_SH' --append '{\"round\":1,\"source\":\"s\",\"title\":\"t\",\"norm_title\":\"t\",\"channel\":\"c\",\"kind\":\"artifact\",\"issue\":9,\"outcome\":\"pending\",\"reason\":\"\",\"ts\":\"2026-01-01T00:00:00Z\"}'"
  [ "$status" -eq 0 ]
  [ ! -s "$BIN_LOG" ]
}

@test "ledger append emit never alters the append's exit code or ledger content when telemetry is disabled" {
  # Same ledger paths for both runs so the appended line is byte-for-byte
  # comparable, not just similarly shaped.
  _repo_cwd
  export AUTOSPEC_EXPLORE_LEDGER="$TMP/explore-ledger-cmp.jsonl"
  append_json='{"round":1,"source":"s","title":"t","norm_title":"t","complexity":"small","confidence":0.9,"issue":7,"pr":null,"outcome":"pending","reason":"","ts":"2026-01-01T00:00:00Z"}'

  run bash -c "cd '$REPO_DIR' && bash '$EXPLORE_LEDGER_SH' --append '$append_json'"
  disabled_status="$status"
  disabled_ledger="$(cat "$AUTOSPEC_EXPLORE_LEDGER")"
  rm -f "$AUTOSPEC_EXPLORE_LEDGER"

  _enable_emit
  run bash -c "cd '$REPO_DIR' && bash '$EXPLORE_LEDGER_SH' --append '$append_json'"
  enabled_status="$status"
  enabled_ledger="$(cat "$AUTOSPEC_EXPLORE_LEDGER")"

  [ "$disabled_status" -eq "$enabled_status" ]
  [ "$disabled_ledger" = "$enabled_ledger" ]
}
