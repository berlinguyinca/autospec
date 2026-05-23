#!/usr/bin/env bash
# skills/autospec-e2e-clone/scripts/expose/dispatch.sh
#
# Routes expose actions to the correct adapter based on expose.kind in the
# resolved contract JSON (read from stdin or --contract flag).
#
# Usage:
#   dispatch.sh <action> --contract <resolved-contract.json> [adapter-args...]
#   echo '<contract-json>' | dispatch.sh <action> [adapter-args...]
#
# Supported expose.kind values:
#   docker_compose            → compose.sh  (C7, this PR)
#   k8s_ephemeral_namespace   → k8s.sh      (C8, deferred)
#   dedicated_staging_slot    → staging.sh  (C8, deferred)
#   custom_cmd                → custom.sh   (C8, deferred)
#
# Exit codes:
#   0  success (delegated to adapter)
#   1  fatal (missing deps, bad contract)
#   2  refuse-to-run (unknown expose.kind, adapter refused)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

die()    { printf 'dispatch.sh: fatal: %s\n' "$*" >&2; exit 1; }
refuse() { printf 'dispatch.sh: refuse-to-run: %s\n' "$*" >&2; exit 2; }

# ---------------------------------------------------------------------------
# Arg parsing
# ---------------------------------------------------------------------------

ACTION="${1:-}"
[ -n "$ACTION" ] || die "action required: up or down"
shift

CONTRACT_FILE=""
REMAINING_ARGS=()

while [ $# -gt 0 ]; do
  case "$1" in
    --contract) CONTRACT_FILE="$2"; shift 2 ;;
    *)          REMAINING_ARGS+=("$1"); shift ;;
  esac
done

# ---------------------------------------------------------------------------
# Load contract JSON
# ---------------------------------------------------------------------------

if [ -n "$CONTRACT_FILE" ]; then
  [ -f "$CONTRACT_FILE" ] || die "contract file not found: $CONTRACT_FILE"
  CONTRACT_JSON="$(cat "$CONTRACT_FILE")"
elif [ ! -t 0 ]; then
  CONTRACT_JSON="$(cat)"
else
  die "contract JSON required: pass --contract <file> or pipe JSON to stdin"
fi

[ -n "$CONTRACT_JSON" ] || die "contract JSON is empty"

# ---------------------------------------------------------------------------
# Extract expose.kind and expose fields
# ---------------------------------------------------------------------------

require_tool() {
  command -v "$1" >/dev/null 2>&1 || die "$1 not found"
}

require_tool python3

expose_kind=$(printf '%s' "$CONTRACT_JSON" | \
  python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('expose',{}).get('kind',''))" 2>/dev/null || true)

[ -n "$expose_kind" ] || die "expose.kind not found in contract"

expose_compose_file=$(printf '%s' "$CONTRACT_JSON" | \
  python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('expose',{}).get('compose_file',''))" 2>/dev/null || true)

expose_url_template=$(printf '%s' "$CONTRACT_JSON" | \
  python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('expose',{}).get('url_template','http://localhost:8080'))" 2>/dev/null || echo "http://localhost:8080")

expose_health_endpoint=$(printf '%s' "$CONTRACT_JSON" | \
  python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('expose',{}).get('health_endpoint','/health'))" 2>/dev/null || echo "/health")

expose_ready_wait=$(printf '%s' "$CONTRACT_JSON" | \
  python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('expose',{}).get('ready_wait_secs',60))" 2>/dev/null || echo "60")

# ---------------------------------------------------------------------------
# Dispatch by expose.kind
# ---------------------------------------------------------------------------

case "$expose_kind" in
  docker_compose)
    [ -n "$expose_compose_file" ] || die "expose.compose_file is required for docker_compose kind"
    exec bash "$SCRIPT_DIR/compose.sh" "$ACTION" "$expose_compose_file" \
      --url "$expose_url_template" \
      --health "$expose_health_endpoint" \
      --wait "$expose_ready_wait" \
      "${REMAINING_ARGS[@]+"${REMAINING_ARGS[@]}"}"
    ;;

  k8s_ephemeral_namespace|dedicated_staging_slot|custom_cmd)
    refuse "expose.kind '${expose_kind}' is not yet implemented (deferred to C8)"
    ;;

  *)
    refuse "unknown expose.kind '${expose_kind}'. Supported: docker_compose (k8s_ephemeral_namespace/dedicated_staging_slot/custom_cmd deferred to C8)"
    ;;
esac
