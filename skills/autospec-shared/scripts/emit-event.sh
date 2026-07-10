#!/usr/bin/env bash
# emit-event.sh — thin shim that dispatches telemetry events to the
# autospec-db Go binary. Guard + binary resolution + dispatch ONLY:
# payload assembly, spool, drain, and the 2s DB timeout all live binary-side
# (autospec-db repo), never here. No psql, ever.
#
# Runtime source line every chokepoint uses:
#   H=${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}
#   [ -f "$H/emit-event.sh" ] && . "$H/emit-event.sh"
#   emit_event <kind> [key=value]...
#
# Guarantees: every path returns exit 0; emission never blocks, retries
# inline, or alters a caller's exit/return value; AUTOSPEC_DB_DSN is never
# echoed or logged.
#
# No top-level `set -euo pipefail` here (unlike growth-ledger.sh, which is
# invoked as its own process): this file is SOURCED into a long-lived
# chokepoint script, and a file-scope `set` would mutate the caller's shell
# options (errexit/nounset/pipefail) for the rest of its run — its own
# "never alters a caller" guarantee. Each guard below is written to be safe
# under the caller's own `set -u`.
emit_event() {
  # Guard 1: no DSN configured means telemetry is off — silent no-op.
  if [ -z "${AUTOSPEC_DB_DSN:-}" ]; then
    return 0
  fi

  # Guard 2: resolve the autospec-db binary — PATH first, then the
  # well-known install fallback. Absent binary is a silent no-op too.
  local bin=""
  if command -v autospec-db >/dev/null 2>&1; then
    bin="$(command -v autospec-db)"
  elif [ -x "${HOME:-}/.autospec/bin/autospec-db" ]; then
    bin="${HOME:-}/.autospec/bin/autospec-db"
  else
    return 0
  fi

  # Dispatch verbatim; the binary owns payload assembly, spool, drain, and
  # the 2s timeout. A literal shell `exec` here would replace the CALLING
  # process image (this function runs sourced, in-process, inside a
  # long-lived chokepoint script) rather than just the dispatch — that would
  # terminate the caller instead of returning to it. `|| true` + explicit
  # `return 0` delivers the same "dispatch verbatim, never propagate a
  # non-zero status" contract without killing the host script.
  "$bin" emit "$@" || true
  return 0
}
