#!/usr/bin/env bats
# tests/autospec-exclusivity.bats — issue #3967: held-lock exclusivity,
# pidfile identity, and the self/ancestor-excluding matcher, exercised
# against the populated #3793 case (a guard called while a competitor is
# running must reject; a guard whose own argv contains the pattern must
# not match itself).

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/.." && pwd)"
LIB="$REPO_ROOT/scripts/lib/autospec-exclusivity.sh"

# ── autospec_gate: held-lock exclusivity ─────────────────────────────────────

@test "gate: first acquisition succeeds and writes a holder note" {
  local dir lock
  dir="$(mktemp -d)"
  lock="$dir/gate.lock"
  run bash -c ". '$LIB'; autospec_gate '$lock' 'first-run'; rc=\$?; cat '$lock'; exit \$rc"
  [ "$status" -eq 0 ]
  [[ "$output" == *label=first-run* ]]
  [[ "$output" == *pid=[0-9]* ]]
  rm -rf "$dir"
}

@test "gate: refuses while a competitor holds the lock (populated #3793 case)" {
  local dir lock holder
  dir="$(mktemp -d)"
  lock="$dir/gate.lock"
  bash -c ". '$LIB'; autospec_gate '$lock' 'holder' || exit 9; sleep 3" &
  holder=$!
  sleep 0.5
  run bash -c ". '$LIB'; autospec_gate '$lock' 'challenger'"
  [ "$status" -eq 1 ]
  [[ "$output" == *REFUSING* ]]
  [[ "$output" == *label=holder* ]]
  # The challenger must not have clobbered the holder's note.
  kill "$holder" 2>/dev/null
  wait "$holder" 2>/dev/null || true
  grep -q label=holder "$lock"
  rm -rf "$dir"
}

@test "gate: lock is released when the holder dies (kernel closes the fd)" {
  local dir lock holder
  dir="$(mktemp -d)"
  lock="$dir/gate.lock"
  bash -c ". '$LIB'; autospec_gate '$lock' 'holder' || exit 9; sleep 30" &
  holder=$!
  sleep 0.5
  kill -9 "$holder" 2>/dev/null
  wait "$holder" 2>/dev/null || true
  run bash -c ". '$LIB'; autospec_gate '$lock' 'after-crash'"
  [ "$status" -eq 0 ]
  rm -rf "$dir"
}

@test "gate: release drops the lock before the shell exits" {
  local dir lock
  dir="$(mktemp -d)"
  lock="$dir/gate.lock"
  run bash -c ". '$LIB'; autospec_gate '$lock' 'a' && autospec_gate_release && autospec_gate '$lock' 'b'"
  [ "$status" -eq 0 ]
  rm -rf "$dir"
}

@test "gate: usage error without a lockfile returns 2" {
  run bash -c ". '$LIB'; autospec_gate"
  [ "$status" -eq 2 ]
  [[ "$output" == *usage* ]]
}

# ── advisory count: reported, never a control ────────────────────────────────

@test "gate_advisory: warns about competitors but always returns 0" {
  local comp
  bash -c 'sleep 20; :' 'zz-advisory-marker-3967' &
  comp=$!
  sleep 0.3
  run bash -c ". '$LIB'; autospec_gate_advisory 'zz-advisory-marker-3967' 'test scan'; rc=\$?; kill $comp 2>/dev/null; exit \$rc"
  [ "$status" -eq 0 ]
  [[ "$output" == *WARN:* ]]
  [[ "$output" == *"advisory only"* ]]
  wait "$comp" 2>/dev/null || true
}

# ── pidfile identity ─────────────────────────────────────────────────────────

@test "spawn: writes a 0600 pidfile whose pid is the live detached leader" {
  local dir pidfile pid mode
  dir="$(mktemp -d)"
  pidfile="$dir/worker.pid"
  run bash -c ". '$LIB'; autospec_spawn '$pidfile' bash -c 'sleep 30; :' 'zz-worker-identity-3967'"
  [ "$status" -eq 0 ]
  pid="$output"
  [[ "$pid" =~ ^[0-9]+$ ]]
  kill -0 "$pid" 2>/dev/null
  [ "$(tr -d '[:space:]' < "$pidfile")" = "$pid" ]
  mode="$(stat -c %a "$pidfile")"
  [ "$mode" = "600" ]
  kill "$pid" 2>/dev/null; wait "$pid" 2>/dev/null || true
  rm -rf "$dir"
}

@test "pidfile_pid: a later run references the same identity by pidfile" {
  local dir pidfile pid
  dir="$(mktemp -d)"
  pidfile="$dir/worker.pid"
  bash -c ". '$LIB'; autospec_spawn '$pidfile' bash -c 'sleep 30; :' 'zz-worker-identity-3967'" > /dev/null
  run bash -c ". '$LIB'; autospec_pidfile_pid '$pidfile'"
  [ "$status" -eq 0 ]
  pid="$output"
  run bash -c ". '$LIB'; autospec_pidfile_pid '$pidfile'"
  [ "$status" -eq 0 ]
  [ "$output" = "$pid" ]
  kill "$pid" 2>/dev/null
  wait "$pid" 2>/dev/null || true
  rm -rf "$dir"
}

@test "pidfile_pid: missing file returns 1 with no output" {
  run bash -c ". '$LIB'; autospec_pidfile_pid /nonexistent-dir-3967/worker.pid"
  [ "$status" -eq 1 ]
  [ -z "$output" ]
}

@test "pidfile_pid: dead pid returns 1 with no output" {
  local dir pidfile
  dir="$(mktemp -d)"
  pidfile="$dir/worker.pid"
  sleep 0.2 &
  local brief=$!
  wait "$brief" 2>/dev/null
  printf '%s\n' "$brief" > "$pidfile"
  run bash -c ". '$LIB'; autospec_pidfile_pid '$pidfile'"
  [ "$status" -eq 1 ]
  [ -z "$output" ]
  rm -rf "$dir"
}

# ── autospec_match_procs: the pattern-matching fallback ──────────────────────

@test "match_procs: finds the competitor and excludes the whole matching session (populated #3793 case)" {
  # The competitor keeps the marker in its argv (two commands prevent
  # bash's exec-optimization from replacing it with `sleep`).
  local comp
  bash -c 'sleep 20; :' 'zz-real-competitor-3967' &
  comp=$!
  sleep 0.3
  # The bash -c wrapper's argv contains the marker verbatim — the exact
  # self-matching case from #3793. Its pid must never appear in the
  # result, and the competitor must.
  # stdout carries only pids; the per-match log goes to stderr and is
  # discarded here so the result can be compared for exact equality.
  # (The exclusion log would otherwise echo this wrapper's own argv —
  # which contains the marker verbatim — and poison any substring
  # assertion against the captured output.)
  run bash -c ". '$LIB'; autospec_match_procs 'zz-real-competitor-3967' 2>/dev/null"
  [ "$status" -eq 0 ]
  # The result is exactly the competitor — no wrapper, subshell, or
  # ancestor pid leaked into it.
  [ "$output" = "$comp" ]
  # Log-before-act: the match was announced on stderr (captured by run).
  run bash -c ". '$LIB'; autospec_match_procs 'zz-real-competitor-3967' 2>&1 >/dev/null"
  [[ "$output" == *"match: $comp"* ]]
  kill "$comp" 2>/dev/null || true
  wait "$comp" 2>/dev/null || true
}

@test "match_procs: a guard whose own argv contains the pattern does not match itself" {
  # No competitor at all: the only processes whose argv carries the
  # marker are the wrapper shell, its subshells, and their ancestors.
  # The correct result is zero matches.
  run bash -c ". '$LIB'; autospec_match_procs 'zz-self-only-marker-3967' >/dev/null; rc=\$?; exit \$rc"
  [ "$status" -eq 1 ]
}

@test "match_procs: no matches returns 1, usage returns 2" {
  run bash -c ". '$LIB'; autospec_match_procs 'zz-no-such-marker-3967'"
  [ "$status" -eq 1 ]
  run bash -c ". '$LIB'; autospec_match_procs"
  [ "$status" -eq 2 ]
  [[ "$output" == *usage* ]]
}

# ── the lib itself respects the -f matcher lint ─────────────────────────────

@test "lib is clean under scripts/lint-process-matchers.sh" {
  run bash "$REPO_ROOT/scripts/lint-process-matchers.sh" "$LIB"
  [ "$status" -eq 0 ]
}
