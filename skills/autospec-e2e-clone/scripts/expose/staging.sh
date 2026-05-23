#!/usr/bin/env bash
# skills/autospec-e2e-clone/scripts/expose/staging.sh
#
# Dedicated staging-slot expose adapter (C8).
#
# Swaps to a pre-allocated staging slot via operator-configured swap/restore commands.
#
# Actions:
#   up   — exec expose.staging.swap_cmd; poll health_endpoint; write clone-url.txt
#   down — exec expose.staging.restore_cmd
#
# Usage:
#   staging.sh up   --swap-cmd <cmd> --url <url> [--health <endpoint>]
#                   [--wait <secs>] [--url-file <path>]
#   staging.sh down --restore-cmd <cmd>
#
# Exit codes:
#   0  success
#   1  fatal (missing args, swap/restore failed)
#   2  refuse-to-run (health check timed out)

set -euo pipefail

die()    { printf 'staging.sh: fatal: %s\n' "$*" >&2; exit 1; }
refuse() { printf 'staging.sh: refuse-to-run: %s\n' "$*" >&2; exit 2; }

# ---------------------------------------------------------------------------
# Arg parsing
# ---------------------------------------------------------------------------

ACTION="${1:-}"
case "$ACTION" in
  up|down) shift ;;
  -h|--help) printf 'Usage: staging.sh up --swap-cmd <cmd> --url <url> [...] | staging.sh down --restore-cmd <cmd>\n'; exit 0 ;;
  "") die "action required: up or down" ;;
  *) die "unknown action: $ACTION. Use 'up' or 'down'." ;;
esac

SWAP_CMD=""
RESTORE_CMD=""
URL_TEMPLATE="http://localhost:8080"
HEALTH_ENDPOINT="/health"
READY_WAIT_SECS=60
URL_FILE=""

while [ $# -gt 0 ]; do
  case "$1" in
    --swap-cmd)    SWAP_CMD="$2";    shift 2 ;;
    --restore-cmd) RESTORE_CMD="$2"; shift 2 ;;
    --url)         URL_TEMPLATE="$2"; shift 2 ;;
    --health)      HEALTH_ENDPOINT="$2"; shift 2 ;;
    --wait)        READY_WAIT_SECS="$2"; shift 2 ;;
    --url-file)    URL_FILE="$2"; shift 2 ;;
    *)             die "unknown option: $1" ;;
  esac
done

# ---------------------------------------------------------------------------
# DOWN action
# ---------------------------------------------------------------------------

if [ "$ACTION" = "down" ]; then
  [ -n "$RESTORE_CMD" ] || die "--restore-cmd is required for down action"
  printf 'staging.sh: running restore command: %s\n' "$RESTORE_CMD"
  eval "$RESTORE_CMD" || die "restore command failed: $RESTORE_CMD"
  printf 'staging.sh: slot restored\n'
  exit 0
fi

# ---------------------------------------------------------------------------
# UP action
# ---------------------------------------------------------------------------

[ -n "$SWAP_CMD" ] || die "--swap-cmd is required for up action"

printf 'staging.sh: running swap command: %s\n' "$SWAP_CMD"
eval "$SWAP_CMD" || die "swap command failed: $SWAP_CMD"

RESOLVED_URL="${URL_TEMPLATE//\{\{host\}\}/localhost}"
TARGET_URL="${RESOLVED_URL}${HEALTH_ENDPOINT}"

printf 'staging.sh: polling health endpoint %s (timeout: %ss)\n' "$TARGET_URL" "$READY_WAIT_SECS"

elapsed=0
interval=2
healthy=false
while [ "$elapsed" -lt "$READY_WAIT_SECS" ]; do
  http_code=$(curl -s -o /dev/null -w '%{http_code}' --max-time 3 "$TARGET_URL" 2>/dev/null || true)
  if [ "$http_code" = "200" ]; then
    healthy=true
    break
  fi
  printf 'staging.sh: waiting... (%ss elapsed, last HTTP %s)\n' "$elapsed" "${http_code:-000}"
  sleep "$interval"
  elapsed=$(( elapsed + interval ))
done

if [ "$healthy" != "true" ]; then
  refuse "health check timed out after ${READY_WAIT_SECS}s — ${TARGET_URL} did not return 200"
fi

printf 'staging.sh: slot active — URL: %s\n' "$RESOLVED_URL"

if [ -z "$URL_FILE" ]; then
  URL_FILE=".autospec/clone-url.txt"
fi
mkdir -p "$(dirname "$URL_FILE")"
printf '%s\n' "$RESOLVED_URL" > "$URL_FILE"
printf 'staging.sh: wrote %s\n' "$URL_FILE"
exit 0
