#!/usr/bin/env bash
# skills/autospec-e2e-clone/scripts/expose/custom.sh
#
# Custom-command expose adapter (C8).
#
# Invokes operator-provided up/down commands from expose.custom.up_cmd /
# expose.custom.down_cmd in the contract.
#
# Actions:
#   up   — exec expose.custom.up_cmd; poll health_endpoint; write clone-url.txt
#   down — exec expose.custom.down_cmd
#
# Usage:
#   custom.sh up   --up-cmd <cmd> --url <url> [--health <endpoint>]
#                  [--wait <secs>] [--url-file <path>]
#   custom.sh down --down-cmd <cmd>
#
# Exit codes:
#   0  success
#   1  fatal (missing args, command failed)
#   2  refuse-to-run (health check timed out)

set -euo pipefail

die()    { printf 'custom.sh: fatal: %s\n' "$*" >&2; exit 1; }
refuse() { printf 'custom.sh: refuse-to-run: %s\n' "$*" >&2; exit 2; }

# ---------------------------------------------------------------------------
# Arg parsing
# ---------------------------------------------------------------------------

ACTION="${1:-}"
case "$ACTION" in
  up|down) shift ;;
  -h|--help) printf 'Usage: custom.sh up --up-cmd <cmd> --url <url> [...] | custom.sh down --down-cmd <cmd>\n'; exit 0 ;;
  "") die "action required: up or down" ;;
  *) die "unknown action: $ACTION. Use 'up' or 'down'." ;;
esac

UP_CMD=""
DOWN_CMD=""
URL_TEMPLATE="http://localhost:8080"
HEALTH_ENDPOINT="/health"
READY_WAIT_SECS=60
URL_FILE=""

while [ $# -gt 0 ]; do
  case "$1" in
    --up-cmd)   UP_CMD="$2";   shift 2 ;;
    --down-cmd) DOWN_CMD="$2"; shift 2 ;;
    --url)      URL_TEMPLATE="$2"; shift 2 ;;
    --health)   HEALTH_ENDPOINT="$2"; shift 2 ;;
    --wait)     READY_WAIT_SECS="$2"; shift 2 ;;
    --url-file) URL_FILE="$2"; shift 2 ;;
    *)          die "unknown option: $1" ;;
  esac
done

# ---------------------------------------------------------------------------
# DOWN action
# ---------------------------------------------------------------------------

if [ "$ACTION" = "down" ]; then
  [ -n "$DOWN_CMD" ] || die "--down-cmd is required for down action"
  printf 'custom.sh: running down command: %s\n' "$DOWN_CMD"
  if ! eval "$DOWN_CMD"; then
    die "down command failed: $DOWN_CMD"
  fi
  printf 'custom.sh: down command completed\n'
  exit 0
fi

# ---------------------------------------------------------------------------
# UP action
# ---------------------------------------------------------------------------

[ -n "$UP_CMD" ] || die "--up-cmd is required for up action"

printf 'custom.sh: running up command: %s\n' "$UP_CMD"
if ! eval "$UP_CMD"; then
  die "up command failed: $UP_CMD"
fi

RESOLVED_URL="${URL_TEMPLATE//\{\{host\}\}/localhost}"
TARGET_URL="${RESOLVED_URL}${HEALTH_ENDPOINT}"

printf 'custom.sh: polling health endpoint %s (timeout: %ss)\n' "$TARGET_URL" "$READY_WAIT_SECS"

elapsed=0
interval=2
healthy=false
while [ "$elapsed" -lt "$READY_WAIT_SECS" ]; do
  http_code=$(curl -s -o /dev/null -w '%{http_code}' --max-time 3 "$TARGET_URL" 2>/dev/null || true)
  if [ "$http_code" = "200" ]; then
    healthy=true
    break
  fi
  printf 'custom.sh: waiting... (%ss elapsed, last HTTP %s)\n' "$elapsed" "${http_code:-000}"
  sleep "$interval"
  elapsed=$(( elapsed + interval ))
done

if [ "$healthy" != "true" ]; then
  refuse "health check timed out after ${READY_WAIT_SECS}s — ${TARGET_URL} did not return 200"
fi

printf 'custom.sh: environment active — URL: %s\n' "$RESOLVED_URL"

if [ -z "$URL_FILE" ]; then
  URL_FILE=".autospec/clone-url.txt"
fi
mkdir -p "$(dirname "$URL_FILE")"
printf '%s\n' "$RESOLVED_URL" > "$URL_FILE"
printf 'custom.sh: wrote %s\n' "$URL_FILE"
exit 0
