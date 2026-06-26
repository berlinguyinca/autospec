#!/usr/bin/env bash
# autonomous-persona-sources.sh — Gathers and orders operator-persona inputs
# into a structured JSON bundle with global and overlay sub-bundles.
#
# Usage:
#   bash scripts/autonomous-persona-sources.sh [--repo-root DIR] [--autospec-home DIR]
#
# Options:
#   --repo-root     Override repository root (default: parent of scripts/)
#   --autospec-home Override ~/.autospec home directory
#
# Output: JSON to stdout
#   {
#     "global":  [ {source, precedence, path, present:true}, ... ],
#     "overlay": [ {source, precedence, path, present:true}, ... ]
#   }
#
# Source classes and precedences:
#   Global base (operator-level):
#     0  ~/.autospec/operator-persona.answers.json  (interview answers)
#     3  ~/.autospec/persona-mined-digest.json      (F3 cross-repo mined patterns)
#   Overlay (repo-local):
#     1  docs/memory/feedback_*.md, docs/memory/project_*.md, AGENTS.md
#     2  docs/AUTONOMY-CHARTER.md
#     2  .autospec/persona-overlay.md (per-repo override; gitignored, may be absent)
#
# Missing sources are silently skipped — never fatal.
# Arrays in each sub-bundle are sorted by precedence (ascending).
#
# Engineering rules:
#   set -euo pipefail; if/then/fi (no one-sided && short-circuits);
#   jq capture()/== not test() with interpolated values; no RETURN traps.
#
# Exit codes:
#   0  Bundle emitted (even if all sources absent)
#   1  jq not available

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="${REPO_ROOT:-$(cd "$SCRIPT_DIR/.." && pwd)}"
AUTOSPEC_HOME="${AUTOSPEC_HOME:-$HOME/.autospec}"

# ---- argument parsing ----
while [[ $# -gt 0 ]]; do
  case "$1" in
    --repo-root)
      REPO_ROOT="$2"
      shift 2
      ;;
    --autospec-home)
      AUTOSPEC_HOME="$2"
      shift 2
      ;;
    -h|--help)
      sed -n 's/^# \?//p' "$0" | head -30
      exit 0
      ;;
    *)
      echo "autonomous-persona-sources: unknown option: $1" >&2
      exit 1
      ;;
  esac
done

# ---- dependency check ----
if ! command -v jq >/dev/null 2>&1; then
  echo "autonomous-persona-sources: jq not found — required for JSON output" >&2
  exit 1
fi

# ---- temp workspace ----
_tmp_dir="$(mktemp -d -t persona-sources.XXXXXX)"
_global_jsonl="$_tmp_dir/global.jsonl"
_overlay_jsonl="$_tmp_dir/overlay.jsonl"
touch "$_global_jsonl" "$_overlay_jsonl"

# Cleanup on process exit (EXIT trap is safe; not a RETURN trap)
trap 'rm -rf "$_tmp_dir"' EXIT

# emit_to OUT SOURCE PRECEDENCE PATH
#   Appends one JSON entry to OUT if PATH is a regular file.
#   Silently skips absent or non-regular paths.
emit_to() {
  local _out="$1"
  local _source="$2"
  local _prec="$3"
  local _path="$4"

  if [ -f "$_path" ]; then
    jq -n \
      --arg   source     "$_source" \
      --argjson precedence "$_prec" \
      --arg   path       "$_path" \
      '{source:$source, precedence:$precedence, path:$path, present:true}' \
      >> "$_out"
  fi
}

# ---- global-base inputs (operator-level) ----

# Precedence 0: supervised interview answers
emit_to "$_global_jsonl" "interview-answers" 0 \
  "$AUTOSPEC_HOME/operator-persona.answers.json"

# Precedence 3: F3 cross-repo mined decision digest (may be absent — F3 not yet shipped)
emit_to "$_global_jsonl" "mined-digest" 3 \
  "$AUTOSPEC_HOME/persona-mined-digest.json"

# ---- overlay inputs (repo-local) ----

# Precedence 1: docs/memory feedback_* and project_* files
_memory_dir="$REPO_ROOT/docs/memory"
if [ -d "$_memory_dir" ]; then
  for _f in "$_memory_dir"/feedback_*.md "$_memory_dir"/project_*.md; do
    if [ -f "$_f" ]; then
      emit_to "$_overlay_jsonl" "repo-memory" 1 "$_f"
    fi
  done
fi

# Precedence 1: AGENTS.md (repo-level conventions and contracts)
emit_to "$_overlay_jsonl" "agents-md" 1 "$REPO_ROOT/AGENTS.md"

# Precedence 2: docs/AUTONOMY-CHARTER.md (operator autonomy decisions)
emit_to "$_overlay_jsonl" "autonomy-charter" 2 "$REPO_ROOT/docs/AUTONOMY-CHARTER.md"

# Precedence 2: .autospec/persona-overlay.md (per-repo operator override; gitignored)
_per_repo_overlay="$REPO_ROOT/.autospec/persona-overlay.md"
if [ -f "$_per_repo_overlay" ]; then
  emit_to "$_overlay_jsonl" "per-repo-overlay" 2 "$_per_repo_overlay"
fi

# ---- emit final bundle, each sub-bundle sorted by precedence ----
_global_json="$(jq -s 'sort_by(.precedence)' "$_global_jsonl")"
_overlay_json="$(jq -s 'sort_by(.precedence)' "$_overlay_jsonl")"

jq -n \
  --argjson global  "$_global_json" \
  --argjson overlay "$_overlay_json" \
  '{global: $global, overlay: $overlay}'
