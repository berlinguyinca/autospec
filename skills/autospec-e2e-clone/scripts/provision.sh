#!/usr/bin/env bash
# skills/autospec-e2e-clone/scripts/provision.sh — Orchestrates the full
# autospec-e2e-clone provisioning pipeline for a target repository.
#
# Usage:
#   provision.sh [<repo_root>] [--repo <dir>] [--contract <path>] [--url-file <path>]
#                [--skip-snapshot] [--skip-anonymize] [--skip-scale-down] [--skip-seed]
#
#   The positional <repo_root> and the --repo flag are interchangeable; --repo
#   takes precedence if both are given. --contract overrides the default
#   contract path ($REPO_ROOT/.autospec/clone.yml). The --skip-* flags suppress
#   their respective pipeline steps.
#
# Steps:
#   1. Load + validate .autospec/clone.yml (--contract overrides the path)
#   2. (optional) snapshot / anonymize / scale-down / seed — each gated by a --skip-* flag
#   3. Dispatch "up" to the expose adapter (via expose/dispatch.sh)
#   4. Write the resolved URL to .autospec/clone-url.txt (done by adapter)
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
CONTRACT_ARG=""
SKIP_SNAPSHOT=false
SKIP_ANONYMIZE=false
SKIP_SCALE_DOWN=false
SKIP_SEED=false

while [ $# -gt 0 ]; do
  case "$1" in
    --repo)           REPO_ROOT="$2";    shift 2 ;;
    --contract)       CONTRACT_ARG="$2"; shift 2 ;;
    --url-file)       URL_FILE_ARG="$2"; shift 2 ;;
    --skip-snapshot)   SKIP_SNAPSHOT=true;   shift ;;
    --skip-anonymize)  SKIP_ANONYMIZE=true;  shift ;;
    --skip-scale-down) SKIP_SCALE_DOWN=true; shift ;;
    --skip-seed)       SKIP_SEED=true;       shift ;;
    -h|--help)
      printf 'Usage: provision.sh [<repo_root>] [--repo <dir>] [--contract <path>] [--url-file <path>]\n'
      printf '                    [--skip-snapshot] [--skip-anonymize] [--skip-scale-down] [--skip-seed]\n'
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

if [ -n "$CONTRACT_ARG" ]; then
  CLONE_YML="$CONTRACT_ARG"
else
  CLONE_YML="$REPO_ROOT/.autospec/clone.yml"
fi
if [ ! -f "$CLONE_YML" ]; then
  refuse "clone contract not found: $CLONE_YML"
fi

info "loading contract from $CLONE_YML"
CONTRACT_JSON=$("$LOAD_CONTRACT_SH" "$REPO_ROOT") || {
  rc=$?
  die "load-contract.sh failed (exit $rc)"
}

# ---------------------------------------------------------------------------
# Pipeline steps — each gated by its --skip-* flag.
# (snapshot / anonymize / scale-down / seed are skipped here; the dogfood
#  fast path runs against an already-prepared target and skips all four.)
# ---------------------------------------------------------------------------

if [ "$SKIP_SNAPSHOT" = "true" ]; then
  info "skipping snapshot step (--skip-snapshot)"
fi
if [ "$SKIP_ANONYMIZE" = "true" ]; then
  info "skipping anonymize step (--skip-anonymize)"
fi
if [ "$SKIP_SCALE_DOWN" = "true" ]; then
  info "skipping scale-down step (--skip-scale-down)"
fi
if [ "$SKIP_SEED" = "true" ]; then
  info "skipping seed step (--skip-seed)"
fi

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
