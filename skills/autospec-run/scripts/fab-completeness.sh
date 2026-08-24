#!/usr/bin/env bash
# fab-completeness.sh — Phase 5.5 fab-completeness dimension (issue #1236).
#
# After an autospec-run fab batch drains, Phase 5.5's gap-remediation review
# also asserts that every printable model actually shipped its proof artifacts.
# For each printable model this helper asserts:
#   (a) its 16-view contact sheet exists
#       (<fab-dir>/renders/<model>/contact-sheet.html),
#   (b) its release-gate.json exists
#       (<fab-dir>/gates/<model>/release-gate.json),
#   (c) the gate is GREEN — no stage carries status "fail"
#       (mirrors stl-release-gate.py's blocking-failed semantics), and
#   (d) the gate is FRESH — its geometry_hash equals the sha256 of the model's
#       current STL (<stl-dir>/<model>.stl), mirroring the engine's hash.
#
# Each failed assertion prints one structured GAP line to stdout
# (`GAP <model>: <reason>`) and makes the helper exit non-zero. A model that
# passes every assertion emits nothing. With zero gaps the helper exits 0.
# Surviving GAP lines become `gap-remediation` issues via the SAME Phase 5.5
# machinery used by every other dimension — this script only DETECTS gaps.
#
# Usage:
#   fab-completeness.sh --fab-dir <.autospec/fab> --stl-dir <build/stls>
#                       [--models m1,m2,...] [--help]
#
# When --models is omitted, the model set is discovered by scanning
# <fab-dir>/gates/*/release-gate.json (one model per gate subdir).
#
# Exit codes:
#   0  every printable model is complete (no gap)
#   1  one or more gaps emitted
#   2  usage error
#
# Requires: bash 3.2+, jq, sha256sum or shasum.

set -uo pipefail

FAB_DIR=""
STL_DIR=""
MODELS_CSV=""

usage() {
  printf 'Usage: fab-completeness.sh --fab-dir <dir> --stl-dir <dir> [--models m1,m2] [--help]\n'
}

# stl_sha256 <file> — sha256 hex digest, or "" if unreadable. Mirrors the
# engine's _geometry_hash (sha256 of the STL bytes).
stl_sha256() {
  [ -f "$1" ] || { printf ''; return 0; }
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

# emit_gap <model> <reason> — print one structured gap line.
emit_gap() {
  printf 'GAP %s: %s\n' "$1" "$2"
}

# check_model <model> — assert sheet + gate (exists/green/fresh); print a GAP
# line per failed assertion. Returns 0 if complete, 1 if any gap emitted.
check_model() {
  model="$1"
  had_gap=0

  sheet="$FAB_DIR/renders/$model/contact-sheet.html"
  gate="$FAB_DIR/gates/$model/release-gate.json"
  stl="$STL_DIR/$model.stl"

  if [ ! -f "$sheet" ]; then
    emit_gap "$model" "missing 16-view contact sheet ($sheet)"
    had_gap=1
  fi

  if [ ! -f "$gate" ]; then
    emit_gap "$model" "missing release-gate.json ($gate)"
    return 1
  fi

  # GREEN: no stage record carries status "fail".
  fail_count="$(jq '[.stages[]? | select(.status == "fail")] | length' "$gate" 2>/dev/null || printf 'ERR')"
  if [ "$fail_count" = "ERR" ]; then
    emit_gap "$model" "release-gate.json is unreadable/invalid ($gate)"
    return 1
  fi
  if [ "${fail_count:-0}" -gt 0 ]; then
    emit_gap "$model" "release-gate.json is not green ($fail_count stage(s) status=fail)"
    had_gap=1
  fi

  # FRESH: gate.geometry_hash must equal the STL's current sha256.
  gate_hash="$(jq -r '.geometry_hash // ""' "$gate" 2>/dev/null || printf '')"
  current_hash="$(stl_sha256 "$stl")"
  if [ -z "$current_hash" ]; then
    emit_gap "$model" "model STL not found for freshness check ($stl)"
    had_gap=1
  elif [ "$gate_hash" != "$current_hash" ]; then
    emit_gap "$model" "release-gate.json is stale (geometry_hash mismatch vs current STL)"
    had_gap=1
  fi

  return "$had_gap"
}

# discover_models — print model names (one per line) from the gates dir.
discover_models() {
  gates_dir="$FAB_DIR/gates"
  [ -d "$gates_dir" ] || return 0
  for d in "$gates_dir"/*/; do
    [ -d "$d" ] || continue
    [ -f "${d}release-gate.json" ] || continue
    name="$(basename "$d")"
    printf '%s\n' "$name"
  done
}

while [ $# -gt 0 ]; do
  case "$1" in
    --fab-dir) FAB_DIR="${2:-}"; shift 2 ;;
    --stl-dir) STL_DIR="${2:-}"; shift 2 ;;
    --models)  MODELS_CSV="${2:-}"; shift 2 ;;
    --help|-h) usage; exit 0 ;;
    *)
      printf 'fab-completeness: unknown option: %s\n' "$1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [ -z "$FAB_DIR" ]; then
  printf 'fab-completeness: --fab-dir is required\n' >&2
  usage >&2
  exit 2
fi

# Resolve the model list: explicit --models (comma-separated) or scan the
# gates dir.
MODELS_FILE="$(mktemp)"
trap 'rm -f "$MODELS_FILE"' EXIT
if [ -n "$MODELS_CSV" ]; then
  saved_ifs="$IFS"
  IFS=','
  # Intentional word-split on comma.
  # shellcheck disable=SC2086
  set -- $MODELS_CSV
  IFS="$saved_ifs"
  for m in "$@"; do
    m="${m#"${m%%[![:space:]]*}"}"
    m="${m%"${m##*[![:space:]]}"}"
    [ -n "$m" ] && printf '%s\n' "$m" >> "$MODELS_FILE"
  done
else
  discover_models >> "$MODELS_FILE"
fi

ANY_GAP=0
while IFS= read -r model; do
  [ -n "$model" ] || continue
  if ! check_model "$model"; then
    ANY_GAP=1
  fi
done < "$MODELS_FILE"

if [ "$ANY_GAP" -ne 0 ]; then
  exit 1
fi
exit 0
