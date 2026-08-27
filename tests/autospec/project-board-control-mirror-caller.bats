#!/usr/bin/env bats
# tests/autospec/project-board-control-mirror-caller.bats
#
# Coverage for the conductor's Step 1c caller wiring in
# scripts/lib/autospec-loop.sh — the seam that invokes
# scripts/project-board-control-mirror.sh once per cycle, BEFORE Step 2's
# Tier-0 control-channel poll, so a board-level autospec:stop/pause/
# priority/steer reaches the fleet in the same cycle it was applied.
#
# Config reaches the caller via the Rust project-board-config bridge
# (`autospec autonomous project-board-config`) — never a new AUTOSPEC_*
# operator-facing env var. Tests stub that bridge binary (AUTOSPEC_QUEUE_BIN)
# to control what JSON the caller sees, and stub
# scripts/project-board-resolve.sh (AUTOSPEC_BOARD_RESOLVE_SCRIPT) to
# control what concrete repos the board resolves to.
#
# Safety properties under test:
#   1. control_issue set + glob allowlist -> mirror invoked with the
#      board's RESOLVED repos, never a literal glob pattern.
#   2. control_issue unset -> zero mirror invocations, zero gh calls, no
#      resolver invocation either.
#   3. mirror failure -> cycle proceeds unchanged, exit status unaffected.
#   4a. the control issue's own repo is outside the allowlist -> no
#       mirroring, no gh call naming it.
#   4b. a board-resolved repo is outside the allowlist -> present in the
#       resolved set, absent from every gh call (allowed repos still
#       reached).
#   5. mirrored labels are visible to the SAME cycle's Tier-0 read.
#   6. resolver failure, or an empty resolved repo set -> no mirror
#      invocation, cycle unaffected.
#
# All gh calls and helper scripts are stubbed via a fake PATH/scripts
# directory so no real GitHub calls are ever made.

bats_require_minimum_version 1.5.0

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  LOOP_LIB="$REPO_ROOT/scripts/lib/autospec-loop.sh"

  TEST_TMP="$(mktemp -d)"
  export HOME="$TEST_TMP"
  mkdir -p "$HOME/.autospec"
  export AUTOSPEC_CONFIG_FILE="$TEST_TMP/missing-autospec.yml"
  # Isolate the board-repos TTL cache per test (shares the directory layout
  # of scripts/autonomous-promote-open-issues.sh's board_plan(), keyed by
  # AUTOSPEC_STATE_DIR — default is $HOME, already test-isolated above, but
  # set it explicitly so the cache dir path is unambiguous in assertions).
  export AUTOSPEC_STATE_DIR="$TEST_TMP/state"
  mkdir -p "$AUTOSPEC_STATE_DIR"

  FAKE_SCRIPTS="$TEST_TMP/fake-scripts"
  mkdir -p "$FAKE_SCRIPTS"
  export AUTOSPEC_QUEUE_BIN="$FAKE_SCRIPTS/autospec"
  cp "$REPO_ROOT/scripts/autospec-runtime-config.sh" "$FAKE_SCRIPTS/autospec-runtime-config.sh"

  FAKE_BIN="$TEST_TMP/fake-bin"
  mkdir -p "$FAKE_BIN"
  export PATH="$FAKE_BIN:$PATH"

  cat > "$FAKE_BIN/notify.sh" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
  chmod +x "$FAKE_BIN/notify.sh"

  GH_CALLS="$TEST_TMP/gh-calls.log"
  MIRROR_LOG="$TEST_TMP/mirror.log"
  RESOLVE_LOG="$TEST_TMP/resolve.log"
  export GH_CALLS MIRROR_LOG RESOLVE_LOG

  export LOOP_LIB REPO_ROOT FAKE_SCRIPTS FAKE_BIN TEST_TMP
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

# The usual battery of passive stubs so a cycle completes with only the
# control-channel/mirror behavior under test varying between cases.
_install_passthrough_stubs() {
  _install_stub "autonomous-control-channel.sh" 'exit 0'
  _install_stub "autonomous-waterfall.sh" \
    'printf '"'"'{"tier":1,"action":"run-backlog","reason":"test"}\n'"'"''
  local gate_log="$TEST_TMP/gate.log"
  export GATE_LOG="$gate_log"
  _install_stub "autonomous-premerge-gate.sh" \
    "printf 'merge-ok\n'; printf 'gate-called\n' >> \"$gate_log\""
  _install_stub "autonomous-spend-ledger.sh" \
    'case "${1:-}" in add) exit 0;; check) printf "continue\n";; *) exit 0;; esac'
  _install_stub "autonomous-resilience.sh" \
    'case "${1:-}" in state) printf "DECISION:state-written\n";; lock) printf "DECISION:lock-acquired\nLOCK_SESSION:test\n";; *) exit 0;; esac'
  _install_stub "autospec-usage-limit.sh" 'exit 0'
}

# _install_resolve_stub REPOS_JSON — installs a fake
# project-board-resolve.sh that logs its invocation (args) to RESOLVE_LOG
# and, for `--emit repos`, prints REPOS_JSON verbatim.
_install_resolve_stub() {
  local repos_json="$1"
  cat > "$TEST_TMP/resolve.sh" <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$*" >> "$RESOLVE_LOG"
printf '%s\n' '$repos_json'
EOF
  chmod +x "$TEST_TMP/resolve.sh"
  export AUTOSPEC_BOARD_RESOLVE_SCRIPT="$TEST_TMP/resolve.sh"
}

# _install_failing_resolve_stub — a resolver that always fails.
_install_failing_resolve_stub() {
  cat > "$TEST_TMP/resolve.sh" <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$*" >> "$RESOLVE_LOG"
exit 1
EOF
  chmod +x "$TEST_TMP/resolve.sh"
  export AUTOSPEC_BOARD_RESOLVE_SCRIPT="$TEST_TMP/resolve.sh"
}

_run_one_cycle() {
  run bash -c "
    . '$LOOP_LIB'
    CONDUCTOR_SCRIPTS_DIR='$FAKE_SCRIPTS' \
    CONDUCTOR_REPO='${CONDUCTOR_REPO:-o/target}' \
    CONDUCTOR_MAX_CYCLES=1 \
    CONDUCTOR_POLL_INTERVAL=0 \
    CONDUCTOR_DRY_RUN=0 \
    CONDUCTOR_NO_DIGEST=1 \
    AUTOSPEC_BOARD_RESOLVE_SCRIPT='${AUTOSPEC_BOARD_RESOLVE_SCRIPT:-}' \
    autospec_conductor_run
  " 2>&1
}

# ── 1. glob allowlist resolves to concrete repos; no literal '*' ever reaches gh ─
@test "conductor: a glob allowlist resolves through the board to concrete repos, never a literal glob" {
  cp "$REPO_ROOT/scripts/project-board-control-mirror.sh" "$FAKE_SCRIPTS/project-board-control-mirror.sh"
  chmod +x "$FAKE_SCRIPTS/project-board-control-mirror.sh"

  cat > "$FAKE_SCRIPTS/autospec" <<'EOF'
#!/usr/bin/env bash
if [ "${1:-}" = "autonomous" ] && [ "${2:-}" = "project-board-config" ]; then
  printf '{"url":"https://github.com/orgs/o/projects/1","allowlist":["o/*"],"control_issue":"o/ctl#9","ttl":300}\n'
  exit 0
fi
exit 0
EOF
  chmod +x "$FAKE_SCRIPTS/autospec"

  _install_resolve_stub '["o/a","o/b"]'

  cat > "$FAKE_BIN/gh" <<EOF
#!/usr/bin/env bash
printf 'gh %s\n' "\$*" >> "$GH_CALLS"
case "\$*" in
  *"issue view"*) printf '{"labels":[{"name":"autospec:stop"}]}\n' ;;
  *"issue list"*) printf '[]' ;;
  *) exit 1 ;;
esac
EOF
  chmod +x "$FAKE_BIN/gh"

  _install_passthrough_stubs
  _run_one_cycle

  [ "$status" -eq 0 ]
  [ -f "$RESOLVE_LOG" ]
  grep -qF -- '--url https://github.com/orgs/o/projects/1' "$RESOLVE_LOG"
  grep -qF -- '--emit repos' "$RESOLVE_LOG"

  [ -f "$GH_CALLS" ]
  # The strongest form of the proof: no gh call, anywhere, names a literal
  # '*' — the allowlist's glob pattern must never reach the GitHub API.
  if grep -qF '*' "$GH_CALLS"; then
    false
  fi
  # And the resolved concrete repos DID get named (the control issue's
  # labels carry autospec:stop, so the mirror proceeds past the label
  # check into per-repo marker lookup/create for o/a and o/b).
  grep -q 'o/a' "$GH_CALLS"
  grep -q 'o/b' "$GH_CALLS"
}

# ── 2. control_issue unset: zero mirror invocations, zero gh calls, no resolve ──
@test "conductor: control_issue unset makes zero mirror invocations, zero resolves, and zero gh calls" {
  cat > "$FAKE_SCRIPTS/autospec" <<'EOF'
#!/usr/bin/env bash
if [ "${1:-}" = "autonomous" ] && [ "${2:-}" = "project-board-config" ]; then
  printf '{"url":null,"allowlist":[],"control_issue":null}\n'
  exit 0
fi
exit 0
EOF
  chmod +x "$FAKE_SCRIPTS/autospec"

  cat > "$FAKE_SCRIPTS/project-board-control-mirror.sh" <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$*" >> "$MIRROR_LOG"
printf '{"mirrored":[],"skipped":[],"failed":[]}\n'
exit 0
EOF
  chmod +x "$FAKE_SCRIPTS/project-board-control-mirror.sh"

  _install_resolve_stub '["o/a"]'

  cat > "$FAKE_BIN/gh" <<EOF
#!/usr/bin/env bash
printf 'gh %s\n' "\$*" >> "$GH_CALLS"
case "\$*" in *"issue list"*) printf '[]' ;; *) printf '' ;; esac
EOF
  chmod +x "$FAKE_BIN/gh"

  _install_passthrough_stubs
  _run_one_cycle

  [ "$status" -eq 0 ]
  if [ -f "$MIRROR_LOG" ]; then
    false
  fi
  if [ -f "$RESOLVE_LOG" ]; then
    false
  fi
  if [ -f "$GH_CALLS" ]; then
    local gh_lines
    gh_lines="$(wc -l < "$GH_CALLS" | tr -d ' ')"
    [ "$gh_lines" -eq 0 ]
  fi
}

# ── 3. mirror failure never blocks/alters the cycle ─────────────────────────
@test "conductor: a mirror failure does not fail, delay, or alter the cycle" {
  cat > "$FAKE_SCRIPTS/autospec" <<'EOF'
#!/usr/bin/env bash
if [ "${1:-}" = "autonomous" ] && [ "${2:-}" = "project-board-config" ]; then
  printf '{"url":"https://github.com/orgs/o/projects/1","allowlist":["o/r1"],"control_issue":"o/ctl#9"}\n'
  exit 0
fi
if [ "${1:-}" = "queue" ] && [ "${2:-}" = "ready" ]; then
  printf '{"ready":[{"number":1886}],"blocked":[],"claimed":[],"conflicts":[],"worker_cap":{"reached":false},"batch":[{"number":1886}]}\n'
  exit 0
fi
exit 0
EOF
  chmod +x "$FAKE_SCRIPTS/autospec"

  _install_resolve_stub '["o/r1"]'

  cat > "$FAKE_SCRIPTS/project-board-control-mirror.sh" <<EOF
#!/usr/bin/env bash
printf 'called\n' >> "$MIRROR_LOG"
exit 1
EOF
  chmod +x "$FAKE_SCRIPTS/project-board-control-mirror.sh"

  _install_passthrough_stubs

  local run_log="$TEST_TMP/run.log"
  export AUTOSPEC_RUN_CMD="printf 'autospec-run-called\n' >> '$run_log'"

  _run_one_cycle

  [ "$status" -eq 0 ]
  [ -f "$MIRROR_LOG" ]
  [ -f "$GATE_LOG" ]
  grep -qF 'gate-called' "$GATE_LOG"
}

# ── 4a. the control issue's own repo is outside the allowlist ───────────────
@test "conductor: the control issue's own out-of-allowlist repo makes no gh call naming it" {
  cp "$REPO_ROOT/scripts/project-board-control-mirror.sh" "$FAKE_SCRIPTS/project-board-control-mirror.sh"
  chmod +x "$FAKE_SCRIPTS/project-board-control-mirror.sh"

  cat > "$FAKE_SCRIPTS/autospec" <<'EOF'
#!/usr/bin/env bash
if [ "${1:-}" = "autonomous" ] && [ "${2:-}" = "project-board-config" ]; then
  printf '{"url":"https://github.com/orgs/o/projects/1","allowlist":["o/target"],"control_issue":"outside/repo#1"}\n'
  exit 0
fi
exit 0
EOF
  chmod +x "$FAKE_SCRIPTS/autospec"

  # Repos DO resolve (the board has real, in-scope repos) — the gate under
  # test is the mirror script's own allowed(ctl_repo) check, not an empty
  # repo set masking the behavior.
  _install_resolve_stub '["o/target"]'

  cat > "$FAKE_BIN/gh" <<EOF
#!/usr/bin/env bash
printf 'gh %s\n' "\$*" >> "$GH_CALLS"
exit 1
EOF
  chmod +x "$FAKE_BIN/gh"

  _install_passthrough_stubs
  _run_one_cycle

  [ "$status" -eq 0 ]
  # The mirror script's allowed(ctl_repo) gate runs BEFORE any gh call, so
  # the strongest outcome here — zero gh calls at all, not merely zero
  # calls naming the out-of-scope repo — is entirely plausible; tolerate a
  # missing log (no gh invocation happened) as well as an empty one.
  if [ -f "$GH_CALLS" ]; then
    if grep -q 'outside/repo' "$GH_CALLS"; then
      false
    fi
    local gh_lines
    gh_lines="$(wc -l < "$GH_CALLS" | tr -d ' ')"
    [ "$gh_lines" -eq 0 ]
  fi
}

# ── 4b. a board-resolved repo outside the allowlist is skipped, others reached ──
@test "conductor: a board-resolved repo outside the allowlist is skipped while allowed repos are still reached" {
  cp "$REPO_ROOT/scripts/project-board-control-mirror.sh" "$FAKE_SCRIPTS/project-board-control-mirror.sh"
  chmod +x "$FAKE_SCRIPTS/project-board-control-mirror.sh"

  cat > "$FAKE_SCRIPTS/autospec" <<'EOF'
#!/usr/bin/env bash
if [ "${1:-}" = "autonomous" ] && [ "${2:-}" = "project-board-config" ]; then
  printf '{"url":"https://github.com/orgs/o/projects/1","allowlist":["o/a"],"control_issue":"o/a#1"}\n'
  exit 0
fi
exit 0
EOF
  chmod +x "$FAKE_SCRIPTS/autospec"

  # The board itself resolves to a repo (o/b) that is NOT in the operator's
  # allowlist — a real, live board can legitimately contain repos the
  # operator never opted into mirroring for.
  _install_resolve_stub '["o/a","o/b"]'

  cat > "$FAKE_BIN/gh" <<EOF
#!/usr/bin/env bash
printf 'gh %s\n' "\$*" >> "$GH_CALLS"
case "\$*" in *"issue list"*) printf '[]' ;; *) printf '' ;; esac
EOF
  chmod +x "$FAKE_BIN/gh"

  _install_passthrough_stubs
  _run_one_cycle

  [ "$status" -eq 0 ]
  [ -f "$GH_CALLS" ]
  if grep -q 'o/b' "$GH_CALLS"; then
    false
  fi
  grep -q 'o/a' "$GH_CALLS"
}

# ── 5. mirrored labels are visible to the SAME cycle's Tier-0 read ──────────
@test "conductor: a mirrored autospec:stop label is honored in the same cycle" {
  cp "$REPO_ROOT/scripts/project-board-control-mirror.sh" "$FAKE_SCRIPTS/project-board-control-mirror.sh"
  chmod +x "$FAKE_SCRIPTS/project-board-control-mirror.sh"

  cat > "$FAKE_SCRIPTS/autospec" <<'EOF'
#!/usr/bin/env bash
if [ "${1:-}" = "autonomous" ] && [ "${2:-}" = "project-board-config" ]; then
  printf '{"url":"https://github.com/orgs/o/projects/1","allowlist":["o/target"],"control_issue":"o/target#1"}\n'
  exit 0
fi
exit 0
EOF
  chmod +x "$FAKE_SCRIPTS/autospec"

  _install_resolve_stub '["o/target"]'

  GH_STATE="$TEST_TMP/gh-state"
  mkdir -p "$GH_STATE"
  export GH_STATE

  cat > "$FAKE_BIN/gh" <<'GHEOF'
#!/usr/bin/env bash
printf 'gh %s\n' "$*" >> "$GH_CALLS"

_find_label() {
  local label=""
  local prev=""
  for arg in "$@"; do
    if [ "$prev" = "--label" ] || [ "$prev" = "--add-label" ]; then
      label="$arg"
    fi
    prev="$arg"
  done
  printf '%s' "$label"
}

case "${1:-}" in
  issue)
    case "${2:-}" in
      view)
        printf '{"labels":[{"name":"autospec:stop"}]}\n'
        ;;
      list)
        label="$(_find_label "$@")"
        if [ "$label" = "autospec:project-board-marker" ]; then
          if [ -f "$GH_STATE/marker.json" ]; then
            cat "$GH_STATE/marker.json"
          else
            printf '[]'
          fi
        elif [ -n "$label" ]; then
          if [ -f "$GH_STATE/label-${label}.flag" ]; then
            printf '[{"number":42,"title":"marker","body":"","author":{"login":"bot"}}]'
          else
            printf '[]'
          fi
        else
          printf '[]'
        fi
        ;;
      create)
        printf '[{"number":100,"title":"[autospec] project-board control relay (do not edit manually)"}]\n' \
          > "$GH_STATE/marker.json"
        prev=""
        for arg in "$@"; do
          if [ "$prev" = "--label" ] && [ "$arg" != "autospec:project-board-marker" ]; then
            : > "$GH_STATE/label-${arg}.flag"
          fi
          prev="$arg"
        done
        printf '{"number":100}\n'
        ;;
      edit)
        prev=""
        for arg in "$@"; do
          if [ "$prev" = "--add-label" ]; then
            : > "$GH_STATE/label-${arg}.flag"
          fi
          prev="$arg"
        done
        ;;
      *) exit 0 ;;
    esac
    ;;
  repo) printf '{"nameWithOwner":"o/target"}\n' ;;
  *) exit 0 ;;
esac
GHEOF
  chmod +x "$FAKE_BIN/gh"

  CONDUCTOR_REPO="o/target"
  export CONDUCTOR_REPO
  _install_passthrough_stubs
  # This test needs the REAL control-channel.sh (copied here) to prove
  # same-cycle visibility, so re-install it after the passive-stub battery
  # above (which would otherwise clobber it with the 'exit 0' stub).
  cp "$REPO_ROOT/scripts/autonomous-control-channel.sh" "$FAKE_SCRIPTS/autonomous-control-channel.sh"
  chmod +x "$FAKE_SCRIPTS/autonomous-control-channel.sh"
  _run_one_cycle

  [ "$status" -eq 0 ]
  grep -qF 'DECISION:graceful-stop received' <<< "$output"
}

# ── 6a. resolver failure: no mirror invocation, cycle unaffected ────────────
@test "conductor: a resolver failure makes zero mirror invocations and leaves the cycle unaffected" {
  cat > "$FAKE_SCRIPTS/autospec" <<'EOF'
#!/usr/bin/env bash
if [ "${1:-}" = "autonomous" ] && [ "${2:-}" = "project-board-config" ]; then
  printf '{"url":"https://github.com/orgs/o/projects/1","allowlist":["o/r1"],"control_issue":"o/ctl#9"}\n'
  exit 0
fi
if [ "${1:-}" = "queue" ] && [ "${2:-}" = "ready" ]; then
  printf '{"ready":[{"number":1886}],"blocked":[],"claimed":[],"conflicts":[],"worker_cap":{"reached":false},"batch":[{"number":1886}]}\n'
  exit 0
fi
exit 0
EOF
  chmod +x "$FAKE_SCRIPTS/autospec"

  _install_failing_resolve_stub

  cat > "$FAKE_SCRIPTS/project-board-control-mirror.sh" <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$*" >> "$MIRROR_LOG"
printf '{"mirrored":[],"skipped":[],"failed":[]}\n'
exit 0
EOF
  chmod +x "$FAKE_SCRIPTS/project-board-control-mirror.sh"

  _install_passthrough_stubs
  _run_one_cycle

  [ "$status" -eq 0 ]
  [ -f "$RESOLVE_LOG" ]
  if [ -f "$MIRROR_LOG" ]; then
    false
  fi
  [ -f "$GATE_LOG" ]
  grep -qF 'gate-called' "$GATE_LOG"
}

# ── 6b. resolver returns an empty repo set: no mirror invocation ────────────
@test "conductor: an empty resolved repo set makes zero mirror invocations and leaves the cycle unaffected" {
  cat > "$FAKE_SCRIPTS/autospec" <<'EOF'
#!/usr/bin/env bash
if [ "${1:-}" = "autonomous" ] && [ "${2:-}" = "project-board-config" ]; then
  printf '{"url":"https://github.com/orgs/o/projects/1","allowlist":["o/r1"],"control_issue":"o/ctl#9"}\n'
  exit 0
fi
if [ "${1:-}" = "queue" ] && [ "${2:-}" = "ready" ]; then
  printf '{"ready":[{"number":1886}],"blocked":[],"claimed":[],"conflicts":[],"worker_cap":{"reached":false},"batch":[{"number":1886}]}\n'
  exit 0
fi
exit 0
EOF
  chmod +x "$FAKE_SCRIPTS/autospec"

  _install_resolve_stub '[]'

  cat > "$FAKE_SCRIPTS/project-board-control-mirror.sh" <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$*" >> "$MIRROR_LOG"
printf '{"mirrored":[],"skipped":[],"failed":[]}\n'
exit 0
EOF
  chmod +x "$FAKE_SCRIPTS/project-board-control-mirror.sh"

  _install_passthrough_stubs
  _run_one_cycle

  [ "$status" -eq 0 ]
  [ -f "$RESOLVE_LOG" ]
  if [ -f "$MIRROR_LOG" ]; then
    false
  fi
  [ -f "$GATE_LOG" ]
  grep -qF 'gate-called' "$GATE_LOG"
}
