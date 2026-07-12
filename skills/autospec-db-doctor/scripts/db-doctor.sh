#!/usr/bin/env bash
# db-doctor.sh — runnable backend for the /autospec-db-doctor skill.
#
# Resolves the optional `autospec-db` binary (PATH, then
# ~/.autospec/bin/autospec-db), runs `autospec-db doctor`, and maps each
# reported FAIL line to a concrete operator fix. Degrades gracefully when the
# binary is absent (prints the install one-liner, exits 0). Never echoes a
# DSN in any code path, including redaction of doctor's own output.
#
# This is a read-only diagnostic report: it always exits 0 (binary absent,
# all-OK, or FAILs mapped) and never runs any autospec-db subcommand other
# than `doctor`.
#
# Conventions: bash 3.2 compatible, set -eu (no pipefail dependency), no
# RETURN traps (repo bash 3.2 gotchas).
set -eu

INSTALL_HINT='curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec-db/main/install.sh | bash'

redact_dsn() {
  # Redacts connection strings wherever they appear in doctor's own output,
  # so a DSN never reaches stdout/stderr even if the binary is misbehaving.
  # Covers the URI form (postgres:// / postgresql://, any letter case — BSD
  # sed has no case-insensitive s///, hence the bracket classes) and the
  # libpq keyword/value form (password=... / user=...), also any case.
  sed -E \
    -e 's#[Pp][Oo][Ss][Tt][Gg][Rr][Ee][Ss]([Qq][Ll])?://[^[:space:]]*#[redacted DSN]#g' \
    -e 's#[Pp][Aa][Ss][Ss][Ww][Oo][Rr][Dd][[:space:]]*=[[:space:]]*[^[:space:]]*#password=[redacted]#g' \
    -e 's#[Uu][Ss][Ee][Rr][[:space:]]*=[[:space:]]*[^[:space:]]*#user=[redacted]#g'
}

map_fail_line() {
  # $1 = one line of doctor output already known to contain "FAIL".
  local line="$1"
  case "$line" in
    *[Dd]b.conf*|*db\.conf*)
      printf '  fix: %s\n' "Run the autospec-db installer: $INSTALL_HINT"
      ;;
    *[Cc]onnect*)
      printf '  fix: %s\n' "Check DSN host/port/sslmode in ~/.autospec/db.conf. If behind pgbouncer, confirm the pooler is reachable on its configured port and that sslmode matches the pooler's TLS posture."
      ;;
    *schema*|*[Mm]igration*)
      printf '  fix: %s\n' "Re-run the autospec-db installer to apply pending schema updates: $INSTALL_HINT"
      ;;
    *spool*|*[Ss]pool*)
      printf '  fix: %s\n' "Local spool has unsent events. Drain it: autospec-db drain"
      ;;
    *)
      printf '  fix: %s\n' "(no mapped fix — see autospec-db docs)"
      ;;
  esac
}

main() {
  local bin=""
  if command -v autospec-db >/dev/null 2>&1; then
    bin="$(command -v autospec-db)"
  elif [ -x "${HOME:-}/.autospec/bin/autospec-db" ]; then
    bin="${HOME:-}/.autospec/bin/autospec-db"
  else
    printf 'autospec-db is not installed. Install it with:\n  %s\n' "$INSTALL_HINT"
    exit 0
  fi

  local out
  out="$("$bin" doctor 2>&1 || true)"
  out="$(printf '%s\n' "$out" | redact_dsn)"

  local total=0
  local fails=0
  local line
  while IFS= read -r line; do
    [ -n "$line" ] || continue
    total=$((total + 1))
    printf '%s\n' "$line"
    case "$line" in
      *FAIL*)
        fails=$((fails + 1))
        map_fail_line "$line"
        ;;
    esac
  done <<EOF
$out
EOF

  if [ "$fails" -eq 0 ]; then
    printf 'autospec-db doctor: all checks OK\n'
  else
    printf 'autospec-db doctor: %s checks, %s FAIL(s)\n' "$total" "$fails"
  fi
  exit 0
}

main "$@"
