#!/usr/bin/env bats
# tests/autonomous/test_conductor_provenance_dispatch.bats — dispatch-time
# provenance split in autospec_conductor_run()
# (docs/specs/2026-07-10-autonomous-integration-branch-design.md,
# §Architecture item 5).
#
# Covers:
#   1. Self batch: integration-branch ensure runs; run-cmd dispatched with a
#      kind=integration mode file active and AUTOSPEC_RUN_ONLY_ISSUES set.
#   2. Operator batch: a kind=integration mode file is parked before dispatch;
#      run-cmd sees no mode file (Phase 4 targets the parent as today).
#   3. Mixed batch: two dispatches — operator subset (no mode file) then self
#      subset (kind=integration) — with the correct issue subsets.
#   4. Provenance resolver failure → treated self (fail closed).
#   5. sync exit 65 → self subset parked + notify.sh +
#      code_health:integration_sync_conflict; operator subset unaffected.
#   6. No resolver/integration scripts → single dispatch (back-compat).
#   7. A kind=explore mode file is never parked by an operator batch.
#   8. Tier 2 discovery entry ensures the integration branch as the sandbox
#      (mode file kind=integration; ephemeral explore sandbox skipped).
#   9. Tier 2 falls back to explore-sandbox.sh --base main when integration
#      routing is unavailable.
#
# Mocking strategy mirrors tests/autonomous/test_loop_growth_dispatch.bats:
# helper scripts stubbed via CONDUCTOR_SCRIPTS_DIR; gh stubbed via a fake PATH
# dir; notify.sh resolved script-relative; bash 3.2-safe (no process
# substitution; fixtures written to real temp files). No real GitHub calls.

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  LOOP_LIB="$REPO_ROOT/scripts/lib/autospec-loop.sh"

  TEST_TMP="$(mktemp -d)"
  export HOME="$TEST_TMP"
  mkdir -p "$HOME/.autospec"
  unset AUTOSPEC_RUN_ONLY_ISSUES
  unset AUTOSPEC_PROVENANCE_BIN
  unset AUTOSPEC_INTEGRATION_BRANCH_BIN

  FAKE_SCRIPTS="$TEST_TMP/fake-scripts"
  mkdir -p "$FAKE_SCRIPTS"
  cp "$REPO_ROOT/scripts/autospec-runtime-config.sh" "$FAKE_SCRIPTS/autospec-runtime-config.sh"

  FAKE_BIN="$TEST_TMP/fake-bin"
  mkdir -p "$FAKE_BIN"
  export PATH="$FAKE_BIN:$PATH"

  cat > "$FAKE_BIN/gh" <<'EOF'
#!/usr/bin/env bash
if [ "${1:-}" = "repo" ] && [ "${2:-}" = "view" ]; then
  case "$*" in
    *defaultBranchRef*) printf '%s\n' "${AUTOSPEC_TEST_DEFAULT_BRANCH:-main}" ;;
    *) echo '{"nameWithOwner":"test-owner/test-repo"}' ;;
  esac
elif [ "${1:-}" = "issue" ]; then
  echo "[]"
else
  exit 0
fi
EOF
  chmod +x "$FAKE_BIN/gh"

  # notify.sh — resolved via ${_sdir}/../skills/autospec-shared/scripts/notify.sh
  # (_sdir = CONDUCTOR_SCRIPTS_DIR = $TEST_TMP/fake-scripts).
  NOTIFY_LOG="$TEST_TMP/notify.log"
  mkdir -p "$TEST_TMP/skills/autospec-shared/scripts"
  cat > "$TEST_TMP/skills/autospec-shared/scripts/notify.sh" <<EOF
#!/usr/bin/env bash
printf '%s | %s\n' "\${1:-}" "\${2:-}" >> "$NOTIFY_LOG"
exit 0
EOF
  chmod +x "$TEST_TMP/skills/autospec-shared/scripts/notify.sh"

  # Mode file location: _repo_root = parent of CONDUCTOR_SCRIPTS_DIR = TEST_TMP.
  MODE_FILE="$TEST_TMP/.autospec/explore-mode.json"
  mkdir -p "$TEST_TMP/.autospec"

  RUN_CMD_LOG="$TEST_TMP/run-cmd.log"
  INT_CALL_LOG="$TEST_TMP/intbranch-calls.log"
  PROV_CALL_LOG="$TEST_TMP/prov-calls.log"
  SANDBOX_CALL_LOG="$TEST_TMP/sandbox-calls.log"
  touch "$RUN_CMD_LOG" "$INT_CALL_LOG" "$PROV_CALL_LOG" "$SANDBOX_CALL_LOG"

  # run-cmd stub: records the issue subset + mode-file kind at invocation time.
  cat > "$TEST_TMP/run-cmd.sh" <<EOF
#!/usr/bin/env bash
if [ -f "$MODE_FILE" ]; then
  _kind="\$(jq -r '.kind // "none"' "$MODE_FILE" 2>/dev/null || echo parse-error)"
else
  _kind="absent"
fi
printf 'issues=%s kind=%s\n' "\${AUTOSPEC_RUN_ONLY_ISSUES:-}" "\$_kind" >> "$RUN_CMD_LOG"
EOF
  chmod +x "$TEST_TMP/run-cmd.sh"
  export AUTOSPEC_RUN_CMD="bash $TEST_TMP/run-cmd.sh"

  export LOOP_LIB REPO_ROOT FAKE_SCRIPTS TEST_TMP FAKE_BIN \
    MODE_FILE RUN_CMD_LOG INT_CALL_LOG PROV_CALL_LOG SANDBOX_CALL_LOG NOTIFY_LOG
}

teardown() {
  rm -rf "$TEST_TMP" 2>/dev/null || true
}

_install_stub() {
  local name="$1"
  local body="$2"
  printf '#!/usr/bin/env bash\n%s\n' "$body" > "$FAKE_SCRIPTS/$name"
  chmod +x "$FAKE_SCRIPTS/$name"
}

_install_common_stubs() {
  _install_stub "autonomous-control-channel.sh" 'exit 0'
  _install_stub "autonomous-premerge-gate.sh" 'printf "merge-ok\n"'
  _install_stub "autonomous-spend-ledger.sh" \
    'case "${1:-}" in add) exit 0;; check) printf "continue\n";; *) exit 0;; esac'
  _install_stub "autonomous-resilience.sh" \
    'case "${1:-}" in state) printf "DECISION:state-written\n";; lock) printf "DECISION:lock-acquired\nLOCK_SESSION:test\n";; *) exit 0;; esac'
  _install_stub "autospec-usage-limit.sh" 'exit 0'
  _install_stub "autonomous-waterfall.sh" \
    'printf '\''{"tier":1,"action":"run-backlog","reason":"test"}\n'\'''
}

# list-ready mock: ready/batch carry the given comma-separated issue numbers.
_install_list_ready() {
  local numbers_csv="$1"
  cat > "$FAKE_SCRIPTS/list-ready-issues.sh" <<EOF
#!/usr/bin/env bash
jq -cn '{
  ready: ([$numbers_csv] | map({number: .})),
  blocked: [], claimed: [], conflicts: [],
  worker_cap: {max_repo_workers: 0, active_count: 0, remaining: 0, reached: false},
  batch: ([$numbers_csv] | map({number: .}))
}'
EOF
  chmod +x "$FAKE_SCRIPTS/list-ready-issues.sh"
}

# provenance mock: issues 1xx -> self, 2xx -> operator, 9xx -> crash (exit 3,
# no stdout — the conductor must fail closed to self).
_install_provenance() {
  cat > "$FAKE_SCRIPTS/autonomous-provenance.sh" <<EOF
#!/usr/bin/env bash
issue=""
while [ \$# -gt 0 ]; do
  case "\$1" in
    --issue) shift; issue="\${1:-}" ;;
  esac
  shift
done
printf 'resolve %s\n' "\$issue" >> "$PROV_CALL_LOG"
case "\$issue" in
  1*) printf 'self\n' ;;
  2*) printf 'operator\n' ;;
  9*) echo "provenance boom" >&2; exit 3 ;;
  *)  printf 'operator\n' ;;
esac
EOF
  chmod +x "$FAKE_SCRIPTS/autonomous-provenance.sh"
}

# integration-branch mock: logs args; `ensure` writes a kind=integration mode
# file (mirrors the real script's write_mode_file); `sync` exits with the rc
# recorded in $TEST_TMP/sync-rc (default 0, 65 = merge conflict).
_install_intbranch() {
  cat > "$FAKE_SCRIPTS/autonomous-integration-branch.sh" <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$*" >> "$INT_CALL_LOG"
case "\${1:-}" in
  ensure)
    mkdir -p "$TEST_TMP/.autospec"
    printf '{"branch":"autospec/autonomous-main","slug":"test-owner/test-repo","base":"main","head_sha":"abc123","kind":"integration"}\n' > "$MODE_FILE"
    ;;
  sync)
    rc="\$(cat "$TEST_TMP/sync-rc" 2>/dev/null || printf '0')"
    exit "\$rc"
    ;;
esac
exit 0
EOF
  chmod +x "$FAKE_SCRIPTS/autonomous-integration-branch.sh"
}

# explore-sandbox mock (Tier 2/3 fallback seam) — logs args, writes an
# explore-kind mode file like the real script.
_install_sandbox_bin() {
  cat > "$TEST_TMP/explore-sandbox.sh" <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$*" >> "$SANDBOX_CALL_LOG"
mkdir -p "$TEST_TMP/.autospec"
printf '{"branch":"autospec/explore/2026-07-10-test","slug":"test","base":"main","head_sha":"abc","created_at":"2026-07-10T00:00:00Z"}\n' > "$MODE_FILE"
exit 0
EOF
  chmod +x "$TEST_TMP/explore-sandbox.sh"
  export AUTOSPEC_SANDBOX_BIN="$TEST_TMP/explore-sandbox.sh"
}

_run_cycle() {
  run bash -c "
    . '$LOOP_LIB'
    CONDUCTOR_SCRIPTS_DIR='$FAKE_SCRIPTS' \
    CONDUCTOR_REPO='test-owner/test-repo' \
    CONDUCTOR_MAX_CYCLES=1 \
    CONDUCTOR_POLL_INTERVAL=0 \
    CONDUCTOR_DRY_RUN=0 \
    CONDUCTOR_NO_DIGEST=1 \
    autospec_conductor_run
  " 2>&1
}

# ── 1. Self batch ─────────────────────────────────────────────────────────────

@test "self batch: ensure runs and dispatch sees kind=integration mode file with self subset" {
  _install_common_stubs
  _install_provenance
  _install_intbranch
  _install_list_ready "101,102"

  _run_cycle

  [ "$status" -eq 0 ]
  grep -q '^ensure --parent main' "$INT_CALL_LOG"
  grep -q '^issues=101 102 kind=integration$' "$RUN_CMD_LOG"
  [ "$(grep -c . "$RUN_CMD_LOG")" -eq 1 ]
}

@test "self batch: mode file persists with kind=integration after dispatch" {
  _install_common_stubs
  _install_provenance
  _install_intbranch
  _install_list_ready "101"

  _run_cycle

  [ "$status" -eq 0 ]
  [ -f "$MODE_FILE" ]
  [ "$(jq -r '.kind // empty' "$MODE_FILE")" = "integration" ]
}


@test "non-main default branch: integration calls use derived parent trunk" {
  _install_common_stubs
  _install_provenance
  _install_intbranch
  _install_list_ready "101"
  export AUTOSPEC_TEST_DEFAULT_BRANCH="trunk"

  _run_cycle

  [ "$status" -eq 0 ]
  grep -q '^status --parent trunk' "$INT_CALL_LOG"
  grep -q '^ensure --parent trunk' "$INT_CALL_LOG"
  grep -q '^sync --parent trunk' "$INT_CALL_LOG"
  ! grep -q -- '--parent main' "$INT_CALL_LOG"
}

# ── 2. Operator batch ─────────────────────────────────────────────────────────

@test "operator batch: parks a kind=integration mode file and dispatches to parent" {
  _install_common_stubs
  _install_provenance
  _install_intbranch
  _install_list_ready "201,202"
  printf '{"branch":"autospec/autonomous-main","base":"main","kind":"integration"}\n' > "$MODE_FILE"

  _run_cycle

  [ "$status" -eq 0 ]
  grep -q '^issues=201 202 kind=absent$' "$RUN_CMD_LOG"
  [ "$(grep -c . "$RUN_CMD_LOG")" -eq 1 ]
  [ ! -f "$MODE_FILE" ]
  [ -f "$MODE_FILE.parked" ]
}

@test "operator batch: never parks a kind=explore mode file (standalone explore untouched)" {
  _install_common_stubs
  _install_provenance
  _install_intbranch
  _install_list_ready "201"
  printf '{"branch":"autospec/explore/2026-07-10-x","base":"main","kind":"explore"}\n' > "$MODE_FILE"

  _run_cycle

  [ "$status" -eq 0 ]
  grep -q '^issues=201 kind=explore$' "$RUN_CMD_LOG"
  [ -f "$MODE_FILE" ]
  [ ! -f "$MODE_FILE.parked" ]
}

# ── 3. Mixed batch ────────────────────────────────────────────────────────────

@test "mixed batch: operator subset dispatches to parent, then self subset to integration branch" {
  _install_common_stubs
  _install_provenance
  _install_intbranch
  _install_list_ready "101,202"

  _run_cycle

  [ "$status" -eq 0 ]
  [ "$(grep -c . "$RUN_CMD_LOG")" -eq 2 ]
  head -1 "$RUN_CMD_LOG" | grep -q '^issues=202 kind=absent$'
  tail -1 "$RUN_CMD_LOG" | grep -q '^issues=101 kind=integration$'
}

@test "mixed batch: provenance re-resolved from GitHub per issue each cycle (no session memory)" {
  _install_common_stubs
  _install_provenance
  _install_intbranch
  _install_list_ready "101,202"

  _run_cycle

  [ "$status" -eq 0 ]
  grep -q '^resolve 101$' "$PROV_CALL_LOG"
  grep -q '^resolve 202$' "$PROV_CALL_LOG"
}

# ── 4. Resolver failure → fail closed ─────────────────────────────────────────

@test "provenance resolver crash fails closed: issue treated as self" {
  _install_common_stubs
  _install_provenance
  _install_intbranch
  _install_list_ready "901"

  _run_cycle

  [ "$status" -eq 0 ]
  grep -q '^issues=901 kind=integration$' "$RUN_CMD_LOG"
}

# ── 5. Sync conflict (exit 65) parks the self subset ──────────────────────────

@test "sync exit 65: self subset parked with code_health marker + notification; operator subset unaffected" {
  _install_common_stubs
  _install_provenance
  _install_intbranch
  _install_list_ready "101,202"
  printf '65\n' > "$TEST_TMP/sync-rc"

  _run_cycle

  [ "$status" -eq 0 ]
  [[ "$output" == *"code_health:integration_sync_conflict"* ]]
  # Operator subset still dispatched to the parent.
  grep -q '^issues=202 kind=absent$' "$RUN_CMD_LOG"
  # Self subset NOT dispatched.
  ! grep -q 'issues=101' "$RUN_CMD_LOG"
  # Notification fired.
  grep -q 'sync conflict' "$NOTIFY_LOG"
  # The kind=integration mode file written by ensure must be parked, so no
  # later dispatch routes work onto the conflicted integration branch.
  [ ! -f "$MODE_FILE" ]
}

@test "ensure failure (non-65): self subset parked, no dispatch onto parent" {
  _install_common_stubs
  _install_provenance
  _install_list_ready "101"
  # ensure fails with the real script's mode-conflict exit code.
  _install_stub "autonomous-integration-branch.sh" 'exit 6'

  _run_cycle

  [ "$status" -eq 0 ]
  # Self issue must not be dispatched anywhere (fail closed, never to parent).
  ! grep -q 'issues=101' "$RUN_CMD_LOG"
}

# ── 6. Back-compat: split inactive without the two scripts ────────────────────

@test "no provenance/integration scripts: single dispatch with no subset filter" {
  _install_common_stubs
  _install_list_ready "101,202"

  _run_cycle

  [ "$status" -eq 0 ]
  grep -q '^issues= kind=absent$' "$RUN_CMD_LOG"
  [ "$(grep -c . "$RUN_CMD_LOG")" -eq 1 ]
}

# ── 8. Tier 2/3: integration branch as the conductor-driven discovery sandbox ─

@test "Tier 2 entry ensures the integration branch as sandbox (kind=integration, ephemeral sandbox skipped)" {
  _install_common_stubs
  _install_provenance
  _install_intbranch
  _install_sandbox_bin
  _install_stub "autonomous-waterfall.sh" \
    'printf '\''{"tier":2,"action":"run-explore-once","reason":"tier2-test"}\n'\'''
  export AUTOSPEC_EXPLORE_CMD="printf '{\"tier\":\"local\",\"proposals_seen\":0,\"new_candidates\":0,\"filed\":0,\"dry\":true,\"reason\":\"test-dry\"}\n'"

  _run_cycle

  [ "$status" -eq 0 ]
  grep -q '^ensure --parent main' "$INT_CALL_LOG"
  [ ! -s "$SANDBOX_CALL_LOG" ]
  [ "$(jq -r '.kind // empty' "$MODE_FILE")" = "integration" ]
}

@test "Tier 2 entry falls back to explore-sandbox.sh --base main when integration routing unavailable" {
  _install_common_stubs
  _install_sandbox_bin
  _install_stub "autonomous-waterfall.sh" \
    'printf '\''{"tier":2,"action":"run-explore-once","reason":"tier2-test"}\n'\'''
  export AUTOSPEC_EXPLORE_CMD="printf '{\"tier\":\"local\",\"proposals_seen\":0,\"new_candidates\":0,\"filed\":0,\"dry\":true,\"reason\":\"test-dry\"}\n'"

  _run_cycle

  [ "$status" -eq 0 ]
  grep -q -- '--base main' "$SANDBOX_CALL_LOG"
}
