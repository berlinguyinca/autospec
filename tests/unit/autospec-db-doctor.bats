#!/usr/bin/env bats
# autospec-db-doctor.bats — unit suite for
# skills/autospec-db-doctor/scripts/db-doctor.sh (issue #1807).
#
# The script is a THIN diagnostic wrapper: resolve the autospec-db binary
# (PATH, then ~/.autospec/bin/autospec-db), run `autospec-db doctor`, map
# each FAIL line to a concrete fix, redact any DSN in output, print a
# summary, and always exit 0. Tests stub the binary via a PATH shim — never
# a live database, never psql. HOME is pinned to an isolated per-test tmpdir
# and AUTOSPEC_DB_DISABLE=1 is exported as a belt-and-braces kill switch
# (this script never emits telemetry itself, but the isolation convention is
# shared across the telemetry test suites).

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
SCRIPT="$REPO_ROOT/skills/autospec-db-doctor/scripts/db-doctor.sh"

setup() {
  TMP="$(mktemp -d)"
  mkdir -p "$TMP/bin" "$TMP/home"
  export HOME="$TMP/home"
  export AUTOSPEC_DB_DISABLE=1
  unset STUB_DOCTOR_OUTPUT STUB_EXIT
}

teardown() {
  rm -rf "$TMP"
  unset HOME AUTOSPEC_DB_DISABLE STUB_DOCTOR_OUTPUT STUB_EXIT
}

stub_binary() {
  # $1 = doctor output to emit verbatim.
  cat > "$TMP/bin/autospec-db" <<SH
#!/usr/bin/env bash
if [ "\$1" = "doctor" ]; then
  cat <<'DOCEOF'
$1
DOCEOF
  exit "\${STUB_EXIT:-0}"
fi
echo "unexpected subcommand: \$*" >&2
exit 1
SH
  chmod +x "$TMP/bin/autospec-db"
}

@test "script exists and is bash -n clean" {
  [ -f "$SCRIPT" ]
  run bash -n "$SCRIPT"
  [ "$status" -eq 0 ]
}

@test "binary absent prints install hint and exits 0" {
  export PATH="$TMP/nonexistent-bin-dir-$$:/usr/bin:/bin"
  run bash "$SCRIPT"
  [ "$status" -eq 0 ]
  [[ "$output" == *"autospec-db is not installed"* ]]
  [[ "$output" == *"raw.githubusercontent.com/berlinguyinca/autospec-db/main/install.sh"* ]]
}

@test "resolves binary from ~/.autospec/bin when not on PATH" {
  mkdir -p "$HOME/.autospec/bin"
  stub_binary "OK: db.conf present"
  cp "$TMP/bin/autospec-db" "$HOME/.autospec/bin/autospec-db"
  chmod +x "$HOME/.autospec/bin/autospec-db"
  export PATH="$TMP/nonexistent-bin-dir-$$:/usr/bin:/bin"
  run bash "$SCRIPT"
  [ "$status" -eq 0 ]
  [[ "$output" == *"all checks OK"* ]]
}

@test "all-OK doctor output yields a clean summary with zero FAILs" {
  stub_binary "OK: db.conf present
OK: connect
OK: schema up to date
OK: spool empty"
  export PATH="$TMP/bin:$PATH"
  run bash "$SCRIPT"
  [ "$status" -eq 0 ]
  [[ "$output" == *"autospec-db doctor: all checks OK"* ]]
  [[ "$output" != *"fix:"* ]]
}

@test "missing db.conf FAIL maps to installer fix" {
  stub_binary "FAIL: db.conf not found"
  export PATH="$TMP/bin:$PATH"
  run bash "$SCRIPT"
  [ "$status" -eq 0 ]
  [[ "$output" == *"FAIL: db.conf not found"* ]]
  [[ "$output" == *"Run the autospec-db installer"* ]]
}

@test "connect FAIL maps to DSN host/port/sslmode + pgbouncer hint" {
  stub_binary "FAIL: connect to database timed out"
  export PATH="$TMP/bin:$PATH"
  run bash "$SCRIPT"
  [ "$status" -eq 0 ]
  [[ "$output" == *"FAIL: connect to database timed out"* ]]
  [[ "$output" == *"DSN host/port/sslmode"* ]]
  [[ "$output" == *"pgbouncer"* ]]
}

@test "pending schema updates FAIL maps to re-run installer fix" {
  stub_binary "FAIL: pending schema updates detected"
  export PATH="$TMP/bin:$PATH"
  run bash "$SCRIPT"
  [ "$status" -eq 0 ]
  [[ "$output" == *"FAIL: pending schema updates detected"* ]]
  [[ "$output" == *"Re-run the autospec-db installer"* ]]
}

@test "spool nonzero FAIL maps to drain fix" {
  stub_binary "FAIL: spool has 42 unsent events"
  export PATH="$TMP/bin:$PATH"
  run bash "$SCRIPT"
  [ "$status" -eq 0 ]
  [[ "$output" == *"FAIL: spool has 42 unsent events"* ]]
  [[ "$output" == *"autospec-db drain"* ]]
}

@test "unmapped FAIL class prints the doctor line with a no-mapped-fix marker" {
  stub_binary "FAIL: something entirely novel"
  export PATH="$TMP/bin:$PATH"
  run bash "$SCRIPT"
  [ "$status" -eq 0 ]
  [[ "$output" == *"FAIL: something entirely novel"* ]]
  [[ "$output" == *"no mapped fix"* ]]
}

@test "multiple FAILs each get their own mapped fix and the summary counts them" {
  stub_binary "OK: db.conf present
FAIL: connect refused
FAIL: spool has 3 unsent events"
  export PATH="$TMP/bin:$PATH"
  run bash "$SCRIPT"
  [ "$status" -eq 0 ]
  [[ "$output" == *"DSN host/port/sslmode"* ]]
  [[ "$output" == *"autospec-db drain"* ]]
  [[ "$output" == *"2 FAIL(s)"* ]]
}

@test "a DSN reaching doctor's own output is redacted, never echoed" {
  stub_binary "FAIL: connect refused to postgres://user:secret@host/db?sslmode=require"
  export PATH="$TMP/bin:$PATH"
  run bash "$SCRIPT"
  [ "$status" -eq 0 ]
  [[ "$output" != *"secret"* ]]
  [[ "$output" != *"postgres://"* ]]
  [[ "$output" == *"[redacted DSN]"* ]]
}

@test "uppercase-scheme DSN is redacted too (case-insensitive URI match)" {
  stub_binary "FAIL: connect refused to POSTGRESQL://user:secret@host/db"
  export PATH="$TMP/bin:$PATH"
  run bash "$SCRIPT"
  [ "$status" -eq 0 ]
  [[ "$output" != *"secret"* ]]
  [[ "$output" != *"POSTGRESQL://"* ]]
  [[ "$output" == *"[redacted DSN]"* ]]
}

@test "libpq keyword/value credentials are redacted (password= / user=)" {
  stub_binary "FAIL: connect failed: host=h user=alice password=secret dbname=db sslmode=require"
  export PATH="$TMP/bin:$PATH"
  run bash "$SCRIPT"
  [ "$status" -eq 0 ]
  [[ "$output" != *"secret"* ]]
  [[ "$output" != *"user=alice"* ]]
  [[ "$output" == *"password=[redacted]"* ]]
}

@test "binary present but doctor itself exits non-zero is still absorbed as exit 0" {
  stub_binary "FAIL: connect refused"
  export STUB_EXIT=2
  export PATH="$TMP/bin:$PATH"
  run bash "$SCRIPT"
  [ "$status" -eq 0 ]
}
