#!/usr/bin/env bash
# skills/autospec-shared/scripts/record-telemetry.sh — Append one JSONL telemetry row per dispatch.
#
# Usage:
#   record-telemetry.sh --dispatch-id <id> --role <role> --issue <n> --tokens-json <file>
#
# Reads token counts from <file> (JSON with keys: input_tokens, output_tokens,
# cache_creation_input_tokens, cache_read_input_tokens) and appends one line to
# $AUTOSPEC_TELEMETRY_FILE (default: ~/.autospec/telemetry.jsonl).
# Creates the parent directory if missing.
#
# Environment overrides (for testing):
#   AUTOSPEC_TELEMETRY_FILE — path to telemetry JSONL (default: ~/.autospec/telemetry.jsonl)
#
# Exit codes:
#   0  Success
#   1  Argument/file error
#
# Compatible with bash 3.2+

set -eu

DISPATCH_ID=""
ROLE=""
ISSUE=""
TOKENS_JSON=""

while [ $# -gt 0 ]; do
  case "$1" in
    --dispatch-id)
      if [ $# -lt 2 ]; then printf 'record-telemetry.sh: --dispatch-id requires an argument\n' >&2; exit 1; fi
      DISPATCH_ID="$2"; shift 2 ;;
    --role)
      if [ $# -lt 2 ]; then printf 'record-telemetry.sh: --role requires an argument\n' >&2; exit 1; fi
      ROLE="$2"; shift 2 ;;
    --issue)
      if [ $# -lt 2 ]; then printf 'record-telemetry.sh: --issue requires an argument\n' >&2; exit 1; fi
      ISSUE="$2"; shift 2 ;;
    --tokens-json)
      if [ $# -lt 2 ]; then printf 'record-telemetry.sh: --tokens-json requires an argument\n' >&2; exit 1; fi
      TOKENS_JSON="$2"; shift 2 ;;
    -*)
      printf 'record-telemetry.sh: unknown option: %s\n' "$1" >&2; exit 1 ;;
    *)
      printf 'record-telemetry.sh: unexpected argument: %s\n' "$1" >&2; exit 1 ;;
  esac
done

if [ -z "$DISPATCH_ID" ] || [ -z "$ROLE" ] || [ -z "$ISSUE" ] || [ -z "$TOKENS_JSON" ]; then
  printf 'Usage: record-telemetry.sh --dispatch-id <id> --role <role> --issue <n> --tokens-json <file>\n' >&2
  exit 1
fi

if [ ! -f "$TOKENS_JSON" ]; then
  printf 'record-telemetry.sh: tokens-json file not found: %s\n' "$TOKENS_JSON" >&2
  exit 1
fi

TELEMETRY_FILE="${AUTOSPEC_TELEMETRY_FILE:-$HOME/.autospec/telemetry.jsonl}"
mkdir -p "$(dirname "$TELEMETRY_FILE")"

# Read token fields; default missing cache fields to 0
input_tokens=$(jq '.input_tokens // 0' "$TOKENS_JSON")
output_tokens=$(jq '.output_tokens // 0' "$TOKENS_JSON")
cache_creation=$(jq '.cache_creation_input_tokens // 0' "$TOKENS_JSON")
cache_read=$(jq '.cache_read_input_tokens // 0' "$TOKENS_JSON")

ts=$(date -u +"%Y-%m-%dT%H:%M:%SZ")

jq -cn \
  --arg     ts          "$ts" \
  --arg     role        "$ROLE" \
  --arg     issue       "$ISSUE" \
  --argjson input       "$input_tokens" \
  --argjson cache_c     "$cache_creation" \
  --argjson cache_r     "$cache_read" \
  --argjson output      "$output_tokens" \
  --arg     dispatch_id "$DISPATCH_ID" \
  '{ts:$ts,role:$role,issue:$issue,input_tokens:$input,cache_creation_input_tokens:$cache_c,cache_read_input_tokens:$cache_r,output_tokens:$output,dispatch_id:$dispatch_id}' \
  >> "$TELEMETRY_FILE"
