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
#     "overlay": [ {source, precedence, path, present:true}, ... ],
#     "meta":    {source_count, confidence}
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

# Per-source byte cap for code-aware gather (issue #1727). Files larger than
# this are skipped so a huge generated/vendored file never dominates the bundle.
: "${AUTOSPEC_PERSONA_SOURCE_MAX_BYTES:=65536}"

# emit_code_source OUT SOURCE PRECEDENCE PATH
#   Like emit_to, but for code-aware text sources (issue #1727): skips vendored
#   paths, binary/empty files, and files exceeding AUTOSPEC_PERSONA_SOURCE_MAX_BYTES.
emit_code_source() {
  local _out="$1"
  local _source="$2"
  local _prec="$3"
  local _path="$4"

  [ -f "$_path" ] || return 0
  case "$_path" in
    */node_modules/*|*/target/*|*/vendor/*|*/dist/*|*/build/*|*/.git/*) return 0 ;;
  esac
  # Binary/empty guard: grep -I reports no match for binary files; `-q .`
  # additionally requires at least one line of text content.
  grep -Iq . "$_path" 2>/dev/null || return 0
  local _size
  _size="$(wc -c < "$_path" 2>/dev/null | tr -d '[:space:]')"
  case "$_size" in
    ''|*[!0-9]*) return 0 ;;
  esac
  [ "$_size" -le "$AUTOSPEC_PERSONA_SOURCE_MAX_BYTES" ] || return 0

  emit_to "$_out" "$_source" "$_prec" "$_path"
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

# ---- code-aware overlay inputs (issue #1727) ----
# Repo intent frequently lives in agent-instruction files, the README, design
# specs, and build manifests rather than in AGENTS.md alone. Gather them so a
# repo with (e.g.) only CLAUDE.md still yields a non-empty bundle. Ranked in the
# overlay (precedence >= 1), always below the precedence-0 interview answers.

# Precedence 1: agent-instruction files (peers of AGENTS.md).
for _agent_file in CLAUDE.md GEMINI.md .cursorrules; do
  emit_code_source "$_overlay_jsonl" "agent-instructions" 1 "$REPO_ROOT/$_agent_file"
done

# Precedence 2: README (first existing variant only).
for _readme in README.md README.markdown README.rst README.txt README; do
  if [ -f "$REPO_ROOT/$_readme" ]; then
    emit_code_source "$_overlay_jsonl" "readme" 2 "$REPO_ROOT/$_readme"
    break
  fi
done

# Precedence 2: design specs under docs/specs (intent, hard rules, doctrine).
if [ -d "$REPO_ROOT/docs/specs" ]; then
  for _spec in "$REPO_ROOT"/docs/specs/*.md; do
    if [ -f "$_spec" ]; then
      emit_code_source "$_overlay_jsonl" "design-spec" 2 "$_spec"
    fi
  done
fi

# Precedence 2: deterministic stack signal — root build manifests name the
# language/framework/domain conventions as real evidence files (not prose).
for _manifest in Cargo.toml package.json go.mod pyproject.toml requirements.txt pom.xml build.sbt; do
  emit_code_source "$_overlay_jsonl" "stack-manifest" 2 "$REPO_ROOT/$_manifest"
done

# ---- emit final bundle, each sub-bundle sorted by precedence ----
_global_json="$(jq -s 'sort_by(.precedence)' "$_global_jsonl")"
_overlay_json="$(jq -s 'sort_by(.precedence)' "$_overlay_jsonl")"

jq -n \
  --argjson global  "$_global_json" \
  --argjson overlay "$_overlay_json" \
  '
  def confidence($global_count; $overlay_count):
    if ($global_count + $overlay_count) == 0 then "none"
    elif ($global_count + $overlay_count) == 1 then "low"
    elif $global_count == 0 then "medium"
    else "high"
    end;
  ($global | length) as $global_count
  | ($overlay | length) as $overlay_count
  | {
      global: $global,
      overlay: $overlay,
      meta: {
        source_count: ($global_count + $overlay_count),
        global_count: $global_count,
        overlay_count: $overlay_count,
        confidence: confidence($global_count; $overlay_count)
      }
    }'
