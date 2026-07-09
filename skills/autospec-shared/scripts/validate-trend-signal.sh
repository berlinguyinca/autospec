#!/usr/bin/env bash
# validate-trend-signal.sh — fail-closed validator for one trend-signal JSON object.
# Usage: validate-trend-signal.sh <signal.json>   (exit 0 valid)
# Exit codes: 0 valid, 1 missing/invalid key or non-object, 2 invalid JSON / unreadable input.
set -euo pipefail

C="${1:?usage: validate-trend-signal.sh <signal.json>}"

if ! jq -e . "$C" >/dev/null 2>&1; then echo "not valid JSON: $C" >&2; exit 2; fi
jq -e 'type=="object"' "$C" >/dev/null 2>&1 || { echo "signal invalid: must be a JSON object" >&2; exit 1; }

fail() { echo "signal invalid: $1" >&2; exit 1; }

REQUIRED_KEYS=(source kind summary norm_key evidence_ref first_seen recurrence sanitized_excerpt ts)
for k in "${REQUIRED_KEYS[@]}"; do
  jq -e --arg k "$k" 'has($k)' "$C" >/dev/null 2>&1 || fail "missing required key: $k"
done

jq -e '(.source|type)=="string" and (.source|length>0)' "$C" >/dev/null || fail "source must be a non-empty string"
jq -e '(.kind|type)=="string" and (.kind|length>0)' "$C" >/dev/null || fail "kind must be a non-empty string"
jq -e '(.summary|type)=="string" and (.summary|length>0)' "$C" >/dev/null || fail "summary must be a non-empty string"
jq -e '(.norm_key|type)=="string" and (.norm_key|length>0)' "$C" >/dev/null || fail "norm_key must be a non-empty string"
jq -e '(.evidence_ref|type)=="string" and (.evidence_ref|length>0)' "$C" >/dev/null || fail "evidence_ref must be a non-empty string"
jq -e '(.first_seen|type)=="string" and (.first_seen|length>0)' "$C" >/dev/null || fail "first_seen must be a non-empty string"
jq -e '(.recurrence|type)=="number" and (.recurrence == (.recurrence|floor))' "$C" >/dev/null || fail "recurrence must be an integer"
jq -e '(.sanitized_excerpt|type)=="string"' "$C" >/dev/null || fail "sanitized_excerpt must be a string"
jq -e '(.ts|type)=="string" and (.ts|length>0)' "$C" >/dev/null || fail "ts must be a non-empty string"

exit 0
