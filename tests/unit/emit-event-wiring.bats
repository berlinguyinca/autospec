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
