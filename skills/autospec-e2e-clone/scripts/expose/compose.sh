#!/usr/bin/env bash
# skills/autospec-e2e-clone/scripts/expose/compose.sh
#
# Docker Compose expose adapter (C7).
#
# Actions:
#   up   — docker compose -f <file> up -d; poll health_endpoint; write clone-url.txt
#   down — docker compose -f <file> down -v
#
# Usage:
#   compose.sh up   <compose-file> [--url <url>] [--health <endpoint>] \
#                   [--wait <secs>] [--url-file <path>]
#   compose.sh down <compose-file>
#
# URL template substitution:
#   {{host}}  — resolved container host (default: localhost)
#   {{port}}  — first mapped host port of the compose service
#
# Exit codes:
#   0  success
#   1  fatal (missing deps, file not found, internal error)
#   2  refuse-to-run (health check timed out, compose failed)

set -euo pipefail

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

usage() {
  printf 'Usage:\n'
  printf '  compose.sh up   <compose-file> [--url <url>] [--health <endpoint>]\n'
  printf '                  [--wait <secs>] [--url-file <path>]\n'
  printf '  compose.sh down <compose-file>\n'
  exit 0
}

die() { printf 'compose.sh: fatal: %s\n' "$*" >&2; exit 1; }
refuse() { printf 'compose.sh: refuse-to-run: %s\n' "$*" >&2; exit 2; }

require_tool() {
  command -v "$1" >/dev/null 2>&1 || die "$1 not found. Install docker (docker.com)."
}

# ---------------------------------------------------------------------------
# Arg parsing
# ---------------------------------------------------------------------------

ACTION="${1:-}"
case "$ACTION" in
  up|down) shift ;;
  -h|--help) usage ;;
  "") die "action required: up or down" ;;
  *) die "unknown action: $ACTION. Use 'up' or 'down'." ;;
esac

COMPOSE_FILE="${1:-}"
[ -n "$COMPOSE_FILE" ] || die "compose-file argument required"
shift

# up-specific options
URL_TEMPLATE="http://localhost:8080"
HEALTH_ENDPOINT="/health"
READY_WAIT_SECS=60
URL_FILE=""

while [ $# -gt 0 ]; do
  case "$1" in
    --url)      URL_TEMPLATE="$2"; shift 2 ;;
    --health)   HEALTH_ENDPOINT="$2"; shift 2 ;;
    --wait)     READY_WAIT_SECS="$2"; shift 2 ;;
    --url-file) URL_FILE="$2"; shift 2 ;;
    -h|--help)  usage ;;
    *)          die "unknown option: $1" ;;
  esac
done

# ---------------------------------------------------------------------------
# Tool checks
# ---------------------------------------------------------------------------

require_tool docker

# Verify compose subcommand is available
docker compose version >/dev/null 2>&1 || die "docker compose plugin not found. Install Docker Desktop or docker-compose-plugin."

# ---------------------------------------------------------------------------
# Resolve compose file
# ---------------------------------------------------------------------------

COMPOSE_FILE="$(cd "$(dirname "$COMPOSE_FILE")" && pwd)/$(basename "$COMPOSE_FILE")"
[ -f "$COMPOSE_FILE" ] || die "compose file not found: $COMPOSE_FILE"

COMPOSE_DIR="$(dirname "$COMPOSE_FILE")"

# ---------------------------------------------------------------------------
# URL template substitution
# ---------------------------------------------------------------------------

resolve_url() {
  local template="$1"
  local host="${DOCKER_HOST_OVERRIDE:-localhost}"

  # Substitute {{host}}
  local url="${template//\{\{host\}\}/$host}"

  # Substitute {{port}} — extract first host port from compose file
  if [[ "$url" == *"{{port}}"* ]]; then
    local port
    port=$(grep -E '^\s+- "[0-9]+:[0-9]+"' "$COMPOSE_FILE" | head -1 | grep -oE '[0-9]+:[0-9]+' | cut -d: -f1 || true)
    if [ -z "$port" ]; then
      port=$(grep -E 'published:' "$COMPOSE_FILE" | head -1 | grep -oE '[0-9]+' | head -1 || true)
    fi
    [ -n "$port" ] || die "could not resolve {{port}} from compose file; specify --url with a literal port"
    url="${url//\{\{port\}\}/$port}"
  fi

  printf '%s' "$url"
}

# ---------------------------------------------------------------------------
# UP action
# ---------------------------------------------------------------------------

if [ "$ACTION" = "up" ]; then
  printf 'compose.sh: starting compose stack from %s\n' "$COMPOSE_FILE"
  docker compose -f "$COMPOSE_FILE" up -d 2>&1 || refuse "docker compose up failed"

  RESOLVED_URL="$(resolve_url "$URL_TEMPLATE")"
  TARGET_URL="${RESOLVED_URL}${HEALTH_ENDPOINT}"

  printf 'compose.sh: polling health endpoint %s (timeout: %ss)\n' "$TARGET_URL" "$READY_WAIT_SECS"

  elapsed=0
  interval=2
  healthy=false
  while [ "$elapsed" -lt "$READY_WAIT_SECS" ]; do
    http_code=$(curl -s -o /dev/null -w '%{http_code}' --max-time 3 "$TARGET_URL" 2>/dev/null || true)
    if [ "$http_code" = "200" ]; then
      healthy=true
      break
    fi
    printf 'compose.sh: waiting... (%ss elapsed, last HTTP %s)\n' "$elapsed" "${http_code:-000}"
    sleep "$interval"
    elapsed=$(( elapsed + interval ))
  done

  if [ "$healthy" != "true" ]; then
    refuse "health check timed out after ${READY_WAIT_SECS}s — ${TARGET_URL} did not return 200"
  fi

  printf 'compose.sh: stack healthy — URL: %s\n' "$RESOLVED_URL"

  # Write clone-url.txt
  if [ -n "$URL_FILE" ]; then
    CLONE_URL_FILE="$URL_FILE"
  else
    # Walk up from compose dir to find .autospec/
    REPO_ROOT=""
    dir="$COMPOSE_DIR"
    for _ in $(seq 1 20); do
      if [ -d "$dir/.autospec" ]; then
        REPO_ROOT="$dir"
        break
      fi
      dir="$(dirname "$dir")"
    done
    if [ -n "$REPO_ROOT" ]; then
      CLONE_URL_FILE="$REPO_ROOT/.autospec/clone-url.txt"
    else
      CLONE_URL_FILE=".autospec/clone-url.txt"
    fi
  fi

  mkdir -p "$(dirname "$CLONE_URL_FILE")"
  printf '%s\n' "$RESOLVED_URL" > "$CLONE_URL_FILE"
  printf 'compose.sh: wrote %s\n' "$CLONE_URL_FILE"
  exit 0
fi

# ---------------------------------------------------------------------------
# DOWN action
# ---------------------------------------------------------------------------

if [ "$ACTION" = "down" ]; then
  printf 'compose.sh: stopping compose stack from %s\n' "$COMPOSE_FILE"
  docker compose -f "$COMPOSE_FILE" down -v 2>&1 || die "docker compose down failed"
  printf 'compose.sh: stack stopped\n'
  exit 0
fi
