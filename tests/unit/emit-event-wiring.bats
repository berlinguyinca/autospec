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
  export AUTOSPEC_BIN="$REPO_ROOT/tests/fixtures/autospec-project-sync-stub.sh"
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
  unset AUTOSPEC_TELEMETRY_ENABLED AUTOSPEC_DB_HOST_LABEL
  unset AUTOSPEC_DB_SPOOL_MAX_BYTES AUTOSPEC_INSTALL_DB_MODULE
}

teardown() {
  rm -rf "$TMP"
  unset AUTOSPEC_DB_DSN BIN_LOG AUTOSPEC_DB_DISABLE AUTOSPEC_BIN
  unset AUTOSPEC_CLAIM_GIT_REMOTE AUTOSPEC_CLAIM_GIT_STATE_DIR
  unset AUTOSPEC_TELEMETRY_ENABLED AUTOSPEC_DB_HOST_LABEL
  unset AUTOSPEC_DB_SPOOL_MAX_BYTES AUTOSPEC_INSTALL_DB_MODULE
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
# ── chokepoint: autospec claim state (issue #1772) ─────────────────────────
#
# The registry resolves the shell shim; Rust claim-state telemetry resolves
# `autospec-db` directly. These cases PATH-shim the binary so no test can reach
# a real database.

REGISTRY="$REPO_ROOT/scripts/autospec-run-registry.sh"
AUTOSPEC="$REPO_ROOT/target/debug/autospec"

# _enable_emit stages the shim under a fresh AUTOSPEC_SCRIPTS_DIR and puts the
# autospec-db stub on PATH + sets a DSN so the shim's guards both pass.
_enable_emit() {
  mkdir -p "$TMP/scripts"
  cp "$SHIM" "$TMP/scripts/emit-event.sh"
  export AUTOSPEC_SCRIPTS_DIR="$TMP/scripts"
  export PATH="$TMP/bin:$PATH"
  export AUTOSPEC_DB_DSN="postgresql://autospec_test:secret@127.0.0.1:5432/autospec?sslmode=require"
}

# _install_gh_stub installs a minimal `gh` stub on PATH for Rust claim state,
# mirroring the idiom in tests/unit/test_autospec_run_state.bats: an
# in-memory comments.json backs `gh issue comment` / `gh api ... -X PATCH|DELETE`.
_install_gh_stub() {
  COMMENTS="$TMP/comments.json"
  printf '[]\n' > "$COMMENTS"
  git init --bare --quiet "$TMP/claim-remote.git"
  export AUTOSPEC_CLAIM_GIT_REMOTE="$TMP/claim-remote.git"
  export AUTOSPEC_CLAIM_GIT_STATE_DIR="$TMP/claim-state"
  cat > "$TMP/bin/gh" <<'SH'
#!/usr/bin/env bash
set -eu
comments="${AUTOSPEC_TEST_COMMENTS:?}"

if [ "$1" = "repo" ] && [ "$2" = "view" ]; then
  printf 'o/n\n'
  exit 0
fi

if [ "$1" = "issue" ] && [ "$2" = "view" ]; then
  cat <<'JSON'
{"labels":["auto-implement","in-progress-by-bot","safety:reviewed"],"body":"## Safety review\n\n<!-- autospec-safety:begin -->\n- **decision:** `SAFETY_PASS`\n<!-- autospec-safety:end -->","title":"telemetry fixture","author":"fixture-agent"}
JSON
  exit 0
fi

if [ "$1" = "label" ] && [ "$2" = "create" ]; then
  exit 0
fi

if [ "$1" = "issue" ] && [ "$2" = "edit" ]; then
  exit 0
fi

if [ "$1" = "issue" ] && [ "$2" = "comment" ]; then
  body=""
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --body-file) body="$(cat "$2")"; shift 2 ;;
      --body) body="$2"; shift 2 ;;
      *) shift ;;
    esac
  done
  jq --arg body "$body" '. + [{id: 1, body: $body, updated_at:"2026-07-14T00:00:00Z"}]' "$comments" > "$comments.tmp"
  mv "$comments.tmp" "$comments"
  exit 0
fi

if [ "$1" = "api" ]; then
  method=""
  body=""
  url="$2"
  shift 2
  while [ "$#" -gt 0 ]; do
    case "$1" in
      -X) method="$2"; shift 2 ;;
      -F)
        case "$2" in body=@*) body="$(cat "${2#body=@}")" ;; esac
        shift 2
        ;;
      -f)
        case "$2" in body=*) body="${2#body=}" ;; esac
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
      jq --argjson id "$id" --arg body "$body" \
        'map(if .id == $id then .body = $body | .updated_at = "2026-07-14T00:00:00Z" else . end)' "$comments" > "$comments.tmp"
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

_acquire_claim() {
  "$AUTOSPEC" claim acquire \
    --issue 42 \
    --repo o/n \
    --worker-id worker-a \
    --branch feat/telemetry
}

_upsert_claim() {
  claim_id="$1"
  step="$2"
  "$AUTOSPEC" claim state upsert \
    --issue 42 \
    --repo o/n \
    --worker-id worker-a \
    --claim-id "$claim_id" \
    --branch feat/telemetry \
    --state claimed \
    --step "$step"
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

@test "claim acquire emits session.started for the first authoritative generation" {
  _enable_emit
  _install_gh_stub

  run _acquire_claim
  [ "$status" -eq 0 ]

  [ -f "$BIN_LOG" ]
  [ "$(wc -l < "$BIN_LOG" | tr -d ' ')" -eq 1 ]
  run cat "$BIN_LOG"
  [[ "$output" == *"emit session.started"* ]]
  [[ "$output" == *"repo=o/n"* ]]
  [[ "$output" == *"issue=42"* ]]
}

@test "claim state upsert over an existing state comment emits session.step" {
  _enable_emit
  _install_gh_stub

  lease="$(_acquire_claim)"
  claim_id="$(printf '%s\n' "$lease" | jq -r '.claim_id')"
  : > "$BIN_LOG"   # only assert on the SECOND upsert's emit
  run _upsert_claim "$claim_id" worktree_ready
  [ "$status" -eq 0 ]

  [ "$(wc -l < "$BIN_LOG" | tr -d ' ')" -eq 1 ]
  run cat "$BIN_LOG"
  [[ "$output" == *"emit session.step"* ]]
}

@test "claim state clear emits session.terminal" {
  _enable_emit
  _install_gh_stub

  lease="$(_acquire_claim)"
  claim_id="$(printf '%s\n' "$lease" | jq -r '.claim_id')"
  : > "$BIN_LOG"   # only assert on clear's emit
  run "$AUTOSPEC" claim state clear \
    --issue 42 \
    --repo o/n \
    --worker-id worker-a \
    --claim-id "$claim_id" \
    --branch feat/telemetry
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

@test "unset AUTOSPEC_DB_DSN yields 0 emit-binary calls from claim state upsert" {
  mkdir -p "$TMP/scripts"
  cp "$SHIM" "$TMP/scripts/emit-event.sh"
  export AUTOSPEC_SCRIPTS_DIR="$TMP/scripts"
  export PATH="$TMP/bin:$PATH"
  _install_gh_stub

  lease="$(_acquire_claim)"
  claim_id="$(printf '%s\n' "$lease" | jq -r '.claim_id')"
  run _upsert_claim "$claim_id" worktree_ready
  [ "$status" -eq 0 ]
  [ ! -s "$BIN_LOG" ]
}

@test "no emitted call-site ever carries the DSN value" {
  _enable_emit
  _install_gh_stub
  export AUTOSPEC_ACTIVE_RUNS_DIR="$TMP/active-runs-dsn-leak"

  bash "$REGISTRY" write --repo o/n --repo-dir /abs/checkout --harness claude --command "echo hi" --host h1 >/dev/null
  lease="$(_acquire_claim)"
  claim_id="$(printf '%s\n' "$lease" | jq -r '.claim_id')"
  _upsert_claim "$claim_id" worktree_ready >/dev/null

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

# ── chokepoint: scripts/claim-guard.sh (issue #1774) ────────────────────────
# ── chokepoint: skills/autospec-shared/scripts/grow-define-file-issues.sh (issue #1774) ──
#
# claim-guard resolves the shim at the installed-runtime path and stamps
# surface= from the raw target args; grow-define-file-issues.sh resolves it
# the same way and stamps repo= via a local-only git remote lookup (no
# network call, mirroring explore-ledger/growth-ledger). claim-guard's own
# state is redirected under AUTOSPEC_STATE_DIR so no test ever touches a
# real ~/.autospec/edit-claims store.

CLAIM_GUARD_SH="$REPO_ROOT/scripts/claim-guard.sh"
GROW_DEFINE_FILE_ISSUES_SH="$REPO_ROOT/skills/autospec-shared/scripts/grow-define-file-issues.sh"

_claim_state_dir() {
  AUTOSPEC_STATE_DIR="$TMP/claim-state"
  mkdir -p "$AUTOSPEC_STATE_DIR"
  export AUTOSPEC_STATE_DIR
}

# _install_gh_issue_stub: minimal `gh` stub for grow-define-file-issues.sh —
# `gh label create` no-ops, `gh issue create` always succeeds and returns a
# fixed issue URL (#501).
_install_gh_issue_stub() {
  cat > "$TMP/bin/gh" <<'SH'
#!/usr/bin/env bash
if [ "$1" = "label" ]; then exit 0; fi
if [ "$1" = "issue" ] && [ "$2" = "create" ]; then
  echo "https://github.com/o/n/issues/501"
  exit 0
fi
exit 1
SH
  chmod +x "$TMP/bin/gh"
}

@test "claim-guard acquire emits exactly one claim event with conflict=false" {
  _enable_emit
  _claim_state_dir
  export AUTOSPEC_SESSION_ID="session-a"
  export AUTOSPEC_CLAIM_GUARD=strict

  run bash "$CLAIM_GUARD_SH" acquire scripts/foo.sh
  [ "$status" -eq 0 ]

  [ -f "$BIN_LOG" ]
  [ "$(wc -l < "$BIN_LOG" | tr -d ' ')" -eq 1 ]
  run cat "$BIN_LOG"
  [[ "$output" == *"emit claim"* ]]
  [[ "$output" == *"surface=scripts/foo.sh"* ]]
  [[ "$output" == *"conflict=false"* ]]
}

@test "claim-guard release emits exactly one claim event" {
  _enable_emit
  _claim_state_dir
  export AUTOSPEC_SESSION_ID="session-a"
  export AUTOSPEC_CLAIM_GUARD=strict

  bash "$CLAIM_GUARD_SH" acquire scripts/foo.sh >/dev/null
  : > "$BIN_LOG"   # only assert on release's emit
  run bash "$CLAIM_GUARD_SH" release scripts/foo.sh
  [ "$status" -eq 0 ]

  [ "$(wc -l < "$BIN_LOG" | tr -d ' ')" -eq 1 ]
  run cat "$BIN_LOG"
  [[ "$output" == *"emit claim"* ]]
  [[ "$output" == *"surface=scripts/foo.sh"* ]]
}

@test "claim-guard acquire conflict emits claim event with conflict=true" {
  _enable_emit
  _claim_state_dir

  AUTOSPEC_SESSION_ID="session-a" AUTOSPEC_CLAIM_GUARD=strict \
    bash "$CLAIM_GUARD_SH" acquire scripts/foo.sh >/dev/null

  : > "$BIN_LOG"   # only assert on the CONFLICTING session's emit
  export AUTOSPEC_SESSION_ID="session-b"
  export AUTOSPEC_CLAIM_GUARD=strict
  run bash "$CLAIM_GUARD_SH" acquire scripts/foo.sh
  [ "$status" -eq 6 ]

  [ "$(wc -l < "$BIN_LOG" | tr -d ' ')" -eq 1 ]
  run cat "$BIN_LOG"
  [[ "$output" == *"emit claim"* ]]
  [[ "$output" == *"conflict=true"* ]]
}

@test "a present-but-broken shim never alters claim-guard's exit code" {
  # Regression (peer review, issue #1774): sourcing a shim that returns
  # non-zero under claim-guard's `set -e` must not change acquire/release
  # exit status — the source+emit block is wrapped in `{ ... } || true`.
  _claim_state_dir
  mkdir -p "$TMP/scripts"
  printf 'return 7\n' > "$TMP/scripts/emit-event.sh"
  export AUTOSPEC_SCRIPTS_DIR="$TMP/scripts"
  export AUTOSPEC_SESSION_ID="session-a"
  export AUTOSPEC_CLAIM_GUARD=strict

  run bash "$CLAIM_GUARD_SH" acquire scripts/foo.sh
  [ "$status" -eq 0 ]
  run bash "$CLAIM_GUARD_SH" release scripts/foo.sh
  [ "$status" -eq 0 ]
}

@test "unset AUTOSPEC_DB_DSN yields 0 emit-binary calls from claim-guard acquire" {
  mkdir -p "$TMP/scripts"
  cp "$SHIM" "$TMP/scripts/emit-event.sh"
  export AUTOSPEC_SCRIPTS_DIR="$TMP/scripts"
  export PATH="$TMP/bin:$PATH"
  _claim_state_dir
  export AUTOSPEC_SESSION_ID="session-a"
  export AUTOSPEC_CLAIM_GUARD=strict

  run bash "$CLAIM_GUARD_SH" acquire scripts/foo.sh
  [ "$status" -eq 0 ]
  [ ! -s "$BIN_LOG" ]
}

@test "claim-guard acquire/release emit never alters exit code when telemetry is disabled" {
  _claim_state_dir
  export AUTOSPEC_SESSION_ID="session-a"
  export AUTOSPEC_CLAIM_GUARD=strict

  run bash "$CLAIM_GUARD_SH" acquire scripts/foo.sh
  disabled_acquire_status="$status"
  disabled_acquire_output="$output"
  run bash "$CLAIM_GUARD_SH" release scripts/foo.sh
  disabled_release_status="$status"
  disabled_release_output="$output"

  _claim_state_dir
  _enable_emit
  export AUTOSPEC_SESSION_ID="session-a"
  export AUTOSPEC_CLAIM_GUARD=strict
  run bash "$CLAIM_GUARD_SH" acquire scripts/foo.sh
  enabled_acquire_status="$status"
  enabled_acquire_output="$output"
  run bash "$CLAIM_GUARD_SH" release scripts/foo.sh
  enabled_release_status="$status"
  enabled_release_output="$output"

  [ "$disabled_acquire_status" -eq "$enabled_acquire_status" ]
  [ "$disabled_release_status" -eq "$enabled_release_status" ]
  [ "$disabled_acquire_output" = "$enabled_acquire_output" ]
  [ "$disabled_release_output" = "$enabled_release_output" ]
}

@test "grow-define-file-issues files an issue and emits exactly one feature.described event, body bound not spliced" {
  _enable_emit
  _repo_cwd
  _install_gh_issue_stub
  export GROWTH_LEDGER="$TMP/growth-ledger-feature.jsonl"

  RANKED="$TMP/ranked.jsonl"
  CONFIG="$TMP/config.json"
  printf '{}\n' > "$CONFIG"
  jq -nc --arg lens l --arg kind artifact --arg channel c --arg title T --arg norm t \
    --arg rationale "quote's value \$\$ and \\backslash" \
    '{lens:$lens,kind:$kind,channel:$channel,title:$title,norm_title:$norm,rationale:$rationale}' \
    > "$RANKED"

  run bash -c "cd '$REPO_DIR' && bash '$GROW_DEFINE_FILE_ISSUES_SH' '$RANKED' '$CONFIG'"
  [ "$status" -eq 0 ]

  [ -f "$BIN_LOG" ]
  [ "$(grep -c '^emit feature\.described' "$BIN_LOG")" -eq 1 ]

  expected="detail=Growth artifact (lens: l, channel: c)"
  run grep -F -- "$expected" "$BIN_LOG"
  [ "$status" -eq 0 ]

  # The bound body carries the rationale's single quote, literal \$\$, and
  # backslash byte-for-byte — proof it arrived as one bound argument rather
  # than being spliced/re-interpreted through a shell or SQL layer.
  expected_body="quote's value \$\$ and \\backslash"
  run grep -F -- "$expected_body" "$BIN_LOG"
  [ "$status" -eq 0 ]
}

@test "unset AUTOSPEC_DB_DSN yields 0 emit-binary calls from grow-define-file-issues" {
  mkdir -p "$TMP/scripts"
  cp "$SHIM" "$TMP/scripts/emit-event.sh"
  export AUTOSPEC_SCRIPTS_DIR="$TMP/scripts"
  export PATH="$TMP/bin:$PATH"
  _repo_cwd
  _install_gh_issue_stub
  export GROWTH_LEDGER="$TMP/growth-ledger-feature-nodsn.jsonl"

  RANKED="$TMP/ranked-nodsn.jsonl"
  CONFIG="$TMP/config-nodsn.json"
  printf '{}\n' > "$CONFIG"
  jq -nc --arg lens l --arg kind artifact --arg channel c --arg title T --arg norm t \
    --arg rationale "r" \
    '{lens:$lens,kind:$kind,channel:$channel,title:$title,norm_title:$norm,rationale:$rationale}' \
    > "$RANKED"

  run bash -c "cd '$REPO_DIR' && bash '$GROW_DEFINE_FILE_ISSUES_SH' '$RANKED' '$CONFIG'"
  [ "$status" -eq 0 ]
  [ ! -s "$BIN_LOG" ]
}

@test "no claim or feature.described call-site ever carries the DSN value" {
  _enable_emit
  _claim_state_dir
  export AUTOSPEC_SESSION_ID="session-a"
  export AUTOSPEC_CLAIM_GUARD=strict
  bash "$CLAIM_GUARD_SH" acquire scripts/foo.sh >/dev/null
  bash "$CLAIM_GUARD_SH" release scripts/foo.sh >/dev/null

  _repo_cwd
  _install_gh_issue_stub
  export GROWTH_LEDGER="$TMP/growth-ledger-dsn-leak.jsonl"
  RANKED="$TMP/ranked-dsn-leak.jsonl"
  CONFIG="$TMP/config-dsn-leak.json"
  printf '{}\n' > "$CONFIG"
  jq -nc --arg lens l --arg kind artifact --arg channel c --arg title T --arg norm t \
    --arg rationale "r" \
    '{lens:$lens,kind:$kind,channel:$channel,title:$title,norm_title:$norm,rationale:$rationale}' \
    > "$RANKED"
  bash -c "cd '$REPO_DIR' && bash '$GROW_DEFINE_FILE_ISSUES_SH' '$RANKED' '$CONFIG'" >/dev/null

  run cat "$BIN_LOG"
  [[ "$output" != *"postgresql://"* ]]
  [[ "$output" != *"secret"* ]]
}

# ── chokepoint: scripts/autonomous-usage-governor.sh (issue #1775) ──────────
# ── chokepoint: scripts/autospec-stop-check.sh (issue #1775) ────────────────
#
# Both helpers source the shared shim from AUTOSPEC_SCRIPTS_DIR and emit
# session.parked at their authoritative park/stop chokepoints. The tests keep
# every dependency local: usage-observe.sh is PATH-shimmed, autospec-stop.sh is
# intentionally absent, and autospec-db is the stub installed by setup().

USAGE_GOVERNOR_SH="$REPO_ROOT/scripts/autonomous-usage-governor.sh"
STOP_CHECK_SH="$REPO_ROOT/scripts/autospec-stop-check.sh"

_install_usage_observe_stub() {
  cat > "$TMP/bin/usage-observe.sh" <<'SH'
#!/usr/bin/env bash
printf '{"observable":true,"percent":95}\n'
SH
  chmod +x "$TMP/bin/usage-observe.sh"
}

_write_immediate_stop_flag() {
  mkdir -p "$HOME/.autospec"
  {
    printf 'immediate\n'
    printf '2026-07-11T00:00:00Z test@host\n'
  } > "$HOME/.autospec/stop.flag"
}

@test "soft-park decision emits exactly one session.parked event" {
  _enable_emit
  _install_usage_observe_stub

  run bash "$USAGE_GOVERNOR_SH" codex --repo-dir "$TMP/no-repo" --resume-at 2026-07-11T12:00:00Z
  [ "$status" -eq 0 ]
  [[ "$output" == *"park 2026-07-11T12:00:00Z"* ]]

  [ -f "$BIN_LOG" ]
  [ "$(wc -l < "$BIN_LOG" | tr -d ' ')" -eq 1 ]
  run cat "$BIN_LOG"
  [ "$output" = "emit session.parked outcome=soft-park detail=2026-07-11T12:00:00Z" ]
}

@test "stop flag emits session.parked and still returns 42" {
  _enable_emit
  _write_immediate_stop_flag

  run bash "$STOP_CHECK_SH" 1775 feat/wire-park-stop self_review
  [ "$status" -eq 42 ]

  [ -f "$BIN_LOG" ]
  [ "$(wc -l < "$BIN_LOG" | tr -d ' ')" -eq 1 ]
  run cat "$BIN_LOG"
  [ "$output" = "emit session.parked outcome=stop detail=self_review" ]
}

@test "unset AUTOSPEC_DB_DSN yields 0 emit-binary calls from park and stop helpers" {
  mkdir -p "$TMP/scripts"
  cp "$SHIM" "$TMP/scripts/emit-event.sh"
  export AUTOSPEC_SCRIPTS_DIR="$TMP/scripts"
  export PATH="$TMP/bin:$PATH"
  _install_usage_observe_stub

  run bash "$USAGE_GOVERNOR_SH" codex --repo-dir "$TMP/no-repo" --resume-at 2026-07-11T12:00:00Z
  [ "$status" -eq 0 ]
  [[ "$output" == *"park 2026-07-11T12:00:00Z"* ]]

  _write_immediate_stop_flag
  run bash "$STOP_CHECK_SH" 1775 feat/wire-park-stop stop_check
  [ "$status" -eq 42 ]

  [ ! -s "$BIN_LOG" ]
}

@test "no park or stop emitted call-site ever carries the DSN value" {
  _enable_emit
  _install_usage_observe_stub

  bash "$USAGE_GOVERNOR_SH" codex --repo-dir "$TMP/no-repo" --resume-at 2026-07-11T12:00:00Z >/dev/null
  _write_immediate_stop_flag
  bash "$STOP_CHECK_SH" 1775 feat/wire-park-stop stop_check >/dev/null 2>&1 || [ "$?" -eq 42 ]

  run cat "$BIN_LOG"
  [[ "$output" != *"postgresql://"* ]]
  [[ "$output" != *"secret"* ]]
}

@test "park and stop emit failures never alter authoritative return values" {
  _enable_emit
  cat > "$TMP/bin/autospec-db" <<'SH'
#!/usr/bin/env bash
exit 99
SH
  chmod +x "$TMP/bin/autospec-db"
  _install_usage_observe_stub

  run bash "$USAGE_GOVERNOR_SH" codex --repo-dir "$TMP/no-repo" --resume-at 2026-07-11T12:00:00Z
  [ "$status" -eq 0 ]
  [[ "$output" == *"park 2026-07-11T12:00:00Z"* ]]

  _write_immediate_stop_flag
  run bash "$STOP_CHECK_SH" 1775 feat/wire-park-stop stop_check
  [ "$status" -eq 42 ]
}

# --- telemetry-config.sh (issue #1776) ---------------------------------
# yaml resolver mirroring advisor-config.sh: env > yaml > built-in default.

TELCFG="$REPO_ROOT/skills/autospec-shared/scripts/telemetry-config.sh"

@test "telemetry-config.sh: exists and is bash -n clean" {
  [ -f "$TELCFG" ]
  run bash -n "$TELCFG"
  [ "$status" -eq 0 ]
}

@test "telemetry-config.sh: enabled false in yaml resolves to false" {
  cd "$TMP"
  mkdir -p .autospec
  cat > .autospec/autospec.yml <<'YAML'
telemetry:
  enabled: false
  host_label: 'site-a'
  spool_max_bytes: 555
  install:
    db_module: never
YAML
  run bash "$TELCFG" --key enabled
  [ "$status" -eq 0 ]
  [ "$output" = "false" ]
}

@test "telemetry-config.sh: host_label and spool_max_bytes map from yaml" {
  cd "$TMP"
  mkdir -p .autospec
  cat > .autospec/autospec.yml <<'YAML'
telemetry:
  enabled: true
  host_label: 'site-a'
  spool_max_bytes: 555
YAML
  run bash "$TELCFG" --key host_label
  [ "$status" -eq 0 ]
  [ "$output" = "site-a" ]

  run bash "$TELCFG" --key spool_max_bytes
  [ "$status" -eq 0 ]
  [ "$output" = "555" ]
}

@test "telemetry-config.sh: pre-set env wins over yaml" {
  cd "$TMP"
  mkdir -p .autospec
  cat > .autospec/autospec.yml <<'YAML'
telemetry:
  enabled: false
  host_label: 'from-yaml'
  spool_max_bytes: 999
YAML
  AUTOSPEC_TELEMETRY_ENABLED=true run bash "$TELCFG" --key enabled
  [ "$status" -eq 0 ]
  [ "$output" = "true" ]

  AUTOSPEC_DB_HOST_LABEL=from-env run bash "$TELCFG" --key host_label
  [ "$status" -eq 0 ]
  [ "$output" = "from-env" ]

  AUTOSPEC_DB_SPOOL_MAX_BYTES=42 run bash "$TELCFG" --key spool_max_bytes
  [ "$status" -eq 0 ]
  [ "$output" = "42" ]
}

@test "telemetry-config.sh: missing yaml file falls back to built-in defaults" {
  cd "$TMP"
  # no .autospec/autospec.yml at all
  run bash "$TELCFG" --key enabled
  [ "$status" -eq 0 ]
  [ "$output" = "true" ]

  run bash "$TELCFG" --key host_label
  [ "$status" -eq 0 ]
  [ "$output" = "" ]

  run bash "$TELCFG" --key spool_max_bytes
  [ "$status" -eq 0 ]
  [ "$output" = "10485760" ]

  run bash "$TELCFG" --key install.db_module
  [ "$status" -eq 0 ]
  [ "$output" = "prompt" ]
}

@test "telemetry-config.sh: yaml file present but missing telemetry block falls back to defaults" {
  cd "$TMP"
  mkdir -p .autospec
  cat > .autospec/autospec.yml <<'YAML'
advisor:
  policy: auto
YAML
  run bash "$TELCFG" --key enabled
  [ "$status" -eq 0 ]
  [ "$output" = "true" ]
}

# --- session bootstrap: ~/.autospec/env sources db.env + telemetry envs -
# Exercises the REAL install.sh ensure_autospec_bin_path function (extracted
# verbatim) rather than a reimplementation, so a bug in the actual heredoc
# would be caught here.

INSTALL_SH="$REPO_ROOT/install.sh"
INSTALL_HELPERS="$REPO_ROOT/scripts/lib/install-helpers.sh"

# Runs the real ensure_autospec_bin_path() against an isolated $TMP/home,
# with $TMP/home/.autospec/scripts/telemetry-config.sh installed so the
# generated env file's runtime calls resolve against our test fixtures.
run_ensure_autospec_bin_path() {
  mkdir -p "$TMP/home/.autospec/scripts" "$TMP/home/repo/.autospec"
  cp "$TELCFG" "$TMP/home/.autospec/scripts/telemetry-config.sh"
  FN_SNIPPET="$TMP/ensure_autospec_bin_path.sh"
  sed -n '/^ensure_autospec_bin_path()/,/^}/p' "$INSTALL_SH" > "$FN_SNIPPET"
  HOME="$TMP/home" DRY_RUN=0 bash -c '
    set -eu
    info() { :; }
    . "'"$INSTALL_HELPERS"'"
    . "'"$FN_SNIPPET"'"
    ensure_autospec_bin_path
  '
}

@test "session bootstrap: ~/.autospec/env sources db.env guarded by [ -f ]" {
  run_ensure_autospec_bin_path
  grep -q '\[ -f "\$HOME/.autospec/db.env" \] && \. "\$HOME/.autospec/db.env"' "$TMP/home/.autospec/env"
}

@test "session bootstrap: absent db.env leaves AUTOSPEC_DB_DSN unset" {
  run_ensure_autospec_bin_path
  rm -f "$TMP/home/.autospec/db.env"
  run bash -c '. "'"$TMP"'/home/.autospec/env"; echo "DSN=${AUTOSPEC_DB_DSN:-unset}"'
  [ "$status" -eq 0 ]
  [[ "$output" == *"DSN=unset"* ]]
}

@test "session bootstrap: enabled false in yaml exports AUTOSPEC_DB_DISABLE=1" {
  run_ensure_autospec_bin_path
  cat > "$TMP/home/repo/.autospec/autospec.yml" <<'YAML'
telemetry:
  enabled: false
  host_label: ''
  spool_max_bytes: 10485760
YAML
  run bash -c 'cd "'"$TMP"'/home/repo" && HOME="'"$TMP"'/home" . "'"$TMP"'/home/.autospec/env"; echo "DISABLE=${AUTOSPEC_DB_DISABLE:-unset}"'
  [ "$status" -eq 0 ]
  [[ "$output" == *"DISABLE=1"* ]]
}

@test "session bootstrap: a pre-set AUTOSPEC_DB_DISABLE wins over yaml enabled:true" {
  run_ensure_autospec_bin_path
  cat > "$TMP/home/repo/.autospec/autospec.yml" <<'YAML'
telemetry:
  enabled: true
YAML
  run bash -c 'cd "'"$TMP"'/home/repo" && HOME="'"$TMP"'/home" AUTOSPEC_DB_DISABLE=1 . "'"$TMP"'/home/.autospec/env"; echo "DISABLE=${AUTOSPEC_DB_DISABLE:-unset}"'
  [ "$status" -eq 0 ]
  [[ "$output" == *"DISABLE=1"* ]]
}

# --- Phase 5.5 integration audit (issue #1779) -------------------------
# Lock the cross-child telemetry contract in one place: when a new shim-backed
# emit kind is added, this test forces the shared wiring suite to cover it.

@test "integration audit: wiring suite covers every shim-backed telemetry kind" {
  actual="$TMP/actual-kinds.txt"
  expected="$TMP/expected-kinds.txt"
  find "$REPO_ROOT/scripts" "$REPO_ROOT/skills/autospec-run/scripts" "$REPO_ROOT/skills/autospec-shared/scripts" \
    -type f -name '*.sh' \
    ! -name 'emit-event.sh' \
    ! -name 'autospec-observatory-events.sh' \
    -print0 \
    | xargs -0 grep -hE '^[[:space:]}]*emit_event[[:space:]]+[A-Za-z0-9_.-]+' \
    | sed -E 's/^[[:space:]}]*emit_event[[:space:]]+([A-Za-z0-9_.-]+).*/\1/' \
    | sort -u > "$actual"

  cat > "$expected" <<'KINDS'
artifact.filed
claim
feature.described
heartbeat
session.parked
KINDS

  run diff -u "$expected" "$actual"
  [ "$status" -eq 0 ]

  coverage="$TMP/pre-audit-coverage.txt"
  awk '/^# --- Phase 5[.]5 integration audit/{exit} {print}' "$BATS_TEST_FILENAME" > "$coverage"

  cat > "$TMP/kind-coverage-patterns.txt" <<'PATTERNS'
artifact.filed|emit artifact.filed
claim|emit claim
feature.described|emit feature\.described
heartbeat|emit heartbeat
session.parked|emit session.parked
PATTERNS

  cut -d '|' -f 1 "$TMP/kind-coverage-patterns.txt" | sort -u > "$TMP/kind-coverage-kinds.txt"
  run diff -u "$expected" "$TMP/kind-coverage-kinds.txt"
  [ "$status" -eq 0 ]

  while IFS='|' read -r kind pattern; do
    grep -Fq "$kind" "$expected"
    grep -Fq "$pattern" "$coverage"
  done < "$TMP/kind-coverage-patterns.txt"
}

@test "integration audit: telemetry shim and chokepoints never invoke psql" {
  run bash -c "grep -RIn --include='*.sh' '^[^#]*\bpsql\b' '$REPO_ROOT/skills/autospec-shared/scripts/emit-event.sh' '$REPO_ROOT/scripts/autospec-run-registry.sh' '$REPO_ROOT/skills/autospec-shared/scripts/explore-ledger.sh' '$REPO_ROOT/skills/autospec-shared/scripts/growth-ledger.sh' '$REPO_ROOT/scripts/claim-guard.sh' '$REPO_ROOT/skills/autospec-shared/scripts/grow-define-file-issues.sh' '$REPO_ROOT/scripts/autonomous-usage-governor.sh' '$REPO_ROOT/scripts/autospec-stop-check.sh'"
  [ "$status" -eq 1 ]
  [ -z "$output" ]
}
