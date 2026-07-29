#!/usr/bin/env bash
# validate-outbound-draft.sh — validate an outbound draft record. Fail-closed.
set -euo pipefail

draft="${1:-}"
[ -n "$draft" ] || { echo "usage: validate-outbound-draft.sh <draft.json>" >&2; exit 2; }
[ -f "$draft" ] || { echo "draft not found: $draft" >&2; exit 2; }
if ! jq . "$draft" >/dev/null 2>&1; then
  echo "malformed json: $draft" >&2; exit 1
fi

# Emit the first failing reason, exit 1. Types checked explicitly (a numeric
# "issue" as a string must fail; a non-array evidence must fail).
reason="$(jq -r '
  if (.issue|type) != "number" or ((.issue|floor) != .issue) then "issue must be an integer"
  elif (.platform|type) != "string" or (.platform|length)==0 then "platform must be a non-empty string"
  elif (.target_url|type) != "string" or (.target_url|length)==0 then "target_url must be a non-empty string"
  elif (.body|type) != "string" or (.body|length)==0 then "body must be a non-empty string"
  elif (.self_promo_rule|type) != "string" or (.self_promo_rule|length)==0 then "self_promo_rule must be a non-empty string"
  elif (has("disclosure") and (.disclosure|type)!="string") then "disclosure must be a string when present"
  elif (.evidence|type) != "array" then "evidence must be an array"
  elif ([.evidence[]|select(type!="string")]|length) > 0 then "evidence items must be strings"
  else "" end' "$draft")"

if [ -n "$reason" ]; then
  echo "invalid draft: $reason" >&2; exit 1
fi
exit 0
