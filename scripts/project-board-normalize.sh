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

# Families are matched with capture(), never test() with an interpolated value:
# a label such as `priority:.*` must not become part of the pattern.
jq --argjson map "$map_json" '
  def canon($family; $raw):
    ($map[$family][$raw] // null) as $mapped
    | if $mapped != null then $mapped
      elif $family == "priority" then
        {"p0":"critical","critical":"critical","p1":"high","high":"high",
         "p2":"normal","normal":"normal","p3":"low","low":"low"}[$raw] // null
      elif $family == "ctx" then
        (if ($raw | test("^[0-9]+k$")) then $raw else null end)
      elif $family == "reasoning" then
        {"deep":"deep","medium":"medium","light":"light"}[$raw] // null
      elif $family == "risk" or $family == "area" then
        (if ($raw | test("^[a-z0-9-]+$")) then $raw else null end)
      else null end;

  def family_value($labels; $family):
    ($labels
     | map(capture("^(?<f>priority|ctx|reasoning|risk|area)[:/](?<v>.+)$") // empty)
     | map(select(.f == $family) | .v)
     | first) as $raw
    | if $raw == null then null else canon($family; $raw) end;

  .items |= map(
    . + {normalized: {
      priority:  family_value(.labels; "priority"),
      ctx:       family_value(.labels; "ctx"),
      reasoning: family_value(.labels; "reasoning"),
      risk:      family_value(.labels; "risk"),
      area:      family_value(.labels; "area")
    }})'
