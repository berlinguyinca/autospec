#!/usr/bin/env bash
# gap-json-lib.sh — shared helpers for the gap JSON contract.
#
# Gap object schema (emitted by `/autospec-review --remediation --emit-gaps`):
#   {gap_id, dimension, severity, file, line, title, body, dedupe_key,
#    attribution_status?, originating_pr?, originating_commit?,
#    review_receipt_digest?, reviewer_harness?, reviewer_reasoning?,
#    provider_diversified?, review_risk?}
#
# Sourceable OR runnable:
#   bash gap-json-lib.sh --validate-file <path>   # exit 0 if file holds a valid gap object
#   bash gap-json-lib.sh --title-hash "<title>"   # print 10-char sha1 hex (dedupe fallback)
#   bash gap-json-lib.sh --selftest               # internal smoke check
#   bash gap-json-lib.sh --help                   # usage
#
# When sourced, exposes:
#   gap_required_keys                  # echoes the required key list
#   gap_validate_object <json-string>  # returns 0/1
#   gap_title_hash <title-string>      # prints 10-char hex
#
# Exit codes:
#   0  ok / valid
#   1  invalid object, missing file, or missing jq
#
# Requires: bash 3.2+, jq, shasum (or sha1sum)

set +e

GAP_REQUIRED_KEYS="gap_id dimension severity file line title body dedupe_key"

gap_required_keys() { printf '%s\n' "$GAP_REQUIRED_KEYS"; }

# gap_validate_object <json-string> — 0 if all required keys present (non-null).
gap_validate_object() {
  local obj="$1" key
  command -v jq >/dev/null 2>&1 || return 1
  printf '%s' "$obj" | jq -e 'type == "object"' >/dev/null 2>&1 || return 1
  for key in $GAP_REQUIRED_KEYS; do
    printf '%s' "$obj" | jq -e --arg k "$key" 'has($k) and (.[$k] != null)' >/dev/null 2>&1 || return 1
  done
  # Historical gaps without attribution remain valid. New normalized gaps carry
  # an explicit status: attributed rows must have the complete typed identity;
  # unavailable rows must not invent partial origin data.
  if printf '%s' "$obj" | jq -e 'has("attribution_status")' >/dev/null 2>&1; then
    printf '%s' "$obj" | jq -e '
      if .attribution_status == "attributed" then
        (.originating_pr | type == "number") and
        (.originating_commit | type == "string" and length > 0) and
        (.review_receipt_digest | type == "string" and length > 0) and
        (.reviewer_harness | type == "string" and length > 0) and
        (.reviewer_reasoning | type == "string" and length > 0) and
        (.provider_diversified | type == "boolean") and
        (.review_risk | type == "string" and length > 0)
      elif .attribution_status == "unavailable" then
        (.originating_pr == null) and (.originating_commit == null) and
        (.review_receipt_digest == null) and (.reviewer_harness == null) and
        (.reviewer_reasoning == null) and (.provider_diversified == null) and
        (.review_risk == null)
      else false end' >/dev/null 2>&1 || return 1
  fi
  return 0
}

gap_sha256() {
  local value="$1" sum
  if command -v shasum >/dev/null 2>&1; then
    sum="$(printf '%s' "$value" | shasum -a 256 | awk '{print $1}')"
  else
    sum="$(printf '%s' "$value" | sha256sum | awk '{print $1}')"
  fi
  printf 'sha256:%s' "$sum"
}

# gap_title_hash <title> — deterministic 10-char hex for dedupe fallback.
gap_title_hash() {
  local title="$1" sum
  if command -v shasum >/dev/null 2>&1; then
    sum="$(printf '%s' "$title" | shasum | awk '{print $1}')"
  else
    sum="$(printf '%s' "$title" | sha1sum | awk '{print $1}')"
  fi
  printf '%s' "${sum:0:10}"
}

# ── Runnable entrypoint (skipped when sourced) ────────────────────────────────
if [ "${BASH_SOURCE[0]}" = "${0}" ]; then
  case "${1:-}" in
    --validate-file)
      [ -f "${2:-}" ] || exit 1
      gap_validate_object "$(cat "$2")" || exit 1
      exit 0
      ;;
    --title-hash)
      gap_title_hash "${2:-}"
      printf '\n'
      exit 0
      ;;
    --selftest)
      gap_validate_object '{"gap_id":"G1","dimension":"correctness","severity":"low","file":"a","line":1,"title":"t","body":"b","dedupe_key":"k"}' || exit 1
      gap_validate_object '{"gap_id":"G1"}' && exit 1
      h="$(gap_title_hash "abc")"; [ "${#h}" -eq 10 ] || exit 1
      exit 0
      ;;
    --help|-h)
      printf 'Usage: gap-json-lib.sh --validate-file <path> | --title-hash <title> | --selftest\n'
      exit 0
      ;;
    *)
      printf 'Usage: gap-json-lib.sh --validate-file <path> | --title-hash <title> | --selftest\n' >&2
      exit 0
      ;;
  esac
fi
