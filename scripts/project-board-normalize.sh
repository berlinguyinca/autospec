#!/usr/bin/env bash
# scripts/project-board-normalize.sh — board labels → normalized attributes.
#
# Reads a board plan on stdin, writes the same plan with .items[].normalized added.
# Pure text: no network, no mutation. Never fails on unknown input — an
# unrecognized label normalizes to null.
#
# Usage: project-board-normalize.sh [--label-map FILE] < plan.json

set -eu

label_map=""
while [ "$#" -gt 0 ]; do
    case "$1" in
        --label-map) label_map="${2:-}"; shift 2 ;;
        --help|-h) printf 'project-board-normalize.sh [--label-map FILE] < plan.json\n'; exit 0 ;;
        *) printf 'project-board-normalize: unknown option: %s\n' "$1" >&2; exit 2 ;;
    esac
done

# The map is advisory. An unreadable or malformed map degrades to the fallback
# regex rather than failing the cycle.
map_json='{}'
if [ -n "$label_map" ] && [ -f "$label_map" ] && command -v yq >/dev/null 2>&1; then
    map_json="$(yq -o=json '.' "$label_map" 2>/dev/null || printf '{}')"
    [ -n "$map_json" ] || map_json='{}'
fi

# Validate and guard the map shape. If it's not an object, degrade to empty object.
map_json="$(printf '%s' "$map_json" | jq -r 'if type == "object" then . else {} end' 2>/dev/null || printf '{}')"

# Parse stdin and guard degenerate cases. Never fail on malformed input.
# If stdin is not JSON or has missing/wrong-shaped fields, pass it through
# unchanged or emit it with no normalized attribute added, then exit 0.
stdin_data="$(cat)"

# Try to parse JSON. If it fails, exit 0 silently with no output.
if ! parsed="$(printf '%s' "$stdin_data" | jq '.' 2>/dev/null)"; then
    exit 0
fi

# Ensure .items exists and is an array. If missing or wrong type, pass through unchanged.
if ! printf '%s' "$parsed" | jq -e '.items | type == "array"' >/dev/null 2>&1; then
    printf '%s' "$parsed"
    exit 0
fi

# Process items with defensive handling for missing/null/wrong-typed labels.
printf '%s' "$parsed" | jq --argjson map "$map_json" '
  def canon($family; $raw):
    ($map[$family][$raw] // null) as $mapped
    | if $mapped != null then $mapped
      elif $family == "priority" then
        {"p0":"critical","critical":"critical","p1":"high","high":"high",
         "p2":"normal","normal":"normal","p3":"low","low":"low"}[$raw] // null
      elif $family == "ctx" then
        (if ($raw | type == "string" and test("^[0-9]+k$")) then $raw else null end)
      elif $family == "reasoning" then
        {"deep":"deep","medium":"medium","light":"light"}[$raw] // null
      elif $family == "risk" or $family == "area" then
        (if ($raw | type == "string" and test("^[a-z0-9-]+$")) then $raw else null end)
      else null end;

  def family_value($labels; $family):
    (($labels // [] | select(type == "array")) as $arr
     | if $arr == null or ($arr | length == 0) then null
       else
         ($arr
          | map(select(type == "string") | capture("^(?<f>priority|ctx|reasoning|risk|area)[:/](?<v>.+)$") // empty)
          | map(select(.f == $family) | .v)
          | first) as $raw
         | if $raw == null then null else canon($family; $raw) end
       end);

  .items |= map(
    . + {normalized: {
      priority:  family_value(.labels; "priority"),
      ctx:       family_value(.labels; "ctx"),
      reasoning: family_value(.labels; "reasoning"),
      risk:      family_value(.labels; "risk"),
      area:      family_value(.labels; "area")
    }})'

