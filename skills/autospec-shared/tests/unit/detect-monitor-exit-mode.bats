#!/usr/bin/env bats
# detect-monitor-exit-mode.bats — Unit tests for detect-monitor-exit-mode.sh
#
# The detector inspects worktree state, label/PR state, and the latest
# heartbeat tool-call count, then prints one of:
#   silent-exit | prompt-overflow | clean | unknown
#
# Driven by docs/memory/feedback_monitor_silent_exit.md (the two known
# Phase-4 monitor exit modes: silent-exit + prompt-overflow).
#
# Run: bats skills/autospec-shared/tests/unit/detect-monitor-exit-mode.bats

SCRIPT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)/scripts/detect-monitor-exit-mode.sh"

setup() {
  TMP_DIR="$(mktemp -d /tmp/autospec-detecttest-XXXXXX)"
  export HOME="$TMP_DIR/home"
  mkdir -p "$HOME"

  # Sandbox the heartbeat root and worktree base so we never touch real state.
  export AUTOSPEC_WATCHDOG_DIR="$HOME/.autospec/process-heartbeats"
  export AUTOSPEC_WORKTREE_BASE="$TMP_DIR/wt"
  mkdir -p "$AUTOSPEC_WORKTREE_BASE"

  REPO="berlinguyinca/autospec"
  REPO_SLUG="berlinguyinca_autospec"
  HB_DIR="$AUTOSPEC_WATCHDOG_DIR/$REPO_SLUG"
  mkdir -p "$HB_DIR"

  # Stub gh so no network calls leak. Behaviour overridden per-test by GH_MODE.
  STUB_BIN="$TMP_DIR/bin"
  mkdir -p "$STUB_BIN"
  cat > "$STUB_BIN/gh" <<'GHSTUB'
#!/usr/bin/env sh
case "$*" in
  *"issue list"*)
    # GH_MODE=stuck -> one open in-progress-by-bot issue; else empty.
    if [ "${GH_MODE:-clean}" = "stuck" ]; then
      printf '[{"number":516}]\n'
    else
      printf '[]\n'
    fi
    ;;
  *"pr list"*)
    # GH_MODE=haspr -> a PR exists; else none.
    if [ "${GH_MODE:-clean}" = "haspr" ]; then
      printf '[{"number":999}]\n'
    else
      printf '[]\n'
    fi
    ;;
  *) exit 0 ;;
esac
GHSTUB
  chmod +x "$STUB_BIN/gh"
  export PATH="$STUB_BIN:$PATH"
}

teardown() {
  rm -rf "$TMP_DIR"
}

write_hb() {
  # write_hb <issue> <step> <tool_calls> [branch]
  local issue="$1" step="$2" tc="$3" branch="${4:-feat/stop-resume-516}"
  printf '{"issue":"%s","branch":"%s","step":"%s","tool_calls":%s,"ts":%s,"pr":"","repo":"berlinguyinca/autospec"}\n' \
    "$issue" "$branch" "$step" "$tc" "$(date -u +%s)" > "$HB_DIR/$issue.json"
}

run_detect() {
  run bash "$SCRIPT" "$@"
}

# ── clean ─────────────────────────────────────────────────────────────────────

@test "clean state with no heartbeats prints clean and exits 0" {
  GH_MODE=clean run_detect --repo "$REPO"
  [ "$status" -eq 0 ]
  echo "$output" | grep -q '^clean$'
}

@test "clean state with terminal heartbeat (merged) prints clean" {
  write_hb 516 merged 12
  GH_MODE=clean run_detect --repo "$REPO"
  [ "$status" -eq 0 ]
  echo "$output" | grep -q '^clean$'
}

# ── silent-exit ─────────────────────────────────────────────────────────────────

@test "silent-exit: orphan worktree + stuck label + no PR prints silent-exit" {
  write_hb 516 claimed 8
  mkdir -p "$AUTOSPEC_WORKTREE_BASE/wt-feat-stop-resume-516"
  GH_MODE=stuck run_detect --repo "$REPO"
  [ "$status" -eq 0 ]
  echo "$output" | grep -q '^silent-exit$'
}

# ── prompt-overflow ─────────────────────────────────────────────────────────────

@test "prompt-overflow: tool-call count over threshold prints prompt-overflow" {
  write_hb 516 implement 200
  GH_MODE=stuck run_detect --repo "$REPO"
  [ "$status" -eq 0 ]
  echo "$output" | grep -q '^prompt-overflow$'
}

@test "prompt-overflow threshold is configurable via AUTOSPEC_OVERFLOW_THRESHOLD" {
  write_hb 516 implement 60
  AUTOSPEC_OVERFLOW_THRESHOLD=50 GH_MODE=stuck run_detect --repo "$REPO"
  [ "$status" -eq 0 ]
  echo "$output" | grep -q '^prompt-overflow$'
}

# ── unknown ─────────────────────────────────────────────────────────────────────

@test "unknown: stuck label but no orphan worktree and low tool-call count" {
  write_hb 516 implement 10
  # No worktree dir created; label stuck.
  GH_MODE=stuck run_detect --repo "$REPO"
  [ "$status" -eq 0 ]
  echo "$output" | grep -q '^unknown$'
}

# ── auditable trail ──────────────────────────────────────────────────────────────

@test "detector references the memory entry in its source comments" {
  grep -q 'feedback_monitor_silent_exit.md' "$SCRIPT"
}

# ── --help ───────────────────────────────────────────────────────────────────────

@test "--help exits 0 and documents the four modes" {
  run bash "$SCRIPT" --help
  [ "$status" -eq 0 ]
  echo "$output" | grep -q 'silent-exit'
  echo "$output" | grep -q 'prompt-overflow'
  echo "$output" | grep -q 'clean'
  echo "$output" | grep -q 'unknown'
}
