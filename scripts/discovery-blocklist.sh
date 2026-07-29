#!/usr/bin/env bash
# discovery-blocklist.sh — single source of truth for the discovery engine's
# forbidden source classes and the seed/probation allowlist gate. Repo config MAY
# extend the block list (discovery.forbidden_classes / guardrails.extra_blocks); it
# MAY NOT remove a built-in. Mirrors growth-ethics-blocklist.sh. Fail-closed.
#
# Usage:
#   discovery-blocklist.sh --list
#   discovery-blocklist.sh --effective <cfg>
#   discovery-blocklist.sh --assert-not-weakened <cfg>
#   discovery-blocklist.sh --allowed <domain> <cfg>
set -euo pipefail

# Built-in forbidden source classes. This list is the immutable spine of the
# discovery trust boundary and can only be extended by config, never weakened.
BUILTIN_BLOCKS="paywalled
pastebin
social_dm
pii_bearing"

PROBATION_FILE="${AUTOSPEC_DISCOVERY_PROBATION:-.autospec/trends/probation.txt}"

usage() {
  echo "usage: discovery-blocklist.sh --list | --effective <cfg> | --assert-not-weakened <cfg> | --allowed <domain> <cfg>" >&2
  exit 2
}

# Read a config file (YAML or JSON) as JSON on stdout. yq handles both; when yq is
# absent we fall back to jq (JSON only). Fail-closed: empty output on parse error.
cfg_to_json() {
  local cfg="$1"
  if command -v yq >/dev/null 2>&1; then
    yq -o=json '.' "$cfg" 2>/dev/null
  else
    jq -e '.' "$cfg" 2>/dev/null
  fi
}

cmd="${1:-}"; shift || true
case "$cmd" in
  --list)
    printf '%s\n' "$BUILTIN_BLOCKS"
    ;;

  --effective)
    cfg="${1:?config path required}"
    [ -f "$cfg" ] || { echo "config not found: $cfg" >&2; exit 2; }
    json="$(cfg_to_json "$cfg")"
    if [ -z "$json" ]; then echo "cannot parse config: $cfg" >&2; exit 1; fi
    extra="$(printf '%s' "$json" | jq -r '
      [ (.discovery.forbidden_classes // [])[], (.guardrails.extra_blocks // [])[] ] | .[]' 2>/dev/null || true)"
    printf '%s\n%s\n' "$BUILTIN_BLOCKS" "$extra" | awk 'NF' | sort -u
    ;;

  --assert-not-weakened)
    cfg="${1:?config path required}"
    [ -f "$cfg" ] || { echo "config not found: $cfg" >&2; exit 2; }
    json="$(cfg_to_json "$cfg")"
    if [ -z "$json" ]; then echo "cannot parse config: $cfg" >&2; exit 1; fi
    # Fail closed if the config names a builtin anywhere under .guardrails or
    # .discovery EXCEPT the two legitimate extend-only lists
    # (.guardrails.extra_blocks and .discovery.forbidden_classes). Any other
    # occurrence of a builtin name reads as an attempt to allow/disable it.
    if ! strings="$(printf '%s' "$json" | jq -r '
        [ (.guardrails // {} | del(.extra_blocks) | .. | strings),
          (.discovery  // {} | del(.forbidden_classes) | .. | strings) ] | .[]' 2>/dev/null)"; then
      echo "cannot evaluate config guardrails: $cfg" >&2; exit 1
    fi
    denied=""
    while IFS= read -r s; do
      [ -n "$s" ] || continue
      if printf '%s\n' "$BUILTIN_BLOCKS" | grep -qFx "$s"; then denied="$denied $s"; fi
    done <<EOF
$strings
EOF
    if [ -n "$denied" ]; then
      echo "discovery config attempts to weaken built-in forbidden classes:$denied" >&2
      exit 1
    fi
    exit 0
    ;;

  --allowed)
    domain="${1:?domain required}"; shift || true
    cfg="${1:?config path required}"
    [ -f "$cfg" ] || { echo "config not found: $cfg" >&2; exit 2; }
    json="$(cfg_to_json "$cfg")"
    if [ -z "$json" ]; then echo "cannot parse config: $cfg" >&2; exit 1; fi
    seeds="$(printf '%s' "$json" | jq -r '(.discovery.seed_sources // [])[]' 2>/dev/null || true)"
    probation=""
    if [ -f "$PROBATION_FILE" ]; then
      probation="$(cat "$PROBATION_FILE")"
    fi
    allowed=1
    while IFS= read -r entry; do
      [ -n "$entry" ] || continue
      if [ "$domain" = "$entry" ]; then allowed=0; break; fi
      # subdomain match: exact literal suffix ".$entry" (no regex — injected
      # metacharacters must match literally or not at all).
      case "$domain" in
        *".$entry") allowed=0; break ;;
      esac
    done <<EOF
$seeds
$probation
EOF
    if [ "$allowed" -eq 0 ]; then
      exit 0
    fi
    echo "discovery: domain not on seed allowlist or probation list: $domain" >&2
    exit 1
    ;;

  *) usage ;;
esac
