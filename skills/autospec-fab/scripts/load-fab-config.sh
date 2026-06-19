#!/usr/bin/env bash
# load-fab-config.sh — parse .autospec/fab.yml → normalized JSON on stdout.
#
# Usage:
#   load-fab-config.sh [--validate] <path-to-fab.yml>
#
# Flags:
#   --validate   Strict mode: exit non-zero (with message to stderr) when a
#                required key is missing or models list is empty.
#
# Output:
#   Normalized JSON to stdout with defaults applied:
#     generator      default: "rm -rf build && .venv/bin/python src/generate.py"
#     stl_roots      default: ["build/stls/manifolds/"]
#     printer        defaults: nozzle_width=0.4, layer_height=0.2,
#                              min_perimeters=3, max_overhang_deg=45
#     metadata_sidecar  (passed through as-is; null if absent)
#     models[]       (passed through as-is)
#
# Required by --validate: generator (non-empty), models (non-empty list).
#
# Exit codes:
#   0  success
#   1  usage / file-not-found / parse error / --validate failure

set -uo pipefail

# ── Constants ─────────────────────────────────────────────────────────────────

DEFAULT_GENERATOR="rm -rf build && .venv/bin/python src/generate.py"
DEFAULT_STL_ROOTS='["build/stls/manifolds/"]'
DEFAULT_NOZZLE_WIDTH="0.4"
DEFAULT_LAYER_HEIGHT="0.2"
DEFAULT_MIN_PERIMETERS="3"
DEFAULT_MAX_OVERHANG_DEG="45"

# ── Argument parsing ──────────────────────────────────────────────────────────

VALIDATE=0
FAB_YML=""

while [ $# -gt 0 ]; do
  case "$1" in
    --validate)
      VALIDATE=1
      shift
      ;;
    -*)
      printf 'load-fab-config: unknown flag: %s\n' "$1" >&2
      exit 1
      ;;
    *)
      FAB_YML="$1"
      shift
      ;;
  esac
done

if [ -z "$FAB_YML" ]; then
  printf 'Usage: load-fab-config.sh [--validate] <path-to-fab.yml>\n' >&2
  exit 1
fi

# ── Dependency checks ─────────────────────────────────────────────────────────

if ! command -v yq >/dev/null 2>&1; then
  printf 'load-fab-config: yq is required but not found in PATH\n' >&2
  exit 1
fi

if ! command -v jq >/dev/null 2>&1; then
  printf 'load-fab-config: jq is required but not found in PATH\n' >&2
  exit 1
fi

# ── File existence ────────────────────────────────────────────────────────────

if [ ! -f "$FAB_YML" ]; then
  printf 'load-fab-config: file not found: %s\n' "$FAB_YML" >&2
  exit 1
fi

# ── Parse YAML → raw JSON via yq ─────────────────────────────────────────────

RAW_JSON=""
RAW_JSON="$(yq -o=json '.' "$FAB_YML" 2>&1)" || {
  printf 'load-fab-config: failed to parse %s: %s\n' "$FAB_YML" "$RAW_JSON" >&2
  exit 1
}

# ── Extract fields with defaults ──────────────────────────────────────────────

# generator: use value from file if present and non-null/non-empty, else default
GENERATOR="$(printf '%s' "$RAW_JSON" | jq -r '.generator // empty' 2>/dev/null || true)"
if [ -z "$GENERATOR" ] || [ "$GENERATOR" = "null" ]; then
  GENERATOR="$DEFAULT_GENERATOR"
fi

# stl_roots: use value from file if present and non-empty array, else default
STL_ROOTS_LEN="$(printf '%s' "$RAW_JSON" | jq 'if .stl_roots and (.stl_roots | length) > 0 then 1 else 0 end' 2>/dev/null || printf '0')"
if [ "$STL_ROOTS_LEN" -eq 1 ]; then
  STL_ROOTS="$(printf '%s' "$RAW_JSON" | jq -c '.stl_roots')"
else
  STL_ROOTS="$DEFAULT_STL_ROOTS"
fi

# printer fields with per-key defaults
PRINTER_NOZZLE="$(printf '%s' "$RAW_JSON" | jq -r '.printer.nozzle_width // empty' 2>/dev/null || true)"
if [ -z "$PRINTER_NOZZLE" ] || [ "$PRINTER_NOZZLE" = "null" ]; then
  PRINTER_NOZZLE="$DEFAULT_NOZZLE_WIDTH"
fi

PRINTER_LAYER="$(printf '%s' "$RAW_JSON" | jq -r '.printer.layer_height // empty' 2>/dev/null || true)"
if [ -z "$PRINTER_LAYER" ] || [ "$PRINTER_LAYER" = "null" ]; then
  PRINTER_LAYER="$DEFAULT_LAYER_HEIGHT"
fi

PRINTER_MIN_PERIM="$(printf '%s' "$RAW_JSON" | jq -r '.printer.min_perimeters // empty' 2>/dev/null || true)"
if [ -z "$PRINTER_MIN_PERIM" ] || [ "$PRINTER_MIN_PERIM" = "null" ]; then
  PRINTER_MIN_PERIM="$DEFAULT_MIN_PERIMETERS"
fi

PRINTER_MAX_OVERHANG="$(printf '%s' "$RAW_JSON" | jq -r '.printer.max_overhang_deg // empty' 2>/dev/null || true)"
if [ -z "$PRINTER_MAX_OVERHANG" ] || [ "$PRINTER_MAX_OVERHANG" = "null" ]; then
  PRINTER_MAX_OVERHANG="$DEFAULT_MAX_OVERHANG_DEG"
fi

# metadata_sidecar: pass through (may be null/absent)
METADATA_SIDECAR="$(printf '%s' "$RAW_JSON" | jq -c '.metadata_sidecar // null')"

# models: pass through (may be absent/null → empty array)
MODELS_LEN="$(printf '%s' "$RAW_JSON" | jq 'if .models and (.models | type) == "array" then (.models | length) else 0 end' 2>/dev/null || printf '0')"
if [ "$MODELS_LEN" -gt 0 ]; then
  MODELS="$(printf '%s' "$RAW_JSON" | jq -c '.models')"
else
  MODELS="[]"
fi

# ── --validate strict checks ──────────────────────────────────────────────────

if [ "$VALIDATE" -eq 1 ]; then
  ERRORS=0

  # Check generator is explicitly present in the file (not just defaulted)
  FILE_GEN="$(printf '%s' "$RAW_JSON" | jq -r '.generator // empty' 2>/dev/null || true)"
  if [ -z "$FILE_GEN" ] || [ "$FILE_GEN" = "null" ]; then
    printf 'load-fab-config: --validate failed: required key "generator" is missing or empty\n' >&2
    ERRORS=1
  fi

  # Check models is a non-empty list
  if [ "$MODELS_LEN" -eq 0 ]; then
    printf 'load-fab-config: --validate failed: "models" must be a non-empty list\n' >&2
    ERRORS=1
  fi

  if [ "$ERRORS" -ne 0 ]; then
    exit 1
  fi
fi

# ── Emit normalized JSON ──────────────────────────────────────────────────────

jq -cn \
  --arg generator "$GENERATOR" \
  --argjson stl_roots "$STL_ROOTS" \
  --argjson nozzle_width "$PRINTER_NOZZLE" \
  --argjson layer_height "$PRINTER_LAYER" \
  --argjson min_perimeters "$PRINTER_MIN_PERIM" \
  --argjson max_overhang_deg "$PRINTER_MAX_OVERHANG" \
  --argjson metadata_sidecar "$METADATA_SIDECAR" \
  --argjson models "$MODELS" \
  '{
    generator: $generator,
    stl_roots: $stl_roots,
    printer: {
      nozzle_width: $nozzle_width,
      layer_height: $layer_height,
      min_perimeters: $min_perimeters,
      max_overhang_deg: $max_overhang_deg
    },
    metadata_sidecar: $metadata_sidecar,
    models: $models
  }'
