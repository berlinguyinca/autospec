#!/usr/bin/env bats
# emit-event.bats — unit suite for skills/autospec-shared/scripts/emit-event.sh.
# The shim is a THIN dispatcher: guard on AUTOSPEC_DB_DSN / ~/.autospec/db.env,
# resolve the autospec-db binary, dispatch. All transport (payload/spool/
# drain/timeout) lives binary-side (autospec-db repo) — never exercised here.
# Tests stub the binary via a PATH shim that logs argv; never psql, never a
# live DB. HOME is pinned to an isolated per-test tmpdir in setup() because a
# real ~/.autospec/db.env on the dev machine would otherwise leak into the
# no-op assertions below.

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
SHIM="$REPO_ROOT/skills/autospec-shared/scripts/emit-event.sh"

setup() {
  TMP="$(mktemp -d)"
  mkdir -p "$TMP/bin" "$TMP/home"
  # stub autospec-db binary: logs argv verbatim, one line per invocation
  cat > "$TMP/bin/autospec-db" <<'SH'
#!/usr/bin/env bash
echo "$@" >> "$BIN_LOG"
exit "${STUB_EXIT:-0}"
SH
  chmod +x "$TMP/bin/autospec-db"
  export BIN_LOG="$TMP/bin.log"
  export HOME="$TMP/home"
  unset AUTOSPEC_DB_DSN
  unset STUB_EXIT
}

teardown() {
  rm -rf "$TMP"
  unset AUTOSPEC_DB_DSN STUB_EXIT BIN_LOG
}
# Note: HOME is intentionally left pinned to the (now-removed) tmpdir for the
# remainder of this bats worker process's life; each @test runs in its own
# forked process, so this never leaks across tests.

@test "script exists and is bash -n clean" {
  [ -f "$SHIM" ]
  run bash -n "$SHIM"
  [ "$status" -eq 0 ]
}

@test "no DSN and no db.env yields 0 binary invocations and returns 0 (optionality proof)" {
  export PATH="$TMP/bin:$PATH"
  run bash -c ". '$SHIM'; emit_event heartbeat repo=acme/site"
  [ "$status" -eq 0 ]
  [ ! -f "$BIN_LOG" ]
}

@test "db.env present, DSN unset, binary IS invoked (bootstrap-independence proof)" {
  mkdir -p "$HOME/.autospec"
  echo "AUTOSPEC_DB_DSN=postgres://user:secret@host/db" > "$HOME/.autospec/db.env"
  export PATH="$TMP/bin:$PATH"
  run bash -c ". '$SHIM'; emit_event heartbeat repo=acme/site"
  [ "$status" -eq 0 ]
  [ -f "$BIN_LOG" ]
  [ "$(cat "$BIN_LOG")" = "emit heartbeat repo=acme/site" ]
}

@test "binary absent yields 0 invocations and returns 0 even with DSN set" {
  export AUTOSPEC_DB_DSN="postgres://user:secret@host/db"
  export PATH="$TMP/nonexistent-bin-dir-$$:/usr/bin:/bin"
  run bash -c ". '$SHIM'; emit_event heartbeat repo=acme/site"
  [ "$status" -eq 0 ]
  [ ! -f "$BIN_LOG" ]
}

@test "happy path dispatches emit <kind> [k=v]... verbatim to the stubbed binary" {
  export AUTOSPEC_DB_DSN="postgres://user:secret@host/db"
  export PATH="$TMP/bin:$PATH"
  run bash -c ". '$SHIM'; emit_event heartbeat repo=acme/site issue=1770 pr=99"
  [ "$status" -eq 0 ]
  [ -f "$BIN_LOG" ]
  [ "$(cat "$BIN_LOG")" = "emit heartbeat repo=acme/site issue=1770 pr=99" ]
}

@test "binary failure is still absorbed as exit 0 (never blocks the caller)" {
  export AUTOSPEC_DB_DSN="postgres://user:secret@host/db"
  export PATH="$TMP/bin:$PATH"
  export STUB_EXIT=17
  run bash -c ". '$SHIM'; emit_event heartbeat repo=acme/site; echo \"after:\$?\""
  [ "$status" -eq 0 ]
  [ -f "$BIN_LOG" ]
  [[ "$output" == *"after:0"* ]]
}

@test "AUTOSPEC_DB_DSN value never appears in shim output (stdout+stderr)" {
  export AUTOSPEC_DB_DSN="postgres://user:s3cr3t-token@host/db"
  export PATH="$TMP/bin:$PATH"
  run bash -c ". '$SHIM'; emit_event heartbeat repo=acme/site" 2>&1
  ! printf '%s' "$output" | grep -q 's3cr3t-token'
}

@test "AUTOSPEC_DB_DSN value never appears when binary is absent" {
  export AUTOSPEC_DB_DSN="postgres://user:s3cr3t-token@host/db"
  export PATH="$TMP/nonexistent-bin-dir-$$:/usr/bin:/bin"
  run bash -c ". '$SHIM'; emit_event heartbeat repo=acme/site" 2>&1
  ! printf '%s' "$output" | grep -q 's3cr3t-token'
}

@test "db.env DSN value never appears in shim output" {
  mkdir -p "$HOME/.autospec"
  echo "AUTOSPEC_DB_DSN=postgres://user:s3cr3t-token@host/db" > "$HOME/.autospec/db.env"
  export PATH="$TMP/bin:$PATH"
  run bash -c ". '$SHIM'; emit_event heartbeat repo=acme/site" 2>&1
  ! printf '%s' "$output" | grep -q 's3cr3t-token'
}

@test "sourcing the shim does not mutate the caller's shell options" {
  export PATH="$TMP/bin:$PATH"
  run bash -c "set +e +u +o pipefail; . '$SHIM'; set -o | awk '\$1 ~ /^(errexit|nounset|pipefail)\$/ {print \$1\":\"\$2}'"
  [ "$status" -eq 0 ]
  [[ "$output" == *"errexit:off"* ]]
  [[ "$output" == *"nounset:off"* ]]
  [[ "$output" == *"pipefail:off"* ]]
}

@test "falls back to ~/.autospec/bin/autospec-db when not on PATH" {
  export AUTOSPEC_DB_DSN="postgres://user:secret@host/db"
  mkdir -p "$HOME/.autospec/bin"
  cp "$TMP/bin/autospec-db" "$HOME/.autospec/bin/autospec-db"
  chmod +x "$HOME/.autospec/bin/autospec-db"
  export PATH="/usr/bin:/bin"
  run bash -c ". '$SHIM'; emit_event heartbeat repo=acme/site"
  [ "$status" -eq 0 ]
  [ -f "$BIN_LOG" ]
  [ "$(cat "$BIN_LOG")" = "emit heartbeat repo=acme/site" ]
}
