#!/usr/bin/env bash
# skills/autospec-e2e-clone/scripts/teardown.sh — Symmetric teardown for an
# autospec-e2e-clone provisioned environment.
#
# Usage:
#   teardown.sh [<repo_root>] [--repo <dir>] [--compose-file <path>]
#               [--keep-snapshots | --purge-snapshots]
#
#   The positional <repo_root> and the --repo flag are interchangeable; --repo
#   takes precedence if both are given. --compose-file, when set and docker is
#   available, brings the named compose stack down (-v) as part of teardown.
#
# Reads the resolved contract from .autospec/clone.yml (via load-contract.sh),
# routes the "down" action to the correct expose adapter via dispatch.sh, and
# removes .autospec/clone-url.txt.
#
# Options:
#   --keep-snapshots    Retain .autospec/clone-snapshots/ (default)
#   --purge-snapshots   Delete .autospec/clone-snapshots/ after teardown
#
# Exit codes:
#   0  success (or idempotent — already torn down)
#   1  fatal (load-contract failed, dispatch failed, internal error)
#   2  refuse-to-run (contract invalid)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
DISPATCH_SH="$SCRIPT_DIR/expose/dispatch.sh"
LOAD_CONTRACT_SH="$SCRIPT_DIR/load-contract.sh"

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

die()    { printf 'teardown.sh: fatal: %s\n' "$*" >&2; exit 1; }
refuse() { printf 'teardown.sh: refuse-to-run: %s\n' "$*" >&2; exit 2; }
info()   { printf 'teardown.sh: %s\n' "$*"; }

# ---------------------------------------------------------------------------
# Arg parsing
# ---------------------------------------------------------------------------

REPO_ROOT="."
PURGE_SNAPSHOTS=false
COMPOSE_FILE_ARG=""

while [ $# -gt 0 ]; do
  case "$1" in
    --repo)             REPO_ROOT="$2";         shift 2 ;;
    --compose-file)     COMPOSE_FILE_ARG="$2";  shift 2 ;;
    --keep-snapshots)   PURGE_SNAPSHOTS=false; shift ;;
    --purge-snapshots)  PURGE_SNAPSHOTS=true;  shift ;;
    -h|--help)
      printf 'Usage: teardown.sh [<repo_root>] [--repo <dir>] [--compose-file <path>] [--keep-snapshots|--purge-snapshots]\n'
      exit 0
      ;;
    -*)
      die "unknown option: $1"
      ;;
    *)
      REPO_ROOT="$1"; shift
      ;;
  esac
done

REPO_ROOT="$(cd "$REPO_ROOT" && pwd)"

# ---------------------------------------------------------------------------
# Load contract
# ---------------------------------------------------------------------------

[ -f "$LOAD_CONTRACT_SH" ] || die "load-contract.sh not found at $LOAD_CONTRACT_SH"
[ -f "$DISPATCH_SH" ]      || die "dispatch.sh not found at $DISPATCH_SH"

CLONE_YML="$REPO_ROOT/.autospec/clone.yml"
if [ ! -f "$CLONE_YML" ]; then
  # Idempotent: if contract never existed, nothing to tear down
  info "no .autospec/clone.yml found — nothing to tear down (idempotent)"
  exit 0
fi

info "loading contract from $CLONE_YML"
CONTRACT_JSON=$("$LOAD_CONTRACT_SH" "$REPO_ROOT") || {
  rc=$?
  die "load-contract.sh failed (exit $rc) — cannot determine expose adapter to tear down"
}

# ---------------------------------------------------------------------------
# Dispatch "down" to expose adapter
# ---------------------------------------------------------------------------

CONTRACT_TMPFILE=$(mktemp -t teardown-contract-XXXXXX.json)
# Inline cleanup — no RETURN trap (avoids bash RETURN trap leak under set -eu)
cleanup_contract() { rm -f "$CONTRACT_TMPFILE"; }
trap 'cleanup_contract' EXIT

printf '%s\n' "$CONTRACT_JSON" > "$CONTRACT_TMPFILE"

info "dispatching 'down' action to expose adapter"
dispatch_rc=0
bash "$DISPATCH_SH" down --contract "$CONTRACT_TMPFILE" || dispatch_rc=$?

if [ "$dispatch_rc" -ne 0 ]; then
  info "WARN: dispatch returned exit $dispatch_rc — continuing teardown cleanup"
fi

# ---------------------------------------------------------------------------
# Remove clone-url.txt
# ---------------------------------------------------------------------------

CLONE_URL_FILE="$REPO_ROOT/.autospec/clone-url.txt"
if [ -f "$CLONE_URL_FILE" ]; then
  rm -f "$CLONE_URL_FILE"
  info "removed $CLONE_URL_FILE"
else
  info "clone-url.txt already absent (idempotent)"
fi

# ---------------------------------------------------------------------------
# Optionally purge snapshots
# ---------------------------------------------------------------------------

SNAPSHOTS_DIR="$REPO_ROOT/.autospec/clone-snapshots"
if [ "$PURGE_SNAPSHOTS" = "true" ]; then
  if [ -d "$SNAPSHOTS_DIR" ]; then
    rm -rf "$SNAPSHOTS_DIR"
    info "purged $SNAPSHOTS_DIR"
  else
    info "clone-snapshots/ already absent (idempotent)"
  fi
else
  if [ -d "$SNAPSHOTS_DIR" ]; then
    info "retaining $SNAPSHOTS_DIR (use --purge-snapshots to remove)"
  fi
fi

# ---------------------------------------------------------------------------
# Optionally bring down a docker compose stack
# ---------------------------------------------------------------------------

if [ -n "$COMPOSE_FILE_ARG" ]; then
  if command -v docker >/dev/null 2>&1 && docker compose version >/dev/null 2>&1; then
    if [ -f "$COMPOSE_FILE_ARG" ]; then
      info "bringing down compose stack: $COMPOSE_FILE_ARG"
      docker compose -f "$COMPOSE_FILE_ARG" down -v >/dev/null 2>&1 || \
        info "WARN: 'docker compose down' returned non-zero — continuing"
    else
      info "compose file not found: $COMPOSE_FILE_ARG (skipping compose down)"
    fi
  else
    info "docker compose unavailable — skipping compose down for $COMPOSE_FILE_ARG"
  fi
fi

info "teardown complete"
exit 0
