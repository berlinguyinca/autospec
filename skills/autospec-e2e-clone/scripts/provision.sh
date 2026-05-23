#!/usr/bin/env bash
# skills/autospec-e2e-clone/scripts/provision.sh — Orchestrates the full
# autospec-e2e-clone provisioning pipeline for a target repository.
#
# Usage:
#   provision.sh [<repo_root>] [--url-file <path>]
#
# Steps:
#   1. Load + validate .autospec/clone.yml
#   2. Dispatch "up" to the expose adapter (via expose/dispatch.sh)
#   3. Write the resolved URL to .autospec/clone-url.txt (done by adapter)
#
# Output: prints the clone URL to stdout on success.
#
# Exit codes:
#   0  success — URL written to .autospec/clone-url.txt
#   1  fatal (missing deps, adapter error)
#   2  refuse-to-run (contract missing or invalid)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
DISPATCH_SH="$SCRIPT_DIR/expose/dispatch.sh"
LOAD_CONTRACT_SH="$SCRIPT_DIR/load-contract.sh"

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

die()    { printf 'provision.sh: fatal: %s\n' "$*" >&2; exit 1; }
refuse() { printf 'provision.sh: refuse-to-run: %s\n' "$*" >&2; exit 2; }
info()   { printf 'provision.sh: %s\n' "$*"; }

# ---------------------------------------------------------------------------
# Arg parsing
# ---------------------------------------------------------------------------

REPO_ROOT="."
URL_FILE_ARG=""

while [ $# -gt 0 ]; do
  case "$1" in
    --url-file) URL_FILE_ARG="$2"; shift 2 ;;
    -h|--help)
      printf 'Usage: provision.sh [<repo_root>] [--url-file <path>]\n'
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
# Dependency checks
# ---------------------------------------------------------------------------

[ -f "$LOAD_CONTRACT_SH" ] || die "load-contract.sh not found at $LOAD_CONTRACT_SH"
[ -f "$DISPATCH_SH" ]      || die "dispatch.sh not found at $DISPATCH_SH"

# ---------------------------------------------------------------------------
# Load contract
# ---------------------------------------------------------------------------

CLONE_YML="$REPO_ROOT/.autospec/clone.yml"
if [ ! -f "$CLONE_YML" ]; then
  refuse ".autospec/clone.yml not found in $REPO_ROOT"
fi

info "loading contract from $CLONE_YML"
CONTRACT_JSON=$("$LOAD_CONTRACT_SH" "$REPO_ROOT") || {
  rc=$?
  die "load-contract.sh failed (exit $rc)"
}

# ---------------------------------------------------------------------------
# Dispatch "up" to expose adapter
# ---------------------------------------------------------------------------

CONTRACT_TMPFILE=$(mktemp -t provision-contract-XXXXXX.json)
# Inline cleanup — no RETURN trap (avoids bash RETURN trap leak under set -eu)
trap 'rm -f "$CONTRACT_TMPFILE"' EXIT

printf '%s\n' "$CONTRACT_JSON" > "$CONTRACT_TMPFILE"

CLONE_URL_FILE="$REPO_ROOT/.autospec/clone-url.txt"
URL_ARG=""
if [ -n "$URL_FILE_ARG" ]; then
  URL_ARG="--url-file $URL_FILE_ARG"
  CLONE_URL_FILE="$URL_FILE_ARG"
fi

info "dispatching 'up' action to expose adapter"
# shellcheck disable=SC2086
bash "$DISPATCH_SH" up --contract "$CONTRACT_TMPFILE" $URL_ARG || {
  rc=$?
  die "expose adapter 'up' failed (exit $rc)"
}

# ---------------------------------------------------------------------------
# Emit URL to stdout
# ---------------------------------------------------------------------------

if [ -f "$CLONE_URL_FILE" ]; then
  CLONE_URL="$(cat "$CLONE_URL_FILE")"
  info "clone URL: $CLONE_URL"
  printf '%s\n' "$CLONE_URL"
else
  info "WARN: adapter did not write clone-url.txt; URL unknown"
fi

exit 0
