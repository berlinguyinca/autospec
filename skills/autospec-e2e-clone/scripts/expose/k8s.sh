#!/usr/bin/env bash
# skills/autospec-e2e-clone/scripts/expose/k8s.sh
#
# Kubernetes ephemeral-namespace expose adapter (C8).
#
# Actions:
#   up   — kubectl create ns <namespace>; kubectl apply -f <manifests_dir>;
#           poll health_endpoint; write clone-url.txt
#   down — kubectl delete ns <namespace>
#
# Usage:
#   k8s.sh up   <manifests-dir> <namespace> [--url <url>] [--health <endpoint>]
#               [--wait <secs>] [--url-file <path>]
#   k8s.sh down <namespace>
#
# Exit codes:
#   0  success
#   1  fatal (missing deps, file not found)
#   2  refuse-to-run (health check timed out, kubectl failed)

set -euo pipefail

die()    { printf 'k8s.sh: fatal: %s\n' "$*" >&2; exit 1; }
refuse() { printf 'k8s.sh: refuse-to-run: %s\n' "$*" >&2; exit 2; }

require_tool() {
  command -v "$1" >/dev/null 2>&1 || die "$1 not found. Install kubectl (kubernetes.io/docs/tasks/tools/)."
}

# ---------------------------------------------------------------------------
# Arg parsing
# ---------------------------------------------------------------------------

ACTION="${1:-}"
case "$ACTION" in
  up|down) shift ;;
  -h|--help) printf 'Usage: k8s.sh up <manifests-dir> <namespace> [...] | k8s.sh down <namespace>\n'; exit 0 ;;
  "") die "action required: up or down" ;;
  *) die "unknown action: $ACTION. Use 'up' or 'down'." ;;
esac

if [ "$ACTION" = "down" ]; then
  NAMESPACE="${1:-}"
  [ -n "$NAMESPACE" ] || die "namespace argument required for down"
  require_tool kubectl
  printf 'k8s.sh: deleting namespace %s\n' "$NAMESPACE"
  kubectl delete ns "$NAMESPACE" --ignore-not-found=true 2>&1 || die "kubectl delete ns failed"
  printf 'k8s.sh: namespace %s deleted\n' "$NAMESPACE"
  exit 0
fi

# up action
MANIFESTS_DIR="${1:-}"
[ -n "$MANIFESTS_DIR" ] || die "manifests-dir argument required"
shift
NAMESPACE="${1:-}"
[ -n "$NAMESPACE" ] || die "namespace argument required"
shift

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
    *)          die "unknown option: $1" ;;
  esac
done

require_tool kubectl

MANIFESTS_DIR="$(cd "$MANIFESTS_DIR" 2>/dev/null && pwd)" || die "manifests-dir not found: $MANIFESTS_DIR"

# ---------------------------------------------------------------------------
# UP: create ns, apply manifests
# ---------------------------------------------------------------------------

printf 'k8s.sh: creating namespace %s\n' "$NAMESPACE"
kubectl create ns "$NAMESPACE" 2>&1 || die "kubectl create ns failed"

printf 'k8s.sh: applying manifests from %s\n' "$MANIFESTS_DIR"
kubectl apply -f "$MANIFESTS_DIR" -n "$NAMESPACE" 2>&1 || refuse "kubectl apply failed"

# ---------------------------------------------------------------------------
# URL substitution (k8s: host = cluster ingress or localhost via port-forward)
# ---------------------------------------------------------------------------

RESOLVED_URL="${URL_TEMPLATE//\{\{host\}\}/localhost}"
# {{port}} substitution: extract first nodePort/containerPort from manifests
if [[ "$RESOLVED_URL" == *"{{port}}"* ]]; then
  port=$(grep -rE 'nodePort:|containerPort:' "$MANIFESTS_DIR" | grep -oE '[0-9]{4,5}' | head -1 || true)
  [ -n "$port" ] || die "could not resolve {{port}} from manifests; specify --url with a literal port"
  RESOLVED_URL="${RESOLVED_URL//\{\{port\}\}/$port}"
fi

TARGET_URL="${RESOLVED_URL}${HEALTH_ENDPOINT}"

# ---------------------------------------------------------------------------
# Poll health endpoint
# ---------------------------------------------------------------------------

printf 'k8s.sh: polling health endpoint %s (timeout: %ss)\n' "$TARGET_URL" "$READY_WAIT_SECS"

elapsed=0
interval=3
healthy=false
while [ "$elapsed" -lt "$READY_WAIT_SECS" ]; do
  http_code=$(curl -s -o /dev/null -w '%{http_code}' --max-time 5 "$TARGET_URL" 2>/dev/null || true)
  if [ "$http_code" = "200" ]; then
    healthy=true
    break
  fi
  printf 'k8s.sh: waiting... (%ss elapsed, last HTTP %s)\n' "$elapsed" "${http_code:-000}"
  sleep "$interval"
  elapsed=$(( elapsed + interval ))
done

if [ "$healthy" != "true" ]; then
  refuse "health check timed out after ${READY_WAIT_SECS}s — ${TARGET_URL} did not return 200"
fi

printf 'k8s.sh: stack healthy — URL: %s\n' "$RESOLVED_URL"

# ---------------------------------------------------------------------------
# Write clone-url.txt
# ---------------------------------------------------------------------------

if [ -z "$URL_FILE" ]; then
  URL_FILE=".autospec/clone-url.txt"
fi
mkdir -p "$(dirname "$URL_FILE")"
printf '%s\n' "$RESOLVED_URL" > "$URL_FILE"
printf 'k8s.sh: wrote %s\n' "$URL_FILE"
exit 0
