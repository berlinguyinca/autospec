#!/usr/bin/env bash
# emit-gaps.sh — shape filtered review findings into the gap JSON contract.
#
# Input findings JSON: array of objects with at least
#   {dimension, severity, file, line, title, body, verdict, dedupe_key}
# where verdict is "keep" or "false_positive" (the evaluate-findings/critic verdict).
# This shaper keeps only verdict=="keep", assigns a stable gap_id (G<n> in order),
# fills a dedupe_key fallback (title-hash) when absent, and writes the gap contract:
#   {gap_id, dimension, severity, file, line, title, body, dedupe_key}
#
# Usage:
#   emit-gaps.sh --findings <path> --out <path>
#   emit-gaps.sh --help
#
# Environment:
#   AUTOSPEC_SCRIPTS_DIR — sibling scripts dir (default: script dir)
#
# Exit codes:
#   0  success (empty/missing input → empty array written)
#   1  jq missing
#
# Requires: bash 3.2+, jq

set +e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
AUTOSPEC_SCRIPTS_DIR="${AUTOSPEC_SCRIPTS_DIR:-$SCRIPT_DIR}"

# shellcheck source=gap-json-lib.sh
. "$AUTOSPEC_SCRIPTS_DIR/gap-json-lib.sh"

FINDINGS=""
OUT=""

while [ $# -gt 0 ]; do
  case "$1" in
    --findings) FINDINGS="$2"; shift 2 ;;
    --out) OUT="$2"; shift 2 ;;
    --help|-h)
      printf 'Usage: emit-gaps.sh --findings <path> --out <path>\n'
      exit 0
      ;;
    *) shift ;;
  esac
done

command -v jq >/dev/null 2>&1 || { printf 'emit-gaps: jq required\n' >&2; exit 1; }
[ -n "$OUT" ] || { printf 'emit-gaps: --out required\n' >&2; exit 1; }

if [ -z "$FINDINGS" ] || [ ! -f "$FINDINGS" ] || ! jq -e 'type=="array"' "$FINDINGS" >/dev/null 2>&1; then
  printf '[]\n' > "$OUT"
  exit 0
fi

# Keep verdict=="keep" (or missing verdict → treat as keep), shape to contract,
# assign G<n> gap_ids in order, and fill dedupe_key fallback with title-hash.
jq -c '[ .[] | select((.verdict // "keep") == "keep") ]' "$FINDINGS" > "$OUT.tmp"

_n="$(jq 'length' "$OUT.tmp")"
printf '[' > "$OUT"
_i=0
while [ "$_i" -lt "$_n" ]; do
  _f="$(jq -c ".[$_i]" "$OUT.tmp")"
  _gid="G$((_i + 1))"
  _dk="$(printf '%s' "$_f" | jq -r '.dedupe_key // empty')"
  if [ -z "$_dk" ]; then
    _title="$(printf '%s' "$_f" | jq -r '.title // ""')"
    _dk="$(gap_title_hash "$_title")"
  fi
  _obj="$(printf '%s' "$_f" | jq -c --arg gid "$_gid" --arg dk "$_dk" '{
    gap_id: $gid,
    dimension: (.dimension // "correctness"),
    severity: (.severity // "low"),
    file: (.file // ""),
    line: (.line // 0),
    title: (.title // ""),
    body: (.body // ""),
    dedupe_key: $dk
  }')"
  [ "$_i" -gt 0 ] && printf ',' >> "$OUT"
  printf '%s' "$_obj" >> "$OUT"
  _i=$((_i + 1))
done
printf ']\n' >> "$OUT"
rm -f "$OUT.tmp"
exit 0
