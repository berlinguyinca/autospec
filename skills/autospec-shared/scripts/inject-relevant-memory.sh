#!/usr/bin/env bash
# inject-relevant-memory.sh — Grep docs/memory/*.md for keyword matches; emit top-k
# filenames + first 200 chars as a context block for skill prompt injection.
#
# Falls back to mempalace search if the CLI is present; otherwise uses grep.
#
# Usage:
#   inject-relevant-memory.sh --context "RETURN trap bash" [--top-k 3] [--memory-dir DIR]
#
# Output (stdout): formatted context block — one section per matched file:
#   ### Memory: <filename>
#   <first 200 chars of file content>
#
# Exit codes:
#   0 always (missing dir / no matches are silent no-ops)
#
# Environment:
#   AUTOSPEC_MEMORY_DIR  — override memory directory (default: auto-detected)
#   AUTOSPEC_SCRIPTS_DIR — directory containing sibling scripts (default: script dir)
#
# Requires: bash 3.2+, grep
# Optional: mempalace CLI (falls back to grep when absent)

set +e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

CONTEXT=""
TOP_K=5
MEMORY_DIR=""

# ── Argument parse ────────────────────────────────────────────────────────────
while [ $# -gt 0 ]; do
  case "$1" in
    --context)
      CONTEXT="$2"
      shift 2
      ;;
    --top-k)
      TOP_K="$2"
      shift 2
      ;;
    --memory-dir)
      MEMORY_DIR="$2"
      shift 2
      ;;
    --help|-h)
      printf 'Usage: inject-relevant-memory.sh --context <keywords> [--top-k N] [--memory-dir DIR]\n'
      exit 0
      ;;
    *)
      shift
      ;;
  esac
done

# ── Resolve memory dir ────────────────────────────────────────────────────────
if [ -z "$MEMORY_DIR" ]; then
  # Try to auto-detect from git repo root
  _repo_root=$(git rev-parse --show-toplevel 2>/dev/null || true)
  if [ -n "$_repo_root" ] && [ -d "$_repo_root/docs/memory" ]; then
    MEMORY_DIR="$_repo_root/docs/memory"
  elif [ -n "${AUTOSPEC_MEMORY_DIR:-}" ] && [ -d "$AUTOSPEC_MEMORY_DIR" ]; then
    MEMORY_DIR="$AUTOSPEC_MEMORY_DIR"
  fi
fi

# ── Guard: no context or no dir → silent no-op ───────────────────────────────
if [ -z "$CONTEXT" ] || [ -z "$MEMORY_DIR" ] || [ ! -d "$MEMORY_DIR" ]; then
  exit 0
fi

# ── Build search terms (split context into words) ────────────────────────────
# Convert context to a grep alternation pattern
_pattern=$(printf '%s' "$CONTEXT" | tr ' ' '|' | sed 's/|$//')

# ── Search: mempalace if available, else grep ─────────────────────────────────
_matched_files=""

if command -v mempalace > /dev/null 2>&1; then
  # Use mempalace search — returns filenames one per line
  _matched_files=$(mempalace search "$CONTEXT" 2>/dev/null \
    | grep -o '[^/]*\.md' \
    | grep -v "^MEMORY\.md$" \
    | head -"$TOP_K" || true)
fi

if [ -z "$_matched_files" ]; then
  # Grep fallback: score files by number of pattern matches, sort descending
  _matched_files=$(grep -ril -E "$_pattern" "$MEMORY_DIR"/*.md 2>/dev/null \
    | grep -v "/MEMORY\.md$" \
    | while IFS= read -r f; do
        count=$(grep -ci -E "$_pattern" "$f" 2>/dev/null || echo 0)
        printf '%s\t%s\n' "$count" "$f"
      done \
    | sort -rn \
    | head -"$TOP_K" \
    | awk '{print $2}' || true)
fi

# ── Emit context block ────────────────────────────────────────────────────────
if [ -z "$_matched_files" ]; then
  exit 0
fi

printf '<!-- inject-relevant-memory: context="%s" top-k=%s -->\n' "$CONTEXT" "$TOP_K"
printf '## Relevant memory\n\n'

while IFS= read -r _file; do
  [ -z "$_file" ] && continue
  [ -f "$_file" ] || _file="$MEMORY_DIR/$_file"
  [ -f "$_file" ] || continue

  _basename=$(basename "$_file")
  # Skip MEMORY.md index
  [ "$_basename" = "MEMORY.md" ] && continue

  # First 200 chars of content (strip frontmatter)
  _content=$(awk '/^---/{c++; next} c>=2{print}' "$_file" 2>/dev/null | head -5 | tr '\n' ' ')
  [ -z "$_content" ] && _content=$(head -5 "$_file" 2>/dev/null | tr '\n' ' ')
  _content=$(printf '%s' "$_content" | cut -c1-200)

  printf '### Memory: %s\n%s\n\n' "$_basename" "$_content"
done <<< "$_matched_files"

exit 0
